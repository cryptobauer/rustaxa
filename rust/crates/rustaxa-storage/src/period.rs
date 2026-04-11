use anyhow::Result;
use ethereum_types::H256;
use std::sync::Arc;

use crate::db::DbReader;
use crate::{Column, StorageError};

pub struct PeriodRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> PeriodRepository<D> {
    pub fn new(db: Arc<D>) -> Self {
        PeriodRepository { db }
    }

    /// Implements getPeriodDataRaw(period) -> bytes
    pub fn period_data_raw(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self
            .db
            .get(Column::PeriodData, &period.to_le_bytes())?
            .map(|value| value.as_ref().to_vec())
            .unwrap_or_default())
    }

    /// Implements getPeriodFromPbftHash(hash) -> optional(period)
    pub fn period_from_pbft_hash(&self, pbft_hash: H256) -> Result<Option<u64>> {
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

    /// Implements getBlockReceipt(period) -> rlp(receipt)
    pub fn block_receipt(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self
            .db
            .get(Column::FinalChainReceiptByPeriod, &period.to_le_bytes())?
            .map(|value| value.as_ref().to_vec())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbIterator;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    struct MockPeriodStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
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

    #[test]
    fn test_period_data_raw_found() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db.clone());

        let period = 77u64;
        let expected = vec![0xC1, 0xAA, 0xBB];
        db.put(Column::PeriodData, &period.to_le_bytes(), &expected);

        let result = repo.period_data_raw(period).unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_period_data_raw_missing() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db);

        let result = repo.period_data_raw(11).unwrap();
        assert!(result.is_empty());
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

        let result = repo.block_receipt(period).unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_block_receipt_missing() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db);

        let result = repo.block_receipt(12).unwrap();
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

        let result = repo.period_from_pbft_hash(hash).unwrap();
        assert_eq!(result, Some(period));
    }

    #[test]
    fn test_period_from_pbft_hash_missing() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db);

        let result = repo
            .period_from_pbft_hash(H256::from_low_u64_be(1))
            .unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_period_from_pbft_hash_invalid_length() {
        let db = Arc::new(MockPeriodStore::new());
        let repo = PeriodRepository::new(db.clone());

        let hash = H256::from_low_u64_be(2);
        db.put(Column::PbftBlockPeriod, hash.as_bytes(), &[1, 2, 3]);

        let err = repo.period_from_pbft_hash(hash).unwrap_err().to_string();
        assert!(err.contains("Invalid pbft_block_period value size"));
    }
}
