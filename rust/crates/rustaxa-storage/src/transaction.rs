use anyhow::Result;
use ethereum_types::H256;
use std::sync::Arc;

use crate::Column;
use crate::StorageError;
use crate::db::{DbReader, DbWriter};

const TRANSACTIONS_POS_IN_PERIOD_DATA: usize = 3;

pub struct TransactionRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> TransactionRepository<D> {
    /// Creates a transaction repository over the shared database handle.
    pub fn new(db: Arc<D>) -> Self {
        TransactionRepository { db }
    }

    /// Returns true when transaction data exists in either pending transaction
    /// storage or the finalized transaction index.
    /// C++ mapping: `DbStorage::transactionInDb(trx_hash_t const&)`.
    pub fn exists(&self, trx_hash: H256) -> Result<bool> {
        // Check potentially non-finalized transactions first.
        if self.db.exist(Column::Transactions, trx_hash.as_bytes())? {
            return Ok(true);
        }

        // Check finalized transaction index.
        self.db.exist(Column::TrxPeriod, trx_hash.as_bytes())
    }

    /// Returns true when transaction hash is indexed as finalized.
    /// C++ mapping: `DbStorage::transactionFinalized(trx_hash_t const&)`.
    pub fn finalized(&self, trx_hash: H256) -> Result<bool> {
        self.db.exist(Column::TrxPeriod, trx_hash.as_bytes())
    }

