//! Bounded read-only partial inversion of the qualified head DPoS inventory.

#[path = "native_inverse_coverage/seeded_undelegations.rs"]
mod seeded_undelegations;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options};
use rustaxa_snapshot_qualifier::native_inverse::{
    DPOS_CONTRACT_ADDRESS, NativeInverseCoverage, analyze_native_head, digest_entries,
};
use rustaxa_storage::{
    Column, ConcreteCheckpointReaders, ConcreteStorageInventoryLimits, FinalChainRepository,
};
use rustaxa_types::FinalChainBlockNumber;
use rustaxa_types::StoredFinalChainBlockHeader;
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStateIdentity};
use serde::Serialize;
use sha2::{Digest, Sha256};

use seeded_undelegations::{
    SeededUndelegationCoverage, analyze_seeded_undelegations, hashed_storage_path,
};

const TARGET_PERIOD: u64 = 25_706_949;
const MAX_NODES: u64 = 50_000;
const MAX_LEAVES: u64 = 50_000;
const MAX_VALUE_BYTES: u64 = 32 * 1024 * 1024;
type Database = DBWithThreadMode<MultiThreaded>;

#[derive(Serialize)]
struct Report {
    schema: u32,
    tool_source_sha256: String,
    input_copy: String,
    open_mode: &'static str,
    identity: IdentityReport,
    limits: LimitsReport,
    inventory: InventoryReport,
    coverage: NativeInverseCoverage,
    seeded_undelegations: SeededUndelegationCoverage,
    qualification: Qualification,
}

#[derive(Serialize)]
struct IdentityReport {
    period: u64,
    state_root_hex: String,
    dpos_address_hex: String,
    dpos_storage_root_hex: String,
}

#[derive(Serialize)]
struct LimitsReport {
    max_nodes: u64,
    max_leaves: u64,
    max_value_bytes: u64,
}

#[derive(Serialize)]
struct InventoryReport {
    nodes_visited: u64,
    live_entries: u64,
    live_value_bytes: u64,
    entries_sha256: String,
}

#[derive(Serialize)]
struct Qualification {
    authoritative_head_header_selected: bool,
    concrete_descriptor_matches_header: bool,
    complete_live_dpos_inventory_authenticated: bool,
    enumerable_rows_strictly_decoded: bool,
    seeded_undelegation_rows_strictly_decoded: bool,
    matched_and_unexplained_partition_live_inventory: bool,
    historical_key_coverage_qualified: bool,
    semantic_dpos_snapshot_complete: bool,
    checkpoint_adoption_authorized: bool,
    production_routing_authorized: bool,
}

