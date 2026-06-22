//! Proposed PBFT block index for the Rust rewrite shim.
//!
//! This module owns the deterministic period/hash cache, proposal payload bytes,
//! and validation flags used by PBFT proposal handling. Legacy C++ behavior is
//! preserved at the domain boundary: duplicate insertions are rejected, validity
//! is tracked per `(period, hash)`, and stale-period cleanup returns removed
//! block hashes. Native storage helpers own proposed-block restore and cleanup
//! batches so the bridge does not iterate or mutate storage columns directly.

use anyhow::{Context, Result, anyhow};
use ethereum_types::H256;
use rustaxa_storage::{Column, Storage};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::pbft::PbftBlockLink;
use std::collections::{BTreeMap, btree_map::Entry};
use std::convert::TryFrom;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProposedBlockState {
    block_rlp: Vec<u8>,
    pivot_hash: H256,
    is_valid: bool,
}

/// Snapshot of one proposed PBFT block entry.
///
/// Inputs/outputs:
/// - `period`: PBFT period that owns the proposal.
/// - `block_hash`: proposed PBFT block hash.
/// - `block_rlp`: serialized PBFT block bytes for bridge reconstruction.
/// - `is_valid`: cached result of expensive PBFT block validation.
///
/// Invariants:
/// - each `(period, block_hash)` pair appears at most once in a
///   `ProposedBlocks` index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedBlockEntry {
    pub period: u64,
    pub block_hash: H256,
    pub block_rlp: Vec<u8>,
    pub pivot_hash: H256,
    pub is_valid: bool,
}

/// Compact metadata for one proposed PBFT block.
///
/// Inputs/outputs:
/// - `period` and `block_hash` identify the proposed block.
/// - `pivot_hash` is decoded from canonical RLP during storage restore or
///   supplied by the temporary C++ sidecar during live insertion.
/// - `is_valid` is the Rust-owned validation cache bit.
///
/// Invariants:
/// - The metadata is available without returning block RLP to C++ callers that
///   only need compact facts for ranking or cached-valid decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProposedBlockMetadata {
    pub period: u64,
    pub block_hash: H256,
    pub pivot_hash: H256,
    pub is_valid: bool,
}

/// Proposed block hashes grouped by PBFT period.
///
/// This payload is intended for cleanup and DB-delete callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedBlockPeriodHashes {
    pub period: u64,
    pub block_hashes: Vec<H256>,
}

/// Proposed blocks grouped by PBFT period.
///
/// Period order is ascending. Block order within each period is ascending by
/// block hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedBlockPeriod {
    pub period: u64,
    pub blocks: Vec<ProposedBlockEntry>,
}

/// Proposed PBFT block restored from native storage.
///
/// The payload carries the decoded period/hash facts and preserves the stored
/// canonical block bytes for the live proposed-block index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedBlockStorageEntry {
    /// PBFT period decoded from the stored proposed block.
    pub period: u64,
    /// Canonical PBFT block hash decoded from the stored proposed block.
    pub block_hash: H256,
    /// Pivot DAG block hash decoded from the stored proposed block.
    pub pivot_hash: H256,
    /// Stored signed PBFT block RLP bytes.
    pub block_rlp: Vec<u8>,
}

/// Rust-owned proposed PBFT block index.
///
/// The index tracks proposal membership, proposal payload bytes, and validation
/// flag per period/hash. Persistence is intentionally external: callers decide
/// when to write or delete DB entries and can use cleanup/snapshot outputs to
/// drive those side effects.
#[derive(Debug, Default, Clone)]
pub struct ProposedBlocks {
    blocks: BTreeMap<u64, BTreeMap<H256, ProposedBlockState>>,
}