    /// Returns serialized transaction location entry containing period, position,
    /// and optional system-transaction marker.
    /// C++ mapping: `DbStorage::getTransactionLocation(trx_hash_t const&) const`.
    pub fn location_rlp(&self, trx_hash: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::TrxPeriod, trx_hash.as_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Returns pending transaction payload by hash when it exists in the
    /// non-finalized transaction column.
    /// C++ mapping: `DbStorage::getTransaction(trx_hash_t const&) const` (pending branch).
    pub fn rlp(&self, trx_hash: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::Transactions, trx_hash.as_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Returns serialized system transaction payload by hash.
    /// C++ mapping: `DbStorage::getSystemTransaction(const trx_hash_t&) const`.
    pub fn system_rlp(&self, trx_hash: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::SystemTransaction, trx_hash.as_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Extracts one transaction payload from period data by transaction position.
    /// C++ mapping: `DbStorage::getTransaction(PbftPeriod, uint32_t) const`.
    pub fn by_period_position_rlp(&self, period: u64, position: u32) -> Result<Option<Vec<u8>>> {
        let Some(period_data) = self.db.get(Column::PeriodData, &period.to_le_bytes())? else {
            return Ok(None);
        };

        let period_data_rlp = rlp::Rlp::new(period_data.as_ref());
        let transactions = period_data_rlp.at(TRANSACTIONS_POS_IN_PERIOD_DATA)?;
        let trx = transactions.at(position as usize)?;
        Ok(Some(trx.as_raw().to_vec()))
    }

    /// Counts transactions embedded in period data for a finalized period.
    /// C++ mapping: `DbStorage::getTransactionCount(PbftPeriod) const`.
    pub fn count(&self, period: u64) -> Result<u64> {
        let Some(period_data) = self.db.get(Column::PeriodData, &period.to_le_bytes())? else {
            return Ok(0);
        };

        let period_data_rlp = rlp::Rlp::new(period_data.as_ref());
        let transactions = period_data_rlp.at(TRANSACTIONS_POS_IN_PERIOD_DATA)?;
        Ok(transactions.item_count()? as u64)
    }

    /// Returns all pending transaction payloads currently stored as non-finalized.
    /// C++ mapping: `DbStorage::getAllNonfinalizedTransactions()`.
    pub fn all_nonfinalized_rlp(&self) -> Result<Vec<Vec<u8>>> {
        let mut result = Vec::new();
        for entry in self.db.iter(Column::Transactions) {
            let (_, value) = entry?;
            result.push(value.into_vec());
        }
        Ok(result)
    }

    /// Returns all non-finalized transactions as persisted hash/RLP pairs.
    ///
    /// The hash is the exact `transactions` column key and the payload is the
    /// canonical RLP value stored under it. Invalid key lengths are reported as
    /// read errors because C++ recovery validates the key against the decoded
    /// transaction before rebuilding live sidecars.
    pub fn all_nonfinalized_with_hash(&self) -> Result<Vec<(H256, Vec<u8>)>> {
        let mut result = Vec::new();
        for entry in self.db.iter(Column::Transactions) {
            let (key, value) = entry?;
            if key.len() != 32 {
                return Err(StorageError::Read(format!(
                    "Invalid transaction hash size in non-finalized cache column: {}",
                    key.len()
                ))
                .into());
            }

            result.push((H256::from_slice(key.as_ref()), value.into_vec()));
        }
        Ok(result)
    }

    /// Returns every finalized transaction hash together with its finalized period.
    /// C++ mapping: `DbStorage::getAllTransactionPeriod()`.
    pub fn all_with_period(&self) -> Result<Vec<(H256, u64)>> {
        let mut result = Vec::new();
        for entry in self.db.iter(Column::TrxPeriod) {
            let (key, value) = entry?;
            if key.len() != 32 {
                return Err(StorageError::Read(format!(
                    "Invalid transaction hash size in trx_period: {}",
                    key.len()
                ))
                .into());
            }

            let period = rlp::Rlp::new(value.as_ref()).val_at(0)?;
            result.push((H256::from_slice(key.as_ref()), period));
        }
        Ok(result)
    }

    /// Returns serialized list of system transaction hashes for a period.
    /// C++ mapping: `DbStorage::getPeriodSystemTransactionsHashes(PbftPeriod) const`.
    pub fn period_system_hashes_rlp(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self
            .db
            .get(Column::PeriodSystemTransactions, &period.to_le_bytes())?
            .map(|value| value.as_ref().to_vec())
            .unwrap_or_default())
    }
}

impl<D: DbReader + DbWriter> TransactionRepository<D> {
    /// Stores a pending transaction payload by hash.
    /// C++ mapping: `DbStorage::addTransactionToBatch(Transaction const&, Batch&)`.
    pub fn write(&self, trx_hash: H256, trx_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::Transactions, trx_hash.as_bytes(), trx_rlp)
    }

    /// Appends a pending transaction payload write to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `trx_hash`: canonical transaction hash used as the pending transaction key.
    /// - `trx_rlp`: canonical transaction RLP payload stored without decoding.
    ///
    /// Outputs:
    /// - Appends a put in `transactions`.
    ///
    /// Invariants and edge behavior:
    /// - Existing pending payloads for the same hash are overwritten, matching
    ///   legacy RocksDB put semantics.
    pub fn write_in_batch(
        &self,
        batch: &mut D::Batch,
        trx_hash: H256,
        trx_rlp: &[u8],
    ) -> Result<()> {
        self.db
            .batch_put(batch, Column::Transactions, trx_hash.as_bytes(), trx_rlp)
    }

    /// Removes a pending transaction payload by hash.
    /// C++ mapping: `DbStorage::removeTransactionToBatch(trx_hash_t const&, Batch&)`.
    pub fn remove(&self, trx_hash: H256) -> Result<()> {
        self.db.delete(Column::Transactions, trx_hash.as_bytes())
    }

    /// Appends pending transaction payload removal to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `trx_hash`: canonical transaction hash used as the pending transaction key.
    ///
    /// Outputs:
    /// - Appends a delete in `transactions`.
    ///
    /// Invariants and edge behavior:
    /// - Missing keys are RocksDB delete no-ops, matching legacy storage
    ///   behavior.
    pub fn remove_in_batch(&self, batch: &mut D::Batch, trx_hash: H256) -> Result<()> {
        self.db
            .batch_delete(batch, Column::Transactions, trx_hash.as_bytes())
    }

    /// Stores finalized transaction location metadata (period, position, optional
    /// system marker) for a transaction hash.
    /// C++ mapping: `DbStorage::addTransactionLocationToBatch(Batch&, trx_hash_t const&, PbftPeriod, uint32_t, bool)`.
    pub fn write_location(
        &self,
        trx_hash: H256,
        period: u64,
        position: u32,
        is_system: bool,
    ) -> Result<()> {
        let mut stream = rlp::RlpStream::new_list(2 + usize::from(is_system));
        stream.append(&period);
        stream.append(&position);
        if is_system {
            stream.append(&is_system);
        }

        self.db.put(
            Column::TrxPeriod,
            trx_hash.as_bytes(),
            stream.out().as_ref(),
        )
    }

    /// Appends finalized transaction location metadata to a caller-owned batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `trx_hash`: canonical transaction hash used as the location key.
    /// - `period`: finalized PBFT period containing the transaction.
    /// - `position`: transaction position inside the finalized period payload.
    /// - `is_system`: when true, appends the legacy third RLP item that marks a
    ///   system transaction.
    ///
    /// Outputs:
    /// - Appends a put in `trx_period`.
    ///
    /// Invariants and edge behavior:
    /// - The value is encoded as legacy RLP `[period, position]` or
    ///   `[period, position, true]`, matching C++ `TransactionLocation`.
    pub fn write_location_in_batch(
        &self,
        batch: &mut D::Batch,
        trx_hash: H256,
        period: u64,
        position: u32,
        is_system: bool,
    ) -> Result<()> {
        let mut stream = rlp::RlpStream::new_list(2 + usize::from(is_system));
        stream.append(&period);
        stream.append(&position);
        if is_system {
            stream.append(&is_system);
        }

        self.db.batch_put(
            batch,
            Column::TrxPeriod,
            trx_hash.as_bytes(),
            stream.out().as_ref(),
        )
    }

    /// Stores a system transaction payload by hash.
    /// C++ mapping: `DbStorage::addSystemTransactionToBatch(Batch&, SharedTransaction)`.
    pub fn write_system(&self, trx_hash: H256, trx_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::SystemTransaction, trx_hash.as_bytes(), trx_rlp)
    }