fn main() -> Result<()> {
    let (input, output, scout) = validated_paths()?;
    if scout {
        return head_sender_scout(&input, &output);
    }
    let app_path = canonical_child(&input, "db/db")?;
    let state_path = canonical_child(&input, "db/state_db")?;

    let application = Arc::new(open_application_read_only(&app_path)?);
    let final_chain = FinalChainRepository::new(application);
    let head = exact_le_u64(
        &final_chain
            .meta_value(1)?
            .context("missing FinalChain head metadata")?,
        "FinalChain head",
    )?;
    ensure!(
        head == TARGET_PERIOD,
        "expected qualified head {TARGET_PERIOD}, observed {head}"
    );
    let header_raw = final_chain
        .block_header_raw(head)?
        .context("qualified head header is missing")?;
    let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&header_raw))
        .context("decode qualified head header")?;
    let identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(head),
        state_root: header.state_root.into(),
    };

    let readers = ConcreteCheckpointReaders::open_read_only(&state_path, identity, [identity])?;
    ensure!(
        readers.committed_identity() == identity,
        "concrete descriptor differs from qualified head header"
    );
    let inventory = match readers.storage_inventory_at(
        identity,
        DPOS_CONTRACT_ADDRESS,
        ConcreteStorageInventoryLimits {
            max_nodes: MAX_NODES,
            max_leaves: MAX_LEAVES,
            max_value_bytes: MAX_VALUE_BYTES,
        },
    )? {
        ConcreteRead::Present(inventory) => inventory,
        ConcreteRead::Absent => anyhow::bail!("DPoS account is absent at qualified head"),
        ConcreteRead::Tombstone => anyhow::bail!("DPoS account is tombstoned at qualified head"),
    };
    let storage_root = inventory
        .storage_root
        .context("qualified DPoS account has no storage root")?;
    let live_inventory = inventory
        .entries
        .iter()
        .map(|entry| (entry.hashed_path, entry.value.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let mut base_matched_paths = BTreeSet::new();
    let coverage = analyze_native_head(&inventory, |key| {
        let read = readers.storage_at(identity, DPOS_CONTRACT_ADDRESS, key);
        if let Ok(ConcreteRead::Present(value)) = &read {
            let path = hashed_storage_path(key);
            if live_inventory
                .get(&path)
                .is_some_and(|inventory_value| *inventory_value == value)
            {
                base_matched_paths.insert(path);
            }
        }
        read
    })?;
    ensure!(
        base_matched_paths.len() == usize::try_from(coverage.matched_live_entries)?,
        "observed base live paths differ from reported matched count"
    );
    let base_matched_value_bytes =
        base_matched_paths
            .iter()
            .try_fold(0_u64, |total, path| -> Result<u64> {
                total
                    .checked_add(u64::try_from(
                        live_inventory
                            .get(path)
                            .context("observed base live path is absent from inventory")?
                            .len(),
                    )?)
                    .context("observed base live value-byte count overflow")
            })?;
    ensure!(
        base_matched_value_bytes == coverage.matched_live_value_bytes,
        "observed base live bytes differ from reported matched byte count"
    );
    let seeded_addresses = coverage
        .seeded_delegations
        .candidates
        .iter()
        .map(|candidate| {
            let address = hex::decode(&candidate.address_hex)
                .context("decode authenticated seeded delegator address")?;
            address.try_into().map_err(|address: Vec<u8>| {
                anyhow::anyhow!(
                    "authenticated seeded delegator address has {} bytes",
                    address.len()
                )
            })
        })
        .collect::<Result<BTreeSet<[u8; 20]>>>()?;
    ensure!(
        seeded_addresses.len() == coverage.seeded_delegations.candidates.len(),
        "authenticated seeded delegator addresses contain duplicates"
    );
    let seeded_undelegations = analyze_seeded_undelegations(
        &inventory,
        &base_matched_paths,
        base_matched_value_bytes,
        seeded_addresses,
        |key| readers.storage_at(identity, DPOS_CONTRACT_ADDRESS, key),
    )?;
    ensure!(
        coverage.matched_live_entries + coverage.unexplained_live_entries
            == u64::try_from(inventory.entries.len())?,
        "coverage does not partition authenticated live inventory"
    );

    let report = Report {
        schema: 2,
        tool_source_sha256: tool_source_sha256(),
        input_copy: input.display().to_string(),
        open_mode: "application DB and ConcreteCheckpointReaders opened read-only",
        identity: IdentityReport {
            period: identity.period.as_u64(),
            state_root_hex: hex::encode(identity.state_root),
            dpos_address_hex: hex::encode(DPOS_CONTRACT_ADDRESS),
            dpos_storage_root_hex: hex::encode(storage_root),
        },
        limits: LimitsReport {
            max_nodes: MAX_NODES,
            max_leaves: MAX_LEAVES,
            max_value_bytes: MAX_VALUE_BYTES,
        },
        inventory: InventoryReport {
            nodes_visited: inventory.nodes_visited,
            live_entries: u64::try_from(inventory.entries.len())?,
            live_value_bytes: inventory.value_bytes,
            entries_sha256: digest_entries(&inventory.entries),
        },
        qualification: Qualification {
            authoritative_head_header_selected: true,
            concrete_descriptor_matches_header: true,
            complete_live_dpos_inventory_authenticated: true,
            enumerable_rows_strictly_decoded: true,
            seeded_undelegation_rows_strictly_decoded: true,
            matched_and_unexplained_partition_live_inventory: seeded_undelegations
                .live_partition_exact,
            historical_key_coverage_qualified: false,
            semantic_dpos_snapshot_complete: false,
            checkpoint_adoption_authorized: false,
            production_routing_authorized: false,
        },
        coverage,
        seeded_undelegations,
    };
    write_report(&output, &report)
}

fn open_application_read_only(path: &Path) -> Result<Database> {
    let mut options = Options::default();
    options.create_if_missing(false);
    options.create_missing_column_families(false);
    options.set_max_open_files(128);
    let columns = Database::list_cf(&options, path)?;
    for required in ["default", "final_chain_meta", "final_chain_blk_by_number"] {
        ensure!(
            columns.iter().any(|column| column == required),
            "application column family {required:?} is missing"
        );
    }
    let descriptors = columns.iter().map(|name| {
        Column::from_name(name).map_or_else(
            |_| ColumnFamilyDescriptor::new(name, Options::default()),
            |column| column.descriptor(&Options::default()),
        )
    });
    Ok(Database::open_cf_descriptors_read_only(
        &options,
        path,
        descriptors,
        false,
    )?)
}

fn validated_paths() -> Result<(PathBuf, PathBuf, bool)> {
    let mut args = env::args_os().skip(1);
    let first = args
        .next()
        .context("usage: native_inverse_coverage [--head-sender-scout] COPY OUTPUT")?;
    let scout = first == "--head-sender-scout";
    let input = PathBuf::from(if scout {
        args.next().context("missing copy")?
    } else {
        first
    });
    let output = PathBuf::from(args.next().context("missing output")?);
    ensure!(args.next().is_none(), "unexpected argument");
    let paths = rustaxa_snapshot_qualifier::paths::validate(&input, &output)?;
    Ok((paths.input, paths.output, scout))
}

fn canonical_child(input: &Path, relative: &str) -> Result<PathBuf> {
    // Shared policy validates both children before opening either database.
    Ok(input.join(relative))
}

fn head_sender_scout(input: &Path, output: &Path) -> Result<()> {
    validate_sender_provenance(include_bytes!(
        "../../../../../doc/evm_research/n4_replay_preflight.json"
    ))?;
    let (readers, identity, pair) = rustaxa_snapshot_qualifier::qualification::qualify(
        &input.join("db/db"),
        &input.join("db/state_db"),
    )?;
    let sender: [u8; 20] = hex::decode("35307b7b24fb1473abb364f0c3dd3082b3730cd5")?
        .try_into()
        .expect("fixed sender width");
    let observations = seeded_undelegations::scout_seeded_address(sender, |key| {
        readers.storage_at(identity, DPOS_CONTRACT_ADDRESS, key)
    })?;
    let successful_logical_reads = observations
        .iter()
        .filter(|row| matches!(row.physical_result, "present" | "absent" | "tombstone"))
        .count();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    serde_json::to_writer_pretty(
        &mut file,
        &serde_json::json!({
            "schema": 1, "mode": "head_sender_scout", "input_copy": input, "tool_source_sha256": tool_source_sha256(), "pair": pair,
            "sender_hex": hex::encode(sender), "sender_provenance": "historical doc/evm_research/n4_replay_preflight.json envelope entries 0..18 and recovered_sender_scout.md; not fresh signature validation",
            "sender_evidence_sha256": "d68ab554634e7907f2b43ab43f753d6b9351d1b760afc8b91eb3f77c523ae437",
        "fresh_transaction_signature_validation": false,
        "open_mode": "application and concrete checkpoint owners read-only",
        "dpos_address_hex": hex::encode(DPOS_CONTRACT_ADDRESS),
        "checkpoint_adoption_authorized": false, "publication_authorized": false, "production_routing_authorized": false,
            "attempted_logical_reads": observations.len(), "successful_logical_reads": successful_logical_reads,
            "logical_read_limit": 4, "child_enumeration_performed": false, "broad_inventory_performed": false,
            "logical_membership_authenticated": false, "semantic_snapshot_complete": false, "observations": observations
        }),
    )?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

/// Validates historical report bytes and all bounded sender facts without
/// claiming a fresh transaction/signature decode on the restored snapshot.
fn validate_sender_provenance(bytes: &[u8]) -> Result<()> {
    ensure!(
        hex::encode(Sha256::digest(bytes))
            == "d68ab554634e7907f2b43ab43f753d6b9351d1b760afc8b91eb3f77c523ae437",
        "historical sender evidence byte hash differs"
    );
    validate_sender_facts(&serde_json::from_slice(bytes)?)
}

fn validate_sender_facts(report: &serde_json::Value) -> Result<()> {
    ensure!(
        report["period"].as_u64() == Some(TARGET_PERIOD),
        "historical sender evidence period differs"
    );
    let envelope = &report["envelope_classification"];
    ensure!(
        envelope["expected_count"].as_u64() == Some(19)
            && envelope["exact_count"].as_u64() == Some(19),
        "historical sender count differs"
    );
    ensure!(
        envelope["every_signature_decoded"].as_bool() == Some(true),
        "historical signature evidence incomplete"
    );
    let entries = envelope["entries"]
        .as_array()
        .context("missing sender entries")?;
    ensure!(entries.len() == 19, "historical sender entry count differs");
    for (position, entry) in entries.iter().enumerate() {
        ensure!(
            entry["position"].as_u64() == Some(position as u64),
            "historical sender position differs"
        );
        ensure!(
            entry["sender_hex"].as_str() == Some("35307b7b24fb1473abb364f0c3dd3082b3730cd5"),
            "historical sender differs"
        );
    }
    Ok(())
}

fn write_report(path: &Path, report: &Report) -> Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut output, report)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    Ok(())
}

