//! Bridge wrapper for PBFT manager daemon-tick runtime planning.
//!
//! C++ supplies the current manager state and live-shell facts for one daemon
//! tick. Rust owns the ordered action cursor for that tick, while C++ executes
//! existing manager methods and reports each action result before the session
//! advances.

use crate::ffi::rustaxa_ffi::{
    PbftFinalizationHash as FfiPbftFinalizationHash,
    PbftManagerBlockValidationFact as FfiPbftManagerBlockValidationFact,
    PbftManagerBlockValidationPlan as FfiPbftManagerBlockValidationPlan,
    PbftManagerLeaderCandidateInputFact as FfiPbftManagerLeaderCandidateInputFact,
    PbftManagerLeaderCandidatePlan as FfiPbftManagerLeaderCandidatePlan,
    PbftManagerLeaderValidBlockCommand as FfiPbftManagerLeaderValidBlockCommand,
    PbftManagerRuntimeActionReport as FfiPbftManagerRuntimeActionReport,
    PbftManagerRuntimeSessionStep as FfiPbftManagerRuntimeSessionStep,
    PbftManagerRuntimeSnapshot as FfiPbftManagerRuntimeSnapshot,
    PbftManagerRuntimeTickFact as FfiPbftManagerRuntimeTickFact,
    PbftManagerStartupFact as FfiPbftManagerStartupFact,
    PbftManagerStateActionFact as FfiPbftManagerStateActionFact,
    PbftManagerStateActionPlan as FfiPbftManagerStateActionPlan,
    PbftManagerTransitionFact as FfiPbftManagerTransitionFact,
    PbftManagerTransitionPlan as FfiPbftManagerTransitionPlan,
    PbftManagerTransitionRuntimeApplyResult as FfiPbftManagerTransitionRuntimeApplyResult,
    PbftManagerTransitionStorageResult as FfiPbftManagerTransitionStorageResult,
};
use crate::ffi::{BridgePbftManagerRuntime, BridgePbftManagerRuntimeSession, BridgeStorage};
use anyhow::anyhow;
use rustaxa_consensus::pbft_manager::{
    abort_pbft_manager_runtime_session as abort_domain_pbft_manager_runtime_session,
    create_pbft_manager_runtime_session as create_domain_pbft_manager_runtime_session,
    next_pbft_manager_runtime_action,
    plan_pbft_manager_block_validation as plan_domain_pbft_manager_block_validation,
    plan_pbft_manager_leader_candidates as plan_domain_pbft_manager_leader_candidates,
    plan_pbft_manager_state_action as plan_domain_pbft_manager_state_action,
    plan_pbft_manager_transition as plan_domain_pbft_manager_transition,
    report_pbft_manager_runtime_action, restore_pbft_manager_runtime,
    PbftManagerBlockValidationFact, PbftManagerBlockValidationFactStatus,
    PbftManagerBlockValidationPlan, PbftManagerLeaderBlockValidationStatus,
    PbftManagerLeaderCandidateInputFact, PbftManagerLeaderCandidatePlan,
    PbftManagerLeaderValidBlockCommand, PbftManagerRuntime, PbftManagerRuntimeAction,
    PbftManagerRuntimeActionReport, PbftManagerRuntimeActionResultCode,
    PbftManagerRuntimeSessionStep, PbftManagerRuntimeSnapshot, PbftManagerRuntimeStateCode,
    PbftManagerRuntimeTickFact, PbftManagerStartupRestoreFact, PbftManagerStartupRestoreStatus,
    PbftManagerStateActionFact, PbftManagerStateActionPlan, PbftManagerTransitionFact,
    PbftManagerTransitionKind, PbftManagerTransitionPlan, PbftManagerTransitionStatus,
};
use rustaxa_storage::StorageWriteBatch;

