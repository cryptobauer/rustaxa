//! Deterministic PBFT manager daemon-tick runtime planning.
//!
//! This module owns the first Rust-side slice of PBFT manager orchestration: the
//! ordered control-flow script for one daemon tick. It is intentionally
//! side-effect-free. C++ supplies already-collected live facts, executes each
//! requested action against the existing manager shell, then reports the result
//! before Rust advances the cursor. Eligible-wallet state is reported after the
//! pre-state cert/round checks so the runtime preserves the legacy branch order.
//!
//! Inputs are a compact `PbftManagerRuntimeTickFact`: current PBFT state,
//! period/round/step telemetry, network availability, sync status, and whether
//! any local wallet is eligible for the current period. Outputs are stable
//! action/status codes and a cursor-managed session.
//!
//! Invariants:
//! - Rust decides the order of manager actions for the tick.
//! - C++ remains the sole owner of live objects, storage writes, network
//!   dispatch, sleeps, and state mutation in this slice.
//! - Early-progress actions such as cert-block push or round advance complete
//!   the session with `restart_loop = true`, matching the old `continue` path.
//! - The active-state vs ineligible-sleep branch is selected from the
//!   `has_eligible_wallet` report supplied after `TryAdvanceRound`.
//! - Branches after `run_certify` and `run_second_finish` are selected only from
//!   explicit report flags returned by the C++ executor.

use std::collections::VecDeque;

/// Stable PBFT manager state codes used by the CXX bridge.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeStateCode {
    /// Value-proposal state.
    ValueProposal,
    /// Filtering / identify-leader state.
    Filter,
    /// Certifying state.
    Certify,
    /// First finish state.
    Finish,
    /// Second finish / polling state.
    FinishPolling,
    /// Unknown bridge state.
    Unknown,
}

impl PbftManagerRuntimeStateCode {
    /// Stable bridge code for the state.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ValueProposal => 0,
            Self::Filter => 1,
            Self::Certify => 2,
            Self::Finish => 3,
            Self::FinishPolling => 4,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::ValueProposal,
            1 => Self::Filter,
            2 => Self::Certify,
            3 => Self::Finish,
            4 => Self::FinishPolling,
            _ => Self::Unknown,
        }
    }
}

/// Runtime status for one Rust-owned PBFT manager tick session.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeStatus {
    /// Session is ready to execute or report the next action.
    Active,
    /// Session completed all actions.
    Complete,
    /// The tick facts were rejected before execution.
    RejectedTick,
    /// Reported action does not match the current cursor.
    ActionMismatch,
    /// The C++ executor reported action failure.
    ActionFailed,
    /// The report was malformed for the current action.
    InvalidReport,
    /// Internal invariant or unknown bridge-code failure.
    ContractError,
    /// Unknown bridge status.
    Unknown,
}

impl PbftManagerRuntimeStatus {
    /// Stable bridge code for the status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::Complete => 1,
            Self::RejectedTick => 2,
            Self::ActionMismatch => 3,
            Self::ActionFailed => 4,
            Self::InvalidReport => 5,
            Self::ContractError => 255,
            Self::Unknown => 254,
        }
    }
}

/// Stable action codes for one PBFT manager daemon tick.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeAction {
    /// Drain synced PBFT blocks into the local chain path.
    ProcessSyncedPbftBlocks,
    /// Broadcast or rebroadcast votes according to existing C++ timers.
    MaybeBroadcastVotes,
    /// Try to push a cert-voted PBFT block into the chain.
    TryPushCertVotesBlock,
    /// Try to advance to a higher round from next-vote facts.
    TryAdvanceRound,
    /// Sleep briefly when the node has no eligible wallet for active steps.
    SleepIneligiblePollingInterval,
    /// Execute value proposal behavior.
    RunValueProposal,
    /// Transition from value proposal to filter state.
    TransitionToFilter,
    /// Execute filtering / leader-identification behavior.
    RunFilter,
    /// Transition from filter to certify state.
    TransitionToCertify,
    /// Execute certifying behavior.
    RunCertify,
    /// Transition from certify to first finish state.
    TransitionToFinish,
    /// Delay certify polling without changing state.
    DelayCertifyPoll,
    /// Execute first finish behavior.
    RunFirstFinish,
    /// Transition from first finish to finish-polling state.
    TransitionToFinishPolling,
    /// Execute second finish / polling behavior.
    RunSecondFinish,
    /// Loop from finish-polling back to first finish.
    LoopBackFinish,
    /// Delay finish-polling without changing state.
    DelayFinishPoll,
    /// Sleep until the next planned step time.
    SleepUntilNextStep,
}

