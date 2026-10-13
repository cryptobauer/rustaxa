//! Fixed compatibility entrypoint. All behavior belongs to the sealed shared engine.
use crate::vrf_shared_window::{self as shared, Profile};
use anyhow::Result;
use std::path::Path;
pub const PROFILE: &str = "n4_remaining_vrf_voter_6bf44c438f6328640b0bc5d7d7766310cf49610f_v1";
pub const REPORT_CAP: usize = 131072;
#[cfg(test)]
const VOTER: &str = "6bf44c438f6328640b0bc5d7d7766310cf49610f";
#[cfg(test)]
const FIRST_VOTER: &str = "138c1b9baea7d93a981ba694f5318f735c997298";
#[cfg(test)]
const FIRST: u64 = 25706894;
/// Authenticate this fixed profile without database access.
pub fn preflight(output: &Path) -> Result<()> {
    shared::preflight(Profile::Fifth, output)
}
/// Capture this fixed profile through a separately reviewed one-use guard.
pub fn capture(bundle: &Path, report: &Path) -> Result<()> {
    shared::capture(Profile::Fifth, bundle, report)
}
/// Replay every proof for this fixed profile offline.
pub fn replay(bundle: &Path, report: &Path) -> Result<()> {
    shared::replay(Profile::Fifth, bundle, report)
}
#[cfg(test)]
use crate::{
    artifact_admission as admission, interval_history, parent_input_handoff, vrf_witness as first,
};
#[cfg(test)]
use rustaxa_storage::{
    ConcreteVrfPlan, ConcreteWitnessLimits, ConcreteWitnessLogical, ConcreteWitnessPhysical,
    replay_vrf_voter_bundle,
};
#[cfg(test)]
use serde_json::{Value, json};
#[cfg(test)]
use shared::{accepted_prior_value, canonical_prior};
#[cfg(test)]
fn select(rows: &Value) -> Result<Value> {
    shared::select(Profile::Fifth, rows)
}
#[cfg(test)]
fn prior(bytes: &[u8], plan: &ConcreteVrfPlan) -> Result<Value> {
    shared::prior(Profile::Fifth, bytes, plan)
}
#[cfg(test)]
fn authority() -> Result<(rustaxa_storage::ConcreteVrfVoterPlan, Value, Value)> {
    shared::authority(Profile::Fifth)
}
#[cfg(test)]
fn base(selected: Value, authority: Value) -> Result<Value> {
    shared::base(Profile::Fifth, selected, authority)
}
#[cfg(test)]
fn after_prior<T>(
    plan: ConcreteVrfPlan,
    bytes: &[u8],
    next: impl FnOnce(ConcreteVrfPlan, Value) -> Result<T>,
) -> Result<T> {
    shared::after_prior(Profile::Fifth, plan, bytes, next)
}
#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
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
        assert_eq!(summary["prior_first_d"]["selected"]["proof_index"], 58);
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
        incomplete[83]["voter_hex"] = json!("0000000000000000000000000000000000000000");
        assert!(select(&incomplete).is_err());
        let mut refs = rows.clone();
        refs[83]["source_references"][0]["period"] = json!(0);
        assert!(select(&refs).is_err());
        let mut reordered = rows;
        reordered.as_array_mut().unwrap().swap(83, 140);
        assert!(select(&reordered).is_err());
    }
    #[test]
    fn fifth_window_rejects_previous_voter_evidence() {
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
    fn fifth_window_requires_fixed_prefix_with_ten_complete_candidates() {
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
