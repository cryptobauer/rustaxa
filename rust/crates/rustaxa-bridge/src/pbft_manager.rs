//! Bridge wrapper for PBFT manager daemon-tick runtime planning.
//!
//! C++ supplies the current manager state and live-shell facts for one daemon
//! tick. Rust owns the ordered action cursor for that tick, while C++ executes
//! existing manager methods and reports each action result before the session
//! advances.

use crate::dag_transaction_service::BridgeDagTransactionService;
use crate::ffi::rustaxa_ffi::{
    BlockPeriodLookup as FfiBlockPeriodLookup,
    PbftDynamicLambdaConfig as FfiPbftDynamicLambdaConfig,
    PbftDynamicLambdaFact as FfiPbftDynamicLambdaFact,
    PbftFinalizationCleanupPlan as FfiPbftFinalizationCleanupPlan,
    PbftFinalizationExecutorStartRequest as FfiPbftFinalizationExecutorStartRequest,
    PbftFinalizationHash as FfiPbftFinalizationHash,
    PbftFinalizationIntentFact as FfiPbftFinalizationIntentFact,
    PbftFinalizationIntentPlan as FfiPbftFinalizationIntentPlan,
    PbftFinalizationPositionedHash as FfiPbftFinalizationPositionedHash,
    PbftFinalizationStorageWritePlan as FfiPbftFinalizationStorageWritePlan,
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
    PbftManagerFinalizationDynamicLambdaPlan as FfiPbftManagerFinalizationDynamicLambdaPlan,
    PbftManagerFinalizationExecutorState as FfiPbftManagerFinalizationExecutorState,
    PbftManagerFinalizationWaitFact as FfiPbftManagerFinalizationWaitFact,
    PbftManagerFinalizationWaitPlan as FfiPbftManagerFinalizationWaitPlan,
    PbftManagerLeaderCandidateInputFact as FfiPbftManagerLeaderCandidateInputFact,
    PbftManagerLeaderCandidatePlan as FfiPbftManagerLeaderCandidatePlan,
    PbftManagerLeaderValidBlockCommand as FfiPbftManagerLeaderValidBlockCommand,
    PbftManagerLifecycleTransitionRequest as FfiPbftManagerLifecycleTransitionRequest,
    PbftManagerLifecycleTransitionResult as FfiPbftManagerLifecycleTransitionResult,
    PbftManagerProposalDagBlockFact as FfiPbftManagerProposalDagBlockFact,
    PbftManagerProposalDagOrderReport as FfiPbftManagerProposalDagOrderReport,
    PbftManagerProposalInitialFact as FfiPbftManagerProposalInitialFact,
    PbftManagerProposalSessionStep as FfiPbftManagerProposalSessionStep,
    PbftManagerProposalWalletFact as FfiPbftManagerProposalWalletFact,
    PbftManagerRuntimeActionReport as FfiPbftManagerRuntimeActionReport,
    PbftManagerRuntimeSessionStep as FfiPbftManagerRuntimeSessionStep,
    PbftManagerRuntimeSnapshot as FfiPbftManagerRuntimeSnapshot,
    PbftManagerRuntimeStorageApplyResult as FfiPbftManagerRuntimeStorageApplyResult,
    PbftManagerRuntimeTickFact as FfiPbftManagerRuntimeTickFact,
    PbftManagerSleepPlan as FfiPbftManagerSleepPlan,
    PbftManagerStartupReplayRangeFact as FfiPbftManagerStartupReplayRangeFact,
    PbftManagerStartupReplayRangePlan as FfiPbftManagerStartupReplayRangePlan,
    PbftManagerStateActionEffect as FfiPbftManagerStateActionEffect,
    PbftManagerStateActionEffectReport as FfiPbftManagerStateActionEffectReport,
    PbftManagerStateActionFact as FfiPbftManagerStateActionFact,
    PbftManagerStateActionSessionStep as FfiPbftManagerStateActionSessionStep,
    PbftServiceConfig as FfiPbftServiceConfig,
    PbftSyncQueueDrainReport as FfiPbftSyncQueueDrainReport,
    PbftSyncQueueDrainReportResult as FfiPbftSyncQueueDrainReportResult,
    PbftSyncQueueDrainStep as FfiPbftSyncQueueDrainStep,
    PeriodDataQueueEntryRef as FfiPeriodDataQueueEntryRef,
    PeriodDataQueuePbftVotePayload as FfiPeriodDataQueuePbftVotePayload,
    PeriodDataQueuePillarVotePayload as FfiPeriodDataQueuePillarVotePayload,
    PeriodDataQueuePopPlan as FfiPeriodDataQueuePopPlan,
    PeriodDataQueuePushOutcome as FfiPeriodDataQueuePushOutcome,
    PeriodDataQueueSnapshot as FfiPeriodDataQueueSnapshot,
    PeriodDataQueueTransactionIdentity as FfiPeriodDataQueueTransactionIdentity,
    PeriodDataQueueTransactionPayload as FfiPeriodDataQueueTransactionPayload,
    TransactionQueueAccountNonceFact as FfiTransactionQueueAccountNonceFact,
};
use crate::ffi::{BridgePbftService, BridgeStorage};
use crate::transaction_manager::bridge_to_service_account_nonce_facts;
use anyhow::anyhow;
use rustaxa_consensus::dag::dag_block_period_from_storage;
use rustaxa_consensus::pbft_chain::pbft_block_exists_in_storage;
#[cfg(test)]
use rustaxa_consensus::pbft_chain::PbftChain;
use rustaxa_consensus::pbft_finalize::{
    load_pbft_finalization_last_period_lambda as load_domain_pbft_finalization_last_period_lambda,
    plan_pbft_dynamic_lambda as plan_domain_pbft_dynamic_lambda,
    plan_pbft_finalization_intent as plan_domain_pbft_finalization_intent, PbftDynamicLambdaConfig,
    PbftDynamicLambdaFact, PbftDynamicLambdaPlan, PbftFinalizationAnchor,
    PbftFinalizationCleanupIntent, PbftFinalizationIntentFact, PbftFinalizationPlan,
    PbftFinalizationPositionedHash, PbftFinalizationRuntimeAction, PbftFinalizationStatus,
    PbftFinalizationStorageWriteIntent, PbftFinalizationStorageWriteStage,
};
use rustaxa_consensus::pbft_manager::{
    abort_pbft_manager_runtime_session as abort_domain_pbft_manager_runtime_session,
    apply_executed_block_reset_storage, apply_next_voted_status_storage,
    apply_pbft_manager_cursor_field_storage, apply_pbft_manager_transition_storage,
    create_pbft_manager_proposal_session as create_domain_pbft_manager_proposal_session,
    create_pbft_manager_runtime_session as create_domain_pbft_manager_runtime_session,
    create_pbft_manager_state_action_effect_session as create_domain_pbft_manager_state_action_effect_session,
    load_pbft_manager_startup_replay_period as load_domain_pbft_manager_startup_replay_period,
    next_pbft_manager_proposal_session as next_domain_pbft_manager_proposal_session,
    next_pbft_manager_runtime_action,
    next_pbft_manager_state_action_effect_session as next_domain_pbft_manager_state_action_effect_session,
    plan_pbft_manager_block_validation as plan_domain_pbft_manager_block_validation,
    plan_pbft_manager_broadcast as plan_domain_pbft_manager_broadcast,
    plan_pbft_manager_candidate_admission as plan_domain_pbft_manager_candidate_admission,
    plan_pbft_manager_eligible_wallet_period_wait as plan_domain_pbft_manager_eligible_wallet_period_wait,
    plan_pbft_manager_finalization_wait as plan_domain_pbft_manager_finalization_wait,
    plan_pbft_manager_leader_candidates as plan_domain_pbft_manager_leader_candidates,
    plan_pbft_manager_runtime_sleep_until_next_step as plan_domain_pbft_manager_runtime_sleep_until_next_step,
    plan_pbft_manager_startup_replay_ranges as plan_domain_pbft_manager_startup_replay_ranges,
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
    PbftManagerLeaderValidBlockCommand, PbftManagerLifecycleTransitionRequest,
    PbftManagerProposalAction, PbftManagerProposalDagBlockFact, PbftManagerProposalDagOrderReport,
    PbftManagerProposalInitialFact, PbftManagerProposalSessionStep, PbftManagerProposalStatus,
    PbftManagerProposalWalletFact, PbftManagerRuntimeAction, PbftManagerRuntimeActionReport,
    PbftManagerRuntimeActionResultCode, PbftManagerRuntimeSessionStep, PbftManagerRuntimeSnapshot,
    PbftManagerRuntimeStateCode, PbftManagerRuntimeStatus, PbftManagerRuntimeTickFact,
    PbftManagerSleepPlan, PbftManagerStartupReplayRangeFact, PbftManagerStartupReplayRangePlan,
    PbftManagerStateActionEffect, PbftManagerStateActionEffectReport,
    PbftManagerStateActionEffectResultCode, PbftManagerStateActionFact,
    PbftManagerStateActionIntent, PbftManagerStateActionSessionStatus,
    PbftManagerStateActionSessionStep, PbftManagerTransitionKind, PbftManagerTransitionStatus,
    PbftManagerTransitionStorageStatus,
};
#[cfg(test)]
use rustaxa_consensus::pbft_manager::{
    create_pbft_manager_runtime_from_storage as create_domain_pbft_manager_runtime_from_storage,
    PbftManagerStorageStartupFact,
};
use rustaxa_consensus::pbft_sync::{
    create_pbft_sync_queue_drain_session as create_domain_pbft_sync_queue_drain_session,
    next_pbft_sync_queue_drain_step as next_domain_pbft_sync_queue_drain_step,
    report_pbft_sync_queue_drain_step as report_domain_pbft_sync_queue_drain_step,
    PbftSyncQueueDrainAction, PbftSyncQueueDrainReport, PbftSyncQueueDrainReportResult,
    PbftSyncQueueDrainStatus, PbftSyncQueueDrainStep,
};
use rustaxa_consensus::pillar_chain::load_own_pillar_block_vote_storage;
use rustaxa_consensus::{PbftService, PbftServiceConfig};

impl From<crate::ffi::rustaxa_ffi::PbftFinalizationStorageWriteStage>
    for PbftFinalizationStorageWriteStage
{
    fn from(value: crate::ffi::rustaxa_ffi::PbftFinalizationStorageWriteStage) -> Self {
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
            extra_reward_vote_hashes: Vec::new(),
            has_prepared_pillar_block: value.has_prepared_pillar_block,
            prepared_pillar_block_period: value.prepared_pillar_block_period,
            prepared_pillar_block_rlp: value.prepared_pillar_block_rlp,
        }
    }
}

impl From<FfiPbftFinalizationIntentFact> for PbftFinalizationIntentFact {
    fn from(value: FfiPbftFinalizationIntentFact) -> Self {
        Self {
            block_hash: ethereum_types::H256::from(value.block_hash),
            pbft_head_hash: ethereum_types::H256::from(value.pbft_head_hash),
            block_period: value.block_period,
            block_prev_hash: ethereum_types::H256::from(value.block_prev_hash),
            chain_last_hash: ethereum_types::H256::from(value.chain_last_hash),
            chain_last_period: value.chain_last_period,
            block_in_chain: value.block_in_chain,
            pivot_dag_anchor_hash: ethereum_types::H256::from(value.pivot_dag_anchor_hash),
            has_pillar_block: value.has_pillar_block,
            pillar_block_finalized: value.pillar_block_finalized,
            request_dynamic_lambda_update: value.request_dynamic_lambda_update,
            cert_vote_count: value.cert_vote_count,
            sample_cert_vote_block_hash: ethereum_types::H256::from(
                value.sample_cert_vote_block_hash,
            ),
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
                .map(|hash| ethereum_types::H256::from(hash.hash))
                .collect(),
            ordered_transaction_hashes: value
                .ordered_transaction_hashes
                .into_iter()
                .map(|hash| ethereum_types::H256::from(hash.hash))
                .collect(),
            process_pillar_block_after_advance: value.process_pillar_block_after_advance,
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
            pbft_block_hash: ethereum_types::H256::from(value.pbft_block_hash),
            pbft_head_hash: ethereum_types::H256::from(value.pbft_head_hash),
            block_period: value.block_period,
            null_anchor: value.null_anchor,
            anchor_hash: ethereum_types::H256::from(value.anchor_hash),
            reward_vote_period: value.reward_vote_period,
            reward_vote_round: value.reward_vote_round,
            reward_vote_step: value.reward_vote_step,
            reward_vote_block_hash: ethereum_types::H256::from(value.reward_vote_block_hash),
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
                    hash: ethereum_types::H256::from(hash.hash),
                    position: hash.position,
                })
                .collect(),
            transaction_location_writes: value
                .transaction_location_writes
                .iter()
                .map(|hash| PbftFinalizationPositionedHash {
                    hash: ethereum_types::H256::from(hash.hash),
                    position: hash.position,
                })
                .collect(),
        }
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

const RUNTIME_STATUS_ACTIVE: u8 = 0;
const RUNTIME_STATUS_COMPLETE: u8 = 1;
const ACTION_NO_ACTION: u8 = 255;
const TRANSITION_STORAGE_STATUS_APPLIED: u8 = 0;
const TRANSITION_STORAGE_STATUS_REJECTED: u8 = 1;
#[cfg(test)]
const PBFT_MGR_STATUS_EXECUTED_BLOCK: u8 = 0;

/// Rust-only startup facts for tests that exercise manager restore independently
/// from the production service's chain-derived bootstrap contract.
#[cfg(test)]
pub(crate) struct TestPbftManagerStartupFact {
    pub current_period: u64,
    pub cacti_active_at_chain_size: bool,
    pub genesis_lambda_ms: u64,
    pub cacti_lambda_max_ms: u64,
    pub cacti_lambda_default_ms: u64,
    pub cacti_block: u64,
    pub max_exponential_lambda_ms: u64,
    pub max_steps: u64,
    pub deadline_ms: u64,
    pub polling_interval_ms: u64,
}

