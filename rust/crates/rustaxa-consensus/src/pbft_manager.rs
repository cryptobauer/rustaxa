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
    /// Unknown bridge action code.
    Unknown,
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
            Self::Unknown => 254,
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
            _ => Some(Self::Unknown),
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

/// Stable action-intent codes for deterministic PBFT state actions.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStateActionIntent {
    /// No consensus-side work should be executed for this state action.
    Noop,
    /// Build and propose a fresh PBFT block for the current period/round.
    ProposeNewBlock,
    /// Re-propose the previous round's 2t+1 next-voted value.
    ReproposePreviousRoundNextValue,
    /// Identify the current round leader block and soft-vote it if present.
    IdentifyLeaderAndSoftVote,
    /// Soft-vote the previous round's 2t+1 next-voted value.
    SoftVotePreviousRoundNextValue,
    /// Cert-vote the current round's 2t+1 soft-voted value.
    CertVoteCurrentSoftValue,
    /// Move from certify polling to the finish state.
    GoFinish,
    /// Next-vote the block this node cert-voted in the current round.
    NextVoteCertVotedBlock,
    /// Next-vote the null block hash.
    NextVoteNullBlock,
    /// Next-vote the previous round's 2t+1 next-voted value.
    NextVotePreviousRoundValue,
    /// Next-vote the current round's 2t+1 soft-voted value.
    NextVoteCurrentSoftValue,
}

impl PbftManagerStateActionIntent {
    /// Stable bridge code for the state-action intent.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Noop => 0,
            Self::ProposeNewBlock => 1,
            Self::ReproposePreviousRoundNextValue => 2,
            Self::IdentifyLeaderAndSoftVote => 3,
            Self::SoftVotePreviousRoundNextValue => 4,
            Self::CertVoteCurrentSoftValue => 5,
            Self::GoFinish => 6,
            Self::NextVoteCertVotedBlock => 7,
            Self::NextVoteNullBlock => 8,
            Self::NextVotePreviousRoundValue => 9,
            Self::NextVoteCurrentSoftValue => 10,
        }
    }

    /// Decodes a stable bridge state-action code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::ProposeNewBlock,
            2 => Self::ReproposePreviousRoundNextValue,
            3 => Self::IdentifyLeaderAndSoftVote,
            4 => Self::SoftVotePreviousRoundNextValue,
            5 => Self::CertVoteCurrentSoftValue,
            6 => Self::GoFinish,
            7 => Self::NextVoteCertVotedBlock,
            8 => Self::NextVoteNullBlock,
            9 => Self::NextVotePreviousRoundValue,
            10 => Self::NextVoteCurrentSoftValue,
            _ => Self::Noop,
        }
    }
}

/// Stable status codes for PBFT manager state-action planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStateActionStatus {
    /// The plan is usable by the C++ executor.
    Ready,
    /// The supplied state is unknown or unsupported.
    InvalidState,
    /// The supplied fact bundle is internally inconsistent.
    InvalidFact,
}

impl PbftManagerStateActionStatus {
    /// Stable bridge code for the state-action status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidState => 1,
            Self::InvalidFact => 2,
        }
    }
}

/// C++-originated facts for one PBFT manager state action.
///
/// The fact bundle is intentionally compact and contains only deterministic
/// branch inputs. C++ remains responsible for sourcing those facts from live
/// vote/proposed-block sidecars, executing returned intents, materializing
/// blocks and votes, writing storage, and emitting network effects.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionFact {
    /// State being executed.
    pub state: PbftManagerRuntimeStateCode,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Elapsed milliseconds in the current round.
    pub elapsed_round_ms: u64,
    /// PBFT deadline for the current round in milliseconds.
    pub deadline_ms: u64,
    /// Current round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Polling interval used by the legacy manager loop.
    pub polling_interval_ms: u64,
    /// Whether the previous round has 2t+1 next votes for null.
    pub has_previous_round_next_null: bool,
    /// Whether the previous round has 2t+1 next votes for a block value.
    pub has_previous_round_next_value: bool,
    /// Previous round 2t+1 next-voted block value, when present.
    pub previous_round_next_value_hash: [u8; 32],
    /// Whether the current round has 2t+1 soft votes for a block value.
    pub has_current_round_soft_value: bool,
    /// Current round 2t+1 soft-voted block value, when present.
    pub current_round_soft_value_hash: [u8; 32],
    /// Whether this node already cert-voted a block in this round.
    pub has_cert_voted_block: bool,
    /// Current round cert-voted block hash, when present.
    pub cert_voted_block_hash: [u8; 32],
    /// Whether this node already emitted a next vote for a soft-voted value.
    pub already_next_voted_value: bool,
    /// Whether this node already emitted a null-block next vote.
    pub already_next_voted_null: bool,
}

/// Side-effect-free PBFT manager state-action plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionPlan {
    /// Planning status.
    pub status: PbftManagerStateActionStatus,
    /// Primary action intent for the C++ executor.
    pub primary_intent: PbftManagerStateActionIntent,
    /// Hash argument for the primary intent, if applicable.
    pub primary_hash: [u8; 32],
    /// Secondary action intent for states that can emit two vote attempts.
    pub secondary_intent: PbftManagerStateActionIntent,
    /// Hash argument for the secondary intent, if applicable.
    pub secondary_hash: [u8; 32],
    /// Planned value for `go_finish_state_`.
    pub go_finish_state: bool,
    /// Planned value for `loop_back_finish_state_`.
    pub loop_back_finish_state: bool,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

/// Stable transition-kind codes for PBFT manager cursor mutation planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerTransitionKind {
    /// Reset the consensus cursor for a new round.
    ResetConsensus,
    /// Move from value-proposal to filtering.
    ToFilter,
    /// Move from filtering to certifying.
    ToCertify,
    /// Move from certifying to first finish.
    ToFinish,
    /// Move from first finish to finish polling.
    ToFinishPolling,
    /// Loop from finish polling back to first finish.
    LoopBackFinish,
    /// Delay certify polling without changing phase.
    DelayCertifyPoll,
    /// Delay finish polling without changing phase.
    DelayFinishPoll,
    /// Unknown bridge transition code.
    Unknown,
}

