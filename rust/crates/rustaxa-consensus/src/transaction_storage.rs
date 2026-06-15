//! Rust-owned TransactionManager storage operations.
//!
//! TransactionManager planners decide which hashes and payloads are accepted.
//! This module owns the storage write groups for those accepted decisions over
//! `rustaxa-storage`, so production routing does not depend on bridge-owned
//! batches or C++ storage orchestration.

use anyhow::{Context, Result};
use ethereum_types::H256;
use rustaxa_storage::{Column, StatusField, Storage};

/// Non-finalized transaction payload accepted by TransactionManager planning.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NonFinalizedTransactionStoragePayload {
    /// Canonical transaction hash used as the pending transaction key.
    pub hash: H256,
    /// Canonical transaction RLP payload to store while non-finalized.
    pub trx_rlp: Vec<u8>,
}

/// Persists accepted non-finalized transactions and the target transaction count.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `transactions`: accepted pending transaction payloads.
/// - `transaction_count`: absolute manager-owned `TrxCount` after accepting
///   the payloads.
///
/// Outputs:
/// - Commits one Rust-owned storage batch on success.
///
/// Invariants and edge behavior:
/// - Transaction payload writes and `TrxCount` update are atomic.
/// - Empty transaction lists still persist the supplied `TrxCount`, matching
///   existing bridge behavior for an explicit accepted-write call.
/// - Payload bytes are stored as supplied; decoding and EVM validation remain
///   outside this storage boundary.
pub fn save_non_finalized_transactions(
    storage: &Storage,
    transactions: Vec<NonFinalizedTransactionStoragePayload>,
    transaction_count: u64,
) -> Result<()> {
    let mut batch = storage.create_write_batch();

    for transaction in transactions {
        storage
            .batch_put_raw(
                &mut batch,
                Column::Transactions,
                transaction.hash.as_bytes(),
                &transaction.trx_rlp,
            )
            .context("NON_FINALIZED_TRANSACTION_BATCH_PUT")?;
    }

    storage
        .batch_put_raw(
            &mut batch,
            Column::Status,
            &[StatusField::TrxCount as u8],
            &transaction_count.to_le_bytes(),
        )
        .context("NON_FINALIZED_TRANSACTION_COUNT_WRITE")?;

    storage
        .commit_write_batch_with_sync(batch, false)
        .context("NON_FINALIZED_TRANSACTION_BATCH_COMMIT")
}

/// Persists the manager-owned finalized transaction count.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `transaction_count`: target `TrxCount` after finalized-status planning.
///
/// Outputs:
/// - Commits the status write through `rustaxa-storage`.
///
/// Invariants and edge behavior:
/// - This function intentionally owns only the storage row; live sidecar
///   recently-finalized/non-finalized transitions are still executed by the
///   TransactionManager runtime after this write succeeds.
pub fn save_transaction_count(storage: &Storage, transaction_count: u64) -> Result<()> {
    storage
        .metadata()
        .write_status_field(StatusField::TrxCount as u8, transaction_count)
        .context("TRANSACTION_MANAGER_COUNT_WRITE")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_storage::{Config, Storage};

    fn temp_storage(name: &str) -> Storage {
        let dir = std::env::temp_dir().join(format!(
            "{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Storage::new(Config::new(dir)).unwrap()
    }

    #[test]
    fn save_non_finalized_transactions_commits_payloads_and_count() {
        let storage = temp_storage("rustaxa_consensus_transaction_storage_non_finalized");
        let hash = H256::from([0x44; 32]);

        save_non_finalized_transactions(
            &storage,
            vec![NonFinalizedTransactionStoragePayload {
                hash,
                trx_rlp: vec![0xC0],
            }],
            12,
        )
        .unwrap();

        assert_eq!(
            storage.transaction().rlp(hash).unwrap().unwrap(),
            vec![0xC0]
        );
        assert_eq!(
            storage
                .metadata()
                .status_field(StatusField::TrxCount as u8)
                .unwrap(),
            12
        );
    }

    #[test]
    fn save_transaction_count_updates_status_row() {
        let storage = temp_storage("rustaxa_consensus_transaction_storage_count");

        save_transaction_count(&storage, 9).unwrap();

        assert_eq!(
            storage
                .metadata()
                .status_field(StatusField::TrxCount as u8)
                .unwrap(),
            9
        );
    }
}
