//! Deterministic PBFT vote validation planning.
//!
//! This module owns the consensus decision table for validating received PBFT
//! votes and locally generated proposer sortitions. It deliberately does not
//! own live vote objects, cryptographic verification, FinalChain reads,
//! key-manager lookups, replay-cache storage, or vote-weight mutation. Callers
//! supply those facts explicitly and execute any returned side effects at the
//! boundary.
//!
//! The current C++ shim still materializes `PbftVote`, performs signature and
//! VRF proof checks, calculates mutable vote weight, and stores the temporary
//! replay marker. Rust decides when those caller-supplied facts are sufficient
//! to accept or reject the vote and when the replay marker must be written.

use anyhow::{Result, anyhow, bail, ensure};
use ethereum_types::{H160, H256};
use rlp::{Rlp, RlpStream};
use rustaxa_vdf::vrf::{self, VRF_OUTPUT_BYTES, VRF_PROOF_BYTES, VRF_PUBLIC_KEY_BYTES};
use std::collections::{HashSet, VecDeque};
use tiny_keccak::{Hasher, Keccak};

use crate::verified_votes::PbftVoteType;

const SIGNATURE_BYTES: usize = 65;
const RECOVERED_PUBLIC_KEY_BYTES: usize = 64;

/// Peer-data status for canonical PBFT vote byte inspection.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftCanonicalVoteInspectionStatus {
    /// The vote decoded and its signature recovered a voter identity.
    Valid,
    /// The vote RLP or nested VRF-sortition RLP is malformed.
    MalformedRlp,
    /// The vote RLP decoded, but signature recovery failed.
    InvalidSignature,
}

impl PbftCanonicalVoteInspectionStatus {
    /// Stable numeric status used by CXX bridge payloads and tests.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Valid => 0,
            Self::MalformedRlp => 1,
            Self::InvalidSignature => 2,
        }
    }
}

/// Canonical PBFT vote inspection decoded from legacy `PbftVote::rlp(true, false)` bytes.
///
/// The inspection owns no live vote object and performs no FinalChain or
/// KeyManager lookups. It preserves both hashes needed by the legacy vote
/// contract: `signing_hash` is the unsigned hash signed by the voter, while
/// `vote_hash` is the signed hash used as the vote identifier.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftCanonicalVoteInspection {
    /// Decode/signature inspection status.
    pub status: PbftCanonicalVoteInspectionStatus,
    /// Stable error code for bridge/log consumers.
    pub error_code: &'static str,
    /// Hash of canonical signed PBFT vote bytes.
    pub vote_hash: H256,
    /// Hash of unsigned `[block_hash, vrf_sortition_rlp]` bytes used for signing.
    pub signing_hash: H256,
    /// Voted PBFT block hash.
    pub block_hash: H256,
    /// PBFT period embedded in the VRF-sortition message.
    pub period: u64,
    /// PBFT round embedded in the VRF-sortition message.
    pub round: u64,
    /// PBFT step embedded in the VRF-sortition message.
    pub step: u64,
    /// Vote type derived from `step`.
    pub vote_type: PbftVoteType,
    /// Recovered 64-byte uncompressed secp256k1 public key without the `0x04` prefix.
    pub recovered_public_key: [u8; RECOVERED_PUBLIC_KEY_BYTES],
    /// Recovered voter address derived from the public key.
    pub recovered_voter: H160,
    /// Whether signature recovery succeeded.
    pub signature_valid: bool,
    /// VRF proof embedded in the sortition payload.
    pub vrf_proof: [u8; VRF_PROOF_BYTES],
    /// Whether a legacy persisted vote supplied an embedded weight.
    pub has_embedded_weight: bool,
    /// Embedded weight when present; ignored by validation.
    pub embedded_weight: u64,
}

/// External state facts needed to validate an inspected canonical PBFT vote.
///
/// C++ supplies these facts after reading FinalChain and KeyManager. Rust uses
/// them together with the canonical vote bytes to verify VRF proof output,
/// compute the sortition threshold, and calculate the authoritative validation
/// weight.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteValidationExternalFacts {
    /// Whether voter DPoS lookup completed without a future-state error.
    pub voter_dpos_ready: bool,
    /// Eligible DPoS vote count for the recovered voter.
    pub voter_dpos_vote_count: u64,
    /// Whether total DPoS lookup completed without a future-state error.
    pub total_dpos_ready: bool,
    /// Total eligible DPoS vote count.
    pub total_dpos_vote_count: u64,
    /// True when FinalChain reported state behind the vote period.
    pub future_dpos_state: bool,
    /// True when a non-future lookup or bridge invariant failed.
    pub unknown_error: bool,
    /// Whether KeyManager lookup completed.
    pub vrf_key_ready: bool,
    /// Whether KeyManager returned a VRF public key.
    pub has_vrf_key: bool,
    /// VRF public key bytes returned for the recovered voter.
    pub vrf_public_key: [u8; VRF_PUBLIC_KEY_BYTES],
    /// Whether strict VRF verification is required.
    pub strict_vrf: bool,
    /// PBFT committee size used for soft/cert/next vote sortition.
    pub committee_size: u64,
    /// Proposal committee size used for proposal vote sortition.
    pub number_of_proposers: u64,
}

/// Complete Rust validation result for one canonical PBFT vote.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftCanonicalVoteValidation {
    /// Existing validation status, extended with inspection failures.
    pub status: PbftVoteValidationStatus,
    /// Stable error code for bridge/log consumers.
    pub error_code: &'static str,
    /// Whether validation accepted the vote.
    pub accepted: bool,
    /// Whether validation rejected the vote.
    pub rejected: bool,
    /// Whether the replay marker should be inserted for this vote hash.
    pub mark_validated_replay: bool,
    /// Hash of canonical signed PBFT vote bytes.
    pub vote_hash: H256,
    /// Hash of unsigned bytes used for signature recovery.
    pub signing_hash: H256,
    /// Voted PBFT block hash.
    pub block_hash: H256,
    /// PBFT period.
    pub period: u64,
    /// PBFT round.
    pub round: u64,
    /// PBFT step.
    pub step: u64,
    /// Vote type derived from step.
    pub vote_type: PbftVoteType,
    /// Recovered voter address.
    pub recovered_voter: H160,
    /// Recovered 64-byte public key.
    pub recovered_public_key: [u8; RECOVERED_PUBLIC_KEY_BYTES],
    /// Whether signature recovery succeeded.
    pub signature_valid: bool,
    /// Whether VRF verification/proof hashing completed and was valid.
    pub vrf_valid: bool,
    /// Whether a sortition threshold was calculated.
    pub has_sortition_threshold: bool,
    /// Sortition threshold used for weight calculation.
    pub sortition_threshold: u64,
    /// Whether a vote weight was calculated.
    pub weight_calculated: bool,
    /// Rust-computed PBFT vote weight.
    pub calculated_weight: u64,
    /// VRF output used for weight calculation.
    pub vrf_output: [u8; VRF_OUTPUT_BYTES],
}