fn to_startup_u32(value: u64, field: &str) -> anyhow::Result<u32> {
    u32::try_from(value).map_err(|_| anyhow!("PBFT_MANAGER_STARTUP_{field}_OVERFLOW"))
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
pub fn create_pbft_service_from_storage(
    storage: &BridgeStorage,
    config: FfiPbftServiceConfig,
) -> anyhow::Result<Box<BridgePbftService>> {
    let config = PbftServiceConfig {
        genesis_lambda_ms: to_startup_u32(config.genesis_lambda_ms, "GENESIS_LAMBDA")?,
        cacti_lambda_max_ms: to_startup_u32(config.cacti_lambda_max_ms, "CACTI_LAMBDA_MAX")?,
        cacti_lambda_default_ms: to_startup_u32(
            config.cacti_lambda_default_ms,
            "CACTI_LAMBDA_DEFAULT",
        )?,
        cacti_block: config.cacti_block,
        max_exponential_lambda_ms: config.max_exponential_lambda_ms,
        max_steps: config.max_steps,
        deadline_ms: config.deadline_ms,
        polling_interval_ms: config.polling_interval_ms,
        report_malicious_behaviour: config.report_malicious_behaviour,
        magnolia_activation_period: config.magnolia_activation_period,
    };
    Ok(Box::new(BridgePbftService(PbftService::restore(
        storage.0.clone(),
        config,
    )?)))
}

/// Marks the typed PBFT startup replay phase complete.
///
/// The transition is one-way and uses release ordering so manager command
/// threads observe every replay mutation performed before this call. Live
/// daemon, proposal, and sync-session entry points remain fail-closed until the
/// application completes replay and calls this function.
pub fn pbft_service_complete_bootstrap(service: &BridgePbftService) -> anyhow::Result<()> {
    service.complete_bootstrap();
    Ok(())
}

#[cfg(test)]
pub fn create_pbft_manager_runtime_from_storage(
    storage: &BridgeStorage,
    fact: TestPbftManagerStartupFact,
) -> anyhow::Result<Box<BridgePbftService>> {
    let service = create_pbft_service_from_storage(
        storage,
        FfiPbftServiceConfig {
            genesis_lambda_ms: fact.genesis_lambda_ms,
            cacti_lambda_max_ms: fact.cacti_lambda_max_ms,
            cacti_lambda_default_ms: fact.cacti_lambda_default_ms,
            cacti_block: if fact.cacti_active_at_chain_size {
                0
            } else {
                u64::MAX
            },
            max_exponential_lambda_ms: fact.max_exponential_lambda_ms,
            max_steps: fact.max_steps,
            deadline_ms: fact.deadline_ms,
            polling_interval_ms: fact.polling_interval_ms,
            report_malicious_behaviour: false,
            magnolia_activation_period: 0,
        },
    )?;
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
            cacti_block: fact.cacti_block,
            max_exponential_lambda_ms: fact.max_exponential_lambda_ms,
            max_steps: fact.max_steps,
            deadline_ms: fact.deadline_ms,
            polling_interval_ms: fact.polling_interval_ms,
        },
    )?;
    service.manager_state().state = runtime;
    pbft_service_complete_bootstrap(&service)?;
    Ok(service)
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
    runtime: &BridgePbftService,
    period: u64,
    load_period_lambda: bool,
) -> anyhow::Result<crate::ffi::rustaxa_ffi::PbftManagerStartupReplayPeriod> {
    let runtime = runtime.manager_state();
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
pub fn pbft_manager_runtime_snapshot(runtime: &BridgePbftService) -> FfiPbftManagerRuntimeSnapshot {
    let runtime = runtime.manager_state();
    runtime.state.snapshot().into()
}

/// Returns the Rust-owned PBFT sync period-data queue snapshot.
///
/// Inputs:
/// - `runtime` owns the in-memory period-data queue metadata.
/// - `pbft_chain_size`, `current_period`, and `chain_last_hash` are the
///   remaining PBFT-chain compatibility facts supplied by the C++ shell.
///
/// Outputs:
/// - Returns the queue marker, syncing period, chain-link hash decision, size,
///   and empty flag in one stable DTO.
///
/// Invariants and edge behavior:
/// - The queue remains intentionally in-memory; restart behavior is unchanged
///   from the legacy non-persistent sync queue contract.
/// - C++ no longer reads individual queue internals through separate bridge
///   exports.
pub fn pbft_manager_runtime_period_data_queue_snapshot(
    runtime: &BridgePbftService,
    pbft_chain_size: u64,
    current_period: u64,
    chain_last_hash: [u8; 32],
) -> FfiPeriodDataQueueSnapshot {
    let runtime = runtime.manager_state();
    FfiPeriodDataQueueSnapshot {
        period: runtime.period_data_queue.period(),
        syncing_period: runtime.period_data_queue.syncing_period(pbft_chain_size),
        last_block_hash_or_chain: runtime
            .period_data_queue
            .last_block_hash_or_chain(current_period, ethereum_types::H256::from(chain_last_hash))
            .into(),
        size: runtime.period_data_queue.size(),
        empty: runtime.period_data_queue.is_empty(),
    }
}

/// Clears PBFT manager runtime queue metadata.
pub fn pbft_manager_runtime_period_data_queue_clear(runtime: &BridgePbftService) {
    let mut runtime = runtime.manager_state();
    runtime.period_data_queue.clear();
}

fn queue_drain_step_into_ffi(value: PbftSyncQueueDrainStep) -> FfiPbftSyncQueueDrainStep {
    FfiPbftSyncQueueDrainStep {
        action: value.action.as_u8(),
        status: value.status.as_u8(),
        clean_before_period: value.clean_before_period,
        can_continue: value.can_continue,
        error_code: value.error_code.to_string(),
    }
}

fn queue_drain_report_result_into_ffi(
    value: PbftSyncQueueDrainReportResult,
) -> FfiPbftSyncQueueDrainReportResult {
    FfiPbftSyncQueueDrainReportResult {
        status: value.status.as_u8(),
        can_continue: value.can_continue,
        error_code: value.error_code.to_string(),
    }
}

fn queue_drain_bootstrap_incomplete_step() -> FfiPbftSyncQueueDrainStep {
    FfiPbftSyncQueueDrainStep {
        action: PbftSyncQueueDrainAction::ContractError.as_u8(),
        status: PbftSyncQueueDrainStatus::InvalidReport.as_u8(),
        clean_before_period: 0,
        can_continue: false,
        error_code: "PBFT_SERVICE_BOOTSTRAP_INCOMPLETE".to_owned(),
    }
}

fn queue_drain_bootstrap_incomplete_report() -> FfiPbftSyncQueueDrainReportResult {
    FfiPbftSyncQueueDrainReportResult {
        status: PbftSyncQueueDrainStatus::InvalidReport.as_u8(),
        can_continue: false,
        error_code: "PBFT_SERVICE_BOOTSTRAP_INCOMPLETE".to_owned(),
    }
}

fn queue_drain_report_from_ffi(value: FfiPbftSyncQueueDrainReport) -> PbftSyncQueueDrainReport {
    PbftSyncQueueDrainReport {
        action: PbftSyncQueueDrainAction::from_u8(value.action),
        success: value.success,
        accepted_period_data: value.accepted_period_data,
    }
}

/// Resets the PBFT sync queue-drain planner owned by the PBFT manager runtime.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime.
///
/// Outputs:
/// - The runtime's ephemeral queue-drain session is reset to its initial state.
///
/// Invariants and edge behavior:
/// - C++ calls this once at the start of each `pushSyncedPbftBlocksIntoChain`
///   pass. The live `PeriodData` sidecars remain C++-owned for now, but the
///   planner session no longer requires a standalone CXX bridge handle.
pub fn pbft_manager_runtime_begin_pbft_sync_queue_drain(runtime: &BridgePbftService) {
    if !runtime.readiness().is_ready() {
        return;
    }
    let mut runtime = runtime.manager_state();
    runtime.pbft_sync_queue_drain_session = create_domain_pbft_sync_queue_drain_session();
}

/// Returns the next queue-drain step from the PBFT manager runtime-owned planner.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime.
/// - `queue_size`: current processable C++ sidecar queue size.
/// - `current_period`: current PBFT period used for stale queue cleanup.
///
/// Outputs:
/// - A CXX-safe queue-drain step for the C++ executor.
///
/// Invariants and edge behavior:
/// - Rust owns action ordering and report validation. C++ remains the
///   temporary executor for live sidecar cleanup, period processing, block
///   pushing, and network sync-state updates.
pub fn pbft_manager_runtime_pbft_sync_queue_drain_next(
    runtime: &BridgePbftService,
    queue_size: usize,
    current_period: u64,
) -> FfiPbftSyncQueueDrainStep {
    if !runtime.readiness().is_ready() {
        return queue_drain_bootstrap_incomplete_step();
    }
    let mut runtime = runtime.manager_state();
    queue_drain_step_into_ffi(next_domain_pbft_sync_queue_drain_step(
        &mut runtime.pbft_sync_queue_drain_session,
        queue_size,
        current_period,
    ))
}

/// Reports one C++ queue-drain executor result to the runtime-owned planner.
///
/// Inputs:
/// - `runtime`: long-lived Rust PBFT manager runtime.
/// - `report`: C++ executor result for the previously issued drain step.
///
/// Outputs:
/// - A CXX-safe validation result that controls whether C++ may continue the
///   current drain pass.
///
/// Invariants and edge behavior:
/// - Invalid, mismatched, or failed reports terminate the runtime-owned session
///   exactly as the retired standalone bridge session did.
pub fn pbft_manager_runtime_pbft_sync_queue_drain_report(
    runtime: &BridgePbftService,
    report: FfiPbftSyncQueueDrainReport,
) -> FfiPbftSyncQueueDrainReportResult {
    if !runtime.readiness().is_ready() {
        return queue_drain_bootstrap_incomplete_report();
    }
    let mut runtime = runtime.manager_state();
    queue_drain_report_result_into_ffi(report_domain_pbft_sync_queue_drain_step(
        &mut runtime.pbft_sync_queue_drain_session,
        queue_drain_report_from_ffi(report),
    ))
}

/// Pushes one period-data payload reference into PBFT manager runtime queue metadata.
///
/// C++ remains the temporary owner of live `PeriodData`, `PbftVote`, and peer sidecars.
/// Rust owns the compact queue ordering, admission, cleanup, and pop-source decisions.
pub fn pbft_manager_runtime_period_data_queue_push(
    runtime: &BridgePbftService,
    entry_id: u64,
    period: u64,
    block_hash: [u8; 32],
    prev_block_hash: [u8; 32],
    pivot_hash: [u8; 32],
    final_chain_hash: [u8; 32],
    reward_vote_hashes: Vec<crate::ffi::rustaxa_ffi::PbftSyncTransactionHash>,
    pillar_vote_rlps: Vec<FfiPeriodDataQueuePillarVotePayload>,
    transaction_rlps: Vec<FfiPeriodDataQueueTransactionPayload>,
    previous_cert_vote_rlps: Vec<FfiPeriodDataQueuePbftVotePayload>,
    dag_transaction_hashes: Vec<crate::ffi::rustaxa_ffi::PbftSyncTransactionHash>,
    period_data_transaction_hashes: Vec<crate::ffi::rustaxa_ffi::PbftSyncTransactionHash>,
    period_data_transaction_identities: Vec<FfiPeriodDataQueueTransactionIdentity>,
    previous_cert_votes_present: bool,
    previous_cert_first_vote_has_weight: bool,
    pillar_votes_present: bool,
    extra_data_present: bool,
    extra_data_pillar_block_hash_present: bool,
    max_pbft_size: u64,
    current_block_cert_vote_rlps: Vec<FfiPeriodDataQueuePbftVotePayload>,
) -> anyhow::Result<FfiPeriodDataQueuePushOutcome> {
    let mut runtime = runtime.manager_state();
    Ok(runtime
        .period_data_queue
        .push(
            entry_id,
            period,
            ethereum_types::H256::from(block_hash),
            ethereum_types::H256::from(prev_block_hash),
            ethereum_types::H256::from(pivot_hash),
            ethereum_types::H256::from(final_chain_hash),
            bridge_hashes_to_h256(reward_vote_hashes),
            pillar_vote_rlps
                .into_iter()
                .map(|payload| payload.vote_rlp)
                .collect(),
            transaction_rlps
                .into_iter()
                .map(|payload| payload.transaction_rlp)
                .collect(),
            pbft_vote_rlps_to_vec(previous_cert_vote_rlps),
            dag_transaction_hashes
                .into_iter()
                .map(|hash| ethereum_types::H256::from(hash.hash))
                .collect(),
            period_data_transaction_hashes
                .into_iter()
                .map(|hash| ethereum_types::H256::from(hash.hash))
                .collect(),
            period_data_transaction_identities
                .into_iter()
                .map(|identity| {
                    rustaxa_consensus::period_data_queue::PeriodDataQueueTransactionIdentity {
                        input_index: identity.input_index,
                        hash: ethereum_types::H256::from(identity.hash),
                        transaction_nonce: identity.transaction_nonce,
                        sender: identity.sender,
                    }
                })
                .collect(),
            previous_cert_votes_present,
            previous_cert_first_vote_has_weight,
            pillar_votes_present,
            extra_data_present,
            extra_data_pillar_block_hash_present,
            max_pbft_size,
            pbft_vote_rlps_to_vec(current_block_cert_vote_rlps),
        )?
        .into())
}

/// Pops one PBFT sync queue metadata entry from PBFT manager runtime state.
pub fn pbft_manager_runtime_period_data_queue_pop(
    runtime: &BridgePbftService,
) -> anyhow::Result<FfiPeriodDataQueuePopPlan> {
    let mut runtime = runtime.manager_state();
    Ok(runtime.period_data_queue.pop()?.into())
}

/// Removes stale PBFT sync queue metadata entries from PBFT manager runtime state.
pub fn pbft_manager_runtime_period_data_queue_clean_old_data(
    runtime: &BridgePbftService,
    period: u64,
) -> Vec<FfiPeriodDataQueueEntryRef> {
    let mut runtime = runtime.manager_state();
    runtime
        .period_data_queue
        .clean_old_data(period)
        .into_iter()
        .map(Into::into)
        .collect()
}

impl From<rustaxa_consensus::period_data_queue::PeriodDataQueueEntryRef>
    for FfiPeriodDataQueueEntryRef
{
    fn from(value: rustaxa_consensus::period_data_queue::PeriodDataQueueEntryRef) -> Self {
        Self {
            entry_id: value.entry_id,
            period: value.period,
            block_hash: value.block_hash.into(),
            prev_block_hash: value.prev_block_hash.into(),
            pivot_hash: value.pivot_hash.into(),
            final_chain_hash: value.final_chain_hash.into(),
            reward_vote_hashes: transaction_hashes_to_bridge(value.reward_vote_hashes),
            pillar_vote_rlps: pillar_vote_rlps_to_bridge(value.pillar_vote_rlps),
            transaction_rlps: transaction_rlps_to_bridge(value.transaction_rlps),
            previous_cert_vote_rlps: pbft_vote_rlps_to_bridge(value.previous_cert_vote_rlps),
            dag_transaction_hashes: transaction_hashes_to_bridge(value.dag_transaction_hashes),
            period_data_transaction_hashes: transaction_hashes_to_bridge(
                value.period_data_transaction_hashes,
            ),
            period_data_transaction_identities: transaction_identities_to_bridge(
                value.period_data_transaction_identities,
            ),
            previous_cert_votes_present: value.previous_cert_votes_present,
            previous_cert_first_vote_has_weight: value.previous_cert_first_vote_has_weight,
            pillar_votes_present: value.pillar_votes_present,
            extra_data_present: value.extra_data_present,
            extra_data_pillar_block_hash_present: value.extra_data_pillar_block_hash_present,
        }
    }
}

impl From<rustaxa_consensus::period_data_queue::PeriodDataQueuePushOutcome>
    for FfiPeriodDataQueuePushOutcome
{
    fn from(value: rustaxa_consensus::period_data_queue::PeriodDataQueuePushOutcome) -> Self {
        Self {
            accepted: value.accepted,
            clear_existing: value.clear_existing,
            expected_next_period: value.expected_next_period,
            actual_period: value.actual_period,
            current_period: value.current_period,
            effective_size: value.effective_size,
        }
    }
}

impl From<rustaxa_consensus::period_data_queue::PeriodDataQueuePopPlan>
    for FfiPeriodDataQueuePopPlan
{
    fn from(value: rustaxa_consensus::period_data_queue::PeriodDataQueuePopPlan) -> Self {
        Self {
            entry_id: value.entry_id,
            entry_period: value.entry_period,
            block_hash: value.block_hash.into(),
            prev_block_hash: value.prev_block_hash.into(),
            pivot_hash: value.pivot_hash.into(),
            final_chain_hash: value.final_chain_hash.into(),
            reward_vote_hashes: transaction_hashes_to_bridge(value.reward_vote_hashes),
            pillar_vote_rlps: pillar_vote_rlps_to_bridge(value.pillar_vote_rlps),
            transaction_rlps: transaction_rlps_to_bridge(value.transaction_rlps),
            cert_vote_rlps: pbft_vote_rlps_to_bridge(value.cert_vote_rlps),
            previous_cert_vote_rlps: pbft_vote_rlps_to_bridge(value.previous_cert_vote_rlps),
            dag_transaction_hashes: transaction_hashes_to_bridge(value.dag_transaction_hashes),
            period_data_transaction_hashes: transaction_hashes_to_bridge(
                value.period_data_transaction_hashes,
            ),
            period_data_transaction_identities: transaction_identities_to_bridge(
                value.period_data_transaction_identities,
            ),
            previous_cert_votes_present: value.previous_cert_votes_present,
            previous_cert_first_vote_has_weight: value.previous_cert_first_vote_has_weight,
            pillar_votes_present: value.pillar_votes_present,
            extra_data_present: value.extra_data_present,
            extra_data_pillar_block_hash_present: value.extra_data_pillar_block_hash_present,
            use_last_block_cert_votes: value.use_last_block_cert_votes,
            next_entry_id: value.next_entry_id,
            current_period: value.current_period,
            effective_size: value.effective_size,
        }
    }
}

fn transaction_hashes_to_bridge(
    hashes: Vec<ethereum_types::H256>,
) -> Vec<crate::ffi::rustaxa_ffi::PbftSyncTransactionHash> {
    hashes
        .into_iter()
        .map(|hash| crate::ffi::rustaxa_ffi::PbftSyncTransactionHash { hash: hash.into() })
        .collect()
}

fn bridge_hashes_to_h256(
    hashes: Vec<crate::ffi::rustaxa_ffi::PbftSyncTransactionHash>,
) -> Vec<ethereum_types::H256> {
    hashes
        .into_iter()
        .map(|hash| ethereum_types::H256::from(hash.hash))
        .collect()
}

fn pillar_vote_rlps_to_bridge(rlps: Vec<Vec<u8>>) -> Vec<FfiPeriodDataQueuePillarVotePayload> {
    rlps.into_iter()
        .map(|vote_rlp| FfiPeriodDataQueuePillarVotePayload { vote_rlp })
        .collect()
}

fn transaction_rlps_to_bridge(rlps: Vec<Vec<u8>>) -> Vec<FfiPeriodDataQueueTransactionPayload> {
    rlps.into_iter()
        .map(|transaction_rlp| FfiPeriodDataQueueTransactionPayload { transaction_rlp })
        .collect()
}

fn pbft_vote_rlps_to_vec(payloads: Vec<FfiPeriodDataQueuePbftVotePayload>) -> Vec<Vec<u8>> {
    payloads
        .into_iter()
        .map(|payload| payload.vote_rlp)
        .collect()
}

fn pbft_vote_rlps_to_bridge(rlps: Vec<Vec<u8>>) -> Vec<FfiPeriodDataQueuePbftVotePayload> {
    rlps.into_iter()
        .map(|vote_rlp| FfiPeriodDataQueuePbftVotePayload { vote_rlp })
        .collect()
}

fn transaction_identities_to_bridge(
    identities: Vec<rustaxa_consensus::period_data_queue::PeriodDataQueueTransactionIdentity>,
) -> Vec<FfiPeriodDataQueueTransactionIdentity> {
    identities
        .into_iter()
        .map(|identity| FfiPeriodDataQueueTransactionIdentity {
            input_index: identity.input_index,
            hash: identity.hash.into(),
            transaction_nonce: identity.transaction_nonce,
            sender: identity.sender,
        })
        .collect()
}

/// Plans PBFT manager startup replay ranges from C++ live height facts.
pub fn plan_pbft_manager_startup_replay_ranges(
    fact: FfiPbftManagerStartupReplayRangeFact,
) -> FfiPbftManagerStartupReplayRangePlan {
    plan_domain_pbft_manager_startup_replay_ranges(fact.into()).into()
}

/// Plans the ordered PBFT manager period-advance effects.
///
/// The runtime derives the ordered tail from its last committed reset. The
/// returned effect list owns the surrounding advance-period order while C++
/// remains the temporary executor.
pub fn pbft_manager_runtime_plan_advance_period_after_reset(
    runtime: &BridgePbftService,
    pbft_chain_size: u64,
) -> FfiPbftManagerAdvancePeriodPlan {
    let runtime = runtime.manager_state();
    runtime
        .state
        .plan_advance_period_after_reset(pbft_chain_size)
        .into()
}

/// Validates one C++ executor report for a Rust-planned PBFT manager period-advance action.
///
/// Inputs:
/// - `plan`: Rust-owned action script returned by
///   `pbft_manager_runtime_plan_advance_period_after_reset`.
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
    runtime: &BridgePbftService,
    new_period: u64,
) -> FfiPbftManagerRuntimeSnapshot {
    let mut runtime = runtime.manager_state();
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
    runtime: &BridgePbftService,
    broadcast_votes_counter: u32,
    rebroadcast_votes_counter: u32,
    broadcast_reward_votes_counter: u32,
    rebroadcast_reward_votes_counter: u32,
) -> FfiPbftManagerRuntimeSnapshot {
    let mut runtime = runtime.manager_state();
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
    runtime: &BridgePbftService,
) -> anyhow::Result<Vec<u8>> {
    let runtime = runtime.manager_state();
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
    runtime: &BridgePbftService,
    period: u64,
    round: u32,
    block_hash: [u8; 32],
    block_rlp: Vec<u8>,
) -> anyhow::Result<FfiPbftManagerRuntimeSnapshot> {
    let mut runtime = runtime.manager_state();
    save_cert_voted_block_in_round_storage(runtime.storage.as_ref(), u64::from(round), &block_rlp)?;
    Ok(runtime
        .state
        .apply_committed_cert_voted_block(
            period,
            u64::from(round),
            ethereum_types::H256::from(block_hash),
        )
        .into())
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
    runtime: &BridgePbftService,
    period: u64,
    round: u32,
    block_hash: [u8; 32],
) -> FfiPbftManagerRuntimeSnapshot {
    let mut runtime = runtime.manager_state();
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
    runtime: &BridgePbftService,
    anchor_hash: &[u8; 32],
) -> bool {
    let runtime = runtime.manager_state();
    runtime
        .state
        .has_cached_anchor_dag_order(ethereum_types::H256::from(*anchor_hash))
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
    runtime: &BridgePbftService,
    anchor_hash: [u8; 32],
) -> FfiPbftManagerRuntimeSnapshot {
    let mut runtime = runtime.manager_state();
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
    runtime: &BridgePbftService,
    anchor_hash: [u8; 32],
) -> FfiPbftManagerRuntimeSnapshot {
    let mut runtime = runtime.manager_state();
    runtime
        .state
        .remove_cached_anchor_dag_order(ethereum_types::H256::from(anchor_hash))
        .into()
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
    runtime: &BridgePbftService,
) -> anyhow::Result<Vec<u8>> {
    let runtime = runtime.manager_state();
    load_own_pillar_block_vote_storage(runtime.storage.as_ref())
}

