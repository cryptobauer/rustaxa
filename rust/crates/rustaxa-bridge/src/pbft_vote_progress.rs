//! Rust-only adapters for deterministic PBFT vote-progress planning.
//!
//! The verified-votes facade keeps the Rust consensus planner side-effect-free
//! while flattening operation-specific execution facts for the C++
//! `VoteManager` shim. C++ remains the executor for effects that touch peers,
//! networking, storage handles, or live sidecars.

use crate::ffi::rustaxa_ffi::PbftVoteProgressContext as FfiPbftVoteProgressContext;
use rustaxa_consensus::pbft_vote_progress::{
    PbftVoteProgressContext, PbftVoteProgressIntent, PbftVoteProgressPlan, PbftVoteProgressStatus,
};

/// Rust-only flattened vote-progress execution facts for the verified-votes
/// facade.
pub(crate) struct PbftVoteProgressExecutionAdapter {
    pub(crate) status: u8,
    pub(crate) error_code: String,
    pub(crate) accepted: bool,
    pub(crate) mark_vote_known: bool,
    pub(crate) mark_vote_known_hash: [u8; 32],
    pub(crate) request_proposed_block_sidecar: bool,
    pub(crate) proposed_block_sidecar_hash: [u8; 32],
    pub(crate) proposed_block_sidecar_period: u64,
    pub(crate) gossip_vote: bool,
    pub(crate) gossip_vote_hash: [u8; 32],
    pub(crate) report_slashing: bool,
    pub(crate) network_t_plus_one_step_updated: bool,
    pub(crate) drive_pbft_progress: bool,
    pub(crate) progress_period: u64,
    pub(crate) progress_round: u64,
}

pub(crate) fn context_to_domain(value: &FfiPbftVoteProgressContext) -> PbftVoteProgressContext {
    PbftVoteProgressContext {
        current_period: value.current_period,
        current_round: value.current_round,
        max_future_period_delta: value.max_future_period_delta,
        two_t_plus_one_threshold: value
            .has_two_t_plus_one_threshold
            .then_some(value.two_t_plus_one_threshold),
        require_proposed_block_sidecar: value.require_proposed_block_sidecar,
        slashing_enabled: value.slashing_enabled,
    }
}

pub(crate) fn execution_plan_to_ffi(
    plan: PbftVoteProgressPlan,
) -> PbftVoteProgressExecutionAdapter {
    let slashing = plan.intents.iter().find_map(|intent| match intent {
        PbftVoteProgressIntent::ReportSlashing {
            incoming_vote_hash,
            conflicting_vote_hash,
        } => Some((*incoming_vote_hash, *conflicting_vote_hash)),
        _ => None,
    });
    let mark_vote_known = plan.intents.iter().find_map(|intent| match intent {
        PbftVoteProgressIntent::MarkKnown { vote_hash } => Some(*vote_hash),
        _ => None,
    });
    let proposed_block_sidecar = plan.intents.iter().find_map(|intent| match intent {
        PbftVoteProgressIntent::RequestProposedBlockSidecar { block_hash, period } => {
            Some((*block_hash, *period))
        }
        _ => None,
    });
    let gossip_vote = plan.intents.iter().find_map(|intent| match intent {
        PbftVoteProgressIntent::GossipVote { vote_hash } => Some(*vote_hash),
        _ => None,
    });
    let drive_progress = plan.intents.iter().find_map(|intent| match intent {
        PbftVoteProgressIntent::DrivePbftProgress { period, round } => Some((*period, *round)),
        _ => None,
    });
    let threshold = plan.threshold_decision;

    PbftVoteProgressExecutionAdapter {
        status: plan.status.as_u8(),
        error_code: error_code(plan.status).to_owned(),
        accepted: matches!(
            plan.status,
            PbftVoteProgressStatus::Accepted | PbftVoteProgressStatus::AcceptedWithProgress
        ),
        mark_vote_known: mark_vote_known.is_some(),
        mark_vote_known_hash: mark_vote_known.unwrap_or_default().into(),
        request_proposed_block_sidecar: proposed_block_sidecar.is_some(),
        proposed_block_sidecar_hash: proposed_block_sidecar
            .map(|(block_hash, _)| block_hash)
            .unwrap_or_default()
            .into(),
        proposed_block_sidecar_period: proposed_block_sidecar
            .map(|(_, period)| period)
            .unwrap_or_default(),
        gossip_vote: gossip_vote.is_some(),
        gossip_vote_hash: gossip_vote.unwrap_or_default().into(),
        report_slashing: slashing.is_some(),
        network_t_plus_one_step_updated: threshold
            .is_some_and(|decision| decision.network_t_plus_one_step_updated),
        drive_pbft_progress: drive_progress.is_some(),
        progress_period: drive_progress.map(|(period, _)| period).unwrap_or_default(),
        progress_round: drive_progress.map(|(_, round)| round).unwrap_or_default(),
    }
}