impl PbftManagerRuntimeAction {
    /// Stable bridge code for the action.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ProcessSyncedPbftBlocks => 0,
            Self::MaybeBroadcastVotes => 1,
            Self::TryPushCertVotesBlock => 2,
            Self::TryAdvanceRound => 3,
            Self::SleepIneligiblePollingInterval => 4,
            Self::RunValueProposal => 5,
            Self::TransitionToFilter => 6,
            Self::RunFilter => 7,
            Self::TransitionToCertify => 8,
            Self::RunCertify => 9,
            Self::TransitionToFinish => 10,
            Self::DelayCertifyPoll => 11,
            Self::RunFirstFinish => 12,
            Self::TransitionToFinishPolling => 13,
            Self::RunSecondFinish => 14,
            Self::LoopBackFinish => 15,
            Self::DelayFinishPoll => 16,
            Self::SleepUntilNextStep => 17,
        }
    }

    /// Decodes a stable bridge action code.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::ProcessSyncedPbftBlocks),
            1 => Some(Self::MaybeBroadcastVotes),
            2 => Some(Self::TryPushCertVotesBlock),
            3 => Some(Self::TryAdvanceRound),
            4 => Some(Self::SleepIneligiblePollingInterval),
            5 => Some(Self::RunValueProposal),
            6 => Some(Self::TransitionToFilter),
            7 => Some(Self::RunFilter),
            8 => Some(Self::TransitionToCertify),
            9 => Some(Self::RunCertify),
            10 => Some(Self::TransitionToFinish),
            11 => Some(Self::DelayCertifyPoll),
            12 => Some(Self::RunFirstFinish),
            13 => Some(Self::TransitionToFinishPolling),
            14 => Some(Self::RunSecondFinish),
            15 => Some(Self::LoopBackFinish),
            16 => Some(Self::DelayFinishPoll),
            17 => Some(Self::SleepUntilNextStep),
            _ => None,
        }
    }
}

/// Stable result code for one C++-executed manager action.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeActionResultCode {
    /// Action succeeded and session should continue normally.
    NoProgressContinue,
    /// Action made progress and the manager loop must restart immediately.
    ProgressRestartLoop,
    /// State action completed.
    StateActionDone,
    /// State transition completed.
    TransitionApplied,
    /// Sleep action completed.
    SleepApplied,
    /// C++ executor reported an error.
    ExecutorError,
    /// Unknown bridge result.
    Unknown,
}

impl PbftManagerRuntimeActionResultCode {
    /// Stable bridge code for action report results.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NoProgressContinue => 0,
            Self::ProgressRestartLoop => 1,
            Self::StateActionDone => 2,
            Self::TransitionApplied => 3,
            Self::SleepApplied => 4,
            Self::ExecutorError => 255,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge result code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::NoProgressContinue,
            1 => Self::ProgressRestartLoop,
            2 => Self::StateActionDone,
            3 => Self::TransitionApplied,
            4 => Self::SleepApplied,
            255 => Self::ExecutorError,
            _ => Self::Unknown,
        }
    }
}

/// C++-originated facts for one PBFT manager daemon tick.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerRuntimeTickFact {
    /// Monotonic caller-local tick id for telemetry.
    pub tick_id: u64,
    /// Current PBFT manager state.
    pub state: PbftManagerRuntimeStateCode,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Whether the network handle is currently available.
    pub network_available: bool,
    /// Whether the network reports PBFT sync mode.
    pub network_pbft_syncing: bool,
    /// Initial eligibility snapshot for telemetry. The runtime branch uses the
    /// post-prestate value reported by C++ after `TryAdvanceRound`.
    pub has_eligible_wallet: bool,
}

/// One C++ action report for the Rust-owned PBFT manager runtime cursor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeActionReport {
    /// Cursor returned by the previous session step.
    pub cursor: u32,
    /// Action that C++ executed.
    pub action: PbftManagerRuntimeAction,
    /// Whether the action call itself succeeded.
    pub success: bool,
    /// Stable action result code.
    pub result: PbftManagerRuntimeActionResultCode,
    /// `go_finish_state_` observed after `RunCertify`.
    pub go_finish_state: bool,
    /// `loop_back_finish_state_` observed after `RunSecondFinish`.
    pub loop_back_finish_state: bool,
    /// Current eligible-wallet state after the reported action.
    pub has_eligible_wallet: bool,
    /// Optional error detail from the C++ executor.
    pub error_code: String,
}

