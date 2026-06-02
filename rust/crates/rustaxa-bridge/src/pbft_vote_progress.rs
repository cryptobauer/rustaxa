//! CXX bridge wrappers for deterministic PBFT vote-progress planning.
//!
//! The bridge keeps the Rust consensus planner side-effect-free while exposing
//! operation-specific payloads for the C++ `VoteManager` shim. C++ supplies
//! compact vote facts and, after applying the authoritative verified-vote
//! mutation, feeds the flattened mutation report back into this module. The
//! returned execution plan names only the side effects this route may execute:
//! reward persistence, slashing submission, PBFT progress notification, and
//! current-round 2t+1 vote-bundle persistence.

use crate::ffi::rustaxa_ffi::{
    PbftVoteProgressContext as FfiPbftVoteProgressContext,
    PbftVoteProgressExecutionPlan as FfiPbftVoteProgressExecutionPlan,
    PbftVoteProgressFact as FfiPbftVoteProgressFact,
    PbftVoteProgressPrecheckPlan as FfiPbftVoteProgressPrecheckPlan, VerifiedVoteAddOutcome,
};
use anyhow::Result;
use ethereum_types::{H160, H256};
use rustaxa_consensus::pbft_vote_progress::{
    plan_pbft_vote_progress, PbftVoteIdentity, PbftVoteProgressContext, PbftVoteProgressFact,
    PbftVoteProgressIntent, PbftVoteProgressPlan, PbftVoteProgressStatus,
};
use rustaxa_consensus::verified_votes::{
    AddVerifiedVoteOutcome, PbftVoteType, ThresholdDecisionOutcome, TwoTPlusOneInsertOutcome,
    TwoTPlusOneVotedBlockType,
};

/// Plans the pre-mutation portion of PBFT vote progress.
///
/// Inputs:
/// - `fact`: compact identity, weight, ingress, and validation facts for one
///   vote.
/// - `context`: scalar current-period, current-round, threshold, sidecar, and
///   slashing settings.
///
/// Outputs:
/// - A terminal rejection/known plan or an instruction to execute exactly one
///   verified-vote insertion mutation.
///
/// Edge behavior:
/// - Malformed vote type values are returned as bridge errors.
/// - Durable side effects are never requested before insertion succeeds.
pub fn pbft_vote_progress_plan_precheck(
    fact: FfiPbftVoteProgressFact,
    context: FfiPbftVoteProgressContext,
) -> Result<FfiPbftVoteProgressPrecheckPlan> {
    let fact = fact_to_domain(fact)?;
    let context = context_to_domain(&context);
    let plan = plan_pbft_vote_progress(fact, context, None);

    Ok(FfiPbftVoteProgressPrecheckPlan {
        status: plan.status.as_u8(),
        error_code: error_code(plan.status).to_owned(),
        should_insert_verified_vote: plan.contains_intent(|intent| {
            matches!(intent, PbftVoteProgressIntent::InsertVerifiedVote { .. })
        }),
        has_two_t_plus_one_threshold: context.two_t_plus_one_threshold.is_some(),
        two_t_plus_one_threshold: context.two_t_plus_one_threshold.unwrap_or_default(),
    })
}

/// Plans the post-mutation portion of PBFT vote progress.
///
/// Inputs:
/// - `fact` and `context`: same values supplied to the precheck call.
/// - `add_vote_outcome`: authoritative verified-vote insertion and threshold
///   report produced by the Rust-backed verified-votes index.
///
/// Outputs:
/// - A flat execution plan for the C++ shim's in-scope side effects.
///
/// Invariants:
/// - The add report is trusted as the only state mutation result for this vote;
///   this function does not repeat insertion or threshold checks.
pub fn pbft_vote_progress_plan_after_add(
    fact: FfiPbftVoteProgressFact,
    context: FfiPbftVoteProgressContext,
    add_vote_outcome: VerifiedVoteAddOutcome,
) -> Result<FfiPbftVoteProgressExecutionPlan> {
    let domain_fact = fact_to_domain(fact)?;
    let domain_context = context_to_domain(&context);
    let outcome = add_outcome_to_domain(add_vote_outcome)?;
    let plan = plan_pbft_vote_progress(domain_fact, domain_context, Some(outcome));

    Ok(execution_plan_to_ffi(plan, domain_fact, context))
}

