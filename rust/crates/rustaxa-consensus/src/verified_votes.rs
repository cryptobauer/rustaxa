//! Verified PBFT vote index for Rust rewrite mode.
//!
//! This module models deterministic verified-vote bookkeeping used by consensus:
//! - uniqueness per `(period, round, step, voter)` with legacy next-vote exception,
//! - per-value vote aggregation by vote hash and cumulative weight,
//! - per-round 2t+1 voted-block mapping,
//! - network `t+1` next-voting step marker,
//! - period-based cleanup and deterministic snapshots.
//!
//! The structure owns only metadata and vote identity fields. Live C++ `PbftVote`
//! objects remain owned by the shim/caller.

use anyhow::{Result, anyhow};
use ethereum_types::{H160, H256};
use std::collections::BTreeMap;

/// PBFT vote type encoded as legacy numeric step-compatible values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum PbftVoteType {
    Invalid = 0,
    Propose = 1,
    Soft = 2,
    Cert = 3,
    Next = 4,
}

impl TryFrom<u8> for PbftVoteType {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Invalid),
            1 => Ok(Self::Propose),
            2 => Ok(Self::Soft),
            3 => Ok(Self::Cert),
            4 => Ok(Self::Next),
            _ => Err(anyhow!("unsupported PBFT vote type value: {value}")),
        }
    }
}

impl From<PbftVoteType> for u8 {
    fn from(value: PbftVoteType) -> Self {
        value as u8
    }
}

/// 2t+1 voted-block category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum TwoTPlusOneVotedBlockType {
    SoftVotedBlock = 0,
    CertVotedBlock = 1,
    NextVotedBlock = 2,
    NextVotedNullBlock = 3,
}

impl TryFrom<u8> for TwoTPlusOneVotedBlockType {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::SoftVotedBlock),
            1 => Ok(Self::CertVotedBlock),
            2 => Ok(Self::NextVotedBlock),
            3 => Ok(Self::NextVotedNullBlock),
            _ => Err(anyhow!("unsupported 2t+1 voted-block type value: {value}")),
        }
    }
}

impl From<TwoTPlusOneVotedBlockType> for u8 {
    fn from(value: TwoTPlusOneVotedBlockType) -> Self {
        value as u8
    }
}

/// Plain vote metadata stored by Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedVote {
    pub vote_hash: H256,
    pub block_hash: H256,
    pub voter: H160,
    pub period: u64,
    pub round: u64,
    pub step: u64,
    pub vote_type: PbftVoteType,
    pub weight: u64,
}

impl VerifiedVote {
    /// Creates a new verified-vote payload.
    ///
    /// `weight` must be non-zero.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        vote_hash: H256,
        block_hash: H256,
        voter: H160,
        period: u64,
        round: u64,
        step: u64,
        vote_type: PbftVoteType,
        weight: u64,
    ) -> Result<Self> {
        if weight == 0 {
            return Err(anyhow!(
                "verified vote cannot be inserted with zero weight: vote {vote_hash:#x}"
            ));
        }

        Ok(Self {
            vote_hash,
            block_hash,
            voter,
            period,
            round,
            step,
            vote_type,
            weight,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UniqueVoteRef {
    vote_hash: H256,
    block_hash: H256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UniqueVoterVotes {
    primary: UniqueVoteRef,
    secondary: Option<UniqueVoteRef>,
}

/// Per-voted-value aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VotesWithWeight {
    pub weight: u64,
    pub votes: BTreeMap<H256, VerifiedVote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct StepVotes {
    votes: BTreeMap<H256, VotesWithWeight>,
    unique_voters: BTreeMap<H160, UniqueVoterVotes>,
}

/// Step-level snapshot for one voted block hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepVotesSnapshotEntry {
    pub block_hash: H256,
    pub total_weight: u64,
    pub vote_hashes: Vec<H256>,
}

/// 2t+1 voted block hash and step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VotedBlock {
    pub hash: H256,
    pub step: u64,
}

/// 2t+1 voted-block insertion outcome for one `(period, round, kind)` key.
///
/// `round_found` indicates whether the requested round exists. `inserted` is
/// true only when the key was previously unset and the new mapping was stored.
/// When `round_found` is true and `inserted` is false, the existing mapping is
/// preserved (first-writer-wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TwoTPlusOneInsertOutcome {
    pub round_found: bool,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RoundVerifiedVotes {
    two_t_plus_one_voted_blocks: BTreeMap<TwoTPlusOneVotedBlockType, VotedBlock>,
    step_votes: BTreeMap<u64, StepVotes>,
    network_t_plus_one_step: u64,
}

/// Unique-voter precheck outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UniqueVoterCheckOutcome {
    pub is_unique: bool,
    pub conflicting_vote_hash: Option<H256>,
}

/// Unique-voter insert outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UniqueVoterInsertOutcome {
    pub accepted: bool,
    pub conflicting_vote_hash: Option<H256>,
    pub used_secondary_slot: bool,
    pub duplicate_vote_hash: bool,
}

/// Insert/update result for voted-value aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VotedValueInsertOutcome {
    pub inserted: bool,
    pub total_weight: u64,
    pub votes_count: usize,
}

/// Result of one deterministic verified-vote insertion with optional threshold
/// decisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddVerifiedVoteOutcome {
    pub inserted: bool,
    pub total_weight: u64,
    pub votes_count: usize,
    pub conflicting_vote_hash: Option<H256>,
    pub used_secondary_slot: bool,
    pub duplicate_vote_hash: bool,
    pub threshold_decision: Option<ThresholdDecisionOutcome>,
}

/// Outcome of an atomic verified-vote insert operation.
///
/// The operation first performs unique-voter tracking and then voted-value
/// aggregation as one logical unit:
/// - uniqueness conflicts return the conflicting vote hash for slashing
///   decisions and do not touch voted-value aggregation,
/// - voted-value insertion reports deterministic aggregate counters,
/// - voted-value insertion errors roll back unique-voter state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtomicVoteInsertOutcome {
    pub inserted: bool,
    pub total_weight: u64,
    pub votes_count: usize,
    pub conflicting_vote_hash: Option<H256>,
    pub used_secondary_slot: bool,
    pub duplicate_vote_hash: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UniqueVoterRollbackState {
    had_period: bool,
    had_round: bool,
    had_step: bool,
    previous_vote: Option<UniqueVoterVotes>,
}

/// Snapshot of one 2t+1 mapping entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TwoTPlusOneSnapshotEntry {
    pub period: u64,
    pub round: u64,
    pub kind: TwoTPlusOneVotedBlockType,
    pub block_hash: H256,
    pub step: u64,
}

