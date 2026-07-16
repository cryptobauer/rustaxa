use crate::dag::build_dag_state_from_storage;
use crate::ffi::rustaxa_ffi::*;
use crate::ffi::{BridgeStorage, DagRuntimeState, TransactionRuntimeState};
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
dag_free_mut_result!(service_dag_manager_runtime_proposer_session_report_transactions, dag_manager_runtime_proposer_session_report_transactions(session_id: u64, report: DagProposerTransactionPackReport) -> DagProposerSessionStep);
dag_free_mut_result!(service_dag_manager_runtime_proposer_session_poll_vdf, dag_manager_runtime_proposer_session_poll_vdf(session_id: u64) -> DagProposerSessionStep);
dag_free_mut_fallible!(service_dag_manager_runtime_proposer_session_report_vdf_proof, dag_manager_runtime_proposer_session_report_vdf_proof(session_id: u64, report: DagProposerVdfProofReport) -> DagProposerSessionStep);
dag_free_mut_fallible!(service_dag_manager_runtime_proposer_session_resume_stale_proof, dag_manager_runtime_proposer_session_resume_stale_proof(session_id: u64) -> DagProposerSessionStep);
dag_free_mut_fallible!(service_dag_manager_runtime_proposer_session_report_signing, dag_manager_runtime_proposer_session_report_signing(session_id: u64, report: DagProposerSigningReport) -> DagProposerSessionStep);
dag_free_mut_result!(service_dag_manager_runtime_proposer_session_report_add_block, dag_manager_runtime_proposer_session_report_add_block(session_id: u64, report: DagProposerAddBlockReport) -> DagProposerSessionStep);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::create_storage;
    use std::fs;

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
}
