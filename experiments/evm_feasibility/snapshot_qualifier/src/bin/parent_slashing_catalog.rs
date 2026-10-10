//! Fixed P-only slashing diagnostic from byte-pinned current validator evidence.
use anyhow::{Context, Result, ensure};
use rustaxa_snapshot_qualifier::{
    native_inverse, native_slashing as slashing, paths, qualification,
};
use rustaxa_storage::ConcreteStorageInventoryLimits;
use rustaxa_types::{
    FinalChainBlockNumber,
    concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStateIdentity},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

const P: u64 = 25_706_948;
const ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
const EVIDENCE_SHA: &str = "b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1";
const MAX_EVIDENCE_BYTES: u64 = 2 * 1024 * 1024;
const STORAGE_ROOT: &str = "d688bfddb847c241b44f78c048b41d7e423f138bc5ee8fe6c24e6f047b051a7a";
const INVENTORY_SHA: &str = "8309d0e7fc00c05fdec78037aee0f09dd24586d73ea4aa6457f5b3c8cd9ff5ef";
const LIMITS: ConcreteStorageInventoryLimits = ConcreteStorageInventoryLimits {
    max_nodes: 256,
    max_leaves: 128,
    max_value_bytes: 4096,
};

/// Hash evidence before parsing; its external path has no authority of its own.
fn validator_seeds(bytes: &[u8]) -> Result<Vec<slashing::JailSeed>> {
    ensure!(
        bytes.len() <= MAX_EVIDENCE_BYTES as usize,
        "availability evidence bound exceeded"
    );
    ensure!(
        hex::encode(Sha256::digest(bytes)) == EVIDENCE_SHA,
        "availability evidence hash drift"
    );
    seed_facts(&serde_json::from_slice(bytes)?)
}
fn seed_facts(report: &Value) -> Result<Vec<slashing::JailSeed>> {
    let periods = report["native_dependencies"]
        .as_array()
        .context("missing native dependencies")?;
    let selected = periods
        .iter()
        .filter(|row| row["period"] == P)
        .collect::<Vec<_>>();
    ensure!(selected.len() == 1, "P evidence must be unique");
    let parent = selected[0];
    ensure!(
        parent["state_root_hex"] == ROOT && parent["root_authenticated"] == true,
        "P evidence identity drift"
    );
    let accounts = parent["accounts"]
        .as_array()
        .context("missing native accounts")?;
    let dpos = accounts
        .iter()
        .filter(|row| row["address_hex"] == hex::encode(native_inverse::DPOS_CONTRACT_ADDRESS))
        .collect::<Vec<_>>();
    ensure!(
        dpos.len() == 1 && dpos[0]["status"] == "complete_live_inventory",
        "DPoS evidence account drift"
    );
    ensure!(
        dpos[0]["inverse"]["status"] == "partial_inverse_completed",
        "DPoS inverse evidence failed"
    );
    let coverage = &dpos[0]["inverse"]["coverage"];
    let validators = coverage["validators"]
        .as_array()
        .context("missing validator evidence")?;
    ensure!(
        coverage["validator_count"] == 196 && validators.len() == slashing::MAX_JAIL_SEEDS,
        "validator count drift"
    );
    let mut seen = BTreeSet::new();
    let mut seeds = Vec::new();
    for (index, row) in validators.iter().enumerate() {
        ensure!(
            row["position"].as_u64() == Some(index as u64 + 1),
            "validator source order drift"
        );
        let address: [u8; 20] = hex::decode(
            row["address_hex"]
                .as_str()
                .context("missing validator address")?,
        )?
        .try_into()
        .map_err(|_| anyhow::anyhow!("validator address width drift"))?;
        ensure!(seen.insert(address), "duplicate validator evidence");
        seeds.push(slashing::JailSeed {
            address,
            source_position: index as u32 + 1,
        });
    }
    Ok(seeds)
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: parent_slashing_catalog COPY OUTPUT AVAILABILITY_REPORT")?,
    );
    let output = PathBuf::from(args.next().context("missing output")?);
    let evidence = PathBuf::from(args.next().context("missing availability report")?);
    ensure!(args.next().is_none(), "unexpected argument");
    let guarded = paths::validate(&input, &output)?;
    let mut raw_evidence = Vec::new();
    File::open(&evidence)?
        .take(MAX_EVIDENCE_BYTES + 1)
        .read_to_end(&mut raw_evidence)?;
    let seeds = validator_seeds(&raw_evidence)?;
    let (readers, _, pair) = qualification::qualify(&guarded.application, &guarded.state)?;
    let identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(P),
        state_root: hex::decode(ROOT)?.try_into().expect("fixed root width"),
    };
    let inventory =
        match readers.storage_inventory_at(identity, slashing::SLASHING_ADDRESS, LIMITS)? {
            ConcreteRead::Present(inventory) => inventory,
            other => anyhow::bail!("P slashing inventory is not live: {other:?}"),
        };
    ensure!(
        inventory.identity == identity && inventory.address == slashing::SLASHING_ADDRESS,
        "inventory identity drift"
    );
    ensure!(
        inventory.storage_root.map(hex::encode).as_deref() == Some(STORAGE_ROOT)
            && inventory.entries.len() == 54
            && inventory.value_bytes == 128
            && inventory.nodes_visited == 71
            && native_inverse::digest_entries(&inventory.entries) == INVENTORY_SHA,
        "accepted slashing inventory drift"
    );
    let mut observations = Vec::new();
    let mut observed_bytes = 0_usize;
    let result = slashing::analyze_slashing_jails(&inventory, &seeds, |key| {
        let read = readers.storage_at(identity, slashing::SLASHING_ADDRESS, key);
        let (physical_result, value) = match &read {
            Ok(ConcreteRead::Present(value)) => {
                observed_bytes = observed_bytes.saturating_add(value.len());
                ("present", Some(value))
            }
            Ok(ConcreteRead::Absent) => ("absent", None),
            Ok(ConcreteRead::Tombstone) => ("tombstone", None),
            Err(ConcreteReadError::HistoryUnavailable(_)) => ("history_unavailable", None),
            Err(ConcreteReadError::Pruned(_)) => ("pruned", None),
            Err(_) => ("read_error", None),
        };
        observations.push(json!({"logical_key_hex": hex::encode(key.0), "physical_result": physical_result,
            "exact_value_hex": value.filter(|_| observed_bytes <= slashing::MAX_JAIL_POINT_BYTES).map(hex::encode),
            "value_sha256": value.map(|raw| hex::encode(Sha256::digest(raw))), "value_bytes": value.map(Vec::len),
            "error": read.as_ref().err().map(|error| format!("{error:?}"))}));
        read
    });
    let successful = result.is_ok();
    let decoded = match result {
        Ok(catalog) => json!({"status": "partial_catalog_decoded", "catalog": catalog}),
        Err(error) => json!({"status": "catalog_rejected", "error": format!("{error:#}")}),
    };
    let report = json!({"schema": 1, "input_copy": guarded.input, "open_mode": "read_only", "pair": pair,
        "period": P, "state_root_hex": ROOT, "account_hex": hex::encode(slashing::SLASHING_ADDRESS), "storage_root_hex": STORAGE_ROOT,
        "inventory": {"nodes_visited": inventory.nodes_visited, "live_entries": inventory.entries.len(), "value_bytes": inventory.value_bytes, "sha256": INVENTORY_SHA},
        "bounds": {"nodes": LIMITS.max_nodes, "leaves": LIMITS.max_leaves, "inventory_bytes": LIMITS.max_value_bytes,
            "seeds": slashing::MAX_JAIL_SEEDS, "point_calls": slashing::MAX_JAIL_READS, "point_value_bytes": slashing::MAX_JAIL_POINT_BYTES, "evidence_bytes": MAX_EVIDENCE_BYTES},
        "source_pins": {"binary_sha256": hex::encode(Sha256::digest(include_bytes!("parent_slashing_catalog.rs"))),
            "decoder_sha256": hex::encode(Sha256::digest(include_bytes!("../native_slashing.rs"))), "availability_sha256": EVIDENCE_SHA,
            "evidence_path": evidence}, "seed_basis": "196 accepted P validator positions; no all-ever jail address claim",
        "observations": observations, "result": decoded,
        "qualification": {"all_ever_jail_keys_complete": false, "proof_key_catalog_complete": false, "semantic_snapshot_complete": false,
            "delayed_eligibility_result": false, "historical_owner_ready": false, "production_routing_authorized": false, "adoption_authorized": false},
        "reference_policy_assumption": "owner-supplied default mainnet, likely v1.14.1; actual producer unverified"});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(guarded.output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    ensure!(successful, "slashing catalog rejected; observations saved");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence() -> Value {
        let validators = (1..=196)
            .map(|n| json!({"position": n, "address_hex": hex::encode([n as u8; 20])}))
            .collect::<Vec<_>>();
        json!({"native_dependencies": [{"period": P, "state_root_hex": ROOT, "root_authenticated": true, "accounts": [{"address_hex": hex::encode(native_inverse::DPOS_CONTRACT_ADDRESS),
            "status": "complete_live_inventory", "inverse": {"status": "partial_inverse_completed", "coverage": {"validator_count": 196, "validators": validators}}}]}]})
    }
    #[test]
    fn validator_evidence_order_width_identity_and_hash_guards() {
        let base = evidence();
        assert_eq!(seed_facts(&base).unwrap().len(), 196);
        assert!(validator_seeds(b"{}").is_err());
        assert!(validator_seeds(&vec![0; MAX_EVIDENCE_BYTES as usize + 1]).is_err());
        for case in 0..8 {
            let mut report = base.clone();
            match case {
                0 => report["native_dependencies"][0]["period"] = 0.into(),
                1 => report["native_dependencies"][0]["state_root_hex"] = "00".into(),
                2 => report["native_dependencies"][0]["accounts"][0]["address_hex"] = "00".into(),
                3 => {
                    report["native_dependencies"][0]["accounts"][0]["inverse"]["coverage"]["validators"]
                        [0]["position"] = 2.into()
                }
                4 => {
                    report["native_dependencies"][0]["accounts"][0]["inverse"]["coverage"]["validators"]
                        [0]["address_hex"] = "00".into()
                }
                5 => {
                    report["native_dependencies"][0]["accounts"][0]["inverse"]["coverage"]["validators"]
                        [1]["address_hex"] = hex::encode([1; 20]).into()
                }
                6 => {
                    let row = report["native_dependencies"][0].clone();
                    report["native_dependencies"]
                        .as_array_mut()
                        .unwrap()
                        .push(row);
                }
                _ => {
                    report["native_dependencies"][0]["accounts"][0]["inverse"]["coverage"]["validator_count"] =
                        195.into()
                }
            }
            assert!(seed_facts(&report).is_err(), "case {case}");
        }
    }
}
