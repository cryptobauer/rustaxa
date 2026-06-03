//! PBFT vote event fact construction from canonical vote bytes.
//!
//! This module is the consensus-side boundary between future ingress pipeline
//! events and the existing PBFT vote progress runtime. It does not own network
//! packets, peer state, live C++ `PbftVote` objects, FinalChain lookups, storage
//! writes, or gossip. Callers provide canonical legacy PBFT vote RLP bytes plus
//! the already-calculated vote weight and ingress/validation flags; Rust
//! inspects the bytes and returns compact facts suitable for vote-progress
//! planning.

use crate::pbft_vote_progress::{PbftVoteIdentity, PbftVoteProgressFact};
use crate::pbft_vote_validation::{
    PbftCanonicalVoteInspectionStatus, PbftCanonicalVoteValidation, PbftVoteValidationStatus,
    inspect_canonical_pbft_vote,
};

/// Stable status for constructing PBFT vote event facts.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftVoteEventFactStatus {
    /// Canonical bytes were inspected and compact progress facts were built.
    Ready,
    /// Canonical vote RLP or nested VRF-sortition RLP was malformed.
    MalformedRlp,
    /// Vote signature recovery failed.
    InvalidSignature,
    /// Caller supplied a zero vote weight.
    InvalidWeight,
    /// Canonical validation has not yet accepted or rejected the vote.
    ValidationPending,
    /// Canonical validation rejected the vote.
    ValidationRejected,
    /// Canonical validation accepted without a calculated weight.
    WeightUnavailable,
}

impl PbftVoteEventFactStatus {
    /// Stable numeric status used by bridge payloads.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::MalformedRlp => 1,
            Self::InvalidSignature => 2,
            Self::InvalidWeight => 3,
            Self::ValidationPending => 4,
            Self::ValidationRejected => 5,
            Self::WeightUnavailable => 6,
        }
    }

    /// Stable error code for bridge and log consumers.
    #[must_use]
    pub const fn error_code(self) -> &'static str {
        match self {
            Self::Ready => "",
            Self::MalformedRlp => "PBFT_VOTE_EVENT_MALFORMED_RLP",
            Self::InvalidSignature => "PBFT_VOTE_EVENT_INVALID_SIGNATURE",
            Self::InvalidWeight => "PBFT_VOTE_EVENT_INVALID_WEIGHT",
            Self::ValidationPending => "PBFT_VOTE_EVENT_VALIDATION_PENDING",
            Self::ValidationRejected => "PBFT_VOTE_EVENT_VALIDATION_REJECTED",
            Self::WeightUnavailable => "PBFT_VOTE_EVENT_WEIGHT_UNAVAILABLE",
        }
    }
}

/// Caller-supplied ingress and validation flags for one PBFT vote event.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteEventFactFlags {
    /// Whether the vote hash is already known to the ingress/peer layer.
    pub vote_already_known: bool,
    /// Whether ingress carried or otherwise confirmed the proposed-block sidecar.
    pub carries_proposed_block: bool,
    /// Whether validation accepted this stale vote as an extra reward vote.
    pub valid_stale_reward_vote: bool,
}

/// Result of deriving compact PBFT vote event facts.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteEventFact {
    /// Primary derivation status.
    pub status: PbftVoteEventFactStatus,
    /// Stable error code for bridge/log consumers.
    pub error_code: &'static str,
    /// Compact progress facts when `status == Ready`.
    pub progress_fact: Option<PbftVoteProgressFact>,
}

impl PbftVoteEventFact {
    fn rejected(status: PbftVoteEventFactStatus) -> Self {
        Self::rejected_with_error(status, status.error_code())
    }

    fn rejected_with_error(status: PbftVoteEventFactStatus, error_code: &'static str) -> Self {
        Self {
            status,
            error_code,
            progress_fact: None,
        }
    }
}

