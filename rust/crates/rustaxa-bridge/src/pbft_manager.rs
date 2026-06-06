//! Bridge wrapper for PBFT manager daemon-tick runtime planning.
//!
//! C++ supplies the current manager state and live-shell facts for one daemon
//! tick. Rust owns the ordered action cursor for that tick, while C++ executes
//! existing manager methods and reports each action result before the session
//! advances.

use crate::ffi::rustaxa_ffi::{
    PbftManagerAdvanceRoundFact as FfiPbftManagerAdvanceRoundFact,
    PbftManagerAdvanceRoundPlan as FfiPbftManagerAdvanceRoundPlan,
    PbftManagerRuntimeActionReport as FfiPbftManagerRuntimeActionReport,
    PbftManagerRuntimeSessionStep as FfiPbftManagerRuntimeSessionStep,
    PbftManagerRuntimeTickFact as FfiPbftManagerRuntimeTickFact,
    PbftManagerStateActionFact as FfiPbftManagerStateActionFact,
    PbftManagerStateActionPlan as FfiPbftManagerStateActionPlan,
    PbftManagerTransitionFact as FfiPbftManagerTransitionFact,
    PbftManagerTransitionPlan as FfiPbftManagerTransitionPlan,
};
use crate::ffi::BridgePbftManagerRuntimeSession;
use rustaxa_consensus::pbft_manager::{
    abort_pbft_manager_runtime_session as abort_domain_pbft_manager_runtime_session,
    create_pbft_manager_runtime_session as create_domain_pbft_manager_runtime_session,
    next_pbft_manager_runtime_action,
    plan_pbft_manager_advance_round as plan_domain_pbft_manager_advance_round,
    plan_pbft_manager_state_action as plan_domain_pbft_manager_state_action,
    plan_pbft_manager_transition as plan_domain_pbft_manager_transition,
    report_pbft_manager_runtime_action, PbftManagerAdvanceRoundFact, PbftManagerAdvanceRoundPlan,
    PbftManagerRuntimeAction, PbftManagerRuntimeActionReport, PbftManagerRuntimeActionResultCode,
    PbftManagerRuntimeSessionStep, PbftManagerRuntimeStateCode, PbftManagerRuntimeTickFact,
    PbftManagerStateActionFact, PbftManagerStateActionPlan, PbftManagerTransitionFact,
    PbftManagerTransitionKind, PbftManagerTransitionPlan,
};

const RUNTIME_STATUS_ACTIVE: u8 = 0;
const RUNTIME_STATUS_COMPLETE: u8 = 1;
const ACTION_NO_ACTION: u8 = 255;

/// Creates an owned PBFT manager runtime session from one daemon-tick fact bundle.
pub fn create_pbft_manager_runtime_session(
    fact: FfiPbftManagerRuntimeTickFact,
) -> Box<BridgePbftManagerRuntimeSession> {
    Box::new(BridgePbftManagerRuntimeSession {
        state: create_domain_pbft_manager_runtime_session(fact.into()),
    })
}

/// Returns the next requested action for this PBFT manager runtime session.
pub fn pbft_manager_runtime_session_next(
    session: &mut BridgePbftManagerRuntimeSession,
) -> FfiPbftManagerRuntimeSessionStep {
    next_pbft_manager_runtime_action(&session.state).into()
}

/// Reports one C++-executed action back to the PBFT manager runtime session.
pub fn pbft_manager_runtime_session_report(
    session: &mut BridgePbftManagerRuntimeSession,
    report: FfiPbftManagerRuntimeActionReport,
) -> FfiPbftManagerRuntimeSessionStep {
    session.state = report_pbft_manager_runtime_action(session.state.clone(), report.into());
    pbft_manager_runtime_session_next(session)
}

/// Aborts this PBFT manager runtime session.
pub fn abort_pbft_manager_runtime_session(session: &mut BridgePbftManagerRuntimeSession) {
    session.state = abort_domain_pbft_manager_runtime_session(session.state.clone());
}

