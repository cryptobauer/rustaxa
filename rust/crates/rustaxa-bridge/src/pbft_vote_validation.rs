//! CXX bridge wrappers for PBFT vote validation planning.
//!
//! The bridge exposes Rust-owned validation decisions to the C++ `VoteManager`
//! shim without transferring ownership of live vote objects or cryptographic
//! primitives. C++ supplies explicit lookup, crypto, and weight facts; Rust
//! returns stable statuses, replay-marker intent, and threshold values.

use crate::ffi::rustaxa_ffi::{
    PbftCanonicalVoteInspection as FfiPbftCanonicalVoteInspection,
    PbftCanonicalVoteValidation as FfiPbftCanonicalVoteValidation,
    PbftProposerSortitionFact as FfiPbftProposerSortitionFact,
    PbftProposerSortitionPlan as FfiPbftProposerSortitionPlan,
    PbftTwoTPlusOneThresholdFact as FfiPbftTwoTPlusOneThresholdFact,
    PbftTwoTPlusOneThresholdPlan as FfiPbftTwoTPlusOneThresholdPlan,
    PbftVoteValidationExternalFacts as FfiPbftVoteValidationExternalFacts,
    PbftVoteValidationFact as FfiPbftVoteValidationFact,
    PbftVoteValidationPlan as FfiPbftVoteValidationPlan,
};
use crate::ffi::BridgePbftVoteValidationRuntime;
use anyhow::Result;
use ethereum_types::H256;
use rustaxa_consensus::pbft_thresholds::{
    PbftTwoTPlusOneThresholdFact, PbftTwoTPlusOneThresholdPlan, PbftTwoTPlusOneThresholdRuntime,
    PbftTwoTPlusOneThresholdStatus,
};
use rustaxa_consensus::pbft_vote_validation::{
    inspect_canonical_pbft_vote, pbft_vote_sortition_threshold, plan_pbft_proposer_sortition,
    plan_pbft_vote_validation, validate_canonical_pbft_vote, PbftCanonicalVoteInspection,
    PbftCanonicalVoteValidation, PbftProposerSortitionFact, PbftVoteReplayCache,
    PbftVoteValidationExternalFacts, PbftVoteValidationFact,
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
        threshold_runtime: Mutex::new(PbftTwoTPlusOneThresholdRuntime::new()),
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

    /// Returns a Rust-owned PBFT `2t+1` threshold plan.
    ///
    /// C++ supplies FinalChain/PBFT-chain scalar facts. Rust owns cache lookup,
    /// threshold calculation, cache update policy, and failure-status mapping.
    pub fn pbft_two_t_plus_one_threshold(
        &self,
        fact: FfiPbftTwoTPlusOneThresholdFact,
    ) -> FfiPbftTwoTPlusOneThresholdPlan {
        let vote_type = match PbftVoteType::try_from(fact.vote_type) {
            Ok(vote_type) => vote_type,
            Err(_) => {
                return threshold_plan_to_ffi(PbftTwoTPlusOneThresholdPlan {
                    status: PbftTwoTPlusOneThresholdStatus::InvalidVoteType,
                    error_code: "PBFT_TWO_T_PLUS_ONE_INVALID_VOTE_TYPE",
                    has_threshold: false,
                    threshold: 0,
                    sortition_threshold: 0,
                    needs_total_dpos_votes: false,
                    cache_hit: false,
                    cached: false,
                });
            }
        };

        let mut runtime = self
            .threshold_runtime
            .lock()
            .expect("PBFT 2t+1 threshold runtime mutex poisoned");
        threshold_plan_to_ffi(runtime.plan_threshold(PbftTwoTPlusOneThresholdFact {
            pbft_period: fact.pbft_period,
            vote_type,
            current_pbft_chain_size: fact.current_pbft_chain_size,
            committee_size: fact.committee_size,
            number_of_proposers: fact.number_of_proposers,
            has_total_dpos_votes_count: fact.has_total_dpos_votes_count,
            total_dpos_votes_count: fact.total_dpos_votes_count,
            future_dpos_state: fact.future_dpos_state,
            unknown_error: fact.unknown_error,
        }))
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

fn threshold_plan_to_ffi(plan: PbftTwoTPlusOneThresholdPlan) -> FfiPbftTwoTPlusOneThresholdPlan {
    FfiPbftTwoTPlusOneThresholdPlan {
        status: plan.status.as_u8(),
        error_code: plan.error_code.to_owned(),
        has_threshold: plan.has_threshold,
        threshold: plan.threshold,
        sortition_threshold: plan.sortition_threshold,
        needs_total_dpos_votes: plan.needs_total_dpos_votes,
        cache_hit: plan.cache_hit,
        cached: plan.cached,
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

/// Validates one canonical PBFT vote from Rust byte inspection and C++ state facts.
pub fn pbft_validate_canonical_vote(
    vote_rlp: &[u8],
    facts: FfiPbftVoteValidationExternalFacts,
) -> Result<FfiPbftCanonicalVoteValidation> {
    Ok(validate_canonical_pbft_vote(
        vote_rlp,
        PbftVoteValidationExternalFacts {
            voter_dpos_ready: facts.voter_dpos_ready,
            voter_dpos_vote_count: facts.voter_dpos_vote_count,
            total_dpos_ready: facts.total_dpos_ready,
            total_dpos_vote_count: facts.total_dpos_vote_count,
            future_dpos_state: facts.future_dpos_state,
            unknown_error: facts.unknown_error,
            vrf_key_ready: facts.vrf_key_ready,
            has_vrf_key: facts.has_vrf_key,
            vrf_public_key: facts.vrf_public_key,
            strict_vrf: facts.strict_vrf,
            committee_size: facts.committee_size,
            number_of_proposers: facts.number_of_proposers,
        },
    )?
    .into())
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
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;

    const VRF_SECRET_KEY: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

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

    fn keccak256(data: &[u8]) -> H256 {
        use tiny_keccak::{Hasher, Keccak};

        let mut output = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(data);
        hasher.finalize(&mut output);
        H256::from(output)
    }

    fn signed_pbft_vote(
        signing_key: &SigningKey,
        block_hash: H256,
        period: u64,
        round: u64,
        step: u64,
    ) -> Vec<u8> {
        let mut vrf_message = RlpStream::new_list(3);
        vrf_message.append(&period);
        vrf_message.append(&round);
        vrf_message.append(&step);
        let proof = rustaxa_vdf::vrf::prove(&VRF_SECRET_KEY, &vrf_message.out()).unwrap();
        let mut sortition = RlpStream::new_list(4);
        sortition.append(&period);
        sortition.append(&round);
        sortition.append(&step);
        sortition.append(&proof.to_vec());
        let sortition_rlp = sortition.out().to_vec();

        let mut signing_stream = RlpStream::new_list(2);
        signing_stream.append(&block_hash);
        signing_stream.append(&sortition_rlp);
        let signing_hash = keccak256(&signing_stream.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(signing_hash.as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut vote = RlpStream::new_list(3);
        vote.append(&block_hash);
        vote.append(&sortition_rlp);
        vote.append(&signature_bytes);
        vote.out().to_vec()
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
    fn bridge_threshold_runtime_requests_facts_then_caches_current_period() {
        let runtime = create_pbft_vote_validation_runtime(100, 10);
        let missing = runtime.pbft_two_t_plus_one_threshold(FfiPbftTwoTPlusOneThresholdFact {
            pbft_period: 4,
            vote_type: 3,
            current_pbft_chain_size: 4,
            committee_size: 90,
            number_of_proposers: 20,
            has_total_dpos_votes_count: false,
            total_dpos_votes_count: 0,
            future_dpos_state: false,
            unknown_error: false,
        });
        assert_eq!(missing.status, 1);
        assert!(missing.needs_total_dpos_votes);

        let computed = runtime.pbft_two_t_plus_one_threshold(FfiPbftTwoTPlusOneThresholdFact {
            pbft_period: 4,
            vote_type: 3,
            current_pbft_chain_size: 4,
            committee_size: 90,
            number_of_proposers: 20,
            has_total_dpos_votes_count: true,
            total_dpos_votes_count: 90,
            future_dpos_state: false,
            unknown_error: false,
        });
        assert_eq!(computed.status, 0);
        assert_eq!(computed.sortition_threshold, 90);
        assert_eq!(computed.threshold, 61);
        assert!(computed.cached);

        let cached = runtime.pbft_two_t_plus_one_threshold(FfiPbftTwoTPlusOneThresholdFact {
            pbft_period: 4,
            vote_type: 3,
            current_pbft_chain_size: 4,
            committee_size: 90,
            number_of_proposers: 20,
            has_total_dpos_votes_count: false,
            total_dpos_votes_count: 0,
            future_dpos_state: false,
            unknown_error: false,
        });
        assert_eq!(cached.threshold, 61);
        assert!(cached.cache_hit);
    }

    #[test]
    fn bridge_threshold_runtime_maps_failure_statuses() {
        let runtime = create_pbft_vote_validation_runtime(100, 10);
        let invalid = runtime.pbft_two_t_plus_one_threshold(FfiPbftTwoTPlusOneThresholdFact {
            pbft_period: 4,
            vote_type: 99,
            current_pbft_chain_size: 4,
            committee_size: 90,
            number_of_proposers: 20,
            has_total_dpos_votes_count: true,
            total_dpos_votes_count: 90,
            future_dpos_state: false,
            unknown_error: false,
        });
        assert_eq!(invalid.status, 4);
        assert_eq!(invalid.error_code, "PBFT_TWO_T_PLUS_ONE_INVALID_VOTE_TYPE");

        let future = runtime.pbft_two_t_plus_one_threshold(FfiPbftTwoTPlusOneThresholdFact {
            pbft_period: 4,
            vote_type: 3,
            current_pbft_chain_size: 4,
            committee_size: 90,
            number_of_proposers: 20,
            has_total_dpos_votes_count: false,
            total_dpos_votes_count: 0,
            future_dpos_state: true,
            unknown_error: false,
        });
        assert_eq!(future.status, 2);
        assert!(!future.has_threshold);
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

    #[test]
    fn bridge_inspects_canonical_pbft_vote_without_throwing_on_peer_errors() {
        let inspected = pbft_inspect_canonical_vote(&[0x01, 0x02, 0x03]).unwrap();

        assert_eq!(inspected.status, 1);
        assert_eq!(inspected.error_code, "PBFT_CANONICAL_VOTE_MALFORMED_RLP");
    }

    #[test]
    fn bridge_validates_canonical_pbft_vote_with_external_facts() {
        let signing_key = SigningKey::from_slice(&[0x41; 32]).unwrap();
        let vote_rlp = signed_pbft_vote(&signing_key, H256::from_low_u64_be(55), 9, 2, 3);
        let vrf_public_key = rustaxa_vdf::vrf::public_key_from_secret(&VRF_SECRET_KEY).unwrap();

        let validation = pbft_validate_canonical_vote(
            &vote_rlp,
            FfiPbftVoteValidationExternalFacts {
                voter_dpos_ready: true,
                voter_dpos_vote_count: 42,
                total_dpos_ready: true,
                total_dpos_vote_count: 100,
                future_dpos_state: false,
                unknown_error: false,
                vrf_key_ready: true,
                has_vrf_key: true,
                vrf_public_key,
                strict_vrf: true,
                committee_size: 100,
                number_of_proposers: 20,
            },
        )
        .unwrap();

        assert_eq!(validation.status, 1);
        assert!(validation.accepted);
        assert!(validation.vrf_valid);
        assert_eq!(validation.calculated_weight, 42);
        assert_eq!(H256::from(validation.vote_hash), keccak256(&vote_rlp));
    }
}