/// Builds PBFT vote progress facts from canonical legacy vote RLP.
///
/// Inputs:
/// - `canonical_vote_rlp`: legacy `PbftVote::rlp(true, false)` bytes.
/// - `weight`: already-calculated vote weight from validation.
/// - `flags`: caller-supplied ingress/validation booleans that are not encoded
///   in the vote bytes.
///
/// Outputs:
/// - `Ready` with compact [`PbftVoteProgressFact`] when the canonical bytes can
///   derive the consensus identity and the supplied weight is non-zero.
/// - A stable rejection status for malformed bytes, invalid signature recovery,
///   or invalid weight.
///
/// Invariants and edge behavior:
/// - This function does not validate DPoS eligibility, VRF proof, replay state,
///   proposed-block sidecar availability, or stale-reward membership.
/// - Peer-controlled malformed bytes return statuses, not errors.
pub fn build_pbft_vote_event_fact(
    canonical_vote_rlp: &[u8],
    weight: u64,
    flags: PbftVoteEventFactFlags,
) -> anyhow::Result<PbftVoteEventFact> {
    let inspection = inspect_canonical_pbft_vote(canonical_vote_rlp)?;
    match inspection.status {
        PbftCanonicalVoteInspectionStatus::MalformedRlp => {
            return Ok(PbftVoteEventFact::rejected(
                PbftVoteEventFactStatus::MalformedRlp,
            ));
        }
        PbftCanonicalVoteInspectionStatus::InvalidSignature => {
            return Ok(PbftVoteEventFact::rejected(
                PbftVoteEventFactStatus::InvalidSignature,
            ));
        }
        PbftCanonicalVoteInspectionStatus::Valid => {}
    }

    if weight == 0 {
        return Ok(PbftVoteEventFact::rejected(
            PbftVoteEventFactStatus::InvalidWeight,
        ));
    }

    Ok(PbftVoteEventFact {
        status: PbftVoteEventFactStatus::Ready,
        error_code: "",
        progress_fact: Some(PbftVoteProgressFact {
            identity: PbftVoteIdentity {
                vote_hash: inspection.vote_hash,
                block_hash: inspection.block_hash,
                period: inspection.period,
                round: inspection.round,
                step: inspection.step,
                voter: inspection.recovered_voter,
            },
            vote_type: inspection.vote_type,
            weight,
            vote_already_known: flags.vote_already_known,
            carries_proposed_block: flags.carries_proposed_block,
            valid_stale_reward_vote: flags.valid_stale_reward_vote,
        }),
    })
}

