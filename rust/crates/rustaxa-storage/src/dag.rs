use anyhow::Result;
use ethereum_types::H256;
use rustaxa_types::DagBlock;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::db::{DbReader, DbWriter};
use crate::{Column, StatusField, StorageError};

pub struct DagRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> DagRepository<D> {
    /// Creates a DAG repository over the shared database handle.
    pub fn new(db: Arc<D>) -> Self {
        DagRepository { db }
    }

    /// Returns true when a DAG block is present in either non-finalized storage
    /// or in finalized period indexing data.
    /// C++ mapping: `DbStorage::dagBlockInDb(blk_hash_t const&)`.
    pub fn exists(&self, block: H256) -> Result<bool> {
        // Check potentially non-finalized consensus data.
        if self.db.exist(Column::DagBlocks, block.as_bytes())? {
            return Ok(true);
        }

        // Check finalized consensus data.
        self.db.exist(Column::DagBlockPeriod, block.as_bytes())
    }

    /// Loads and decodes a DAG block by hash, resolving from non-finalized
    /// storage first and then from finalized period data.
    /// C++ mapping: `DbStorage::getDagBlock(blk_hash_t const&)`.
    pub fn by_hash(&self, block: H256) -> Result<DagBlock> {
        let bytes = self.by_hash_rlp(block)?;
        DagBlock::try_from(rustaxa_types::codec::rlp::dag::DagBlockRlp::new(&bytes))
    }

    /// Loads the serialized DAG block RLP by hash and returns `None` when the
    /// block cannot be resolved in either non-finalized or finalized storage.
    pub fn by_hash_rlp_optional(&self, block: H256) -> Result<Option<Vec<u8>>> {
        if let Some(val) = self.db.get(Column::DagBlocks, block.as_bytes())? {
            return Ok(Some(val.as_ref().to_vec()));
        }
        if let Some(val) = self.db.get(Column::DagBlockPeriod, block.as_bytes())? {
            let rlp = rlp::Rlp::new(val.as_ref());
            let period: u64 = rlp.val_at(0)?;
            let position: usize = rlp.val_at(1)?;

            if let Some(period_data) = self.db.get(Column::PeriodData, &period.to_le_bytes())? {
                let period_rlp = rlp::Rlp::new(period_data.as_ref());
                // DAG_BLOCKS_POS_IN_PERIOD_DATA = 2 in C++
                let dag_blocks_rlp = period_rlp.at(2)?;
                let block_rlp = dag_blocks_rlp.at(position)?;
                return Ok(Some(block_rlp.as_raw().to_vec()));
            }
        }
        Ok(None)
    }

    /// Loads the serialized DAG block RLP by hash using both non-finalized and
    /// finalized storage layouts.
    /// C++ mapping: `DbStorage::getDagBlock(blk_hash_t const&)`.
    pub fn by_hash_rlp(&self, block: H256) -> Result<Vec<u8>> {
        self.by_hash_rlp_optional(block)?
            .ok_or_else(|| StorageError::Dag("DAG block not found".to_string()).into())
    }

    /// Returns finalized period and position for a DAG block that is already
    /// indexed as finalized.
    /// C++ mapping: `DbStorage::getDagBlockPeriod(blk_hash_t const&)`.
    pub fn period(&self, block: H256) -> Result<(u64, u32)> {
        self.period_optional(block)?
            .ok_or_else(|| StorageError::Dag("DAG block not found".to_string()).into())
    }

    /// Returns finalized period and position for a DAG block, or `None` when
    /// no finalized location is recorded.
    pub fn period_optional(&self, block: H256) -> Result<Option<(u64, u32)>> {
        let Some(value) = self.db.get(Column::DagBlockPeriod, block.as_bytes())? else {
            return Ok(None);
        };

        let rlp = rlp::Rlp::new(value.as_ref());
        let period: u64 = rlp.val_at(0)?;
        let position: u32 = rlp.val_at(1)?;
        Ok(Some((period, position)))
    }

