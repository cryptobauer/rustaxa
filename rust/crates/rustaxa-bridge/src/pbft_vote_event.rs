//! CXX bridge wrapper for PBFT vote event fact construction.
//!
//! The bridge lets C++ shims derive compact vote-progress facts from canonical
//! PBFT vote bytes without moving network ingress or live side-effect execution
//! into Rust. C++ still supplies the validation weight and ingress flags; Rust
//! owns canonical byte inspection and consensus identity extraction.

use crate::ffi::rustaxa_ffi::{
    PbftCanonicalVoteValidation as FfiPbftCanonicalVoteValidation,
    PbftVoteEventFact as FfiPbftVoteEventFact, PbftVoteEventFactFlags as FfiPbftVoteEventFactFlags,
    PbftVoteFactBoundaryResult as FfiPbftVoteFactBoundaryResult,
    PbftVoteProgressFact as FfiPbftVoteProgressFact,
    PbftVoteValidationExternalFacts as FfiPbftVoteValidationExternalFacts, VerifiedVotePayload,
};
use anyhow::Result;
use rustaxa_consensus::pbft_vote_event::{
    build_pbft_vote_event_fact, build_pbft_vote_event_fact_from_validation, PbftVoteEventFact,
    PbftVoteEventFactFlags,
};
use rustaxa_consensus::pbft_vote_progress::PbftVoteProgressFact;
use rustaxa_consensus::pbft_vote_validation::{
    validate_canonical_pbft_vote, PbftCanonicalVoteValidation, PbftVoteValidationExternalFacts,
};

/// Builds compact PBFT vote event facts from canonical vote RLP.
///
/// Inputs:
/// - `canonical_vote_rlp`: legacy `PbftVote::rlp(true, false)` bytes.
/// - `weight`: already-calculated vote weight from validation.
/// - `flags`: ingress/validation flags that are not encoded in the vote bytes.
///
/// Outputs:
/// - A stable status plus a progress fact when canonical inspection and weight
///   checks succeed. Malformed peer-controlled bytes are returned as statuses
///   rather than bridge errors.
pub fn pbft_vote_event_fact_from_canonical_vote(
    canonical_vote_rlp: &[u8],
    weight: u64,
    flags: FfiPbftVoteEventFactFlags,
) -> Result<FfiPbftVoteEventFact> {
    Ok(event_to_ffi(build_pbft_vote_event_fact(
        canonical_vote_rlp,
        weight,
        flags_to_domain(flags),
    )?))
}

/// Derives a validation-backed PBFT vote progress fact from canonical vote RLP.
///
/// Inputs:
/// - `canonical_vote_rlp`: legacy `PbftVote::rlp(true, false)` bytes.
/// - `validation_facts`: FinalChain/key/VRF facts collected by C++.
/// - `flags`: ingress/validation flags not encoded in the vote bytes.
///
/// Outputs:
/// - The full canonical validation result plus an optional progress fact built
///   from validation identity and calculated weight. Non-accepted validation
///   statuses never produce progress facts.
pub fn pbft_derive_vote_progress_fact_from_canonical_vote(
    canonical_vote_rlp: &[u8],
    validation_facts: FfiPbftVoteValidationExternalFacts,
    flags: FfiPbftVoteEventFactFlags,
) -> Result<FfiPbftVoteFactBoundaryResult> {
    let validation = validate_canonical_pbft_vote(
        canonical_vote_rlp,
        validation_facts_to_domain(validation_facts),
    )?;
    let event = build_pbft_vote_event_fact_from_validation(&validation, flags_to_domain(flags));
    Ok(boundary_result_to_ffi(validation, event))
}

fn flags_to_domain(value: FfiPbftVoteEventFactFlags) -> PbftVoteEventFactFlags {
    PbftVoteEventFactFlags {
        vote_already_known: value.vote_already_known,
        carries_proposed_block: value.carries_proposed_block,
        valid_stale_reward_vote: value.valid_stale_reward_vote,
    }
}

fn validation_facts_to_domain(
    value: FfiPbftVoteValidationExternalFacts,
) -> PbftVoteValidationExternalFacts {
    PbftVoteValidationExternalFacts {
        voter_dpos_ready: value.voter_dpos_ready,
        voter_dpos_vote_count: value.voter_dpos_vote_count,
        total_dpos_ready: value.total_dpos_ready,
        total_dpos_vote_count: value.total_dpos_vote_count,
        future_dpos_state: value.future_dpos_state,
        unknown_error: value.unknown_error,
        vrf_key_ready: value.vrf_key_ready,
        has_vrf_key: value.has_vrf_key,
        vrf_public_key: value.vrf_public_key,
        strict_vrf: value.strict_vrf,
        committee_size: value.committee_size,
        number_of_proposers: value.number_of_proposers,
        has_preverified_weight: value.has_preverified_weight,
        preverified_weight: value.preverified_weight,
    }
}

fn boundary_result_to_ffi(
    validation: PbftCanonicalVoteValidation,
    event: PbftVoteEventFact,
) -> FfiPbftVoteFactBoundaryResult {
    let validation = FfiPbftCanonicalVoteValidation::from(validation);
    let has_progress_fact = event.progress_fact.is_some();
    FfiPbftVoteFactBoundaryResult {
        status: event.status.as_u8(),
        error_code: event.error_code.to_owned(),
        validation,
        has_progress_fact,
        progress_fact: event
            .progress_fact
            .map(progress_fact_to_ffi)
            .unwrap_or_else(empty_progress_fact),
    }
}