fn transition_runtime_apply_result(
    status: u8,
    applied_writes: u64,
    snapshot: PbftManagerRuntimeSnapshot,
    error_code: String,
) -> FfiPbftManagerRuntimeStorageApplyResult {
    FfiPbftManagerRuntimeStorageApplyResult {
        status,
        applied_writes,
        snapshot: snapshot.into(),
        error_code,
    }
}

/// Plans, persists, and commits one lifecycle transition as a Rust-owned operation.
pub fn pbft_manager_runtime_execute_lifecycle_transition(
    runtime: &BridgePbftService,
    request: FfiPbftManagerLifecycleTransitionRequest,
) -> anyhow::Result<FfiPbftManagerLifecycleTransitionResult> {
    let mut runtime = runtime.manager_state();
    let kind = PbftManagerTransitionKind::from_u8(request.kind);
    let plan = runtime
        .state
        .plan_lifecycle_transition(PbftManagerLifecycleTransitionRequest {
            kind,
            target_period: request.target_period,
            target_round: request.target_round,
            has_network_next_voting_step: request.has_network_next_voting_step,
            network_next_voting_step: request.network_next_voting_step,
        });
    if plan.status != PbftManagerTransitionStatus::Ready {
        return Ok(FfiPbftManagerLifecycleTransitionResult {
            status: TRANSITION_STORAGE_STATUS_REJECTED,
            snapshot: runtime.state.snapshot().into(),
            remove_cert_voted_sidecar: false,
            clear_broadcasted_vote_sidecars: false,
            set_vote_manager_period_round: false,
            reset_current_round_timer: false,
            reset_second_finish_timer: false,
            print_cert_step_info: false,
            print_second_finish_step_info: false,
            reset_executed_block_follow_up: false,
            error_code: plan.error_code,
        });
    }
    let own_votes_guard = if plan.clear_own_votes {
        Some(runtime.storage.lock_own_verified_votes()?)
    } else {
        None
    };
    let own_vote_hashes = if own_votes_guard.is_some() {
        runtime.storage.pbft().own_verified_vote_hashes()?
    } else {
        Vec::new()
    };
    let storage_result = apply_pbft_manager_transition_storage(
        runtime.storage.as_ref(),
        &plan,
        &own_vote_hashes,
        false,
    )?;
    drop(own_votes_guard);
    if storage_result.status != PbftManagerTransitionStorageStatus::Applied {
        return Ok(FfiPbftManagerLifecycleTransitionResult {
            status: TRANSITION_STORAGE_STATUS_REJECTED,
            snapshot: runtime.state.snapshot().into(),
            remove_cert_voted_sidecar: false,
            clear_broadcasted_vote_sidecars: false,
            set_vote_manager_period_round: false,
            reset_current_round_timer: false,
            reset_second_finish_timer: false,
            print_cert_step_info: false,
            print_second_finish_step_info: false,
            reset_executed_block_follow_up: false,
            error_code: storage_result.error_code,
        });
    }
    runtime.state.apply_committed_transition(&plan);
    runtime
        .state
        .record_committed_reset(request.target_period, &plan);
    Ok(FfiPbftManagerLifecycleTransitionResult {
        status: TRANSITION_STORAGE_STATUS_APPLIED,
        snapshot: runtime.state.snapshot().into(),
        remove_cert_voted_sidecar: plan.remove_cert_voted_block,
        clear_broadcasted_vote_sidecars: plan.clear_broadcasted_votes,
        set_vote_manager_period_round: plan.set_vote_manager_period_round,
        reset_current_round_timer: plan.reset_current_round_start,
        reset_second_finish_timer: plan.reset_second_finish_start,
        print_cert_step_info: plan.print_cert_step_info,
        print_second_finish_step_info: plan.print_second_finish_step_info,
        reset_executed_block_follow_up: plan.reset_executed_block_status,
        error_code: String::new(),
    })
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
    runtime: &BridgePbftService,
) -> anyhow::Result<FfiPbftManagerRuntimeStorageApplyResult> {
    let mut runtime = runtime.manager_state();
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
    runtime: &BridgePbftService,
    status: u8,
) -> anyhow::Result<FfiPbftManagerRuntimeSnapshot> {
    let mut runtime = runtime.manager_state();
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
    runtime: &BridgePbftService,
    field: u8,
    value: u32,
) -> anyhow::Result<FfiPbftManagerRuntimeSnapshot> {
    let mut runtime = runtime.manager_state();
    apply_pbft_manager_cursor_field_storage(runtime.storage.as_ref(), field, value)?;
    runtime.state.apply_committed_cursor_field(field, value);
    Ok(runtime.state.snapshot().into())
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
    runtime: &BridgePbftService,
    hash: &[u8; 32],
) -> anyhow::Result<FfiBlockPeriodLookup> {
    let runtime = runtime.manager_state();
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
    runtime: &BridgePbftService,
    hash: &[u8; 32],
) -> anyhow::Result<bool> {
    let runtime = runtime.manager_state();
    pbft_block_exists_in_storage(runtime.storage.as_ref(), ethereum_types::H256::from(*hash))
}