    /// Returns the highest DAG level currently indexed, or zero when empty.
    /// C++ mapping: `DbStorage::getLastBlocksLevel() const`.
    pub fn last_level(&self) -> Result<u64> {
        let mut iter = self.db.iter_rev(Column::DagBlocksLevel);
        if let Some(res) = iter.next() {
            let (key, _) = res?;
            if key.len() == 8 {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&key);
                return Ok(u64::from_le_bytes(bytes));
            }
        }
        Ok(0)
    }

    /// Returns all DAG block hashes stored for a single level.
    /// C++ mapping: `DbStorage::getBlocksByLevel(level_t)`.
    pub fn hashes_at_level(&self, level: u64) -> Result<Vec<H256>> {
        match self.db.get(Column::DagBlocksLevel, &level.to_le_bytes())? {
            Some(value) => {
                let rlp = rlp::Rlp::new(value.as_ref());
                let hashes: Vec<H256> = rlp.as_list()?;
                Ok(hashes)
            }
            None => Ok(vec![]),
        }
    }

    /// Returns hashes for a contiguous level window, skipping genesis level.
    /// C++ mapping: `DbStorage::getDagBlocksAtLevel(level_t, int)` (hash collection stage).
    pub fn hashes_at_level_range(&self, level: u64, number_of_levels: u32) -> Result<Vec<H256>> {
        let hashes = (0..number_of_levels)
            .map(|depth| level + depth as u64)
            .filter(|&lvl| lvl > 0) // Skip genesis
            .try_fold(Vec::new(), |mut acc, lvl| {
                acc.extend(self.hashes_at_level(lvl)?);
                Ok::<Vec<H256>, anyhow::Error>(acc)
            })?;

        Ok(hashes)
    }

    /// Resolves the proposal period associated with a DAG level using the first
    /// mapping entry at-or-after the requested level.
    /// C++ mapping: `DbStorage::getProposalPeriodForDagLevel(uint64_t)`.
    pub fn proposal_period_at_level(&self, level: u64) -> Result<Option<u64>> {
        match self
            .db
            .get_at_or_after(Column::ProposalPeriodLevelsMap, &level.to_le_bytes())?
        {
            Some((_key, value)) => {
                if value.as_ref().len() != 8 {
                    return Err(StorageError::Dag("Invalid period data size".to_string()).into());
                }
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(value.as_ref());
                Ok(Some(u64::from_le_bytes(bytes)))
            }
            None => Ok(None),
        }
    }

    /// Returns serialized DAG blocks for a contiguous level window, omitting
    /// hashes that cannot be resolved to block payloads.
    pub fn at_level_range(&self, level: u64, number_of_levels: u32) -> Result<Vec<Vec<u8>>> {
        let mut res = Vec::new();
        for i in 0..number_of_levels {
            let l = level + i as u64;
            let blocks = self.hashes_at_level(l)?;
            for hash in blocks {
                if let Ok(rlp) = self.by_hash_rlp(hash) {
                    res.push(rlp);
                }
            }
        }
        Ok(res)
    }

    /// Groups non-finalized DAG blocks by level, preserving each block's raw RLP.
    /// C++ mapping: `DbStorage::getNonfinalizedDagBlocks()`.
    pub fn non_finalized(&self) -> Result<BTreeMap<u64, Vec<Vec<u8>>>> {
        let mut map: BTreeMap<u64, Vec<Vec<u8>>> = BTreeMap::new();
        for res in self.db.iter(Column::DagBlocks) {
            let (_, val) = res?;
            let rlp = rlp::Rlp::new(&val);
            let level: u64 = rlp.val_at(1)?;
            map.entry(level).or_default().push(val.into_vec());
        }
        Ok(map)
    }
}

impl<D: DbReader + DbWriter> DagRepository<D> {
    /// Persists a non-finalized DAG block, updates level index entries, and
    /// increments DAG block and edge counters atomically.
    /// C++ mapping: `DbStorage::saveDagBlock(const std::shared_ptr<DagBlock>&, Batch*)` (no batch path).
    pub fn write(&self, hash: H256, level: u64, tips_count: u64, block_rlp: &[u8]) -> Result<()> {
        let mut write_batch = self.db.create_batch();
        self.db.batch_put(
            &mut write_batch,
            Column::DagBlocks,
            hash.as_bytes(),
            block_rlp,
        )?;

        let level_bytes = self.encode_level_hashes(level, hash)?;
        self.db.batch_put(
            &mut write_batch,
            Column::DagBlocksLevel,
            &level.to_le_bytes(),
            &level_bytes,
        )?;

        let dag_blocks_count = self
            .status_field(StatusField::DagBlkCount as u8)?
            .wrapping_add(1);
        let dag_edge_count = self
            .status_field(StatusField::DagEdgeCount as u8)?
            .wrapping_add(tips_count.wrapping_add(1));

        self.db.batch_put(
            &mut write_batch,
            Column::Status,
            &[StatusField::DagBlkCount as u8],
            &dag_blocks_count.to_le_bytes(),
        )?;
        self.db.batch_put(
            &mut write_batch,
            Column::Status,
            &[StatusField::DagEdgeCount as u8],
            &dag_edge_count.to_le_bytes(),
        )?;

        self.db.commit_batch(write_batch)
    }

