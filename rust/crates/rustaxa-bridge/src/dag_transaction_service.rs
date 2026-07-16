use crate::dag::build_dag_state_from_storage;
use crate::ffi::rustaxa_ffi::*;
use crate::ffi::{
    BridgeStorage, DagRuntimeState, TransactionPackSessionOwner, TransactionRuntimeState,
};
use crate::transaction_manager::{
    build_transaction_state_for_gas_pricer, build_transaction_state_from_storage,
};
use anyhow::{anyhow, Result};
use std::sync::{Mutex, MutexGuard};

/// Application-owned consensus service containing the sibling DAG and transaction runtimes.
///
/// Full production construction initializes both domains from one shared Rust storage handle
/// before publishing the service. Compatibility construction may omit the DAG runtime for
/// standalone TransactionManager or GasPricer tests; DAG calls then fail with the stable
/// `DAG_SERVICE_UNAVAILABLE` error. Each bridge call holds only its domain-specific mutex for
/// the duration of the Rust operation, so no lock guard crosses a C++ executor callback.
pub struct BridgeDagTransactionService {
    transaction: Mutex<TransactionRuntimeState>,
    dag: Mutex<Option<DagRuntimeState>>,
}

impl BridgeDagTransactionService {
    pub(crate) fn transaction(&self) -> MutexGuard<'_, TransactionRuntimeState> {
        self.transaction
            .lock()
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
    }

    fn try_transaction(&self) -> Result<MutexGuard<'_, TransactionRuntimeState>> {
        self.transaction
            .lock()
            .map_err(|_| anyhow!("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED"))
    }

    pub(crate) fn dag(&self) -> Result<MutexGuard<'_, Option<DagRuntimeState>>> {
        let guard = self
            .dag
            .lock()
            .map_err(|_| anyhow!("DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED"))?;
        if guard.is_none() {
            return Err(anyhow!("DAG_SERVICE_UNAVAILABLE"));
        }
        Ok(guard)
    }

    /// Locks the composed runtimes in the universal DAG-then-transaction order.
    fn dag_and_transaction(
        &self,
    ) -> Result<(
        MutexGuard<'_, Option<DagRuntimeState>>,
        MutexGuard<'_, TransactionRuntimeState>,
    )> {
        let dag = self.dag()?;
        let transaction = self.try_transaction()?;
        Ok((dag, transaction))
    }
}

/// Constructs the production application service and restores both runtimes before publication.
pub fn create_dag_transaction_service_from_storage(
    storage: &BridgeStorage,
    genesis: &[u8; 32],
    dag_expiry_limit: u32,
    max_levels_per_period: u64,
    transaction_queue_config: TransactionQueueConfig,
    gas_pricer_config: GasPricerConfig,
    proposal_dag_gas_limit: u64,
) -> Result<Box<BridgeDagTransactionService>> {
    let transaction = *build_transaction_state_from_storage(
        storage,
        transaction_queue_config,
        gas_pricer_config,
        proposal_dag_gas_limit,
    )?;
    let mut dag = *build_dag_state_from_storage(genesis, dag_expiry_limit, storage)?;
    dag.dag_manager_runtime_restore_from_storage()?;
    dag.dag_manager_runtime_ensure_proposal_period_mapping(max_levels_per_period, 0)?;
    Ok(Box::new(BridgeDagTransactionService {
        transaction: Mutex::new(transaction),
        dag: Mutex::new(Some(dag)),
    }))
}

/// Constructs a storage-backed transaction-only compatibility service.
pub fn create_dag_transaction_service_for_transaction_manager(
    storage: &BridgeStorage,
    transaction_queue_config: TransactionQueueConfig,
    gas_pricer_config: GasPricerConfig,
    proposal_dag_gas_limit: u64,
) -> Result<Box<BridgeDagTransactionService>> {
    let transaction = *build_transaction_state_from_storage(
        storage,
        transaction_queue_config,
        gas_pricer_config,
        proposal_dag_gas_limit,
    )?;
    Ok(Box::new(BridgeDagTransactionService {
        transaction: Mutex::new(transaction),
        dag: Mutex::new(None),
    }))
}

/// Constructs a storage-free transaction-only service for standalone GasPricer tests.
pub fn create_dag_transaction_service_for_gas_pricer(
    gas_pricer_config: GasPricerConfig,
) -> Result<Box<BridgeDagTransactionService>> {
    let transaction = *build_transaction_state_for_gas_pricer(gas_pricer_config)?;
    Ok(Box::new(BridgeDagTransactionService {
        transaction: Mutex::new(transaction),
        dag: Mutex::new(None),
    }))
}

macro_rules! transaction_shared {
    ($name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty) => {
        pub fn $name(&self, $( $arg: $ty ),*) -> $ret {
            self.transaction().$name($( $arg ),*)
        }
    };
}

macro_rules! transaction_mut {
    ($name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty) => {
        pub fn $name(&self, $( $arg: $ty ),*) -> $ret {
            self.transaction().$name($( $arg ),*)
        }
    };
}

