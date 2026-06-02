//! Deterministic PBFT reward-vote selection planning.
//!
//! Reward votes are the certified votes from the previous finalized PBFT block
//! that a proposer references in the next PBFT block. This planner owns the
//! deterministic lookup decision from compact verified-vote facts: first search
//! the locally recorded reward round, then search all known rounds in reverse
//! order for the same reward period and block hash. It does not own
//! `VerifiedVotes`, parse vote RLP, materialize C++ `PbftVote` sidecars, read
//! storage, or mutate reward metadata.

use std::collections::HashSet;

use ethereum_types::H256;

/// Stable reward-vote selection status.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftRewardVotesStatus {
    /// Period-one PBFT block does not need reward votes.
    FirstPeriod,
    /// All requested reward-vote hashes were found.
    Accepted,
    /// Preferred reward round facts were not available.
    MissingPreferredRound,
    /// No round facts were available for the reward period.
    MissingRewardPeriod,
    /// A candidate round did not contain the cert-vote step.
    MissingCertStep,
    /// A candidate cert-vote step did not contain the reward block hash.
    MissingRewardBlock,
    /// At least one requested reward-vote hash was missing from all candidate rounds.
    MissingRewardVote,
}

impl PbftRewardVotesStatus {
    /// Stable numeric status for CXX bridge payloads and tests.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::FirstPeriod => 0,
            Self::Accepted => 1,
            Self::MissingPreferredRound => 2,
            Self::MissingRewardPeriod => 3,
            Self::MissingCertStep => 4,
            Self::MissingRewardBlock => 5,
            Self::MissingRewardVote => 6,
        }
    }
}

/// Compact membership facts for one reward-vote candidate round.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftRewardVoteRoundCandidate {
    /// PBFT round represented by this snapshot.
    pub round: u64,
    /// Whether the round has a cert-vote step bucket.
    pub has_cert_step: bool,
    /// Whether the cert-vote step has a bucket for the expected reward block.
    pub has_reward_block: bool,
    /// Vote hashes present in the expected reward block bucket.
    pub vote_hashes: Vec<H256>,
}

/// Input facts for PBFT reward-vote selection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftRewardVoteSelectionFact {
    /// Period of the PBFT block being checked.
    pub block_period: u64,
    /// Previous finalized reward-vote period recorded by VoteManager.
    pub reward_period: u64,
    /// Preferred reward-vote round recorded by VoteManager.
    pub preferred_reward_round: u64,
    /// Previous finalized reward block hash.
    pub reward_block_hash: H256,
    /// Reward-vote hashes listed by the PBFT block being checked.
    pub requested_vote_hashes: Vec<H256>,
    /// Whether the preferred round snapshot exists.
    pub has_preferred_round: bool,
    /// Preferred round snapshot when available.
    pub preferred_round: PbftRewardVoteRoundCandidate,
    /// Whether any round snapshots exist for the reward period.
    pub has_reward_period: bool,
    /// Candidate round snapshots in caller-supplied search order. The C++ shim
    /// preserves legacy reverse-round order.
    pub period_rounds: Vec<PbftRewardVoteRoundCandidate>,
}

/// Deterministic output of PBFT reward-vote selection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftRewardVoteSelectionPlan {
    /// True when the reward-vote reference is valid.
    pub accepted: bool,
    /// Stable status code.
    pub status: PbftRewardVotesStatus,
    /// Reward period used for lookup.
    pub selected_period: u64,
    /// Round that satisfied the requested vote hashes.
    pub selected_round: u64,
    /// Reward block hash used for lookup.
    pub selected_block_hash: H256,
    /// Requested vote hashes in PBFT-block order when accepted.
    pub selected_vote_hashes: Vec<H256>,
    /// First missing requested vote hash when status is `MissingRewardVote`.
    pub missing_vote_hash: Option<H256>,
}