const RUNTIME_STATUS_ACTIVE: u8 = 0;
const RUNTIME_STATUS_COMPLETE: u8 = 1;
const ACTION_NO_ACTION: u8 = 255;
const TRANSITION_STATUS_READY: u8 = 0;
const TRANSITION_STORAGE_STATUS_APPLIED: u8 = 0;
const TRANSITION_STORAGE_STATUS_REJECTED: u8 = 1;
const PBFT_MGR_FIELD_ROUND: u8 = 0;
const PBFT_MGR_FIELD_STEP: u8 = 1;
const PBFT_MGR_FIELD_LAMBDA: u8 = 2;
const PBFT_MGR_STATUS_EXECUTED_BLOCK: u8 = 0;
const PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE: u8 = 2;
const PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH: u8 = 3;

fn transition_storage_applied(applied_writes: u64) -> FfiPbftManagerTransitionStorageResult {
    FfiPbftManagerTransitionStorageResult {
        status: TRANSITION_STORAGE_STATUS_APPLIED,
        applied_writes,
        error_code: String::new(),
    }
}

fn transition_storage_rejected(error_code: &str) -> FfiPbftManagerTransitionStorageResult {
    FfiPbftManagerTransitionStorageResult {
        status: TRANSITION_STORAGE_STATUS_REJECTED,
        applied_writes: 0,
        error_code: error_code.to_string(),
    }
}

fn to_manager_u32(
    value: u64,
    error_code: &str,
) -> Result<u32, FfiPbftManagerTransitionStorageResult> {
    u32::try_from(value).map_err(|_| transition_storage_rejected(error_code))
}

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

fn append_transition_storage_to_batch(
    storage: &BridgeStorage,
    batch: &mut StorageWriteBatch,
    plan: &FfiPbftManagerTransitionPlan,
) -> Result<u64, FfiPbftManagerTransitionStorageResult> {
    let mut applied_writes = 0;
    let pbft = storage.0.pbft();

    if plan.persist_round {
        let round = to_manager_u32(
            plan.new_round,
            "PBFT_MANAGER_TRANSITION_STORAGE_ROUND_OVERFLOW",
        )?;
        if pbft
            .write_manager_field_in_batch(batch, PBFT_MGR_FIELD_ROUND, round)
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 1;
    }

    if plan.persist_step {
        let step = to_manager_u32(
            plan.new_step,
            "PBFT_MANAGER_TRANSITION_STORAGE_STEP_OVERFLOW",
        )?;
        if pbft
            .write_manager_field_in_batch(batch, PBFT_MGR_FIELD_STEP, step)
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 1;
    }

    if plan.reset_next_voted_statuses {
        if pbft
            .write_manager_status_in_batch(batch, PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH, false)
            .and_then(|_| {
                pbft.write_manager_status_in_batch(
                    batch,
                    PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE,
                    false,
                )
            })
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 2;
    }

    if plan.remove_cert_voted_block {
        if pbft
            .remove_cert_voted_block_in_round_in_batch(batch)
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 1;
    }

    Ok(applied_writes)
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
    let pbft = storage.0.pbft();
    let mut snapshot = restore_pbft_manager_runtime(PbftManagerStartupRestoreFact {
        current_period: fact.current_period,
        persisted_round: u64::from(pbft.manager_field(PBFT_MGR_FIELD_ROUND)?.unwrap_or(1)),
        persisted_step: u64::from(pbft.manager_field(PBFT_MGR_FIELD_STEP)?.unwrap_or(1)),
        cacti_active_at_chain_size: fact.cacti_active_at_chain_size,
        rounds_count_dynamic_lambda: storage.0.metadata().rounds_count_dynamic_lambda()?,
        persisted_dynamic_lambda_ms: pbft.manager_field(PBFT_MGR_FIELD_LAMBDA)?.unwrap_or(1),
        genesis_lambda_ms: to_startup_u32(fact.genesis_lambda_ms, "GENESIS_LAMBDA")?,
        cacti_lambda_max_ms: to_startup_u32(fact.cacti_lambda_max_ms, "CACTI_LAMBDA_MAX")?,
        cacti_lambda_default_ms: to_startup_u32(
            fact.cacti_lambda_default_ms,
            "CACTI_LAMBDA_DEFAULT",
        )?,
        executed_pbft_block: pbft
            .manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)?
            .unwrap_or(false),
        already_next_voted_value: pbft
            .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE)?
            .unwrap_or(false),
        already_next_voted_null: pbft
            .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH)?
            .unwrap_or(false),
    });

    if snapshot.status != PbftManagerStartupRestoreStatus::Ready {
        return Err(anyhow!(snapshot.error_code.clone()));
    }
    if snapshot.persist_normalized_step {
        pbft.write_manager_field(
            PBFT_MGR_FIELD_STEP,
            u32::try_from(snapshot.step)
                .map_err(|_| anyhow!("PBFT_MANAGER_STARTUP_NORMALIZED_STEP_OVERFLOW"))?,
        )?;
        snapshot.persist_normalized_step = false;
    }

    Ok(Box::new(BridgePbftManagerRuntime {
        state: PbftManagerRuntime::new(snapshot),
    }))
}