impl ProposedBlocks {
    /// Creates an empty proposed block index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a proposed block with an initially invalid validation cache bit.
    ///
    /// Returns `true` when the block was newly inserted and `false` when the
    /// same period/hash already existed. Existing entries are not overwritten.
    pub fn push(
        &mut self,
        period: u64,
        block_hash: H256,
        pivot_hash: H256,
        block_rlp: Vec<u8>,
    ) -> bool {
        match self.blocks.entry(period).or_default().entry(block_hash) {
            Entry::Vacant(entry) => {
                entry.insert(ProposedBlockState {
                    block_rlp,
                    pivot_hash,
                    is_valid: false,
                });
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    /// Marks an existing proposed block as valid.
    ///
    /// Returns an error when the block is absent. The C++ legacy code asserted
    /// this invariant; the Rust rewrite reports it explicitly instead of
    /// dereferencing an invalid iterator.
    pub fn mark_valid(&mut self, period: u64, block_hash: H256) -> Result<()> {
        let valid = self
            .blocks
            .get_mut(&period)
            .and_then(|period_blocks| period_blocks.get_mut(&block_hash));
        let Some(valid) = valid else {
            return Err(anyhow!(
                "cannot mark missing proposed PBFT block as valid: period {period}, hash {block_hash:#x}"
            ));
        };
        valid.is_valid = true;
        Ok(())
    }

    /// Returns the proposed block entry for `period` and `block_hash`.
    pub fn get(&self, period: u64, block_hash: H256) -> Option<ProposedBlockEntry> {
        self.blocks
            .get(&period)
            .and_then(|period_blocks| period_blocks.get(&block_hash))
            .map(|state| ProposedBlockEntry {
                period,
                block_hash,
                block_rlp: state.block_rlp.clone(),
                pivot_hash: state.pivot_hash,
                is_valid: state.is_valid,
            })
    }

    /// Returns compact metadata for `period` and `block_hash` without block RLP.
    pub fn metadata(&self, period: u64, block_hash: H256) -> Option<ProposedBlockMetadata> {
        self.blocks
            .get(&period)
            .and_then(|period_blocks| period_blocks.get(&block_hash))
            .map(|state| ProposedBlockMetadata {
                period,
                block_hash,
                pivot_hash: state.pivot_hash,
                is_valid: state.is_valid,
            })
    }

    /// Returns true when `period` contains `block_hash`.
    pub fn contains(&self, period: u64, block_hash: H256) -> bool {
        self.get(period, block_hash).is_some()
    }

    /// Removes and returns all proposed block hashes with period lower than `period`.
    pub fn cleanup_before(&mut self, period: u64) -> Vec<ProposedBlockPeriodHashes> {
        let old_periods = self
            .blocks
            .keys()
            .copied()
            .take_while(|candidate| *candidate < period)
            .collect::<Vec<_>>();

        old_periods
            .into_iter()
            .filter_map(|old_period| {
                self.blocks
                    .remove(&old_period)
                    .map(|blocks| ProposedBlockPeriodHashes {
                        period: old_period,
                        block_hashes: blocks.into_keys().collect(),
                    })
            })
            .collect()
    }

    /// Returns all cleanup candidate periods and hashes for `< period`.
    ///
    /// This is the non-mutating half of legacy cleanup flow where callers first
    /// delete persisted entries and then drop periods from memory.
    pub fn cleanup_candidates(&self, period: u64) -> Vec<ProposedBlockPeriodHashes> {
        self.blocks
            .iter()
            .take_while(|(candidate, _)| **candidate < period)
            .map(|(candidate, hashes)| ProposedBlockPeriodHashes {
                period: *candidate,
                block_hashes: hashes.keys().copied().collect(),
            })
            .collect()
    }

    /// Removes one period from the in-memory cache.
    pub fn remove_period(&mut self, period: u64) {
        self.blocks.remove(&period);
    }

    /// Returns all proposed blocks grouped by ascending period.
    pub fn snapshot(&self) -> Vec<ProposedBlockPeriod> {
        self.blocks
            .iter()
            .map(|(period, blocks)| ProposedBlockPeriod {
                period: *period,
                blocks: blocks
                    .iter()
                    .map(|(block_hash, state)| ProposedBlockEntry {
                        period: *period,
                        block_hash: *block_hash,
                        block_rlp: state.block_rlp.clone(),
                        pivot_hash: state.pivot_hash,
                        is_valid: state.is_valid,
                    })
                    .collect(),
            })
            .collect()
    }

    /// Returns all proposed blocks as a flat list in deterministic order.
    pub fn snapshot_entries(&self) -> Vec<ProposedBlockEntry> {
        self.snapshot()
            .into_iter()
            .flat_map(|period| period.blocks)
            .collect()
    }
}

/// Restores proposed PBFT block facts from native Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
///
/// Outputs:
/// - decoded proposed block entries in storage iteration order.
///
/// Invariants and edge behavior:
/// - Each stored RLP is decoded through the Rust PBFT block codec.
/// - The storage key must match the decoded canonical PBFT block hash.
/// - Corrupt RLP, iterator failure, or hash mismatch is returned before the
///   bridge mutates the in-memory proposed-block index.
pub fn restore_proposed_blocks_from_storage(
    storage: &Storage,
) -> Result<Vec<ProposedBlockStorageEntry>> {
    let mut restored = Vec::new();
    for item in storage.iter(Column::ProposedPbftBlocks) {
        let (key, block_rlp) = item.context("PROPOSED_BLOCKS_RESTORE_STORAGE_ITER")?;
        let stored_key = key.into_vec();
        let block_rlp = block_rlp.into_vec();
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block_rlp))
            .context("PROPOSED_BLOCKS_RESTORE_DECODE_BLOCK")?;
        if stored_key.as_slice() != link.block_hash.as_bytes() {
            return Err(anyhow!(
                "PROPOSED_BLOCKS_RESTORE_HASH_MISMATCH: stored key {:?} decoded hash {:?}",
                stored_key,
                link.block_hash
            ));
        }

        restored.push(ProposedBlockStorageEntry {
            period: link.period,
            block_hash: link.block_hash,
            pivot_hash: link.pivot_dag_block_hash,
            block_rlp,
        });
    }

    Ok(restored)
}

/// Persists one proposed PBFT block through native Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `expected_period`: period observed by the live C++ sidecar.
/// - `expected_hash`: block hash observed by the live C++ sidecar.
/// - `block_rlp`: canonical signed PBFT block bytes to persist.
///
/// Outputs:
/// - Decoded storage entry that should be inserted into the live
///   `ProposedBlocks` index after the storage write succeeds.
///
/// Invariants and edge behavior:
/// - The RLP must decode as a signed PBFT block link.
/// - Decoded period/hash must match the C++ sidecar facts supplied by the
///   caller; mismatches are rejected before storage mutation.
/// - Existing proposed-block rows are overwritten, matching the legacy
///   `DbStorage::saveProposedPbftBlock` put semantics.
pub fn save_proposed_block_storage(
    storage: &Storage,
    expected_period: u64,
    expected_hash: H256,
    block_rlp: &[u8],
) -> Result<ProposedBlockStorageEntry> {
    let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(block_rlp))
        .context("PROPOSED_BLOCKS_SAVE_DECODE_BLOCK")?;
    if link.period != expected_period {
        return Err(anyhow!(
            "PROPOSED_BLOCKS_SAVE_PERIOD_MISMATCH: expected {}, decoded {}",
            expected_period,
            link.period
        ));
    }
    if link.block_hash != expected_hash {
        return Err(anyhow!(
            "PROPOSED_BLOCKS_SAVE_HASH_MISMATCH: expected {:?}, decoded {:?}",
            expected_hash,
            link.block_hash
        ));
    }

