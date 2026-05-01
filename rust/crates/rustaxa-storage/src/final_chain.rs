use anyhow::Result;
use ethereum_types::H256;
use std::sync::Arc;

use crate::Column;
use crate::db::DbReader;

pub struct FinalChainRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> FinalChainRepository<D> {
    /// Creates a final-chain repository over the shared database handle.
    pub fn new(db: Arc<D>) -> Self {
        FinalChainRepository { db }
    }

    /// Returns raw final-chain metadata payload by metadata key.
    /// C++ mapping: `DbStorage::lookup(..., Columns::final_chain_meta)`.
    pub fn meta_value(&self, key: u32) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::FinalChainMeta, &key.to_le_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Returns serialized final-chain block header payload by block number.
    /// C++ mapping: `DbStorage::lookup(..., Columns::final_chain_blk_by_number)`.
    pub fn block_header_raw(&self, number: u64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::FinalChainBlkByNumber, &number.to_le_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Returns block hash bytes for a finalized block number.
    /// C++ mapping: `DbStorage::lookup(..., Columns::final_chain_blk_hash_by_number)`.
    pub fn block_hash_by_number(&self, number: u64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::FinalChainBlkHashByNumber, &number.to_le_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Returns block number bytes for a finalized block hash.
    /// C++ mapping: `DbStorage::lookup(..., Columns::final_chain_blk_number_by_hash)`.
    pub fn block_number_by_hash(&self, hash: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::FinalChainBlkNumberByHash, hash.as_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Returns serialized bloom chunk payload by bloom chunk identifier.
    /// C++ mapping: `DbStorage::lookup(..., Columns::final_chain_log_blooms_index)`.
    pub fn log_blooms_chunk_raw(&self, chunk_id: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::FinalChainLogBloomsIndex, chunk_id.as_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Returns serialized transaction receipt payload by transaction hash.
    /// C++ mapping: `DbStorage::lookup(..., Columns::final_chain_receipt_by_trx_hash)`.
    pub fn receipt_by_trx_hash(&self, trx_hash: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::FinalChainReceiptByTrxHash, trx_hash.as_bytes())?
            .map(|value| value.as_ref().to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbIterator;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    struct MockFinalChainStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
    }

    impl MockFinalChainStore {
        fn new() -> Self {
            MockFinalChainStore {
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

    impl DbReader for MockFinalChainStore {
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
    fn test_meta_value() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        db.put(
            Column::FinalChainMeta,
            &1u32.to_le_bytes(),
            &77u64.to_le_bytes(),
        );

        let result = repo.meta_value(1).unwrap();
        assert_eq!(result, Some(77u64.to_le_bytes().to_vec()));
    }

    #[test]
    fn test_block_header_raw() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        db.put(
            Column::FinalChainBlkByNumber,
            &12u64.to_le_bytes(),
            b"header",
        );

        let result = repo.block_header_raw(12).unwrap();
        assert_eq!(result, Some(b"header".to_vec()));
    }

    #[test]
    fn test_block_hash_by_number() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        let hash = vec![0xAB; 32];
        db.put(
            Column::FinalChainBlkHashByNumber,
            &9u64.to_le_bytes(),
            &hash,
        );

        let result = repo.block_hash_by_number(9).unwrap();
        assert_eq!(result, Some(hash));
    }

    #[test]
    fn test_block_number_by_hash() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        let hash = H256::from_low_u64_be(0x1234);
        db.put(
            Column::FinalChainBlkNumberByHash,
            hash.as_bytes(),
            &44u64.to_le_bytes(),
        );

        let result = repo.block_number_by_hash(hash).unwrap();
        assert_eq!(result, Some(44u64.to_le_bytes().to_vec()));
    }

    #[test]
    fn test_log_blooms_chunk_raw() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        let chunk_id = H256::from_low_u64_be(0x3333);
        db.put(
            Column::FinalChainLogBloomsIndex,
            chunk_id.as_bytes(),
            b"chunk",
        );

        let result = repo.log_blooms_chunk_raw(chunk_id).unwrap();
        assert_eq!(result, Some(b"chunk".to_vec()));
    }

    #[test]
    fn test_receipt_by_trx_hash() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        let trx_hash = H256::from_low_u64_be(0x4444);
        db.put(
            Column::FinalChainReceiptByTrxHash,
            trx_hash.as_bytes(),
            b"receipt",
        );

        let result = repo.receipt_by_trx_hash(trx_hash).unwrap();
        assert_eq!(result, Some(b"receipt".to_vec()));
    }
}
