use anyhow::Result;
use ethereum_types::H256;
use std::sync::Arc;

use crate::db::{DbReader, DbWriter};
use crate::{Column, StatusField};

/// Final-chain execution counters persisted with finalized block visibility.
///
/// The fields mirror C++ `StatusDbField::ExecutedBlkCount` and
/// `StatusDbField::ExecutedTrxCount`. Callers pass absolute counter values,
/// not deltas, so the repository can atomically publish block indexes,
/// Rust-owned snapshots, and legacy status counters in one database batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalChainExecutionStatus {
    /// Total number of DAG blocks executed by finalized PBFT blocks.
    pub executed_dag_block_count: u64,
    /// Total number of transactions executed by finalized PBFT blocks.
    pub executed_transaction_count: u64,
}

pub struct FinalChainRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> FinalChainRepository<D> {
    const DPOS_SNAPSHOT_KEY_PREFIX: &'static [u8] = b"rustaxa:dpos_snapshot:";
    const ACCOUNT_SNAPSHOT_KEY_PREFIX: &'static [u8] = b"rustaxa:account_snapshot:";

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

    /// Returns the Rust-owned DPoS snapshot payload for a finalized block.
    ///
    /// The payload is stored in `final_chain_meta` under a Rust-prefixed key so
    /// the existing C++ `u32` metadata keys stay unchanged. This is intentionally
    /// a Rust rewrite sidecar until FinalChain persistence is fully typed.
    pub fn dpos_snapshot_raw(&self, number: u64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::FinalChainMeta, &Self::dpos_snapshot_key(number))?
            .map(|value| value.as_ref().to_vec()))
    }

    /// Returns the Rust-owned account snapshot payload for a finalized block.
    ///
    /// The payload is a Rust rewrite sidecar stored in `final_chain_meta` under
    /// a Rust-prefixed key. It is intentionally separate from legacy C++
    /// metadata keys so Rust account-state durability can advance without
    /// changing existing column-family contracts.
    pub fn account_snapshot_raw(&self, number: u64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::FinalChainMeta, &Self::account_snapshot_key(number))?
            .map(|value| value.as_ref().to_vec()))
    }

    fn dpos_snapshot_key(number: u64) -> Vec<u8> {
        let mut key = Vec::with_capacity(Self::DPOS_SNAPSHOT_KEY_PREFIX.len() + 8);
        key.extend_from_slice(Self::DPOS_SNAPSHOT_KEY_PREFIX);
        key.extend_from_slice(&number.to_le_bytes());
        key
    }

    fn account_snapshot_key(number: u64) -> Vec<u8> {
        let mut key = Vec::with_capacity(Self::ACCOUNT_SNAPSHOT_KEY_PREFIX.len() + 8);
        key.extend_from_slice(Self::ACCOUNT_SNAPSHOT_KEY_PREFIX);
        key.extend_from_slice(&number.to_le_bytes());
        key
    }
}

impl<D: DbReader + DbWriter> FinalChainRepository<D> {
    /// Persists a finalized block header and its lookup indexes atomically.
    ///
    /// C++ mapping: the final-chain portions of `FinalChain::appendBlock`.
    pub fn write_block_header(
        &self,
        number: u64,
        hash: H256,
        stored_header_rlp: &[u8],
        receipts_rlp: &[u8],
    ) -> Result<()> {
        self.write_block_header_with_dpos_snapshot(
            number,
            hash,
            stored_header_rlp,
            receipts_rlp,
            None,
        )
    }

    /// Persists a finalized block header, its lookup indexes, and an optional
    /// Rust-owned DPoS snapshot atomically.
    pub fn write_block_header_with_dpos_snapshot(
        &self,
        number: u64,
        hash: H256,
        stored_header_rlp: &[u8],
        receipts_rlp: &[u8],
        dpos_snapshot_rlp: Option<&[u8]>,
    ) -> Result<()> {
        self.write_block_header_with_snapshots(
            number,
            hash,
            stored_header_rlp,
            receipts_rlp,
            dpos_snapshot_rlp,
            None,
        )
    }

    /// Persists a finalized block header, lookup indexes, optional Rust-owned
    /// DPoS snapshot, optional Rust-owned account snapshot, and LAST_NUMBER in
    /// one database batch.
    ///
    /// LAST_NUMBER is written after the sidecar snapshot records, so a committed
    /// finalized block number is never made visible without the Rust snapshot
    /// payloads supplied by the caller. Callers may pass `None` for a snapshot
    /// only when that snapshot is intentionally not available for the block.
    pub fn write_block_header_with_snapshots(
        &self,
        number: u64,
        hash: H256,
        stored_header_rlp: &[u8],
        receipts_rlp: &[u8],
        dpos_snapshot_rlp: Option<&[u8]>,
        account_snapshot_rlp: Option<&[u8]>,
    ) -> Result<()> {
        self.write_block_header_with_snapshots_and_execution_status(
            number,
            hash,
            stored_header_rlp,
            receipts_rlp,
            dpos_snapshot_rlp,
            account_snapshot_rlp,
            None,
        )
    }

