//! CXX bridge wrappers for PBFT vote validation planning.
//!
//! The bridge exposes Rust-owned validation decisions to the C++ `VoteManager`
//! shim without transferring ownership of live vote objects or cryptographic
//! primitives. C++ supplies explicit lookup, crypto, and weight facts; Rust
//! returns stable statuses, replay-marker intent, and threshold values.

use crate::ffi::rustaxa_ffi::{
    PbftProposerSortitionFact as FfiPbftProposerSortitionFact,
    PbftProposerSortitionPlan as FfiPbftProposerSortitionPlan,
    PbftVoteValidationFact as FfiPbftVoteValidationFact,
    PbftVoteValidationPlan as FfiPbftVoteValidationPlan,
};
use crate::ffi::BridgePbftVoteValidationRuntime;
use anyhow::Result;
use ethereum_types::H256;
use rustaxa_consensus::pbft_vote_validation::{
    pbft_vote_sortition_threshold, plan_pbft_proposer_sortition, plan_pbft_vote_validation,
    PbftProposerSortitionFact, PbftVoteReplayCache, PbftVoteValidationFact,
};
use rustaxa_consensus::verified_votes::PbftVoteType;
use std::sync::Mutex;

/// Creates a Rust-owned PBFT vote validation runtime.
///
/// Inputs:
/// - `max_size`: maximum retained replay hashes.
/// - `delete_step`: number of oldest hashes evicted when capacity is crossed.
///
/// Outputs:
/// - A bridge handle whose replay cache is independent from verified-vote
///   storage and can be queried by `VoteManager::voteAlreadyValidated`.
pub fn create_pbft_vote_validation_runtime(
    max_size: usize,
    delete_step: usize,
) -> Box<BridgePbftVoteValidationRuntime> {
    Box::new(BridgePbftVoteValidationRuntime {
        replay_cache: Mutex::new(PbftVoteReplayCache::new(max_size, delete_step)),
    })
}

impl BridgePbftVoteValidationRuntime {
    /// Returns whether the vote hash is already in Rust replay protection.
    pub fn pbft_vote_replay_contains(&self, vote_hash: &[u8; 32]) -> bool {
        self.replay_cache
            .lock()
            .expect("PBFT vote replay cache mutex poisoned")
            .contains(H256::from(*vote_hash))
    }

    /// Inserts a vote hash into Rust replay protection.
    ///
    /// The return value is true only for a newly inserted hash. Duplicate
    /// inserts are accepted and return false to match legacy cache semantics.
    pub fn pbft_vote_replay_insert(&self, vote_hash: &[u8; 32]) -> bool {
        self.replay_cache
            .lock()
            .expect("PBFT vote replay cache mutex poisoned")
            .insert(H256::from(*vote_hash))
    }
}

/// Computes the PBFT sortition threshold from legacy-compatible scalar facts.
///
/// Inputs are plain integers so C++ shims can call this helper without
/// materializing any Rust state. Unsupported vote-type values are returned as
/// bridge errors.
pub fn pbft_vote_sortition_threshold_for_bridge(
    total_dpos_vote_count: u64,
    vote_type: u8,
    committee_size: u64,
    number_of_proposers: u64,
) -> Result<u64> {
    pbft_vote_sortition_threshold(
        total_dpos_vote_count,
        PbftVoteType::try_from(vote_type)?,
        committee_size,
        number_of_proposers,
    )
}