macro_rules! transaction_shared_result {
    ($name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty) => {
        pub fn $name(&self, $( $arg: $ty ),*) -> Result<$ret> {
            self.try_transaction()?.$name($( $arg ),*)
        }
    };
}

macro_rules! transaction_mut_result {
    ($name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty) => {
        pub fn $name(&self, $( $arg: $ty ),*) -> Result<$ret> {
            self.try_transaction()?.$name($( $arg ),*)
        }
    };
}

macro_rules! dag_shared_value_result {
    ($name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty) => {
        pub fn $name(&self, $( $arg: $ty ),*) -> Result<$ret> {
            let guard = self.dag()?;
            let runtime = guard.as_ref().ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
            Ok(runtime.$name($( $arg ),*))
        }
    };
}

macro_rules! dag_shared_result {
    ($name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty) => {
        pub fn $name(&self, $( $arg: $ty ),*) -> Result<$ret> {
            let guard = self.dag()?;
            let runtime = guard.as_ref().ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
            runtime.$name($( $arg ),*)
        }
    };
}

macro_rules! dag_mut_result {
    ($name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty) => {
        pub fn $name(&self, $( $arg: $ty ),*) -> Result<$ret> {
            let mut guard = self.dag()?;
            let runtime = guard.as_mut().ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
            runtime.$name($( $arg ),*)
        }
    };
}

impl BridgeDagTransactionService {
    transaction_shared!(transaction_manager_runtime_gas_price_bid() -> [u8; 32]);
    transaction_mut!(transaction_manager_runtime_gas_price_update(gas_prices: Vec<GasPricerGasPrice>) -> ());
    transaction_mut_result!(transaction_manager_runtime_pack_prepare_sharded(weight_limit: u64, min_transaction_gas: u64, proposal_period: u64, estimate_gas_limit: u64, last_block_number: u64, total_shards: u16, node_shard: u16, shard_period_interval: u64) -> TransactionPackPreparedPlan);
    transaction_mut_result!(transaction_manager_runtime_pack_finalize_with_estimates(inputs: Vec<TransactionPackSessionEstimateInput>) -> TransactionPackSessionStep);
    transaction_mut!(transaction_manager_runtime_pack_abort() -> bool);
    transaction_shared_result!(transaction_manager_runtime_plan_gas_estimation(fact: TransactionManagerGasEstimationFact) -> TransactionManagerGasEstimationPlan);
    transaction_mut_result!(transaction_manager_runtime_store_gas_estimation(result: TransactionManagerGasEstimationResult) -> bool);
    transaction_shared!(transaction_manager_runtime_transaction_count() -> u64);
    transaction_shared_result!(transaction_manager_runtime_is_transaction_known_hash(hash: &[u8; 32]) -> bool);
    transaction_mut_result!(transaction_manager_runtime_initialize_recently_finalized_payloads(period: u64, payloads: Vec<TransactionManagerSidecarInsertInput>) -> ());
    transaction_shared!(transaction_manager_runtime_non_finalized_size() -> usize);
    transaction_mut_result!(transaction_manager_runtime_remove_non_finalized(requests: Vec<TransactionManagerSidecarLookupRequest>) -> u64);
    transaction_mut_result!(transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts_command_report(fact: TransactionManagerValidatedInsertRuntimeFact, final_chain_fact: TransactionManagerFinalChainAdmissionFact, input: TransactionQueueInsertInput) -> TransactionManagerAdmissionCommandReport);
    transaction_mut_result!(transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_facts_command_report(verify_fact: TransactionManagerVerifyTransactionFact, admission_fact: TransactionManagerValidatedInsertRuntimeFact, final_chain_fact: TransactionManagerFinalChainAdmissionFact, input: TransactionQueueInsertInput) -> TransactionManagerPublicAdmissionCommandReport);
    transaction_shared_result!(transaction_manager_runtime_queue_lookup_transaction_views(requests: Vec<TransactionManagerTransactionViewRequest>) -> Vec<TransactionManagerTransactionView>);
    transaction_shared!(transaction_manager_runtime_queue_all_transaction_groups() -> Vec<TransactionQueueTransactionGroup>);
    transaction_shared!(transaction_manager_runtime_queue_size() -> usize);
    transaction_shared!(transaction_manager_runtime_queue_proposable_accounts() -> Vec<TransactionQueueProposableAccountFact>);
    transaction_mut!(transaction_manager_runtime_queue_block_finalized(block_number: u64) -> Vec<TransactionQueueHash>);
    transaction_shared!(transaction_manager_runtime_queue_transactions_dropped() -> bool);
    transaction_shared!(transaction_manager_runtime_queue_non_proposable_over_limit() -> bool);
    transaction_shared!(transaction_manager_runtime_queue_min_gas_price_for_block_inclusion(limit: u64) -> [u8; 32]);
    transaction_shared_result!(transaction_manager_runtime_lookup_non_finalized_transaction_views(requests: Vec<TransactionManagerTransactionViewRequest>) -> Vec<TransactionManagerTransactionView>);
    transaction_shared_result!(transaction_manager_runtime_lookup_transaction_views(requests: Vec<TransactionManagerTransactionViewRequest>, max_count: u64) -> TransactionManagerTransactionViewPlan);
    transaction_shared_result!(transaction_manager_runtime_lookup_proposal_transaction_views_with_account_nonce_facts(proposal_period: u64, requests: Vec<TransactionManagerTransactionViewRequest>, account_nonce_facts: Vec<TransactionQueueAccountNonceFact>, max_count: u64) -> TransactionManagerTransactionViewPlan);