pub(crate) fn fact_to_domain(value: FfiPbftVoteProgressFact) -> Result<PbftVoteProgressFact> {
    Ok(PbftVoteProgressFact {
        identity: PbftVoteIdentity {
            vote_hash: H256::from(value.vote.vote_hash),
            block_hash: H256::from(value.vote.block_hash),
            period: value.vote.period,
            round: value.vote.round,
            step: value.vote.step,
            voter: H160::from(value.vote.voter),
        },
        vote_type: PbftVoteType::try_from(value.vote.vote_type)?,
        weight: value.vote.weight,
        vote_already_known: value.vote_already_known,
        carries_proposed_block: value.carries_proposed_block,
        valid_stale_reward_vote: value.valid_stale_reward_vote,
    })
}

pub(crate) fn context_to_domain(value: &FfiPbftVoteProgressContext) -> PbftVoteProgressContext {
    PbftVoteProgressContext {
        current_period: value.current_period,
        max_future_period_delta: value.max_future_period_delta,
        two_t_plus_one_threshold: value
            .has_two_t_plus_one_threshold
            .then_some(value.two_t_plus_one_threshold),
        require_proposed_block_sidecar: value.require_proposed_block_sidecar,
        slashing_enabled: value.slashing_enabled,
    }
}

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
    fact: PbftVoteProgressFact,
    context: FfiPbftVoteProgressContext,
) -> FfiPbftVoteProgressExecutionPlan {
    let slashing = plan.intents.iter().find_map(|intent| match intent {
        PbftVoteProgressIntent::ReportSlashing {
            incoming_vote_hash,
            conflicting_vote_hash,
        } => Some((*incoming_vote_hash, *conflicting_vote_hash)),
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

    let threshold = plan.threshold_decision;
    let two_t_plus_one_kind = threshold.and_then(|decision| decision.two_t_plus_one_kind);
    let persist_two_t_plus_one_votes = threshold.is_some_and(|decision| {
        decision
            .two_t_plus_one_insert_outcome
            .is_some_and(|outcome| outcome.round_found && outcome.inserted)
            && decision.two_t_plus_one_kind.is_some()
            && fact.vote_type != PbftVoteType::Cert
            && fact.identity.period == context.current_period
            && fact.identity.round == context.current_round
    });

    FfiPbftVoteProgressExecutionPlan {
        status: plan.status.as_u8(),
        error_code: error_code(plan.status).to_owned(),
        accepted: matches!(
            plan.status,
            PbftVoteProgressStatus::Accepted | PbftVoteProgressStatus::AcceptedWithProgress
        ),
        report_slashing: slashing.is_some(),
        slashing_incoming_vote_hash: slashing
            .map(|(incoming, _)| incoming)
            .unwrap_or_default()
            .into(),
        slashing_conflicting_vote_hash: slashing
            .map(|(_, conflicting)| conflicting)
            .unwrap_or_default()
            .into(),
        persist_extra_reward_vote: extra_reward_vote.is_some(),
        extra_reward_vote_hash: extra_reward_vote.unwrap_or_default().into(),
        network_t_plus_one_step_updated: threshold
            .is_some_and(|decision| decision.network_t_plus_one_step_updated),
        drive_pbft_progress: drive_progress.is_some(),
        progress_period: drive_progress.map(|(period, _)| period).unwrap_or_default(),
        progress_round: drive_progress.map(|(_, round)| round).unwrap_or_default(),
        persist_two_t_plus_one_votes,
        two_t_plus_one_kind: two_t_plus_one_kind.map(Into::into).unwrap_or_default(),
        two_t_plus_one_period: fact.identity.period,
        two_t_plus_one_round: fact.identity.round,
        two_t_plus_one_step: fact.identity.step,
        two_t_plus_one_block_hash: fact.identity.block_hash.into(),
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

    fn fact(weight: u64) -> FfiPbftVoteProgressFact {
        FfiPbftVoteProgressFact {
            vote: vote(weight),
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
            vote: vote(1),
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
    fn bridge_precheck_flattens_insert_decision_without_durable_effects() {
        let plan = pbft_vote_progress_plan_precheck(fact(1), context()).unwrap();

        assert_eq!(plan.status, 0);
        assert!(plan.should_insert_verified_vote);
        assert!(plan.has_two_t_plus_one_threshold);
        assert_eq!(plan.two_t_plus_one_threshold, 2);
        assert!(plan.error_code.is_empty());
    }

    #[test]
    fn bridge_after_add_flattens_slashing_decision() {
        let mut outcome = add_outcome();
        outcome.conflict_found = true;
        outcome.conflicting_vote_hash = [9; 32];

        let plan = pbft_vote_progress_plan_after_add(fact(1), context(), outcome).unwrap();

        assert_eq!(plan.status, 9);
        assert!(!plan.accepted);
        assert!(plan.report_slashing);
        assert_eq!(plan.slashing_conflicting_vote_hash, [9; 32]);
        assert!(!plan.persist_extra_reward_vote);
        assert!(!plan.persist_two_t_plus_one_votes);
    }

    #[test]
    fn bridge_after_add_plans_current_round_two_t_plus_one_persistence() {
        let mut outcome = add_outcome();
        outcome.inserted = true;
        outcome.total_weight = 2;
        outcome.votes_count = 2;
        outcome.threshold_applied = true;
        outcome.t_plus_one_reached = true;
        outcome.network_t_plus_one_step_updated = true;
        outcome.two_t_plus_one_reached = true;
        outcome.two_t_plus_one_kind_found = true;
        outcome.two_t_plus_one_kind = 0;
        outcome.two_t_plus_one_round_found = true;
        outcome.two_t_plus_one_inserted = true;

        let plan = pbft_vote_progress_plan_after_add(fact(1), context(), outcome).unwrap();

        assert_eq!(plan.status, 2);
        assert!(plan.accepted);
        assert!(plan.drive_pbft_progress);
        assert!(plan.network_t_plus_one_step_updated);
        assert!(plan.persist_two_t_plus_one_votes);
        assert_eq!(plan.two_t_plus_one_kind, 0);
    }

    #[test]
    fn bridge_after_add_defers_reward_persistence_until_acceptance() {
        let mut stale_fact = fact(1);
        stale_fact.vote.period = 9;
        stale_fact.vote.vote_type = 3;
        stale_fact.valid_stale_reward_vote = true;

        let mut ctx = context();
        ctx.current_period = 10;
        ctx.max_future_period_delta = u64::MAX - ctx.current_period;
        let precheck = pbft_vote_progress_plan_precheck(stale_fact, ctx).unwrap();
        assert!(precheck.should_insert_verified_vote);

        let mut stale_fact = fact(1);
        stale_fact.vote.period = 9;
        stale_fact.vote.vote_type = 3;
        stale_fact.valid_stale_reward_vote = true;
        let mut ctx = context();
        ctx.current_period = 10;
        ctx.max_future_period_delta = u64::MAX - ctx.current_period;
        let mut accepted = add_outcome();
        accepted.inserted = true;
        accepted.total_weight = 1;
        accepted.votes_count = 1;
        let plan = pbft_vote_progress_plan_after_add(stale_fact, ctx, accepted).unwrap();

        assert!(plan.accepted);
        assert!(plan.persist_extra_reward_vote);
        assert_eq!(plan.extra_reward_vote_hash, [1; 32]);
    }
}
