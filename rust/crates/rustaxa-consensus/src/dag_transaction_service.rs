//! Native application ownership for the DAG, sortition, and transaction services.
//!
//! This module is the single supported Rust production composition for the DAG
//! cluster. It restores all sibling services from one storage owner before
//! publication and defines native access to the canonical
//! DAG-then-sortition-then-transaction lock domains. Bridge code may temporarily borrow the typed guards exposed here
//! while FFI-shaped task methods move into native owners; no guard may cross CXX
//! or an external executor boundary.

use crate::dag::{
    DagPersistenceCounters, dag_block_transaction_hashes, dag_manager_block_from_rlp,
};
use crate::dag_service::{
    DagAddBlockPreparedTransaction, DagAddBlockSession, DagAddBlockStoredPlan, DagService,
    DagServiceConfig, DagServiceGuard,
};
use crate::sortition::{SortitionConfig, SortitionService, SortitionServiceGuard};
use crate::transaction_service::{
    DagTransactionSaveInput, TransactionService, TransactionServiceConfig, TransactionServiceGuard,
    append_prepared_dag_transactions, prepare_dag_transaction_publication,
    prepare_dag_transactions, publish_dag_transactions,
    remove_non_finalized_sidecars_after_dag_commit,
};
use anyhow::{Context, Result, ensure};
use ethereum_types::{H160, H256, U256};
use rustaxa_storage::{Storage, StorageWriteBatch};
use rustaxa_types::LegacyTransactionEnvelope;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Immutable configuration for the native DAG application root.
///
/// Construction restores transaction state first, DAG state second, and
/// sortition state last. This preserves the production startup error precedence
/// while publishing no partially constructed root.
#[derive(Clone, Debug)]
pub struct DagTransactionServiceConfig {
    /// Native transaction runtime configuration.
    pub transaction: TransactionServiceConfig,
    /// Native DAG runtime configuration.
    pub dag: DagServiceConfig,
    /// Native sortition runtime configuration.
    pub sortition: SortitionConfig,
}

/// Native application root for DAG, sortition, and transaction consensus state.
///
/// Every sibling receives the same `Arc<Storage>`. The root owns construction,
/// restoration, lifetime, and cross-domain lock domains. Its lock accessors are
/// a temporary CRW-12 Rust-only escape hatch for bridge task adapters and never
/// expose raw mutexes or guards through CXX.
pub struct DagTransactionService {
    transaction: TransactionService,
    dag: DagService,
    sortition: SortitionService,
}

/// Canonical transaction payload supplied while preparing a DAG block.
#[derive(Clone, Debug)]
pub struct DagAddBlockTransactionPayload {
    /// Expected signed transaction hash.
    pub hash: H256,
    /// Canonical signed transaction RLP.
    pub transaction_rlp: Vec<u8>,
}

/// Native request for one DAG add-block preparation.
#[derive(Debug)]
pub struct DagAddBlockPrepareRequest {
    /// Expected DAG block identity.
    pub expected_hash: H256,
    /// Canonical signed DAG block RLP.
    pub block_rlp: Vec<u8>,
    /// Whether the canonical RLP hash must equal `expected_hash`.
    pub validate_hash: bool,
    /// Whether the accepted block and transactions should be persisted.
    pub save: bool,
    /// Whether the local node proposed the block.
    pub proposed: bool,
    /// Ordered transaction payloads materialized by the retained C++ boundary.
    pub transactions: Vec<DagAddBlockTransactionPayload>,
}

/// Latest-account query requested for one inspected block transaction.
#[derive(Clone, Copy, Debug)]
pub struct DagAddBlockAccountRequest {
    /// Stable index into the request transaction list.
    pub input_index: u64,
    /// Recovered transaction sender.
    pub sender: H160,
}

/// Native non-mutating or cursor-opening add-block preparation.
#[derive(Debug)]
pub struct DagAddBlockPreparation {
    /// Nonzero cursor for an accepted nonterminal transition.
    pub cursor_id: u64,
    /// Candidate block level.
    pub block_level: u64,
    /// Whether the block is accepted.
    pub accepted: bool,
    /// Whether this is an idempotent persisted duplicate.
    pub duplicate: bool,
    /// Whether the block is below the live expiry level.
    pub expired: bool,
    /// Missing pivot/tip identities in deterministic order.
    pub missing_references: Vec<H256>,
    /// External latest-account queries required before completion.
    pub account_requests: Vec<DagAddBlockAccountRequest>,
}