/// Plans one deterministic PBFT manager state action from compact C++ facts.
pub fn plan_pbft_manager_state_action(
    fact: FfiPbftManagerStateActionFact,
) -> FfiPbftManagerStateActionPlan {
    plan_domain_pbft_manager_state_action(fact.into()).into()
}

/// Plans one deterministic PBFT manager transition from compact C++ facts.
pub fn plan_pbft_manager_transition(
    fact: FfiPbftManagerTransitionFact,
) -> FfiPbftManagerTransitionPlan {
    plan_domain_pbft_manager_transition(fact.into()).into()
}

/// Plans whether a PBFT manager round-advance candidate should reset consensus.
pub fn plan_pbft_manager_advance_round(
    fact: FfiPbftManagerAdvanceRoundFact,
) -> FfiPbftManagerAdvanceRoundPlan {
    plan_domain_pbft_manager_advance_round(fact.into()).into()
}

impl BridgePbftManagerRuntimeSession {
    /// Returns the next requested action for this runtime session.
    pub fn pbft_manager_runtime_session_next(&mut self) -> FfiPbftManagerRuntimeSessionStep {
        pbft_manager_runtime_session_next(self)
    }

    /// Reports one action after C++ executes it.
    pub fn pbft_manager_runtime_session_report(
        &mut self,
        report: FfiPbftManagerRuntimeActionReport,
    ) -> FfiPbftManagerRuntimeSessionStep {
        pbft_manager_runtime_session_report(self, report)
    }

    /// Aborts this runtime session.
    pub fn abort_pbft_manager_runtime_session(&mut self) {
        abort_pbft_manager_runtime_session(self)
    }
}

impl From<FfiPbftManagerRuntimeTickFact> for PbftManagerRuntimeTickFact {
    fn from(value: FfiPbftManagerRuntimeTickFact) -> Self {
        Self {
            tick_id: value.tick_id,
            state: PbftManagerRuntimeStateCode::from_u8(value.state),
            period: value.period,
            round: value.round,
            step: value.step,
            network_available: value.network_available,
            network_pbft_syncing: value.network_pbft_syncing,
            has_eligible_wallet: value.has_eligible_wallet,
        }
    }
}

impl From<FfiPbftManagerRuntimeActionReport> for PbftManagerRuntimeActionReport {
    fn from(value: FfiPbftManagerRuntimeActionReport) -> Self {
        Self {
            cursor: value.cursor,
            action: PbftManagerRuntimeAction::from_u8(value.action)
                .unwrap_or(PbftManagerRuntimeAction::Unknown),
            success: value.success,
            result: PbftManagerRuntimeActionResultCode::from_u8(value.result),
            go_finish_state: value.go_finish_state,
            loop_back_finish_state: value.loop_back_finish_state,
            has_eligible_wallet: value.has_eligible_wallet,
            error_code: value.error_code,
        }
    }
}

impl From<FfiPbftManagerStateActionFact> for PbftManagerStateActionFact {
    fn from(value: FfiPbftManagerStateActionFact) -> Self {
        Self {
            state: PbftManagerRuntimeStateCode::from_u8(value.state),
            period: value.period,
            round: value.round,
            step: value.step,
            elapsed_round_ms: value.elapsed_round_ms,
            deadline_ms: value.deadline_ms,
            current_round_lambda_ms: value.current_round_lambda_ms,
            polling_interval_ms: value.polling_interval_ms,
            has_previous_round_next_null: value.has_previous_round_next_null,
            has_previous_round_next_value: value.has_previous_round_next_value,
            previous_round_next_value_hash: value.previous_round_next_value_hash,
            has_current_round_soft_value: value.has_current_round_soft_value,
            current_round_soft_value_hash: value.current_round_soft_value_hash,
            has_cert_voted_block: value.has_cert_voted_block,
            cert_voted_block_hash: value.cert_voted_block_hash,
            already_next_voted_value: value.already_next_voted_value,
            already_next_voted_null: value.already_next_voted_null,
        }
    }
}

