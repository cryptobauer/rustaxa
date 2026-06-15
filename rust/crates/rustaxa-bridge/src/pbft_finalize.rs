//! Bridge wrapper for PBFT finalization intent planning.
//!
//! C++ passes a compact, plain fact bundle gathered from existing execute/finalize
//! flow steps (validation, pillar-finalization check, anchor classification, etc.).
//! Rust performs deterministic intent planning and returns bridge-safe flags and
//! status codes so C++ can apply side effects explicitly.
//!
//! The finalized-period storage apply path is the first native persistence
//! cutover for this path. It writes the storage records that are already
//! represented as stable keys and canonical bytes through Rust-owned storage
//! batches. Live VoteManager, sortition manager mutation, FinalChain, and PBFT
//! runtime side effects remain caller-owned until their Rust transition APIs
//! exist.

use crate::ffi::rustaxa_ffi::{
    PbftDynamicLambdaConfig as FfiPbftDynamicLambdaConfig,
    PbftDynamicLambdaFact as FfiPbftDynamicLambdaFact,
    PbftDynamicLambdaPlan as FfiPbftDynamicLambdaPlan,
    PbftFinalizationCleanupPlan as FfiPbftFinalizationCleanupPlan,
    PbftFinalizationIntentFact as FfiPbftFinalizationIntentFact,
    PbftFinalizationIntentPlan as FfiPbftFinalizationIntentPlan,
    PbftFinalizationLiveMutationReport as FfiPbftFinalizationLiveMutationReport,
    PbftFinalizationLiveMutationValidation as FfiPbftFinalizationLiveMutationValidation,
    PbftFinalizationPositionedHash as FfiPbftFinalizationPositionedHash,
    PbftFinalizationResumePlan as FfiPbftFinalizationResumePlan,
    PbftFinalizationRuntimeActionReport as FfiPbftFinalizationRuntimeActionReport,
    PbftFinalizationRuntimePlan as FfiPbftFinalizationRuntimePlan,
    PbftFinalizationRuntimeSessionStep as FfiPbftFinalizationRuntimeSessionStep,
    PbftFinalizationStorageWritePlan as FfiPbftFinalizationStorageWritePlan,
    PbftFinalizationStorageWriteStage as FfiPbftFinalizationStorageWriteStage,
    PbftFinalizedPeriodApplyResult as FfiPbftFinalizedPeriodApplyResult,
};
use crate::ffi::{BridgePbftFinalizationRuntimeSession, BridgeStorage};
use anyhow::Result;
#[cfg(test)]
use anyhow::{anyhow, Context};
use ethereum_types::H256;
use rustaxa_consensus::pbft_finalize::{
    apply_pbft_finalization_storage_writes as apply_domain_pbft_finalization_storage_writes,
    inspect_pbft_finalization_resume as inspect_domain_pbft_finalization_resume,
    next_pbft_finalization_runtime_action,
    plan_pbft_dynamic_lambda as plan_domain_pbft_dynamic_lambda,
    plan_pbft_finalization_intent as plan_domain_pbft_finalization_intent,
    plan_pbft_finalization_runtime as plan_domain_pbft_finalization_runtime,
    report_pbft_finalization_runtime_action, start_pbft_finalization_resume_runtime,
    start_pbft_finalization_runtime,
    validate_pbft_finalization_live_mutation_report as validate_domain_pbft_finalization_live_mutation_report,
    PbftDynamicLambdaConfig, PbftDynamicLambdaFact, PbftDynamicLambdaPlan, PbftFinalizationAnchor,
    PbftFinalizationCleanupIntent, PbftFinalizationIntentFact, PbftFinalizationLiveMutationReport,
    PbftFinalizationLiveMutationValidation, PbftFinalizationPlan, PbftFinalizationPositionedHash,
    PbftFinalizationResumePlan, PbftFinalizationRuntimeAction, PbftFinalizationRuntimeActionResult,
    PbftFinalizationRuntimeStatus, PbftFinalizationStatus, PbftFinalizationStorageWriteIntent,
    PbftFinalizationStorageWriteStage, PbftFinalizedPeriodApplyResult,
};
#[cfg(test)]
use rustaxa_consensus::sortition::SortitionParamsChange;
#[cfg(test)]
use rustaxa_storage::Column;

#[cfg(test)]
const APPLY_STATUS_APPLIED: u8 = 0;
#[cfg(test)]
const APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES: u8 = 1;
const APPLY_STATUS_REJECTED_WRITE_SET: u8 = 2;
#[cfg(test)]
const APPLY_STATUS_MISSING_REQUIRED_PAYLOAD: u8 = 3;
#[cfg(test)]
const APPLY_STATUS_CONFLICTING_EXISTING_WRITE: u8 = 4;
#[cfg(test)]
const APPEND_STAGE_PRIMARY_FINALIZATION: u8 = 0;
#[cfg(test)]
const APPEND_STAGE_DYNAMIC_LAMBDA: u8 = 1;
#[cfg(test)]
const APPEND_STAGE_EXECUTED_STATUS: u8 = 2;
#[cfg(test)]
const APPEND_STAGE_SORTITION_PARAMS_CHANGE: u8 = 3;
#[cfg(test)]
const APPEND_STAGE_REWARD_VOTES_RESET: u8 = 4;
#[cfg(test)]
const PBFT_MGR_FIELD_LAMBDA: u8 = 2;
#[cfg(test)]
const PBFT_MGR_STATUS_EXECUTED_BLOCK: u8 = 0;
#[cfg(test)]
const PBFT_TWO_T_PLUS_ONE_CERT_VOTED_TYPE: u8 = 1;
const RUNTIME_STATUS_ACTIVE: u8 = 0;
const RUNTIME_STATUS_COMPLETE: u8 = 1;
const RUNTIME_NO_ACTION: u8 = 255;
#[cfg(test)]
const SINGLE_VALUE_KEY: [u8; 4] = 0i32.to_le_bytes();

/// Compatibility helper that appends one explicit PBFT finalization persistence
/// stage to an existing Rust storage batch.
///
/// Stage values:
/// - `0`: primary finalized-period writes (`append_pbft_finalized_period_storage_writes`).
/// - `1`: dynamic-lambda post-adjustment writes.
/// - `2`: executed-status write after FinalChain dispatch.
/// - `3`: sortition params-change write emitted by the Rust sortition runtime.
/// - `4`: reward-vote reset write emitted by the reward-vote reset path.
///
/// `write_set` is validated for stage compatibility; unknown stages return
/// `APPLY_STATUS_REJECTED_WRITE_SET`. This helper is intentionally not exposed
/// through the CXX bridge because Rust-mode production finalization must use
/// `apply_pbft_finalization_storage_writes`, which owns batch creation, commit,
/// and drop behavior inside Rust.
#[cfg(test)]
pub fn append_pbft_finalization_storage_write(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    stage: FfiPbftFinalizationStorageWriteStage,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    match stage.stage {
        APPEND_STAGE_PRIMARY_FINALIZATION => {
            append_pbft_finalized_period_storage_writes_impl(storage, batch_id, write_set)
        }
        APPEND_STAGE_DYNAMIC_LAMBDA => append_pbft_finalization_dynamic_lambda_storage_writes_impl(
            storage,
            batch_id,
            write_set,
            stage.rounds_count_dynamic_lambda,
            stage.dynamic_lambda,
        ),
        APPEND_STAGE_EXECUTED_STATUS => {
            append_pbft_finalization_executed_status_storage_write_impl(
                storage, batch_id, write_set,
            )
        }
        APPEND_STAGE_SORTITION_PARAMS_CHANGE => {
            append_pbft_finalization_sortition_storage_write_impl(
                storage, batch_id, write_set, &stage,
            )
        }
        APPEND_STAGE_REWARD_VOTES_RESET => {
            append_pbft_finalization_reward_votes_reset_storage_write_impl(
                storage, batch_id, write_set, &stage,
            )
        }
        _ => Ok(apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_UNKNOWN_STORAGE_WRITE_STAGE",
        )),
    }
}

/// Applies one or more PBFT finalization persistence stages in a Rust-owned
/// storage batch.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge used only to access the native
///   `rustaxa-storage` handle.
/// - `write_set`: accepted PBFT finalization storage intent from the Rust planner.
/// - `stages`: ordered persistence stages to append to one batch.
/// - `sync`: whether the storage commit should use a synchronous write option.
///
/// Outputs:
/// - The combined apply result. A rejected, missing-payload, or conflicting
///   stage result is returned from the consensus-owned apply helper before any
///   commit.
///
/// Invariants and edge behavior:
/// - Stages are appended in the supplied order and committed atomically in one
///   Rust storage batch.
/// - Staged append APIs remain Rust-side compatibility/test helpers only.
/// - Empty stage lists are rejected without creating durable writes.
/// - Rust storage failures are returned as bridge errors. The bridge does not
///   create, own, or commit a batch for this production apply path.
pub fn apply_pbft_finalization_storage_writes(
    storage: &BridgeStorage,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    stages: Vec<FfiPbftFinalizationStorageWriteStage>,
    sync: bool,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    if stages.is_empty() {
        return Ok(sidecar_apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            "PBFT_FINALIZE_NO_STORAGE_WRITE_STAGES",
        ));
    }

    let domain_stages = stages.into_iter().map(Into::into).collect();

    Ok(apply_result_from_domain(
        apply_domain_pbft_finalization_storage_writes(
            storage.0.as_ref(),
            &PbftFinalizationStorageWriteIntent::from(write_set),
            domain_stages,
            sync,
        )?,
    ))
}

#[cfg(test)]
fn empty_stage(stage: u8) -> FfiPbftFinalizationStorageWriteStage {
    FfiPbftFinalizationStorageWriteStage {
        stage,
        rounds_count_dynamic_lambda: 0,
        dynamic_lambda: 0,
        has_sortition_params_change: false,
        sortition_params_change_period: 0,
        sortition_params_change_interval_efficiency: 0,
        sortition_params_change_threshold_upper: 0,
        has_reward_votes_reset: false,
        reward_votes_bundle_rlp: Vec::new(),
        extra_reward_vote_hashes: Vec::new(),
    }
}

impl From<FfiPbftFinalizationStorageWriteStage> for PbftFinalizationStorageWriteStage {
    fn from(value: FfiPbftFinalizationStorageWriteStage) -> Self {
        Self {
            stage: value.stage,
            rounds_count_dynamic_lambda: value.rounds_count_dynamic_lambda,
            dynamic_lambda: value.dynamic_lambda,
            has_sortition_params_change: value.has_sortition_params_change,
            sortition_params_change_period: value.sortition_params_change_period,
            sortition_params_change_interval_efficiency: value
                .sortition_params_change_interval_efficiency,
            sortition_params_change_threshold_upper: value.sortition_params_change_threshold_upper,
            has_reward_votes_reset: value.has_reward_votes_reset,
            reward_votes_bundle_rlp: value.reward_votes_bundle_rlp,
            extra_reward_vote_hashes: value
                .extra_reward_vote_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
        }
    }
}

fn apply_result_from_domain(
    value: PbftFinalizedPeriodApplyResult,
) -> FfiPbftFinalizedPeriodApplyResult {
    FfiPbftFinalizedPeriodApplyResult {
        status: value.status.as_u8(),
        wrote_pbft_head: value.wrote_pbft_head,
        wrote_period_data: value.wrote_period_data,
        dag_index_writes: value.dag_index_writes,
        transaction_location_writes: value.transaction_location_writes,
        block_period: value.block_period,
        pbft_block_hash: value.pbft_block_hash.0,
        error_code: value.error_code,
    }
}

/// C++/Rust bridge entry for one deterministic PBFT finalization intent.
pub fn plan_pbft_finalization_intent(
    fact: FfiPbftFinalizationIntentFact,
) -> FfiPbftFinalizationIntentPlan {
    plan_domain_pbft_finalization_intent(fact.into()).into()
}