/// Fixed-capacity replay cache for PBFT vote validation.
///
/// The cache mirrors the legacy insertion/eviction shape: inserting an
/// existing hash is idempotent, new hashes are appended to FIFO expiration
/// order, and crossing `max_size` removes `delete_step` oldest hashes. It owns
/// only vote hashes; callers remain responsible for deciding when validation
/// has progressed far enough to insert.
#[derive(Debug, Clone)]
pub struct PbftVoteReplayCache {
    max_size: usize,
    delete_step: usize,
    hashes: HashSet<H256>,
    expiration: VecDeque<H256>,
}

impl PbftVoteReplayCache {
    /// Creates an empty replay cache with legacy-compatible capacity controls.
    ///
    /// `delete_step` values of zero are normalized to one so eviction always
    /// makes progress if a caller provides a malformed configuration.
    #[must_use]
    pub fn new(max_size: usize, delete_step: usize) -> Self {
        Self {
            max_size,
            delete_step: delete_step.max(1),
            hashes: HashSet::new(),
            expiration: VecDeque::new(),
        }
    }

    /// Returns whether `vote_hash` is already present in replay protection.
    #[must_use]
    pub fn contains(&self, vote_hash: H256) -> bool {
        self.hashes.contains(&vote_hash)
    }

    /// Inserts `vote_hash` and returns whether it was newly inserted.
    ///
    /// Edge behavior:
    /// - Duplicate inserts return false and do not refresh expiration order.
    /// - If `max_size` is zero, the inserted hash is immediately evicted.
    pub fn insert(&mut self, vote_hash: H256) -> bool {
        if !self.hashes.insert(vote_hash) {
            return false;
        }

        self.expiration.push_back(vote_hash);
        if self.hashes.len() > self.max_size {
            for _ in 0..self.delete_step {
                let Some(expired) = self.expiration.pop_front() else {
                    break;
                };
                self.hashes.remove(&expired);
                if self.hashes.len() <= self.max_size {
                    break;
                }
            }
        }
        true
    }

    /// Returns the number of hashes currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    /// Returns true when no hashes are currently retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }
}

/// Deterministic status for one received PBFT vote validation plan.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftVoteValidationStatus {
    /// More caller facts are required before the vote can be accepted or rejected.
    Pending,
    /// All supplied facts are valid and the vote is accepted.
    Valid,
    /// FinalChain reported zero eligible stake for the voter.
    ZeroStake,
    /// KeyManager has no VRF public key for the voter at the vote period.
    MissingVrfKey,
    /// Vote signature verification failed.
    InvalidSignature,
    /// VRF proof verification failed.
    InvalidVrfProof,
    /// Weight calculation completed but returned zero.
    ZeroWeight,
    /// FinalChain state is behind the vote period and the vote must not be cached.
    FutureDposState,
    /// The caller reported an unexpected validation failure.
    UnknownError,
    /// The vote type is not a valid PBFT validation target.
    InvalidVoteType,
}

impl PbftVoteValidationStatus {
    /// Stable numeric status used by CXX bridge payloads and tests.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Valid => 1,
            Self::ZeroStake => 2,
            Self::MissingVrfKey => 3,
            Self::InvalidSignature => 4,
            Self::InvalidVrfProof => 5,
            Self::ZeroWeight => 6,
            Self::FutureDposState => 7,
            Self::UnknownError => 8,
            Self::InvalidVoteType => 9,
        }
    }
}

/// Caller-supplied fact bundle for one received PBFT vote validation pass.
///
/// Each `*_ready` flag distinguishes facts not collected yet from collected
/// facts whose value is false or zero. This lets C++ preserve the legacy
/// validation order while Rust owns the decision at every checkpoint.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteValidationFact {
    /// Legacy PBFT vote type.
    pub vote_type: PbftVoteType,
    /// Whether FinalChain voter-count lookup completed.
    pub dpos_vote_count_ready: bool,
    /// DPoS eligible vote count for the voter when ready.
    pub dpos_vote_count: u64,
    /// Whether VRF key lookup completed.
    pub vrf_key_ready: bool,
    /// Whether a VRF key exists for the voter.
    pub has_vrf_key: bool,
    /// Whether signature verification completed.
    pub signature_ready: bool,
    /// Result of signature verification.
    pub signature_valid: bool,
    /// Whether VRF proof verification completed.
    pub vrf_sortition_ready: bool,
    /// Result of VRF proof verification.
    pub vrf_sortition_valid: bool,
    /// Whether total DPoS vote-count lookup completed.
    pub total_dpos_vote_count_ready: bool,
    /// Total DPoS eligible vote count when ready.
    pub total_dpos_vote_count: u64,
    /// Whether vote-weight calculation completed.
    pub weight_ready: bool,
    /// Calculated vote weight when ready.
    pub weight: u64,
    /// True when FinalChain state is behind the requested vote period.
    pub future_dpos_state: bool,
    /// True when the caller caught an unexpected validation failure.
    pub unknown_error: bool,
    /// PBFT committee size used for soft/cert/next vote sortition.
    pub committee_size: u64,
    /// Proposer committee size used for proposal vote sortition.
    pub number_of_proposers: u64,
}