/// One Rust-owned session step for C++ execution.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeSessionStep {
    /// Session status.
    pub status: PbftManagerRuntimeStatus,
    /// Cursor for the returned action.
    pub cursor: u32,
    /// Action to execute, if any.
    pub action: Option<PbftManagerRuntimeAction>,
    /// Whether `action` is valid.
    pub has_action: bool,
    /// Whether the session completed all actions.
    pub complete: bool,
    /// Whether C++ should restart the daemon loop immediately.
    pub restart_loop: bool,
    /// Caller-local tick id.
    pub tick_id: u64,
    /// Stable error detail.
    pub error_code: String,
}

/// Stateful Rust cursor for one PBFT manager daemon tick.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeSession {
    /// Immutable tick facts.
    pub fact: PbftManagerRuntimeTickFact,
    /// Current session status.
    pub status: PbftManagerRuntimeStatus,
    /// Pending actions that have not yet been handed to C++.
    pub pending: VecDeque<PbftManagerRuntimeAction>,
    /// Cursor of the next action.
    pub cursor: u32,
    /// Completed restart-loop signal.
    pub restart_loop: bool,
    /// Stable error detail.
    pub error_code: String,
}

fn reject_session(fact: PbftManagerRuntimeTickFact, error_code: &str) -> PbftManagerRuntimeSession {
    PbftManagerRuntimeSession {
        fact,
        status: PbftManagerRuntimeStatus::RejectedTick,
        pending: VecDeque::new(),
        cursor: 0,
        restart_loop: false,
        error_code: error_code.to_string(),
    }
}

fn append_state_script(
    actions: &mut VecDeque<PbftManagerRuntimeAction>,
    state: PbftManagerRuntimeStateCode,
) {
    match state {
        PbftManagerRuntimeStateCode::ValueProposal => {
            actions.push_back(PbftManagerRuntimeAction::RunValueProposal);
            actions.push_back(PbftManagerRuntimeAction::TransitionToFilter);
            actions.push_back(PbftManagerRuntimeAction::SleepUntilNextStep);
        }
        PbftManagerRuntimeStateCode::Filter => {
            actions.push_back(PbftManagerRuntimeAction::RunFilter);
            actions.push_back(PbftManagerRuntimeAction::TransitionToCertify);
            actions.push_back(PbftManagerRuntimeAction::SleepUntilNextStep);
        }
        PbftManagerRuntimeStateCode::Certify => {
            actions.push_back(PbftManagerRuntimeAction::RunCertify);
        }
        PbftManagerRuntimeStateCode::Finish => {
            actions.push_back(PbftManagerRuntimeAction::RunFirstFinish);
            actions.push_back(PbftManagerRuntimeAction::TransitionToFinishPolling);
            actions.push_back(PbftManagerRuntimeAction::SleepUntilNextStep);
        }
        PbftManagerRuntimeStateCode::FinishPolling => {
            actions.push_back(PbftManagerRuntimeAction::RunSecondFinish);
        }
        PbftManagerRuntimeStateCode::Unknown => {}
    }
}

/// Creates a Rust-owned PBFT manager runtime session for one daemon tick.
pub fn create_pbft_manager_runtime_session(
    fact: PbftManagerRuntimeTickFact,
) -> PbftManagerRuntimeSession {
    if fact.state == PbftManagerRuntimeStateCode::Unknown {
        return reject_session(fact, "PBFT_MANAGER_RUNTIME_UNKNOWN_STATE");
    }

    if fact.period == 0 || fact.round == 0 || fact.step == 0 {
        return reject_session(fact, "PBFT_MANAGER_RUNTIME_INVALID_CURSOR");
    }

    let mut pending = VecDeque::new();
    pending.push_back(PbftManagerRuntimeAction::ProcessSyncedPbftBlocks);
    if fact.network_available && !fact.network_pbft_syncing {
        pending.push_back(PbftManagerRuntimeAction::MaybeBroadcastVotes);
        pending.push_back(PbftManagerRuntimeAction::TryPushCertVotesBlock);
    }
    pending.push_back(PbftManagerRuntimeAction::TryAdvanceRound);

    PbftManagerRuntimeSession {
        fact,
        status: PbftManagerRuntimeStatus::Active,
        pending,
        cursor: 0,
        restart_loop: false,
        error_code: String::new(),
    }
}

