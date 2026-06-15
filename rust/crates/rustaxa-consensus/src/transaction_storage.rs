//! Rust-owned TransactionManager storage operations.
//!
//! TransactionManager planners decide which hashes and payloads are accepted.
//! This module owns the storage write groups for those accepted decisions over
//! `rustaxa-storage`, so production routing does not depend on bridge-owned
//! batches or C++ storage orchestration.

use anyhow::{Context, Result};
use ethereum_types::H256;
use rustaxa_storage::{Column, StatusField, Storage};

/// Stored transaction lookup missed every storage source.
pub const STORED_TRANSACTION_SOURCE_MISSING: u8 = 0;
/// Stored transaction lookup found a pending non-finalized payload.
pub const STORED_TRANSACTION_SOURCE_PENDING: u8 = 1;
/// Stored transaction lookup found a finalized regular transaction payload.
pub const STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR: u8 = 2;
/// Stored transaction lookup found a finalized system transaction payload.
pub const STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM: u8 = 3;

/// Non-finalized transaction payload accepted by TransactionManager planning.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NonFinalizedTransactionStoragePayload {
    /// Canonical transaction hash used as the pending transaction key.
    pub hash: H256,
    /// Canonical transaction RLP payload to store while non-finalized.
    pub trx_rlp: Vec<u8>,
}

/// Ordered transaction hash request for storage-backed TransactionManager lookups.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredTransactionLookupRequest {
    /// Caller-owned index echoed back so C++ can preserve request identity.
    pub input_index: u64,
    /// Canonical transaction hash to resolve from pending or finalized storage.
    pub hash: H256,
}

/// Storage-backed TransactionManager lookup result.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredTransactionLookup {
    /// Caller-owned index from the request.
    pub input_index: u64,
    /// Canonical transaction hash that was requested.
    pub hash: H256,
    /// True when `tx_rlp` was loaded from one of the storage sources.
    pub found: bool,
    /// One of the `STORED_TRANSACTION_SOURCE_*` constants.
    pub source: u8,
    /// Canonical transaction RLP payload, or empty when `found` is false.
    pub tx_rlp: Vec<u8>,
}

/// Stored non-finalized transaction row used by TransactionManager restart recovery.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NonFinalizedTransactionRecoveryEntry {
    /// Hash from the persisted non-finalized transaction key.
    pub hash: H256,
    /// True when the same hash is already indexed as finalized.
    pub finalized: bool,
    /// Canonical transaction RLP payload stored under the non-finalized key.
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

/// Returns whether a transaction hash is indexed as finalized.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `hash`: canonical transaction hash to check.
///
/// Outputs:
/// - `true` when the finalized transaction-location index contains the hash.
///
/// Invariants and edge behavior:
/// - This is a storage fact lookup only; recent-finalized sidecars and
///   account-nonce gates are supplied by the TransactionManager runtime
///   boundary.
/// - Storage backend errors are returned with TransactionManager context.
pub fn transaction_finalized(storage: &Storage, hash: H256) -> Result<bool> {
    storage
        .transaction()
        .finalized(hash)
        .context("TRANSACTION_MANAGER_FINALIZED_LOOKUP")
}

