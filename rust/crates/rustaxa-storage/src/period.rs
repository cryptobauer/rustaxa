use anyhow::Result;
use ethereum_types::H256;
use std::sync::Arc;

use crate::db::{DbReader, DbWriter};
use crate::{Column, StorageError};

pub struct PeriodRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> PeriodRepository<D> {
    /// Creates a period repository over the shared database handle.
    pub fn new(db: Arc<D>) -> Self {
        PeriodRepository { db }
    }

    /// Returns serialized period payload bytes, or empty bytes when missing.
    /// C++ mapping: `DbStorage::getPeriodDataRaw(PbftPeriod) const`.
    pub fn data_raw(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self
            .db
            .get(Column::PeriodData, &period.to_le_bytes())?
            .map(|value| value.as_ref().to_vec())
            .unwrap_or_default())
    }

    /// Resolves finalized period number for a PBFT block hash.
    /// C++ mapping: `DbStorage::getPeriodFromPbftHash(taraxa::blk_hash_t const&)`.
    pub fn by_pbft_hash(&self, pbft_hash: H256) -> Result<Option<u64>> {
        match self.db.get(Column::PbftBlockPeriod, pbft_hash.as_bytes())? {
            Some(value) => {
                let value = value.as_ref();
                if value.len() != std::mem::size_of::<u64>() {
                    return Err(StorageError::Read(format!(
                        "Invalid pbft_block_period value size: expected {}, got {}",
                        std::mem::size_of::<u64>(),
                        value.len()
                    ))
                    .into());
                }

                let mut period_bytes = [0u8; 8];
                period_bytes.copy_from_slice(value);
                Ok(Some(u64::from_le_bytes(period_bytes)))
            }
            None => Ok(None),
        }
    }

    /// Returns serialized final-chain receipts for a period, or empty bytes
    /// when no receipts are stored.
    /// C++ mapping: `DbStorage::getBlockReceipts(PbftPeriod) const`.
    pub fn receipt(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self
            .db
            .get(Column::FinalChainReceiptByPeriod, &period.to_le_bytes())?
            .map(|value| value.as_ref().to_vec())
            .unwrap_or_default())
    }
}