/// Returns the next action for a PBFT manager runtime session.
pub fn next_pbft_manager_runtime_action(
    session: &PbftManagerRuntimeSession,
) -> PbftManagerRuntimeSessionStep {
    if session.status != PbftManagerRuntimeStatus::Active {
        return PbftManagerRuntimeSessionStep {
            status: session.status,
            cursor: session.cursor,
            action: None,
            has_action: false,
            complete: session.status == PbftManagerRuntimeStatus::Complete,
            restart_loop: session.restart_loop,
            tick_id: session.fact.tick_id,
            error_code: session.error_code.clone(),
        };
    }

    match session.pending.front().copied() {
        Some(action) => PbftManagerRuntimeSessionStep {
            status: PbftManagerRuntimeStatus::Active,
            cursor: session.cursor,
            action: Some(action),
            has_action: true,
            complete: false,
            restart_loop: false,
            tick_id: session.fact.tick_id,
            error_code: String::new(),
        },
        None => PbftManagerRuntimeSessionStep {
            status: PbftManagerRuntimeStatus::Complete,
            cursor: session.cursor,
            action: None,
            has_action: false,
            complete: true,
            restart_loop: session.restart_loop,
            tick_id: session.fact.tick_id,
            error_code: String::new(),
        },
    }
}

fn fail_session(
    mut session: PbftManagerRuntimeSession,
    status: PbftManagerRuntimeStatus,
    error_code: String,
) -> PbftManagerRuntimeSession {
    session.status = status;
    session.pending.clear();
    session.error_code = error_code;
    session
}

fn report_error(report: &PbftManagerRuntimeActionReport, fallback: &str) -> String {
    if report.error_code.is_empty() {
        fallback.to_string()
    } else {
        report.error_code.clone()
    }
}

/// Reports a C++-executed manager action and advances the Rust cursor.
pub fn report_pbft_manager_runtime_action(
    mut session: PbftManagerRuntimeSession,
    report: PbftManagerRuntimeActionReport,
) -> PbftManagerRuntimeSession {
    if session.status != PbftManagerRuntimeStatus::Active {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ContractError,
            "PBFT_MANAGER_RUNTIME_NOT_ACTIVE".to_string(),
        );
    }

    if report.cursor != session.cursor {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ActionMismatch,
            "PBFT_MANAGER_RUNTIME_CURSOR_MISMATCH".to_string(),
        );
    }

    let Some(expected_action) = session.pending.pop_front() else {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ContractError,
            "PBFT_MANAGER_RUNTIME_MISSING_ACTION".to_string(),
        );
    };

    if report.action != expected_action {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ActionMismatch,
            "PBFT_MANAGER_RUNTIME_ACTION_MISMATCH".to_string(),
        );
    }

    if !report.success || report.result == PbftManagerRuntimeActionResultCode::ExecutorError {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ActionFailed,
            report_error(&report, "PBFT_MANAGER_RUNTIME_ACTION_FAILED"),
        );
    }

    match expected_action {
        PbftManagerRuntimeAction::TryPushCertVotesBlock
        | PbftManagerRuntimeAction::TryAdvanceRound => {
            if report.result == PbftManagerRuntimeActionResultCode::ProgressRestartLoop {
                session.status = PbftManagerRuntimeStatus::Complete;
                session.pending.clear();
                session.restart_loop = true;
                session.cursor = session.cursor.saturating_add(1);
                return session;
            }
            if expected_action == PbftManagerRuntimeAction::TryAdvanceRound {
                if report.has_eligible_wallet {
                    append_state_script(&mut session.pending, session.fact.state);
                } else {
                    session
                        .pending
                        .push_back(PbftManagerRuntimeAction::SleepIneligiblePollingInterval);
                }
            }
        }
        PbftManagerRuntimeAction::SleepIneligiblePollingInterval => {
            if report.result != PbftManagerRuntimeActionResultCode::SleepApplied {
                return fail_session(
                    session,
                    PbftManagerRuntimeStatus::InvalidReport,
                    "PBFT_MANAGER_RUNTIME_INELIGIBLE_SLEEP_REPORT_MISMATCH".to_string(),
                );
            }
            session.status = PbftManagerRuntimeStatus::Complete;
            session.pending.clear();
            session.restart_loop = true;
            session.cursor = session.cursor.saturating_add(1);
            return session;
        }
        PbftManagerRuntimeAction::RunCertify => {
            if report.go_finish_state {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::TransitionToFinish);
            } else {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::DelayCertifyPoll);
            }
        }
        PbftManagerRuntimeAction::RunSecondFinish => {
            if report.loop_back_finish_state {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::LoopBackFinish);
            } else {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::DelayFinishPoll);
            }
        }
        _ => {}
    }

    session.cursor = session.cursor.saturating_add(1);
    if session.pending.is_empty() {
        session.status = PbftManagerRuntimeStatus::Complete;
    }
    session
}

