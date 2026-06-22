//! Bridge wrapper for PBFT manager daemon-tick runtime planning.
//!
//! C++ supplies the current manager state and live-shell facts for one daemon
//! tick. Rust owns the ordered action cursor for that tick, while C++ executes
//! existing manager methods and reports each action result before the session
//! advances.

use crate::ffi::rustaxa_ffi::{
    BlockPeriodLookup as FfiBlockPeriodLookup, PbftFinalizationHash as FfiPbftFinalizationHash,
    PbftFinalizationResumePlan as FfiPbftFinalizationResumePlan,
    PbftFinalizationStorageWritePlan as FfiPbftFinalizationStorageWritePlan,
    PbftFinalizationStorageWriteStage as FfiPbftFinalizationStorageWriteStage,
    PbftFinalizedPeriodApplyResult as FfiPbftFinalizedPeriodApplyResult,
    PbftManagerAdvancePeriodActionReport as FfiPbftManagerAdvancePeriodActionReport,
    PbftManagerAdvancePeriodActionReportResult as FfiPbftManagerAdvancePeriodActionReportResult,
    PbftManagerAdvancePeriodPlan as FfiPbftManagerAdvancePeriodPlan,
    PbftManagerBlockValidationFact as FfiPbftManagerBlockValidationFact,
    PbftManagerBlockValidationPlan as FfiPbftManagerBlockValidationPlan,
    PbftManagerBroadcastFact as FfiPbftManagerBroadcastFact,
    PbftManagerBroadcastPlan as FfiPbftManagerBroadcastPlan,
    PbftManagerBroadcastReport as FfiPbftManagerBroadcastReport,
    PbftManagerBroadcastReportResult as FfiPbftManagerBroadcastReportResult,
    PbftManagerCandidateAdmissionFact as FfiPbftManagerCandidateAdmissionFact,
    PbftManagerCandidateAdmissionPlan as FfiPbftManagerCandidateAdmissionPlan,
    PbftManagerEligibleWalletPeriodWaitFact as FfiPbftManagerEligibleWalletPeriodWaitFact,
    PbftManagerEligibleWalletPeriodWaitPlan as FfiPbftManagerEligibleWalletPeriodWaitPlan,
    PbftManagerFinalizationWaitFact as FfiPbftManagerFinalizationWaitFact,
    PbftManagerFinalizationWaitPlan as FfiPbftManagerFinalizationWaitPlan,
    PbftManagerLeaderCandidateInputFact as FfiPbftManagerLeaderCandidateInputFact,
    PbftManagerLeaderCandidatePlan as FfiPbftManagerLeaderCandidatePlan,
    PbftManagerLeaderValidBlockCommand as FfiPbftManagerLeaderValidBlockCommand,
    PbftManagerProposalDagBlockFact as FfiPbftManagerProposalDagBlockFact,
    PbftManagerProposalDagOrderReport as FfiPbftManagerProposalDagOrderReport,
    PbftManagerProposalInitialFact as FfiPbftManagerProposalInitialFact,
    PbftManagerProposalSessionStep as FfiPbftManagerProposalSessionStep,
    PbftManagerProposalWalletFact as FfiPbftManagerProposalWalletFact,
    PbftManagerRuntimeActionReport as FfiPbftManagerRuntimeActionReport,
    PbftManagerRuntimeSessionStep as FfiPbftManagerRuntimeSessionStep,
    PbftManagerRuntimeSnapshot as FfiPbftManagerRuntimeSnapshot,
    PbftManagerRuntimeTickFact as FfiPbftManagerRuntimeTickFact,
    PbftManagerSleepFact as FfiPbftManagerSleepFact,
    PbftManagerSleepPlan as FfiPbftManagerSleepPlan,
    PbftManagerStartupFact as FfiPbftManagerStartupFact,
    PbftManagerStartupReplayRangeFact as FfiPbftManagerStartupReplayRangeFact,
    PbftManagerStartupReplayRangePlan as FfiPbftManagerStartupReplayRangePlan,
    PbftManagerStateActionEffect as FfiPbftManagerStateActionEffect,
    PbftManagerStateActionEffectPlan as FfiPbftManagerStateActionEffectPlan,
    PbftManagerStateActionEffectReport as FfiPbftManagerStateActionEffectReport,
    PbftManagerStateActionFact as FfiPbftManagerStateActionFact,
    PbftManagerStateActionPlan as FfiPbftManagerStateActionPlan,
    PbftManagerStateActionSessionStep as FfiPbftManagerStateActionSessionStep,
    PbftManagerTransitionFact as FfiPbftManagerTransitionFact,
    PbftManagerTransitionPlan as FfiPbftManagerTransitionPlan,
    PbftManagerTransitionRuntimeApplyResult as FfiPbftManagerTransitionRuntimeApplyResult,
    PeriodLambda as FfiPeriodLambda,
};
use crate::ffi::{
    BridgePbftManagerBlockValidationSession, BridgePbftManagerProposalSession,
    BridgePbftManagerRuntime, BridgePbftManagerRuntimeSession,
    BridgePbftManagerStateActionEffectSession, BridgeStorage,
};
use anyhow::anyhow;
use rustaxa_consensus::dag::dag_block_period_from_storage;
use rustaxa_consensus::pbft_chain::pbft_block_exists_in_storage;
use rustaxa_consensus::pbft_finalize::{
    apply_pbft_finalization_storage_writes as apply_domain_pbft_finalization_storage_writes,
    inspect_pbft_finalization_resume as inspect_domain_pbft_finalization_resume,
    load_pbft_finalization_last_period_lambda as load_domain_pbft_finalization_last_period_lambda,
    PbftFinalizationStorageWriteIntent,
};
use rustaxa_consensus::pbft_manager::{
    abort_pbft_manager_proposal_session as abort_domain_pbft_manager_proposal_session,
    abort_pbft_manager_runtime_session as abort_domain_pbft_manager_runtime_session,
    apply_executed_block_reset_storage, apply_next_voted_status_storage,
    apply_pbft_manager_cursor_field_storage, apply_pbft_manager_transition_storage,
    create_pbft_manager_block_validation_session as create_domain_pbft_manager_block_validation_session,
    create_pbft_manager_proposal_session as create_domain_pbft_manager_proposal_session,
    create_pbft_manager_runtime_from_storage as create_domain_pbft_manager_runtime_from_storage,
    create_pbft_manager_runtime_session as create_domain_pbft_manager_runtime_session,
    create_pbft_manager_state_action_effect_session as create_domain_pbft_manager_state_action_effect_session,
    load_pbft_manager_startup_replay_period as load_domain_pbft_manager_startup_replay_period,
    next_pbft_manager_block_validation_session as next_domain_pbft_manager_block_validation_session,
    next_pbft_manager_proposal_session as next_domain_pbft_manager_proposal_session,
    next_pbft_manager_runtime_action,
    next_pbft_manager_state_action_effect_session as next_domain_pbft_manager_state_action_effect_session,
    plan_pbft_manager_advance_period_from_transition as plan_domain_pbft_manager_advance_period_from_transition,
    plan_pbft_manager_block_validation as plan_domain_pbft_manager_block_validation,
    plan_pbft_manager_broadcast as plan_domain_pbft_manager_broadcast,
    plan_pbft_manager_candidate_admission as plan_domain_pbft_manager_candidate_admission,
    plan_pbft_manager_eligible_wallet_period_wait as plan_domain_pbft_manager_eligible_wallet_period_wait,
    plan_pbft_manager_finalization_wait as plan_domain_pbft_manager_finalization_wait,
    plan_pbft_manager_leader_candidates as plan_domain_pbft_manager_leader_candidates,
    plan_pbft_manager_runtime_sleep_until_next_step as plan_domain_pbft_manager_runtime_sleep_until_next_step,
    plan_pbft_manager_sleep_until_next_step as plan_domain_pbft_manager_sleep_until_next_step,
    plan_pbft_manager_startup_replay_ranges as plan_domain_pbft_manager_startup_replay_ranges,
    plan_pbft_manager_state_action as plan_domain_pbft_manager_state_action,
    plan_pbft_manager_state_action_effects as plan_domain_pbft_manager_state_action_effects,
    plan_pbft_manager_transition as plan_domain_pbft_manager_transition,
    report_pbft_manager_block_validation_session_check as report_domain_pbft_manager_block_validation_session_check,
    report_pbft_manager_broadcast as report_domain_pbft_manager_broadcast,
    report_pbft_manager_proposal_dag_order as report_domain_pbft_manager_proposal_dag_order,
    report_pbft_manager_runtime_action,
    report_pbft_manager_state_action_effect_session as report_domain_pbft_manager_state_action_effect_session,
    save_cert_voted_block_in_round_storage,
    validate_pbft_manager_advance_period_action_report as validate_domain_pbft_manager_advance_period_action_report,
    PbftManagerAdvancePeriodActionReport, PbftManagerAdvancePeriodActionReportResult,
    PbftManagerAdvancePeriodPlan, PbftManagerBlockValidationFact,
    PbftManagerBlockValidationFactStatus, PbftManagerBlockValidationPlan,
    PbftManagerBroadcastAction, PbftManagerBroadcastFact, PbftManagerBroadcastPlan,
    PbftManagerBroadcastReport, PbftManagerBroadcastReportResult, PbftManagerBroadcastStatus,
    PbftManagerCandidateAdmissionFact, PbftManagerCandidateAdmissionPlan,
    PbftManagerCandidateAdmissionValidationStatus, PbftManagerEligibleWalletPeriodWaitFact,
    PbftManagerEligibleWalletPeriodWaitPlan, PbftManagerFinalizationWaitFact,
    PbftManagerFinalizationWaitPlan, PbftManagerLeaderBlockValidationStatus,
    PbftManagerLeaderCandidateInputFact, PbftManagerLeaderCandidatePlan,
    PbftManagerLeaderValidBlockCommand, PbftManagerProposalDagBlockFact,
    PbftManagerProposalDagOrderReport, PbftManagerProposalInitialFact,
    PbftManagerProposalSessionStep, PbftManagerProposalWalletFact, PbftManagerRuntimeAction,
    PbftManagerRuntimeActionReport, PbftManagerRuntimeActionResultCode,
    PbftManagerRuntimeSessionStep, PbftManagerRuntimeSnapshot, PbftManagerRuntimeStateCode,
    PbftManagerRuntimeTickFact, PbftManagerSleepFact, PbftManagerSleepPlan,
    PbftManagerStartupReplayRangeFact, PbftManagerStartupReplayRangePlan,
    PbftManagerStateActionEffect, PbftManagerStateActionEffectPlan,
    PbftManagerStateActionEffectReport, PbftManagerStateActionEffectResultCode,
    PbftManagerStateActionFact, PbftManagerStateActionIntent, PbftManagerStateActionPlan,
    PbftManagerStateActionSessionStep, PbftManagerStorageStartupFact, PbftManagerTransitionFact,
    PbftManagerTransitionKind, PbftManagerTransitionPlan, PbftManagerTransitionStatus,
    PbftManagerTransitionStorageStatus,
};
use rustaxa_consensus::pillar_chain::load_own_pillar_block_vote_storage;

const RUNTIME_STATUS_ACTIVE: u8 = 0;
const RUNTIME_STATUS_COMPLETE: u8 = 1;
const ACTION_NO_ACTION: u8 = 255;
const TRANSITION_STORAGE_STATUS_APPLIED: u8 = 0;
const TRANSITION_STORAGE_STATUS_REJECTED: u8 = 1;
#[cfg(test)]
const PBFT_MGR_STATUS_EXECUTED_BLOCK: u8 = 0;

fn to_startup_u32(value: u64, field: &str) -> anyhow::Result<u32> {
    u32::try_from(value).map_err(|_| anyhow!("PBFT_MANAGER_STARTUP_{field}_OVERFLOW"))
}

fn transition_status_from_u8(value: u8) -> PbftManagerTransitionStatus {
    match value {
        0 => PbftManagerTransitionStatus::Ready,
        1 => PbftManagerTransitionStatus::InvalidKind,
        2 => PbftManagerTransitionStatus::InvalidFact,
        _ => PbftManagerTransitionStatus::InvalidFact,
    }
}

fn broadcast_status_from_u8(value: u8) -> PbftManagerBroadcastStatus {
    match value {
        0 => PbftManagerBroadcastStatus::Ready,
        1 => PbftManagerBroadcastStatus::InvalidFact,
        2 => PbftManagerBroadcastStatus::ExecutorFailed,
        3 => PbftManagerBroadcastStatus::InvalidReport,
        _ => PbftManagerBroadcastStatus::InvalidReport,
    }
}

fn domain_transition_plan_from_ffi(
    value: &FfiPbftManagerTransitionPlan,
) -> PbftManagerTransitionPlan {
    PbftManagerTransitionPlan {
        status: transition_status_from_u8(value.status),
        kind: PbftManagerTransitionKind::from_u8(value.kind),
        new_state: PbftManagerRuntimeStateCode::from_u8(value.new_state),
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
        error_code: value.error_code.clone(),
    }
}

/// Creates a long-lived Rust PBFT manager runtime from persisted storage facts.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
/// - `fact`: startup period and lambda configuration supplied by the C++ shim.
///
/// Outputs:
/// - A runtime handle seeded with a Rust-owned scalar PBFT manager snapshot.
///
/// Invariants and edge behavior:
/// - Storage remains the source of durable manager fields/statuses.
/// - Missing legacy round/step/lambda fields keep existing compatibility
///   defaults through the storage repository.
/// - Rejected startup facts return a Rust error before C++ mirrors are updated.
pub fn create_pbft_manager_runtime_from_storage(
    storage: &BridgeStorage,
    fact: FfiPbftManagerStartupFact,
) -> anyhow::Result<Box<BridgePbftManagerRuntime>> {
    let runtime = create_domain_pbft_manager_runtime_from_storage(
        &storage.0,
        PbftManagerStorageStartupFact {
            current_period: fact.current_period,
            cacti_active_at_chain_size: fact.cacti_active_at_chain_size,
            genesis_lambda_ms: to_startup_u32(fact.genesis_lambda_ms, "GENESIS_LAMBDA")?,
            cacti_lambda_max_ms: to_startup_u32(fact.cacti_lambda_max_ms, "CACTI_LAMBDA_MAX")?,
            cacti_lambda_default_ms: to_startup_u32(
                fact.cacti_lambda_default_ms,
                "CACTI_LAMBDA_DEFAULT",
            )?,
        },
    )?;

    Ok(Box::new(BridgePbftManagerRuntime {
        state: runtime,
        storage: storage.0.clone(),
    }))
}

/// Loads one finalized period for PBFT manager startup replay through runtime-owned storage.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime with its storage handle.
/// - `period`: finalized PBFT period to replay into temporary live C++ mirrors.
/// - `load_period_lambda`: whether Cacti replay needs closest dynamic-lambda facts.
///
/// Outputs:
/// - A CXX-safe payload containing the raw period data, finalized DAG hashes,
///   and optional dynamic lambda.
///
/// Invariants and edge behavior:
/// - Storage reads and malformed-period handling remain owned by
///   `rustaxa-consensus`; C++ does not pass a generic storage facade after the
///   runtime is constructed.
/// - `found = false` is an explicit missing-period result and does not trigger
///   a legacy storage fallback.
pub fn pbft_manager_runtime_load_startup_replay_period(
    runtime: &BridgePbftManagerRuntime,
    period: u64,
    load_period_lambda: bool,
) -> anyhow::Result<crate::ffi::rustaxa_ffi::PbftManagerStartupReplayPeriod> {
    let replay = load_domain_pbft_manager_startup_replay_period(
        runtime.storage.as_ref(),
        period,
        load_period_lambda,
    )?;
    Ok(crate::ffi::rustaxa_ffi::PbftManagerStartupReplayPeriod {
        found: replay.found,
        period_data_rlp: replay.period_data_rlp,
        finalized_dag_hashes: replay
            .finalized_dag_hashes
            .into_iter()
            .map(|hash| FfiPbftFinalizationHash { hash: hash.0 })
            .collect(),
        has_period_lambda: replay.period_lambda.is_some(),
        period_lambda: replay.period_lambda.unwrap_or_default(),
    })
}

