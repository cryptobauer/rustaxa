//! Deterministic pillar-vote aggregation for Rust rewrite work.
//!
//! This module mirrors the pure in-memory state owned by C++
//! `pillar_chain::PillarVotes`: period initialization, per-period voter
//! uniqueness, per-block vote-weight accumulation, threshold subset selection,
//! and stale-period cleanup. It deliberately consumes already-verified vote
//! facts. Signature recovery, DPoS eligibility, vote-count lookup, storage
//! fallback, networking, and finalization side effects remain outside this
//! domain boundary until the broader `PillarChainManager` rewrite lands.

use anyhow::{Result, anyhow, ensure};
use ethereum_types::{H160, H256};
use rustaxa_types::PillarVote;
use std::cmp::Reverse;
use std::collections::{BTreeMap, btree_map::Entry};

/// Plain pillar-vote fact used by PBFT pillar-vote bundle validation.
///
/// Purpose:
/// - Carries one C++-decoded pillar vote plus external facts that Rust does not
///   own yet, such as signature/eligibility prevalidation and DPoS vote weight.
///
/// Invariants:
/// - `vote_hash`, `period`, and `block_hash` must identify the same live C++
///   `PillarVote` object that will be inserted if the bundle is accepted.
/// - `weight` is the C++/FinalChain-derived eligible vote count for `voter`.
/// - `prevalidated` must be true only after C++ has accepted non-ported checks
///   such as signature recovery, eligibility, and existing local-state conflict
///   checks.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PillarVoteFact {
    pub vote_hash: H256,
    pub period: u64,
    pub block_hash: H256,
    pub voter: H160,
    pub weight: u64,
    pub prevalidated: bool,
}

/// Stable status for a PBFT pillar-vote bundle planning pass.
///
/// These values are mapped to integer bridge codes and should remain stable for
/// C++ logging/tests.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PillarVoteBundleValidationStatus {
    Valid,
    EmptyBundle,
    VotePeriodMismatch,
    VoteBlockHashMismatch,
    PrevalidationFailed,
    ZeroWeight,
    VoterConflict,
    ThresholdNotReached,
    WeightOverflow,
}

impl PillarVoteBundleValidationStatus {
    /// Returns the stable CXX bridge code for this status.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Valid => 0,
            Self::EmptyBundle => 1,
            Self::VotePeriodMismatch => 2,
            Self::VoteBlockHashMismatch => 3,
            Self::PrevalidationFailed => 4,
            Self::ZeroWeight => 5,
            Self::VoterConflict => 6,
            Self::ThresholdNotReached => 7,
            Self::WeightOverflow => 8,
        }
    }
}

/// Deterministic decision returned for one PBFT pillar-vote bundle.
///
/// Outputs:
/// - `status` communicates the first deterministic rejection reason, or `Valid`.
/// - `accepted_vote_hashes` contains unique accepted vote hashes in deterministic
///   all-vote lookup order when the bundle reaches threshold; C++ uses this as
///   the side-effect insertion plan.
/// - `block_weight` is the unique accepted weight for the expected pillar block.
/// - `selected_weight` is the minimal above-threshold prefix weight.
/// - `first_bad_vote_hash` identifies the offending vote when a per-vote
///   validation status fails; otherwise it is zero.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PillarVoteBundlePlan {
    pub status: PillarVoteBundleValidationStatus,
    pub accepted_vote_hashes: Vec<H256>,
    pub block_weight: u64,
    pub selected_weight: u64,
    pub first_bad_vote_hash: H256,
}

/// Planner for deterministic PBFT pillar-vote bundle acceptance.
///
/// The planner is side-effect free. It does not decode RLP, recover signatures,
/// query FinalChain, or mutate C++ vote indexes. Those dependencies are supplied
/// as plain facts by the C++ shim-owned caller.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PillarVoteBundlePlanner {
    expected_period: u64,
    expected_block_hash: H256,
    threshold: u64,
}

impl PillarVoteBundlePlanner {
    /// Creates a planner bound to the expected period/block context.
    pub fn new(expected_period: u64, expected_block_hash: H256, threshold: u64) -> Self {
        Self {
            expected_period,
            expected_block_hash,
            threshold,
        }
    }

