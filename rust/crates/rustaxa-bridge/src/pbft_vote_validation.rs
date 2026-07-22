//! CXX bridge wrappers for PBFT vote validation planning.
//!
//! The bridge exposes Rust-owned validation decisions without transferring
//! ownership of live vote objects or cryptographic primitives. The production
//! `VoteManager` shim uses verified-vote methods on `BridgePbftService` so
//! validation replay protection, threshold caching, verified-vote metadata,
//! and retained vote payloads share one Rust runtime.

use crate::ffi::rustaxa_ffi::{
    PbftCanonicalVoteInspection as FfiPbftCanonicalVoteInspection,
    PbftCanonicalVoteValidation as FfiPbftCanonicalVoteValidation,
    PbftProposerSortitionFact as FfiPbftProposerSortitionFact,
    PbftProposerSortitionPlan as FfiPbftProposerSortitionPlan,
    PbftTwoTPlusOneThresholdPlan as FfiPbftTwoTPlusOneThresholdPlan,
};
use anyhow::Result;
use rustaxa_consensus::pbft_thresholds::PbftTwoTPlusOneThresholdPlan;
use rustaxa_consensus::pbft_vote_validation::{
    inspect_canonical_pbft_vote, plan_pbft_proposer_sortition, PbftCanonicalVoteInspection,
    PbftCanonicalVoteValidation, PbftProposerSortitionFact,
};

pub(crate) fn threshold_plan_to_ffi(
    plan: PbftTwoTPlusOneThresholdPlan,
) -> FfiPbftTwoTPlusOneThresholdPlan {
    FfiPbftTwoTPlusOneThresholdPlan {
        status: plan.status.as_u8(),
        error_code: plan.error_code.to_owned(),
        has_threshold: plan.has_threshold,
        threshold: plan.threshold,
    }
}

/// Inspects peer-controlled canonical PBFT vote RLP in Rust.
///
/// Malformed vote bytes are returned as an inspection status rather than a
/// bridge error so C++ can reject network data without treating it as an
/// internal failure.
pub fn pbft_inspect_canonical_vote(vote_rlp: &[u8]) -> Result<FfiPbftCanonicalVoteInspection> {
    Ok(inspect_canonical_pbft_vote(vote_rlp)?.into())
}

/// Plans screening for one locally generated proposer sortition.
pub fn pbft_proposer_sortition_plan(
    fact: FfiPbftProposerSortitionFact,
) -> Result<FfiPbftProposerSortitionPlan> {
    let plan = plan_pbft_proposer_sortition(PbftProposerSortitionFact {
        dpos_vote_count_ready: fact.dpos_vote_count_ready,
        dpos_vote_count: fact.dpos_vote_count,
        total_dpos_vote_count_ready: fact.total_dpos_vote_count_ready,
        total_dpos_vote_count: fact.total_dpos_vote_count,
        weight_ready: fact.weight_ready,
        weight: fact.weight,
        future_dpos_state: fact.future_dpos_state,
        unknown_error: fact.unknown_error,
        number_of_proposers: fact.number_of_proposers,
    });

    Ok(FfiPbftProposerSortitionPlan {
        status: plan.status.as_u8(),
        error_code: proposer_sortition_error_code(plan.status).to_owned(),
        accepted: plan.accepted,
        rejected: plan.rejected,
        has_sortition_threshold: plan.has_sortition_threshold,
        sortition_threshold: plan.sortition_threshold,
    })
}

impl From<PbftCanonicalVoteInspection> for FfiPbftCanonicalVoteInspection {
    fn from(value: PbftCanonicalVoteInspection) -> Self {
        Self {
            status: value.status.as_u8(),
            error_code: value.error_code.to_owned(),
            vote_hash: value.vote_hash.into(),
            signing_hash: value.signing_hash.into(),
            block_hash: value.block_hash.into(),
            period: value.period,
            round: value.round,
            step: value.step,
            vote_type: value.vote_type.into(),
            recovered_public_key: value.recovered_public_key,
            recovered_voter: value.recovered_voter.0,
            signature_valid: value.signature_valid,
            vrf_proof: value.vrf_proof,
            has_embedded_weight: value.has_embedded_weight,
            embedded_weight: value.embedded_weight,
        }
    }
}

impl From<PbftCanonicalVoteValidation> for FfiPbftCanonicalVoteValidation {
    fn from(value: PbftCanonicalVoteValidation) -> Self {
        Self {
            status: value.status.as_u8(),
            error_code: value.error_code.to_owned(),
            accepted: value.accepted,
            rejected: value.rejected,
            mark_validated_replay: value.mark_validated_replay,
            vote_hash: value.vote_hash.into(),
            signing_hash: value.signing_hash.into(),
            block_hash: value.block_hash.into(),
            period: value.period,
            round: value.round,
            step: value.step,
            vote_type: value.vote_type.into(),
            recovered_voter: value.recovered_voter.0,
            recovered_public_key: value.recovered_public_key,
            signature_valid: value.signature_valid,
            vrf_valid: value.vrf_valid,
            has_sortition_threshold: value.has_sortition_threshold,
            sortition_threshold: value.sortition_threshold,
            weight_calculated: value.weight_calculated,
            calculated_weight: value.calculated_weight,
            vrf_output: value.vrf_output,
        }
    }
}

const fn proposer_sortition_error_code(
    status: rustaxa_consensus::pbft_vote_validation::PbftProposerSortitionStatus,
) -> &'static str {
    match status {
        rustaxa_consensus::pbft_vote_validation::PbftProposerSortitionStatus::Pending
        | rustaxa_consensus::pbft_vote_validation::PbftProposerSortitionStatus::Valid => "",
        rustaxa_consensus::pbft_vote_validation::PbftProposerSortitionStatus::ZeroStake => {
            "PBFT_PROPOSER_SORTITION_ZERO_STAKE"
        }
        rustaxa_consensus::pbft_vote_validation::PbftProposerSortitionStatus::ZeroWeight => {
            "PBFT_PROPOSER_SORTITION_ZERO_WEIGHT"
        }
        rustaxa_consensus::pbft_vote_validation::PbftProposerSortitionStatus::FutureDposState => {
            "PBFT_PROPOSER_SORTITION_FUTURE_DPOS_STATE"
        }
        rustaxa_consensus::pbft_vote_validation::PbftProposerSortitionStatus::UnknownError => {
            "PBFT_PROPOSER_SORTITION_UNKNOWN_ERROR"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_screens_local_proposer_sortition() {
        let plan = pbft_proposer_sortition_plan(FfiPbftProposerSortitionFact {
            dpos_vote_count_ready: true,
            dpos_vote_count: 10,
            total_dpos_vote_count_ready: true,
            total_dpos_vote_count: 100,
            weight_ready: true,
            weight: 1,
            future_dpos_state: false,
            unknown_error: false,
            number_of_proposers: 20,
        })
        .unwrap();

        assert_eq!(plan.status, 1);
        assert!(plan.accepted);
        assert_eq!(plan.sortition_threshold, 20);
    }

    #[test]
    fn bridge_inspects_canonical_pbft_vote_without_throwing_on_peer_errors() {
        let inspected = pbft_inspect_canonical_vote(&[0x01, 0x02, 0x03]).unwrap();

        assert_eq!(inspected.status, 1);
        assert_eq!(inspected.error_code, "PBFT_CANONICAL_VOTE_MALFORMED_RLP");
    }
}
