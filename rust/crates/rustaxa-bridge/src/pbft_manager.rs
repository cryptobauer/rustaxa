//! Bridge wrapper for PBFT manager daemon-tick runtime planning.
//!
//! C++ supplies the current manager state and live-shell facts for one daemon
//! tick. Rust owns the ordered action cursor for that tick, while C++ executes
//! existing manager methods and reports each action result before the session
//! advances.

use crate::ffi::rustaxa_ffi::{
    PbftManagerRuntimeActionReport as FfiPbftManagerRuntimeActionReport,
    PbftManagerRuntimeSessionStep as FfiPbftManagerRuntimeSessionStep,
    PbftManagerRuntimeTickFact as FfiPbftManagerRuntimeTickFact,
};
use crate::ffi::BridgePbftManagerRuntimeSession;
use rustaxa_consensus::pbft_manager::{
    abort_pbft_manager_runtime_session as abort_domain_pbft_manager_runtime_session,
    create_pbft_manager_runtime_session as create_domain_pbft_manager_runtime_session,
    next_pbft_manager_runtime_action, report_pbft_manager_runtime_action, PbftManagerRuntimeAction,
    PbftManagerRuntimeActionReport, PbftManagerRuntimeActionResultCode,
    PbftManagerRuntimeSessionStep, PbftManagerRuntimeStateCode, PbftManagerRuntimeTickFact,
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
                .unwrap_or(PbftManagerRuntimeAction::ProcessSyncedPbftBlocks),
            success: value.success,
            result: PbftManagerRuntimeActionResultCode::from_u8(value.result),
            go_finish_state: value.go_finish_state,
            loop_back_finish_state: value.loop_back_finish_state,
            has_eligible_wallet: value.has_eligible_wallet,
            error_code: value.error_code,
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

#[cfg(test)]
mod tests {
    use super::*;

    const STATE_VALUE_PROPOSAL: u8 = 0;
    const STATE_CERTIFY: u8 = 2;
    const ACTION_PROCESS_SYNCED: u8 = 0;
    const ACTION_BROADCAST: u8 = 1;
    const ACTION_TRY_CERT: u8 = 2;
    const ACTION_TRY_ROUND: u8 = 3;
    const ACTION_RUN_CERTIFY: u8 = 9;
    const ACTION_TRANSITION_FINISH: u8 = 10;
    const ACTION_RUN_VALUE_PROPOSAL: u8 = 5;
    const RESULT_CONTINUE: u8 = 0;
    const RESULT_PROGRESS_RESTART: u8 = 1;
    const RESULT_STATE_DONE: u8 = 2;
    const RESULT_TRANSITION: u8 = 3;

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
                ACTION_RUN_VALUE_PROPOSAL => RESULT_STATE_DONE,
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
}
