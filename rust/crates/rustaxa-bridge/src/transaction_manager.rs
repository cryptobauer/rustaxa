//! CXX bridge wrappers for Rust `TransactionManager` decision boundaries.
//!
//! The bridge exposes:
//! - a short-lived planner used while one DAG proposal is being packed
//! - a storage-complete planner for `TransactionManager::saveTransactionsFromDagBlock`
//!
//! C++ supplies live transaction metadata and storage facts; Rust keeps planning and
//! persistence decisions deterministic and does not own live pointers or shared caches.

use crate::ffi::rustaxa_ffi::{
    DagTransactionSaveAccepted, DagTransactionSaveFact, DagTransactionSaveOutcome,
    FinalizedTransactionStatusAction, FinalizedTransactionStatusFact,
    FinalizedTransactionStatusPlan, NonFinalizedTransactionPayload,
    TransactionManagerRecoveryEntry, TransactionManagerStoredTransactionLookup,
    TransactionManagerStoredTransactionRequest, TransactionPackCandidateDecision,
    TransactionPackCandidateInput, TransactionPackEstimateInput, TransactionPackEstimateOutcome,
};
use crate::ffi::{BridgeStorage, BridgeTransactionPackPlanner};
use anyhow::{Context, Result};
use ethereum_types::{H256, U256};
use rustaxa_consensus::transaction_manager::{
    plan_finalized_transactions_status, plan_transactions_from_dag_block,
    DagTransactionSaveFact as ConsensusDagTransactionSaveFact, DagTransactionSavePayload,
    FinalizedTransactionStatusFact as ConsensusFinalizedTransactionStatusFact,
    FinalizedTransactionStatusPlan as ConsensusFinalizedTransactionStatusPlan,
    TransactionPackCandidate, TransactionPackEstimate, TransactionPackingPlanner,
};

/// Plans and persists accepted transactions for one incoming DAG block.
///
/// This call is storage-complete: accepted payloads are written with one atomic
/// batch through `save_non_finalized_transactions`, and `target_transaction_count`
/// is returned for C++ cache-state bookkeeping.
pub fn save_transactions_from_dag_block(
    storage: &BridgeStorage,
    current_transaction_count: u64,
    facts: Vec<DagTransactionSaveFact>,
) -> Result<DagTransactionSaveOutcome> {
    let plan = plan_transactions_from_dag_block(
        facts
            .into_iter()
            .map(consensus_fact_from_ffi_fact)
            .collect(),
        current_transaction_count,
        |hash| {
            storage
                .0
                .transaction()
                .finalized(hash)
                .context("TM_DAG_TX_FINALIZED_LOOKUP_FAILED")
        },
    )?;

    let mut accepted: Vec<DagTransactionSaveAccepted> =
        Vec::with_capacity(plan.accepted_transactions.len());
    let mut accepted_payloads: Vec<NonFinalizedTransactionPayload> =
        Vec::with_capacity(plan.accepted_transactions.len());

    for DagTransactionSavePayload {
        input_index,
        hash,
        trx_rlp,
    } in plan.accepted_transactions
    {
        accepted.push(DagTransactionSaveAccepted {
            input_index,
            hash: hash.0,
        });
        accepted_payloads.push(NonFinalizedTransactionPayload {
            hash: hash.0,
            trx_rlp,
        });
    }

    if !accepted_payloads.is_empty() {
        storage
            .save_non_finalized_transactions(accepted_payloads, plan.target_transaction_count)?;
    }

    Ok(DagTransactionSaveOutcome {
        accepted,
        target_transaction_count: plan.target_transaction_count,
    })
}

/// Plans finalized-transaction status transitions for one period.
///
/// This call is storage-bound for the `StatusDbField::TrxCount` counter only.
/// The counter persists only when the finalized transaction list is non-empty,
/// matching existing Rust-side storage behavior for conditional status updates.
pub fn update_finalized_transactions_status(
    storage: &BridgeStorage,
    period: u64,
    retention_window: u64,
    current_transaction_count: u64,
    facts: Vec<FinalizedTransactionStatusFact>,
) -> Result<FinalizedTransactionStatusPlan> {
    let plan: ConsensusFinalizedTransactionStatusPlan = plan_finalized_transactions_status(
        facts
            .into_iter()
            .map(consensus_finalized_status_fact_from_ffi_fact)
            .collect(),
        current_transaction_count,
        period,
        retention_window,
    )?;

    if !plan.accepted_transactions.is_empty() {
        storage
            .save_status_field(
                rustaxa_storage::StatusField::TrxCount as u8,
                plan.target_transaction_count,
            )
            .context("TM_FINALIZED_STATUS_TRXCOUNT_WRITE")?;
    }

    Ok(FinalizedTransactionStatusPlan {
        accepted: plan
            .accepted_transactions
            .into_iter()
            .map(|action| FinalizedTransactionStatusAction {
                input_index: action.input_index,
                hash: action.hash.0,
            })
            .collect(),
        target_transaction_count: plan.target_transaction_count,
        stale_period: plan.stale_period.unwrap_or(0),
        has_stale_period: plan.stale_period.is_some(),
        purge_transaction_queue: plan.purge_transactions,
    })
}

