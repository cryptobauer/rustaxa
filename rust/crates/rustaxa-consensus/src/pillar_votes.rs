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