    /// Evaluates a batch of plain vote facts.
    ///
    /// Validation behavior:
    /// - Context mismatches reject the whole bundle with the offending hash.
    /// - `prevalidated == false` rejects the whole bundle; C++ owns the concrete
    ///   non-ported validation reason.
    /// - Duplicate vote hashes are idempotent and do not recount weight.
    /// - Same-period same-voter different-hash conflicts reject the bundle.
    /// - Threshold accounting uses unique accepted votes only.
    pub fn plan(&self, facts: &[PillarVoteFact]) -> PillarVoteBundlePlan {
        if facts.is_empty() {
            return self.empty_plan(PillarVoteBundleValidationStatus::EmptyBundle, H256::zero());
        }

        let mut votes = PillarVotes::new();
        votes.initialize_period_data(self.expected_period, self.threshold);

        for fact in facts {
            if fact.period != self.expected_period {
                return self.empty_plan(
                    PillarVoteBundleValidationStatus::VotePeriodMismatch,
                    fact.vote_hash,
                );
            }

            if fact.block_hash != self.expected_block_hash {
                return self.empty_plan(
                    PillarVoteBundleValidationStatus::VoteBlockHashMismatch,
                    fact.vote_hash,
                );
            }

            if !fact.prevalidated {
                return self.empty_plan(
                    PillarVoteBundleValidationStatus::PrevalidationFailed,
                    fact.vote_hash,
                );
            }

            if fact.weight == 0 {
                return self
                    .empty_plan(PillarVoteBundleValidationStatus::ZeroWeight, fact.vote_hash);
            }

            let vote = fact.to_verified_pillar_vote();
            match votes.add_verified_vote(vote) {
                Ok(outcome) => {
                    if !outcome.accepted {
                        return self.empty_plan(
                            PillarVoteBundleValidationStatus::VoterConflict,
                            fact.vote_hash,
                        );
                    }
                }
                Err(_) => {
                    return self.empty_plan(
                        PillarVoteBundleValidationStatus::WeightOverflow,
                        fact.vote_hash,
                    );
                }
            }
        }

        let all_votes =
            votes.get_verified_votes(self.expected_period, self.expected_block_hash, false);
        let above_threshold =
            votes.get_verified_votes(self.expected_period, self.expected_block_hash, true);

        if !above_threshold.threshold_met {
            return PillarVoteBundlePlan {
                status: PillarVoteBundleValidationStatus::ThresholdNotReached,
                accepted_vote_hashes: Vec::new(),
                block_weight: all_votes.block_weight,
                selected_weight: 0,
                first_bad_vote_hash: H256::zero(),
            };
        }

        PillarVoteBundlePlan {
            status: PillarVoteBundleValidationStatus::Valid,
            accepted_vote_hashes: all_votes
                .votes
                .into_iter()
                .map(|vote| vote.vote_hash)
                .collect(),
            block_weight: all_votes.block_weight,
            selected_weight: above_threshold.selected_weight,
            first_bad_vote_hash: H256::zero(),
        }
    }

    fn empty_plan(
        &self,
        status: PillarVoteBundleValidationStatus,
        first_bad_vote_hash: H256,
    ) -> PillarVoteBundlePlan {
        PillarVoteBundlePlan {
            status,
            accepted_vote_hashes: Vec::new(),
            block_weight: 0,
            selected_weight: 0,
            first_bad_vote_hash,
        }
    }
}