    pub fn dag_transaction_service_proposer_pack_prepare(
        &self,
        session_id: u64,
        network_throttled: bool,
        min_transaction_gas: u64,
        estimate_gas_limit: u64,
        last_block_number: u64,
    ) -> Result<DagProposerSessionStep> {
        dag_transaction_service_proposer_pack_prepare(
            self,
            session_id,
            network_throttled,
            min_transaction_gas,
            estimate_gas_limit,
            last_block_number,
        )
    }

    pub fn dag_transaction_service_proposer_pack_finalize(
        &self,
        session_id: u64,
        estimates: Vec<TransactionPackSessionEstimateInput>,
    ) -> Result<DagProposerSessionStep> {
        dag_transaction_service_proposer_pack_finalize(self, session_id, estimates)
    }

    pub fn dag_transaction_service_proposer_pack_abort(&self, session_id: u64) -> Result<bool> {
        dag_transaction_service_proposer_pack_abort(self, session_id)
    }

    dag_mut_result!(dag_manager_runtime_restore_from_storage() -> ());
    dag_mut_result!(dag_manager_runtime_add_block(block: DagManagerBlock) -> ());
    dag_shared_result!(dag_manager_runtime_plan_add_block(input: DagAddBlockRuntimeInput) -> DagAddBlockEffectPlan);
    dag_shared_result!(dag_manager_runtime_validate_pivot_tips(block_level: u64, pivot: &[u8; 32], tips: Vec<DagHash>) -> DagPivotTipsValidation);
    dag_mut_result!(dag_manager_runtime_apply_finalized_order(new_anchor: [u8; 32], new_period: u64, finalized_order: Vec<DagHash>) -> DagManagerFinalizationApplyPayload);
    dag_shared_result!(dag_manager_runtime_non_finalized_sync_payload(known_hashes: Vec<DagHash>) -> DagManagerNonFinalizedSyncPayload);
    dag_shared_value_result!(dag_manager_runtime_compute_order(anchor: &[u8; 32]) -> DagOrder);
    dag_shared_value_result!(dag_manager_runtime_frontier() -> DagFrontier);
    dag_shared_value_result!(dag_manager_runtime_ghost_path(source: &[u8; 32]) -> Vec<DagHash>);
    dag_shared_value_result!(dag_manager_runtime_anchor_ghost_path() -> Vec<DagHash>);
    dag_shared_value_result!(dag_manager_runtime_graphviz_dot(pivot_tree: bool) -> String);
    dag_shared_value_result!(dag_manager_runtime_vertex_count() -> usize);
    dag_shared_value_result!(dag_manager_runtime_edge_count() -> usize);
    dag_shared_value_result!(dag_manager_runtime_max_level() -> u64);
    dag_shared_value_result!(dag_manager_runtime_latest_period() -> u64);
    dag_shared_value_result!(dag_manager_runtime_anchors() -> DagManagerAnchors);
    dag_shared_value_result!(dag_manager_runtime_dag_expiry_limit() -> u32);
    dag_shared_value_result!(dag_manager_runtime_dag_expiry_level() -> u64);
    dag_shared_value_result!(dag_manager_runtime_non_finalized_blocks() -> Vec<DagLevelHashes>);
    dag_shared_value_result!(dag_manager_runtime_non_finalized_blocks_size() -> DagManagerNonFinalizedSize);
    dag_shared_value_result!(dag_manager_runtime_non_finalized_min_difficulty() -> u32);
    dag_shared_result!(dag_manager_runtime_is_block_known(hash: &[u8; 32]) -> bool);
    dag_shared_result!(dag_manager_runtime_tip_gas_estimations(tips: Vec<DagHash>) -> Vec<DagTipGas>);
    dag_shared_result!(dag_manager_runtime_load_block(hash: &[u8; 32]) -> DagBlockLookup);
    dag_shared_result!(dag_manager_runtime_save_block(hash: &[u8; 32], level: u64, tips_count: u64, block_rlp: Vec<u8>) -> ());
    dag_shared_result!(dag_manager_runtime_plan_proposal_tip_selection(input: DagProposerStorageTipSelectionInput) -> DagProposerTipSelectionPlan);
    dag_shared_result!(dag_manager_runtime_ensure_proposal_period_mapping(level: u64, period: u64) -> bool);
    dag_shared_result!(dag_manager_runtime_period_block_hash(period: u64) -> HashLookup);
    dag_shared_result!(dag_manager_runtime_persistence_counters() -> DagPersistenceCounters);
}

pub fn service_save_transactions_from_dag_block_command_report_with_runtime(
    service: &BridgeDagTransactionService,
    facts: Vec<DagTransactionSaveSidecarFact>,
) -> Result<TransactionManagerDagSaveCommandReport> {
    let mut transaction = service.try_transaction()?;
    crate::transaction_manager::save_transactions_from_dag_block_command_report_with_runtime(
        &mut transaction,
        facts,
    )
}

