//! Candidate Aspen DAG counters from regenerated signed parent fields.
//! This distinct report supplies no native facts, full reward cache or owner.
#[path = "aspen_policy_sources.rs"]
mod sources;
use crate::{
    artifact_admission as admission, interval_history::IntervalHistoryRows,
    parent_reward_fields as fields,
};
use admission::{parse_unique, sha};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub const OUTPUT_CAP: usize = 262_144;
const FIRST: u64 = 25_706_901;
const LAST: u64 = 25_706_948;
const ASPEN: u64 = 8_118_000;
const AVAILABLE_SHA: &str = "91f5d4f11dc37d576dc452561ca8ba5a342c5490b47ffb9cb4770ebf407b1a90";
const INPUTS: [(&str, usize, &str); 2] = [
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2/parent-cache-fields-cap-tests.json",
        2_257_600,
        "b54906443d6504bd1ad59d2208791d4cd710ffe2ae4a0731f8306af7b5521a3b",
    ),
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/interval-history-authentication.json",
        2_766_744,
        "3513c1720759ed3dd07784ab66d69661742af82ceeee56d0c527c835da594aa5",
    ),
];

/// Sealed derived rows. A caller cannot construct it from claimed counters.
pub struct AuthenticatedPolicy {
    periods: Vec<Value>,
    available_sha: String,
}
#[derive(Clone)]
struct DagInput {
    position: u32,
    signed_hash: [u8; 32],
    author: [u8; 20],
    difficulty: u16,
}
fn hex_array<const N: usize>(value: &Value) -> Result<[u8; N]> {
    let raw = value.as_str().context("hex string")?;
    ensure!(
        raw.len() == 2 * N
            && raw
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "fixed lowercase hex"
    );
    hex::decode(raw)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("hex width"))
}
fn exact_keys(value: &Value, keys: &[&str]) -> Result<()> {
    let object = value.as_object().context("object")?;
    ensure!(
        object.len() == keys.len() && keys.iter().all(|k| object.contains_key(*k)),
        "fixed schema keys"
    );
    Ok(())
}
fn projection(period: u64, header: [u8; 32], pbft: [u8; 32], dags: &[DagInput]) -> Result<Value> {
    ensure!(period >= ASPEN && dags.len() <= 10, "Aspen period/DAG cap");
    let minimum = dags.iter().map(|d| d.difficulty).min();
    let mut seen = BTreeSet::new();
    let mut positions = Vec::new();
    let mut authors = BTreeMap::<[u8; 20], u32>::new();
    let mut blocks = Vec::new();
    for (position, block) in dags.iter().enumerate() {
        ensure!(
            usize::try_from(block.position)? == position && seen.insert(block.signed_hash),
            "DAG position/duplicate signed hash"
        );
        blocks.push(
            json!({"position":block.position,"signed_hash_hex":hex::encode(block.signed_hash),
            "author_hex":hex::encode(block.author),"raw_difficulty":block.difficulty}),
        );
        if Some(block.difficulty) == minimum {
            positions.push(block.position);
            let count = authors.entry(block.author).or_default();
            *count = count.checked_add(1).context("author count overflow")?;
        }
    }
    let total = u32::try_from(positions.len())?;
    let sum = authors.values().try_fold(0_u32, |sum, n| {
        sum.checked_add(*n).context("author sum overflow")
    })?;
    ensure!(sum == total, "author count sum");
    Ok(
        json!({"period":period,"header_hash_hex":hex::encode(header),"pbft_signed_hash_hex":hex::encode(pbft),
        "minimum_raw_difficulty":minimum,"dag_blocks":blocks,"qualifying_positions":positions,
        "total_dag_blocks_count":total,"per_author_dag_counts":authors.into_iter().map(|(author,count)|
            json!({"author_hex":hex::encode(author),"dag_blocks_count":count})).collect::<Vec<_>>()}),
    )
}