const TM_STORED_TX_SOURCE_MISSING: u8 = 0;
const TM_STORED_TX_SOURCE_PENDING: u8 = 1;
const TM_STORED_TX_SOURCE_FINALIZED_REGULAR: u8 = 2;
const TM_STORED_TX_SOURCE_FINALIZED_SYSTEM: u8 = 3;

/// Resolves transaction hashes through TransactionManager storage rules.
///
/// Inputs are ordered requests from C++ after live transaction caches miss.
/// Outputs preserve request order, echo `input_index` and `hash`, classify the
/// storage source, and carry canonical transaction RLP bytes for C++ object
/// materialization. Missing hashes are returned as `source = 0` instead of
/// errors. Storage backend failures, malformed transaction-location RLP, and
/// malformed period data return `anyhow::Error` with stable context labels.
pub fn transaction_manager_load_stored_transactions(
    storage: &BridgeStorage,
    requests: Vec<TransactionManagerStoredTransactionRequest>,
) -> Result<Vec<TransactionManagerStoredTransactionLookup>> {
    let mut out = Vec::with_capacity(requests.len());
    let transaction = storage.0.transaction();

    for request in requests {
        let hash = H256::from(request.hash);
        let (tx_rlp, source) = if let Some(tx_rlp) = transaction
            .rlp(hash)
            .context("TM_TRANSACTION_RLP_PENDING_LOOKUP")?
        {
            (Some(tx_rlp), TM_STORED_TX_SOURCE_PENDING)
        } else if let Some(location_rlp) = transaction
            .location_rlp(hash)
            .context("TM_TRANSACTION_RLP_LOCATION_LOOKUP")?
        {
            let location = rlp::Rlp::new(&location_rlp);
            let period = location
                .val_at::<u64>(0)
                .context("TM_TRANSACTION_RLP_LOCATION_PERIOD")?;
            let position = location
                .val_at::<u32>(1)
                .context("TM_TRANSACTION_RLP_LOCATION_POSITION")?;
            let is_system = location
                .item_count()
                .context("TM_TRANSACTION_RLP_LOCATION_SHAPE")?
                == 3
                && location
                    .val_at::<bool>(2)
                    .context("TM_TRANSACTION_RLP_LOCATION_SYSTEM_FLAG")?;
            let tx_rlp = if is_system {
                transaction
                    .system_rlp(hash)
                    .context("TM_TRANSACTION_RLP_SYSTEM_LOOKUP")?
                    .map(|tx_rlp| (tx_rlp, TM_STORED_TX_SOURCE_FINALIZED_SYSTEM))
            } else {
                transaction
                    .by_period_position_rlp(period, position)
                    .context("TM_TRANSACTION_RLP_FINALIZED_LOOKUP")?
                    .map(|tx_rlp| (tx_rlp, TM_STORED_TX_SOURCE_FINALIZED_REGULAR))
            };

            match tx_rlp {
                Some((tx_rlp, source)) => (Some(tx_rlp), source),
                None => (None, TM_STORED_TX_SOURCE_MISSING),
            }
        } else {
            (None, TM_STORED_TX_SOURCE_MISSING)
        };

        out.push(TransactionManagerStoredTransactionLookup {
            input_index: request.input_index,
            hash: hash.0,
            found: tx_rlp.is_some(),
            source,
            tx_rlp: tx_rlp.unwrap_or_default(),
        });
    }

    Ok(out)
}

