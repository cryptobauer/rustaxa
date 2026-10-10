//! Bounded P-only delegation catalog from pinned historical transfer addresses.
//! This diagnostic creates no semantic snapshot or historical execution owner.
use anyhow::{Context, Result, ensure};
use rustaxa_snapshot_qualifier::{native_inverse as inverse, paths, qualification};
use rustaxa_storage::ConcreteStorageInventoryLimits;
use rustaxa_types::{
    FinalChainBlockNumber,
    concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStateIdentity},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
};
use tiny_keccak::{Hasher, Keccak};

const EVIDENCE: &[u8] = include_bytes!("../../../../../doc/evm_research/n4_replay_preflight.json");
const EVIDENCE_SHA: &str = "d68ab554634e7907f2b43ab43f753d6b9351d1b760afc8b91eb3f77c523ae437";
const P: u64 = 25_706_948;
const ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
const INVENTORY_SHA: &str = "a1b2bc6fe3c31e8fde3e597338e2c63a536a4c7badb4de3bcf44a304bc65765c";
const STORAGE_ROOT: &str = "c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504";
const LIMITS: ConcreteStorageInventoryLimits = ConcreteStorageInventoryLimits {
    max_nodes: 50_000,
    max_leaves: 50_000,
    max_value_bytes: 32 * 1024 * 1024,
};
type Seeds = BTreeMap<[u8; 20], BTreeSet<&'static str>>;

fn address(value: &Value) -> Result<[u8; 20]> {
    hex::decode(value.as_str().context("missing address")?)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("address must have twenty bytes"))
}

/// Checks the pinned accepted report before using its bounded address evidence.
fn seeds(bytes: &[u8]) -> Result<(Seeds, Vec<Value>)> {
    ensure!(
        hex::encode(Sha256::digest(bytes)) == EVIDENCE_SHA,
        "transfer evidence hash drift"
    );
    seed_facts(&serde_json::from_slice(bytes)?)
}

