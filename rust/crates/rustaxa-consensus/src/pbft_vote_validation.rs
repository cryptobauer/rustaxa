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

use anyhow::{Result, anyhow};
use ethereum_types::H256;
use std::collections::{HashSet, VecDeque};

use crate::verified_votes::PbftVoteType;

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
}