/// C++/Rust bridge entry for the ordered PBFT finalization runtime script.
///
/// Inputs:
/// - `plan`: finalization intent returned by `plan_pbft_finalization_intent`.
///
/// Outputs:
/// - Stable runtime action codes in the order the mixed shim executor must
///   apply side effects.
///
/// Rejected plans return no actions. This keeps finalization candidate
/// decisions, live runtime sequencing, and storage-apply status in separate
/// bridge status spaces.
pub fn plan_pbft_finalization_runtime(
    plan: &FfiPbftFinalizationIntentPlan,
) -> FfiPbftFinalizationRuntimePlan {
    let domain_plan = PbftFinalizationPlan::from(plan);
    plan_domain_pbft_finalization_runtime(&domain_plan).into()
}

/// Creates a Rust-owned PBFT finalization runtime session.
///
/// The session owns the runtime cursor. C++ can only request the next action and
/// report whether that action succeeded. The existing one-shot runtime plan
/// remains available for compatibility tests and callers that only need to
/// inspect action order.
pub fn create_pbft_finalization_runtime_session(
    plan: &FfiPbftFinalizationIntentPlan,
) -> Box<BridgePbftFinalizationRuntimeSession> {
    let domain_plan = PbftFinalizationPlan::from(plan);
    let runtime_plan = plan_domain_pbft_finalization_runtime(&domain_plan);
    Box::new(BridgePbftFinalizationRuntimeSession {
        state: start_pbft_finalization_runtime(&runtime_plan),
    })
}

/// Creates a Rust-owned runtime session for a durable PBFT finalization resume
/// plan.
///
/// The returned session uses the same next/report cursor contract as normal
/// finalization. Its action script is the resume plan's storage-derived replay
/// actions, so C++ can only execute the bounded tail that Rust classified as
/// safe from durable facts.
pub fn create_pbft_finalization_resume_runtime_session(
    plan: &FfiPbftFinalizationResumePlan,
) -> Box<BridgePbftFinalizationRuntimeSession> {
    let domain_plan = PbftFinalizationResumePlan::from(plan);
    Box::new(BridgePbftFinalizationRuntimeSession {
        state: start_pbft_finalization_resume_runtime(&domain_plan),
    })
}

/// Returns the next action requested by a Rust-owned PBFT finalization runtime
/// session without advancing the cursor.
pub fn pbft_finalization_runtime_session_next(
    session: &mut BridgePbftFinalizationRuntimeSession,
) -> FfiPbftFinalizationRuntimeSessionStep {
    next_pbft_finalization_runtime_action(&session.state).into()
}

/// Reports one C++-executed action back to the Rust-owned runtime session.
///
/// `cursor` and `action` must match the current Rust-planned step. On success
/// the Rust cursor advances. On failure or mismatch the session enters a
/// terminal failure state and returns no further action.
pub fn pbft_finalization_runtime_session_report(
    session: &mut BridgePbftFinalizationRuntimeSession,
    cursor: u32,
    action: u8,
    success: bool,
    action_status: u8,
) -> FfiPbftFinalizationRuntimeSessionStep {
    let step = next_pbft_finalization_runtime_action(&session.state);
    if step.action_index != cursor {
        session.state.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
        session.state.error_code = "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH".to_string();
        return next_pbft_finalization_runtime_action(&session.state).into();
    }

    let Some(action) = PbftFinalizationRuntimeAction::from_u8(action) else {
        session.state.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
        session.state.error_code = "PBFT_FINALIZE_RUNTIME_UNKNOWN_ACTION".to_string();
        return next_pbft_finalization_runtime_action(&session.state).into();
    };

    let error_code = if success {
        String::new()
    } else {
        format!("PBFT_FINALIZE_RUNTIME_ACTION_STATUS_{action_status}")
    };
    let state = session.state.clone();
    session.state = report_pbft_finalization_runtime_action(
        state,
        PbftFinalizationRuntimeActionResult {
            action,
            success,
            error_code,
        },
    );
    next_pbft_finalization_runtime_action(&session.state).into()
}

/// Reports one structured PBFT finalization action result back to the Rust-owned
/// runtime session.
///
/// This entrypoint preserves action-specific error codes in the Rust terminal
/// state. The legacy scalar-status report API remains as a compatibility wrapper
/// for callers that only need generic status-code errors.
pub fn pbft_finalization_runtime_session_report_action(
    session: &mut BridgePbftFinalizationRuntimeSession,
    report: FfiPbftFinalizationRuntimeActionReport,
) -> FfiPbftFinalizationRuntimeSessionStep {
    let step = next_pbft_finalization_runtime_action(&session.state);
    if step.action_index != report.cursor {
        session.state.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
        session.state.error_code = "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH".to_string();
        return next_pbft_finalization_runtime_action(&session.state).into();
    }

    let Some(action) = PbftFinalizationRuntimeAction::from_u8(report.action) else {
        session.state.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
        session.state.error_code = "PBFT_FINALIZE_RUNTIME_UNKNOWN_ACTION".to_string();
        return next_pbft_finalization_runtime_action(&session.state).into();
    };

    let error_code = if report.success {
        String::new()
    } else if report.error_code.is_empty() {
        format!("PBFT_FINALIZE_RUNTIME_ACTION_STATUS_{}", report.status)
    } else {
        report.error_code.to_string()
    };
    let state = session.state.clone();
    session.state = report_pbft_finalization_runtime_action(
        state,
        PbftFinalizationRuntimeActionResult {
            action,
            success: report.success,
            error_code,
        },
    );
    next_pbft_finalization_runtime_action(&session.state).into()
}

/// Validates a post-action live mutation report against the accepted Rust PBFT
/// finalization plan.
///
/// C++ still executes the DAG, TransactionManager, and PBFT-chain shim calls, but
/// the Rust planner verifies their post-state proofs before the PBFT runtime
/// cursor is advanced.
pub fn validate_pbft_finalization_live_mutation_report(
    plan: &FfiPbftFinalizationIntentPlan,
    report: FfiPbftFinalizationLiveMutationReport,
) -> FfiPbftFinalizationLiveMutationValidation {
    if PbftFinalizationRuntimeAction::from_u8(report.action).is_none() {
        return FfiPbftFinalizationLiveMutationValidation {
            accepted: false,
            status:
                rustaxa_consensus::pbft_finalize::PbftFinalizationLiveMutationStatus::UnknownAction
                    .as_u8(),
            action: report.action,
            error_code: "PBFT_FINALIZE_LIVE_MUTATION_UNKNOWN_ACTION".to_string(),
        };
    }
    let domain_plan = PbftFinalizationPlan::from(plan);
    validate_domain_pbft_finalization_live_mutation_report(&domain_plan, report.into()).into()
}

/// Aborts a Rust-owned PBFT finalization runtime session after C++ gives up on
/// the mixed executor path.
pub fn abort_pbft_finalization_runtime_session(session: &mut BridgePbftFinalizationRuntimeSession) {
    if session.state.runtime_status == PbftFinalizationRuntimeStatus::Active {
        session.state.runtime_status = PbftFinalizationRuntimeStatus::ActionFailed;
        session.state.error_code = "PBFT_FINALIZE_RUNTIME_ABORTED".to_string();
    }
}

/// Inspects Rust-owned finalized-period storage for a duplicate PBFT block and
/// returns a durable resume classification.
///
/// Inputs:
/// - `storage`: Rust storage bridge used by the C++ storage shim.
/// - `write_set`: expected finalized-period write set for the duplicate block.
/// - `final_chain_last_block`: C++ FinalChain durable height, which Rust cannot
///   read from consensus storage.
///
/// Outputs:
/// - A bridge-safe resume plan. The plan classifies whether primary PBFT
///   finalization storage is complete, whether dynamic-lambda and executed
///   status are durable, and which durable replay actions remain visible.
///
/// Edge behavior:
/// - Mutable PBFT-head JSON is not used for conflict detection because startup
///   may already have recovered the live PBFT chain from that head. Immutable
///   hash-period, period-data, DAG-position, transaction-position, period
///   lambda, and executed-status rows are checked exactly.
/// - Storage conflicts are never repaired by this API.
pub fn inspect_pbft_finalization_resume(
    storage: &BridgeStorage,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    final_chain_last_block: u64,
) -> Result<FfiPbftFinalizationResumePlan> {
    let write_set: PbftFinalizationStorageWriteIntent = write_set.into();
    inspect_domain_pbft_finalization_resume(&storage.0, &write_set, final_chain_last_block)
        .map(Into::into)
}

impl BridgePbftFinalizationRuntimeSession {
    /// Returns the next action requested by this Rust-owned PBFT finalization
    /// runtime session without advancing the cursor.
    pub fn pbft_finalization_runtime_session_next(
        &mut self,
    ) -> FfiPbftFinalizationRuntimeSessionStep {
        pbft_finalization_runtime_session_next(self)
    }

    /// Reports one C++-executed action back to this Rust-owned PBFT
    /// finalization runtime session.
    pub fn pbft_finalization_runtime_session_report(
        &mut self,
        cursor: u32,
        action: u8,
        success: bool,
        action_status: u8,
    ) -> FfiPbftFinalizationRuntimeSessionStep {
        pbft_finalization_runtime_session_report(self, cursor, action, success, action_status)
    }

    /// Reports one structured action result back to this Rust-owned PBFT
    /// finalization runtime session.
    pub fn pbft_finalization_runtime_session_report_action(
        &mut self,
        report: FfiPbftFinalizationRuntimeActionReport,
    ) -> FfiPbftFinalizationRuntimeSessionStep {
        pbft_finalization_runtime_session_report_action(self, report)
    }

    /// Aborts this runtime session after C++ gives up on the mixed executor path.
    pub fn abort_pbft_finalization_runtime_session(&mut self) {
        abort_pbft_finalization_runtime_session(self);
    }
}

/// C++/Rust bridge entry for Cacti dynamic-lambda calculation.
///
/// Inputs are only live pre-state, finalized-period facts, and Cacti config.
/// The function does not read storage or mutate live state. On success, C++ must
/// assign the returned live fields before passing them to the dynamic-lambda
/// storage stage.
pub fn plan_pbft_dynamic_lambda(fact: FfiPbftDynamicLambdaFact) -> FfiPbftDynamicLambdaPlan {
    plan_domain_pbft_dynamic_lambda(fact.into()).into()
}

/// Rust-side compatibility helper that appends finalized-period storage writes
/// to an existing bridge batch.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge used by the C++ storage shim.
/// - `batch_id`: an existing bridge batch id owned by the caller's `Batch`.
/// - `write_set`: accepted PBFT finalization storage intent from the Rust planner.
///
/// Outputs:
/// - `status` reports whether writes were appended, already present, rejected, or conflicted.
/// - count fields report how many finalized DAG/transaction indexes were appended.
///
/// Invariants and edge behavior:
/// - The function appends primary finalized-period records: PBFT head,
///   PBFT hash-to-period, period-data RLP, DAG finalized indexes, transaction
///   finalized indexes, and deletes of pending DAG/transaction rows.
/// - It does not commit the batch. Production Rust-mode finalization should use
///   `apply_pbft_finalization_storage_writes` instead so batch ownership stays
///   inside `rustaxa-consensus`.
/// - Missing required payloads or conflicting existing immutable finalized
///   records return a non-applied status and do not mutate the batch. `PbftHead`
///   is mutable chain-head state and is intentionally replaced when present.
/// - Storage backend or unknown-batch failures are returned as bridge errors.
#[cfg(test)]
pub fn append_pbft_finalized_period_storage_writes(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    append_pbft_finalization_storage_write(
        storage,
        batch_id,
        write_set,
        empty_stage(APPEND_STAGE_PRIMARY_FINALIZATION),
    )
}

