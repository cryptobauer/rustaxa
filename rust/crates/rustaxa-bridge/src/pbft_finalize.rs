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
    PbftFinalizationCleanupPlan as FfiPbftFinalizationCleanupPlan,
    PbftFinalizationIntentFact as FfiPbftFinalizationIntentFact,
    PbftFinalizationIntentPlan as FfiPbftFinalizationIntentPlan,
    PbftFinalizationLiveMutationReport as FfiPbftFinalizationLiveMutationReport,
    PbftFinalizationLiveMutationValidation as FfiPbftFinalizationLiveMutationValidation,
    PbftFinalizationPillarPreflightFact as FfiPbftFinalizationPillarPreflightFact,
    PbftFinalizationPillarPreflightPlan as FfiPbftFinalizationPillarPreflightPlan,
    PbftFinalizationPillarPreflightReport as FfiPbftFinalizationPillarPreflightReport,
    PbftFinalizationPositionedHash as FfiPbftFinalizationPositionedHash,
    PbftFinalizationResumePlan as FfiPbftFinalizationResumePlan,
    PbftFinalizationRuntimePlan as FfiPbftFinalizationRuntimePlan,
    PbftFinalizationRuntimeSessionStep as FfiPbftFinalizationRuntimeSessionStep,
    PbftFinalizationStorageWritePlan as FfiPbftFinalizationStorageWritePlan,
    PbftFinalizationStorageWriteStage as FfiPbftFinalizationStorageWriteStage,
    PbftFinalizedPeriodApplyResult as FfiPbftFinalizedPeriodApplyResult,
};
use crate::ffi::BridgeStorage;
#[cfg(test)]
use crate::storage::create_period_storage_queries;
use anyhow::Result;
use ethereum_types::H256;
use rustaxa_consensus::pbft_finalize::{
    apply_pbft_finalization_storage_writes as apply_domain_pbft_finalization_storage_writes,
    plan_pbft_finalization_intent as plan_domain_pbft_finalization_intent,
    plan_pbft_finalization_pillar_preflight as plan_domain_pbft_finalization_pillar_preflight,
    plan_pbft_finalization_runtime as plan_domain_pbft_finalization_runtime,
    report_pbft_finalization_pillar_preflight as report_domain_pbft_finalization_pillar_preflight,
    validate_pbft_finalization_live_mutation_report as validate_domain_pbft_finalization_live_mutation_report,
    PbftDynamicLambdaConfig, PbftDynamicLambdaFact, PbftFinalizationAnchor,
    PbftFinalizationCleanupIntent, PbftFinalizationIntentFact, PbftFinalizationLiveMutationReport,
    PbftFinalizationLiveMutationValidation, PbftFinalizationPillarPreflightAction,
    PbftFinalizationPillarPreflightFact, PbftFinalizationPillarPreflightPlan,
    PbftFinalizationPillarPreflightReport, PbftFinalizationPillarPreflightStatus,
    PbftFinalizationPlan, PbftFinalizationPositionedHash, PbftFinalizationResumePlan,
    PbftFinalizationRuntimeAction, PbftFinalizationStatus, PbftFinalizationStorageWriteIntent,
    PbftFinalizationStorageWriteStage, PbftFinalizedPeriodApplyResult,
};
#[cfg(test)]
use rustaxa_storage::Column;

const APPLY_STATUS_REJECTED_WRITE_SET: u8 = 2;
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
const PBFT_TWO_T_PLUS_ONE_CERT_VOTED_TYPE: u8 = 1;
const RUNTIME_STATUS_ACTIVE: u8 = 0;
const RUNTIME_STATUS_COMPLETE: u8 = 1;
const RUNTIME_NO_ACTION: u8 = 255;

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

