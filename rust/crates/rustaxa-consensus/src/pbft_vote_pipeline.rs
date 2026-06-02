//! PBFT vote pipeline runtime session.
//!
//! This module owns the staged protocol order for one VoteManager-local PBFT
//! vote event. It composes the existing side-effect-free vote-progress planner
//! around the one state mutation that still lives at the boundary: inserting
//! into the Rust-backed verified-vote index while C++ retains live `PbftVote`
//! sidecars.
//!
//! The session does not validate signatures, read FinalChain, write storage,
//! submit slashing proofs, or send network messages. Callers supply compact
//! vote facts and scalar context, execute the requested verified-vote mutation,
//! then report the mutation outcome back to the session. This keeps the future
//! pipeline shape explicit while preserving the current C++ executor boundary.

use crate::pbft_vote_progress::{
    PbftVoteProgressContext, PbftVoteProgressFact, PbftVoteProgressPlan, PbftVoteProgressStatus,
    plan_pbft_vote_progress,
};
use crate::verified_votes::AddVerifiedVoteOutcome;

/// Stage status for one PBFT vote pipeline session.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftVotePipelineStatus {
    /// The session has been created and can produce its pre-insert plan.
    Ready,
    /// The session requested verified-vote insertion and awaits its report.
    AwaitingVerifiedVoteInsert,
    /// The session reached a terminal plan.
    Complete,
    /// The caller attempted a stage transition in the wrong order.
    InvalidStage,
}

impl PbftVotePipelineStatus {
    /// Stable numeric status used by bridge payloads.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::AwaitingVerifiedVoteInsert => 1,
            Self::Complete => 2,
            Self::InvalidStage => 3,
        }
    }
}

/// Output from a PBFT vote pipeline session stage.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVotePipelineStep {
    /// Session-stage status after this call.
    pub pipeline_status: PbftVotePipelineStatus,
    /// Vote-progress protocol plan produced for this stage.
    pub progress_plan: PbftVoteProgressPlan,
    /// Whether the session is terminal.
    pub complete: bool,
}

/// Runtime state for one PBFT vote event.
#[derive(Debug, Clone)]
pub struct PbftVotePipelineSession {
    fact: PbftVoteProgressFact,
    context: PbftVoteProgressContext,
    stage: PbftVotePipelineStatus,
}

impl PbftVotePipelineSession {
    /// Creates a new PBFT vote pipeline session.
    ///
    /// Inputs:
    /// - `fact`: compact vote identity, weight, ingress, and stale-reward facts.
    /// - `context`: scalar current-period, threshold, sidecar, and slashing settings.
    ///
    /// Invariants:
    /// - The session owns these facts for the whole vote event, so the
    ///   post-insert report cannot be planned against a different context.
    #[must_use]
    pub fn new(fact: PbftVoteProgressFact, context: PbftVoteProgressContext) -> Self {
        Self {
            fact,
            context,
            stage: PbftVotePipelineStatus::Ready,
        }
    }

    /// Returns the current session stage.
    #[must_use]
    pub const fn stage(&self) -> PbftVotePipelineStatus {
        self.stage
    }

    /// Returns the compact vote facts owned by this session.
    #[must_use]
    pub const fn fact(&self) -> &PbftVoteProgressFact {
        &self.fact
    }

    /// Returns the scalar context owned by this session.
    #[must_use]
    pub const fn context(&self) -> &PbftVoteProgressContext {
        &self.context
    }

    /// Produces the pre-insert plan for this vote event.
    ///
    /// Outputs:
    /// - `AwaitingVerifiedVoteInsert` when the caller must execute exactly one
    ///   verified-vote insertion mutation and then report the outcome.
    /// - `Complete` for terminal reject/known plans that need no insertion.
    ///
    /// Edge behavior:
    /// - Calling this method after the session has advanced returns
    ///   `InvalidStage` without mutating the session further.
    pub fn precheck(&mut self) -> PbftVotePipelineStep {
        if self.stage != PbftVotePipelineStatus::Ready {
            return invalid_stage_step();
        }

        let progress_plan = plan_pbft_vote_progress(self.fact, self.context, None);
        let awaiting_insert =
            progress_plan.status == PbftVoteProgressStatus::PendingVerifiedVoteInsert;
        self.stage = if awaiting_insert {
            PbftVotePipelineStatus::AwaitingVerifiedVoteInsert
        } else {
            PbftVotePipelineStatus::Complete
        };

        PbftVotePipelineStep {
            pipeline_status: self.stage,
            progress_plan,
            complete: !awaiting_insert,
        }
    }