impl PbftRewardVoteSelectionPlan {
    fn rejected(
        fact: &PbftRewardVoteSelectionFact,
        status: PbftRewardVotesStatus,
        selected_round: u64,
        missing_vote_hash: Option<H256>,
    ) -> Self {
        Self {
            accepted: false,
            status,
            selected_period: fact.reward_period,
            selected_round,
            selected_block_hash: fact.reward_block_hash,
            selected_vote_hashes: Vec::new(),
            missing_vote_hash,
        }
    }
}

/// Selects PBFT reward votes from compact verified-vote membership facts.
///
/// Inputs:
/// - `fact`: PBFT block reward-vote references plus live verified-vote
///   membership snapshots for the preferred round and all known period rounds.
///
/// Outputs:
/// - An accepted plan carrying the selected round and requested vote hashes, or
///   a stable rejection status naming the first missing fact class.
///
/// Invariants and edge behavior:
/// - PBFT period 1 accepts immediately and does not require round facts.
/// - Empty requested-vote lists are accepted when the relevant candidate round
///   and reward-block bucket exist, matching membership semantics for "all
///   requested hashes are present".
/// - Search order after the preferred round is caller-supplied so the C++ shim
///   can preserve legacy reverse-round lookup order while future Rust indexes
///   can provide the same order directly.
#[must_use]
pub fn plan_pbft_reward_votes(fact: PbftRewardVoteSelectionFact) -> PbftRewardVoteSelectionPlan {
    if fact.block_period == 1 {
        return PbftRewardVoteSelectionPlan {
            accepted: true,
            status: PbftRewardVotesStatus::FirstPeriod,
            selected_period: fact.reward_period,
            selected_round: fact.preferred_reward_round,
            selected_block_hash: fact.reward_block_hash,
            selected_vote_hashes: Vec::new(),
            missing_vote_hash: None,
        };
    }

    if !fact.has_preferred_round {
        return PbftRewardVoteSelectionPlan::rejected(
            &fact,
            PbftRewardVotesStatus::MissingPreferredRound,
            fact.preferred_reward_round,
            None,
        );
    }

    match candidate_matches(&fact.preferred_round, &fact.requested_vote_hashes) {
        CandidateMatch::Accepted => {
            return PbftRewardVoteSelectionPlan {
                accepted: true,
                status: PbftRewardVotesStatus::Accepted,
                selected_period: fact.reward_period,
                selected_round: fact.preferred_round.round,
                selected_block_hash: fact.reward_block_hash,
                selected_vote_hashes: fact.requested_vote_hashes,
                missing_vote_hash: None,
            };
        }
        CandidateMatch::MissingCertStep
        | CandidateMatch::MissingRewardBlock
        | CandidateMatch::MissingVote(_) => {}
    }

    if !fact.has_reward_period {
        return PbftRewardVoteSelectionPlan::rejected(
            &fact,
            PbftRewardVotesStatus::MissingRewardPeriod,
            fact.preferred_reward_round,
            None,
        );
    }

    let mut best_missing_status = None;
    let mut best_missing_round = fact.preferred_reward_round;
    let mut best_missing_vote = None;

    for round in &fact.period_rounds {
        match candidate_matches(round, &fact.requested_vote_hashes) {
            CandidateMatch::Accepted => {
                return PbftRewardVoteSelectionPlan {
                    accepted: true,
                    status: PbftRewardVotesStatus::Accepted,
                    selected_period: fact.reward_period,
                    selected_round: round.round,
                    selected_block_hash: fact.reward_block_hash,
                    selected_vote_hashes: fact.requested_vote_hashes,
                    missing_vote_hash: None,
                };
            }
            CandidateMatch::MissingCertStep => {
                if best_missing_status.is_none() {
                    best_missing_status = Some(PbftRewardVotesStatus::MissingCertStep);
                    best_missing_round = round.round;
                    best_missing_vote = None;
                }
            }
            CandidateMatch::MissingRewardBlock => {
                if !matches!(
                    best_missing_status,
                    Some(PbftRewardVotesStatus::MissingRewardVote)
                ) {
                    best_missing_status = Some(PbftRewardVotesStatus::MissingRewardBlock);
                    best_missing_round = round.round;
                    best_missing_vote = None;
                }
            }
            CandidateMatch::MissingVote(hash) => {
                best_missing_status = Some(PbftRewardVotesStatus::MissingRewardVote);
                best_missing_round = round.round;
                best_missing_vote = Some(hash);
            }
        }
    }

    PbftRewardVoteSelectionPlan::rejected(
        &fact,
        best_missing_status.unwrap_or(PbftRewardVotesStatus::MissingRewardVote),
        best_missing_round,
        best_missing_vote,
    )
}