impl PillarVoteFact {
    fn to_verified_pillar_vote(&self) -> VerifiedPillarVote {
        VerifiedPillarVote {
            vote: PillarVote {
                period: self.period,
                block_hash: self.block_hash,
                signature: [0u8; 65],
            },
            vote_hash: self.vote_hash,
            voter: self.voter,
            weight: self.weight,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct WeightedPillarVotes {
    votes: BTreeMap<H256, VerifiedPillarVote>,
    weight: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PeriodPillarVotes {
    block_votes: BTreeMap<H256, WeightedPillarVotes>,
    unique_voters: BTreeMap<H160, H256>,
    threshold: u64,
}

/// Verified pillar vote metadata accepted by the Rust aggregation domain.
///
/// Inputs:
/// - `vote`: typed pillar vote payload, including period, block hash, and
///   signature bytes.
/// - `vote_hash`: canonical signed vote hash. C++ computes this as
///   `sha3(vote.encodeSolidity(true))`; callers may pass it explicitly so a
///   future bridge can preserve the C++ sidecar identity without recomputing.
/// - `voter`: signer address already recovered and validated by the caller.
/// - `weight`: validator vote count used for weighted threshold aggregation.
///
/// Invariants:
/// - `vote_hash` identifies the signed vote and must be stable for the lifetime
///   of the entry.
/// - `weight` is non-zero. Zero vote counts are rejected before aggregation,
///   matching the manager-level C++ behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPillarVote {
    pub vote: PillarVote,
    pub vote_hash: H256,
    pub voter: H160,
    pub weight: u64,
}

impl VerifiedPillarVote {
    /// Builds a verified vote fact and computes the canonical signed vote hash.
    pub fn new(vote: PillarVote, voter: H160, weight: u64) -> Result<Self> {
        let vote_hash = vote.hash(true);
        Self::from_parts(vote, vote_hash, voter, weight)
    }

    /// Builds a verified vote fact from caller-supplied identity fields.
    ///
    /// This constructor is intended for bridge and parity-test callers that
    /// already have the C++ vote hash sidecar. It checks only domain-level
    /// invariants that are local to `PillarVotes`; cryptographic validation
    /// stays outside this module.
    pub fn from_parts(vote: PillarVote, vote_hash: H256, voter: H160, weight: u64) -> Result<Self> {
        ensure!(
            weight > 0,
            "verified pillar vote weight must be non-zero: vote {vote_hash:#x}"
        );
        Ok(Self {
            vote,
            vote_hash,
            voter,
            weight,
        })
    }

    /// Returns the period this vote belongs to.
    pub fn period(&self) -> u64 {
        self.vote.period
    }

    /// Returns the pillar block hash this vote supports.
    pub fn block_hash(&self) -> H256 {
        self.vote.block_hash
    }
}

/// Result of inserting one verified pillar vote.
///
/// Outputs:
/// - `accepted` is false only when the same voter already contributed a
///   different vote hash in the period.
/// - `duplicate` is true when the exact vote hash was already present; duplicate
///   inserts do not increase accumulated weight.
/// - `conflicting_vote_hash` identifies the existing vote for non-unique voter
///   attempts.
/// - `block_weight` is the full accumulated weight for the vote's block after
///   the operation, or the existing block weight for rejected conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PillarVoteInsertOutcome {
    pub accepted: bool,
    pub duplicate: bool,
    pub conflicting_vote_hash: Option<H256>,
    pub block_weight: u64,
}

/// Lookup result for one period and pillar block.
///
/// `votes` contains either all verified votes for the block or, when
/// above-threshold selection is requested, the minimum deterministic prefix
/// whose cumulative weight reaches the period threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PillarVotesLookup {
    pub threshold_met: bool,
    pub block_weight: u64,
    pub selected_weight: u64,
    pub votes: Vec<VerifiedPillarVote>,
}

/// Rust-owned pillar-vote registry.
///
/// The registry stores deterministic metadata only. It does not own C++ live
/// vote pointers, perform signature recovery, read storage, or call FinalChain.
/// Ordering is intentionally stable: maps are keyed by hashes/addresses and
/// above-threshold ties are broken by vote hash, unlike C++'s unordered-map
/// dependent equal-weight behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PillarVotes {
    periods: BTreeMap<u64, PeriodPillarVotes>,
}

impl PillarVotes {
    /// Creates an empty pillar-vote registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the total number of stored vote hashes.
    pub fn len(&self) -> usize {
        self.periods
            .values()
            .flat_map(|period| period.block_votes.values())
            .map(|block_votes| block_votes.votes.len())
            .sum()
    }