pub fn service_update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts(
    service: &BridgeDagTransactionService,
    period: u64,
    retention_window: u64,
    account_nonce_facts: Vec<TransactionQueueAccountNonceFact>,
    facts: Vec<FinalizedTransactionStatusSidecarFact>,
) -> Result<TransactionManagerFinalizedStatusCommandReport> {
    let mut transaction = service.try_transaction()?;
    crate::transaction_manager::update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts(
        &mut transaction, period, retention_window, account_nonce_facts, facts,
    )
}

pub fn service_transaction_manager_filter_non_finalized_with_runtime(
    service: &BridgeDagTransactionService,
    requests: Vec<TransactionManagerSidecarLookupRequest>,
) -> Result<FinalizedTransactionFilterPlan> {
    let transaction = service.try_transaction()?;
    crate::transaction_manager::transaction_manager_filter_non_finalized_with_runtime(
        &transaction,
        requests,
    )
}

pub fn service_transaction_manager_verify_not_finalized_with_runtime(
    service: &BridgeDagTransactionService,
    facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
) -> Result<TransactionManagerVerifyNotFinalizedOutcome> {
    let transaction = service.try_transaction()?;
    crate::transaction_manager::transaction_manager_verify_not_finalized_with_runtime(
        &transaction,
        facts,
    )
}

pub fn service_transaction_manager_recover_nonfinalized_with_runtime(
    service: &BridgeDagTransactionService,
) -> Result<()> {
    let mut transaction = service.try_transaction()?;
    crate::transaction_manager::transaction_manager_recover_nonfinalized_with_runtime(
        &mut transaction,
    )
}

/// Prepares one DAG-owned transaction pack without exposing private pack configuration.
///
/// The service validates the DAG cursor before touching transaction state and locks DAG then transaction. A throttled
/// request never opens a transaction cursor. Otherwise Rust returns either EVM-only estimate candidates while retaining
/// both owner-bound cursors, or directly advances the DAG cursor when declared gas/cache facts complete packing. No lock
/// guard survives this call, so C++ may perform EVM callbacks after it returns. Any failure aborts both matching cursors.
pub fn dag_transaction_service_proposer_pack_prepare(
    service: &BridgeDagTransactionService,
    session_id: u64,
    network_throttled: bool,
    min_transaction_gas: u64,
    estimate_gas_limit: u64,
    last_block_number: u64,
) -> Result<DagProposerSessionStep> {
    if network_throttled {
        let mut dag_guard = service.dag()?;
        let dag = dag_guard
            .as_mut()
            .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
        if let Err(error) =
            crate::dag::dag_manager_runtime_proposer_pack_parameters(dag, session_id)
        {
            crate::dag::dag_manager_runtime_abort_proposer_session(dag, session_id);
            return Err(error);
        }
        return match crate::dag::dag_manager_runtime_apply_proposer_pack(
            dag,
            session_id,
            true,
            Vec::new(),
        ) {
            Ok(step) => Ok(step),
            Err(error) => {
                crate::dag::dag_manager_runtime_abort_proposer_session(dag, session_id);
                Err(error)
            }
        };
    }

    let owner = TransactionPackSessionOwner::DagProposer(session_id);
    let (mut dag_guard, mut transaction) = service.dag_and_transaction()?;
    let dag = dag_guard
        .as_mut()
        .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
    let params = match crate::dag::dag_manager_runtime_proposer_pack_parameters(dag, session_id) {
        Ok(params) => params,
        Err(error) => {
            transaction.transaction_manager_runtime_pack_abort_for_owner(owner);
            crate::dag::dag_manager_runtime_abort_proposer_session(dag, session_id);
            return Err(error);
        }
    };
    let plan = match transaction.transaction_manager_runtime_pack_prepare_sharded_for_owner(
        owner,
        params.weight_limit,
        min_transaction_gas,
        params.proposal_period,
        estimate_gas_limit,
        last_block_number,
        params.total_transaction_shards,
        params.node_transaction_shard,
        params.shard_period_interval,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            transaction.transaction_manager_runtime_pack_abort_for_owner(owner);
            crate::dag::dag_manager_runtime_abort_proposer_session(dag, session_id);
            return Err(error);
        }
    };

    if plan.request_estimates.is_empty() {
        return match crate::dag::dag_manager_runtime_apply_proposer_pack(
            dag,
            session_id,
            false,
            plan.selected_transactions,
        ) {
            Ok(step) => Ok(step),
            Err(error) => {
                transaction.transaction_manager_runtime_pack_abort_for_owner(owner);
                crate::dag::dag_manager_runtime_abort_proposer_session(dag, session_id);
                Err(error)
            }
        };
    }

    let mut step = crate::dag::dag_manager_runtime_proposer_session_next(dag, session_id);
    step.transaction_estimate_requests = plan.request_estimates;
    Ok(step)
}