pub(crate) fn apply_result_from_domain(
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

/// C++/Rust bridge entry for pillar finalization preflight.
///
/// Inputs:
/// - `fact`: compact hardfork/extra-data facts from the PBFT manager.
///
/// Outputs:
/// - A stable preflight action. When Rust requests pillar finalization, C++ must
///   execute it and attach the returned pillar votes to period data before
///   creating the normal finalization intent.
pub fn plan_pbft_finalization_pillar_preflight(
    fact: FfiPbftFinalizationPillarPreflightFact,
) -> FfiPbftFinalizationPillarPreflightPlan {
    plan_domain_pbft_finalization_pillar_preflight(fact.into()).into()
}

/// Validates a C++ executor report for the Rust-planned pillar preflight action.
pub fn report_pbft_finalization_pillar_preflight(
    plan: &FfiPbftFinalizationPillarPreflightPlan,
    report: FfiPbftFinalizationPillarPreflightReport,
) -> FfiPbftFinalizationPillarPreflightPlan {
    report_domain_pbft_finalization_pillar_preflight(&plan.into(), report.into()).into()
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
            rounds_count_dynamic_lambda: value.rounds_count_dynamic_lambda,
            dynamic_lambda: value.dynamic_lambda,
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
            process_pillar_block_after_advance: value.process_pillar_block_after_advance,
        }
    }
}

impl From<FfiPbftFinalizationPillarPreflightFact> for PbftFinalizationPillarPreflightFact {
    fn from(value: FfiPbftFinalizationPillarPreflightFact) -> Self {
        Self {
            pbft_block_hash: H256::from(value.pbft_block_hash),
            block_period: value.block_period,
            block_in_chain: value.block_in_chain,
            pillar_finalization_required: value.pillar_finalization_required,
            has_pillar_block_hash: value.has_pillar_block_hash,
            pillar_block_hash: H256::from(value.pillar_block_hash),
            pillar_block_finalized: value.pillar_block_finalized,
        }
    }
}

impl From<&FfiPbftFinalizationPillarPreflightPlan> for PbftFinalizationPillarPreflightPlan {
    fn from(value: &FfiPbftFinalizationPillarPreflightPlan) -> Self {
        Self {
            pbft_block_hash: H256::from(value.pbft_block_hash),
            block_period: value.block_period,
            pillar_block_hash: H256::from(value.pillar_block_hash),
            action: PbftFinalizationPillarPreflightAction::from_u8(value.action)
                .unwrap_or(PbftFinalizationPillarPreflightAction::None),
            finalize_pillar_block: value.finalize_pillar_block,
            accepted: value.accepted,
            status: match value.status {
                0 => PbftFinalizationPillarPreflightStatus::Accepted,
                1 => PbftFinalizationPillarPreflightStatus::NotRequired,
                2 => PbftFinalizationPillarPreflightStatus::BlockAlreadyInChain,
                3 => PbftFinalizationPillarPreflightStatus::MissingPillarBlockHash,
                4 => PbftFinalizationPillarPreflightStatus::ActionMismatch,
                5 => PbftFinalizationPillarPreflightStatus::ActionFailed,
                6 => PbftFinalizationPillarPreflightStatus::BlockMismatch,
                7 => PbftFinalizationPillarPreflightStatus::PillarBlockMismatch,
                8 => PbftFinalizationPillarPreflightStatus::EmptyPillarVotes,
                _ => PbftFinalizationPillarPreflightStatus::ContractError,
            },
            error_code: value.error_code.to_string(),
        }
    }
}

impl From<PbftFinalizationPillarPreflightPlan> for FfiPbftFinalizationPillarPreflightPlan {
    fn from(value: PbftFinalizationPillarPreflightPlan) -> Self {
        Self {
            pbft_block_hash: value.pbft_block_hash.0,
            block_period: value.block_period,
            pillar_block_hash: value.pillar_block_hash.0,
            action: value.action.as_u8(),
            finalize_pillar_block: value.finalize_pillar_block,
            accepted: value.accepted,
            status: value.status.as_u8(),
            error_code: value.error_code,
        }
    }
}

