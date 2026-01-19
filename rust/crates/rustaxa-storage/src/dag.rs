use anyhow::Result;
use ethereum_types::H256;
use rocksdb::{DBWithThreadMode, MultiThreaded};
use rustaxa_types::{DagBlock, TypesError};
use std::sync::Arc;

use crate::Column;

pub struct DagRepository {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
}

impl DagRepository {
    pub fn new(db: Arc<DBWithThreadMode<MultiThreaded>>) -> Self {
        DagRepository { db }
    }

    /// Implements GetDagBlock(blockHash) -> DagBlock
    pub fn dag_block(&self, block: H256) -> Result<DagBlock> {
        let handle = self
            .db
            .cf_handle(Column::DagBlocks.name())
            .expect("Missing DAG column family");
        match self.db.get_pinned_cf(&handle, block.as_bytes()) {
            Ok(Some(value)) => Ok(DagBlock::from_rlp_bytes(&value)?),
            Ok(None) => Err(anyhow::anyhow!("DAG block not found")),
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }

    /// Implements GetLastBlocksLevel() -> uint64
    pub fn last_blocks_level(&self) -> Result<u64> {
        let handle = self
            .db
            .cf_handle(Column::DagBlocksLevel.name())
            .expect("Missing DAG column family");
        let mut iter = self.db.raw_iterator_cf(&handle);
        iter.seek_to_last();

        if let Some(key) = iter.key() {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(key);
            return Ok(u64::from_le_bytes(bytes));
        }

        Ok(0)
    }

    /// Implements GetBlocksByLevel(level) -> [blockHash]
    pub fn blocks_by_level(&self, level: u64) -> Result<Vec<H256>> {
        let handle = self
            .db
            .cf_handle(Column::DagBlocksLevel.name())
            .expect("Missing DAG column family");
        match self.db.get_pinned_cf(&handle, level.to_le_bytes()) {
            Ok(Some(value)) => {
                let rlp = rlp::Rlp::new(&value);
                let hashes: Vec<H256> = rlp.as_list().map_err(TypesError::from)?;
                Ok(hashes)
            }
            Ok(None) => Ok(vec![]), // No blocks at this level
            Err(e) => Err(anyhow::anyhow!(e)),
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
