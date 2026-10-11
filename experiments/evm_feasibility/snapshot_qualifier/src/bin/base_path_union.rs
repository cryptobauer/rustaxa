//! Fixed saved-evidence base/undelegation path-union adapter. No physical probe
//! or semantic snapshot qualification is supplied by path-union evidence.
#[path = "parent_cache_fields/admission.rs"]
mod admission;
#[path = "base_path_union/ledger.rs"]
mod ledger;
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{env, path::Path};
const INPUTS: [(&str, usize, &str); 3] = [
    (
        "/home/fry/artifacts/light-snapshot-check-20261010/availability.json",
        1088773,
        "b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1",
    ),
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/parent-raw-inventory.json",
        2894865,
        "63dc30da528d294418f237e24e580d78d17653de643847140dedfac4faf51056",
    ),
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2/offline-undelegations-final.json",
        1117732,
        "cf63dd21b22d249b4013bfcc9d94b1ea89ee1354982e320d5f194ee922157d53",
    ),
];
struct Bounded(Vec<u8>);
impl std::io::Write for Bounded {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if b.len() > (16 * 1024 * 1024usize).saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("output byte cap"));
        }
        self.0.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().collect();
    ensure!(args.len() == 2, "usage: base_path_union OUTPUT");
    let output = Path::new(&args[1]);
    admission::outside_db(
        &output
            .parent()
            .ok_or_else(|| anyhow::anyhow!("output parent"))?
            .canonicalize()?,
    )?;
    ensure!(!output.exists(), "output already exists");
    let mut bytes = Vec::new();
    let mut total = 0usize;
    for (path, size, pin) in INPUTS {
        total = total
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("input size overflow"))?;
        ensure!(total <= 8 * 1024 * 1024, "aggregate input cap");
        bytes.push(admission::evidence(
            Path::new(path),
            Path::new(path),
            pin,
            size,
            4 * 1024 * 1024,
        )?);
    }
    // Admit all whole-file pins before any JSON parsing.
    let inputs: Vec<Value> = bytes
        .iter()
        .map(|b| admission::parse_unique(b))
        .collect::<Result<_>>()?;
    for key in [
        "head",
        "prior_period",
        "state_root_hex",
        "prior_state_root_hex",
        "genesis_hash_hex",
        "header_sha256",
    ] {
        ensure!(
            inputs[0]["pair"][key] == inputs[1]["pair"][key],
            "cross-input H/P pair drift: {key}"
        );
    }
    ensure!(
        inputs[0]["pair"]["head"] == 25706949
            && inputs[0]["pair"]["prior_period"] == 25706948
            && inputs[0]["pair"]["genesis_hash_hex"]
                == "8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377",
        "fixed H/P/genesis identity"
    );
    let raw = ledger::Raw::admit(&inputs[1])?;
    let base = ledger::rebuild(&inputs[0], &raw)?;
    eprintln!(
        "base derivations={} matched={} bytes={}",
        base.lookups,
        base.rows.len(),
        base.bytes
    );
    let mut report = ledger::union_report(&inputs[0], &raw, &base, &inputs[2])?;
    report["inputs"] = serde_json::json!(
        INPUTS
            .iter()
            .map(|(p, n, h)| serde_json::json!({"path":p,"bytes":n,"sha256":h}))
            .collect::<Vec<_>>()
    );
    report["implementation_pins"] = serde_json::json!({"adapter_sha256":admission::sha(include_bytes!("base_path_union.rs")),"ledger_sha256":admission::sha(include_bytes!("base_path_union/ledger.rs")),"shared_admission_sha256":admission::sha(include_bytes!("parent_cache_fields/admission.rs")),"native_inverse_sha256":admission::sha(include_bytes!("../native_inverse.rs")),"base_rows_sha256":admission::sha(include_bytes!("../native_inverse/base_rows.rs"))});
    let mut encoded = Bounded(Vec::new());
    serde_json::to_writer(&mut encoded, &report)?;
    admission::publish_exclusive(output, &encoded.0)?;
    println!("union report_sha256={}", admission::sha(&encoded.0));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn saved_inputs() -> Vec<Value> {
        let bytes: Vec<_> = INPUTS
            .iter()
            .map(|(p, n, h)| {
                admission::evidence(Path::new(p), Path::new(p), h, *n, 4 * 1024 * 1024).unwrap()
            })
            .collect();
        bytes
            .iter()
            .map(|b| admission::parse_unique(b).unwrap())
            .collect()
    }
    #[test]
    fn admitted_real_base_and_exact_union_reject_identity_order_role_and_link_drift() {
        let inputs = saved_inputs();
        let raw = ledger::Raw::admit(&inputs[1]).unwrap();
        let base = ledger::rebuild(&inputs[0], &raw).unwrap();
        let report = ledger::union_report(&inputs[0], &raw, &base, &inputs[2]).unwrap();
        assert_eq!(report["partitions"]["base"]["rows"], 2065);
        assert_eq!(report["partitions"]["undelegation"]["rows"], 217);
        assert_eq!(
            report["partitions"]["union"]["rows"].as_u64().unwrap()
                + report["partitions"]["union_residual"]["rows"]
                    .as_u64()
                    .unwrap(),
            23278
        );
        for (pointer, value) in [
            ("/native_dependencies/0/period", serde_json::json!(25706949)),
            (
                "/native_dependencies/0/accounts/0/address_hex",
                serde_json::json!("00000000000000000000000000000000000000ff"),
            ),
            (
                "/native_dependencies/0/state_root_hex",
                serde_json::json!("wrong"),
            ),
            (
                "/native_dependencies/0/accounts/0/inverse/coverage/validators/0/position",
                serde_json::json!(2),
            ),
            (
                "/native_dependencies/0/accounts/0/inverse/coverage/validators/0/stake_hex",
                serde_json::json!("01"),
            ),
            (
                "/native_dependencies/0/accounts/0/inverse/coverage/validators/0/owner_hex",
                serde_json::json!("0000000000000000000000000000000000000000"),
            ),
            (
                "/native_dependencies/0/accounts/0/inverse/coverage/seeded_delegations/candidates/0/sources",
                serde_json::json!(["owner"]),
            ),
            (
                "/native_dependencies/0/accounts/0/inverse/coverage/seeded_delegations/delegations/0/reward_node_result",
                serde_json::json!("live_present_already_matched"),
            ),
            (
                "/native_dependencies/0/accounts/0/inverse/coverage/unexplained_paths_sha256",
                serde_json::json!("wrong"),
            ),
        ] {
            let mut changed = inputs[0].clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(ledger::rebuild(&changed, &raw).is_err(), "{pointer}");
        }
        let mut changed = inputs[0].clone();
        let duplicate = changed["native_dependencies"][0].clone();
        changed["native_dependencies"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(ledger::rebuild(&changed, &raw).is_err());
        for pointer in ["/entries_sha256", "/account_hex", "/state_root_hex"] {
            let mut changed = inputs[1].clone();
            *changed.pointer_mut(pointer).unwrap() = serde_json::json!("wrong");
            assert!(ledger::Raw::admit(&changed).is_err());
        }
        let mut changed = inputs[1].clone();
        changed["entries"].as_array_mut().unwrap().swap(0, 1);
        assert!(ledger::Raw::admit(&changed).is_err());
        let mut changed = inputs[1].clone();
        changed["entries"][1] = changed["entries"][0].clone();
        assert!(ledger::Raw::admit(&changed).is_err());
        // A synthetic additional B member tests a nonempty overlap; this result
        // is unpublished and does not assert that the real fixed B has this row.
        let tuple = inputs[2]["tuples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["status"] == "validated_live")
            .unwrap();
        let path: [u8; 32] = hex::decode(tuple["trie_path_hex"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert!(!base.rows.contains_key(&path));
        let mut synthetic = base.clone();
        synthetic.rows.insert(
            path,
            ledger::Row {
                preimage: hex::decode(tuple["preimage_hex"].as_str().unwrap()).unwrap(),
                key: hex::decode(tuple["logical_key_hex"].as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap(),
                refs: vec![serde_json::json!({"synthetic_reference":true})],
                family: "synthetic_base".into(),
            },
        );
        let overlap = ledger::union_report(&inputs[0], &raw, &synthetic, &inputs[2]).unwrap();
        assert_eq!(overlap["partitions"]["intersection"]["rows"], 1);
        assert_eq!(
            overlap["overlaps"][0]["base_primary_family"],
            "synthetic_base"
        );
        assert!(
            overlap["overlaps"][0]["base_provenance"]
                .as_array()
                .unwrap()
                .len()
                == 1
                && !overlap["overlaps"][0]["undelegation_tuple_indexes"]
                    .as_array()
                    .unwrap()
                    .is_empty()
        );
        synthetic.rows.get_mut(&path).unwrap().key = [0; 32];
        assert!(ledger::union_report(&inputs[0], &raw, &synthetic, &inputs[2]).is_err());
        let live = inputs[2]["tuples"]
            .as_array()
            .unwrap()
            .iter()
            .position(|t| t["status"] == "validated_live")
            .unwrap();
        for (field, value) in [
            ("logical_key_hex", serde_json::json!("00")),
            ("raw_row_index", serde_json::json!(0)),
            ("raw_hex", serde_json::json!("00")),
            ("status", serde_json::json!("invalid_live")),
        ] {
            let mut changed = inputs[2].clone();
            changed["tuples"][live][field] = value;
            assert!(ledger::union_report(&inputs[0], &raw, &base, &changed).is_err());
        }
        let mut changed = inputs[2].clone();
        changed["coverage"]["matched_paths_hex"][1] =
            changed["coverage"]["matched_paths_hex"][0].clone();
        assert!(ledger::union_report(&inputs[0], &raw, &base, &changed).is_err());
    }
    #[test]
    fn output_writer_accepts_exact_cap_and_rejects_one_extra_byte() {
        use std::io::Write;
        let mut writer = Bounded(Vec::new());
        writer.write_all(&vec![0; 16 * 1024 * 1024]).unwrap();
        assert_eq!(writer.0.len(), 16 * 1024 * 1024);
        assert_eq!(
            writer.write(&[1]).unwrap_err().to_string(),
            "output byte cap"
        );
        assert_eq!(writer.0.len(), 16 * 1024 * 1024);
    }
    #[test]
    fn shared_admission_rejects_duplicate_json_growth_and_exclusive_overwrite() {
        assert!(admission::parse_unique(br#"{"same":1,"same":2}"#).is_err());
        assert!(admission::read_capped([0u8; 5].as_slice(), 4).is_err());
        let folder = std::path::PathBuf::from("/home/fry/artifacts").join(format!(
            "base-union-publication-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let output = folder.join("report.json");
        std::fs::write(&output, b"retained").unwrap();
        assert!(admission::publish_exclusive(&output, b"replacement").is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"retained");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}