enum CandidateMatch {
    Accepted,
    MissingCertStep,
    MissingRewardBlock,
    MissingVote(H256),
}

fn candidate_matches(
    candidate: &PbftRewardVoteRoundCandidate,
    requested: &[H256],
) -> CandidateMatch {
    if !candidate.has_cert_step {
        return CandidateMatch::MissingCertStep;
    }
    if !candidate.has_reward_block {
        return CandidateMatch::MissingRewardBlock;
    }

    let available: HashSet<H256> = candidate.vote_hashes.iter().copied().collect();
    for hash in requested {
        if !available.contains(hash) {
            return CandidateMatch::MissingVote(*hash);
        }
    }

    CandidateMatch::Accepted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: u8) -> H256 {
        H256::from([value; 32])
    }

    fn candidate(round: u64, hashes: Vec<H256>) -> PbftRewardVoteRoundCandidate {
        PbftRewardVoteRoundCandidate {
            round,
            has_cert_step: true,
            has_reward_block: true,
            vote_hashes: hashes,
        }
    }

    fn fact(requested: Vec<H256>) -> PbftRewardVoteSelectionFact {
        PbftRewardVoteSelectionFact {
            block_period: 10,
            reward_period: 9,
            preferred_reward_round: 2,
            reward_block_hash: hash(7),
            requested_vote_hashes: requested,
            has_preferred_round: true,
            preferred_round: candidate(2, vec![hash(1), hash(2)]),
            has_reward_period: true,
            period_rounds: vec![
                candidate(3, vec![hash(3)]),
                candidate(2, vec![hash(1), hash(2)]),
            ],
        }
    }

    #[test]
    fn first_period_accepts_without_reward_votes() {
        let mut fact = fact(vec![hash(1)]);
        fact.block_period = 1;
        fact.has_preferred_round = false;
        fact.has_reward_period = false;

        let plan = plan_pbft_reward_votes(fact);

        assert!(plan.accepted);
        assert_eq!(plan.status, PbftRewardVotesStatus::FirstPeriod);
        assert!(plan.selected_vote_hashes.is_empty());
    }

    #[test]
    fn accepts_preferred_round_when_all_hashes_present() {
        let plan = plan_pbft_reward_votes(fact(vec![hash(1), hash(2)]));

        assert!(plan.accepted);
        assert_eq!(plan.status, PbftRewardVotesStatus::Accepted);
        assert_eq!(plan.selected_round, 2);
        assert_eq!(plan.selected_vote_hashes, vec![hash(1), hash(2)]);
    }

    #[test]
    fn scans_period_rounds_after_preferred_miss() {
        let plan = plan_pbft_reward_votes(fact(vec![hash(3)]));

        assert!(plan.accepted);
        assert_eq!(plan.selected_round, 3);
        assert_eq!(plan.selected_vote_hashes, vec![hash(3)]);
    }

    #[test]
    fn reports_missing_reward_vote() {
        let plan = plan_pbft_reward_votes(fact(vec![hash(9)]));

        assert!(!plan.accepted);
        assert_eq!(plan.status, PbftRewardVotesStatus::MissingRewardVote);
        assert_eq!(plan.missing_vote_hash, Some(hash(9)));
    }

    #[test]
    fn reports_missing_preferred_round_before_period_scan() {
        let mut fact = fact(vec![hash(1)]);
        fact.has_preferred_round = false;

        let plan = plan_pbft_reward_votes(fact);

        assert!(!plan.accepted);
        assert_eq!(plan.status, PbftRewardVotesStatus::MissingPreferredRound);
    }
}