/// Returns the current Rust-owned PBFT manager runtime snapshot.
pub fn pbft_manager_runtime_snapshot(
    runtime: &BridgePbftManagerRuntime,
) -> FfiPbftManagerRuntimeSnapshot {
    runtime.state.snapshot().into()
}

/// Plans PBFT manager startup replay ranges from C++ live height facts.
pub fn plan_pbft_manager_startup_replay_ranges(
    fact: FfiPbftManagerStartupReplayRangeFact,
) -> FfiPbftManagerStartupReplayRangePlan {
    plan_domain_pbft_manager_startup_replay_ranges(fact.into()).into()
}

/// Plans the ordered PBFT manager period-advance effects.
///
/// The caller supplies the accepted Rust transition plan that will reset the
/// PBFT cursor to round one. The returned effect list owns the surrounding
/// advance-period order while C++ remains the temporary executor.
pub fn plan_pbft_manager_advance_period(
    pbft_chain_size: u64,
    transition_plan: &FfiPbftManagerTransitionPlan,
) -> FfiPbftManagerAdvancePeriodPlan {
    plan_domain_pbft_manager_advance_period_from_transition(
        pbft_chain_size,
        domain_transition_plan_from_ffi(transition_plan),
    )
    .into()
}

/// Validates one C++ executor report for a Rust-planned PBFT manager period-advance action.
///
/// Inputs:
/// - `plan`: Rust-owned action script returned by `plan_pbft_manager_advance_period`.
/// - `report`: zero-based executor report for one action.
///
/// Outputs:
/// - Accepted only when the report matches the action at the same script index
///   and the executor marks it successful.
///
/// Invariants and edge behavior:
/// - The validation is side-effect free and does not commit the final runtime
///   period. C++ must report every action before calling
///   `pbft_manager_runtime_apply_period_advance`.
pub fn validate_pbft_manager_advance_period_action_report(
    plan: &FfiPbftManagerAdvancePeriodPlan,
    report: FfiPbftManagerAdvancePeriodActionReport,
) -> FfiPbftManagerAdvancePeriodActionReportResult {
    validate_domain_pbft_manager_advance_period_action_report(
        &domain_advance_period_plan_from_ffi(plan),
        report.into(),
    )
    .into()
}

/// Records a completed Rust-planned period advance in the long-lived runtime.
pub fn pbft_manager_runtime_apply_period_advance(
    runtime: &mut BridgePbftManagerRuntime,
    new_period: u64,
) -> FfiPbftManagerRuntimeSnapshot {
    runtime
        .state
        .apply_committed_period_advance(new_period)
        .into()
}

/// Records live broadcast counters in the long-lived PBFT manager runtime.
///
/// Inputs:
/// - `runtime`: Rust-owned scalar PBFT manager runtime.
/// - The four counters are the post-executor values selected by the Rust
///   broadcast planner/report contract, or compatibility reset values for
///   force-broadcast and reward-vote reset effects.
///
/// Outputs:
/// - Returns a runtime snapshot that C++ may use to hydrate temporary
///   compatibility mirrors.
///
/// Invariants and edge behavior:
/// - This does not write durable storage; broadcast counters are live runtime
///   state only.
/// - Zero counters are rejected by the runtime and returned as an invalid
///   snapshot without mutating the previous counter state.
pub fn pbft_manager_runtime_apply_broadcast_counters(
    runtime: &mut BridgePbftManagerRuntime,
    broadcast_votes_counter: u32,
    rebroadcast_votes_counter: u32,
    broadcast_reward_votes_counter: u32,
    rebroadcast_reward_votes_counter: u32,
) -> FfiPbftManagerRuntimeSnapshot {
    runtime
        .state
        .apply_committed_broadcast_counters(
            broadcast_votes_counter,
            rebroadcast_votes_counter,
            broadcast_reward_votes_counter,
            rebroadcast_reward_votes_counter,
        )
        .into()
}

/// Loads the persisted cert-voted PBFT block payload through the runtime-owned
/// Rust storage handle.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime created from Rust storage.
///
/// Outputs:
/// - The legacy two-item RLP payload `[round, pbft_block]` when a recovery
///   block is present.
/// - An empty byte vector when no cert-voted block is persisted.
///
/// Invariants and edge behavior:
/// - This is a read-only compatibility view; Rust storage remains the source of
///   truth and C++ may only decode the returned payload to populate temporary
///   live sidecars.
/// - Storage read errors are returned to C++ instead of being mapped to an
///   empty result.
pub fn pbft_manager_runtime_cert_voted_block_in_round(
    runtime: &BridgePbftManagerRuntime,
) -> anyhow::Result<Vec<u8>> {
    Ok(runtime
        .storage
        .pbft()
        .cert_voted_block_in_round_rlp()?
        .unwrap_or_default())
}

/// Persists the latest cert-voted PBFT block and records its runtime metadata.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime created from Rust storage.
/// - `period`: PBFT period of the cert-voted block.
/// - `round`: PBFT round that produced the cert vote.
/// - `block_hash`: hash of the cert-voted PBFT block.
/// - `block_rlp`: canonical signed PBFT block RLP payload.
///
/// Outputs:
/// - Returns the updated runtime snapshot after the legacy cert-voted-block
///   recovery row is written.
///
/// Invariants and edge behavior:
/// - The bridge adapts CXX-safe bytes plus compact metadata; row encoding and
///   validation live in `rustaxa-consensus`.
/// - C++ must update its live `cert_voted_block_for_round_` sidecar only after
///   this call succeeds and the returned snapshot is ready.
pub fn pbft_manager_runtime_save_cert_voted_block_in_round(
    runtime: &mut BridgePbftManagerRuntime,
    period: u64,
    round: u32,
    block_hash: [u8; 32],
    block_rlp: Vec<u8>,
) -> anyhow::Result<FfiPbftManagerRuntimeSnapshot> {
    save_cert_voted_block_in_round_storage(runtime.storage.as_ref(), u64::from(round), &block_rlp)?;
    Ok(pbft_manager_runtime_apply_cert_voted_block_metadata(
        runtime, period, round, block_hash,
    ))
}

/// Records cert-voted block metadata after an existing recovery payload was materialized.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime created from Rust storage.
/// - `period`, `round`, and `block_hash` identify the already persisted
///   cert-voted block sidecar.
///
/// Outputs:
/// - Returns the updated runtime snapshot for compatibility mirror checks.
///
/// Invariants and edge behavior:
/// - This method does not write storage; use it only after loading an existing
///   recovery row or after another path has already persisted the row.
/// - C++ may still keep a temporary `PbftBlock` object for APIs and vote
///   generation, but Rust owns the compact metadata used by protocol planners.
pub fn pbft_manager_runtime_apply_cert_voted_block_metadata(
    runtime: &mut BridgePbftManagerRuntime,
    period: u64,
    round: u32,
    block_hash: [u8; 32],
) -> FfiPbftManagerRuntimeSnapshot {
    runtime
        .state
        .apply_committed_cert_voted_block(
            period,
            u64::from(round),
            ethereum_types::H256::from(block_hash),
        )
        .into()
}

/// Checks Rust-owned DAG-order cache membership metadata.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime.
/// - `anchor_hash`: PBFT pivot DAG block hash whose materialized sidecar
///   availability is being queried.
///
/// Outputs:
/// - Returns whether the C++ compatibility shell has reported a materialized
///   DAG-order sidecar for the anchor.
///
/// Invariants and edge behavior:
/// - Rust owns compact membership metadata only. C++ still owns the temporary
///   `DagBlock` vector sidecar used by FinalChain/finalization executors.
/// - The result is live runtime state and is not persisted.
pub fn pbft_manager_runtime_has_cached_anchor_dag_order(
    runtime: &BridgePbftManagerRuntime,
    anchor_hash: &[u8; 32],
) -> bool {
    runtime
        .state
        .has_cached_anchor_dag_order(ethereum_types::H256::from(*anchor_hash))
}

/// Returns the count of Rust-owned DAG-order cache membership entries.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime.
///
/// Outputs:
/// - Number of materialized anchor DAG-order sidecars still tracked in Rust
///   runtime metadata.
///
/// Invariants and edge behavior:
/// - This is live metadata, not durable storage. Finalization cleanup reports
///   use it to prove the Rust-side cache was cleared before the runtime cursor
///   advances.
pub fn pbft_manager_runtime_cached_anchor_dag_order_count(
    runtime: &BridgePbftManagerRuntime,
) -> u64 {
    runtime.state.cached_anchor_dag_order_count()
}

/// Records Rust-owned DAG-order cache membership metadata.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime.
/// - `anchor_hash`: PBFT pivot DAG block hash whose materialized sidecar has
///   been accepted by the compatibility executor.
///
/// Outputs:
/// - Returns the runtime snapshot for bridge consistency; scalar fields are
///   unchanged.
///
/// Invariants and edge behavior:
/// - This must be called only after C++ has materialized the matching DAG-order
///   sidecar or refreshed an existing one.
/// - Re-recording an anchor is idempotent.
pub fn pbft_manager_runtime_record_cached_anchor_dag_order(
    runtime: &mut BridgePbftManagerRuntime,
    anchor_hash: [u8; 32],
) -> FfiPbftManagerRuntimeSnapshot {
    runtime
        .state
        .record_cached_anchor_dag_order(ethereum_types::H256::from(anchor_hash))
        .into()
}

/// Removes Rust-owned DAG-order cache membership metadata for one anchor.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime.
/// - `anchor_hash`: PBFT pivot DAG block hash whose materialized sidecar has
///   been erased or rejected.
///
/// Outputs:
/// - Returns the runtime snapshot for bridge consistency; scalar fields are
///   unchanged.
///
/// Invariants and edge behavior:
/// - Removing a missing anchor is idempotent and leaves scalar runtime state
///   untouched.
pub fn pbft_manager_runtime_remove_cached_anchor_dag_order(
    runtime: &mut BridgePbftManagerRuntime,
    anchor_hash: [u8; 32],
) -> FfiPbftManagerRuntimeSnapshot {
    runtime
        .state
        .remove_cached_anchor_dag_order(ethereum_types::H256::from(anchor_hash))
        .into()
}

/// Clears all Rust-owned DAG-order cache membership metadata.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime.
///
/// Outputs:
/// - Returns the runtime snapshot for bridge consistency; scalar fields are
///   unchanged.
///
/// Invariants and edge behavior:
/// - C++ calls this after clearing the period-scoped materialized DAG-order
///   sidecar map during finalization cleanup.
pub fn pbft_manager_runtime_clear_cached_anchor_dag_order(
    runtime: &mut BridgePbftManagerRuntime,
) -> FfiPbftManagerRuntimeSnapshot {
    runtime.state.clear_cached_anchor_dag_order().into()
}

/// Loads the local node's own pillar-block vote through PBFT-manager runtime storage.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime created from Rust storage.
///
/// Outputs:
/// - Returns C++-decodable `PillarVote` RLP bytes, or empty bytes when no own
///   pillar vote is stored.
///
/// Invariants and edge behavior:
/// - The durable read is owned by `rustaxa-consensus::pillar_chain` over the
///   PBFT manager runtime's storage handle.
/// - C++ still materializes the temporary `PillarVote` sidecar for network
///   gossip until pillar vote network payloads move to Rust.
pub fn pbft_manager_runtime_own_pillar_block_vote(
    runtime: &BridgePbftManagerRuntime,
) -> anyhow::Result<Vec<u8>> {
    load_own_pillar_block_vote_storage(runtime.storage.as_ref())
}

fn transition_runtime_apply_result(
    status: u8,
    applied_writes: u64,
    snapshot: PbftManagerRuntimeSnapshot,
    error_code: String,
) -> FfiPbftManagerTransitionRuntimeApplyResult {
    FfiPbftManagerTransitionRuntimeApplyResult {
        status,
        applied_writes,
        snapshot: snapshot.into(),
        error_code,
    }
}

/// Applies transition persistence through a long-lived PBFT manager runtime.
///
/// Inputs:
/// - `runtime`: Rust-owned scalar PBFT manager cursor.
/// - `plan`: accepted transition plan.
/// - `own_vote_hashes`: latest own-vote keys to delete when requested by the
///   plan.
///
/// Outputs:
/// - `status = 0` after storage commit and Rust cursor update.
/// - `status = 1` with an unchanged snapshot on rejection.
///
/// Invariants and edge behavior:
/// - The runtime-owned Rust storage handle performs the durable commit.
/// - The Rust runtime cursor advances only after that storage batch commits.
/// - C++ live mirrors must be updated from the returned snapshot only after an
///   applied status.
pub fn pbft_manager_runtime_apply_transition_storage_write(
    runtime: &mut BridgePbftManagerRuntime,
    plan: FfiPbftManagerTransitionPlan,
    own_vote_hashes: Vec<FfiPbftFinalizationHash>,
) -> anyhow::Result<FfiPbftManagerTransitionRuntimeApplyResult> {
    let domain_plan = domain_transition_plan_from_ffi(&plan);
    let own_vote_hashes: Vec<_> = own_vote_hashes
        .into_iter()
        .map(|hash| ethereum_types::H256::from(hash.hash))
        .collect();
    let storage_result = apply_pbft_manager_transition_storage(
        runtime.storage.as_ref(),
        &domain_plan,
        &own_vote_hashes,
        false,
    )?;
    if storage_result.status != PbftManagerTransitionStorageStatus::Applied {
        return Ok(transition_runtime_apply_result(
            TRANSITION_STORAGE_STATUS_REJECTED,
            0,
            runtime.state.snapshot(),
            storage_result.error_code,
        ));
    }

    runtime.state.apply_committed_transition(&domain_plan);
    Ok(transition_runtime_apply_result(
        TRANSITION_STORAGE_STATUS_APPLIED,
        storage_result.applied_writes,
        runtime.state.snapshot(),
        String::new(),
    ))
}

/// Applies the delayed executed-block manager-status reset through Rust storage.
///
/// Inputs:
/// - `runtime`: Rust-owned scalar PBFT manager cursor.
///
/// Outputs:
/// - `status = 0` after the durable `ExecutedBlock` status is set to false and
///   the runtime snapshot is updated.
/// - `status = 1` with the prior snapshot when storage rejects the write.
///
/// Invariants and edge behavior:
/// - C++ must call this only after preserving the legacy
///   `waitForPeriodFinalization()` ordering.
/// - The runtime-owned Rust storage handle performs the durable write.
/// - The Rust runtime changes only after that Rust storage write succeeds.
/// - The returned snapshot is the authoritative source for C++ live mirrors.
pub fn pbft_manager_runtime_apply_executed_block_reset(
    runtime: &mut BridgePbftManagerRuntime,
) -> anyhow::Result<FfiPbftManagerTransitionRuntimeApplyResult> {
    if apply_executed_block_reset_storage(runtime.storage.as_ref()).is_err() {
        return Ok(transition_runtime_apply_result(
            TRANSITION_STORAGE_STATUS_REJECTED,
            0,
            runtime.state.snapshot(),
            "PBFT_MANAGER_EXECUTED_BLOCK_RESET_WRITE_FAILURE".to_string(),
        ));
    }

    runtime.state.apply_committed_executed_block_reset();
    Ok(transition_runtime_apply_result(
        TRANSITION_STORAGE_STATUS_APPLIED,
        1,
        runtime.state.snapshot(),
        String::new(),
    ))
}

