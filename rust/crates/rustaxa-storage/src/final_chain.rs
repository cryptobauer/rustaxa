use anyhow::{Result, bail};
use ethereum_types::{H256, U256};
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

/// Rewards-stat cache mutation committed with finalized-block visibility.
///
/// Native Rust FinalChain uses this intent to keep legacy `BlockRewardsStats`
/// cache rows in the same database batch as the finalized block header,
/// snapshots, execution counters, and `LAST_NUMBER`. A cache-current-period
/// update writes the supplied current block stats under `current_period`; a
/// clear update removes all cached interval rows after the current period has
/// reached a distribution boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalChainRewardsStatsUpdate<'a> {
    /// Finalized PBFT period whose rewards-stat planner produced the update.
    pub current_period: u64,
    /// Whether to persist `current_block_stats_rlp` for later distribution.
    pub cache_current_period: bool,
    /// Whether to clear every persisted rewards-stat cache row.
    pub clear_cached_stats: bool,
    /// Legacy-compatible `rewards::BlockStats` RLP for `current_period`.
    pub current_block_stats_rlp: &'a [u8],
}

/// Number of bloom entries stored in one legacy FinalChain bloom-index chunk.
pub const FINAL_CHAIN_BLOOM_INDEX_SIZE: usize = 16;
/// Number of recursive bloom-index levels used by legacy FinalChain queries.
pub const FINAL_CHAIN_BLOOM_INDEX_LEVELS: u64 = 2;
/// Byte width of one Ethereum log bloom.
pub const FINAL_CHAIN_LOG_BLOOM_BYTES: usize = 256;

/// Fixed-width FinalChain log bloom stored in index chunks.
pub type FinalChainLogBloom = [u8; FINAL_CHAIN_LOG_BLOOM_BYTES];
/// Legacy RLP list of bloom entries for one FinalChain bloom-index chunk.
pub type FinalChainLogBloomChunk = [FinalChainLogBloom; FINAL_CHAIN_BLOOM_INDEX_SIZE];

/// Log-bloom index mutation committed with finalized-block visibility.
///
/// The bloom must already include the receipt-log bloom plus legacy author
/// bloom augmentation. The repository ORs it into every legacy index level and
/// writes all affected chunks before publishing `LAST_NUMBER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalChainLogBloomIndexUpdate<'a> {
    /// Finalized block number being inserted into the bloom index.
    pub block_number: u64,
    /// Receipt and author bloom used by legacy FinalChain bloom queries.
    pub bloom: &'a FinalChainLogBloom,
}

/// Transaction index mutation committed with finalized-block visibility.
///
/// Each item writes the legacy finalized transaction location and receipt-by-hash
/// lookup for one transaction in the finalized block. The repository commits
/// these rows before `LAST_NUMBER`, so restart and RPC readers cannot observe a
/// finalized head whose transaction index rows are missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalChainTransactionIndexUpdate<'a> {
    /// Canonical transaction hash used by both legacy indexes.
    pub transaction_hash: H256,
    /// Zero-based transaction position in the finalized block.
    pub position: u32,
    /// Whether the location points to a system transaction.
    pub is_system: bool,
    /// Canonical legacy transaction receipt RLP.
    pub receipt_rlp: &'a [u8],
}

/// Period-level system transaction hash list committed with block visibility.
///
/// The payload is the legacy RLP list stored in `period_system_transactions`.
/// External-EVM FinalChain publication commits this row in the same batch as
/// header, receipt, transaction-index, bloom-index, counter, and `LAST_NUMBER`
/// rows so restart and RPC readers cannot observe a finalized block whose
/// system transaction hash list is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalChainPeriodSystemTransactionsUpdate<'a> {
    /// Finalized PBFT period whose system transaction hash list is being stored.
    pub period: u64,
    /// Canonical legacy RLP list of system transaction hashes.
    pub hashes_rlp: &'a [u8],
}

/// Returns the legacy FinalChain bloom chunk identifier.
///
/// C++ maps `(level, index)` to `h256(index * 0xff + level)`, encoded as a
/// 32-byte big-endian integer. Overflow is rejected so callers do not silently
/// address the wrong chunk for very large synthetic inputs.
pub fn final_chain_log_bloom_chunk_id(level: u64, index: u64) -> Result<H256> {
    let value = U256::from(index)
        .checked_mul(U256::from(0xffu64))
        .and_then(|value| value.checked_add(U256::from(level)))
        .ok_or_else(|| anyhow::anyhow!("final-chain bloom chunk id overflow"))?;
    Ok(H256::from(value.to_big_endian()))
}

