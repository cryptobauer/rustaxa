//! CXX bridge wrapper for PBFT vote pipeline sessions.
//!
//! The bridge exposes one Rust-owned session per VoteManager-local PBFT vote
//! event. C++ supplies the compact facts once, executes the requested
//! verified-vote insertion mutation, and reports the mutation outcome back to
//! the same session. Rust validates the report identity before returning the
//! terminal execution plan.

use crate::ffi::rustaxa_ffi::{
    PbftVotePipelineExecutionPlan as FfiPbftVotePipelineExecutionPlan,
    PbftVotePipelinePrecheckPlan as FfiPbftVotePipelinePrecheckPlan,
    PbftVotePipelineTransitionKey as FfiPbftVotePipelineTransitionKey,
    PbftVoteProgressContext as FfiPbftVoteProgressContext,
    PbftVoteProgressFact as FfiPbftVoteProgressFact,
    VerifiedVoteAddOutcome as FfiVerifiedVoteAddOutcome, VerifiedVotePayload,
};
use crate::ffi::BridgePbftVotePipelineSession;
use crate::pbft_vote_progress::{
    add_outcome_to_domain, context_to_domain, error_code, execution_plan_to_ffi, fact_to_domain,
};
use anyhow::Result;
use rustaxa_consensus::pbft_vote_pipeline::{
    create_pbft_vote_pipeline_session as create_domain_pbft_vote_pipeline_session,
    PbftVotePipelineSession, PbftVotePipelineStatus, PbftVotePipelineStep,
};
use rustaxa_consensus::pbft_vote_progress::{PbftVoteProgressPlan, PbftVoteProgressStatus};

/// Creates one Rust-owned PBFT vote pipeline session.
///
/// Inputs:
/// - `fact`: compact vote identity/weight/ingress facts.
/// - `context`: scalar current-period, threshold, sidecar, and slashing facts.
///
/// Outputs:
/// - A session that owns the transition facts until C++ reports the verified
///   vote insertion result.
pub fn create_pbft_vote_pipeline_session(
    fact: FfiPbftVoteProgressFact,
    context: FfiPbftVoteProgressContext,
) -> Result<Box<BridgePbftVotePipelineSession>> {
    Ok(Box::new(BridgePbftVotePipelineSession {
        state: create_domain_pbft_vote_pipeline_session(
            fact_to_domain(fact)?,
            context_to_domain(&context),
        ),
        context,
    }))
}

impl BridgePbftVotePipelineSession {
    /// Returns this session's pre-insert plan.
    pub fn pbft_vote_pipeline_precheck(&mut self) -> FfiPbftVotePipelinePrecheckPlan {
        precheck_to_ffi(self.state.precheck(), &self.state)
    }

    /// Reports the verified-vote insertion result and returns a terminal plan.
    pub fn pbft_vote_pipeline_complete(
        &mut self,
        add_vote_outcome: FfiVerifiedVoteAddOutcome,
    ) -> FfiPbftVotePipelineExecutionPlan {
        if !report_matches_session(&add_vote_outcome.vote, &self.state) {
            return invalid_report_execution(self);
        }

        let Ok(outcome) = add_outcome_to_domain(add_vote_outcome) else {
            return invalid_report_execution(self);
        };
        let step = self.state.report_verified_vote_add(outcome);
        execution_to_ffi(step, self)
    }
}

fn report_matches_session(vote: &VerifiedVotePayload, session: &PbftVotePipelineSession) -> bool {
    let fact = session.fact();
    vote.vote_hash == fact.identity.vote_hash.0
        && vote.block_hash == fact.identity.block_hash.0
        && vote.voter == fact.identity.voter.0
        && vote.period == fact.identity.period
        && vote.round == fact.identity.round
        && vote.step == fact.identity.step
        && vote.vote_type == u8::from(fact.vote_type)
        && vote.weight == fact.weight
}

fn transition_key(session: &PbftVotePipelineSession) -> FfiPbftVotePipelineTransitionKey {
    let fact = session.fact();
    FfiPbftVotePipelineTransitionKey {
        vote_hash: fact.identity.vote_hash.0,
        period: fact.identity.period,
        round: fact.identity.round,
        step: fact.identity.step,
        voter: fact.identity.voter.0,
    }
}

fn precheck_to_ffi(
    step: PbftVotePipelineStep,
    session: &PbftVotePipelineSession,
) -> FfiPbftVotePipelinePrecheckPlan {
    let threshold = step
        .progress_plan
        .intents
        .iter()
        .find_map(|intent| match intent {
            rustaxa_consensus::pbft_vote_progress::PbftVoteProgressIntent::InsertVerifiedVote {
                two_t_plus_one_threshold,
                ..
            } => *two_t_plus_one_threshold,
            _ => None,
        });

    FfiPbftVotePipelinePrecheckPlan {
        pipeline_status: step.pipeline_status.as_u8(),
        status: step.progress_plan.status.as_u8(),
        error_code: error_code(step.progress_plan.status).to_owned(),
        transition_key: transition_key(session),
        should_insert_verified_vote: step.progress_plan.status
            == PbftVoteProgressStatus::PendingVerifiedVoteInsert,
        has_two_t_plus_one_threshold: threshold.is_some(),
        two_t_plus_one_threshold: threshold.unwrap_or_default(),
        complete: step.complete,
    }
}