impl From<FfiPbftManagerTransitionFact> for PbftManagerTransitionFact {
    fn from(value: FfiPbftManagerTransitionFact) -> Self {
        Self {
            kind: PbftManagerTransitionKind::from_u8(value.kind),
            period: value.period,
            round: value.round,
            step: value.step,
            target_round: value.target_round,
            current_round_lambda_ms: value.current_round_lambda_ms,
            target_round_lambda_ms: value.target_round_lambda_ms,
            default_lambda_ms: value.default_lambda_ms,
            max_exponential_lambda_ms: value.max_exponential_lambda_ms,
            max_steps: value.max_steps,
            network_next_voting_step: value.network_next_voting_step,
            deadline_ms: value.deadline_ms,
            polling_interval_ms: value.polling_interval_ms,
            next_step_time_ms: value.next_step_time_ms,
            cacti_hardfork: value.cacti_hardfork,
            has_cert_voted_block: value.has_cert_voted_block,
            executed_pbft_block: value.executed_pbft_block,
        }
    }
}

impl From<FfiPbftManagerAdvanceRoundFact> for PbftManagerAdvanceRoundFact {
    fn from(value: FfiPbftManagerAdvanceRoundFact) -> Self {
        Self {
            period: value.period,
            current_round: value.current_round,
            has_new_round: value.has_new_round,
            new_round: value.new_round,
        }
    }
}

