//! Fixed P/DPoS raw retention and offline digest verification; no semantic model.
use anyhow::{Context, Result, ensure};
use rustaxa_snapshot_qualifier::{interval_history, native_inverse, paths, qualification};
use rustaxa_storage::{ConcreteStorageInventoryEntry, ConcreteStorageInventoryLimits};
use rustaxa_types::{
    FinalChainBlockNumber,
    concrete_state::{ConcreteRead, ConcreteStateIdentity},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
const P: u64 = 25_706_948;
const ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
const STORAGE_ROOT: &str = "c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504";
const DIGEST: &str = "a1b2bc6fe3c31e8fde3e597338e2c63a536a4c7badb4de3bcf44a304bc65765c";
const ACCOUNT: &str = "00000000000000000000000000000000000000fe";
const OUTPUT_CAP: usize = 64 * 1024 * 1024;
const LIMITS: ConcreteStorageInventoryLimits = ConcreteStorageInventoryLimits {
    max_nodes: 50_000,
    max_leaves: 50_000,
    max_value_bytes: 32 * 1024 * 1024,
};
const DIGEST_VERSION: &str = "sha256-ordered-path32-length-u64be-value-v1";
/// Fixed production pins; tests may supply small explicit expectations to this pure verifier.
struct Expected<'a> {
    rows: usize,
    bytes: u64,
    digest: &'a str,
}
fn expected() -> Expected<'static> {
    Expected {
        rows: 23_278,
        bytes: 259_077,
        digest: DIGEST,
    }
}
fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn canonical_hex(value: &Value) -> Result<Vec<u8>> {
    let s = value.as_str().context("hex string missing")?;
    ensure!(
        s.len() % 2 == 0
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "noncanonical hex"
    );
    Ok(hex::decode(s)?)
}
/// Verify all saved raw rows against fixed authenticated prior pins. No DB access.
/// File/serialized caps apply before parse; row/hex byte caps apply before decoding.
fn verify(bytes: &[u8], pins: Expected<'_>) -> Result<Vec<ConcreteStorageInventoryEntry>> {
    ensure!(bytes.len() <= OUTPUT_CAP, "artifact size cap");
    let value: Value = serde_json::from_slice(bytes)?;
    ensure!(
        value["schema"] == 1
            && value["status"] == "raw_capture_complete"
            && value["digest_version"] == DIGEST_VERSION,
        "artifact schema/status"
    );
    ensure!(
        value["period"] == P
            && value["state_root_hex"] == ROOT
            && value["account_hex"] == ACCOUNT
            && value["storage_root_hex"] == STORAGE_ROOT,
        "artifact identity/root"
    );
    ensure!(
        value["nodes_visited"]
            .as_u64()
            .is_some_and(|n| n > 0 && n <= LIMITS.max_nodes),
        "node cap"
    );
    let rows = value["entries"].as_array().context("raw entries missing")?;
    ensure!(
        rows.len() <= LIMITS.max_leaves as usize && rows.len() == pins.rows,
        "entry count"
    );
    let mut entries = Vec::with_capacity(rows.len());
    let mut total = 0u64;
    let mut previous = None;
    for row in rows {
        ensure!(
            row["hashed_path_hex"]
                .as_str()
                .is_some_and(|s| s.len() == 64),
            "path width"
        );
        let path = canonical_hex(&row["hashed_path_hex"])?;
        let path: [u8; 32] = path.try_into().map_err(|_| anyhow::anyhow!("path width"))?;
        ensure!(
            previous.is_none_or(|p| p < path),
            "duplicate or unsorted path"
        );
        previous = Some(path);
        let hex = row["value_hex"].as_str().context("value missing")?;
        ensure!(
            hex.len() as u64 <= LIMITS.max_value_bytes * 2,
            "row byte cap"
        );
        total = total
            .checked_add((hex.len() / 2) as u64)
            .context("value byte overflow")?;
        ensure!(total <= LIMITS.max_value_bytes, "total byte cap");
        entries.push(ConcreteStorageInventoryEntry {
            hashed_path: path,
            value: canonical_hex(&row["value_hex"])?,
        });
    }
    ensure!(
        total == pins.bytes
            && value["value_bytes"] == pins.bytes
            && value["row_count"] == pins.rows,
        "value/count summary drift"
    );
    let digest = native_inverse::digest_entries(&entries);
    ensure!(
        digest == pins.digest && value["entries_sha256"] == pins.digest,
        "ordered raw digest drift"
    );
    for name in [
        "historical_key_completeness",
        "logical_catalog_completeness",
        "semantic_snapshot_complete",
        "historical_owner_ready",
        "adoption_authorized",
        "production_routing_authorized",
    ] {
        ensure!(value["qualification"][name] == false, "claim drift: {name}");
    }
    Ok(entries)
}
/// Metadata and cap-plus-one bounded reads detect oversized or growing artifacts.
fn artifact_bytes(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= OUTPUT_CAP as u64,
        "artifact file cap"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(OUTPUT_CAP as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= OUTPUT_CAP, "artifact grew beyond cap");
    Ok(bytes)
}
fn write_all(mut writer: impl Write, bytes: &[u8]) -> Result<()> {
    writer.write_all(bytes)?;
    writer.flush()?;
    Ok(())
}
/// Exclusive output creation follows bounded serialization. Any write/sync error fails.
fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= OUTPUT_CAP, "output cap");
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    write_all(&mut file, bytes)?;
    file.sync_all()?;
    Ok(())
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let first = args
        .next()
        .context("usage: parent_raw_inventory COPY OUTPUT | --verify ARTIFACT")?;
    if first == "--verify" {
        let path = PathBuf::from(args.next().context("missing artifact")?);
        ensure!(args.next().is_none(), "unexpected argument");
        let bytes = artifact_bytes(&path)?;
        let entries = verify(&bytes, expected())?;
        println!(
            "verified {} rows; artifact_sha256={}",
            entries.len(),
            sha(&bytes)
        );
        return Ok(());
    }
    let input = PathBuf::from(first);
    let output = PathBuf::from(args.next().context("missing output")?);
    ensure!(args.next().is_none(), "unexpected argument");
    let guarded = paths::validate(&input, &output)?;
    let (readers, _, pair) = qualification::qualify(&guarded.application, &guarded.state)?;
    let identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(P),
        state_root: hex::decode(ROOT)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("fixed root width"))?,
    };
    let inventory = match readers.storage_inventory_at(
        identity,
        native_inverse::DPOS_CONTRACT_ADDRESS,
        LIMITS,
    )? {
        ConcreteRead::Present(v) => v,
        other => anyhow::bail!("inventory not present: {other:?}"),
    };
    ensure!(
        inventory.identity == identity
            && inventory.address == native_inverse::DPOS_CONTRACT_ADDRESS
            && inventory.storage_root.map(hex::encode).as_deref() == Some(STORAGE_ROOT),
        "capture identity/root drift"
    );
    let artifact = json!({"schema":1,"status":"raw_capture_complete","digest_version":DIGEST_VERSION,"period":P,"state_root_hex":ROOT,"account_hex":ACCOUNT,"storage_root_hex":STORAGE_ROOT,
        "input_copy":guarded.input,"open_mode":"existing read-only qualification then one bounded P DPoS inventory","pair":pair,
        "nodes_visited":inventory.nodes_visited,"row_count":inventory.entries.len(),"value_bytes":inventory.value_bytes,"entries_sha256":native_inverse::digest_entries(&inventory.entries),
        "entries":inventory.entries.iter().map(|e|json!({"hashed_path_hex":hex::encode(e.hashed_path),"value_hex":hex::encode(&e.value)})).collect::<Vec<_>>(),
        "bounds":{"inventory_calls":1,"direct_storage_points":0,"qualification_application_point_reads":4,"nodes":LIMITS.max_nodes,"leaves":LIMITS.max_leaves,"raw_value_bytes":LIMITS.max_value_bytes,"serialized_bytes":OUTPUT_CAP},
        "source":{"accepted_base":"a3d8ad3","tool_source_sha256":sha(include_bytes!("parent_raw_inventory.rs")),"inverse_source_sha256":sha(include_bytes!("../native_inverse.rs")),"qualification_source_sha256":sha(include_bytes!("../qualification.rs"))},
        "qualification":{"historical_key_completeness":false,"logical_catalog_completeness":false,"semantic_snapshot_complete":false,"historical_owner_ready":false,"adoption_authorized":false,"production_routing_authorized":false},
        "reference_policy_assumption":"owner-supplied default mainnet, likely v1.14.1; actual producer unverified",
        "authority_limit":"exact raw retention and digest verification against authenticated prior evidence; no standalone Merkle proof"});
    let bytes = interval_history::encode_bounded(&artifact, OUTPUT_CAP)?;
    verify(&bytes, expected())?;
    save(&guarded.output, &bytes)?;
    let saved = artifact_bytes(&guarded.output)?;
    verify(&saved, expected())?;
    ensure!(saved == bytes, "saved artifact differs");
    println!(
        "captured {} rows; artifact_sha256={}",
        inventory.entries.len(),
        sha(&saved)
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Value, String) {
        let entries = vec![
            ConcreteStorageInventoryEntry {
                hashed_path: [1; 32],
                value: vec![0, 255],
            },
            ConcreteStorageInventoryEntry {
                hashed_path: [2; 32],
                value: vec![],
            },
        ];
        let digest = native_inverse::digest_entries(&entries);
        (
            json!({"schema":1,"status":"raw_capture_complete","digest_version":DIGEST_VERSION,"period":P,"state_root_hex":ROOT,"account_hex":ACCOUNT,"storage_root_hex":STORAGE_ROOT,"nodes_visited":2,"row_count":2,"value_bytes":2,"entries_sha256":digest,
            "entries":entries.iter().map(|e|json!({"hashed_path_hex":hex::encode(e.hashed_path),"value_hex":hex::encode(&e.value)})).collect::<Vec<_>>(),
            "qualification":{"historical_key_completeness":false,"logical_catalog_completeness":false,"semantic_snapshot_complete":false,"historical_owner_ready":false,"adoption_authorized":false,"production_routing_authorized":false}}),
            digest,
        )
    }
    fn valid(value: &Value, digest: &str) -> Result<Vec<ConcreteStorageInventoryEntry>> {
        verify(
            &serde_json::to_vec(value)?,
            Expected {
                rows: 2,
                bytes: 2,
                digest,
            },
        )
    }
    #[test]
    fn raw_roundtrip_and_structural_faults_fail_closed() {
        let (v, d) = fixture();
        let entries = valid(&v, &d).unwrap();
        assert_eq!(entries[0].value, vec![0, 255]);
        assert!(entries[1].value.is_empty());
        for key in [
            "schema",
            "status",
            "period",
            "state_root_hex",
            "account_hex",
            "storage_root_hex",
            "nodes_visited",
            "row_count",
            "value_bytes",
            "entries_sha256",
            "digest_version",
        ] {
            let mut x = v.clone();
            x[key] = json!("changed");
            assert!(valid(&x, &d).is_err());
        }
        let mut x = v.clone();
        x["entries"].as_array_mut().unwrap().swap(0, 1);
        assert!(valid(&x, &d).is_err());
        let mut x = v.clone();
        x["entries"][1] = x["entries"][0].clone();
        assert!(valid(&x, &d).is_err());
        for value in ["0", "ZZ", "FF", "0x00", "01"] {
            let mut x = v.clone();
            x["entries"][0]["hashed_path_hex"] = json!(value);
            assert!(valid(&x, &d).is_err());
        }
        for bad_path in ["F".repeat(64), "z".repeat(64)] {
            let mut x = v.clone();
            x["entries"][0]["hashed_path_hex"] = json!(bad_path);
            assert!(valid(&x, &d).is_err());
        }
        for bad_value in ["0", "FF", "0x00", "zz"] {
            let mut x = v.clone();
            x["entries"][0]["value_hex"] = json!(bad_value);
            assert!(valid(&x, &d).is_err());
        }
        let mut x = v.clone();
        x["entries"][0]["value_hex"] = json!("00fe");
        assert!(valid(&x, &d).is_err());
        let mut x = v.clone();
        x["entries"].as_array_mut().unwrap().pop();
        assert!(valid(&x, &d).is_err());
        for (key, value) in [
            ("nodes_visited", LIMITS.max_nodes + 1),
            ("row_count", 1),
            ("value_bytes", 3),
        ] {
            let mut x = v.clone();
            x[key] = json!(value);
            assert!(valid(&x, &d).is_err());
        }
        let bytes = serde_json::to_vec(&v).unwrap();
        assert!(
            verify(
                &bytes[..bytes.len() - 1],
                Expected {
                    rows: 2,
                    bytes: 2,
                    digest: &d
                }
            )
            .is_err()
        );
        assert!(verify(b"bad", expected()).is_err());
        assert!(verify(&bytes, expected()).is_err());
        let mut x = v.clone();
        x["qualification"]["semantic_snapshot_complete"] = json!(true);
        assert!(valid(&x, &d).is_err());
        assert!(interval_history::encode_bounded(&v, bytes.len() - 1).is_err());
    }
    #[test]
    fn exclusive_output_and_failed_write_cannot_succeed() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("raw-inventory-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("artifact.json");
        save(&path, b"complete").unwrap();
        assert!(save(&path, b"overwrite").is_err());
        assert_eq!(artifact_bytes(&path).unwrap(), b"complete");
        fs::remove_file(&path).unwrap();
        let oversized = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        oversized.set_len(OUTPUT_CAP as u64 + 1).unwrap();
        assert!(artifact_bytes(&path).is_err());
        drop(oversized);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&dir).unwrap();
        struct Failing;
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("failed write"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(write_all(Failing, b"raw bytes").is_err());
    }
}
