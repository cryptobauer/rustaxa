//! Offline admission and join of the fixed N4 parent-input evidence.
//! Native proof and candidate-policy qualification is per family. This module
//! does not open a database, construct a reward cache or grant owner authority.
use crate::{
    artifact_admission as admission, aspen_policy, interval_history, native_witness,
    parent_reward_fields as fields,
};
use anyhow::{Context, Result, ensure};
use rustaxa_storage::{
    ConcreteDiagnosticPlan, ConcreteWitnessLimits, ConcreteWitnessResource as Resource,
    ConcreteWitnessUsage, replay_diagnostic_bundle,
};
use rustaxa_types::concrete_state::ConcreteStateIdentity;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Read, path::Path};

pub const OUTPUT_CAP: usize = 262_144;
const ARTIFACT_CAP: usize = 16 * 1024 * 1024;
const FIRST: u64 = 25_706_901;
const INPUTS: [(&str, usize, &str); 6] = [
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/interval-history-authentication.json",
        2766744,
        "3513c1720759ed3dd07784ab66d69661742af82ceeee56d0c527c835da594aa5",
    ),
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2/parent-cache-fields-cap-tests.json",
        2257600,
        "b54906443d6504bd1ad59d2208791d4cd710ffe2ae4a0731f8306af7b5521a3b",
    ),
    (
        "/home/fry/artifacts/n4-run-01a12b7a-2f06-7661-9183-a37257d00590-20261011/delayed-header-capture.json",
        256840,
        "7d57845a66978c6107b2d3f733f7e9e743cde0761926d07060517bc11e7c867f",
    ),
    (
        "/home/fry/artifacts/n4-run-01a12b7a-2f06-7661-9183-a37257d00590-20261011/aspen-candidate-policy.json",
        49682,
        "f172a66100d41d26b3d0f860d73892fc8c7f52bb4392ba77c1f91ac292d0fcc8",
    ),
    (
        "/home/fry/artifacts/n4-binding-resume-20261011/native-live-bundle-frame-cost.rcb1",
        1026265,
        "a0a36875f90defe9dad0d7450416c75f57c06e9eee16cc53f27c666f95fcc761",
    ),
    (
        "/home/fry/artifacts/n4-binding-resume-20261011/native-live-replay-frame-cost.json",
        10911,
        "2ccf155e2a851cd52161f090bcd3cc76b09f212a44647ba8f76b7db0b7511fbb",
    ),
];
const RESOURCES: [Resource; 13] = [
    Resource::KeyBytes,
    Resource::RowBytes,
    Resource::RawCopyBytes,
    Resource::CanonicalBytes,
    Resource::EncodedBytes,
    Resource::Operations,
    Resource::Visits,
    Resource::Records,
    Resource::FrameInspections,
    Resource::ConstructorIdentities,
    Resource::Registrations,
    Resource::DistinctIdentities,
    Resource::PayloadTuples,
];