fn exact_le_u64(bytes: &[u8], label: &str) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .try_into()
            .with_context(|| format!("{label} is not eight bytes"))?,
    ))
}

fn tool_source_sha256() -> String {
    let mut digest = Sha256::new();
    digest.update(include_bytes!("../native_inverse.rs"));
    digest.update(include_bytes!("../paths.rs"));
    digest.update(include_bytes!("../qualification.rs"));
    digest.update(include_bytes!("native_inverse_coverage.rs"));
    digest.update(include_bytes!(
        "native_inverse_coverage/seeded_undelegations.rs"
    ));
    hex::encode(digest.finalize())
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    const EVIDENCE: &[u8] =
        include_bytes!("../../../../../doc/evm_research/n4_replay_preflight.json");
    #[test]
    fn historical_sender_evidence_is_bound_and_rejects_drift() {
        validate_sender_provenance(EVIDENCE).unwrap();
        assert!(validate_sender_provenance(b"{}").is_err());
        let source: serde_json::Value = serde_json::from_slice(EVIDENCE).unwrap();
        for field in [
            "period",
            "count",
            "signature",
            "sender",
            "position",
            "missing",
        ] {
            let mut changed = source.clone();
            match field {
                "period" => changed["period"] = 0.into(),
                "count" => changed["envelope_classification"]["exact_count"] = 20.into(),
                "signature" => {
                    changed["envelope_classification"]["every_signature_decoded"] = false.into()
                }
                "sender" => {
                    changed["envelope_classification"]["entries"][0]["sender_hex"] = "00".into()
                }
                "position" => {
                    changed["envelope_classification"]["entries"][0]["position"] = 1.into()
                }
                _ => {
                    changed["envelope_classification"]["entries"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
            }
            assert!(validate_sender_facts(&changed).is_err(), "{field}");
        }
    }
}