/// Reauthenticates the complete fixed interval and derives candidate counters.
/// Saved fields only serve comparison; altered authors, order, signatures or
/// authority flags fail. No expected counter, minimum or cache value enters.
pub fn derive(rows: &IntervalHistoryRows, saved: &Value) -> Result<AuthenticatedPolicy> {
    source_manifest()?; // compiled semantic sources must match the reviewed pins
    fields::validate_field_report(saved)?;
    let available = fields::derive_available_fields(rows)?;
    let digest = fields::normalized_digest(&available["periods"])?;
    ensure!(
        digest == AVAILABLE_SHA
            && saved["digests"]["available_fields_digest"] == digest
            && saved["periods"] == available["periods"],
        "saved fields differ from regenerated raw authority"
    );
    let mut periods = Vec::new();
    let mut dags_total = 0_usize;
    for (index, period) in available["periods"]
        .as_array()
        .context("regenerated periods")?
        .iter()
        .enumerate()
    {
        let number = period["period"].as_u64().context("period")?;
        ensure!(number == FIRST + u64::try_from(index)?, "period order");
        let dags = period["dags"]
            .as_array()
            .context("DAG facts")?
            .iter()
            .map(|d| {
                Ok(DagInput {
                    position: u32::try_from(d["position"].as_u64().context("position")?)?,
                    signed_hash: hex_array(&d["signed_hash_hex"])?,
                    author: hex_array(&d["author_hex"])?,
                    difficulty: u16::try_from(
                        d["raw_difficulty"].as_u64().context("raw u16 difficulty")?,
                    )?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        dags_total = dags_total
            .checked_add(dags.len())
            .context("DAG sum overflow")?;
        ensure!(dags_total <= 139, "DAG aggregate cap");
        periods.push(projection(
            number,
            hex_array(&period["header_hash_hex"])?,
            hex_array(&period["pbft_signed_hash_hex"])?,
            &dags,
        )?);
    }
    ensure!(
        periods.len() == 48
            && dags_total == 139
            && periods.last().context("last period")?["period"] == LAST,
        "fixed complete policy coverage"
    );
    Ok(AuthenticatedPolicy {
        periods,
        available_sha: digest,
    })
}
fn input_manifest() -> Value {
    json!(
        INPUTS
            .iter()
            .map(|(p, n, h)| json!({"path":p,"bytes":n,"sha256":h}))
            .collect::<Vec<_>>()
    )
}
/// Verifies the 16 compiled semantic/configuration sources and their exact
/// bounded manifest. Configuration remains an owner reference assumption.
pub fn source_manifest() -> Result<Value> {
    let mut total = 0usize;
    let mut manifest = Vec::new();
    for (path, n, digest, bytes) in sources::SOURCES {
        total = total.checked_add(n).context("source total overflow")?;
        ensure!(
            n <= 1024 * 1024 && bytes.len() == n && sha(bytes) == digest && total <= 465_085,
            "compiled semantic source pin drift"
        );
        manifest.push(json!({"path":path,"bytes":n,"sha256":digest}));
    }
    ensure!(
        manifest.len() == 16 && total == 465_085,
        "fixed semantic manifest"
    );
    manifest.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let config = sources::SOURCES
        .iter()
        .find(|(p, _, _, _)| p.ends_with("mainnet_genesis.json"))
        .context("config source")?
        .3;
    ensure!(
        parse_unique(config)?["hardforks"]["aspen_hf"]["block_num_part_one"] == ASPEN,
        "candidate Aspen boundary drift"
    );
    Ok(json!(manifest))
}
fn implementation_fields(executable_sha: String) -> Value {
    json!({"core_sha256":sha(include_bytes!("aspen_policy.rs")),
        "adapter_sha256":sha(include_bytes!("bin/aspen_candidate_policy.rs")),
        "admission_sha256":sha(include_bytes!("bin/parent_cache_fields/admission.rs")),
        "executable_sha256":executable_sha})
}
impl AuthenticatedPolicy {
    /// Builds a separate fixed schema report. Every non-candidate authority
    /// stays false; whole output is encoded within its cap before publication.
    pub fn report(&self, implementation: Value) -> Result<Value> {
        exact_keys(
            &implementation,
            &[
                "core_sha256",
                "adapter_sha256",
                "admission_sha256",
                "executable_sha256",
            ],
        )?;
        for value in implementation.as_object().unwrap().values() {
            hex_array::<32>(value)?;
        }
        ensure!(
            implementation
                == implementation_fields(
                    implementation["executable_sha256"].as_str().unwrap().into()
                ),
            "implementation pins differ"
        );
        let mut qualifying = 0_u64;
        let mut author_rows = 0_usize;
        for period in &self.periods {
            qualifying = qualifying
                .checked_add(
                    period["total_dag_blocks_count"]
                        .as_u64()
                        .context("counter")?,
                )
                .context("qualifying sum overflow")?;
            author_rows = author_rows
                .checked_add(
                    period["per_author_dag_counts"]
                        .as_array()
                        .context("author counts")?
                        .len(),
                )
                .context("author rows overflow")?;
        }
        ensure!(
            qualifying <= 139 && author_rows <= 139,
            "derived aggregate cap"
        );
        let inputs = input_manifest();
        let sources = source_manifest()?;
        let policy = json!({"id":"aspen-period-min-raw-u16-v1","basis":"owner-supplied default mainnet, likely v1.14.1",
            "chain_id":841,"aspen_part_one_period":ASPEN,"producer_verified":false});
        let input_digest = fields::normalized_digest(
            &json!({"domain":"n4/aspen-candidate-policy/inputs/v1",
            "inputs":inputs,"sources":sources,"reference_policy":policy,"regenerated_available_fields_sha256":self.available_sha}),
        )?;
        let rows_digest =
            fields::normalized_digest(&json!({"domain":"n4/aspen-candidate-policy/rows/v1",
            "policy_inputs_sha256":input_digest,"periods":self.periods}))?;
        let mut value = json!({"schema":1,"kind":"n4_parent_aspen_candidate_policy","status":"candidate_policy_derived",
            "base_commit":"eb13f969acb49c593692ad651a070abb20b62711","inputs":inputs,"sources":sources,"implementation":implementation,
            "reference_policy":policy,"bounds":{"input_bytes":5024344,"source_files":16,"source_file_bytes":1048576,
                "source_total_bytes":465085,"output_bytes":OUTPUT_CAP,"periods":48,"dags_per_period":10,"dag_blocks":139,"author_rows":139},
            "actual":{"periods":48,"dag_blocks":139,"qualifying_dag_blocks":qualifying,"author_rows":author_rows},
            "runtime":{"database_opens":0,"storage_points":0,"proof_calls":0,"new_inventories":0},
            "qualification":{"regenerated_local_chain_authenticated":true,"signed_dag_fields_authenticated":true,
                "candidate_policy_source_pinned":true,"candidate_policy_derived":true,"producer_policy_verified":false,
                "independent_public_finality":false,"vdf_vrf_validity_qualified":false,"stake_eligibility_qualified":false,
                "complete_rewards_qualified":false,"vote_weights_qualified":false,"vote_totals_qualified":false,
                "rate_inputs_qualified":false,"cache_generated":false,"typed_cache_equal":false,"raw_cache_equal":false,
                "owner_authority":false,"historical_key_completeness":false,"reward_transition_executed":false,
                "production_routing_authorized":false,"adoption_authorized":false},"periods":self.periods,
            "digests":{"regenerated_available_fields_sha256":self.available_sha,"policy_inputs_sha256":input_digest,"policy_rows_sha256":rows_digest}});
        value["digests"]["report_content_sha256"] = json!(fields::normalized_digest(
            &json!({"domain":"n4/aspen-candidate-policy/report/v1","report":value})
        )?);
        crate::interval_history::encode_bounded(&value, OUTPUT_CAP)?;
        Ok(value)
    }
    /// Exact replay comparison from this raw-authenticated owner; extra keys,
    /// substituted policy/flags, tampered counters and recomputed digests fail.
    pub fn validate_report(&self, value: &Value) -> Result<()> {
        ensure!(
            &self.report(value["implementation"].clone())? == value,
            "policy report differs from authenticated recomputation"
        );
        Ok(())
    }
}
fn no_symlinks(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for part in absolute.components() {
        current.push(part);
        ensure!(
            !fs::symlink_metadata(&current)?.file_type().is_symlink(),
            "symlink component rejected"
        );
    }
    Ok(())
}
fn read_fixed(path: &Path, n: usize, digest: &str) -> Result<Vec<u8>> {
    no_symlinks(path)?;
    ensure!(
        fs::canonicalize(path)? == path,
        "canonical fixed source path"
    );
    admission::evidence(path, path, digest, n, n)
}
/// Admits exactly two artifacts and 16 source files before deriving any policy.
/// Embedded paths never become open targets; there are no DB or probe modes.
pub fn load_fixed() -> Result<AuthenticatedPolicy> {
    let repo = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))?;
    let sources = source_manifest()?;
    for source in sources.as_array().unwrap() {
        read_fixed(
            &repo.join(source["path"].as_str().unwrap()),
            usize::try_from(source["bytes"].as_u64().unwrap())?,
            source["sha256"].as_str().unwrap(),
        )?;
    }
    let saved = read_fixed(Path::new(INPUTS[0].0), INPUTS[0].1, INPUTS[0].2)?;
    let history = read_fixed(Path::new(INPUTS[1].0), INPUTS[1].1, INPUTS[1].2)?;
    ensure!(
        saved.len().checked_add(history.len()) == Some(5_024_344),
        "aggregate input cap"
    );
    let saved = parse_unique(&saved)?;
    let history = parse_unique(&history)?;
    ensure!(
        history["schema"] == 1
            && history["authentication"]["status"] == "complete_interval_lists_authenticated",
        "history report identity"
    );
    let rows = serde_json::from_value(history["authentication"]["rows"].clone())?;
    derive(&rows, &saved)
}
/// Current implementation pins, including the bounded executable file hash.
pub fn implementation() -> Result<Value> {
    let bytes =
        admission::read_capped(fs::File::open(std::env::current_exe()?)?, 256 * 1024 * 1024)?;
    Ok(implementation_fields(sha(&bytes)))
}
/// Guards a new output outside both DB trees before any publication. Existing
/// files or aliases fail; no overwrite or unbounded partial report is exposed.
pub fn output_path(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    no_symlinks(parent)?;
    let parent = fs::canonicalize(parent)?;
    admission::outside_db(&parent)?;
    let output = parent.join(path.file_name().context("output filename")?);
    match fs::symlink_metadata(&output) {
        Ok(_) => anyhow::bail!("output exists"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(output)
}
/// Validate and fully encode before synced exclusive publication and cleanup.
pub fn publish(output: &Path, policy: &AuthenticatedPolicy, report: &Value) -> Result<()> {
    let output = output_path(output)?;
    policy.validate_report(report)?;
    admission::publish_exclusive(
        &output,
        &crate::interval_history::encode_bounded(report, OUTPUT_CAP)?,
    )
}

#[cfg(test)]
#[path = "aspen_policy_tests.rs"]
mod tests;