/// Resolves pending and finalized transaction payloads from Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `requests`: ordered transaction hash requests with caller indexes.
///
/// Outputs:
/// - One lookup result per request, preserving request order.
/// - Missing hashes are represented with `found = false`, source `MISSING`,
///   and empty RLP bytes.
///
/// Invariants and edge behavior:
/// - Pending non-finalized payloads take precedence over finalized locations,
///   matching the legacy TransactionManager lookup order.
/// - Finalized regular transaction payloads are resolved through period data
///   position; finalized system payloads are resolved through the system
///   transaction column.
/// - Malformed location RLP or period-data RLP returns an error with a stable
///   TransactionManager context label.
pub fn load_stored_transactions(
    storage: &Storage,
    requests: Vec<StoredTransactionLookupRequest>,
) -> Result<Vec<StoredTransactionLookup>> {
    let transaction = storage.transaction();
    let mut out = Vec::with_capacity(requests.len());

    for request in requests {
        let (tx_rlp, source) = if let Some(tx_rlp) = transaction
            .rlp(request.hash)
            .context("TRANSACTION_MANAGER_STORED_LOOKUP_PENDING")?
        {
            (Some(tx_rlp), STORED_TRANSACTION_SOURCE_PENDING)
        } else if let Some(location_rlp) = transaction
            .location_rlp(request.hash)
            .context("TRANSACTION_MANAGER_STORED_LOOKUP_LOCATION")?
        {
            let location = rlp::Rlp::new(&location_rlp);
            let period = location
                .val_at::<u64>(0)
                .context("TRANSACTION_MANAGER_STORED_LOOKUP_LOCATION_PERIOD")?;
            let position = location
                .val_at::<u32>(1)
                .context("TRANSACTION_MANAGER_STORED_LOOKUP_LOCATION_POSITION")?;
            let is_system = location
                .item_count()
                .context("TRANSACTION_MANAGER_STORED_LOOKUP_LOCATION_SHAPE")?
                == 3
                && location
                    .val_at::<bool>(2)
                    .context("TRANSACTION_MANAGER_STORED_LOOKUP_LOCATION_SYSTEM_FLAG")?;

            let tx_rlp = if is_system {
                transaction
                    .system_rlp(request.hash)
                    .context("TRANSACTION_MANAGER_STORED_LOOKUP_SYSTEM")?
                    .map(|tx_rlp| (tx_rlp, STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM))
            } else {
                transaction
                    .by_period_position_rlp(period, position)
                    .context("TRANSACTION_MANAGER_STORED_LOOKUP_FINALIZED")?
                    .map(|tx_rlp| (tx_rlp, STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR))
            };

            match tx_rlp {
                Some((tx_rlp, source)) => (Some(tx_rlp), source),
                None => (None, STORED_TRANSACTION_SOURCE_MISSING),
            }
        } else {
            (None, STORED_TRANSACTION_SOURCE_MISSING)
        };

        out.push(StoredTransactionLookup {
            input_index: request.input_index,
            hash: request.hash,
            found: tx_rlp.is_some(),
            source,
            tx_rlp: tx_rlp.unwrap_or_default(),
        });
    }

    Ok(out)
}