/// Plans one deterministic PBFT finalization intent through Rust for a PBFT
/// manager runtime.
///
/// The runtime argument is intentionally explicit to keep the API bound to
/// the manager-runtime bridge boundary. The current planner is stateless and
/// uses only the supplied fact; future runtime policy can be added here without
/// reintroducing a standalone CXX entry point.
pub fn pbft_manager_runtime_plan_finalization_intent(
    _runtime: &BridgePbftService,
    fact: FfiPbftFinalizationIntentFact,
) -> FfiPbftFinalizationIntentPlan {
    plan_domain_pbft_finalization_intent(fact.into()).into()
}

/// Plans finalization dynamic-lambda state and loads the prior persisted lambda through runtime storage.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime with its Rust storage handle.
/// - `fact`: Cacti dynamic-lambda inputs gathered by the C++ compatibility shim.
///
/// Outputs:
/// - The Rust dynamic-lambda decision for this finalized PBFT block.
/// - The closest persisted period-lambda at or before `finalized_period - 1`
///   when Cacti dynamic-lambda is active.
///
/// Invariants and edge behavior:
/// - This function does not mutate storage or live manager state.
/// - Storage is read only when the dynamic-lambda plan is accepted and active;
///   disabled or rejected plans return an empty previous-lambda lookup.
/// - `finalized_period = 0` with active dynamic-lambda is treated as having no
///   previous persisted lambda, matching the lower-bound lookup contract.
pub fn pbft_manager_runtime_plan_finalization_dynamic_lambda(
    runtime: &BridgePbftService,
    fact: FfiPbftDynamicLambdaFact,
) -> anyhow::Result<FfiPbftManagerFinalizationDynamicLambdaPlan> {
    let runtime = runtime.manager_state();
    let dynamic_lambda_active = fact.dynamic_lambda_active;
    let finalized_period = fact.finalized_period;
    let plan = plan_domain_pbft_dynamic_lambda(PbftDynamicLambdaFact::from(fact));
    let (last_saved_period_lambda_found, last_saved_period_lambda) = if dynamic_lambda_active
        && plan.status == rustaxa_consensus::pbft_finalize::PbftFinalizationStatus::Accepted
    {
        let lookup = load_domain_pbft_finalization_last_period_lambda(
            runtime.storage.as_ref(),
            finalized_period.saturating_sub(1),
        )?;
        (lookup.found, lookup.value)
    } else {
        (false, 0)
    };
    Ok(FfiPbftManagerFinalizationDynamicLambdaPlan::from((
        plan,
        last_saved_period_lambda_found,
        last_saved_period_lambda,
    )))
}

/// Starts a runtime-owned PBFT manager daemon-tick session.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime that owns the temporary tick
///   cursor.
/// - `fact`: compact C++ daemon-loop facts for one PBFT manager tick.
///
/// Outputs:
/// - Replaces any previous runtime session cursor. C++ drives the cursor with
///   `pbft_manager_runtime_session_next` and
///   `pbft_manager_runtime_session_report`.
///
/// Invariants and edge behavior:
/// - The tick session is PBFT-manager implementation state and is not exported
///   as a standalone CXX handle.
/// - Starting a new tick replaces any incomplete previous tick, matching the
///   legacy per-loop allocation behavior.
pub fn pbft_manager_runtime_begin_session(
    runtime: &BridgePbftService,
    fact: FfiPbftManagerRuntimeTickFact,
) {
    if !runtime.readiness().is_ready() {
        return;
    }
    let mut runtime = runtime.manager_state();
    runtime.runtime_session = Some(create_domain_pbft_manager_runtime_session(fact.into()));
}

/// Returns the next requested action for the runtime-owned tick session.
pub fn pbft_manager_runtime_session_next(
    runtime: &BridgePbftService,
) -> FfiPbftManagerRuntimeSessionStep {
    let runtime = runtime.manager_state();
    let Some(session) = runtime.runtime_session.as_ref() else {
        return runtime_session_not_started_step();
    };
    next_pbft_manager_runtime_action(session).into()
}

/// Reports one C++-executed action back to the runtime-owned tick session.
pub fn pbft_manager_runtime_session_report(
    runtime: &BridgePbftService,
    report: FfiPbftManagerRuntimeActionReport,
) -> FfiPbftManagerRuntimeSessionStep {
    let mut runtime = runtime.manager_state();
    let Some(session) = runtime.runtime_session.take() else {
        return runtime_session_not_started_step();
    };
    runtime.runtime_session = Some(report_pbft_manager_runtime_action(session, report.into()));
    runtime
        .runtime_session
        .as_ref()
        .map(next_pbft_manager_runtime_action)
        .map(Into::into)
        .unwrap_or_else(runtime_session_not_started_step)
}