/// Snapshot of one round-level marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundMarkerSnapshot {
    pub period: u64,
    pub round: u64,
    pub network_t_plus_one_step: u64,
}

/// Deterministic round-advance decision derived from stored next-vote 2t+1
/// mappings for one period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetermineNewRoundOutcome {
    pub new_round: u64,
    pub source_round: u64,
    pub source_kind: TwoTPlusOneVotedBlockType,
    pub block_hash: H256,
    pub step: u64,
}

/// Deterministic threshold decision outcome for one verified vote.
///
/// This captures state transitions owned by `VerifiedVotes` once vote
/// aggregation has produced `total_weight` and the caller supplies the per-vote
/// 2t+1 threshold:
/// - `t_plus_one_reached` / `network_t_plus_one_step_updated` describe `next`
///   vote t+1 handling,
/// - `two_t_plus_one_reached` describes whether 2t+1 was met,
/// - `two_t_plus_one_kind` and `two_t_plus_one_insert_outcome` describe mapped
///   2t+1 marker insertion (first-writer-wins, existing-round-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThresholdDecisionOutcome {
    pub t_plus_one_reached: bool,
    pub network_t_plus_one_step_updated: bool,
    pub two_t_plus_one_reached: bool,
    pub two_t_plus_one_kind: Option<TwoTPlusOneVotedBlockType>,
    pub two_t_plus_one_insert_outcome: Option<TwoTPlusOneInsertOutcome>,
}

/// Rust-owned verified-votes index.
#[derive(Debug, Clone, Default)]
pub struct VerifiedVotes {
    votes: BTreeMap<u64, BTreeMap<u64, RoundVerifiedVotes>>,
}

impl VerifiedVotes {
    /// Creates an empty verified-votes index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns total count of stored vote hashes across all periods/rounds/steps.
    pub fn size(&self) -> u64 {
        self.votes
            .values()
            .flat_map(|rounds| rounds.values())
            .flat_map(|round| round.step_votes.values())
            .flat_map(|step| step.votes.values())
            .map(|value| value.votes.len() as u64)
            .sum()
    }

    /// Returns all stored votes in deterministic order.
    pub fn snapshot_votes(&self) -> Vec<VerifiedVote> {
        let mut out = Vec::with_capacity(self.size() as usize);
        for rounds in self.votes.values() {
            for round in rounds.values() {
                for step_votes in round.step_votes.values() {
                    for votes_with_weight in step_votes.votes.values() {
                        out.extend(votes_with_weight.votes.values().cloned());
                    }
                }
            }
        }
        out
    }

    /// Returns deterministic snapshot of all 2t+1 mappings.
    pub fn snapshot_two_t_plus_one(&self) -> Vec<TwoTPlusOneSnapshotEntry> {
        let mut out = Vec::new();
        for (period, rounds) in &self.votes {
            for (round, round_votes) in rounds {
                for (kind, voted) in &round_votes.two_t_plus_one_voted_blocks {
                    out.push(TwoTPlusOneSnapshotEntry {
                        period: *period,
                        round: *round,
                        kind: *kind,
                        block_hash: voted.hash,
                        step: voted.step,
                    });
                }
            }
        }
        out
    }

    /// Returns deterministic snapshot of per-round network t+1 markers.
    pub fn snapshot_round_markers(&self) -> Vec<RoundMarkerSnapshot> {
        let mut out = Vec::new();
        for (period, rounds) in &self.votes {
            for (round, round_votes) in rounds {
                out.push(RoundMarkerSnapshot {
                    period: *period,
                    round: *round,
                    network_t_plus_one_step: round_votes.network_t_plus_one_step,
                });
            }
        }
        out
    }

    /// Checks uniqueness against current stored unique-voter map for the same
    /// `(period, round, step, voter)`.
    pub fn check_unique_voter(&self, vote: &VerifiedVote) -> UniqueVoterCheckOutcome {
        let Some(round_votes) = self.get_round(vote.period, vote.round) else {
            return UniqueVoterCheckOutcome {
                is_unique: true,
                conflicting_vote_hash: None,
            };
        };

        let Some(step_votes) = round_votes.step_votes.get(&vote.step) else {
            return UniqueVoterCheckOutcome {
                is_unique: true,
                conflicting_vote_hash: None,
            };
        };

        let Some(voter_votes) = step_votes.unique_voters.get(&vote.voter) else {
            return UniqueVoterCheckOutcome {
                is_unique: true,
                conflicting_vote_hash: None,
            };
        };

        if voter_votes.primary.vote_hash == vote.vote_hash
            || voter_votes
                .secondary
                .map(|second| second.vote_hash == vote.vote_hash)
                .unwrap_or(false)
        {
            return UniqueVoterCheckOutcome {
                is_unique: true,
                conflicting_vote_hash: None,
            };
        }

        if Self::can_insert_secondary_next_vote(voter_votes, vote) {
            return UniqueVoterCheckOutcome {
                is_unique: true,
                conflicting_vote_hash: None,
            };
        }

        UniqueVoterCheckOutcome {
            is_unique: false,
            conflicting_vote_hash: Some(
                voter_votes
                    .secondary
                    .map(|v| v.vote_hash)
                    .unwrap_or(voter_votes.primary.vote_hash),
            ),
        }
    }

    /// Inserts vote into unique-voter tracking and returns acceptance outcome.
    pub fn insert_unique_voter(&mut self, vote: &VerifiedVote) -> UniqueVoterInsertOutcome {
        let step_votes = self.ensure_step_mut(vote.period, vote.round, vote.step);
        let vote_ref = UniqueVoteRef {
            vote_hash: vote.vote_hash,
            block_hash: vote.block_hash,
        };

        let Some(voter_votes) = step_votes.unique_voters.get_mut(&vote.voter) else {
            step_votes.unique_voters.insert(
                vote.voter,
                UniqueVoterVotes {
                    primary: vote_ref,
                    secondary: None,
                },
            );
            return UniqueVoterInsertOutcome {
                accepted: true,
                conflicting_vote_hash: None,
                used_secondary_slot: false,
                duplicate_vote_hash: false,
            };
        };

        if voter_votes.primary.vote_hash == vote.vote_hash
            || voter_votes
                .secondary
                .map(|second| second.vote_hash == vote.vote_hash)
                .unwrap_or(false)
        {
            return UniqueVoterInsertOutcome {
                accepted: true,
                conflicting_vote_hash: None,
                used_secondary_slot: false,
                duplicate_vote_hash: true,
            };
        }

        if Self::can_insert_secondary_next_vote(voter_votes, vote) {
            voter_votes.secondary = Some(vote_ref);
            return UniqueVoterInsertOutcome {
                accepted: true,
                conflicting_vote_hash: None,
                used_secondary_slot: true,
                duplicate_vote_hash: false,
            };
        }

        UniqueVoterInsertOutcome {
            accepted: false,
            conflicting_vote_hash: Some(
                voter_votes
                    .secondary
                    .map(|v| v.vote_hash)
                    .unwrap_or(voter_votes.primary.vote_hash),
            ),
            used_secondary_slot: false,
            duplicate_vote_hash: false,
        }
    }