/// Latest sender nonce for one prepared transaction.
#[derive(Clone, Copy, Debug)]
pub struct DagAddBlockAccountNonceFact {
    /// Stable transaction input index.
    pub input_index: u64,
    /// Latest FinalChain account nonce.
    pub account_nonce: U256,
}

/// Cursor-bound completion facts for one prepared add-block transition.
#[derive(Debug)]
pub struct DagAddBlockCompletion {
    /// Prepared cursor identity.
    pub cursor_id: u64,
    /// Latest account nonces for all retained transactions.
    pub account_nonce_facts: Vec<DagAddBlockAccountNonceFact>,
}

/// Durable add-block result and leaf-adapter effects.
#[derive(Debug)]
pub struct DagAddBlockCommitReport {
    /// Always true for a completed accepted cursor.
    pub accepted: bool,
    /// Whether the C++ event adapter should emit verification.
    pub emit_verified: bool,
    /// Whether the transport adapter should gossip the block.
    pub gossip: bool,
    /// Whether the local node proposed the block.
    pub proposed: bool,
    /// Transaction hashes erased from the native pending queue.
    pub queue_erased: Vec<H256>,
    /// Persisted DAG block and edge counters after the transition.
    pub counters: DagPersistenceCounters,
}

/// Native finalization result returned to the retained event adapter.
#[derive(Debug)]
pub struct DagFinalizationReport {
    /// Number of hashes finalized by the transition.
    pub finalized_count: usize,
    /// Expired DAG identities for temporary external seen-block cleanup.
    pub expired_hashes: Vec<H256>,
}

impl DagTransactionService {
    /// Restores all sibling services and publishes one coherent application root.
    ///
    /// Transaction restoration runs before DAG restoration, which runs before
    /// sortition restoration. Any validation, decoding, or storage error returns
    /// without publishing the root. The shared storage owner is cloned only into
    /// the native sibling services.
    pub fn restore(storage: Arc<Storage>, config: DagTransactionServiceConfig) -> Result<Self> {
        let transaction = TransactionService::restore(storage.clone(), config.transaction)?;
        let dag = DagService::restore(storage.clone(), config.dag)?;
        let sortition = SortitionService::restore(config.sortition, storage)?;
        Ok(Self {
            transaction,
            dag,
            sortition,
        })
    }