/// Returns the current Rust-owned PBFT manager runtime snapshot.
pub fn pbft_manager_runtime_snapshot(
    runtime: &BridgePbftManagerRuntime,
) -> FfiPbftManagerRuntimeSnapshot {
    runtime.state.snapshot().into()
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
/// - `storage`: shared Rust storage bridge handle.
/// - `plan`: accepted transition plan.
/// - `own_vote_hashes`: latest own-vote keys to delete when requested by the
///   plan.
///
/// Outputs:
/// - `status = 0` after storage commit and Rust cursor update.
/// - `status = 1` with an unchanged snapshot on rejection.
///
/// Invariants and edge behavior:
/// - The Rust runtime cursor advances only after the Rust storage batch commits.
/// - C++ live mirrors must be updated from the returned snapshot only after an
///   applied status.
pub fn pbft_manager_runtime_apply_transition_storage_write(
    runtime: &mut BridgePbftManagerRuntime,
    storage: &BridgeStorage,
    plan: FfiPbftManagerTransitionPlan,
    own_vote_hashes: Vec<FfiPbftFinalizationHash>,
) -> anyhow::Result<FfiPbftManagerTransitionRuntimeApplyResult> {
    let domain_plan = domain_transition_plan_from_ffi(&plan);
    let storage_result =
        apply_pbft_manager_transition_storage_write(storage, plan, own_vote_hashes)?;
    if storage_result.status != TRANSITION_STORAGE_STATUS_APPLIED {
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
/// - `storage`: shared Rust storage bridge handle.
///
/// Outputs:
/// - `status = 0` after the durable `ExecutedBlock` status is set to false and
///   the runtime snapshot is updated.
/// - `status = 1` with the prior snapshot when storage rejects the write.
///
/// Invariants and edge behavior:
/// - C++ must call this only after preserving the legacy
///   `waitForPeriodFinalization()` ordering.
/// - The Rust runtime changes only after the Rust storage write succeeds.
/// - The returned snapshot is the authoritative source for C++ live mirrors.
pub fn pbft_manager_runtime_apply_executed_block_reset(
    runtime: &mut BridgePbftManagerRuntime,
    storage: &BridgeStorage,
) -> anyhow::Result<FfiPbftManagerTransitionRuntimeApplyResult> {
    if storage
        .0
        .pbft()
        .write_manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK, false)
        .is_err()
    {
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

/// Plans the next Rust-owned PBFT block validation check from live C++ facts.
pub fn plan_pbft_manager_block_validation(
    fact: FfiPbftManagerBlockValidationFact,
) -> FfiPbftManagerBlockValidationPlan {
    plan_domain_pbft_manager_block_validation(fact.into()).into()
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

/// Appends Rust-owned PBFT manager cursor/status transition writes to a batch.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
/// - `batch_id`: caller-owned Rust storage batch id.
/// - `plan`: accepted transition plan from the PBFT manager planner/runtime.
///
/// Outputs:
/// - `status = 0` after all requested manager writes are appended.
/// - `status = 1` with a stable error code if the plan or batch is invalid.
///
/// Invariants and edge behavior:
/// - This function does not commit the batch. C++ owns the existing atomic
///   transition boundary and may append VoteManager own-vote cleanup before
///   committing.
/// - Executed-block reset is intentionally not appended here because legacy
///   ordering performs that write only after `waitForPeriodFinalization()`.
/// - Live C++ mirrors are updated only after the caller commits this batch.
pub fn append_pbft_manager_transition_storage_write(
    storage: &BridgeStorage,
    batch_id: u64,
    plan: FfiPbftManagerTransitionPlan,
) -> Result<FfiPbftManagerTransitionStorageResult, anyhow::Error> {
    if plan.status != TRANSITION_STATUS_READY {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_PLAN_NOT_READY",
        ));
    }

    let mut batches = match storage.1.lock() {
        Ok(batches) => batches,
        Err(_) => {
            return Ok(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_BATCH_REGISTRY_POISONED",
            ));
        }
    };
    let Some(batch) = batches.get_mut(&batch_id) else {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_UNKNOWN_BATCH",
        ));
    };

    match append_transition_storage_to_batch(storage, batch, &plan) {
        Ok(applied_writes) => Ok(transition_storage_applied(applied_writes)),
        Err(result) => Ok(result),
    }
}