/// Applies a successful next-vote PBFT manager status through runtime-owned storage.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime with its storage handle.
/// - `status`: stable PBFT manager status id for next-voted soft value or
///   next-voted null-block hash.
///
/// Outputs:
/// - Returns the updated runtime snapshot after the status row is durably set
///   to `true` and the runtime snapshot is updated.
///
/// Invariants and edge behavior:
/// - Vote generation, gossip, and temporary C++ live mirrors remain shim
///   executor side effects until the state-action executor moves fully to Rust.
/// - Unsupported status ids are rejected by `rustaxa-consensus`; this is not a
///   generic PBFT manager status write bridge.
pub fn pbft_manager_runtime_apply_next_voted_status(
    runtime: &mut BridgePbftManagerRuntime,
    status: u8,
) -> anyhow::Result<FfiPbftManagerRuntimeSnapshot> {
    apply_next_voted_status_storage(runtime.storage.as_ref(), status)?;
    runtime.state.apply_committed_next_voted_status(status);
    Ok(runtime.state.snapshot().into())
}

/// Applies a PBFT manager cursor field through runtime-owned Rust storage.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime with its storage handle.
/// - `field`: stable PBFT manager field id for round or step.
/// - `value`: absolute cursor value to persist.
///
/// Outputs:
/// - Returns the updated Rust runtime snapshot after the durable storage write
///   succeeds.
///
/// Invariants and edge behavior:
/// - This is not a generic PBFT manager field write. Dynamic-lambda writes stay
///   owned by the finalization/dynamic-lambda storage paths.
/// - The runtime snapshot changes only after Rust storage accepts the write.
pub fn pbft_manager_runtime_apply_cursor_field(
    runtime: &mut BridgePbftManagerRuntime,
    field: u8,
    value: u32,
) -> anyhow::Result<FfiPbftManagerRuntimeSnapshot> {
    apply_pbft_manager_cursor_field_storage(runtime.storage.as_ref(), field, value)?;
    runtime.state.apply_committed_cursor_field(field, value);
    Ok(runtime.state.snapshot().into())
}

/// Records an accepted dynamic-lambda finalization stage in the Rust runtime.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime with its storage handle.
/// - `rounds_count_dynamic_lambda`: post-adjust accumulator from the accepted
///   Rust dynamic-lambda planner/storage stage.
/// - `dynamic_lambda_ms`: post-adjust dynamic lambda from the accepted Rust
///   dynamic-lambda planner/storage stage.
///
/// Outputs:
/// - Returns the updated Rust runtime snapshot for C++ compatibility mirror
///   hydration.
///
/// Invariants and edge behavior:
/// - This bridge function does not write storage. The caller must invoke it
///   only after `pbft_manager_runtime_apply_finalization_storage_writes`
///   accepts the dynamic-lambda stage, keeping storage authority with the
///   finalization storage runtime.
pub fn pbft_manager_runtime_apply_dynamic_lambda(
    runtime: &mut BridgePbftManagerRuntime,
    rounds_count_dynamic_lambda: u32,
    dynamic_lambda_ms: u32,
) -> FfiPbftManagerRuntimeSnapshot {
    runtime
        .state
        .apply_committed_dynamic_lambda(rounds_count_dynamic_lambda, dynamic_lambda_ms)
        .into()
}

/// Records the executed-PBFT flag selected by an accepted Rust finalization plan.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime.
/// - `write_intent`: accepted finalization storage-write intent whose
///   `executed_pbft_status` is the source of truth.
///
/// Outputs:
/// - Returns the updated runtime snapshot so C++ can hydrate temporary manager
///   mirrors without independently deriving the executed flag.
///
/// Invariants and edge behavior:
/// - This function does not write storage. The caller must run it only after
///   the finalization runtime has accepted any required executed-status storage
///   stage and is executing the `SetExecutedFlag` action.
pub fn pbft_manager_runtime_apply_finalization_executed_status(
    runtime: &mut BridgePbftManagerRuntime,
    write_intent: &FfiPbftFinalizationStorageWritePlan,
) -> FfiPbftManagerRuntimeSnapshot {
    runtime
        .state
        .apply_committed_finalization_executed_status(write_intent.executed_pbft_status)
        .into()
}

/// Resolves a finalized DAG block period through PBFT-manager runtime storage.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime with its storage handle.
/// - `hash`: canonical DAG block hash.
///
/// Outputs:
/// - Returns a stable lookup DTO with `found = false` when the finalized DAG
///   index has no row for `hash`.
///
/// Invariants and edge behavior:
/// - C++ may still use the result to preserve existing optional-return APIs,
///   but the durable lookup is owned by `rustaxa-consensus` over
///   `rustaxa-storage`.
pub fn pbft_manager_runtime_dag_block_period(
    runtime: &BridgePbftManagerRuntime,
    hash: &[u8; 32],
) -> anyhow::Result<FfiBlockPeriodLookup> {
    let lookup =
        dag_block_period_from_storage(runtime.storage.as_ref(), ethereum_types::H256::from(*hash))?;
    Ok(FfiBlockPeriodLookup {
        found: lookup.found,
        period: lookup.period,
        position: lookup.position,
    })
}

/// Checks PBFT block existence through PBFT-manager runtime storage.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime with its storage handle.
/// - `hash`: canonical PBFT block hash.
///
/// Outputs:
/// - Returns whether the Rust PBFT block index contains `hash`.
///
/// Invariants and edge behavior:
/// - This is a storage fact lookup only. PBFT block materialization for network
///   and API compatibility remains outside this helper.
pub fn pbft_manager_runtime_pbft_block_in_db(
    runtime: &BridgePbftManagerRuntime,
    hash: &[u8; 32],
) -> anyhow::Result<bool> {
    pbft_block_exists_in_storage(runtime.storage.as_ref(), ethereum_types::H256::from(*hash))
}

/// Loads the latest persisted dynamic lambda through PBFT-manager runtime storage.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime with its Rust storage handle.
/// - `period`: upper-bound period for the closest-at-or-before lambda lookup.
///
/// Outputs:
/// - `PeriodLambda { found: true, value }` when a prior persisted lambda exists.
/// - `PeriodLambda { found: false, value: 0 }` when no row exists.
///
/// Invariants and edge behavior:
/// - This wrapper exists only to keep PBFT manager finalization paths on the
///   runtime-owned storage handle; it does not change dynamic-lambda planning.
pub fn pbft_manager_runtime_load_finalization_last_period_lambda(
    runtime: &BridgePbftManagerRuntime,
    period: u64,
) -> anyhow::Result<FfiPeriodLambda> {
    let lookup =
        load_domain_pbft_finalization_last_period_lambda(runtime.storage.as_ref(), period)?;
    Ok(FfiPeriodLambda {
        found: lookup.found,
        value: lookup.value,
    })
}

/// Inspects duplicate PBFT finalization progress through runtime-owned storage.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime with its Rust storage handle.
/// - `write_set`: expected finalized-period storage intent for the duplicate block.
/// - `final_chain_last_block`: C++ FinalChain durable height, which Rust
///   consensus storage cannot infer.
///
/// Outputs:
/// - A bridge-safe resume classification describing which durable stages are
///   complete and which runtime replay actions remain.
///
/// Invariants and edge behavior:
/// - FinalChain execution remains a typed runtime action owned by C++ for now.
/// - Storage conflicts are reported by the Rust resume inspector and are never
///   repaired by this wrapper.
pub fn pbft_manager_runtime_inspect_finalization_resume(
    runtime: &BridgePbftManagerRuntime,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    final_chain_last_block: u64,
) -> anyhow::Result<FfiPbftFinalizationResumePlan> {
    let write_set: PbftFinalizationStorageWriteIntent = write_set.into();
    inspect_domain_pbft_finalization_resume(
        runtime.storage.as_ref(),
        &write_set,
        final_chain_last_block,
    )
    .map(Into::into)
}

/// Applies PBFT finalization storage stages through runtime-owned storage.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime with its Rust storage handle.
/// - `write_set`: accepted PBFT finalization storage intent from the Rust planner.
/// - `stages`: ordered persistence stages to append to one Rust storage batch.
/// - `sync`: whether the storage commit should use synchronous write options.
///
/// Outputs:
/// - The combined finalized-period apply result from Rust consensus storage.
///
/// Invariants and edge behavior:
/// - Batch creation, stage append, and commit are owned by Rust storage.
/// - Empty stage lists and storage conflicts are rejected by the existing
///   finalization storage helper without C++ batch participation.
pub fn pbft_manager_runtime_apply_finalization_storage_writes(
    runtime: &BridgePbftManagerRuntime,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    stages: Vec<FfiPbftFinalizationStorageWriteStage>,
    sync: bool,
) -> anyhow::Result<FfiPbftFinalizedPeriodApplyResult> {
    let write_set: PbftFinalizationStorageWriteIntent = write_set.into();
    let stages = stages.into_iter().map(Into::into).collect();
    Ok(crate::pbft_finalize::apply_result_from_domain(
        apply_domain_pbft_finalization_storage_writes(
            runtime.storage.as_ref(),
            &write_set,
            stages,
            sync,
        )?,
    ))
}

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

/// Plans whether the C++ PBFT manager shell should wait before the next step.
///
/// Inputs:
/// - `fact`: Rust snapshot deadline, observed round elapsed time, and current
///   PBFT step.
///
/// Outputs:
/// - A Rust-owned wait/no-wait decision. The C++ shell only executes the
///   returned condition-variable wait and must not recompute the comparison.
pub fn plan_pbft_manager_sleep_until_next_step(
    fact: FfiPbftManagerSleepFact,
) -> FfiPbftManagerSleepPlan {
    plan_domain_pbft_manager_sleep_until_next_step(fact.into()).into()
}

/// Plans whether the C++ PBFT manager shell should wait using the Rust runtime deadline.
///
/// Inputs:
/// - `runtime`: Rust-owned PBFT manager runtime containing the current
///   next-step deadline and step.
/// - `round_elapsed_ms`: C++-observed elapsed milliseconds for the current
///   round.
///
/// Outputs:
/// - A Rust-owned wait/no-wait decision. C++ remains the condition-variable
///   executor and no longer copies deadline fields out of the snapshot to make
///   this decision.
pub fn plan_pbft_manager_runtime_sleep_until_next_step(
    runtime: &BridgePbftManagerRuntime,
    round_elapsed_ms: i64,
) -> FfiPbftManagerSleepPlan {
    plan_domain_pbft_manager_runtime_sleep_until_next_step(
        &runtime.state.snapshot(),
        round_elapsed_ms,
    )
    .into()
}

/// Plans the PBFT manager startup wait for FinalChain readiness.
///
/// Inputs:
/// - `fact`: PBFT-chain size, FinalChain finalized height, delegation delay,
///   and polling interval facts supplied by the C++ shell.
///
/// Outputs:
/// - A Rust-owned wait/no-wait command. C++ keeps the startup loop and sleep
///   execution.
pub fn plan_pbft_manager_finalization_wait(
    fact: FfiPbftManagerFinalizationWaitFact,
) -> FfiPbftManagerFinalizationWaitPlan {
    plan_domain_pbft_manager_finalization_wait(fact.into()).into()
}

