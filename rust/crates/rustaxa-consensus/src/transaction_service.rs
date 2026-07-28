use crate::gas_pricer::{GasPriceOracle, GasPricerConfig};
use crate::transaction_manager::TransactionManagerSidecar;
use crate::transaction_manager::{DagTransactionSaveFact, plan_transactions_from_dag_block};
use crate::transaction_packing_service::TransactionPackingService;
use crate::transaction_queue::TransactionQueue;
use crate::transaction_storage::{
    NonFinalizedTransactionStoragePayload, append_non_finalized_transactions_to_batch,
    transaction_finalized,
};
use anyhow::{Context, Result, anyhow};
use ethereum_types::{H256, U256};
use rustaxa_storage::StorageWriteBatch;
use rustaxa_storage::{StatusField, Storage};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

/// Stable failure identifier returned when the native transaction lock is poisoned.
pub const DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED: &str =
    "DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED";

/// Immutable inputs for constructing and restoring the native transaction owner.
///
/// Queue and gas-cache capacities are derived from `queue_max_size`. The gas
/// oracle validates its own percentile configuration and restores finalized
/// history according to `gas_pricer_config`. `proposal_dag_gas_limit` is the
/// queue-weight limit used by pool-mode gas-price bids.
#[derive(Clone, Debug)]
pub struct TransactionServiceConfig {
    /// Maximum number of transactions retained by the native queue.
    pub queue_max_size: usize,
    /// Gas-oracle policy and restoration mode.
    pub gas_pricer_config: GasPricerConfig,
    /// Proposal weight used when deriving a pool gas-price bid.
    pub proposal_dag_gas_limit: u64,
}

/// Complete mutable transaction application state.
///
/// The state is published only inside [`TransactionService`] after transaction
/// count and gas-history restoration succeed. Its public fields are a temporary
/// CRW-12 bridge escape hatch for short-lived Rust adapters; callers must hold a
/// [`TransactionServiceGuard`] and must not retain references across external
/// executor callbacks.
pub struct TransactionServiceState {
    /// Live transaction count, payload sidecars, and gas-estimation cache.
    pub sidecar: TransactionManagerSidecar,
    /// Native pending-transaction queue.
    pub queue: TransactionQueue,
    /// Finalized-history gas oracle.
    pub gas_price_oracle: GasPriceOracle,
    /// Proposal weight used to derive the queue inclusion price.
    pub proposal_dag_gas_limit: u64,
    /// Shared durable storage used by migrated transaction routes.
    pub storage: Arc<Storage>,
    /// Last queue-drop observation used by compatibility telemetry.
    pub last_drop_observed: Option<Instant>,
    /// Native owner of the current proposal-packing session.
    pub transaction_packing: TransactionPackingService,
}

/// Native fact for one transaction considered during DAG-block persistence.
#[derive(Clone)]
pub(crate) struct DagTransactionSaveInput {
    pub input_index: u64,
    pub hash: H256,
    pub transaction_rlp: Vec<u8>,
    pub transaction_nonce: U256,
    pub sender_account_nonce: U256,
}

/// Native accepted-transaction publication result.
#[derive(Clone, Copy)]
pub struct DagTransactionSaveAccepted {
    /// Canonical transaction identity.
    pub hash: H256,
    /// Whether publication erased the transaction from the live queue.
    pub erased_from_queue: bool,
}

/// Native live-state result for a committed DAG transaction save.
pub struct DagTransactionSaveOutcome {
    /// Accepted transactions in canonical input order.
    pub accepted: Vec<DagTransactionSaveAccepted>,
}

/// Prepared persistence retained until a shared DAG/transaction batch commits.
pub(crate) struct PreparedDagTransactionSave {
    accepted: Vec<DagTransactionSaveAccepted>,
    accepted_payloads: Vec<NonFinalizedTransactionStoragePayload>,
    target_transaction_count: u64,
}

/// Fully prevalidated transaction live-state publication.
pub(crate) struct PreparedDagTransactionPublication {
    queue: TransactionQueue,
    sidecar: TransactionManagerSidecar,
    outcome: DagTransactionSaveOutcome,
}