    /// Locks the transaction sibling for one short-lived native task.
    ///
    /// Coupled tasks must use [`Self::lock_dag_and_transaction`] or explicitly
    /// acquire DAG and sortition first. The guard must not cross an external
    /// executor, callback, thread handoff, asynchronous boundary, or CXX return.
    #[doc(hidden)]
    pub fn lock_transaction(&self) -> Result<TransactionServiceGuard<'_>> {
        self.transaction.lock()
    }

    /// Locks the DAG sibling for one short-lived native task.
    ///
    /// This is the first lock in every coupled operation. The guard must not
    /// cross an external executor, callback, sleep, thread handoff, asynchronous
    /// boundary, or CXX return.
    #[doc(hidden)]
    pub fn lock_dag(&self) -> Result<DagServiceGuard<'_>> {
        self.dag.lock()
    }

    /// Locks the sortition sibling for one short-lived native task.
    ///
    /// Coupled tasks acquire this only after DAG and before transaction. A
    /// standalone sortition task may acquire it directly.
    #[doc(hidden)]
    pub fn lock_sortition(&self) -> Result<SortitionServiceGuard<'_>> {
        self.sortition.lock()
    }

    /// Locks DAG and transaction state in their canonical relative order.
    ///
    /// Sortition is not part of this operation. A three-domain operation must
    /// acquire sortition between these two locks. If transaction locking fails,
    /// the DAG guard is dropped with the returned error.
    #[doc(hidden)]
    pub fn lock_dag_and_transaction(
        &self,
    ) -> Result<(DagServiceGuard<'_>, TransactionServiceGuard<'_>)> {
        let dag = self.dag.lock()?;
        let transaction = self.transaction.lock()?;
        Ok((dag, transaction))
    }

    /// Prepares one add-block transition without mutating live or durable state.
    ///
    /// The method acquires DAG then transaction, decodes and validates canonical
    /// payloads, and publishes at most one pending native cursor. Terminal
    /// duplicate, expired, or missing-reference outcomes return cursor zero.
    /// No external account lookup is performed while locks are held.
    pub fn prepare_add_block(
        &self,
        request: DagAddBlockPrepareRequest,
    ) -> Result<DagAddBlockPreparation> {
        let (mut dag, _transaction) = self.lock_dag_and_transaction()?;
        ensure!(
            dag.pending_add_block.is_none(),
            "DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"
        );
        let mut block = dag_manager_block_from_rlp(&request.block_rlp)
            .context("DAG_ADD_BLOCK_PREPARE_DECODE")?;
        if request.validate_hash {
            ensure!(
                block.hash == request.expected_hash,
                "DAG_ADD_BLOCK_PREPARE_HASH_MISMATCH"
            );
        } else {
            block.hash = request.expected_hash;
        }
        let plan = dag.plan_add_block(&block, request.save, request.proposed)?;
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
            let expected_hashes = if request.validate_hash {
                let block_hashes = dag_block_transaction_hashes(&request.block_rlp)
                    .context("DAG_ADD_BLOCK_PREPARE_TRANSACTION_HASHES")?;
                ensure!(
                    block_hashes.len() == request.transactions.len(),
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_COUNT_MISMATCH"
                );
                block_hashes
            } else {
                request
                    .transactions
                    .iter()
                    .map(|payload| payload.hash)
                    .collect()
            };
            for (input_index, (expected_hash, payload)) in expected_hashes
                .into_iter()
                .zip(request.transactions)
                .enumerate()
            {
                ensure!(
                    expected_hash == payload.hash,
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_ORDER_MISMATCH"
                );
                let envelope = LegacyTransactionEnvelope::decode(&payload.transaction_rlp)
                    .context("DAG_ADD_BLOCK_PREPARE_TRANSACTION_DECODE")?;
                ensure!(
                    envelope.hash == payload.hash,
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_HASH_MISMATCH"
                );
                let sender = envelope
                    .sender
                    .context("DAG_ADD_BLOCK_PREPARE_TRANSACTION_SENDER_MISSING")?;
                transactions.push(DagAddBlockPreparedTransaction {
                    input_index: input_index as u64,
                    hash: expected_hash,
                    trx_rlp: payload.transaction_rlp,
                    transaction_nonce: envelope.nonce.to_big_endian(),
                });
                account_requests.push(DagAddBlockAccountRequest {
                    input_index: input_index as u64,
                    sender,
                });
            }
        }

        let cursor_id = dag.next_add_block_session_id;
        dag.next_add_block_session_id = cursor_id.wrapping_add(1).max(1);
        let block_level = block.level;
        dag.pending_add_block = Some(DagAddBlockSession {
            cursor_id,
            block,
            block_rlp: request.block_rlp,
            save: request.save,
            proposed: request.proposed,
            transactions,
            plan: stored_add_block_plan(&plan),
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

    /// Completes one prepared add-block through a single shared durable batch.
    ///
    /// DAG and transaction next states are fully prevalidated before persistence.
    /// The pending cursor is retained when the batch commit fails. Neither live
    /// state is published before commit; after success both are published
    /// infallibly while DAG and transaction locks remain held.
    pub fn complete_add_block(
        &self,
        completion: DagAddBlockCompletion,
    ) -> Result<DagAddBlockCommitReport> {
        self.complete_add_block_with_commit(completion, |storage, batch| {
            storage.commit_write_batch_with_sync(batch, false)
        })
    }

    fn complete_add_block_with_commit(
        &self,
        completion: DagAddBlockCompletion,
        commit: impl FnOnce(&Storage, StorageWriteBatch) -> Result<()>,
    ) -> Result<DagAddBlockCommitReport> {
        let (mut dag, mut transaction) = self.lock_dag_and_transaction()?;
        let session = dag
            .pending_add_block
            .as_ref()
            .context("DAG_ADD_BLOCK_SESSION_NOT_STARTED")?
            .clone();
        ensure!(
            session.cursor_id == completion.cursor_id,
            "DAG_ADD_BLOCK_SESSION_CURSOR_MISMATCH"
        );
        let current_plan = dag.plan_add_block(&session.block, session.save, session.proposed)?;
        ensure!(
            stored_add_block_plan(&current_plan) == session.plan
                && current_plan.accepted
                && !current_plan.duplicate
                && !current_plan.expired,
            "DAG_ADD_BLOCK_SESSION_STALE_PLAN"
        );

        let mut nonce_facts = BTreeMap::new();
        for fact in completion.account_nonce_facts {
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
            .map(|input| {
                Ok(DagTransactionSaveInput {
                    input_index: input.input_index,
                    hash: input.hash,
                    transaction_rlp: input.trx_rlp.clone(),
                    transaction_nonce: U256::from_big_endian(&input.transaction_nonce),
                    sender_account_nonce: *nonce_facts
                        .get(&input.input_index)
                        .context("DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_MISSING")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let prepared_transactions = session
            .plan
            .persist_transactions
            .then(|| prepare_dag_transactions(&transaction, transaction_facts))
            .transpose()?;
        let transaction_publication = prepared_transactions
            .as_ref()
            .map(|prepared| prepare_dag_transaction_publication(&transaction, prepared))
            .transpose()?;
        let mut next_dag_state = dag.state.clone();
        if session.plan.add_to_graph {
            next_dag_state
                .add_block(session.block.clone())
                .context("DAG_ADD_BLOCK_GRAPH_PREVALIDATE")?;
        }

        ensure!(
            Arc::ptr_eq(&dag.storage, &transaction.storage),
            "DAG_ADD_BLOCK_STORAGE_OWNER_MISMATCH"
        );
        let counters;
        let mut pending_batch = None;
        if session.plan.persist_block {
            let mut batch = dag.storage.create_write_batch();
            if let Some(prepared) = prepared_transactions.as_ref() {
                append_prepared_dag_transactions(dag.storage.as_ref(), &mut batch, prepared)?;
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
            counters = dag.persistence_counters()?;
        }

        let removed_session = dag
            .pending_add_block
            .take()
            .context("DAG_ADD_BLOCK_SESSION_DISAPPEARED_BEFORE_COMMIT")?;
        let commit_result = pending_batch
            .map(|batch| commit(dag.storage.as_ref(), batch))
            .transpose();
        if let Err(error) = commit_result {
            dag.pending_add_block = Some(removed_session);
            return Err(error).context("DAG_ADD_BLOCK_BATCH_COMMIT");
        }
        dag.state = next_dag_state;
        let queue_erased = transaction_publication
            .map(|publication| publish_dag_transactions(&mut transaction, publication))
            .into_iter()
            .flat_map(|outcome| outcome.accepted)
            .filter(|accepted| accepted.erased_from_queue)
            .map(|accepted| accepted.hash)
            .collect();
        Ok(DagAddBlockCommitReport {
            accepted: true,
            emit_verified: session.plan.emit_verified,
            gossip: session.plan.gossip,
            proposed: session.plan.proposed,
            queue_erased,
            counters,
        })
    }

    /// Idempotently aborts only the matching pending add-block cursor.
    pub fn abort_add_block(&self, cursor_id: u64) -> Result<bool> {
        let mut dag = self.lock_dag()?;
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

    /// Commits DAG finalization and clears sibling transaction sidecars.
    ///
    /// DAG cleanup owns the complete durable batch. The transaction sibling is
    /// mutated only after that commit succeeds, while both locks remain held in
    /// DAG-then-transaction order. Only retained external event facts escape.
    pub fn apply_finalized_order(
        &self,
        new_anchor: H256,
        new_period: u64,
        finalized_order: Vec<H256>,
    ) -> Result<DagFinalizationReport> {
        let (mut dag, mut transaction) = self.lock_dag_and_transaction()?;
        let committed = dag.apply_finalized_order(new_anchor, new_period, finalized_order)?;
        remove_non_finalized_sidecars_after_dag_commit(
            &mut transaction,
            &committed.remove_transaction_hashes,
        );
        Ok(DagFinalizationReport {
            finalized_count: committed.finalized_count,
            expired_hashes: committed.expired_hashes,
        })
    }
}

fn stored_add_block_plan(plan: &crate::dag::DagAddBlockEffectPlan) -> DagAddBlockStoredPlan {
    DagAddBlockStoredPlan {
        accepted: plan.accepted,
        persist_transactions: plan.persist_transactions,
        persist_block: plan.persist_block,
        add_to_graph: plan.add_to_graph,
        emit_verified: plan.emit_verified,
        gossip: plan.gossip,
        proposed: plan.proposed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::{DagManagerBlock, ensure_proposal_period_mapping, save_dag_block_to_storage};
    use crate::gas_pricer::GasPricerConfig;
    use crate::sortition::{HUNDRED_PERCENT, SortitionParams, VdfParams, VrfParams};
    use crate::transaction_queue::TransactionQueueEntry;
    use anyhow::{Result, anyhow};
    use ethereum_types::{H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::Config;
    use std::path::PathBuf;
    use std::sync::Barrier;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tiny_keccak::{Hasher, Keccak};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn service_config() -> DagTransactionServiceConfig {
        DagTransactionServiceConfig {
            transaction: TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: U256::one(),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
            dag: DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
            sortition: SortitionConfig {
                params: SortitionParams {
                    vrf: VrfParams {
                        threshold_upper: 0x100,
                    },
                    vdf: VdfParams {
                        difficulty_min: 1,
                        difficulty_max: 10,
                        difficulty_stale: 5,
                        lambda_bound: 100,
                    },
                },
                changes_count_for_average: 8,
                dag_efficiency_targets: (HUNDRED_PERCENT / 2, HUNDRED_PERCENT),
                changing_interval: 10,
                computation_interval: 5,
            },
        }
    }

    fn keccak256(bytes: &[u8]) -> H256 {
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(bytes);
        hasher.finalize(&mut output);
        H256(output)
    }

    fn signed_legacy_transaction_rlp(signing_key: &SigningKey) -> Vec<u8> {
        let chain_id = 2999_u64;
        let mut unsigned = RlpStream::new_list(9);
        unsigned.append(&U256::from(1));
        unsigned.append(&U256::from(2));
        unsigned.append(&21_000_u64);
        unsigned.append(&H160::repeat_byte(0x44));
        unsigned.append(&U256::from(3));
        unsigned.append(&Vec::<u8>::new());
        unsigned.append(&U256::from(chain_id));
        unsigned.append(&U256::zero());
        unsigned.append(&U256::zero());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(keccak256(&unsigned.out()).as_bytes())
            .expect("test transaction signing must succeed");
        let signature = signature.to_bytes();
        let mut signed = RlpStream::new_list(9);
        signed.append(&U256::from(1));
        signed.append(&U256::from(2));
        signed.append(&21_000_u64);
        signed.append(&H160::repeat_byte(0x44));
        signed.append(&U256::from(3));
        signed.append(&Vec::<u8>::new());
        signed.append(&U256::from(
            chain_id * 2 + 35 + u64::from(recovery_id.to_byte()),
        ));
        signed.append(&U256::from_big_endian(&signature[..32]));
        signed.append(&U256::from_big_endian(&signature[32..]));
        signed.out().to_vec()
    }

    fn composed_add_block_rlp(pivot: H256, level: u64, transactions: &[H256]) -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11_u8; 80]);
        vdf.append(&vec![0x22_u8]);
        vdf.append(&vec![0x33_u8]);
        vdf.append(&1_u16);
        let mut block = RlpStream::new_list(8);
        block.append(&pivot);
        block.append(&level);
        block.append(&0_u64);
        block.append(&vdf.out().to_vec());
        block.begin_list(0);
        block.begin_list(transactions.len());
        for hash in transactions {
            block.append(hash);
        }
        block.append(&&[0_u8; 65][..]);
        block.append(&0_u64);
        block.out().to_vec()
    }

    fn add_block_request(block_rlp: Vec<u8>, save: bool) -> DagAddBlockPrepareRequest {
        DagAddBlockPrepareRequest {
            expected_hash: keccak256(&block_rlp),
            block_rlp,
            validate_hash: true,
            save,
            proposed: false,
            transactions: Vec::new(),
        }
    }

    #[test]
    fn restore_publishes_all_siblings_with_one_storage_owner() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_root");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;

        let (dag, transaction) = root.lock_dag_and_transaction()?;
        assert!(Arc::ptr_eq(&dag.storage, &storage));
        assert!(Arc::ptr_eq(&transaction.storage, &storage));
        assert_eq!(dag.state.vertex_count(), 1);
        assert_eq!(transaction.sidecar.transaction_count(), 0);
        drop(transaction);
        drop(dag);
        assert_eq!(
            root.lock_sortition()?.current_params().vrf.threshold_upper,
            0x100
        );

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_is_restart_safe_and_keeps_initial_mapping_idempotent() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_restart");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);

        for _ in 0..2 {
            let root = DagTransactionService::restore(storage.clone(), service_config())?;
            assert_eq!(root.lock_dag()?.state.vertex_count(), 1);
            assert!(!ensure_proposal_period_mapping(storage.as_ref(), 100, 0)?);
            drop(root);
        }

        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_preserves_transaction_then_dag_then_sortition_error_precedence() -> Result<()> {
        let transaction_path =
            unique_temp_dir("rustaxa_consensus_dag_transaction_order_transaction");
        let transaction_storage = Arc::new(Storage::new(Config::new(transaction_path.clone()))?);
        let mut invalid_transaction = service_config();
        invalid_transaction.transaction.gas_pricer_config.percentile = 101;
        invalid_transaction.dag.genesis_hash = H256::zero();
        invalid_transaction.sortition.computation_interval = 11;
        let transaction_error =
            DagTransactionService::restore(transaction_storage.clone(), invalid_transaction)
                .err()
                .expect("transaction configuration must fail first");
        assert!(transaction_error.to_string().contains("percentile"));
        drop(transaction_storage);
        std::fs::remove_dir_all(transaction_path)?;

        let dag_path = unique_temp_dir("rustaxa_consensus_dag_transaction_order_dag");
        let dag_storage = Arc::new(Storage::new(Config::new(dag_path.clone()))?);
        let mut invalid_dag = service_config();
        invalid_dag.dag.genesis_hash = H256::zero();
        invalid_dag.sortition.computation_interval = 11;
        let dag_error = DagTransactionService::restore(dag_storage.clone(), invalid_dag)
            .err()
            .expect("DAG configuration must fail before sortition");
        assert!(dag_error.to_string().contains("nonzero genesis"));
        drop(dag_storage);
        std::fs::remove_dir_all(dag_path)?;

        let sortition_path = unique_temp_dir("rustaxa_consensus_dag_transaction_order_sortition");
        let sortition_storage = Arc::new(Storage::new(Config::new(sortition_path.clone()))?);
        let mut invalid_sortition = service_config();
        invalid_sortition.sortition.computation_interval = 11;
        let sortition_error =
            DagTransactionService::restore(sortition_storage.clone(), invalid_sortition)
                .err()
                .expect("sortition configuration must fail last");
        assert!(
            sortition_error
                .to_string()
                .contains("SORTITION_STORAGE_CREATE_RUNTIME")
        );
        drop(sortition_storage);
        std::fs::remove_dir_all(sortition_path)?;
        Ok(())
    }

    #[test]
    fn add_block_commits_once_and_restores_native_state() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_add_block");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let block_rlp = composed_add_block_rlp(H256::repeat_byte(1), 1, &[]);

        let preparation = root.prepare_add_block(add_block_request(block_rlp.clone(), true))?;
        assert!(preparation.accepted);
        assert_ne!(preparation.cursor_id, 0);
        assert!(preparation.account_requests.is_empty());
        let report = root.complete_add_block(DagAddBlockCompletion {
            cursor_id: preparation.cursor_id,
            account_nonce_facts: Vec::new(),
        })?;
        assert!(report.accepted);
        assert!(report.emit_verified);
        assert!(report.gossip);
        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 2);

        let duplicate = root.prepare_add_block(add_block_request(block_rlp, true))?;
        assert!(duplicate.accepted);
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.cursor_id, 0);
        drop(root);
        drop(storage);

        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let restored = DagTransactionService::restore(storage.clone(), service_config())?;
        assert_eq!(restored.lock_dag()?.state.vertex_count(), 2);
        assert_eq!(restored.lock_dag()?.persistence_counters()?.dag_blocks, 1);
        drop(restored);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_commit_failure_retains_cursor_and_publishes_neither_state() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_commit_failure");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let transaction_rlp = signed_legacy_transaction_rlp(
            &SigningKey::from_slice(&[0x42; 32]).expect("valid test signing key"),
        );
        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)?;
        let sender = envelope.sender.context("test transaction sender")?;
        root.lock_transaction()?.queue.insert(
            TransactionQueueEntry {
                hash: envelope.hash,
                sender,
                nonce: envelope.nonce,
                gas_price: envelope.gas_price,
                gas: envelope.gas,
                data_size: envelope.data.len() as u64,
                rlp: transaction_rlp.clone(),
                last_block_number: 0,
            },
            true,
        )?;
        let block_rlp = composed_add_block_rlp(
            H256::repeat_byte(1),
            1,
            std::slice::from_ref(&envelope.hash),
        );
        let preparation = root.prepare_add_block(DagAddBlockPrepareRequest {
            expected_hash: keccak256(&block_rlp),
            block_rlp,
            validate_hash: true,
            save: true,
            proposed: false,
            transactions: vec![DagAddBlockTransactionPayload {
                hash: envelope.hash,
                transaction_rlp: transaction_rlp.clone(),
            }],
        })?;
        assert_eq!(preparation.account_requests.len(), 1);
        assert_eq!(preparation.account_requests[0].sender, sender);

        let error = root
            .complete_add_block_with_commit(
                DagAddBlockCompletion {
                    cursor_id: preparation.cursor_id,
                    account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                        input_index: 0,
                        account_nonce: U256::zero(),
                    }],
                },
                |_storage, _batch| Err(anyhow!("injected commit failure")),
            )
            .expect_err("the injected durable commit must fail");
        assert!(error.to_string().contains("DAG_ADD_BLOCK_BATCH_COMMIT"));
        {
            let (dag, transaction) = root.lock_dag_and_transaction()?;
            assert_eq!(dag.state.vertex_count(), 1);
            assert_eq!(dag.persistence_counters()?.dag_blocks, 0);
            assert_eq!(
                dag.pending_add_block
                    .as_ref()
                    .map(|session| session.cursor_id),
                Some(preparation.cursor_id)
            );
            assert_eq!(transaction.sidecar.transaction_count(), 0);
            assert!(transaction.queue.contains(envelope.hash));
            assert!(!transaction.sidecar.contains_non_finalized(envelope.hash));
        }
        assert!(storage.transaction().rlp(envelope.hash)?.is_none());

        let report = root.complete_add_block(DagAddBlockCompletion {
            cursor_id: preparation.cursor_id,
            account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                input_index: 0,
                account_nonce: U256::zero(),
            }],
        })?;
        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(report.queue_erased, vec![envelope.hash]);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 2);
        {
            let transaction_state = root.lock_transaction()?;
            assert_eq!(transaction_state.sidecar.transaction_count(), 1);
            assert!(!transaction_state.queue.contains(envelope.hash));
            assert!(
                transaction_state
                    .sidecar
                    .contains_non_finalized(envelope.hash)
            );
        }
        assert_eq!(
            storage.transaction().rlp(envelope.hash)?,
            Some(transaction_rlp)
        );
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_cursor_rejects_overlap_and_abort_is_stale_safe() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_cursor");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let block_rlp = composed_add_block_rlp(H256::repeat_byte(1), 1, &[]);
        let first = root.prepare_add_block(add_block_request(block_rlp.clone(), true))?;

        let overlap = root
            .prepare_add_block(add_block_request(block_rlp.clone(), false))
            .expect_err("a second prepare must not replace the active cursor");
        assert!(
            overlap
                .to_string()
                .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE")
        );
        assert!(!root.abort_add_block(first.cursor_id + 1)?);
        assert!(root.abort_add_block(first.cursor_id)?);
        assert!(!root.abort_add_block(first.cursor_id)?);

        let second = root.prepare_add_block(add_block_request(block_rlp, false))?;
        assert_ne!(second.cursor_id, first.cursor_id);
        let stale = root
            .complete_add_block(DagAddBlockCompletion {
                cursor_id: first.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .expect_err("a stale cursor must not consume the active session");
        assert!(
            stale
                .to_string()
                .contains("DAG_ADD_BLOCK_SESSION_CURSOR_MISMATCH")
        );
        assert!(root.abort_add_block(second.cursor_id)?);
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_terminal_and_save_false_paths_do_not_persist() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_terminal");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;

        let missing = root.prepare_add_block(add_block_request(
            composed_add_block_rlp(H256::repeat_byte(9), 1, &[]),
            true,
        ))?;
        assert!(!missing.accepted);
        assert_eq!(missing.cursor_id, 0);
        assert_eq!(missing.missing_references, vec![H256::repeat_byte(9)]);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 1);

        let transient = root.prepare_add_block(add_block_request(
            composed_add_block_rlp(H256::repeat_byte(1), 1, &[]),
            false,
        ))?;
        let report = root.complete_add_block(DagAddBlockCompletion {
            cursor_id: transient.cursor_id,
            account_nonce_facts: Vec::new(),
        })?;
        assert_eq!(report.counters.dag_blocks, 0);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 2);
        drop(root);
        drop(storage);

        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let restored = DagTransactionService::restore(storage.clone(), service_config())?;
        assert_eq!(restored.lock_dag()?.state.vertex_count(), 1);
        drop(restored);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn concurrent_add_block_prepares_publish_exactly_one_cursor() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_concurrent");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = Arc::new(DagTransactionService::restore(
            storage.clone(),
            service_config(),
        )?);
        let block_rlp = composed_add_block_rlp(H256::repeat_byte(1), 1, &[]);
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let root = root.clone();
            let block_rlp = block_rlp.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                root.prepare_add_block(add_block_request(block_rlp, false))
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| worker.join().expect("prepare worker must not panic"))
            .collect::<Vec<_>>();
        let cursors = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().ok().map(|prepared| prepared.cursor_id))
            .collect::<Vec<_>>();
        assert_eq!(cursors.len(), 1);
        assert_ne!(cursors[0], 0);
        let errors = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().err())
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0]
                .to_string()
                .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE")
        );
        assert!(root.abort_add_block(cursors[0])?);
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn finalized_order_clears_transaction_sidecars_only_after_dag_commit() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_finalization");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let mut config = service_config();
        config.dag.dag_expiry_limit = 1;
        let root = DagTransactionService::restore(storage.clone(), config)?;
        let expired_hash = H256::repeat_byte(3);
        let anchor_hash = H256::repeat_byte(8);
        let transaction_hash = H256::repeat_byte(7);
        {
            let mut dag = root.lock_dag()?;
            dag.state.add_block(DagManagerBlock {
                hash: expired_hash,
                pivot: H256::repeat_byte(1),
                tips: Vec::new(),
                level: 3,
                difficulty: 90,
            })?;
        }
        save_dag_block_to_storage(
            storage.as_ref(),
            expired_hash,
            3,
            0,
            &composed_add_block_rlp(
                H256::repeat_byte(1),
                3,
                std::slice::from_ref(&transaction_hash),
            ),
        )?;
        save_dag_block_to_storage(
            storage.as_ref(),
            anchor_hash,
            5,
            0,
            &composed_add_block_rlp(H256::repeat_byte(1), 5, &[]),
        )?;
        storage.transaction().write(transaction_hash, &[0xA7])?;
        root.lock_transaction()?
            .sidecar
            .insert_non_finalized(transaction_hash, vec![0xA7])?;

        let error = root
            .apply_finalized_order(anchor_hash, 2, vec![anchor_hash])
            .expect_err("an invalid period must fail before sidecar publication");
        assert!(
            error
                .to_string()
                .contains("DAG_RUNTIME_SET_FINALIZED_ORDER")
        );
        assert!(
            root.lock_transaction()?
                .sidecar
                .contains_non_finalized(transaction_hash)
        );

        let report = root.apply_finalized_order(anchor_hash, 1, vec![anchor_hash])?;
        assert_eq!(report.finalized_count, 1);
        assert_eq!(report.expired_hashes, vec![expired_hash]);
        assert!(
            !root
                .lock_transaction()?
                .sidecar
                .contains_non_finalized(transaction_hash)
        );
        assert!(
            storage
                .transaction()
                .rlp(transaction_hash)?
                .unwrap_or_default()
                .is_empty()
        );
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }
}