/// Returns all payloads currently persisted for non-finalized transaction recovery.
///
/// The returned list preserves storage iteration order, carries the DB key hash
/// for invariant validation by C++, and flags any payloads that are stale by
/// checking finalized-index membership in Rust. Stale finalized rows are removed
/// from non-finalized storage in one Rust storage batch before returning.
pub fn transaction_manager_load_nonfinalized_recovery(
    storage: &BridgeStorage,
) -> Result<Vec<TransactionManagerRecoveryEntry>> {
    let transaction = storage.0.transaction();
    let non_finalized = transaction.all_nonfinalized_with_hash()?;
    let mut out = Vec::with_capacity(non_finalized.len());
    let mut stale_hashes = Vec::new();

    for (hash, tx_rlp) in non_finalized {
        let finalized = transaction
            .finalized(hash)
            .context("TM_NONFINALIZED_RECOVERY_FINALIZED_LOOKUP")?;
        if finalized {
            stale_hashes.push(hash);
        }

        out.push(TransactionManagerRecoveryEntry {
            hash: hash.0,
            finalized,
            tx_rlp,
        });
    }

    if !stale_hashes.is_empty() {
        let mut batch = storage.0.create_write_batch();
        for hash in stale_hashes {
            storage
                .0
                .batch_delete_raw(
                    &mut batch,
                    rustaxa_storage::Column::Transactions,
                    hash.as_bytes(),
                )
                .context("TM_NONFINALIZED_RECOVERY_STALE_DELETE")?;
        }
        storage
            .0
            .commit_write_batch_with_sync(batch, false)
            .context("TM_NONFINALIZED_RECOVERY_STALE_COMMIT")?;
    }

    Ok(out)
}

fn consensus_fact_from_ffi_fact(fact: DagTransactionSaveFact) -> ConsensusDagTransactionSaveFact {
    ConsensusDagTransactionSaveFact {
        input_index: fact.input_index,
        hash: H256::from(fact.hash),
        trx_rlp: fact.trx_rlp,
        transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
        sender_account_nonce: U256::from_big_endian(&fact.sender_account_nonce),
        in_non_finalized_cache: fact.in_non_finalized_cache,
        in_recently_finalized_cache: fact.in_recently_finalized_cache,
    }
}

fn consensus_finalized_status_fact_from_ffi_fact(
    fact: FinalizedTransactionStatusFact,
) -> ConsensusFinalizedTransactionStatusFact {
    ConsensusFinalizedTransactionStatusFact {
        input_index: fact.input_index,
        hash: H256::from(fact.hash),
        in_non_finalized_cache: fact.in_non_finalized_cache,
    }
}

/// Creates a planner for one `TransactionManager::packTrxs` invocation.
pub fn create_transaction_pack_planner(
    weight_limit: u64,
    min_transaction_gas: u64,
) -> Result<Box<BridgeTransactionPackPlanner>> {
    Ok(Box::new(BridgeTransactionPackPlanner(
        TransactionPackingPlanner::new(weight_limit, min_transaction_gas)?,
    )))
}

impl BridgeTransactionPackPlanner {
    /// Returns the maximum number of ordered queue candidates C++ should snapshot.
    pub fn transaction_pack_max_candidate_count(&self) -> u64 {
        self.0.max_candidate_count()
    }

    /// Decides whether C++ should run a live gas estimate for this candidate.
    pub fn transaction_pack_consider_candidate(
        &self,
        input: TransactionPackCandidateInput,
    ) -> Result<TransactionPackCandidateDecision> {
        let decision = self.0.consider_candidate(TransactionPackCandidate {
            hash: H256::from(input.hash),
            declared_gas: input.declared_gas,
        })?;
        Ok(TransactionPackCandidateDecision {
            should_estimate: decision.should_estimate,
        })
    }

