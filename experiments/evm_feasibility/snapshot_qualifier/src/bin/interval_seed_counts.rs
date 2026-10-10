//! Fixed signed-envelope lookup seeds; physical slot observations only.
use anyhow::{Context, Result, ensure};
use rustaxa_snapshot_qualifier::{
    interval_history as interval, native_inverse as inverse, parent_history as parent, paths,
    qualification,
};
use rustaxa_types::{
    FinalChainBlockNumber,
    concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStateIdentity, ConcreteStorageKey},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tiny_keccak::{Hasher, Keccak};
const INTERVAL_SHA: &str = "3513c1720759ed3dd07784ab66d69661742af82ceeee56d0c527c835da594aa5";
const OLD_SHA: &str = "4dac8c162b532ae8a5328c40c6045bd7eb7186db844025ae02ded34af4cce7ba";
const ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
const STORAGE_ROOT: &str = "c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504";
const EXPECTED: [&str; 3] = [
    "7d06cc8210228365896a3cc8aac88c5551529112",
    "873415f6633a0c42b8717bc898354638f52b13f3",
    "8dcc94cc57c72eaa6609ee77cb50c84141fa39bd",
];
const ROW_CAP: usize = 4096;
#[cfg(test)]
const TOTAL_CAP: usize = 3 * ROW_CAP;
const OUTPUT_CAP: usize = 1024 * 1024;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Delegation,
    Undelegations,
}
impl Profile {
    fn calls(self) -> usize {
        if self == Self::Delegation { 3 } else { 9 }
    }
    fn status(self) -> &'static str {
        if self == Self::Delegation {
            "three_physical_outcomes_observed"
        } else {
            "nine_physical_outcomes_observed"
        }
    }
    fn slots(self, seed: [u8; 20]) -> Vec<(&'static str, ConcreteStorageKey)> {
        if self == Self::Delegation {
            vec![("delegation_count", inverse::delegation_count_key(seed))]
        } else {
            vec![
                (
                    "v1_validator_count",
                    inverse::undelegation_v1_count_key(seed),
                ),
                (
                    "v2_validator_count",
                    inverse::undelegation_v2_count_key(seed),
                ),
                (
                    "v2_last_allocated_id",
                    inverse::undelegation_v2_last_id_key(seed),
                ),
            ]
        }
    }
}
/// Accept only the explicit nine-read mode; reject unknown or extra flags before I/O.
fn profile(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Profile> {
    let values: Vec<_> = args.into_iter().collect();
    match values.as_slice() {
        [] => Ok(Profile::Delegation),
        [flag] if flag == "--undelegation-counts" => Ok(Profile::Undelegations),
        _ => anyhow::bail!("unexpected argument"),
    }
}
type Seeds = BTreeMap<[u8; 20], Vec<Value>>;
fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn address(value: &str) -> Result<[u8; 20]> {
    hex::decode(value)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("address width differs"))
}
/// Metadata/path/size checks precede bounded file reads and exact source pins.
fn evidence(path: &Path, name: &str, cap: usize) -> Result<Vec<u8>> {
    let canonical = fs::canonicalize(path)?;
    let repo = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))?;
    ensure!(
        !canonical.starts_with(repo.join("data"))
            && !canonical.starts_with(repo.join("local/evm-state-db/snapshot-litenode-copy")),
        "evidence overlaps database tree"
    );
    ensure!(
        canonical.file_name().and_then(|s| s.to_str()) == Some(name),
        "unexpected evidence filename"
    );
    let metadata = fs::metadata(&canonical)?;
    ensure!(
        metadata.is_file() && metadata.len() <= cap as u64,
        "evidence file cap exceeded"
    );
    let mut bytes = Vec::new();
    fs::File::open(canonical)?
        .take(cap as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= cap, "evidence grew beyond cap");
    Ok(bytes)
}
/// Exact report bytes plus fresh offline reauthentication precede state access.
fn seeds(bytes: &[u8], old: &[u8]) -> Result<Seeds> {
    ensure!(
        bytes.len() <= 8 * 1024 * 1024 && old.len() <= 1024 * 1024,
        "evidence cap exceeded"
    );
    ensure!(
        sha(bytes) == INTERVAL_SHA && sha(old) == OLD_SHA,
        "evidence byte pin drift"
    );
    seed_facts(
        &serde_json::from_slice(bytes)?,
        &serde_json::from_slice(old)?,
    )
}
fn seed_facts(report: &Value, old: &Value) -> Result<Seeds> {
    ensure!(
        report["authentication"]["status"] == "complete_interval_lists_authenticated",
        "incomplete interval report"
    );
    let rows: interval::IntervalHistoryRows =
        serde_json::from_value(report["authentication"]["rows"].clone())?;
    let witness = interval::authenticate_interval(&rows)?;
    ensure!(
        witness.first_period == parent::MIN_PERIOD
            && witness.last_period == parent::P
            && witness.periods.len() == 48,
        "interval identity drift"
    );
    ensure!(
        old["period"] == parent::P
            && old["state_root_hex"] == ROOT
            && old["storage_root_hex"] == STORAGE_ROOT
            && old["account_hex"] == "00000000000000000000000000000000000000fe",
        "old seed identity drift"
    );
    let old_facts = old["seed_provenance"]
        .as_array()
        .context("old seed records missing")?;
    ensure!(old_facts.len() == 38, "old provenance count differs");
    let mut previous = BTreeSet::new();
    for fact in old_facts {
        previous.insert(address(
            fact["address_hex"]
                .as_str()
                .context("old address missing")?,
        )?);
    }
    ensure!(previous.len() == 20, "old seed count differs");
    let mut result = Seeds::new();
    for (period_index, period) in witness.periods.iter().enumerate() {
        for fact in &period.position_facts {
            for (field, role) in [
                ("sender_hex", "signed_envelope_sender"),
                ("receiver_hex", "signed_envelope_receiver"),
            ] {
                let Some(value) = fact[field].as_str() else {
                    continue;
                };
                let key = address(value)?;
                if previous.contains(&key) {
                    continue;
                }
                result.entry(key).or_default().push(json!({"address_hex":value,"source_role":role,"period":period.period,
                    "position":fact["position"],"transaction_hash_hex":fact["transaction_hash_hex"],"receipt_status":fact["receipt_status"],
                    "interval_evidence_sha256":INTERVAL_SHA,"period_witness_index":period_index,
                    "raw_transaction_reference":"authentication.witness.periods[period_witness_index].signed_transactions_hex[position]",
                    "raw_receipt_reference":"authentication.witness.periods[period_witness_index].receipts_hex[position]"}));
            }
        }
    }
    ensure!(
        result.keys().copied().collect::<Vec<_>>()
            == EXPECTED
                .iter()
                .map(|v| address(v))
                .collect::<Result<Vec<_>>>()?,
        "novel seed subtraction differs"
    );
    ensure!(
        result.values().map(Vec::len).sum::<usize>() == 34,
        "novel provenance count differs"
    );
    Ok(result)
}
fn identity() -> Result<ConcreteStateIdentity> {
    Ok(ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(parent::P),
        state_root: hex::decode(ROOT)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("root width"))?,
    })
}
/// Calls only the three fixed count keys; every physical class stays distinct.
/// Returned values never trigger child reads or semantic decoding.
fn observe_counts(
    seeds: &Seeds,
    read: impl FnMut(
        ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
) -> Result<Value> {
    observe_profile(seeds, Profile::Delegation, read)
}
/// Observe only the predeclared keys, in address then slot order. Raw classes and
/// partial errors are retained; no result can authorize child reads or decoding.
fn observe_profile(
    seeds: &Seeds,
    profile: Profile,
    mut read: impl FnMut(
        ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
) -> Result<Value> {
    ensure!(
        seeds.keys().copied().collect::<Vec<_>>()
            == EXPECTED
                .iter()
                .map(|s| address(s))
                .collect::<Result<Vec<_>>>()?,
        "unapproved point seed set"
    );
    let mut calls = 0;
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut rows = Vec::new();
    let mut error = None;
    let queries: Vec<_> = seeds
        .keys()
        .flat_map(|seed| {
            profile
                .slots(*seed)
                .into_iter()
                .map(move |(family, key)| (*seed, family, key))
        })
        .collect();
    for (seed, family, key) in queries {
        ensure!(calls < profile.calls(), "storage point cap exceeded");
        calls += 1;
        let mut path = [0; 32];
        let mut hash = Keccak::v256();
        hash.update(&key.0);
        hash.finalize(&mut path);
        let mut row = json!({"address_hex":hex::encode(seed),"slot_role":family,"decoded_last_id":null,"logical_key_hex":hex::encode(key.0),"trie_path_hex":hex::encode(path),"decoded_count":null});
        match read(key) {
            Ok(ConcreteRead::Present(value)) => {
                row["returned_bytes"] = json!(value.len());
                row["returned_sha256"] = json!(sha(&value));
                let total = accepted
                    .checked_add(value.len())
                    .context("point byte overflow")?;
                if value.len() > ROW_CAP || total > profile.calls() * ROW_CAP {
                    rejected = rejected
                        .checked_add(value.len())
                        .context("rejected byte overflow")?;
                    row["physical_result"] = json!("present_rejected_byte_cap");
                    error = Some("point byte cap exceeded".to_owned());
                } else {
                    accepted = total;
                    row["physical_result"] = json!("present");
                    row["raw_hex"] = json!(hex::encode(value));
                }
            }
            Ok(ConcreteRead::Absent) => row["physical_result"] = json!("absent"),
            Ok(ConcreteRead::Tombstone) => row["physical_result"] = json!("tombstone"),
            Err(ConcreteReadError::HistoryUnavailable(_)) => {
                row["physical_result"] = json!("history_unavailable")
            }
            Err(ConcreteReadError::Pruned(_)) => row["physical_result"] = json!("pruned"),
            Err(other) => {
                row["physical_result"] = json!("read_error");
                row["error"] = json!(other.to_string());
                error = Some(other.to_string());
            }
        }
        rows.push(row);
        if error.is_some() {
            break;
        }
    }
    Ok(
        json!({"status":if error.is_none(){profile.status()}else{"observations_rejected"},
        "error":error,"point_calls":calls,"accepted_present_bytes":accepted,"rejected_present_bytes":rejected,"observations":rows}),
    )
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: interval_seed_counts COPY INTERVAL OLD OUTPUT")?,
    );
    let interval_path = PathBuf::from(args.next().context("missing interval report")?);
    let old_path = PathBuf::from(args.next().context("missing old catalog")?);
    let output = PathBuf::from(args.next().context("missing output")?);
    let profile = profile(args)?;
    let guarded = paths::validate(&input, &output)?;
    let source = seeds(
        &evidence(
            &interval_path,
            "interval-history-authentication.json",
            8 * 1024 * 1024,
        )?,
        &evidence(&old_path, "parent-transfer-catalog.json", 1024 * 1024)?,
    )?;
    let (readers, _, pair) = qualification::qualify(&guarded.application, &guarded.state)?;
    let identity = identity()?;
    let read = |key| readers.storage_at(identity, inverse::DPOS_CONTRACT_ADDRESS, key);
    let observations = match profile {
        Profile::Delegation => observe_counts(&source, read),
        Profile::Undelegations => observe_profile(&source, profile, read),
    }?;
    let successful = observations["status"] == profile.status();
    let report = json!({"schema":1,"profile":if profile == Profile::Delegation {"delegation_counts"} else {"undelegation_counts_and_last_id"},"input_copy":guarded.input,"period":parent::P,"state_root_hex":ROOT,
        "account_hex":hex::encode(inverse::DPOS_CONTRACT_ADDRESS),"inherited_storage_root_hex":STORAGE_ROOT,
        "open_mode":format!("existing read-only pair qualification, then {} P storage points", profile.calls()),"pair":pair,
        "source":{"interval_sha256":INTERVAL_SHA,"old_catalog_sha256":OLD_SHA,"old_catalog_origin":"accepted_prior_report"},
        "seed_provenance":source.values().flatten().collect::<Vec<_>>(),"result":observations,
        "bounds":{"diagnostic_storage_calls":profile.calls(),"qualification_application_point_reads":4,"qualification_owner_root_checks":"existing owner, separate from diagnostic counter",
            "row_present_bytes":ROW_CAP,"total_present_bytes":profile.calls() * ROW_CAP,"serialized_output_bytes":OUTPUT_CAP},
        "source_pins":{"adapter_source_sha256":sha(include_bytes!("interval_seed_counts.rs")),"inverse_source_sha256":sha(include_bytes!("../native_inverse.rs")),
            "interval_verifier_source_sha256":sha(include_bytes!("../interval_history.rs"))},
        "qualification":{"fresh_inventory_performed":false,"live_membership_checked":false,"new_live_partition_computed":false,"semantic_counts_decoded":false,
            "historical_key_completeness":false,"semantic_snapshot_complete":false,"historical_owner_ready":false,
            "reward_transition_executed":false,"adoption_authorized":false,"production_routing_authorized":false},
        "reference_policy_assumption":"owner-supplied default mainnet, likely v1.14.1; actual producer unverified"});
    let (bytes, successful) = match interval::encode_bounded(&report, OUTPUT_CAP) {
        Ok(bytes) => (bytes, successful),
        Err(error) => (
            interval::encode_bounded(
                &json!({"status":"output_cap_rejected","error":error.to_string()}),
                OUTPUT_CAP,
            )?,
            false,
        ),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(guarded.output)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    ensure!(successful, "physical observations rejected; report saved");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn old() -> Value {
        serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/parent_transfer_catalog.json"
        ))
        .unwrap()
    }
    fn report() -> Value {
        let rows: Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/interval_history_rows.json"
        ))
        .unwrap();
        json!({"authentication":{"status":"complete_interval_lists_authenticated","rows":rows}})
    }
    fn keys() -> Seeds {
        EXPECTED
            .iter()
            .map(|s| (address(s).unwrap(), vec![]))
            .collect()
    }
    #[test]
    fn undelegation_keys_order_and_fixed_bounds() {
        assert!(profile(["--unknown".into()]).is_err());
        assert!(profile(["--undelegation-counts".into(), "extra".into()]).is_err());
        assert!(profile([]).unwrap() == Profile::Delegation);
        assert!(profile(["--undelegation-counts".into()]).unwrap() == Profile::Undelegations);
        let mut expected = Vec::new();
        for seed in keys().keys() {
            for (prefix, suffix) in [
                (vec![3, 1], vec![1]),
                (vec![3, 2], vec![1]),
                (vec![3, 4], vec![]),
            ] {
                let mut bytes = prefix;
                bytes.extend_from_slice(seed);
                bytes.extend(suffix);
                let mut key = [0; 32];
                let mut hash = Keccak::v256();
                hash.update(&bytes);
                hash.finalize(&mut key);
                assert_ne!(key, inverse::delegation_count_key(*seed).0);
                expected.push(ConcreteStorageKey(key));
            }
        }
        assert_eq!(
            expected.iter().map(|k| k.0).collect::<BTreeSet<_>>().len(),
            9
        );
        let mut calls = 0;
        let out = observe_profile(&keys(), Profile::Undelegations, |key| {
            assert_eq!(key, expected[calls]);
            calls += 1;
            Ok(ConcreteRead::Present(vec![0; ROW_CAP]))
        })
        .unwrap();
        assert_eq!(calls, 9);
        assert_eq!(out["accepted_present_bytes"], 9 * ROW_CAP);
        for row in out["observations"].as_array().unwrap() {
            assert!(row["decoded_count"].is_null());
            assert!(row["decoded_last_id"].is_null());
        }
        let id = identity().unwrap();
        let mut values = vec![
            Ok(ConcreteRead::Absent),
            Ok(ConcreteRead::Tombstone),
            Err(ConcreteReadError::HistoryUnavailable(id)),
            Err(ConcreteReadError::Pruned(id)),
            Ok(ConcreteRead::Present(vec![])),
            Err(ConcreteReadError::Io("stop".into())),
        ]
        .into_iter();
        let out = observe_profile(&keys(), Profile::Undelegations, |_| {
            values.next().expect("no expansion")
        })
        .unwrap();
        assert_eq!(out["point_calls"], 6);
        assert_eq!(out["status"], "observations_rejected");
        let out = observe_profile(&keys(), Profile::Undelegations, |_| {
            Ok(ConcreteRead::Present(vec![0; ROW_CAP + 1]))
        })
        .unwrap();
        assert_eq!(out["point_calls"], 1);
        assert_eq!(out["rejected_present_bytes"], ROW_CAP + 1);
        assert!(
            observe_profile(&Seeds::new(), Profile::Undelegations, |_| panic!(
                "unapproved seed"
            ))
            .is_err()
        );
        assert!(interval::encode_bounded(&json!("x".repeat(OUTPUT_CAP)), OUTPUT_CAP).is_err());
    }
    #[test]
    fn source_subtraction_preserves_every_role_and_rejects_drift() {
        let report = report();
        let seeds = seed_facts(&report, &old()).unwrap();
        assert_eq!(seeds.len(), 3);
        assert_eq!(
            seeds.values().map(Vec::len).collect::<Vec<_>>(),
            vec![2, 2, 30]
        );
        assert!(super::seeds(b"changed", b"changed").is_err());
        let mut old = old();
        old["period"] = json!(parent::P + 1);
        assert!(seed_facts(&report, &old).is_err());
        let mut report = report.clone();
        report["authentication"]["status"] = json!("rejected");
        assert!(seed_facts(&report, &self::old()).is_err());
    }
    #[test]
    fn physical_classes_never_expand_or_decode_and_errors_stop() {
        let identity = identity().unwrap();
        let mut values = vec![
            Ok(ConcreteRead::Present(vec![0xff; 4])),
            Ok(ConcreteRead::Absent),
            Ok(ConcreteRead::Tombstone),
        ]
        .into_iter();
        let out = observe_counts(&keys(), |_| values.next().expect("extra point")).unwrap();
        assert_eq!(out["point_calls"], 3);
        for row in out["observations"].as_array().unwrap() {
            assert!(row["decoded_count"].is_null());
        }
        let mut values = vec![
            Err(ConcreteReadError::HistoryUnavailable(identity)),
            Err(ConcreteReadError::Pruned(identity)),
            Ok(ConcreteRead::Present(vec![])),
        ]
        .into_iter();
        let out = observe_counts(&keys(), |_| values.next().expect("extra point")).unwrap();
        assert_eq!(out["point_calls"], 3);
        assert_eq!(
            out["observations"][0]["physical_result"],
            "history_unavailable"
        );
        assert_eq!(out["observations"][1]["physical_result"], "pruned");
        let mut calls = 0;
        let out = observe_counts(&keys(), |_| {
            calls += 1;
            assert_eq!(calls, 1);
            Err(ConcreteReadError::Io("fail".into()))
        })
        .unwrap();
        assert_eq!(out["point_calls"], 1);
        let out = observe_counts(&keys(), |_| Ok(ConcreteRead::Present(vec![0; ROW_CAP]))).unwrap();
        assert_eq!(out["accepted_present_bytes"], TOTAL_CAP);
        let mut calls = 0;
        let out = observe_counts(&keys(), |_| {
            calls += 1;
            assert_eq!(calls, 1);
            Ok(ConcreteRead::Present(vec![0; ROW_CAP + 1]))
        })
        .unwrap();
        assert_eq!(out["rejected_present_bytes"], ROW_CAP + 1);
        assert!(out["observations"][0]["raw_hex"].is_null());
        assert!(observe_counts(&Seeds::new(), |_| panic!("unapproved point")).is_err());
        let key = inverse::delegation_count_key(address(EXPECTED[0]).unwrap());
        let mut data = vec![2, 1];
        data.extend_from_slice(&address(EXPECTED[0]).unwrap());
        data.push(1);
        let mut expected = [0; 32];
        let mut hash = Keccak::v256();
        hash.update(&data);
        hash.finalize(&mut expected);
        assert_eq!(key.0, expected);
    }
}