    /// Appends a system transaction payload write to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `trx_hash`: canonical system transaction hash.
    /// - `trx_rlp`: canonical system transaction RLP payload stored without decoding.
    ///
    /// Outputs:
    /// - Appends a put in `system_transaction`.
    pub fn write_system_in_batch(
        &self,
        batch: &mut D::Batch,
        trx_hash: H256,
        trx_rlp: &[u8],
    ) -> Result<()> {
        self.db.batch_put(
            batch,
            Column::SystemTransaction,
            trx_hash.as_bytes(),
            trx_rlp,
        )
    }

    /// Stores serialized system transaction hash list for a finalized period.
    /// C++ mapping: `DbStorage::addPeriodSystemTransactions(Batch&, SharedTransactions, PbftPeriod)`.
    pub fn write_period_system_hashes(&self, period: u64, hashes_rlp: &[u8]) -> Result<()> {
        self.db.put(
            Column::PeriodSystemTransactions,
            &period.to_le_bytes(),
            hashes_rlp,
        )
    }

    /// Appends serialized system transaction hash list to a caller-owned batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `period`: finalized PBFT period used as the hash-list key.
    /// - `hashes_rlp`: legacy RLP list of system transaction hashes.
    ///
    /// Outputs:
    /// - Appends a put in `period_system_transactions`.
    ///
    /// Invariants and edge behavior:
    /// - The hash list remains opaque bytes at this boundary because C++ still
    ///   materializes the system transaction objects.
    pub fn write_period_system_hashes_in_batch(
        &self,
        batch: &mut D::Batch,
        period: u64,
        hashes_rlp: &[u8],
    ) -> Result<()> {
        self.db.batch_put(
            batch,
            Column::PeriodSystemTransactions,
            &period.to_le_bytes(),
            hashes_rlp,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DbIterator, DbWriter};
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    struct MockTransactionStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
    }