    /// Records a C++ gas estimate and returns the live-state action C++ must apply.
    pub fn transaction_pack_record_estimate(
        &mut self,
        input: TransactionPackEstimateInput,
    ) -> Result<TransactionPackEstimateOutcome> {
        let outcome = self.0.record_estimate(TransactionPackEstimate {
            hash: H256::from(input.hash),
            gas_used: input.gas_used,
        })?;
        Ok(TransactionPackEstimateOutcome {
            hash: outcome.hash.0,
            selected: outcome.selected,
            demote_to_non_proposable: outcome.demote_to_non_proposable,
            stop: outcome.stop,
            gas_used: outcome.gas_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::RlpStream;
    use rustaxa_storage::StatusField;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn bridge_planner_round_trips_estimation_decisions() {
        let mut planner = create_transaction_pack_planner(63_000, 21_000).unwrap();
        assert_eq!(planner.transaction_pack_max_candidate_count(), 3);

        let decision = planner
            .transaction_pack_consider_candidate(TransactionPackCandidateInput {
                hash: [1; 32],
                declared_gas: 42_000,
            })
            .unwrap();
        assert!(decision.should_estimate);

        let outcome = planner
            .transaction_pack_record_estimate(TransactionPackEstimateInput {
                hash: [1; 32],
                gas_used: 42_000,
            })
            .unwrap();
        assert!(outcome.selected);
        assert_eq!(outcome.hash, [1; 32]);
        assert!(outcome.stop);
    }

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn dag_tx_fact(
        input_index: u64,
        hash: u8,
        tx_nonce: u64,
        sender_nonce: u64,
        in_non_finalized_cache: bool,
        in_recently_finalized_cache: bool,
        rlp: u8,
    ) -> DagTransactionSaveFact {
        DagTransactionSaveFact {
            input_index,
            hash: [hash; 32],
            trx_rlp: vec![rlp],
            transaction_nonce: u256_bytes(tx_nonce),
            sender_account_nonce: u256_bytes(sender_nonce),
            in_non_finalized_cache,
            in_recently_finalized_cache,
        }
    }

    fn u256_bytes(value: u64) -> [u8; 32] {
        U256::from(value).to_big_endian()
    }

    fn finalized_status_fact(
        input_index: u64,
        hash: u8,
        in_non_finalized_cache: bool,
    ) -> FinalizedTransactionStatusFact {
        FinalizedTransactionStatusFact {
            input_index,
            hash: [hash; 32],
            in_non_finalized_cache,
        }
    }

    #[test]
    fn bridge_save_transactions_from_dag_block_persists_accepted_hashes_and_count() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_transaction_manager_save_dag");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");
        storage
            .save_transaction_location(&[4; 32], 1, 0, false)
            .expect("finalized transaction location should persist");

        let out = save_transactions_from_dag_block(
            &storage,
            7,
            vec![
                dag_tx_fact(0, 1, 5, 4, false, false, 11),
                dag_tx_fact(1, 1, 5, 4, false, false, 12),
                dag_tx_fact(2, 2, 5, 4, true, false, 21),
                dag_tx_fact(3, 3, 5, 4, false, true, 31),
                dag_tx_fact(4, 4, 5, 11, false, false, 41),
                dag_tx_fact(5, 5, 2, 1, false, false, 51),
            ],
        )
        .expect("dag transactions from block should save");

        assert_eq!(
            out.accepted
                .iter()
                .map(|entry| (entry.input_index, entry.hash))
                .collect::<Vec<_>>(),
            vec![(0, [1; 32]), (5, [5; 32])]
        );
        assert_eq!(out.target_transaction_count, 9);

        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should persist"),
            9,
        );
        assert_eq!(
            storage
                .get_transaction(&[1u8; 32])
                .expect("transaction should persist"),
            vec![11]
        );
        assert_eq!(
            storage
                .get_transaction(&[5u8; 32])
                .expect("transaction should persist"),
            vec![51]
        );
        assert_eq!(
            storage
                .get_transaction(&[2u8; 32])
                .expect("non-accepted transaction should be missing"),
            Vec::<u8>::new()
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_save_transactions_from_dag_block_skips_all_when_no_new_accepts() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_transaction_manager_save_dag_skip");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");
        storage
            .save_transaction_location(&[2; 32], 1, 0, false)
            .expect("finalized transaction location should persist");

        let out = save_transactions_from_dag_block(
            &storage,
            7,
            vec![
                dag_tx_fact(0, 1, 5, 4, true, false, 11),
                dag_tx_fact(1, 2, 5, 8, false, false, 21),
                dag_tx_fact(2, 3, 5, 4, false, true, 31),
            ],
        )
        .expect("skip-only list should not fail");

        assert!(out.accepted.is_empty());
        assert_eq!(out.target_transaction_count, 7);
        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should remain"),
            7
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_update_finalized_transactions_status_plans_and_persists_count_when_non_empty() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_update_finalized_status");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");

        let out = update_finalized_transactions_status(
            &storage,
            200,
            10,
            7,
            vec![
                finalized_status_fact(0, 1, false),
                finalized_status_fact(1, 2, true),
                finalized_status_fact(2, 3, false),
            ],
        )
        .expect("finalized status plan should be computed");

        assert_eq!(
            out.accepted
                .iter()
                .map(|entry| (entry.input_index, entry.hash))
                .collect::<Vec<_>>(),
            vec![(0, [1; 32]), (1, [2; 32]), (2, [3; 32])]
        );
        assert_eq!(out.target_transaction_count, 9);
        assert_eq!(out.stale_period, 190);
        assert!(out.has_stale_period);
        assert!(out.purge_transaction_queue);
        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should persist"),
            9,
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_update_finalized_transactions_status_skips_status_persistence_when_no_inputs() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_update_finalized_status_skip");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_status_field(StatusField::TrxCount as u8, 11)
            .expect("status field seed should persist");

