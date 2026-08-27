use anyhow::{Result, bail};
use ethereum_types::{H256, U256};
#[cfg(test)]
use rustaxa_types::FINAL_CHAIN_LOG_BLOOM_BYTES;
use rustaxa_types::FinalChainLogBloom;
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
/// lookup for one transaction in the finalized block. System entries also carry
/// their canonical payload for the `system_transaction` column. The repository
/// commits these rows before `LAST_NUMBER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalChainTransactionIndexUpdate<'a> {
    /// Canonical transaction hash used by both legacy indexes.
    pub transaction_hash: H256,
    /// Zero-based transaction position in the finalized block.
    pub position: u32,
    /// Whether the location points to a system transaction.
    pub is_system: bool,
    /// Canonical system transaction RLP, or `None` for regular transactions.
    pub system_transaction_rlp: Option<&'a [u8]>,
    /// Canonical legacy transaction receipt RLP.
    pub receipt_rlp: &'a [u8],
}

/// Proposal-period DAG-level boundary committed with finalized-block visibility.
///
/// The legacy DAG proposer reads this map with a seek-at-or-after lookup to
/// resolve which PBFT period owns a DAG level. External-EVM publication passes
/// the optional anchor-derived boundary here so Rust can publish the mapping
/// in the same batch as FinalChain block visibility instead of relying on a
/// separate C++ `DbStorage` write after publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalChainProposalPeriodDagLevelUpdate {
    /// Boundary DAG level stored as the column key.
    pub level: u64,
    /// Finalized PBFT period stored as the column value.
    pub period: u64,
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

/// Rust-owned external-EVM publication recovery marker.
///
/// The marker is stored in `final_chain_meta` under a Rust-prefixed key before
/// the external `StateAPI` staged-state commit is attempted. If the process
/// crashes after that commit but before FinalChain storage publication, startup
/// can load this payload, verify the external committed state descriptor, and
/// replay the Rust-owned publication batch without re-executing EVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalChainExternalEvmPendingPublication<'a> {
    /// Opaque Rust consensus payload containing the publication plan and
    /// state-commit authorization facts.
    pub payload: &'a [u8],
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
    [FinalChainLogBloom::ZERO; FINAL_CHAIN_BLOOM_INDEX_SIZE]
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
        *bloom = FinalChainLogBloom::try_from(data).map_err(|error| {
            anyhow::anyhow!("final-chain bloom chunk entry {index} is malformed: {error}")
        })?;
    }
    Ok(chunk)
}

/// Encodes a legacy FinalChain bloom-index chunk as an RLP list of raw blooms.
pub fn encode_final_chain_log_bloom_chunk(chunk: &FinalChainLogBloomChunk) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(FINAL_CHAIN_BLOOM_INDEX_SIZE);
    for bloom in chunk {
        stream.append(&bloom.as_ref());
    }
    stream.out().to_vec()
}

pub struct FinalChainRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> FinalChainRepository<D> {
    const DPOS_SNAPSHOT_KEY_PREFIX: &'static [u8] = b"rustaxa:dpos_snapshot:";
    const ACCOUNT_SNAPSHOT_KEY_PREFIX: &'static [u8] = b"rustaxa:account_snapshot:";
    const EXTERNAL_EVM_PENDING_PUBLICATION_KEY: &'static [u8] =
        b"rustaxa:external_evm_pending_publication";

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

