use anyhow::Result;
use ethereum_types::H256;
use std::sync::Arc;

use crate::Column;
use crate::db::DbReader;

pub struct TransactionRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> TransactionRepository<D> {
    pub fn new(db: Arc<D>) -> Self {
        TransactionRepository { db }
    }

    /// Implements transactionInDb(hash) -> bool
    pub fn transaction_in_db(&self, trx_hash: H256) -> Result<bool> {
        // Check potentially non-finalized transactions first.
        if self.db.exist(Column::Transactions, trx_hash.as_bytes())? {
            return Ok(true);
        }

        // Check finalized transaction index.
        self.db.exist(Column::TrxPeriod, trx_hash.as_bytes())
    }

    /// Implements transactionFinalized(hash) -> bool
    pub fn transaction_finalized(&self, trx_hash: H256) -> Result<bool> {
        self.db.exist(Column::TrxPeriod, trx_hash.as_bytes())
    }

    /// Implements getTransactionLocation(hash) -> optional(rlp(period, position, is_system?))
    pub fn transaction_location_rlp(&self, trx_hash: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::TrxPeriod, trx_hash.as_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbIterator;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    struct MockTransactionStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
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
    fn test_transaction_in_db_from_pending() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());

        let hash = H256::from_low_u64_be(1);
        db.put(Column::Transactions, hash.as_bytes(), &[0xAA]);

        assert!(repo.transaction_in_db(hash).unwrap());
    }

    #[test]
    fn test_transaction_in_db_from_finalized() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());

        let hash = H256::from_low_u64_be(2);
        db.put(Column::TrxPeriod, hash.as_bytes(), &[0xC2, 0x01, 0x02]);

        assert!(repo.transaction_in_db(hash).unwrap());
    }

    #[test]
    fn test_transaction_in_db_missing() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db);

        assert!(!repo.transaction_in_db(H256::from_low_u64_be(3)).unwrap());
    }

    #[test]
    fn test_transaction_finalized() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let hash = H256::from_low_u64_be(4);

        assert!(!repo.transaction_finalized(hash).unwrap());
        db.put(Column::TrxPeriod, hash.as_bytes(), &[0xC2, 0x01, 0x02]);
        assert!(repo.transaction_finalized(hash).unwrap());
    }

    #[test]
    fn test_transaction_location_rlp() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let hash = H256::from_low_u64_be(5);
        let location = vec![0xC2, 0x01, 0x02];

        assert!(repo.transaction_location_rlp(hash).unwrap().is_none());
        db.put(Column::TrxPeriod, hash.as_bytes(), &location);
        assert_eq!(repo.transaction_location_rlp(hash).unwrap(), Some(location));
    }
}