    /// Inserts vote into voted-value aggregation keyed by `(period, round, step,
    /// block_hash, vote_hash)`.
    pub fn insert_voted_value(&mut self, vote: VerifiedVote) -> Result<VotedValueInsertOutcome> {
        let step_votes = self.ensure_step_mut(vote.period, vote.round, vote.step);
        let votes_with_weight = step_votes.votes.entry(vote.block_hash).or_default();

        if votes_with_weight.votes.contains_key(&vote.vote_hash) {
            return Ok(VotedValueInsertOutcome {
                inserted: false,
                total_weight: votes_with_weight.weight,
                votes_count: votes_with_weight.votes.len(),
            });
        }

        votes_with_weight.weight = votes_with_weight
            .weight
            .checked_add(vote.weight)
            .ok_or_else(|| {
                anyhow!(
                    "vote weight overflow for period {}, round {}, step {}, block {:#x}",
                    vote.period,
                    vote.round,
                    vote.step,
                    vote.block_hash
                )
            })?;
        votes_with_weight.votes.insert(vote.vote_hash, vote);

        Ok(VotedValueInsertOutcome {
            inserted: true,
            total_weight: votes_with_weight.weight,
            votes_count: votes_with_weight.votes.len(),
        })
    }

    /// Atomically inserts one verified vote into unique-voter tracking and
    /// voted-value aggregation.
    ///
    /// The odd-step `Next` dual-vote special case is preserved exactly as in
    /// `insert_unique_voter`. When uniqueness fails, this returns
    /// `conflicting_vote_hash` for slashing decisions and leaves voted-value
    /// aggregation unchanged.
    ///
    /// If voted-value insertion errors (for example, weight overflow), unique
    /// voter state is rolled back to its pre-call contents.
    pub fn insert_vote_atomic(&mut self, vote: VerifiedVote) -> Result<AtomicVoteInsertOutcome> {
        let rollback = self.snapshot_unique_voter_rollback_state(
            vote.period,
            vote.round,
            vote.step,
            vote.voter,
        );
        let unique_outcome = self.insert_unique_voter(&vote);
        if !unique_outcome.accepted {
            return Ok(AtomicVoteInsertOutcome {
                inserted: false,
                total_weight: 0,
                votes_count: 0,
                conflicting_vote_hash: unique_outcome.conflicting_vote_hash,
                used_secondary_slot: false,
                duplicate_vote_hash: false,
            });
        }

        match self.insert_voted_value(vote) {
            Ok(voted_outcome) => Ok(AtomicVoteInsertOutcome {
                inserted: voted_outcome.inserted,
                total_weight: voted_outcome.total_weight,
                votes_count: voted_outcome.votes_count,
                conflicting_vote_hash: None,
                used_secondary_slot: unique_outcome.used_secondary_slot,
                duplicate_vote_hash: unique_outcome.duplicate_vote_hash,
            }),
            Err(err) => {
                self.restore_unique_voter_state(
                    rollback.0, rollback.1, rollback.2, rollback.3, rollback.4,
                );
                Err(err)
            }
        }
    }

    /// Adds one verified vote and, when requested, applies deterministic
    /// threshold effects for that vote.
    ///
    /// This is the smallest full primitive for deterministic VoteManager
    /// transitions: uniqueness and voted-value insertion happen atomically, and
    /// callers may request threshold processing in the same call.
    pub fn add_verified_vote(
        &mut self,
        vote: VerifiedVote,
        two_t_plus_one_threshold: Option<u64>,
    ) -> Result<AddVerifiedVoteOutcome> {
        let outcome = self.insert_vote_atomic(vote.clone())?;

        let threshold_decision = match (outcome.inserted, two_t_plus_one_threshold) {
            (true, Some(threshold)) => {
                Some(self.apply_threshold_decision(&vote, outcome.total_weight, threshold)?)
            }
            _ => None,
        };

        Ok(AddVerifiedVoteOutcome {
            inserted: outcome.inserted,
            total_weight: outcome.total_weight,
            votes_count: outcome.votes_count,
            conflicting_vote_hash: outcome.conflicting_vote_hash,
            used_secondary_slot: outcome.used_secondary_slot,
            duplicate_vote_hash: outcome.duplicate_vote_hash,
            threshold_decision,
        })
    }

    /// Returns true when vote hash exists under exact period/round/step/value key.
    pub fn vote_in_verified_map(
        &self,
        period: u64,
        round: u64,
        step: u64,
        block_hash: H256,
        vote_hash: H256,
    ) -> bool {
        self.get_round(period, round)
            .and_then(|round_votes| round_votes.step_votes.get(&step))
            .and_then(|step_votes| step_votes.votes.get(&block_hash))
            .map(|votes_with_weight| votes_with_weight.votes.contains_key(&vote_hash))
            .unwrap_or(false)
    }

    /// Returns all voted values and vote hashes for one step.
    pub fn get_step_votes(
        &self,
        period: u64,
        round: u64,
        step: u64,
    ) -> Option<Vec<StepVotesSnapshotEntry>> {
        self.get_round(period, round).and_then(|round_votes| {
            round_votes.step_votes.get(&step).map(|step_votes| {
                step_votes
                    .votes
                    .iter()
                    .map(|(hash, votes)| StepVotesSnapshotEntry {
                        block_hash: *hash,
                        total_weight: votes.weight,
                        vote_hashes: votes.votes.keys().copied().collect(),
                    })
                    .collect()
            })
        })
    }

    /// Returns the aggregate voted-value bucket for one exact vote target.
    ///
    /// This is a read-only helper for runtime bridges that need the same
    /// weight and vote-hash set as legacy C++ `VotesWithWeight` after Rust has
    /// applied one insertion. It clones only the selected bucket and does not
    /// expose mutable access to internal maps.
    pub fn votes_with_weight(
        &self,
        period: u64,
        round: u64,
        step: u64,
        block_hash: H256,
    ) -> Option<VotesWithWeight> {
        self.get_round(period, round)
            .and_then(|round_votes| round_votes.step_votes.get(&step))
            .and_then(|step_votes| step_votes.votes.get(&block_hash))
            .cloned()
    }