/// Plans whether the C++ PBFT manager shell should wait using the Rust runtime
/// deadline.
pub fn plan_pbft_manager_runtime_sleep_until_next_step(
    runtime: &BridgePbftService,
    round_elapsed_ms: i64,
) -> FfiPbftManagerSleepPlan {
    let runtime = runtime.manager_state();
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

/// Aborts the runtime-owned PBFT manager tick session.
pub fn abort_pbft_manager_runtime_session(runtime: &BridgePbftService) {
    let mut runtime = runtime.manager_state();
    if let Some(session) = runtime.runtime_session.take() {
        runtime.runtime_session = Some(abort_domain_pbft_manager_runtime_session(session));
    }
}

fn runtime_session_not_started_step() -> FfiPbftManagerRuntimeSessionStep {
    FfiPbftManagerRuntimeSessionStep {
        status: PbftManagerRuntimeStatus::ContractError.as_u8(),
        cursor: 0,
        action: ACTION_NO_ACTION,
        has_action: false,
        complete: true,
        restart_loop: false,
        has_target_round: false,
        target_round: 0,
        sleep_ms: 0,
        tick_id: 0,
        can_continue: false,
        error_code: "PBFT_MANAGER_RUNTIME_SESSION_NOT_STARTED".to_owned(),
    }
}

/// Starts a runtime-owned state-action effect session from compact C++ facts.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime that owns temporary session
///   cursors for C++ compatibility execution.
/// - `fact`: compact manager state, timing, and vote-status facts sourced by C++.
///
/// Outputs:
/// - Resets the runtime-owned state-action effect cursor. Callers then drive it
///   through `pbft_manager_runtime_state_action_effect_session_next` and
///   `pbft_manager_runtime_state_action_effect_session_report`.
///
/// Invariants and edge behavior:
/// - The session is not exported as a separate CXX handle; it is PBFT-manager
///   runtime implementation state.
/// - Starting a new session replaces any previous incomplete state-action
///   cursor, matching the legacy per-call session allocation behavior.
pub fn pbft_manager_runtime_begin_state_action_effect_session(
    runtime: &BridgePbftService,
    fact: FfiPbftManagerStateActionFact,
) {
    let mut runtime = runtime.manager_state();
    runtime.state_action_effect_session = Some(
        create_domain_pbft_manager_state_action_effect_session(fact.into()),
    );
}

/// Returns the next effect requested by the runtime-owned state-action session.
pub fn pbft_manager_runtime_state_action_effect_session_next(
    runtime: &BridgePbftService,
) -> FfiPbftManagerStateActionSessionStep {
    let mut runtime = runtime.manager_state();
    let Some(session) = runtime.state_action_effect_session.as_mut() else {
        return state_action_effect_session_not_started_step();
    };
    next_domain_pbft_manager_state_action_effect_session(session).into()
}

/// Reports one C++-executed state-action effect to Rust and returns the next step.
pub fn pbft_manager_runtime_state_action_effect_session_report(
    runtime: &BridgePbftService,
    report: FfiPbftManagerStateActionEffectReport,
) -> FfiPbftManagerStateActionSessionStep {
    let mut runtime = runtime.manager_state();
    let Some(session) = runtime.state_action_effect_session.as_mut() else {
        return state_action_effect_session_not_started_step();
    };
    report_domain_pbft_manager_state_action_effect_session(session, report.into()).into()
}

fn state_action_effect_session_not_started_step() -> FfiPbftManagerStateActionSessionStep {
    FfiPbftManagerStateActionSessionStep {
        status: PbftManagerStateActionSessionStatus::ContractError.as_u8(),
        cursor: 0,
        has_effect: false,
        effect: FfiPbftManagerStateActionEffect {
            intent: PbftManagerStateActionIntent::Noop.as_u8(),
            hash: [0; 32],
            request_proposed_block_sidecar: false,
            proposed_block_sidecar_hash: [0; 32],
            proposed_block_sidecar_period: 0,
        },
        go_finish_state: false,
        loop_back_finish_state: false,
        complete: true,
        can_continue: false,
        error_code: "PBFT_MANAGER_STATE_ACTION_EFFECT_SESSION_NOT_STARTED".to_owned(),
    }
}

/// Starts a runtime-owned PBFT proposal-construction session from compact C++ facts.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime that owns the temporary proposal
///   cursor.
/// - `fact`: compact C++ facts needed to select a proposer and DAG anchor.
///
/// Outputs:
/// - Replaces any previous proposal cursor. C++ drives the cursor with
///   `pbft_manager_proposal_session_next` and
///   `pbft_manager_proposal_session_report_dag_order`.
///
/// Invariants and edge behavior:
/// - Proposal planning is PBFT-manager implementation state and is not exported
///   as a standalone CXX handle.
/// - Starting a new proposal replaces any incomplete previous proposal, matching
///   the legacy per-call allocation behavior.
pub(crate) fn pbft_manager_runtime_begin_proposal_session_with_hash(
    runtime: &BridgePbftService,
    fact: FfiPbftManagerProposalInitialFact,
    final_chain_hash: Option<[u8; 32]>,
) {
    if !runtime.readiness().is_ready() {
        return;
    }
    let mut runtime = runtime.manager_state();
    runtime.proposal_session = Some(create_domain_pbft_manager_proposal_session(
        proposal_initial_fact_from_ffi(fact, final_chain_hash),
    ));
}

/// Returns the next proposal-construction action or build command.
pub fn pbft_manager_proposal_session_next(
    runtime: &BridgePbftService,
) -> FfiPbftManagerProposalSessionStep {
    if !runtime.readiness().is_ready() {
        return proposal_session_not_started_step();
    }
    let mut runtime = runtime.manager_state();
    let Some(session) = runtime.proposal_session.as_mut() else {
        return proposal_session_not_started_step();
    };
    next_domain_pbft_manager_proposal_session(session).into()
}

/// Reports one C++-loaded DAG order to the runtime-owned proposal session.
pub fn pbft_manager_proposal_session_report_dag_order(
    runtime: &BridgePbftService,
    report: FfiPbftManagerProposalDagOrderReport,
) -> FfiPbftManagerProposalSessionStep {
    let mut runtime = runtime.manager_state();
    let Some(session) = runtime.proposal_session.as_mut() else {
        return proposal_session_not_started_step();
    };
    report_domain_pbft_manager_proposal_dag_order(session, report.into()).into()
}

fn proposal_session_not_started_step() -> FfiPbftManagerProposalSessionStep {
    FfiPbftManagerProposalSessionStep {
        action: PbftManagerProposalAction::ContractError.as_u8(),
        status: PbftManagerProposalStatus::InvalidBridgeFacts.as_u8(),
        requested_anchor_hash: [0; 32],
        previous_pbft_block_hash: [0; 32],
        anchor_hash: [0; 32],
        order_hash: [0; 32],
        final_chain_hash: [0; 32],
        eligible_wallet_indices: Vec::new(),
        dag_blocks_included: 0,
        selected_null_anchor: false,
        error_code: "PBFT_MANAGER_PROPOSAL_SESSION_NOT_STARTED".to_owned(),
    }
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

/// Plans one PBFT block-validation step from the current live-fact bundle.
///
/// Inputs:
/// - `fact`: block identity, static validation flags, and current live-check
///   statuses supplied by C++.
///
/// Outputs:
/// - Returns the next requested live check, terminal accept/reject, or
///   contract error for invalid bridge facts.
///
/// Invariants and edge behavior:
/// - Rust owns the deterministic check ordering and rejection reason.
/// - C++ owns live PBFT chain, FinalChain, reward-vote, pillar, and DAG
///   execution and calls this again after updating the corresponding status in
///   the fact bundle.
/// - No validation cursor is stored in `BridgePbftService`; callers keep
///   the per-block fact bundle local to the validation path.
pub fn plan_pbft_manager_block_validation(
    fact: FfiPbftManagerBlockValidationFact,
) -> FfiPbftManagerBlockValidationPlan {
    plan_domain_pbft_manager_block_validation(fact.into()).into()
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

const FINALIZATION_EXECUTOR_MODE_FRESH: u8 = 0;
const FINALIZATION_EXECUTOR_MODE_RESUME: u8 = 1;

struct FinalizationRuntimeSessionStep {
    status: u8,
    cursor: u32,
    action: u8,
    has_action: bool,
    complete: bool,
    can_continue: bool,
    error_code: String,
}

impl From<rustaxa_consensus::pbft_finalize::PbftFinalizationRuntimeStep>
    for FinalizationRuntimeSessionStep
{
    fn from(value: rustaxa_consensus::pbft_finalize::PbftFinalizationRuntimeStep) -> Self {
        let status = value.runtime_status.as_u8();
        Self {
            status,
            cursor: value.action_index,
            action: value
                .action
                .map(PbftFinalizationRuntimeAction::as_u8)
                .unwrap_or(ACTION_NO_ACTION),
            has_action: value.has_action,
            complete: value.complete,
            can_continue: status == RUNTIME_STATUS_ACTIVE || status == RUNTIME_STATUS_COMPLETE,
            error_code: value.error_code,
        }
    }
}

fn finalization_executor_state_from_boundary(
    boundary: rustaxa_consensus::pbft_manager::PbftFinalizationExecutorBoundary,
) -> FfiPbftManagerFinalizationExecutorState {
    let next_step: FinalizationRuntimeSessionStep = boundary.next_step.into();
    FfiPbftManagerFinalizationExecutorState {
        status: next_step.status,
        cursor: next_step.cursor,
        action: next_step.action,
        has_action: next_step.has_action,
        complete: next_step.complete,
        can_continue: next_step.can_continue,
        cleared_anchor_dag_cache: boundary.cleared_anchor_dag_cache,
        has_snapshot: boundary.has_snapshot,
        expired_dag_hashes: boundary
            .expired_dag_hashes
            .into_iter()
            .map(|hash| FfiPbftFinalizationHash { hash: hash.0 })
            .collect(),
        refresh_dag_counters: boundary.refresh_dag_counters,
        snapshot: boundary.snapshot.into(),
        error_code: if boundary.error_code.is_empty() {
            next_step.error_code
        } else {
            boundary.error_code
        },
    }
}

/// Starts the manager-owned PBFT finalization executor and advances to the first external action.
///
/// Inputs:
/// - `runtime`: long-lived PBFT manager runtime that owns storage and the
///   finalization cursor.
/// - `dag_transaction_service`: Rust DAG/sortition service used to preview
///   canonical sortition facts before fresh primary storage.
/// - `request`: accepted finalization intent plus either fresh primary-storage
///   stages or the FinalChain height needed to inspect duplicate-finalization
///   resume from runtime-owned storage.
///
/// Outputs:
/// - Applies fresh primary storage when requested, drains Rust-owned manager
///   actions, and returns the next cursor/action that must be executed by C++.
///
/// Invariants and edge behavior:
/// - This never executes external FinalChain/EVM, DAG, transaction-manager,
///   sortition, vote-manager, advance-period, pillar, or network work.
/// - PBFT-chain publication, dynamic-lambda persistence/publication, executed
///   status persistence/publication, and anchor-cache clearing are native
///   manager actions drained before returning.
/// - Fresh mode requires the first Rust action to be primary storage and reports
///   storage apply success or failure to the same cursor before returning.
/// - Resume mode derives the durable replay transcript from runtime-owned
///   storage and does not reapply primary storage.
/// - The caller must echo the returned `cursor` to
///   one of the typed finalization advancement APIs after executing one
///   external action.
pub fn pbft_manager_runtime_start_finalization_executor(
    runtime: &BridgePbftService,
    dag_transaction_service: &BridgeDagTransactionService,
    request: FfiPbftFinalizationExecutorStartRequest,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    let mode = match request.mode {
        FINALIZATION_EXECUTOR_MODE_FRESH => {
            rustaxa_consensus::pbft_manager::PbftFinalizationExecutorStartMode::Fresh {
                primary_stages: request.primary_stages.into_iter().map(Into::into).collect(),
                sync: request.sync,
            }
        }
        FINALIZATION_EXECUTOR_MODE_RESUME => {
            rustaxa_consensus::pbft_manager::PbftFinalizationExecutorStartMode::Resume {
                final_chain_last_block: request.final_chain_last_block,
            }
        }
        _ => rustaxa_consensus::pbft_manager::PbftFinalizationExecutorStartMode::Unknown,
    };
    let boundary = runtime.0.start_finalization_executor(
        dag_transaction_service.native(),
        rustaxa_consensus::pbft_manager::PbftFinalizationExecutorStartRequest {
            plan: PbftFinalizationPlan::from(&request.plan),
            mode,
        },
    )?;
    Ok(finalization_executor_state_from_boundary(boundary))
}

/// Reports failure for the current external PBFT finalization action.
///
/// Inputs:
/// - `runtime`: PBFT manager runtime that owns the current finalization cursor.
/// - `cursor`: executor cursor previously returned to C++.
/// - `status` and `error_code`: failure facts observed while C++ executed the
///   external action.
///
/// Outputs:
/// - Returns the terminal executor state after recording the failed action.
///
/// Invariants and edge behavior:
/// - C++ reports only failure facts through this API. Successful subsystem
///   reports must use the typed advancement API for that subsystem.
/// - Cursor mismatch, missing action, and unknown action preserve the same
///   finalization executor contract as successful typed advancement APIs.
pub fn pbft_manager_runtime_fail_finalization_external_effect(
    runtime: &BridgePbftService,
    cursor: u32,
    status: u8,
    error_code: String,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    runtime
        .0
        .fail_finalization_external_effect(cursor, status, error_code)
        .map(finalization_executor_state_from_boundary)
}

/// Applies finalized transaction status and advances the manager-owned executor.
///
/// Inputs:
/// - `runtime`: PBFT manager runtime that owns the accepted canonical period data.
/// - `dag_transaction_service`: native transaction owner.
/// - `cursor`: executor cursor previously returned to C++.
/// - `retention_window`: configured recently-finalized sidecar window.
/// - `account_nonce_facts`: narrow facts read through the external EVM boundary.
///
/// Outputs:
/// - The next PBFT finalization executor state.
///
/// Invariants and edge behavior:
/// - Rust decodes canonical period transactions, applies native storage,
///   sidecar, queue, and purge effects, then validates the accepted count.
/// - C++ never materializes transaction payload facts or receives mutation
///   buckets. Cursor mismatch does not call the transaction owner.
pub fn pbft_manager_runtime_advance_finalization_transaction_status(
    runtime: &BridgePbftService,
    dag_transaction_service: &BridgeDagTransactionService,
    cursor: u32,
    retention_window: u64,
    account_nonce_facts: Vec<FfiTransactionQueueAccountNonceFact>,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    runtime
        .0
        .advance_finalization_transaction_status(
            dag_transaction_service.native(),
            cursor,
            retention_window,
            bridge_to_service_account_nonce_facts(account_nonce_facts),
        )
        .map(finalization_executor_state_from_boundary)
}

/// Applies the retained finalized DAG order and advances its native PBFT cursor.
/// The borrowed PBFT and DAG/transaction services form one task: Rust rejects
/// stale actions before mutation, derives all order inputs, validates the result,
/// and returns the next state plus committed cache/counter compatibility effects.
/// Operational failures propagate without fabricating effects.
pub fn pbft_manager_runtime_advance_finalization_dag_order(
    runtime: &BridgePbftService,
    dag_transaction_service: &BridgeDagTransactionService,
    cursor: u32,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    runtime
        .0
        .advance_finalization_dag_order(dag_transaction_service.native(), cursor)
        .map(finalization_executor_state_from_boundary)
}

/// Reports sortition finalization commit facts to the manager-owned PBFT
/// finalization executor.
///
/// Inputs:
/// - `runtime`: PBFT manager runtime that owns the current finalization cursor.
/// - `cursor`: executor cursor previously returned to C++.
/// - Retained preparation: canonical facts and expected optional change captured
///   before primary storage; C++ supplies no sortition facts.
///
/// Outputs:
/// - The next PBFT finalization executor state.
///
/// Invariants and edge behavior:
/// - C++ does not construct a generic PBFT finalization external-effect report
///   for the sortition client.
/// - Rust derives the PBFT finalization action from the cursor and maps only
///   sortition change/current-threshold/cache-count facts needed for
///   live-mutation validation.
/// - Cursor mismatch and validation failure use the same executor-state
///   contract as every typed finalization advancement API.
pub fn pbft_manager_runtime_advance_finalization_sortition_commit(
    runtime: &BridgePbftService,
    dag_transaction_service: &BridgeDagTransactionService,
    cursor: u32,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    runtime
        .0
        .advance_finalization_sortition_commit(dag_transaction_service.native(), cursor)
        .map(finalization_executor_state_from_boundary)
}

/// Reports reward-vote reset finalization facts to the manager-owned PBFT
/// finalization executor.
///
/// Inputs:
/// - `runtime`: PBFT manager runtime that owns the current finalization cursor.
/// - `cursor`: executor cursor previously returned to C++.
///
/// Outputs:
/// - The next PBFT finalization executor state.
///
/// Invariants and edge behavior:
/// - C++ does not construct a generic PBFT finalization external-effect report
///   for the reward-vote reset client.
/// - Rust derives the PBFT finalization action and reward-vote identity from the
///   accepted plan, then commits the cursor through the native verified-vote
///   owner. No C++ report participates in the operation.
/// - Cursor mismatch and validation failure use the same executor-state
///   contract as every typed finalization advancement API.
pub fn pbft_manager_runtime_advance_finalization_reward_votes_reset(
    runtime: &BridgePbftService,
    cursor: u32,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    runtime
        .0
        .advance_finalization_reward_votes_reset(cursor)
        .map(finalization_executor_state_from_boundary)
}

/// Reports FinalChain dispatch or replay facts to the manager-owned PBFT
/// finalization executor.
///
/// Inputs:
/// - `runtime`: PBFT manager runtime that owns the current finalization cursor.
/// - `cursor`: executor cursor previously returned to C++.
/// - `last_block`: FinalChain height observed after the external dispatch.
///
/// Outputs:
/// - The next PBFT finalization executor state.
///
/// Invariants and edge behavior:
/// - C++ does not construct a generic PBFT finalization external-effect report
///   for the FinalChain dispatch/replay client.
/// - Rust derives the PBFT finalization action from the cursor, marks the
///   FinalChain dispatch as observed, derives blocks-per-year from its retained
///   plan, and maps only the observed height into live-mutation validation.
/// - Cursor mismatch and validation failure use the same executor-state
///   contract as every typed finalization advancement API.
pub fn pbft_manager_runtime_advance_finalization_final_chain_dispatch(
    runtime: &BridgePbftService,
    cursor: u32,
    last_block: u64,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    runtime
        .0
        .advance_finalization_final_chain_dispatch(cursor, last_block)
        .map(finalization_executor_state_from_boundary)
}

/// Reports PBFT finalization pillar post-processing facts to the manager-owned executor.
///
/// Inputs:
/// - `runtime`: PBFT manager runtime that owns the current finalization cursor
///   and live manager snapshot.
/// - `cursor`: executor cursor previously returned to C++.
/// - `request_period`: FinalChain request period used by the pillar leaf.
///
/// Outputs:
/// - The next PBFT finalization executor state.
///
/// Invariants and edge behavior:
/// - C++ does not construct a generic PBFT finalization external-effect report
///   for the pillar post-processing client.
/// - Rust derives the PBFT finalization action from the cursor, injects the
///   manager period from the runtime snapshot, derives the processed period
///   from its retained plan, and maps only the request period into validation.
/// - Cursor mismatch and validation failure use the same executor-state
///   contract as every typed finalization advancement API.
pub fn pbft_manager_runtime_advance_finalization_pillar_post_processing(
    runtime: &BridgePbftService,
    cursor: u32,
    request_period: u64,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    runtime
        .0
        .advance_finalization_pillar_post_processing(cursor, request_period)
        .map(finalization_executor_state_from_boundary)
}

/// Reports PBFT finalization advance-period facts to the manager-owned executor.
///
/// Inputs:
/// - `runtime`: PBFT manager runtime that owns the current finalization cursor.
/// - `cursor`: executor cursor previously returned to C++.
///
/// Outputs:
/// - The next PBFT finalization executor state.
///
/// Invariants and edge behavior:
/// - C++ does not construct a generic PBFT finalization external-effect report
///   for the advance-period client.
/// - Rust derives the PBFT finalization action and post-advance manager period
///   from its lock-held native state.
/// - Cursor mismatch and validation failure use the same executor-state
///   contract as every typed finalization advancement API.
pub fn pbft_manager_runtime_advance_finalization_advance_period(
    runtime: &BridgePbftService,
    cursor: u32,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    runtime
        .0
        .advance_finalization_advance_period(cursor)
        .map(finalization_executor_state_from_boundary)
}

impl From<(PbftDynamicLambdaPlan, bool, u32)> for FfiPbftManagerFinalizationDynamicLambdaPlan {
    fn from(value: (PbftDynamicLambdaPlan, bool, u32)) -> Self {
        let (plan, last_saved_period_lambda_found, last_saved_period_lambda) = value;
        let error_code = if plan.status
            == rustaxa_consensus::pbft_finalize::PbftFinalizationStatus::ContractError
        {
            "PBFT_DYNAMIC_LAMBDA_CONTRACT_ERROR".to_string()
        } else {
            String::new()
        };
        Self {
            apply_dynamic_lambda_update: plan.apply_dynamic_lambda_update,
            period_lambda: plan.period_lambda,
            blocks_per_year: plan.blocks_per_year,
            rounds_count_dynamic_lambda: plan.rounds_count_dynamic_lambda,
            dynamic_lambda: plan.dynamic_lambda,
            decreased_dynamic_lambda: plan.decreased_dynamic_lambda,
            increased_dynamic_lambda: plan.increased_dynamic_lambda,
            status: plan.status.as_u8(),
            error_code,
            last_saved_period_lambda_found,
            last_saved_period_lambda,
        }
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

fn proposal_initial_fact_from_ffi(
    value: FfiPbftManagerProposalInitialFact,
    final_chain_hash: Option<[u8; 32]>,
) -> PbftManagerProposalInitialFact {
    PbftManagerProposalInitialFact {
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
        final_chain_hash_valid: final_chain_hash.is_some(),
        final_chain_hash: final_chain_hash.unwrap_or([0; 32]).into(),
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
    use crate::ffi::rustaxa_ffi::PbftDynamicLambdaConfig as FfiPbftDynamicLambdaConfig;
    use crate::ffi::{BridgePbftStorageQueries, BridgeStorage};
    use crate::pillar_chain::create_pillar_test_service_from_storage;
    use crate::storage::{
        create_pbft_storage_queries, create_pbft_vote_storage_queries, create_storage,
    };
    use ethereum_types::H256;
    use rustaxa_consensus::pbft_finalize::{PbftFinalizationRuntimeStatus, PbftFinalizationStatus};
    use rustaxa_consensus::pbft_manager::save_cert_voted_block_in_round_storage;
    use rustaxa_consensus::{save_own_verified_vote, PbftVoteStorageRecord};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Clones the storage handle from the native manager owner for bridge
    /// boundary fixtures. This deliberately avoids recreating the removed
    /// bridge-root storage sidecar.
    fn native_service_storage(
        service: &BridgePbftService,
    ) -> std::sync::Arc<rustaxa_storage::Storage> {
        service.manager_state().storage.clone()
    }

    const STATE_VALUE_PROPOSAL: u8 = 0;
    const ACTION_PROCESS_SYNCED: u8 = 0;
    const ACTION_BROADCAST: u8 = 1;
    const ACTION_TRY_CERT: u8 = 2;
    const ACTION_TRY_ROUND: u8 = 3;
    const ACTION_SLEEP_INELIGIBLE: u8 = 4;
    const RESULT_CONTINUE: u8 = 0;
    const RESULT_STATE_DONE: u8 = 2;
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
    const TRANSITION_RESET: u8 = 0;
    const TRANSITION_FILTER: u8 = 1;
    const TRANSITION_STORAGE_STATUS_APPLIED: u8 = 0;
    const TRANSITION_STORAGE_STATUS_REJECTED: u8 = 1;
    const ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND: u8 = 2;
    const ADVANCE_ACTION_RESET_CURRENT_ROUND_TIMER: u8 = 3;
    const ADVANCE_ACTION_RESET_REWARD_VOTE_COUNTERS: u8 = 4;
    const ADVANCE_ACTION_RESET_PERIOD_TIMER: u8 = 5;
    const ADVANCE_ACTION_UPDATE_WALLET_ELIGIBILITY: u8 = 6;
    const ADVANCE_ACTION_CLEANUP_PERIOD_STATE: u8 = 7;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), nanos))
    }

    fn queue_hash(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    fn queue_transaction_hashes(
        seeds: &[u8],
    ) -> Vec<crate::ffi::rustaxa_ffi::PbftSyncTransactionHash> {
        seeds
            .iter()
            .map(|seed| crate::ffi::rustaxa_ffi::PbftSyncTransactionHash {
                hash: queue_hash(*seed),
            })
            .collect()
    }

    fn queue_pillar_vote_rlps(seeds: &[u8]) -> Vec<FfiPeriodDataQueuePillarVotePayload> {
        seeds
            .iter()
            .map(|seed| FfiPeriodDataQueuePillarVotePayload {
                vote_rlp: vec![*seed, seed.wrapping_add(1)],
            })
            .collect()
    }

    fn queue_transaction_rlps(seeds: &[u8]) -> Vec<FfiPeriodDataQueueTransactionPayload> {
        seeds
            .iter()
            .map(|seed| FfiPeriodDataQueueTransactionPayload {
                transaction_rlp: vec![*seed, seed.wrapping_add(1)],
            })
            .collect()
    }

    fn queue_pbft_vote_rlps(seeds: &[u8]) -> Vec<FfiPeriodDataQueuePbftVotePayload> {
        seeds
            .iter()
            .map(|seed| FfiPeriodDataQueuePbftVotePayload {
                vote_rlp: vec![*seed, seed.wrapping_add(1)],
            })
            .collect()
    }

    fn queue_transaction_identities(seeds: &[u8]) -> Vec<FfiPeriodDataQueueTransactionIdentity> {
        seeds
            .iter()
            .enumerate()
            .map(
                |(input_index, seed)| FfiPeriodDataQueueTransactionIdentity {
                    input_index: input_index as u64,
                    hash: queue_hash(*seed),
                    transaction_nonce: queue_hash(seed.wrapping_add(1)),
                    sender: [seed.wrapping_add(2); 20],
                },
            )
            .collect()
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

    fn runtime_for_startup(name: &str) -> Box<BridgePbftService> {
        let temp_path = unique_temp_dir(name);
        let storage =
            crate::storage::create_storage(temp_path.to_str().expect("utf-8 temp path")).unwrap();
        let mut startup = startup_fact();
        startup.cacti_active_at_chain_size = false;
        create_pbft_manager_runtime_from_storage(&storage, startup).unwrap()
    }

    fn runtime_for_tick(tick: FfiPbftManagerRuntimeTickFact) -> Box<BridgePbftService> {
        let mut runtime = runtime_for_startup("rustaxa_bridge_pbft_manager_runtime_session");
        pbft_manager_runtime_begin_session(&mut runtime, tick);
        runtime
    }

    #[test]
    fn bridge_session_returns_ineligible_polling_sleep_ms() {
        let mut tick = fact(STATE_VALUE_PROPOSAL);
        tick.polling_interval_ms = 250;
        let mut runtime = runtime_for_tick(tick);
        for expected in [ACTION_PROCESS_SYNCED, ACTION_BROADCAST, ACTION_TRY_CERT] {
            let step = pbft_manager_runtime_session_next(&mut runtime);
            let result = if expected == ACTION_TRY_CERT {
                RESULT_CONTINUE
            } else {
                RESULT_STATE_DONE
            };
            let _ = pbft_manager_runtime_session_report(
                &mut runtime,
                report(step.cursor, expected, result),
            );
        }

        let step = pbft_manager_runtime_session_next(&mut runtime);
        assert_eq!(step.action, ACTION_TRY_ROUND);
        let mut action_report = report(step.cursor, ACTION_TRY_ROUND, RESULT_CONTINUE);
        action_report.has_eligible_wallet = false;
        let sleep = pbft_manager_runtime_session_report(&mut runtime, action_report);

        assert_eq!(sleep.action, ACTION_SLEEP_INELIGIBLE);
        assert_eq!(sleep.sleep_ms, 250);
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

    fn lifecycle_transition_request(kind: u8) -> FfiPbftManagerLifecycleTransitionRequest {
        FfiPbftManagerLifecycleTransitionRequest {
            kind,
            target_period: 10,
            target_round: 4,
            has_network_next_voting_step: false,
            network_next_voting_step: 0,
        }
    }

    fn startup_fact() -> TestPbftManagerStartupFact {
        TestPbftManagerStartupFact {
            current_period: 10,
            cacti_active_at_chain_size: true,
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
            cacti_block: 1,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            deadline_ms: 1_000,
            polling_interval_ms: 100,
        }
    }

    fn service_config(cacti_block: u64) -> FfiPbftServiceConfig {
        let startup = startup_fact();
        FfiPbftServiceConfig {
            genesis_lambda_ms: startup.genesis_lambda_ms,
            cacti_lambda_max_ms: startup.cacti_lambda_max_ms,
            cacti_lambda_default_ms: startup.cacti_lambda_default_ms,
            cacti_block,
            max_exponential_lambda_ms: startup.max_exponential_lambda_ms,
            max_steps: startup.max_steps,
            deadline_ms: startup.deadline_ms,
            polling_interval_ms: startup.polling_interval_ms,
            report_malicious_behaviour: true,
            magnolia_activation_period: 0,
        }
    }

    #[test]
    fn pbft_service_rejects_live_sessions_until_bootstrap_completes() {
        let path = unique_temp_dir("rustaxa_bridge_pbft_service_bootstrap_phase");
        let storage = create_storage(path.to_str().expect("UTF-8 path")).unwrap();
        let service = create_pbft_service_from_storage(&storage, service_config(1)).unwrap();

        pbft_manager_runtime_begin_session(&service, fact(STATE_VALUE_PROPOSAL));
        assert_eq!(
            pbft_manager_runtime_session_next(&service).error_code,
            "PBFT_MANAGER_RUNTIME_SESSION_NOT_STARTED"
        );
        pbft_manager_runtime_begin_proposal_session_with_hash(
            &service,
            proposal_fact(),
            Some([0x22; 32]),
        );
        assert_eq!(
            pbft_manager_proposal_session_next(&service).error_code,
            "PBFT_MANAGER_PROPOSAL_SESSION_NOT_STARTED"
        );
        crate::pbft_sync::pbft_manager_runtime_begin_pbft_sync_admission(
            &service,
            crate::ffi::rustaxa_ffi::PbftSyncAdmissionInitialFact {
                block_period: 1,
                block_prev_hash: [0; 32],
                chain_last_hash: [0; 32],
                chain_last_period: 0,
                block_in_chain: false,
                dag_transaction_hashes: Vec::new(),
                period_data_transaction_hashes: Vec::new(),
                extra_data_required: false,
                extra_data_present: false,
                extra_data_pillar_block_hash_present: false,
                pillar_votes_required: false,
                pillar_votes_present: false,
                previous_cert_votes_present: true,
                previous_cert_first_vote_has_weight: false,
            },
        );
        assert_eq!(
            crate::pbft_sync::pbft_manager_runtime_pbft_sync_admission_next(&service).error_code,
            "PBFT_SYNC_ADMISSION_SESSION_NOT_STARTED"
        );
        pbft_manager_runtime_begin_pbft_sync_queue_drain(&service);
        let blocked_drain = pbft_manager_runtime_pbft_sync_queue_drain_next(&service, 1, 1);
        assert!(!blocked_drain.can_continue);
        assert_eq!(
            blocked_drain.error_code,
            "PBFT_SERVICE_BOOTSTRAP_INCOMPLETE"
        );

        pbft_service_complete_bootstrap(&service).unwrap();
        pbft_manager_runtime_begin_session(&service, fact(STATE_VALUE_PROPOSAL));
        assert!(pbft_manager_runtime_session_next(&service).can_continue);
        pbft_manager_runtime_begin_proposal_session_with_hash(
            &service,
            proposal_fact(),
            Some([0x22; 32]),
        );
        assert!(pbft_manager_proposal_session_next(&service)
            .error_code
            .is_empty());
        crate::pbft_sync::pbft_manager_runtime_begin_pbft_sync_admission(
            &service,
            crate::ffi::rustaxa_ffi::PbftSyncAdmissionInitialFact {
                block_period: 1,
                block_prev_hash: [0; 32],
                chain_last_hash: [0; 32],
                chain_last_period: 0,
                block_in_chain: false,
                dag_transaction_hashes: Vec::new(),
                period_data_transaction_hashes: Vec::new(),
                extra_data_required: false,
                extra_data_present: false,
                extra_data_pillar_block_hash_present: false,
                pillar_votes_required: false,
                pillar_votes_present: false,
                previous_cert_votes_present: true,
                previous_cert_first_vote_has_weight: false,
            },
        );
        assert!(
            crate::pbft_sync::pbft_manager_runtime_pbft_sync_admission_next(&service).has_check
        );
        pbft_manager_runtime_begin_pbft_sync_queue_drain(&service);
        let ready_drain = pbft_manager_runtime_pbft_sync_queue_drain_next(&service, 1, 1);
        assert!(ready_drain.can_continue);
        assert_ne!(ready_drain.error_code, "PBFT_SERVICE_BOOTSTRAP_INCOMPLETE");
    }

    fn dynamic_lambda_fact(finalized_period: u64) -> FfiPbftDynamicLambdaFact {
        FfiPbftDynamicLambdaFact {
            dynamic_lambda_active: true,
            finalized_period,
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

    fn runtime_for_finalization_test(name: &str) -> (PathBuf, Box<BridgePbftService>) {
        let temp_dir = unique_temp_dir(name);
        let storage = create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
            .expect("storage should initialize");
        let mut startup = startup_fact();
        startup.cacti_active_at_chain_size = false;
        let runtime = create_pbft_manager_runtime_from_storage(&storage, startup)
            .expect("runtime should initialize");
        runtime
            .chain()
            .write()
            .expect("PBFT chain lock should remain healthy")
            .state = PbftChain::new(rustaxa_consensus::pbft_chain::PbftChainHead {
            head_hash: ethereum_types::H256::from([8; 32]),
            size: 9,
            non_empty_size: 5,
            last_pbft_block_hash: ethereum_types::H256::from([3; 32]),
            last_non_null_pbft_dag_anchor_hash: ethereum_types::H256::from([2; 32]),
        })
        .expect("test PBFT chain head should be valid");
        (temp_dir, runtime)
    }

    fn finalized_dag_period_data_rlp_with_counts(
        pivot: H256,
        unique_transactions: usize,
        dag_transaction_ref_counts: &[usize],
    ) -> Vec<u8> {
        let mut pbft_block = rlp::RlpStream::new_list(8);
        pbft_block.append(&H256::from_low_u64_be(1));
        pbft_block.append(&pivot);
        pbft_block.append(&H256::from_low_u64_be(2));
        pbft_block.append(&H256::from_low_u64_be(3));
        pbft_block.append(&10_u64);
        pbft_block.append(&123_u64);
        pbft_block.begin_list(0);
        pbft_block.append(&vec![0_u8; 65]);

        let ordered_transaction_hashes = rlp::RlpStream::new_list(0);
        let mut transaction_indexes = rlp::RlpStream::new_list(dag_transaction_ref_counts.len());
        for count in dag_transaction_ref_counts {
            transaction_indexes.begin_list(*count);
            for index in 0..*count {
                transaction_indexes.append(&index);
            }
        }
        let compact_blocks = rlp::RlpStream::new_list(0);
        let mut bundle = rlp::RlpStream::new_list(3);
        bundle.append_raw(&ordered_transaction_hashes.out(), 1);
        bundle.append_raw(&transaction_indexes.out(), 1);
        bundle.append_raw(&compact_blocks.out(), 1);

        let mut period_data = rlp::RlpStream::new_list(4);
        period_data.append_raw(&pbft_block.out(), 1);
        period_data.append_empty_data();
        period_data.append_raw(&bundle.out(), 1);
        period_data.begin_list(unique_transactions);
        for _ in 0..unique_transactions {
            period_data.append_empty_data();
        }
        period_data.out().to_vec()
    }

    fn finalized_dag_period_data_rlp(pivot: H256) -> Vec<u8> {
        finalized_dag_period_data_rlp_with_counts(pivot, 0, &[])
    }

    fn empty_finalized_dag_period_data_rlp() -> Vec<u8> {
        finalized_dag_period_data_rlp(H256::zero())
    }

    #[test]
    fn manager_runtime_projects_native_owned_drain_compatibility_flags() {
        let (_temp_dir, runtime) =
            runtime_for_finalization_test("rustaxa_bridge_pbft_manager_owned_drain_projection");
        let state = finalization_executor_state_from_boundary(
            rustaxa_consensus::pbft_manager::PbftFinalizationExecutorBoundary {
                cleared_anchor_dag_cache: true,
                has_snapshot: true,
                expired_dag_hashes: vec![H256::repeat_byte(9)],
                refresh_dag_counters: true,
                next_step: rustaxa_consensus::pbft_finalize::PbftFinalizationRuntimeStep {
                    runtime_status: PbftFinalizationRuntimeStatus::Active,
                    has_action: true,
                    action: Some(PbftFinalizationRuntimeAction::FinalizeFinalChain),
                    action_index: 7,
                    complete: false,
                    error_code: String::new(),
                },
                snapshot: runtime.manager_state().state.snapshot(),
                error_code: String::new(),
            },
        );

        assert!(state.cleared_anchor_dag_cache);
        assert!(state.has_snapshot);
        assert_eq!(state.expired_dag_hashes[0].hash, [9; 32]);
        assert!(state.refresh_dag_counters);
        assert_eq!(state.cursor, 7);
        assert_eq!(
            state.action,
            PbftFinalizationRuntimeAction::FinalizeFinalChain.as_u8()
        );
    }

    #[test]
    fn bridge_runtime_owns_period_data_queue_metadata() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_period_data_queue");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .0
                .pbft()
                .write_manager_field(2, 500)
                .expect("lambda seed should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should initialize");

            let first = pbft_manager_runtime_period_data_queue_push(
                &mut runtime,
                11,
                1,
                queue_hash(0x11),
                queue_hash(0xa1),
                queue_hash(0xb1),
                queue_hash(0xe1),
                queue_transaction_hashes(&[0xf1]),
                queue_pillar_vote_rlps(&[0xa1]),
                queue_transaction_rlps(&[0xb1]),
                queue_pbft_vote_rlps(&[]),
                queue_transaction_hashes(&[0xc1]),
                queue_transaction_hashes(&[0xd1]),
                queue_transaction_identities(&[0xd1]),
                false,
                false,
                false,
                false,
                false,
                0,
                queue_pbft_vote_rlps(&[0x81]),
            )
            .expect("first push should succeed");
            assert!(first.accepted);
            assert!(!first.clear_existing);
            let first_snapshot =
                pbft_manager_runtime_period_data_queue_snapshot(&runtime, 5, 1, queue_hash(0xee));
            assert_eq!(first_snapshot.period, 1);
            assert_eq!(first_snapshot.syncing_period, 5);
            assert_eq!(first_snapshot.size, 1);

            let second = pbft_manager_runtime_period_data_queue_push(
                &mut runtime,
                22,
                2,
                queue_hash(0x22),
                queue_hash(0xa2),
                queue_hash(0xb2),
                queue_hash(0xe2),
                queue_transaction_hashes(&[0xf2]),
                queue_pillar_vote_rlps(&[0xa2]),
                queue_transaction_rlps(&[0xb2]),
                queue_pbft_vote_rlps(&[0x92]),
                queue_transaction_hashes(&[0xc2]),
                queue_transaction_hashes(&[0xd2]),
                queue_transaction_identities(&[0xd2]),
                true,
                false,
                true,
                true,
                false,
                0,
                queue_pbft_vote_rlps(&[0x82]),
            )
            .expect("second push should succeed");
            assert!(second.accepted);
            let second_snapshot =
                pbft_manager_runtime_period_data_queue_snapshot(&runtime, 5, 1, queue_hash(0xee));
            assert_eq!(second_snapshot.last_block_hash_or_chain, queue_hash(0x22));

            let first_pop = pbft_manager_runtime_period_data_queue_pop(&mut runtime)
                .expect("first pop should produce a handoff");
            assert_eq!(first_pop.entry_id, 11);
            assert_eq!(first_pop.entry_period, 1);
            assert_eq!(first_pop.block_hash, queue_hash(0x11));
            assert_eq!(first_pop.cert_vote_rlps[0].vote_rlp, vec![0x92, 0x93]);
            assert!(!first_pop.use_last_block_cert_votes);
            assert_eq!(first_pop.next_entry_id, 22);
            assert_eq!(first_pop.effective_size, 1);

            let second_pop = pbft_manager_runtime_period_data_queue_pop(&mut runtime)
                .expect("second pop should produce a handoff");
            assert_eq!(second_pop.entry_id, 22);
            assert_eq!(second_pop.cert_vote_rlps[0].vote_rlp, vec![0x82, 0x83]);
            assert!(second_pop.use_last_block_cert_votes);
            let empty_snapshot =
                pbft_manager_runtime_period_data_queue_snapshot(&runtime, 5, 1, queue_hash(0xee));
            assert!(empty_snapshot.empty);
            assert_eq!(empty_snapshot.period, 0);

            pbft_manager_runtime_period_data_queue_push(
                &mut runtime,
                33,
                6,
                queue_hash(0x33),
                queue_hash(0xa3),
                queue_hash(0xb3),
                queue_hash(0xe3),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
                false,
                false,
                false,
                false,
                4,
                queue_pbft_vote_rlps(&[0x83]),
            )
            .expect("empty queue backfill should succeed");
            let removed = pbft_manager_runtime_period_data_queue_clean_old_data(&mut runtime, 7);
            assert_eq!(removed.len(), 1);
            assert_eq!(removed[0].entry_id, 33);
            let cleaned_snapshot =
                pbft_manager_runtime_period_data_queue_snapshot(&runtime, 5, 1, queue_hash(0xee));
            assert!(cleaned_snapshot.empty);

            pbft_manager_runtime_period_data_queue_clear(&mut runtime);
            let cleared_snapshot =
                pbft_manager_runtime_period_data_queue_snapshot(&runtime, 5, 1, queue_hash(0xee));
            assert_eq!(cleared_snapshot.period, 0);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_owns_state_action_effect_session() {
        let mut finish_polling_fact = state_fact(4);
        finish_polling_fact.current_round_lambda_ms = 1_000;
        finish_polling_fact.has_current_round_soft_value = true;
        finish_polling_fact.has_previous_round_next_null = true;
        let temp_path = unique_temp_dir("rustaxa_bridge_pbft_manager_state_action_runtime");
        let storage =
            crate::storage::create_storage(temp_path.to_str().expect("utf-8 temp path")).unwrap();
        let mut startup = startup_fact();
        startup.cacti_active_at_chain_size = false;
        let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup).unwrap();

        pbft_manager_runtime_begin_state_action_effect_session(&mut runtime, finish_polling_fact);

        let first = pbft_manager_runtime_state_action_effect_session_next(&mut runtime);
        assert_eq!(first.status, STATE_ACTION_SESSION_ACTIVE);
        assert!(first.has_effect);
        assert_eq!(
            first.effect.intent,
            STATE_ACTION_NEXT_VOTE_CURRENT_SOFT_VALUE
        );
        assert!(first.effect.request_proposed_block_sidecar);
        assert_eq!(first.effect.proposed_block_sidecar_hash, [0x55; 32]);
        assert_eq!(first.effect.proposed_block_sidecar_period, 10);

        let second = pbft_manager_runtime_state_action_effect_session_report(
            &mut runtime,
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

        let done = pbft_manager_runtime_state_action_effect_session_report(
            &mut runtime,
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
        let mut runtime = runtime_for_startup("rustaxa_bridge_pbft_manager_proposal_session");
        pbft_manager_runtime_begin_proposal_session_with_hash(
            &runtime,
            proposal_fact(),
            Some([0x22; 32]),
        );

        let request = pbft_manager_proposal_session_next(&mut runtime);
        assert_eq!(request.action, PROPOSAL_ACTION_REQUEST_DAG_ORDER);
        assert_eq!(request.status, PROPOSAL_STATUS_ACTIVE);
        assert_eq!(request.requested_anchor_hash, [0x03; 32]);

        let build = pbft_manager_proposal_session_report_dag_order(
            &mut runtime,
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
        let mut runtime = runtime_for_startup("rustaxa_bridge_pbft_manager_runtime_sleep");
        let applied = pbft_manager_runtime_execute_lifecycle_transition(
            &mut runtime,
            lifecycle_transition_request(TRANSITION_FILTER),
        )
        .expect("runtime transition should apply");
        assert_eq!(applied.status, TRANSITION_STORAGE_STATUS_APPLIED);

        let snapshot = pbft_manager_runtime_snapshot(&runtime);
        assert_eq!(snapshot.next_step_time_ms, 200);
        let near_elapsed_ms = (snapshot.next_step_time_ms - 1)
            .try_into()
            .expect("snapshot value should fit i64");
        let wait = plan_pbft_manager_runtime_sleep_until_next_step(&runtime, near_elapsed_ms);
        assert!(wait.accepted);
        assert!(wait.should_sleep);
        assert_eq!(wait.sleep_ms, 1);
        assert!(wait.error_code.is_empty());
        assert_eq!(wait.step, snapshot.step);

        let reached_elapsed_ms = snapshot
            .next_step_time_ms
            .try_into()
            .expect("snapshot value should fit i64");
        let reached = plan_pbft_manager_runtime_sleep_until_next_step(&runtime, reached_elapsed_ms);
        assert!(reached.accepted);
        assert!(!reached.should_sleep);
        assert_eq!(reached.sleep_ms, 0);
        assert!(reached.error_code.is_empty());
        assert_eq!(reached.step, snapshot.step);
    }

    #[test]
    fn bridge_runtime_executes_lifecycle_transition_from_owned_cursor() {
        let mut runtime = runtime_for_startup("rustaxa_bridge_lifecycle_transition");
        let before = runtime.manager_state().state.snapshot();
        let result = pbft_manager_runtime_execute_lifecycle_transition(
            &mut runtime,
            lifecycle_transition_request(PbftManagerTransitionKind::ToFilter.as_u8()),
        )
        .unwrap();

        assert_eq!(result.status, TRANSITION_STORAGE_STATUS_APPLIED);
        assert_eq!(result.snapshot.period, before.period);
        assert_eq!(result.snapshot.round, before.round);
        assert_eq!(result.snapshot.step, before.step + 1);
        assert_eq!(result.snapshot.state, 1);
        assert!(!result.remove_cert_voted_sidecar);
        assert!(result.error_code.is_empty());
    }

    #[test]
    fn bridge_runtime_reset_clears_native_own_votes_without_sidecar_command() {
        let mut runtime = runtime_for_startup("rustaxa_bridge_lifecycle_clear_own_votes");
        native_service_storage(&runtime)
            .pbft()
            .write_own_verified_vote(H256::from_low_u64_be(71), &[0xC1])
            .unwrap();

        let result = pbft_manager_runtime_execute_lifecycle_transition(
            &mut runtime,
            lifecycle_transition_request(TRANSITION_RESET),
        )
        .unwrap();

        assert_eq!(result.status, TRANSITION_STORAGE_STATUS_APPLIED);
        assert!(native_service_storage(&runtime)
            .pbft()
            .own_verified_vote_hashes()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn bridge_runtime_advance_requires_matching_committed_reset() {
        let mut runtime = runtime_for_startup("rustaxa_bridge_runtime_advance_provenance");
        let before = runtime.manager_state().state.snapshot();
        assert!(!pbft_manager_runtime_plan_advance_period_after_reset(&runtime, 9).accepted);
        assert_eq!(runtime.manager_state().state.snapshot(), before);

        let mut request = lifecycle_transition_request(TRANSITION_RESET);
        request.target_period = 10;
        request.target_round = 1;
        let reset =
            pbft_manager_runtime_execute_lifecycle_transition(&mut runtime, request).unwrap();
        assert_eq!(reset.status, TRANSITION_STORAGE_STATUS_APPLIED);
        assert!(!pbft_manager_runtime_plan_advance_period_after_reset(&runtime, 9).accepted);

        let mut advance_request = lifecycle_transition_request(TRANSITION_RESET);
        advance_request.target_period = 11;
        advance_request.target_round = 1;
        let advance_reset =
            pbft_manager_runtime_execute_lifecycle_transition(&mut runtime, advance_request)
                .unwrap();
        assert_eq!(advance_reset.status, TRANSITION_STORAGE_STATUS_APPLIED);
        let committed = runtime.manager_state().state.snapshot();

        assert!(!pbft_manager_runtime_plan_advance_period_after_reset(&runtime, 0).accepted);
        assert!(!pbft_manager_runtime_plan_advance_period_after_reset(&runtime, 9).accepted);
        assert_eq!(runtime.manager_state().state.snapshot(), committed);
        assert!(pbft_manager_runtime_plan_advance_period_after_reset(&runtime, 10).accepted);
        let applied = pbft_manager_runtime_apply_period_advance(&mut runtime, 11);
        assert_eq!(applied.status, 0);
        assert!(!pbft_manager_runtime_plan_advance_period_after_reset(&runtime, 10).accepted);
    }

    #[test]
    fn bridge_runtime_rejects_unneeded_network_step_presence_without_mutation() {
        let mut runtime = runtime_for_startup("rustaxa_bridge_lifecycle_network_presence");
        let before = runtime.manager_state().state.snapshot();
        let mut request = lifecycle_transition_request(TRANSITION_FILTER);
        request.has_network_next_voting_step = true;
        request.network_next_voting_step = 7;
        let result =
            pbft_manager_runtime_execute_lifecycle_transition(&mut runtime, request).unwrap();
        assert_eq!(result.status, TRANSITION_STORAGE_STATUS_REJECTED);
        assert_eq!(runtime.manager_state().state.snapshot(), before);
        assert_eq!(
            result.error_code,
            "PBFT_MANAGER_TRANSITION_NETWORK_STEP_PRESENCE_MISMATCH"
        );
    }

    #[test]
    fn bridge_runtime_rejects_missing_cacti_lambda_without_mutation() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_startup_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
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
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");
            storage
                .0
                .pbft()
                .write_manager_status(2, true)
                .expect("soft next status should persist");
            storage
                .0
                .pbft()
                .write_manager_status(3, true)
                .expect("null next status should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");
            save_own_verified_vote(
                &storage.0,
                PbftVoteStorageRecord {
                    hash: H256::from(own_hash),
                    vote_rlp: vec![0xC0],
                },
            )
            .expect("own vote should persist after the service has restored");
            let before = pbft_manager_runtime_snapshot(&runtime);
            let result = pbft_manager_runtime_execute_lifecycle_transition(
                &mut runtime,
                lifecycle_transition_request(TRANSITION_RESET),
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
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");
            storage
                .0
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK, true)
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
                .0
                .pbft()
                .write_manager_field(2, 1_500)
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
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
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
    fn bridge_runtime_records_broadcast_counters() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_broadcast_counters");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
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
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");
            let dag_hash = [0xDA; 32];
            let pbft_hash = [0xBE; 32];
            storage
                .0
                .dag()
                .write_period(H256::from(dag_hash), 12, 4)
                .expect("DAG period should persist");
            storage
                .0
                .period()
                .write_pbft_period(H256::from(pbft_hash), 9)
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
    fn bridge_runtime_owns_pbft_sync_queue_drain_session() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_queue_drain");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .0
                .pbft()
                .write_manager_field(2, 500)
                .expect("lambda seed should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should initialize");

            pbft_manager_runtime_begin_pbft_sync_queue_drain(&mut runtime);
            let clean = pbft_manager_runtime_pbft_sync_queue_drain_next(&mut runtime, 2, 10);
            assert_eq!(clean.action, PbftSyncQueueDrainAction::CleanOldData.as_u8());
            assert_eq!(clean.clean_before_period, 10);
            let report = pbft_manager_runtime_pbft_sync_queue_drain_report(
                &mut runtime,
                FfiPbftSyncQueueDrainReport {
                    action: clean.action,
                    success: true,
                    accepted_period_data: false,
                },
            );
            assert_eq!(report.status, 0);
            assert!(report.can_continue);

            let pop = pbft_manager_runtime_pbft_sync_queue_drain_next(&mut runtime, 1, 10);
            assert_eq!(pop.action, PbftSyncQueueDrainAction::PopAndProcess.as_u8());
            let report = pbft_manager_runtime_pbft_sync_queue_drain_report(
                &mut runtime,
                FfiPbftSyncQueueDrainReport {
                    action: pop.action,
                    success: true,
                    accepted_period_data: true,
                },
            );
            assert!(report.can_continue);

            let push = pbft_manager_runtime_pbft_sync_queue_drain_next(&mut runtime, 1, 10);
            assert_eq!(push.action, PbftSyncQueueDrainAction::PushAccepted.as_u8());
            let report = pbft_manager_runtime_pbft_sync_queue_drain_report(
                &mut runtime,
                FfiPbftSyncQueueDrainReport {
                    action: push.action,
                    success: true,
                    accepted_period_data: false,
                },
            );
            assert!(report.can_continue);

            let update = pbft_manager_runtime_pbft_sync_queue_drain_next(&mut runtime, 1, 11);
            assert_eq!(
                update.action,
                PbftSyncQueueDrainAction::UpdateSyncState.as_u8()
            );
            let report = pbft_manager_runtime_pbft_sync_queue_drain_report(
                &mut runtime,
                FfiPbftSyncQueueDrainReport {
                    action: update.action,
                    success: true,
                    accepted_period_data: false,
                },
            );
            assert!(report.can_continue);

            let stop = pbft_manager_runtime_pbft_sync_queue_drain_next(&mut runtime, 0, 11);
            assert_eq!(stop.action, PbftSyncQueueDrainAction::Stop.as_u8());
            assert_eq!(stop.status, 1);
            assert!(!stop.can_continue);

            pbft_manager_runtime_begin_pbft_sync_queue_drain(&mut runtime);
            let restarted = pbft_manager_runtime_pbft_sync_queue_drain_next(&mut runtime, 0, 12);
            assert_eq!(
                restarted.action,
                PbftSyncQueueDrainAction::CleanOldData.as_u8()
            );
            assert_eq!(restarted.clean_before_period, 12);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_plans_finalization_dynamic_lambda_from_owned_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_manager_runtime_finalization_lambda");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");
            storage
                .0
                .metadata()
                .write_period_lambda(19, 1_234)
                .expect("period lambda should persist");
            let runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let plan = pbft_manager_runtime_plan_finalization_dynamic_lambda(
                &runtime,
                dynamic_lambda_fact(20),
            )
            .expect("runtime dynamic-lambda planner should run");
            let missing = pbft_manager_runtime_plan_finalization_dynamic_lambda(
                &runtime,
                dynamic_lambda_fact(1),
            )
            .expect("runtime missing dynamic-lambda planner should run");

            assert_eq!(plan.status, PbftFinalizationStatus::Accepted.as_u8());
            assert!(plan.apply_dynamic_lambda_update);
            assert_eq!(plan.period_lambda, 1_500);
            assert_eq!(plan.blocks_per_year, 9_275_294);
            assert_eq!(plan.rounds_count_dynamic_lambda, 0);
            assert_eq!(plan.dynamic_lambda, 1_490);
            assert!(plan.last_saved_period_lambda_found);
            assert_eq!(plan.last_saved_period_lambda, 1_234);
            assert!(!missing.last_saved_period_lambda_found);
            assert_eq!(missing.last_saved_period_lambda, 0);
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
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");
            save_cert_voted_block_in_round_storage(storage.0.as_ref(), 3, &[0xC0])
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
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
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
                runtime
                    .manager_state()
                    .state
                    .cached_anchor_dag_order_count(),
                2
            );

            let clear_snapshot = runtime
                .manager_state()
                .state
                .clear_cached_anchor_dag_order();
            assert_eq!(
                clear_snapshot.status,
                rustaxa_consensus::pbft_manager::PbftManagerStartupRestoreStatus::Ready
            );
            assert_eq!(
                runtime
                    .manager_state()
                    .state
                    .cached_anchor_dag_order_count(),
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
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");
            create_pillar_test_service_from_storage(&storage)
                .expect("pillar runtime should initialize")
                .pbft_service_pillar_apply_own_vote(vec![0xC0])
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
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");
            let period_data = empty_finalized_dag_period_data_rlp();
            storage
                .0
                .period()
                .write(12, &period_data.clone())
                .expect("period data should persist");
            storage
                .0
                .metadata()
                .write_period_lambda(11, 1_234)
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
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");
            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");

            let mut request = lifecycle_transition_request(TRANSITION_RESET);
            request.target_period = 13;
            request.target_round = 1;
            let transition =
                pbft_manager_runtime_execute_lifecycle_transition(&mut runtime, request)
                    .expect("reset transition should apply");
            assert_eq!(transition.status, TRANSITION_STORAGE_STATUS_APPLIED);
            assert_eq!(transition.snapshot.current_round_lambda_ms, 1_500);

            let advance = pbft_manager_runtime_plan_advance_period_after_reset(&runtime, 12);
            assert!(advance.accepted);
            assert_eq!(advance.finalized_chain_size, 12);
            assert_eq!(advance.new_period, 13);
            assert_eq!(
                advance.actions,
                vec![
                    ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND,
                    ADVANCE_ACTION_RESET_CURRENT_ROUND_TIMER,
                    ADVANCE_ACTION_RESET_REWARD_VOTE_COUNTERS,
                    ADVANCE_ACTION_RESET_PERIOD_TIMER,
                    ADVANCE_ACTION_UPDATE_WALLET_ELIGIBILITY,
                    ADVANCE_ACTION_CLEANUP_PERIOD_STATE,
                ]
            );

            let report = validate_pbft_manager_advance_period_action_report(
                &advance,
                FfiPbftManagerAdvancePeriodActionReport {
                    action_index: 0,
                    action: ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND,
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
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");

            let mut runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");
            let before = pbft_manager_runtime_snapshot(&runtime);
            let result = pbft_manager_runtime_execute_lifecycle_transition(
                &mut runtime,
                lifecycle_transition_request(255),
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
            assert_eq!(pbft_queries(&storage).get_pbft_mgr_field(0).unwrap(), 1);
        }

        let _ = fs::remove_dir_all(temp_dir);
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

    #[test]
    fn bridge_planner_carriers_preserve_normal_path_sentinels() {
        let finalization_wait_fact: PbftManagerFinalizationWaitFact =
            FfiPbftManagerFinalizationWaitFact {
                pbft_chain_size: 21,
                final_chain_last_block: 15,
                delegation_delay: 5,
                polling_interval_ms: 123,
            }
            .into();
        assert_eq!(
            (
                finalization_wait_fact.pbft_chain_size,
                finalization_wait_fact.final_chain_last_block,
                finalization_wait_fact.delegation_delay,
                finalization_wait_fact.polling_interval_ms,
            ),
            (21, 15, 5, 123)
        );
        let finalization_wait_plan: FfiPbftManagerFinalizationWaitPlan =
            PbftManagerFinalizationWaitPlan {
                accepted: true,
                should_wait: true,
                sleep_ms: 123,
                error_code: "WAIT_SENTINEL".to_string(),
            }
            .into();
        assert!(finalization_wait_plan.accepted);
        assert!(finalization_wait_plan.should_wait);
        assert_eq!(finalization_wait_plan.sleep_ms, 123);
        assert_eq!(finalization_wait_plan.error_code, "WAIT_SENTINEL");

        let eligible_wait_fact: PbftManagerEligibleWalletPeriodWaitFact =
            FfiPbftManagerEligibleWalletPeriodWaitFact {
                eligible_wallet_period: 31,
                pbft_chain_size: 32,
                polling_interval_ms: 456,
            }
            .into();
        assert_eq!(
            (
                eligible_wait_fact.eligible_wallet_period,
                eligible_wait_fact.pbft_chain_size,
                eligible_wait_fact.polling_interval_ms,
            ),
            (31, 32, 456)
        );
        let eligible_wait_plan: FfiPbftManagerEligibleWalletPeriodWaitPlan =
            PbftManagerEligibleWalletPeriodWaitPlan {
                should_wait: true,
                sleep_ms: 456,
            }
            .into();
        assert!(eligible_wait_plan.should_wait);
        assert_eq!(eligible_wait_plan.sleep_ms, 456);

        let block_fact: PbftManagerBlockValidationFact = FfiPbftManagerBlockValidationFact {
            block_hash: [0x11; 32],
            period: 12,
            pivot_hash: [0x22; 32],
            pivot_is_null: false,
            dag_order_cached: true,
            dag_order_required: true,
            pillar_block_required: false,
            dag_weight_check_required: true,
            pbft_chain_status: 1,
            final_chain_hash_status: 3,
            reward_votes_status: 4,
            extra_data_status: 2,
            pillar_block_status: 4,
            dag_order_status: 1,
            dag_weight_status: 0,
        }
        .into();
        assert_eq!(block_fact.block_hash, ethereum_types::H256([0x11; 32]));
        assert_eq!(block_fact.pivot_hash, ethereum_types::H256([0x22; 32]));
        assert_eq!(
            block_fact.final_chain_hash_status,
            PbftManagerBlockValidationFactStatus::Missing
        );

        let candidate_fact: PbftManagerCandidateAdmissionFact =
            FfiPbftManagerCandidateAdmissionFact {
                period: 13,
                block_hash: [0x33; 32],
                lookup_performed: true,
                proposed_block_found: true,
                proposed_block_already_valid: false,
                validation_status: 1,
            }
            .into();
        assert_eq!(candidate_fact.period, 13);
        assert_eq!(
            candidate_fact.validation_status,
            PbftManagerCandidateAdmissionValidationStatus::Valid
        );

        let leader_fact: PbftManagerLeaderCandidateInputFact =
            leader_candidate_input(0x44, 0x55, 1).into();
        assert_eq!(leader_fact.vote_hash, ethereum_types::H256([0x44; 32]));
        assert_eq!(
            leader_fact.block_validation_status,
            PbftManagerLeaderBlockValidationStatus::Validated
        );

        let block_plan: FfiPbftManagerBlockValidationPlan =
            PbftManagerBlockValidationPlan {
                action:
                    rustaxa_consensus::pbft_manager::PbftManagerBlockValidationAction::WaitForFinalization,
                status:
                    rustaxa_consensus::pbft_manager::PbftManagerBlockValidationStatus::FinalChainHashMissing,
                next_check:
                    rustaxa_consensus::pbft_manager::PbftManagerBlockValidationNextCheck::ValidateFinalChainHash,
                error_code: "BLOCK_SENTINEL",
            }
            .into();
        assert_eq!(
            (block_plan.action, block_plan.status, block_plan.next_check),
            (3, 3, 1)
        );
        assert_eq!(block_plan.error_code, "BLOCK_SENTINEL");

        let candidate_plan: FfiPbftManagerCandidateAdmissionPlan =
            PbftManagerCandidateAdmissionPlan {
                action:
                    rustaxa_consensus::pbft_manager::PbftManagerCandidateAdmissionAction::Accept,
                status:
                    rustaxa_consensus::pbft_manager::PbftManagerCandidateAdmissionStatus::AcceptedNewlyValidated,
                mark_valid: true,
                error_code: "CANDIDATE_SENTINEL",
            }
            .into();
        assert_eq!((candidate_plan.action, candidate_plan.status), (2, 3));
        assert!(candidate_plan.mark_valid);
        assert_eq!(candidate_plan.error_code, "CANDIDATE_SENTINEL");

        let leader_plan: FfiPbftManagerLeaderCandidatePlan = PbftManagerLeaderCandidatePlan {
            status: rustaxa_consensus::pbft_manager::PbftManagerLeaderSelectionStatus::Selected,
            selected: true,
            selected_vote_hash: ethereum_types::H256([0x66; 32]),
            selected_block_hash: ethereum_types::H256([0x77; 32]),
            selected_period: 14,
            selected_from_null_anchor: false,
            valid_blocks: vec![PbftManagerLeaderValidBlockCommand {
                period: 14,
                block_hash: ethereum_types::H256([0x88; 32]),
            }],
            error_code: "LEADER_SENTINEL",
        }
        .into();
        assert_eq!(leader_plan.status, 0);
        assert!(leader_plan.selected);
        assert_eq!(leader_plan.selected_vote_hash, [0x66; 32]);
        assert_eq!(leader_plan.selected_block_hash, [0x77; 32]);
        assert_eq!(leader_plan.selected_period, 14);
        assert!(!leader_plan.selected_from_null_anchor);
        assert_eq!(leader_plan.valid_blocks.len(), 1);
        assert_eq!(leader_plan.valid_blocks[0].period, 14);
        assert_eq!(leader_plan.valid_blocks[0].block_hash, [0x88; 32]);
        assert_eq!(leader_plan.error_code, "LEADER_SENTINEL");
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
}
