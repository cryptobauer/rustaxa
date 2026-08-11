//! Bridge wrapper for PBFT manager daemon-tick runtime planning.
//!
//! C++ supplies the current manager state and live-shell facts for one daemon
//! tick. Rust owns the ordered action cursor for that tick, while C++ executes
//! existing manager methods and reports each action result before the session
//! advances.

use crate::dag_transaction_service::BridgeDagTransactionService;
use crate::ffi::rustaxa_ffi::{
    BlockPeriodLookup as FfiBlockPeriodLookup, PbftCertVoteRlp as FfiPbftCertVoteRlp,
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
    PbftManagerProposalInitialFact as FfiPbftManagerProposalInitialFact,
    PbftManagerProposalSessionStep as FfiPbftManagerProposalSessionStep,
    PbftManagerProposalWalletFact as FfiPbftManagerProposalWalletFact,
    PbftManagerRuntimeActionReport as FfiPbftManagerRuntimeActionReport,
    PbftManagerRuntimeSessionStep as FfiPbftManagerRuntimeSessionStep,
    PbftManagerRuntimeSnapshot as FfiPbftManagerRuntimeSnapshot,
    PbftManagerRuntimeTickFact as FfiPbftManagerRuntimeTickFact,
    PbftManagerSleepPlan as FfiPbftManagerSleepPlan,
    PbftManagerStartupReplayRangeFact as FfiPbftManagerStartupReplayRangeFact,
    PbftManagerStartupReplayRangePlan as FfiPbftManagerStartupReplayRangePlan,
    PbftManagerStateActionEffect as FfiPbftManagerStateActionEffect,
    PbftManagerStateActionEffectReport as FfiPbftManagerStateActionEffectReport,
    PbftManagerStateActionFact as FfiPbftManagerStateActionFact,
    PbftManagerStateActionSessionStep as FfiPbftManagerStateActionSessionStep,
    PbftServiceConfig as FfiPbftServiceConfig, PbftSyncIngressStep as FfiPbftSyncIngressStep,
    PbftSyncQueueDrainReport as FfiPbftSyncQueueDrainReport,
    PbftSyncQueueDrainReportResult as FfiPbftSyncQueueDrainReportResult,
    PbftSyncQueueDrainStep as FfiPbftSyncQueueDrainStep,
    PeriodDataQueuePopPlan as FfiPeriodDataQueuePopPlan,
    PeriodDataQueuePushOutcome as FfiPeriodDataQueuePushOutcome,
    PeriodDataQueueSnapshot as FfiPeriodDataQueueSnapshot,
    PeriodDataQueueTransactionIdentity as FfiPeriodDataQueueTransactionIdentity,
    PeriodDataQueueTransactionPayload as FfiPeriodDataQueueTransactionPayload,
    PillarVoteRlpPayload as FfiPillarVoteRlpPayload,
    TransactionQueueAccountNonceFact as FfiTransactionQueueAccountNonceFact,
};
use crate::ffi::{BridgeFinalChain, BridgePbftService, BridgeStorage};
use crate::transaction_manager::bridge_to_service_account_nonce_facts;
use crate::verified_votes::{
    empty_slashing_transaction_effect, slashing_submitter_identity_to_domain,
    slashing_transaction_effect_to_ffi,
};
use anyhow::anyhow;
use rustaxa_consensus::dag::DagBlockPeriodStorageLookup;
use rustaxa_consensus::pbft_finalize::{
    plan_pbft_finalization_intent as plan_domain_pbft_finalization_intent, PbftDynamicLambdaConfig,
    PbftDynamicLambdaFact, PbftDynamicLambdaPlan, PbftFinalizationAnchor,
    PbftFinalizationCleanupIntent, PbftFinalizationIntentFact, PbftFinalizationPlan,
    PbftFinalizationPositionedHash, PbftFinalizationRuntimeAction, PbftFinalizationStatus,
    PbftFinalizationStorageWriteIntent, PbftFinalizationStorageWriteStage,
};
use rustaxa_consensus::pbft_manager::{
    plan_pbft_manager_broadcast as plan_domain_pbft_manager_broadcast,
    plan_pbft_manager_candidate_admission as plan_domain_pbft_manager_candidate_admission,
    plan_pbft_manager_eligible_wallet_period_wait as plan_domain_pbft_manager_eligible_wallet_period_wait,
    plan_pbft_manager_finalization_wait as plan_domain_pbft_manager_finalization_wait,
    plan_pbft_manager_leader_candidates as plan_domain_pbft_manager_leader_candidates,
    plan_pbft_manager_startup_replay_ranges as plan_domain_pbft_manager_startup_replay_ranges,
    report_pbft_manager_broadcast as report_domain_pbft_manager_broadcast,
    validate_pbft_manager_advance_period_action_report as validate_domain_pbft_manager_advance_period_action_report,
    PbftFinalizationExecutorStartRequest, PbftManagerAdvancePeriodActionReport,
    PbftManagerAdvancePeriodActionReportResult, PbftManagerAdvancePeriodPlan,
    PbftManagerBlockValidationFact, PbftManagerBlockValidationFactStatus,
    PbftManagerBlockValidationPlan, PbftManagerBroadcastAction, PbftManagerBroadcastFact,
    PbftManagerBroadcastPlan, PbftManagerBroadcastReport, PbftManagerBroadcastReportResult,
    PbftManagerBroadcastStatus, PbftManagerCandidateAdmissionFact,
    PbftManagerCandidateAdmissionPlan, PbftManagerCandidateAdmissionValidationStatus,
    PbftManagerEligibleWalletPeriodWaitFact, PbftManagerEligibleWalletPeriodWaitPlan,
    PbftManagerFinalizationWaitFact, PbftManagerFinalizationWaitPlan,
    PbftManagerLeaderBlockValidationStatus, PbftManagerLeaderCandidateInputFact,
    PbftManagerLeaderCandidatePlan, PbftManagerLeaderValidBlockCommand,
    PbftManagerLifecycleTransitionRequest, PbftManagerProposalAction,
    PbftManagerProposalInitialFact, PbftManagerProposalSessionStep, PbftManagerProposalStatus,
    PbftManagerProposalWalletFact, PbftManagerRuntimeAction, PbftManagerRuntimeActionReport,
    PbftManagerRuntimeActionResultCode, PbftManagerRuntimeSessionStep, PbftManagerRuntimeSnapshot,
    PbftManagerRuntimeStateCode, PbftManagerRuntimeStatus, PbftManagerRuntimeTickFact,
    PbftManagerSleepPlan, PbftManagerStartupReplayPeriod, PbftManagerStartupReplayRangeFact,
    PbftManagerStartupReplayRangePlan, PbftManagerStateActionEffect,
    PbftManagerStateActionEffectReport, PbftManagerStateActionEffectResultCode,
    PbftManagerStateActionFact, PbftManagerStateActionIntent, PbftManagerStateActionSessionStatus,
    PbftManagerStateActionSessionStep, PbftManagerTransitionKind,
};
use rustaxa_consensus::pbft_service::{
    PbftBlockValidationCandidate, PbftManagerLifecycleTransitionOutcome,
};
use rustaxa_consensus::pbft_sync::{
    PbftSyncQueueDrainAction, PbftSyncQueueDrainReport, PbftSyncQueueDrainReportResult,
    PbftSyncQueueDrainStatus, PbftSyncQueueDrainStep,
};
use rustaxa_consensus::period_data_queue::EncodedPeriodDataQueuePushRequest;
use rustaxa_consensus::{PbftService, PbftServiceConfig, PbftSyncIngressStep};

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
            has_reward_votes_reset: false,
            reward_votes_bundle_rlp: Vec::new(),
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
        ficus_activation_period: config.ficus_activation_period,
        pillar_blocks_interval: config.pillar_blocks_interval,
        sync_level_size: config.sync_level_size,
        is_light_node: config.is_light_node,
        light_node_history: config.light_node_history,
        committee_size: config.committee_size,
        number_of_proposers: config.number_of_proposers,
        slashing_submitters: config
            .slashing_submitters
            .into_iter()
            .map(slashing_submitter_identity_to_domain)
            .collect(),
    };
    Ok(Box::new(BridgePbftService(PbftService::restore(
        storage.0.clone(),
        config,
    )?)))
}