fn event_to_ffi(event: PbftVoteEventFact) -> FfiPbftVoteEventFact {
    let has_progress_fact = event.progress_fact.is_some();
    FfiPbftVoteEventFact {
        status: event.status.as_u8(),
        error_code: event.error_code.to_owned(),
        has_progress_fact,
        progress_fact: event
            .progress_fact
            .map(progress_fact_to_ffi)
            .unwrap_or_else(empty_progress_fact),
    }
}

fn progress_fact_to_ffi(value: PbftVoteProgressFact) -> FfiPbftVoteProgressFact {
    FfiPbftVoteProgressFact {
        vote: VerifiedVotePayload {
            vote_hash: value.identity.vote_hash.into(),
            block_hash: value.identity.block_hash.into(),
            voter: value.identity.voter.0,
            period: value.identity.period,
            round: value.identity.round,
            step: value.identity.step,
            vote_type: value.vote_type.into(),
            weight: value.weight,
        },
        vote_already_known: value.vote_already_known,
        carries_proposed_block: value.carries_proposed_block,
        valid_stale_reward_vote: value.valid_stale_reward_vote,
    }
}

fn empty_progress_fact() -> FfiPbftVoteProgressFact {
    FfiPbftVoteProgressFact {
        vote: VerifiedVotePayload {
            vote_hash: [0; 32],
            block_hash: [0; 32],
            voter: [0; 20],
            period: 0,
            round: 0,
            step: 0,
            vote_type: 0,
            weight: 0,
        },
        vote_already_known: false,
        carries_proposed_block: false,
        valid_stale_reward_vote: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::PbftVoteGenerationInput;
    use crate::pbft_vote_generation::pbft_generate_signed_vote;
    use k256::ecdsa::SigningKey;

    fn flags() -> FfiPbftVoteEventFactFlags {
        FfiPbftVoteEventFactFlags {
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    fn voter_from_secret(secret: &[u8; 32]) -> [u8; 20] {
        let key = SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = tiny_keccak::Keccak::v256();
        tiny_keccak::Hasher::update(&mut hasher, &public_key.as_bytes()[1..]);
        tiny_keccak::Hasher::finalize(hasher, &mut output);
        output[12..].try_into().unwrap()
    }

    fn generated_vote_rlp() -> Vec<u8> {
        const NODE_SECRET: [u8; 32] = [0x42; 32];
        const VRF_SECRET: [u8; 64] = [
            0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57,
            0xa4, 0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8,
            0xd5, 0x1c, 0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38,
            0xdb, 0x7e, 0x28, 0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4,
            0x02, 0xea, 0x69, 0x97, 0xad, 0xe4, 0x00, 0x81,
        ];
        let generated = pbft_generate_signed_vote(PbftVoteGenerationInput {
            block_hash: [7; 32],
            vote_type: 3,
            period: 12,
            round: 2,
            step: 3,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&NODE_SECRET),
            expected_vrf_public_key: rustaxa_vdf::vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap();
        assert!(generated.accepted);
        generated.vote_rlp.into_iter().collect()
    }

    #[test]
    fn bridge_builds_progress_fact_from_canonical_vote_bytes() {
        let vote_rlp = generated_vote_rlp();

        let event = pbft_vote_event_fact_from_canonical_vote(&vote_rlp, 42, flags()).unwrap();

        assert_eq!(event.status, 0);
        assert!(event.has_progress_fact);
        assert_eq!(event.progress_fact.vote.block_hash, [7; 32]);
        assert_eq!(event.progress_fact.vote.period, 12);
        assert_eq!(event.progress_fact.vote.round, 2);
        assert_eq!(event.progress_fact.vote.step, 3);
        assert_eq!(event.progress_fact.vote.vote_type, 3);
        assert_eq!(event.progress_fact.vote.weight, 42);
    }

    #[test]
    fn bridge_reports_malformed_vote_bytes_as_status() {
        let event = pbft_vote_event_fact_from_canonical_vote(&[0x01, 0x02], 42, flags()).unwrap();

        assert_eq!(event.status, 1);
        assert_eq!(event.error_code, "PBFT_VOTE_EVENT_MALFORMED_RLP");
        assert!(!event.has_progress_fact);
    }

    #[test]
    fn bridge_derives_progress_fact_from_full_validation() {
        let vote_rlp = generated_vote_rlp();
        let vrf_public_key = rustaxa_vdf::vrf::public_key_from_secret(&[
            0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57,
            0xa4, 0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8,
            0xd5, 0x1c, 0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38,
            0xdb, 0x7e, 0x28, 0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4,
            0x02, 0xea, 0x69, 0x97, 0xad, 0xe4, 0x00, 0x81,
        ])
        .unwrap();

        let result = pbft_derive_vote_progress_fact_from_canonical_vote(
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
                has_preverified_weight: false,
                preverified_weight: 0,
            },
            flags(),
        )
        .unwrap();

        assert_eq!(result.status, 0);
        assert_eq!(result.validation.status, 1);
        assert!(result.validation.accepted);
        assert!(result.has_progress_fact);
        assert_eq!(
            result.progress_fact.vote.weight,
            result.validation.calculated_weight
        );
        assert_eq!(
            result.progress_fact.vote.voter,
            result.validation.recovered_voter
        );
    }
}
