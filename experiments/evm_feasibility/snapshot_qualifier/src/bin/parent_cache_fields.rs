//! Two fixed saved artifacts; no DB path or physical callback.
#[path = "parent_cache_fields/admission.rs"]
mod admission;
use admission::*;
use anyhow::{Context, Result, ensure};
use rustaxa_snapshot_qualifier::{
    interval_history::{self, IntervalHistoryRows},
    parent_reward_fields::{self, CacheExpectedRow, ParentCacheMetadata},
};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
};
const SOURCES: [(&str, usize, &str); 10] = [
    (
        "experiments/evm_feasibility/snapshot_qualifier/src/interval_history.rs",
        11245,
        "44e1430d4a0206325de12a7c804f964760b6d261b83f1954185761ca2f3be6e4",
    ),
    (
        "experiments/evm_feasibility/snapshot_qualifier/src/parent_history.rs",
        23298,
        "46786d43ff10d076d425c23f901b149e462b139e138ee253852bbe7a3554319b",
    ),
    (
        "experiments/evm_feasibility/snapshot_qualifier/src/head_history.rs",
        19542,
        "cc0ea9b27712d67e75ec46b6a858dbfcdeb65affce048339e3ad6cfcb624379b",
    ),
    (
        "experiments/evm_feasibility/snapshot_qualifier/src/reward_votes.rs",
        2443,
        "9854994cf3cd49677b132c26095d7a01b2add8e5370a64f90ed75337879a577c",
    ),
    (
        "rust/crates/rustaxa-types/src/codec/rlp/dag.rs",
        7430,
        "8b8efb5c5b3741027bf3ef73b0bebb9dbc7e9e96e369806d45fab9c325e33c04",
    ),
    (
        "rust/crates/rustaxa-types/src/dag.rs",
        4217,
        "1076ec5c6c6710dc51042a5d6c621615ee59e033b3654ec257b2bad7317f6ed0",
    ),
    (
        "rust/crates/rustaxa-types/src/codec/rlp/pbft.rs",
        6906,
        "3793a52d31c7c5fbccbb2d795d2a2bf9199996489af0ab24478dad8a2a8c0a85",
    ),
    (
        "rust/crates/rustaxa-consensus/src/pbft_vote_validation.rs",
        65589,
        "cd5670f1ea93167b1ef4075be6b52e81d9a35d9ad2823da98f3c6d1a6531a38d",
    ),
    (
        "libraries/core_libs/consensus/src/pbft/pbft_manager.cpp",
        108544,
        "9da5866ace56961e6c81fb06308eaa1f9be906f78facc25265182c70126d3060",
    ),
    (
        "libraries/types/dag_block/src/dag_block.cpp",
        6236,
        "82b778175acff3ccb01686e1c2d2015c15bfa94b14e921f8879b69775669dad6",
    ),
];
const INPUTS: [(&str, usize, &str); 2] = [
    (
        "/home/fry/artifacts/light-snapshot-check-20261010/availability.json",
        1088773,
        "b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1",
    ),
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/interval-history-authentication.json",
        2766744,
        "3513c1720759ed3dd07784ab66d69661742af82ceeee56d0c527c835da594aa5",
    ),
];
fn pinned() -> Result<([Vec<u8>; 2], Vec<Value>)> {
    let repo = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))?;
    let mut data = Vec::new();
    let mut total = 0usize;
    for (p, n, h) in INPUTS {
        let bytes = evidence(Path::new(p), Path::new(p), h, n, n)?;
        total = total
            .checked_add(bytes.len())
            .context("input sum overflow")?;
        ensure!(total <= 3855517, "combined evidence cap");
        data.push(bytes);
    }
    let mut sources = Vec::new();
    for (p, n, h) in SOURCES {
        ensure!(n <= 1024 * 1024, "source cap");
        let path = repo.join(p);
        evidence(&path, &path, h, n, 1024 * 1024)?;
        sources.push(json!({"path":p,"bytes":n,"sha256":h}));
    }
    sources.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok((
        data.try_into()
            .map_err(|_| anyhow::anyhow!("source count"))?,
        sources,
    ))
}
fn report(data: &[Vec<u8>; 2], sources: Vec<Value>) -> Result<Value> {
    let availability = parse_unique(&data[0])?;
    let history = parse_unique(&data[1])?;
    ensure!(
        history["schema"] == 1
            && history["authentication"]["witness"]["source_authenticated_to_local_h"] == true,
        "accepted interval report identity"
    );
    let rows: IntervalHistoryRows =
        serde_json::from_value(history["authentication"]["rows"].clone())?;
    ensure!(
        availability["reward_cache"]["bytes"] == 27006,
        "cache aggregate metadata bytes"
    );
    let metadata: Vec<CacheExpectedRow> =
        serde_json::from_value(availability["reward_cache"]["rows"].clone())?;
    let mut value = parent_reward_fields::derive_parent_reward_fields(
        &rows,
        &ParentCacheMetadata { rows: metadata },
    )?
    .value;
    value["schema"] = json!(1);
    value["scope"] = json!("available signed parent fields; missing native/rate/cache inputs");
    value["base_commit"] = json!("604664af8ecdee6c25eb1c24c2f42865b5e1724a");
    value["source_manifest"] = json!(sources);
    value["inputs"] = json!(
        INPUTS
            .iter()
            .map(|(p, n, h)| json!({"path":p,"bytes":n,"sha256":h}))
            .collect::<Vec<_>>()
    );
    value["reference_policy"] = json!({"basis_id":"owner-supplied default mainnet, likely v1.14.1","producer_verified":false,"chain":841,"delay":5,"committee":1000,"frequency":100,"interval_first":25706901,"minimum_difficulty_policy_qualified":false});
    value["bounds"] = json!({"input_bytes":3855517,"output_bytes":8388608,"periods":48,"certificate_entries":914,"certificates_per_period":20,"dag_blocks":139,"dags_per_period":10,"transactions":583,"symbolic_facts":2838});
    value["runtime"] = json!({"database_opens":0,"storage_points":0,"new_inventories":0});
    value["implementation_pins"] = json!({"adapter_sha256":sha(include_bytes!("parent_cache_fields.rs")),"admission_sha256":sha(include_bytes!("parent_cache_fields/admission.rs")),"core_sha256":sha(include_bytes!("../parent_reward_fields.rs"))});
    ensure!(
        value.as_object().context("complete report")?.len() == 16,
        "complete output schema"
    );
    parent_reward_fields::validate_field_report(&value)?;
    value["digests"]["inputs_digest"] = json!(parent_reward_fields::normalized_digest(
        &json!({"inputs":value["inputs"],"source_manifest":value["source_manifest"],"reference_policy":value["reference_policy"]})
    )?);
    let content = parent_reward_fields::normalized_digest(&value)?;
    value["digests"]["report_content_digest"] = json!(content);
    Ok(value)
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let output = PathBuf::from(args.next().context("usage: parent_cache_fields OUTPUT")?);
    ensure!(
        args.next().is_none() && !output.to_string_lossy().starts_with("--"),
        "unexpected mode"
    );
    let parent = fs::canonicalize(output.parent().context("output parent")?)?;
    outside_db(&parent)?;
    let output = parent.join(output.file_name().context("output file")?);
    ensure!(!output.exists(), "output exists");
    let (data, sources) = pinned()?;
    let value = report(&data, sources)?;
    let bytes = interval_history::encode_bounded(&value, 8388608)?;
    publish_exclusive(&output, &bytes)?;
    println!("parent field report_sha256={}", sha(&bytes));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::OpenOptions, io::Write};
    #[test]
    fn strict_json_and_bounded_admission() {
        assert!(parse_unique(br#"{"a":1,"a":2}"#).is_err());
        assert!(parse_unique(br#"{"nested":{"a":1,"a":2}}"#).is_err());
        assert!(parse_unique(br#"{"a":1.5}"#).is_err());
        assert!(parse_unique(br#"{"a":1} trailing"#).is_err());
        assert_eq!(parse_unique(br#"{"a":1}"#).unwrap(), json!({"a":1}));
        assert!(read_capped(std::io::Cursor::new([0; 5]), 4).is_err());
        assert!(interval_history::encode_bounded(&json!({"large":"abc"}), 2).is_err());
        let repo =
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap();
        assert!(outside_db(&repo.join("data/x")).is_err());
        assert!(outside_db(&repo.join("local/evm-state-db/snapshot-litenode-copy/x")).is_err());
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/cache-fields-admission-fixture");
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
            .unwrap();
        f.write_all(b"{}").unwrap();
        assert!(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&p)
                .is_err()
        );
        assert!(evidence(&p, &p, &"00".repeat(32), 2, 2).is_err());
        assert!(evidence(&p, &p, &sha(b"{}"), 3, 3).is_err());
        assert!(publish_exclusive(&p, b"replacement").is_err());
        assert_eq!(fs::read(&p).unwrap(), b"{}");
        let target = p.with_file_name("cache-fields-publish-fixture");
        assert!(!target.exists());
        publish_exclusive(&target, b"complete").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"complete");
        fs::remove_file(target).unwrap();
        fs::remove_file(p).unwrap();
    }
}