    /// Returns the pending external-EVM publication recovery marker.
    ///
    /// The payload is intentionally opaque to storage. Consensus code owns the
    /// codec and validates it against `StateAPI` restart facts before replaying
    /// publication.
    pub fn external_evm_pending_publication_raw(&self) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(
                Column::FinalChainMeta,
                Self::EXTERNAL_EVM_PENDING_PUBLICATION_KEY,
            )?
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
    /// Atomically removes block lookup rows below `first_to_keep`.
    ///
    /// Missing rows terminate the backwards walk, block zero is retained, and
    /// unrelated receipts, snapshots, and head metadata remain unchanged. The
    /// returned count is the number of removed index triples.
    pub fn prune_block_indexes_before(&self, first_to_keep: u64) -> Result<u64> {
        let mut batch = self.db.create_batch();
        let mut removed = 0u64;
        let mut number = first_to_keep.saturating_sub(1);
        while number > 0 {
            let Some(hash) = self.block_hash_by_number(number)? else {
                break;
            };
            self.db.batch_delete(
                &mut batch,
                Column::FinalChainBlkByNumber,
                &number.to_le_bytes(),
            )?;
            self.db.batch_delete(
                &mut batch,
                Column::FinalChainBlkHashByNumber,
                &number.to_le_bytes(),
            )?;
            self.db
                .batch_delete(&mut batch, Column::FinalChainBlkNumberByHash, &hash)?;
            removed += 1;
            number -= 1;
        }
        self.db.commit_batch(batch)?;
        Ok(removed)
    }

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
            None,
            false,
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
        proposal_period_dag_level_update: Option<FinalChainProposalPeriodDagLevelUpdate>,
        external_evm_pending_publication_clear: bool,
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
        if let Some(update) = proposal_period_dag_level_update {
            if update.period != number {
                bail!(
                    "final-chain proposal-period DAG-level update period {} does not match block number {number}",
                    update.period
                );
            }
            self.db.batch_put(
                &mut batch,
                Column::ProposalPeriodLevelsMap,
                &update.level.to_le_bytes(),
                &update.period.to_le_bytes(),
            )?;
        }
        if external_evm_pending_publication_clear {
            self.db.batch_delete(
                &mut batch,
                Column::FinalChainMeta,
                Self::EXTERNAL_EVM_PENDING_PUBLICATION_KEY,
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

    /// Persists the external-EVM pending publication marker.
    ///
    /// This write is separate from the later block-publication batch because it
    /// must be durable before `StateAPI::transition_state_commit()` is called.
    pub fn write_external_evm_pending_publication(
        &self,
        marker: FinalChainExternalEvmPendingPublication<'_>,
    ) -> Result<()> {
        let mut batch = self.db.create_batch();
        self.db.batch_put(
            &mut batch,
            Column::FinalChainMeta,
            Self::EXTERNAL_EVM_PENDING_PUBLICATION_KEY,
            marker.payload,
        )?;
        self.db.commit_batch_with_sync(batch, true)
    }

    /// Deletes the external-EVM pending publication marker.
    pub fn delete_external_evm_pending_publication(&self) -> Result<()> {
        self.db.delete(
            Column::FinalChainMeta,
            Self::EXTERNAL_EVM_PENDING_PUBLICATION_KEY,
        )
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
            for (stored, added) in chunk[slot].as_mut().iter_mut().zip(update.bloom.as_ref()) {
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
        if let Some(transaction_rlp) = update.system_transaction_rlp {
            if !update.is_system {
                bail!("regular final-chain transaction cannot carry a system transaction payload");
            }
            if transaction_rlp.is_empty() {
                bail!("system final-chain transaction cannot carry an empty canonical payload");
            }
            use tiny_keccak::{Hasher, Keccak};
            let mut hasher = Keccak::v256();
            hasher.update(transaction_rlp);
            let mut canonical_hash = [0u8; 32];
            hasher.finalize(&mut canonical_hash);
            if canonical_hash != update.transaction_hash.0 {
                bail!("system final-chain transaction payload hash does not match its index key");
            }
            self.db.batch_put(
                batch,
                Column::SystemTransaction,
                update.transaction_hash.as_bytes(),
                transaction_rlp,
            )?;
        } else if update.is_system {
            bail!("system final-chain transaction is missing its canonical payload");
        }
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

    /// Persists the raw FinalChain lookup rows used by storage conformance fixtures.
    ///
    /// Inputs are already legacy-compatible encoded bytes for the metadata value,
    /// stored block header, transaction receipt, log-bloom chunk, and
    /// receipt-by-period row. The method commits every row in one native Rust
    /// storage batch so conformance setup no longer needs a CXX-visible generic
    /// batch registry. It is intentionally narrow: production FinalChain
    /// publication should continue using typed block-publication writers.
    #[allow(clippy::too_many_arguments)]
    pub fn write_conformance_lookup_rows(
        &self,
        meta_key: u32,
        meta_value: &[u8],
        block_number: u64,
        block_hash: H256,
        block_header_rlp: &[u8],
        receipt_hash: H256,
        receipt_rlp: &[u8],
        blooms_chunk: H256,
        blooms_rlp: &[u8],
        receipt_period: u64,
        receipts_rlp: &[u8],
    ) -> Result<()> {
        let mut batch = self.db.create_batch();
        self.db.batch_put(
            &mut batch,
            Column::FinalChainMeta,
            &meta_key.to_le_bytes(),
            meta_value,
        )?;
        self.db.batch_put(
            &mut batch,
            Column::FinalChainBlkByNumber,
            &block_number.to_le_bytes(),
            block_header_rlp,
        )?;
        self.db.batch_put(
            &mut batch,
            Column::FinalChainBlkHashByNumber,
            &block_number.to_le_bytes(),
            block_hash.as_bytes(),
        )?;
        self.db.batch_put(
            &mut batch,
            Column::FinalChainBlkNumberByHash,
            block_hash.as_bytes(),
            &block_number.to_le_bytes(),
        )?;
        self.db.batch_put(
            &mut batch,
            Column::FinalChainReceiptByTrxHash,
            receipt_hash.as_bytes(),
            receipt_rlp,
        )?;
        self.db.batch_put(
            &mut batch,
            Column::FinalChainLogBloomsIndex,
            blooms_chunk.as_bytes(),
            blooms_rlp,
        )?;
        self.db.batch_put(
            &mut batch,
            Column::FinalChainReceiptByPeriod,
            &receipt_period.to_le_bytes(),
            receipts_rlp,
        )?;
        self.db.commit_batch(batch)
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
    fn test_external_evm_pending_publication_marker_round_trips_and_deletes() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());

        repo.write_external_evm_pending_publication(FinalChainExternalEvmPendingPublication {
            payload: b"pending-publication",
        })
        .unwrap();
        assert_eq!(
            repo.external_evm_pending_publication_raw().unwrap(),
            Some(b"pending-publication".to_vec())
        );

        repo.delete_external_evm_pending_publication().unwrap();
        assert_eq!(repo.external_evm_pending_publication_raw().unwrap(), None);
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
    fn prune_block_indexes_removes_only_history_below_retained_block() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        for number in 0u64..=3 {
            let hash = H256::from_low_u64_be(number + 10);
            db.put(
                Column::FinalChainBlkByNumber,
                &number.to_le_bytes(),
                &[number as u8],
            );
            db.put(
                Column::FinalChainBlkHashByNumber,
                &number.to_le_bytes(),
                hash.as_bytes(),
            );
            db.put(
                Column::FinalChainBlkNumberByHash,
                hash.as_bytes(),
                &number.to_le_bytes(),
            );
        }

        assert_eq!(repo.prune_block_indexes_before(3).unwrap(), 2);
        assert!(repo.block_header_raw(1).unwrap().is_none());
        assert!(repo.block_header_raw(2).unwrap().is_none());
        assert!(repo.block_header_raw(0).unwrap().is_some());
        assert!(repo.block_header_raw(3).unwrap().is_some());
        assert!(
            repo.block_number_by_hash(H256::from_low_u64_be(11))
                .unwrap()
                .is_none()
        );
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
        chunk[3].as_mut_bytes()[17] = 0x80;
        chunk[3].as_mut_bytes()[99] = 0x02;

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
    fn test_write_conformance_lookup_rows_commits_expected_raw_indexes() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        let block_hash = H256::from_low_u64_be(0x6161);
        let receipt_hash = H256::from_low_u64_be(0x6262);
        let blooms_chunk = H256::from_low_u64_be(0x6363);

        repo.write_conformance_lookup_rows(
            99,
            b"meta",
            42,
            block_hash,
            b"blk",
            receipt_hash,
            b"rcp",
            blooms_chunk,
            b"blm",
            15,
            &[0xc0],
        )
        .unwrap();

        assert_eq!(repo.meta_value(99).unwrap(), Some(b"meta".to_vec()));
        assert_eq!(repo.block_header_raw(42).unwrap(), Some(b"blk".to_vec()));
        assert_eq!(
            repo.block_hash_by_number(42).unwrap(),
            Some(block_hash.as_bytes().to_vec())
        );
        assert_eq!(
            repo.block_number_by_hash(block_hash).unwrap(),
            Some(42u64.to_le_bytes().to_vec())
        );
        assert_eq!(
            repo.receipt_by_trx_hash(receipt_hash).unwrap(),
            Some(b"rcp".to_vec())
        );
        assert_eq!(
            repo.log_blooms_chunk_raw(blooms_chunk).unwrap(),
            Some(b"blm".to_vec())
        );
        assert_eq!(
            db.get(Column::FinalChainReceiptByPeriod, &15u64.to_le_bytes())
                .unwrap()
                .map(|value| value.to_vec()),
            Some(vec![0xc0])
        );
    }

    #[test]
    fn test_external_evm_publication_batch_clears_marker_and_updates_indexes_before_last_number() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        let mut bloom = FinalChainLogBloom::ZERO;
        bloom.as_mut_bytes()[0] = 0x01;
        bloom.as_mut_bytes()[255] = 0x80;
        repo.write_external_evm_pending_publication(FinalChainExternalEvmPendingPublication {
            payload: b"stale-pending-publication",
        })
        .unwrap();

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
                system_transaction_rlp: None,
                receipt_rlp: b"receipt-by-hash",
            }],
            Some(FinalChainPeriodSystemTransactionsUpdate {
                period: 17,
                hashes_rlp: b"system-hashes",
            }),
            None,
            true,
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
        assert_eq!(repo.external_evm_pending_publication_raw().unwrap(), None);
    }

    #[test]
    fn system_transaction_index_rejects_missing_or_mismatched_payload() {
        let db = Arc::new(MockFinalChainStore::new());
        let repo = FinalChainRepository::new(db.clone());
        for (payload, expected) in [(&[][..], "empty"), (&[0xc0][..], "hash")] {
            let mut batch = db.create_batch();
            let error = repo
                .write_transaction_index_update(
                    1,
                    &mut batch,
                    FinalChainTransactionIndexUpdate {
                        transaction_hash: H256::zero(),
                        position: 0,
                        is_system: true,
                        system_transaction_rlp: Some(payload),
                        receipt_rlp: &[],
                    },
                )
                .unwrap_err();
            assert!(error.to_string().contains(expected));
        }
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
