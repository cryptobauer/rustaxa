//! CXX bridge wrappers for PBFT reward-vote selection planning.
//!
//! The bridge exposes a fact-only contract for `VoteManager::checkRewardVotes`.
//! C++ supplies the current live verified-vote membership snapshots and PBFT
//! block reward-vote references; Rust returns the selected round and vote hashes
//! without touching storage or materializing `PbftVote` sidecars.

use crate::ffi::rustaxa_ffi::{
    PbftFinalizationHash as FfiHash, PbftRewardVoteRoundCandidate as FfiRoundCandidate,
    PbftRewardVoteSelectionFact as FfiSelectionFact,
    PbftRewardVoteSelectionPlan as FfiSelectionPlan,
};
use ethereum_types::H256;
use rustaxa_consensus::pbft_reward_votes::{
    plan_pbft_reward_votes, PbftRewardVoteRoundCandidate, PbftRewardVoteSelectionFact,
    PbftRewardVotesStatus,
};

/// Plans PBFT reward-vote selection for a block under validation.
///
/// Inputs:
/// - `fact`: reward metadata, PBFT-block requested vote hashes, and per-round
///   live membership facts sourced by the C++ VoteManager shim.
///
/// Outputs:
/// - A flat bridge plan with stable status, selected round, selected block hash,
///   selected vote hashes, and first missing hash when rejected for incomplete
///   membership.
///
/// Edge behavior:
/// - Period 1 returns accepted with an empty selected-vote list.
/// - The planner is side-effect-free; C++ remains responsible for mapping
///   selected hashes back to live sidecars when `copy_votes` is true.
pub fn pbft_reward_votes_plan(fact: FfiSelectionFact) -> FfiSelectionPlan {
    let plan = plan_pbft_reward_votes(fact_to_domain(fact));
    FfiSelectionPlan {
        accepted: plan.accepted,
        status: plan.status.as_u8(),
        error_code: error_code(plan.status).to_owned(),
        selected_period: plan.selected_period,
        selected_round: plan.selected_round,
        selected_block_hash: plan.selected_block_hash.into(),
        selected_vote_hashes: hashes_to_ffi(plan.selected_vote_hashes),
        missing_vote_hash: plan.missing_vote_hash.unwrap_or_default().into(),
    }
}

fn fact_to_domain(value: FfiSelectionFact) -> PbftRewardVoteSelectionFact {
    PbftRewardVoteSelectionFact {
        block_period: value.block_period,
        reward_period: value.reward_period,
        preferred_reward_round: value.preferred_reward_round,
        reward_block_hash: H256::from(value.reward_block_hash),
        requested_vote_hashes: value
            .requested_vote_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect(),
        has_preferred_round: value.has_preferred_round,
        preferred_round: candidate_to_domain(value.preferred_round),
        has_reward_period: value.has_reward_period,
        period_rounds: value
            .period_rounds
            .into_iter()
            .map(candidate_to_domain)
            .collect(),
    }
}

fn candidate_to_domain(value: FfiRoundCandidate) -> PbftRewardVoteRoundCandidate {
    PbftRewardVoteRoundCandidate {
        round: value.round,
        has_cert_step: value.has_cert_step,
        has_reward_block: value.has_reward_block,
        vote_hashes: value
            .vote_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect(),
    }
}

fn hashes_to_ffi(hashes: Vec<H256>) -> Vec<FfiHash> {
    hashes
        .into_iter()
        .map(|hash| FfiHash { hash: hash.into() })
        .collect()
}

const fn error_code(status: PbftRewardVotesStatus) -> &'static str {
    match status {
        PbftRewardVotesStatus::FirstPeriod | PbftRewardVotesStatus::Accepted => "",
        PbftRewardVotesStatus::MissingPreferredRound => "PBFT_REWARD_VOTES_MISSING_PREFERRED_ROUND",
        PbftRewardVotesStatus::MissingRewardPeriod => "PBFT_REWARD_VOTES_MISSING_REWARD_PERIOD",
        PbftRewardVotesStatus::MissingCertStep => "PBFT_REWARD_VOTES_MISSING_CERT_STEP",
        PbftRewardVotesStatus::MissingRewardBlock => "PBFT_REWARD_VOTES_MISSING_REWARD_BLOCK",
        PbftRewardVotesStatus::MissingRewardVote => "PBFT_REWARD_VOTES_MISSING_REWARD_VOTE",
        PbftRewardVotesStatus::MissingRetainedPayload => {
            "PBFT_REWARD_VOTES_MISSING_RETAINED_PAYLOAD"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: u8) -> FfiHash {
        FfiHash { hash: [value; 32] }
    }

    fn candidate(round: u64, hashes: Vec<FfiHash>) -> FfiRoundCandidate {
        FfiRoundCandidate {
            round,
            has_cert_step: true,
            has_reward_block: true,
            vote_hashes: hashes,
        }
    }

    fn fact(requested: Vec<FfiHash>) -> FfiSelectionFact {
        FfiSelectionFact {
            block_period: 8,
            reward_period: 7,
            preferred_reward_round: 4,
            reward_block_hash: [9; 32],
            requested_vote_hashes: requested,
            has_preferred_round: true,
            preferred_round: candidate(4, vec![hash(1), hash(2)]),
            has_reward_period: true,
            period_rounds: vec![
                candidate(5, vec![hash(3)]),
                candidate(4, vec![hash(1), hash(2)]),
            ],
        }
    }

    #[test]
    fn bridge_returns_selected_hashes_for_preferred_round() {
        let plan = pbft_reward_votes_plan(fact(vec![hash(1), hash(2)]));

        assert!(plan.accepted);
        assert_eq!(plan.status, PbftRewardVotesStatus::Accepted.as_u8());
        assert_eq!(plan.selected_round, 4);
        assert_eq!(
            plan.selected_vote_hashes
                .iter()
                .map(|hash| hash.hash)
                .collect::<Vec<_>>(),
            vec![[1; 32], [2; 32]]
        );
    }

    #[test]
    fn bridge_reports_missing_vote_hash() {
        let plan = pbft_reward_votes_plan(fact(vec![hash(8)]));

        assert!(!plan.accepted);
        assert_eq!(
            plan.status,
            PbftRewardVotesStatus::MissingRewardVote.as_u8()
        );
        assert_eq!(plan.missing_vote_hash, [8; 32]);
        assert!(!plan.error_code.is_empty());
    }
}