impl PbftManagerTransitionKind {
    /// Stable bridge code for this transition kind.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ResetConsensus => 0,
            Self::ToFilter => 1,
            Self::ToCertify => 2,
            Self::ToFinish => 3,
            Self::ToFinishPolling => 4,
            Self::LoopBackFinish => 5,
            Self::DelayCertifyPoll => 6,
            Self::DelayFinishPoll => 7,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge transition code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::ResetConsensus,
            1 => Self::ToFilter,
            2 => Self::ToCertify,
            3 => Self::ToFinish,
            4 => Self::ToFinishPolling,
            5 => Self::LoopBackFinish,
            6 => Self::DelayCertifyPoll,
            7 => Self::DelayFinishPoll,
            _ => Self::Unknown,
        }
    }
}

/// Stable status codes for PBFT manager transition planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerTransitionStatus {
    /// The transition plan is usable by the C++ executor.
    Ready,
    /// The supplied transition kind is unknown.
    InvalidKind,
    /// The supplied facts are internally inconsistent.
    InvalidFact,
}

impl PbftManagerTransitionStatus {
    /// Stable bridge code for the transition status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidKind => 1,
            Self::InvalidFact => 2,
        }
    }
}

/// C++-originated facts for one PBFT manager cursor/status transition.
///
/// The fact bundle contains only scalar state, timing, and already-sourced
/// network vote progress. Rust decides the resulting manager cursor, lambda,
/// next-step deadline, and manager-status reset intents. C++ remains the
/// executor for storage writes, VoteManager side effects, live status fields,
/// timestamps, and compatibility logging.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerTransitionFact {
    /// Transition kind requested by the runtime cursor.
    pub kind: PbftManagerTransitionKind,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Target round for `ResetConsensus`; ignored by phase transitions.
    pub target_round: u64,
    /// Current round lambda before the transition.
    pub current_round_lambda_ms: u64,
    /// Lambda calculated for the target round under Cacti.
    pub target_round_lambda_ms: u64,
    /// Genesis/default lambda used before Cacti and for exponential reset.
    pub default_lambda_ms: u64,
    /// Maximum exponential lambda.
    pub max_exponential_lambda_ms: u64,
    /// Odd step where exponential backoff starts.
    pub max_steps: u64,
    /// Greatest network t+1 next-voting step already sourced by C++.
    pub network_next_voting_step: u64,
    /// PBFT deadline for the current round in milliseconds.
    pub deadline_ms: u64,
    /// Polling interval used by finish polling.
    pub polling_interval_ms: u64,
    /// Current `next_step_time_ms_`.
    pub next_step_time_ms: u64,
    /// Whether the target period is on the Cacti hardfork.
    pub cacti_hardfork: bool,
    /// Whether a cert-voted block sidecar exists and may need removal.
    pub has_cert_voted_block: bool,
    /// Whether an executed PBFT block flag is set and requires executor reset.
    pub executed_pbft_block: bool,
}

/// Side-effect-free plan for one PBFT manager cursor/status transition.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerTransitionPlan {
    /// Planning status.
    pub status: PbftManagerTransitionStatus,
    /// Transition kind echoed back for executor validation.
    pub kind: PbftManagerTransitionKind,
    /// Planned PBFT state after the transition.
    pub new_state: PbftManagerRuntimeStateCode,
    /// Planned round after the transition.
    pub new_round: u64,
    /// Planned step after the transition.
    pub new_step: u64,
    /// Planned current-round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Planned next-step deadline in milliseconds.
    pub next_step_time_ms: u64,
    /// Persist the planned round field.
    pub persist_round: bool,
    /// Persist the planned step field.
    pub persist_step: bool,
    /// Reset next-voted manager status bits and live flags.
    pub reset_next_voted_statuses: bool,
    /// Remove the saved cert-voted block if present.
    pub remove_cert_voted_block: bool,
    /// Clear local own-vote records through the VoteManager executor.
    pub clear_own_votes: bool,
    /// Clear current-round broadcast bookkeeping.
    pub clear_broadcasted_votes: bool,
    /// Reset current-round broadcast counters.
    pub reset_broadcast_counters: bool,
    /// Reset the executed-block manager status after period finalization.
    pub reset_executed_block_status: bool,
    /// Update the VoteManager period/round executor boundary.
    pub set_vote_manager_period_round: bool,
    /// Reset current round start time in C++.
    pub reset_current_round_start: bool,
    /// Reset second-finish polling start time in C++.
    pub reset_second_finish_start: bool,
    /// Set the certify-step log flag.
    pub print_cert_step_info: bool,
    /// Set the second-finish-step log flag.
    pub print_second_finish_step_info: bool,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

/// Stable status codes for PBFT manager runtime startup restore.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStartupRestoreStatus {
    /// Startup facts are valid and the runtime snapshot is usable.
    Ready,
    /// Startup facts are internally inconsistent or represent corrupted
    /// persisted manager state.
    InvalidFact,
}

impl PbftManagerStartupRestoreStatus {
    /// Stable bridge code for the startup restore status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidFact => 1,
        }
    }
}

/// Persisted and configuration facts used to restore the PBFT manager runtime.
///
/// The fact bundle is deliberately scalar-only. Storage and bridge code read
/// persisted DB values, then Rust decides the normalized PBFT cursor and live
/// startup flags without materializing PBFT blocks, votes, network handles, or
/// FinalChain objects.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStartupRestoreFact {
    /// Current PBFT period at startup.
    pub current_period: u64,
    /// Persisted manager round, defaulted by storage compatibility to `1` when
    /// absent.
    pub persisted_round: u64,
    /// Persisted manager step, defaulted by storage compatibility to `1` when
    /// absent.
    pub persisted_step: u64,
    /// Whether the Cacti dynamic-lambda rules are active for
    /// `current_period - 1`.
    pub cacti_active_at_chain_size: bool,
    /// Persisted rounds-count dynamic-lambda accumulator.
    pub rounds_count_dynamic_lambda: u32,
    /// Persisted dynamic lambda manager field, defaulted by storage
    /// compatibility to `1` when absent.
    pub persisted_dynamic_lambda_ms: u32,
    /// Genesis PBFT lambda used before Cacti.
    pub genesis_lambda_ms: u32,
    /// Cacti maximum lambda used as live default before any finalized Cacti
    /// period has saved a dynamic lambda.
    pub cacti_lambda_max_ms: u32,
    /// Cacti non-round-one lambda.
    pub cacti_lambda_default_ms: u32,
    /// Persisted executed-block manager status.
    pub executed_pbft_block: bool,
    /// Persisted next-voted-value manager status.
    pub already_next_voted_value: bool,
    /// Persisted next-voted-null manager status.
    pub already_next_voted_null: bool,
}