/// Applies Rust-owned PBFT manager transition persistence in one committed batch.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
/// - `plan`: accepted transition plan from the PBFT manager planner/runtime.
/// - `own_vote_hashes`: exact latest-round own-vote keys to delete when
///   `plan.clear_own_votes` is set.
///
/// Outputs:
/// - `status = 0` after the Rust batch commits.
/// - `status = 1` with a stable error code if validation, appending, or commit
///   fails. Rejected writes are dropped with the uncommitted Rust batch.
///
/// Invariants and edge behavior:
/// - This owns the storage commit for manager cursor/status transitions.
/// - Live C++ mirrors and VoteManager sidecars must be updated only after an
///   applied result.
/// - Executed-block reset remains outside this batch to preserve the
///   post-`waitForPeriodFinalization()` legacy ordering.
pub fn apply_pbft_manager_transition_storage_write(
    storage: &BridgeStorage,
    plan: FfiPbftManagerTransitionPlan,
    own_vote_hashes: Vec<FfiPbftFinalizationHash>,
) -> Result<FfiPbftManagerTransitionStorageResult, anyhow::Error> {
    if plan.status != TRANSITION_STATUS_READY {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_PLAN_NOT_READY",
        ));
    }
    if !plan.clear_own_votes && !own_vote_hashes.is_empty() {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_UNEXPECTED_OWN_VOTE_HASHES",
        ));
    }

    let mut batch = storage.0.create_write_batch();
    let mut applied_writes = match append_transition_storage_to_batch(storage, &mut batch, &plan) {
        Ok(applied_writes) => applied_writes,
        Err(result) => return Ok(result),
    };

    if plan.clear_own_votes {
        for hash in &own_vote_hashes {
            if storage
                .0
                .pbft()
                .remove_own_verified_vote_in_batch(
                    &mut batch,
                    ethereum_types::H256::from(hash.hash),
                )
                .is_err()
            {
                return Ok(transition_storage_rejected(
                    "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
                ));
            }
        }
        applied_writes += own_vote_hashes.len() as u64;
    }

    if storage
        .0
        .commit_write_batch_with_sync(batch, false)
        .is_err()
    {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_COMMIT_FAILURE",
        ));
    }

    Ok(transition_storage_applied(applied_writes))
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
            has_new_round: value.has_new_round,
            new_round: value.new_round,
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