fn seed_facts(report: &Value) -> Result<(Seeds, Vec<Value>)> {
    ensure!(
        report["period"] == P + 1
            && report["prior_period"] == P
            && report["prior_root_hex"] == ROOT,
        "transfer evidence identity drift"
    );
    let envelopes = &report["envelope_classification"];
    for name in [
        "every_signature_decoded",
        "every_chain_id_841",
        "every_call",
        "every_input_empty",
    ] {
        ensure!(
            envelopes[name] == true,
            "incomplete transfer evidence: {name}"
        );
    }
    ensure!(
        envelopes["expected_count"] == 19 && envelopes["exact_count"] == 19,
        "transfer count drift"
    );
    let entries = envelopes["entries"]
        .as_array()
        .context("missing envelopes")?;
    ensure!(entries.len() == 19, "wrong envelope count");
    let expected_sender = address(&json!("35307b7b24fb1473abb364f0c3dd3082b3730cd5"))?;
    let mut sources = Seeds::new();
    let mut provenance = Vec::new();
    let mut recipients = BTreeSet::new();
    for (position, entry) in entries.iter().enumerate() {
        ensure!(
            entry["position"].as_u64() == Some(position as u64),
            "envelope order drift"
        );
        ensure!(
            entry["chain_id"] == 841 && entry["kind"] == "call" && entry["input_hex"] == "",
            "envelope class drift"
        );
        let sender = address(&entry["sender_hex"])?;
        let recipient = address(&entry["receiver_hex"])?;
        ensure!(sender == expected_sender, "sender drift");
        ensure!(recipients.insert(recipient), "duplicate recipient");
        for (key, role) in [
            (sender, "retained_transfer_sender"),
            (recipient, "retained_transfer_recipient"),
        ] {
            sources.entry(key).or_default().insert(role);
            provenance.push(json!({"address_hex": hex::encode(key), "role": role, "envelope_position": position, "evidence_sha256": EVIDENCE_SHA}));
        }
    }
    ensure!(
        sources.len() == 20,
        "transfer seed union must contain twenty addresses"
    );
    let accounts = report["prior_state_dependencies"]["accounts"]
        .as_array()
        .context("missing prior accounts")?;
    ensure!(accounts.len() == 20, "prior account count drift");
    let mut prior = BTreeSet::new();
    for account in accounts {
        ensure!(
            account["result"] == "present",
            "prior account is not present"
        );
        ensure!(
            prior.insert(address(&account["address_hex"])?),
            "duplicate prior account"
        );
    }
    ensure!(
        prior == sources.keys().copied().collect(),
        "prior accounts differ from transfer seeds"
    );
    Ok((sources, provenance))
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: parent_transfer_catalog COPY OUTPUT")?,
    );
    let output = PathBuf::from(args.next().context("missing output")?);
    ensure!(args.next().is_none(), "unexpected argument");
    let guarded = paths::validate(&input, &output)?;
    let (sources, provenance) = seeds(EVIDENCE)?;
    let (readers, _, pair) = qualification::qualify(&guarded.application, &guarded.state)?;
    let identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(P),
        state_root: hex::decode(ROOT)?.try_into().expect("fixed root width"),
    };
    let inventory =
        match readers.storage_inventory_at(identity, inverse::DPOS_CONTRACT_ADDRESS, LIMITS)? {
            ConcreteRead::Present(inventory) => inventory,
            other => anyhow::bail!("parent DPoS inventory is not live: {other:?}"),
        };
    ensure!(
        inventory.identity == identity && inventory.address == inverse::DPOS_CONTRACT_ADDRESS,
        "inventory identity drift"
    );
    ensure!(
        inventory.storage_root.map(hex::encode).as_deref() == Some(STORAGE_ROOT)
            && inventory.entries.len() == 23_278
            && inventory.value_bytes == 259_077
            && inverse::digest_entries(&inventory.entries) == INVENTORY_SHA,
        "accepted inventory drift"
    );
    let mut observations = Vec::new();
    let mut observed_bytes = 0_usize;
    let result = inverse::analyze_address_delegations(&inventory, sources, |key| {
        let read = readers.storage_at(identity, inverse::DPOS_CONTRACT_ADDRESS, key);
        let (physical_result, raw) = match &read {
            Ok(ConcreteRead::Present(value)) => {
                observed_bytes = observed_bytes.saturating_add(value.len());
                (
                    "present",
                    (observed_bytes <= inverse::MAX_TRANSFER_VALUE_BYTES)
                        .then(|| hex::encode(value)),
                )
            }
            Ok(ConcreteRead::Absent) => ("absent", None),
            Ok(ConcreteRead::Tombstone) => ("tombstone", None),
            Err(ConcreteReadError::HistoryUnavailable(_)) => ("history_unavailable", None),
            Err(ConcreteReadError::Pruned(_)) => ("pruned", None),
            Err(_) => ("read_error", None),
        };
        let mut path = [0; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&key.0);
        hasher.finalize(&mut path);
        observations.push(json!({"logical_key_hex": hex::encode(key.0), "trie_path_hex": hex::encode(path), "physical_result": physical_result,
            "exact_value_hex": raw,
            "value_sha256": read.as_ref().ok().and_then(|row| match row { ConcreteRead::Present(value) => Some(hex::encode(Sha256::digest(value))), _ => None }),
            "value_bytes": read.as_ref().ok().and_then(|row| match row { ConcreteRead::Present(value) => Some(value.len()), _ => None }), "error": read.as_ref().err().map(|error| format!("{error:?}"))}));
        read
    });
    let successful = result.is_ok();
    let decoded = match result {
        Ok(catalog) => json!({"status": "partial_catalog_decoded", "catalog": catalog}),
        Err(error) => json!({"status": "catalog_rejected", "error": format!("{error:#}")}),
    };
    let report = json!({"schema": 1, "input_copy": guarded.input, "open_mode": "read_only", "pair": pair,
        "period": P, "state_root_hex": ROOT, "account_hex": hex::encode(inverse::DPOS_CONTRACT_ADDRESS), "storage_root_hex": STORAGE_ROOT,
        "inventory": {"entries": inventory.entries.len(), "value_bytes": inventory.value_bytes, "nodes_visited": inventory.nodes_visited, "sha256": INVENTORY_SHA},
        "bounds": {"seeds": 20, "entries": inverse::MAX_TRANSFER_DELEGATIONS, "point_calls": inverse::MAX_TRANSFER_READS,
            "accepted_point_value_bytes": inverse::MAX_TRANSFER_VALUE_BYTES, "inventory_nodes": LIMITS.max_nodes, "inventory_leaves": LIMITS.max_leaves, "inventory_value_bytes": LIMITS.max_value_bytes},
        "source_pins": {"binary_sha256": hex::encode(Sha256::digest(include_bytes!("parent_transfer_catalog.rs"))),
            "inverse_sha256": hex::encode(Sha256::digest(include_bytes!("../native_inverse.rs"))), "evidence_sha256": EVIDENCE_SHA},
        "seed_provenance": provenance, "observations": observations, "result": decoded,
        "qualification": {"fresh_signature_validation": false, "global_delegator_set_complete": false, "historical_key_completeness": false,
            "semantic_snapshot_complete": false, "historical_owner_ready": false, "adoption_authorized": false, "production_routing_authorized": false,
            "additive_coverage_with_base_measured": false}, "reference_policy_assumption": "owner-supplied default mainnet, likely v1.14.1; actual producer unverified"});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(guarded.output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    ensure!(
        successful,
        "catalog rejected; diagnostic observations saved"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_transfer_seed_union_and_drift() {
        let (sources, provenance) = seeds(EVIDENCE).unwrap();
        assert_eq!(sources.len(), 20);
        assert_eq!(provenance.len(), 38);
        assert!(seeds(b"{}").is_err());
        for case in 0..8 {
            let mut report: Value = serde_json::from_slice(EVIDENCE).unwrap();
            match case {
                0 => report["prior_period"] = 0.into(),
                1 => report["prior_root_hex"] = "00".into(),
                2 => report["envelope_classification"]["entries"][0]["position"] = 1.into(),
                3 => report["envelope_classification"]["entries"][0]["receiver_hex"] = "00".into(),
                4 => {
                    report["envelope_classification"]["entries"][1]["receiver_hex"] =
                        report["envelope_classification"]["entries"][0]["receiver_hex"].clone()
                }
                5 => {
                    report["prior_state_dependencies"]["accounts"][0]["address_hex"] =
                        "ff".repeat(20).into()
                }
                6 => report["envelope_classification"]["entries"][0]["chain_id"] = 1.into(),
                _ => report["prior_state_dependencies"]["accounts"][0]["result"] = "absent".into(),
            }
            assert!(seed_facts(&report).is_err(), "case {case}");
        }
    }
}
