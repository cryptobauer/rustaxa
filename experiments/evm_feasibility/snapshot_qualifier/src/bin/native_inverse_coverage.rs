//! Bounded read-only partial inversion of the qualified head DPoS inventory.

#[path = "native_inverse_coverage/seeded_undelegations.rs"]
mod seeded_undelegations;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
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

const QUALIFIED_COPY: &str = "/tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy";
const FORBIDDEN_ORIGINAL: &str = "/tmp/snapshot-litenode";
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
    let (input, output) = validated_paths()?;
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

fn validated_paths() -> Result<(PathBuf, PathBuf)> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: native_inverse_coverage QUALIFIED_COPY OUTPUT_JSON")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: native_inverse_coverage QUALIFIED_COPY OUTPUT_JSON")?,
    );
    ensure!(
        args.next().is_none(),
        "usage: native_inverse_coverage QUALIFIED_COPY OUTPUT_JSON"
    );

    let input = fs::canonicalize(input)?;
    let qualified = fs::canonicalize(QUALIFIED_COPY)?;
    ensure!(
        input == qualified,
        "only the recorded qualified copy is accepted"
    );
    ensure!(
        input != Path::new(FORBIDDEN_ORIGINAL) && !input.starts_with(FORBIDDEN_ORIGINAL),
        "refusing original snapshot"
    );
    ensure!(!output.exists(), "output already exists");
    let name = output.file_name().context("output path has no file name")?;
    let parent = fs::canonicalize(output.parent().unwrap_or_else(|| Path::new(".")))?;
    let output = parent.join(name);
    ensure!(
        !output.starts_with(&input) && !output.starts_with(FORBIDDEN_ORIGINAL),
        "output must remain outside snapshot paths"
    );
    Ok((input, output))
}

fn canonical_child(input: &Path, relative: &str) -> Result<PathBuf> {
    let child = fs::canonicalize(input.join(relative))?;
    ensure!(
        child.starts_with(input) && !child.starts_with(FORBIDDEN_ORIGINAL),
        "snapshot child escapes qualified copy"
    );
    Ok(child)
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
    digest.update(include_bytes!("native_inverse_coverage.rs"));
    digest.update(include_bytes!(
        "native_inverse_coverage/seeded_undelegations.rs"
    ));
    hex::encode(digest.finalize())
}