    /// Sets network t+1 step marker for an existing round.
    ///
    /// Returns `true` when round existed and marker was updated.
    pub fn set_network_t_plus_one_step(&mut self, period: u64, round: u64, step: u64) -> bool {
        let Some(round_votes) = self
            .votes
            .get_mut(&period)
            .and_then(|rounds| rounds.get_mut(&round))
        else {
            return false;
        };
        round_votes.network_t_plus_one_step = step;
        true
    }

    /// Gets network t+1 step marker for round.
    pub fn network_t_plus_one_step(&self, period: u64, round: u64) -> Option<u64> {
        self.get_round(period, round)
            .map(|round_votes| round_votes.network_t_plus_one_step)
    }

    /// Applies deterministic t+1 / 2t+1 threshold decisions for one vote.
    ///
    /// Inputs:
    /// - `vote`: vote metadata that identifies (`period`, `round`, `step`,
    ///   type, voted block hash).
    /// - `total_weight`: aggregated voted-value weight for this vote's bucket.
    /// - `two_t_plus_one_threshold`: threshold that defines 2t+1.
    ///
    /// Behavior:
    /// - Computes `t_plus_one = ((2t+1 - 1) / 2) + 1`.
    /// - For `next` votes with `total_weight >= t_plus_one`, updates round
    ///   `network_t_plus_one_step` only when `vote.step` is strictly greater
    ///   than current marker and round exists.
    /// - For votes with `total_weight >= 2t+1`, derives marker kind from vote
    ///   type/hash and inserts 2t+1 mapping with first-writer-wins semantics.
    ///
    /// Returns a structured decision summary used by the C++ caller for side
    /// effects (logs/database writes) while Rust owns consensus marker state.
    pub fn apply_threshold_decision(
        &mut self,
        vote: &VerifiedVote,
        total_weight: u64,
        two_t_plus_one_threshold: u64,
    ) -> Result<ThresholdDecisionOutcome> {
        if two_t_plus_one_threshold == 0 {
            return Err(anyhow!(
                "2t+1 threshold cannot be zero for period {}, round {}, step {}",
                vote.period,
                vote.round,
                vote.step
            ));
        }

        let t_plus_one = ((two_t_plus_one_threshold - 1) / 2) + 1;
        let t_plus_one_reached = vote.vote_type == PbftVoteType::Next && total_weight >= t_plus_one;
        let current_marker = self
            .network_t_plus_one_step(vote.period, vote.round)
            .unwrap_or_default();
        let network_t_plus_one_step_updated = t_plus_one_reached
            && vote.step > current_marker
            && self.set_network_t_plus_one_step(vote.period, vote.round, vote.step);

        if total_weight < two_t_plus_one_threshold {
            return Ok(ThresholdDecisionOutcome {
                t_plus_one_reached,
                network_t_plus_one_step_updated,
                two_t_plus_one_reached: false,
                two_t_plus_one_kind: None,
                two_t_plus_one_insert_outcome: None,
            });
        }

        let Some(kind) = Self::two_t_plus_one_kind_from_vote(vote) else {
            return Ok(ThresholdDecisionOutcome {
                t_plus_one_reached,
                network_t_plus_one_step_updated,
                two_t_plus_one_reached: true,
                two_t_plus_one_kind: None,
                two_t_plus_one_insert_outcome: None,
            });
        };

        let insert_outcome = self.insert_two_t_plus_one_voted_block(
            vote.period,
            vote.round,
            kind,
            vote.block_hash,
            vote.step,
        );

        Ok(ThresholdDecisionOutcome {
            t_plus_one_reached,
            network_t_plus_one_step_updated,
            two_t_plus_one_reached: true,
            two_t_plus_one_kind: Some(kind),
            two_t_plus_one_insert_outcome: Some(insert_outcome),
        })
    }

    /// Inserts one 2t+1 voted-block mapping for existing round.
    ///
    /// Missing rounds are rejected without side effects. Existing mappings for
    /// the same key are preserved (first-writer-wins) and never overwritten.
    pub fn insert_two_t_plus_one_voted_block(
        &mut self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
        block_hash: H256,
        step: u64,
    ) -> TwoTPlusOneInsertOutcome {
        let Some(round_votes) = self
            .votes
            .get_mut(&period)
            .and_then(|rounds| rounds.get_mut(&round))
        else {
            return TwoTPlusOneInsertOutcome {
                round_found: false,
                inserted: false,
            };
        };

        if round_votes.two_t_plus_one_voted_blocks.contains_key(&kind) {
            return TwoTPlusOneInsertOutcome {
                round_found: true,
                inserted: false,
            };
        }

        round_votes.two_t_plus_one_voted_blocks.insert(
            kind,
            VotedBlock {
                hash: block_hash,
                step,
            },
        );

        TwoTPlusOneInsertOutcome {
            round_found: true,
            inserted: true,
        }
    }