    /// Reports the verified-vote insertion outcome and returns the terminal plan.
    ///
    /// Inputs:
    /// - `add_vote_outcome`: authoritative mutation report from the Rust-backed
    ///   verified-vote index.
    ///
    /// Edge behavior:
    /// - Reports are accepted only after `precheck` requested insertion.
    /// - The session becomes complete after accepting one report.
    pub fn report_verified_vote_add(
        &mut self,
        add_vote_outcome: AddVerifiedVoteOutcome,
    ) -> PbftVotePipelineStep {
        if self.stage != PbftVotePipelineStatus::AwaitingVerifiedVoteInsert {
            return invalid_stage_step();
        }

        let progress_plan =
            plan_pbft_vote_progress(self.fact, self.context, Some(add_vote_outcome));
        self.stage = PbftVotePipelineStatus::Complete;
        PbftVotePipelineStep {
            pipeline_status: PbftVotePipelineStatus::Complete,
            progress_plan,
            complete: true,
        }
    }
}

fn invalid_stage_step() -> PbftVotePipelineStep {
    PbftVotePipelineStep {
        pipeline_status: PbftVotePipelineStatus::InvalidStage,
        progress_plan: PbftVoteProgressPlan {
            status: PbftVoteProgressStatus::RejectedExecutorReport,
            intents: Vec::new(),
            add_vote_outcome: None,
            threshold_decision: None,
            conflicting_vote_hash: None,
        },
        complete: true,
    }
}

/// Creates a PBFT vote pipeline session.
#[must_use]
pub fn create_pbft_vote_pipeline_session(
    fact: PbftVoteProgressFact,
    context: PbftVoteProgressContext,
) -> PbftVotePipelineSession {
    PbftVotePipelineSession::new(fact, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_vote_progress::{PbftVoteIdentity, PbftVoteProgressIntent};
    use crate::verified_votes::{PbftVoteType, ThresholdDecisionOutcome};
    use ethereum_types::{H160, H256};

    fn h256(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    fn fact(weight: u64) -> PbftVoteProgressFact {
        PbftVoteProgressFact {
            identity: PbftVoteIdentity {
                vote_hash: h256(1),
                block_hash: h256(2),
                period: 10,
                round: 1,
                step: 2,
                voter: H160::from_low_u64_be(3),
            },
            vote_type: PbftVoteType::Soft,
            weight,
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    const fn context() -> PbftVoteProgressContext {
        PbftVoteProgressContext {
            current_period: 10,
            max_future_period_delta: 1,
            two_t_plus_one_threshold: Some(2),
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    const fn add_outcome(
        inserted: bool,
        threshold_decision: Option<ThresholdDecisionOutcome>,
    ) -> AddVerifiedVoteOutcome {
        AddVerifiedVoteOutcome {
            inserted,
            total_weight: 2,
            votes_count: 2,
            conflicting_vote_hash: None,
            used_secondary_slot: false,
            duplicate_vote_hash: false,
            threshold_decision,
        }
    }

    #[test]
    fn session_requests_insert_then_accepts_report() {
        let mut session = create_pbft_vote_pipeline_session(fact(1), context());

        let precheck = session.precheck();
        assert_eq!(
            precheck.pipeline_status,
            PbftVotePipelineStatus::AwaitingVerifiedVoteInsert
        );
        assert!(precheck.progress_plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::InsertVerifiedVote { .. }
        )));
        assert!(!precheck.complete);

        let terminal = session.report_verified_vote_add(add_outcome(true, None));
        assert_eq!(terminal.pipeline_status, PbftVotePipelineStatus::Complete);
        assert_eq!(
            terminal.progress_plan.status,
            PbftVoteProgressStatus::Accepted
        );
        assert!(terminal.complete);
    }

    #[test]
    fn terminal_precheck_completes_without_insert() {
        let mut known = fact(1);
        known.vote_already_known = true;
        let mut session = create_pbft_vote_pipeline_session(known, context());

        let precheck = session.precheck();
        assert_eq!(precheck.pipeline_status, PbftVotePipelineStatus::Complete);
        assert_eq!(
            precheck.progress_plan.status,
            PbftVoteProgressStatus::AlreadyKnown
        );
        assert!(precheck.complete);
    }

    #[test]
    fn session_rejects_out_of_order_report() {
        let mut session = create_pbft_vote_pipeline_session(fact(1), context());
        let terminal = session.report_verified_vote_add(add_outcome(true, None));

        assert_eq!(
            terminal.pipeline_status,
            PbftVotePipelineStatus::InvalidStage
        );
        assert_eq!(
            terminal.progress_plan.status,
            PbftVoteProgressStatus::RejectedExecutorReport
        );
        assert!(terminal.complete);
    }
}