    /// Returns true when no vote hashes are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Checks exact `(period, block_hash, vote_hash)` membership.
    pub fn vote_exists(&self, vote: &VerifiedPillarVote) -> bool {
        self.periods
            .get(&vote.period())
            .and_then(|period| period.block_votes.get(&vote.block_hash()))
            .is_some_and(|block_votes| block_votes.votes.contains_key(&vote.vote_hash))
    }

    /// Checks whether `vote` is unique for its `(period, voter)`.
    ///
    /// Returns true when no vote for the voter exists in the period or the
    /// existing vote hash is identical. Returns false when the voter already
    /// contributed a different vote hash.
    pub fn is_unique_vote(&self, vote: &VerifiedPillarVote) -> bool {
        self.periods
            .get(&vote.period())
            .and_then(|period| period.unique_voters.get(&vote.voter))
            .is_none_or(|existing_hash| *existing_hash == vote.vote_hash)
    }

    /// Returns whether threshold/vote state has been initialized for `period`.
    pub fn period_data_initialized(&self, period: u64) -> bool {
        self.periods.contains_key(&period)
    }

    /// Initializes period state with its consensus threshold.
    ///
    /// The first call wins, matching C++ `std::map::insert` behavior. Existing
    /// period state and threshold are not overwritten.
    pub fn initialize_period_data(&mut self, period: u64, threshold: u64) -> bool {
        match self.periods.entry(period) {
            Entry::Vacant(entry) => {
                entry.insert(PeriodPillarVotes {
                    threshold,
                    ..Default::default()
                });
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    /// Adds an already-verified vote to the registry.
    ///
    /// Errors:
    /// - returns an error when the period has not been initialized,
    /// - returns an error on accumulated weight overflow.
    ///
    /// Edge behavior:
    /// - exact duplicate vote hashes are accepted but do not add weight again,
    /// - same-period same-voter different-hash votes are rejected without
    ///   mutating aggregation state.
    pub fn add_verified_vote(
        &mut self,
        vote: VerifiedPillarVote,
    ) -> Result<PillarVoteInsertOutcome> {
        let period = vote.period();
        let block_hash = vote.block_hash();
        let period_votes = self.periods.get_mut(&period).ok_or_else(|| {
            anyhow!(
                "pillar period {period} is not initialized for verified vote {:#x}",
                vote.vote_hash
            )
        })?;

        match period_votes.unique_voters.entry(vote.voter) {
            Entry::Occupied(existing) if *existing.get() != vote.vote_hash => {
                let block_weight = period_votes
                    .block_votes
                    .get(&block_hash)
                    .map(|block_votes| block_votes.weight)
                    .unwrap_or_default();
                return Ok(PillarVoteInsertOutcome {
                    accepted: false,
                    duplicate: false,
                    conflicting_vote_hash: Some(*existing.get()),
                    block_weight,
                });
            }
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                entry.insert(vote.vote_hash);
            }
        }

        let block_votes = period_votes.block_votes.entry(block_hash).or_default();
        if block_votes.votes.contains_key(&vote.vote_hash) {
            return Ok(PillarVoteInsertOutcome {
                accepted: true,
                duplicate: true,
                conflicting_vote_hash: None,
                block_weight: block_votes.weight,
            });
        }

        block_votes.weight = block_votes.weight.checked_add(vote.weight).ok_or_else(|| {
            anyhow!("pillar vote weight overflow for period {period}, block {block_hash:#x}")
        })?;
        block_votes.votes.insert(vote.vote_hash, vote);

        Ok(PillarVoteInsertOutcome {
            accepted: true,
            duplicate: false,
            conflicting_vote_hash: None,
            block_weight: block_votes.weight,
        })
    }

    /// Returns verified votes for one period and pillar block.
    ///
    /// When `above_threshold` is false, all block votes are returned in vote-hash
    /// order. When `above_threshold` is true, the block's accumulated weight
    /// must reach the period threshold; the result is then the smallest prefix
    /// sorted by descending weight and ascending vote hash whose selected weight
    /// reaches the threshold.
    pub fn get_verified_votes(
        &self,
        period: u64,
        pillar_block_hash: H256,
        above_threshold: bool,
    ) -> PillarVotesLookup {
        let Some(period_votes) = self.periods.get(&period) else {
            return empty_lookup();
        };
        let Some(block_votes) = period_votes.block_votes.get(&pillar_block_hash) else {
            return empty_lookup();
        };

        let threshold_met = block_votes.weight >= period_votes.threshold;
        if !above_threshold {
            return PillarVotesLookup {
                threshold_met,
                block_weight: block_votes.weight,
                selected_weight: block_votes.weight,
                votes: block_votes.votes.values().cloned().collect(),
            };
        }

        if !threshold_met {
            return PillarVotesLookup {
                threshold_met: false,
                block_weight: block_votes.weight,
                selected_weight: 0,
                votes: Vec::new(),
            };
        }

        let mut ranked_votes = block_votes.votes.values().collect::<Vec<_>>();
        ranked_votes.sort_by_key(|vote| (Reverse(vote.weight), vote.vote_hash));

        let mut selected = Vec::with_capacity(ranked_votes.len());
        let mut selected_weight = 0u64;
        for vote in ranked_votes {
            selected_weight = selected_weight.saturating_add(vote.weight);
            selected.push(vote.clone());
            if selected_weight >= period_votes.threshold {
                break;
            }
        }

        PillarVotesLookup {
            threshold_met: true,
            block_weight: block_votes.weight,
            selected_weight,
            votes: selected,
        }
    }

    /// Returns accumulated weight for one period and pillar block.
    pub fn vote_weight(&self, period: u64, pillar_block_hash: H256) -> u64 {
        self.periods
            .get(&period)
            .and_then(|period| period.block_votes.get(&pillar_block_hash))
            .map(|block_votes| block_votes.weight)
            .unwrap_or_default()
    }

    /// Returns the configured threshold for `period`, when initialized.
    pub fn threshold(&self, period: u64) -> Option<u64> {
        self.periods.get(&period).map(|period| period.threshold)
    }

    /// Returns all stored verified pillar votes in deterministic order.
    ///
    /// Entries are ordered by period, pillar block hash, and vote hash. This is
    /// intended for C++ shim sidecar pruning and diagnostic snapshots; it does
    /// not expose storage, networking, or signature validation behavior.
    pub fn snapshot_votes(&self) -> Vec<VerifiedPillarVote> {
        self.periods
            .values()
            .flat_map(|period| period.block_votes.values())
            .flat_map(|block_votes| block_votes.votes.values().cloned())
            .collect()
    }

    /// Removes all vote state with period lower than `min_period`.
    pub fn erase_votes(&mut self, min_period: u64) {
        self.periods = self.periods.split_off(&min_period);
    }
}

fn empty_lookup() -> PillarVotesLookup {
    PillarVotesLookup {
        threshold_met: false,
        block_weight: 0,
        selected_weight: 0,
        votes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(vote_hash: u64, period: u64, block: u64, voter: u64, weight: u64) -> PillarVoteFact {
        PillarVoteFact {
            vote_hash: H256::from_low_u64_be(vote_hash),
            period,
            block_hash: H256::from_low_u64_be(block),
            voter: H160::from_low_u64_be(voter),
            weight,
            prevalidated: true,
        }
    }

    #[test]
    fn bundle_plan_accepts_unique_votes_when_threshold_is_met() {
        let planner = PillarVoteBundlePlanner::new(10, H256::from_low_u64_be(1234), 6);
        let votes = [
            fact(11, 10, 1234, 1, 1),
            fact(12, 10, 1234, 2, 2),
            fact(13, 10, 1234, 3, 3),
        ];

        let plan = planner.plan(&votes);

        assert_eq!(plan.status, PillarVoteBundleValidationStatus::Valid);
        assert_eq!(plan.block_weight, 6);
        assert_eq!(plan.selected_weight, 6);
        assert_eq!(
            plan.accepted_vote_hashes,
            vec![
                H256::from_low_u64_be(11),
                H256::from_low_u64_be(12),
                H256::from_low_u64_be(13)
            ]
        );
    }

    #[test]
    fn bundle_plan_below_threshold_returns_empty_selection() {
        let planner = PillarVoteBundlePlanner::new(10, H256::from_low_u64_be(1235), 10);
        let votes = [fact(21, 10, 1235, 1, 3), fact(22, 10, 1235, 2, 3)];

        let plan = planner.plan(&votes);

        assert_eq!(
            plan.status,
            PillarVoteBundleValidationStatus::ThresholdNotReached
        );
        assert_eq!(plan.block_weight, 6);
        assert_eq!(plan.selected_weight, 0);
        assert!(plan.accepted_vote_hashes.is_empty());
    }

    #[test]
    fn bundle_plan_rejects_period_and_block_mismatch() {
        let planner = PillarVoteBundlePlanner::new(11, H256::from_low_u64_be(1111), 3);

        let mismatch_period = fact(31, 12, 1111, 1, 4);
        let mismatch_block = fact(32, 11, 1112, 2, 4);

        let period_plan = planner.plan(&[mismatch_period]);
        assert_eq!(
            period_plan.status,
            PillarVoteBundleValidationStatus::VotePeriodMismatch
        );
        assert_eq!(period_plan.first_bad_vote_hash, H256::from_low_u64_be(31));

        let block_plan = planner.plan(&[mismatch_block]);
        assert_eq!(
            block_plan.status,
            PillarVoteBundleValidationStatus::VoteBlockHashMismatch
        );
        assert_eq!(block_plan.first_bad_vote_hash, H256::from_low_u64_be(32));
    }

    #[test]
    fn bundle_plan_rejects_prevalidation_and_zero_weight_failures() {
        let planner = PillarVoteBundlePlanner::new(10, H256::from_low_u64_be(2222), 5);

        let prevalidation_plan = planner.plan(&[PillarVoteFact {
            prevalidated: false,
            ..fact(41, 10, 2222, 1, 3)
        }]);
        assert_eq!(
            prevalidation_plan.status,
            PillarVoteBundleValidationStatus::PrevalidationFailed
        );
        assert_eq!(
            prevalidation_plan.first_bad_vote_hash,
            H256::from_low_u64_be(41)
        );

        let zero_weight_plan = planner.plan(&[fact(42, 10, 2222, 2, 0)]);
        assert_eq!(
            zero_weight_plan.status,
            PillarVoteBundleValidationStatus::ZeroWeight
        );
        assert_eq!(
            zero_weight_plan.first_bad_vote_hash,
            H256::from_low_u64_be(42)
        );
    }

    #[test]
    fn bundle_plan_rejects_voter_conflicts_without_recounting_duplicates() {
        let planner = PillarVoteBundlePlanner::new(10, H256::from_low_u64_be(3333), 6);
        let duplicate = fact(51, 10, 3333, 1, 3);

        let duplicate_plan =
            planner.plan(&[duplicate.clone(), duplicate, fact(52, 10, 3333, 2, 3)]);
        assert_eq!(
            duplicate_plan.status,
            PillarVoteBundleValidationStatus::Valid
        );
        assert_eq!(duplicate_plan.block_weight, 6);
        assert_eq!(
            duplicate_plan.accepted_vote_hashes,
            vec![H256::from_low_u64_be(51), H256::from_low_u64_be(52)]
        );

        let conflict_plan = planner.plan(&[fact(61, 10, 3333, 1, 3), fact(62, 10, 3333, 1, 3)]);
        assert_eq!(
            conflict_plan.status,
            PillarVoteBundleValidationStatus::VoterConflict
        );
        assert_eq!(conflict_plan.first_bad_vote_hash, H256::from_low_u64_be(62));
    }

    fn signature(seed: u8) -> [u8; 65] {
        let mut signature = [seed; 65];
        signature[64] = seed & 1;
        signature
    }

    fn vote(
        period: u64,
        block: u64,
        voter: u64,
        vote_hash: u64,
        weight: u64,
    ) -> VerifiedPillarVote {
        VerifiedPillarVote::from_parts(
            PillarVote {
                period,
                block_hash: H256::from_low_u64_be(block),
                signature: signature(vote_hash as u8),
            },
            H256::from_low_u64_be(vote_hash),
            H160::from_low_u64_be(voter),
            weight,
        )
        .unwrap()
    }

    #[test]
    fn initializes_period_once_without_overwriting_threshold() {
        let mut votes = PillarVotes::new();

        assert!(votes.initialize_period_data(10, 7));
        assert!(!votes.initialize_period_data(10, 99));

        assert!(votes.period_data_initialized(10));
        assert_eq!(votes.threshold(10), Some(7));
    }

    #[test]
    fn vote_exists_matches_exact_period_block_and_hash() {
        let mut votes = PillarVotes::new();
        votes.initialize_period_data(11, 1);
        let accepted = vote(11, 100, 1, 7, 5);
        let different_block = vote(11, 101, 1, 7, 5);

        assert!(!votes.vote_exists(&accepted));
        votes.add_verified_vote(accepted.clone()).unwrap();

        assert!(votes.vote_exists(&accepted));
        assert!(!votes.vote_exists(&different_block));
        assert_eq!(votes.vote_weight(11, H256::from_low_u64_be(100)), 5);
        assert_eq!(votes.len(), 1);
    }

    #[test]
    fn duplicate_vote_hash_is_accepted_without_recount() {
        let mut votes = PillarVotes::new();
        votes.initialize_period_data(12, 100);
        let accepted = vote(12, 100, 1, 9, 6);

        let first = votes.add_verified_vote(accepted.clone()).unwrap();
        let second = votes.add_verified_vote(accepted.clone()).unwrap();

        assert!(first.accepted);
        assert!(!first.duplicate);
        assert!(second.accepted);
        assert!(second.duplicate);
        assert_eq!(second.block_weight, 6);
        assert_eq!(
            votes
                .get_verified_votes(12, H256::from_low_u64_be(100), false)
                .votes
                .len(),
            1
        );
    }

    #[test]
    fn same_voter_different_hash_is_rejected_without_mutation() {
        let mut votes = PillarVotes::new();
        votes.initialize_period_data(13, 1);
        let first = vote(13, 100, 1, 10, 4);
        let conflict = vote(13, 101, 1, 11, 8);

        votes.add_verified_vote(first.clone()).unwrap();
        assert!(!votes.is_unique_vote(&conflict));
        let outcome = votes.add_verified_vote(conflict.clone()).unwrap();

        assert!(!outcome.accepted);
        assert_eq!(outcome.conflicting_vote_hash, Some(first.vote_hash));
        assert!(!votes.vote_exists(&conflict));
        assert_eq!(votes.len(), 1);
    }

    #[test]
    fn missing_period_is_an_explicit_error() {
        let mut votes = PillarVotes::new();

        assert!(votes.add_verified_vote(vote(14, 100, 1, 12, 1)).is_err());
    }

    #[test]
    fn above_threshold_returns_empty_when_block_weight_is_low() {
        let mut votes = PillarVotes::new();
        votes.initialize_period_data(15, 10);
        votes.add_verified_vote(vote(15, 100, 1, 13, 4)).unwrap();
        votes.add_verified_vote(vote(15, 100, 2, 14, 5)).unwrap();

        let lookup = votes.get_verified_votes(15, H256::from_low_u64_be(100), true);

        assert!(!lookup.threshold_met);
        assert_eq!(lookup.block_weight, 9);
        assert_eq!(lookup.selected_weight, 0);
        assert!(lookup.votes.is_empty());
    }

    #[test]
    fn above_threshold_selects_minimum_weighted_prefix() {
        let mut votes = PillarVotes::new();
        votes.initialize_period_data(16, 7);
        let low = vote(16, 100, 1, 21, 1);
        let high = vote(16, 100, 2, 22, 5);
        let mid = vote(16, 100, 3, 23, 2);
        votes.add_verified_vote(low).unwrap();
        votes.add_verified_vote(high.clone()).unwrap();
        votes.add_verified_vote(mid.clone()).unwrap();

        let lookup = votes.get_verified_votes(16, H256::from_low_u64_be(100), true);

        assert!(lookup.threshold_met);
        assert_eq!(lookup.block_weight, 8);
        assert_eq!(lookup.selected_weight, 7);
        assert_eq!(
            lookup
                .votes
                .iter()
                .map(|vote| vote.vote_hash)
                .collect::<Vec<_>>(),
            vec![high.vote_hash, mid.vote_hash]
        );
    }

    #[test]
    fn equal_weight_threshold_selection_uses_vote_hash_tie_breaker() {
        let mut votes = PillarVotes::new();
        votes.initialize_period_data(17, 8);
        let later_hash = vote(17, 100, 1, 31, 4);
        let earlier_hash = vote(17, 100, 2, 30, 4);
        let lower_weight = vote(17, 100, 3, 29, 3);
        votes.add_verified_vote(later_hash).unwrap();
        votes.add_verified_vote(earlier_hash.clone()).unwrap();
        votes.add_verified_vote(lower_weight).unwrap();

        let lookup = votes.get_verified_votes(17, H256::from_low_u64_be(100), true);

        assert_eq!(lookup.selected_weight, 8);
        assert_eq!(
            lookup
                .votes
                .iter()
                .map(|vote| vote.vote_hash)
                .collect::<Vec<_>>(),
            vec![H256::from_low_u64_be(30), H256::from_low_u64_be(31)]
        );
    }

    #[test]
    fn all_votes_lookup_uses_vote_hash_order() {
        let mut votes = PillarVotes::new();
        votes.initialize_period_data(18, 1);
        votes.add_verified_vote(vote(18, 100, 1, 42, 1)).unwrap();
        votes.add_verified_vote(vote(18, 100, 2, 40, 1)).unwrap();
        votes.add_verified_vote(vote(18, 100, 3, 41, 1)).unwrap();

        let lookup = votes.get_verified_votes(18, H256::from_low_u64_be(100), false);

        assert_eq!(
            lookup
                .votes
                .iter()
                .map(|vote| vote.vote_hash)
                .collect::<Vec<_>>(),
            vec![
                H256::from_low_u64_be(40),
                H256::from_low_u64_be(41),
                H256::from_low_u64_be(42)
            ]
        );
    }

    #[test]
    fn erase_votes_removes_only_periods_below_min_period() {
        let mut votes = PillarVotes::new();
        for period in [20, 21, 22] {
            votes.initialize_period_data(period, 1);
            votes
                .add_verified_vote(vote(period, 100, period, period, 1))
                .unwrap();
        }

        votes.erase_votes(21);

        assert!(!votes.period_data_initialized(20));
        assert!(votes.period_data_initialized(21));
        assert!(votes.period_data_initialized(22));
        assert_eq!(votes.len(), 2);
        assert_eq!(
            votes
                .snapshot_votes()
                .into_iter()
                .map(|vote| vote.period())
                .collect::<Vec<_>>(),
            vec![21, 22]
        );
    }

    #[test]
    fn zero_weight_vote_fact_is_rejected() {
        assert!(vote_result(23, 100, 1, 50, 0).is_err());
    }

    fn vote_result(
        period: u64,
        block: u64,
        voter: u64,
        vote_hash: u64,
        weight: u64,
    ) -> Result<VerifiedPillarVote> {
        VerifiedPillarVote::from_parts(
            PillarVote {
                period,
                block_hash: H256::from_low_u64_be(block),
                signature: signature(vote_hash as u8),
            },
            H256::from_low_u64_be(vote_hash),
            H160::from_low_u64_be(voter),
            weight,
        )
    }

    #[test]
    fn weight_overflow_is_an_explicit_error() {
        let mut votes = PillarVotes::new();
        votes.initialize_period_data(24, 1);
        votes
            .add_verified_vote(vote(24, 100, 1, 51, u64::MAX))
            .unwrap();

        assert!(votes.add_verified_vote(vote(24, 100, 2, 52, 1)).is_err());
    }
}
