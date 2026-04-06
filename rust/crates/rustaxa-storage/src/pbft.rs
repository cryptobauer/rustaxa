use anyhow::Result;
use ethereum_types::H256;
use std::sync::Arc;

use crate::Column;
use crate::db::DbReader;

pub struct PbftRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> PbftRepository<D> {
    pub fn new(db: Arc<D>) -> Self {
        PbftRepository { db }
    }

    /// Implements pbftBlockInDb(hash) -> bool
    pub fn pbft_block_in_db(&self, pbft_hash: H256) -> Result<bool> {
        self.db.exist(Column::PbftBlockPeriod, pbft_hash.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbIterator;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    struct MockPbftStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
    }

    impl MockPbftStore {
        fn new() -> Self {
            MockPbftStore {
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

    impl DbReader for MockPbftStore {
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
    fn test_mock_period_store_exist() {
        let db = MockPbftStore::new();
        let hash = H256::from_low_u64_be(3);

        assert!(!db.exist(Column::PbftBlockPeriod, hash.as_bytes()).unwrap());

        db.put(Column::PbftBlockPeriod, hash.as_bytes(), &[0xAA]);

        assert!(db.exist(Column::PbftBlockPeriod, hash.as_bytes()).unwrap());
        assert!(
            !db.exist(Column::ProposedPbftBlocks, hash.as_bytes())
                .unwrap()
        );
    }
}