impl From<PbftManagerRuntimeSessionStep> for FfiPbftManagerRuntimeSessionStep {
    fn from(value: PbftManagerRuntimeSessionStep) -> Self {
        let status = value.status.as_u8();
        Self {
            status,
            cursor: value.cursor,
            action: value
                .action
                .map(PbftManagerRuntimeAction::as_u8)
                .unwrap_or(ACTION_NO_ACTION),
            has_action: value.has_action,
            complete: value.complete,
            restart_loop: value.restart_loop,
            can_continue: status == RUNTIME_STATUS_ACTIVE || status == RUNTIME_STATUS_COMPLETE,
            tick_id: value.tick_id,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerStateActionPlan> for FfiPbftManagerStateActionPlan {
    fn from(value: PbftManagerStateActionPlan) -> Self {
        Self {
            status: value.status.as_u8(),
            primary_intent: value.primary_intent.as_u8(),
            primary_hash: value.primary_hash,
            secondary_intent: value.secondary_intent.as_u8(),
            secondary_hash: value.secondary_hash,
            go_finish_state: value.go_finish_state,
            loop_back_finish_state: value.loop_back_finish_state,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerTransitionPlan> for FfiPbftManagerTransitionPlan {
    fn from(value: PbftManagerTransitionPlan) -> Self {
        Self {
            status: value.status.as_u8(),
            kind: value.kind.as_u8(),
            new_state: value.new_state.as_u8(),
            new_round: value.new_round,
            new_step: value.new_step,
            current_round_lambda_ms: value.current_round_lambda_ms,
            next_step_time_ms: value.next_step_time_ms,
            persist_round: value.persist_round,
            persist_step: value.persist_step,
            reset_next_voted_statuses: value.reset_next_voted_statuses,
            remove_cert_voted_block: value.remove_cert_voted_block,
            clear_own_votes: value.clear_own_votes,
            clear_broadcasted_votes: value.clear_broadcasted_votes,
            reset_broadcast_counters: value.reset_broadcast_counters,
            reset_executed_block_status: value.reset_executed_block_status,
            set_vote_manager_period_round: value.set_vote_manager_period_round,
            reset_current_round_start: value.reset_current_round_start,
            reset_second_finish_start: value.reset_second_finish_start,
            print_cert_step_info: value.print_cert_step_info,
            print_second_finish_step_info: value.print_second_finish_step_info,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerAdvanceRoundPlan> for FfiPbftManagerAdvanceRoundPlan {
    fn from(value: PbftManagerAdvanceRoundPlan) -> Self {
        Self {
            status: value.status.as_u8(),
            should_advance: value.should_advance,
            target_round: value.target_round,
            error_code: value.error_code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE_VALUE_PROPOSAL: u8 = 0;
    const STATE_FILTER: u8 = 1;
    const STATE_CERTIFY: u8 = 2;
    const STATE_FINISH: u8 = 3;
    const ACTION_PROCESS_SYNCED: u8 = 0;
    const ACTION_BROADCAST: u8 = 1;
    const ACTION_TRY_CERT: u8 = 2;
    const ACTION_TRY_ROUND: u8 = 3;
    const ACTION_RUN_CERTIFY: u8 = 9;
    const ACTION_TRANSITION_FINISH: u8 = 10;
    const ACTION_RUN_VALUE_PROPOSAL: u8 = 5;
    const ACTION_RUN_FILTER: u8 = 7;
    const ACTION_RUN_FIRST_FINISH: u8 = 12;
    const RESULT_CONTINUE: u8 = 0;
    const RESULT_PROGRESS_RESTART: u8 = 1;
    const RESULT_STATE_DONE: u8 = 2;
    const RESULT_TRANSITION: u8 = 3;
    const RESULT_SLEEP: u8 = 4;
    const STATE_ACTION_STATUS_READY: u8 = 0;
    const STATE_ACTION_PROPOSE_NEW_BLOCK: u8 = 1;
    const STATE_ACTION_SOFT_VOTE_PREVIOUS_VALUE: u8 = 4;
    const STATE_ACTION_NEXT_VOTE_CERT_BLOCK: u8 = 7;
    const TRANSITION_STATUS_READY: u8 = 0;
    const TRANSITION_STATUS_INVALID_FACT: u8 = 2;
    const TRANSITION_RESET: u8 = 0;
    const TRANSITION_FILTER: u8 = 1;
    const TRANSITION_LOOP_BACK_FINISH: u8 = 5;

    fn fact(state: u8) -> FfiPbftManagerRuntimeTickFact {
        FfiPbftManagerRuntimeTickFact {
            tick_id: 77,
            state,
            period: 10,
            round: 2,
            step: 3,
            network_available: true,
            network_pbft_syncing: false,
            has_eligible_wallet: true,
        }
    }

    fn report(cursor: u32, action: u8, result: u8) -> FfiPbftManagerRuntimeActionReport {
        FfiPbftManagerRuntimeActionReport {
            cursor,
            action,
            success: true,
            result,
            go_finish_state: false,
            loop_back_finish_state: false,
            has_eligible_wallet: true,
            error_code: String::new(),
        }
    }

    #[test]
    fn bridge_session_maps_tick_fact_into_stable_action_order() {
        let mut session = create_pbft_manager_runtime_session(fact(STATE_VALUE_PROPOSAL));

        let mut seen = Vec::new();
        loop {
            let step = pbft_manager_runtime_session_next(&mut session);
            if !step.has_action {
                break;
            }
            seen.push(step.action);
            let result = match step.action {
                ACTION_TRY_CERT | ACTION_TRY_ROUND => RESULT_CONTINUE,
                ACTION_PROCESS_SYNCED
                | ACTION_BROADCAST
                | ACTION_RUN_VALUE_PROPOSAL
                | ACTION_RUN_FILTER
                | ACTION_RUN_CERTIFY
                | ACTION_RUN_FIRST_FINISH => RESULT_STATE_DONE,
                17 => RESULT_SLEEP,
                _ => RESULT_TRANSITION,
            };
            let _ = pbft_manager_runtime_session_report(
                &mut session,
                report(step.cursor, step.action, result),
            );
        }

        assert_eq!(
            seen,
            vec![
                ACTION_PROCESS_SYNCED,
                ACTION_BROADCAST,
                ACTION_TRY_CERT,
                ACTION_TRY_ROUND,
                ACTION_RUN_VALUE_PROPOSAL,
                6,
                17
            ]
        );
    }

    #[test]
    fn bridge_session_uses_certify_report_flag_for_next_action() {
        let mut session = create_pbft_manager_runtime_session(fact(STATE_CERTIFY));
        loop {
            let step = pbft_manager_runtime_session_next(&mut session);
            if step.action == ACTION_RUN_CERTIFY {
                let mut action_report = report(step.cursor, step.action, RESULT_STATE_DONE);
                action_report.go_finish_state = true;
                let next = pbft_manager_runtime_session_report(&mut session, action_report);
                assert_eq!(next.action, ACTION_TRANSITION_FINISH);
                break;
            }
            let result = if step.action == ACTION_TRY_CERT || step.action == ACTION_TRY_ROUND {
                RESULT_CONTINUE
            } else {
                RESULT_STATE_DONE
            };
            let _ = pbft_manager_runtime_session_report(
                &mut session,
                report(step.cursor, step.action, result),
            );
        }
    }

    #[test]
    fn bridge_session_completes_with_restart_loop_on_cert_progress() {
        let mut session = create_pbft_manager_runtime_session(fact(STATE_VALUE_PROPOSAL));
        for expected in [ACTION_PROCESS_SYNCED, ACTION_BROADCAST] {
            let step = pbft_manager_runtime_session_next(&mut session);
            assert_eq!(step.action, expected);
            let _ = pbft_manager_runtime_session_report(
                &mut session,
                report(step.cursor, expected, RESULT_STATE_DONE),
            );
        }

        let step = pbft_manager_runtime_session_next(&mut session);
        assert_eq!(step.action, ACTION_TRY_CERT);
        let complete = pbft_manager_runtime_session_report(
            &mut session,
            report(step.cursor, ACTION_TRY_CERT, RESULT_PROGRESS_RESTART),
        );

        assert!(complete.complete);
        assert!(complete.restart_loop);
    }

    #[test]
    fn bridge_session_detects_cursor_mismatch() {
        let mut session = create_pbft_manager_runtime_session(fact(STATE_VALUE_PROPOSAL));
        let step = pbft_manager_runtime_session_next(&mut session);
        let failed = pbft_manager_runtime_session_report(
            &mut session,
            report(step.cursor + 1, step.action, RESULT_STATE_DONE),
        );

        assert_eq!(failed.status, 3);
        assert!(!failed.can_continue);
    }

    fn state_fact(state: u8) -> FfiPbftManagerStateActionFact {
        FfiPbftManagerStateActionFact {
            state,
            period: 10,
            round: 2,
            step: 3,
            elapsed_round_ms: 250,
            deadline_ms: 1_000,
            current_round_lambda_ms: 100,
            polling_interval_ms: 100,
            has_previous_round_next_null: false,
            has_previous_round_next_value: false,
            previous_round_next_value_hash: [0x44; 32],
            has_current_round_soft_value: false,
            current_round_soft_value_hash: [0x55; 32],
            has_cert_voted_block: false,
            cert_voted_block_hash: [0x66; 32],
            already_next_voted_value: false,
            already_next_voted_null: false,
        }
    }

    fn transition_fact(kind: u8) -> FfiPbftManagerTransitionFact {
        FfiPbftManagerTransitionFact {
            kind,
            period: 10,
            round: 2,
            step: 3,
            target_round: 4,
            current_round_lambda_ms: 100,
            target_round_lambda_ms: 400,
            default_lambda_ms: 100,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            network_next_voting_step: 0,
            deadline_ms: 1_000,
            polling_interval_ms: 100,
            next_step_time_ms: 900,
            cacti_hardfork: true,
            has_cert_voted_block: true,
            executed_pbft_block: true,
        }
    }

    #[test]
    fn bridge_plans_state_action_intents_with_hash_payloads() {
        let mut value_fact = state_fact(STATE_VALUE_PROPOSAL);
        value_fact.has_previous_round_next_null = true;
        let value_plan = plan_pbft_manager_state_action(value_fact);
        assert_eq!(value_plan.status, STATE_ACTION_STATUS_READY);
        assert_eq!(value_plan.primary_intent, STATE_ACTION_PROPOSE_NEW_BLOCK);

        let mut filter_fact = state_fact(1);
        filter_fact.has_previous_round_next_value = true;
        let filter_plan = plan_pbft_manager_state_action(filter_fact);
        assert_eq!(filter_plan.status, STATE_ACTION_STATUS_READY);
        assert_eq!(
            filter_plan.primary_intent,
            STATE_ACTION_SOFT_VOTE_PREVIOUS_VALUE
        );
        assert_eq!(filter_plan.primary_hash, [0x44; 32]);

        let mut finish_fact = state_fact(3);
        finish_fact.has_cert_voted_block = true;
        let finish_plan = plan_pbft_manager_state_action(finish_fact);
        assert_eq!(finish_plan.status, STATE_ACTION_STATUS_READY);
        assert_eq!(
            finish_plan.primary_intent,
            STATE_ACTION_NEXT_VOTE_CERT_BLOCK
        );
        assert_eq!(finish_plan.primary_hash, [0x66; 32]);
    }

    #[test]
    fn bridge_plans_transition_fields_and_reset_effects() {
        let filter = plan_pbft_manager_transition(transition_fact(TRANSITION_FILTER));
        assert_eq!(filter.status, TRANSITION_STATUS_READY);
        assert_eq!(filter.kind, TRANSITION_FILTER);
        assert_eq!(filter.new_state, STATE_FILTER);
        assert_eq!(filter.new_round, 2);
        assert_eq!(filter.new_step, 4);
        assert_eq!(filter.current_round_lambda_ms, 100);
        assert_eq!(filter.next_step_time_ms, 200);
        assert!(filter.persist_step);
        assert!(!filter.persist_round);

        let reset = plan_pbft_manager_transition(transition_fact(TRANSITION_RESET));
        assert_eq!(reset.status, TRANSITION_STATUS_READY);
        assert_eq!(reset.new_state, STATE_VALUE_PROPOSAL);
        assert_eq!(reset.new_round, 4);
        assert_eq!(reset.new_step, 1);
        assert_eq!(reset.current_round_lambda_ms, 400);
        assert!(reset.persist_round);
        assert!(reset.persist_step);
        assert!(reset.reset_next_voted_statuses);
        assert!(reset.remove_cert_voted_block);
        assert!(reset.clear_own_votes);
        assert!(reset.reset_executed_block_status);
        assert!(reset.set_vote_manager_period_round);
    }

    #[test]
    fn bridge_plans_loopback_lambda_backoff_and_round_advance() {
        let mut fact = transition_fact(TRANSITION_LOOP_BACK_FINISH);
        fact.step = 12;
        fact.next_step_time_ms = 900;
        let plan = plan_pbft_manager_transition(fact);
        assert_eq!(plan.status, TRANSITION_STATUS_READY);
        assert_eq!(plan.new_state, STATE_FINISH);
        assert_eq!(plan.new_step, 13);
        assert_eq!(plan.current_round_lambda_ms, 200);
        assert_eq!(plan.next_step_time_ms, 1_000);
        assert!(plan.reset_next_voted_statuses);

        let advance = plan_pbft_manager_advance_round(FfiPbftManagerAdvanceRoundFact {
            period: 10,
            current_round: 2,
            has_new_round: true,
            new_round: 5,
        });
        assert_eq!(advance.status, TRANSITION_STATUS_READY);
        assert!(advance.should_advance);
        assert_eq!(advance.target_round, 5);

        let invalid = plan_pbft_manager_advance_round(FfiPbftManagerAdvanceRoundFact {
            period: 10,
            current_round: 2,
            has_new_round: true,
            new_round: 2,
        });
        assert_eq!(invalid.status, TRANSITION_STATUS_INVALID_FACT);
        assert!(!invalid.should_advance);
    }
}
