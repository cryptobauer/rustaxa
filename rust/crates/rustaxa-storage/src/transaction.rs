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

    /// Implements getTransaction(hash) pending-transaction branch -> optional(rlp(tx))
    pub fn transaction_rlp(&self, trx_hash: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::Transactions, trx_hash.as_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Implements getSystemTransaction(hash) -> optional(rlp(tx))
    pub fn system_transaction_rlp(&self, trx_hash: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::SystemTransaction, trx_hash.as_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Implements getTransaction(period, position) -> optional(rlp(tx))
    pub fn transaction_by_period_position_rlp(
        &self,
        period: u64,
        position: u32,
    ) -> Result<Option<Vec<u8>>> {
        let Some(period_data) = self.db.get(Column::PeriodData, &period.to_le_bytes())? else {
            return Ok(None);
        };

        let period_data_rlp = rlp::Rlp::new(period_data.as_ref());
        let transactions = period_data_rlp.at(TRANSACTIONS_POS_IN_PERIOD_DATA)?;
        let trx = transactions.at(position as usize)?;
        Ok(Some(trx.as_raw().to_vec()))
    }

    /// Implements getTransactionCount(period) -> count
    pub fn transaction_count(&self, period: u64) -> Result<u64> {
        let Some(period_data) = self.db.get(Column::PeriodData, &period.to_le_bytes())? else {
            return Ok(0);
        };

        let period_data_rlp = rlp::Rlp::new(period_data.as_ref());
        let transactions = period_data_rlp.at(TRANSACTIONS_POS_IN_PERIOD_DATA)?;
        Ok(transactions.item_count()? as u64)
    }

    /// Implements getAllNonfinalizedTransactions() -> [transaction_rlp]
    pub fn all_nonfinalized_transactions_rlp(&self) -> Result<Vec<Vec<u8>>> {
        let mut result = Vec::new();
        for entry in self.db.iter(Column::Transactions) {
            let (_, value) = entry?;
            result.push(value.into_vec());
        }
        Ok(result)
    }

    /// Implements getAllTransactionPeriod() -> [(trx_hash, period)]
    pub fn all_transaction_period(&self) -> Result<Vec<(H256, u64)>> {
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

    /// Implements getPeriodSystemTransactionsHashes(period) -> rlp([trx_hash])
    pub fn period_system_transactions_hashes_rlp(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self
            .db
            .get(Column::PeriodSystemTransactions, &period.to_le_bytes())?
            .map(|value| value.as_ref().to_vec())
            .unwrap_or_default())
    }
}

impl<D: DbReader + DbWriter> TransactionRepository<D> {
    /// Implements addTransactionToBatch(trx, ...)
    pub fn save_transaction(&self, trx_hash: H256, trx_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::Transactions, trx_hash.as_bytes(), trx_rlp)
    }

    /// Implements removeTransactionToBatch(hash, ...)
    pub fn remove_transaction(&self, trx_hash: H256) -> Result<()> {
        self.db.delete(Column::Transactions, trx_hash.as_bytes())
    }

    /// Implements addTransactionLocationToBatch(..., hash, period, position, is_system)
    pub fn save_transaction_location(
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

    /// Implements addSystemTransactionToBatch(..., trx)
    pub fn save_system_transaction(&self, trx_hash: H256, trx_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::SystemTransaction, trx_hash.as_bytes(), trx_rlp)
    }

    /// Implements addPeriodSystemTransactions(..., trxs, period)
    pub fn save_period_system_transactions_hashes(
        &self,
        period: u64,
        hashes_rlp: &[u8],
    ) -> Result<()> {
        self.db.put(
            Column::PeriodSystemTransactions,
            &period.to_le_bytes(),
            hashes_rlp,
        )
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

    #[test]
    fn test_transaction_rlp() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let hash = H256::from_low_u64_be(6);
        let tx = vec![0xC1, 0x11];

        assert!(repo.transaction_rlp(hash).unwrap().is_none());
        db.put(Column::Transactions, hash.as_bytes(), &tx);
        assert_eq!(repo.transaction_rlp(hash).unwrap(), Some(tx));
    }

    #[test]
    fn test_system_transaction_rlp() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let hash = H256::from_low_u64_be(7);
        let tx = vec![0xC1, 0x22];

        assert!(repo.system_transaction_rlp(hash).unwrap().is_none());
        db.put(Column::SystemTransaction, hash.as_bytes(), &tx);
        assert_eq!(repo.system_transaction_rlp(hash).unwrap(), Some(tx));
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

        assert_eq!(
            repo.transaction_by_period_position_rlp(period, 0).unwrap(),
            Some(tx0)
        );
        assert_eq!(
            repo.transaction_by_period_position_rlp(period, 1).unwrap(),
            Some(tx1)
        );
        assert_eq!(repo.transaction_count(period).unwrap(), 2);

        assert!(repo.transaction_by_period_position_rlp(period, 2).is_err());
        assert_eq!(repo.transaction_count(999).unwrap(), 0);
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

        let mut result = repo.all_nonfinalized_transactions_rlp().unwrap();
        result.sort();
        assert_eq!(result, vec![vec![0xAA, 0xBB], vec![0xCC]]);
    }

    #[test]
    fn test_all_transaction_period() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());

        let hash1 = H256::from_low_u64_be(9);
        let hash2 = H256::from_low_u64_be(10);
        db.put(Column::TrxPeriod, hash1.as_bytes(), &[0xC2, 0x01, 0x05]); // [1, 5]
        db.put(Column::TrxPeriod, hash2.as_bytes(), &[0xC2, 0x02, 0x06]); // [2, 6]

        let result = repo.all_transaction_period().unwrap();
        assert!(result.contains(&(hash1, 1)));
        assert!(result.contains(&(hash2, 2)));
    }

    #[test]
    fn test_period_system_transactions_hashes_rlp() {
        let db = Arc::new(MockTransactionStore::new());
        let repo = TransactionRepository::new(db.clone());
        let period = 42u64;
        let hashes_rlp = vec![0xC1, 0x01];

        assert!(
            repo.period_system_transactions_hashes_rlp(period)
                .unwrap()
                .is_empty()
        );

        db.put(
            Column::PeriodSystemTransactions,
            &period.to_le_bytes(),
            &hashes_rlp,
        );
        assert_eq!(
            repo.period_system_transactions_hashes_rlp(period).unwrap(),
            hashes_rlp
        );
    }
}
