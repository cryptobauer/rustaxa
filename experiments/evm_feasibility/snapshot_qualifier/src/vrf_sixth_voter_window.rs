//! The fixed sixth complete remaining VRF voter window. The fixed seven-input authority
//! authenticates raw votes/roots and the prior first-period proof before opens.
//! Each capture/replay is consuming and qualifies no prefix or voter weight.
use crate::{
    artifact_admission as admission, interval_history, native_witness as native,
    parent_input_handoff, parent_reward_fields as fields, paths, vrf_witness as first,
};
use anyhow::{Context, Result, ensure};
use rustaxa_storage::{
    ConcreteVrfPlan, ConcreteVrfVoterPlan, ConcreteWitnessLimits, ConcreteWitnessLogical,
    ConcreteWitnessPhysical, capture_vrf_voter_run, replay_vrf_bundle, replay_vrf_voter_bundle,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub const PROFILE: &str = "n4_remaining_vrf_voter_a6cec53b4d7920709f05ca1f7ac0d66338d29cb9_v1";
pub const REPORT_CAP: usize = 131072;
const VOTER: &str = "a6cec53b4d7920709f05ca1f7ac0d66338d29cb9";
const FIRST_VOTER: &str = "138c1b9baea7d93a981ba694f5318f735c997298";
const FIFTH_VOTER: &str = "6bf44c438f6328640b0bc5d7d7766310cf49610f";
const FOURTH_VOTER: &str = "575910424ac437467726542781508b2220b71ce8";
const THIRD_VOTER: &str = "329d40b5f15cfe4de0e9953d0599d2a39a65fc04";
const PREVIOUS_VOTER: &str = "264a30e880d3b53bb2cdeb234e34201ebf9a9965";
const PRIOR_PATH: &str =
    "/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-first-d.rcv1";
const PRIOR_BYTES: usize = 417594;
const PRIOR_SHA: &str = "6382e8f25cc9ec2225e563111e37b9ad8c8beae4167543cb4d9e915a7d5cbb61";
const FIRST: u64 = 25706894;
const INDICES: [usize; 47] = [
    89, 149, 209, 263, 314, 371, 431, 491, 551, 608, 665, 722, 776, 836, 893, 947, 1004, 1061,
    1121, 1181, 1241, 1301, 1352, 1406, 1457, 1517, 1574, 1631, 1688, 1745, 1802, 1859, 1913, 1967,
    2027, 2087, 2141, 2201, 2255, 2315, 2372, 2429, 2486, 2546, 2600, 2654, 2714,
];

fn select(rows: &Value) -> Result<Value> {
    let rows = rows.as_array().context("complete voter rows")?;
    ensure!(rows.len() == 2742, "window original count");
    let mut periods = BTreeMap::<&str, BTreeSet<u64>>::new();
    for row in rows {
        if row["semantic_key"] == "fe_vrf_00_04_address" {
            periods
                .entry(row["voter_hex"].as_str().context("window voter")?)
                .or_default()
                .insert(row["delayed_period"].as_u64().context("window period")?);
        }
    }
    let all: BTreeSet<_> = (FIRST..FIRST + 48).collect();
    let candidates: Vec<_> = periods
        .iter()
        .filter(|(_, p)| **p == all)
        .map(|(v, _)| *v)
        .collect();
    ensure!(
        candidates.len() == 10
            && candidates.first() == Some(&FIRST_VOTER)
            && candidates.get(1) == Some(&PREVIOUS_VOTER)
            && candidates.get(2) == Some(&THIRD_VOTER)
            && candidates.get(3) == Some(&FOURTH_VOTER)
            && candidates.get(4) == Some(&FIFTH_VOTER)
            && candidates.get(5) == Some(&VOTER),
        "fixed complete voter selector"
    );
    let selected: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r["voter_hex"] == VOTER
                && r["semantic_key"] == "fe_vrf_00_04_address"
                && r["delayed_period"] != FIRST
        })
        .collect();
    ensure!(selected.len() == 47, "complete remaining voter family");
    let mut output = Vec::with_capacity(47);
    for (i, (index, row)) in selected.into_iter().enumerate() {
        let d = FIRST + 1 + i as u64;
        ensure!(
            index == INDICES[i]
                && row["delayed_period"] == d
                && row["qualified"] == false
                && row["value"].is_null()
                && row["proof"].is_null(),
            "window request/order drift"
        );
        let refs = row["source_references"]
            .as_array()
            .context("window vote references")?;
        ensure!(
            !refs.is_empty() && refs.iter().all(|r| r["period"] == d + 7),
            "window source period"
        );
        output.push(json!({"parent_manifest_index":index,"source_row":row}));
    }
    Ok(json!(output))
}
fn prior(bytes: &[u8], plan: &ConcreteVrfPlan) -> Result<Value> {
    ensure!(
        bytes.len() == PRIOR_BYTES && admission::sha(bytes) == PRIOR_SHA,
        "prior VRF pin drift"
    );
    let verified = replay_vrf_bundle(bytes, plan, ConcreteWitnessLimits::default())
        .map_err(|e| anyhow::anyhow!("prior VRF proof: {e:?}"))?;
    let index = plan
        .voters()
        .iter()
        .position(|v| hex::encode(v) == VOTER)
        .context("prior selected voter")?;
    ensure!(index == 10, "prior selected index");
    let values = first::decode_values(&verified.logical)?;
    let value = accepted_prior_value(&verified.logical[index], &verified.physical[index])?;
    Ok(
        json!({"input":{"path":PRIOR_PATH,"bytes":PRIOR_BYTES,"sha256":PRIOR_SHA},
        "actual":native::usage(verified.usage),"semantic_sha256":fields::normalized_digest(&values)?,
        "selected":{"proof_index":61,"identity":native::identity_json(plan.context().delayed[0]),
            "voter_hex":VOTER,"value_hex":hex::encode(value),"usable_vrf_key":true}}),
    )
}
fn accepted_prior_value<'a>(
    logical: &'a ConcreteWitnessLogical,
    physical: &ConcreteWitnessPhysical,
) -> Result<&'a [u8]> {
    let value = match logical {
        ConcreteWitnessLogical::Member(v) if v.len() == 32 && v.iter().any(|b| *b != 0) => v,
        _ => anyhow::bail!("required accepted firstD member unavailable"),
    };
    ensure!(
        matches!(physical,ConcreteWitnessPhysical::Present(v) if v==value),
        "prior physical member mismatch"
    );
    Ok(value)
}
fn canonical_prior() -> Result<Vec<u8>> {
    let p = Path::new(PRIOR_PATH);
    let mut current = std::path::PathBuf::new();
    for part in p.components() {
        current.push(part);
        ensure!(
            !fs::symlink_metadata(&current)?.file_type().is_symlink(),
            "prior symlink"
        );
    }
    ensure!(
        fs::canonicalize(p)? == p && fs::metadata(p)?.is_file(),
        "prior regular file"
    );
    admission::evidence(p, p, PRIOR_SHA, PRIOR_BYTES, PRIOR_BYTES)
}
fn authority() -> Result<(ConcreteVrfVoterPlan, Value, Value)> {
    let (context, rows, six) = parent_input_handoff::load_authenticated_voter_authority()?;
    let (first_plan, _) = first::selection(context, &rows)?;
    let mut selected = select(&rows)?;
    after_prior(first_plan, &canonical_prior()?, |first_plan, old_proof| {
        let voter: [u8; 20] = hex::decode(VOTER)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("fixed voter width"))?;
        let plan = ConcreteVrfVoterPlan::new(first_plan, voter)?;
        for (i, row) in selected
            .as_array_mut()
            .context("window selection")?
            .iter_mut()
            .enumerate()
        {
            let tuple = plan.tuples()[i];
            row["tuple"] = json!({"identity":native::identity_json(tuple.identity),"address_hex":hex::encode(tuple.address),
            "logical_key_hex":hex::encode(tuple.key.0),"key_preimage_hex":format!("0004{VOTER}")});
        }
        Ok((
            plan,
            selected,
            json!({"six_input_owner":six,"prior_first_d":old_proof,
        "raw_artifact_reads":7,"raw_artifact_bytes":6785636}),
        ))
    })
}
/// The continuation reaches the window/open stage only after fresh prior proof.
fn after_prior<T>(
    plan: ConcreteVrfPlan,
    bytes: &[u8],
    next: impl FnOnce(ConcreteVrfPlan, Value) -> Result<T>,
) -> Result<T> {
    let proof = prior(bytes, &plan)?;
    next(plan, proof)
}