impl<D: DbReader + DbWriter> PeriodRepository<D> {
    /// Stores serialized period payload keyed by period number.
    /// C++ mapping: `DbStorage::savePeriodData(const PeriodData&, Batch&)`.
    pub fn write(&self, period: u64, period_data_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::PeriodData, &period.to_le_bytes(), period_data_rlp)
    }

    /// Appends serialized period payload bytes to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `period`: finalized PBFT period used as the period-data key.
    /// - `period_data_rlp`: legacy-compatible serialized `PeriodData` payload.
    ///
    /// Outputs:
    /// - Appends a put in `period_data`.
    ///
    /// Invariants and edge behavior:
    /// - Existing period payloads for the same period are overwritten, matching
    ///   legacy RocksDB put semantics.
    /// - The payload is intentionally opaque here; consensus period-data
    ///   compatibility owns the encoded object shape until it is fully
    ///   Rust-owned.
    pub fn write_in_batch(
        &self,
        batch: &mut D::Batch,
        period: u64,
        period_data_rlp: &[u8],
    ) -> Result<()> {
        self.db.batch_put(
            batch,
            Column::PeriodData,
            &period.to_le_bytes(),
            period_data_rlp,
        )
    }

    /// Stores finalized PBFT hash-to-period index entry.
    /// C++ mapping: `DbStorage::addPbftBlockPeriodToBatch(PbftPeriod, taraxa::blk_hash_t const&, Batch&)`.
    pub fn write_pbft_period(&self, pbft_block_hash: H256, period: u64) -> Result<()> {
        self.db.put(
            Column::PbftBlockPeriod,
            pbft_block_hash.as_bytes(),
            &period.to_le_bytes(),
        )
    }

    /// Appends a finalized PBFT hash-to-period index entry to a caller-owned batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `pbft_block_hash`: canonical PBFT block hash used as the index key.
    /// - `period`: finalized PBFT period encoded as little-endian `uint64_t`.
    ///
    /// Outputs:
    /// - Appends a put in `pbft_block_period`.
    ///
    /// Invariants and edge behavior:
    /// - Existing entries for the same hash are overwritten, matching legacy
    ///   RocksDB put semantics.
    pub fn write_pbft_period_in_batch(
        &self,
        batch: &mut D::Batch,
        pbft_block_hash: H256,
        period: u64,
    ) -> Result<()> {
        self.db.batch_put(
            batch,
            Column::PbftBlockPeriod,
            pbft_block_hash.as_bytes(),
            &period.to_le_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DbIterator, DbWriter};
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    struct MockPeriodStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
    }

    enum MockBatchOp {
        Put(Column, Vec<u8>, Vec<u8>),
        Delete(Column, Vec<u8>),
    }

    impl MockPeriodStore {
        fn new() -> Self {
            MockPeriodStore {
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

    impl DbReader for MockPeriodStore {
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

    impl DbWriter for MockPeriodStore {
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
            MockPeriodStore::put(self, col, key, value);
            Ok(())
        }

        fn delete(&self, col: Column, key: &[u8]) -> Result<()> {
            MockPeriodStore::delete(self, col, key);
            Ok(())
        }
    }

    #[test]
    fn test_period_data_raw_found() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db.clone());

        let period = 77u64;
        let expected = vec![0xC1, 0xAA, 0xBB];
        db.put(Column::PeriodData, &period.to_le_bytes(), &expected);

        let result = repo.data_raw(period).unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_period_data_raw_missing() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db);

        let result = repo.data_raw(11).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_period_data_batch_write_waits_for_commit() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db.clone());
        let period = 78u64;

        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.write_in_batch(&mut batch, period, &[0xC1, 0xCC])
            .unwrap();

        assert!(repo.data_raw(period).unwrap().is_empty());

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();

        assert_eq!(repo.data_raw(period).unwrap(), vec![0xC1, 0xCC]);
    }

    #[test]
    fn test_block_receipt_found() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db.clone());

        let period = 88u64;
        let expected = vec![0xC2, 0xAA, 0xBB];
        db.put(
            Column::FinalChainReceiptByPeriod,
            &period.to_le_bytes(),
            &expected,
        );

        let result = repo.receipt(period).unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_block_receipt_missing() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db);

        let result = repo.receipt(12).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_mock_period_store_exist() {
        let db = MockPeriodStore::new();
        let hash = H256::from_low_u64_be(3);

        assert!(!db.exist(Column::PeriodData, hash.as_bytes()).unwrap());

        db.put(Column::PeriodData, hash.as_bytes(), &[0xAA]);

        assert!(db.exist(Column::PeriodData, hash.as_bytes()).unwrap());
        assert!(!db.exist(Column::PbftBlockPeriod, hash.as_bytes()).unwrap());
    }

    #[test]
    fn test_period_from_pbft_hash_found() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db.clone());

        let hash = H256::from_low_u64_be(42);
        let period = 1234u64;
        db.put(
            Column::PbftBlockPeriod,
            hash.as_bytes(),
            &period.to_le_bytes(),
        );

        let result = repo.by_pbft_hash(hash).unwrap();
        assert_eq!(result, Some(period));
    }

    #[test]
    fn test_period_from_pbft_hash_missing() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db);

        let result = repo.by_pbft_hash(H256::from_low_u64_be(1)).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_pbft_period_batch_write_waits_for_commit() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db.clone());
        let hash = H256::from_low_u64_be(43);

        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.write_pbft_period_in_batch(&mut batch, hash, 1235)
            .unwrap();

        assert_eq!(repo.by_pbft_hash(hash).unwrap(), None);

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();

        assert_eq!(repo.by_pbft_hash(hash).unwrap(), Some(1235));
    }

    #[test]
    fn test_period_from_pbft_hash_invalid_length() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db.clone());

        let hash = H256::from_low_u64_be(2);
        db.put(Column::PbftBlockPeriod, hash.as_bytes(), &[1, 2, 3]);

        let err = repo.by_pbft_hash(hash).unwrap_err().to_string();
        assert!(err.contains("Invalid pbft_block_period value size"));
    }
}