/// Finalizes an owner-bound proposer pack after the unlocked EVM executor interval.
///
/// DAG state is validated before transaction mutation. Estimates must exactly match the retained transaction cursor and
/// owner. Success transfers selected hash/RLP/gas payloads directly into the DAG cursor and returns its VDF or terminal
/// step. Wrong owner, malformed estimates, or out-of-order DAG state abort both matching cursors before returning `Err`.
pub fn dag_transaction_service_proposer_pack_finalize(
    service: &BridgeDagTransactionService,
    session_id: u64,
    estimates: Vec<TransactionPackSessionEstimateInput>,
) -> Result<DagProposerSessionStep> {
    let owner = TransactionPackSessionOwner::DagProposer(session_id);
    let (mut dag_guard, mut transaction) = service.dag_and_transaction()?;
    let dag = dag_guard
        .as_mut()
        .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
    if let Err(error) = crate::dag::dag_manager_runtime_proposer_pack_parameters(dag, session_id) {
        transaction.transaction_manager_runtime_pack_abort_for_owner(owner);
        crate::dag::dag_manager_runtime_abort_proposer_session(dag, session_id);
        return Err(error);
    }
    let packed = match transaction
        .transaction_manager_runtime_pack_finalize_with_estimates_for_owner(owner, estimates)
    {
        Ok(packed) => packed,
        Err(error) => {
            transaction.transaction_manager_runtime_pack_abort_for_owner(owner);
            crate::dag::dag_manager_runtime_abort_proposer_session(dag, session_id);
            return Err(error);
        }
    };
    match crate::dag::dag_manager_runtime_apply_proposer_pack(
        dag,
        session_id,
        false,
        packed.selected_transactions,
    ) {
        Ok(step) => Ok(step),
        Err(error) => {
            transaction.transaction_manager_runtime_pack_abort_for_owner(owner);
            crate::dag::dag_manager_runtime_abort_proposer_session(dag, session_id);
            Err(error)
        }
    }
}

/// Idempotently aborts the matching transaction and DAG proposer cursors.
///
/// Transaction-only services return `DAG_SERVICE_UNAVAILABLE` before acquiring or mutating transaction state. The return
/// value is true when either matching cursor was removed; wrong-owner transaction cursors are preserved.
pub fn dag_transaction_service_proposer_pack_abort(
    service: &BridgeDagTransactionService,
    session_id: u64,
) -> Result<bool> {
    let owner = TransactionPackSessionOwner::DagProposer(session_id);
    let (mut dag_guard, mut transaction) = service.dag_and_transaction()?;
    let dag = dag_guard
        .as_mut()
        .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
    let transaction_aborted = transaction.transaction_manager_runtime_pack_abort_for_owner(owner);
    let dag_aborted = crate::dag::dag_manager_runtime_abort_proposer_session(dag, session_id);
    Ok(transaction_aborted || dag_aborted)
}

macro_rules! dag_free_mut_result {
    ($outer:ident, $inner:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty) => {
        pub fn $outer(service: &BridgeDagTransactionService, $( $arg: $ty ),*) -> Result<$ret> {
            let mut guard = service.dag()?;
            let runtime = guard.as_mut().ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
            Ok($inner(runtime, $( $arg ),*))
        }
    };
}

macro_rules! dag_free_mut_fallible {
    ($outer:ident, $inner:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty) => {
        pub fn $outer(service: &BridgeDagTransactionService, $( $arg: $ty ),*) -> Result<$ret> {
            let mut guard = service.dag()?;
            let runtime = guard.as_mut().ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
            $inner(runtime, $( $arg ),*)
        }
    };
}

