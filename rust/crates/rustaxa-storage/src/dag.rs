use anyhow::Result;
use ethereum_types::H256;
use rocksdb::{DBPinnableSlice, DBWithThreadMode, MultiThreaded};
use rustaxa_types::{DagBlock, TypesError};
use std::sync::Arc;

use crate::{Column, StorageError};

/// Trait abstracting database read operations.
pub trait DbReader: Send + Sync {
    type Slice<'a>: AsRef<[u8]>
    where
        Self: 'a;

    fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>>;
    fn get_last_key(&self, col: Column) -> Result<Option<Vec<u8>>>;
}

impl DbReader for DBWithThreadMode<MultiThreaded> {
    type Slice<'a> = DBPinnableSlice<'a>;

    fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        self.get_pinned_cf(&handle, key)
            .map_err(|e| StorageError::Database(e).into())
    }

    fn get_last_key(&self, col: Column) -> Result<Option<Vec<u8>>> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        let mut iter = self.raw_iterator_cf(&handle);
        iter.seek_to_last();
        if let Some(key) = iter.key() {
            Ok(Some(key.to_vec()))
        } else {
            Ok(None)
        }
    }
}

pub struct DagRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> DagRepository<D> {
    pub fn new(db: Arc<D>) -> Self {
        DagRepository { db }
    }

    /// Implements GetDagBlock(blockHash) -> DagBlock
    pub fn dag_block(&self, block: H256) -> Result<DagBlock> {
        let value = self
            .db
            .get(Column::DagBlocks, block.as_bytes())?
            .ok_or_else(|| StorageError::Dag("DAG block not found".to_string()))?;
        Ok(DagBlock::from_rlp_bytes(value.as_ref())?)
    }

    /// Implements GetDagBlockPeriod() -> (uint64, uint32)
    pub fn dag_block_period(&self, block: H256) -> Result<(u64, u32)> {
        let value = self
            .db
            .get(Column::DagBlockPeriod, block.as_bytes())?
            .ok_or_else(|| StorageError::Dag("DAG block not found".to_string()))?;

        let rlp = rlp::Rlp::new(value.as_ref());
        let period: u64 = rlp.val_at(0)?;
        let position: u32 = rlp.val_at(1)?;
        Ok((period, position))
    }