/// Deterministic validation plan for one received PBFT vote.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteValidationPlan {
    /// Primary validation status.
    pub status: PbftVoteValidationStatus,
    /// Whether validation is complete and accepted.
    pub accepted: bool,
    /// Whether validation is complete and rejected.
    pub rejected: bool,
    /// Whether the caller must write the replay marker for this vote hash.
    pub mark_validated_replay: bool,
    /// Whether a sortition threshold has been computed.
    pub has_sortition_threshold: bool,
    /// Sortition threshold to use for weight calculation when present.
    pub sortition_threshold: u64,
}

impl PbftVoteValidationPlan {
    fn pending(mark_validated_replay: bool, threshold: Option<u64>) -> Self {
        Self {
            status: PbftVoteValidationStatus::Pending,
            accepted: false,
            rejected: false,
            mark_validated_replay,
            has_sortition_threshold: threshold.is_some(),
            sortition_threshold: threshold.unwrap_or_default(),
        }
    }

    fn rejected(status: PbftVoteValidationStatus, mark_validated_replay: bool) -> Self {
        Self {
            status,
            accepted: false,
            rejected: true,
            mark_validated_replay,
            has_sortition_threshold: false,
            sortition_threshold: 0,
        }
    }

    fn accepted(threshold: Option<u64>) -> Self {
        Self {
            status: PbftVoteValidationStatus::Valid,
            accepted: true,
            rejected: false,
            mark_validated_replay: true,
            has_sortition_threshold: threshold.is_some(),
            sortition_threshold: threshold.unwrap_or_default(),
        }
    }
}

/// Deterministic status for locally generated proposer sortition screening.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftProposerSortitionStatus {
    /// More caller facts are required before the sortition can be accepted or rejected.
    Pending,
    /// The generated sortition has eligible stake and non-zero weight.
    Valid,
    /// The local proposer has zero eligible stake.
    ZeroStake,
    /// Sortition weight calculation completed but returned zero.
    ZeroWeight,
    /// FinalChain state is behind the requested proposer period.
    FutureDposState,
    /// The caller reported an unexpected sortition failure.
    UnknownError,
}

impl PbftProposerSortitionStatus {
    /// Stable numeric status used by CXX bridge payloads and tests.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Valid => 1,
            Self::ZeroStake => 2,
            Self::ZeroWeight => 3,
            Self::FutureDposState => 4,
            Self::UnknownError => 5,
        }
    }
}

/// Caller-supplied facts for locally generated proposer sortition screening.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftProposerSortitionFact {
    /// Whether FinalChain voter-count lookup completed.
    pub dpos_vote_count_ready: bool,
    /// DPoS eligible vote count for the local node when ready.
    pub dpos_vote_count: u64,
    /// Whether total DPoS vote-count lookup completed.
    pub total_dpos_vote_count_ready: bool,
    /// Total DPoS eligible vote count when ready.
    pub total_dpos_vote_count: u64,
    /// Whether sortition weight calculation completed.
    pub weight_ready: bool,
    /// Calculated proposer sortition weight when ready.
    pub weight: u64,
    /// True when FinalChain state is behind the requested proposer period.
    pub future_dpos_state: bool,
    /// True when the caller caught an unexpected proposer-sortition failure.
    pub unknown_error: bool,
    /// Proposer committee size used for proposal vote sortition.
    pub number_of_proposers: u64,
}

/// Deterministic screening plan for one local proposer sortition.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftProposerSortitionPlan {
    /// Primary screening status.
    pub status: PbftProposerSortitionStatus,
    /// Whether the local proposer sortition is accepted.
    pub accepted: bool,
    /// Whether screening is complete and rejected.
    pub rejected: bool,
    /// Whether a proposer threshold has been computed.
    pub has_sortition_threshold: bool,
    /// Proposer threshold to use for weight calculation when present.
    pub sortition_threshold: u64,
}

impl PbftProposerSortitionPlan {
    fn pending(threshold: Option<u64>) -> Self {
        Self {
            status: PbftProposerSortitionStatus::Pending,
            accepted: false,
            rejected: false,
            has_sortition_threshold: threshold.is_some(),
            sortition_threshold: threshold.unwrap_or_default(),
        }
    }

    fn rejected(status: PbftProposerSortitionStatus) -> Self {
        Self {
            status,
            accepted: false,
            rejected: true,
            has_sortition_threshold: false,
            sortition_threshold: 0,
        }
    }

    fn accepted(threshold: Option<u64>) -> Self {
        Self {
            status: PbftProposerSortitionStatus::Valid,
            accepted: true,
            rejected: false,
            has_sortition_threshold: threshold.is_some(),
            sortition_threshold: threshold.unwrap_or_default(),
        }
    }
}

/// Computes the PBFT sortition threshold for a vote type and total DPoS votes.
///
/// Proposal votes use `number_of_proposers`; soft, cert, and next votes use
/// `committee_size`. In all cases the value is clamped by
/// `total_dpos_vote_count`, matching the legacy consensus rule.
pub fn pbft_vote_sortition_threshold(
    total_dpos_vote_count: u64,
    vote_type: PbftVoteType,
    committee_size: u64,
    number_of_proposers: u64,
) -> Result<u64> {
    if matches!(vote_type, PbftVoteType::Invalid) {
        return Err(anyhow!("invalid PBFT vote type for sortition threshold"));
    }

    let target = match vote_type {
        PbftVoteType::Propose => number_of_proposers,
        PbftVoteType::Soft | PbftVoteType::Cert | PbftVoteType::Next => committee_size,
        PbftVoteType::Invalid => unreachable!(),
    };
    Ok(target.min(total_dpos_vote_count))
}

