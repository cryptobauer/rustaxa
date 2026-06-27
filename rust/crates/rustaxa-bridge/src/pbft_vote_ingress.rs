//! Facade adapters for PBFT vote ingress planning.
//!
//! `BridgeConsensusNetworkApi` exposes deterministic packet-adjacent PBFT vote
//! gates to C++ as scalar facts. Rust decides relevance, legacy-compatible
//! period/round/step windows, bundle propose-vote rejection, and sync-request
//! hints. C++ remains responsible for packet decoding, peer timers,
//! disconnects, vote admission, proposed-block execution, and gossip.

use crate::ffi::rustaxa_ffi::{
    PbftVoteIngressContext as FfiPbftVoteIngressContext,
    PbftVoteIngressFact as FfiPbftVoteIngressFact, PbftVoteIngressPlan as FfiPbftVoteIngressPlan,
};
use anyhow::Result;
use rustaxa_consensus::pbft_vote_ingress::{
    PbftVoteIngressContext, PbftVoteIngressFact, PbftVoteIngressPlan, PbftVoteIngressStatus,
};
use rustaxa_consensus::verified_votes::PbftVoteType;

pub(crate) fn fact_to_domain(value: FfiPbftVoteIngressFact) -> Result<PbftVoteIngressFact> {
    Ok(PbftVoteIngressFact {
        period: value.period,
        round: value.round,
        step: value.step,
        vote_type: PbftVoteType::try_from(value.vote_type)?,
    })
}

pub(crate) const fn context_to_domain(value: FfiPbftVoteIngressContext) -> PbftVoteIngressContext {
    PbftVoteIngressContext {
        current_period: value.current_period,
        current_round: value.current_round,
        current_step: value.current_step,
        max_future_period_delta: value.max_future_period_delta,
        max_future_round_delta: value.max_future_round_delta,
        max_future_step_delta: value.max_future_step_delta,
        validate_max_round_step: value.validate_max_round_step,
        source_peer_is_voter: value.source_peer_is_voter,
        can_request_pbft_sync: value.can_request_pbft_sync,
        can_request_next_votes_sync: value.can_request_next_votes_sync,
    }
}

pub(crate) fn plan_to_ffi(plan: PbftVoteIngressPlan) -> FfiPbftVoteIngressPlan {
    FfiPbftVoteIngressPlan {
        status: plan.status.as_u8(),
        error_code: error_code(plan.status).to_owned(),
        accepted: plan.accepted,
        relevant: plan.relevant,
        request_pbft_sync: plan.request_pbft_sync,
        request_next_votes_sync: plan.request_next_votes_sync,
        checking_round: plan.checking_round,
        checking_step: plan.checking_step,
    }
}

const fn error_code(status: PbftVoteIngressStatus) -> &'static str {
    match status {
        PbftVoteIngressStatus::Accepted => "",
        PbftVoteIngressStatus::Irrelevant => "PBFT_VOTE_INGRESS_IRRELEVANT",
        PbftVoteIngressStatus::InvalidPeriodTooSmall => {
            "PBFT_VOTE_INGRESS_INVALID_PERIOD_TOO_SMALL"
        }
        PbftVoteIngressStatus::InvalidPeriodTooBig => "PBFT_VOTE_INGRESS_INVALID_PERIOD_TOO_BIG",
        PbftVoteIngressStatus::InvalidRoundTooSmall => "PBFT_VOTE_INGRESS_INVALID_ROUND_TOO_SMALL",
        PbftVoteIngressStatus::InvalidRoundTooBig => "PBFT_VOTE_INGRESS_INVALID_ROUND_TOO_BIG",
        PbftVoteIngressStatus::InvalidStepTooBig => "PBFT_VOTE_INGRESS_INVALID_STEP_TOO_BIG",
        PbftVoteIngressStatus::UnsupportedBundleProposeVote => {
            "PBFT_VOTE_INGRESS_UNSUPPORTED_BUNDLE_PROPOSE_VOTE"
        }
        PbftVoteIngressStatus::BundleVoteMismatch => "PBFT_VOTE_INGRESS_BUNDLE_VOTE_MISMATCH",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_consensus::pbft_vote_ingress::{
        plan_pbft_vote_bundle_ingress, plan_pbft_vote_ingress,
    };

    const fn fact(period: u64, round: u64, step: u64, vote_type: u8) -> FfiPbftVoteIngressFact {
        FfiPbftVoteIngressFact {
            period,
            round,
            step,
            vote_type,
        }
    }

    const fn context() -> FfiPbftVoteIngressContext {
        FfiPbftVoteIngressContext {
            current_period: 10,
            current_round: 3,
            current_step: 2,
            max_future_period_delta: 2,
            max_future_round_delta: 3,
            max_future_step_delta: 4,
            validate_max_round_step: true,
            source_peer_is_voter: true,
            can_request_pbft_sync: true,
            can_request_next_votes_sync: true,
        }
    }

    #[test]
    fn facade_adapter_maps_single_vote_ingress_plan() {
        let plan = plan_to_ffi(plan_pbft_vote_ingress(
            fact_to_domain(fact(10, 3, 2, 2)).unwrap(),
            context_to_domain(context()),
        ));

        assert!(plan.accepted);
        assert_eq!(plan.status, 0);
        assert_eq!(plan.error_code, "");
        assert_eq!(plan.checking_round, 3);
        assert_eq!(plan.checking_step, 2);
    }

    #[test]
    fn facade_adapter_maps_sync_hints() {
        let plan = plan_to_ffi(plan_pbft_vote_ingress(
            fact_to_domain(fact(14, 3, 1, 2)).unwrap(),
            context_to_domain(context()),
        ));

        assert!(!plan.accepted);
        assert!(plan.request_pbft_sync);
        assert_eq!(plan.error_code, "PBFT_VOTE_INGRESS_INVALID_PERIOD_TOO_BIG");
    }

    #[test]
    fn facade_adapter_rejects_bundle_mismatch() {
        let plan = plan_to_ffi(plan_pbft_vote_bundle_ingress(
            fact_to_domain(fact(10, 3, 2, 2)).unwrap(),
            fact_to_domain(fact(10, 3, 3, 2)).unwrap(),
            context_to_domain(context()),
        ));

        assert!(!plan.accepted);
        assert_eq!(plan.error_code, "PBFT_VOTE_INGRESS_BUNDLE_VOTE_MISMATCH");
    }
}