/// Runtime cursor and live scalar facts restored for the PBFT manager shim.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeSnapshot {
    /// Restore/apply status.
    pub status: PbftManagerStartupRestoreStatus,
    /// Current PBFT manager state.
    pub state: PbftManagerRuntimeStateCode,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Current-round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Next-step deadline in milliseconds.
    pub next_step_time_ms: u64,
    /// Live dynamic-lambda accumulator restored from storage.
    pub rounds_count_dynamic_lambda: u32,
    /// Live dynamic lambda in milliseconds.
    pub dynamic_lambda_ms: u32,
    /// Live executed-block flag.
    pub executed_pbft_block: bool,
    /// Live next-voted-value flag.
    pub already_next_voted_value: bool,
    /// Live next-voted-null flag.
    pub already_next_voted_null: bool,
    /// Whether startup normalized persisted step and must persist the new step
    /// before C++ mirrors are updated.
    pub persist_normalized_step: bool,
    /// Whether C++ should reset the second-finish polling timestamp.
    pub reset_second_finish_start: bool,
    /// Stable error detail for rejected startup facts.
    pub error_code: String,
}

/// Long-lived PBFT manager runtime cursor owned by Rust.
///
/// This runtime owns the scalar PBFT manager cursor restored from storage and
/// updated after accepted transition storage commits. It does not own timers,
/// network effects, FinalChain/EVM execution, or live C++ PBFT object
/// materialization.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntime {
    snapshot: PbftManagerRuntimeSnapshot,
}

impl PbftManagerRuntime {
    /// Creates a runtime from an already accepted startup snapshot.
    pub fn new(snapshot: PbftManagerRuntimeSnapshot) -> Self {
        Self { snapshot }
    }

    /// Returns the current Rust-owned scalar snapshot.
    pub fn snapshot(&self) -> PbftManagerRuntimeSnapshot {
        self.snapshot.clone()
    }

    /// Advances the Rust-owned scalar cursor after transition storage commits.
    ///
    /// The caller must only invoke this after the corresponding Rust storage
    /// batch has been committed. Rejected plans are ignored so storage failure
    /// cannot move the in-memory Rust cursor ahead of durable state.
    pub fn apply_committed_transition(&mut self, plan: &PbftManagerTransitionPlan) {
        if plan.status != PbftManagerTransitionStatus::Ready {
            return;
        }

        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.state = plan.new_state;
        self.snapshot.round = plan.new_round;
        self.snapshot.step = plan.new_step;
        self.snapshot.current_round_lambda_ms = plan.current_round_lambda_ms;
        self.snapshot.next_step_time_ms = plan.next_step_time_ms;
        self.snapshot.persist_normalized_step = false;
        self.snapshot.reset_second_finish_start = plan.reset_second_finish_start;
        self.snapshot.error_code.clear();
        if plan.reset_next_voted_statuses {
            self.snapshot.already_next_voted_value = false;
            self.snapshot.already_next_voted_null = false;
        }
    }
}

fn reject_startup_restore(error_code: &str) -> PbftManagerRuntimeSnapshot {
    PbftManagerRuntimeSnapshot {
        status: PbftManagerStartupRestoreStatus::InvalidFact,
        state: PbftManagerRuntimeStateCode::Unknown,
        period: 0,
        round: 0,
        step: 0,
        current_round_lambda_ms: 0,
        next_step_time_ms: 0,
        rounds_count_dynamic_lambda: 0,
        dynamic_lambda_ms: 0,
        executed_pbft_block: false,
        already_next_voted_value: false,
        already_next_voted_null: false,
        persist_normalized_step: false,
        reset_second_finish_start: false,
        error_code: error_code.to_string(),
    }
}

/// Restores the Rust-owned PBFT manager runtime cursor from persisted facts.
///
/// The restored snapshot mirrors legacy startup semantics: missing round/step
/// default to one, steps below four restart in first-finish at step four, even
/// steps restart in first-finish, and odd steps restart in finish-polling. Cacti
/// dynamic lambda is restored from the persisted manager field after at least
/// one Cacti period has finalized; a default `1` value in that case is rejected
/// as corrupted storage to preserve the legacy safety check.
pub fn restore_pbft_manager_runtime(
    fact: PbftManagerStartupRestoreFact,
) -> PbftManagerRuntimeSnapshot {
    if fact.current_period == 0 || fact.persisted_round == 0 || fact.persisted_step == 0 {
        return reject_startup_restore("PBFT_MANAGER_STARTUP_INVALID_CURSOR");
    }
    if fact.genesis_lambda_ms == 0
        || fact.cacti_lambda_max_ms == 0
        || fact.cacti_lambda_default_ms == 0
    {
        return reject_startup_restore("PBFT_MANAGER_STARTUP_INVALID_LAMBDA_CONFIG");
    }

    let chain_size = fact.current_period.saturating_sub(1);
    let dynamic_lambda_ms = if fact.cacti_active_at_chain_size {
        if chain_size >= 1 {
            if fact.persisted_dynamic_lambda_ms == 1 {
                return reject_startup_restore("PBFT_MANAGER_STARTUP_MISSING_DYNAMIC_LAMBDA");
            }
            fact.persisted_dynamic_lambda_ms
        } else {
            fact.cacti_lambda_max_ms
        }
    } else {
        fact.cacti_lambda_max_ms
    };

    let current_round_lambda_ms = if fact.cacti_active_at_chain_size {
        if fact.persisted_round == 1 {
            dynamic_lambda_ms
        } else {
            fact.cacti_lambda_default_ms
        }
    } else {
        fact.genesis_lambda_ms
    };

    let (state, step, persist_normalized_step, reset_second_finish_start) =
        if fact.persisted_round == 1 && fact.persisted_step == 1 {
            (PbftManagerRuntimeStateCode::ValueProposal, 1, false, false)
        } else if fact.persisted_step < 4 {
            (PbftManagerRuntimeStateCode::Finish, 4, true, false)
        } else if fact.persisted_step % 2 == 0 {
            (
                PbftManagerRuntimeStateCode::Finish,
                fact.persisted_step,
                false,
                false,
            )
        } else {
            (
                PbftManagerRuntimeStateCode::FinishPolling,
                fact.persisted_step,
                false,
                true,
            )
        };

    PbftManagerRuntimeSnapshot {
        status: PbftManagerStartupRestoreStatus::Ready,
        state,
        period: fact.current_period,
        round: fact.persisted_round,
        step,
        current_round_lambda_ms: u64::from(current_round_lambda_ms),
        next_step_time_ms: 0,
        rounds_count_dynamic_lambda: if fact.cacti_active_at_chain_size {
            fact.rounds_count_dynamic_lambda
        } else {
            0
        },
        dynamic_lambda_ms,
        executed_pbft_block: fact.executed_pbft_block,
        already_next_voted_value: fact.already_next_voted_value,
        already_next_voted_null: fact.already_next_voted_null,
        persist_normalized_step,
        reset_second_finish_start,
        error_code: String::new(),
    }
}