/// Returns an all-zero legacy bloom-index chunk.
pub fn zero_final_chain_log_bloom_chunk() -> FinalChainLogBloomChunk {
    [[0u8; FINAL_CHAIN_LOG_BLOOM_BYTES]; FINAL_CHAIN_BLOOM_INDEX_SIZE]
}

/// Decodes a legacy FinalChain bloom-index chunk.
///
/// Missing chunks are all-zero chunks. Present chunks must be an RLP list of
/// exactly sixteen raw 256-byte blooms; malformed data is returned as an error
/// so Rust-mode queries do not silently skip persisted facts.
pub fn decode_final_chain_log_bloom_chunk(raw: Option<&[u8]>) -> Result<FinalChainLogBloomChunk> {
    let Some(raw) = raw else {
        return Ok(zero_final_chain_log_bloom_chunk());
    };
    if raw.is_empty() {
        return Ok(zero_final_chain_log_bloom_chunk());
    }

    let rlp = rlp::Rlp::new(raw);
    let item_count = rlp.item_count()?;
    if item_count != FINAL_CHAIN_BLOOM_INDEX_SIZE {
        bail!(
            "final-chain bloom chunk has {item_count} entries, expected {FINAL_CHAIN_BLOOM_INDEX_SIZE}"
        );
    }

    let mut chunk = zero_final_chain_log_bloom_chunk();
    for (index, bloom) in chunk.iter_mut().enumerate() {
        let data = rlp.at(index)?.data()?;
        if data.len() != FINAL_CHAIN_LOG_BLOOM_BYTES {
            bail!(
                "final-chain bloom chunk entry {index} has {} bytes, expected {FINAL_CHAIN_LOG_BLOOM_BYTES}",
                data.len()
            );
        }
        bloom.copy_from_slice(data);
    }
    Ok(chunk)
}