    enum MockBatchOp {
        Put(Column, Vec<u8>, Vec<u8>),
        Delete(Column, Vec<u8>),
    }

    impl MockTransactionStore {
        fn new() -> Self {
            MockTransactionStore {
                data: RwLock::new(HashMap::new()),
            }
        }

        fn put(&self, col: Column, key: &[u8], value: &[u8]) {
            let mut data = self.data.write().unwrap();
            let cf = data
                .entry(col.name().to_string())
                .or_insert_with(BTreeMap::new);
            cf.insert(key.to_vec(), value.to_vec());
        }

        fn delete(&self, col: Column, key: &[u8]) {
            let mut data = self.data.write().unwrap();
            if let Some(cf) = data.get_mut(col.name()) {
                cf.remove(key);
            }
        }
    }

    impl DbReader for MockTransactionStore {
        type Slice<'a> = Vec<u8>;

        fn exist(&self, col: Column, key: &[u8]) -> Result<bool> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                Ok(cf.contains_key(key))
            } else {
                Ok(false)
            }
        }

        fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                Ok(cf.get(key).cloned())
            } else {
                Ok(None)
            }
        }

        fn get_at_or_before(
            &self,
            col: Column,
            key: &[u8],
        ) -> Result<Option<(Box<[u8]>, Box<[u8]>)>> {
            let data = self.data.read().unwrap();
            let Some(cf) = data.get(col.name()) else {
                return Ok(None);
            };
            let key = key.to_vec();
            Ok(cf
                .range(..=key)
                .next_back()
                .map(|(k, v)| (k.clone().into_boxed_slice(), v.clone().into_boxed_slice())))
        }

        fn get_at_or_after(
            &self,
            col: Column,
            key: &[u8],
        ) -> Result<Option<(Box<[u8]>, Box<[u8]>)>> {
            let data = self.data.read().unwrap();
            let Some(cf) = data.get(col.name()) else {
                return Ok(None);
            };
            let key = key.to_vec();
            Ok(cf
                .range(key..)
                .next()
                .map(|(k, v)| (k.clone().into_boxed_slice(), v.clone().into_boxed_slice())))
        }

        fn iter<'a>(&'a self, col: Column) -> DbIterator<'a> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                let items: Vec<_> = cf
                    .iter()
                    .map(|(k, v)| Ok((k.clone().into_boxed_slice(), v.clone().into_boxed_slice())))
                    .collect();
                Box::new(items.into_iter())
            } else {
                Box::new(std::iter::empty())
            }
        }

        fn iter_rev<'a>(&'a self, col: Column) -> DbIterator<'a> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                let items: Vec<_> = cf
                    .iter()
                    .rev()
                    .map(|(k, v)| Ok((k.clone().into_boxed_slice(), v.clone().into_boxed_slice())))
                    .collect();
                Box::new(items.into_iter())
            } else {
                Box::new(std::iter::empty())
            }
        }
    }

    impl DbWriter for MockTransactionStore {
        type Batch = Vec<MockBatchOp>;

        fn create_batch(&self) -> Self::Batch {
            Vec::new()
        }

        fn batch_put(
            &self,
            batch: &mut Self::Batch,
            col: Column,
            key: &[u8],
            value: &[u8],
        ) -> Result<()> {
            batch.push(MockBatchOp::Put(col, key.to_vec(), value.to_vec()));
            Ok(())
        }

        fn batch_delete(&self, batch: &mut Self::Batch, col: Column, key: &[u8]) -> Result<()> {
            batch.push(MockBatchOp::Delete(col, key.to_vec()));
            Ok(())
        }

        fn commit_batch(&self, batch: Self::Batch) -> Result<()> {
            for op in batch {
                match op {
                    MockBatchOp::Put(col, key, value) => self.put(col, &key, &value),
                    MockBatchOp::Delete(col, key) => self.delete(col, &key),
                }
            }
            Ok(())
        }

        fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()> {
            MockTransactionStore::put(self, col, key, value);
            Ok(())
        }

        fn delete(&self, col: Column, key: &[u8]) -> Result<()> {
            MockTransactionStore::delete(self, col, key);
            Ok(())
        }
    }

    #[test]
    fn test_transaction_in_db_from_pending() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());

        let hash = H256::from_low_u64_be(1);
        db.put(Column::Transactions, hash.as_bytes(), &[0xAA]);

        assert!(repo.exists(hash).unwrap());
    }

    #[test]
    fn test_transaction_in_db_from_finalized() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());

        let hash = H256::from_low_u64_be(2);
        db.put(Column::TrxPeriod, hash.as_bytes(), &[0xC2, 0x01, 0x02]);

        assert!(repo.exists(hash).unwrap());
    }

    #[test]
    fn test_transaction_in_db_missing() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db);

        assert!(!repo.exists(H256::from_low_u64_be(3)).unwrap());
    }

    #[test]
    fn test_transaction_finalized() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let hash = H256::from_low_u64_be(4);

        assert!(!repo.finalized(hash).unwrap());
        db.put(Column::TrxPeriod, hash.as_bytes(), &[0xC2, 0x01, 0x02]);
        assert!(repo.finalized(hash).unwrap());
    }

    #[test]
    fn test_transaction_location_rlp() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let hash = H256::from_low_u64_be(5);
        let location = vec![0xC2, 0x01, 0x02];

        assert!(repo.location_rlp(hash).unwrap().is_none());
        db.put(Column::TrxPeriod, hash.as_bytes(), &location);
        assert_eq!(repo.location_rlp(hash).unwrap(), Some(location));
    }

    #[test]
    fn test_transaction_rlp() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let hash = H256::from_low_u64_be(6);
        let tx = vec![0xC1, 0x11];

        assert!(repo.rlp(hash).unwrap().is_none());
        db.put(Column::Transactions, hash.as_bytes(), &tx);
        assert_eq!(repo.rlp(hash).unwrap(), Some(tx));
    }

    #[test]
    fn test_system_transaction_rlp() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let hash = H256::from_low_u64_be(7);
        let tx = vec![0xC1, 0x22];

        assert!(repo.system_rlp(hash).unwrap().is_none());
        db.put(Column::SystemTransaction, hash.as_bytes(), &tx);
        assert_eq!(repo.system_rlp(hash).unwrap(), Some(tx));
    }

    #[test]
    fn test_transaction_by_period_position_and_count() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());

        let period = 42u64;
        let tx0 = vec![0xC1, 0xA0];
        let tx1 = vec![0xC1, 0xA1];

        let mut txs = rlp::RlpStream::new_list(2);
        txs.append_raw(&tx0, 1);
        txs.append_raw(&tx1, 1);

        let mut period_data = rlp::RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        db.put(
            Column::PeriodData,
            &period.to_le_bytes(),
            period_data.out().as_ref(),
        );

        assert_eq!(repo.by_period_position_rlp(period, 0).unwrap(), Some(tx0));
        assert_eq!(repo.by_period_position_rlp(period, 1).unwrap(), Some(tx1));
        assert_eq!(repo.count(period).unwrap(), 2);

        assert!(repo.by_period_position_rlp(period, 2).is_err());
        assert_eq!(repo.count(999).unwrap(), 0);
    }

    #[test]
    fn test_all_nonfinalized_transactions_rlp() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());

        db.put(
            Column::Transactions,
            H256::from_low_u64_be(8).as_bytes(),
            &[0xAA, 0xBB],
        );
        db.put(
            Column::Transactions,
            H256::from_low_u64_be(9).as_bytes(),
            &[0xCC],
        );

        let mut result = repo.all_nonfinalized_rlp().unwrap();
        result.sort();
        assert_eq!(result, vec![vec![0xAA, 0xBB], vec![0xCC]]);
    }

    #[test]
    fn test_all_nonfinalized_transactions_with_hash() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let hash1 = H256::from_low_u64_be(8);
        let hash2 = H256::from_low_u64_be(9);

        db.put(Column::Transactions, hash1.as_bytes(), &[0xAA, 0xBB]);
        db.put(Column::Transactions, hash2.as_bytes(), &[0xCC]);

        let result = repo.all_nonfinalized_with_hash().unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], (hash1, vec![0xAA, 0xBB]));
        assert_eq!(result[1], (hash2, vec![0xCC]));
    }

    #[test]
    fn test_all_transaction_period() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());

        let hash1 = H256::from_low_u64_be(9);
        let hash2 = H256::from_low_u64_be(10);
        db.put(Column::TrxPeriod, hash1.as_bytes(), &[0xC2, 0x01, 0x05]); // [1, 5]
        db.put(Column::TrxPeriod, hash2.as_bytes(), &[0xC2, 0x02, 0x06]); // [2, 6]

        let result = repo.all_with_period().unwrap();
        assert!(result.contains(&(hash1, 1)));
        assert!(result.contains(&(hash2, 2)));
    }

    #[test]
    fn test_period_system_transactions_hashes_rlp() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let period = 42u64;
        let hashes_rlp = vec![0xC1, 0x01];

        assert!(repo.period_system_hashes_rlp(period).unwrap().is_empty());

        db.put(
            Column::PeriodSystemTransactions,
            &period.to_le_bytes(),
            &hashes_rlp,
        );
        assert_eq!(repo.period_system_hashes_rlp(period).unwrap(), hashes_rlp);
    }

    #[test]
    fn test_transaction_batch_writes_wait_for_commit() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let pending_hash = H256::from_low_u64_be(21);
        let removed_hash = H256::from_low_u64_be(22);
        let location_hash = H256::from_low_u64_be(23);
        let system_hash = H256::from_low_u64_be(24);
        let period = 43u64;
        let system_hashes_rlp = vec![0xE1, 0x80];

        db.put(Column::Transactions, removed_hash.as_bytes(), &[0xC1, 0xDD]);

        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.write_in_batch(&mut batch, pending_hash, &[0xC1, 0xAA])
            .unwrap();
        repo.remove_in_batch(&mut batch, removed_hash).unwrap();
        repo.write_location_in_batch(&mut batch, location_hash, period, 7, true)
            .unwrap();
        repo.write_system_in_batch(&mut batch, system_hash, &[0xC1, 0xBB])
            .unwrap();
        repo.write_period_system_hashes_in_batch(&mut batch, period, &system_hashes_rlp)
            .unwrap();

        assert_eq!(repo.rlp(pending_hash).unwrap(), None);
        assert_eq!(repo.rlp(removed_hash).unwrap(), Some(vec![0xC1, 0xDD]));
        assert_eq!(repo.location_rlp(location_hash).unwrap(), None);
        assert_eq!(repo.system_rlp(system_hash).unwrap(), None);
        assert!(repo.period_system_hashes_rlp(period).unwrap().is_empty());

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();

        assert_eq!(repo.rlp(pending_hash).unwrap(), Some(vec![0xC1, 0xAA]));
        assert_eq!(repo.rlp(removed_hash).unwrap(), None);
        assert!(repo.finalized(location_hash).unwrap());
        assert_eq!(
            repo.system_rlp(system_hash).unwrap(),
            Some(vec![0xC1, 0xBB])
        );
        assert_eq!(
            repo.period_system_hashes_rlp(period).unwrap(),
            system_hashes_rlp
        );
    }
}