use crate::dag::*;
dag_free_mut_fallible!(service_dag_manager_runtime_begin_verify_block_session, dag_manager_runtime_begin_verify_block_session(input: DagVerifyBlockSessionInput) -> ());
dag_free_mut_result!(service_dag_manager_runtime_verify_block_session_next, dag_manager_runtime_verify_block_session_next() -> DagVerifyBlockSessionStep);
dag_free_mut_result!(service_dag_manager_runtime_verify_block_session_report_transactions, dag_manager_runtime_verify_block_session_report_transactions(report: DagVerifyBlockTransactionReport) -> DagVerifyBlockSessionStep);
dag_free_mut_result!(service_dag_manager_runtime_verify_block_session_report_authorization, dag_manager_runtime_verify_block_session_report_authorization(report: DagVerifyBlockAuthorizationReport) -> DagVerifyBlockSessionStep);
dag_free_mut_result!(service_dag_manager_runtime_verify_block_session_report_vdf, dag_manager_runtime_verify_block_session_report_vdf(report: DagVerifyBlockVdfReport) -> DagVerifyBlockSessionStep);
dag_free_mut_result!(service_dag_manager_runtime_verify_block_session_report_gas, dag_manager_runtime_verify_block_session_report_gas(report: DagVerifyBlockGasReport) -> DagVerifyBlockSessionStep);
dag_free_mut_fallible!(service_dag_manager_runtime_begin_proposer_session, dag_manager_runtime_begin_proposer_session(input: DagProposerSessionBeginInput) -> u64);
dag_free_mut_result!(service_dag_manager_runtime_abort_proposer_session, dag_manager_runtime_abort_proposer_session(session_id: u64) -> bool);
dag_free_mut_result!(service_dag_manager_runtime_proposer_session_next, dag_manager_runtime_proposer_session_next(session_id: u64) -> DagProposerSessionStep);
dag_free_mut_fallible!(service_dag_manager_runtime_proposer_session_report_external_facts, dag_manager_runtime_proposer_session_report_external_facts(session_id: u64, report: DagProposerExternalProposalFactsReport) -> DagProposerSessionStep);
dag_free_mut_result!(service_dag_manager_runtime_proposer_session_poll_vdf, dag_manager_runtime_proposer_session_poll_vdf(session_id: u64) -> DagProposerSessionStep);
dag_free_mut_fallible!(service_dag_manager_runtime_proposer_session_report_vdf_proof, dag_manager_runtime_proposer_session_report_vdf_proof(session_id: u64, report: DagProposerVdfProofReport) -> DagProposerSessionStep);
dag_free_mut_fallible!(service_dag_manager_runtime_proposer_session_resume_stale_proof, dag_manager_runtime_proposer_session_resume_stale_proof(session_id: u64) -> DagProposerSessionStep);
dag_free_mut_fallible!(service_dag_manager_runtime_proposer_session_report_signing, dag_manager_runtime_proposer_session_report_signing(session_id: u64, report: DagProposerSigningReport) -> DagProposerSessionStep);
dag_free_mut_result!(service_dag_manager_runtime_proposer_session_report_add_block, dag_manager_runtime_proposer_session_report_add_block(session_id: u64, report: DagProposerAddBlockReport) -> DagProposerSessionStep);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::create_storage;
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_types::LegacyTransactionEnvelope;
    use rustaxa_vdf::vrf::public_key_from_secret;
    use std::fs;
    use tiny_keccak::{Hasher, Keccak};

    const SECRET_KEY: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn queue_config() -> TransactionQueueConfig {
        TransactionQueueConfig { max_size: 100 }
    }
    fn gas_config() -> GasPricerConfig {
        GasPricerConfig {
            percentile: 50,
            minimum_price: [0; 32],
            history_blocks: 10,
            is_light_node: false,
            blocks_gas_pricer: false,
        }
    }

    fn keccak256(bytes: &[u8]) -> H256 {
        let mut out = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(bytes);
        hasher.finalize(&mut out);
        H256::from(out)
    }

    fn address_from_signing_key(signing_key: &SigningKey) -> [u8; 20] {
        let point = signing_key.verifying_key().to_encoded_point(false);
        let hash = keccak256(&point.as_bytes()[1..]);
        hash.as_bytes()[12..].try_into().unwrap()
    }

    fn signed_legacy_transaction_rlp(signing_key: &SigningKey) -> Vec<u8> {
        let chain_id = 2999u64;
        let mut unsigned = RlpStream::new_list(9);
        unsigned.append(&U256::from(1));
        unsigned.append(&U256::from(2));
        unsigned.append(&21_000u64);
        unsigned.append(&H160::from([0x44; 20]));
        unsigned.append(&U256::from(3));
        unsigned.append(&Vec::<u8>::new());
        unsigned.append(&U256::from(chain_id));
        unsigned.append(&U256::zero());
        unsigned.append(&U256::zero());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(keccak256(&unsigned.out()).as_bytes())
            .unwrap();
        let signature = signature.to_bytes();
        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::from(1));
        stream.append(&U256::from(2));
        stream.append(&21_000u64);
        stream.append(&H160::from([0x44; 20]));
        stream.append(&U256::from(3));
        stream.append(&Vec::<u8>::new());
        stream.append(&U256::from(
            chain_id * 2 + 35 + u64::from(recovery_id.to_byte()),
        ));
        stream.append(&U256::from_big_endian(&signature[..32]));
        stream.append(&U256::from_big_endian(&signature[32..]));
        stream.out().to_vec()
    }

    fn sign_hash(hash: [u8; 32]) -> Vec<u8> {
        let key = SigningKey::from_slice(&[0x44; 32]).unwrap();
        let (signature, recovery_id) = key.sign_prehash_recoverable(&hash).unwrap();
        let mut bytes = signature.to_bytes().to_vec();
        bytes.push(recovery_id.to_byte());
        bytes
    }

    fn proposer_begin_input() -> DagProposerSessionBeginInput {
        let vrf_key = public_key_from_secret(&SECRET_KEY).expect("VRF key");
        DagProposerSessionBeginInput {
            transaction_pool_size: 1,
            non_finalized_transaction_count: 0,
            max_non_finalized_transactions: 100,
            dag_expiry_level_limit: 100,
            wallet_vrf_public_key: vrf_key,
            wallet_vrf_secret: SECRET_KEY,
            proposer_address: address_from_signing_key(
                &SigningKey::from_slice(&[0x44; 32]).unwrap(),
            ),
            max_non_finalized_dag_blocks: 100,
            max_non_finalized_dag_blocks_low_difficulty: 50,
            max_retry_count: 20,
            proposal_weight_limit: 100_000,
            total_transaction_shards: 1,
            node_transaction_shard: 0,
            shard_period_interval: 10,
            pbft_gas_limit: 1_000_000,
            dag_gas_limit: 100_000,
            max_tips: 16,
        }
    }

    fn open_proposer_pack(service: &BridgeDagTransactionService) -> u64 {
        let vrf_key = public_key_from_secret(&SECRET_KEY).expect("VRF key");
        let session_id =
            service_dag_manager_runtime_begin_proposer_session(service, proposer_begin_input())
                .expect("proposer session");
        let step = service_dag_manager_runtime_proposer_session_report_external_facts(
            service,
            session_id,
            DagProposerExternalProposalFactsReport {
                last_finalized_period: 0,
                authorization_facts: DagDposAuthorizationFacts {
                    vrf_key_found: true,
                    vrf_key: vrf_key.to_vec(),
                    sender_eligible_vote_count: 10,
                    vdf_sortition_max_vote_count: 20,
                    eligibility_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
                },
                sortition_params: SortitionRuntimeParams {
                    threshold_upper: u16::MAX,
                    difficulty_min: 3,
                    difficulty_max: 3,
                    difficulty_stale: 9,
                    lambda_bound: 128,
                },
            },
        )
        .expect("external facts");
        assert_eq!(step.action, 1);
        session_id
    }

    #[test]
    fn full_service_restores_both_domains_and_transaction_only_rejects_dag() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let full = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        assert_eq!(full.transaction_manager_runtime_transaction_count(), 0);
        assert_eq!(full.dag_manager_runtime_vertex_count().unwrap(), 1);
        assert!(!full
            .dag_manager_runtime_ensure_proposal_period_mapping(100, 0)
            .unwrap());

        let restored = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        assert_eq!(restored.dag_manager_runtime_latest_period().unwrap(), 0);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert!(!full.transaction_manager_runtime_pack_abort());
            });
            scope.spawn(|| {
                full.dag_manager_runtime_restore_from_storage().unwrap();
            });
        });

        let compat = create_dag_transaction_service_for_transaction_manager(
            &storage,
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        assert_eq!(compat.transaction_manager_runtime_transaction_count(), 0);
        let error = match compat.dag() {
            Ok(_) => panic!("transaction-only service unexpectedly exposed DAG state"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "DAG_SERVICE_UNAVAILABLE");
        assert_eq!(
            compat
                .dag_manager_runtime_vertex_count()
                .unwrap_err()
                .to_string(),
            "DAG_SERVICE_UNAVAILABLE"
        );
        let gas_pricer = create_dag_transaction_service_for_gas_pricer(gas_config()).unwrap();
        assert_eq!(
            gas_pricer.transaction_manager_runtime_transaction_count(),
            0
        );
        assert_eq!(
            gas_pricer
                .dag_manager_runtime_vertex_count()
                .unwrap_err()
                .to_string(),
            "DAG_SERVICE_UNAVAILABLE"
        );

        drop(gas_pricer);
        drop(compat);
        drop(restored);
        drop(full);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_pack_throttle_and_empty_pack_leave_no_transaction_cursor() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_pack_terminal");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();

        let throttled_id = open_proposer_pack(&service);
        let throttled = service
            .dag_transaction_service_proposer_pack_prepare(throttled_id, true, 21_000, 0, 0)
            .expect("throttle should terminate");
        assert_eq!(throttled.status, 1);
        assert_eq!(
            throttled.reason_code,
            rustaxa_consensus::dag::DAG_PROPOSER_REASON_TRANSACTION_PACK_THROTTLED
        );
        assert!(service.transaction().transaction_pack_session.is_none());

        let empty_id = open_proposer_pack(&service);
        let empty = service
            .dag_transaction_service_proposer_pack_prepare(empty_id, false, 21_000, 0, 0)
            .expect("empty pack should terminate");
        assert_eq!(empty.status, 1);
        assert_eq!(
            empty.reason_code,
            rustaxa_consensus::dag::DAG_PROPOSER_REASON_PACKED_TRANSACTIONS_EMPTY
        );
        assert!(service.transaction().transaction_pack_session.is_none());
        assert!(!service
            .dag_transaction_service_proposer_pack_abort(empty_id)
            .unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn out_of_order_throttled_pack_removes_only_dag_cursor() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_pack_throttle_order");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let session_id =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .expect("session should begin in external-facts stage");
        assert!(service.transaction().transaction_pack_session.is_none());

        let error = service
            .dag_transaction_service_proposer_pack_prepare(session_id, true, 21_000, 0, 0)
            .err()
            .expect("out-of-order throttle must fail");
        assert!(error
            .to_string()
            .contains("DAG_PROPOSER_PACK_SESSION_WRONG_STAGE"));
        assert!(service.transaction().transaction_pack_session.is_none());
        let after = service_dag_manager_runtime_proposer_session_next(&service, session_id)
            .expect("missing session should return invalid step");
        assert_eq!(after.status, 2);
        assert!(!service
            .dag_transaction_service_proposer_pack_abort(session_id)
            .unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_pack_estimate_finalize_retains_payload_and_cache_only_reuses_it() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_pack_estimate");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let key = SigningKey::from_slice(&[0x47; 32]).unwrap();
        let tx_rlp = signed_legacy_transaction_rlp(&key);
        let envelope = LegacyTransactionEnvelope::decode(&tx_rlp).unwrap();
        service
            .transaction()
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: envelope.hash.0,
                sender: address_from_signing_key(&key),
                nonce: envelope.nonce.to_big_endian(),
                gas_price: envelope.gas_price.to_big_endian(),
                gas: envelope.gas,
                data_size: envelope.data.len(),
                tx_rlp: tx_rlp.clone(),
                proposable: true,
                last_block_number: 0,
            })
            .unwrap();

        let session_id = open_proposer_pack(&service);
        let prepare = service
            .dag_transaction_service_proposer_pack_prepare(session_id, false, 21_000, 0, 10)
            .expect("estimate prepare");
        assert_eq!(prepare.action, 1);
        assert_eq!(prepare.transaction_estimate_requests.len(), 1);
        assert!(prepare.selected_transactions.is_empty());
        let estimate = &prepare.transaction_estimate_requests[0];
        let start_vdf = service
            .dag_transaction_service_proposer_pack_finalize(
                session_id,
                vec![TransactionPackSessionEstimateInput {
                    hash: estimate.hash,
                    gas_used: 21_000,
                    last_block_number: 10,
                    result_rlp: vec![0xC0],
                }],
            )
            .expect("estimate finalize");
        assert_eq!(start_vdf.action, 2);
        assert!(start_vdf.transaction_estimate_requests.is_empty());
        assert!(start_vdf.selected_transactions.is_empty());

        let sign = service_dag_manager_runtime_proposer_session_report_vdf_proof(
            &service,
            session_id,
            DagProposerVdfProofReport {
                proof_ok: true,
                vdf_rlp: vec![0xC0],
            },
        )
        .expect("VDF proof");
        let add = service_dag_manager_runtime_proposer_session_report_signing(
            &service,
            session_id,
            DagProposerSigningReport {
                signature: sign_hash(sign.signing_hash),
            },
        )
        .expect("signing");
        assert_eq!(add.action, 6);
        assert_eq!(add.selected_transactions.len(), 1);
        assert_eq!(add.selected_transactions[0].hash, envelope.hash.0);
        assert_eq!(add.selected_transactions[0].gas_used, 21_000);
        assert_eq!(add.selected_transactions[0].tx_rlp, tx_rlp);
        service_dag_manager_runtime_proposer_session_report_add_block(
            &service,
            session_id,
            DagProposerAddBlockReport {
                accepted: true,
                duplicate: false,
                expired: false,
                missing_references: Vec::new(),
            },
        )
        .expect("add report");

        let cache_id = open_proposer_pack(&service);
        let cache_only = service
            .dag_transaction_service_proposer_pack_prepare(cache_id, false, 21_000, 0, 10)
            .expect("cache-only prepare");
        assert_eq!(cache_only.action, 2);
        assert!(cache_only.transaction_estimate_requests.is_empty());
        assert!(service.transaction().transaction_pack_session.is_none());
        assert!(service
            .dag_transaction_service_proposer_pack_abort(cache_id)
            .unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn transaction_only_proposer_pack_fails_before_transaction_mutation() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_pack_unavailable");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_for_transaction_manager(
            &storage,
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();

        let error = service
            .dag_transaction_service_proposer_pack_prepare(7, false, 21_000, 0, 0)
            .err()
            .expect("transaction-only service must reject proposer pack");
        assert_eq!(error.to_string(), "DAG_SERVICE_UNAVAILABLE");
        assert!(service.transaction().transaction_pack_session.is_none());
        assert_eq!(
            service
                .dag_transaction_service_proposer_pack_abort(7)
                .unwrap_err()
                .to_string(),
            "DAG_SERVICE_UNAVAILABLE"
        );
        assert!(service.transaction().transaction_pack_session.is_none());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_pack_finalize_failure_cleans_both_owned_cursors() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_pack_failure");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let key = SigningKey::from_slice(&[0x48; 32]).unwrap();
        let tx_rlp = signed_legacy_transaction_rlp(&key);
        let envelope = LegacyTransactionEnvelope::decode(&tx_rlp).unwrap();
        service
            .transaction()
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: envelope.hash.0,
                sender: address_from_signing_key(&key),
                nonce: envelope.nonce.to_big_endian(),
                gas_price: envelope.gas_price.to_big_endian(),
                gas: envelope.gas,
                data_size: envelope.data.len(),
                tx_rlp,
                proposable: true,
                last_block_number: 0,
            })
            .unwrap();
        let session_id = open_proposer_pack(&service);
        let prepare = service
            .dag_transaction_service_proposer_pack_prepare(session_id, false, 21_000, 0, 10)
            .unwrap();
        assert_eq!(prepare.transaction_estimate_requests.len(), 1);

        assert!(service
            .dag_transaction_service_proposer_pack_finalize(session_id, Vec::new())
            .is_err());
        assert!(service.transaction().transaction_pack_session.is_none());
        assert!(!service
            .dag_transaction_service_proposer_pack_abort(session_id)
            .unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }
}