/// C++-originated facts for deciding whether PBFT can advance to a new round.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvanceRoundFact {
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub current_round: u64,
    /// Whether C++/VoteManager found a candidate new round.
    pub has_new_round: bool,
    /// Candidate new round when present.
    pub new_round: u64,
}

/// Side-effect-free plan for PBFT round advancement.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvanceRoundPlan {
    /// Planning status.
    pub status: PbftManagerTransitionStatus,
    /// Whether C++ should apply a reset transition to `target_round`.
    pub should_advance: bool,
    /// Planned target round when `should_advance` is true.
    pub target_round: u64,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

fn ready_state_action_plan(
    primary_intent: PbftManagerStateActionIntent,
    primary_hash: [u8; 32],
    secondary_intent: PbftManagerStateActionIntent,
    secondary_hash: [u8; 32],
    go_finish_state: bool,
    loop_back_finish_state: bool,
) -> PbftManagerStateActionPlan {
    PbftManagerStateActionPlan {
        status: PbftManagerStateActionStatus::Ready,
        primary_intent,
        primary_hash,
        secondary_intent,
        secondary_hash,
        go_finish_state,
        loop_back_finish_state,
        error_code: String::new(),
    }
}

fn reject_state_action_plan(
    status: PbftManagerStateActionStatus,
    error_code: &str,
) -> PbftManagerStateActionPlan {
    PbftManagerStateActionPlan {
        status,
        primary_intent: PbftManagerStateActionIntent::Noop,
        primary_hash: [0; 32],
        secondary_intent: PbftManagerStateActionIntent::Noop,
        secondary_hash: [0; 32],
        go_finish_state: false,
        loop_back_finish_state: false,
        error_code: error_code.to_string(),
    }
}

fn reject_transition_plan(
    status: PbftManagerTransitionStatus,
    kind: PbftManagerTransitionKind,
    error_code: &str,
) -> PbftManagerTransitionPlan {
    PbftManagerTransitionPlan {
        status,
        kind,
        new_state: PbftManagerRuntimeStateCode::Unknown,
        new_round: 0,
        new_step: 0,
        current_round_lambda_ms: 0,
        next_step_time_ms: 0,
        persist_round: false,
        persist_step: false,
        reset_next_voted_statuses: false,
        remove_cert_voted_block: false,
        clear_own_votes: false,
        clear_broadcasted_votes: false,
        reset_broadcast_counters: false,
        reset_executed_block_status: false,
        set_vote_manager_period_round: false,
        reset_current_round_start: false,
        reset_second_finish_start: false,
        print_cert_step_info: false,
        print_second_finish_step_info: false,
        error_code: error_code.to_string(),
    }
}

fn transition_base_plan(
    fact: &PbftManagerTransitionFact,
    new_state: PbftManagerRuntimeStateCode,
    new_round: u64,
    new_step: u64,
    current_round_lambda_ms: u64,
    next_step_time_ms: u64,
) -> PbftManagerTransitionPlan {
    PbftManagerTransitionPlan {
        status: PbftManagerTransitionStatus::Ready,
        kind: fact.kind,
        new_state,
        new_round,
        new_step,
        current_round_lambda_ms,
        next_step_time_ms,
        persist_round: false,
        persist_step: true,
        reset_next_voted_statuses: false,
        remove_cert_voted_block: false,
        clear_own_votes: false,
        clear_broadcasted_votes: false,
        reset_broadcast_counters: false,
        reset_executed_block_status: false,
        set_vote_manager_period_round: false,
        reset_current_round_start: false,
        reset_second_finish_start: false,
        print_cert_step_info: false,
        print_second_finish_step_info: false,
        error_code: String::new(),
    }
}

fn planned_lambda_for_step(fact: &PbftManagerTransitionFact, new_step: u64) -> u64 {
    if new_step >= fact.max_steps && new_step % 2 == 1 {
        let mut lambda = if new_step == fact.max_steps {
            fact.default_lambda_ms
        } else {
            fact.current_round_lambda_ms
        };
        let catch_up_delay = fact.max_steps.saturating_sub(4);
        if fact.network_next_voting_step > new_step
            && fact.network_next_voting_step - new_step >= catch_up_delay
        {
            fact.default_lambda_ms
        } else if lambda < fact.max_exponential_lambda_ms {
            lambda = lambda.saturating_mul(2).min(fact.max_exponential_lambda_ms);
            lambda
        } else {
            lambda
        }
    } else {
        fact.current_round_lambda_ms
    }
}

fn validate_transition_fact(fact: &PbftManagerTransitionFact) -> Option<&'static str> {
    if fact.kind == PbftManagerTransitionKind::Unknown {
        return Some("PBFT_MANAGER_TRANSITION_UNKNOWN_KIND");
    }
    if fact.period == 0 || fact.round == 0 || fact.step == 0 {
        return Some("PBFT_MANAGER_TRANSITION_INVALID_CURSOR");
    }
    if fact.current_round_lambda_ms == 0
        || fact.default_lambda_ms == 0
        || fact.max_exponential_lambda_ms == 0
        || fact.max_steps == 0
    {
        return Some("PBFT_MANAGER_TRANSITION_INVALID_TIMING_FACTS");
    }
    if fact.kind == PbftManagerTransitionKind::ResetConsensus && fact.target_round == 0 {
        return Some("PBFT_MANAGER_TRANSITION_INVALID_TARGET_ROUND");
    }
    None
}