    /// Stores finalized DAG block location (period, position) for a block hash.
    /// C++ mapping: `DbStorage::addDagBlockPeriodToBatch(blk_hash_t const&, PbftPeriod, uint32_t, Batch&)`.
    pub fn write_period(&self, hash: H256, period: u64, position: u32) -> Result<()> {
        let mut stream = rlp::RlpStream::new_list(2);
        stream.append(&period);
        stream.append(&position);

        self.db.put(
            Column::DagBlockPeriod,
            hash.as_bytes(),
            stream.out().as_ref(),
        )
    }

    /// Updates level index and DAG counters for an already-saved block.
    /// C++ mapping: `DbStorage::updateDagBlockCounters(std::vector<std::shared_ptr<DagBlock>>)`.
    pub fn update_counter(&self, hash: H256, level: u64, tips_count: u64) -> Result<()> {
        let mut write_batch = self.db.create_batch();

        let level_bytes = self.encode_level_hashes(level, hash)?;
        self.db.batch_put(
            &mut write_batch,
            Column::DagBlocksLevel,
            &level.to_le_bytes(),
            &level_bytes,
        )?;

        let dag_blocks_count = self
            .status_field(StatusField::DagBlkCount as u8)?
            .wrapping_add(1);
        let dag_edge_count = self
            .status_field(StatusField::DagEdgeCount as u8)?
            .wrapping_add(tips_count.wrapping_add(1));

        self.db.batch_put(
            &mut write_batch,
            Column::Status,
            &[StatusField::DagBlkCount as u8],
            &dag_blocks_count.to_le_bytes(),
        )?;
        self.db.batch_put(
            &mut write_batch,
            Column::Status,
            &[StatusField::DagEdgeCount as u8],
            &dag_edge_count.to_le_bytes(),
        )?;

        self.db.commit_batch(write_batch)
    }

    /// Removes a non-finalized DAG block payload by hash.
    /// C++ mapping: `DbStorage::removeDagBlock(blk_hash_t const&)`.
    pub fn remove(&self, hash: H256) -> Result<()> {
        self.db.delete(Column::DagBlocks, hash.as_bytes())
    }

    /// Writes the proposal-period mapping entry for a DAG level.
    /// C++ mapping: `DbStorage::saveProposalPeriodDagLevelsMap(uint64_t, PbftPeriod)`.
    pub fn write_proposal_period_at_level(&self, level: u64, period: u64) -> Result<()> {
        self.db.put(
            Column::ProposalPeriodLevelsMap,
            &level.to_le_bytes(),
            &period.to_le_bytes(),
        )
    }

    // Helper

    fn encode_level_hashes(&self, level: u64, new_hash: H256) -> Result<Vec<u8>> {
        let existing = self.hashes_at_level(level)?;
        let mut merged = BTreeSet::new();
        for hash in existing {
            merged.insert(hash);
        }
        merged.insert(new_hash);

        let mut stream = rlp::RlpStream::new_list(merged.len());
        for hash in merged {
            stream.append(&hash);
        }
        Ok(stream.out().to_vec())
    }

    fn status_field(&self, field: u8) -> Result<u64> {
        let Some(value) = self.db.get(Column::Status, &[field])? else {
            return Ok(0);
        };
        let value = value.as_ref();
        if value.len() != std::mem::size_of::<u64>() {
            return Err(StorageError::Read(format!(
                "Invalid status value size: expected {}, got {}",
                std::mem::size_of::<u64>(),
                value.len()
            ))
            .into());
        }

        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(value);
        Ok(u64::from_le_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbIterator;
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
                // We need to clone the data because we can't manually keep the lock
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
                // We need to clone the data because we can't manually keep the lock
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

    struct ErrorDagStore;

    impl DbReader for ErrorDagStore {
        type Slice<'a> = Vec<u8>;

        fn exist(&self, _col: Column, _key: &[u8]) -> Result<bool> {
            Err(StorageError::Dag("exist failed".to_string()).into())
        }

        fn get<'a>(&'a self, _col: Column, _key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
            Ok(None)
        }

        fn get_at_or_before(
            &self,
            _col: Column,
            _key: &[u8],
        ) -> Result<Option<(Box<[u8]>, Box<[u8]>)>> {
            Ok(None)
        }

        fn get_at_or_after(
            &self,
            _col: Column,
            _key: &[u8],
        ) -> Result<Option<(Box<[u8]>, Box<[u8]>)>> {
            Ok(None)
        }

        fn iter<'a>(&'a self, _col: Column) -> DbIterator<'a> {
            Box::new(std::iter::empty())
        }