    /// Returns one 2t+1 voted-block mapping.
    pub fn get_two_t_plus_one_voted_block(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Option<VotedBlock> {
        self.get_round(period, round)
            .and_then(|round_votes| round_votes.two_t_plus_one_voted_blocks.get(&kind).copied())
    }

    /// Returns vote hashes that correspond to the mapped 2t+1 voted block.
    pub fn get_two_t_plus_one_voted_block_vote_hashes(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Vec<H256> {
        let Some(voted_block) = self.get_two_t_plus_one_voted_block(period, round, kind) else {
            return Vec::new();
        };

        self.get_round(period, round)
            .and_then(|round_votes| round_votes.step_votes.get(&voted_block.step))
            .and_then(|step_votes| step_votes.votes.get(&voted_block.hash))
            .map(|votes_with_weight| votes_with_weight.votes.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Removes all periods `< pbft_period`.
    pub fn cleanup_votes_by_period(&mut self, pbft_period: u64) {
        let stale_periods: Vec<u64> = self
            .votes
            .keys()
            .copied()
            .take_while(|period| *period < pbft_period)
            .collect();

        for period in stale_periods {
            self.votes.remove(&period);
        }
    }

    /// Determines next round from next-vote 2t+1 mappings for `period`.
    ///
    /// Behavior mirrors legacy C++:
    /// - scan rounds from highest to lowest,
    /// - stop with no decision once rounds are below `current_round`,
    /// - prefer `NextVotedBlock` over `NextVotedNullBlock` in the same round.
    pub fn determine_new_round(
        &self,
        period: u64,
        current_round: u64,
    ) -> Option<DetermineNewRoundOutcome> {
        let rounds = self.votes.get(&period)?;
        for (&round, round_votes) in rounds.iter().rev() {
            if round < current_round {
                return None;
            }

            if let Some(voted) = round_votes
                .two_t_plus_one_voted_blocks
                .get(&TwoTPlusOneVotedBlockType::NextVotedBlock)
            {
                let new_round = round.checked_add(1)?;
                return Some(DetermineNewRoundOutcome {
                    new_round,
                    source_round: round,
                    source_kind: TwoTPlusOneVotedBlockType::NextVotedBlock,
                    block_hash: voted.hash,
                    step: voted.step,
                });
            }

            if let Some(voted) = round_votes
                .two_t_plus_one_voted_blocks
                .get(&TwoTPlusOneVotedBlockType::NextVotedNullBlock)
            {
                let new_round = round.checked_add(1)?;
                return Some(DetermineNewRoundOutcome {
                    new_round,
                    source_round: round,
                    source_kind: TwoTPlusOneVotedBlockType::NextVotedNullBlock,
                    block_hash: voted.hash,
                    step: voted.step,
                });
            }
        }

        None
    }

    fn ensure_step_mut(&mut self, period: u64, round: u64, step: u64) -> &mut StepVotes {
        self.votes
            .entry(period)
            .or_default()
            .entry(round)
            .or_default()
            .step_votes
            .entry(step)
            .or_default()
    }

    fn get_round(&self, period: u64, round: u64) -> Option<&RoundVerifiedVotes> {
        self.votes
            .get(&period)
            .and_then(|rounds| rounds.get(&round))
    }

    fn can_insert_secondary_next_vote(existing: &UniqueVoterVotes, vote: &VerifiedVote) -> bool {
        if vote.vote_type != PbftVoteType::Next
            || vote.step.is_multiple_of(2)
            || existing.secondary.is_some()
        {
            return false;
        }

        let existing_is_null = existing.primary.block_hash == H256::zero();
        let incoming_is_null = vote.block_hash == H256::zero();
        existing_is_null != incoming_is_null
    }

    fn two_t_plus_one_kind_from_vote(vote: &VerifiedVote) -> Option<TwoTPlusOneVotedBlockType> {
        match vote.vote_type {
            PbftVoteType::Soft => Some(TwoTPlusOneVotedBlockType::SoftVotedBlock),
            PbftVoteType::Cert => Some(TwoTPlusOneVotedBlockType::CertVotedBlock),
            PbftVoteType::Next if vote.block_hash == H256::zero() => {
                Some(TwoTPlusOneVotedBlockType::NextVotedNullBlock)
            }
            PbftVoteType::Next => Some(TwoTPlusOneVotedBlockType::NextVotedBlock),
            PbftVoteType::Invalid | PbftVoteType::Propose => None,
        }
    }

    fn snapshot_unique_voter_rollback_state(
        &self,
        period: u64,
        round: u64,
        step: u64,
        voter: H160,
    ) -> (u64, u64, u64, H160, UniqueVoterRollbackState) {
        let had_period = self.votes.contains_key(&period);
        let had_round = self
            .votes
            .get(&period)
            .map(|rounds| rounds.contains_key(&round))
            .unwrap_or(false);
        let had_step = self
            .votes
            .get(&period)
            .and_then(|rounds| rounds.get(&round))
            .map(|round_votes| round_votes.step_votes.contains_key(&step))
            .unwrap_or(false);
        let previous_vote = self
            .votes
            .get(&period)
            .and_then(|rounds| rounds.get(&round))
            .and_then(|round_votes| round_votes.step_votes.get(&step))
            .and_then(|step_votes| step_votes.unique_voters.get(&voter))
            .cloned();

        (
            period,
            round,
            step,
            voter,
            UniqueVoterRollbackState {
                had_period,
                had_round,
                had_step,
                previous_vote,
            },
        )
    }

    #[allow(clippy::collapsible_if)]
    fn restore_unique_voter_state(
        &mut self,
        period: u64,
        round: u64,
        step: u64,
        voter: H160,
        rollback: UniqueVoterRollbackState,
    ) {
        if let Some(previous_vote) = rollback.previous_vote {
            self.ensure_step_mut(period, round, step)
                .unique_voters
                .insert(voter, previous_vote);
            return;
        }

        if let Some(step_votes) = self
            .votes
            .get_mut(&period)
            .and_then(|rounds| rounds.get_mut(&round))
            .and_then(|round_votes| round_votes.step_votes.get_mut(&step))
        {
            step_votes.unique_voters.remove(&voter);
        }

        if !rollback.had_step {
            if let Some(round_votes) = self
                .votes
                .get_mut(&period)
                .and_then(|rounds| rounds.get_mut(&round))
            {
                let remove_step = round_votes
                    .step_votes
                    .get(&step)
                    .map(|step_votes| {
                        step_votes.votes.is_empty() && step_votes.unique_voters.is_empty()
                    })
                    .unwrap_or(false);
                if remove_step {
                    round_votes.step_votes.remove(&step);
                }
            }
        }

        if !rollback.had_round {
            if let Some(rounds) = self.votes.get_mut(&period) {
                let remove_round = rounds
                    .get(&round)
                    .map(|round_votes| {
                        round_votes.step_votes.is_empty()
                            && round_votes.two_t_plus_one_voted_blocks.is_empty()
                            && round_votes.network_t_plus_one_step == 0
                    })
                    .unwrap_or(false);
                if remove_round {
                    rounds.remove(&round);
                }
            }
        }

        if !rollback.had_period {
            let remove_period = self
                .votes
                .get(&period)
                .map(|rounds| rounds.is_empty())
                .unwrap_or(false);
            if remove_period {
                self.votes.remove(&period);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h256(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    fn h160(v: u64) -> H160 {
        H160::from_low_u64_be(v)
    }

    #[allow(clippy::too_many_arguments)]
    fn vote(
        vote_hash: u64,
        block_hash: u64,
        voter: u64,
        period: u64,
        round: u64,
        step: u64,
        vote_type: PbftVoteType,
        weight: u64,
    ) -> VerifiedVote {
        VerifiedVote::new(
            h256(vote_hash),
            h256(block_hash),
            h160(voter),
            period,
            round,
            step,
            vote_type,
            weight,
        )
        .unwrap()
    }

    #[test]
    fn unique_voter_allows_odd_step_next_vote_pair_null_and_non_null() {
        let mut verified = VerifiedVotes::new();
        let first = vote(1, 0, 42, 7, 3, 5, PbftVoteType::Next, 1);
        let second = vote(2, 99, 42, 7, 3, 5, PbftVoteType::Next, 1);
        let third = vote(3, 100, 42, 7, 3, 5, PbftVoteType::Next, 1);

        let first_outcome = verified.insert_unique_voter(&first);
        assert!(first_outcome.accepted);
        assert!(!first_outcome.used_secondary_slot);

        let second_outcome = verified.insert_unique_voter(&second);
        assert!(second_outcome.accepted);
        assert!(second_outcome.used_secondary_slot);

        let third_check = verified.check_unique_voter(&third);
        assert!(!third_check.is_unique);
        assert_eq!(third_check.conflicting_vote_hash, Some(second.vote_hash));

        let third_outcome = verified.insert_unique_voter(&third);
        assert!(!third_outcome.accepted);
        assert_eq!(third_outcome.conflicting_vote_hash, Some(second.vote_hash));
    }

    #[test]
    fn unique_voter_rejects_second_next_vote_on_even_step() {
        let mut verified = VerifiedVotes::new();
        let first = vote(1, 0, 42, 7, 3, 4, PbftVoteType::Next, 1);
        let second = vote(2, 99, 42, 7, 3, 4, PbftVoteType::Next, 1);

        assert!(verified.insert_unique_voter(&first).accepted);
        let second_outcome = verified.insert_unique_voter(&second);
        assert!(!second_outcome.accepted);
        assert_eq!(second_outcome.conflicting_vote_hash, Some(first.vote_hash));
    }

    #[test]
    fn voted_value_accumulates_weight_and_is_idempotent_by_vote_hash() {
        let mut verified = VerifiedVotes::new();
        let first = vote(1, 9, 11, 2, 1, 3, PbftVoteType::Cert, 2);
        let second = vote(2, 9, 12, 2, 1, 3, PbftVoteType::Cert, 3);

        let first_outcome = verified.insert_voted_value(first.clone()).unwrap();
        assert!(first_outcome.inserted);
        assert_eq!(first_outcome.total_weight, 2);
        assert_eq!(first_outcome.votes_count, 1);

        let second_outcome = verified.insert_voted_value(second.clone()).unwrap();
        assert!(second_outcome.inserted);
        assert_eq!(second_outcome.total_weight, 5);
        assert_eq!(second_outcome.votes_count, 2);

        let duplicate = verified.insert_voted_value(second).unwrap();
        assert!(!duplicate.inserted);
        assert_eq!(duplicate.total_weight, 5);
        assert_eq!(duplicate.votes_count, 2);

        assert!(verified.vote_in_verified_map(2, 1, 3, h256(9), h256(1)));
        assert!(verified.vote_in_verified_map(2, 1, 3, h256(9), h256(2)));
        assert!(!verified.vote_in_verified_map(2, 1, 3, h256(9), h256(3)));
    }

    #[test]
    fn atomic_insert_preserves_odd_step_next_dual_vote_and_conflict_hash() {
        let mut verified = VerifiedVotes::new();
        let first = vote(1, 0, 42, 7, 3, 5, PbftVoteType::Next, 2);
        let second = vote(2, 99, 42, 7, 3, 5, PbftVoteType::Next, 3);
        let third = vote(3, 100, 42, 7, 3, 5, PbftVoteType::Next, 1);

        let first_outcome = verified.insert_vote_atomic(first.clone()).unwrap();
        assert!(first_outcome.inserted);
        assert_eq!(first_outcome.total_weight, 2);
        assert_eq!(first_outcome.votes_count, 1);
        assert_eq!(first_outcome.conflicting_vote_hash, None);

        let second_outcome = verified.insert_vote_atomic(second.clone()).unwrap();
        assert!(second_outcome.inserted);
        assert_eq!(second_outcome.total_weight, 3);
        assert_eq!(second_outcome.votes_count, 1);
        assert!(second_outcome.used_secondary_slot);
        assert_eq!(second_outcome.conflicting_vote_hash, None);

        let third_outcome = verified.insert_vote_atomic(third).unwrap();
        assert!(!third_outcome.inserted);
        assert_eq!(third_outcome.conflicting_vote_hash, Some(second.vote_hash));
    }

    #[test]
    fn atomic_insert_rolls_back_unique_voter_on_weight_overflow() {
        let mut verified = VerifiedVotes::new();
        let first = vote(10, 88, 11, 9, 4, 3, PbftVoteType::Cert, u64::MAX);
        let second = vote(11, 88, 12, 9, 4, 3, PbftVoteType::Cert, 1);
        let probe = vote(12, 77, 12, 9, 4, 3, PbftVoteType::Cert, 1);

        let first_outcome = verified.insert_vote_atomic(first).unwrap();
        assert!(first_outcome.inserted);

        let err = verified.insert_vote_atomic(second).unwrap_err();
        assert!(
            err.to_string()
                .contains("vote weight overflow for period 9, round 4, step 3")
        );

        let check_after_rollback = verified.check_unique_voter(&probe);
        assert!(check_after_rollback.is_unique);
    }

    #[test]
    fn two_t_plus_one_lookup_returns_mapped_votes_for_step_and_block() {
        let mut verified = VerifiedVotes::new();
        let first = vote(1, 55, 10, 4, 2, 3, PbftVoteType::Cert, 2);
        let second = vote(2, 55, 11, 4, 2, 3, PbftVoteType::Cert, 3);

        verified.insert_voted_value(first.clone()).unwrap();
        verified.insert_voted_value(second.clone()).unwrap();

        let insert_outcome = verified.insert_two_t_plus_one_voted_block(
            4,
            2,
            TwoTPlusOneVotedBlockType::CertVotedBlock,
            h256(55),
            3,
        );
        assert!(insert_outcome.round_found);
        assert!(insert_outcome.inserted);

        let mapped = verified
            .get_two_t_plus_one_voted_block(4, 2, TwoTPlusOneVotedBlockType::CertVotedBlock)
            .unwrap();
        assert_eq!(mapped.hash, h256(55));
        assert_eq!(mapped.step, 3);

        let votes = verified.get_two_t_plus_one_voted_block_vote_hashes(
            4,
            2,
            TwoTPlusOneVotedBlockType::CertVotedBlock,
        );
        assert_eq!(votes, vec![h256(1), h256(2)]);
    }

    #[test]
    fn two_t_plus_one_insert_is_first_writer_wins_and_reports_existing_mapping() {
        let mut verified = VerifiedVotes::new();

        verified
            .insert_voted_value(vote(1, 111, 10, 8, 4, 3, PbftVoteType::Cert, 1))
            .unwrap();
        verified
            .insert_voted_value(vote(2, 222, 11, 8, 4, 7, PbftVoteType::Cert, 1))
            .unwrap();

        let first_insert = verified.insert_two_t_plus_one_voted_block(
            8,
            4,
            TwoTPlusOneVotedBlockType::CertVotedBlock,
            h256(111),
            3,
        );
        assert!(first_insert.round_found);
        assert!(first_insert.inserted);

        let second_insert = verified.insert_two_t_plus_one_voted_block(
            8,
            4,
            TwoTPlusOneVotedBlockType::CertVotedBlock,
            h256(222),
            7,
        );
        assert!(second_insert.round_found);
        assert!(!second_insert.inserted);

        let mapped = verified
            .get_two_t_plus_one_voted_block(8, 4, TwoTPlusOneVotedBlockType::CertVotedBlock)
            .unwrap();
        assert_eq!(mapped.hash, h256(111));
        assert_eq!(mapped.step, 3);
    }

    #[test]
    fn two_t_plus_one_insert_reports_missing_round_without_side_effects() {
        let mut verified = VerifiedVotes::new();

        let outcome = verified.insert_two_t_plus_one_voted_block(
            99,
            1,
            TwoTPlusOneVotedBlockType::SoftVotedBlock,
            h256(55),
            5,
        );
        assert!(!outcome.round_found);
        assert!(!outcome.inserted);
        assert_eq!(
            verified.get_two_t_plus_one_voted_block(
                99,
                1,
                TwoTPlusOneVotedBlockType::SoftVotedBlock
            ),
            None
        );
    }

    #[test]
    fn threshold_decision_next_vote_updates_t_plus_one_and_inserts_next_marker() {
        let mut verified = VerifiedVotes::new();
        let next_vote = vote(1, 77, 10, 5, 2, 9, PbftVoteType::Next, 1);
        verified.insert_voted_value(next_vote.clone()).unwrap();

        let outcome = verified.apply_threshold_decision(&next_vote, 5, 5).unwrap();

        assert!(outcome.t_plus_one_reached);
        assert!(outcome.network_t_plus_one_step_updated);
        assert!(outcome.two_t_plus_one_reached);
        assert_eq!(
            outcome.two_t_plus_one_kind,
            Some(TwoTPlusOneVotedBlockType::NextVotedBlock)
        );
        assert_eq!(
            outcome.two_t_plus_one_insert_outcome,
            Some(TwoTPlusOneInsertOutcome {
                round_found: true,
                inserted: true
            })
        );
        assert_eq!(verified.network_t_plus_one_step(5, 2), Some(9));
        assert_eq!(
            verified.get_two_t_plus_one_voted_block(
                5,
                2,
                TwoTPlusOneVotedBlockType::NextVotedBlock
            ),
            Some(VotedBlock {
                hash: h256(77),
                step: 9
            })
        );
    }

    #[test]
    fn threshold_decision_non_next_vote_skips_t_plus_one_and_sets_two_t_plus_one() {
        let mut verified = VerifiedVotes::new();
        let cert_vote = vote(2, 91, 11, 6, 3, 8, PbftVoteType::Cert, 1);
        verified.insert_voted_value(cert_vote.clone()).unwrap();

        let outcome = verified.apply_threshold_decision(&cert_vote, 7, 7).unwrap();

        assert!(!outcome.t_plus_one_reached);
        assert!(!outcome.network_t_plus_one_step_updated);
        assert!(outcome.two_t_plus_one_reached);
        assert_eq!(
            outcome.two_t_plus_one_kind,
            Some(TwoTPlusOneVotedBlockType::CertVotedBlock)
        );
        assert_eq!(
            outcome.two_t_plus_one_insert_outcome,
            Some(TwoTPlusOneInsertOutcome {
                round_found: true,
                inserted: true
            })
        );
        assert_eq!(verified.network_t_plus_one_step(6, 3), Some(0));
    }

    #[test]
    fn threshold_decision_below_two_t_plus_one_only_applies_t_plus_one_for_next_votes() {
        let mut verified = VerifiedVotes::new();
        let next_vote = vote(3, 101, 12, 7, 4, 6, PbftVoteType::Next, 1);
        verified.insert_voted_value(next_vote.clone()).unwrap();

        let outcome = verified.apply_threshold_decision(&next_vote, 4, 7).unwrap();

        assert!(outcome.t_plus_one_reached);
        assert!(outcome.network_t_plus_one_step_updated);
        assert!(!outcome.two_t_plus_one_reached);
        assert_eq!(outcome.two_t_plus_one_kind, None);
        assert_eq!(outcome.two_t_plus_one_insert_outcome, None);
        assert_eq!(
            verified.get_two_t_plus_one_voted_block(
                7,
                4,
                TwoTPlusOneVotedBlockType::NextVotedBlock
            ),
            None
        );
    }

    #[test]
    fn threshold_decision_preserves_existing_markers_when_already_set() {
        let mut verified = VerifiedVotes::new();
        let existing_vote = vote(4, 202, 21, 8, 5, 10, PbftVoteType::Next, 1);
        let candidate_vote = vote(5, 202, 22, 8, 5, 8, PbftVoteType::Next, 1);
        verified.insert_voted_value(existing_vote.clone()).unwrap();
        verified.insert_voted_value(candidate_vote.clone()).unwrap();

        assert!(verified.set_network_t_plus_one_step(8, 5, 10));
        let inserted = verified.insert_two_t_plus_one_voted_block(
            8,
            5,
            TwoTPlusOneVotedBlockType::NextVotedBlock,
            existing_vote.block_hash,
            existing_vote.step,
        );
        assert!(inserted.round_found);
        assert!(inserted.inserted);

        let outcome = verified
            .apply_threshold_decision(&candidate_vote, 5, 5)
            .unwrap();

        assert!(outcome.t_plus_one_reached);
        assert!(!outcome.network_t_plus_one_step_updated);
        assert!(outcome.two_t_plus_one_reached);
        assert_eq!(
            outcome.two_t_plus_one_insert_outcome,
            Some(TwoTPlusOneInsertOutcome {
                round_found: true,
                inserted: false
            })
        );
        assert_eq!(verified.network_t_plus_one_step(8, 5), Some(10));
        assert_eq!(
            verified.get_two_t_plus_one_voted_block(
                8,
                5,
                TwoTPlusOneVotedBlockType::NextVotedBlock
            ),
            Some(VotedBlock {
                hash: h256(202),
                step: 10
            })
        );
    }

    #[test]
    fn threshold_decision_reports_missing_round_without_state_changes() {
        let mut verified = VerifiedVotes::new();
        let next_vote = vote(6, 303, 31, 9, 6, 3, PbftVoteType::Next, 1);

        let outcome = verified.apply_threshold_decision(&next_vote, 5, 5).unwrap();

        assert!(outcome.t_plus_one_reached);
        assert!(!outcome.network_t_plus_one_step_updated);
        assert!(outcome.two_t_plus_one_reached);
        assert_eq!(
            outcome.two_t_plus_one_kind,
            Some(TwoTPlusOneVotedBlockType::NextVotedBlock)
        );
        assert_eq!(
            outcome.two_t_plus_one_insert_outcome,
            Some(TwoTPlusOneInsertOutcome {
                round_found: false,
                inserted: false
            })
        );
        assert_eq!(verified.network_t_plus_one_step(9, 6), None);
    }

    #[test]
    fn determine_new_round_prefers_next_block_over_next_null() {
        let mut verified = VerifiedVotes::new();
        verified
            .insert_voted_value(vote(1, 11, 101, 5, 3, 4, PbftVoteType::Next, 1))
            .unwrap();
        verified
            .insert_voted_value(vote(2, 0, 102, 5, 3, 5, PbftVoteType::Next, 1))
            .unwrap();
        assert!(
            verified
                .insert_two_t_plus_one_voted_block(
                    5,
                    3,
                    TwoTPlusOneVotedBlockType::NextVotedBlock,
                    h256(11),
                    4
                )
                .inserted
        );
        assert!(
            verified
                .insert_two_t_plus_one_voted_block(
                    5,
                    3,
                    TwoTPlusOneVotedBlockType::NextVotedNullBlock,
                    h256(0),
                    5
                )
                .inserted
        );

        let decision = verified.determine_new_round(5, 3).unwrap();
        assert_eq!(decision.new_round, 4);
        assert_eq!(decision.source_round, 3);
        assert_eq!(
            decision.source_kind,
            TwoTPlusOneVotedBlockType::NextVotedBlock
        );
        assert_eq!(decision.block_hash, h256(11));
        assert_eq!(decision.step, 4);
    }

    #[test]
    fn determine_new_round_ignores_rounds_below_current() {
        let mut verified = VerifiedVotes::new();
        verified
            .insert_voted_value(vote(1, 21, 201, 7, 2, 4, PbftVoteType::Next, 1))
            .unwrap();
        assert!(
            verified
                .insert_two_t_plus_one_voted_block(
                    7,
                    2,
                    TwoTPlusOneVotedBlockType::NextVotedBlock,
                    h256(21),
                    4
                )
                .inserted
        );

        assert!(verified.determine_new_round(7, 3).is_none());
    }

    #[test]
    fn network_t_plus_one_step_and_cleanup_follow_round_and_period_boundaries() {
        let mut verified = VerifiedVotes::new();

        assert!(!verified.set_network_t_plus_one_step(9, 1, 7));
        assert_eq!(verified.network_t_plus_one_step(9, 1), None);

        verified
            .insert_voted_value(vote(1, 5, 1, 9, 1, 3, PbftVoteType::Cert, 1))
            .unwrap();
        assert!(verified.set_network_t_plus_one_step(9, 1, 7));
        assert_eq!(verified.network_t_plus_one_step(9, 1), Some(7));

        verified
            .insert_voted_value(vote(2, 6, 2, 10, 1, 3, PbftVoteType::Cert, 1))
            .unwrap();
        verified.cleanup_votes_by_period(10);

        assert_eq!(verified.network_t_plus_one_step(9, 1), None);
        assert_eq!(verified.network_t_plus_one_step(10, 1), Some(0));
    }

    #[test]
    fn snapshots_are_deterministic() {
        let mut verified = VerifiedVotes::new();
        verified
            .insert_voted_value(vote(10, 100, 1, 3, 2, 5, PbftVoteType::Next, 1))
            .unwrap();
        verified
            .insert_voted_value(vote(9, 100, 2, 3, 2, 5, PbftVoteType::Next, 1))
            .unwrap();
        verified
            .insert_voted_value(vote(20, 200, 3, 2, 1, 3, PbftVoteType::Cert, 1))
            .unwrap();

        let votes = verified.snapshot_votes();
        let hashes: Vec<H256> = votes.into_iter().map(|v| v.vote_hash).collect();
        assert_eq!(hashes, vec![h256(20), h256(9), h256(10)]);

        verified.insert_two_t_plus_one_voted_block(
            2,
            1,
            TwoTPlusOneVotedBlockType::CertVotedBlock,
            h256(200),
            3,
        );
        verified.insert_two_t_plus_one_voted_block(
            3,
            2,
            TwoTPlusOneVotedBlockType::NextVotedBlock,
            h256(100),
            5,
        );

        let snapshot = verified.snapshot_two_t_plus_one();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].period, 2);
        assert_eq!(snapshot[0].round, 1);
        assert_eq!(snapshot[1].period, 3);
        assert_eq!(snapshot[1].round, 2);
    }

    #[test]
    fn get_step_votes_returns_deterministic_bucket_snapshot() {
        let mut verified = VerifiedVotes::new();

        let first = vote(10, 77, 1, 1, 1, 5, PbftVoteType::Cert, 1);
        let second = vote(11, 77, 2, 1, 1, 5, PbftVoteType::Cert, 2);
        let third = vote(12, 88, 3, 1, 1, 5, PbftVoteType::Cert, 3);

        verified.insert_voted_value(first).unwrap();
        verified.insert_voted_value(second).unwrap();
        verified.insert_voted_value(third).unwrap();

        let step_votes = verified.get_step_votes(1, 1, 5).expect("step should exist");

        assert_eq!(step_votes.len(), 2);
        assert_eq!(step_votes[0].block_hash, h256(77));
        assert_eq!(step_votes[0].total_weight, 3);
        assert_eq!(step_votes[0].vote_hashes, vec![h256(10), h256(11)]);
        assert_eq!(step_votes[1].block_hash, h256(88));
        assert_eq!(step_votes[1].total_weight, 3);
        assert_eq!(step_votes[1].vote_hashes, vec![h256(12)]);
    }

    #[test]
    fn add_verified_vote_can_apply_threshold_decisions() {
        let mut verified = VerifiedVotes::new();

        let soft_a = vote(1, 44, 1, 2, 1, 2, PbftVoteType::Next, 2);
        let soft_b = vote(2, 44, 2, 2, 1, 2, PbftVoteType::Next, 3);

        let below = verified
            .add_verified_vote(soft_a, Some(3))
            .expect("add should insert vote");
        assert!(below.inserted);
        assert!(below.threshold_decision.is_some());
        let below_decision = below.threshold_decision.expect("threshold decision");
        assert_eq!(below_decision.t_plus_one_reached, true);
        assert_eq!(below_decision.two_t_plus_one_reached, false);

        let above = verified
            .add_verified_vote(soft_b, Some(5))
            .expect("add should insert vote and reach threshold");
        assert!(above.inserted);
        assert!(above.threshold_decision.is_some());
        let above_decision = above.threshold_decision.expect("threshold decision");
        assert_eq!(above_decision.two_t_plus_one_reached, true);
        assert!(above_decision.two_t_plus_one_insert_outcome.is_some());
        assert_eq!(
            above_decision.two_t_plus_one_kind,
            Some(TwoTPlusOneVotedBlockType::NextVotedBlock)
        );
    }
}