/// Monotonic artifact admission accounting; charges precede file allocation.
#[derive(Default)]
struct AdmissionBudget {
    reads: usize,
    bytes: usize,
    distinct: BTreeSet<usize>,
}
impl AdmissionBudget {
    fn charge(&mut self, index: usize, bytes: usize) -> Result<()> {
        ensure!(index < INPUTS.len(), "unknown artifact");
        let reads = self.reads.checked_add(1).context("read overflow")?;
        let total = self.bytes.checked_add(bytes).context("byte overflow")?;
        ensure!(
            reads <= 11 && total <= ARTIFACT_CAP,
            "artifact admission cap"
        );
        self.distinct.insert(index);
        self.reads = reads;
        self.bytes = total;
        Ok(())
    }
}
fn canonical_regular(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "absolute fixed path required");
    let mut component = std::path::PathBuf::new();
    for part in path.components() {
        component.push(part);
        ensure!(
            !fs::symlink_metadata(&component)?.file_type().is_symlink(),
            "symlink rejected"
        );
    }
    ensure!(
        fs::canonicalize(path)? == path && fs::metadata(path)?.is_file(),
        "canonical regular file required"
    );
    Ok(())
}
fn read_fixed(path: &Path, bytes: usize, digest: &str) -> Result<Vec<u8>> {
    canonical_regular(path)?;
    admission::evidence(path, path, digest, bytes, bytes)
}
fn admit() -> Result<(Vec<Vec<u8>>, AdmissionBudget)> {
    let mut budget = AdmissionBudget::default();
    let mut admitted = Vec::with_capacity(6);
    for (index, (path, bytes, digest)) in INPUTS.iter().enumerate() {
        budget.charge(index, *bytes)?;
        admitted.push(read_fixed(Path::new(path), *bytes, digest)?);
    }
    ensure!(
        budget.reads == 6 && budget.distinct.len() == 6 && budget.bytes == 6_368_042,
        "admission composition drift"
    );
    Ok((admitted, budget))
}
fn source_profile() -> Result<Value> {
    macro_rules! pin { ($path:literal) => {{ let bytes=include_bytes!($path);
        ensure!(bytes.len() <= 1_048_576, "source file cap");
        json!({"path":$path,"bytes":bytes.len(),"sha256":admission::sha(bytes)}) }}; }
    let sources = vec![
        pin!("parent_input_handoff.rs"),
        pin!("bin/parent_input_handoff.rs"),
        pin!("lib.rs"),
        pin!("native_witness.rs"),
        pin!("delayed_headers.rs"),
        pin!("interval_history.rs"),
        pin!("head_history.rs"),
        pin!("parent_history.rs"),
        pin!("parent_reward_fields.rs"),
        pin!("reward_votes.rs"),
        pin!("paths.rs"),
        pin!("bin/parent_cache_fields/admission.rs"),
        pin!("aspen_policy.rs"),
        pin!("aspen_policy_sources.rs"),
        pin!("../Cargo.toml"),
        pin!("../Cargo.lock"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/witness.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_binary.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/physical_node.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/codec.rs"),
        pin!(
            "../../../../rust/crates/rustaxa-consensus/src/final_chain/native_session/vote_inputs.rs"
        ),
        pin!("../../../../rust/crates/rustaxa-consensus/src/pbft_vote_validation.rs"),
        pin!("../../../../rust/crates/rustaxa-types/src/codec/rlp/final_chain.rs"),
    ];
    source_bounds(&sources)?;
    Ok(json!(sources))
}
fn source_bounds(sources: &[Value]) -> Result<()> {
    ensure!(sources.len() == 25, "source profile count");
    let mut total = 0_u64;
    for source in sources {
        let bytes = source["bytes"].as_u64().context("source bytes")?;
        ensure!(bytes <= 1_048_576, "source file cap");
        total = total.checked_add(bytes).context("source total overflow")?;
        ensure!(total <= 4_194_304, "source aggregate cap");
    }
    Ok(())
}

fn admit_aspen_sources() -> Result<Value> {
    let sources = aspen_policy::source_manifest()?;
    let repo = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))?;
    let mut total = 0usize;
    for source in sources.as_array().context("Aspen sources")? {
        let bytes = usize::try_from(source["bytes"].as_u64().context("source bytes")?)?;
        total = total.checked_add(bytes).context("source byte overflow")?;
        ensure!(bytes <= 1_048_576 && total <= 465_085, "Aspen source cap");
        // Only the compiled fixed manifest supplies paths; JSON artifacts do not.
        read_fixed(
            &repo.join(source["path"].as_str().context("source path")?),
            bytes,
            source["sha256"].as_str().context("source digest")?,
        )?;
    }
    ensure!(
        total == 465_085 && sources.as_array().unwrap().len() == 16,
        "Aspen source count"
    );
    Ok(sources)
}
fn executable_profile() -> Result<Value> {
    let path = std::env::current_exe()?;
    let mut file = fs::File::open(&path)?;
    let mut buffer = [0u8; 8192];
    let mut bytes = 0usize;
    let mut hash = Sha256::new();
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        bytes = bytes.checked_add(n).context("executable bytes overflow")?;
        ensure!(bytes <= 268_435_456, "executable cap");
        hash.update(&buffer[..n]);
    }
    Ok(json!({"path":path,"bytes":bytes,"sha256":hex::encode(hash.finalize())}))
}
fn identity(identity: ConcreteStateIdentity) -> Value {
    json!({"period":identity.period.as_u64(),"state_root_hex":hex::encode(identity.state_root)})
}
fn usage(actual: ConcreteWitnessUsage) -> Value {
    json!({"admitted_input_bytes":actual.admitted_input_bytes,"resources":RESOURCES.iter()
        .map(|r|json!({"resource":format!("{r:?}"),"used":actual.used(*r)})).collect::<Vec<_>>()})
}
fn storage_limits() -> Value {
    let limits = ConcreteWitnessLimits::default();
    json!(
        RESOURCES
            .iter()
            .map(|r| json!({"resource":format!("{r:?}"),"limit":limits.limit(*r)}))
            .collect::<Vec<_>>()
    )
}
fn native_comparison(
    saved: &Value,
    plan: &ConcreteDiagnosticPlan,
    totals: &[u64; 48],
    actual: ConcreteWitnessUsage,
) -> Result<()> {
    let expected_totals = json!(
        totals
            .iter()
            .enumerate()
            .map(|(i, total)| json!({"identity":identity(plan.delayed[i]),"total_votes":total}))
            .collect::<Vec<_>>()
    );
    ensure!(
        saved["schema"] == 1
            && saved["profile"] == "fixed_n4_native_total_jail"
            && saved["status"] == "all_96_authenticated_native_totals"
            && saved["totals"] == expected_totals
            && saved["actual"] == usage(actual),
        "native report differs from fresh replay"
    );
    ensure!(
        saved["bundle"] == json!({"path":INPUTS[4].0,"bytes":INPUTS[4].1,"sha256":INPUTS[4].2}),
        "bundle comparison drift"
    );
    ensure!(
        saved["bounds"]
            == json!({"authority_input_bytes":native_witness::AUTHORITY_INPUT_CAP,
        "bundle_bytes":native_witness::BUNDLE_CAP,"report_bytes":native_witness::REPORT_CAP,
        "executable_bytes":268435456,"executable_hash_buffer_bytes":8192,"storage_resources":storage_limits(),
        "exclusions":["authority admission allocations","engine memory","allocator overhead","OS I/O","RSS"]}),
        "native limits drift"
    );
    ensure!(
        saved["qualification"]
            == json!({"all_96_authenticated":true,"historical_totals_qualified":true,
        "voter_weights_qualified":false,"reward_cache_equal":false,"owner_ready":false,"producer_verified":false,
        "independent_public_finality":false,"production_routing_authorized":false}),
        "native comparison authority drift"
    );
    Ok(())
}
fn join(
    available: &Value,
    policy: &Value,
    plan: &ConcreteDiagnosticPlan,
    totals: &[u64; 48],
) -> Result<Value> {
    let periods = available["periods"].as_array().context("parent periods")?;
    let candidates = policy["periods"].as_array().context("candidate periods")?;
    let roots = available["missing_inputs"]["delayed_roots_and_totals"]
        .as_array()
        .context("root manifest")?;
    ensure!(
        periods.len() == 48 && candidates.len() == 48 && roots.len() == 48,
        "complete join count"
    );
    let mut joined = Vec::with_capacity(48);
    for (i, period) in periods.iter().enumerate() {
        let source = period["period"].as_u64().context("source period")?;
        let delayed = source.checked_sub(7).context("delayed period underflow")?;
        ensure!(
            source == FIRST + u64::try_from(i)?
                && delayed == plan.delayed[i].period.as_u64()
                && period["certificate"]["delayed_period"] == delayed
                && roots[i]["delayed_period"] == delayed,
            "source/delayed period binding"
        );
        if !roots[i]["root_source"].is_null() {
            ensure!(
                roots[i]["root_source"]["period"] == delayed
                    && roots[i]["root_source"]["state_root_hex"]
                        == hex::encode(plan.delayed[i].state_root),
                "delayed root binding"
            );
        }
        let candidate = &candidates[i];
        ensure!(
            candidate["period"] == source
                && candidate["header_hash_hex"] == period["header_hash_hex"]
                && candidate["pbft_signed_hash_hex"] == period["pbft_signed_hash_hex"],
            "candidate identity binding"
        );
        ensure!(totals[i] != 0, "qualified total must be nonzero");
        joined.push(json!({"period":source,"header_hash_hex":period["header_hash_hex"],
            "pbft_signed_hash_hex":period["pbft_signed_hash_hex"],"delayed_identity":identity(plan.delayed[i]),
            "total_votes":totals[i],"jail_precondition":"authenticated_empty_or_nonmember","candidate_aspen":candidate}));
    }
    Ok(json!(joined))
}
fn voter_manifest(available: &Value) -> Result<Value> {
    let manifest = &available["missing_inputs"]["voter_keys"];
    let rows = manifest.as_array().context("voter manifest")?;
    let mut pairs = BTreeSet::new();
    let mut voters = BTreeSet::new();
    for row in rows {
        let d = row["delayed_period"].as_u64().context("voter D")?;
        let voter = row["voter_hex"].as_str().context("voter")?;
        pairs.insert((d, voter));
        voters.insert(voter);
        ensure!(
            row["qualified"] == false && row["value"].is_null() && row["proof"].is_null(),
            "voter authority drift"
        );
    }
    ensure!(
        rows.len() == 2742 && pairs.len() == 914 && voters.len() == 20,
        "voter manifest count"
    );
    Ok(
        json!({"input_index":1,"manifest_sha256":fields::normalized_digest(manifest)?,"rows":rows.len(),"pairs":pairs.len(),"voters":voters.len(),"qualified":false}),
    )
}
fn qualification() -> Value {
    json!({"delayed_roots_authenticated":true,"native_totals_qualified":true,"native_jail_preconditions_qualified":true,
        "candidate_aspen_derived":true,"producer_policy_verified":false,"voter_weights_qualified":false,
        "lambda_qualified":false,"typed_cache_equal":false,"raw_cache_equal":false,"model_complete":false,
        "owner_authority":false,"independent_public_finality":false,"reward_transition_executed":false,
        "production_routing_authorized":false,"adoption_authorized":false})
}
fn content_digest(report: &Value) -> Result<String> {
    let mut content = report.clone();
    let object = content.as_object_mut().context("handoff object")?;
    object.remove("executable");
    object.remove("digests");
    fields::normalized_digest(&json!({"domain":"n4/parent-input-handoff/v1","report":content}))
}
/// Admit six fixed artifacts, authenticate all roots and proofs, and join every
/// source row. Failure returns no report. Authority/parser allocations are
/// separate from the unchanged storage witness budget; no RSS bound is claimed.
fn derive(
    admitted: &[Vec<u8>],
    budget: &AdmissionBudget,
    sources: Value,
    aspen_sources: Value,
    executable: Value,
) -> Result<Value> {
    ensure!(admitted.len() == 6, "fixed artifact count");
    for (bytes, (_, n, digest)) in admitted.iter().zip(INPUTS) {
        ensure!(
            bytes.len() == n && admission::sha(bytes) == digest,
            "admitted pin drift"
        );
    }
    let history = admission::parse_unique(&admitted[0])?;
    ensure!(
        history["schema"] == 1
            && history["authentication"]["status"] == "complete_interval_lists_authenticated",
        "interval report identity"
    );
    let rows = serde_json::from_value(history["authentication"]["rows"].clone())?;
    let parent = admission::parse_unique(&admitted[1])?;
    let plan = native_witness::plan_from_admitted(&admitted[0], &admitted[1], &admitted[2])?;
    let available = fields::derive_available_fields(&rows)?;
    ensure!(
        parent["periods"] == available["periods"],
        "parent comparison drift"
    );
    let missing = available["missing_inputs"]
        .as_object()
        .context("regenerated missing manifest")?;
    for (key, value) in missing {
        ensure!(
            parent["missing_inputs"][key] == *value,
            "parent missing family comparison drift"
        );
    }
    // The old report also carries expected-cache size/hash metadata. Its
    // unchanged validator checks that extra family; it supplies no raw oracle.
    ensure!(
        parent["missing_inputs"]
            .as_object()
            .context("saved missing manifest")?
            .len()
            == missing.len() + 1,
        "unused parent missing family"
    );
    let candidate = aspen_policy::derive(&rows, &parent)?;
    let policy = admission::parse_unique(&admitted[3])?;
    candidate.validate_report(&policy)?;
    let verified = replay_diagnostic_bundle(&admitted[4], &plan, ConcreteWitnessLimits::default())
        .map_err(|failure| anyhow::anyhow!("native replay: {:?}", failure))?;
    let totals = native_witness::decode_totals(&verified.logical)?;
    native_comparison(
        &admission::parse_unique(&admitted[5])?,
        &plan,
        &totals,
        verified.usage,
    )?;
    let periods = join(&available, &policy, &plan, &totals)?;
    let manifest = voter_manifest(&available)?;
    let mut report = json!({"schema":1,"kind":"n4_parent_input_handoff","status":"offline_parent_inputs_authenticated",
        "inputs":INPUTS.iter().map(|(p,n,h)|json!({"path":p,"bytes":n,"sha256":h})).collect::<Vec<_>>(),
        "sources":sources,"aspen_sources":aspen_sources,"reference_policy":policy["reference_policy"],"executable":executable,
        "bounds":{"distinct_artifacts":6,"artifact_reads":11,"artifact_bytes":ARTIFACT_CAP,"output_bytes":OUTPUT_CAP,
            "source_files":25,"source_file_bytes":1048576,"source_bytes":4194304,"aspen_source_files":16,"aspen_source_bytes":465085,
            "executable_bytes":268435456,"executable_hash_buffer_bytes":8192,"storage_resources":storage_limits(),
            "exclusions":["parser and authority allocations","engine memory","allocator overhead","OS I/O","RSS"]},
        "actual":{"distinct_artifacts":budget.distinct.len(),"artifact_reads":budget.reads,"artifact_bytes":budget.bytes,
            "aspen_source_reads":16,"aspen_source_bytes":465085,"source_periods":48,"native":usage(verified.usage),
            "database_opens":0,"storage_points":0,"cursors":0,"new_inventories":0},
        "qualification":qualification(),"voter_manifest":manifest,"periods":periods});
    report["digests"] = json!({"content_sha256":content_digest(&report)?});
    interval_history::encode_bounded(&report, OUTPUT_CAP)?;
    Ok(report)
}
/// Execute the fixed offline profile and publish one complete, capped report.
/// The output must be new and outside DB trees. No input override is accepted.
pub fn run(output: &Path) -> Result<()> {
    ensure!(output.is_absolute(), "absolute output required");
    let output = aspen_policy::output_path(output)?;
    let sources = source_profile()?;
    let aspen_sources = admit_aspen_sources()?;
    let (admitted, budget) = admit()?;
    let report = derive(
        &admitted,
        &budget,
        sources,
        aspen_sources,
        executable_profile()?,
    )?;
    admission::publish_exclusive(
        &output,
        &interval_history::encode_bounded(&report, OUTPUT_CAP)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;
    struct Fixture {
        admitted: Vec<Vec<u8>>,
        budget: AdmissionBudget,
        available: Value,
        policy: Value,
        plan: ConcreteDiagnosticPlan,
        totals: [u64; 48],
        usage: ConcreteWitnessUsage,
    }
    static FIXTURE: LazyLock<Fixture> = LazyLock::new(|| {
        let (admitted, budget) = admit().unwrap();
        let history = admission::parse_unique(&admitted[0]).unwrap();
        let rows = serde_json::from_value(history["authentication"]["rows"].clone()).unwrap();
        let plan =
            native_witness::plan_from_admitted(&admitted[0], &admitted[1], &admitted[2]).unwrap();
        let available = fields::derive_available_fields(&rows).unwrap();
        let policy = admission::parse_unique(&admitted[3]).unwrap();
        let verified =
            replay_diagnostic_bundle(&admitted[4], &plan, ConcreteWitnessLimits::default())
                .unwrap();
        let totals = native_witness::decode_totals(&verified.logical).unwrap();
        Fixture {
            admitted,
            budget,
            available,
            policy,
            plan,
            totals,
            usage: verified.usage,
        }
    });
    #[test]
    fn complete_fixed_handoff_authenticates_all_roots_and_preserves_false_authority() {
        let f = &*FIXTURE;
        let report = derive(
            &f.admitted,
            &f.budget,
            source_profile().unwrap(),
            admit_aspen_sources().unwrap(),
            json!({"test":true}),
        )
        .unwrap();
        assert_eq!(report["periods"].as_array().unwrap().len(), 48);
        assert_eq!(
            report["periods"][0]["delayed_identity"]["period"],
            25_706_894
        );
        assert_eq!(
            report["periods"][47]["delayed_identity"]["period"],
            25_706_941
        );
        assert_eq!(report["actual"]["artifact_reads"], 6);
        assert_eq!(report["actual"]["artifact_bytes"], 6_368_042);
        assert_eq!(report["qualification"], qualification());
        for key in [
            "producer_policy_verified",
            "voter_weights_qualified",
            "lambda_qualified",
            "typed_cache_equal",
            "raw_cache_equal",
            "model_complete",
            "owner_authority",
            "independent_public_finality",
            "reward_transition_executed",
            "production_routing_authorized",
            "adoption_authorized",
        ] {
            assert_eq!(
                report["qualification"][key], false,
                "forbidden authority {key}"
            );
        }
        for key in [
            "delayed_roots_authenticated",
            "native_totals_qualified",
            "native_jail_preconditions_qualified",
            "candidate_aspen_derived",
        ] {
            assert_eq!(
                report["qualification"][key], true,
                "accepted input family {key}"
            );
        }
        assert_eq!(
            report["reference_policy"],
            json!({"id":"aspen-period-min-raw-u16-v1",
            "basis":"owner-supplied default mainnet, likely v1.14.1","chain_id":841,
            "aspen_part_one_period":8_118_000,"producer_verified":false})
        );
        assert_eq!(report["voter_manifest"]["rows"], 2742);
        assert_eq!(report["voter_manifest"]["pairs"], 914);
        assert_eq!(report["voter_manifest"]["voters"], 20);
        assert_eq!(
            report["digests"]["content_sha256"],
            content_digest(&report).unwrap()
        );
        let mut tampered = report.clone();
        tampered["qualification"]["owner_authority"] = json!(true);
        assert_ne!(
            content_digest(&tampered).unwrap(),
            content_digest(&report).unwrap()
        );
        assert!(interval_history::encode_bounded(&report, OUTPUT_CAP).is_ok());
        assert!(interval_history::encode_bounded(&report, 1).is_err());
    }
    #[test]
    fn join_rejects_wrong_period_root_order_missing_duplicate_and_unused_bindings() {
        let f = &*FIXTURE;
        assert!(join(&f.available, &f.policy, &f.plan, &f.totals).is_ok());
        for kind in 0..7 {
            let mut available = f.available.clone();
            match kind {
                0 => available["periods"][0]["certificate"]["delayed_period"] = json!(25_706_893),
                1 => {
                    available["missing_inputs"]["delayed_roots_and_totals"][7]["root_source"]["state_root_hex"] =
                        json!("00".repeat(32))
                }
                2 => available["periods"].as_array_mut().unwrap().swap(0, 1),
                3 => {
                    available["periods"].as_array_mut().unwrap().pop();
                }
                4 => available["periods"][1] = available["periods"][0].clone(),
                5 => {
                    let extra = available["periods"][0].clone();
                    available["periods"].as_array_mut().unwrap().push(extra);
                }
                _ => available["periods"][0]["period"] = json!(6),
            }
            assert!(
                join(&available, &f.policy, &f.plan, &f.totals).is_err(),
                "mutation {kind}"
            );
        }
        let mut policy = f.policy.clone();
        policy["periods"][0]["header_hash_hex"] = json!("11".repeat(32));
        assert!(join(&f.available, &policy, &f.plan, &f.totals).is_err());
    }
    #[test]
    fn claimed_reports_cannot_replace_replay_or_candidate_derivation() {
        let f = &*FIXTURE;
        let saved = admission::parse_unique(&f.admitted[5]).unwrap();
        native_comparison(&saved, &f.plan, &f.totals, f.usage).unwrap();
        for field in ["totals", "qualification", "actual", "bounds"] {
            let mut tampered = saved.clone();
            tampered[field] = json!({"claimed_success":true});
            assert!(native_comparison(&tampered, &f.plan, &f.totals, f.usage).is_err());
        }
        let history = admission::parse_unique(&f.admitted[0]).unwrap();
        let rows = serde_json::from_value(history["authentication"]["rows"].clone()).unwrap();
        let parent = admission::parse_unique(&f.admitted[1]).unwrap();
        let owner = aspen_policy::derive(&rows, &parent).unwrap();
        let mut tampered = f.policy.clone();
        tampered["qualification"]["producer_policy_verified"] = json!(true);
        assert!(owner.validate_report(&tampered).is_err());
        tampered = f.policy.clone();
        tampered["periods"][0]["total_dag_blocks_count"] = json!(999);
        assert!(owner.validate_report(&tampered).is_err());
    }
    #[test]
    fn witness_tamper_and_trailing_unconsumed_records_fail_without_prefix() {
        let f = &*FIXTURE;
        let mut bytes = f.admitted[4].clone();
        bytes[100] ^= 1;
        assert!(
            replay_diagnostic_bundle(&bytes, &f.plan, ConcreteWitnessLimits::default()).is_err()
        );
        bytes = f.admitted[4].clone();
        bytes.push(0);
        assert!(
            replay_diagnostic_bundle(&bytes, &f.plan, ConcreteWitnessLimits::default()).is_err()
        );
        let mut delayed = f.admitted[2].clone();
        delayed[100] ^= 1;
        assert!(
            native_witness::plan_from_admitted(&f.admitted[0], &f.admitted[1], &delayed).is_err()
        );
    }
    #[test]
    fn admission_caps_repeated_reads_and_pin_errors_stop_before_allocation() {
        let mut budget = AdmissionBudget::default();
        for _ in 0..11 {
            budget.charge(0, 1).unwrap();
        }
        assert!(budget.charge(0, 1).is_err());
        assert_eq!(budget.reads, 11);
        let mut budget = AdmissionBudget::default();
        budget.charge(0, ARTIFACT_CAP).unwrap();
        assert!(budget.charge(1, 1).is_err());
        assert_eq!(budget.bytes, ARTIFACT_CAP);
        assert!(budget.charge(6, 0).is_err());
        assert!(native_witness::plan_from_admitted(&[], &[], &[]).is_err());
        assert!(admission::parse_unique(br#"{"success":true,"success":false}"#).is_err());
    }
    #[test]
    fn fixed_source_profile_count_file_and_aggregate_caps_fail_closed() {
        let sources = source_profile().unwrap();
        let mut rows = sources.as_array().unwrap().clone();
        rows[0]["bytes"] = json!(1_048_577);
        assert!(source_bounds(&rows).is_err());
        rows = sources.as_array().unwrap().clone();
        for row in &mut rows {
            row["bytes"] = json!(200_000);
        }
        assert!(source_bounds(&rows).is_err());
        rows = sources.as_array().unwrap().clone();
        rows.pop();
        assert!(source_bounds(&rows).is_err());
    }
}