/// Rust-side compatibility helper that appends dynamic-lambda persistence after
/// the caller has supplied the post-adjustment live `PbftManager` fields.
///
/// Inputs:
/// - `storage` and `batch_id` identify a compatibility batch.
/// - `write_set` is the accepted PBFT finalization storage intent.
/// - `rounds_count_dynamic_lambda` and `dynamic_lambda` are the post-adjust live
///   values that must become durable with the optional period-lambda row.
///
/// Outputs and invariants:
/// - Returns the same apply status envelope as the primary appender.
/// - Rejects write sets that did not request dynamic-lambda persistence.
/// - Treats `period_lambda` as immutable for a finalized period and reports a
///   conflict when an existing value differs. Manager lambda and round-count
///   fields are mutable PBFT manager state and are overwritten.
#[cfg(test)]
pub fn append_pbft_finalization_dynamic_lambda_storage_writes(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    rounds_count_dynamic_lambda: u32,
    dynamic_lambda: u32,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    append_pbft_finalization_storage_write(
        storage,
        batch_id,
        write_set,
        FfiPbftFinalizationStorageWriteStage {
            stage: APPEND_STAGE_DYNAMIC_LAMBDA,
            rounds_count_dynamic_lambda,
            dynamic_lambda,
            has_sortition_params_change: false,
            sortition_params_change_period: 0,
            sortition_params_change_interval_efficiency: 0,
            sortition_params_change_threshold_upper: 0,
            has_reward_votes_reset: false,
            reward_votes_bundle_rlp: Vec::new(),
            extra_reward_vote_hashes: Vec::new(),
        },
    )
}

/// Rust-side compatibility helper that appends the PBFT manager executed-block
/// status after FinalChain finalization has been dispatched.
///
/// This preserves the legacy ordering where durable `ExecutedBlock=true` is not
/// written before the final-chain path is invoked, while keeping the byte-level
/// persistence in Rust.
#[cfg(test)]
pub fn append_pbft_finalization_executed_status_storage_write(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    append_pbft_finalization_storage_write(
        storage,
        batch_id,
        write_set,
        empty_stage(APPEND_STAGE_EXECUTED_STATUS),
    )
}

#[cfg(test)]
fn append_pbft_finalized_period_storage_writes_impl(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    if !write_set.persist_pbft_head && !write_set.persist_period_data {
        return Ok(apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_REJECTED_WRITE_SET",
        ));
    }

    if write_set.persist_pbft_head && write_set.pbft_head_payload.is_empty() {
        return Ok(apply_result(
            APPLY_STATUS_MISSING_REQUIRED_PAYLOAD,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_MISSING_PBFT_HEAD_PAYLOAD",
        ));
    }

    if write_set.persist_period_data && write_set.period_data_rlp.is_empty() {
        return Ok(apply_result(
            APPLY_STATUS_MISSING_REQUIRED_PAYLOAD,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_MISSING_PERIOD_DATA_RLP",
        ));
    }

    let mut already_applied = true;
    let mut pending_deletes_absent = true;
    let pbft_block_hash = H256::from(write_set.pbft_block_hash);
    let pbft_head_hash = H256::from(write_set.pbft_head_hash);

    if write_set.persist_pbft_head {
        already_applied &= storage
            .0
            .get_raw(Column::PbftHead, pbft_head_hash.as_bytes())?
            .as_deref()
            == Some(write_set.pbft_head_payload.as_slice());
    }

    if write_set.persist_period_data {
        let period_key = write_set.block_period.to_le_bytes();
        let period_value = write_set.block_period.to_le_bytes();
        if check_existing_value(
            storage,
            Column::PbftBlockPeriod,
            pbft_block_hash.as_bytes(),
            &period_value,
            "PBFT_FINALIZE_CONFLICTING_PBFT_PERIOD",
        )? {
            return Ok(apply_result(
                APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                write_set,
                0,
                0,
                "PBFT_FINALIZE_CONFLICTING_PBFT_PERIOD",
            ));
        }
        already_applied &= storage
            .0
            .get_raw(Column::PbftBlockPeriod, pbft_block_hash.as_bytes())?
            .is_some();

        if check_existing_value(
            storage,
            Column::PeriodData,
            &period_key,
            &write_set.period_data_rlp,
            "PBFT_FINALIZE_CONFLICTING_PERIOD_DATA",
        )? {
            return Ok(apply_result(
                APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                write_set,
                0,
                0,
                "PBFT_FINALIZE_CONFLICTING_PERIOD_DATA",
            ));
        }
        already_applied &= storage
            .0
            .get_raw(Column::PeriodData, &period_key)?
            .is_some();

        for write in &write_set.dag_block_period_writes {
            let hash = H256::from(write.hash);
            let value = block_position_rlp(write_set.block_period, write.position);
            if check_existing_value(
                storage,
                Column::DagBlockPeriod,
                hash.as_bytes(),
                &value,
                "PBFT_FINALIZE_CONFLICTING_DAG_PERIOD",
            )? {
                return Ok(apply_result(
                    APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                    write_set,
                    0,
                    0,
                    "PBFT_FINALIZE_CONFLICTING_DAG_PERIOD",
                ));
            }
            already_applied &= storage
                .0
                .get_raw(Column::DagBlockPeriod, hash.as_bytes())?
                .is_some();
            pending_deletes_absent &= storage
                .0
                .get_raw(Column::DagBlocks, hash.as_bytes())?
                .is_none();
        }

        for write in &write_set.transaction_location_writes {
            let hash = H256::from(write.hash);
            let value = block_position_rlp(write_set.block_period, write.position);
            if check_existing_value(
                storage,
                Column::TrxPeriod,
                hash.as_bytes(),
                &value,
                "PBFT_FINALIZE_CONFLICTING_TRANSACTION_LOCATION",
            )? {
                return Ok(apply_result(
                    APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                    write_set,
                    0,
                    0,
                    "PBFT_FINALIZE_CONFLICTING_TRANSACTION_LOCATION",
                ));
            }
            already_applied &= storage
                .0
                .get_raw(Column::TrxPeriod, hash.as_bytes())?
                .is_some();
            pending_deletes_absent &= storage
                .0
                .get_raw(Column::Transactions, hash.as_bytes())?
                .is_none();
        }
    }

    {
        let mut batches = storage
            .1
            .lock()
            .map_err(|_| anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow!("unknown batch id: {batch_id}"))?;

        if write_set.persist_pbft_head {
            storage
                .0
                .batch_put_raw(
                    batch,
                    Column::PbftHead,
                    pbft_head_hash.as_bytes(),
                    &write_set.pbft_head_payload,
                )
                .context("PBFT_FINALIZE_BATCH_PBFT_HEAD")?;
        }

        if write_set.persist_period_data {
            storage
                .0
                .batch_put_raw(
                    batch,
                    Column::PbftBlockPeriod,
                    pbft_block_hash.as_bytes(),
                    &write_set.block_period.to_le_bytes(),
                )
                .context("PBFT_FINALIZE_BATCH_PBFT_PERIOD")?;
            storage
                .0
                .batch_put_raw(
                    batch,
                    Column::PeriodData,
                    &write_set.block_period.to_le_bytes(),
                    &write_set.period_data_rlp,
                )
                .context("PBFT_FINALIZE_BATCH_PERIOD_DATA")?;

            for write in &write_set.dag_block_period_writes {
                let hash = H256::from(write.hash);
                storage
                    .0
                    .batch_delete_raw(batch, Column::DagBlocks, hash.as_bytes())
                    .context("PBFT_FINALIZE_BATCH_DELETE_PENDING_DAG")?;
                storage
                    .0
                    .batch_put_raw(
                        batch,
                        Column::DagBlockPeriod,
                        hash.as_bytes(),
                        &block_position_rlp(write_set.block_period, write.position),
                    )
                    .context("PBFT_FINALIZE_BATCH_DAG_PERIOD")?;
            }

            for write in &write_set.transaction_location_writes {
                let hash = H256::from(write.hash);
                storage
                    .0
                    .batch_delete_raw(batch, Column::Transactions, hash.as_bytes())
                    .context("PBFT_FINALIZE_BATCH_DELETE_PENDING_TRANSACTION")?;
                storage
                    .0
                    .batch_put_raw(
                        batch,
                        Column::TrxPeriod,
                        hash.as_bytes(),
                        &block_position_rlp(write_set.block_period, write.position),
                    )
                    .context("PBFT_FINALIZE_BATCH_TRANSACTION_LOCATION")?;
            }
        }
    }

    let status = if already_applied && pending_deletes_absent {
        APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES
    } else {
        APPLY_STATUS_APPLIED
    };
    Ok(apply_result(
        status,
        write_set,
        write_set.dag_block_period_writes.len(),
        write_set.transaction_location_writes.len(),
        "",
    ))
}

#[cfg(test)]
fn append_pbft_finalization_dynamic_lambda_storage_writes_impl(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    rounds_count_dynamic_lambda: u32,
    dynamic_lambda: u32,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    if !write_set.apply_dynamic_lambda_update {
        return Ok(apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_DYNAMIC_LAMBDA_NOT_REQUESTED",
        ));
    }

    let mut already_applied = true;
    if write_set.persist_period_lambda {
        let period_key = write_set.block_period.to_le_bytes();
        let period_lambda = write_set.period_lambda.to_le_bytes();
        if check_existing_value(
            storage,
            Column::PeriodLambda,
            &period_key,
            &period_lambda,
            "PBFT_FINALIZE_CONFLICTING_PERIOD_LAMBDA",
        )? {
            return Ok(apply_result(
                APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                write_set,
                0,
                0,
                "PBFT_FINALIZE_CONFLICTING_PERIOD_LAMBDA",
            ));
        }
        already_applied &= storage
            .0
            .get_raw(Column::PeriodLambda, &period_key)?
            .is_some();
    }
    already_applied &= storage
        .0
        .get_raw(Column::RoundsCountDynamicLambda, &SINGLE_VALUE_KEY)?
        .as_deref()
        == Some(&rounds_count_dynamic_lambda.to_le_bytes());
    already_applied &= storage
        .0
        .get_raw(Column::PbftMgrRoundStep, &[PBFT_MGR_FIELD_LAMBDA])?
        .as_deref()
        == Some(&dynamic_lambda.to_le_bytes());

    {
        let mut batches = storage
            .1
            .lock()
            .map_err(|_| anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow!("unknown batch id: {batch_id}"))?;

        if write_set.persist_period_lambda {
            storage
                .0
                .batch_put_raw(
                    batch,
                    Column::PeriodLambda,
                    &write_set.block_period.to_le_bytes(),
                    &write_set.period_lambda.to_le_bytes(),
                )
                .context("PBFT_FINALIZE_BATCH_PERIOD_LAMBDA")?;
        }
        storage
            .0
            .batch_put_raw(
                batch,
                Column::RoundsCountDynamicLambda,
                &SINGLE_VALUE_KEY,
                &rounds_count_dynamic_lambda.to_le_bytes(),
            )
            .context("PBFT_FINALIZE_BATCH_DYNAMIC_LAMBDA_ROUNDS")?;
        storage
            .0
            .batch_put_raw(
                batch,
                Column::PbftMgrRoundStep,
                &[PBFT_MGR_FIELD_LAMBDA],
                &dynamic_lambda.to_le_bytes(),
            )
            .context("PBFT_FINALIZE_BATCH_DYNAMIC_LAMBDA_FIELD")?;
    }

    let status = if already_applied {
        APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES
    } else {
        APPLY_STATUS_APPLIED
    };
    Ok(sidecar_apply_result(status, write_set, ""))
}

#[cfg(test)]
fn append_pbft_finalization_executed_status_storage_write_impl(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    if !write_set.persist_executed_pbft_status {
        return Ok(apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_EXECUTED_STATUS_NOT_REQUESTED",
        ));
    }

    let status_key = [PBFT_MGR_STATUS_EXECUTED_BLOCK];
    let already_applied = storage
        .0
        .get_raw(Column::PbftMgrStatus, &status_key)?
        .as_deref()
        == Some(&[u8::from(write_set.executed_pbft_status)]);

    {
        let mut batches = storage
            .1
            .lock()
            .map_err(|_| anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow!("unknown batch id: {batch_id}"))?;
        storage
            .0
            .batch_put_raw(
                batch,
                Column::PbftMgrStatus,
                &status_key,
                &[u8::from(write_set.executed_pbft_status)],
            )
            .context("PBFT_FINALIZE_BATCH_EXECUTED_STATUS")?;
    }

    let status = if already_applied {
        APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES
    } else {
        APPLY_STATUS_APPLIED
    };
    Ok(sidecar_apply_result(status, write_set, ""))
}