/// Plans validation for one received PBFT vote from caller-supplied facts.
pub fn pbft_vote_validation_plan(
    fact: FfiPbftVoteValidationFact,
) -> Result<FfiPbftVoteValidationPlan> {
    let plan = plan_pbft_vote_validation(PbftVoteValidationFact {
        vote_type: PbftVoteType::try_from(fact.vote_type)?,
        dpos_vote_count_ready: fact.dpos_vote_count_ready,
        dpos_vote_count: fact.dpos_vote_count,
        vrf_key_ready: fact.vrf_key_ready,
        has_vrf_key: fact.has_vrf_key,
        signature_ready: fact.signature_ready,
        signature_valid: fact.signature_valid,
        vrf_sortition_ready: fact.vrf_sortition_ready,
        vrf_sortition_valid: fact.vrf_sortition_valid,
        total_dpos_vote_count_ready: fact.total_dpos_vote_count_ready,
        total_dpos_vote_count: fact.total_dpos_vote_count,
        weight_ready: fact.weight_ready,
        weight: fact.weight,
        future_dpos_state: fact.future_dpos_state,
        unknown_error: fact.unknown_error,
        committee_size: fact.committee_size,
        number_of_proposers: fact.number_of_proposers,
    });

    Ok(FfiPbftVoteValidationPlan {
        status: plan.status.as_u8(),
        error_code: vote_validation_error_code(plan.status).to_owned(),
        accepted: plan.accepted,
        rejected: plan.rejected,
        mark_validated_replay: plan.mark_validated_replay,
        has_sortition_threshold: plan.has_sortition_threshold,
        sortition_threshold: plan.sortition_threshold,
    })
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

const fn vote_validation_error_code(
    status: rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus,
) -> &'static str {
    match status {
        rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::Pending
        | rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::Valid => "",
        rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::ZeroStake => {
            "PBFT_VOTE_VALIDATION_ZERO_STAKE"
        }
        rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::MissingVrfKey => {
            "PBFT_VOTE_VALIDATION_MISSING_VRF_KEY"
        }
        rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::InvalidSignature => {
            "PBFT_VOTE_VALIDATION_INVALID_SIGNATURE"
        }
        rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::InvalidVrfProof => {
            "PBFT_VOTE_VALIDATION_INVALID_VRF_PROOF"
        }
        rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::ZeroWeight => {
            "PBFT_VOTE_VALIDATION_ZERO_WEIGHT"
        }
        rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::FutureDposState => {
            "PBFT_VOTE_VALIDATION_FUTURE_DPOS_STATE"
        }
        rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::UnknownError => {
            "PBFT_VOTE_VALIDATION_UNKNOWN_ERROR"
        }
        rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus::InvalidVoteType => {
            "PBFT_VOTE_VALIDATION_INVALID_VOTE_TYPE"
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

    fn valid_fact() -> FfiPbftVoteValidationFact {
        FfiPbftVoteValidationFact {
            vote_type: 3,
            dpos_vote_count_ready: true,
            dpos_vote_count: 10,
            vrf_key_ready: true,
            has_vrf_key: true,
            signature_ready: true,
            signature_valid: true,
            vrf_sortition_ready: true,
            vrf_sortition_valid: true,
            total_dpos_vote_count_ready: true,
            total_dpos_vote_count: 100,
            weight_ready: true,
            weight: 4,
            future_dpos_state: false,
            unknown_error: false,
            committee_size: 50,
            number_of_proposers: 20,
        }
    }

    #[test]
    fn bridge_returns_stable_vote_validation_codes() {
        let mut fact = valid_fact();
        fact.signature_valid = false;

        let plan = pbft_vote_validation_plan(fact).unwrap();

        assert_eq!(plan.status, 4);
        assert_eq!(plan.error_code, "PBFT_VOTE_VALIDATION_INVALID_SIGNATURE");
        assert!(plan.rejected);
        assert!(plan.mark_validated_replay);
    }

    #[test]
    fn bridge_rejects_malformed_vote_type() {
        let mut fact = valid_fact();
        fact.vote_type = 99;

        assert!(pbft_vote_validation_plan(fact).is_err());
    }

    #[test]
    fn bridge_exposes_threshold_helper() {
        assert_eq!(
            pbft_vote_sortition_threshold_for_bridge(100, 1, 50, 20).unwrap(),
            20
        );
        assert_eq!(
            pbft_vote_sortition_threshold_for_bridge(12, 3, 50, 20).unwrap(),
            12
        );
    }

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
}
