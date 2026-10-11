//! Fixed saved-evidence admission for the offline family; no database reader.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, env, fs, io::Read, path::Path};
use tiny_keccak::{Hasher, Keccak};
pub(super) const P: u64 = 25_706_948;
pub(super) const ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
pub(super) const ACCOUNT: &str = "00000000000000000000000000000000000000fe";
pub(super) const STORAGE_ROOT: &str =
    "c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504";
pub(super) const RAW_DIGEST: &str =
    "a1b2bc6fe3c31e8fde3e597338e2c63a536a4c7badb4de3bcf44a304bc65765c";
const AVAIL_SHA: &str = "b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1";
pub(super) type Raw = BTreeMap<[u8; 32], (usize, Vec<u8>)>;
pub(super) type Seeds = BTreeMap<[u8; 20], Vec<usize>>;
pub(super) fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub(super) fn hash(bytes: &[u8]) -> [u8; 32] {
    let mut v = [0; 32];
    let mut h = Keccak::v256();
    h.update(bytes);
    h.finalize(&mut v);
    v
}
pub(super) fn canonical(value: &Value, width: Option<usize>) -> Result<Vec<u8>> {
    let s = value.as_str().context("hex missing")?;
    ensure!(
        s.len() % 2 == 0
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "noncanonical hex"
    );
    if let Some(n) = width {
        ensure!(s.len() == 2 * n, "hex width drift");
    }
    Ok(hex::decode(s)?)
}
pub(super) fn address(value: &Value) -> Result<[u8; 20]> {
    Ok(canonical(value, Some(20))?
        .try_into()
        .expect("checked width"))
}
pub(super) fn outside_db(path: &Path) -> Result<()> {
    let repo = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))?;
    ensure!(
        !path.starts_with(repo.join("data"))
            && !path.starts_with(repo.join("local/evm-state-db/snapshot-litenode-copy")),
        "path overlaps database tree"
    );
    Ok(())
}
pub(super) fn evidence(
    path: &Path,
    expected_path: &Path,
    pin: &str,
    size: usize,
    cap: usize,
) -> Result<Vec<u8>> {
    let path = fs::canonicalize(path)?;
    ensure!(
        path == fs::canonicalize(expected_path)?,
        "source path differs"
    );
    outside_db(&path)?;
    let metadata = fs::metadata(&path)?;
    ensure!(
        metadata.is_file() && metadata.len() == size as u64 && size <= cap,
        "source size/cap drift"
    );
    let bytes = read_capped(fs::File::open(path)?, cap)?;
    ensure!(
        bytes.len() == size && sha(&bytes) == pin,
        "source byte pin drift"
    );
    Ok(bytes)
}
pub(super) fn read_capped(reader: impl Read, cap: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(cap as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= cap, "growing/oversized input");
    Ok(bytes)
}
pub(super) fn pair(value: &Value) -> Result<()> {
    ensure!(
        value["head"] == P + 1
            && value["prior_period"] == P
            && value["prior_state_root_hex"] == ROOT
            && value["state_root_hex"]
                == "b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227"
            && value["genesis_hash_hex"]
                == "8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377"
            && value["current_and_prior_roots_authenticated"] == true
            && value["descriptor_matches_header"] == true,
        "pair identity/authentication drift"
    );
    Ok(())
}
pub(super) fn identity(value: &Value) -> Result<()> {
    ensure!(
        value["period"] == P && value["state_root_hex"] == ROOT && value["account_hex"] == ACCOUNT,
        "P account identity drift"
    );
    pair(&value["pair"])
}
pub(super) fn raw_index(value: &Value, rows: usize, bytes: u64, digest: &str) -> Result<Raw> {
    identity(value)?;
    ensure!(
        value["schema"] == 1
            && value["status"] == "raw_capture_complete"
            && value["storage_root_hex"] == STORAGE_ROOT
            && value["digest_version"] == "sha256-ordered-path32-length-u64be-value-v1"
            && value["nodes_visited"]
                .as_u64()
                .is_some_and(|n| n > 0 && n <= 50000),
        "raw artifact metadata drift"
    );
    let entries = value["entries"].as_array().context("raw entries missing")?;
    ensure!(entries.len() == rows && rows <= 50000, "raw row cap/drift");
    let mut raw = Raw::new();
    let mut total = 0u64;
    let mut hasher = Sha256::new();
    let mut previous = None;
    for (index, row) in entries.iter().enumerate() {
        let path: [u8; 32] = canonical(&row["hashed_path_hex"], Some(32))?
            .try_into()
            .expect("checked width");
        ensure!(
            previous.is_none_or(|p| p < path),
            "raw duplicate/order drift"
        );
        previous = Some(path);
        let s = row["value_hex"].as_str().context("raw value missing")?;
        ensure!(s.len() <= 64 * 1024 * 1024, "raw row cap");
        total = total
            .checked_add((s.len() / 2) as u64)
            .context("raw byte overflow")?;
        ensure!(total <= 32 * 1024 * 1024, "raw total byte cap");
        let value = canonical(&row["value_hex"], None)?;
        hasher.update(path);
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(&value);
        raw.insert(path, (index, value));
    }
    ensure!(
        total == bytes
            && value["value_bytes"] == bytes
            && value["row_count"] == rows
            && hex::encode(hasher.finalize()) == digest
            && value["entries_sha256"] == digest,
        "raw bytes/count/digest drift"
    );
    Ok(raw)
}
pub(super) fn coverage<'a>(value: &'a Value, raw: &Value) -> Result<&'a Value> {
    pair(&value["pair"])?;
    ensure!(value["pair"] == raw["pair"], "source pair drift");
    let deps = value["native_dependencies"]
        .as_array()
        .context("dependencies missing")?;
    let selected: Vec<_> = deps.iter().filter(|d| d["period"] == P).collect();
    ensure!(selected.len() == 1, "P selector missing/duplicate");
    let dep = selected[0];
    ensure!(
        dep["state_root_hex"] == ROOT
            && dep["root_authenticated"] == true
            && dep["header_sha256"]
                == "1e9c7f4e77cbbabad4ec2f8b065332aa998c881dff503c2fcbcfb0d21005adac",
        "P root/header drift"
    );
    let accounts = dep["accounts"].as_array().context("accounts missing")?;
    let selected: Vec<_> = accounts
        .iter()
        .filter(|a| a["address_hex"] == ACCOUNT)
        .collect();
    ensure!(selected.len() == 1, "FE selector missing/duplicate");
    let account = selected[0];
    ensure!(
        account["status"] == "complete_live_inventory"
            && account["storage_root_hex"] == STORAGE_ROOT
            && account["live_entries"] == 23278
            && account["value_bytes"] == 259077
            && account["entries_sha256"] == RAW_DIGEST
            && account["inverse"]["status"] == "partial_inverse_completed",
        "P inventory/inverse drift"
    );
    Ok(&account["inverse"]["coverage"])
}
pub(super) fn catalog_digest(seeds: &Seeds, provenance: &[Value]) -> Result<String> {
    let mut h = Sha256::new();
    h.update(b"seed-provenance-catalog-v1");
    h.update(P.to_be_bytes());
    h.update(hex::decode(ACCOUNT)?);
    h.update(hex::decode(AVAIL_SHA)?);
    h.update((provenance.len() as u32).to_be_bytes());
    for row in provenance {
        h.update((row["validator_position"].as_u64().context("position")? as u32).to_be_bytes());
        h.update(address(&row["validator_address_hex"])?);
        h.update(address(&row["address_hex"])?);
        h.update([if row["role"] == "validator" { 0 } else { 1 }]);
        let selector = row["inverse_selector"]
            .as_str()
            .context("selector")?
            .as_bytes();
        h.update((selector.len() as u32).to_be_bytes());
        h.update(selector);
        let refs = row["link_indexes"].as_array().context("link refs")?;
        h.update((refs.len() as u32).to_be_bytes());
        for index in refs {
            h.update((index.as_u64().context("link index")? as u32).to_be_bytes());
        }
    }
    h.update((seeds.len() as u32).to_be_bytes());
    for (seed, refs) in seeds {
        h.update(seed);
        h.update((refs.len() as u32).to_be_bytes());
        for i in refs {
            h.update((*i as u32).to_be_bytes());
        }
    }
    Ok(hex::encode(h.finalize()))
}
pub(super) fn plan_digest(rows: &[Value]) -> Result<String> {
    let mut h = Sha256::new();
    h.update(b"residual-ordered-tuples-v1");
    h.update((rows.len() as u32).to_be_bytes());
    for row in rows {
        h.update(P.to_be_bytes());
        h.update(hex::decode(ACCOUNT)?);
        h.update(address(&row["address_hex"])?);
        h.update([match row["slot_role"].as_str().context("role")? {
            "v1_validator_count" => 0,
            "v2_validator_count" => 1,
            "v2_last_allocated_id" => 2,
            _ => anyhow::bail!("unknown role"),
        }]);
        h.update(canonical(&row["logical_key_hex"], Some(32))?);
    }
    Ok(hex::encode(h.finalize()))
}