/// Plans whether a PBFT manager vote-count query should wait for eligible-wallet period readiness.
pub fn plan_pbft_manager_eligible_wallet_period_wait(
    fact: FfiPbftManagerEligibleWalletPeriodWaitFact,
) -> FfiPbftManagerEligibleWalletPeriodWaitPlan {
    plan_domain_pbft_manager_eligible_wallet_period_wait(fact.into()).into()
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

/// Plans one deterministic PBFT manager state action as ordered effects.
///
/// Inputs:
/// - `fact`: compact manager state, timing, and vote-status facts sourced by C++.
///
/// Outputs:
/// - An ordered effect list plus follow-up state flags for the C++ executor.
///
/// Invariants and edge behavior:
/// - Rust owns the branch ordering and no-op decisions.
/// - C++ executes only the returned effects in order and keeps live
///   materialization, storage mutation, and network gossip outside the bridge.
pub fn plan_pbft_manager_state_action_effects(
    fact: FfiPbftManagerStateActionFact,
) -> FfiPbftManagerStateActionEffectPlan {
    plan_domain_pbft_manager_state_action_effects(fact.into()).into()
}

/// Creates a Rust-owned state-action effect session from compact C++ facts.
pub fn create_pbft_manager_state_action_effect_session(
    fact: FfiPbftManagerStateActionFact,
) -> Box<BridgePbftManagerStateActionEffectSession> {
    Box::new(BridgePbftManagerStateActionEffectSession {
        state: create_domain_pbft_manager_state_action_effect_session(fact.into()),
    })
}

/// Returns the next effect requested by a Rust-owned state-action session.
pub fn pbft_manager_state_action_effect_session_next(
    session: &mut BridgePbftManagerStateActionEffectSession,
) -> FfiPbftManagerStateActionSessionStep {
    next_domain_pbft_manager_state_action_effect_session(&mut session.state).into()
}

/// Reports one C++-executed state-action effect to Rust and returns the next step.
pub fn pbft_manager_state_action_effect_session_report(
    session: &mut BridgePbftManagerStateActionEffectSession,
    report: FfiPbftManagerStateActionEffectReport,
) -> FfiPbftManagerStateActionSessionStep {
    report_domain_pbft_manager_state_action_effect_session(&mut session.state, report.into()).into()
}

/// Aborts this state-action effect session by exhausting it with a contract error.
pub fn abort_pbft_manager_state_action_effect_session(
    session: &mut BridgePbftManagerStateActionEffectSession,
) {
    let mut report_step = next_domain_pbft_manager_state_action_effect_session(&mut session.state);
    while report_step.has_effect {
        report_step = report_domain_pbft_manager_state_action_effect_session(
            &mut session.state,
            PbftManagerStateActionEffectReport {
                cursor: report_step.cursor,
                intent: report_step.effect.intent,
                result: PbftManagerStateActionEffectResultCode::ExecutorError,
                error_code: "PBFT_MANAGER_STATE_ACTION_EFFECT_SESSION_ABORTED".to_string(),
            },
        );
    }
}

/// Creates a Rust-owned PBFT proposal-construction session from compact C++ facts.
pub fn create_pbft_manager_proposal_session(
    fact: FfiPbftManagerProposalInitialFact,
) -> Box<BridgePbftManagerProposalSession> {
    Box::new(BridgePbftManagerProposalSession {
        state: create_domain_pbft_manager_proposal_session(fact.into()),
    })
}

/// Returns the next Rust-owned proposal-construction action or build command.
pub fn pbft_manager_proposal_session_next(
    session: &mut BridgePbftManagerProposalSession,
) -> FfiPbftManagerProposalSessionStep {
    next_domain_pbft_manager_proposal_session(&mut session.state).into()
}

/// Reports one C++-loaded DAG order to the Rust-owned proposal session.
pub fn pbft_manager_proposal_session_report_dag_order(
    session: &mut BridgePbftManagerProposalSession,
    report: FfiPbftManagerProposalDagOrderReport,
) -> FfiPbftManagerProposalSessionStep {
    report_domain_pbft_manager_proposal_dag_order(&mut session.state, report.into()).into()
}

/// Aborts a proposal-construction session with a stable contract-error status.
pub fn abort_pbft_manager_proposal_session(
    session: &mut BridgePbftManagerProposalSession,
) -> FfiPbftManagerProposalSessionStep {
    abort_domain_pbft_manager_proposal_session(&mut session.state).into()
}

/// Plans one Rust-owned PBFT vote broadcast action from timing/counter facts.
pub fn plan_pbft_manager_broadcast(
    fact: FfiPbftManagerBroadcastFact,
) -> FfiPbftManagerBroadcastPlan {
    plan_domain_pbft_manager_broadcast(fact.into()).into()
}

/// Validates a C++ broadcast executor report before counters are applied.
pub fn report_pbft_manager_broadcast(
    plan: FfiPbftManagerBroadcastPlan,
    report: FfiPbftManagerBroadcastReport,
) -> FfiPbftManagerBroadcastReportResult {
    report_domain_pbft_manager_broadcast(plan.into(), report.into()).into()
}

/// Plans the next Rust-owned PBFT block validation check from live C++ facts.
pub fn plan_pbft_manager_block_validation(
    fact: FfiPbftManagerBlockValidationFact,
) -> FfiPbftManagerBlockValidationPlan {
    plan_domain_pbft_manager_block_validation(fact.into()).into()
}

/// Creates a Rust-owned PBFT block-validation session from initial live facts.
///
/// Inputs:
/// - `fact`: initial block identity and check status bundle supplied by C++.
///
/// Outputs:
/// - A session handle that owns the evolving validation facts and pending
///   requested check.
///
/// Invariants and edge behavior:
/// - C++ must call `pbft_manager_block_validation_session_next` before
///   reporting a check.
/// - The session is a compatibility executor boundary; it does not perform
///   live checks or materialize C++ sidecars.
pub fn create_pbft_manager_block_validation_session(
    fact: FfiPbftManagerBlockValidationFact,
) -> Box<BridgePbftManagerBlockValidationSession> {
    Box::new(BridgePbftManagerBlockValidationSession {
        state: create_domain_pbft_manager_block_validation_session(fact.into()),
    })
}

/// Returns the next validation plan for a Rust-owned PBFT block-validation session.
pub fn pbft_manager_block_validation_session_next(
    session: &mut BridgePbftManagerBlockValidationSession,
) -> FfiPbftManagerBlockValidationPlan {
    next_domain_pbft_manager_block_validation_session(&mut session.state).into()
}

/// Reports one requested live check to a Rust-owned PBFT block-validation session.
///
/// Inputs:
/// - `status`: stable fact status for the most recently requested check.
/// - `dag_weight_check_required`: meaningful only for successful DAG-order
///   reports, where the executor discovers whether a DAG-weight check must
///   follow.
///
/// Outputs:
/// - The next validation plan after Rust applies the reported status.
pub fn pbft_manager_block_validation_session_report(
    session: &mut BridgePbftManagerBlockValidationSession,
    status: u8,
    dag_weight_check_required: bool,
) -> FfiPbftManagerBlockValidationPlan {
    report_domain_pbft_manager_block_validation_session_check(
        &mut session.state,
        PbftManagerBlockValidationFactStatus::from_u8(status),
        dag_weight_check_required,
    )
    .into()
}

/// Plans one Rust-owned proposed PBFT block admission attempt from live C++ facts.
pub fn plan_pbft_manager_candidate_admission(
    fact: FfiPbftManagerCandidateAdmissionFact,
) -> FfiPbftManagerCandidateAdmissionPlan {
    plan_domain_pbft_manager_candidate_admission(fact.into()).into()
}

/// Plans grouped PBFT proposal candidate validation, mark-valid commands, and leader selection.
pub fn plan_pbft_manager_leader_candidates(
    candidates: Vec<FfiPbftManagerLeaderCandidateInputFact>,
) -> FfiPbftManagerLeaderCandidatePlan {
    let candidates = candidates
        .into_iter()
        .map(PbftManagerLeaderCandidateInputFact::from)
        .collect();
    plan_domain_pbft_manager_leader_candidates(candidates).into()
}

/// Plans one deterministic PBFT manager transition from compact C++ facts.
pub fn plan_pbft_manager_transition(
    fact: FfiPbftManagerTransitionFact,
) -> FfiPbftManagerTransitionPlan {
    plan_domain_pbft_manager_transition(fact.into()).into()
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

impl BridgePbftManagerStateActionEffectSession {
    /// Returns the next requested state-action effect.
    pub fn pbft_manager_state_action_effect_session_next(
        &mut self,
    ) -> FfiPbftManagerStateActionSessionStep {
        pbft_manager_state_action_effect_session_next(self)
    }

    /// Reports one executed state-action effect.
    pub fn pbft_manager_state_action_effect_session_report(
        &mut self,
        report: FfiPbftManagerStateActionEffectReport,
    ) -> FfiPbftManagerStateActionSessionStep {
        pbft_manager_state_action_effect_session_report(self, report)
    }

    /// Aborts this state-action effect session.
    pub fn abort_pbft_manager_state_action_effect_session(&mut self) {
        abort_pbft_manager_state_action_effect_session(self)
    }
}

impl BridgePbftManagerProposalSession {
    /// Returns the next proposal-construction action or build command.
    pub fn pbft_manager_proposal_session_next(&mut self) -> FfiPbftManagerProposalSessionStep {
        pbft_manager_proposal_session_next(self)
    }

    /// Reports one DAG-order fact response and returns the next proposal step.
    pub fn pbft_manager_proposal_session_report_dag_order(
        &mut self,
        report: FfiPbftManagerProposalDagOrderReport,
    ) -> FfiPbftManagerProposalSessionStep {
        pbft_manager_proposal_session_report_dag_order(self, report)
    }

    /// Aborts this proposal session.
    pub fn abort_pbft_manager_proposal_session(&mut self) -> FfiPbftManagerProposalSessionStep {
        abort_pbft_manager_proposal_session(self)
    }
}

impl BridgePbftManagerBlockValidationSession {
    /// Returns the next validation plan for this PBFT block-validation session.
    pub fn pbft_manager_block_validation_session_next(
        &mut self,
    ) -> FfiPbftManagerBlockValidationPlan {
        pbft_manager_block_validation_session_next(self)
    }

    /// Reports one requested live check and returns the next validation plan.
    pub fn pbft_manager_block_validation_session_report(
        &mut self,
        status: u8,
        dag_weight_check_required: bool,
    ) -> FfiPbftManagerBlockValidationPlan {
        pbft_manager_block_validation_session_report(self, status, dag_weight_check_required)
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
            polling_interval_ms: value.polling_interval_ms,
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
            has_new_round: value.has_new_round,
            new_round: value.new_round,
            error_code: value.error_code,
        }
    }
}

impl From<FfiPbftManagerSleepFact> for PbftManagerSleepFact {
    fn from(value: FfiPbftManagerSleepFact) -> Self {
        Self {
            next_step_time_ms: value.next_step_time_ms,
            round_elapsed_ms: value.round_elapsed_ms,
            step: value.step,
        }
    }
}

impl From<FfiPbftManagerFinalizationWaitFact> for PbftManagerFinalizationWaitFact {
    fn from(value: FfiPbftManagerFinalizationWaitFact) -> Self {
        Self {
            pbft_chain_size: value.pbft_chain_size,
            final_chain_last_block: value.final_chain_last_block,
            delegation_delay: value.delegation_delay,
            polling_interval_ms: value.polling_interval_ms,
        }
    }
}

impl From<FfiPbftManagerEligibleWalletPeriodWaitFact> for PbftManagerEligibleWalletPeriodWaitFact {
    fn from(value: FfiPbftManagerEligibleWalletPeriodWaitFact) -> Self {
        Self {
            eligible_wallet_period: value.eligible_wallet_period,
            pbft_chain_size: value.pbft_chain_size,
            polling_interval_ms: value.polling_interval_ms,
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

impl From<FfiPbftManagerBlockValidationFact> for PbftManagerBlockValidationFact {
    fn from(value: FfiPbftManagerBlockValidationFact) -> Self {
        Self {
            block_hash: value.block_hash.into(),
            period: value.period,
            pivot_hash: value.pivot_hash.into(),
            pivot_is_null: value.pivot_is_null,
            dag_order_cached: value.dag_order_cached,
            dag_order_required: value.dag_order_required,
            pillar_block_required: value.pillar_block_required,
            dag_weight_check_required: value.dag_weight_check_required,
            pbft_chain_status: PbftManagerBlockValidationFactStatus::from_u8(
                value.pbft_chain_status,
            ),
            final_chain_hash_status: PbftManagerBlockValidationFactStatus::from_u8(
                value.final_chain_hash_status,
            ),
            reward_votes_status: PbftManagerBlockValidationFactStatus::from_u8(
                value.reward_votes_status,
            ),
            extra_data_status: PbftManagerBlockValidationFactStatus::from_u8(
                value.extra_data_status,
            ),
            pillar_block_status: PbftManagerBlockValidationFactStatus::from_u8(
                value.pillar_block_status,
            ),
            dag_order_status: PbftManagerBlockValidationFactStatus::from_u8(value.dag_order_status),
            dag_weight_status: PbftManagerBlockValidationFactStatus::from_u8(
                value.dag_weight_status,
            ),
        }
    }
}

impl From<FfiPbftManagerProposalWalletFact> for PbftManagerProposalWalletFact {
    fn from(value: FfiPbftManagerProposalWalletFact) -> Self {
        Self {
            wallet_index: value.wallet_index,
            dpos_eligible: value.dpos_eligible,
            sortition_valid: value.sortition_valid,
        }
    }
}

impl From<FfiPbftManagerProposalDagBlockFact> for PbftManagerProposalDagBlockFact {
    fn from(value: FfiPbftManagerProposalDagBlockFact) -> Self {
        Self {
            hash: value.hash.into(),
            gas_estimation: value.gas_estimation,
        }
    }
}

impl From<FfiPbftManagerProposalInitialFact> for PbftManagerProposalInitialFact {
    fn from(value: FfiPbftManagerProposalInitialFact) -> Self {
        Self {
            period: value.period,
            round: value.round,
            previous_pbft_block_hash: value.previous_pbft_block_hash.into(),
            last_period_dag_anchor_hash: value.last_period_dag_anchor_hash.into(),
            dag_genesis_hash: value.dag_genesis_hash.into(),
            dag_blocks_size: value.dag_blocks_size,
            ghost_path_move_back: value.ghost_path_move_back,
            pbft_gas_limit: value.pbft_gas_limit,
            extra_data_required: value.extra_data_required,
            extra_data_available: value.extra_data_available,
            final_chain_hash_valid: value.final_chain_hash_valid,
            final_chain_hash: value.final_chain_hash.into(),
            wallets: value.wallets.into_iter().map(Into::into).collect(),
            ghost_path: value
                .ghost_path
                .into_iter()
                .map(|hash| ethereum_types::H256::from(hash.hash))
                .collect(),
            has_non_finalized_fallback: value.has_non_finalized_fallback,
            non_finalized_fallback_hash: value.non_finalized_fallback_hash.into(),
        }
    }
}

impl From<FfiPbftManagerProposalDagOrderReport> for PbftManagerProposalDagOrderReport {
    fn from(value: FfiPbftManagerProposalDagOrderReport) -> Self {
        Self {
            anchor_hash: value.anchor_hash.into(),
            dag_blocks: value.dag_blocks.into_iter().map(Into::into).collect(),
            order_available: value.order_available,
        }
    }
}

impl From<FfiPbftManagerBroadcastFact> for PbftManagerBroadcastFact {
    fn from(value: FfiPbftManagerBroadcastFact) -> Self {
        Self {
            round_elapsed_ms: value.round_elapsed_ms,
            period_elapsed_ms: value.period_elapsed_ms,
            current_round_lambda_ms: value.current_round_lambda_ms,
            broadcast_lambda_threshold: value.broadcast_lambda_threshold,
            rebroadcast_lambda_threshold: value.rebroadcast_lambda_threshold,
            broadcast_votes_counter: value.broadcast_votes_counter,
            rebroadcast_votes_counter: value.rebroadcast_votes_counter,
            broadcast_reward_votes_counter: value.broadcast_reward_votes_counter,
            rebroadcast_reward_votes_counter: value.rebroadcast_reward_votes_counter,
        }
    }
}

impl From<FfiPbftManagerBroadcastPlan> for PbftManagerBroadcastPlan {
    fn from(value: FfiPbftManagerBroadcastPlan) -> Self {
        Self {
            status: broadcast_status_from_u8(value.status),
            action: PbftManagerBroadcastAction::from_u8(value.action),
            rebroadcast: value.rebroadcast,
            next_broadcast_votes_counter: value.next_broadcast_votes_counter,
            next_rebroadcast_votes_counter: value.next_rebroadcast_votes_counter,
            next_broadcast_reward_votes_counter: value.next_broadcast_reward_votes_counter,
            next_rebroadcast_reward_votes_counter: value.next_rebroadcast_reward_votes_counter,
            error_code: value.error_code,
        }
    }
}

impl From<FfiPbftManagerBroadcastReport> for PbftManagerBroadcastReport {
    fn from(value: FfiPbftManagerBroadcastReport) -> Self {
        Self {
            action: PbftManagerBroadcastAction::from_u8(value.action),
            rebroadcast: value.rebroadcast,
            success: value.success,
            error_code: value.error_code,
        }
    }
}

impl From<FfiPbftManagerCandidateAdmissionFact> for PbftManagerCandidateAdmissionFact {
    fn from(value: FfiPbftManagerCandidateAdmissionFact) -> Self {
        Self {
            period: value.period,
            block_hash: value.block_hash.into(),
            lookup_performed: value.lookup_performed,
            proposed_block_found: value.proposed_block_found,
            proposed_block_already_valid: value.proposed_block_already_valid,
            validation_status: PbftManagerCandidateAdmissionValidationStatus::from_u8(
                value.validation_status,
            ),
        }
    }
}

impl From<FfiPbftManagerLeaderCandidateInputFact> for PbftManagerLeaderCandidateInputFact {
    fn from(value: FfiPbftManagerLeaderCandidateInputFact) -> Self {
        Self {
            vote_hash: value.vote_hash.into(),
            block_hash: value.block_hash.into(),
            period: value.period,
            credential: value.credential,
            voter_public_key: value.voter_public_key,
            weight_found: value.weight_found,
            weight: value.weight,
            block_in_chain: value.block_in_chain,
            proposed_block_found: value.proposed_block_found,
            block_validation_status: PbftManagerLeaderBlockValidationStatus::from_u8(
                value.block_validation_status,
            ),
            pivot_hash: value.pivot_hash.into(),
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
            has_target_round: value.has_target_round,
            target_round: value.target_round,
            sleep_ms: value.sleep_ms,
            tick_id: value.tick_id,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerRuntimeSnapshot> for FfiPbftManagerRuntimeSnapshot {
    fn from(value: PbftManagerRuntimeSnapshot) -> Self {
        Self {
            status: value.status.as_u8(),
            state: value.state.as_u8(),
            period: value.period,
            round: value.round,
            step: value.step,
            current_round_lambda_ms: value.current_round_lambda_ms,
            next_step_time_ms: value.next_step_time_ms,
            rounds_count_dynamic_lambda: value.rounds_count_dynamic_lambda,
            dynamic_lambda_ms: value.dynamic_lambda_ms,
            executed_pbft_block: value.executed_pbft_block,
            already_next_voted_value: value.already_next_voted_value,
            already_next_voted_null: value.already_next_voted_null,
            broadcast_votes_counter: value.broadcast_votes_counter,
            rebroadcast_votes_counter: value.rebroadcast_votes_counter,
            broadcast_reward_votes_counter: value.broadcast_reward_votes_counter,
            rebroadcast_reward_votes_counter: value.rebroadcast_reward_votes_counter,
            has_cert_voted_block: value.has_cert_voted_block,
            cert_voted_block_period: value.cert_voted_block_period,
            cert_voted_block_round: value.cert_voted_block_round,
            cert_voted_block_hash: value.cert_voted_block_hash.into(),
            persist_normalized_step: value.persist_normalized_step,
            reset_second_finish_start: value.reset_second_finish_start,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerSleepPlan> for FfiPbftManagerSleepPlan {
    fn from(value: PbftManagerSleepPlan) -> Self {
        Self {
            accepted: value.accepted,
            should_sleep: value.should_sleep,
            sleep_ms: value.sleep_ms,
            step: value.step,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerFinalizationWaitPlan> for FfiPbftManagerFinalizationWaitPlan {
    fn from(value: PbftManagerFinalizationWaitPlan) -> Self {
        Self {
            accepted: value.accepted,
            should_wait: value.should_wait,
            sleep_ms: value.sleep_ms,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerEligibleWalletPeriodWaitPlan> for FfiPbftManagerEligibleWalletPeriodWaitPlan {
    fn from(value: PbftManagerEligibleWalletPeriodWaitPlan) -> Self {
        Self {
            should_wait: value.should_wait,
            sleep_ms: value.sleep_ms,
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

impl From<PbftManagerStateActionEffect> for FfiPbftManagerStateActionEffect {
    fn from(value: PbftManagerStateActionEffect) -> Self {
        Self {
            intent: value.intent.as_u8(),
            hash: value.hash,
            request_proposed_block_sidecar: value.request_proposed_block_sidecar,
            proposed_block_sidecar_hash: value.proposed_block_sidecar_hash,
            proposed_block_sidecar_period: value.proposed_block_sidecar_period,
        }
    }
}

impl From<PbftManagerStateActionEffectPlan> for FfiPbftManagerStateActionEffectPlan {
    fn from(value: PbftManagerStateActionEffectPlan) -> Self {
        Self {
            status: value.status.as_u8(),
            effects: value.effects.into_iter().map(Into::into).collect(),
            go_finish_state: value.go_finish_state,
            loop_back_finish_state: value.loop_back_finish_state,
            error_code: value.error_code,
        }
    }
}

impl From<FfiPbftManagerStateActionEffectReport> for PbftManagerStateActionEffectReport {
    fn from(value: FfiPbftManagerStateActionEffectReport) -> Self {
        Self {
            cursor: value.cursor,
            intent: PbftManagerStateActionIntent::from_u8(value.intent),
            result: PbftManagerStateActionEffectResultCode::from_u8(value.result),
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerStateActionSessionStep> for FfiPbftManagerStateActionSessionStep {
    fn from(value: PbftManagerStateActionSessionStep) -> Self {
        Self {
            status: value.status.as_u8(),
            cursor: value.cursor,
            has_effect: value.has_effect,
            effect: value.effect.into(),
            go_finish_state: value.go_finish_state,
            loop_back_finish_state: value.loop_back_finish_state,
            complete: value.complete,
            can_continue: value.can_continue,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerBlockValidationPlan> for FfiPbftManagerBlockValidationPlan {
    fn from(value: PbftManagerBlockValidationPlan) -> Self {
        Self {
            action: value.action.as_u8(),
            status: value.status.as_u8(),
            next_check: value.next_check.as_u8(),
            error_code: value.error_code.to_string(),
        }
    }
}

impl From<PbftManagerProposalSessionStep> for FfiPbftManagerProposalSessionStep {
    fn from(value: PbftManagerProposalSessionStep) -> Self {
        Self {
            action: value.action.as_u8(),
            status: value.status.as_u8(),
            requested_anchor_hash: value.requested_anchor_hash.into(),
            previous_pbft_block_hash: value.previous_pbft_block_hash.into(),
            anchor_hash: value.anchor_hash.into(),
            order_hash: value.order_hash.into(),
            final_chain_hash: value.final_chain_hash.into(),
            eligible_wallet_indices: value.eligible_wallet_indices,
            dag_blocks_included: value.dag_blocks_included,
            selected_null_anchor: value.selected_null_anchor,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerBroadcastPlan> for FfiPbftManagerBroadcastPlan {
    fn from(value: PbftManagerBroadcastPlan) -> Self {
        Self {
            status: value.status.as_u8(),
            action: value.action.as_u8(),
            rebroadcast: value.rebroadcast,
            next_broadcast_votes_counter: value.next_broadcast_votes_counter,
            next_rebroadcast_votes_counter: value.next_rebroadcast_votes_counter,
            next_broadcast_reward_votes_counter: value.next_broadcast_reward_votes_counter,
            next_rebroadcast_reward_votes_counter: value.next_rebroadcast_reward_votes_counter,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerBroadcastReportResult> for FfiPbftManagerBroadcastReportResult {
    fn from(value: PbftManagerBroadcastReportResult) -> Self {
        Self {
            status: value.status.as_u8(),
            apply_counters: value.apply_counters,
            broadcast_votes_counter: value.broadcast_votes_counter,
            rebroadcast_votes_counter: value.rebroadcast_votes_counter,
            broadcast_reward_votes_counter: value.broadcast_reward_votes_counter,
            rebroadcast_reward_votes_counter: value.rebroadcast_reward_votes_counter,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerCandidateAdmissionPlan> for FfiPbftManagerCandidateAdmissionPlan {
    fn from(value: PbftManagerCandidateAdmissionPlan) -> Self {
        Self {
            action: value.action.as_u8(),
            status: value.status.as_u8(),
            mark_valid: value.mark_valid,
            error_code: value.error_code.to_string(),
        }
    }
}

impl From<PbftManagerLeaderValidBlockCommand> for FfiPbftManagerLeaderValidBlockCommand {
    fn from(value: PbftManagerLeaderValidBlockCommand) -> Self {
        Self {
            period: value.period,
            block_hash: value.block_hash.into(),
        }
    }
}

impl From<PbftManagerLeaderCandidatePlan> for FfiPbftManagerLeaderCandidatePlan {
    fn from(value: PbftManagerLeaderCandidatePlan) -> Self {
        Self {
            status: value.status.as_u8(),
            selected: value.selected,
            selected_vote_hash: value.selected_vote_hash.into(),
            selected_block_hash: value.selected_block_hash.into(),
            selected_period: value.selected_period,
            selected_from_null_anchor: value.selected_from_null_anchor,
            valid_blocks: value.valid_blocks.into_iter().map(Into::into).collect(),
            error_code: value.error_code.to_string(),
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

impl From<FfiPbftManagerStartupReplayRangeFact> for PbftManagerStartupReplayRangeFact {
    fn from(value: FfiPbftManagerStartupReplayRangeFact) -> Self {
        Self {
            final_chain_last_block: value.final_chain_last_block,
            pbft_chain_size: value.pbft_chain_size,
            delegation_delay: value.delegation_delay,
            recently_finalized_factor: value.recently_finalized_factor,
        }
    }
}

impl From<PbftManagerStartupReplayRangePlan> for FfiPbftManagerStartupReplayRangePlan {
    fn from(value: PbftManagerStartupReplayRangePlan) -> Self {
        Self {
            accepted: value.accepted,
            has_finalization_range: value.has_finalization_range,
            finalization_from_period: value.finalization_from_period,
            finalization_to_period: value.finalization_to_period,
            recent_from_period: value.recent_from_period,
            recent_to_period: value.recent_to_period,
            error_code: value.error_code,
        }
    }
}

impl From<PbftManagerAdvancePeriodPlan> for FfiPbftManagerAdvancePeriodPlan {
    fn from(value: PbftManagerAdvancePeriodPlan) -> Self {
        Self {
            accepted: value.accepted,
            finalized_chain_size: value.finalized_chain_size,
            new_period: value.new_period,
            actions: value
                .actions
                .into_iter()
                .map(|action| action.as_u8())
                .collect(),
            error_code: value.error_code,
        }
    }
}

fn domain_advance_period_plan_from_ffi(
    value: &FfiPbftManagerAdvancePeriodPlan,
) -> PbftManagerAdvancePeriodPlan {
    PbftManagerAdvancePeriodPlan {
        accepted: value.accepted,
        finalized_chain_size: value.finalized_chain_size,
        new_period: value.new_period,
        transition_plan: PbftManagerTransitionPlan {
            status: PbftManagerTransitionStatus::InvalidFact,
            kind: PbftManagerTransitionKind::Unknown,
            new_state: PbftManagerRuntimeStateCode::ValueProposal,
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
            error_code: String::new(),
        },
        actions: value
            .actions
            .iter()
            .filter_map(|action| {
                rustaxa_consensus::pbft_manager::PbftManagerAdvancePeriodAction::from_u8(*action)
            })
            .collect(),
        error_code: value.error_code.to_string(),
    }
}

impl From<FfiPbftManagerAdvancePeriodActionReport> for PbftManagerAdvancePeriodActionReport {
    fn from(value: FfiPbftManagerAdvancePeriodActionReport) -> Self {
        Self {
            action_index: value.action_index,
            action: value.action,
            succeeded: value.succeeded,
        }
    }
}

impl From<PbftManagerAdvancePeriodActionReportResult>
    for FfiPbftManagerAdvancePeriodActionReportResult
{
    fn from(value: PbftManagerAdvancePeriodActionReportResult) -> Self {
        Self {
            accepted: value.accepted,
            status: value.status.as_u8(),
            error_code: value.error_code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{BridgePbftStorageQueries, BridgeStorage};
    use crate::pillar_chain::create_pillar_chain_storage;
    use crate::storage::{
        create_pbft_storage_queries, create_pbft_vote_storage_queries, create_storage,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const STATE_VALUE_PROPOSAL: u8 = 0;
    const STATE_FILTER: u8 = 1;
    const STATE_CERTIFY: u8 = 2;
    const STATE_FINISH: u8 = 3;
    const ACTION_PROCESS_SYNCED: u8 = 0;
    const ACTION_BROADCAST: u8 = 1;
    const ACTION_TRY_CERT: u8 = 2;
    const ACTION_TRY_ROUND: u8 = 3;
    const ACTION_SLEEP_INELIGIBLE: u8 = 4;
    const ACTION_RESET_CONSENSUS: u8 = 18;
    const ACTION_RUN_CERTIFY: u8 = 9;
    const ACTION_TRANSITION_FINISH: u8 = 10;
    const ACTION_RUN_VALUE_PROPOSAL: u8 = 5;
    const ACTION_RUN_FILTER: u8 = 7;
    const ACTION_RUN_FIRST_FINISH: u8 = 12;
    const RESULT_CONTINUE: u8 = 0;
    const RESULT_PROGRESS_RESTART: u8 = 1;
    const LEADER_STATUS_SELECTED: u8 = 0;
    const LEADER_BLOCK_VALIDATION_ALREADY_VALID: u8 = 0;
    const LEADER_BLOCK_VALIDATION_VALIDATED: u8 = 1;
    const BLOCK_VALIDATION_FACT_NOT_CHECKED: u8 = 0;
    const BLOCK_VALIDATION_FACT_VALID: u8 = 1;
    const BLOCK_VALIDATION_FACT_MISSING: u8 = 3;
    const BLOCK_VALIDATION_FACT_NOT_REQUIRED: u8 = 4;
    const BLOCK_VALIDATION_ACTION_RUN_CHECK: u8 = 0;
    const BLOCK_VALIDATION_ACTION_ACCEPT: u8 = 1;
    const BLOCK_VALIDATION_ACTION_WAIT_FOR_FINALIZATION: u8 = 3;
    const BLOCK_VALIDATION_STATUS_ACCEPTED: u8 = 1;
    const BLOCK_VALIDATION_STATUS_FINAL_CHAIN_MISSING: u8 = 3;
    const BLOCK_VALIDATION_CHECK_PBFT_CHAIN: u8 = 0;
    const BLOCK_VALIDATION_CHECK_FINAL_CHAIN_HASH: u8 = 1;
    const BLOCK_VALIDATION_CHECK_REWARD_VOTES: u8 = 2;
    const CANDIDATE_ADMISSION_VALIDATION_NOT_CHECKED: u8 = 0;
    const CANDIDATE_ADMISSION_VALIDATION_VALID: u8 = 1;
    const CANDIDATE_ADMISSION_ACTION_REQUEST_LOOKUP: u8 = 0;
    const CANDIDATE_ADMISSION_ACTION_REQUEST_VALIDATION: u8 = 1;
    const CANDIDATE_ADMISSION_ACTION_ACCEPT: u8 = 2;
    const CANDIDATE_ADMISSION_ACTION_DEFER_MISSING_BLOCK: u8 = 4;
    const CANDIDATE_ADMISSION_STATUS_LOOKUP_REQUIRED: u8 = 0;
    const CANDIDATE_ADMISSION_STATUS_VALIDATION_REQUIRED: u8 = 1;
    const CANDIDATE_ADMISSION_STATUS_ACCEPTED_NEWLY_VALIDATED: u8 = 3;
    const CANDIDATE_ADMISSION_STATUS_BLOCK_MISSING: u8 = 4;
    const RESULT_STATE_DONE: u8 = 2;
    const RESULT_TRANSITION: u8 = 3;
    const RESULT_SLEEP: u8 = 4;
    const STATE_ACTION_STATUS_READY: u8 = 0;
    const STATE_ACTION_PROPOSE_NEW_BLOCK: u8 = 1;
    const STATE_ACTION_CERT_VOTE_CURRENT_SOFT_VALUE: u8 = 5;
    const STATE_ACTION_SOFT_VOTE_PREVIOUS_VALUE: u8 = 4;
    const STATE_ACTION_NEXT_VOTE_CERT_BLOCK: u8 = 7;
    const STATE_ACTION_NEXT_VOTE_NULL_BLOCK: u8 = 8;
    const STATE_ACTION_NEXT_VOTE_CURRENT_SOFT_VALUE: u8 = 10;
    const STATE_ACTION_SESSION_ACTIVE: u8 = 0;
    const STATE_ACTION_SESSION_COMPLETE: u8 = 1;
    const STATE_ACTION_EFFECT_APPLIED: u8 = 0;
    const PROPOSAL_ACTION_REQUEST_DAG_ORDER: u8 = 0;
    const PROPOSAL_ACTION_BUILD: u8 = 1;
    const PROPOSAL_STATUS_ACTIVE: u8 = 0;
    const PROPOSAL_STATUS_BUILD_READY: u8 = 1;
    const BROADCAST_ACTION_ROUND_VOTES: u8 = 2;
    const BROADCAST_STATUS_READY: u8 = 0;
    const STARTUP_STATUS_READY: u8 = 0;
    const TRANSITION_STATUS_READY: u8 = 0;
    const TRANSITION_STATUS_INVALID_FACT: u8 = 2;
    const TRANSITION_RESET: u8 = 0;
    const TRANSITION_FILTER: u8 = 1;
    const TRANSITION_LOOP_BACK_FINISH: u8 = 5;
    const TRANSITION_STORAGE_STATUS_APPLIED: u8 = 0;
    const TRANSITION_STORAGE_STATUS_REJECTED: u8 = 1;
    const ADVANCE_ACTION_RESET_CONSENSUS: u8 = 0;
    const ADVANCE_ACTION_EXECUTED_BLOCK_RESET: u8 = 1;
    const ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND: u8 = 2;
    const ADVANCE_ACTION_RESET_CURRENT_ROUND_TIMER: u8 = 3;
    const ADVANCE_ACTION_RESET_REWARD_VOTE_COUNTERS: u8 = 4;
    const ADVANCE_ACTION_RESET_PERIOD_TIMER: u8 = 5;
    const ADVANCE_ACTION_UPDATE_WALLET_ELIGIBILITY: u8 = 6;
    const ADVANCE_ACTION_CLEANUP_VOTES: u8 = 7;
    const ADVANCE_ACTION_CLEANUP_PROPOSED_BLOCKS: u8 = 8;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), nanos))
    }

    fn pbft_queries(storage: &BridgeStorage) -> Box<BridgePbftStorageQueries> {
        create_pbft_storage_queries(storage)
    }

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
            polling_interval_ms: 100,
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
            has_new_round: false,
            new_round: 0,
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
    fn bridge_session_emits_reset_effect_for_round_advance_candidate() {
        let mut session = create_pbft_manager_runtime_session(fact(STATE_VALUE_PROPOSAL));
        for expected in [ACTION_PROCESS_SYNCED, ACTION_BROADCAST, ACTION_TRY_CERT] {
            let step = pbft_manager_runtime_session_next(&mut session);
            assert_eq!(step.action, expected);
            let result = if expected == ACTION_TRY_CERT {
                RESULT_CONTINUE
            } else {
                RESULT_STATE_DONE
            };
            let _ = pbft_manager_runtime_session_report(
                &mut session,
                report(step.cursor, expected, result),
            );
        }

        let step = pbft_manager_runtime_session_next(&mut session);
        assert_eq!(step.action, ACTION_TRY_ROUND);
        let mut action_report = report(step.cursor, ACTION_TRY_ROUND, RESULT_CONTINUE);
        action_report.has_new_round = true;
        action_report.new_round = 6;
        let reset = pbft_manager_runtime_session_report(&mut session, action_report);

        assert_eq!(reset.action, ACTION_RESET_CONSENSUS);
        assert!(reset.has_target_round);
        assert_eq!(reset.target_round, 6);

        let complete = pbft_manager_runtime_session_report(
            &mut session,
            report(reset.cursor, ACTION_RESET_CONSENSUS, RESULT_TRANSITION),
        );
        assert!(complete.complete);
        assert!(complete.restart_loop);
    }

    #[test]
    fn bridge_session_returns_ineligible_polling_sleep_ms() {
        let mut tick = fact(STATE_VALUE_PROPOSAL);
        tick.polling_interval_ms = 250;
        let mut session = create_pbft_manager_runtime_session(tick);
        for expected in [ACTION_PROCESS_SYNCED, ACTION_BROADCAST, ACTION_TRY_CERT] {
            let step = pbft_manager_runtime_session_next(&mut session);
            let result = if expected == ACTION_TRY_CERT {
                RESULT_CONTINUE
            } else {
                RESULT_STATE_DONE
            };
            let _ = pbft_manager_runtime_session_report(
                &mut session,
                report(step.cursor, expected, result),
            );
        }

        let step = pbft_manager_runtime_session_next(&mut session);
        assert_eq!(step.action, ACTION_TRY_ROUND);
        let mut action_report = report(step.cursor, ACTION_TRY_ROUND, RESULT_CONTINUE);
        action_report.has_eligible_wallet = false;
        let sleep = pbft_manager_runtime_session_report(&mut session, action_report);

        assert_eq!(sleep.action, ACTION_SLEEP_INELIGIBLE);
        assert_eq!(sleep.sleep_ms, 250);
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

    #[test]
    fn bridge_applies_finalization_executed_status_from_write_intent() {
        let temp_path = unique_temp_dir("rustaxa_bridge_pbft_manager_finalized_executed_status");
        let storage =
            crate::storage::create_storage(temp_path.to_str().expect("utf-8 temp path")).unwrap();
        let mut fact = startup_fact();
        fact.cacti_active_at_chain_size = false;
        let mut runtime = create_pbft_manager_runtime_from_storage(&storage, fact).unwrap();
        let mut write_intent = FfiPbftFinalizationStorageWritePlan {
            persist_pbft_head: false,
            persist_period_data: false,
            reset_reward_votes: false,
            update_sortition_params: false,
            apply_dynamic_lambda_update: false,
            persist_period_lambda: false,
            persist_executed_pbft_status: true,
            process_pillar_block: false,
            pbft_block_hash: [0; 32],
            pbft_head_hash: [0; 32],
            block_period: 10,
            null_anchor: true,
            anchor_hash: [0; 32],
            reward_vote_period: 0,
            reward_vote_round: 0,
            reward_vote_step: 0,
            reward_vote_block_hash: [0; 32],
            period_lambda: 0,
            blocks_per_year: 0,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            executed_pbft_status: true,
            pbft_head_payload: Vec::new(),
            period_data_rlp: Vec::new(),
            dag_block_period_writes: Vec::new(),
            transaction_location_writes: Vec::new(),
        };

        let applied =
            pbft_manager_runtime_apply_finalization_executed_status(&mut runtime, &write_intent);
        assert!(applied.executed_pbft_block);

        write_intent.executed_pbft_status = false;
        let cleared =
            pbft_manager_runtime_apply_finalization_executed_status(&mut runtime, &write_intent);
        assert!(!cleared.executed_pbft_block);
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

    fn proposal_fact() -> FfiPbftManagerProposalInitialFact {
        FfiPbftManagerProposalInitialFact {
            period: 10,
            round: 2,
            previous_pbft_block_hash: [0x11; 32],
            last_period_dag_anchor_hash: [0x01; 32],
            dag_genesis_hash: [0x01; 32],
            dag_blocks_size: 10,
            ghost_path_move_back: 0,
            pbft_gas_limit: 100,
            extra_data_required: false,
            extra_data_available: false,
            final_chain_hash_valid: true,
            final_chain_hash: [0x22; 32],
            wallets: vec![
                FfiPbftManagerProposalWalletFact {
                    wallet_index: 0,
                    dpos_eligible: false,
                    sortition_valid: true,
                },
                FfiPbftManagerProposalWalletFact {
                    wallet_index: 1,
                    dpos_eligible: true,
                    sortition_valid: true,
                },
            ],
            ghost_path: vec![
                FfiPbftFinalizationHash { hash: [0x01; 32] },
                FfiPbftFinalizationHash { hash: [0x02; 32] },
                FfiPbftFinalizationHash { hash: [0x03; 32] },
            ],
            has_non_finalized_fallback: false,
            non_finalized_fallback_hash: [0; 32],
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

    fn startup_fact() -> FfiPbftManagerStartupFact {
        FfiPbftManagerStartupFact {
            current_period: 10,
            cacti_active_at_chain_size: true,
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
        }
    }

    fn empty_finalized_dag_period_data_rlp() -> Vec<u8> {
        let ordered_transaction_hashes = rlp::RlpStream::new_list(0);
        let transaction_indexes = rlp::RlpStream::new_list(0);
        let compact_blocks = rlp::RlpStream::new_list(0);
        let mut bundle = rlp::RlpStream::new_list(3);
        bundle.append_raw(&ordered_transaction_hashes.out(), 1);
        bundle.append_raw(&transaction_indexes.out(), 1);
        bundle.append_raw(&compact_blocks.out(), 1);

        let mut period_data = rlp::RlpStream::new_list(4);
        period_data.append_empty_data();
        period_data.append_empty_data();
        period_data.append_raw(&bundle.out(), 1);
        period_data.begin_list(0);
        period_data.out().to_vec()
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

        let mut certify_fact = state_fact(STATE_CERTIFY);
        certify_fact.has_current_round_soft_value = true;
        let certify_plan = plan_pbft_manager_state_action(certify_fact);
        assert_eq!(certify_plan.status, STATE_ACTION_STATUS_READY);
        assert_eq!(
            certify_plan.primary_intent,
            STATE_ACTION_CERT_VOTE_CURRENT_SOFT_VALUE
        );
        assert_eq!(certify_plan.primary_hash, [0x55; 32]);

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
    fn bridge_plans_state_action_effects_in_order() {
        let mut finish_polling_fact = state_fact(4);
        finish_polling_fact.current_round_lambda_ms = 1_000;
        finish_polling_fact.has_current_round_soft_value = true;
        finish_polling_fact.has_previous_round_next_null = true;

        let plan = plan_pbft_manager_state_action_effects(finish_polling_fact);

        assert_eq!(plan.status, STATE_ACTION_STATUS_READY);
        assert_eq!(plan.effects.len(), 2);
        assert_eq!(
            plan.effects[0].intent,
            STATE_ACTION_NEXT_VOTE_CURRENT_SOFT_VALUE
        );
        assert_eq!(plan.effects[0].hash, [0x55; 32]);
        assert_eq!(plan.effects[1].intent, STATE_ACTION_NEXT_VOTE_NULL_BLOCK);
    }

    #[test]
    fn bridge_state_action_effect_session_reports_each_effect() {
        let mut finish_polling_fact = state_fact(4);
        finish_polling_fact.current_round_lambda_ms = 1_000;
        finish_polling_fact.has_current_round_soft_value = true;
        finish_polling_fact.has_previous_round_next_null = true;
        let mut session = create_pbft_manager_state_action_effect_session(finish_polling_fact);

        let first = pbft_manager_state_action_effect_session_next(&mut session);
        assert_eq!(first.status, STATE_ACTION_SESSION_ACTIVE);
        assert!(first.has_effect);
        assert_eq!(
            first.effect.intent,
            STATE_ACTION_NEXT_VOTE_CURRENT_SOFT_VALUE
        );
        assert!(first.effect.request_proposed_block_sidecar);
        assert_eq!(first.effect.proposed_block_sidecar_hash, [0x55; 32]);
        assert_eq!(first.effect.proposed_block_sidecar_period, 10);

        let second = pbft_manager_state_action_effect_session_report(
            &mut session,
            FfiPbftManagerStateActionEffectReport {
                cursor: first.cursor,
                intent: first.effect.intent,
                result: STATE_ACTION_EFFECT_APPLIED,
                error_code: String::new(),
            },
        );
        assert_eq!(second.status, STATE_ACTION_SESSION_ACTIVE);
        assert_eq!(second.effect.intent, STATE_ACTION_NEXT_VOTE_NULL_BLOCK);
        assert!(!second.effect.request_proposed_block_sidecar);
        assert_eq!(second.effect.proposed_block_sidecar_hash, [0; 32]);
        assert_eq!(second.effect.proposed_block_sidecar_period, 0);

        let done = pbft_manager_state_action_effect_session_report(
            &mut session,
            FfiPbftManagerStateActionEffectReport {
                cursor: second.cursor,
                intent: second.effect.intent,
                result: STATE_ACTION_EFFECT_APPLIED,
                error_code: String::new(),
            },
        );
        assert_eq!(done.status, STATE_ACTION_SESSION_COMPLETE);
        assert!(done.complete);
        assert!(done.can_continue);
    }

    #[test]
    fn bridge_proposal_session_requests_order_and_builds_command() {
        let mut session = create_pbft_manager_proposal_session(proposal_fact());

        let request = pbft_manager_proposal_session_next(&mut session);
        assert_eq!(request.action, PROPOSAL_ACTION_REQUEST_DAG_ORDER);
        assert_eq!(request.status, PROPOSAL_STATUS_ACTIVE);
        assert_eq!(request.requested_anchor_hash, [0x03; 32]);

        let build = pbft_manager_proposal_session_report_dag_order(
            &mut session,
            FfiPbftManagerProposalDagOrderReport {
                anchor_hash: request.requested_anchor_hash,
                dag_blocks: vec![
                    FfiPbftManagerProposalDagBlockFact {
                        hash: [0x02; 32],
                        gas_estimation: 10,
                    },
                    FfiPbftManagerProposalDagBlockFact {
                        hash: [0x03; 32],
                        gas_estimation: 10,
                    },
                ],
                order_available: true,
            },
        );

        assert_eq!(build.action, PROPOSAL_ACTION_BUILD);
        assert_eq!(build.status, PROPOSAL_STATUS_BUILD_READY);
        assert_eq!(build.anchor_hash, [0x03; 32]);
        assert_eq!(build.final_chain_hash, [0x22; 32]);
        assert_eq!(build.eligible_wallet_indices, vec![1]);
        assert_eq!(build.dag_blocks_included, 2);
        assert_ne!(build.order_hash, [0; 32]);
    }

    #[test]
    fn bridge_broadcast_plan_reports_before_counter_apply() {
        let plan = plan_pbft_manager_broadcast(FfiPbftManagerBroadcastFact {
            round_elapsed_ms: 2_100,
            period_elapsed_ms: 0,
            current_round_lambda_ms: 100,
            broadcast_lambda_threshold: 20,
            rebroadcast_lambda_threshold: 60,
            broadcast_votes_counter: 1,
            rebroadcast_votes_counter: 1,
            broadcast_reward_votes_counter: 1,
            rebroadcast_reward_votes_counter: 1,
        });

        assert_eq!(plan.status, BROADCAST_STATUS_READY);
        assert_eq!(plan.action, BROADCAST_ACTION_ROUND_VOTES);
        assert!(!plan.rebroadcast);
        assert_eq!(plan.next_broadcast_votes_counter, 2);

        let result = report_pbft_manager_broadcast(
            plan,
            FfiPbftManagerBroadcastReport {
                action: BROADCAST_ACTION_ROUND_VOTES,
                rebroadcast: false,
                success: true,
                error_code: String::new(),
            },
        );

        assert_eq!(result.status, BROADCAST_STATUS_READY);
        assert!(result.apply_counters);
        assert_eq!(result.broadcast_votes_counter, 2);
        assert_eq!(result.rebroadcast_votes_counter, 1);
    }

    #[test]
    fn bridge_plans_sleep_wait_and_deadline_reached() {
        let wait = plan_pbft_manager_sleep_until_next_step(FfiPbftManagerSleepFact {
            next_step_time_ms: 1_000,
            round_elapsed_ms: 250,
            step: 2,
        });
        assert!(wait.accepted);
        assert!(wait.should_sleep);
        assert_eq!(wait.sleep_ms, 750);
        assert_eq!(wait.step, 2);
        assert!(wait.error_code.is_empty());

        let reached = plan_pbft_manager_sleep_until_next_step(FfiPbftManagerSleepFact {
            next_step_time_ms: 1_000,
            round_elapsed_ms: 1_000,
            step: 3,
        });
        assert!(reached.accepted);
        assert!(!reached.should_sleep);
        assert_eq!(reached.sleep_ms, 0);
        assert_eq!(reached.step, 3);
        assert!(reached.error_code.is_empty());
    }

    #[test]
    fn bridge_plans_finalization_wait_readiness() {
        let wait = plan_pbft_manager_finalization_wait(FfiPbftManagerFinalizationWaitFact {
            pbft_chain_size: 20,
            final_chain_last_block: 14,
            delegation_delay: 5,
            polling_interval_ms: 100,
        });
        assert!(wait.accepted);
        assert!(wait.should_wait);
        assert_eq!(wait.sleep_ms, 100);
        assert!(wait.error_code.is_empty());

        let ready = plan_pbft_manager_finalization_wait(FfiPbftManagerFinalizationWaitFact {
            pbft_chain_size: 19,
            final_chain_last_block: 14,
            delegation_delay: 5,
            polling_interval_ms: 100,
        });
        assert!(ready.accepted);
        assert!(!ready.should_wait);
        assert_eq!(ready.sleep_ms, 0);
        assert!(ready.error_code.is_empty());
    }

    #[test]
    fn bridge_plans_eligible_wallet_period_wait_readiness() {
        let wait = plan_pbft_manager_eligible_wallet_period_wait(
            FfiPbftManagerEligibleWalletPeriodWaitFact {
                eligible_wallet_period: 8,
                pbft_chain_size: 10,
                polling_interval_ms: 10,
            },
        );
        assert!(wait.should_wait);
        assert_eq!(wait.sleep_ms, 10);

        let ready = plan_pbft_manager_eligible_wallet_period_wait(
            FfiPbftManagerEligibleWalletPeriodWaitFact {
                eligible_wallet_period: 10,
                pbft_chain_size: 10,
                polling_interval_ms: 10,
            },
        );
        assert!(!ready.should_wait);
        assert_eq!(ready.sleep_ms, 0);
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
    fn bridge_plans_loopback_lambda_backoff() {
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
    }

    #[test]
    fn bridge_runtime_restores_startup_snapshot_and_persists_normalized_step() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_startup");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 2)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 2)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            storage
                .save_pbft_mgr_status(0, true)
                .expect("executed status should persist");
            storage
                .save_pbft_mgr_status(2, true)
                .expect("next value status should persist");

            let runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");
            let snapshot = pbft_manager_runtime_snapshot(&runtime);

            assert_eq!(snapshot.status, STARTUP_STATUS_READY);
            assert_eq!(snapshot.state, STATE_FINISH);
            assert_eq!(snapshot.period, 10);
            assert_eq!(snapshot.round, 2);
            assert_eq!(snapshot.step, 4);
            assert_eq!(snapshot.current_round_lambda_ms, 500);
            assert_eq!(snapshot.dynamic_lambda_ms, 1_500);
            assert!(snapshot.executed_pbft_block);
            assert!(snapshot.already_next_voted_value);
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(1).unwrap(), 4);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_rejects_missing_cacti_lambda_without_mutation() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_startup_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");

            let error = match create_pbft_manager_runtime_from_storage(&storage, startup_fact()) {
                Ok(_) => panic!("missing cacti lambda should reject startup"),
                Err(error) => error,
            };
            assert!(error
                .to_string()
                .contains("PBFT_MANAGER_STARTUP_MISSING_DYNAMIC_LAMBDA"));
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(1).unwrap(), 1);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_applies_transition_storage_before_cursor_update() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_transition_apply");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let own_hash = [0xBC; 32];
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            storage
                .save_pbft_mgr_status(2, true)
                .expect("soft next status should persist");
            storage
                .save_pbft_mgr_status(3, true)
                .expect("null next status should persist");
            storage
                .save_own_verified_vote(&own_hash, vec![0xC0])
                .expect("own vote should persist");

            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");
            let before = pbft_manager_runtime_snapshot(&runtime);
            let plan = plan_pbft_manager_transition(transition_fact(TRANSITION_RESET));
            let result = pbft_manager_runtime_apply_transition_storage_write(
                &mut runtime,
                plan,
                vec![FfiPbftFinalizationHash { hash: own_hash }],
            )
            .expect("runtime apply should not throw");

            assert_eq!(result.status, TRANSITION_STORAGE_STATUS_APPLIED);
            assert_eq!(result.snapshot.round, 4);
            assert_eq!(result.snapshot.step, 1);
            assert_eq!(result.snapshot.state, STATE_VALUE_PROPOSAL);
            assert!(!result.snapshot.already_next_voted_value);
            assert!(!result.snapshot.already_next_voted_null);
            assert_ne!(before.round, result.snapshot.round);
            let current = pbft_manager_runtime_snapshot(&runtime);
            assert_eq!(current.round, result.snapshot.round);
            assert_eq!(current.step, result.snapshot.step);
            assert_eq!(current.state, result.snapshot.state);
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(0).unwrap(), 4);
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(1).unwrap(), 1);
            assert!(!pbft_queries(&storage).get_pbft_mgr_status(2).unwrap());
            assert!(!pbft_queries(&storage).get_pbft_mgr_status(3).unwrap());
            let vote_queries = create_pbft_vote_storage_queries(&storage);
            assert!(vote_queries.get_own_verified_votes().unwrap().is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_persists_executed_block_reset_before_snapshot_update() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_executed_reset");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            storage
                .save_pbft_mgr_status(PBFT_MGR_STATUS_EXECUTED_BLOCK, true)
                .expect("executed status should persist");

            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");
            assert!(pbft_manager_runtime_snapshot(&runtime).executed_pbft_block);
            assert!(pbft_queries(&storage)
                .get_pbft_mgr_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)
                .expect("status should load"));

            let result = pbft_manager_runtime_apply_executed_block_reset(&mut runtime)
                .expect("executed-block reset should not throw");

            assert_eq!(result.status, TRANSITION_STORAGE_STATUS_APPLIED);
            assert_eq!(result.applied_writes, 1);
            assert!(!result.snapshot.executed_pbft_block);
            assert!(!pbft_manager_runtime_snapshot(&runtime).executed_pbft_block);
            assert!(!pbft_queries(&storage)
                .get_pbft_mgr_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)
                .expect("status should load"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_persists_next_voted_status_through_consensus_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_next_voted_status");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let soft_snapshot = pbft_manager_runtime_apply_next_voted_status(&mut runtime, 2)
                .expect("soft next-voted status should persist");
            let null_snapshot = pbft_manager_runtime_apply_next_voted_status(&mut runtime, 3)
                .expect("null next-voted status should persist");
            let err = match pbft_manager_runtime_apply_next_voted_status(
                &mut runtime,
                PBFT_MGR_STATUS_EXECUTED_BLOCK,
            ) {
                Ok(_) => panic!("generic manager status should reject"),
                Err(err) => err,
            };

            assert!(soft_snapshot.already_next_voted_value);
            assert!(!soft_snapshot.already_next_voted_null);
            assert!(null_snapshot.already_next_voted_value);
            assert!(null_snapshot.already_next_voted_null);
            let snapshot = pbft_manager_runtime_snapshot(&runtime);
            assert!(snapshot.already_next_voted_value);
            assert!(snapshot.already_next_voted_null);
            assert!(pbft_queries(&storage).get_pbft_mgr_status(2).unwrap());
            assert!(pbft_queries(&storage).get_pbft_mgr_status(3).unwrap());
            assert_eq!(
                err.to_string(),
                "PBFT_MANAGER_NEXT_VOTED_STATUS_UNSUPPORTED"
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_persists_cursor_fields_through_consensus_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_cursor_fields");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let round_snapshot = pbft_manager_runtime_apply_cursor_field(&mut runtime, 0, 8)
                .expect("round cursor should persist");
            let step_snapshot = pbft_manager_runtime_apply_cursor_field(&mut runtime, 1, 6)
                .expect("step cursor should persist");
            let err = match pbft_manager_runtime_apply_cursor_field(&mut runtime, 2, 1) {
                Ok(_) => panic!("dynamic lambda should not use cursor field API"),
                Err(err) => err,
            };

            assert_eq!(round_snapshot.round, 8);
            assert_eq!(step_snapshot.round, 8);
            assert_eq!(step_snapshot.step, 6);
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(0).unwrap(), 8);
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(1).unwrap(), 6);
            assert!(err
                .to_string()
                .contains("unsupported PBFT manager cursor field"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_records_committed_dynamic_lambda_snapshot() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_dynamic_lambda_snapshot");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let snapshot = pbft_manager_runtime_apply_dynamic_lambda(&mut runtime, 12, 1_250);

            assert_eq!(snapshot.status, STARTUP_STATUS_READY);
            assert_eq!(snapshot.rounds_count_dynamic_lambda, 12);
            assert_eq!(snapshot.dynamic_lambda_ms, 1_250);
            assert_eq!(
                pbft_manager_runtime_snapshot(&runtime).dynamic_lambda_ms,
                1_250
            );
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(2).unwrap(), 1_500);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_records_broadcast_counters() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_broadcast_counters");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let snapshot = pbft_manager_runtime_apply_broadcast_counters(&mut runtime, 2, 3, 4, 5);

            assert_eq!(snapshot.status, STARTUP_STATUS_READY);
            assert_eq!(snapshot.broadcast_votes_counter, 2);
            assert_eq!(snapshot.rebroadcast_votes_counter, 3);
            assert_eq!(snapshot.broadcast_reward_votes_counter, 4);
            assert_eq!(snapshot.rebroadcast_reward_votes_counter, 5);
            assert_eq!(
                pbft_manager_runtime_snapshot(&runtime).rebroadcast_reward_votes_counter,
                5
            );

            let rejected = pbft_manager_runtime_apply_broadcast_counters(&mut runtime, 0, 1, 1, 1);
            assert_ne!(rejected.status, STARTUP_STATUS_READY);
            assert_eq!(rejected.error_code, "PBFT_MANAGER_BROADCAST_COUNTER_ZERO");
            assert_eq!(
                pbft_manager_runtime_snapshot(&runtime).broadcast_votes_counter,
                2
            );
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(2).unwrap(), 1_500);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_reads_dag_period_and_pbft_existence_from_owned_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_storage_facts");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            let dag_hash = [0xDA; 32];
            let pbft_hash = [0xBE; 32];
            storage
                .save_dag_block_period(&dag_hash, 12, 4)
                .expect("DAG period should persist");
            storage
                .save_pbft_block_period(&pbft_hash, 9)
                .expect("PBFT period index should persist");
            let runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let dag_lookup = pbft_manager_runtime_dag_block_period(&runtime, &dag_hash)
                .expect("DAG period should load");
            let missing_dag = pbft_manager_runtime_dag_block_period(&runtime, &[0xDB; 32])
                .expect("missing DAG period should load");

            assert!(dag_lookup.found);
            assert_eq!(dag_lookup.period, 12);
            assert_eq!(dag_lookup.position, 4);
            assert!(!missing_dag.found);
            assert!(pbft_manager_runtime_pbft_block_in_db(&runtime, &pbft_hash)
                .expect("PBFT existence should load"));
            assert!(
                !pbft_manager_runtime_pbft_block_in_db(&runtime, &[0xBF; 32])
                    .expect("missing PBFT existence should load")
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_loads_finalization_lambda_from_owned_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_finalization_lambda");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            storage
                .save_period_lambda(10, 1_234)
                .expect("period lambda should persist");
            let runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let lookup = pbft_manager_runtime_load_finalization_last_period_lambda(&runtime, 11)
                .expect("runtime lambda lookup should run");
            let missing = pbft_manager_runtime_load_finalization_last_period_lambda(&runtime, 1)
                .expect("runtime missing lambda lookup should run");

            assert!(lookup.found);
            assert_eq!(lookup.value, 1_234);
            assert!(!missing.found);
            assert_eq!(missing.value, 0);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_reads_cert_voted_block_from_owned_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_cert_voted_block");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            storage
                .save_cert_voted_block_in_round(3, vec![0xC0])
                .expect("cert-voted block should persist");

            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");
            let pbft_queries = create_pbft_storage_queries(&storage);
            let runtime_payload = pbft_manager_runtime_cert_voted_block_in_round(&runtime)
                .expect("runtime-owned storage read should succeed");
            assert_eq!(
                runtime_payload,
                pbft_queries
                    .get_cert_voted_block_in_round()
                    .expect("compatibility storage view should load")
            );

            let snapshot = pbft_manager_runtime_save_cert_voted_block_in_round(
                &mut runtime,
                10,
                4,
                [0x44; 32],
                vec![0xC0],
            )
            .expect("runtime-owned storage write should succeed");
            let rewritten_payload = pbft_manager_runtime_cert_voted_block_in_round(&runtime)
                .expect("rewritten cert-voted block should load");
            let err = match pbft_manager_runtime_save_cert_voted_block_in_round(
                &mut runtime,
                10,
                5,
                [0x55; 32],
                Vec::new(),
            ) {
                Ok(_) => panic!("empty cert-voted block payload should reject"),
                Err(err) => err,
            };

            assert!(snapshot.has_cert_voted_block);
            assert_eq!(snapshot.cert_voted_block_period, 10);
            assert_eq!(snapshot.cert_voted_block_round, 4);
            assert_eq!(snapshot.cert_voted_block_hash, [0x44; 32]);
            assert_eq!(
                pbft_manager_runtime_snapshot(&runtime).cert_voted_block_hash,
                [0x44; 32]
            );
            let rewritten_rlp = rlp::Rlp::new(&rewritten_payload);
            assert_eq!(rewritten_rlp.at(0).unwrap().as_val::<u64>().unwrap(), 4);
            assert_eq!(rewritten_rlp.at(1).unwrap().as_raw(), &[0xC0]);
            assert_eq!(
                err.to_string(),
                "PBFT_MANAGER_CERT_VOTED_BLOCK_EMPTY_PAYLOAD"
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_records_cached_anchor_dag_order_metadata() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_cached_anchor_dag_order");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");
            let first_anchor = [0xDA; 32];
            let second_anchor = [0xDB; 32];

            assert!(!pbft_manager_runtime_has_cached_anchor_dag_order(
                &runtime,
                &first_anchor
            ));

            let record_snapshot =
                pbft_manager_runtime_record_cached_anchor_dag_order(&mut runtime, first_anchor);
            assert_eq!(record_snapshot.status, STARTUP_STATUS_READY);
            assert!(pbft_manager_runtime_has_cached_anchor_dag_order(
                &runtime,
                &first_anchor
            ));
            assert!(!pbft_manager_runtime_has_cached_anchor_dag_order(
                &runtime,
                &second_anchor
            ));

            let remove_snapshot =
                pbft_manager_runtime_remove_cached_anchor_dag_order(&mut runtime, first_anchor);
            assert_eq!(remove_snapshot.status, STARTUP_STATUS_READY);
            assert!(!pbft_manager_runtime_has_cached_anchor_dag_order(
                &runtime,
                &first_anchor
            ));

            pbft_manager_runtime_record_cached_anchor_dag_order(&mut runtime, first_anchor);
            pbft_manager_runtime_record_cached_anchor_dag_order(&mut runtime, second_anchor);
            assert!(pbft_manager_runtime_has_cached_anchor_dag_order(
                &runtime,
                &first_anchor
            ));
            assert!(pbft_manager_runtime_has_cached_anchor_dag_order(
                &runtime,
                &second_anchor
            ));
            assert_eq!(
                pbft_manager_runtime_cached_anchor_dag_order_count(&runtime),
                2
            );

            let clear_snapshot = pbft_manager_runtime_clear_cached_anchor_dag_order(&mut runtime);
            assert_eq!(clear_snapshot.status, STARTUP_STATUS_READY);
            assert_eq!(
                pbft_manager_runtime_cached_anchor_dag_order_count(&runtime),
                0
            );
            assert!(!pbft_manager_runtime_has_cached_anchor_dag_order(
                &runtime,
                &first_anchor
            ));
            assert!(!pbft_manager_runtime_has_cached_anchor_dag_order(
                &runtime,
                &second_anchor
            ));
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(2).unwrap(), 1_500);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_reads_own_pillar_vote_from_owned_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_own_pillar_vote");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            create_pillar_chain_storage(&storage)
                .pillar_chain_storage_apply_own_vote(vec![0xC0])
                .expect("own pillar vote should persist");
            let runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let vote_rlp = pbft_manager_runtime_own_pillar_block_vote(&runtime)
                .expect("runtime-owned pillar vote should load");

            assert_eq!(vote_rlp, vec![0xC0]);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_loads_startup_replay_period_from_owned_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_startup_replay_period");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            let period_data = empty_finalized_dag_period_data_rlp();
            storage
                .save_period_data(12, period_data.clone())
                .expect("period data should persist");
            storage
                .save_period_lambda(11, 1_234)
                .expect("period lambda should persist");
            let runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let replay = pbft_manager_runtime_load_startup_replay_period(&runtime, 12, true)
                .expect("runtime startup replay read should succeed");
            let missing = pbft_manager_runtime_load_startup_replay_period(&runtime, 13, true)
                .expect("runtime missing startup replay read should succeed");

            assert!(replay.found);
            assert_eq!(replay.period_data_rlp, period_data);
            assert!(replay.finalized_dag_hashes.is_empty());
            assert!(replay.has_period_lambda);
            assert_eq!(replay.period_lambda, 1_234);
            assert!(!missing.found);
            assert!(missing.period_data_rlp.is_empty());
            assert!(missing.finalized_dag_hashes.is_empty());
            assert!(!missing.has_period_lambda);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_plans_startup_replay_ranges_and_advance_period_commit() {
        let replay =
            plan_pbft_manager_startup_replay_ranges(FfiPbftManagerStartupReplayRangeFact {
                final_chain_last_block: 8,
                pbft_chain_size: 12,
                delegation_delay: 3,
                recently_finalized_factor: 2,
            });

        assert!(replay.accepted);
        assert!(replay.has_finalization_range);
        assert_eq!(replay.finalization_from_period, 9);
        assert_eq!(replay.finalization_to_period, 12);
        assert_eq!(replay.recent_from_period, 6);
        assert_eq!(replay.recent_to_period, 12);

        let rejected =
            plan_pbft_manager_startup_replay_ranges(FfiPbftManagerStartupReplayRangeFact {
                final_chain_last_block: 13,
                pbft_chain_size: 12,
                delegation_delay: 1,
                recently_finalized_factor: 1,
            });
        assert!(!rejected.accepted);
        assert_eq!(
            rejected.error_code,
            "PBFT_MANAGER_STARTUP_REPLAY_FINAL_CHAIN_AHEAD"
        );

        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_advance_period");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let mut transition_fact = transition_fact(TRANSITION_RESET);
            transition_fact.target_round = 1;
            transition_fact.target_round_lambda_ms = 100;
            let transition = plan_pbft_manager_transition(transition_fact);
            assert_eq!(transition.status, TRANSITION_STATUS_READY);

            let advance = plan_pbft_manager_advance_period(12, &transition);
            assert!(advance.accepted);
            assert_eq!(advance.finalized_chain_size, 12);
            assert_eq!(advance.new_period, 13);
            assert_eq!(
                advance.actions,
                vec![
                    ADVANCE_ACTION_RESET_CONSENSUS,
                    ADVANCE_ACTION_EXECUTED_BLOCK_RESET,
                    ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND,
                    ADVANCE_ACTION_RESET_CURRENT_ROUND_TIMER,
                    ADVANCE_ACTION_RESET_REWARD_VOTE_COUNTERS,
                    ADVANCE_ACTION_RESET_PERIOD_TIMER,
                    ADVANCE_ACTION_UPDATE_WALLET_ELIGIBILITY,
                    ADVANCE_ACTION_CLEANUP_VOTES,
                    ADVANCE_ACTION_CLEANUP_PROPOSED_BLOCKS,
                ]
            );

            let report = validate_pbft_manager_advance_period_action_report(
                &advance,
                FfiPbftManagerAdvancePeriodActionReport {
                    action_index: 0,
                    action: ADVANCE_ACTION_RESET_CONSENSUS,
                    succeeded: true,
                },
            );
            assert!(report.accepted);
            assert_eq!(report.status, 0);
            assert!(report.error_code.is_empty());

            let mismatch = validate_pbft_manager_advance_period_action_report(
                &advance,
                FfiPbftManagerAdvancePeriodActionReport {
                    action_index: 1,
                    action: ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND,
                    succeeded: true,
                },
            );
            assert!(!mismatch.accepted);
            assert_eq!(mismatch.status, 4);
            assert_eq!(
                mismatch.error_code,
                "PBFT_MANAGER_ADVANCE_PERIOD_REPORT_ACTION_MISMATCH"
            );

            let snapshot =
                pbft_manager_runtime_apply_period_advance(&mut runtime, advance.new_period);
            assert_eq!(snapshot.status, STARTUP_STATUS_READY);
            assert_eq!(snapshot.period, 13);

            let rejected_snapshot =
                pbft_manager_runtime_apply_period_advance(&mut runtime, advance.new_period);
            assert_ne!(rejected_snapshot.status, STARTUP_STATUS_READY);
            assert_eq!(rejected_snapshot.period, 13);
            assert_eq!(
                rejected_snapshot.error_code,
                "PBFT_MANAGER_ADVANCE_PERIOD_NON_INCREASING_PERIOD"
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_rejected_transition_preserves_snapshot() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_transition_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_field(2, 1_500)
                .expect("lambda seed should persist");

            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");
            let before = pbft_manager_runtime_snapshot(&runtime);
            let mut plan = plan_pbft_manager_transition(transition_fact(TRANSITION_RESET));
            plan.status = TRANSITION_STATUS_INVALID_FACT;
            let result =
                pbft_manager_runtime_apply_transition_storage_write(&mut runtime, plan, Vec::new())
                    .expect("runtime apply should return deterministic rejection");

            assert_eq!(result.status, TRANSITION_STORAGE_STATUS_REJECTED);
            assert_eq!(result.snapshot.round, before.round);
            assert_eq!(result.snapshot.step, before.step);
            assert_eq!(result.snapshot.state, before.state);
            let current = pbft_manager_runtime_snapshot(&runtime);
            assert_eq!(current.round, before.round);
            assert_eq!(current.step, before.step);
            assert_eq!(current.state, before.state);
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(0).unwrap(), 1);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_plans_pbft_block_validation_checks_and_acceptance() {
        let plan = plan_pbft_manager_block_validation(block_validation_fact());
        assert_eq!(plan.action, BLOCK_VALIDATION_ACTION_RUN_CHECK);
        assert_eq!(plan.next_check, BLOCK_VALIDATION_CHECK_PBFT_CHAIN);

        let mut final_chain_fact = block_validation_fact();
        final_chain_fact.pbft_chain_status = BLOCK_VALIDATION_FACT_VALID;
        let plan = plan_pbft_manager_block_validation(final_chain_fact);
        assert_eq!(plan.action, BLOCK_VALIDATION_ACTION_RUN_CHECK);
        assert_eq!(plan.next_check, BLOCK_VALIDATION_CHECK_FINAL_CHAIN_HASH);

        let mut accept_fact = block_validation_fact();
        accept_fact.pbft_chain_status = BLOCK_VALIDATION_FACT_VALID;
        accept_fact.final_chain_hash_status = BLOCK_VALIDATION_FACT_VALID;
        accept_fact.reward_votes_status = BLOCK_VALIDATION_FACT_VALID;
        accept_fact.extra_data_status = BLOCK_VALIDATION_FACT_VALID;
        accept_fact.pivot_is_null = true;
        let plan = plan_pbft_manager_block_validation(accept_fact);
        assert_eq!(plan.action, BLOCK_VALIDATION_ACTION_ACCEPT);
        assert_eq!(plan.status, BLOCK_VALIDATION_STATUS_ACCEPTED);
    }

    #[test]
    fn bridge_pbft_block_validation_reports_final_chain_wait() {
        let mut fact = block_validation_fact();
        fact.pbft_chain_status = BLOCK_VALIDATION_FACT_VALID;
        fact.final_chain_hash_status = BLOCK_VALIDATION_FACT_MISSING;

        let plan = plan_pbft_manager_block_validation(fact);

        assert_eq!(plan.action, BLOCK_VALIDATION_ACTION_WAIT_FOR_FINALIZATION);
        assert_eq!(plan.status, BLOCK_VALIDATION_STATUS_FINAL_CHAIN_MISSING);
        assert_eq!(plan.next_check, BLOCK_VALIDATION_CHECK_FINAL_CHAIN_HASH);
    }

    #[test]
    fn bridge_pbft_block_validation_session_reports_checks_and_retry() {
        let mut session = create_pbft_manager_block_validation_session(block_validation_fact());
        let plan = session.pbft_manager_block_validation_session_next();
        assert_eq!(plan.action, BLOCK_VALIDATION_ACTION_RUN_CHECK);
        assert_eq!(plan.next_check, BLOCK_VALIDATION_CHECK_PBFT_CHAIN);

        let plan = session
            .pbft_manager_block_validation_session_report(BLOCK_VALIDATION_FACT_VALID, false);
        assert_eq!(plan.action, BLOCK_VALIDATION_ACTION_RUN_CHECK);
        assert_eq!(plan.next_check, BLOCK_VALIDATION_CHECK_FINAL_CHAIN_HASH);

        let plan = session
            .pbft_manager_block_validation_session_report(BLOCK_VALIDATION_FACT_MISSING, false);
        assert_eq!(plan.action, BLOCK_VALIDATION_ACTION_WAIT_FOR_FINALIZATION);
        assert_eq!(plan.status, BLOCK_VALIDATION_STATUS_FINAL_CHAIN_MISSING);

        let plan = session
            .pbft_manager_block_validation_session_report(BLOCK_VALIDATION_FACT_NOT_CHECKED, false);
        assert_eq!(plan.action, BLOCK_VALIDATION_ACTION_RUN_CHECK);
        assert_eq!(plan.next_check, BLOCK_VALIDATION_CHECK_FINAL_CHAIN_HASH);

        let plan = session
            .pbft_manager_block_validation_session_report(BLOCK_VALIDATION_FACT_VALID, false);
        assert_eq!(plan.action, BLOCK_VALIDATION_ACTION_RUN_CHECK);
        assert_eq!(plan.next_check, BLOCK_VALIDATION_CHECK_REWARD_VOTES);
    }

    #[test]
    fn bridge_plans_pbft_candidate_admission() {
        let plan = plan_pbft_manager_candidate_admission(candidate_admission_fact());
        assert_eq!(plan.action, CANDIDATE_ADMISSION_ACTION_REQUEST_LOOKUP);
        assert_eq!(plan.status, CANDIDATE_ADMISSION_STATUS_LOOKUP_REQUIRED);
        assert!(!plan.mark_valid);

        let plan = plan_pbft_manager_candidate_admission(FfiPbftManagerCandidateAdmissionFact {
            lookup_performed: true,
            proposed_block_found: true,
            ..candidate_admission_fact()
        });
        assert_eq!(plan.action, CANDIDATE_ADMISSION_ACTION_REQUEST_VALIDATION);
        assert_eq!(plan.status, CANDIDATE_ADMISSION_STATUS_VALIDATION_REQUIRED);

        let plan = plan_pbft_manager_candidate_admission(FfiPbftManagerCandidateAdmissionFact {
            lookup_performed: true,
            proposed_block_found: true,
            validation_status: CANDIDATE_ADMISSION_VALIDATION_VALID,
            ..candidate_admission_fact()
        });
        assert_eq!(plan.action, CANDIDATE_ADMISSION_ACTION_ACCEPT);
        assert_eq!(
            plan.status,
            CANDIDATE_ADMISSION_STATUS_ACCEPTED_NEWLY_VALIDATED
        );
        assert!(plan.mark_valid);

        let plan = plan_pbft_manager_candidate_admission(FfiPbftManagerCandidateAdmissionFact {
            lookup_performed: true,
            proposed_block_found: false,
            ..candidate_admission_fact()
        });
        assert_eq!(plan.action, CANDIDATE_ADMISSION_ACTION_DEFER_MISSING_BLOCK);
        assert_eq!(plan.status, CANDIDATE_ADMISSION_STATUS_BLOCK_MISSING);
        assert!(!plan.mark_valid);
    }

    #[test]
    fn bridge_plans_pbft_leader_candidates_and_mark_valid_commands() {
        let missing_weight = leader_candidate_input(1, 1, LEADER_BLOCK_VALIDATION_VALIDATED);
        let in_chain = FfiPbftManagerLeaderCandidateInputFact {
            block_in_chain: true,
            ..leader_candidate_input(2, 2, LEADER_BLOCK_VALIDATION_VALIDATED)
        };
        let already_valid = FfiPbftManagerLeaderCandidateInputFact {
            block_validation_status: LEADER_BLOCK_VALIDATION_ALREADY_VALID,
            pivot_hash: [8; 32],
            ..leader_candidate_input(3, 3, LEADER_BLOCK_VALIDATION_VALIDATED)
        };
        let validated = FfiPbftManagerLeaderCandidateInputFact {
            pivot_hash: [9; 32],
            ..leader_candidate_input(4, 4, LEADER_BLOCK_VALIDATION_VALIDATED)
        };
        let mut missing_weight = missing_weight;
        missing_weight.weight_found = false;
        let expected_vote_hash = validated.vote_hash;
        let expected_block_hash = validated.block_hash;

        let plan = plan_pbft_manager_leader_candidates(vec![
            missing_weight,
            in_chain,
            already_valid,
            validated,
        ]);

        assert_eq!(plan.status, LEADER_STATUS_SELECTED);
        assert!(plan.selected);
        assert_eq!(plan.selected_vote_hash, expected_vote_hash);
        assert_eq!(plan.selected_block_hash, expected_block_hash);
        assert_eq!(plan.valid_blocks.len(), 1);
        assert_eq!(plan.valid_blocks[0].period, 11);
        assert_eq!(plan.valid_blocks[0].block_hash, expected_block_hash);
    }

    #[test]
    fn bridge_pbft_leader_candidates_reject_unknown_validation_status() {
        let plan = plan_pbft_manager_leader_candidates(vec![leader_candidate_input(1, 1, 99)]);

        assert_eq!(plan.status, 3);
        assert_eq!(
            plan.error_code,
            "PBFT_MANAGER_LEADER_UNKNOWN_BLOCK_VALIDATION_STATUS"
        );
    }

    fn leader_candidate_input(
        id: u8,
        block: u8,
        block_validation_status: u8,
    ) -> FfiPbftManagerLeaderCandidateInputFact {
        FfiPbftManagerLeaderCandidateInputFact {
            vote_hash: [id; 32],
            block_hash: [block; 32],
            period: 11,
            credential: [id; 64],
            voter_public_key: [id.wrapping_add(17); 64],
            weight_found: true,
            weight: 1,
            block_in_chain: false,
            proposed_block_found: true,
            block_validation_status,
            pivot_hash: [block.wrapping_add(20); 32],
        }
    }

    fn candidate_admission_fact() -> FfiPbftManagerCandidateAdmissionFact {
        FfiPbftManagerCandidateAdmissionFact {
            period: 11,
            block_hash: [1; 32],
            lookup_performed: false,
            proposed_block_found: false,
            proposed_block_already_valid: false,
            validation_status: CANDIDATE_ADMISSION_VALIDATION_NOT_CHECKED,
        }
    }

    fn block_validation_fact() -> FfiPbftManagerBlockValidationFact {
        FfiPbftManagerBlockValidationFact {
            block_hash: [1; 32],
            period: 11,
            pivot_hash: [2; 32],
            pivot_is_null: false,
            dag_order_cached: false,
            dag_order_required: true,
            pillar_block_required: false,
            dag_weight_check_required: false,
            pbft_chain_status: BLOCK_VALIDATION_FACT_NOT_CHECKED,
            final_chain_hash_status: BLOCK_VALIDATION_FACT_NOT_CHECKED,
            reward_votes_status: BLOCK_VALIDATION_FACT_NOT_CHECKED,
            extra_data_status: BLOCK_VALIDATION_FACT_NOT_CHECKED,
            pillar_block_status: BLOCK_VALIDATION_FACT_NOT_REQUIRED,
            dag_order_status: BLOCK_VALIDATION_FACT_NOT_CHECKED,
            dag_weight_status: BLOCK_VALIDATION_FACT_NOT_CHECKED,
        }
    }
}