/// Marks a PBFT manager runtime session as aborted.
pub fn abort_pbft_manager_runtime_session(
    mut session: PbftManagerRuntimeSession,
) -> PbftManagerRuntimeSession {
    session.status = PbftManagerRuntimeStatus::ContractError;
    session.pending.clear();
    session.restart_loop = false;
    session.error_code = "PBFT_MANAGER_RUNTIME_ABORTED".to_string();
    session
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(state: PbftManagerRuntimeStateCode) -> PbftManagerRuntimeTickFact {
        PbftManagerRuntimeTickFact {
            tick_id: 42,
            state,
            period: 10,
            round: 2,
            step: 3,
            network_available: true,
            network_pbft_syncing: false,
            has_eligible_wallet: true,
        }
    }

    fn report(cursor: u32, action: PbftManagerRuntimeAction) -> PbftManagerRuntimeActionReport {
        PbftManagerRuntimeActionReport {
            cursor,
            action,
            success: true,
            result: PbftManagerRuntimeActionResultCode::StateActionDone,
            go_finish_state: false,
            loop_back_finish_state: false,
            has_eligible_wallet: true,
            error_code: String::new(),
        }
    }

    fn drain_actions(mut session: PbftManagerRuntimeSession) -> Vec<PbftManagerRuntimeAction> {
        let mut actions = Vec::new();
        loop {
            let step = next_pbft_manager_runtime_action(&session);
            if !step.has_action {
                break;
            }
            let action = step.action.expect("action is present");
            actions.push(action);
            let mut action_report = report(step.cursor, action);
            action_report.result = match action {
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                | PbftManagerRuntimeAction::TryAdvanceRound => {
                    PbftManagerRuntimeActionResultCode::NoProgressContinue
                }
                PbftManagerRuntimeAction::TransitionToFilter
                | PbftManagerRuntimeAction::TransitionToCertify
                | PbftManagerRuntimeAction::TransitionToFinish
                | PbftManagerRuntimeAction::TransitionToFinishPolling
                | PbftManagerRuntimeAction::LoopBackFinish => {
                    PbftManagerRuntimeActionResultCode::TransitionApplied
                }
                PbftManagerRuntimeAction::SleepUntilNextStep
                | PbftManagerRuntimeAction::SleepIneligiblePollingInterval
                | PbftManagerRuntimeAction::DelayCertifyPoll
                | PbftManagerRuntimeAction::DelayFinishPoll => {
                    PbftManagerRuntimeActionResultCode::SleepApplied
                }
                _ => PbftManagerRuntimeActionResultCode::StateActionDone,
            };
            session = report_pbft_manager_runtime_action(session, action_report);
        }
        actions
    }

    #[test]
    fn value_proposal_tick_orders_prestate_state_transition_and_sleep() {
        let actions = drain_actions(create_pbft_manager_runtime_session(fact(
            PbftManagerRuntimeStateCode::ValueProposal,
        )));

        assert_eq!(
            actions,
            vec![
                PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
                PbftManagerRuntimeAction::MaybeBroadcastVotes,
                PbftManagerRuntimeAction::TryPushCertVotesBlock,
                PbftManagerRuntimeAction::TryAdvanceRound,
                PbftManagerRuntimeAction::RunValueProposal,
                PbftManagerRuntimeAction::TransitionToFilter,
                PbftManagerRuntimeAction::SleepUntilNextStep,
            ]
        );
    }

    #[test]
    fn network_syncing_skips_broadcast_and_cert_push_but_keeps_round_check() {
        let mut tick = fact(PbftManagerRuntimeStateCode::Filter);
        tick.network_pbft_syncing = true;
        let actions = drain_actions(create_pbft_manager_runtime_session(tick));

        assert_eq!(
            actions,
            vec![
                PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
                PbftManagerRuntimeAction::TryAdvanceRound,
                PbftManagerRuntimeAction::RunFilter,
                PbftManagerRuntimeAction::TransitionToCertify,
                PbftManagerRuntimeAction::SleepUntilNextStep,
            ]
        );
    }

    #[test]
    fn cert_push_progress_completes_with_restart_loop() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        for expected in [
            PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
            PbftManagerRuntimeAction::MaybeBroadcastVotes,
        ] {
            let step = next_pbft_manager_runtime_action(&session);
            assert_eq!(step.action, Some(expected));
            session = report_pbft_manager_runtime_action(session, report(step.cursor, expected));
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(
            step.action,
            Some(PbftManagerRuntimeAction::TryPushCertVotesBlock)
        );
        let mut action_report =
            report(step.cursor, PbftManagerRuntimeAction::TryPushCertVotesBlock);
        action_report.result = PbftManagerRuntimeActionResultCode::ProgressRestartLoop;
        session = report_pbft_manager_runtime_action(session, action_report);

        let final_step = next_pbft_manager_runtime_action(&session);
        assert!(final_step.complete);
        assert!(final_step.restart_loop);
    }

    #[test]
    fn ineligible_wallet_path_sleeps_and_restarts_without_state_action() {
        let mut tick = fact(PbftManagerRuntimeStateCode::ValueProposal);
        tick.has_eligible_wallet = false;
        let mut session = create_pbft_manager_runtime_session(tick);

        loop {
            let step = next_pbft_manager_runtime_action(&session);
            if step.action == Some(PbftManagerRuntimeAction::SleepIneligiblePollingInterval) {
                let mut action_report = report(
                    step.cursor,
                    PbftManagerRuntimeAction::SleepIneligiblePollingInterval,
                );
                action_report.result = PbftManagerRuntimeActionResultCode::SleepApplied;
                session = report_pbft_manager_runtime_action(session, action_report);
                break;
            }
            let action = step.action.expect("action");
            let mut action_report = report(step.cursor, action);
            if action == PbftManagerRuntimeAction::TryAdvanceRound {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
                action_report.has_eligible_wallet = false;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let final_step = next_pbft_manager_runtime_action(&session);
        assert!(final_step.complete);
        assert!(final_step.restart_loop);
    }

    #[test]
    fn certify_branch_uses_reported_go_finish_flag() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Certify));
        loop {
            let step = next_pbft_manager_runtime_action(&session);
            let action = step.action.expect("action");
            if action == PbftManagerRuntimeAction::RunCertify {
                let mut action_report = report(step.cursor, action);
                action_report.go_finish_state = true;
                session = report_pbft_manager_runtime_action(session, action_report);
                break;
            }
            let mut action_report = report(step.cursor, action);
            if matches!(
                action,
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                    | PbftManagerRuntimeAction::TryAdvanceRound
            ) {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(
            step.action,
            Some(PbftManagerRuntimeAction::TransitionToFinish)
        );
    }

    #[test]
    fn second_finish_branch_uses_reported_loopback_flag() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::FinishPolling));
        loop {
            let step = next_pbft_manager_runtime_action(&session);
            let action = step.action.expect("action");
            if action == PbftManagerRuntimeAction::RunSecondFinish {
                let mut action_report = report(step.cursor, action);
                action_report.loop_back_finish_state = true;
                session = report_pbft_manager_runtime_action(session, action_report);
                break;
            }
            let mut action_report = report(step.cursor, action);
            if matches!(
                action,
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                    | PbftManagerRuntimeAction::TryAdvanceRound
            ) {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(step.action, Some(PbftManagerRuntimeAction::LoopBackFinish));
    }

    #[test]
    fn cursor_mismatch_and_unknown_state_are_explicit_errors() {
        let session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Unknown));
        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(step.status, PbftManagerRuntimeStatus::RejectedTick);
        assert_eq!(step.error_code, "PBFT_MANAGER_RUNTIME_UNKNOWN_STATE");

        let session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        let step = next_pbft_manager_runtime_action(&session);
        let mut bad_report = report(step.cursor + 1, step.action.expect("action"));
        bad_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
        let session = report_pbft_manager_runtime_action(session, bad_report);
        assert_eq!(session.status, PbftManagerRuntimeStatus::ActionMismatch);
    }
}