impl From<FfiPbftFinalizationPillarPreflightReport> for PbftFinalizationPillarPreflightReport {
    fn from(value: FfiPbftFinalizationPillarPreflightReport) -> Self {
        Self {
            action: PbftFinalizationPillarPreflightAction::from_u8(value.action)
                .unwrap_or(PbftFinalizationPillarPreflightAction::None),
            success: value.success,
            status: value.status,
            error_code: value.error_code,
            block_period: value.block_period,
            pbft_block_hash: H256::from(value.pbft_block_hash),
            pillar_block_hash: H256::from(value.pillar_block_hash),
            pillar_vote_count: value.pillar_vote_count,
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
            process_pillar_block: value.process_pillar_block,
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
                7 => {
                    rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::NeedsPillarPostProcessingReplay
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
            process_pillar_block: value.process_pillar_block,
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
            process_pillar_block: value.process_pillar_block,
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
            rounds_count_dynamic_lambda: value.rounds_count_dynamic_lambda,
            dynamic_lambda: value.dynamic_lambda,
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
            process_pillar_block: value.process_pillar_block,
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
            rounds_count_dynamic_lambda: value.rounds_count_dynamic_lambda,
            dynamic_lambda: value.dynamic_lambda,
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
            rounds_count_dynamic_lambda: value.rounds_count_dynamic_lambda,
            dynamic_lambda: value.dynamic_lambda,
            executed_pbft_block: value.executed_pbft_block,
            manager_period: value.manager_period,
            pillar_processed_period: value.pillar_processed_period,
            pillar_request_period: value.pillar_request_period,
            anchor_dag_cache_count: value.anchor_dag_cache_count,
            final_chain_dispatched: value.final_chain_dispatched,
            final_chain_blocks_per_year: value.final_chain_blocks_per_year,
            final_chain_last_block: value.final_chain_last_block,
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
    use crate::ffi::{
        BridgeDagStorageQueries, BridgeMetadataStorageQueries, BridgePbftStorageQueries,
        BridgeStorage, BridgeTransactionStorageQueries,
    };
    use crate::storage::{
        create_dag_storage_queries, create_metadata_storage_queries, create_pbft_storage_queries,
        create_storage, create_transaction_storage_queries,
    };
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
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 1_490,
            dpos_blocks_per_year: 500,
            pbft_head_payload: br#"{"last":true}"#.to_vec(),
            period_data_rlp: vec![0xc0],
            ordered_dag_block_hashes: vec![
                FfiPbftFinalizationHash { hash: [1; 32] },
                FfiPbftFinalizationHash { hash: [2; 32] },
            ],
            ordered_transaction_hashes: vec![FfiPbftFinalizationHash { hash: [3; 32] }],
            process_pillar_block_after_advance: false,
        }
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn pbft_queries(storage: &BridgeStorage) -> Box<BridgePbftStorageQueries> {
        create_pbft_storage_queries(storage)
    }

    fn metadata_queries(storage: &BridgeStorage) -> Box<BridgeMetadataStorageQueries> {
        create_metadata_storage_queries(storage)
    }

    fn dag_queries(storage: &BridgeStorage) -> Box<BridgeDagStorageQueries> {
        create_dag_storage_queries(storage)
    }

    fn transaction_queries(storage: &BridgeStorage) -> Box<BridgeTransactionStorageQueries> {
        create_transaction_storage_queries(storage)
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
    fn bridge_maps_pillar_preflight_plan_and_report() {
        let plan =
            plan_pbft_finalization_pillar_preflight(FfiPbftFinalizationPillarPreflightFact {
                pbft_block_hash: [7; 32],
                block_period: 10,
                block_in_chain: false,
                pillar_finalization_required: true,
                has_pillar_block_hash: true,
                pillar_block_hash: [9; 32],
                pillar_block_finalized: false,
            });

        assert!(plan.accepted);
        assert!(plan.finalize_pillar_block);
        assert_eq!(plan.action, 1);

        let accepted = report_pbft_finalization_pillar_preflight(
            &plan,
            FfiPbftFinalizationPillarPreflightReport {
                action: 1,
                success: true,
                status: 0,
                error_code: String::new(),
                block_period: 10,
                pbft_block_hash: [7; 32],
                pillar_block_hash: [9; 32],
                pillar_vote_count: 2,
            },
        );

        assert!(accepted.accepted);
        assert_eq!(accepted.status, 0);
        assert!(accepted.error_code.is_empty());
    }

    #[test]
    fn runtime_planner_maps_ordered_finalization_actions() {
        let mut fact = fact();
        fact.process_pillar_block_after_advance = true;
        let plan = plan_pbft_finalization_intent(fact);

        let runtime = plan_pbft_finalization_runtime(&plan);

        assert!(runtime.finalize_block);
        assert_eq!(runtime.status, PbftFinalizationStatus::Accepted.as_u8());
        assert_eq!(
            runtime.actions,
            vec![0, 14, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 15]
        );
        assert!(runtime.error_code.is_empty());
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
                rounds_count_dynamic_lambda: 0,
                dynamic_lambda: 1_490,
                executed_pbft_block: true,
                manager_period: 11,
                pillar_processed_period: 10,
                pillar_request_period: 5,
                anchor_dag_cache_count: 0,
                final_chain_dispatched: true,
                final_chain_blocks_per_year: 500,
                final_chain_last_block: 9,
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
                rounds_count_dynamic_lambda: 0,
                dynamic_lambda: 1_490,
                executed_pbft_block: true,
                manager_period: 11,
                pillar_processed_period: 10,
                pillar_request_period: 5,
                anchor_dag_cache_count: 0,
                final_chain_dispatched: true,
                final_chain_blocks_per_year: 500,
                final_chain_last_block: 9,
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
                rounds_count_dynamic_lambda: 0,
                dynamic_lambda: 1_490,
                executed_pbft_block: true,
                manager_period: 11,
                pillar_processed_period: 10,
                pillar_request_period: 5,
                anchor_dag_cache_count: 0,
                final_chain_dispatched: true,
                final_chain_blocks_per_year: 500,
                final_chain_last_block: 9,
            },
        );
        assert!(!reward_rejected.accepted);
        assert_eq!(reward_rejected.status, 12);
    }

    #[test]
    fn applies_finalized_period_storage_writes_in_rust_owned_batch() {
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
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_PRIMARY_FINALIZATION)],
                false,
            )
            .expect("Rust-owned primary stage should apply");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(result.wrote_pbft_head);
            assert!(result.wrote_period_data);
            assert_eq!(result.dag_index_writes, 2);
            assert_eq!(result.transaction_location_writes, 1);

            assert_eq!(
                pbft_queries(&storage)
                    .get_pbft_head(&[8; 32])
                    .expect("pbft head should load"),
                br#"{"last":true}"#.to_vec()
            );
            assert_eq!(
                create_period_storage_queries(&storage)
                    .get_period_data_raw(10)
                    .expect("period data should load"),
                vec![0xc0]
            );
            assert!(storage
                .0
                .get_raw(Column::DagBlocks, &[2; 32])
                .expect("pending DAG row lookup should succeed")
                .is_none());
            assert!(transaction_queries(&storage)
                .get_transaction(&[3; 32])
                .expect("pending transaction row should be deleted")
                .is_empty());
            assert_eq!(
                dag_queries(&storage)
                    .get_dag_block_period_lookup(&[2; 32])
                    .expect("DAG period lookup should load")
                    .position,
                1
            );
            assert!(!transaction_queries(&storage)
                .get_transaction_location(&[3; 32])
                .expect("transaction location should load")
                .is_empty());
            assert!(
                !metadata_queries(&storage)
                    .get_period_lambda(10, false)
                    .expect("period lambda should remain sidecar-owned")
                    .found
            );
            assert!(!pbft_queries(&storage)
                .get_pbft_mgr_status(EXECUTED_BLOCK_STATUS_FIELD)
                .expect("executed status should remain sidecar-owned"));

            let retry_result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_PRIMARY_FINALIZATION)],
                false,
            )
            .expect("idempotent Rust-owned primary stage should apply");
            assert_eq!(retry_result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
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
                create_period_storage_queries(&storage)
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

            let write_set: PbftFinalizationStorageWriteIntent = (&plan.storage_write_intent).into();
            let missing = rustaxa_consensus::pbft_finalize::inspect_pbft_finalization_resume(
                &storage.0, &write_set, 9,
            )
            .expect("resume inspection should run");
            assert_eq!(
                missing.status,
                rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::NotPersisted
            );
            assert!(!missing.duplicate_classified);

            let primary = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_PRIMARY_FINALIZATION)],
                false,
            )
            .expect("primary stage should apply");
            assert_eq!(primary.status, APPLY_STATUS_APPLIED_TEST);

            let needs_dynamic = rustaxa_consensus::pbft_finalize::inspect_pbft_finalization_resume(
                &storage.0, &write_set, 9,
            )
            .expect("resume inspection should run");
            assert_eq!(
                needs_dynamic.status,
                rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::NeedsDynamicLambdaPersistence
            );
            assert_eq!(
                needs_dynamic.replay_actions,
                vec![
                    PbftFinalizationRuntimeAction::ApplyDynamicLambda,
                    PbftFinalizationRuntimeAction::FinalizeFinalChain,
                    PbftFinalizationRuntimeAction::PersistExecutedStatus,
                    PbftFinalizationRuntimeAction::SetExecutedFlag,
                    PbftFinalizationRuntimeAction::AdvancePeriod,
                ]
            );

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
                rustaxa_consensus::pbft_finalize::inspect_pbft_finalization_resume(
                    &storage.0, &write_set, 9,
                )
                .expect("resume inspection should run");
            assert_eq!(
                needs_final_chain.status,
                rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::NeedsFinalChainReplay
            );
            assert_eq!(
                needs_final_chain.replay_actions,
                vec![
                    PbftFinalizationRuntimeAction::FinalizeFinalChain,
                    PbftFinalizationRuntimeAction::PersistExecutedStatus,
                    PbftFinalizationRuntimeAction::SetExecutedFlag,
                    PbftFinalizationRuntimeAction::AdvancePeriod,
                ]
            );

            let executed = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_EXECUTED_STATUS)],
                false,
            )
            .expect("executed stage should apply");
            assert_eq!(executed.status, APPLY_STATUS_APPLIED_TEST);

            let complete = rustaxa_consensus::pbft_finalize::inspect_pbft_finalization_resume(
                &storage.0, &write_set, 10,
            )
            .expect("resume inspection should run");
            assert_eq!(
                complete.status,
                rustaxa_consensus::pbft_finalize::PbftFinalizationResumeStatus::Complete
            );
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
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &missing_plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_PRIMARY_FINALIZATION)],
                false,
            )
            .expect("missing payload should return status");
            assert_eq!(result.status, APPLY_STATUS_MISSING_PAYLOAD_TEST);
            assert!(!result.wrote_pbft_head);

            storage
                .0
                .period()
                .write_pbft_period(H256::from([7; 32]), 99)
                .expect("conflicting PBFT block period should seed");
            let plan = plan_pbft_finalization_intent(fact());
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_PRIMARY_FINALIZATION)],
                false,
            )
            .expect("conflict should return status");
            assert_eq!(result.status, APPLY_STATUS_CONFLICT_TEST);
            assert_eq!(result.error_code, "PBFT_FINALIZE_CONFLICTING_PBFT_PERIOD");
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
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![FfiPbftFinalizationStorageWriteStage {
                    stage: 255,
                    ..empty_stage(APPEND_STAGE_PRIMARY_FINALIZATION)
                }],
                false,
            )
            .expect("unknown stage should return status");
            assert_eq!(result.status, APPLY_STATUS_REJECTED_TEST);
            assert_eq!(
                result.error_code,
                "PBFT_FINALIZE_UNKNOWN_STORAGE_WRITE_STAGE"
            );
            assert!(!result.wrote_pbft_head);
            assert!(!result.wrote_period_data);
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
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![sortition_stage(10)],
                false,
            )
            .expect("sortition stage should apply");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(!result.wrote_pbft_head);
            assert!(!result.wrote_period_data);

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

            let retry_result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![sortition_stage(10)],
                false,
            )
            .expect("sortition retry should return status");
            assert_eq!(retry_result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
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
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_SORTITION_PARAMS_CHANGE)],
                false,
            )
            .expect("missing sortition facts should return status");
            assert_eq!(result.status, APPLY_STATUS_REJECTED_TEST);
            assert_eq!(
                result.error_code,
                "PBFT_FINALIZE_MISSING_SORTITION_PARAMS_CHANGE_FACTS"
            );
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
            let mut dynamic_stage = empty_stage(APPEND_STAGE_DYNAMIC_LAMBDA);
            dynamic_stage.rounds_count_dynamic_lambda = 7;
            dynamic_stage.dynamic_lambda = 1_450;
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![dynamic_stage],
                false,
            )
            .expect("dynamic-lambda stage should apply");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(!result.wrote_pbft_head);
            assert!(!result.wrote_period_data);

            let period_lambda = metadata_queries(&storage)
                .get_period_lambda(10, false)
                .expect("period lambda should load");
            assert!(period_lambda.found);
            assert_eq!(period_lambda.value, 1_500);
            assert_eq!(
                metadata_queries(&storage)
                    .get_rounds_count_dynamic_lambda()
                    .expect("rounds count should load"),
                7
            );
            assert_eq!(
                pbft_queries(&storage)
                    .get_pbft_mgr_field(PBFT_MGR_FIELD_LAMBDA)
                    .expect("lambda field should load"),
                1_450
            );

            let retry_result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                {
                    let mut retry_stage = empty_stage(APPEND_STAGE_DYNAMIC_LAMBDA);
                    retry_stage.rounds_count_dynamic_lambda = 7;
                    retry_stage.dynamic_lambda = 1_450;
                    vec![retry_stage]
                },
                false,
            )
            .expect("dynamic-lambda retry should succeed");
            assert_eq!(retry_result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
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
                .0
                .metadata()
                .write_period_lambda(10, 1_600)
                .expect("lambda mismatch should seed");
            let mut dynamic_stage = empty_stage(APPEND_STAGE_DYNAMIC_LAMBDA);
            dynamic_stage.rounds_count_dynamic_lambda = 7;
            dynamic_stage.dynamic_lambda = 1_450;
            let lambda_result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![dynamic_stage],
                false,
            )
            .expect("lambda mismatch should return status");
            assert_eq!(lambda_result.status, APPLY_STATUS_CONFLICT_TEST);
            assert_eq!(
                lambda_result.error_code,
                "PBFT_FINALIZE_CONFLICTING_PERIOD_LAMBDA"
            );
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
                .0
                .pbft()
                .write_manager_status(EXECUTED_BLOCK_STATUS_FIELD, false)
                .expect("previous executed status should seed");
            let status_result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![empty_stage(APPEND_STAGE_EXECUTED_STATUS)],
                false,
            )
            .expect("status overwrite should apply");
            assert_eq!(status_result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(pbft_queries(&storage)
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
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![FfiPbftFinalizationStorageWriteStage {
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
                }],
                false,
            )
            .expect("reward-vote reset stage should apply");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);

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

            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![FfiPbftFinalizationStorageWriteStage {
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
                }],
                false,
            )
            .expect("idempotent reward-vote stage should return status");
            assert_eq!(result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
            assert_eq!(result.error_code, "");
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

            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![FfiPbftFinalizationStorageWriteStage {
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
                }],
                false,
            )
            .expect("missing reward-vote flag should return status");
            assert_eq!(result.status, APPLY_STATUS_REJECTED_TEST);
            assert_eq!(
                result.error_code,
                "PBFT_FINALIZE_MISSING_REWARD_VOTES_RESET_FACTS"
            );
        }

        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            let result = apply_pbft_finalization_storage_writes(
                &storage,
                &plan.storage_write_intent,
                vec![FfiPbftFinalizationStorageWriteStage {
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
                }],
                false,
            )
            .expect("empty reward-vote bundle should return status");
            assert_eq!(result.status, APPLY_STATUS_MISSING_PAYLOAD_TEST);
            assert_eq!(result.error_code, "PBFT_FINALIZE_REWARD_VOTES_BUNDLE_EMPTY");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