/// Inspects canonical legacy PBFT vote bytes without external state access.
///
/// Malformed peer-controlled payloads and invalid signatures are returned as
/// inspection statuses rather than errors. The only `Err` cases are internal
/// invariants that would indicate a code bug in the inspection implementation.
pub fn inspect_canonical_pbft_vote(vote_rlp: &[u8]) -> Result<PbftCanonicalVoteInspection> {
    match decode_canonical_pbft_vote(vote_rlp) {
        Ok(decoded) => Ok(decoded),
        Err(_) => Ok(PbftCanonicalVoteInspection {
            status: PbftCanonicalVoteInspectionStatus::MalformedRlp,
            error_code: "PBFT_CANONICAL_VOTE_MALFORMED_RLP",
            vote_hash: H256::zero(),
            signing_hash: H256::zero(),
            block_hash: H256::zero(),
            period: 0,
            round: 0,
            step: 0,
            vote_type: PbftVoteType::Invalid,
            recovered_public_key: [0; RECOVERED_PUBLIC_KEY_BYTES],
            recovered_voter: H160::zero(),
            signature_valid: false,
            vrf_proof: [0; VRF_PROOF_BYTES],
            has_embedded_weight: false,
            embedded_weight: 0,
        }),
    }
}

/// Validates canonical legacy PBFT vote bytes from external state facts.
///
/// This function composes byte inspection, VRF proof verification, threshold
/// selection, weight calculation, and the existing PBFT vote validation planner.
/// It does not read node state or mutate live vote objects.
pub fn validate_canonical_pbft_vote(
    vote_rlp: &[u8],
    facts: PbftVoteValidationExternalFacts,
) -> Result<PbftCanonicalVoteValidation> {
    let inspection = inspect_canonical_pbft_vote(vote_rlp)?;
    if inspection.status == PbftCanonicalVoteInspectionStatus::MalformedRlp {
        return Ok(validation_from_inspection(
            inspection,
            PbftVoteValidationStatus::InvalidVoteType,
            "PBFT_CANONICAL_VOTE_MALFORMED_RLP",
            false,
            false,
            0,
            false,
            0,
            [0; VRF_OUTPUT_BYTES],
        ));
    }
    if inspection.status == PbftCanonicalVoteInspectionStatus::InvalidSignature {
        return Ok(validation_from_inspection(
            inspection,
            PbftVoteValidationStatus::InvalidSignature,
            "PBFT_VOTE_VALIDATION_INVALID_SIGNATURE",
            false,
            false,
            0,
            false,
            0,
            [0; VRF_OUTPUT_BYTES],
        ));
    }

    let mut fact = PbftVoteValidationFact {
        vote_type: inspection.vote_type,
        dpos_vote_count_ready: facts.voter_dpos_ready,
        dpos_vote_count: facts.voter_dpos_vote_count,
        vrf_key_ready: facts.vrf_key_ready,
        has_vrf_key: facts.has_vrf_key,
        signature_ready: true,
        signature_valid: inspection.signature_valid,
        vrf_sortition_ready: false,
        vrf_sortition_valid: false,
        total_dpos_vote_count_ready: facts.total_dpos_ready,
        total_dpos_vote_count: facts.total_dpos_vote_count,
        weight_ready: false,
        weight: 0,
        future_dpos_state: facts.future_dpos_state,
        unknown_error: facts.unknown_error,
        committee_size: facts.committee_size,
        number_of_proposers: facts.number_of_proposers,
    };

    let mut plan = plan_pbft_vote_validation(fact);
    if plan.rejected || !facts.vrf_key_ready || !facts.has_vrf_key {
        return Ok(validation_from_plan(
            inspection,
            plan,
            vote_validation_error_code(plan.status),
            false,
            false,
            0,
            false,
            0,
            [0; VRF_OUTPUT_BYTES],
        ));
    }

    let vrf_message = legacy_vrf_message_rlp(inspection.period, inspection.round, inspection.step);
    let vrf_output = verify_pbft_vrf_output(
        &facts.vrf_public_key,
        &inspection.vrf_proof,
        &vrf_message,
        facts.strict_vrf,
    )?;
    fact.vrf_sortition_ready = true;
    fact.vrf_sortition_valid = vrf_output.is_some();
    plan = plan_pbft_vote_validation(fact);
    let Some(vrf_output) = vrf_output else {
        return Ok(validation_from_plan(
            inspection,
            plan,
            vote_validation_error_code(plan.status),
            true,
            false,
            0,
            false,
            0,
            [0; VRF_OUTPUT_BYTES],
        ));
    };

    if plan.rejected || !facts.total_dpos_ready {
        return Ok(validation_from_plan(
            inspection,
            plan,
            vote_validation_error_code(plan.status),
            true,
            false,
            0,
            false,
            0,
            vrf_output,
        ));
    }
    if !plan.has_sortition_threshold {
        return Ok(validation_from_plan(
            inspection,
            plan,
            "PBFT_VOTE_VALIDATION_MISSING_SORTITION_THRESHOLD",
            true,
            false,
            0,
            false,
            0,
            vrf_output,
        ));
    }

    let weight = calculate_pbft_vote_weight(
        facts.voter_dpos_vote_count,
        facts.total_dpos_vote_count,
        plan.sortition_threshold,
        &vrf_output,
        &inspection.recovered_public_key,
    )?;
    fact.weight_ready = true;
    fact.weight = weight;
    plan = plan_pbft_vote_validation(fact);

    Ok(validation_from_plan(
        inspection,
        plan,
        vote_validation_error_code(plan.status),
        true,
        true,
        plan.sortition_threshold,
        true,
        weight,
        vrf_output,
    ))
}