/// Native owner of transaction construction, restoration, and serialization.
///
/// One mutex protects queue, sidecar/cache/count, gas oracle, durable storage,
/// drop observation, and the packing subowner. Poisoning is mapped to the stable
/// [`DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED`] identifier.
pub struct TransactionService {
    state: Mutex<TransactionServiceState>,
}

impl TransactionService {
    /// Restores and publishes the complete transaction runtime.
    ///
    /// The durable transaction count and any enabled finalized-block gas-price
    /// history are restored before the mutex-owning service is constructed.
    /// Missing count metadata retains storage's canonical zero behavior.
    /// Light/full-node missing-history behavior is delegated unchanged to the
    /// gas oracle. Any validation, storage, or decode error publishes no owner.
    pub fn restore(storage: Arc<Storage>, config: TransactionServiceConfig) -> Result<Self> {
        Ok(Self {
            state: Mutex::new(TransactionServiceState::restore(storage, config)?),
        })
    }

    /// Locks the complete transaction serialization domain.
    ///
    /// The returned guard exposes native state only to short-lived Rust
    /// adapters. A poisoned lock returns the stable transaction-lock identifier.
    pub fn lock(&self) -> Result<TransactionServiceGuard<'_>> {
        Ok(TransactionServiceGuard(self.state.lock().map_err(
            |_| anyhow!(DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED),
        )?))
    }
}

/// Exclusive native transaction runtime guard.
///
/// The guard dereferences to the complete state and releases the transaction
/// lock on drop. It must never cross CXX or an EVM, FinalChain, network, event,
/// logging, thread-pool, or asynchronous executor boundary.
pub struct TransactionServiceGuard<'a>(MutexGuard<'a, TransactionServiceState>);

impl Deref for TransactionServiceGuard<'_> {
    type Target = TransactionServiceState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TransactionServiceGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TransactionServiceState {
    /// Restores the complete state used by [`TransactionService`].
    ///
    /// This is public only for the temporary bridge adapter and its focused
    /// mechanics tests. Production publication must use
    /// [`TransactionService::restore`] so a partially restored state cannot
    /// escape without its native lock owner.
    pub fn restore(storage: Arc<Storage>, config: TransactionServiceConfig) -> Result<Self> {
        let initial_transaction_count = storage
            .metadata()
            .status_field(StatusField::TrxCount as u8)
            .context("TM_RUNTIME_TRANSACTION_COUNT_READ")?;
        let blocks_gas_pricer = config.gas_pricer_config.blocks_gas_pricer;
        let mut gas_price_oracle = GasPriceOracle::new(config.gas_pricer_config)?;
        if blocks_gas_pricer {
            gas_price_oracle
                .restore_from_storage(storage.as_ref())
                .context("TM_RUNTIME_GAS_PRICE_HISTORY_RESTORE")?;
        }
        let gas_estimation_cache_size = config.queue_max_size / 10;
        let gas_estimation_cache_delete_step = config.queue_max_size / 100;
        Ok(Self {
            sidecar: TransactionManagerSidecar::new_with_gas_estimation_cache(
                initial_transaction_count,
                gas_estimation_cache_size,
                gas_estimation_cache_delete_step,
            ),
            queue: TransactionQueue::new(config.queue_max_size as u64),
            gas_price_oracle,
            proposal_dag_gas_limit: config.proposal_dag_gas_limit,
            storage,
            last_drop_observed: None,
            transaction_packing: TransactionPackingService::new(),
        })
    }
}

