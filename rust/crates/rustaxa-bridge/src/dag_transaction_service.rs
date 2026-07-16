use crate::dag::{build_dag_state_from_storage, DagAddBlockEffectPlan, DagAddBlockRuntimeInput};
use crate::ffi::rustaxa_ffi::*;
use crate::ffi::{
    BridgeStorage, DagRuntimeState, TransactionPackSessionOwner, TransactionRuntimeState,
};
use crate::transaction::legacy_transaction_inspection_from_bytes;
use crate::transaction_manager::{
    append_prepared_dag_transactions_to_batch, build_transaction_state_for_gas_pricer,
    build_transaction_state_from_storage, dag_save_command_report,
    prepare_dag_transaction_publication, prepare_transactions_from_dag_block_with_runtime,
    publish_prepared_dag_transactions,
};
use anyhow::{anyhow, ensure, Context, Result};
use ethereum_types::H256;
use rustaxa_consensus::dag::dag_block_transaction_hashes;
use std::collections::BTreeMap;
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

    /// Commits finalized DAG cleanup, then clears matching private transaction sidecars.
    ///
    /// Both runtimes are locked DAG then transaction. Transaction state remains untouched when the fallible DAG/storage
    /// phase fails. After a successful DAG commit, sidecar removal is infallible because storage deletion already
    /// occurred inside the DAG runtime. Only finalized count and expired DAG hashes cross CXX.
    pub fn dag_manager_runtime_apply_finalized_order(
        &self,
        new_anchor: [u8; 32],
        new_period: u64,
        finalized_order: Vec<DagHash>,
    ) -> Result<DagManagerFinalizationApplyPayload> {
        let (mut dag_guard, mut transaction) = self.dag_and_transaction()?;
        let dag = dag_guard
            .as_mut()
            .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
        let committed =
            dag.dag_manager_runtime_apply_finalized_order(new_anchor, new_period, finalized_order)?;
        transaction
            .remove_non_finalized_sidecars_after_dag_commit(&committed.remove_transaction_hashes);
        Ok(committed.payload)
    }

    dag_shared_result!(dag_manager_runtime_validate_pivot_tips(block_level: u64, pivot: &[u8; 32], tips: Vec<DagHash>) -> DagPivotTipsValidation);
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
    dag_shared_result!(dag_manager_runtime_load_block(hash: &[u8; 32]) -> DagBlockLookup);
    dag_shared_result!(dag_manager_runtime_plan_proposal_tip_selection(input: DagProposerStorageTipSelectionInput) -> DagProposerTipSelectionPlan);
    dag_shared_result!(dag_manager_runtime_period_block_hash(period: u64) -> HashLookup);
    dag_shared_result!(dag_manager_runtime_persistence_counters() -> DagPersistenceCounters);

    /// Prepares one canonical add-block transition without mutating DAG,
    /// transaction, or storage state.
    pub fn dag_transaction_service_prepare_add_block(
        &self,
        input: DagAddBlockPrepareInput,
    ) -> Result<DagAddBlockPreparation> {
        let (mut dag_guard, _transaction) = self.dag_and_transaction()?;
        let dag = dag_guard
            .as_mut()
            .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
        ensure!(
            dag.pending_add_block.is_none(),
            "DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"
        );
        let mut block = rustaxa_consensus::dag::dag_manager_block_from_rlp(&input.block_rlp)
            .context("DAG_ADD_BLOCK_PREPARE_DECODE")?;
        if input.validate_block_hash {
            ensure!(
                block.hash == H256::from(input.expected_block_hash),
                "DAG_ADD_BLOCK_PREPARE_HASH_MISMATCH"
            );
        } else {
            block.hash = H256::from(input.expected_block_hash);
        }
        let runtime_input = add_block_runtime_input(&block, input.save, input.proposed);
        let plan = dag.dag_manager_runtime_plan_add_block(runtime_input)?;
        if !plan.accepted || plan.duplicate || plan.expired {
            return Ok(DagAddBlockPreparation {
                cursor_id: 0,
                block_level: block.level,
                accepted: plan.accepted,
                duplicate: plan.duplicate,
                expired: plan.expired,
                missing_references: plan.missing_references,
                account_requests: Vec::new(),
            });
        }

        let mut transactions = Vec::new();
        let mut account_requests = Vec::new();
        if plan.persist_transactions {
            let expected_hashes = if input.validate_block_hash {
                let block_transaction_hashes = dag_block_transaction_hashes(&input.block_rlp)
                    .context("DAG_ADD_BLOCK_PREPARE_TRANSACTION_HASHES")?;
                ensure!(
                    block_transaction_hashes.len() == input.transactions.len(),
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_COUNT_MISMATCH"
                );
                block_transaction_hashes
            } else {
                input
                    .transactions
                    .iter()
                    .map(|payload| H256::from(payload.hash))
                    .collect()
            };
            for (input_index, (expected_hash, payload)) in expected_hashes
                .into_iter()
                .zip(input.transactions)
                .enumerate()
            {
                ensure!(
                    expected_hash == H256::from(payload.hash),
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_ORDER_MISMATCH"
                );
                let inspection = legacy_transaction_inspection_from_bytes(&payload.trx_rlp, 0)
                    .context("DAG_ADD_BLOCK_PREPARE_TRANSACTION_DECODE")?;
                ensure!(
                    inspection.hash == payload.hash,
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_HASH_MISMATCH"
                );
                ensure!(
                    inspection.sender_found,
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_SENDER_MISSING"
                );
                transactions.push(crate::dag::DagAddBlockPreparedTransaction {
                    input_index: input_index as u64,
                    hash: expected_hash,
                    trx_rlp: payload.trx_rlp,
                    transaction_nonce: inspection.nonce,
                });
                account_requests.push(DagAddBlockAccountRequest {
                    input_index: input_index as u64,
                    sender: inspection.sender,
                });
            }
        }

        let cursor_id = dag.next_add_block_session_id;
        dag.next_add_block_session_id = cursor_id.wrapping_add(1).max(1);
        let stored_plan = stored_add_block_plan(&plan);
        let block_level = block.level;
        dag.pending_add_block = Some(crate::dag::DagAddBlockSession {
            cursor_id,
            block,
            block_rlp: input.block_rlp,
            save: input.save,
            proposed: input.proposed,
            transactions,
            plan: stored_plan,
        });
        Ok(DagAddBlockPreparation {
            cursor_id,
            block_level,
            accepted: true,
            duplicate: false,
            expired: false,
            missing_references: Vec::new(),
            account_requests,
        })
    }

    /// Completes one prepared add-block transition through a single durable
    /// storage batch, then publishes prevalidated live state.
    pub fn dag_transaction_service_complete_add_block(
        &self,
        input: DagAddBlockCompletionInput,
    ) -> Result<DagAddBlockCommitReport> {
        let (mut dag_guard, mut transaction) = self.dag_and_transaction()?;
        let dag = dag_guard
            .as_mut()
            .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
        let session = dag
            .pending_add_block
            .as_ref()
            .context("DAG_ADD_BLOCK_SESSION_NOT_STARTED")?
            .clone();
        ensure!(
            session.cursor_id == input.cursor_id,
            "DAG_ADD_BLOCK_SESSION_CURSOR_MISMATCH"
        );
        let current_plan = dag.dag_manager_runtime_plan_add_block(add_block_runtime_input(
            &session.block,
            session.save,
            session.proposed,
        ))?;
        ensure!(
            stored_add_block_plan(&current_plan) == session.plan
                && current_plan.accepted
                && !current_plan.duplicate
                && !current_plan.expired,
            "DAG_ADD_BLOCK_SESSION_STALE_PLAN"
        );

        let mut nonce_facts = BTreeMap::new();
        for fact in input.account_nonce_facts {
            ensure!(
                nonce_facts
                    .insert(fact.input_index, fact.account_nonce)
                    .is_none(),
                "DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_DUPLICATE"
            );
        }
        ensure!(
            nonce_facts.len() == session.transactions.len(),
            "DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_COUNT_MISMATCH"
        );
        let transaction_facts = session
            .transactions
            .iter()
            .map(|transaction| {
                Ok(DagTransactionSaveSidecarFact {
                    input_index: transaction.input_index,
                    hash: transaction.hash.0,
                    trx_rlp: transaction.trx_rlp.clone(),
                    transaction_nonce: transaction.transaction_nonce,
                    sender_account_nonce: *nonce_facts
                        .get(&transaction.input_index)
                        .context("DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_MISSING")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let prepared_transactions = if session.plan.persist_transactions {
            Some(prepare_transactions_from_dag_block_with_runtime(
                &transaction,
                transaction_facts,
            )?)
        } else {
            None
        };
        let prepared_transaction_publication = prepared_transactions
            .as_ref()
            .map(|prepared| prepare_dag_transaction_publication(&transaction, prepared))
            .transpose()?;
        let mut next_state = dag.state.clone();
        if session.plan.add_to_graph {
            next_state
                .add_block(session.block.clone())
                .context("DAG_ADD_BLOCK_GRAPH_PREVALIDATE")?;
        }

        let transaction_report = prepared_transaction_publication
            .as_ref()
            .map(|publication| dag_save_command_report(&publication.outcome))
            .unwrap_or(TransactionManagerDagSaveCommandReport {
                queue_erased: Vec::new(),
            });
        let counters;
        let mut pending_batch = None;
        if session.plan.persist_block {
            let transaction_storage = transaction
                .storage
                .as_ref()
                .context("TM_RUNTIME_STORAGE_UNAVAILABLE")?;
            ensure!(
                std::sync::Arc::ptr_eq(&dag.storage, transaction_storage),
                "DAG_ADD_BLOCK_STORAGE_OWNER_MISMATCH"
            );
            let mut batch = dag.storage.create_write_batch();
            if let Some(prepared) = prepared_transactions.as_ref() {
                if !session.transactions.is_empty() {
                    append_prepared_dag_transactions_to_batch(
                        dag.storage.as_ref(),
                        &mut batch,
                        prepared,
                    )?;
                }
            }
            let (dag_blocks, dag_edges) = dag.storage.dag().append_write_to_batch(
                &mut batch,
                session.block.hash,
                session.block.level,
                session.block.tips.len() as u64,
                &session.block_rlp,
            )?;
            counters = DagPersistenceCounters {
                dag_blocks,
                dag_edges,
            };
            pending_batch = Some(batch);
        } else {
            counters = dag.dag_manager_runtime_persistence_counters()?;
        }

        let removed_session = dag
            .pending_add_block
            .take()
            .context("DAG_ADD_BLOCK_SESSION_DISAPPEARED_BEFORE_COMMIT")?;
        if let Some(batch) = pending_batch {
            if let Err(error) = dag.storage.commit_write_batch_with_sync(batch, false) {
                dag.pending_add_block = Some(removed_session);
                return Err(error).context("DAG_ADD_BLOCK_BATCH_COMMIT");
            }
        }
        dag.state = next_state;
        if let Some(publication) = prepared_transaction_publication {
            let _ = publish_prepared_dag_transactions(&mut transaction, publication);
        }
        Ok(DagAddBlockCommitReport {
            accepted: true,
            emit_verified: session.plan.emit_verified,
            gossip: session.plan.gossip,
            proposed: session.plan.proposed,
            queue_erased: transaction_report.queue_erased,
            counters,
        })
    }

    /// Idempotently aborts the matching add-block cursor without affecting a
    /// newer or unrelated preparation.
    pub fn dag_transaction_service_abort_add_block(&self, cursor_id: u64) -> Result<bool> {
        let mut dag_guard = self.dag()?;
        let dag = dag_guard
            .as_mut()
            .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
        if dag
            .pending_add_block
            .as_ref()
            .is_some_and(|session| session.cursor_id == cursor_id)
        {
            dag.pending_add_block = None;
            return Ok(true);
        }
        Ok(false)
    }
}

fn add_block_runtime_input(
    block: &rustaxa_consensus::dag::DagManagerBlock,
    save: bool,
    proposed: bool,
) -> DagAddBlockRuntimeInput {
    DagAddBlockRuntimeInput {
        save,
        proposed,
        block_hash: block.hash.0,
        pivot: block.pivot.0,
        tips: block
            .tips
            .iter()
            .map(|tip| DagHash { hash: tip.0 })
            .collect(),
        block_level: block.level,
    }
}

fn stored_add_block_plan(plan: &DagAddBlockEffectPlan) -> crate::dag::DagAddBlockStoredPlan {
    crate::dag::DagAddBlockStoredPlan {
        accepted: plan.accepted,
        persist_transactions: plan.persist_transactions,
        persist_block: plan.persist_block,
        add_to_graph: plan.add_to_graph,
        emit_verified: plan.emit_verified,
        gossip: plan.gossip,
        proposed: plan.proposed,
    }
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

/// Prepares the active DAG verification transaction query without advancing it.
///
/// Locks are acquired DAG then transaction. Query hashes and proposal period
/// remain Rust-private. The returned cursor identity and proposal period bind a
/// later completion to this exact session after C++ has materialized and
/// hash-validated every returned payload.
pub fn service_dag_manager_runtime_verify_block_session_prepare_transactions(
    service: &BridgeDagTransactionService,
) -> Result<DagVerifyBlockTransactionPreparation> {
    let (dag_guard, transaction) = service.dag_and_transaction()?;
    let dag = dag_guard
        .as_ref()
        .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
    let query = dag_manager_runtime_verify_block_transaction_query(dag)?;
    let requests = verify_block_transaction_view_requests(&query);
    let plan = transaction.transaction_manager_runtime_lookup_transaction_views(
        requests,
        query.hashes.len() as u64,
    )?;
    Ok(DagVerifyBlockTransactionPreparation {
        cursor_id: query.cursor_id,
        proposal_period: query.proposal_period,
        transactions: plan.views,
    })
}

/// Completes prepared transaction availability after C++ materialization.
///
/// Cursor and proposal-period identity are checked before TransactionManager
/// lookup. Finalized-storage senders require explicit account facts, and all
/// proposal-period filtering completes before the DAG action advances. Any
/// identity or lookup error leaves the session unchanged.
pub fn service_dag_manager_runtime_verify_block_session_complete_transactions(
    service: &BridgeDagTransactionService,
    report: DagVerifyBlockTransactionCompletionReport,
) -> Result<DagVerifyBlockSessionStep> {
    let (mut dag_guard, transaction) = service.dag_and_transaction()?;
    let dag = dag_guard
        .as_mut()
        .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
    let query = dag_manager_runtime_validate_verify_block_transaction_completion(
        dag,
        report.cursor_id,
        report.proposal_period,
    )?;
    let requests = verify_block_transaction_view_requests(&query);
    let plan = transaction
        .transaction_manager_runtime_lookup_proposal_transaction_views_requiring_account_nonce_facts(
            query.proposal_period,
            requests,
            report.account_nonce_facts,
            query.hashes.len() as u64,
        )?;
    let all_resolved = plan.complete
        && plan.views.len() == query.hashes.len()
        && plan
            .views
            .iter()
            .all(|view| view.found && !view.old_finalized);
    let resolved_transactions = if all_resolved {
        query.expected_transactions
    } else {
        0
    };
    Ok(
        dag_manager_runtime_verify_block_session_apply_transaction_resolution(
            dag,
            resolved_transactions,
        ),
    )
}

fn verify_block_transaction_view_requests(
    query: &DagVerifyBlockTransactionQuery,
) -> Vec<TransactionManagerTransactionViewRequest> {
    query
        .hashes
        .iter()
        .enumerate()
        .map(
            |(input_index, hash)| TransactionManagerTransactionViewRequest {
                input_index: input_index as u64,
                hash: hash.0,
            },
        )
        .collect()
}
dag_free_mut_result!(service_dag_manager_runtime_verify_block_session_report_authorization, dag_manager_runtime_verify_block_session_report_authorization(report: DagVerifyBlockAuthorizationReport) -> DagVerifyBlockSessionStep);
dag_free_mut_result!(service_dag_manager_runtime_verify_block_session_report_vdf, dag_manager_runtime_verify_block_session_report_vdf(report: DagVerifyBlockVdfReport) -> DagVerifyBlockSessionStep);
dag_free_mut_fallible!(service_dag_manager_runtime_verify_block_session_report_gas, dag_manager_runtime_verify_block_session_report_gas(report: DagVerifyBlockGasReport) -> DagVerifyBlockSessionStep);
/// Opens a proposer session with transaction pressure derived from the sibling
/// Rust TransactionManager while holding the universal DAG-then-transaction lock order.
///
/// CXX supplies wallet and configuration inputs only. Queue and non-finalized
/// sidecar counts are captured with session creation and retained by the DAG
/// cursor for threshold and skip decisions.
pub fn service_dag_manager_runtime_begin_proposer_session(
    service: &BridgeDagTransactionService,
    input: DagProposerSessionBeginInput,
) -> Result<u64> {
    let (mut dag_guard, transaction) = service.dag_and_transaction()?;
    let dag = dag_guard
        .as_mut()
        .ok_or_else(|| anyhow!("DAG_SERVICE_UNAVAILABLE"))?;
    let transaction_observation = DagProposerTransactionObservation {
        transaction_pool_size: transaction.transaction_manager_runtime_queue_size() as u64,
        non_finalized_transaction_count: transaction
            .transaction_manager_runtime_non_finalized_size()
            as u64,
    };
    dag_manager_runtime_begin_proposer_session(dag, input, transaction_observation)
}
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
    use crate::dag::DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS;
    use crate::storage::{create_storage, create_transaction_storage_queries};
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_types::LegacyTransactionEnvelope;
    use rustaxa_vdf::vrf::public_key_from_secret;
    use std::fs;
    use std::sync::{Arc, Barrier};
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

    fn append_pbft_block_fields(stream: &mut RlpStream, period: u64) {
        stream.append(&H256::from_low_u64_be(10));
        stream.append(&H256::from_low_u64_be(11));
        stream.append(&H256::from_low_u64_be(12));
        stream.append(&H256::from_low_u64_be(13));
        stream.append(&period);
        stream.append(&1_000u64);
        stream.begin_list(0);
    }

    fn signed_pbft_block(signing_key: &SigningKey, period: u64) -> Vec<u8> {
        let mut unsigned = RlpStream::new_list(7);
        append_pbft_block_fields(&mut unsigned, period);
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(keccak256(&unsigned.out()).as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());
        let mut signed = RlpStream::new_list(8);
        append_pbft_block_fields(&mut signed, period);
        signed.append(&signature_bytes);
        signed.out().to_vec()
    }

    fn period_data_rlp(pbft_block: &[u8], transaction_rlp: &[u8]) -> Vec<u8> {
        let mut transactions = RlpStream::new_list(1);
        transactions.append_raw(transaction_rlp, 1);
        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(pbft_block, 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&transactions.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.out().to_vec()
    }

    fn dag_block_rlp(level: u64, transactions: &[[u8; 32]]) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from([1u8; 32]));
        block.append(&level);
        block.append(&0u64);
        block.append(&vec![0xC0]);
        block.begin_list(0);
        block.begin_list(transactions.len());
        for hash in transactions {
            block.append(&H256::from(*hash));
        }
        block.append(&&[0u8; 65][..]);
        block.append(&0u64);
        block.out().to_vec()
    }

    fn composed_add_block_rlp(pivot: [u8; 32], level: u64, transactions: &[[u8; 32]]) -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11u8; 80]);
        vdf.append(&vec![0x22u8]);
        vdf.append(&vec![0x33u8]);
        vdf.append(&1u16);
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from(pivot));
        block.append(&level);
        block.append(&0u64);
        block.append(&vdf.out().to_vec());
        block.begin_list(0);
        block.begin_list(transactions.len());
        for hash in transactions {
            block.append(&H256::from(*hash));
        }
        block.append(&&[0u8; 65][..]);
        block.append(&0u64);
        block.out().to_vec()
    }

    fn add_block_prepare_input(
        block_rlp: Vec<u8>,
        save: bool,
        transactions: Vec<DagAddBlockTransactionPayload>,
    ) -> DagAddBlockPrepareInput {
        DagAddBlockPrepareInput {
            expected_block_hash: keccak256(&block_rlp).0,
            validate_block_hash: true,
            block_rlp,
            save,
            proposed: true,
            transactions,
        }
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
        let session_id =
            service_dag_manager_runtime_begin_proposer_session(service, proposer_begin_input())
                .expect("proposer session");
        let step = report_proposer_external_facts(service, session_id);
        assert_eq!(step.action, 1);
        session_id
    }

    fn report_proposer_external_facts(
        service: &BridgeDagTransactionService,
        session_id: u64,
    ) -> DagProposerSessionStep {
        let vrf_key = public_key_from_secret(&SECRET_KEY).expect("VRF key");
        service_dag_manager_runtime_proposer_session_report_external_facts(
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
        .expect("external facts")
    }

    fn insert_test_queue_transaction(
        service: &BridgeDagTransactionService,
        secret: u8,
    ) -> LegacyTransactionEnvelope {
        let key = SigningKey::from_slice(&[secret; 32]).unwrap();
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
        envelope
    }

    fn begin_verify_block_session(
        service: &BridgeDagTransactionService,
        block_hashes: &[[u8; 32]],
        supplied_hashes: &[[u8; 32]],
    ) {
        service_dag_manager_runtime_begin_verify_block_session(
            service,
            DagVerifyBlockSessionInput {
                block_level: 1,
                pivot: [1; 32],
                tips: Vec::new(),
                block_transaction_hashes: block_hashes
                    .iter()
                    .map(|hash| DagTransactionHash { hash: *hash })
                    .collect(),
                supplied_transaction_hashes: supplied_hashes
                    .iter()
                    .map(|hash| DagTransactionHash { hash: *hash })
                    .collect(),
            },
        )
        .expect("verify-block session should begin");
    }

    #[test]
    fn proposer_session_start_derives_queue_and_sidecar_pressure_for_skip_thresholds() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_proposer_pressure");
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

        // The caller-shaped input contains configuration only; the empty sibling
        // runtime drives the legacy empty-pool skip.
        let empty_id =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();
        let empty = report_proposer_external_facts(&service, empty_id);
        assert_eq!(empty.status, 1);
        assert_eq!(
            empty.reason_code,
            rustaxa_consensus::dag::DAG_PROPOSER_REASON_TRANSACTION_POOL_EMPTY
        );

        insert_test_queue_transaction(&service, 0x51);
        let sidecar_rlp =
            signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x52; 32]).unwrap());
        service
            .transaction()
            .sidecar
            .insert_non_finalized(keccak256(&sidecar_rlp), sidecar_rlp)
            .unwrap();

        let mut limited_input = proposer_begin_input();
        limited_input.max_non_finalized_transactions = 0;
        let limited_id =
            service_dag_manager_runtime_begin_proposer_session(&service, limited_input).unwrap();
        let limited = report_proposer_external_facts(&service, limited_id);
        assert_eq!(limited.status, 1);
        assert_eq!(
            limited.reason_code,
            rustaxa_consensus::dag::DAG_PROPOSER_REASON_NON_FINALIZED_TRANSACTION_LIMIT
        );

        let ready_id =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();
        let ready = report_proposer_external_facts(&service, ready_id);
        assert_eq!(ready.status, 0);
        assert_eq!(ready.action, 1);
        assert!(service
            .dag_transaction_service_proposer_pack_abort(ready_id)
            .unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_commits_transactions_block_graph_and_restart_state() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add");
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
        let key = SigningKey::from_slice(&[0x61; 32]).unwrap();
        let transaction_rlp = signed_legacy_transaction_rlp(&key);
        let transaction = LegacyTransactionEnvelope::decode(&transaction_rlp).unwrap();
        insert_test_queue_transaction(&service, 0x61);
        let block_rlp = composed_add_block_rlp([1; 32], 1, &[transaction.hash.0]);
        let block_hash = keccak256(&block_rlp);
        let preparation = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                block_rlp.clone(),
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: transaction.hash.0,
                    trx_rlp: transaction_rlp.clone(),
                }],
            ))
            .unwrap();
        assert!(preparation.accepted);
        assert_eq!(preparation.account_requests.len(), 1);
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 1);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);

        let missing_fact = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .err()
            .expect("missing nonce fact must fail before the shared commit");
        assert!(missing_fact
            .to_string()
            .contains("DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_COUNT_MISMATCH"));
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 1);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);
        assert!(
            !service
                .dag_manager_runtime_load_block(&block_hash.0)
                .unwrap()
                .found
        );
        assert!(storage
            .0
            .transaction()
            .rlp(transaction.hash)
            .unwrap()
            .is_none());

        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                    input_index: 0,
                    account_nonce: U256::zero().to_big_endian(),
                }],
            })
            .unwrap();
        assert!(report.accepted);
        assert!(report.emit_verified);
        assert!(report.gossip);
        assert_eq!(report.queue_erased.len(), 1);
        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 1);
        assert!(service
            .transaction()
            .sidecar
            .contains_non_finalized(transaction.hash));
        assert_eq!(
            storage.0.transaction().rlp(transaction.hash).unwrap(),
            Some(transaction_rlp)
        );
        assert_eq!(
            service
                .dag_manager_runtime_load_block(&block_hash.0)
                .unwrap()
                .block_rlp,
            block_rlp
        );

        drop(service);
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
        assert_eq!(restored.dag_manager_runtime_vertex_count().unwrap(), 2);
        assert_eq!(restored.transaction_manager_runtime_transaction_count(), 1);

        let duplicate = restored
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                block_rlp,
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: [0; 32],
                    trx_rlp: Vec::new(),
                }],
            ))
            .unwrap();
        assert!(duplicate.accepted);
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.cursor_id, 0);

        drop(restored);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_terminal_and_save_false_paths_do_not_persist_transactions() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_terminal");
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
        let missing_rlp = composed_add_block_rlp([0x77; 32], 1, &[[0xAA; 32]]);
        let missing = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                missing_rlp,
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: [0; 32],
                    trx_rlp: Vec::new(),
                }],
            ))
            .unwrap();
        assert!(!missing.accepted);
        assert!(!missing.missing_references.is_empty());
        assert_eq!(missing.cursor_id, 0);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);

        let no_save_rlp = composed_add_block_rlp([1; 32], 1, &[[0xBB; 32]]);
        let no_save_hash = keccak256(&no_save_rlp);
        let object_hash = [0xBC; 32];
        let mut no_save_input = add_block_prepare_input(no_save_rlp, false, Vec::new());
        no_save_input.expected_block_hash = object_hash;
        no_save_input.validate_block_hash = false;
        let no_save = service
            .dag_transaction_service_prepare_add_block(no_save_input)
            .unwrap();
        assert!(no_save.accepted);
        assert!(no_save.account_requests.is_empty());
        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: no_save.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .unwrap();
        assert!(!report.emit_verified);
        assert!(!report.gossip);
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);
        assert!(service
            .dag_manager_runtime_is_block_known(&object_hash)
            .unwrap());
        assert!(!service
            .dag_manager_runtime_is_block_known(&no_save_hash.0)
            .unwrap());
        assert!(
            !service
                .dag_manager_runtime_load_block(&no_save_hash.0)
                .unwrap()
                .found
        );
        assert_eq!(report.counters.dag_blocks, 0);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_object_compatibility_persists_only_supplied_transactions() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_object");
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
        let block_rlp = composed_add_block_rlp([1; 32], 1, &[[0xBD; 32]]);
        let canonical_hash = keccak256(&block_rlp);
        let object_hash = [0xBE; 32];
        let mut input = add_block_prepare_input(block_rlp.clone(), true, Vec::new());
        input.expected_block_hash = object_hash;
        input.validate_block_hash = false;

        let preparation = service
            .dag_transaction_service_prepare_add_block(input)
            .unwrap();
        assert!(preparation.accepted);
        assert!(preparation.account_requests.is_empty());
        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .unwrap();

        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);
        assert_eq!(
            service
                .dag_manager_runtime_load_block(&object_hash)
                .unwrap()
                .block_rlp,
            block_rlp
        );
        assert!(
            !service
                .dag_manager_runtime_load_block(&canonical_hash.0)
                .unwrap()
                .found
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_preserves_finalized_nonce_filtering() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_finalized");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let key = SigningKey::from_slice(&[0x62; 32]).unwrap();
        let transaction_rlp = signed_legacy_transaction_rlp(&key);
        let transaction = LegacyTransactionEnvelope::decode(&transaction_rlp).unwrap();
        let pbft_block = signed_pbft_block(&SigningKey::from_slice(&[0x63; 32]).unwrap(), 1);
        storage
            .0
            .transaction()
            .write_location(transaction.hash, 1, 0, false)
            .unwrap();
        storage
            .0
            .period()
            .write(1, &period_data_rlp(&pbft_block, &transaction_rlp))
            .unwrap();
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
        let block_rlp = composed_add_block_rlp([1; 32], 1, &[transaction.hash.0]);
        let preparation = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                block_rlp,
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: transaction.hash.0,
                    trx_rlp: transaction_rlp,
                }],
            ))
            .unwrap();
        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                    input_index: 0,
                    account_nonce: U256::from(2u64).to_big_endian(),
                }],
            })
            .unwrap();
        assert!(report.accepted);
        assert!(report.queue_erased.is_empty());
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);
        assert!(!service
            .transaction()
            .sidecar
            .contains_non_finalized(transaction.hash));
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_active_cursor_survives_second_terminal_and_malformed_prepares() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_stale");
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
        let first_rlp = composed_add_block_rlp([1; 32], 1, &[]);
        let first = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                first_rlp,
                true,
                Vec::new(),
            ))
            .unwrap();
        let second = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([1; 32], 1, &[[0xDD; 32]]),
                false,
                Vec::new(),
            ))
            .err()
            .expect("a second accepted prepare must not replace the active cursor");
        assert!(second
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"));

        let terminal = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([0x77; 32], 1, &[]),
                true,
                Vec::new(),
            ))
            .err()
            .expect("a terminal second prepare must leave the active cursor intact");
        assert!(terminal
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"));

        let malformed = service
            .dag_transaction_service_prepare_add_block(DagAddBlockPrepareInput {
                expected_block_hash: [0; 32],
                validate_block_hash: true,
                block_rlp: vec![0x80],
                save: true,
                proposed: false,
                transactions: Vec::new(),
            })
            .err()
            .expect("malformed second prepare must leave the active cursor intact");
        assert!(malformed
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"));

        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: first.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .unwrap();
        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);
        let retry = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: first.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .err()
            .expect("a committed cursor must already be gone");
        assert!(retry
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_NOT_STARTED"));

        let malformed_without_active = service
            .dag_transaction_service_prepare_add_block(DagAddBlockPrepareInput {
                expected_block_hash: [0; 32],
                validate_block_hash: true,
                block_rlp: vec![0x80],
                save: true,
                proposed: false,
                transactions: Vec::new(),
            })
            .err()
            .expect("malformed input must be decoded once no cursor is active");
        assert!(malformed_without_active
            .to_string()
            .contains("DAG_ADD_BLOCK_PREPARE_DECODE"));
        let malformed_transaction = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([1; 32], 1, &[[0xCC; 32]]),
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: [0xCC; 32],
                    trx_rlp: vec![0x80],
                }],
            ))
            .err()
            .expect("malformed transaction must reject accepted preparation");
        assert!(malformed_transaction
            .to_string()
            .contains("DAG_ADD_BLOCK_PREPARE_TRANSACTION_DECODE"));
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_abort_is_matching_stale_safe_and_idempotent() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_abort");
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
        let first = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([1; 32], 1, &[]),
                true,
                Vec::new(),
            ))
            .unwrap();
        assert!(!service
            .dag_transaction_service_abort_add_block(first.cursor_id + 1)
            .unwrap());
        assert!(service
            .dag_transaction_service_abort_add_block(first.cursor_id)
            .unwrap());
        assert!(!service
            .dag_transaction_service_abort_add_block(first.cursor_id)
            .unwrap());

        let second = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([1; 32], 1, &[]),
                false,
                Vec::new(),
            ))
            .unwrap();
        assert_ne!(first.cursor_id, second.cursor_id);
        assert!(!service
            .dag_transaction_service_abort_add_block(first.cursor_id)
            .unwrap());
        let stale = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: first.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .err()
            .expect("an old cursor must not consume the active cursor");
        assert!(stale
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_CURSOR_MISMATCH"));
        assert!(service
            .dag_transaction_service_abort_add_block(second.cursor_id)
            .unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_concurrent_prepares_publish_exactly_one_cursor() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_concurrent");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = Arc::new(
            *create_dag_transaction_service_from_storage(
                &storage,
                &[1; 32],
                32,
                100,
                queue_config(),
                gas_config(),
                u64::MAX,
            )
            .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(3));
        let handles = [[0xD1; 32], [0xD2; 32]].map(|transaction_hash| {
            let service = Arc::clone(&service);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                service
                    .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                        composed_add_block_rlp([1; 32], 1, &[transaction_hash]),
                        false,
                        Vec::new(),
                    ))
                    .map_err(|error| error.to_string())
            })
        });
        barrier.wait();
        let results = handles.map(|handle| handle.join().unwrap());
        let winner = results
            .iter()
            .find_map(|result| result.as_ref().ok())
            .expect("one prepare must acquire the cursor");
        let loser = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .expect("one prepare must observe the active cursor");
        assert!(loser.contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"));
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);

        service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: winner.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .unwrap();
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_prepare_does_not_advance_and_completion_succeeds() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_all_supplied");
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
        let supplied = [7; 32];
        begin_verify_block_session(&service, &[supplied, supplied], &[supplied]);

        let preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        assert!(preparation.transactions.is_empty());
        let unadvanced = service_dag_manager_runtime_verify_block_session_next(&service).unwrap();
        assert_eq!(unadvanced.action, 1);
        let completed = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            completed.action,
            DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_prepare_returns_runtime_views_in_canonical_query_order() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_mixed");
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
        let supplied = [8; 32];
        let key = SigningKey::from_slice(&[0x49; 32]).unwrap();
        let queued_rlp = signed_legacy_transaction_rlp(&key);
        let queued = LegacyTransactionEnvelope::decode(&queued_rlp).unwrap();
        service
            .transaction()
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: queued.hash.0,
                sender: address_from_signing_key(&key),
                nonce: queued.nonce.to_big_endian(),
                gas_price: queued.gas_price.to_big_endian(),
                gas: queued.gas,
                data_size: queued.data.len(),
                tx_rlp: queued_rlp.clone(),
                proposable: true,
                last_block_number: 0,
            })
            .unwrap();
        let sidecar_rlp =
            signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x4C; 32]).unwrap());
        let sidecar_hash = keccak256(&sidecar_rlp);
        service
            .transaction()
            .sidecar
            .insert_non_finalized(sidecar_hash, sidecar_rlp.clone())
            .unwrap();
        begin_verify_block_session(
            &service,
            &[supplied, queued.hash.0, sidecar_hash.0, queued.hash.0],
            &[supplied],
        );

        let preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        assert_eq!(preparation.transactions.len(), 2);
        assert_eq!(preparation.transactions[0].input_index, 0);
        assert_eq!(preparation.transactions[0].hash, queued.hash.0);
        assert_eq!(preparation.transactions[0].tx_rlp, queued_rlp);
        assert_eq!(preparation.transactions[1].input_index, 1);
        assert_eq!(preparation.transactions[1].hash, sidecar_hash.0);
        assert_eq!(preparation.transactions[1].tx_rlp, sidecar_rlp);
        let completed = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            completed.action,
            DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_resolution_rejects_missing_transactions() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_missing");
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
        begin_verify_block_session(&service, &[[10; 32]], &[]);

        let preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        assert_eq!(preparation.transactions.len(), 1);
        assert!(!preparation.transactions[0].found);
        let completed = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .unwrap();
        assert!(completed.complete);
        assert_eq!(
            completed.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_MISSING_TRANSACTION
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_completion_requires_nonce_facts_and_rejects_old_finalized_transactions() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_old_finalized");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let key = SigningKey::from_slice(&[0x4A; 32]).unwrap();
        let transaction_rlp = signed_legacy_transaction_rlp(&key);
        let transaction_hash = keccak256(&transaction_rlp);
        let pbft_block = signed_pbft_block(&SigningKey::from_slice(&[0x4B; 32]).unwrap(), 1);
        storage
            .0
            .transaction()
            .write_location(transaction_hash, 1, 0, false)
            .unwrap();
        storage
            .0
            .period()
            .write(1, &period_data_rlp(&pbft_block, &transaction_rlp))
            .unwrap();
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
        begin_verify_block_session(&service, &[transaction_hash.0], &[]);

        let missing_fact_preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        assert!(missing_fact_preparation.transactions[0].found);
        let missing_fact = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: missing_fact_preparation.cursor_id,
                proposal_period: missing_fact_preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .err()
        .expect("missing finalized sender fact must reject completion");
        assert!(missing_fact
            .to_string()
            .contains("TM_PROPOSAL_FINALIZED_ACCOUNT_NONCE_FACT_MISSING"));
        assert_eq!(
            service_dag_manager_runtime_verify_block_session_next(&service)
                .unwrap()
                .action,
            1
        );

        begin_verify_block_session(&service, &[transaction_hash.0], &[]);
        let old_preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        let old = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: old_preparation.cursor_id,
                proposal_period: old_preparation.proposal_period,
                account_nonce_facts: vec![TransactionQueueAccountNonceFact {
                    sender: address_from_signing_key(&key),
                    account_found: true,
                    account_nonce: U256::from(2u64).to_big_endian(),
                }],
            },
        )
        .unwrap();
        assert!(old.complete);
        assert_eq!(
            old.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_MISSING_TRANSACTION
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_completion_rejects_stale_period_and_action_misuse_without_advancing() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_misuse");
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

        let not_started =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .err()
                .expect("missing session must reject preparation");
        assert!(not_started
            .to_string()
            .contains("DAG_VERIFY_SESSION_NOT_STARTED"));

        begin_verify_block_session(&service, &[[12; 32]], &[[12; 32]]);
        let stale = service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
            .unwrap();
        begin_verify_block_session(&service, &[[11; 32]], &[[11; 32]]);
        let current =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        let stale_error = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: stale.cursor_id,
                proposal_period: stale.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .err()
        .expect("stale cursor must reject completion");
        assert!(stale_error
            .to_string()
            .contains("DAG_VERIFY_SESSION_TRANSACTION_CURSOR_MISMATCH"));
        assert_eq!(
            service_dag_manager_runtime_verify_block_session_next(&service)
                .unwrap()
                .action,
            1
        );

        let period_error = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: current.cursor_id,
                proposal_period: current.proposal_period + 1,
                account_nonce_facts: Vec::new(),
            },
        )
        .err()
        .expect("wrong proposal period must reject completion");
        assert!(period_error
            .to_string()
            .contains("DAG_VERIFY_SESSION_TRANSACTION_PERIOD_MISMATCH"));

        let completed = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: current.cursor_id,
                proposal_period: current.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            completed.action,
            DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
        );
        let wrong_action =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .err()
                .expect("wrong action must reject preparation");
        assert!(wrong_action
            .to_string()
            .contains("DAG_VERIFY_SESSION_UNEXPECTED_TRANSACTION_COMPLETION"));

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
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
            .dag()
            .unwrap()
            .as_ref()
            .unwrap()
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
                full.dag()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .dag_manager_runtime_restore_from_storage()
                    .unwrap();
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
        let queued = insert_test_queue_transaction(&service, 0x53);

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
        assert!(service.transaction().queue.erase(queued.hash));
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

    #[test]
    fn finalized_order_removes_private_sidecars_only_after_dag_commit() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_finalization_cleanup");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            1,
            100,
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let tx_hash = H256::from([7u8; 32]);
        {
            let mut dag_guard = service.dag().unwrap();
            let dag = dag_guard.as_mut().unwrap();
            dag.dag_manager_runtime_add_block(DagManagerBlock {
                hash: [3u8; 32],
                pivot: [1u8; 32],
                tips: Vec::new(),
                level: 3,
                difficulty: 90,
            })
            .unwrap();
            dag.dag_manager_runtime_save_block(&[3u8; 32], 3, 0, dag_block_rlp(3, &[[7u8; 32]]))
                .unwrap();
            dag.dag_manager_runtime_save_block(&[8u8; 32], 5, 0, dag_block_rlp(5, &[]))
                .unwrap();
        }
        storage.0.transaction().write(tx_hash, &[0xA7]).unwrap();
        service
            .transaction()
            .sidecar
            .insert_non_finalized(tx_hash, vec![0xA7])
            .unwrap();

        let failed = service.dag_manager_runtime_apply_finalized_order(
            [8u8; 32],
            2,
            vec![DagHash { hash: [8u8; 32] }],
        );
        assert!(failed.is_err());
        assert!(service
            .transaction()
            .sidecar
            .contains_non_finalized(tx_hash));

        let applied = service
            .dag_manager_runtime_apply_finalized_order(
                [8u8; 32],
                1,
                vec![DagHash { hash: [8u8; 32] }],
            )
            .expect("DAG commit should drive sidecar cleanup");
        assert_eq!(applied.finalized_count, 1);
        assert_eq!(applied.expired_hashes.len(), 1);
        assert!(!service
            .transaction()
            .sidecar
            .contains_non_finalized(tx_hash));
        assert!(create_transaction_storage_queries(&storage)
            .get_transaction(&tx_hash.0)
            .unwrap()
            .is_empty());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }
}