fn decode_canonical_pbft_vote(vote_rlp: &[u8]) -> Result<PbftCanonicalVoteInspection> {
    let vote = Rlp::new(vote_rlp);
    let item_count = vote.item_count()?;
    ensure!(
        item_count == 3 || item_count == 4,
        "PBFT vote RLP must contain block_hash, vrf_sortition, signature and optional weight"
    );

    let block_hash: H256 = vote.val_at(0)?;
    let vrf_sortition_rlp = vote.val_at::<Vec<u8>>(1)?;
    let signature = vote.val_at::<Vec<u8>>(2)?;
    ensure!(
        signature.len() == SIGNATURE_BYTES,
        "PBFT vote signature must be exactly {SIGNATURE_BYTES} bytes"
    );
    let signature: [u8; SIGNATURE_BYTES] = signature
        .try_into()
        .map_err(|_| anyhow!("PBFT vote signature length checked above"))?;

    let sortition = decode_legacy_vrf_sortition(&vrf_sortition_rlp)?;
    let signing_hash = legacy_pbft_vote_signing_hash(block_hash, &vrf_sortition_rlp);
    let vote_hash = legacy_pbft_vote_signed_hash(block_hash, &vrf_sortition_rlp, &signature);
    let embedded_weight = if item_count == 4 {
        Some(vote.val_at(3)?)
    } else {
        None
    };

    let Some((recovered_public_key, recovered_voter)) =
        recover_vote_public_key_and_address(&signing_hash, &signature)
    else {
        return Ok(PbftCanonicalVoteInspection {
            status: PbftCanonicalVoteInspectionStatus::InvalidSignature,
            error_code: "PBFT_CANONICAL_VOTE_INVALID_SIGNATURE",
            vote_hash,
            signing_hash,
            block_hash,
            period: sortition.period,
            round: sortition.round,
            step: sortition.step,
            vote_type: sortition.vote_type,
            recovered_public_key: [0; RECOVERED_PUBLIC_KEY_BYTES],
            recovered_voter: H160::zero(),
            signature_valid: false,
            vrf_proof: sortition.proof,
            has_embedded_weight: embedded_weight.is_some(),
            embedded_weight: embedded_weight.unwrap_or_default(),
        });
    };

    Ok(PbftCanonicalVoteInspection {
        status: PbftCanonicalVoteInspectionStatus::Valid,
        error_code: "",
        vote_hash,
        signing_hash,
        block_hash,
        period: sortition.period,
        round: sortition.round,
        step: sortition.step,
        vote_type: sortition.vote_type,
        recovered_public_key,
        recovered_voter,
        signature_valid: true,
        vrf_proof: sortition.proof,
        has_embedded_weight: embedded_weight.is_some(),
        embedded_weight: embedded_weight.unwrap_or_default(),
    })
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct LegacyVrfSortition {
    period: u64,
    round: u64,
    step: u64,
    vote_type: PbftVoteType,
    proof: [u8; VRF_PROOF_BYTES],
}

fn decode_legacy_vrf_sortition(vrf_sortition_rlp: &[u8]) -> Result<LegacyVrfSortition> {
    let sortition = Rlp::new(vrf_sortition_rlp);
    ensure!(
        sortition.item_count()? == 4,
        "PBFT VRF sortition RLP must contain period, round, step and proof"
    );

    let period = sortition.val_at(0)?;
    let round = sortition.val_at(1)?;
    let step = sortition.val_at(2)?;
    let proof = sortition.val_at::<Vec<u8>>(3)?;
    ensure!(
        proof.len() == VRF_PROOF_BYTES,
        "PBFT VRF proof must be exactly {VRF_PROOF_BYTES} bytes"
    );

    Ok(LegacyVrfSortition {
        period,
        round,
        step,
        vote_type: pbft_vote_type_from_step(step),
        proof: proof
            .try_into()
            .map_err(|_| anyhow!("PBFT VRF proof length checked above"))?,
    })
}

fn legacy_pbft_vote_signing_hash(block_hash: H256, vrf_sortition_rlp: &[u8]) -> H256 {
    let mut stream = RlpStream::new_list(2);
    stream.append(&block_hash);
    stream.append(&vrf_sortition_rlp);
    keccak256(&stream.out())
}

fn legacy_pbft_vote_signed_hash(
    block_hash: H256,
    vrf_sortition_rlp: &[u8],
    signature: &[u8; SIGNATURE_BYTES],
) -> H256 {
    let mut stream = RlpStream::new_list(3);
    stream.append(&block_hash);
    stream.append(&vrf_sortition_rlp);
    stream.append(&signature.as_slice());
    keccak256(&stream.out())
}

fn legacy_vrf_message_rlp(period: u64, round: u64, step: u64) -> Vec<u8> {
    let mut stream = RlpStream::new_list(3);
    stream.append(&period);
    stream.append(&round);
    stream.append(&step);
    stream.out().to_vec()
}

fn recover_vote_public_key_and_address(
    signing_hash: &H256,
    signature: &[u8; SIGNATURE_BYTES],
) -> Option<([u8; RECOVERED_PUBLIC_KEY_BYTES], H160)> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let recovery_id = RecoveryId::try_from(signature[64]).ok()?;
    let signature = Signature::try_from(&signature[..SIGNATURE_BYTES - 1]).ok()?;
    let recovered_key =
        VerifyingKey::recover_from_prehash(signing_hash.as_bytes(), &signature, recovery_id)
            .ok()?;
    let uncompressed = recovered_key.to_encoded_point(false);
    let public_key_bytes = &uncompressed.as_bytes()[1..];
    let mut recovered_public_key = [0_u8; RECOVERED_PUBLIC_KEY_BYTES];
    recovered_public_key.copy_from_slice(public_key_bytes);
    let public_key_hash = keccak256(public_key_bytes);
    let recovered_voter = H160::from_slice(&public_key_hash.as_bytes()[12..]);
    Some((recovered_public_key, recovered_voter))
}

fn verify_pbft_vrf_output(
    public_key: &[u8; VRF_PUBLIC_KEY_BYTES],
    proof: &[u8; VRF_PROOF_BYTES],
    message: &[u8],
    strict: bool,
) -> Result<Option<[u8; VRF_OUTPUT_BYTES]>> {
    if strict {
        vrf::verify_output(public_key, proof, message)
    } else {
        Ok(Some(vrf::proof_to_hash(proof)?))
    }
}

fn calculate_pbft_vote_weight(
    stake: u64,
    total_dpos_vote_count: u64,
    threshold: u64,
    vrf_output: &[u8; VRF_OUTPUT_BYTES],
    recovered_public_key: &[u8; RECOVERED_PUBLIC_KEY_BYTES],
) -> Result<u64> {
    if stake == 0 {
        return Ok(0);
    }
    if total_dpos_vote_count == 0 {
        bail!("PBFT vote weight cannot be calculated with zero total DPoS vote count");
    }

    let voter_index_hash = voter_index_hash(vrf_output, recovered_public_key, 0);
    Ok(binomial_weight(
        stake,
        total_dpos_vote_count,
        threshold,
        &voter_index_hash,
    ))
}