/// Plans accepted DAG transactions without mutating storage or live state.
pub(crate) fn prepare_dag_transactions(
    runtime: &TransactionServiceState,
    facts: Vec<DagTransactionSaveInput>,
) -> Result<PreparedDagTransactionSave> {
    let plan = plan_transactions_from_dag_block(
        facts
            .into_iter()
            .map(|fact| DagTransactionSaveFact {
                input_index: fact.input_index,
                hash: fact.hash,
                trx_rlp: fact.transaction_rlp,
                transaction_nonce: fact.transaction_nonce,
                sender_account_nonce: fact.sender_account_nonce,
                in_non_finalized_cache: runtime.sidecar.contains_non_finalized(fact.hash),
                in_recently_finalized_cache: runtime.sidecar.contains_recently_finalized(fact.hash),
            })
            .collect(),
        runtime.sidecar.transaction_count(),
        |hash| {
            transaction_finalized(runtime.storage.as_ref(), hash)
                .context("TM_DAG_TX_FINALIZED_LOOKUP_FAILED")
        },
    )?;

    let accepted = plan
        .accepted_transactions
        .iter()
        .map(|payload| DagTransactionSaveAccepted {
            hash: payload.hash,
            erased_from_queue: false,
        })
        .collect();
    let accepted_payloads = plan
        .accepted_transactions
        .into_iter()
        .map(|payload| NonFinalizedTransactionStoragePayload {
            hash: payload.hash,
            trx_rlp: payload.trx_rlp,
        })
        .collect();
    Ok(PreparedDagTransactionSave {
        accepted,
        accepted_payloads,
        target_transaction_count: plan.target_transaction_count,
    })
}

/// Appends prepared DAG transaction writes to a caller-owned atomic batch.
pub(crate) fn append_prepared_dag_transactions(
    storage: &Storage,
    batch: &mut StorageWriteBatch,
    prepared: &PreparedDagTransactionSave,
) -> Result<()> {
    if prepared.accepted_payloads.is_empty() {
        return Ok(());
    }
    append_non_finalized_transactions_to_batch(
        storage,
        batch,
        prepared.accepted_payloads.clone(),
        prepared.target_transaction_count,
    )
}

/// Precomputes queue and sidecar state for post-commit publication.
pub(crate) fn prepare_dag_transaction_publication(
    runtime: &TransactionServiceState,
    prepared: &PreparedDagTransactionSave,
) -> Result<PreparedDagTransactionPublication> {
    let mut queue = runtime.queue.clone();
    let mut sidecar = runtime.sidecar.clone();
    let mut accepted = prepared.accepted.clone();
    for (accepted_entry, payload) in accepted.iter_mut().zip(prepared.accepted_payloads.iter()) {
        accepted_entry.erased_from_queue = queue.erase(payload.hash);
        sidecar.insert_non_finalized(payload.hash, payload.trx_rlp.clone())?;
    }
    sidecar.set_transaction_count(prepared.target_transaction_count);
    Ok(PreparedDagTransactionPublication {
        queue,
        sidecar,
        outcome: DagTransactionSaveOutcome { accepted },
    })
}

/// Publishes a prevalidated transaction transition after shared-batch commit.
pub(crate) fn publish_dag_transactions(
    runtime: &mut TransactionServiceState,
    publication: PreparedDagTransactionPublication,
) -> DagTransactionSaveOutcome {
    runtime.queue = publication.queue;
    runtime.sidecar = publication.sidecar;
    publication.outcome
}