        let out = update_finalized_transactions_status(&storage, 200, 10, 11, vec![])
            .expect("empty finalized list should still plan");

        assert!(out.accepted.is_empty());
        assert_eq!(out.target_transaction_count, 11);
        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should remain unchanged"),
            11,
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_load_stored_transactions_orders_and_classifies_sources() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_load_stored_transactions");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_transaction(&[1u8; 32], vec![0x11])
            .expect("pending transaction should persist");

        // Persist finalized location metadata and tx-by-position data for hash 2 so lookup
        // exercises finalized fallback path after non-finalized miss.
        storage
            .save_transaction_location(&[2u8; 32], 8, 0, false)
            .expect("finalized location should persist");

        let mut txs = RlpStream::new_list(1);
        txs.append_raw(&[0x22], 1);

        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        storage
            .save_period_data(8, period_data.out().as_ref().to_vec())
            .expect("period data should persist");
        storage
            .save_system_transaction(&[4u8; 32], vec![0x44])
            .expect("system transaction should persist");
        storage
            .save_transaction_location(&[4u8; 32], 9, 0, true)
            .expect("system finalized location should persist");

        let out = transaction_manager_load_stored_transactions(
            &storage,
            vec![
                TransactionManagerStoredTransactionRequest {
                    input_index: 7,
                    hash: [2u8; 32],
                },
                TransactionManagerStoredTransactionRequest {
                    input_index: 8,
                    hash: [3u8; 32],
                },
                TransactionManagerStoredTransactionRequest {
                    input_index: 9,
                    hash: [1u8; 32],
                },
                TransactionManagerStoredTransactionRequest {
                    input_index: 10,
                    hash: [4u8; 32],
                },
            ],
        )
        .expect("transaction payload lookup should preserve order");

        let out = out
            .into_iter()
            .map(|entry| {
                (
                    entry.input_index,
                    entry.hash,
                    entry.found,
                    entry.source,
                    entry.tx_rlp,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(out.len(), 4);
        assert_eq!(
            out[0],
            (
                7,
                [2u8; 32],
                true,
                TM_STORED_TX_SOURCE_FINALIZED_REGULAR,
                vec![0x22]
            )
        );
        assert_eq!(
            out[1],
            (
                8,
                [3u8; 32],
                false,
                TM_STORED_TX_SOURCE_MISSING,
                Vec::<u8>::new()
            )
        );
        assert_eq!(
            out[2],
            (9, [1u8; 32], true, TM_STORED_TX_SOURCE_PENDING, vec![0x11])
        );
        assert_eq!(
            out[3],
            (
                10,
                [4u8; 32],
                true,
                TM_STORED_TX_SOURCE_FINALIZED_SYSTEM,
                vec![0x44]
            )
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_recovery_payloads_mark_stale_finalized_entries() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_transaction_manager_recovery_payloads");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_transaction(&[1u8; 32], vec![0x11])
            .expect("non-finalized transaction should persist");
        storage
            .save_transaction(&[2u8; 32], vec![0x22])
            .expect("finalized stale entry should persist");
        storage
            .save_transaction_location(&[2u8; 32], 11, 0, false)
            .expect("stale finalized entry location should persist");

        let mut txs = RlpStream::new_list(1);
        txs.append_raw(&[0x22], 1);

        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        storage
            .save_period_data(11, period_data.out().as_ref().to_vec())
            .expect("period data should persist");

        let out = transaction_manager_load_nonfinalized_recovery(&storage)
            .expect("recovery payload lookup should inspect all non-finalized storage rows");

        assert_eq!(out.len(), 2);
        let mut by_hash = out
            .into_iter()
            .map(|entry| (entry.hash[0], entry.finalized))
            .collect::<Vec<_>>();
        by_hash.sort_unstable();
        assert_eq!(by_hash, vec![(1u8, false), (2u8, true)]);
        assert_eq!(
            storage
                .get_transaction(&[2u8; 32])
                .expect("stale finalized entry should be removed"),
            Vec::<u8>::new()
        );
        assert_eq!(
            storage
                .get_transaction(&[1u8; 32])
                .expect("live non-finalized entry should remain"),
            vec![0x11]
        );

        let _ = fs::remove_dir_all(temp_dir);
    }
}