fn pbft_sync_ingress_step_to_ffi(value: PbftSyncIngressStep) -> FfiPbftSyncIngressStep {
    let has_effect = value.slashing_transaction_effect.is_some();
    FfiPbftSyncIngressStep {
        action: value.action.as_u8(),
        error_code: value.error_code,
        source_payload_id: value.source_payload_id,
        block_hash: value.block_hash.0,
        period: value.period,
        max_dag_level: value.max_dag_level,
        last_block: value.last_block,
        current_cert_present: value.current_cert_present,
        has_slashing_transaction_effect: has_effect,
        slashing_transaction_effect: value
            .slashing_transaction_effect
            .map(slashing_transaction_effect_to_ffi)
            .unwrap_or_else(empty_slashing_transaction_effect),
    }
}

/// Begins or replaces the native PBFT-sync ingress session.
pub fn pbft_service_begin_pbft_sync_ingress(
    service: &BridgePbftService,
    final_chain: &BridgeFinalChain,
    packet_rlp: &[u8],
    source_payload_id: u64,
    source_peer_id: [u8; 64],
) -> anyhow::Result<FfiPbftSyncIngressStep> {
    service
        .0
        .begin_pbft_sync_ingress(
            &final_chain.0,
            packet_rlp,
            source_payload_id,
            source_peer_id,
        )
        .map(pbft_sync_ingress_step_to_ffi)
}

/// Reports one pending slashing effect and advances the same ingress session.
pub fn pbft_service_report_pbft_sync_ingress_slashing(
    service: &BridgePbftService,
    final_chain: &BridgeFinalChain,
    proof_hash: [u8; 32],
    transaction_inserted: bool,
) -> anyhow::Result<FfiPbftSyncIngressStep> {
    service
        .0
        .report_pbft_sync_ingress_slashing(&final_chain.0, proof_hash.into(), transaction_inserted)
        .map(pbft_sync_ingress_step_to_ffi)
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
    let replay = runtime
        .0
        .load_startup_replay_period(period, load_period_lambda)?;
    Ok(startup_replay_period_into_ffi(replay))
}

fn startup_replay_period_into_ffi(
    replay: PbftManagerStartupReplayPeriod,
) -> crate::ffi::rustaxa_ffi::PbftManagerStartupReplayPeriod {
    crate::ffi::rustaxa_ffi::PbftManagerStartupReplayPeriod {
        found: replay.found,
        period_data_rlp: replay.period_data_rlp,
        finalized_dag_hashes: replay
            .finalized_dag_hashes
            .into_iter()
            .map(|hash| FfiPbftFinalizationHash { hash: hash.0 })
            .collect(),
        has_period_lambda: replay.period_lambda.is_some(),
        period_lambda: replay.period_lambda.unwrap_or_default(),
    }
}

/// Returns the current Rust-owned PBFT manager runtime snapshot.
pub fn pbft_manager_runtime_snapshot(runtime: &BridgePbftService) -> FfiPbftManagerRuntimeSnapshot {
    runtime.0.manager_snapshot().into()
}