fn voter_index_hash(
    vrf_output: &[u8; VRF_OUTPUT_BYTES],
    recovered_public_key: &[u8; RECOVERED_PUBLIC_KEY_BYTES],
    index: u64,
) -> H256 {
    let mut stream = RlpStream::new_list(3);
    stream.append(&vrf_output.as_slice());
    stream.append(&recovered_public_key.as_slice());
    stream.append(&index);
    keccak256(&stream.out())
}

fn binomial_weight(stake: u64, total_dpos_vote_count: u64, threshold: u64, hash: &H256) -> u64 {
    if stake == 0 {
        return 0;
    }

    let probability = threshold as f64 / total_dpos_vote_count as f64;
    if probability <= 0.0 {
        return 0;
    }
    if probability >= 1.0 {
        return stake;
    }

    let ratio = hash_ratio(hash.as_bytes());
    let mut probability_mass = (1.0 - probability).powf(stake as f64);
    let mut cumulative = probability_mass;
    for j in 0..stake {
        if ratio <= cumulative {
            return j;
        }
        let next = j + 1;
        probability_mass *=
            ((stake - j) as f64 / next as f64) * (probability / (1.0 - probability));
        cumulative += probability_mass;
    }

    stake
}

fn hash_ratio(hash: &[u8]) -> f64 {
    let mut ratio = 0.0;
    let mut factor = 1.0 / 256.0;
    for byte in hash {
        ratio += f64::from(*byte) * factor;
        factor /= 256.0;
    }
    ratio
}

const fn pbft_vote_type_from_step(step: u64) -> PbftVoteType {
    match step {
        0 => PbftVoteType::Invalid,
        1 => PbftVoteType::Propose,
        2 => PbftVoteType::Soft,
        3 => PbftVoteType::Cert,
        _ => PbftVoteType::Next,
    }
}

fn validation_from_inspection(
    inspection: PbftCanonicalVoteInspection,
    status: PbftVoteValidationStatus,
    error_code: &'static str,
    vrf_valid: bool,
    has_sortition_threshold: bool,
    sortition_threshold: u64,
    weight_calculated: bool,
    calculated_weight: u64,
    vrf_output: [u8; VRF_OUTPUT_BYTES],
) -> PbftCanonicalVoteValidation {
    let rejected = !matches!(
        status,
        PbftVoteValidationStatus::Pending | PbftVoteValidationStatus::Valid
    );
    PbftCanonicalVoteValidation {
        status,
        error_code,
        accepted: status == PbftVoteValidationStatus::Valid,
        rejected,
        mark_validated_replay: status != PbftVoteValidationStatus::InvalidVoteType,
        vote_hash: inspection.vote_hash,
        signing_hash: inspection.signing_hash,
        block_hash: inspection.block_hash,
        period: inspection.period,
        round: inspection.round,
        step: inspection.step,
        vote_type: inspection.vote_type,
        recovered_voter: inspection.recovered_voter,
        recovered_public_key: inspection.recovered_public_key,
        signature_valid: inspection.signature_valid,
        vrf_valid,
        has_sortition_threshold,
        sortition_threshold,
        weight_calculated,
        calculated_weight,
        vrf_output,
    }
}

fn validation_from_plan(
    inspection: PbftCanonicalVoteInspection,
    plan: PbftVoteValidationPlan,
    error_code: &'static str,
    vrf_valid: bool,
    has_sortition_threshold: bool,
    sortition_threshold: u64,
    weight_calculated: bool,
    calculated_weight: u64,
    vrf_output: [u8; VRF_OUTPUT_BYTES],
) -> PbftCanonicalVoteValidation {
    PbftCanonicalVoteValidation {
        status: plan.status,
        error_code,
        accepted: plan.accepted,
        rejected: plan.rejected,
        mark_validated_replay: plan.mark_validated_replay,
        vote_hash: inspection.vote_hash,
        signing_hash: inspection.signing_hash,
        block_hash: inspection.block_hash,
        period: inspection.period,
        round: inspection.round,
        step: inspection.step,
        vote_type: inspection.vote_type,
        recovered_voter: inspection.recovered_voter,
        recovered_public_key: inspection.recovered_public_key,
        signature_valid: inspection.signature_valid,
        vrf_valid,
        has_sortition_threshold: has_sortition_threshold || plan.has_sortition_threshold,
        sortition_threshold: if has_sortition_threshold {
            sortition_threshold
        } else {
            plan.sortition_threshold
        },
        weight_calculated,
        calculated_weight,
        vrf_output,
    }
}

const fn vote_validation_error_code(status: PbftVoteValidationStatus) -> &'static str {
    match status {
        PbftVoteValidationStatus::Pending | PbftVoteValidationStatus::Valid => "",
        PbftVoteValidationStatus::ZeroStake => "PBFT_VOTE_VALIDATION_ZERO_STAKE",
        PbftVoteValidationStatus::MissingVrfKey => "PBFT_VOTE_VALIDATION_MISSING_VRF_KEY",
        PbftVoteValidationStatus::InvalidSignature => "PBFT_VOTE_VALIDATION_INVALID_SIGNATURE",
        PbftVoteValidationStatus::InvalidVrfProof => "PBFT_VOTE_VALIDATION_INVALID_VRF_PROOF",
        PbftVoteValidationStatus::ZeroWeight => "PBFT_VOTE_VALIDATION_ZERO_WEIGHT",
        PbftVoteValidationStatus::FutureDposState => "PBFT_VOTE_VALIDATION_FUTURE_DPOS_STATE",
        PbftVoteValidationStatus::UnknownError => "PBFT_VOTE_VALIDATION_UNKNOWN_ERROR",
        PbftVoteValidationStatus::InvalidVoteType => "PBFT_VOTE_VALIDATION_INVALID_VOTE_TYPE",
    }
}

fn keccak256(data: &[u8]) -> H256 {
    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut output);
    H256::from(output)
}