    storage
        .pbft()
        .write_proposed(link.block_hash, block_rlp)
        .context("PROPOSED_BLOCKS_SAVE_STORAGE")?;

    Ok(ProposedBlockStorageEntry {
        period: link.period,
        block_hash: link.block_hash,
        pivot_hash: link.pivot_dag_block_hash,
        block_rlp: block_rlp.to_vec(),
    })
}

/// Deletes stale proposed PBFT block rows from native Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `removed`: deterministic stale period/hash groups produced by the
///   `ProposedBlocks` cleanup planner.
///
/// Outputs:
/// - Commits one Rust-owned delete batch when `removed` is non-empty.
///
/// Invariants and edge behavior:
/// - Empty cleanup candidates are a no-op and do not create a write batch.
/// - In-memory cleanup remains the caller's responsibility after this function
///   succeeds, preserving storage-first mutation ordering.
pub fn cleanup_proposed_blocks_storage(
    storage: &Storage,
    removed: &[ProposedBlockPeriodHashes],
) -> Result<()> {
    if removed.is_empty() {
        return Ok(());
    }

    let mut batch = storage.create_write_batch();
    for period_hashes in removed {
        for hash in &period_hashes.block_hashes {
            storage
                .batch_delete_raw(&mut batch, Column::ProposedPbftBlocks, hash.as_bytes())
                .context("PROPOSED_BLOCKS_CLEANUP_DELETE")?;
        }
    }
    storage
        .commit_write_batch_with_sync(batch, false)
        .context("PROPOSED_BLOCKS_CLEANUP_COMMIT")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_storage::{Config, Storage};

    fn hash(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    fn rlp(bytes: &[u8]) -> Vec<u8> {
        bytes.to_vec()
    }

    fn push(blocks: &mut ProposedBlocks, period: u64, block_hash: H256, payload: Vec<u8>) -> bool {
        blocks.push(period, block_hash, hash(period + 100), payload)
    }

    fn temp_storage(name: &str) -> Storage {
        let dir = std::env::temp_dir().join(format!(
            "{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Storage::new(Config::new(dir)).unwrap()
    }

    fn build_signed_pbft_block_rlp(period: u64, timestamp: u64) -> Vec<u8> {
        let mut stream = rlp::RlpStream::new_list(8);
        stream.append(&H256::from_low_u64_be(period));
        stream.append(&H256::from_low_u64_be(period + 1));
        stream.append(&H256::from_low_u64_be(period + 2));
        stream.append(&H256::from_low_u64_be(period + 3));
        stream.append(&period);
        stream.append(&timestamp);
        stream.append(&H256::from_low_u64_be(period + 4));
        stream.append(&vec![0u8; 65]);
        stream.out().to_vec()
    }

    fn proposed_link_and_hash(period: u64, timestamp: u64) -> (Vec<u8>, PbftBlockLink) {
        let rlp = build_signed_pbft_block_rlp(period, timestamp);
        let link =
            PbftBlockLink::try_from(SignedPbftBlockRlp::new(&rlp)).expect("decode should succeed");
        (rlp, link)
    }

    #[test]
    fn push_tracks_membership_and_payload_without_overwriting_existing_entry() {
        let mut blocks = ProposedBlocks::new();

        assert!(push(&mut blocks, 2, hash(10), rlp(&[0xAA])));
        assert!(!push(&mut blocks, 2, hash(10), rlp(&[0xBB])));
        assert!(blocks.contains(2, hash(10)));
        assert!(!blocks.contains(2, hash(11)));

        let entry = blocks.get(2, hash(10)).unwrap();
        assert_eq!(entry.period, 2);
        assert_eq!(entry.block_hash, hash(10));
        assert_eq!(entry.pivot_hash, hash(102));
        assert_eq!(entry.block_rlp, rlp(&[0xAA]));
        assert!(!entry.is_valid);

        let metadata = blocks.metadata(2, hash(10)).unwrap();
        assert_eq!(metadata.period, 2);
        assert_eq!(metadata.block_hash, hash(10));
        assert_eq!(metadata.pivot_hash, hash(102));
        assert!(!metadata.is_valid);
    }

    #[test]
    fn mark_valid_updates_existing_entry_and_rejects_missing_entry() {
        let mut blocks = ProposedBlocks::new();

        assert!(push(&mut blocks, 3, hash(30), rlp(&[0xC0])));
        blocks.mark_valid(3, hash(30)).unwrap();
        assert!(blocks.get(3, hash(30)).unwrap().is_valid);
        assert!(blocks.metadata(3, hash(30)).unwrap().is_valid);

        let err = blocks.mark_valid(3, hash(31)).unwrap_err().to_string();
        assert!(err.contains("missing proposed PBFT block"));
    }

    #[test]
    fn cleanup_removes_only_periods_lower_than_current_period() {
        let mut blocks = ProposedBlocks::new();
        push(&mut blocks, 1, hash(11), rlp(&[0x11]));
        push(&mut blocks, 2, hash(22), rlp(&[0x22]));
        push(&mut blocks, 3, hash(33), rlp(&[0x33]));

        let removed = blocks.cleanup_before(3);
        assert_eq!(removed.len(), 2);
        assert_eq!(removed[0].period, 1);
        assert_eq!(removed[0].block_hashes, vec![hash(11)]);
        assert_eq!(removed[1].period, 2);
        assert_eq!(removed[1].block_hashes, vec![hash(22)]);
        assert!(!blocks.contains(1, hash(11)));
        assert!(blocks.contains(3, hash(33)));
    }

    #[test]
    fn snapshot_and_entries_preserve_payload_and_validation_state() {
        let mut blocks = ProposedBlocks::new();
        push(&mut blocks, 1, hash(1), rlp(&[0xA1]));
        push(&mut blocks, 1, hash(2), rlp(&[0xA2]));
        push(&mut blocks, 2, hash(3), rlp(&[0xB3]));
        blocks.mark_valid(1, hash(2)).unwrap();

        let periods = blocks.snapshot();
        assert_eq!(periods.len(), 2);
        assert_eq!(periods[0].period, 1);
        assert_eq!(periods[0].blocks.len(), 2);
        assert_eq!(periods[1].period, 2);
        assert_eq!(periods[1].blocks.len(), 1);

        let entries = blocks.snapshot_entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].block_hash, hash(1));
        assert!(!entries[0].is_valid);
        assert_eq!(entries[1].block_hash, hash(2));
        assert!(entries[1].is_valid);
        assert_eq!(entries[2].block_hash, hash(3));
        assert_eq!(entries[2].pivot_hash, hash(102));
        assert_eq!(entries[2].block_rlp, rlp(&[0xB3]));
    }

    #[test]
    fn restore_proposed_blocks_from_storage_decodes_and_validates_keys() {
        let storage = temp_storage("rustaxa_consensus_proposed_blocks_restore");
        let (rlp_0, link_0) = proposed_link_and_hash(9, 12_345);
        let (rlp_1, link_1) = proposed_link_and_hash(10, 12_346);

        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::ProposedPbftBlocks,
                link_0.block_hash.as_bytes(),
                &rlp_0,
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::ProposedPbftBlocks,
                link_1.block_hash.as_bytes(),
                &rlp_1,
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        let restored = restore_proposed_blocks_from_storage(&storage).unwrap();

        assert_eq!(
            restored,
            vec![
                ProposedBlockStorageEntry {
                    period: link_0.period,
                    block_hash: link_0.block_hash,
                    pivot_hash: link_0.pivot_dag_block_hash,
                    block_rlp: rlp_0,
                },
                ProposedBlockStorageEntry {
                    period: link_1.period,
                    block_hash: link_1.block_hash,
                    pivot_hash: link_1.pivot_dag_block_hash,
                    block_rlp: rlp_1,
                },
            ]
        );
    }

    #[test]
    fn restore_proposed_blocks_from_storage_rejects_hash_mismatched_key() {
        let storage = temp_storage("rustaxa_consensus_proposed_blocks_restore_bad_key");
        let (rlp, link) = proposed_link_and_hash(11, 43_001);
        let wrong_hash = H256::from_low_u64_be(999);
        assert_ne!(wrong_hash, link.block_hash);

        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::ProposedPbftBlocks,
                wrong_hash.as_bytes(),
                &rlp,
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        let err = restore_proposed_blocks_from_storage(&storage)
            .unwrap_err()
            .to_string();

        assert!(err.contains("PROPOSED_BLOCKS_RESTORE_HASH_MISMATCH"));
    }

    #[test]
    fn save_proposed_block_storage_validates_sidecar_facts_before_write() {
        let storage = temp_storage("rustaxa_consensus_proposed_blocks_save");
        let (rlp, link) = proposed_link_and_hash(9, 12_345);

        let saved =
            save_proposed_block_storage(&storage, link.period, link.block_hash, &rlp).unwrap();
        let period_mismatch =
            save_proposed_block_storage(&storage, link.period + 1, link.block_hash, &rlp)
                .unwrap_err()
                .to_string();
        let hash_mismatch = save_proposed_block_storage(&storage, link.period, hash(999), &rlp)
            .unwrap_err()
            .to_string();

        assert_eq!(saved.period, link.period);
        assert_eq!(saved.block_hash, link.block_hash);
        assert_eq!(saved.pivot_hash, link.pivot_dag_block_hash);
        assert_eq!(saved.block_rlp, rlp);
        assert_eq!(
            storage
                .pbft()
                .proposed_rlp()
                .unwrap()
                .into_iter()
                .next()
                .unwrap(),
            rlp
        );
        assert!(period_mismatch.contains("PROPOSED_BLOCKS_SAVE_PERIOD_MISMATCH"));
        assert!(hash_mismatch.contains("PROPOSED_BLOCKS_SAVE_HASH_MISMATCH"));
    }

    #[test]
    fn cleanup_proposed_blocks_storage_deletes_candidates_only_after_commit() {
        let storage = temp_storage("rustaxa_consensus_proposed_blocks_cleanup");
        let (old_rlp, old_link) = proposed_link_and_hash(1, 42_001);
        let (new_rlp, new_link) = proposed_link_and_hash(3, 42_002);

        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::ProposedPbftBlocks,
                old_link.block_hash.as_bytes(),
                &old_rlp,
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::ProposedPbftBlocks,
                new_link.block_hash.as_bytes(),
                &new_rlp,
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        cleanup_proposed_blocks_storage(
            &storage,
            &[ProposedBlockPeriodHashes {
                period: old_link.period,
                block_hashes: vec![old_link.block_hash],
            }],
        )
        .unwrap();

        assert!(
            storage
                .get_raw(Column::ProposedPbftBlocks, old_link.block_hash.as_bytes())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            storage
                .get_raw(Column::ProposedPbftBlocks, new_link.block_hash.as_bytes())
                .unwrap(),
            Some(new_rlp)
        );
        cleanup_proposed_blocks_storage(&storage, &[]).unwrap();
    }
}