/// Encodes a legacy FinalChain bloom-index chunk as an RLP list of raw blooms.
pub fn encode_final_chain_log_bloom_chunk(chunk: &FinalChainLogBloomChunk) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(FINAL_CHAIN_BLOOM_INDEX_SIZE);
    for bloom in chunk {
        stream.append(&bloom.as_slice());
    }
    stream.out().to_vec()
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
        self.write_block_header_with_snapshots_execution_status_and_rewards_stats(
            number,
            hash,
            stored_header_rlp,
            receipts_rlp,
            dpos_snapshot_rlp,
            account_snapshot_rlp,
            execution_status,
            None,
            None,
            &[],
            None,
        )
    }

    /// Persists finalized-block state, optional rewards-stat cache mutation,
    /// and optional log-bloom index mutation in one batch.
    ///
    /// Inputs match `write_block_header_with_snapshots_and_execution_status`
    /// with additional rewards-stat, log-bloom index, transaction-index, and
    /// period system transaction intents. If supplied, all mutations are
    /// committed before `LAST_NUMBER`, so startup cannot observe the new
    /// finalized head without the corresponding native cache, bloom-index
    /// state, transaction indexes, and period system transaction hash list.
    #[allow(clippy::too_many_arguments)]
    pub fn write_block_header_with_snapshots_execution_status_and_rewards_stats(
        &self,
        number: u64,
        hash: H256,
        stored_header_rlp: &[u8],
        receipts_rlp: &[u8],
        dpos_snapshot_rlp: Option<&[u8]>,
        account_snapshot_rlp: Option<&[u8]>,
        execution_status: Option<FinalChainExecutionStatus>,
        rewards_stats_update: Option<FinalChainRewardsStatsUpdate<'_>>,
        log_bloom_index_update: Option<FinalChainLogBloomIndexUpdate<'_>>,
        transaction_index_updates: &[FinalChainTransactionIndexUpdate<'_>],
        period_system_transactions_update: Option<FinalChainPeriodSystemTransactionsUpdate<'_>>,
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
        if let Some(update) = rewards_stats_update {
            if update.cache_current_period && update.clear_cached_stats {
                bail!("final-chain rewards stats update cannot both cache and clear");
            }
            if update.cache_current_period && update.current_block_stats_rlp.is_empty() {
                bail!("final-chain rewards stats update is missing current block stats RLP");
            }
            if update.clear_cached_stats {
                for item in self.db.iter(Column::BlockRewardsStats) {
                    let (key, _) = item?;
                    self.db
                        .batch_delete(&mut batch, Column::BlockRewardsStats, &key)?;
                }
            } else if update.cache_current_period {
                self.db.batch_put(
                    &mut batch,
                    Column::BlockRewardsStats,
                    &update.current_period.to_le_bytes(),
                    update.current_block_stats_rlp,
                )?;
            }
        }
        if let Some(update) = log_bloom_index_update {
            self.write_log_bloom_index_update(&mut batch, update)?;
        }
        for update in transaction_index_updates {
            self.write_transaction_index_update(number, &mut batch, *update)?;
        }
        if let Some(update) = period_system_transactions_update {
            if update.period != number {
                bail!(
                    "final-chain period system transaction update period {} does not match block number {number}",
                    update.period
                );
            }
            self.db.batch_put(
                &mut batch,
                Column::PeriodSystemTransactions,
                &update.period.to_le_bytes(),
                update.hashes_rlp,
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

    fn write_log_bloom_index_update(
        &self,
        batch: &mut D::Batch,
        update: FinalChainLogBloomIndexUpdate<'_>,
    ) -> Result<()> {
        let mut index = update.block_number;
        for level in 0..FINAL_CHAIN_BLOOM_INDEX_LEVELS {
            let chunk_index = index / FINAL_CHAIN_BLOOM_INDEX_SIZE as u64;
            let chunk_id = final_chain_log_bloom_chunk_id(level, chunk_index)?;
            let raw = self.log_blooms_chunk_raw(chunk_id)?;
            let mut chunk = decode_final_chain_log_bloom_chunk(raw.as_deref())?;
            let slot = (index % FINAL_CHAIN_BLOOM_INDEX_SIZE as u64) as usize;
            for (stored, added) in chunk[slot].iter_mut().zip(update.bloom.iter()) {
                *stored |= *added;
            }
            let encoded = encode_final_chain_log_bloom_chunk(&chunk);
            self.db.batch_put(
                batch,
                Column::FinalChainLogBloomsIndex,
                chunk_id.as_bytes(),
                &encoded,
            )?;
            index /= FINAL_CHAIN_BLOOM_INDEX_SIZE as u64;
        }
        Ok(())
    }

    fn write_transaction_index_update(
        &self,
        block_number: u64,
        batch: &mut D::Batch,
        update: FinalChainTransactionIndexUpdate<'_>,
    ) -> Result<()> {
        let mut location = rlp::RlpStream::new_list(2 + usize::from(update.is_system));
        location.append(&block_number);
        location.append(&update.position);
        if update.is_system {
            location.append(&update.is_system);
        }
        self.db.batch_put(
            batch,
            Column::TrxPeriod,
            update.transaction_hash.as_bytes(),
            location.out().as_ref(),
        )?;
        self.db.batch_put(
            batch,
            Column::FinalChainReceiptByTrxHash,
            update.transaction_hash.as_bytes(),
            update.receipt_rlp,
        )
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

    enum BatchOp {
        Put(Column, Vec<u8>, Vec<u8>),
        Delete(Column, Vec<u8>),
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

        fn delete(&self, col: Column, key: &[u8]) {
            let mut data = self.data.write().unwrap();
            if let Some(cf) = data.get_mut(col.name()) {
                cf.remove(key);
            }
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

    impl DbWriter for MockFinalChainStore {
        type Batch = Vec<BatchOp>;

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
            batch.push(BatchOp::Put(col, key.to_vec(), value.to_vec()));
            Ok(())
        }

        fn batch_delete(&self, batch: &mut Self::Batch, col: Column, key: &[u8]) -> Result<()> {
            batch.push(BatchOp::Delete(col, key.to_vec()));
            Ok(())
        }

        fn commit_batch(&self, batch: Self::Batch) -> Result<()> {
            for op in batch {
                match op {
                    BatchOp::Put(col, key, value) => {
                        MockFinalChainStore::put(self, col, &key, &value)
                    }
                    BatchOp::Delete(col, key) => MockFinalChainStore::delete(self, col, &key),
                }
            }
            Ok(())
        }

        fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()> {
            MockFinalChainStore::put(self, col, key, value);
            Ok(())
        }

        fn delete(&self, col: Column, key: &[u8]) -> Result<()> {
            MockFinalChainStore::delete(self, col, key);
            Ok(())
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
    fn test_log_bloom_chunk_id_matches_legacy_integer_mapping() {
        assert_eq!(
            final_chain_log_bloom_chunk_id(1, 2).unwrap(),
            H256::from_low_u64_be(0x1ff)
        );
    }

    #[test]
    fn test_log_bloom_chunk_codec_round_trips_and_rejects_malformed_entries() {
        let mut chunk = zero_final_chain_log_bloom_chunk();
        chunk[3][17] = 0x80;
        chunk[3][99] = 0x02;

        let encoded = encode_final_chain_log_bloom_chunk(&chunk);
        let decoded = decode_final_chain_log_bloom_chunk(Some(&encoded)).unwrap();
        assert_eq!(decoded, chunk);
        assert_eq!(
            decode_final_chain_log_bloom_chunk(None).unwrap(),
            zero_final_chain_log_bloom_chunk()
        );

        let mut malformed = rlp::RlpStream::new_list(FINAL_CHAIN_BLOOM_INDEX_SIZE);
        for index in 0..FINAL_CHAIN_BLOOM_INDEX_SIZE {
            let len = if index == 4 {
                FINAL_CHAIN_LOG_BLOOM_BYTES - 1
            } else {
                FINAL_CHAIN_LOG_BLOOM_BYTES
            };
            malformed.append(&vec![0u8; len]);
        }
        assert!(decode_final_chain_log_bloom_chunk(Some(&malformed.out())).is_err());
    }

    #[test]
    fn test_write_block_header_updates_log_bloom_index_before_last_number() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        let mut bloom = [0u8; FINAL_CHAIN_LOG_BLOOM_BYTES];
        bloom[0] = 0x01;
        bloom[255] = 0x80;

        repo.write_block_header_with_snapshots_execution_status_and_rewards_stats(
            17,
            H256::from_low_u64_be(0x5555),
            b"header",
            b"receipts",
            None,
            None,
            None,
            None,
            Some(FinalChainLogBloomIndexUpdate {
                block_number: 17,
                bloom: &bloom,
            }),
            &[FinalChainTransactionIndexUpdate {
                transaction_hash: H256::from_low_u64_be(0x7777),
                position: 2,
                is_system: false,
                receipt_rlp: b"receipt-by-hash",
            }],
            Some(FinalChainPeriodSystemTransactionsUpdate {
                period: 17,
                hashes_rlp: b"system-hashes",
            }),
        )
        .unwrap();

        let level_zero_chunk_id = final_chain_log_bloom_chunk_id(0, 1).unwrap();
        let level_zero_raw = repo
            .log_blooms_chunk_raw(level_zero_chunk_id)
            .unwrap()
            .unwrap();
        let level_zero_chunk = decode_final_chain_log_bloom_chunk(Some(&level_zero_raw)).unwrap();
        assert_eq!(level_zero_chunk[1], bloom);

        let level_one_chunk_id = final_chain_log_bloom_chunk_id(1, 0).unwrap();
        let level_one_raw = repo
            .log_blooms_chunk_raw(level_one_chunk_id)
            .unwrap()
            .unwrap();
        let level_one_chunk = decode_final_chain_log_bloom_chunk(Some(&level_one_raw)).unwrap();
        assert_eq!(level_one_chunk[1], bloom);
        assert_eq!(
            repo.meta_value(1).unwrap(),
            Some(17u64.to_le_bytes().to_vec())
        );
        assert_eq!(
            db.get(Column::TrxPeriod, H256::from_low_u64_be(0x7777).as_bytes())
                .unwrap()
                .map(|value| value.to_vec()),
            Some({
                let mut stream = rlp::RlpStream::new_list(2);
                stream.append(&17u64);
                stream.append(&2u32);
                stream.out().to_vec()
            })
        );
        assert_eq!(
            repo.receipt_by_trx_hash(H256::from_low_u64_be(0x7777))
                .unwrap(),
            Some(b"receipt-by-hash".to_vec())
        );
        assert_eq!(
            db.get(Column::PeriodSystemTransactions, &17u64.to_le_bytes())
                .unwrap()
                .map(|value| value.to_vec()),
            Some(b"system-hashes".to_vec())
        );
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