/// Plans validation for one received PBFT vote from explicit caller facts.
#[must_use]
pub fn plan_pbft_vote_validation(fact: PbftVoteValidationFact) -> PbftVoteValidationPlan {
    if matches!(fact.vote_type, PbftVoteType::Invalid) {
        return PbftVoteValidationPlan::rejected(PbftVoteValidationStatus::InvalidVoteType, false);
    }
    if fact.future_dpos_state {
        return PbftVoteValidationPlan::rejected(PbftVoteValidationStatus::FutureDposState, false);
    }
    if fact.unknown_error {
        return PbftVoteValidationPlan::rejected(PbftVoteValidationStatus::UnknownError, false);
    }
    if !fact.dpos_vote_count_ready {
        return PbftVoteValidationPlan::pending(false, None);
    }

    let threshold = if fact.total_dpos_vote_count_ready {
        pbft_vote_sortition_threshold(
            fact.total_dpos_vote_count,
            fact.vote_type,
            fact.committee_size,
            fact.number_of_proposers,
        )
        .ok()
    } else {
        None
    };

    if fact.dpos_vote_count == 0 {
        return PbftVoteValidationPlan::rejected(PbftVoteValidationStatus::ZeroStake, true);
    }
    if !fact.vrf_key_ready {
        return PbftVoteValidationPlan::pending(true, threshold);
    }
    if !fact.has_vrf_key {
        return PbftVoteValidationPlan::rejected(PbftVoteValidationStatus::MissingVrfKey, true);
    }
    if !fact.signature_ready {
        return PbftVoteValidationPlan::pending(true, threshold);
    }
    if !fact.signature_valid {
        return PbftVoteValidationPlan::rejected(PbftVoteValidationStatus::InvalidSignature, true);
    }
    if !fact.vrf_sortition_ready {
        return PbftVoteValidationPlan::pending(true, threshold);
    }
    if !fact.vrf_sortition_valid {
        return PbftVoteValidationPlan::rejected(PbftVoteValidationStatus::InvalidVrfProof, true);
    }
    if !fact.total_dpos_vote_count_ready || !fact.weight_ready {
        return PbftVoteValidationPlan::pending(true, threshold);
    }
    if fact.weight == 0 {
        return PbftVoteValidationPlan::rejected(PbftVoteValidationStatus::ZeroWeight, true);
    }

    PbftVoteValidationPlan::accepted(threshold)
}