        fn iter_rev<'a>(&'a self, _col: Column) -> DbIterator<'a> {
            Box::new(std::iter::empty())
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

        let result = repo.by_hash(block_hash);
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
        let result = repo.by_hash(block_hash);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.to_string().contains("DAG block not found"));

        let optional = repo.by_hash_rlp_optional(block_hash).unwrap();
        assert!(optional.is_none());
    }

    #[test]
    fn test_mock_dag_store_exist() {
        let db = MockDagStore::new();
        let key = H256::from_low_u64_be(7);

        assert!(!db.exist(Column::DagBlocks, key.as_bytes()).unwrap());

        db.put(Column::DagBlocks, key.as_bytes(), &[]);

        assert!(db.exist(Column::DagBlocks, key.as_bytes()).unwrap());
        assert!(!db.exist(Column::DagBlockPeriod, key.as_bytes()).unwrap());
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

        let result = repo.period(block_hash);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), (period, position));

        let optional = repo.period_optional(block_hash).unwrap();
        assert_eq!(optional, Some((period, position)));
    }

    #[test]
    fn test_dag_block_period_missing_optional() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db);

        let missing = repo.period_optional(H256::random()).unwrap();
        assert!(missing.is_none());
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

        let result = repo.hashes_at_level(level);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), hashes);

        // Test non-existent level
        let result = repo.hashes_at_level(level + 1);
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

        let result = repo.last_level();
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
        let result = repo.hashes_at_level_range(10, 3);
        assert!(result.is_ok());
        let all_hashes = result.unwrap();
        // Should contain hashes from 10 and 11
        assert_eq!(all_hashes.len(), 3);
        assert_eq!(all_hashes[0], hashes10[0]);
        assert_eq!(all_hashes[1], hashes10[1]);
        assert_eq!(all_hashes[2], hashes11[0]);
    }

    #[test]
    fn test_dag_block_in_db() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db.clone());
        let block_hash = H256::random();
        let block_hash_finalized = H256::random();

        // Initially not in DB
        assert!(!repo.exists(block_hash).unwrap());

        // Add to DagBlocks (non-finalized)
        db.put(Column::DagBlocks, block_hash.as_bytes(), &[]);
        assert!(repo.exists(block_hash).unwrap());

        // Add to DagBlockPeriod (finalized)
        db.put(Column::DagBlockPeriod, block_hash_finalized.as_bytes(), &[]);
        assert!(repo.exists(block_hash_finalized).unwrap());
    }

    #[test]
    fn test_dag_block_in_db_propagates_exist_error() {
        let repo = DagRepository::new(Arc::new(ErrorDagStore));
        let err = repo.exists(H256::from_low_u64_be(9)).unwrap_err();

        assert!(err.to_string().contains("exist failed"));
    }

    #[test]
    fn test_proposal_period_for_dag_level() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db.clone());

        // Initially not set
        assert!(repo.proposal_period_at_level(10).unwrap().is_none());

        // Map is sparse and lookup should return first key >= requested level.
        db.put(
            Column::ProposalPeriodLevelsMap,
            &100u64.to_le_bytes(),
            &0u64.to_le_bytes(),
        );
        db.put(
            Column::ProposalPeriodLevelsMap,
            &103u64.to_le_bytes(),
            &1u64.to_le_bytes(),
        );
        db.put(
            Column::ProposalPeriodLevelsMap,
            &106u64.to_le_bytes(),
            &3u64.to_le_bytes(),
        );

        assert_eq!(repo.proposal_period_at_level(5).unwrap(), Some(0));
        assert_eq!(repo.proposal_period_at_level(100).unwrap(), Some(0));
        assert_eq!(repo.proposal_period_at_level(101).unwrap(), Some(1));
        assert_eq!(repo.proposal_period_at_level(103).unwrap(), Some(1));
        assert_eq!(repo.proposal_period_at_level(105).unwrap(), Some(3));
        assert_eq!(repo.proposal_period_at_level(106).unwrap(), Some(3));
        assert!(repo.proposal_period_at_level(107).unwrap().is_none());
    }

    #[test]
    fn test_nonfinalized_dag_blocks() {
        let db = Arc::new(MockDagStore::new());
        let repo = DagRepository::new(db.clone());

        // Create 2 blocks at same level
        let block1_hash = H256::random();
        let block1 = create_dummy_dag_block_rlp(); // Assumes level 10 inside dummy

        let block2_hash = H256::random();
        let block2 = create_dummy_dag_block_rlp(); // Assumes level 10 inside dummy

        // Adjust dummy creation helper or just patch bytes?
        // create_dummy_dag_block_rlp creates block with level 10.
        // We can use it directly.

        db.put(Column::DagBlocks, block1_hash.as_bytes(), &block1);
        db.put(Column::DagBlocks, block2_hash.as_bytes(), &block2);

        let result = repo.non_finalized().unwrap();
        assert_eq!(result.len(), 1); // 1 level
        assert_eq!(result.get(&10).unwrap().len(), 2);
    }
}