/// Removes live non-finalized sidecars after native DAG cleanup committed storage.
///
/// The operation is infallible and performs no storage writes. Hashes absent
/// from the sidecar are ignored so restart and duplicate cleanup remain safe.
pub(crate) fn remove_non_finalized_sidecars_after_dag_commit(
    runtime: &mut TransactionServiceState,
    hashes: &[H256],
) -> u64 {
    hashes.iter().fold(0, |removed, hash| {
        removed + u64::from(runtime.sidecar.remove_non_finalized(*hash))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use ethereum_types::H256;
    use rlp::RlpStream;
    use rustaxa_storage::{Config, Storage};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn build_service_with_defaults(
        status_field: Option<u64>,
        initial_queue_size: usize,
        gas_pricer_config: GasPricerConfig,
    ) -> Result<(TransactionService, std::path::PathBuf)> {
        let temp_dir = unique_temp_dir("rustaxa_consensus_transaction_service_test");
        let storage = Arc::new(Storage::new(Config::new(temp_dir.clone()))?);
        if let Some(count) = status_field {
            storage
                .metadata()
                .write_status_field(StatusField::TrxCount as u8, count)?;
        }
        let service = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: initial_queue_size,
                gas_pricer_config,
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;
        Ok((service, temp_dir))
    }

    fn append_gas_price_transaction(stream: &mut RlpStream, gas_price: u64) {
        stream.begin_list(9);
        stream.append(&0u64);
        stream.append(&gas_price);
        stream.append(&21_000u64);
        stream.append_empty_data();
        stream.append(&0u64);
        stream.append_empty_data();
        stream.append(&27u64);
        stream.append(&1u64);
        stream.append(&1u64);
    }

    fn seed_gas_price_history(storage: &Storage, blocks: &[(u64, &[u64])]) -> Result<()> {
        for &(period, prices) in blocks {
            let mut period_rlp = RlpStream::new_list(4);
            period_rlp.append_empty_data();
            period_rlp.append_empty_data();
            period_rlp.begin_list(0);
            period_rlp.begin_list(prices.len());
            for &gas_price in prices {
                append_gas_price_transaction(&mut period_rlp, gas_price);
            }
            storage.period().write(period, &period_rlp.out())?;
        }
        Ok(())
    }

    fn seed_last_finalized_block(storage: &Storage, block: u64) -> Result<()> {
        storage
            .final_chain()
            .write_block_header(block, H256::zero(), &[], &[])
            .context("seed_last_finalized_block")
    }

    #[test]
    fn restore_defaults_missing_count_to_zero() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            None,
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: ethereum_types::U256::one(),
                history_blocks: 10,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let runtime = service.lock()?;

        assert_eq!(runtime.sidecar.transaction_count(), 0);

        drop(runtime);
        drop(service);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_reads_persisted_count() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            Some(73),
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: ethereum_types::U256::one(),
                history_blocks: 10,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let runtime = service.lock()?;

        assert_eq!(runtime.sidecar.transaction_count(), 73);

        drop(runtime);
        drop(service);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restored_owner_keeps_storage_queue_sidecar_and_packing_coherent() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_transaction_service_coherence");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = TransactionService::restore(
            storage.clone(),
            TransactionServiceConfig {
                queue_max_size: 8,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::one(),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 42_000,
            },
        )?;
        let runtime = service.lock()?;

        assert!(Arc::ptr_eq(&runtime.storage, &storage));
        assert_eq!(runtime.sidecar.transaction_count(), 0);
        assert_eq!(runtime.queue.size(), 0);
        assert!(!runtime.transaction_packing.is_active()?);
        assert_eq!(runtime.proposal_dag_gas_limit, 42_000);
        assert!(runtime.last_drop_observed.is_none());

        drop(runtime);
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn gas_pricer_updates_history_and_ignores_empty_blocks() -> Result<()> {
        let temp_dir = unique_temp_dir("rustaxa_consensus_transaction_service_test_gas_oracle");
        let storage = Arc::new(Storage::new(Config::new(temp_dir.clone()))?);
        let config = GasPricerConfig {
            percentile: 50,
            minimum_price: ethereum_types::U256::one(),
            history_blocks: 10,
            is_light_node: false,
            blocks_gas_pricer: true,
        };
        let service = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: config,
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;
        let mut runtime = service.lock()?;

        runtime
            .gas_price_oracle
            .update_from_gas_prices(std::iter::empty::<ethereum_types::U256>());
        assert_eq!(
            runtime.gas_price_oracle.bid(),
            ethereum_types::U256::from(1_u64)
        );

        for price in [1_u64, 2, 3, 4, 5] {
            runtime
                .gas_price_oracle
                .update_from_gas_prices([ethereum_types::U256::from(price)]);
        }
        assert_eq!(
            runtime.gas_price_oracle.bid(),
            ethereum_types::U256::from(3_u64)
        );

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn restore_restores_gas_history_and_restarts() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_transaction_service_restart");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        seed_gas_price_history(&storage, &[(2, &[9, 5]), (1, &[8])])?;
        seed_last_finalized_block(&storage, 2)?;

        for _ in 0..2 {
            let service = TransactionService::restore(
                storage.clone(),
                TransactionServiceConfig {
                    queue_max_size: 16,
                    gas_pricer_config: GasPricerConfig {
                        percentile: 50,
                        minimum_price: ethereum_types::U256::one(),
                        history_blocks: 10,
                        is_light_node: false,
                        blocks_gas_pricer: true,
                    },
                    proposal_dag_gas_limit: 1_000_000,
                },
            )?;
            let runtime = service.lock()?;
            assert_eq!(
                runtime.gas_price_oracle.bid(),
                ethereum_types::U256::from(5_u64)
            );
        }

        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn light_mode_stops_without_full_missing_history() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_transaction_service_light_history");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        seed_gas_price_history(&storage, &[(2, &[9])])?;
        seed_last_finalized_block(&storage, 3)?;

        let light = TransactionService::restore(
            storage.clone(),
            TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::from(7_u64),
                    history_blocks: 10,
                    is_light_node: true,
                    blocks_gas_pricer: true,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;
        let runtime = light.lock()?;
        assert_eq!(
            runtime.gas_price_oracle.bid(),
            ethereum_types::U256::from(7_u64)
        );

        let err = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::one(),
                    history_blocks: 10,
                    is_light_node: false,
                    blocks_gas_pricer: true,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
        )
        .err()
        .expect("full-node history restore must reject missing period data");
        assert!(
            format!("{err:#}").contains("missing finalized transactions for block 3"),
            "unexpected restoration error: {err:#}"
        );

        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn pool_bid_respects_queue_limit_and_oracle_floor() -> Result<()> {
        let temp_dir = unique_temp_dir("rustaxa_consensus_transaction_service_test_pool_bid");
        let storage = Arc::new(Storage::new(Config::new(temp_dir.clone()))?);
        let service = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 8,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::from(4_u64),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 42_000,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            runtime
                .queue
                .insert(
                    crate::transaction_queue::TransactionQueueEntry {
                        hash: ethereum_types::H256::from_low_u64_be(1),
                        sender: ethereum_types::H160::from_low_u64_be(1),
                        nonce: ethereum_types::U256::zero(),
                        gas_price: ethereum_types::U256::from(2_u64),
                        gas: 21_000,
                        data_size: 0,
                        rlp: vec![1],
                        last_block_number: 0,
                    },
                    true,
                )
                .expect("queue insert should work");
            runtime
                .queue
                .insert(
                    crate::transaction_queue::TransactionQueueEntry {
                        hash: ethereum_types::H256::from_low_u64_be(2),
                        sender: ethereum_types::H160::from_low_u64_be(2),
                        nonce: ethereum_types::U256::zero(),
                        gas_price: ethereum_types::U256::from(4_u64),
                        gas: 21_000,
                        data_size: 0,
                        rlp: vec![2],
                        last_block_number: 0,
                    },
                    true,
                )
                .expect("queue insert should work");

            let pool_price = runtime.queue.min_gas_price_for_block_inclusion(42_000);
            assert_eq!(pool_price, ethereum_types::U256::from(3_u64));
            assert_eq!(
                runtime.gas_price_oracle.configured_bid(pool_price),
                ethereum_types::U256::from(4_u64)
            );
        }

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn invalid_gas_config_fails_before_service_publication() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_transaction_service_invalid_config");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let error = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 8,
                gas_pricer_config: GasPricerConfig {
                    percentile: 101,
                    minimum_price: ethereum_types::U256::one(),
                    history_blocks: 1,
                    is_light_node: false,
                    blocks_gas_pricer: true,
                },
                proposal_dag_gas_limit: 42_000,
            },
        )
        .err()
        .expect("invalid percentile must reject construction");
        assert!(error.to_string().contains("percentile"));
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn poisoned_service_lock_uses_stable_identifier() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            None,
            8,
            GasPricerConfig {
                percentile: 50,
                minimum_price: ethereum_types::U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                let _guard = service.state.lock().unwrap();
                panic!("poison native transaction service");
            });
            assert!(handle.join().is_err());
        });
        assert_eq!(
            service
                .lock()
                .err()
                .expect("poisoned service must reject locking")
                .to_string(),
            DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED
        );
        std::fs::remove_dir_all(path)?;
        Ok(())
    }
}