    /// Persists a finalized block header, lookup indexes, Rust-owned snapshots,
    /// optional execution counters, and LAST_NUMBER in one database batch.
    ///
    /// Inputs are the finalized block number and hash, stored-header and
    /// receipt RLP payloads, optional Rust snapshot payloads, and optional
    /// absolute execution counters. Outputs are durable database records only.
    /// LAST_NUMBER remains the final write in the batch, so a committed block is
    /// never visible without the supplied snapshot payloads and status counters.
    #[allow(clippy::too_many_arguments)]
    pub fn write_block_header_with_snapshots_and_execution_status(
        &self,
        number: u64,
        hash: H256,
        stored_header_rlp: &[u8],
        receipts_rlp: &[u8],
        dpos_snapshot_rlp: Option<&[u8]>,
        account_snapshot_rlp: Option<&[u8]>,
        execution_status: Option<FinalChainExecutionStatus>,
    ) -> Result<()> {
        const DB_META_LAST_NUMBER: u32 = 1;

        let mut batch = self.db.create_batch();
        self.db.batch_put(
            &mut batch,
            Column::FinalChainBlkByNumber,
            &number.to_le_bytes(),
            stored_header_rlp,
        )?;
        self.db.batch_put(
            &mut batch,
            Column::FinalChainReceiptByPeriod,
            &number.to_le_bytes(),
            receipts_rlp,
        )?;
        self.db.batch_put(
            &mut batch,
            Column::FinalChainBlkHashByNumber,
            &number.to_le_bytes(),
            hash.as_bytes(),
        )?;
        self.db.batch_put(
            &mut batch,
            Column::FinalChainBlkNumberByHash,
            hash.as_bytes(),
            &number.to_le_bytes(),
        )?;
        if let Some(dpos_snapshot_rlp) = dpos_snapshot_rlp {
            self.db.batch_put(
                &mut batch,
                Column::FinalChainMeta,
                &Self::dpos_snapshot_key(number),
                dpos_snapshot_rlp,
            )?;
        }
        if let Some(account_snapshot_rlp) = account_snapshot_rlp {
            self.db.batch_put(
                &mut batch,
                Column::FinalChainMeta,
                &Self::account_snapshot_key(number),
                account_snapshot_rlp,
            )?;
        }
        if let Some(execution_status) = execution_status {
            self.db.batch_put(
                &mut batch,
                Column::Status,
                &[StatusField::ExecutedBlkCount as u8],
                &execution_status.executed_dag_block_count.to_le_bytes(),
            )?;
            self.db.batch_put(
                &mut batch,
                Column::Status,
                &[StatusField::ExecutedTrxCount as u8],
                &execution_status.executed_transaction_count.to_le_bytes(),
            )?;
        }
        self.db.batch_put(
            &mut batch,
            Column::FinalChainMeta,
            &DB_META_LAST_NUMBER.to_le_bytes(),
            &number.to_le_bytes(),
        )?;
        self.db.commit_batch(batch)
    }

    /// Persists one finalized transaction receipt by transaction hash.
    ///
    /// C++ mapping: `DbStorage::insert(..., Columns::final_chain_receipt_by_trx_hash)`.
    pub fn write_receipt_by_trx_hash(&self, trx_hash: H256, receipt_rlp: &[u8]) -> Result<()> {
        self.db.put(
            Column::FinalChainReceiptByTrxHash,
            trx_hash.as_bytes(),
            receipt_rlp,
        )
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
    fn test_dpos_snapshot_raw_uses_rust_prefixed_meta_key() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        let snapshot = vec![0xc0];
        db.put(
            Column::FinalChainMeta,
            &FinalChainRepository::<MockFinalChainStore>::dpos_snapshot_key(7),
            &snapshot,
        );

        assert_eq!(repo.dpos_snapshot_raw(7).unwrap(), Some(snapshot));
        assert_eq!(repo.dpos_snapshot_raw(8).unwrap(), None);
    }

    #[test]
    fn test_account_snapshot_raw_uses_rust_prefixed_meta_key() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        let snapshot = vec![0xc0];
        db.put(
            Column::FinalChainMeta,
            &FinalChainRepository::<MockFinalChainStore>::account_snapshot_key(7),
            &snapshot,
        );

        assert_eq!(repo.account_snapshot_raw(7).unwrap(), Some(snapshot));
        assert_eq!(repo.account_snapshot_raw(8).unwrap(), None);
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