impl From<FfiPbftManagerBlockValidationFact> for PbftManagerBlockValidationFact {
    fn from(value: FfiPbftManagerBlockValidationFact) -> Self {
        Self {
            block_hash: value.block_hash.into(),
            period: value.period,
            pivot_hash: value.pivot_hash.into(),
            pivot_is_null: value.pivot_is_null,
            dag_order_cached: value.dag_order_cached,
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
            persist_normalized_step: value.persist_normalized_step,
            reset_second_finish_start: value.reset_second_finish_start,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::create_storage;
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
    const RESULT_STATE_DONE: u8 = 2;
    const RESULT_TRANSITION: u8 = 3;
    const RESULT_SLEEP: u8 = 4;
    const STATE_ACTION_STATUS_READY: u8 = 0;
    const STATE_ACTION_PROPOSE_NEW_BLOCK: u8 = 1;
    const STATE_ACTION_SOFT_VOTE_PREVIOUS_VALUE: u8 = 4;
    const STATE_ACTION_NEXT_VOTE_CERT_BLOCK: u8 = 7;
    const STARTUP_STATUS_READY: u8 = 0;
    const TRANSITION_STATUS_READY: u8 = 0;
    const TRANSITION_STATUS_INVALID_FACT: u8 = 2;
    const TRANSITION_RESET: u8 = 0;
    const TRANSITION_FILTER: u8 = 1;
    const TRANSITION_LOOP_BACK_FINISH: u8 = 5;
    const TRANSITION_STORAGE_STATUS_APPLIED: u8 = 0;
    const TRANSITION_STORAGE_STATUS_REJECTED: u8 = 1;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), nanos))
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

    fn startup_fact() -> FfiPbftManagerStartupFact {
        FfiPbftManagerStartupFact {
            current_period: 10,
            cacti_active_at_chain_size: true,
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
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
            assert_eq!(storage.get_pbft_mgr_field(1).unwrap(), 4);
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
            assert_eq!(storage.get_pbft_mgr_field(1).unwrap(), 1);
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
                &storage,
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
            assert_eq!(storage.get_pbft_mgr_field(0).unwrap(), 4);
            assert_eq!(storage.get_pbft_mgr_field(1).unwrap(), 1);
            assert!(!storage.get_pbft_mgr_status(2).unwrap());
            assert!(!storage.get_pbft_mgr_status(3).unwrap());
            assert!(storage.get_own_verified_votes().unwrap().is_empty());
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
            assert!(storage
                .get_pbft_mgr_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)
                .expect("status should load"));

            let result = pbft_manager_runtime_apply_executed_block_reset(&mut runtime, &storage)
                .expect("executed-block reset should not throw");

            assert_eq!(result.status, TRANSITION_STORAGE_STATUS_APPLIED);
            assert_eq!(result.applied_writes, 1);
            assert!(!result.snapshot.executed_pbft_block);
            assert!(!pbft_manager_runtime_snapshot(&runtime).executed_pbft_block);
            assert!(!storage
                .get_pbft_mgr_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)
                .expect("status should load"));
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
            let result = pbft_manager_runtime_apply_transition_storage_write(
                &mut runtime,
                &storage,
                plan,
                Vec::new(),
            )
            .expect("runtime apply should return deterministic rejection");

            assert_eq!(result.status, TRANSITION_STORAGE_STATUS_REJECTED);
            assert_eq!(result.snapshot.round, before.round);
            assert_eq!(result.snapshot.step, before.step);
            assert_eq!(result.snapshot.state, before.state);
            let current = pbft_manager_runtime_snapshot(&runtime);
            assert_eq!(current.round, before.round);
            assert_eq!(current.step, before.step);
            assert_eq!(current.state, before.state);
            assert_eq!(storage.get_pbft_mgr_field(0).unwrap(), 1);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_appends_transition_storage_to_existing_batch() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_transition_storage");
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
                .save_pbft_mgr_status(2, true)
                .expect("soft next status should persist");
            storage
                .save_pbft_mgr_status(3, true)
                .expect("null next status should persist");
            storage
                .save_cert_voted_block_in_round(1, vec![0xC0])
                .expect("cert-voted seed should persist");

            let batch_id = storage
                .create_write_batch()
                .expect("batch should be created");
            let mut plan = plan_pbft_manager_transition(transition_fact(TRANSITION_RESET));
            plan.remove_cert_voted_block = true;
            let result = append_pbft_manager_transition_storage_write(&storage, batch_id, plan)
                .expect("append should not throw");

            assert_eq!(result.status, TRANSITION_STORAGE_STATUS_APPLIED);
            assert_eq!(result.applied_writes, 5);
            assert_eq!(storage.get_pbft_mgr_field(0).unwrap(), 1);
            assert_eq!(storage.get_pbft_mgr_field(1).unwrap(), 1);
            assert!(storage.get_pbft_mgr_status(2).unwrap());
            assert!(storage.get_pbft_mgr_status(3).unwrap());
            assert!(!storage.get_cert_voted_block_in_round().unwrap().is_empty());

            storage
                .commit_write_batch(batch_id, false)
                .expect("batch commit should persist");

            assert_eq!(storage.get_pbft_mgr_field(0).unwrap(), 4);
            assert_eq!(storage.get_pbft_mgr_field(1).unwrap(), 1);
            assert!(!storage.get_pbft_mgr_status(2).unwrap());
            assert!(!storage.get_pbft_mgr_status(3).unwrap());
            assert!(storage.get_cert_voted_block_in_round().unwrap().is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_rejects_transition_storage_append_without_mutation() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_transition_storage_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .save_pbft_mgr_field(0, 3)
                .expect("round seed should persist");

            let plan = plan_pbft_manager_transition(transition_fact(TRANSITION_RESET));
            let result = append_pbft_manager_transition_storage_write(&storage, 999_999, plan)
                .expect("append should return a deterministic rejection");

            assert_eq!(result.status, TRANSITION_STORAGE_STATUS_REJECTED);
            assert_eq!(
                result.error_code,
                "PBFT_MANAGER_TRANSITION_STORAGE_UNKNOWN_BATCH"
            );
            assert_eq!(storage.get_pbft_mgr_field(0).unwrap(), 3);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_applies_transition_storage_and_own_vote_cleanup_atomically() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_transition_storage_apply");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let own_hash = [0xAB; 32];
            storage
                .save_pbft_mgr_field(0, 1)
                .expect("round seed should persist");
            storage
                .save_pbft_mgr_field(1, 1)
                .expect("step seed should persist");
            storage
                .save_pbft_mgr_status(2, true)
                .expect("soft next status should persist");
            storage
                .save_pbft_mgr_status(3, true)
                .expect("null next status should persist");
            storage
                .save_own_verified_vote(&own_hash, vec![0xC0])
                .expect("own vote should persist");

            let plan = plan_pbft_manager_transition(transition_fact(TRANSITION_RESET));
            let result = apply_pbft_manager_transition_storage_write(
                &storage,
                plan,
                vec![FfiPbftFinalizationHash { hash: own_hash }],
            )
            .expect("apply should not throw");

            assert_eq!(result.status, TRANSITION_STORAGE_STATUS_APPLIED);
            assert_eq!(result.applied_writes, 6);
            assert_eq!(storage.get_pbft_mgr_field(0).unwrap(), 4);
            assert_eq!(storage.get_pbft_mgr_field(1).unwrap(), 1);
            assert!(!storage.get_pbft_mgr_status(2).unwrap());
            assert!(!storage.get_pbft_mgr_status(3).unwrap());
            assert!(storage.get_own_verified_votes().unwrap().is_empty());
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

    fn block_validation_fact() -> FfiPbftManagerBlockValidationFact {
        FfiPbftManagerBlockValidationFact {
            block_hash: [1; 32],
            period: 11,
            pivot_hash: [2; 32],
            pivot_is_null: false,
            dag_order_cached: false,
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