#[cfg(test)]
fn append_pbft_finalization_sortition_storage_write_impl(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    stage: &FfiPbftFinalizationStorageWriteStage,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    if !write_set.update_sortition_params {
        return Ok(apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_SORTITION_PARAMS_UPDATE_NOT_REQUESTED",
        ));
    }

    if !stage.has_sortition_params_change {
        return Ok(apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_MISSING_SORTITION_PARAMS_CHANGE_FACTS",
        ));
    }

    let change = SortitionParamsChange {
        period: stage.sortition_params_change_period,
        interval_efficiency: stage.sortition_params_change_interval_efficiency,
        threshold_upper: stage.sortition_params_change_threshold_upper,
    };
    let change_rlp = change.to_rlp_bytes();
    if check_existing_value(
        storage,
        Column::SortitionParamsChange,
        &change.period.to_le_bytes(),
        &change_rlp,
        "PBFT_FINALIZE_CONFLICTING_SORTITION_PARAMS_CHANGE",
    )? {
        return Ok(apply_result(
            APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_CONFLICTING_SORTITION_PARAMS_CHANGE",
        ));
    }

    let already_applied = storage
        .0
        .get_raw(Column::SortitionParamsChange, &change.period.to_le_bytes())?
        .as_deref()
        == Some(change_rlp.as_slice());

    {
        let mut batches = storage
            .1
            .lock()
            .map_err(|_| anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow!("unknown batch id: {batch_id}"))?;
        storage
            .0
            .batch_put_raw(
                batch,
                Column::SortitionParamsChange,
                &change.period.to_le_bytes(),
                &change_rlp,
            )
            .context("PBFT_FINALIZE_BATCH_SORTITION_CHANGE")?;
    }

    let status = if already_applied {
        APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES
    } else {
        APPLY_STATUS_APPLIED
    };
    Ok(sidecar_apply_result(status, write_set, ""))
}

#[cfg(test)]
fn append_pbft_finalization_reward_votes_reset_storage_write_impl(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    stage: &FfiPbftFinalizationStorageWriteStage,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    if !write_set.reset_reward_votes {
        return Ok(sidecar_apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            "PBFT_FINALIZE_REWARD_VOTES_RESET_NOT_REQUESTED",
        ));
    }

    if !stage.has_reward_votes_reset {
        return Ok(sidecar_apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            "PBFT_FINALIZE_MISSING_REWARD_VOTES_RESET_FACTS",
        ));
    }

    if stage.reward_votes_bundle_rlp.is_empty() {
        return Ok(sidecar_apply_result(
            APPLY_STATUS_MISSING_REQUIRED_PAYLOAD,
            write_set,
            "PBFT_FINALIZE_MISSING_REWARD_VOTES_BUNDLE_RLP",
        ));
    }

    let rewards_bundle = rlp::Rlp::new(stage.reward_votes_bundle_rlp.as_slice());
    if !rewards_bundle.is_list() {
        return Ok(sidecar_apply_result(
            APPLY_STATUS_MISSING_REQUIRED_PAYLOAD,
            write_set,
            "PBFT_FINALIZE_REWARD_VOTES_BUNDLE_NOT_LIST",
        ));
    }
    let rewards_count = match rewards_bundle.item_count() {
        Ok(count) => count,
        Err(_) => {
            return Ok(sidecar_apply_result(
                APPLY_STATUS_MISSING_REQUIRED_PAYLOAD,
                write_set,
                "PBFT_FINALIZE_REWARD_VOTES_BUNDLE_NOT_LIST",
            ));
        }
    };
    if rewards_count == 0 {
        return Ok(sidecar_apply_result(
            APPLY_STATUS_MISSING_REQUIRED_PAYLOAD,
            write_set,
            "PBFT_FINALIZE_REWARD_VOTES_BUNDLE_EMPTY",
        ));
    }

    let cert_voted_key = [PBFT_TWO_T_PLUS_ONE_CERT_VOTED_TYPE];
    let mut already_applied = storage
        .0
        .get_raw(Column::LatestRoundTwoTPlusOneVotes, &cert_voted_key)?
        .as_deref()
        == Some(stage.reward_votes_bundle_rlp.as_slice());
    for vote_hash in &stage.extra_reward_vote_hashes {
        let vote_hash = H256::from(vote_hash.hash);
        already_applied &= storage
            .0
            .get_raw(Column::ExtraRewardVotes, vote_hash.as_bytes())?
            .is_none();
    }

    {
        let mut batches = storage
            .1
            .lock()
            .map_err(|_| anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow!("unknown batch id: {batch_id}"))?;
        storage
            .0
            .batch_delete_raw(batch, Column::LatestRoundTwoTPlusOneVotes, &cert_voted_key)
            .context("PBFT_FINALIZE_BATCH_DELETE_REWARD_VOTES_BUNDLE")?;
        storage
            .0
            .batch_put_raw(
                batch,
                Column::LatestRoundTwoTPlusOneVotes,
                &cert_voted_key,
                &stage.reward_votes_bundle_rlp,
            )
            .context("PBFT_FINALIZE_BATCH_REPLACE_REWARD_VOTES_BUNDLE")?;
        for vote_hash in &stage.extra_reward_vote_hashes {
            storage
                .0
                .batch_delete_raw(
                    batch,
                    Column::ExtraRewardVotes,
                    H256::from(vote_hash.hash).as_bytes(),
                )
                .context("PBFT_FINALIZE_BATCH_DELETE_EXTRA_REWARD_VOTE")?;
        }
    }

    let status = if already_applied {
        APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES
    } else {
        APPLY_STATUS_APPLIED
    };
    Ok(sidecar_apply_result(status, write_set, ""))
}

impl From<FfiPbftFinalizationIntentFact> for PbftFinalizationIntentFact {
    fn from(value: FfiPbftFinalizationIntentFact) -> Self {
        Self {
            block_hash: H256::from(value.block_hash),
            pbft_head_hash: H256::from(value.pbft_head_hash),
            block_period: value.block_period,
            block_prev_hash: H256::from(value.block_prev_hash),
            chain_last_hash: H256::from(value.chain_last_hash),
            chain_last_period: value.chain_last_period,
            block_in_chain: value.block_in_chain,
            pivot_dag_anchor_hash: H256::from(value.pivot_dag_anchor_hash),
            has_pillar_block: value.has_pillar_block,
            pillar_block_finalized: value.pillar_block_finalized,
            request_dynamic_lambda_update: value.request_dynamic_lambda_update,
            cert_vote_count: value.cert_vote_count,
            sample_cert_vote_block_hash: H256::from(value.sample_cert_vote_block_hash),
            sample_cert_vote_period: value.sample_cert_vote_period,
            sample_cert_vote_round: value.sample_cert_vote_round,
            sample_cert_vote_step: value.sample_cert_vote_step,
            block_lambda: value.block_lambda,
            last_saved_period_lambda_found: value.last_saved_period_lambda_found,
            last_saved_period_lambda: value.last_saved_period_lambda,
            dynamic_blocks_per_year: value.dynamic_blocks_per_year,
            dpos_blocks_per_year: value.dpos_blocks_per_year,
            pbft_head_payload: value.pbft_head_payload,
            period_data_rlp: value.period_data_rlp,
            ordered_dag_block_hashes: value
                .ordered_dag_block_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            ordered_transaction_hashes: value
                .ordered_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
        }
    }
}

impl From<FfiPbftDynamicLambdaConfig> for PbftDynamicLambdaConfig {
    fn from(value: FfiPbftDynamicLambdaConfig) -> Self {
        Self {
            cacti_block_num: value.cacti_block_num,
            lambda_min: value.lambda_min,
            lambda_max: value.lambda_max,
            lambda_default: value.lambda_default,
            lambda_change_interval: value.lambda_change_interval,
            lambda_change: value.lambda_change,
            consensus_delay: value.consensus_delay,
            dpos_blocks_per_year: value.dpos_blocks_per_year,
        }
    }
}

impl From<FfiPbftDynamicLambdaFact> for PbftDynamicLambdaFact {
    fn from(value: FfiPbftDynamicLambdaFact) -> Self {
        Self {
            dynamic_lambda_active: value.dynamic_lambda_active,
            finalized_period: value.finalized_period,
            finalized_round: value.finalized_round,
            pre_adjust_rounds_count_dynamic_lambda: value.pre_adjust_rounds_count_dynamic_lambda,
            pre_adjust_dynamic_lambda: value.pre_adjust_dynamic_lambda,
            config: value.config.into(),
        }
    }
}

impl From<PbftFinalizationCleanupIntent> for FfiPbftFinalizationCleanupPlan {
    fn from(value: PbftFinalizationCleanupIntent) -> Self {
        Self {
            persist_pbft_block_metadata: value.persist_pbft_block_metadata,
            reset_reward_votes: value.reset_reward_votes,
            set_dag_block_order: value.set_dag_block_order,
            update_sortition_params: value.update_sortition_params,
            update_finalized_transactions_status: value.update_finalized_transactions_status,
            update_pbft_chain: value.update_pbft_chain,
            clear_anchor_dag_cache: value.clear_anchor_dag_cache,
            finalize_final_chain: value.finalize_final_chain,
            maybe_update_dynamic_lambda: value.maybe_update_dynamic_lambda,
            advance_period: value.advance_period,
        }
    }
}

impl From<&FfiPbftFinalizationResumePlan> for PbftFinalizationResumePlan {
    fn from(value: &FfiPbftFinalizationResumePlan) -> Self {
        Self {
            status: match value.status {
                0 => rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::NotPersisted,
                1 => rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::Complete,
                2 => {
                    rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::NeedsFinalChainReplay
                }
                3 => {
                    rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::NeedsExecutedStatusPersistence
                }
                4 => {
                    rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::MissingPrimaryFacts
                }
                5 => {
                    rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::ConflictingPrimaryFacts
                }
                6 => {
                    rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::NeedsDynamicLambdaPersistence
                }
                255 => rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::ContractError,
                _ => rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::Unknown,
            },
            duplicate_classified: value.duplicate_classified,
            complete: value.complete,
            replay_actions: value
                .replay_actions
                .iter()
                .filter_map(|action| PbftFinalizationRuntimeAction::from_u8(*action))
                .collect(),
            error_code: value.error_code.to_string(),
        }
    }
}

impl From<PbftFinalizationResumePlan> for FfiPbftFinalizationResumePlan {
    fn from(value: PbftFinalizationResumePlan) -> Self {
        Self {
            status: value.status.as_u8(),
            duplicate_classified: value.duplicate_classified,
            complete: value.complete,
            replay_actions: value
                .replay_actions
                .into_iter()
                .map(PbftFinalizationRuntimeAction::as_u8)
                .collect(),
            error_code: value.error_code,
        }
    }
}

impl From<&FfiPbftFinalizationCleanupPlan> for PbftFinalizationCleanupIntent {
    fn from(value: &FfiPbftFinalizationCleanupPlan) -> Self {
        Self {
            persist_pbft_block_metadata: value.persist_pbft_block_metadata,
            reset_reward_votes: value.reset_reward_votes,
            set_dag_block_order: value.set_dag_block_order,
            update_sortition_params: value.update_sortition_params,
            update_finalized_transactions_status: value.update_finalized_transactions_status,
            update_pbft_chain: value.update_pbft_chain,
            clear_anchor_dag_cache: value.clear_anchor_dag_cache,
            finalize_final_chain: value.finalize_final_chain,
            maybe_update_dynamic_lambda: value.maybe_update_dynamic_lambda,
            advance_period: value.advance_period,
        }
    }
}