pub(crate) const fn error_code(status: PbftVoteProgressStatus) -> &'static str {
    match status {
        PbftVoteProgressStatus::PendingVerifiedVoteInsert
        | PbftVoteProgressStatus::Accepted
        | PbftVoteProgressStatus::AcceptedWithProgress
        | PbftVoteProgressStatus::AlreadyKnown
        | PbftVoteProgressStatus::DuplicateVerifiedVote
        | PbftVoteProgressStatus::ConflictingVote => "",
        PbftVoteProgressStatus::RejectedStalePeriod => "PBFT_VOTE_PROGRESS_STALE_PERIOD",
        PbftVoteProgressStatus::RejectedFuturePeriod => "PBFT_VOTE_PROGRESS_FUTURE_PERIOD",
        PbftVoteProgressStatus::RejectedInvalidVote => "PBFT_VOTE_PROGRESS_INVALID_VOTE",
        PbftVoteProgressStatus::RejectedExecutorReport => {
            "PBFT_VOTE_PROGRESS_REJECTED_EXECUTOR_REPORT"
        }
        PbftVoteProgressStatus::MissingProposedBlockSidecar => {
            "PBFT_VOTE_PROGRESS_MISSING_PROPOSED_BLOCK_SIDECAR"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;
    use rustaxa_consensus::verified_votes::ThresholdDecisionOutcome;

    #[test]
    fn adapter_projects_native_effects_and_boundary_codes() {
        let vote_hash = H256::from([1; 32]);
        let block_hash = H256::from([2; 32]);
        let conflicting_vote_hash = H256::from([3; 32]);
        let plan = PbftVoteProgressPlan {
            status: PbftVoteProgressStatus::AcceptedWithProgress,
            intents: vec![
                PbftVoteProgressIntent::MarkKnown { vote_hash },
                PbftVoteProgressIntent::RequestProposedBlockSidecar {
                    block_hash,
                    period: 10,
                },
                PbftVoteProgressIntent::ReportSlashing {
                    incoming_vote_hash: vote_hash,
                    conflicting_vote_hash,
                },
                PbftVoteProgressIntent::GossipVote { vote_hash },
                PbftVoteProgressIntent::DrivePbftProgress {
                    period: 10,
                    round: 2,
                },
            ],
            add_vote_outcome: None,
            threshold_decision: Some(ThresholdDecisionOutcome {
                t_plus_one_reached: true,
                network_t_plus_one_step_updated: true,
                two_t_plus_one_reached: false,
                two_t_plus_one_kind: None,
                two_t_plus_one_insert_outcome: None,
            }),
            conflicting_vote_hash: Some(conflicting_vote_hash),
        };

        let ffi = execution_plan_to_ffi(plan);

        assert_eq!(
            ffi.status,
            PbftVoteProgressStatus::AcceptedWithProgress.as_u8()
        );
        assert_eq!(ffi.error_code, "");
        assert!(ffi.accepted);
        assert!(ffi.mark_vote_known);
        assert_eq!(ffi.mark_vote_known_hash, [1; 32]);
        assert!(ffi.request_proposed_block_sidecar);
        assert_eq!(ffi.proposed_block_sidecar_hash, [2; 32]);
        assert_eq!(ffi.proposed_block_sidecar_period, 10);
        assert!(ffi.gossip_vote);
        assert_eq!(ffi.gossip_vote_hash, [1; 32]);
        assert!(ffi.report_slashing);
        assert!(ffi.network_t_plus_one_step_updated);
        assert!(ffi.drive_pbft_progress);
        assert_eq!(ffi.progress_period, 10);
        assert_eq!(ffi.progress_round, 2);

        assert_eq!(
            error_code(PbftVoteProgressStatus::MissingProposedBlockSidecar),
            "PBFT_VOTE_PROGRESS_MISSING_PROPOSED_BLOCK_SIDECAR"
        );
        assert_eq!(
            error_code(PbftVoteProgressStatus::RejectedExecutorReport),
            "PBFT_VOTE_PROGRESS_REJECTED_EXECUTOR_REPORT"
        );
    }
}
