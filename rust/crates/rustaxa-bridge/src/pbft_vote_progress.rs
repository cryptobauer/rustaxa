//! Rust-only adapters for deterministic PBFT vote-progress planning.
//!
//! The verified-votes facade keeps the Rust consensus planner side-effect-free
//! while flattening operation-specific execution facts for the C++
//! `VoteManager` shim. C++ remains the executor for effects that touch peers,
//! networking, storage handles, or live sidecars.

use crate::ffi::rustaxa_ffi::PbftVoteProgressContext as FfiPbftVoteProgressContext;
#[cfg(test)]
use crate::ffi::rustaxa_ffi::VerifiedVoteAddOutcome;
#[cfg(test)]
use anyhow::Result;
#[cfg(test)]
use ethereum_types::H256;
use rustaxa_consensus::pbft_vote_progress::{
    PbftVoteProgressContext, PbftVoteProgressFact, PbftVoteProgressIntent, PbftVoteProgressPlan,
    PbftVoteProgressStatus,
};
#[cfg(test)]
use rustaxa_consensus::verified_votes::{
    AddVerifiedVoteOutcome, ThresholdDecisionOutcome, TwoTPlusOneInsertOutcome,
    TwoTPlusOneVotedBlockType,
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
    pub(crate) persist_extra_reward_vote: bool,
    pub(crate) network_t_plus_one_step_updated: bool,
    pub(crate) drive_pbft_progress: bool,
    pub(crate) progress_period: u64,
    pub(crate) progress_round: u64,
    pub(crate) persist_two_t_plus_one_votes: bool,
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

#[cfg(test)]
pub(crate) fn add_outcome_to_domain(
    value: VerifiedVoteAddOutcome,
) -> Result<AddVerifiedVoteOutcome> {
    let threshold_decision = if value.threshold_applied {
        Some(ThresholdDecisionOutcome {
            t_plus_one_reached: value.t_plus_one_reached,
            network_t_plus_one_step_updated: value.network_t_plus_one_step_updated,
            two_t_plus_one_reached: value.two_t_plus_one_reached,
            two_t_plus_one_kind: value
                .two_t_plus_one_kind_found
                .then(|| TwoTPlusOneVotedBlockType::try_from(value.two_t_plus_one_kind))
                .transpose()?,
            two_t_plus_one_insert_outcome: value.two_t_plus_one_round_found.then_some(
                TwoTPlusOneInsertOutcome {
                    round_found: value.two_t_plus_one_round_found,
                    inserted: value.two_t_plus_one_inserted,
                },
            ),
        })
    } else {
        None
    };

    Ok(AddVerifiedVoteOutcome {
        inserted: value.inserted,
        total_weight: value.total_weight,
        votes_count: value.votes_count,
        conflicting_vote_hash: value
            .conflict_found
            .then_some(H256::from(value.conflicting_vote_hash)),
        used_secondary_slot: value.used_secondary_slot,
        duplicate_vote_hash: value.duplicate_vote_hash,
        threshold_decision,
    })
}

pub(crate) fn execution_plan_to_ffi(
    plan: PbftVoteProgressPlan,
    _fact: PbftVoteProgressFact,
    _context: FfiPbftVoteProgressContext,
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
    let extra_reward_vote = plan.intents.iter().find_map(|intent| match intent {
        PbftVoteProgressIntent::PersistExtraRewardVote { vote_hash } => Some(*vote_hash),
        _ => None,
    });
    let drive_progress = plan.intents.iter().find_map(|intent| match intent {
        PbftVoteProgressIntent::DrivePbftProgress { period, round } => Some((*period, *round)),
        _ => None,
    });
    let two_t_plus_one = plan.intents.iter().find_map(|intent| match intent {
        PbftVoteProgressIntent::PersistTwoTPlusOneVotes {
            kind,
            period,
            round,
            step,
            block_hash,
        } => Some((*kind, *period, *round, *step, *block_hash)),
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
        persist_extra_reward_vote: extra_reward_vote.is_some(),
        network_t_plus_one_step_updated: threshold
            .is_some_and(|decision| decision.network_t_plus_one_step_updated),
        drive_pbft_progress: drive_progress.is_some(),
        progress_period: drive_progress.map(|(period, _)| period).unwrap_or_default(),
        progress_round: drive_progress.map(|(_, round)| round).unwrap_or_default(),
        persist_two_t_plus_one_votes: two_t_plus_one.is_some(),
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
    use crate::ffi::rustaxa_ffi::VerifiedVotePayload;
    use ethereum_types::H160;
    use rustaxa_consensus::pbft_vote_progress::{plan_pbft_vote_progress, PbftVoteIdentity};
    use rustaxa_consensus::verified_votes::PbftVoteType;

    fn vote_payload(weight: u64) -> VerifiedVotePayload {
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
            identity: PbftVoteIdentity {
                vote_hash: H256::from([1; 32]),
                block_hash: H256::from([2; 32]),
                voter: H160::from([3; 20]),
                period: 10,
                round: 1,
                step: 2,
            },
            vote_type: PbftVoteType::Cert,
            weight,
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    fn context() -> FfiPbftVoteProgressContext {
        FfiPbftVoteProgressContext {
            current_period: 10,
            current_round: 1,
            max_future_period_delta: 0,
            has_two_t_plus_one_threshold: true,
            two_t_plus_one_threshold: 2,
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    fn add_outcome() -> VerifiedVoteAddOutcome {
        VerifiedVoteAddOutcome {
            vote: vote_payload(1),
            inserted: false,
            total_weight: 0,
            votes_count: 0,
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
    fn planner_still_reports_insert_precheck_without_durable_effects() {
        let ctx = context();
        let plan = plan_pbft_vote_progress(fact(1), context_to_domain(&ctx), None);

        assert_eq!(plan.status.as_u8(), 0);
        assert!(plan.contains_intent(|intent| {
            matches!(intent, PbftVoteProgressIntent::InsertVerifiedVote { .. })
        }));
        assert!(plan.threshold_decision.is_none());
    }

    #[test]
    fn facade_adapter_flattens_slashing_decision() {
        let mut outcome = add_outcome();
        outcome.conflict_found = true;
        outcome.conflicting_vote_hash = [9; 32];

        let domain_fact = fact(1);
        let domain_context = context_to_domain(&context());
        let outcome = add_outcome_to_domain(outcome).unwrap();
        let plan = plan_pbft_vote_progress(domain_fact, domain_context, Some(outcome));
        let plan = execution_plan_to_ffi(plan, domain_fact, context());

        assert_eq!(plan.status, 9);
        assert!(!plan.accepted);
        assert!(plan.mark_vote_known);
        assert_eq!(plan.mark_vote_known_hash, [1; 32]);
        assert!(!plan.gossip_vote);
        assert!(plan.report_slashing);
        assert!(!plan.persist_extra_reward_vote);
        assert!(!plan.persist_two_t_plus_one_votes);
    }

    #[test]
    fn facade_adapter_plans_current_round_two_t_plus_one_persistence() {
        let mut outcome = add_outcome();
        outcome.inserted = true;
        outcome.total_weight = 2;
        outcome.votes_count = 2;
        outcome.threshold_applied = true;
        outcome.t_plus_one_reached = true;
        outcome.network_t_plus_one_step_updated = true;
        outcome.two_t_plus_one_reached = true;
        outcome.two_t_plus_one_kind_found = true;
        outcome.two_t_plus_one_kind = TwoTPlusOneVotedBlockType::NextVotedBlock.into();
        outcome.two_t_plus_one_round_found = true;
        outcome.two_t_plus_one_inserted = true;

        let mut domain_fact = fact(1);
        domain_fact.vote_type = PbftVoteType::Next;
        let domain_context = context_to_domain(&context());
        let outcome = add_outcome_to_domain(outcome).unwrap();
        let plan = plan_pbft_vote_progress(domain_fact, domain_context, Some(outcome));
        let plan = execution_plan_to_ffi(plan, domain_fact, context());

        assert_eq!(plan.status, 2);
        assert!(plan.accepted);
        assert!(plan.mark_vote_known);
        assert!(plan.gossip_vote);
        assert_eq!(plan.gossip_vote_hash, [1; 32]);
        assert!(plan.drive_pbft_progress);
        assert!(plan.network_t_plus_one_step_updated);
        assert!(plan.persist_two_t_plus_one_votes);
    }

    #[test]
    fn facade_adapter_defers_reward_persistence_until_acceptance() {
        let mut stale_fact = fact(1);
        stale_fact.identity.period = 9;
        stale_fact.vote_type = PbftVoteType::Next;
        stale_fact.valid_stale_reward_vote = true;

        let mut ctx = context();
        ctx.current_period = 10;
        ctx.max_future_period_delta = u64::MAX - ctx.current_period;
        let precheck = plan_pbft_vote_progress(stale_fact, context_to_domain(&ctx), None);
        assert!(precheck.contains_intent(|intent| {
            matches!(intent, PbftVoteProgressIntent::InsertVerifiedVote { .. })
        }));

        let mut stale_fact = fact(1);
        stale_fact.identity.period = 9;
        stale_fact.vote_type = PbftVoteType::Next;
        stale_fact.valid_stale_reward_vote = true;
        let mut ctx = context();
        ctx.current_period = 10;
        ctx.max_future_period_delta = u64::MAX - ctx.current_period;
        let mut accepted = add_outcome();
        accepted.inserted = true;
        accepted.total_weight = 1;
        accepted.votes_count = 1;
        let outcome = add_outcome_to_domain(accepted).unwrap();
        let plan = plan_pbft_vote_progress(stale_fact, context_to_domain(&ctx), Some(outcome));
        let plan = execution_plan_to_ffi(plan, stale_fact, ctx);

        assert!(plan.accepted);
        assert!(plan.gossip_vote);
        assert!(plan.persist_extra_reward_vote);
    }

    #[test]
    fn facade_adapter_flattens_missing_proposed_block_sidecar_request() {
        let mut fact = fact(1);
        fact.vote_type = PbftVoteType::Propose;
        fact.carries_proposed_block = false;
        let mut context = context();
        context.require_proposed_block_sidecar = true;

        let plan = plan_pbft_vote_progress(fact, context_to_domain(&context), None);
        let ffi = execution_plan_to_ffi(plan, fact, context);

        assert!(!ffi.accepted);
        assert!(!ffi.mark_vote_known);
        assert!(ffi.request_proposed_block_sidecar);
        assert_eq!(ffi.proposed_block_sidecar_hash, [2; 32]);
        assert_eq!(ffi.proposed_block_sidecar_period, 10);
    }
}