/// Loads non-finalized restart rows and removes stale finalized duplicates.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
///
/// Outputs:
/// - Returns every non-finalized row as a hash/RLP pair with a finalized
///   classification.
/// - If stale rows are found, commits one Rust-owned batch deleting those
///   non-finalized transaction keys before returning.
///
/// Invariants and edge behavior:
/// - Storage iteration order is preserved in the returned entries.
/// - The finalized flag is derived from the finalized transaction index, not
///   from C++ sidecars.
/// - Malformed non-finalized keys and storage errors are reported without
///   partially rebuilding live TransactionManager sidecars.
pub fn load_non_finalized_recovery_entries(
    storage: &Storage,
) -> Result<Vec<NonFinalizedTransactionRecoveryEntry>> {
    let transaction = storage.transaction();
    let non_finalized = transaction
        .all_nonfinalized_with_hash()
        .context("TRANSACTION_MANAGER_NON_FINALIZED_RECOVERY_SCAN")?;
    let mut out = Vec::with_capacity(non_finalized.len());
    let mut stale_hashes = Vec::new();

    for (hash, tx_rlp) in non_finalized {
        let finalized = transaction
            .finalized(hash)
            .context("TRANSACTION_MANAGER_NON_FINALIZED_RECOVERY_FINALIZED_LOOKUP")?;
        if finalized {
            stale_hashes.push(hash);
        }

        out.push(NonFinalizedTransactionRecoveryEntry {
            hash,
            finalized,
            trx_rlp: tx_rlp,
        });
    }

    if !stale_hashes.is_empty() {
        let mut batch = storage.create_write_batch();
        for hash in stale_hashes {
            storage
                .batch_delete_raw(&mut batch, Column::Transactions, hash.as_bytes())
                .context("TRANSACTION_MANAGER_NON_FINALIZED_RECOVERY_STALE_DELETE")?;
        }
        storage
            .commit_write_batch_with_sync(batch, false)
            .context("TRANSACTION_MANAGER_NON_FINALIZED_RECOVERY_STALE_COMMIT")?;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::RlpStream;
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

    #[test]
    fn transaction_finalized_reads_finalized_location_index() {
        let storage = temp_storage("rustaxa_consensus_transaction_storage_finalized");
        let hash = H256::from([0x33; 32]);

        assert!(!transaction_finalized(&storage, hash).unwrap());

        storage
            .transaction()
            .write_location(hash, 12, 1, false)
            .unwrap();

        assert!(transaction_finalized(&storage, hash).unwrap());
    }

    #[test]
    fn load_stored_transactions_preserves_order_and_classifies_sources() {
        let storage = temp_storage("rustaxa_consensus_transaction_storage_lookup");

        storage
            .transaction()
            .write(H256::from([1u8; 32]), &[0x11])
            .unwrap();
        storage
            .transaction()
            .write_location(H256::from([2u8; 32]), 9, 0, false)
            .unwrap();
        storage
            .transaction()
            .write_system(H256::from([4u8; 32]), &[0x44])
            .unwrap();
        storage
            .transaction()
            .write_location(H256::from([4u8; 32]), 9, 0, true)
            .unwrap();

        let mut txs = RlpStream::new_list(1);
        txs.append_raw(&[0x22], 1);
        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);

        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::PeriodData,
                &9u64.to_le_bytes(),
                period_data.out().as_ref(),
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        let out = load_stored_transactions(
            &storage,
            vec![
                StoredTransactionLookupRequest {
                    input_index: 7,
                    hash: H256::from([2u8; 32]),
                },
                StoredTransactionLookupRequest {
                    input_index: 8,
                    hash: H256::from([3u8; 32]),
                },
                StoredTransactionLookupRequest {
                    input_index: 9,
                    hash: H256::from([1u8; 32]),
                },
                StoredTransactionLookupRequest {
                    input_index: 10,
                    hash: H256::from([4u8; 32]),
                },
            ],
        )
        .unwrap();

        assert_eq!(
            out,
            vec![
                StoredTransactionLookup {
                    input_index: 7,
                    hash: H256::from([2u8; 32]),
                    found: true,
                    source: STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR,
                    tx_rlp: vec![0x22],
                },
                StoredTransactionLookup {
                    input_index: 8,
                    hash: H256::from([3u8; 32]),
                    found: false,
                    source: STORED_TRANSACTION_SOURCE_MISSING,
                    tx_rlp: Vec::new(),
                },
                StoredTransactionLookup {
                    input_index: 9,
                    hash: H256::from([1u8; 32]),
                    found: true,
                    source: STORED_TRANSACTION_SOURCE_PENDING,
                    tx_rlp: vec![0x11],
                },
                StoredTransactionLookup {
                    input_index: 10,
                    hash: H256::from([4u8; 32]),
                    found: true,
                    source: STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM,
                    tx_rlp: vec![0x44],
                },
            ]
        );
    }

    #[test]
    fn load_non_finalized_recovery_entries_marks_and_removes_stale_finalized_rows() {
        let storage = temp_storage("rustaxa_consensus_transaction_storage_recovery");
        let live_hash = H256::from([0x11; 32]);
        let stale_hash = H256::from([0x22; 32]);

        storage.transaction().write(live_hash, &[0xC1]).unwrap();
        storage.transaction().write(stale_hash, &[0xC2]).unwrap();
        storage
            .transaction()
            .write_location(stale_hash, 7, 3, false)
            .unwrap();

        let mut entries = load_non_finalized_recovery_entries(&storage).unwrap();
        entries.sort_by_key(|entry| entry.hash);

        assert_eq!(
            entries,
            vec![
                NonFinalizedTransactionRecoveryEntry {
                    hash: live_hash,
                    finalized: false,
                    trx_rlp: vec![0xC1],
                },
                NonFinalizedTransactionRecoveryEntry {
                    hash: stale_hash,
                    finalized: true,
                    trx_rlp: vec![0xC2],
                },
            ]
        );
        assert_eq!(
            storage.transaction().rlp(live_hash).unwrap(),
            Some(vec![0xC1])
        );
        assert_eq!(storage.transaction().rlp(stale_hash).unwrap(), None);
    }
}