/// Returns the Rust-owned PBFT sync period-data queue snapshot.
///
/// Inputs:
/// - `runtime` owns the in-memory period-data queue metadata.
/// - Native PBFT-chain head state supplies the chain size, current period, and
///   last hash; C++ cannot inject a divergent queue view.
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
) -> anyhow::Result<FfiPeriodDataQueueSnapshot> {
    let snapshot = runtime.0.period_data_queue_snapshot()?;
    Ok(FfiPeriodDataQueueSnapshot {
        period: snapshot.period,
        syncing_period: snapshot.syncing_period,
        last_block_hash_or_chain: snapshot.last_block_hash_or_chain.into(),
        size: snapshot.size,
        empty: snapshot.empty,
    })
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
///   pass. Rust retains the encoded `PeriodData` payloads and peer identities;
///   the planner session requires no parallel C++ queue or standalone handle.
pub fn pbft_manager_runtime_begin_pbft_sync_queue_drain(runtime: &BridgePbftService) {
    runtime.0.begin_pbft_sync_queue_drain();
}

/// Returns the next CXX-safe external queue-drain step for `runtime`.
///
/// Queue size and period are sampled from native siblings. Rust orders actions,
/// applies and acknowledges cleanup, and validates reports; C++ temporarily
/// materializes popped periods, pushes blocks, and publishes network sync state.
pub fn pbft_manager_runtime_pbft_sync_queue_drain_next(
    runtime: &BridgePbftService,
) -> FfiPbftSyncQueueDrainStep {
    runtime
        .0
        .pbft_sync_queue_drain_next()
        .map(queue_drain_step_into_ffi)
        .unwrap_or_else(queue_drain_bootstrap_incomplete_step)
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
    runtime
        .0
        .report_pbft_sync_queue_drain(queue_drain_report_from_ffi(report))
        .map(queue_drain_report_result_into_ffi)
        .unwrap_or_else(queue_drain_bootstrap_incomplete_report)
}

/// Pushes one encoded period-data payload into the native manager queue.
///
/// C++ supplies canonical payload bytes, the fixed peer identity, and temporary
/// compact validation facts. The bridge converts the complete request; native
/// `PbftService` owns payload retention, locking, admission, cleanup, and
/// pop-source state.
/// Sequencing rejection is returned as an outcome and checked period overflow
/// is returned as an error without partial queue mutation.
pub fn pbft_manager_runtime_period_data_queue_push(
    runtime: &BridgePbftService,
    period_data_rlp: Vec<u8>,
    source_peer_id: [u8; 64],
    previous_cert_vote_rlps: Vec<FfiPbftCertVoteRlp>,
    current_block_cert_vote_rlps: Vec<FfiPbftCertVoteRlp>,
) -> anyhow::Result<FfiPeriodDataQueuePushOutcome> {
    Ok(runtime
        .0
        .push_encoded_period_data_queue(EncodedPeriodDataQueuePushRequest {
            period_data_rlp,
            source_peer_id,
            previous_cert_vote_rlps: pbft_vote_rlps_to_vec(previous_cert_vote_rlps),
            current_block_cert_vote_rlps: pbft_vote_rlps_to_vec(current_block_cert_vote_rlps),
        })?
        .into())
}

/// Pops one encoded PBFT sync payload and its executor facts from runtime state.
pub fn pbft_manager_runtime_period_data_queue_pop(
    runtime: &BridgePbftService,
) -> anyhow::Result<FfiPeriodDataQueuePopPlan> {
    Ok(runtime.0.pop_period_data_queue()?.into())
}

impl From<rustaxa_consensus::period_data_queue::PeriodDataQueuePushOutcome>
    for FfiPeriodDataQueuePushOutcome
{
    fn from(value: rustaxa_consensus::period_data_queue::PeriodDataQueuePushOutcome) -> Self {
        Self {
            accepted: value.accepted,
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
            period_data_rlp: value.period_data_rlp,
            source_peer_id: value.source_peer_id,
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

fn pillar_vote_rlps_to_bridge(rlps: Vec<Vec<u8>>) -> Vec<FfiPillarVoteRlpPayload> {
    rlps.into_iter()
        .map(|vote_rlp| FfiPillarVoteRlpPayload { vote_rlp })
        .collect()
}

fn transaction_rlps_to_bridge(rlps: Vec<Vec<u8>>) -> Vec<FfiPeriodDataQueueTransactionPayload> {
    rlps.into_iter()
        .map(|transaction_rlp| FfiPeriodDataQueueTransactionPayload { transaction_rlp })
        .collect()
}

fn pbft_vote_rlps_to_vec(payloads: Vec<FfiPbftCertVoteRlp>) -> Vec<Vec<u8>> {
    payloads
        .into_iter()
        .map(|payload| payload.vote_rlp)
        .collect()
}

fn pbft_vote_rlps_to_bridge(rlps: Vec<Vec<u8>>) -> Vec<FfiPbftCertVoteRlp> {
    rlps.into_iter()
        .map(|vote_rlp| FfiPbftCertVoteRlp { vote_rlp })
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
    runtime
        .0
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

/// Commits a completed Rust-planned period advance in the long-lived runtime.
///
/// Rust validates committed-reset provenance, durably removes stale proposed
/// blocks, cleans native vote/proposal siblings, and publishes the manager
/// period in one ordered operation. Invalid or duplicate reports return a
/// rejected snapshot. Operational lock or storage failures cross CXX as an
/// exception so the C++ Boolean executor boundary can log and return failure;
/// native state and reset provenance remain retryable.
pub fn pbft_manager_runtime_apply_period_advance(
    runtime: &BridgePbftService,
    new_period: u64,
) -> anyhow::Result<FfiPbftManagerRuntimeSnapshot> {
    runtime.0.apply_period_advance(new_period).map(Into::into)
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
    runtime
        .0
        .apply_broadcast_counters(
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
    runtime.0.cert_voted_block_in_round()
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
    Ok(runtime
        .0
        .save_cert_voted_block_in_round(
            period,
            round,
            ethereum_types::H256::from(block_hash),
            &block_rlp,
        )?
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
    runtime
        .0
        .apply_cert_voted_block_metadata(period, round, ethereum_types::H256::from(block_hash))
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
    runtime.0.own_pillar_block_vote()
}

/// Plans, persists, and commits one lifecycle transition as a Rust-owned operation.
pub fn pbft_manager_runtime_execute_lifecycle_transition(
    runtime: &BridgePbftService,
    request: FfiPbftManagerLifecycleTransitionRequest,
) -> anyhow::Result<FfiPbftManagerLifecycleTransitionResult> {
    let outcome = runtime
        .0
        .execute_lifecycle_transition(lifecycle_transition_request_from_ffi(request))?;
    Ok(lifecycle_transition_result_from_domain(outcome))
}

fn lifecycle_transition_request_from_ffi(
    request: FfiPbftManagerLifecycleTransitionRequest,
) -> PbftManagerLifecycleTransitionRequest {
    PbftManagerLifecycleTransitionRequest {
        kind: PbftManagerTransitionKind::from_u8(request.kind),
        target_period: request.target_period,
        target_round: request.target_round,
        has_network_next_voting_step: request.has_network_next_voting_step,
        network_next_voting_step: request.network_next_voting_step,
    }
}

fn lifecycle_transition_result_from_domain(
    outcome: PbftManagerLifecycleTransitionOutcome,
) -> FfiPbftManagerLifecycleTransitionResult {
    FfiPbftManagerLifecycleTransitionResult {
        status: outcome.status.as_u8(),
        snapshot: outcome.snapshot.into(),
        remove_cert_voted_sidecar: outcome.remove_cert_voted_sidecar,
        clear_broadcasted_vote_sidecars: outcome.clear_broadcasted_vote_sidecars,
        set_vote_manager_period_round: outcome.set_vote_manager_period_round,
        reset_current_round_timer: outcome.reset_current_round_timer,
        reset_second_finish_timer: outcome.reset_second_finish_timer,
        print_cert_step_info: outcome.print_cert_step_info,
        print_second_finish_step_info: outcome.print_second_finish_step_info,
        reset_executed_block_follow_up: outcome.reset_executed_block_follow_up,
        error_code: outcome.error_code,
    }
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
/// - Every lifecycle sidecar and timer command is `false`; this storage
///   follow-up reuses the lifecycle result carrier without emitting effects.
///
/// Invariants and edge behavior:
/// - C++ must call this only after preserving the legacy
///   `waitForPeriodFinalization()` ordering.
/// - The runtime-owned Rust storage handle performs the durable write.
/// - The Rust runtime changes only after that Rust storage write succeeds.
/// - The returned snapshot is the authoritative source for C++ live mirrors.
pub fn pbft_manager_runtime_apply_executed_block_reset(
    runtime: &BridgePbftService,
) -> anyhow::Result<FfiPbftManagerLifecycleTransitionResult> {
    let outcome = runtime.0.apply_executed_block_reset();
    Ok(FfiPbftManagerLifecycleTransitionResult {
        status: outcome.status.as_u8(),
        snapshot: outcome.snapshot.into(),
        remove_cert_voted_sidecar: false,
        clear_broadcasted_vote_sidecars: false,
        set_vote_manager_period_round: false,
        reset_current_round_timer: false,
        reset_second_finish_timer: false,
        print_cert_step_info: false,
        print_second_finish_step_info: false,
        reset_executed_block_follow_up: false,
        error_code: outcome.error_code,
    })
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
    runtime.0.apply_next_voted_status(status).map(Into::into)
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
    runtime.0.apply_cursor_field(field, value).map(Into::into)
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
    let lookup = runtime
        .0
        .dag_block_period(ethereum_types::H256::from(*hash))?;
    Ok(dag_block_period_lookup_into_ffi(lookup))
}

fn dag_block_period_lookup_into_ffi(lookup: DagBlockPeriodStorageLookup) -> FfiBlockPeriodLookup {
    FfiBlockPeriodLookup {
        found: lookup.found,
        period: lookup.period,
        position: lookup.position,
    }
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
    let decision = runtime
        .0
        .plan_finalization_dynamic_lambda(PbftDynamicLambdaFact::from(fact))?;
    Ok(FfiPbftManagerFinalizationDynamicLambdaPlan::from((
        decision.plan,
        decision.last_saved_period_lambda.found,
        decision.last_saved_period_lambda.value,
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
    runtime.0.begin_runtime_session(fact.into());
}

/// Returns the next requested action for the runtime-owned tick session.
pub fn pbft_manager_runtime_session_next(
    runtime: &BridgePbftService,
) -> FfiPbftManagerRuntimeSessionStep {
    runtime
        .0
        .runtime_session_next()
        .map(Into::into)
        .unwrap_or_else(runtime_session_not_started_step)
}

/// Reports one C++-executed action back to the runtime-owned tick session.
pub fn pbft_manager_runtime_session_report(
    runtime: &BridgePbftService,
    report: FfiPbftManagerRuntimeActionReport,
) -> FfiPbftManagerRuntimeSessionStep {
    runtime
        .0
        .report_runtime_session(report.into())
        .map(Into::into)
        .unwrap_or_else(runtime_session_not_started_step)
}

/// Plans whether the C++ PBFT manager shell should wait using the Rust runtime
/// deadline.
pub fn plan_pbft_manager_runtime_sleep_until_next_step(
    runtime: &BridgePbftService,
    round_elapsed_ms: i64,
) -> FfiPbftManagerSleepPlan {
    runtime
        .0
        .plan_runtime_sleep_until_next_step(round_elapsed_ms)
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
    runtime.0.abort_runtime_session();
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
    runtime.0.begin_state_action_effect_session(fact.into());
}

/// Returns the next effect requested by the runtime-owned state-action session.
pub fn pbft_manager_runtime_state_action_effect_session_next(
    runtime: &BridgePbftService,
) -> FfiPbftManagerStateActionSessionStep {
    runtime
        .0
        .state_action_effect_session_next()
        .map(Into::into)
        .unwrap_or_else(state_action_effect_session_not_started_step)
}

/// Reports one C++-executed state-action effect to Rust and returns the next step.
pub fn pbft_manager_runtime_state_action_effect_session_report(
    runtime: &BridgePbftService,
    report: FfiPbftManagerStateActionEffectReport,
) -> FfiPbftManagerStateActionSessionStep {
    runtime
        .0
        .report_state_action_effect_session(report.into())
        .map(Into::into)
        .unwrap_or_else(state_action_effect_session_not_started_step)
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
/// - Replaces any previous proposal cursor. The composed native task drains
///   DAG-order requests before returning a terminal or build step to C++.
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
    runtime
        .0
        .begin_proposal_session(proposal_initial_fact_from_ffi(fact, final_chain_hash));
}

/// Returns the next proposal-construction action or build command.
#[cfg(test)]
pub(crate) fn pbft_manager_proposal_session_next(
    runtime: &BridgePbftService,
) -> FfiPbftManagerProposalSessionStep {
    runtime
        .0
        .proposal_session_next()
        .map(Into::into)
        .unwrap_or_else(proposal_session_not_started_step)
}

pub(crate) fn proposal_session_not_started_step() -> FfiPbftManagerProposalSessionStep {
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

/// Composes ordinary PBFT validation from immutable candidate facts, resolving
/// native chain, FinalChain, reward, extra-data, pillar, and DAG checks. It returns a
/// terminal plan or typed wait/error.
pub fn plan_pbft_manager_block_validation(
    runtime: &BridgePbftService,
    final_chain: &BridgeFinalChain,
    dag_transaction_service: &BridgeDagTransactionService,
    fact: &FfiPbftManagerBlockValidationFact,
) -> anyhow::Result<FfiPbftManagerBlockValidationPlan> {
    Ok(dag_transaction_service
        .validate_pbft_block(
            runtime,
            final_chain,
            block_validation_candidate_from_ffi(fact),
        )?
        .into())
}

fn block_validation_candidate_from_ffi(
    fact: &FfiPbftManagerBlockValidationFact,
) -> PbftBlockValidationCandidate {
    PbftBlockValidationCandidate {
        fact: fact.into(),
        previous_pbft_block_hash: fact.previous_pbft_block_hash.into(),
        candidate_final_chain_hash: fact.candidate_final_chain_hash.into(),
        expected_order_hash: fact.expected_order_hash.into(),
        pbft_gas_limit: fact.pbft_gas_limit,
        reward_vote_hashes: fact
            .reward_vote_hashes
            .iter()
            .map(|hash| ethereum_types::H256::from(hash.hash))
            .collect(),
        pillar_block_hash: fact
            .has_pillar_block_hash
            .then(|| ethereum_types::H256::from(fact.pillar_block_hash)),
    }
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
    let boundary = dag_transaction_service.start_finalization_executor(
        runtime,
        PbftFinalizationExecutorStartRequest {
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

/// Advances one external finalization action selected by boundary action code.
///
/// Inputs:
/// - `runtime`: PBFT manager runtime that owns the current finalization cursor.
/// - `dag_transaction_service`: Native DAG/transaction service for action-specific
///   external work.
/// - `cursor`: executor cursor previously returned to C++.
/// - `action`: legacy boundary action code selected from the Rust session.
/// - `last_block`: FinalChain last-block proof for finalization dispatch.
/// - `request_period`: pillar request period for pillar post-processing.
/// - `retention_window`: finalized transaction retention window.
/// - `account_nonce_facts`: retained EVM nonce facts for transaction status.
///
/// Outputs:
/// - The next PBFT finalization executor state.
pub fn pbft_manager_runtime_advance_finalization_action(
    runtime: &BridgePbftService,
    dag_transaction_service: &BridgeDagTransactionService,
    cursor: u32,
    action: u8,
    last_block: u64,
    request_period: u64,
    retention_window: u64,
    account_nonce_facts: Vec<FfiTransactionQueueAccountNonceFact>,
) -> anyhow::Result<FfiPbftManagerFinalizationExecutorState> {
    dag_transaction_service
        .advance_finalization_action(
            runtime,
            cursor,
            action,
            last_block,
            request_period,
            retention_window,
            bridge_to_service_account_nonce_facts(account_nonce_facts),
        )
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

impl From<&FfiPbftManagerBlockValidationFact> for PbftManagerBlockValidationFact {
    fn from(value: &FfiPbftManagerBlockValidationFact) -> Self {
        Self {
            block_hash: value.block_hash.into(),
            period: value.period,
            pivot_hash: value.pivot_hash.into(),
            pivot_is_null: value.pivot_hash == [0; 32],
            dag_order_required: true,
            extra_data_required: value.extra_data_required,
            extra_data_present: value.extra_data_present,
            extra_data_pillar_hash_present: value.extra_data_pillar_hash_present,
            pillar_block_required: value.pillar_block_required,
            pbft_chain_status: PbftManagerBlockValidationFactStatus::NotChecked,
            final_chain_hash_status: PbftManagerBlockValidationFactStatus::NotChecked,
            reward_votes_status: PbftManagerBlockValidationFactStatus::NotChecked,
            pillar_block_status: if value.pillar_block_required {
                PbftManagerBlockValidationFactStatus::NotChecked
            } else {
                PbftManagerBlockValidationFactStatus::NotRequired
            },
            dag_order_status: PbftManagerBlockValidationFactStatus::NotChecked,
            dag_weight_status: PbftManagerBlockValidationFactStatus::NotChecked,
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
#[path = "../test_support/pbft_manager_period_data_queue.rs"]
mod period_data_queue_adapter_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;
    use rustaxa_consensus::pbft_manager::{
        PbftManagerRuntimeStateCode, PbftManagerStartupRestoreStatus, PbftManagerTransitionKind,
    };

    const TRANSITION_FILTER: u8 = 1;
    const TRANSITION_STORAGE_STATUS_REJECTED: u8 = 1;
    const ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND: u8 = 2;
    const ADVANCE_ACTION_RESET_CURRENT_ROUND_TIMER: u8 = 3;
    const ADVANCE_ACTION_RESET_REWARD_VOTE_COUNTERS: u8 = 4;
    const ADVANCE_ACTION_RESET_PERIOD_TIMER: u8 = 5;
    const ADVANCE_ACTION_UPDATE_WALLET_ELIGIBILITY: u8 = 6;

    fn lifecycle_transition_request(kind: u8) -> FfiPbftManagerLifecycleTransitionRequest {
        FfiPbftManagerLifecycleTransitionRequest {
            kind,
            target_period: 10,
            target_round: 4,
            has_network_next_voting_step: false,
            network_next_voting_step: 0,
        }
    }

    fn runtime_snapshot() -> rustaxa_consensus::pbft_manager::PbftManagerRuntimeSnapshot {
        rustaxa_consensus::pbft_manager::PbftManagerRuntimeSnapshot {
            status: PbftManagerStartupRestoreStatus::Ready,
            state: PbftManagerRuntimeStateCode::ValueProposal,
            period: 10,
            round: 2,
            step: 3,
            current_round_lambda_ms: 100,
            next_step_time_ms: 99,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda_ms: 1000,
            executed_pbft_block: false,
            already_next_voted_value: false,
            already_next_voted_null: false,
            broadcast_votes_counter: 1,
            rebroadcast_votes_counter: 1,
            broadcast_reward_votes_counter: 1,
            rebroadcast_reward_votes_counter: 1,
            has_cert_voted_block: false,
            cert_voted_block_period: 0,
            cert_voted_block_round: 0,
            cert_voted_block_hash: H256::repeat_byte(0x11),
            persist_normalized_step: false,
            reset_second_finish_start: false,
            error_code: String::new(),
        }
    }

    fn applied_transition_result(effects: [bool; 8]) -> FfiPbftManagerLifecycleTransitionResult {
        lifecycle_transition_result_from_domain(PbftManagerLifecycleTransitionOutcome {
            status: rustaxa_consensus::pbft_manager::PbftManagerTransitionStorageStatus::Applied,
            snapshot: runtime_snapshot(),
            remove_cert_voted_sidecar: effects[0],
            clear_broadcasted_vote_sidecars: effects[1],
            set_vote_manager_period_round: effects[2],
            reset_current_round_timer: effects[3],
            reset_second_finish_timer: effects[4],
            print_cert_step_info: effects[5],
            print_second_finish_step_info: effects[6],
            reset_executed_block_follow_up: effects[7],
            error_code: String::new(),
        })
    }

    #[test]
    fn bootstrap_fallback_adapters_preserve_status_and_errors() {
        let runtime_step = runtime_session_not_started_step();
        assert_eq!(
            runtime_step.error_code,
            "PBFT_MANAGER_RUNTIME_SESSION_NOT_STARTED"
        );
        assert_eq!(
            runtime_step.status,
            PbftManagerRuntimeStatus::ContractError.as_u8()
        );
        assert!(!runtime_step.can_continue);

        let proposal_step = proposal_session_not_started_step();
        assert_eq!(
            proposal_step.error_code,
            "PBFT_MANAGER_PROPOSAL_SESSION_NOT_STARTED"
        );
        assert_eq!(
            proposal_step.action,
            PbftManagerProposalAction::ContractError.as_u8()
        );

        let blocked_drain = queue_drain_bootstrap_incomplete_step();
        assert_eq!(
            blocked_drain.error_code,
            "PBFT_SERVICE_BOOTSTRAP_INCOMPLETE"
        );
        assert_eq!(
            blocked_drain.status,
            PbftSyncQueueDrainStatus::InvalidReport.as_u8()
        );
        assert!(!blocked_drain.can_continue);

        let blocked_report = queue_drain_bootstrap_incomplete_report();
        assert_eq!(
            blocked_report.error_code,
            "PBFT_SERVICE_BOOTSTRAP_INCOMPLETE"
        );
        assert_eq!(
            blocked_report.status,
            PbftSyncQueueDrainStatus::InvalidReport.as_u8()
        );
        assert!(!blocked_report.can_continue);
    }

    #[test]
    fn bridge_session_adapters_preserve_boundary_codes() {
        let queue_step = queue_drain_step_into_ffi(PbftSyncQueueDrainStep {
            action: PbftSyncQueueDrainAction::PushAccepted,
            status: PbftSyncQueueDrainStatus::Active,
            clean_before_period: 42,
            can_continue: true,
            error_code: "QUEUE_STEP_SENTINEL",
        });
        assert_eq!(
            (queue_step.action, queue_step.status),
            (
                PbftSyncQueueDrainAction::PushAccepted.as_u8(),
                PbftSyncQueueDrainStatus::Active.as_u8()
            )
        );
        let queue_report = queue_drain_report_result_into_ffi(PbftSyncQueueDrainReportResult {
            status: PbftSyncQueueDrainStatus::PushFailed,
            can_continue: false,
            error_code: "QUEUE_REPORT_SENTINEL",
        });
        assert_eq!(queue_step.clean_before_period, 42);
        assert_eq!(queue_step.error_code, "QUEUE_STEP_SENTINEL");
        assert_eq!(
            queue_report.status,
            PbftSyncQueueDrainStatus::PushFailed.as_u8()
        );
        assert_eq!(queue_report.error_code, "QUEUE_REPORT_SENTINEL");

        let state_report_input: PbftManagerStateActionEffectReport =
            FfiPbftManagerStateActionEffectReport {
                cursor: 9,
                intent: PbftManagerStateActionIntent::NextVoteNullBlock.as_u8(),
                result: PbftManagerStateActionEffectResultCode::SkippedNoWork.as_u8(),
                error_code: "STATE_REPORT_SENTINEL".to_owned(),
            }
            .into();
        assert_eq!(
            state_report_input.intent,
            PbftManagerStateActionIntent::NextVoteNullBlock
        );
        assert_eq!(
            state_report_input.result,
            PbftManagerStateActionEffectResultCode::SkippedNoWork
        );

        let state_step: FfiPbftManagerStateActionSessionStep = PbftManagerStateActionSessionStep {
            status: PbftManagerStateActionSessionStatus::Active,
            cursor: 7,
            has_effect: true,
            effect: PbftManagerStateActionEffect {
                intent: PbftManagerStateActionIntent::NextVoteCurrentSoftValue,
                hash: [0x55; 32],
                request_proposed_block_sidecar: true,
                proposed_block_sidecar_hash: [0x66; 32],
                proposed_block_sidecar_period: 11,
            },
            go_finish_state: true,
            loop_back_finish_state: false,
            complete: false,
            can_continue: true,
            error_code: "STATE_STEP_SENTINEL".to_owned(),
        }
        .into();
        assert_eq!(
            state_step.status,
            PbftManagerStateActionSessionStatus::Active.as_u8()
        );
        assert_eq!(
            state_step.effect.intent,
            PbftManagerStateActionIntent::NextVoteCurrentSoftValue.as_u8()
        );
        assert_eq!(state_step.effect.hash, [0x55; 32]);
        assert_eq!(state_step.effect.proposed_block_sidecar_hash, [0x66; 32]);
        assert_eq!(state_step.error_code, "STATE_STEP_SENTINEL");
    }

    #[test]
    fn bridge_storage_read_adapters_preserve_boundary_fields() {
        let dag_lookup = dag_block_period_lookup_into_ffi(DagBlockPeriodStorageLookup {
            found: true,
            period: 12,
            position: 4,
        });
        assert!(dag_lookup.found);
        assert_eq!((dag_lookup.period, dag_lookup.position), (12, 4));

        let replay = startup_replay_period_into_ffi(PbftManagerStartupReplayPeriod {
            found: true,
            period_data_rlp: vec![0xC0],
            finalized_dag_hashes: vec![H256::repeat_byte(0xDA)],
            period_lambda: Some(1_234),
        });
        assert!(replay.found);
        assert_eq!(replay.period_data_rlp, vec![0xC0]);
        assert_eq!(replay.finalized_dag_hashes.len(), 1);
        assert_eq!(replay.finalized_dag_hashes[0].hash, [0xDA; 32]);
        assert!(replay.has_period_lambda);
        assert_eq!(replay.period_lambda, 1_234);

        let missing = startup_replay_period_into_ffi(PbftManagerStartupReplayPeriod {
            found: false,
            period_data_rlp: Vec::new(),
            finalized_dag_hashes: Vec::new(),
            period_lambda: None,
        });
        assert!(!missing.found);
        assert!(!missing.has_period_lambda);
        assert_eq!(missing.period_lambda, 0);

        let dynamic_lambda = FfiPbftManagerFinalizationDynamicLambdaPlan::from((
            PbftDynamicLambdaPlan {
                apply_dynamic_lambda_update: true,
                period_lambda: 1_500,
                blocks_per_year: 9_275_294,
                rounds_count_dynamic_lambda: 0,
                dynamic_lambda: 1_490,
                decreased_dynamic_lambda: true,
                increased_dynamic_lambda: false,
                status: PbftFinalizationStatus::Accepted,
            },
            true,
            1_234,
        ));
        assert!(dynamic_lambda.last_saved_period_lambda_found);
        assert_eq!(dynamic_lambda.last_saved_period_lambda, 1_234);
    }

    #[test]
    fn startup_replay_and_advance_period_adapters_preserve_boundary_codes() {
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

        let plan_from_fixtures = FfiPbftManagerAdvancePeriodPlan {
            accepted: true,
            finalized_chain_size: 12,
            new_period: 13,
            actions: vec![
                ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND,
                ADVANCE_ACTION_RESET_CURRENT_ROUND_TIMER,
                ADVANCE_ACTION_RESET_REWARD_VOTE_COUNTERS,
                ADVANCE_ACTION_RESET_PERIOD_TIMER,
                ADVANCE_ACTION_UPDATE_WALLET_ELIGIBILITY,
            ],
            error_code: String::new(),
        };
        let accepted_report = validate_pbft_manager_advance_period_action_report(
            &plan_from_fixtures,
            FfiPbftManagerAdvancePeriodActionReport {
                action_index: 0,
                action: ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND,
                succeeded: true,
            },
        );
        assert!(accepted_report.accepted);
        assert_eq!(accepted_report.status, 0);
        assert!(accepted_report.error_code.is_empty());

        let mismatch = validate_pbft_manager_advance_period_action_report(
            &plan_from_fixtures,
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

        let rejected_script = validate_pbft_manager_advance_period_action_report(
            &plan_from_fixtures,
            FfiPbftManagerAdvancePeriodActionReport {
                action_index: 0,
                action: ADVANCE_ACTION_SET_VOTE_MANAGER_PERIOD_ROUND,
                succeeded: false,
            },
        );
        assert!(!rejected_script.accepted);
        assert_eq!(rejected_script.status, 5);
    }

    #[test]
    fn lifecycle_transition_adapters_preserve_unknown_kind_fields_and_effects() {
        let mut request = lifecycle_transition_request_from_ffi(lifecycle_transition_request(255));
        assert_eq!(request.kind, PbftManagerTransitionKind::Unknown);
        assert_eq!(request.target_period, 10);
        assert_eq!(request.target_round, 4);
        assert_eq!(request.has_network_next_voting_step, false);

        let before = runtime_snapshot();
        let unknown =
            lifecycle_transition_result_from_domain(PbftManagerLifecycleTransitionOutcome {
                status:
                    rustaxa_consensus::pbft_manager::PbftManagerTransitionStorageStatus::Rejected,
                snapshot: before.clone(),
                remove_cert_voted_sidecar: false,
                clear_broadcasted_vote_sidecars: false,
                set_vote_manager_period_round: false,
                reset_current_round_timer: false,
                reset_second_finish_timer: false,
                print_cert_step_info: false,
                print_second_finish_step_info: false,
                reset_executed_block_follow_up: false,
                error_code: "PBFT_MANAGER_TRANSITION_UNKNOWN_KIND".to_string(),
            });
        assert_eq!(unknown.status, TRANSITION_STORAGE_STATUS_REJECTED);
        assert_eq!(unknown.error_code, "PBFT_MANAGER_TRANSITION_UNKNOWN_KIND");
        assert_eq!(unknown.snapshot.period, before.period);
        assert_eq!(unknown.snapshot.round, before.round);
        assert_eq!(unknown.snapshot.step, before.step);
        assert_eq!(unknown.snapshot.state, before.state.as_u8());
        assert!(!unknown.remove_cert_voted_sidecar);
        assert!(!unknown.clear_broadcasted_vote_sidecars);
        assert!(!unknown.set_vote_manager_period_round);
        assert!(!unknown.reset_current_round_timer);
        assert!(!unknown.reset_second_finish_timer);
        assert!(!unknown.print_cert_step_info);
        assert!(!unknown.print_second_finish_step_info);
        assert!(!unknown.reset_executed_block_follow_up);

        let mut network_request = lifecycle_transition_request(TRANSITION_FILTER);
        network_request.has_network_next_voting_step = true;
        network_request.network_next_voting_step = 7;
        request = lifecycle_transition_request_from_ffi(network_request);
        assert_eq!(request.kind, PbftManagerTransitionKind::ToFilter);
        assert!(request.has_network_next_voting_step);
        assert_eq!(request.network_next_voting_step, 7);
        for expected in [
            // Reachable reset with cert-voted and executed-block facts present.
            [true, true, true, true, false, false, false, true],
            // Reachable transition to certify.
            [false, false, false, false, false, true, false, false],
            // Reachable transition to finish polling.
            [false, false, false, false, true, false, true, false],
        ] {
            let applied = applied_transition_result(expected);
            assert_eq!(
                applied.status,
                rustaxa_consensus::pbft_manager::PbftManagerTransitionStorageStatus::Applied
                    .as_u8()
            );
            assert!(applied.error_code.is_empty());
            assert_eq!(
                [
                    applied.remove_cert_voted_sidecar,
                    applied.clear_broadcasted_vote_sidecars,
                    applied.set_vote_manager_period_round,
                    applied.reset_current_round_timer,
                    applied.reset_second_finish_timer,
                    applied.print_cert_step_info,
                    applied.print_second_finish_step_info,
                    applied.reset_executed_block_follow_up,
                ],
                expected
            );
        }
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
    fn bridge_manager_action_conversions_preserve_boundary_codes() {
        let session: FfiPbftManagerRuntimeSessionStep = PbftManagerRuntimeSessionStep {
            status: PbftManagerRuntimeStatus::Active,
            cursor: 7,
            action: Some(PbftManagerRuntimeAction::SleepIneligiblePollingInterval),
            has_action: true,
            complete: false,
            restart_loop: false,
            has_target_round: false,
            target_round: 0,
            sleep_ms: 250,
            tick_id: 9,
            error_code: String::new(),
        }
        .into();
        assert_eq!((session.action, session.sleep_ms), (4, 250));

        let sleep: FfiPbftManagerSleepPlan = PbftManagerSleepPlan {
            accepted: true,
            should_sleep: true,
            sleep_ms: 1,
            step: 3,
            error_code: String::new(),
        }
        .into();
        assert_eq!(
            (
                sleep.accepted,
                sleep.should_sleep,
                sleep.sleep_ms,
                sleep.step,
                sleep.error_code.as_str(),
            ),
            (true, true, 1, 3, "")
        );

        let proposal: FfiPbftManagerProposalSessionStep = PbftManagerProposalSessionStep {
            action: PbftManagerProposalAction::RequestDagOrder,
            status: PbftManagerProposalStatus::Active,
            requested_anchor_hash: H256::repeat_byte(3),
            previous_pbft_block_hash: H256::zero(),
            anchor_hash: H256::repeat_byte(4),
            order_hash: H256::repeat_byte(5),
            final_chain_hash: H256::zero(),
            eligible_wallet_indices: vec![6],
            dag_blocks_included: 7,
            selected_null_anchor: false,
            error_code: String::new(),
        }
        .into();
        assert_eq!((proposal.action, proposal.status), (0, 0));
        assert_eq!(proposal.requested_anchor_hash, [3; 32]);
        assert_eq!(proposal.anchor_hash, [4; 32]);
        assert_eq!(proposal.order_hash, [5; 32]);
        assert_eq!(proposal.eligible_wallet_indices, vec![6]);
        assert_eq!(proposal.dag_blocks_included, 7);
        let broadcast_fact: PbftManagerBroadcastFact = FfiPbftManagerBroadcastFact {
            round_elapsed_ms: 1,
            period_elapsed_ms: 2,
            current_round_lambda_ms: 3,
            broadcast_lambda_threshold: 4,
            rebroadcast_lambda_threshold: 5,
            broadcast_votes_counter: 6,
            rebroadcast_votes_counter: 7,
            broadcast_reward_votes_counter: 8,
            rebroadcast_reward_votes_counter: 9,
        }
        .into();
        assert_eq!(
            (
                broadcast_fact.round_elapsed_ms,
                broadcast_fact.period_elapsed_ms,
                broadcast_fact.current_round_lambda_ms,
                broadcast_fact.broadcast_lambda_threshold,
                broadcast_fact.rebroadcast_lambda_threshold,
                broadcast_fact.broadcast_votes_counter,
                broadcast_fact.rebroadcast_votes_counter,
                broadcast_fact.broadcast_reward_votes_counter,
                broadcast_fact.rebroadcast_reward_votes_counter,
            ),
            (1, 2, 3, 4, 5, 6, 7, 8, 9)
        );
        let broadcast: FfiPbftManagerBroadcastPlan = PbftManagerBroadcastPlan {
            status: PbftManagerBroadcastStatus::Ready,
            action: PbftManagerBroadcastAction::RoundVotes,
            rebroadcast: false,
            next_broadcast_votes_counter: 2,
            next_rebroadcast_votes_counter: 1,
            next_broadcast_reward_votes_counter: 1,
            next_rebroadcast_reward_votes_counter: 1,
            error_code: String::new(),
        }
        .into();
        assert_eq!((broadcast.status, broadcast.action), (0, 2));
        assert!(!broadcast.rebroadcast);
        assert_eq!(broadcast.next_broadcast_votes_counter, 2);
        let broadcast_report: PbftManagerBroadcastReport = FfiPbftManagerBroadcastReport {
            action: 2,
            rebroadcast: false,
            success: true,
            error_code: String::new(),
        }
        .into();
        assert_eq!(
            broadcast_report.action,
            PbftManagerBroadcastAction::RoundVotes
        );
        let broadcast_result: FfiPbftManagerBroadcastReportResult =
            PbftManagerBroadcastReportResult {
                status: PbftManagerBroadcastStatus::Ready,
                apply_counters: true,
                broadcast_votes_counter: 2,
                rebroadcast_votes_counter: 1,
                broadcast_reward_votes_counter: 1,
                rebroadcast_reward_votes_counter: 1,
                error_code: String::new(),
            }
            .into();
        assert_eq!(
            (broadcast_result.status, broadcast_result.apply_counters),
            (0, true)
        );
        assert_eq!(broadcast_result.broadcast_votes_counter, 2);
        assert_eq!(broadcast_result.rebroadcast_votes_counter, 1);
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

        let ffi_block_fact = FfiPbftManagerBlockValidationFact {
            block_hash: [0x11; 32],
            period: 12,
            previous_pbft_block_hash: [0x12; 32],
            candidate_final_chain_hash: [0x13; 32],
            expected_order_hash: [0x21; 32],
            pbft_gas_limit: 42_000,
            reward_vote_hashes: vec![FfiPbftFinalizationHash { hash: [0x14; 32] }],
            has_pillar_block_hash: true,
            pillar_block_hash: [0x15; 32],
            pivot_hash: [0x22; 32],
            extra_data_required: true,
            extra_data_present: true,
            extra_data_pillar_hash_present: true,
            pillar_block_required: true,
        };
        let block_fact: PbftManagerBlockValidationFact = (&ffi_block_fact).into();
        assert_eq!(block_fact.block_hash, ethereum_types::H256([0x11; 32]));
        assert_eq!(block_fact.pivot_hash, ethereum_types::H256([0x22; 32]));
        assert!(block_fact.extra_data_required);
        assert!(block_fact.extra_data_present);
        assert!(block_fact.extra_data_pillar_hash_present);
        let not_checked = PbftManagerBlockValidationFactStatus::NotChecked;
        assert_eq!(block_fact.final_chain_hash_status, not_checked);
        assert_eq!(block_fact.pbft_chain_status, not_checked);
        assert_eq!(block_fact.reward_votes_status, not_checked);
        assert_eq!(block_fact.pillar_block_status, not_checked);
        let candidate = block_validation_candidate_from_ffi(&ffi_block_fact);
        assert_eq!(candidate.previous_pbft_block_hash, [0x12; 32].into());
        assert_eq!(candidate.candidate_final_chain_hash, [0x13; 32].into());
        assert_eq!(candidate.expected_order_hash, [0x21; 32].into());
        assert_eq!(candidate.pbft_gas_limit, 42_000);
        assert_eq!(candidate.reward_vote_hashes, vec![[0x14; 32].into()]);
        assert_eq!(candidate.pillar_block_hash, Some([0x15; 32].into()));

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
        assert_eq!((block_plan.action, block_plan.status), (3, 3));
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