/// Plans screening for one locally generated proposer sortition.
#[must_use]
pub fn plan_pbft_proposer_sortition(fact: PbftProposerSortitionFact) -> PbftProposerSortitionPlan {
    if fact.future_dpos_state {
        return PbftProposerSortitionPlan::rejected(PbftProposerSortitionStatus::FutureDposState);
    }
    if fact.unknown_error {
        return PbftProposerSortitionPlan::rejected(PbftProposerSortitionStatus::UnknownError);
    }
    if !fact.dpos_vote_count_ready {
        return PbftProposerSortitionPlan::pending(None);
    }
    if fact.dpos_vote_count == 0 {
        return PbftProposerSortitionPlan::rejected(PbftProposerSortitionStatus::ZeroStake);
    }

    let threshold = fact
        .total_dpos_vote_count_ready
        .then_some(fact.number_of_proposers.min(fact.total_dpos_vote_count));
    if !fact.total_dpos_vote_count_ready || !fact.weight_ready {
        return PbftProposerSortitionPlan::pending(threshold);
    }
    if fact.weight == 0 {
        return PbftProposerSortitionPlan::rejected(PbftProposerSortitionStatus::ZeroWeight);
    }

    PbftProposerSortitionPlan::accepted(threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    const VRF_SECRET_KEY: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn signer_address(signing_key: &SigningKey) -> H160 {
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
        H160::from_slice(&public_key_hash.as_bytes()[12..])
    }

    fn signed_pbft_vote(
        signing_key: &SigningKey,
        block_hash: H256,
        period: u64,
        round: u64,
        step: u64,
        vrf_secret: &[u8; 64],
    ) -> Vec<u8> {
        let vrf_message = legacy_vrf_message_rlp(period, round, step);
        let proof = vrf::prove(vrf_secret, &vrf_message).unwrap();
        signed_pbft_vote_with_proof(signing_key, block_hash, period, round, step, proof)
    }

    fn signed_pbft_vote_with_proof(
        signing_key: &SigningKey,
        block_hash: H256,
        period: u64,
        round: u64,
        step: u64,
        proof: [u8; VRF_PROOF_BYTES],
    ) -> Vec<u8> {
        let mut sortition = RlpStream::new_list(4);
        sortition.append(&period);
        sortition.append(&round);
        sortition.append(&step);
        sortition.append(&proof.to_vec());
        let sortition_rlp = sortition.out().to_vec();
        let signing_hash = legacy_pbft_vote_signing_hash(block_hash, &sortition_rlp);
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

    fn vote_fact() -> PbftVoteValidationFact {
        PbftVoteValidationFact {
            vote_type: PbftVoteType::Cert,
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
            weight: 3,
            future_dpos_state: false,
            unknown_error: false,
            committee_size: 50,
            number_of_proposers: 20,
        }
    }

    #[test]
    fn accepts_valid_vote_facts_and_marks_replay() {
        let plan = plan_pbft_vote_validation(vote_fact());

        assert_eq!(plan.status, PbftVoteValidationStatus::Valid);
        assert!(plan.accepted);
        assert!(plan.mark_validated_replay);
        assert_eq!(plan.sortition_threshold, 50);
    }

    #[test]
    fn rejects_future_state_without_replay_marker() {
        let mut fact = vote_fact();
        fact.dpos_vote_count_ready = false;
        fact.future_dpos_state = true;

        let plan = plan_pbft_vote_validation(fact);

        assert_eq!(plan.status, PbftVoteValidationStatus::FutureDposState);
        assert!(plan.rejected);
        assert!(!plan.mark_validated_replay);
    }

    #[test]
    fn rejects_post_dpos_failures_with_replay_marker() {
        for (fact, status) in [
            {
                let mut fact = vote_fact();
                fact.dpos_vote_count = 0;
                (fact, PbftVoteValidationStatus::ZeroStake)
            },
            {
                let mut fact = vote_fact();
                fact.has_vrf_key = false;
                (fact, PbftVoteValidationStatus::MissingVrfKey)
            },
            {
                let mut fact = vote_fact();
                fact.signature_valid = false;
                (fact, PbftVoteValidationStatus::InvalidSignature)
            },
            {
                let mut fact = vote_fact();
                fact.vrf_sortition_valid = false;
                (fact, PbftVoteValidationStatus::InvalidVrfProof)
            },
            {
                let mut fact = vote_fact();
                fact.weight = 0;
                (fact, PbftVoteValidationStatus::ZeroWeight)
            },
        ] {
            let plan = plan_pbft_vote_validation(fact);
            assert_eq!(plan.status, status);
            assert!(plan.rejected);
            assert!(plan.mark_validated_replay);
        }
    }

    #[test]
    fn exposes_threshold_before_weight_is_ready() {
        let mut fact = vote_fact();
        fact.weight_ready = false;
        fact.weight = 0;

        let plan = plan_pbft_vote_validation(fact);

        assert_eq!(plan.status, PbftVoteValidationStatus::Pending);
        assert!(plan.mark_validated_replay);
        assert!(plan.has_sortition_threshold);
        assert_eq!(plan.sortition_threshold, 50);
    }

    #[test]
    fn proposer_threshold_uses_number_of_proposers() {
        assert_eq!(
            pbft_vote_sortition_threshold(100, PbftVoteType::Propose, 50, 20).unwrap(),
            20
        );
        assert_eq!(
            pbft_vote_sortition_threshold(15, PbftVoteType::Propose, 50, 20).unwrap(),
            15
        );
        assert_eq!(
            pbft_vote_sortition_threshold(100, PbftVoteType::Soft, 50, 20).unwrap(),
            50
        );
    }

    #[test]
    fn screens_local_proposer_sortition() {
        let fact = PbftProposerSortitionFact {
            dpos_vote_count_ready: true,
            dpos_vote_count: 10,
            total_dpos_vote_count_ready: true,
            total_dpos_vote_count: 100,
            weight_ready: true,
            weight: 1,
            future_dpos_state: false,
            unknown_error: false,
            number_of_proposers: 20,
        };

        let plan = plan_pbft_proposer_sortition(fact);

        assert_eq!(plan.status, PbftProposerSortitionStatus::Valid);
        assert!(plan.accepted);
        assert_eq!(plan.sortition_threshold, 20);
    }

    #[test]
    fn replay_cache_preserves_legacy_fifo_eviction_shape() {
        let mut cache = PbftVoteReplayCache::new(2, 1);
        let a = H256::from_low_u64_be(1);
        let b = H256::from_low_u64_be(2);
        let c = H256::from_low_u64_be(3);

        assert!(cache.insert(a));
        assert!(!cache.insert(a));
        assert!(cache.insert(b));
        assert!(cache.insert(c));

        assert!(!cache.contains(a));
        assert!(cache.contains(b));
        assert!(cache.contains(c));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn inspects_canonical_pbft_vote_rlp_and_recovers_identity() {
        let signing_key = SigningKey::from_slice(&[0x21; 32]).unwrap();
        let vote_rlp = signed_pbft_vote(
            &signing_key,
            H256::from_low_u64_be(77),
            12,
            3,
            3,
            &VRF_SECRET_KEY,
        );

        let inspection = inspect_canonical_pbft_vote(&vote_rlp).unwrap();

        assert_eq!(inspection.status, PbftCanonicalVoteInspectionStatus::Valid);
        assert!(inspection.signature_valid);
        assert_eq!(inspection.block_hash, H256::from_low_u64_be(77));
        assert_eq!(inspection.period, 12);
        assert_eq!(inspection.round, 3);
        assert_eq!(inspection.step, 3);
        assert_eq!(inspection.vote_type, PbftVoteType::Cert);
        assert_eq!(inspection.recovered_voter, signer_address(&signing_key));
        assert_eq!(inspection.vote_hash, keccak256(&vote_rlp));
    }

    #[test]
    fn canonical_validation_accepts_strict_vrf_and_calculates_weight() {
        let signing_key = SigningKey::from_slice(&[0x22; 32]).unwrap();
        let vote_rlp = signed_pbft_vote(
            &signing_key,
            H256::from_low_u64_be(88),
            13,
            4,
            3,
            &VRF_SECRET_KEY,
        );
        let vrf_public_key = vrf::public_key_from_secret(&VRF_SECRET_KEY).unwrap();

        let validation = validate_canonical_pbft_vote(
            &vote_rlp,
            PbftVoteValidationExternalFacts {
                voter_dpos_ready: true,
                voter_dpos_vote_count: 70,
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

        assert_eq!(validation.status, PbftVoteValidationStatus::Valid);
        assert!(validation.accepted);
        assert!(validation.vrf_valid);
        assert_eq!(validation.sortition_threshold, 100);
        assert_eq!(validation.calculated_weight, 70);
        assert_eq!(validation.recovered_voter, signer_address(&signing_key));
    }

    #[test]
    fn canonical_vote_inspection_reports_peer_data_failures_as_statuses() {
        let malformed = inspect_canonical_pbft_vote(&[0x01, 0x02, 0x03]).unwrap();
        assert_eq!(
            malformed.status,
            PbftCanonicalVoteInspectionStatus::MalformedRlp
        );

        let signing_key = SigningKey::from_slice(&[0x24; 32]).unwrap();
        let mut vote_rlp = signed_pbft_vote(
            &signing_key,
            H256::from_low_u64_be(90),
            15,
            6,
            3,
            &VRF_SECRET_KEY,
        );
        let vote = Rlp::new(&vote_rlp);
        let block_hash: H256 = vote.val_at(0).unwrap();
        let sortition_rlp: Vec<u8> = vote.val_at(1).unwrap();
        let mut invalid_signature_vote = RlpStream::new_list(3);
        invalid_signature_vote.append(&block_hash);
        invalid_signature_vote.append(&sortition_rlp);
        invalid_signature_vote.append(&vec![0_u8; SIGNATURE_BYTES]);
        vote_rlp = invalid_signature_vote.out().to_vec();

        let invalid_signature = inspect_canonical_pbft_vote(&vote_rlp).unwrap();
        assert_eq!(
            invalid_signature.status,
            PbftCanonicalVoteInspectionStatus::InvalidSignature
        );
        assert!(!invalid_signature.signature_valid);
    }
}