    /// Implements GetLastBlocksLevel() -> uint64
    pub fn last_blocks_level(&self) -> Result<u64> {
        if let Some(key) = self.db.get_last_key(Column::DagBlocksLevel)? {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&key);
            return Ok(u64::from_le_bytes(bytes));
        }
        Ok(0)
    }

    /// Implements GetBlocksByLevel(level) -> [blockHash]
    pub fn blocks_by_level(&self, level: u64) -> Result<Vec<H256>> {
        match self.db.get(Column::DagBlocksLevel, &level.to_le_bytes())? {
            Some(value) => {
                let rlp = rlp::Rlp::new(value.as_ref());
                let hashes: Vec<H256> = rlp.as_list().map_err(TypesError::from)?;
                Ok(hashes)
            }
            None => Ok(vec![]),
        }
    }

    /// Implements GetDagBlocksAtLevel(level, number_of_levels) -> [blockHash]
    pub fn dag_blocks_at_level(&self, level: u64, number_of_levels: u32) -> Result<Vec<H256>> {
        let hashes = (0..number_of_levels)
            .map(|depth| level + depth as u64)
            .filter(|&lvl| lvl > 0) // Skip genesis
            .try_fold(Vec::new(), |mut acc, lvl| {
                acc.extend(self.blocks_by_level(lvl)?);
                Ok::<Vec<H256>, anyhow::Error>(acc)
            })?;

        Ok(hashes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::RlpStream;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    // In-memory mock implementation
    struct MockDagStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
    }

    impl MockDagStore {
        fn new() -> Self {
            MockDagStore {
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

    impl DbReader for MockDagStore {
        type Slice<'a> = Vec<u8>;

        fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                Ok(cf.get(key).cloned())
            } else {
                Ok(None)
            }
        }

        fn get_last_key(&self, col: Column) -> Result<Option<Vec<u8>>> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                Ok(cf.keys().last().cloned())
            } else {
                Ok(None)
            }
        }
    }

    fn create_dummy_dag_block_rlp() -> Vec<u8> {
        let mut stream = RlpStream::new_list(8);
        stream.append(&H256::zero()); // pivot
        stream.append(&10u64); // level
        stream.append(&123456789u64); // timestamp
        stream.append(&vec![1u8, 2, 3]); // vdf
        stream.begin_list(0); // tips
        stream.begin_list(0); // transactions
        stream.append(&vec![0u8; 65]); // signature
        stream.append(&1000u64); // gas_estimation
        stream.out().to_vec()
    }

    #[test]
    fn test_dag_block_found() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db.clone());

        let block_hash = H256::random();
        let block_rlp = create_dummy_dag_block_rlp();

        db.put(Column::DagBlocks, block_hash.as_bytes(), &block_rlp);

        let result = repo.dag_block(block_hash);
        assert!(result.is_ok());
        let block = result.unwrap();
        assert_eq!(block.level, 10);
        assert_eq!(block.timestamp, 123456789);
    }

    #[test]
    fn test_dag_block_not_found() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db.clone());

        let block_hash = H256::random();
        let result = repo.dag_block(block_hash);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.to_string().contains("DAG block not found"));
    }

    #[test]
    fn test_dag_block_period() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db.clone());

        let block_hash = H256::random();
        let period = 5u64;
        let position = 2u32;

        let mut stream = RlpStream::new_list(2);
        stream.append(&period);
        stream.append(&position);
        let data = stream.out().to_vec();

        db.put(Column::DagBlockPeriod, block_hash.as_bytes(), &data);

        let result = repo.dag_block_period(block_hash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), (period, position));
    }

    #[test]
    fn test_blocks_by_level() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db.clone());

        let level = 5u64;
        let hashes = vec![H256::random(), H256::random()];

        let mut stream = RlpStream::new_list(hashes.len());
        for h in &hashes {
            stream.append(h);
        }
        let data = stream.out().to_vec();

        db.put(Column::DagBlocksLevel, &level.to_le_bytes(), &data);

        let result = repo.blocks_by_level(level);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), hashes);

        // Test non-existent level
        let result = repo.blocks_by_level(level + 1);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_last_blocks_level() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db.clone());

        // Insert levels 1, 5, 10
        let levels = vec![1u64, 5, 10];
        for l in levels {
            db.put(Column::DagBlocksLevel, &l.to_le_bytes(), &[]);
        }

        let result = repo.last_blocks_level();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 10);
    }

    #[test]
    fn test_dag_blocks_at_level() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db.clone());

        // Level 10: 2 blocks
        let level10 = 10u64;
        let hashes10 = vec![H256::random(), H256::random()];
        let mut s10 = RlpStream::new_list(hashes10.len());
        for h in &hashes10 {
            s10.append(h);
        }
        db.put(Column::DagBlocksLevel, &level10.to_le_bytes(), &s10.out());

        // Level 11: 1 block
        let level11 = 11u64;
        let hashes11 = vec![H256::random()];
        let mut s11 = RlpStream::new_list(hashes11.len());
        for h in &hashes11 {
            s11.append(h);
        }
        db.put(Column::DagBlocksLevel, &level11.to_le_bytes(), &s11.out());

        // Level 12: 0 blocks (empty entry - implicitly handled by mock key not found or empty value)
        // Note: Mock iterators work on BTreeMap which sorts by Key (bytes).

        // Fetch 3 levels starting from 10
        let result = repo.dag_blocks_at_level(10, 3);
        assert!(result.is_ok());
        let all_hashes = result.unwrap();
        // Should contain hashes from 10 and 11
        assert_eq!(all_hashes.len(), 3);
        assert_eq!(all_hashes[0], hashes10[0]);
        assert_eq!(all_hashes[1], hashes10[1]);
        assert_eq!(all_hashes[2], hashes11[0]);
    }
}