/// Plans one PBFT manager cursor/status transition from explicit protocol facts.
///
/// The plan is side-effect-free. It owns the deterministic state/round/step,
/// lambda, next-step timing, and manager-status reset decisions. C++ consumes
/// the plan as an executor by persisting fields, updating live compatibility
/// state, clearing sidecars, and setting timestamps.
pub fn plan_pbft_manager_transition(fact: PbftManagerTransitionFact) -> PbftManagerTransitionPlan {
    if let Some(error) = validate_transition_fact(&fact) {
        let status = if fact.kind == PbftManagerTransitionKind::Unknown {
            PbftManagerTransitionStatus::InvalidKind
        } else {
            PbftManagerTransitionStatus::InvalidFact
        };
        return reject_transition_plan(status, fact.kind, error);
    }

    match fact.kind {
        PbftManagerTransitionKind::ResetConsensus => {
            let lambda = if fact.cacti_hardfork {
                fact.target_round_lambda_ms
            } else {
                fact.default_lambda_ms
            };
            if lambda == 0 {
                return reject_transition_plan(
                    PbftManagerTransitionStatus::InvalidFact,
                    fact.kind,
                    "PBFT_MANAGER_TRANSITION_INVALID_RESET_LAMBDA",
                );
            }
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::ValueProposal,
                fact.target_round,
                1,
                lambda,
                fact.next_step_time_ms,
            );
            plan.persist_round = true;
            plan.reset_next_voted_statuses = true;
            plan.remove_cert_voted_block = fact.has_cert_voted_block;
            plan.clear_own_votes = true;
            plan.clear_broadcasted_votes = true;
            plan.reset_broadcast_counters = true;
            plan.reset_executed_block_status = fact.executed_pbft_block;
            plan.set_vote_manager_period_round = true;
            plan.reset_current_round_start = true;
            plan
        }
        PbftManagerTransitionKind::ToFilter => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Filter,
                fact.round,
                new_step,
                lambda,
                lambda.saturating_mul(2),
            )
        }
        PbftManagerTransitionKind::ToCertify => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Certify,
                fact.round,
                new_step,
                lambda,
                lambda.saturating_mul(2),
            );
            plan.print_cert_step_info = true;
            plan
        }
        PbftManagerTransitionKind::ToFinish => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Finish,
                fact.round,
                new_step,
                lambda,
                fact.deadline_ms,
            )
        }
        PbftManagerTransitionKind::ToFinishPolling => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::FinishPolling,
                fact.round,
                new_step,
                lambda,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.reset_next_voted_statuses = true;
            plan.reset_second_finish_start = true;
            plan.print_second_finish_step_info = true;
            plan
        }
        PbftManagerTransitionKind::LoopBackFinish => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Finish,
                fact.round,
                new_step,
                lambda,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.reset_next_voted_statuses = true;
            plan
        }
        PbftManagerTransitionKind::DelayCertifyPoll => {
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Certify,
                fact.round,
                fact.step,
                fact.current_round_lambda_ms,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.persist_step = false;
            plan
        }
        PbftManagerTransitionKind::DelayFinishPoll => {
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::FinishPolling,
                fact.round,
                fact.step,
                fact.current_round_lambda_ms,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.persist_step = false;
            plan
        }
        PbftManagerTransitionKind::Unknown => unreachable!("unknown transition rejected above"),
    }
}

/// Plans whether a PBFT manager round-advance candidate should reset consensus.
///
/// C++ still sources the candidate from the VoteManager/verified-votes runtime,
/// but Rust validates the protocol condition that advancement requires a
/// strictly greater round before C++ applies the reset transition.
pub fn plan_pbft_manager_advance_round(
    fact: PbftManagerAdvanceRoundFact,
) -> PbftManagerAdvanceRoundPlan {
    if fact.period == 0 || fact.current_round == 0 {
        return PbftManagerAdvanceRoundPlan {
            status: PbftManagerTransitionStatus::InvalidFact,
            should_advance: false,
            target_round: 0,
            error_code: "PBFT_MANAGER_ADVANCE_ROUND_INVALID_CURSOR".to_string(),
        };
    }
    if !fact.has_new_round {
        return PbftManagerAdvanceRoundPlan {
            status: PbftManagerTransitionStatus::Ready,
            should_advance: false,
            target_round: 0,
            error_code: String::new(),
        };
    }
    if fact.new_round <= fact.current_round {
        return PbftManagerAdvanceRoundPlan {
            status: PbftManagerTransitionStatus::InvalidFact,
            should_advance: false,
            target_round: 0,
            error_code: "PBFT_MANAGER_ADVANCE_ROUND_NON_INCREASING_ROUND".to_string(),
        };
    }
    PbftManagerAdvanceRoundPlan {
        status: PbftManagerTransitionStatus::Ready,
        should_advance: true,
        target_round: fact.new_round,
        error_code: String::new(),
    }
}

/// Plans one PBFT manager state action from explicit protocol facts.
///
/// The plan is side-effect-free. It deliberately does not validate or
/// materialize PBFT blocks, generate votes, write storage, sleep, gossip, or
/// execute FinalChain/EVM logic. Those remain executor responsibilities around
/// the Rust-owned protocol branch decision.
pub fn plan_pbft_manager_state_action(
    fact: PbftManagerStateActionFact,
) -> PbftManagerStateActionPlan {
    if fact.state == PbftManagerRuntimeStateCode::Unknown {
        return reject_state_action_plan(
            PbftManagerStateActionStatus::InvalidState,
            "PBFT_MANAGER_STATE_ACTION_UNKNOWN_STATE",
        );
    }
    if fact.period == 0 || fact.round == 0 || fact.step == 0 {
        return reject_state_action_plan(
            PbftManagerStateActionStatus::InvalidFact,
            "PBFT_MANAGER_STATE_ACTION_INVALID_CURSOR",
        );
    }

    match fact.state {
        PbftManagerRuntimeStateCode::ValueProposal => plan_value_proposal_state_action(&fact),
        PbftManagerRuntimeStateCode::Filter => plan_filter_state_action(&fact),
        PbftManagerRuntimeStateCode::Certify => plan_certify_state_action(&fact),
        PbftManagerRuntimeStateCode::Finish => plan_first_finish_state_action(&fact),
        PbftManagerRuntimeStateCode::FinishPolling => plan_second_finish_state_action(&fact),
        PbftManagerRuntimeStateCode::Unknown => unreachable!("unknown state rejected above"),
    }
}

