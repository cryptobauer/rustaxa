//! Offline P validator/owner seed links and undelegation-key membership audit.
//! Four immutable accepted artifacts are pinned before parsing; no DB opener exists.
use anyhow::{Context, Result, ensure};
use rustaxa_snapshot_qualifier::{interval_history, native_inverse as inverse};
use rustaxa_types::concrete_state::ConcreteStorageKey;
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
const P: u64 = 25_706_948;
const ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
const STORAGE_ROOT: &str = "c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504";
const ACCOUNT: &str = "00000000000000000000000000000000000000fe";
const RAW_DIGEST: &str = "a1b2bc6fe3c31e8fde3e597338e2c63a536a4c7badb4de3bcf44a304bc65765c";
const AVAIL_SHA: &str = "b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1";
const OUTPUT_CAP: usize = 16 * 1024 * 1024;
const TOTAL_INPUT_CAP: usize = 8 * 1024 * 1024;
const SOURCES: [(&str, &str, usize, usize); 4] = [
    (
        "/home/fry/artifacts/light-snapshot-check-20261010/availability.json",
        AVAIL_SHA,
        1088773,
        2 * 1024 * 1024,
    ),
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/parent-raw-inventory.json",
        "63dc30da528d294418f237e24e580d78d17653de643847140dedfac4faf51056",
        2894865,
        4 * 1024 * 1024,
    ),
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/undelegation-counts.json",
        "aef2814768a14ac0639d217cea255770f7f2273e5b538708df1a85ac16aa25b0",
        25397,
        1024 * 1024,
    ),
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/old-seed-undelegation-counts.json",
        "be7c0c76ca7f36fbe5c2ef5c8b22ef2009f717c6f2b068ddee9628c50dce525c",
        31079,
        1024 * 1024,
    ),
];
type Raw = BTreeMap<[u8; 32], (usize, Vec<u8>)>;
type Seeds = BTreeMap<[u8; 20], Vec<usize>>;
type Prior = BTreeMap<[u8; 32], Vec<Value>>;
fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    let mut v = [0; 32];
    let mut h = Keccak::v256();
    h.update(bytes);
    h.finalize(&mut v);
    v
}
fn canonical(value: &Value, width: Option<usize>) -> Result<Vec<u8>> {
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
fn address(value: &Value) -> Result<[u8; 20]> {
    Ok(canonical(value, Some(20))?
        .try_into()
        .expect("checked width"))
}
fn outside_db(path: &Path) -> Result<()> {
    let repo = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))?;
    ensure!(
        !path.starts_with(repo.join("data"))
            && !path.starts_with(repo.join("local/evm-state-db/snapshot-litenode-copy")),
        "path overlaps database tree"
    );
    Ok(())
}
/// Exact canonical path, regular-file metadata, growth cap and SHA precede parsing.
fn evidence(
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
fn read_capped(reader: impl Read, cap: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(cap as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= cap, "growing/oversized input");
    Ok(bytes)
}
fn pair(value: &Value) -> Result<()> {
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
fn identity(value: &Value) -> Result<()> {
    ensure!(
        value["period"] == P && value["state_root_hex"] == ROOT && value["account_hex"] == ACCOUNT,
        "P account identity drift"
    );
    pair(&value["pair"])
}
/// Recompute all ordered raw bytes and digest. Tests supply explicit small raw pins.
fn raw_index(value: &Value, rows: usize, bytes: u64, digest: &str) -> Result<Raw> {
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
fn coverage<'a>(value: &'a Value, raw: &Value) -> Result<&'a Value> {
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
fn link(
    raw: &Raw,
    links: &mut Vec<Value>,
    family: &str,
    key: ConcreteStorageKey,
    expected: &[u8],
) -> Result<usize> {
    let path = hash(&key.0);
    let (index, value) = raw.get(&path).context("required seed link not live")?;
    ensure!(value == expected, "seed link value drift: {family}");
    ensure!(links.len() < 589, "seed link cap");
    let id = links.len();
    links.push(json!({"family":family,"logical_key_hex":hex::encode(key.0),"trie_path_hex":hex::encode(path),"raw_hex":hex::encode(value),"raw_sha256":sha(value),"raw_row_index":index}));
    Ok(id)
}
/// Authenticate count/forward/reverse/owner links; preserve each occurrence before union.
fn seed_catalog(coverage: &Value, raw: &Raw) -> Result<(Seeds, Vec<Value>, Vec<Value>)> {
    ensure!(coverage["validator_count"] == 196, "validator count drift");
    let validators = coverage["validators"]
        .as_array()
        .context("validators missing")?;
    ensure!(validators.len() == 196, "validator row count");
    let mut links = Vec::new();
    let count = link(
        raw,
        &mut links,
        "global_validator_count",
        inverse::validator_seed_count_key(),
        &196u32.to_le_bytes(),
    )?;
    let mut seen = BTreeSet::new();
    let mut owners = BTreeSet::new();
    let mut seeds = Seeds::new();
    let mut provenance = Vec::new();
    for (i, row) in validators.iter().enumerate() {
        let position = (i + 1) as u32;
        ensure!(
            row["position"] == position,
            "validator position order drift"
        );
        let validator = address(&row["address_hex"])?;
        let owner = address(&row["owner_hex"])?;
        ensure!(seen.insert(validator), "duplicate validator");
        owners.insert(owner);
        let forward = link(
            raw,
            &mut links,
            "validator_forward",
            inverse::validator_seed_item_key(position),
            &validator,
        )?;
        let reverse = link(
            raw,
            &mut links,
            "validator_reverse",
            inverse::validator_seed_reverse_key(validator),
            &position.to_le_bytes(),
        )?;
        let owner_link = link(
            raw,
            &mut links,
            "validator_owner",
            inverse::validator_seed_owner_key(validator),
            &owner,
        )?;
        for (role, seed, refs) in [
            ("validator", validator, vec![count, forward, reverse]),
            ("owner", owner, vec![count, forward, reverse, owner_link]),
        ] {
            let occurrence = provenance.len();
            seeds.entry(seed).or_default().push(occurrence);
            provenance.push(json!({"period":P,"account_hex":ACCOUNT,"inverse_evidence_sha256":AVAIL_SHA,"inverse_selector":"native_dependencies[P].accounts[FE].inverse.coverage.validators[position-1]","validator_position":position,"validator_address_hex":hex::encode(validator),"role":role,"address_hex":hex::encode(seed),"link_indexes":refs}));
        }
    }
    ensure!(
        owners.len() == 102 && seeds.len() == 290 && provenance.len() == 392 && links.len() == 589,
        "seed/provenance bounds drift"
    );
    let old = &coverage["seeded_delegations"];
    ensure!(
        old["candidate_count"] == 290
            && old["count_results"]["live_present"] == 103
            && old["count_results"]["history_unavailable"] == 187
            && old["delegations"]
                .as_array()
                .is_some_and(|v| v.len() == 108),
        "prior delegation cross-check drift"
    );
    let candidates = old["candidates"]
        .as_array()
        .context("candidate facts missing")?;
    ensure!(candidates.len() == 290, "candidate count drift");
    let mut candidate_seen = BTreeSet::new();
    let mut outcomes = BTreeMap::<String, usize>::new();
    for candidate in candidates {
        let a = address(&candidate["address_hex"])?;
        ensure!(candidate_seen.insert(a), "duplicate candidate");
        let refs = seeds.get(&a).context("candidate seed not in union")?;
        let roles: BTreeSet<_> = refs
            .iter()
            .map(|i| provenance[*i]["role"].as_str().expect("constructed role"))
            .collect();
        let values = candidate["sources"]
            .as_array()
            .context("candidate roles missing")?;
        let supplied: BTreeSet<_> = values
            .iter()
            .map(|v| v.as_str().context("role missing"))
            .collect::<Result<_>>()?;
        ensure!(
            supplied == roles && supplied.len() == values.len(),
            "candidate role drift"
        );
        let outcome = candidate["delegation_count_result"]
            .as_str()
            .context("delegation result missing")?;
        ensure!(
            matches!(outcome, "live_present" | "history_unavailable"),
            "delegation result class drift"
        );
        *outcomes.entry(outcome.to_owned()).or_default() += 1;
    }
    ensure!(
        outcomes.get("live_present") == Some(&103)
            && outcomes.get("history_unavailable") == Some(&187),
        "delegation outcome counts drift"
    );
    Ok((seeds, provenance, links))
}
fn slots(seed: [u8; 20]) -> [(&'static str, ConcreteStorageKey); 3] {
    [
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
/// Only actual observed tuples can be subtracted; reconstructed older read plans are excluded.
fn prior_report(
    value: &Value,
    expected_calls: usize,
    status: &str,
    source: usize,
    pair_value: &Value,
) -> Result<Prior> {
    identity(value)?;
    ensure!(
        value["pair"] == *pair_value
            && value["inherited_storage_root_hex"] == STORAGE_ROOT
            && value["result"]["status"] == status
            && value["result"]["point_calls"] == expected_calls
            && value["result"]["error"].is_null()
            && value["result"]["accepted_present_bytes"] == 0
            && value["result"]["rejected_present_bytes"] == 0,
        "prior report status/identity drift"
    );
    let rows = value["result"]["observations"]
        .as_array()
        .context("actual observations missing")?;
    ensure!(
        rows.len() == expected_calls,
        "prior called/observed count drift"
    );
    let mut result = Prior::new();
    for (index, row) in rows.iter().enumerate() {
        let seed = address(&row["address_hex"])?;
        let role = row["slot_role"].as_str().context("slot role missing")?;
        let key = slots(seed)
            .into_iter()
            .find(|(r, _)| *r == role)
            .context("unknown slot role")?
            .1;
        ensure!(
            row["logical_key_hex"] == hex::encode(key.0)
                && row["trie_path_hex"] == hex::encode(hash(&key.0)),
            "prior key/path/role drift"
        );
        ensure!(
            row["physical_result"] == "history_unavailable"
                && row["decoded_count"].is_null()
                && row["decoded_last_id"].is_null()
                && row["raw_hex"].is_null()
                && row["returned_bytes"].is_null()
                && row["returned_sha256"].is_null(),
            "prior raw/result/number drift"
        );
        ensure!(!result.contains_key(&key.0), "duplicate prior tuple");
        result.insert(key.0,vec![json!({"source_index":source,"observation_index":index,"physical_result":"history_unavailable"})]);
    }
    Ok(result)
}
/// Named deterministic catalog framing: fixed-width fields and u32be lengths/indexes.
fn catalog_digest(seeds: &Seeds, provenance: &[Value]) -> Result<String> {
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
fn plan_digest(rows: &[Value]) -> Result<String> {
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
struct Plan {
    candidates: Vec<Value>,
    residual: Vec<Value>,
    live: usize,
    excluded: usize,
    excluded_by_source: [usize; 4],
}
/// Live membership is distinct from physical outcomes; only prior exact keys exclude a tuple.
fn build_plan(seeds: &Seeds, raw: &Raw, prior: &Prior) -> Result<Plan> {
    let mut candidates = Vec::new();
    let mut residual = Vec::new();
    let mut keys = BTreeSet::new();
    let mut live = 0;
    let mut excluded = 0;
    let mut excluded_by_source = [0usize; 4];
    for seed in seeds.keys() {
        for (role, key) in slots(*seed) {
            ensure!(
                keys.insert(key.0) && candidates.len() < 870,
                "candidate tuple duplicate/cap"
            );
            let path = hash(&key.0);
            let refs = prior.get(&key.0).cloned().unwrap_or_default();
            let mut row = json!({"period":P,"account_hex":ACCOUNT,"address_hex":hex::encode(seed),"slot_role":role,"logical_key_hex":hex::encode(key.0),"trie_path_hex":hex::encode(path),"prior_observations":refs,"decoded_count":null,"decoded_last_id":null});
            if let Some((index, value)) = raw.get(&path) {
                live += 1;
                row["raw_membership"] = json!("present_in_authenticated_live_inventory");
                row["raw_hex"] = json!(hex::encode(value));
                row["raw_sha256"] = json!(sha(value));
                row["raw_row_index"] = json!(index);
            } else {
                row["raw_membership"] = json!("not_in_live_inventory");
            }
            if refs.is_empty() {
                residual.push(row.clone());
            } else {
                excluded += 1;
                for reference in &refs {
                    excluded_by_source
                        [reference["source_index"].as_u64().context("source index")? as usize] += 1;
                }
            }
            candidates.push(row);
        }
    }
    Ok(Plan {
        candidates,
        residual,
        live,
        excluded,
        excluded_by_source,
    })
}
fn audit(values: &[Value; 4]) -> Result<Value> {
    let raw = raw_index(&values[1], 23278, 259077, RAW_DIGEST)?;
    let coverage = coverage(&values[0], &values[1])?;
    let (seeds, provenance, links) = seed_catalog(coverage, &raw)?;
    let mut prior = prior_report(
        &values[2],
        9,
        "nine_physical_outcomes_observed",
        2,
        &values[0]["pair"],
    )?;
    for (key, refs) in prior_report(
        &values[3],
        60,
        "sixty_physical_outcomes_observed",
        3,
        &values[0]["pair"],
    )? {
        prior.entry(key).or_default().extend(refs);
    }
    ensure!(prior.len() == 69, "prior unique tuple drift");
    let plan = build_plan(&seeds, &raw, &prior)?;
    let candidates = plan.candidates;
    let residual = plan.residual;
    let live = plan.live;
    let excluded = plan.excluded;
    let excluded_by_source = plan.excluded_by_source;
    ensure!(
        candidates.len() == 870
            && excluded == 60
            && residual.len() == 810
            && excluded_by_source[2] == 3
            && excluded_by_source[3] == 57,
        "exact prior subtraction drift"
    );
    Ok(
        json!({"schema":1,"status":"offline_seed_links_and_plan_verified","period":P,"state_root_hex":ROOT,"account_hex":ACCOUNT,"storage_root_hex":STORAGE_ROOT,"raw_entries_sha256":RAW_DIGEST,
 "source_manifest":SOURCES.iter().enumerate().map(|(i,(path,sha,bytes,cap))|json!({"index":i,"path":path,"sha256":sha,"bytes":bytes,"read_cap":cap})).collect::<Vec<_>>(),"verified_links":links,"seed_provenance":provenance,"seeds":seeds.iter().map(|(a,refs)|json!({"address_hex":hex::encode(a),"provenance_indexes":refs})).collect::<Vec<_>>(),"candidate_tuples":candidates,"residual_tuples":residual,
 "digests":{"catalog_version":"seed-provenance-catalog-v1","catalog_sha256":catalog_digest(&seeds,&provenance)?,"residual_version":"residual-ordered-tuples-v1","residual_sha256":plan_digest(&residual)?},
 "counts":{"verified_links":589,"seed_occurrences":392,"unique_seeds":290,"candidate_tuples":870,"prior_actual_tuples":69,"excluded_tuples":excluded,"residual_tuples":810,"live_candidate_paths":live,"not_live_candidate_paths":870-live},
 "bounds":{"total_input_bytes":TOTAL_INPUT_CAP,"verified_links":589,"seed_occurrences":392,"unique_seeds":290,"candidate_tuples":870,"serialized_output_bytes":OUTPUT_CAP},
 "runtime":{"database_opens":0,"storage_points":0,"new_inventories":0},"source_pins":{"adapter_sha256":sha(include_bytes!("validator_seed_audit.rs")),"inverse_sha256":sha(include_bytes!("../native_inverse.rs"))},
 "qualification":{"logical_catalog_complete":false,"global_delegator_complete":false,"semantic_snapshot_complete":false,"historical_owner_ready":false,"complete_owner_database_preservation":false,"reward_transition_executed":false,"adoption_authorized":false,"production_routing_authorized":false},"reference_policy_assumption":"owner-supplied default mainnet, likely v1.14.1; actual producer unverified"}),
    )
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let output = PathBuf::from(args.next().context("usage: validator_seed_audit OUTPUT")?);
    ensure!(
        args.next().is_none() && !output.as_os_str().to_string_lossy().starts_with("--"),
        "unexpected argument"
    );
    let parent = fs::canonicalize(output.parent().context("output parent")?)?;
    outside_db(&parent)?;
    let output = parent.join(output.file_name().context("output filename")?);
    ensure!(!output.exists(), "output exists");
    let mut bytes = Vec::new();
    let mut total = 0usize;
    for (path, pin, size, cap) in SOURCES {
        let data = evidence(Path::new(path), Path::new(path), pin, size, cap)?;
        total = total
            .checked_add(data.len())
            .context("input total overflow")?;
        ensure!(total <= TOTAL_INPUT_CAP, "total input cap");
        bytes.push(data);
    } // All four pins complete before any JSON parse.
    let values: Vec<Value> = bytes
        .iter()
        .map(|b| serde_json::from_slice(b))
        .collect::<std::result::Result<_, _>>()?;
    let values: [Value; 4] = values
        .try_into()
        .map_err(|_| anyhow::anyhow!("fixed source count"))?;
    let report = audit(&values)?;
    let bytes = interval_history::encode_bounded(&report, OUTPUT_CAP)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    println!(
        "verified offline seed catalog; report_sha256={}",
        sha(&bytes)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(n: u32) -> [u8; 20] {
        let mut a = [0; 20];
        a[16..].copy_from_slice(&n.to_be_bytes());
        a
    }
    fn fixture_pair() -> Value {
        json!({"head":P+1,"prior_period":P,"prior_state_root_hex":ROOT,"state_root_hex":"b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227","genesis_hash_hex":"8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377","current_and_prior_roots_authenticated":true,"descriptor_matches_header":true})
    }
    fn literal_key(parts: &[&[u8]]) -> [u8; 32] {
        hash(&parts.concat())
    }
    fn fixture() -> (Value, Raw) {
        let mut data = BTreeMap::new();
        data.insert(
            hash(&literal_key(&[&[0, 5, 1]])),
            196u32.to_le_bytes().to_vec(),
        );
        let owners: Vec<_> = (1..=8).chain(1000..1094).map(a).collect();
        let mut validators = Vec::new();
        let mut roles = BTreeMap::<[u8; 20], BTreeSet<&str>>::new();
        for i in 0..196 {
            let position = i + 1;
            let validator = a(position);
            let owner = owners[i as usize % 102];
            roles.entry(validator).or_default().insert("validator");
            roles.entry(owner).or_default().insert("owner");
            data.insert(
                hash(&literal_key(&[&[0, 5, 2], &position.to_le_bytes()])),
                validator.to_vec(),
            );
            data.insert(
                hash(&literal_key(&[&[0, 5, 2], &validator])),
                position.to_le_bytes().to_vec(),
            );
            data.insert(hash(&literal_key(&[&[0, 3], &validator])), owner.to_vec());
            validators.push(json!({"position":position,"address_hex":hex::encode(validator),"owner_hex":hex::encode(owner)}));
        }
        let candidates:Vec<_>=roles.iter().enumerate().map(|(i,(seed,roles))|json!({"address_hex":hex::encode(seed),"sources":roles,"delegation_count_result":if i<103{"live_present"}else{"history_unavailable"}})).collect();
        let coverage = json!({"validator_count":196,"validators":validators,"seeded_delegations":{"candidate_count":290,"count_results":{"live_present":103,"history_unavailable":187},"candidates":candidates,"delegations":vec![Value::Null;108]}});
        (
            coverage,
            data.into_iter()
                .enumerate()
                .map(|(i, (path, value))| (path, (i, value)))
                .collect(),
        )
    }
    fn prior_fixture(addresses: &[[u8; 20]], status: &str) -> Value {
        let observations:Vec<_>=addresses.iter().flat_map(|seed|{[("v1_validator_count",vec![3,1],vec![1]),("v2_validator_count",vec![3,2],vec![1]),("v2_last_allocated_id",vec![3,4],vec![])].into_iter().map(move |(role,prefix,suffix)|{let key=literal_key(&[&prefix,seed,&suffix]);json!({"address_hex":hex::encode(seed),"slot_role":role,"logical_key_hex":hex::encode(key),"trie_path_hex":hex::encode(hash(&key)),"physical_result":"history_unavailable","decoded_count":null,"decoded_last_id":null})})}).collect();
        json!({"period":P,"state_root_hex":ROOT,"account_hex":ACCOUNT,"pair":fixture_pair(),"inherited_storage_root_hex":STORAGE_ROOT,"result":{"status":status,"point_calls":observations.len(),"error":null,"accepted_present_bytes":0,"rejected_present_bytes":0,"observations":observations}})
    }
    #[test]
    fn authenticates_exact_links_preserves_shared_roles_and_rejects_faults() {
        let (value, raw) = fixture();
        let (seeds, provenance, links) = seed_catalog(&value, &raw).unwrap();
        assert_eq!(
            (seeds.len(), provenance.len(), links.len()),
            (290, 392, 589)
        );
        assert!(seeds[&a(1)].len() > 1);
        let digest = catalog_digest(&seeds, &provenance).unwrap();
        assert_eq!(digest, catalog_digest(&seeds, &provenance).unwrap());
        let mut changed = provenance.clone();
        changed.swap(0, 1);
        assert_ne!(digest, catalog_digest(&seeds, &changed).unwrap());
        changed.pop();
        assert_ne!(digest, catalog_digest(&seeds, &changed).unwrap());
        for key in [
            literal_key(&[&[0, 5, 1]]),
            literal_key(&[&[0, 5, 2], &1u32.to_le_bytes()]),
            literal_key(&[&[0, 5, 2], &a(1)]),
            literal_key(&[&[0, 3], &a(1)]),
        ] {
            let mut broken = raw.clone();
            broken.remove(&hash(&key));
            assert!(seed_catalog(&value, &broken).is_err());
            let mut broken = raw.clone();
            broken.get_mut(&hash(&key)).unwrap().1 = vec![0];
            assert!(seed_catalog(&value, &broken).is_err());
        }
        let mut x = value.clone();
        x["validators"][0]["position"] = json!(2);
        assert!(seed_catalog(&x, &raw).is_err());
        let mut x = value.clone();
        x["validators"][1]["address_hex"] = x["validators"][0]["address_hex"].clone();
        assert!(seed_catalog(&x, &raw).is_err());
        for field in ["address_hex", "owner_hex"] {
            let mut x = value.clone();
            x["validators"][0][field] = json!("01");
            assert!(seed_catalog(&x, &raw).is_err());
        }
        let mut x = value.clone();
        x["seeded_delegations"]["candidates"][0]["sources"] = json!(["owner"]);
        assert!(seed_catalog(&x, &raw).is_err());
        let mut x = value.clone();
        x["validators"].as_array_mut().unwrap().pop();
        assert!(seed_catalog(&x, &raw).is_err());
        let mut bad = raw.clone();
        let k = literal_key(&[&[0, 5, 2], &1u32.to_le_bytes()]);
        let entry = bad.remove(&hash(&k)).unwrap();
        bad.insert(
            hash(&literal_key(&[&[0, 5, 0], &1u32.to_le_bytes()])),
            entry,
        );
        assert!(seed_catalog(&value, &bad).is_err());
    }
    #[test]
    fn raw_rows_and_identity_selectors_fail_closed() {
        let value = json!({"schema":1,"status":"raw_capture_complete","digest_version":"sha256-ordered-path32-length-u64be-value-v1","period":P,"state_root_hex":ROOT,"account_hex":ACCOUNT,"pair":fixture_pair(),"storage_root_hex":STORAGE_ROOT,"nodes_visited":2,"row_count":2,"value_bytes":2,"entries":[{"hashed_path_hex":hex::encode([1;32]),"value_hex":"00ff"},{"hashed_path_hex":hex::encode([2;32]),"value_hex":""}]});
        let mut value = value;
        let entries = vec![
            rustaxa_storage::ConcreteStorageInventoryEntry {
                hashed_path: [1; 32],
                value: vec![0, 255],
            },
            rustaxa_storage::ConcreteStorageInventoryEntry {
                hashed_path: [2; 32],
                value: vec![],
            },
        ];
        let digest = inverse::digest_entries(&entries);
        value["entries_sha256"] = json!(digest);
        assert_eq!(raw_index(&value, 2, 2, &digest).unwrap().len(), 2);
        for field in [
            "period",
            "state_root_hex",
            "account_hex",
            "storage_root_hex",
            "row_count",
            "value_bytes",
            "entries_sha256",
            "digest_version",
        ] {
            let mut x = value.clone();
            x[field] = json!("changed");
            assert!(raw_index(&x, 2, 2, &digest).is_err());
        }
        let mut x = value.clone();
        x["entries"].as_array_mut().unwrap().swap(0, 1);
        assert!(raw_index(&x, 2, 2, &digest).is_err());
        let mut x = value.clone();
        x["entries"][1] = x["entries"][0].clone();
        assert!(raw_index(&x, 2, 2, &digest).is_err());
        for bad in ["F".repeat(64), "z".repeat(64), "01".to_owned()] {
            let mut x = value.clone();
            x["entries"][0]["hashed_path_hex"] = json!(bad);
            assert!(raw_index(&x, 2, 2, &digest).is_err());
        }
        for bad in ["0", "FF", "0x00", "zz", "00fe"] {
            let mut x = value.clone();
            x["entries"][0]["value_hex"] = json!(bad);
            assert!(raw_index(&x, 2, 2, &digest).is_err());
        }
        let dep = json!({"period":P,"state_root_hex":ROOT,"root_authenticated":true,"header_sha256":"1e9c7f4e77cbbabad4ec2f8b065332aa998c881dff503c2fcbcfb0d21005adac","accounts":[{"address_hex":ACCOUNT,"status":"complete_live_inventory","storage_root_hex":STORAGE_ROOT,"live_entries":23278,"value_bytes":259077,"entries_sha256":RAW_DIGEST,"inverse":{"status":"partial_inverse_completed","coverage":{}}}]});
        let a = json!({"pair":fixture_pair(),"native_dependencies":[dep]});
        assert!(coverage(&a, &value).is_ok());
        let mut x = a.clone();
        let duplicate = x["native_dependencies"][0].clone();
        x["native_dependencies"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(coverage(&x, &value).is_err());
        let mut x = a.clone();
        x["native_dependencies"][0]["period"] = json!(P + 1);
        assert!(coverage(&x, &value).is_err());
        let mut x = a.clone();
        x["native_dependencies"][0]["accounts"][0]["address_hex"] =
            json!("00000000000000000000000000000000000000ee");
        assert!(coverage(&x, &value).is_err());
    }
    #[test]
    fn exact_tuple_subtraction_and_membership_never_infer_physical_absence() {
        let (value, raw) = fixture();
        let (seeds, _, _) = seed_catalog(&value, &raw).unwrap();
        let sorted: Vec<_> = seeds.keys().copied().collect();
        let first = prior_fixture(
            &[sorted[0], a(10000), a(10001)],
            "nine_physical_outcomes_observed",
        );
        let mut old = sorted[1..20].to_vec();
        old.push(a(10002));
        let second = prior_fixture(&old, "sixty_physical_outcomes_observed");
        let mut prior = prior_report(
            &first,
            9,
            "nine_physical_outcomes_observed",
            2,
            &fixture_pair(),
        )
        .unwrap();
        prior.extend(
            prior_report(
                &second,
                60,
                "sixty_physical_outcomes_observed",
                3,
                &fixture_pair(),
            )
            .unwrap(),
        );
        let plan = build_plan(&seeds, &raw, &prior).unwrap();
        assert_eq!(
            (
                plan.candidates.len(),
                plan.residual.len(),
                plan.excluded,
                plan.excluded_by_source[2],
                plan.excluded_by_source[3]
            ),
            (870, 810, 60, 3, 57)
        );
        for row in &plan.candidates {
            assert_eq!(row["raw_membership"], "not_in_live_inventory");
            assert!(row.get("physical_result").is_none());
            assert!(row["decoded_count"].is_null());
            assert!(row["decoded_last_id"].is_null());
        }
        for (seed, row_group) in sorted.iter().zip(plan.candidates.chunks_exact(3)) {
            for (i, row) in row_group.iter().enumerate() {
                let (prefix, suffix) = match i {
                    0 => (vec![3, 1], vec![1]),
                    1 => (vec![3, 2], vec![1]),
                    _ => (vec![3, 4], vec![]),
                };
                let key = literal_key(&[&prefix, seed, &suffix]);
                assert_eq!(row["logical_key_hex"], hex::encode(key));
                assert_eq!(row["trie_path_hex"], hex::encode(hash(&key)));
            }
        }
        let mut live = raw.clone();
        let key = literal_key(&[&[3, 4], &sorted[0]]);
        live.insert(hash(&key), (999, vec![0xff; 8]));
        let observed = build_plan(&seeds, &live, &prior).unwrap();
        assert_eq!(observed.live, 1);
        assert_eq!(observed.candidates[2]["raw_hex"], "ffffffffffffffff");
        assert!(observed.candidates[2]["decoded_last_id"].is_null());
        let digest = plan_digest(&plan.residual).unwrap();
        assert_eq!(
            digest,
            plan_digest(&build_plan(&seeds, &raw, &prior).unwrap().residual).unwrap()
        );
        let mut swapped = plan.residual.clone();
        swapped.swap(0, 1);
        assert_ne!(digest, plan_digest(&swapped).unwrap());
        for field in ["period", "state_root_hex", "account_hex"] {
            let mut x = first.clone();
            x[field] = json!("changed");
            assert!(
                prior_report(&x, 9, "nine_physical_outcomes_observed", 2, &fixture_pair()).is_err()
            );
        }
        for field in [
            "logical_key_hex",
            "trie_path_hex",
            "slot_role",
            "physical_result",
            "raw_hex",
            "decoded_count",
        ] {
            let mut x = first.clone();
            x["result"]["observations"][0][field] = json!("changed");
            assert!(
                prior_report(&x, 9, "nine_physical_outcomes_observed", 2, &fixture_pair()).is_err()
            );
        }
        let mut x = first.clone();
        x["result"]["point_calls"] = json!(8);
        assert!(
            prior_report(&x, 9, "nine_physical_outcomes_observed", 2, &fixture_pair()).is_err()
        );
        let mut x = first.clone();
        x["result"]["observations"][1] = x["result"]["observations"][0].clone();
        assert!(
            prior_report(&x, 9, "nine_physical_outcomes_observed", 2, &fixture_pair()).is_err()
        );
        let mut x = first.clone();
        x["result"]["observations"].as_array_mut().unwrap().pop();
        assert!(
            prior_report(&x, 9, "nine_physical_outcomes_observed", 2, &fixture_pair()).is_err()
        );
    }
    #[test]
    fn bounded_source_reads_output_and_db_exclusions() {
        assert!(read_capped(std::io::Cursor::new(b"grow"), 3).is_err());
        assert_eq!(
            read_capped(std::io::Cursor::new(b"raw"), 3).unwrap(),
            b"raw"
        );
        let repo =
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap();
        assert!(outside_db(&repo.join("data/file.json")).is_err());
        assert!(
            outside_db(&repo.join("local/evm-state-db/snapshot-litenode-copy/file.json")).is_err()
        );
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("validator-seed-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("source");
        fs::write(&path, b"raw").unwrap();
        assert!(evidence(&path, &path, &sha(b"raw"), 3, 3).is_ok());
        assert!(evidence(&path, &path, &sha(b"bad"), 3, 3).is_err());
        assert!(evidence(&path, &path, &sha(b"raw"), 3, 2).is_err());
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&dir).unwrap();
        let value = json!("output");
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(interval_history::encode_bounded(&value, bytes.len() - 1).is_err());
    }
}