/// Builds PBFT vote progress facts from a canonical validation result.
///
/// Inputs:
/// - `validation`: Rust canonical vote validation output. It must be accepted
///   and include a calculated weight before a progress fact can be produced.
/// - `flags`: caller-supplied ingress/validation booleans that are not encoded
///   in the vote bytes.
///
/// Outputs:
/// - `Ready` with compact progress facts when validation accepted and returned
///   a non-zero calculated weight.
/// - Stable pending/rejected/weight statuses otherwise.
///
/// Invariants and edge behavior:
/// - Embedded RLP weight is never used. The progress fact weight is the
///   Rust-calculated validation weight.
/// - This function does not mutate replay state or verified-vote state.
#[must_use]
pub fn build_pbft_vote_event_fact_from_validation(
    validation: &PbftCanonicalVoteValidation,
    flags: PbftVoteEventFactFlags,
) -> PbftVoteEventFact {
    if validation.status == PbftVoteValidationStatus::Pending {
        return PbftVoteEventFact::rejected_with_error(
            PbftVoteEventFactStatus::ValidationPending,
            validation.error_code,
        );
    }

    if !validation.accepted {
        return PbftVoteEventFact::rejected_with_error(
            PbftVoteEventFactStatus::ValidationRejected,
            validation.error_code,
        );
    }

    if !validation.weight_calculated {
        return PbftVoteEventFact::rejected(PbftVoteEventFactStatus::WeightUnavailable);
    }

    if validation.calculated_weight == 0 {
        return PbftVoteEventFact::rejected(PbftVoteEventFactStatus::InvalidWeight);
    }

    PbftVoteEventFact {
        status: PbftVoteEventFactStatus::Ready,
        error_code: "",
        progress_fact: Some(PbftVoteProgressFact {
            identity: PbftVoteIdentity {
                vote_hash: validation.vote_hash,
                block_hash: validation.block_hash,
                period: validation.period,
                round: validation.round,
                step: validation.step,
                voter: validation.recovered_voter,
            },
            vote_type: validation.vote_type,
            weight: validation.calculated_weight,
            vote_already_known: flags.vote_already_known,
            carries_proposed_block: flags.carries_proposed_block,
            valid_stale_reward_vote: flags.valid_stale_reward_vote,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_vote_generation::{PbftVoteGenerationInput, generate_pbft_vote};
    use crate::pbft_vote_validation::PbftVoteValidationStatus;
    use crate::verified_votes::PbftVoteType;
    use ethereum_types::H160;
    use ethereum_types::H256;
    use k256::ecdsa::SigningKey;
    use rustaxa_vdf::vrf;
    use tiny_keccak::{Hasher, Keccak};

    const NODE_SECRET: [u8; 32] = [0x51; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn keccak256(data: &[u8]) -> H256 {
        let mut output = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(data);
        hasher.finalize(&mut output);
        H256(output)
    }

    fn public_key_from_signing_key(signing_key: &SigningKey) -> [u8; 64] {
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let mut out = [0_u8; 64];
        out.copy_from_slice(&encoded.as_bytes()[1..]);
        out
    }

    fn address_from_public_key(public_key: &[u8; 64]) -> H160 {
        let public_key_hash = keccak256(public_key);
        H160::from_slice(&public_key_hash.as_bytes()[12..])
    }

    fn signed_pbft_vote(block_hash: H256, period: u64, round: u64, step: u64) -> Vec<u8> {
        let signing_key = SigningKey::from_slice(&NODE_SECRET).unwrap();
        let public_key = public_key_from_signing_key(&signing_key);
        let generated = generate_pbft_vote(PbftVoteGenerationInput {
            block_hash,
            vote_type: PbftVoteType::try_from(step as u8).unwrap(),
            period,
            round,
            step,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: address_from_public_key(&public_key),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap();
        assert!(generated.accepted);
        generated.vote_rlp
    }

    const fn flags() -> PbftVoteEventFactFlags {
        PbftVoteEventFactFlags {
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    #[test]
    fn event_fact_uses_canonical_vote_identity_and_supplied_weight() {
        let vote_rlp = signed_pbft_vote(H256::from_low_u64_be(7), 12, 2, 3);

        let event = build_pbft_vote_event_fact(&vote_rlp, 42, flags()).unwrap();

        assert_eq!(event.status, PbftVoteEventFactStatus::Ready);
        let fact = event.progress_fact.unwrap();
        assert_eq!(fact.identity.vote_hash, keccak256(&vote_rlp));
        assert_eq!(fact.identity.block_hash, H256::from_low_u64_be(7));
        assert_eq!(fact.identity.period, 12);
        assert_eq!(fact.identity.round, 2);
        assert_eq!(fact.identity.step, 3);
        assert_eq!(fact.vote_type, PbftVoteType::Cert);
        assert_eq!(fact.weight, 42);
        assert!(fact.carries_proposed_block);
    }

    #[test]
    fn malformed_or_zero_weight_inputs_return_stable_statuses() {
        let malformed = build_pbft_vote_event_fact(&[0x01, 0x02], 1, flags()).unwrap();
        assert_eq!(malformed.status, PbftVoteEventFactStatus::MalformedRlp);
        assert!(malformed.progress_fact.is_none());

        let vote_rlp = signed_pbft_vote(H256::from_low_u64_be(8), 12, 2, 3);
        let zero_weight = build_pbft_vote_event_fact(&vote_rlp, 0, flags()).unwrap();
        assert_eq!(zero_weight.status, PbftVoteEventFactStatus::InvalidWeight);
        assert!(zero_weight.progress_fact.is_none());
    }

    #[test]
    fn validation_output_builds_progress_fact_from_calculated_weight() {
        let vote_rlp = signed_pbft_vote(H256::from_low_u64_be(9), 14, 3, 2);
        let inspected = inspect_canonical_pbft_vote(&vote_rlp).unwrap();
        let validation = PbftCanonicalVoteValidation {
            status: PbftVoteValidationStatus::Valid,
            error_code: "",
            accepted: true,
            rejected: false,
            mark_validated_replay: true,
            vote_hash: inspected.vote_hash,
            signing_hash: inspected.signing_hash,
            block_hash: inspected.block_hash,
            period: inspected.period,
            round: inspected.round,
            step: inspected.step,
            vote_type: inspected.vote_type,
            recovered_voter: inspected.recovered_voter,
            recovered_public_key: inspected.recovered_public_key,
            signature_valid: true,
            vrf_valid: true,
            has_sortition_threshold: true,
            sortition_threshold: 10,
            weight_calculated: true,
            calculated_weight: 33,
            vrf_output: [0; 64],
        };

        let event = build_pbft_vote_event_fact_from_validation(&validation, flags());

        assert_eq!(event.status, PbftVoteEventFactStatus::Ready);
        assert_eq!(event.progress_fact.unwrap().weight, 33);
    }

    #[test]
    fn rejected_validation_does_not_build_progress_fact() {
        let vote_rlp = signed_pbft_vote(H256::from_low_u64_be(10), 14, 3, 2);
        let inspected = inspect_canonical_pbft_vote(&vote_rlp).unwrap();
        let validation = PbftCanonicalVoteValidation {
            status: PbftVoteValidationStatus::ZeroStake,
            error_code: "PBFT_VOTE_VALIDATION_ZERO_STAKE",
            accepted: false,
            rejected: true,
            mark_validated_replay: true,
            vote_hash: inspected.vote_hash,
            signing_hash: inspected.signing_hash,
            block_hash: inspected.block_hash,
            period: inspected.period,
            round: inspected.round,
            step: inspected.step,
            vote_type: inspected.vote_type,
            recovered_voter: inspected.recovered_voter,
            recovered_public_key: inspected.recovered_public_key,
            signature_valid: true,
            vrf_valid: true,
            has_sortition_threshold: false,
            sortition_threshold: 0,
            weight_calculated: false,
            calculated_weight: 0,
            vrf_output: [0; 64],
        };

        let event = build_pbft_vote_event_fact_from_validation(&validation, flags());

        assert_eq!(event.status, PbftVoteEventFactStatus::ValidationRejected);
        assert_eq!(event.error_code, "PBFT_VOTE_VALIDATION_ZERO_STAKE");
        assert!(event.progress_fact.is_none());
    }
}