fn base(selected: Value, authority: Value) -> Result<Value> {
    macro_rules! pin {($p:literal)=>{{let b=include_bytes!($p);
        ensure!(b.len()<=1048576,"window source file cap");
        json!({"path":$p,"bytes":b.len(),"sha256":admission::sha(b)})}};}
    let mut sources = first::sources()?
        .as_array()
        .context("shared VRF sources")?
        .clone();
    sources.extend([
        pin!("vrf_sixth_voter_window.rs"),
        pin!("bin/live_vrf_sixth_voter_window.rs"),
        pin!("bin/replay_vrf_sixth_voter_window.rs"),
    ]);
    ensure!(sources.len() == 22, "window source count");
    let bytes = authority["six_input_owner"]["sources"]
        .as_array()
        .context("owner sources")?
        .iter()
        .chain(&sources)
        .try_fold(0u64, |n, p| {
            n.checked_add(p["bytes"].as_u64().context("source bytes")?)
                .context("source total")
        })?;
    ensure!(bytes <= 4194304, "combined window source cap");
    let mut bounds = native::policy();
    bounds["report_bytes"] = json!(REPORT_CAP);
    bounds["authority_input_bytes"] = json!(6785636);
    bounds["raw_artifacts_exact"] = json!(7);
    bounds["payload_tuples_exact"] = json!(47);
    bounds["proofs_exact"] = json!(98);
    let digest = fields::normalized_digest(&selected)?;
    Ok(
        json!({"schema":1,"profile":PROFILE,"selection":selected,"selection_sha256":digest,
        "authority":authority,"vrf_sources":sources,"bounds":bounds,
        "qualification":{"vrf_family_qualified":false,"voter_weights_qualified":false,
        "reward_cache_equal":false,"owner_ready":false,"producer_verified":false,"production_routing_authorized":false}}),
    )
}
fn publish(output: &Path, report: &Value) -> Result<()> {
    admission::publish_exclusive(
        output,
        &interval_history::encode_bounded(report, REPORT_CAP)?,
    )
}
fn stopped(report: &mut Value, failure: &rustaxa_storage::ConcreteDiagnosticFailure) {
    let f = native::failure_report(failure);
    report["status"] = json!("stopped");
    report["failure"] = f["failure"].clone();
    report["actual"] = f["actual"].clone();
    report["values"] = Value::Null;
}
/// Preflight repeats raw-seven authentication and prior proof without DB path
/// inspection. It publishes the complete fixed request/source/bound profile.
pub fn preflight(output: &Path) -> Result<()> {
    let output = native::output_path(output)?;
    let (plan, selected, authority) = authority()?;
    let mut report = base(selected, authority)?;
    report["status"] = json!("authority_inputs_authenticated_no_database_open");
    report["head"] = native::identity_json(plan.context().head);
    report["parent"] = native::identity_json(plan.context().parent);
    report["delayed"] = json!(
        plan.context()
            .delayed
            .iter()
            .copied()
            .map(native::identity_json)
            .collect::<Vec<_>>()
    );
    report["genesis_hash_hex"] = json!(hex::encode(plan.context().genesis_hash));
    report["head_header_sha256"] = json!(hex::encode(plan.context().head_header_sha256));
    publish(&output, &report)
}
/// Capture exactly forty-seven remaining requests after complete prior-proof
/// authority. One failure stops later reads and publishes no accepted prefix.
pub fn capture(bundle_output: &Path, report_output: &Path) -> Result<()> {
    let paths = paths::validate(Path::new(native::COPY), bundle_output)?;
    let reports = paths::validate(Path::new(native::COPY), report_output)?;
    ensure!(paths.output != reports.output, "window outputs overlap");
    let (plan, selected, authority) = authority()?;
    let mut report = base(selected, authority)?;
    interval_history::encode_bounded(&report, REPORT_CAP)?;
    match capture_vrf_voter_run(
        &paths.application,
        &paths.state,
        &plan,
        ConcreteWitnessLimits::default(),
    ) {
        Err(f) => {
            stopped(&mut report, &f);
            publish(&reports.output, &report)?;
            anyhow::bail!("window capture stopped; no prefix")
        }
        Ok(c) => {
            ensure!(c.bundle.len() <= native::BUNDLE_CAP, "window bundle cap");
            report["status"] = json!("captured_requires_offline_replay");
            report["actual"] = native::usage(c.usage);
            report["bundle"] = json!({"path":paths.output,"bytes":c.bundle.len(),"sha256":admission::sha(&c.bundle)});
            let encoded = interval_history::encode_bounded(&report, REPORT_CAP)?;
            admission::publish_exclusive(&paths.output, &c.bundle)?;
            admission::publish_exclusive(&reports.output, &encoded)
        }
    }
}
/// Offline replay repeats raw-seven authority, consumes every proof and returns
/// all forty-seven canonical values/provenance. No DB input is available.
pub fn replay(bundle_input: &Path, report_output: &Path) -> Result<()> {
    let output = native::output_path(report_output)?;
    let input = fs::canonicalize(bundle_input)?;
    admission::outside_db(&input)?;
    ensure!(
        fs::metadata(&input)?.is_file() && input != output,
        "window bundle input"
    );
    let (plan, selected, authority) = authority()?;
    let mut report = base(selected, authority)?;
    let bytes = admission::read_capped(fs::File::open(&input)?, native::BUNDLE_CAP)?;
    let verified = match replay_vrf_voter_bundle(&bytes, &plan, ConcreteWitnessLimits::default()) {
        Ok(v) => v,
        Err(f) => {
            stopped(&mut report, &f);
            publish(&output, &report)?;
            anyhow::bail!("window replay stopped; no prefix")
        }
    };
    let mut values = first::decode_values(&verified.logical)?;
    for (i, (value, physical)) in values
        .as_array_mut()
        .context("window values")?
        .iter_mut()
        .zip(verified.physical)
        .enumerate()
    {
        value["proof_index"] = json!(51 + i);
        value["physical"] = match physical {
            ConcreteWitnessPhysical::Present(v) => {
                json!({"kind":"present","bytes":v.len(),"sha256":admission::sha(&v)})
            }
            ConcreteWitnessPhysical::Tombstone => json!({"kind":"tombstone"}),
            ConcreteWitnessPhysical::HistoryUnavailable(id) => {
                json!({"kind":"successful_seek_without_matching_version","identity":native::identity_json(id)})
            }
        };
    }
    report["status"] = json!("all_forty_seven_vrf_inputs_authenticated");
    report["actual"] = native::usage(verified.usage);
    report["bundle"] = json!({"path":input,"bytes":bytes.len(),"sha256":admission::sha(&bytes)});
    report["values"] = values;
    report["qualification"]["vrf_family_qualified"] = json!(true);
    publish(&output, &report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_prior_value_must_match_proved_nonzero_member() {
        let member = ConcreteWitnessLogical::Member(vec![1; 32]);
        assert!(
            accepted_prior_value(&member, &ConcreteWitnessPhysical::Present(vec![1; 32])).is_ok()
        );
        for physical in [
            ConcreteWitnessPhysical::Tombstone,
            ConcreteWitnessPhysical::Present(vec![2; 32]),
        ] {
            assert!(accepted_prior_value(&member, &physical).is_err());
        }
        for logical in [
            ConcreteWitnessLogical::NonMember,
            ConcreteWitnessLogical::Member(vec![0; 32]),
            ConcreteWitnessLogical::Member(vec![1; 31]),
            ConcreteWitnessLogical::Member(vec![1; 33]),
        ] {
            assert!(
                accepted_prior_value(&logical, &ConcreteWitnessPhysical::Present(vec![1; 32]))
                    .is_err()
            );
        }
    }
    #[test]
    fn window_fresh_authority_selects_exact_remaining_voter_and_proves_prior() {
        let (plan, selected, summary) = authority().unwrap();
        assert_eq!(selected.as_array().unwrap().len(), 47);
        assert_eq!(hex::encode(plan.voter()), VOTER);
        assert_eq!(summary["raw_artifact_reads"], 7);
        assert_eq!(summary["raw_artifact_bytes"], 6785636);
        assert_eq!(summary["prior_first_d"]["selected"]["proof_index"], 61);
        for (i, t) in plan.tuples().iter().enumerate() {
            assert_eq!(t.identity.period.as_u64(), FIRST + 1 + i as u64);
        }
        let report = base(selected, summary).unwrap();
        assert!(interval_history::encode_bounded(&report, REPORT_CAP).is_ok());
        assert_eq!(report["vrf_sources"].as_array().unwrap().len(), 22);
    }
    #[test]
    fn window_wrong_prior_pin_and_incomplete_selector_stop() {
        let (context, rows, _) =
            parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let (first, _) = first::selection(context, &rows).unwrap();
        let c = first.context();
        let mut bad_context = rustaxa_storage::ConcreteDiagnosticPlan {
            genesis_hash: c.genesis_hash,
            head_header_sha256: c.head_header_sha256,
            head: c.head,
            parent: c.parent,
            delayed: c.delayed,
            tuples: c.tuples,
        };
        bad_context.head.state_root[0] ^= 1;
        let wrong = ConcreteVrfPlan::new(bad_context, *first.voters()).unwrap();
        let reached = std::cell::Cell::new(false);
        let valid = canonical_prior().unwrap();
        assert!(
            after_prior(wrong, &valid, |_, _| {
                reached.set(true);
                Ok(())
            })
            .is_err()
        );
        assert!(
            !reached.get(),
            "failed prior reached continuation/opener stage"
        );
        let mut bytes = canonical_prior().unwrap();
        bytes[0] ^= 1;
        assert!(prior(&bytes, &first).is_err());
        assert!(prior(&bytes[..bytes.len() - 1], &first).is_err());
        let mut incomplete = rows.clone();
        incomplete[89]["voter_hex"] = json!("0000000000000000000000000000000000000000");
        assert!(select(&incomplete).is_err());
        let mut refs = rows.clone();
        refs[89]["source_references"][0]["period"] = json!(0);
        assert!(select(&refs).is_err());
        let mut reordered = rows;
        reordered.as_array_mut().unwrap().swap(89, 149);
        assert!(select(&reordered).is_err());
    }
    #[test]
    fn sixth_window_rejects_previous_voter_evidence() {
        let (plan, _, _) = authority().unwrap();
        let input = Path::new(
            "/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/vrf-next-voter-window.rcv2",
        );
        let bytes = admission::evidence(
            input,
            input,
            "6c3e64369bd5243b20b315648f263b4db990d266fb5f0630097dfc5a7ada13b9",
            649774,
            649774,
        )
        .unwrap();
        let failure = replay_vrf_voter_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(
            failure.phase,
            rustaxa_storage::ConcreteDiagnosticPhase::Payload
        );
        assert_eq!(failure.index, 0);
        assert_eq!(
            failure.error,
            rustaxa_storage::ConcreteWitnessError::Identity
        );
        assert_eq!(
            failure
                .usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
        let third = Path::new(
            "/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/third-voter/vrf-third-voter-window.rcv2",
        );
        let bytes = admission::evidence(
            third,
            third,
            "b8d2c96ef4e244f07432319998d30c664a116dbbea08ce1bc470687f02626922",
            658422,
            658422,
        )
        .unwrap();
        let failure = replay_vrf_voter_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(
            failure.error,
            rustaxa_storage::ConcreteWitnessError::Identity
        );
        assert_eq!(
            failure.phase,
            rustaxa_storage::ConcreteDiagnosticPhase::Payload
        );
        assert_eq!(failure.index, 0);
        assert_eq!(
            failure
                .usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
        let fourth = Path::new(
            "/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/fourth-voter/vrf-fourth-voter-window.rcv2",
        );
        let bytes = admission::evidence(
            fourth,
            fourth,
            "5ffd64782835a0db3e3937ea7d5292bc0733887c31e90a90371c3af9d48b9e7a",
            653017,
            653017,
        )
        .unwrap();
        let failure = replay_vrf_voter_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(
            failure.error,
            rustaxa_storage::ConcreteWitnessError::Identity
        );
        assert_eq!(
            failure.phase,
            rustaxa_storage::ConcreteDiagnosticPhase::Payload
        );
        assert_eq!(failure.index, 0);
        assert_eq!(
            failure
                .usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
        let fifth = Path::new(
            "/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/fifth-voter/vrf-fifth-voter-window.rcv2",
        );
        let bytes = admission::evidence(
            fifth,
            fifth,
            "aef483116caabce7b5080bcbf2a7a67db51cdbcdf2465b1024cced711f8a30a7",
            677739,
            677739,
        )
        .unwrap();
        let failure = replay_vrf_voter_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(
            failure.error,
            rustaxa_storage::ConcreteWitnessError::Identity
        );
        assert_eq!(
            failure.phase,
            rustaxa_storage::ConcreteDiagnosticPhase::Payload
        );
        assert_eq!(failure.index, 0);
        assert_eq!(
            failure
                .usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
        let first = Path::new(
            "/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-voter-window.rcv2",
        );
        let bytes = admission::evidence(
            first,
            first,
            "5889004e9300ebd6ad3fb8dcccdd51cf12214528bc7ecf106057e0a2c4784e68",
            640045,
            640045,
        )
        .unwrap();
        let failure = replay_vrf_voter_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(
            failure.error,
            rustaxa_storage::ConcreteWitnessError::Identity
        );
        assert_eq!(
            failure.phase,
            rustaxa_storage::ConcreteDiagnosticPhase::Payload
        );
        assert_eq!(failure.index, 0);
        assert_eq!(
            failure
                .usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
    }
    #[test]
    fn sixth_window_requires_fixed_prefix_with_ten_complete_candidates() {
        let (_, mut rows, _) = parent_input_handoff::load_authenticated_voter_authority().unwrap();
        for row in rows.as_array_mut().unwrap() {
            if row["semantic_key"] == "fe_vrf_00_04_address" && row["voter_hex"] == FIRST_VOTER {
                row["voter_hex"] = json!("ffffffffffffffffffffffffffffffffffffffff");
            }
        }
        let mut periods = BTreeMap::<&str, BTreeSet<u64>>::new();
        for row in rows.as_array().unwrap() {
            if row["semantic_key"] == "fe_vrf_00_04_address" {
                periods
                    .entry(row["voter_hex"].as_str().unwrap())
                    .or_default()
                    .insert(row["delayed_period"].as_u64().unwrap());
            }
        }
        let complete: BTreeSet<_> = (FIRST..FIRST + 48).collect();
        assert_eq!(
            periods
                .values()
                .filter(|periods| **periods == complete)
                .count(),
            10
        );
        assert!(select(&rows).is_err());
    }
    #[test]
    fn window_report_and_semantic_caps_are_explicit() {
        assert!(
            interval_history::encode_bounded(&json!({"data":"x".repeat(REPORT_CAP)}), REPORT_CAP)
                .is_err()
        );
        let mut v = std::array::from_fn::<_, 47, _>(|_| ConcreteWitnessLogical::NonMember);
        v[0] = ConcreteWitnessLogical::Member(vec![0; 32]);
        v[46] = ConcreteWitnessLogical::Member(vec![1; 32]);
        let out = first::decode_values(&v).unwrap();
        assert_eq!(out[0]["usable_vrf_key"], false);
        assert_eq!(out[46]["usable_vrf_key"], true);
        v[46] = ConcreteWitnessLogical::Member(vec![1; 31]);
        assert!(first::decode_values(&v).is_err());
        assert!(
            base(
                json!([]),
                json!({"six_input_owner":{"sources":[{"bytes":4194304}]}})
            )
            .is_err()
        );
    }
}