fn execution_to_ffi(
    step: PbftVotePipelineStep,
    session: &BridgePbftVotePipelineSession,
) -> FfiPbftVotePipelineExecutionPlan {
    let progress = execution_plan_to_ffi(
        step.progress_plan,
        *session.state.fact(),
        copy_context(&session.context),
    );
    FfiPbftVotePipelineExecutionPlan {
        pipeline_status: step.pipeline_status.as_u8(),
        status: progress.status,
        error_code: progress.error_code,
        transition_key: transition_key(&session.state),
        accepted: progress.accepted,
        report_slashing: progress.report_slashing,
        slashing_incoming_vote_hash: progress.slashing_incoming_vote_hash,
        slashing_conflicting_vote_hash: progress.slashing_conflicting_vote_hash,
        persist_extra_reward_vote: progress.persist_extra_reward_vote,
        extra_reward_vote_hash: progress.extra_reward_vote_hash,
        network_t_plus_one_step_updated: progress.network_t_plus_one_step_updated,
        drive_pbft_progress: progress.drive_pbft_progress,
        progress_period: progress.progress_period,
        progress_round: progress.progress_round,
        persist_two_t_plus_one_votes: progress.persist_two_t_plus_one_votes,
        two_t_plus_one_kind: progress.two_t_plus_one_kind,
        two_t_plus_one_period: progress.two_t_plus_one_period,
        two_t_plus_one_round: progress.two_t_plus_one_round,
        two_t_plus_one_step: progress.two_t_plus_one_step,
        two_t_plus_one_block_hash: progress.two_t_plus_one_block_hash,
        complete: step.complete,
    }
}

fn invalid_report_execution(
    session: &BridgePbftVotePipelineSession,
) -> FfiPbftVotePipelineExecutionPlan {
    let step = PbftVotePipelineStep {
        pipeline_status: PbftVotePipelineStatus::InvalidStage,
        progress_plan: PbftVoteProgressPlan {
            status: PbftVoteProgressStatus::RejectedExecutorReport,
            intents: Vec::new(),
            add_vote_outcome: None,
            threshold_decision: None,
            conflicting_vote_hash: None,
        },
        complete: true,
    };
    let mut execution = execution_to_ffi(step, session);
    execution.error_code = "PBFT_VOTE_PIPELINE_INVALID_EXECUTOR_REPORT".to_owned();
    execution
}

fn copy_context(value: &FfiPbftVoteProgressContext) -> FfiPbftVoteProgressContext {
    FfiPbftVoteProgressContext {
        current_period: value.current_period,
        current_round: value.current_round,
        max_future_period_delta: value.max_future_period_delta,
        has_two_t_plus_one_threshold: value.has_two_t_plus_one_threshold,
        two_t_plus_one_threshold: value.two_t_plus_one_threshold,
        require_proposed_block_sidecar: value.require_proposed_block_sidecar,
        slashing_enabled: value.slashing_enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::{PbftVoteProgressContext, PbftVoteProgressFact};

    fn vote(weight: u64) -> VerifiedVotePayload {
        VerifiedVotePayload {
            vote_hash: [1; 32],
            block_hash: [2; 32],
            voter: [3; 20],
            period: 10,
            round: 1,
            step: 2,
            vote_type: 2,
            weight,
        }
    }

    fn fact(weight: u64) -> PbftVoteProgressFact {
        PbftVoteProgressFact {
            vote: vote(weight),
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    fn context() -> PbftVoteProgressContext {
        PbftVoteProgressContext {
            current_period: 10,
            current_round: 1,
            max_future_period_delta: 0,
            has_two_t_plus_one_threshold: true,
            two_t_plus_one_threshold: 2,
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    fn add_outcome() -> FfiVerifiedVoteAddOutcome {
        FfiVerifiedVoteAddOutcome {
            vote: vote(1),
            inserted: true,
            total_weight: 1,
            votes_count: 1,
            conflict_found: false,
            conflicting_vote_hash: [0; 32],
            used_secondary_slot: false,
            duplicate_vote_hash: false,
            threshold_applied: false,
            t_plus_one_reached: false,
            network_t_plus_one_step_updated: false,
            two_t_plus_one_reached: false,
            two_t_plus_one_kind_found: false,
            two_t_plus_one_kind: 0,
            two_t_plus_one_round_found: false,
            two_t_plus_one_inserted: false,
        }
    }

    #[test]
    fn bridge_session_precheck_and_complete_share_transition_key() {
        let mut session = create_pbft_vote_pipeline_session(fact(1), context()).unwrap();
        let precheck = session.pbft_vote_pipeline_precheck();
        assert_eq!(precheck.pipeline_status, 1);
        assert!(precheck.should_insert_verified_vote);
        assert_eq!(precheck.transition_key.vote_hash, [1; 32]);

        let execution = session.pbft_vote_pipeline_complete(add_outcome());
        assert_eq!(execution.pipeline_status, 2);
        assert!(execution.accepted);
        assert_eq!(
            execution.transition_key.vote_hash,
            precheck.transition_key.vote_hash
        );
    }

    #[test]
    fn bridge_session_rejects_mismatched_executor_report() {
        let mut session = create_pbft_vote_pipeline_session(fact(1), context()).unwrap();
        let _ = session.pbft_vote_pipeline_precheck();

        let mut outcome = add_outcome();
        outcome.vote.vote_hash = [9; 32];
        let execution = session.pbft_vote_pipeline_complete(outcome);

        assert_eq!(execution.pipeline_status, 3);
        assert_eq!(
            execution.error_code,
            "PBFT_VOTE_PIPELINE_INVALID_EXECUTOR_REPORT"
        );
        assert!(!execution.accepted);
    }
}