impl From<PbftFinalizationStorageWriteIntent> for FfiPbftFinalizationStorageWritePlan {
    fn from(value: PbftFinalizationStorageWriteIntent) -> Self {
        Self {
            persist_pbft_head: value.persist_pbft_head,
            persist_period_data: value.persist_period_data,
            reset_reward_votes: value.reset_reward_votes,
            update_sortition_params: value.update_sortition_params,
            apply_dynamic_lambda_update: value.apply_dynamic_lambda_update,
            persist_period_lambda: value.persist_period_lambda,
            persist_executed_pbft_status: value.persist_executed_pbft_status,
            pbft_block_hash: value.pbft_block_hash.0,
            pbft_head_hash: value.pbft_head_hash.0,
            block_period: value.block_period,
            null_anchor: value.null_anchor,
            anchor_hash: value.anchor_hash.0,
            reward_vote_period: value.reward_vote_period,
            reward_vote_round: value.reward_vote_round,
            reward_vote_step: value.reward_vote_step,
            reward_vote_block_hash: value.reward_vote_block_hash.0,
            period_lambda: value.period_lambda,
            blocks_per_year: value.blocks_per_year,
            executed_pbft_status: value.executed_pbft_status,
            pbft_head_payload: value.pbft_head_payload,
            period_data_rlp: value.period_data_rlp,
            dag_block_period_writes: value
                .dag_block_period_writes
                .into_iter()
                .map(Into::into)
                .collect(),
            transaction_location_writes: value
                .transaction_location_writes
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

impl From<&FfiPbftFinalizationStorageWritePlan> for PbftFinalizationStorageWriteIntent {
    fn from(value: &FfiPbftFinalizationStorageWritePlan) -> Self {
        Self {
            persist_pbft_head: value.persist_pbft_head,
            persist_period_data: value.persist_period_data,
            reset_reward_votes: value.reset_reward_votes,
            update_sortition_params: value.update_sortition_params,
            apply_dynamic_lambda_update: value.apply_dynamic_lambda_update,
            persist_period_lambda: value.persist_period_lambda,
            persist_executed_pbft_status: value.persist_executed_pbft_status,
            pbft_block_hash: H256::from(value.pbft_block_hash),
            pbft_head_hash: H256::from(value.pbft_head_hash),
            block_period: value.block_period,
            null_anchor: value.null_anchor,
            anchor_hash: H256::from(value.anchor_hash),
            reward_vote_period: value.reward_vote_period,
            reward_vote_round: value.reward_vote_round,
            reward_vote_step: value.reward_vote_step,
            reward_vote_block_hash: H256::from(value.reward_vote_block_hash),
            period_lambda: value.period_lambda,
            blocks_per_year: value.blocks_per_year,
            executed_pbft_status: value.executed_pbft_status,
            pbft_head_payload: value.pbft_head_payload.clone(),
            period_data_rlp: value.period_data_rlp.clone(),
            dag_block_period_writes: value
                .dag_block_period_writes
                .iter()
                .map(|hash| PbftFinalizationPositionedHash {
                    hash: H256::from(hash.hash),
                    position: hash.position,
                })
                .collect(),
            transaction_location_writes: value
                .transaction_location_writes
                .iter()
                .map(|hash| PbftFinalizationPositionedHash {
                    hash: H256::from(hash.hash),
                    position: hash.position,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
fn block_position_rlp(period: u64, position: u32) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(2);
    stream.append(&period);
    stream.append(&position);
    stream.out().to_vec()
}

#[cfg(test)]
fn check_existing_value(
    storage: &BridgeStorage,
    column: Column,
    key: &[u8],
    expected: &[u8],
    error_code: &str,
) -> Result<bool> {
    if let Some(existing) = storage.0.get_raw(column, key)? {
        if existing != expected {
            let _ = error_code;
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
fn apply_result(
    status: u8,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    dag_index_writes: usize,
    transaction_location_writes: usize,
    error_code: &str,
) -> FfiPbftFinalizedPeriodApplyResult {
    FfiPbftFinalizedPeriodApplyResult {
        status,
        wrote_pbft_head: status != APPLY_STATUS_REJECTED_WRITE_SET
            && status != APPLY_STATUS_MISSING_REQUIRED_PAYLOAD
            && status != APPLY_STATUS_CONFLICTING_EXISTING_WRITE
            && write_set.persist_pbft_head,
        wrote_period_data: status != APPLY_STATUS_REJECTED_WRITE_SET
            && status != APPLY_STATUS_MISSING_REQUIRED_PAYLOAD
            && status != APPLY_STATUS_CONFLICTING_EXISTING_WRITE
            && write_set.persist_period_data,
        dag_index_writes,
        transaction_location_writes,
        block_period: write_set.block_period,
        pbft_block_hash: write_set.pbft_block_hash,
        error_code: error_code.to_string(),
    }
}

fn sidecar_apply_result(
    status: u8,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    error_code: &str,
) -> FfiPbftFinalizedPeriodApplyResult {
    FfiPbftFinalizedPeriodApplyResult {
        status,
        wrote_pbft_head: false,
        wrote_period_data: false,
        dag_index_writes: 0,
        transaction_location_writes: 0,
        block_period: write_set.block_period,
        pbft_block_hash: write_set.pbft_block_hash,
        error_code: error_code.to_string(),
    }
}

impl From<PbftFinalizationPositionedHash> for FfiPbftFinalizationPositionedHash {
    fn from(value: PbftFinalizationPositionedHash) -> Self {
        Self {
            hash: value.hash.0,
            position: value.position,
        }
    }
}

impl From<PbftFinalizationPlan> for FfiPbftFinalizationIntentPlan {
    fn from(value: PbftFinalizationPlan) -> Self {
        Self {
            finalize_block: value.finalize_block,
            anchor: value.anchor.as_u8(),
            executed_pbft_block: value.executed_pbft_block,
            status: value.status.as_u8(),
            cleanup: value.cleanup.into(),
            storage_write_intent: value.storage_write_intent.into(),
        }
    }
}

impl From<&FfiPbftFinalizationIntentPlan> for PbftFinalizationPlan {
    fn from(value: &FfiPbftFinalizationIntentPlan) -> Self {
        Self {
            finalize_block: value.finalize_block,
            anchor: PbftFinalizationAnchor::from_u8(value.anchor),
            executed_pbft_block: value.executed_pbft_block,
            cleanup: (&value.cleanup).into(),
            storage_write_intent: (&value.storage_write_intent).into(),
            status: PbftFinalizationStatus::from_u8(value.status),
        }
    }
}

impl From<PbftDynamicLambdaPlan> for FfiPbftDynamicLambdaPlan {
    fn from(value: PbftDynamicLambdaPlan) -> Self {
        let error_code = if value.status == PbftFinalizationStatus::ContractError {
            "PBFT_DYNAMIC_LAMBDA_CONTRACT_ERROR".to_string()
        } else {
            String::new()
        };
        Self {
            apply_dynamic_lambda_update: value.apply_dynamic_lambda_update,
            period_lambda: value.period_lambda,
            blocks_per_year: value.blocks_per_year,
            rounds_count_dynamic_lambda: value.rounds_count_dynamic_lambda,
            dynamic_lambda: value.dynamic_lambda,
            decreased_dynamic_lambda: value.decreased_dynamic_lambda,
            increased_dynamic_lambda: value.increased_dynamic_lambda,
            status: value.status.as_u8(),
            error_code,
        }
    }
}

impl From<rustaxa_consensus::pbft_finalize::PbftFinalizationRuntimePlan>
    for FfiPbftFinalizationRuntimePlan
{
    fn from(value: rustaxa_consensus::pbft_finalize::PbftFinalizationRuntimePlan) -> Self {
        Self {
            finalize_block: value.finalize_block,
            status: value.status.as_u8(),
            actions: value
                .actions
                .into_iter()
                .map(PbftFinalizationRuntimeAction::as_u8)
                .collect(),
            error_code: String::new(),
        }
    }
}

impl From<rustaxa_consensus::pbft_finalize::PbftFinalizationRuntimeStep>
    for FfiPbftFinalizationRuntimeSessionStep
{
    fn from(value: rustaxa_consensus::pbft_finalize::PbftFinalizationRuntimeStep) -> Self {
        let status = value.runtime_status.as_u8();
        Self {
            status,
            cursor: value.action_index,
            action: value
                .action
                .map(PbftFinalizationRuntimeAction::as_u8)
                .unwrap_or(RUNTIME_NO_ACTION),
            has_action: value.has_action,
            complete: value.complete,
            can_continue: status == RUNTIME_STATUS_ACTIVE || status == RUNTIME_STATUS_COMPLETE,
            error_code: value.error_code,
        }
    }
}

impl From<FfiPbftFinalizationLiveMutationReport> for PbftFinalizationLiveMutationReport {
    fn from(value: FfiPbftFinalizationLiveMutationReport) -> Self {
        Self {
            action: PbftFinalizationRuntimeAction::from_u8(value.action)
                .unwrap_or(PbftFinalizationRuntimeAction::Complete),
            block_period: value.block_period,
            pbft_block_hash: H256::from(value.pbft_block_hash),
            anchor_hash: H256::from(value.anchor_hash),
            dag_finalized_count: value.dag_finalized_count,
            finalized_transaction_count: value.finalized_transaction_count,
            pbft_chain_size: value.pbft_chain_size,
            pbft_chain_head_hash: H256::from(value.pbft_chain_head_hash),
            pbft_chain_last_anchor_hash: H256::from(value.pbft_chain_last_anchor_hash),
            reward_votes_period: value.reward_votes_period,
            reward_votes_round: value.reward_votes_round,
            reward_votes_block_hash: H256::from(value.reward_votes_block_hash),
            reward_votes_extra_count: value.reward_votes_extra_count,
            sortition_changed: value.sortition_changed,
            sortition_change_period: value.sortition_change_period,
            sortition_change_interval_efficiency: value.sortition_change_interval_efficiency,
            sortition_change_threshold_upper: value.sortition_change_threshold_upper,
            sortition_current_threshold_upper: value.sortition_current_threshold_upper,
            sortition_params_changes_count: value.sortition_params_changes_count,
        }
    }
}

impl From<PbftFinalizationLiveMutationValidation> for FfiPbftFinalizationLiveMutationValidation {
    fn from(value: PbftFinalizationLiveMutationValidation) -> Self {
        Self {
            accepted: value.accepted,
            status: value.status.as_u8(),
            action: value.action.as_u8(),
            error_code: value.error_code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::PbftFinalizationHash as FfiPbftFinalizationHash;
    use crate::storage::create_storage;
    use rustaxa_consensus::pbft_finalize::PbftFinalizationAnchor::{Anchored, Null};
    use rustaxa_consensus::pbft_finalize::PbftFinalizationStatus;
    use rustaxa_consensus::sortition::SortitionParamsChange;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const APPLY_STATUS_APPLIED_TEST: u8 = 0;
    const APPLY_STATUS_ALREADY_APPLIED_TEST: u8 = 1;
    const APPLY_STATUS_REJECTED_TEST: u8 = 2;
    const APPLY_STATUS_MISSING_PAYLOAD_TEST: u8 = 3;
    const APPLY_STATUS_CONFLICT_TEST: u8 = 4;
    const EXECUTED_BLOCK_STATUS_FIELD: u8 = 0;
    const LIVE_STATUS_ACCEPTED_TEST: u8 = 0;
    const LIVE_STATUS_TRANSACTION_COUNT_MISMATCH_TEST: u8 = 8;

    fn fact() -> FfiPbftFinalizationIntentFact {
        FfiPbftFinalizationIntentFact {
            block_hash: [7; 32],
            pbft_head_hash: [8; 32],
            block_period: 10,
            block_prev_hash: [3; 32],
            chain_last_hash: [3; 32],
            chain_last_period: 9,
            block_in_chain: false,
            pivot_dag_anchor_hash: [4; 32],
            has_pillar_block: false,
            pillar_block_finalized: false,
            request_dynamic_lambda_update: true,
            cert_vote_count: 3,
            sample_cert_vote_block_hash: [7; 32],
            sample_cert_vote_period: 10,
            sample_cert_vote_round: 2,
            sample_cert_vote_step: 5,
            block_lambda: 1_500,
            last_saved_period_lambda_found: false,
            last_saved_period_lambda: 0,
            dynamic_blocks_per_year: 1_000,
            dpos_blocks_per_year: 500,
            pbft_head_payload: br#"{"last":true}"#.to_vec(),
            period_data_rlp: vec![0xc0],
            ordered_dag_block_hashes: vec![
                FfiPbftFinalizationHash { hash: [1; 32] },
                FfiPbftFinalizationHash { hash: [2; 32] },
            ],
            ordered_transaction_hashes: vec![FfiPbftFinalizationHash { hash: [3; 32] }],
        }
    }

    fn dynamic_lambda_fact() -> FfiPbftDynamicLambdaFact {
        FfiPbftDynamicLambdaFact {
            dynamic_lambda_active: true,
            finalized_period: 20,
            finalized_round: 1,
            pre_adjust_rounds_count_dynamic_lambda: 9,
            pre_adjust_dynamic_lambda: 1_500,
            config: FfiPbftDynamicLambdaConfig {
                cacti_block_num: 10,
                lambda_min: 500,
                lambda_max: 1_500,
                lambda_default: 2_000,
                lambda_change_interval: 10,
                lambda_change: 10,
                consensus_delay: 400,
                dpos_blocks_per_year: 500,
            },
        }
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn reward_vote_bundle_rlp(raw_votes: Vec<Vec<u8>>) -> Vec<u8> {
        let mut stream = rlp::RlpStream::new_list(raw_votes.len());
        for vote in raw_votes {
            stream.append(&vote);
        }
        stream.out().to_vec()
    }

    fn sortition_stage(period: u64) -> FfiPbftFinalizationStorageWriteStage {
        FfiPbftFinalizationStorageWriteStage {
            stage: APPEND_STAGE_SORTITION_PARAMS_CHANGE,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            has_sortition_params_change: true,
            sortition_params_change_period: period,
            sortition_params_change_interval_efficiency: 2_500,
            sortition_params_change_threshold_upper: 1_300,
            has_reward_votes_reset: false,
            reward_votes_bundle_rlp: Vec::new(),
            extra_reward_vote_hashes: Vec::new(),
        }
    }

    fn reward_reset_stage(
        bundle: Vec<u8>,
        extra_hash: [u8; 32],
    ) -> FfiPbftFinalizationStorageWriteStage {
        FfiPbftFinalizationStorageWriteStage {
            stage: APPEND_STAGE_REWARD_VOTES_RESET,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            has_sortition_params_change: false,
            sortition_params_change_period: 0,
            sortition_params_change_interval_efficiency: 0,
            sortition_params_change_threshold_upper: 0,
            has_reward_votes_reset: true,
            reward_votes_bundle_rlp: bundle,
            extra_reward_vote_hashes: vec![FfiPbftFinalizationHash { hash: extra_hash }],
        }
    }

    #[test]
    fn bridge_bridge_accepts_anchored_block_and_maps_cleanup_intent() {
        let plan = plan_pbft_finalization_intent(fact());

        assert!(plan.finalize_block);
        assert_eq!(plan.anchor, Anchored.as_u8());
        assert_eq!(plan.status, PbftFinalizationStatus::Accepted.as_u8());
        assert!(plan.executed_pbft_block);
        assert!(plan.cleanup.persist_pbft_block_metadata);
        assert!(plan.cleanup.update_sortition_params);
        assert!(plan.cleanup.set_dag_block_order);
        assert!(plan.storage_write_intent.persist_pbft_head);
        assert!(plan.storage_write_intent.persist_period_data);
        assert!(plan.storage_write_intent.reset_reward_votes);
        assert!(plan.storage_write_intent.update_sortition_params);
        assert!(plan.storage_write_intent.apply_dynamic_lambda_update);
        assert!(plan.storage_write_intent.persist_period_lambda);
        assert!(plan.storage_write_intent.persist_executed_pbft_status);
        assert_eq!(plan.storage_write_intent.pbft_block_hash, [7; 32]);
        assert_eq!(plan.storage_write_intent.pbft_head_hash, [8; 32]);
        assert_eq!(plan.storage_write_intent.anchor_hash, [4; 32]);
        assert_eq!(plan.storage_write_intent.reward_vote_block_hash, [7; 32]);
        assert_eq!(plan.storage_write_intent.period_lambda, 1_500);
        assert_eq!(plan.storage_write_intent.blocks_per_year, 1_000);
        assert_eq!(
            plan.storage_write_intent.pbft_head_payload,
            br#"{"last":true}"#.to_vec()
        );
        assert_eq!(plan.storage_write_intent.period_data_rlp, vec![0xc0]);
        assert_eq!(plan.storage_write_intent.dag_block_period_writes.len(), 2);
        assert_eq!(
            plan.storage_write_intent.dag_block_period_writes[1].position,
            1
        );
        assert_eq!(
            plan.storage_write_intent.transaction_location_writes.len(),
            1
        );
        assert_eq!(
            plan.storage_write_intent.transaction_location_writes[0].hash,
            [3; 32]
        );
    }

    #[test]
    fn bridge_maps_anchor_and_status_for_null_and_rejects() {
        let mut rejected = fact();
        rejected.pivot_dag_anchor_hash = [0; 32];
        rejected.has_pillar_block = true;
        rejected.pillar_block_finalized = false;

        let rejected_plan = plan_pbft_finalization_intent(rejected);
        assert!(!rejected_plan.finalize_block);
        assert_eq!(rejected_plan.anchor, Null.as_u8());
        assert_eq!(
            rejected_plan.status,
            PbftFinalizationStatus::PillarDependencyMissing.as_u8()
        );
    }

    #[test]
    fn runtime_planner_maps_ordered_finalization_actions() {
        let plan = plan_pbft_finalization_intent(fact());

        let runtime = plan_pbft_finalization_runtime(&plan);

        assert!(runtime.finalize_block);
        assert_eq!(runtime.status, PbftFinalizationStatus::Accepted.as_u8());
        assert_eq!(
            runtime.actions,
            vec![0, 14, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
        assert!(runtime.error_code.is_empty());
    }

    #[test]
    fn runtime_session_tracks_cursor_and_completion_for_bridge() {
        let plan = plan_pbft_finalization_intent(fact());
        let mut session = create_pbft_finalization_runtime_session(&plan);

        let step = pbft_finalization_runtime_session_next(&mut session);
        assert_eq!(step.status, RUNTIME_STATUS_ACTIVE);
        assert!(step.has_action);
        assert_eq!(step.cursor, 0);
        assert_eq!(step.action, 0);
        assert!(!step.complete);

        let step = pbft_finalization_runtime_session_report(&mut session, 0, 0, true, 0);
        assert_eq!(step.status, RUNTIME_STATUS_ACTIVE);
        assert_eq!(step.cursor, 1);
        assert_eq!(step.action, 14);

        let mut cursor = step.cursor;
        let mut action = step.action;
        loop {
            let next =
                pbft_finalization_runtime_session_report(&mut session, cursor, action, true, 0);
            if next.complete {
                assert_eq!(next.status, RUNTIME_STATUS_COMPLETE);
                assert!(!next.has_action);
                break;
            }
            cursor = next.cursor;
            action = next.action;
        }
    }

    #[test]
    fn runtime_session_stops_on_failed_or_mismatched_report() {
        let plan = plan_pbft_finalization_intent(fact());
        let mut session = create_pbft_finalization_runtime_session(&plan);

        let failed = pbft_finalization_runtime_session_report(&mut session, 0, 0, false, 77);
        assert_eq!(failed.status, 4);
        assert!(!failed.has_action);
        assert_eq!(failed.cursor, 0);
        assert_eq!(failed.error_code, "PBFT_FINALIZE_RUNTIME_ACTION_STATUS_77");

        let mut session = create_pbft_finalization_runtime_session(&plan);
        let mismatch = pbft_finalization_runtime_session_report(&mut session, 1, 0, true, 0);
        assert_eq!(mismatch.status, 3);
        assert!(!mismatch.has_action);
        assert_eq!(mismatch.error_code, "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH");
    }

    #[test]
    fn runtime_session_preserves_structured_action_error_codes() {
        let plan = plan_pbft_finalization_intent(fact());
        let mut session = create_pbft_finalization_runtime_session(&plan);

        let failed = pbft_finalization_runtime_session_report_action(
            &mut session,
            FfiPbftFinalizationRuntimeActionReport {
                cursor: 0,
                action: 0,
                success: false,
                status: 7,
                error_code: "PBFT_FINALIZE_DAG_ORDER_APPLY_FAILED".to_string(),
            },
        );

        assert_eq!(failed.status, 4);
        assert_eq!(failed.error_code, "PBFT_FINALIZE_DAG_ORDER_APPLY_FAILED");
    }

    #[test]
    fn live_mutation_validation_maps_bridge_reports() {
        let plan = plan_pbft_finalization_intent(fact());

        let accepted = validate_pbft_finalization_live_mutation_report(
            &plan,
            FfiPbftFinalizationLiveMutationReport {
                action: 5,
                block_period: 10,
                pbft_block_hash: [7; 32],
                anchor_hash: [4; 32],
                dag_finalized_count: 0,
                finalized_transaction_count: 1,
                pbft_chain_size: 0,
                pbft_chain_head_hash: [0; 32],
                pbft_chain_last_anchor_hash: [0; 32],
                reward_votes_period: 10,
                reward_votes_round: 2,
                reward_votes_block_hash: [7; 32],
                reward_votes_extra_count: 0,
                sortition_changed: false,
                sortition_change_period: 0,
                sortition_change_interval_efficiency: 0,
                sortition_change_threshold_upper: 0,
                sortition_current_threshold_upper: 0,
                sortition_params_changes_count: 0,
            },
        );
        assert!(accepted.accepted);
        assert_eq!(accepted.status, LIVE_STATUS_ACCEPTED_TEST);

        let rejected = validate_pbft_finalization_live_mutation_report(
            &plan,
            FfiPbftFinalizationLiveMutationReport {
                action: 5,
                block_period: 10,
                pbft_block_hash: [7; 32],
                anchor_hash: [4; 32],
                dag_finalized_count: 0,
                finalized_transaction_count: 0,
                pbft_chain_size: 0,
                pbft_chain_head_hash: [0; 32],
                pbft_chain_last_anchor_hash: [0; 32],
                reward_votes_period: 10,
                reward_votes_round: 2,
                reward_votes_block_hash: [7; 32],
                reward_votes_extra_count: 0,
                sortition_changed: false,
                sortition_change_period: 0,
                sortition_change_interval_efficiency: 0,
                sortition_change_threshold_upper: 0,
                sortition_current_threshold_upper: 0,
                sortition_params_changes_count: 0,
            },
        );
        assert!(!rejected.accepted);
        assert_eq!(rejected.status, LIVE_STATUS_TRANSACTION_COUNT_MISMATCH_TEST);

        let reward_rejected = validate_pbft_finalization_live_mutation_report(
            &plan,
            FfiPbftFinalizationLiveMutationReport {
                action: 3,
                block_period: 10,
                pbft_block_hash: [7; 32],
                anchor_hash: [4; 32],
                dag_finalized_count: 0,
                finalized_transaction_count: 0,
                pbft_chain_size: 0,
                pbft_chain_head_hash: [0; 32],
                pbft_chain_last_anchor_hash: [0; 32],
                reward_votes_period: 10,
                reward_votes_round: 2,
                reward_votes_block_hash: [7; 32],
                reward_votes_extra_count: 1,
                sortition_changed: false,
                sortition_change_period: 0,
                sortition_change_interval_efficiency: 0,
                sortition_change_threshold_upper: 0,
                sortition_current_threshold_upper: 0,
                sortition_params_changes_count: 0,
            },
        );
        assert!(!reward_rejected.accepted);
        assert_eq!(reward_rejected.status, 12);
    }

    #[test]
    fn resume_runtime_session_drives_storage_derived_tail_actions() {
        let resume = FfiPbftFinalizationResumePlan {
            status: 2,
            duplicate_classified: true,
            complete: false,
            replay_actions: vec![9, 10, 11, 12],
            error_code: "PBFT_FINALIZE_RESUME_NEEDS_FINAL_CHAIN_REPLAY".to_string(),
        };
        let mut session = create_pbft_finalization_resume_runtime_session(&resume);

        let mut step = pbft_finalization_runtime_session_next(&mut session);
        let mut actions = Vec::new();
        while step.has_action {
            actions.push(step.action);
            step = pbft_finalization_runtime_session_report(
                &mut session,
                step.cursor,
                step.action,
                true,
                0,
            );
        }

        assert_eq!(actions, vec![9, 10, 11, 12]);
        assert!(step.complete);
        assert_eq!(step.status, RUNTIME_STATUS_COMPLETE);
    }

    #[test]
    fn dynamic_lambda_planner_maps_next_state_for_bridge() {
        let plan = plan_pbft_dynamic_lambda(dynamic_lambda_fact());

        assert_eq!(plan.status, PbftFinalizationStatus::Accepted.as_u8());
        assert!(plan.error_code.is_empty());
        assert!(plan.apply_dynamic_lambda_update);
        assert_eq!(plan.period_lambda, 1_500);
        assert_eq!(plan.blocks_per_year, 9_275_294);
        assert_eq!(plan.rounds_count_dynamic_lambda, 0);
        assert_eq!(plan.dynamic_lambda, 1_490);
        assert!(plan.decreased_dynamic_lambda);
        assert!(!plan.increased_dynamic_lambda);

        let mut disabled = dynamic_lambda_fact();
        disabled.dynamic_lambda_active = false;
        let disabled_plan = plan_pbft_dynamic_lambda(disabled);
        assert_eq!(
            disabled_plan.status,
            PbftFinalizationStatus::Accepted.as_u8()
        );
        assert!(!disabled_plan.apply_dynamic_lambda_update);
        assert_eq!(disabled_plan.blocks_per_year, 500);
        assert_eq!(disabled_plan.dynamic_lambda, 1_500);
    }

    #[test]
    fn appends_finalized_period_storage_writes_to_existing_batch() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_finalization_apply");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");

            let mut seed = storage.0.create_write_batch();
            storage
                .0
                .batch_put_raw(&mut seed, Column::DagBlocks, &[2u8; 32], &[0xDA])
                .expect("pending DAG block should seed");
            storage
                .0
                .batch_put_raw(&mut seed, Column::Transactions, &[3u8; 32], &[0xD0])
                .expect("pending transaction should seed");
            storage
                .0
                .commit_write_batch_with_sync(seed, false)
                .expect("seed batch should commit");

            let plan = plan_pbft_finalization_intent(fact());
            let batch_id = storage
                .create_write_batch()
                .expect("bridge batch should be created");
            let result = append_pbft_finalized_period_storage_writes(
                &storage,
                batch_id,
                &plan.storage_write_intent,
            )
            .expect("append should succeed");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(result.wrote_pbft_head);
            assert!(result.wrote_period_data);
            assert_eq!(result.dag_index_writes, 2);
            assert_eq!(result.transaction_location_writes, 1);
            storage
                .commit_write_batch(batch_id, false)
                .expect("append batch should commit");

            assert_eq!(
                storage
                    .get_pbft_head(&[8; 32])
                    .expect("pbft head should load"),
                br#"{"last":true}"#.to_vec()
            );
            assert_eq!(
                storage
                    .get_period_data_raw(10)
                    .expect("period data should load"),
                vec![0xc0]
            );
            assert!(storage
                .0
                .get_raw(Column::DagBlocks, &[2; 32])
                .expect("pending DAG row lookup should succeed")
                .is_none());
            assert!(storage
                .get_transaction(&[3; 32])
                .expect("pending transaction row should be deleted")
                .is_empty());
            assert_eq!(
                storage
                    .get_dag_block_period_lookup(&[2; 32])
                    .expect("DAG period lookup should load")
                    .position,
                1
            );
            assert!(!storage
                .get_transaction_location(&[3; 32])
                .expect("transaction location should load")
                .is_empty());
            assert!(
                !storage
                    .get_period_lambda(10, false)
                    .expect("period lambda should remain sidecar-owned")
                    .found
            );
            assert!(!storage
                .get_pbft_mgr_status(EXECUTED_BLOCK_STATUS_FIELD)
                .expect("executed status should remain sidecar-owned"));

            let retry_batch = storage
                .create_write_batch()
                .expect("retry batch should be created");
            let retry_result = append_pbft_finalized_period_storage_writes(
                &storage,
                retry_batch,
                &plan.storage_write_intent,
            )
            .expect("idempotent append should succeed");
            assert_eq!(retry_result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
            storage
                .drop_write_batch(retry_batch)
                .expect("retry batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn applies_primary_reward_and_sortition_stages_in_one_rust_owned_batch() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_finalization_apply_owned_batch");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let mut seed = storage.0.create_write_batch();
            storage
                .0
                .batch_put_raw(&mut seed, Column::DagBlocks, &[1u8; 32], &[0xDA])
                .expect("pending DAG block should seed");
            storage
                .0
                .batch_put_raw(&mut seed, Column::Transactions, &[3u8; 32], &[0xD0])
                .expect("pending transaction should seed");
            storage
                .0
                .batch_put_raw(&mut seed, Column::ExtraRewardVotes, &[9u8; 32], &[0xEE])
                .expect("extra reward vote should seed");
            storage
                .0
                .commit_write_batch_with_sync(seed, false)
                .expect("seed batch should commit");

            let plan = plan_pbft_finalization_intent(fact());
            let bundle = reward_vote_bundle_rlp(vec![vec![0x01], vec![0x02]]);
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![
                    empty_stage(APPEND_STAGE_PRIMARY_FINALIZATION),
                    reward_reset_stage(bundle.clone(), [9; 32]),
                    sortition_stage(10),
                ],
                false,
            )
            .expect("Rust-owned finalization batch should apply");

            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(result.wrote_pbft_head);
            assert!(result.wrote_period_data);
            assert_eq!(result.dag_index_writes, 2);
            assert_eq!(result.transaction_location_writes, 1);
            assert_eq!(
                storage
                    .get_period_data_raw(10)
                    .expect("period data should load"),
                vec![0xc0]
            );
            assert_eq!(
                storage
                    .0
                    .get_raw(
                        Column::LatestRoundTwoTPlusOneVotes,
                        &[PBFT_TWO_T_PLUS_ONE_CERT_VOTED_TYPE],
                    )
                    .expect("reward-vote bundle lookup should succeed")
                    .expect("reward-vote bundle should exist"),
                bundle
            );
            assert!(storage
                .0
                .get_raw(Column::ExtraRewardVotes, &[9; 32])
                .expect("stale extra reward lookup should succeed")
                .is_none());
            assert!(storage
                .0
                .get_raw(Column::SortitionParamsChange, &10_u64.to_le_bytes())
                .expect("sortition change lookup should succeed")
                .is_some());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn resume_inspector_classifies_primary_finalization_crash_windows() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_finalization_resume");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());

            let missing = inspect_pbft_finalization_resume(&storage, &plan.storage_write_intent, 9)
                .expect("resume inspection should run");
            assert_eq!(missing.status, 0);
            assert!(!missing.duplicate_classified);

            let primary = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_PRIMARY_FINALIZATION)],
                false,
            )
            .expect("primary stage should apply");
            assert_eq!(primary.status, APPLY_STATUS_APPLIED_TEST);

            let needs_dynamic =
                inspect_pbft_finalization_resume(&storage, &plan.storage_write_intent, 9)
                    .expect("resume inspection should run");
            assert_eq!(needs_dynamic.status, 6);
            assert_eq!(needs_dynamic.replay_actions, vec![8]);

            let mut dynamic_stage = empty_stage(APPEND_STAGE_DYNAMIC_LAMBDA);
            dynamic_stage.rounds_count_dynamic_lambda = 7;
            dynamic_stage.dynamic_lambda = 1450;
            let dynamic = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![dynamic_stage],
                false,
            )
            .expect("dynamic stage should apply");
            assert_eq!(dynamic.status, APPLY_STATUS_APPLIED_TEST);

            let needs_final_chain =
                inspect_pbft_finalization_resume(&storage, &plan.storage_write_intent, 9)
                    .expect("resume inspection should run");
            assert_eq!(needs_final_chain.status, 2);
            assert_eq!(needs_final_chain.replay_actions, vec![9, 10, 11, 12]);

            let executed = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_EXECUTED_STATUS)],
                false,
            )
            .expect("executed stage should apply");
            assert_eq!(executed.status, APPLY_STATUS_APPLIED_TEST);

            let complete =
                inspect_pbft_finalization_resume(&storage, &plan.storage_write_intent, 10)
                    .expect("resume inspection should run");
            assert_eq!(complete.status, 1);
            assert!(complete.duplicate_classified);
            assert!(complete.complete);
            assert!(complete.replay_actions.is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn rejects_missing_or_conflicting_finalized_period_payloads() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_finalization_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let mut missing_plan = plan_pbft_finalization_intent(fact());
            missing_plan.storage_write_intent.pbft_head_payload.clear();
            let batch_id = storage
                .create_write_batch()
                .expect("bridge batch should be created");
            let result = append_pbft_finalized_period_storage_writes(
                &storage,
                batch_id,
                &missing_plan.storage_write_intent,
            )
            .expect("missing payload should return status");
            assert_eq!(result.status, APPLY_STATUS_MISSING_PAYLOAD_TEST);
            assert!(!result.wrote_pbft_head);
            storage
                .drop_write_batch(batch_id)
                .expect("missing-payload batch should drop");

            storage
                .save_pbft_block_period(&[7; 32], 99)
                .expect("conflicting PBFT block period should seed");
            let plan = plan_pbft_finalization_intent(fact());
            let batch_id = storage
                .create_write_batch()
                .expect("conflict batch should be created");
            let result = append_pbft_finalized_period_storage_writes(
                &storage,
                batch_id,
                &plan.storage_write_intent,
            )
            .expect("conflict should return status");
            assert_eq!(result.status, APPLY_STATUS_CONFLICT_TEST);
            assert_eq!(result.error_code, "PBFT_FINALIZE_CONFLICTING_PBFT_PERIOD");
            storage
                .drop_write_batch(batch_id)
                .expect("conflict batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn rejects_unknown_finalization_storage_stage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_unknown_stage");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            let batch_id = storage
                .create_write_batch()
                .expect("unknown-stage batch should be created");
            let result = append_pbft_finalization_storage_write(
                &storage,
                batch_id,
                &plan.storage_write_intent,
                FfiPbftFinalizationStorageWriteStage {
                    stage: 255,
                    rounds_count_dynamic_lambda: 0,
                    dynamic_lambda: 0,
                    has_sortition_params_change: false,
                    sortition_params_change_period: 0,
                    sortition_params_change_interval_efficiency: 0,
                    sortition_params_change_threshold_upper: 0,
                    has_reward_votes_reset: false,
                    reward_votes_bundle_rlp: Vec::new(),
                    extra_reward_vote_hashes: Vec::new(),
                },
            )
            .expect("unknown stage should return status");
            assert_eq!(result.status, APPLY_STATUS_REJECTED_TEST);
            assert_eq!(
                result.error_code,
                "PBFT_FINALIZE_UNKNOWN_STORAGE_WRITE_STAGE"
            );
            assert!(!result.wrote_pbft_head);
            assert!(!result.wrote_period_data);
            storage
                .drop_write_batch(batch_id)
                .expect("unknown-stage batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn appends_sortition_params_change_from_finalization_stage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_sortition_stage");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            let batch_id = storage
                .create_write_batch()
                .expect("sortition batch should be created");
            let result = append_pbft_finalization_storage_write(
                &storage,
                batch_id,
                &plan.storage_write_intent,
                FfiPbftFinalizationStorageWriteStage {
                    stage: APPEND_STAGE_SORTITION_PARAMS_CHANGE,
                    rounds_count_dynamic_lambda: 0,
                    dynamic_lambda: 0,
                    has_sortition_params_change: true,
                    sortition_params_change_period: 10,
                    sortition_params_change_interval_efficiency: 2_500,
                    sortition_params_change_threshold_upper: 1_300,
                    has_reward_votes_reset: false,
                    reward_votes_bundle_rlp: Vec::new(),
                    extra_reward_vote_hashes: Vec::new(),
                },
            )
            .expect("sortition stage should append");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(!result.wrote_pbft_head);
            assert!(!result.wrote_period_data);
            storage
                .commit_write_batch(batch_id, false)
                .expect("sortition batch should commit");

            let persisted = storage
                .0
                .get_raw(Column::SortitionParamsChange, &10_u64.to_le_bytes())
                .expect("sortition change lookup should succeed")
                .expect("sortition change should be written");
            let decoded = SortitionParamsChange::from_rlp_bytes(persisted.as_ref())
                .expect("sortition change should decode");
            assert_eq!(
                decoded,
                SortitionParamsChange {
                    period: 10,
                    interval_efficiency: 2_500,
                    threshold_upper: 1_300
                }
            );

            let retry_batch = storage
                .create_write_batch()
                .expect("retry sortition batch should be created");
            let retry_result = append_pbft_finalization_storage_write(
                &storage,
                retry_batch,
                &plan.storage_write_intent,
                FfiPbftFinalizationStorageWriteStage {
                    stage: APPEND_STAGE_SORTITION_PARAMS_CHANGE,
                    rounds_count_dynamic_lambda: 0,
                    dynamic_lambda: 0,
                    has_sortition_params_change: true,
                    sortition_params_change_period: 10,
                    sortition_params_change_interval_efficiency: 2_500,
                    sortition_params_change_threshold_upper: 1_300,
                    has_reward_votes_reset: false,
                    reward_votes_bundle_rlp: Vec::new(),
                    extra_reward_vote_hashes: Vec::new(),
                },
            )
            .expect("sortition retry should return status");
            assert_eq!(retry_result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
            storage
                .drop_write_batch(retry_batch)
                .expect("retry sortition batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn rejects_missing_sortition_params_change_facts() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_sortition_stage_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            let batch_id = storage
                .create_write_batch()
                .expect("sortition reject batch should be created");
            let result = append_pbft_finalization_storage_write(
                &storage,
                batch_id,
                &plan.storage_write_intent,
                empty_stage(APPEND_STAGE_SORTITION_PARAMS_CHANGE),
            )
            .expect("missing sortition facts should return status");
            assert_eq!(result.status, APPLY_STATUS_REJECTED_TEST);
            assert_eq!(
                result.error_code,
                "PBFT_FINALIZE_MISSING_SORTITION_PARAMS_CHANGE_FACTS"
            );
            storage
                .drop_write_batch(batch_id)
                .expect("sortition reject batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn appends_dynamic_lambda_storage_writes_after_live_adjustment() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_dynamic_lambda_apply");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            let batch_id = storage
                .create_write_batch()
                .expect("dynamic-lambda batch should be created");
            let result = append_pbft_finalization_dynamic_lambda_storage_writes(
                &storage,
                batch_id,
                &plan.storage_write_intent,
                7,
                1_450,
            )
            .expect("dynamic-lambda append should succeed");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(!result.wrote_pbft_head);
            assert!(!result.wrote_period_data);
            storage
                .commit_write_batch(batch_id, false)
                .expect("dynamic-lambda batch should commit");

            let period_lambda = storage
                .get_period_lambda(10, false)
                .expect("period lambda should load");
            assert!(period_lambda.found);
            assert_eq!(period_lambda.value, 1_500);
            assert_eq!(
                storage
                    .get_rounds_count_dynamic_lambda()
                    .expect("rounds count should load"),
                7
            );
            assert_eq!(
                storage
                    .get_pbft_mgr_field(PBFT_MGR_FIELD_LAMBDA)
                    .expect("lambda field should load"),
                1_450
            );

            let retry_batch = storage
                .create_write_batch()
                .expect("retry dynamic-lambda batch should be created");
            let retry_result = append_pbft_finalization_dynamic_lambda_storage_writes(
                &storage,
                retry_batch,
                &plan.storage_write_intent,
                7,
                1_450,
            )
            .expect("dynamic-lambda retry should succeed");
            assert_eq!(retry_result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
            storage
                .drop_write_batch(retry_batch)
                .expect("retry dynamic-lambda batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn rejects_conflicting_dynamic_lambda_period_value() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_dynamic_lambda_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            storage
                .save_period_lambda(10, 1_600)
                .expect("lambda mismatch should seed");
            let lambda_batch = storage
                .create_write_batch()
                .expect("conflicting-lambda batch should be created");
            let lambda_result = append_pbft_finalization_dynamic_lambda_storage_writes(
                &storage,
                lambda_batch,
                &plan.storage_write_intent,
                7,
                1_450,
            )
            .expect("lambda mismatch should return status");
            assert_eq!(lambda_result.status, APPLY_STATUS_CONFLICT_TEST);
            assert_eq!(
                lambda_result.error_code,
                "PBFT_FINALIZE_CONFLICTING_PERIOD_LAMBDA"
            );
            storage
                .drop_write_batch(lambda_batch)
                .expect("lambda-conflict batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn appends_executed_status_after_final_chain_dispatch() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_executed_status_apply");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            storage
                .save_pbft_mgr_status(EXECUTED_BLOCK_STATUS_FIELD, false)
                .expect("previous executed status should seed");
            let status_batch = storage
                .create_write_batch()
                .expect("status overwrite batch should be created");
            let status_result = append_pbft_finalization_executed_status_storage_write(
                &storage,
                status_batch,
                &plan.storage_write_intent,
            )
            .expect("status overwrite should append");
            assert_eq!(status_result.status, APPLY_STATUS_APPLIED_TEST);
            storage
                .commit_write_batch(status_batch, false)
                .expect("status overwrite batch should commit");
            assert!(storage
                .get_pbft_mgr_status(EXECUTED_BLOCK_STATUS_FIELD)
                .expect("executed status should load"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn appends_reward_vote_reset_and_removes_extra_reward_votes() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_reward_votes_reset_apply");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            let mut seed_batch = storage.0.create_write_batch();
            storage
                .0
                .batch_put_raw(
                    &mut seed_batch,
                    Column::ExtraRewardVotes,
                    &[11u8; 32],
                    &[0xDE],
                )
                .expect("extra reward vote 1 should seed");
            storage
                .0
                .batch_put_raw(
                    &mut seed_batch,
                    Column::ExtraRewardVotes,
                    &[12u8; 32],
                    &[0xAD],
                )
                .expect("extra reward vote 2 should seed");
            storage
                .0
                .commit_write_batch_with_sync(seed_batch, false)
                .expect("seed extras should commit");

            let bundle = reward_vote_bundle_rlp(vec![vec![0x01], vec![0x02]]);
            let batch_id = storage
                .create_write_batch()
                .expect("reward-vote batch should be created");
            let result = append_pbft_finalization_storage_write(
                &storage,
                batch_id,
                &plan.storage_write_intent,
                FfiPbftFinalizationStorageWriteStage {
                    stage: APPEND_STAGE_REWARD_VOTES_RESET,
                    rounds_count_dynamic_lambda: 0,
                    dynamic_lambda: 0,
                    has_sortition_params_change: false,
                    sortition_params_change_period: 0,
                    sortition_params_change_interval_efficiency: 0,
                    sortition_params_change_threshold_upper: 0,
                    has_reward_votes_reset: true,
                    reward_votes_bundle_rlp: bundle.clone(),
                    extra_reward_vote_hashes: vec![
                        FfiPbftFinalizationHash { hash: [11; 32] },
                        FfiPbftFinalizationHash { hash: [12; 32] },
                    ],
                },
            )
            .expect("reward-vote reset stage should append");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            storage
                .commit_write_batch(batch_id, false)
                .expect("reward-vote reset batch should commit");

            assert_eq!(
                storage
                    .0
                    .get_raw(
                        Column::LatestRoundTwoTPlusOneVotes,
                        &[PBFT_TWO_T_PLUS_ONE_CERT_VOTED_TYPE],
                    )
                    .expect("reward-vote bundle should load")
                    .expect("reward-vote bundle should be persisted"),
                bundle,
            );
            assert!(storage
                .0
                .get_raw(Column::ExtraRewardVotes, &[11; 32])
                .expect("extra reward lookup should succeed")
                .is_none());
            assert!(storage
                .0
                .get_raw(Column::ExtraRewardVotes, &[12; 32])
                .expect("extra reward lookup should succeed")
                .is_none());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn reward_vote_reset_stage_is_idempotent_when_bundle_and_extras_are_already_reset() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_reward_votes_reset_idempotent");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            let bundle = reward_vote_bundle_rlp(vec![vec![0x03], vec![0x04]]);
            let mut seed_batch = storage.0.create_write_batch();
            storage
                .0
                .batch_put_raw(
                    &mut seed_batch,
                    Column::LatestRoundTwoTPlusOneVotes,
                    &[PBFT_TWO_T_PLUS_ONE_CERT_VOTED_TYPE],
                    &bundle,
                )
                .expect("reward-vote bundle should seed");
            storage
                .0
                .commit_write_batch_with_sync(seed_batch, false)
                .expect("seed batch should commit");

            let batch_id = storage
                .create_write_batch()
                .expect("idempotent reward-vote batch should be created");
            let result = append_pbft_finalization_storage_write(
                &storage,
                batch_id,
                &plan.storage_write_intent,
                FfiPbftFinalizationStorageWriteStage {
                    stage: APPEND_STAGE_REWARD_VOTES_RESET,
                    rounds_count_dynamic_lambda: 0,
                    dynamic_lambda: 0,
                    has_sortition_params_change: false,
                    sortition_params_change_period: 0,
                    sortition_params_change_interval_efficiency: 0,
                    sortition_params_change_threshold_upper: 0,
                    has_reward_votes_reset: true,
                    reward_votes_bundle_rlp: bundle,
                    extra_reward_vote_hashes: Vec::new(),
                },
            )
            .expect("idempotent reward-vote stage should return status");
            assert_eq!(result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
            assert_eq!(result.error_code, "");
            storage
                .drop_write_batch(batch_id)
                .expect("idempotent reward-vote batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn rejects_reward_vote_reset_with_invalid_payloads() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_reward_votes_reset_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());

            let batch_id = storage
                .create_write_batch()
                .expect("invalid reset batch should be created");
            let result = append_pbft_finalization_storage_write(
                &storage,
                batch_id,
                &plan.storage_write_intent,
                FfiPbftFinalizationStorageWriteStage {
                    stage: APPEND_STAGE_REWARD_VOTES_RESET,
                    rounds_count_dynamic_lambda: 0,
                    dynamic_lambda: 0,
                    has_sortition_params_change: false,
                    sortition_params_change_period: 0,
                    sortition_params_change_interval_efficiency: 0,
                    sortition_params_change_threshold_upper: 0,
                    has_reward_votes_reset: false,
                    reward_votes_bundle_rlp: vec![0x99],
                    extra_reward_vote_hashes: Vec::new(),
                },
            )
            .expect("missing reward-vote flag should return status");
            assert_eq!(result.status, APPLY_STATUS_REJECTED_TEST);
            assert_eq!(
                result.error_code,
                "PBFT_FINALIZE_MISSING_REWARD_VOTES_RESET_FACTS"
            );
            storage
                .drop_write_batch(batch_id)
                .expect("invalid reset batch should drop");
        }

        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            let empty_bundle_batch = storage
                .create_write_batch()
                .expect("empty-list reset batch should be created");
            let result = append_pbft_finalization_storage_write(
                &storage,
                empty_bundle_batch,
                &plan.storage_write_intent,
                FfiPbftFinalizationStorageWriteStage {
                    stage: APPEND_STAGE_REWARD_VOTES_RESET,
                    rounds_count_dynamic_lambda: 0,
                    dynamic_lambda: 0,
                    has_sortition_params_change: false,
                    sortition_params_change_period: 0,
                    sortition_params_change_interval_efficiency: 0,
                    sortition_params_change_threshold_upper: 0,
                    has_reward_votes_reset: true,
                    reward_votes_bundle_rlp: vec![0xc0],
                    extra_reward_vote_hashes: Vec::new(),
                },
            )
            .expect("empty reward-vote bundle should return status");
            assert_eq!(result.status, APPLY_STATUS_MISSING_PAYLOAD_TEST);
            assert_eq!(result.error_code, "PBFT_FINALIZE_REWARD_VOTES_BUNDLE_EMPTY");
            storage
                .drop_write_batch(empty_bundle_batch)
                .expect("empty-list reset batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