fn previous_round_starts_from_null(fact: &PbftManagerStateActionFact) -> bool {
    fact.round == 1 || fact.has_previous_round_next_null
}

fn plan_value_proposal_state_action(
    fact: &PbftManagerStateActionFact,
) -> PbftManagerStateActionPlan {
    if previous_round_starts_from_null(fact) {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::ProposeNewBlock,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_previous_round_next_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::ReproposePreviousRoundNextValue,
            fact.previous_round_next_value_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    reject_state_action_plan(
        PbftManagerStateActionStatus::InvalidFact,
        "PBFT_MANAGER_VALUE_PROPOSAL_MISSING_PREVIOUS_ROUND_STARTING_VALUE",
    )
}

fn plan_filter_state_action(fact: &PbftManagerStateActionFact) -> PbftManagerStateActionPlan {
    if previous_round_starts_from_null(fact) {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::IdentifyLeaderAndSoftVote,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_previous_round_next_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue,
            fact.previous_round_next_value_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    ready_state_action_plan(
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        false,
        false,
    )
}

fn plan_certify_state_action(fact: &PbftManagerStateActionFact) -> PbftManagerStateActionPlan {
    let finish_deadline_ms = fact.deadline_ms.saturating_sub(fact.polling_interval_ms);
    let go_finish_state = fact.elapsed_round_ms > finish_deadline_ms;
    if go_finish_state {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::GoFinish,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            true,
            false,
        );
    }

    if fact.elapsed_round_ms < fact.current_round_lambda_ms.saturating_mul(2) {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_cert_voted_block || !fact.has_current_round_soft_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    ready_state_action_plan(
        PbftManagerStateActionIntent::CertVoteCurrentSoftValue,
        fact.current_round_soft_value_hash,
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        false,
        false,
    )
}

fn plan_first_finish_state_action(fact: &PbftManagerStateActionFact) -> PbftManagerStateActionPlan {
    if fact.has_cert_voted_block {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::NextVoteCertVotedBlock,
            fact.cert_voted_block_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.round >= 2 && fact.has_previous_round_next_null {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::NextVoteNullBlock,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_previous_round_next_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::NextVotePreviousRoundValue,
            fact.previous_round_next_value_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    ready_state_action_plan(
        PbftManagerStateActionIntent::NextVoteNullBlock,
        [0; 32],
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        false,
        false,
    )
}

fn plan_second_finish_state_action(
    fact: &PbftManagerStateActionFact,
) -> PbftManagerStateActionPlan {
    let primary = if !fact.already_next_voted_value && fact.has_current_round_soft_value {
        PbftManagerStateActionIntent::NextVoteCurrentSoftValue
    } else {
        PbftManagerStateActionIntent::Noop
    };
    let primary_hash = if primary == PbftManagerStateActionIntent::NextVoteCurrentSoftValue {
        fact.current_round_soft_value_hash
    } else {
        [0; 32]
    };

    let secondary = if !fact.has_cert_voted_block
        && !fact.already_next_voted_null
        && fact.round >= 2
        && fact.has_previous_round_next_null
    {
        PbftManagerStateActionIntent::NextVoteNullBlock
    } else {
        PbftManagerStateActionIntent::Noop
    };

    let loop_back_finish_state = fact.elapsed_round_ms
        > fact
            .current_round_lambda_ms
            .saturating_sub(fact.polling_interval_ms)
            .saturating_mul(2);

    ready_state_action_plan(
        primary,
        primary_hash,
        secondary,
        [0; 32],
        false,
        loop_back_finish_state,
    )
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

fn valid_action_result(
    action: PbftManagerRuntimeAction,
    result: PbftManagerRuntimeActionResultCode,
) -> bool {
    match action {
        PbftManagerRuntimeAction::TryPushCertVotesBlock
        | PbftManagerRuntimeAction::TryAdvanceRound => matches!(
            result,
            PbftManagerRuntimeActionResultCode::NoProgressContinue
                | PbftManagerRuntimeActionResultCode::ProgressRestartLoop
        ),
        PbftManagerRuntimeAction::TransitionToFilter
        | PbftManagerRuntimeAction::TransitionToCertify
        | PbftManagerRuntimeAction::TransitionToFinish
        | PbftManagerRuntimeAction::TransitionToFinishPolling
        | PbftManagerRuntimeAction::LoopBackFinish => {
            result == PbftManagerRuntimeActionResultCode::TransitionApplied
        }
        PbftManagerRuntimeAction::SleepIneligiblePollingInterval
        | PbftManagerRuntimeAction::DelayCertifyPoll
        | PbftManagerRuntimeAction::DelayFinishPoll
        | PbftManagerRuntimeAction::SleepUntilNextStep => {
            result == PbftManagerRuntimeActionResultCode::SleepApplied
        }
        PbftManagerRuntimeAction::ProcessSyncedPbftBlocks
        | PbftManagerRuntimeAction::MaybeBroadcastVotes
        | PbftManagerRuntimeAction::RunValueProposal
        | PbftManagerRuntimeAction::RunFilter
        | PbftManagerRuntimeAction::RunCertify
        | PbftManagerRuntimeAction::RunFirstFinish
        | PbftManagerRuntimeAction::RunSecondFinish => {
            result == PbftManagerRuntimeActionResultCode::StateActionDone
        }
        PbftManagerRuntimeAction::Unknown => false,
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

    if !valid_action_result(expected_action, report.result) {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::InvalidReport,
            "PBFT_MANAGER_RUNTIME_RESULT_MISMATCH".to_string(),
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

    fn state_fact(state: PbftManagerRuntimeStateCode) -> PbftManagerStateActionFact {
        PbftManagerStateActionFact {
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
            previous_round_next_value_hash: [0x11; 32],
            has_current_round_soft_value: false,
            current_round_soft_value_hash: [0x22; 32],
            has_cert_voted_block: false,
            cert_voted_block_hash: [0x33; 32],
            already_next_voted_value: false,
            already_next_voted_null: false,
        }
    }

    fn transition_fact(kind: PbftManagerTransitionKind) -> PbftManagerTransitionFact {
        PbftManagerTransitionFact {
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

    fn startup_fact(round: u64, step: u64) -> PbftManagerStartupRestoreFact {
        PbftManagerStartupRestoreFact {
            current_period: 10,
            persisted_round: round,
            persisted_step: step,
            cacti_active_at_chain_size: true,
            rounds_count_dynamic_lambda: 7,
            persisted_dynamic_lambda_ms: 1_500,
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
            executed_pbft_block: true,
            already_next_voted_value: true,
            already_next_voted_null: false,
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
            if matches!(
                action,
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                    | PbftManagerRuntimeAction::TryAdvanceRound
            ) {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            if action == PbftManagerRuntimeAction::TryAdvanceRound {
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

    #[test]
    fn runtime_rejects_unknown_action_and_result_codes() {
        let session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        let step = next_pbft_manager_runtime_action(&session);

        let mut bad_action = report(step.cursor, PbftManagerRuntimeAction::Unknown);
        bad_action.result = PbftManagerRuntimeActionResultCode::StateActionDone;
        let failed = report_pbft_manager_runtime_action(session.clone(), bad_action);
        assert_eq!(failed.status, PbftManagerRuntimeStatus::ActionMismatch);

        let mut bad_result = report(step.cursor, step.action.expect("action"));
        bad_result.result = PbftManagerRuntimeActionResultCode::Unknown;
        let failed = report_pbft_manager_runtime_action(session, bad_result);
        assert_eq!(failed.status, PbftManagerRuntimeStatus::InvalidReport);
        assert_eq!(failed.error_code, "PBFT_MANAGER_RUNTIME_RESULT_MISMATCH");
    }

    #[test]
    fn state_action_planner_selects_value_proposal_starting_value() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::ValueProposal);
        fact.has_previous_round_next_null = true;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(plan.status, PbftManagerStateActionStatus::Ready);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::ProposeNewBlock
        );

        fact.has_previous_round_next_null = false;
        fact.has_previous_round_next_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::ReproposePreviousRoundNextValue
        );
        assert_eq!(plan.primary_hash, [0x11; 32]);
    }

    #[test]
    fn state_action_planner_selects_filter_branches() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Filter);
        fact.round = 1;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::IdentifyLeaderAndSoftVote
        );

        fact.round = 2;
        fact.has_previous_round_next_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue
        );
        assert_eq!(plan.primary_hash, [0x11; 32]);
    }

    #[test]
    fn state_action_planner_selects_certify_timeout_and_vote() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Certify);
        fact.elapsed_round_ms = 950;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(plan.primary_intent, PbftManagerStateActionIntent::GoFinish);
        assert!(plan.go_finish_state);

        fact.elapsed_round_ms = 250;
        fact.has_current_round_soft_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::CertVoteCurrentSoftValue
        );
        assert_eq!(plan.primary_hash, [0x22; 32]);
    }

    #[test]
    fn state_action_planner_selects_finish_votes() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Finish);
        fact.has_cert_voted_block = true;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVoteCertVotedBlock
        );
        assert_eq!(plan.primary_hash, [0x33; 32]);

        fact.has_cert_voted_block = false;
        fact.has_previous_round_next_null = true;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVoteNullBlock
        );

        fact.has_previous_round_next_null = false;
        fact.has_previous_round_next_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVotePreviousRoundValue
        );
        assert_eq!(plan.primary_hash, [0x11; 32]);
    }

    #[test]
    fn state_action_planner_selects_second_finish_primary_secondary_and_loopback() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::FinishPolling);
        fact.current_round_lambda_ms = 1_000;
        fact.has_current_round_soft_value = true;
        fact.has_previous_round_next_null = true;
        fact.elapsed_round_ms = 50;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVoteCurrentSoftValue
        );
        assert_eq!(
            plan.secondary_intent,
            PbftManagerStateActionIntent::NextVoteNullBlock
        );
        assert!(!plan.loop_back_finish_state);

        fact.elapsed_round_ms = 2_000;
        fact.already_next_voted_value = true;
        fact.already_next_voted_null = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(plan.primary_intent, PbftManagerStateActionIntent::Noop);
        assert_eq!(plan.secondary_intent, PbftManagerStateActionIntent::Noop);
        assert!(plan.loop_back_finish_state);
    }

    #[test]
    fn startup_restore_normalizes_cursor_and_restores_status_flags() {
        let snapshot = restore_pbft_manager_runtime(startup_fact(2, 2));

        assert_eq!(snapshot.status, PbftManagerStartupRestoreStatus::Ready);
        assert_eq!(snapshot.state, PbftManagerRuntimeStateCode::Finish);
        assert_eq!(snapshot.round, 2);
        assert_eq!(snapshot.step, 4);
        assert_eq!(snapshot.current_round_lambda_ms, 500);
        assert_eq!(snapshot.rounds_count_dynamic_lambda, 7);
        assert_eq!(snapshot.dynamic_lambda_ms, 1_500);
        assert!(snapshot.executed_pbft_block);
        assert!(snapshot.already_next_voted_value);
        assert!(!snapshot.already_next_voted_null);
        assert!(snapshot.persist_normalized_step);
    }

    #[test]
    fn startup_restore_maps_scratch_and_finish_polling_states() {
        let scratch = restore_pbft_manager_runtime(startup_fact(1, 1));
        assert_eq!(scratch.state, PbftManagerRuntimeStateCode::ValueProposal);
        assert_eq!(scratch.step, 1);
        assert!(!scratch.persist_normalized_step);
        assert!(!scratch.reset_second_finish_start);

        let polling = restore_pbft_manager_runtime(startup_fact(4, 5));
        assert_eq!(polling.state, PbftManagerRuntimeStateCode::FinishPolling);
        assert_eq!(polling.step, 5);
        assert!(polling.reset_second_finish_start);
    }

    #[test]
    fn startup_restore_rejects_missing_cacti_dynamic_lambda() {
        let mut fact = startup_fact(1, 1);
        fact.persisted_dynamic_lambda_ms = 1;
        let snapshot = restore_pbft_manager_runtime(fact);

        assert_eq!(
            snapshot.status,
            PbftManagerStartupRestoreStatus::InvalidFact
        );
        assert_eq!(
            snapshot.error_code,
            "PBFT_MANAGER_STARTUP_MISSING_DYNAMIC_LAMBDA"
        );
    }

    #[test]
    fn runtime_snapshot_advances_only_after_committed_transition_report() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(2, 4)));
        let before = runtime.snapshot();
        let rejected = reject_transition_plan(
            PbftManagerTransitionStatus::InvalidFact,
            PbftManagerTransitionKind::ToFilter,
            "rejected",
        );
        runtime.apply_committed_transition(&rejected);
        assert_eq!(runtime.snapshot(), before);

        let plan =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFilter));
        runtime.apply_committed_transition(&plan);
        let after = runtime.snapshot();
        assert_eq!(after.state, PbftManagerRuntimeStateCode::Filter);
        assert_eq!(after.round, 2);
        assert_eq!(after.step, 4);
        assert_eq!(after.current_round_lambda_ms, 100);
        assert_eq!(after.next_step_time_ms, 200);
    }

    #[test]
    fn transition_planner_selects_phase_targets_and_timing() {
        let filter =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFilter));
        assert_eq!(filter.status, PbftManagerTransitionStatus::Ready);
        assert_eq!(filter.new_state, PbftManagerRuntimeStateCode::Filter);
        assert_eq!(filter.new_round, 2);
        assert_eq!(filter.new_step, 4);
        assert_eq!(filter.current_round_lambda_ms, 100);
        assert_eq!(filter.next_step_time_ms, 200);
        assert!(filter.persist_step);

        let certify =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToCertify));
        assert_eq!(certify.new_state, PbftManagerRuntimeStateCode::Certify);
        assert!(certify.print_cert_step_info);

        let finish =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFinish));
        assert_eq!(finish.new_state, PbftManagerRuntimeStateCode::Finish);
        assert_eq!(finish.next_step_time_ms, 1_000);

        let finish_polling = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::ToFinishPolling,
        ));
        assert_eq!(
            finish_polling.new_state,
            PbftManagerRuntimeStateCode::FinishPolling
        );
        assert_eq!(finish_polling.next_step_time_ms, 1_000);
        assert!(finish_polling.reset_next_voted_statuses);
        assert!(finish_polling.reset_second_finish_start);
        assert!(finish_polling.print_second_finish_step_info);

        let delay_certify = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::DelayCertifyPoll,
        ));
        assert_eq!(
            delay_certify.new_state,
            PbftManagerRuntimeStateCode::Certify
        );
        assert_eq!(delay_certify.new_step, 3);
        assert_eq!(delay_certify.next_step_time_ms, 1_000);
        assert!(!delay_certify.persist_step);

        let delay_finish = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::DelayFinishPoll,
        ));
        assert_eq!(
            delay_finish.new_state,
            PbftManagerRuntimeStateCode::FinishPolling
        );
        assert_eq!(delay_finish.new_step, 3);
        assert_eq!(delay_finish.next_step_time_ms, 1_000);
        assert!(!delay_finish.persist_step);
    }

    #[test]
    fn transition_planner_selects_reset_effects() {
        let reset = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::ResetConsensus,
        ));
        assert_eq!(reset.status, PbftManagerTransitionStatus::Ready);
        assert_eq!(reset.new_state, PbftManagerRuntimeStateCode::ValueProposal);
        assert_eq!(reset.new_round, 4);
        assert_eq!(reset.new_step, 1);
        assert_eq!(reset.current_round_lambda_ms, 400);
        assert!(reset.persist_round);
        assert!(reset.persist_step);
        assert!(reset.reset_next_voted_statuses);
        assert!(reset.remove_cert_voted_block);
        assert!(reset.clear_own_votes);
        assert!(reset.clear_broadcasted_votes);
        assert!(reset.reset_broadcast_counters);
        assert!(reset.reset_executed_block_status);
        assert!(reset.set_vote_manager_period_round);
        assert!(reset.reset_current_round_start);
    }

    #[test]
    fn transition_planner_applies_finish_loopback_and_lambda_backoff() {
        let mut fact = transition_fact(PbftManagerTransitionKind::LoopBackFinish);
        fact.step = 12;
        fact.current_round_lambda_ms = 100;
        fact.next_step_time_ms = 900;
        let plan = plan_pbft_manager_transition(fact);

        assert_eq!(plan.status, PbftManagerTransitionStatus::Ready);
        assert_eq!(plan.new_state, PbftManagerRuntimeStateCode::Finish);
        assert_eq!(plan.new_step, 13);
        assert_eq!(plan.current_round_lambda_ms, 200);
        assert_eq!(plan.next_step_time_ms, 1_000);
        assert!(plan.reset_next_voted_statuses);
    }

    #[test]
    fn transition_planner_resets_lambda_when_network_is_far_ahead() {
        let mut fact = transition_fact(PbftManagerTransitionKind::LoopBackFinish);
        fact.step = 14;
        fact.current_round_lambda_ms = 800;
        fact.network_next_voting_step = 24;
        let plan = plan_pbft_manager_transition(fact);

        assert_eq!(plan.new_step, 15);
        assert_eq!(plan.current_round_lambda_ms, 100);
    }

    #[test]
    fn transition_and_advance_planners_reject_invalid_facts() {
        let mut invalid = transition_fact(PbftManagerTransitionKind::ToFilter);
        invalid.step = 0;
        let plan = plan_pbft_manager_transition(invalid);
        assert_eq!(plan.status, PbftManagerTransitionStatus::InvalidFact);
        assert_eq!(plan.error_code, "PBFT_MANAGER_TRANSITION_INVALID_CURSOR");

        let no_candidate = plan_pbft_manager_advance_round(PbftManagerAdvanceRoundFact {
            period: 10,
            current_round: 2,
            has_new_round: false,
            new_round: 0,
        });
        assert_eq!(no_candidate.status, PbftManagerTransitionStatus::Ready);
        assert!(!no_candidate.should_advance);

        let invalid_round = plan_pbft_manager_advance_round(PbftManagerAdvanceRoundFact {
            period: 10,
            current_round: 2,
            has_new_round: true,
            new_round: 2,
        });
        assert_eq!(
            invalid_round.status,
            PbftManagerTransitionStatus::InvalidFact
        );
        assert_eq!(
            invalid_round.error_code,
            "PBFT_MANAGER_ADVANCE_ROUND_NON_INCREASING_ROUND"
        );
    }
}
