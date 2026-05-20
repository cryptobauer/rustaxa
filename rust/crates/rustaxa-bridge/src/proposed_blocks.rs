use crate::ffi::rustaxa_ffi::{
    DagHash, ProposedBlockLookup, ProposedBlockPeriodHashes, ProposedBlockSnapshotEntry,
};
use crate::ffi::BridgeProposedBlocks;
use crate::ffi::BridgeStorage;
use anyhow::{anyhow, Context};
use ethereum_types::H256;
use rustaxa_consensus::proposed_blocks::{ProposedBlockPeriod, ProposedBlocks};
use rustaxa_storage::Column;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::pbft::PbftBlockLink;
use std::convert::TryFrom;

/// Creates an empty Rust proposed-block index for the C++ PBFT shim.
pub fn create_proposed_blocks_index() -> Box<BridgeProposedBlocks> {
    Box::new(BridgeProposedBlocks(ProposedBlocks::new()))
}

impl BridgeProposedBlocks {
    /// Inserts a proposed PBFT block and returns whether it was newly inserted.
    pub fn proposed_blocks_push(
        &mut self,
        period: u64,
        block_hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> bool {
        self.0.push(period, H256::from(*block_hash), block_rlp)
    }

    /// Marks an existing proposed PBFT block as valid.
    pub fn proposed_blocks_mark_valid(
        &mut self,
        period: u64,
        block_hash: &[u8; 32],
    ) -> Result<(), anyhow::Error> {
        self.0.mark_valid(period, H256::from(*block_hash))
    }

    /// Looks up a proposed PBFT block and its cached validation flag.
    pub fn proposed_blocks_get(&self, period: u64, block_hash: &[u8; 32]) -> ProposedBlockLookup {
        self.0
            .get(period, H256::from(*block_hash))
            .map(|entry| ProposedBlockLookup {
                found: true,
                is_valid: entry.is_valid,
                block_rlp: entry.block_rlp,
            })
            .unwrap_or(ProposedBlockLookup {
                found: false,
                is_valid: false,
                block_rlp: Vec::new(),
            })
    }

    /// Returns whether a proposed PBFT block is present.
    pub fn proposed_blocks_contains(&self, period: u64, block_hash: &[u8; 32]) -> bool {
        self.0.contains(period, H256::from(*block_hash))
    }

    /// Returns cleanup candidates for all periods lower than `period`.
    pub fn proposed_blocks_cleanup_candidates(
        &self,
        period: u64,
    ) -> Vec<ProposedBlockPeriodHashes> {
        self.0
            .cleanup_candidates(period)
            .into_iter()
            .map(|period| ProposedBlockPeriodHashes {
                period: period.period,
                block_hashes: period
                    .block_hashes
                    .into_iter()
                    .map(|hash| DagHash { hash: hash.into() })
                    .collect(),
            })
            .collect()
    }

    /// Restores Rust-owned proposed-block metadata from persisted PBFT block RLPs.
    ///
    /// Inputs:
    /// - `storage`: shared Rust storage handle used to iterate the proposed-PBFT
    ///   column without materializing C++ `PbftBlock` objects.
    ///
    /// Output:
    /// - number of persisted entries that were newly inserted into this index.
    ///
    /// Behavior:
    /// - decodes each stored RLP through the Rust PBFT block codec
    /// - validates that the stored DB key matches the decoded canonical block hash
    /// - preserves the stored RLP bytes in the Rust `ProposedBlocks` index
    /// - returns an error for corrupt RLP, iterator failure, or hash mismatch.
    pub fn proposed_blocks_restore_from_storage(
        &mut self,
        storage: &BridgeStorage,
    ) -> Result<usize, anyhow::Error> {
        let mut proposed_entries = Vec::new();
        for item in storage.0.iter(Column::ProposedPbftBlocks) {
            let (key, block_rlp) = item.context("PROPOSED_BLOCKS_RESTORE_STORAGE_ITER")?;
            proposed_entries.push((key.into_vec(), block_rlp.into_vec()));
        }
        let mut restored = 0;

        for (stored_key, block_rlp) in proposed_entries {
            let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block_rlp))
                .context("PROPOSED_BLOCKS_RESTORE_DECODE_BLOCK")?;
            if stored_key.as_slice() != link.block_hash.as_bytes() {
                return Err(anyhow!(
                    "PROPOSED_BLOCKS_RESTORE_HASH_MISMATCH: stored key {:?} decoded hash {:?}",
                    stored_key,
                    link.block_hash
                ));
            }
            let inserted = self.0.push(link.period, link.block_hash, block_rlp);
            if inserted {
                restored += 1;
            }
        }

        Ok(restored)
    }

    /// Cleans stale proposed PBFT blocks from Rust storage and memory.
    ///
    /// Inputs:
    /// - `storage`: shared Rust storage handle used for proposed-block deletes.
    /// - `period`: first period to keep; all lower periods are removed.
    ///
    /// Output:
    /// - deterministic period/hash groups that were deleted and removed.
    ///
    /// Behavior:
    /// - plans cleanup from the Rust in-memory index without mutation
    /// - deletes all stale proposed-block storage keys in one write batch
    /// - mutates the Rust in-memory index only after storage commit succeeds
    /// - returns an empty list without creating a write batch when no stale
    ///   periods exist.
    pub fn proposed_blocks_cleanup_with_storage(
        &mut self,
        storage: &BridgeStorage,
        period: u64,
    ) -> Result<Vec<ProposedBlockPeriodHashes>, anyhow::Error> {
        let removed = self.0.cleanup_candidates(period);
        if removed.is_empty() {
            return Ok(Vec::new());
        }

        let mut batch = storage.0.create_write_batch();
        for period_hashes in &removed {
            for hash in &period_hashes.block_hashes {
                storage.0.batch_delete_raw(
                    &mut batch,
                    Column::ProposedPbftBlocks,
                    hash.as_bytes(),
                )?;
            }
        }
        storage.0.commit_write_batch_with_sync(batch, false)?;

        for period_hashes in &removed {
            self.0.remove_period(period_hashes.period);
        }

        Ok(removed
            .into_iter()
            .map(|value| ProposedBlockPeriodHashes {
                period: value.period,
                block_hashes: value
                    .block_hashes
                    .into_iter()
                    .map(|hash| DagHash { hash: hash.into() })
                    .collect(),
            })
            .collect())
    }

    /// Removes one period from the in-memory proposed-block index.
    pub fn proposed_blocks_remove_period(&mut self, period: u64) {
        self.0.remove_period(period);
    }

    /// Returns the legacy old-blocks diagnostic string.
    pub fn proposed_blocks_old_blocks_message(&self, current_period: u64) -> String {
        self.0
            .old_blocks_message(current_period)
            .unwrap_or_default()
    }

    /// Returns all proposed PBFT block entries with validation flags.
    pub fn proposed_blocks_snapshot_entries(&self) -> Vec<ProposedBlockSnapshotEntry> {
        self.0
            .snapshot_entries()
            .into_iter()
            .map(|entry| ProposedBlockSnapshotEntry {
                period: entry.period,
                block_hash: entry.block_hash.into(),
                block_rlp: entry.block_rlp,
                is_valid: entry.is_valid,
            })
            .collect()
    }

    /// Returns all proposed PBFT block hashes grouped by period.
    pub fn proposed_blocks_snapshot(&self) -> Vec<ProposedBlockPeriodHashes> {
        self.0.snapshot().into_iter().map(Into::into).collect()
    }
}

impl From<ProposedBlockPeriod> for ProposedBlockPeriodHashes {
    fn from(value: ProposedBlockPeriod) -> Self {
        Self {
            period: value.period,
            block_hashes: value
                .blocks
                .into_iter()
                .map(|entry| DagHash {
                    hash: entry.block_hash.into(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::create_storage;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}_{nonce}"))
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
    fn restore_from_storage_decodes_pbft_links_and_inserts_candidates() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_proposed_blocks_restore");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");

            let (rlp_0, link_0) = proposed_link_and_hash(9, 12_345);
            let (rlp_1, link_1) = proposed_link_and_hash(10, 12_346);

            storage
                .save_proposed_pbft_block(&link_0.block_hash.0, rlp_0)
                .expect("proposed block 0 should save");
            storage
                .save_proposed_pbft_block(&link_1.block_hash.0, rlp_1)
                .expect("proposed block 1 should save");

            let mut index = create_proposed_blocks_index();
            let restored = index
                .proposed_blocks_restore_from_storage(&storage)
                .expect("restore should decode and restore");
            let snapshot = index.proposed_blocks_snapshot();

            assert_eq!(restored, 2);
            assert_eq!(snapshot.len(), 2);
            assert_eq!(snapshot[0].period, link_0.period);
            assert_eq!(snapshot[0].block_hashes.len(), 1);
            assert_eq!(snapshot[0].block_hashes[0].hash, link_0.block_hash.0);
            assert_eq!(snapshot[1].period, link_1.period);
            assert_eq!(snapshot[1].block_hashes.len(), 1);
            assert_eq!(snapshot[1].block_hashes[0].hash, link_1.block_hash.0);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn cleanup_with_storage_deletes_only_stale_periods_with_single_batch_semantics() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_proposed_blocks_cleanup_storage");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");

            let (rlp_old, old_link) = proposed_link_and_hash(1, 42_001);
            let (rlp_new, new_link) = proposed_link_and_hash(3, 42_002);

            storage
                .save_proposed_pbft_block(&old_link.block_hash.0, rlp_old)
                .expect("old proposal should save");
            storage
                .save_proposed_pbft_block(&new_link.block_hash.0, rlp_new)
                .expect("new proposal should save");

            let mut index = create_proposed_blocks_index();
            index
                .proposed_blocks_restore_from_storage(&storage)
                .expect("restore for baseline");
            let removed = index
                .proposed_blocks_cleanup_with_storage(&storage, 2)
                .expect("cleanup should succeed");

            assert_eq!(removed.len(), 1);
            assert_eq!(removed[0].period, old_link.period);
            assert_eq!(removed[0].block_hashes.len(), 1);
            assert_eq!(removed[0].block_hashes[0].hash, old_link.block_hash.0);

            let remaining = index.proposed_blocks_snapshot();
            assert_eq!(remaining.len(), 1);
            assert_eq!(remaining[0].period, new_link.period);
            assert_eq!(remaining[0].block_hashes.len(), 1);
            assert_eq!(remaining[0].block_hashes[0].hash, new_link.block_hash.0);

            let no_removed = index
                .proposed_blocks_cleanup_with_storage(&storage, 3)
                .expect("cleanup no-op should succeed");
            assert!(no_removed.is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn restore_from_storage_rejects_hash_mismatched_storage_key() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_proposed_blocks_restore_bad_key");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let (rlp, link) = proposed_link_and_hash(11, 43_001);
            let wrong_hash = H256::from_low_u64_be(999);
            assert_ne!(wrong_hash, link.block_hash);
            storage
                .save_proposed_pbft_block(&wrong_hash.0, rlp)
                .expect("mismatched proposed block key should save");

            let mut index = create_proposed_blocks_index();
            let err = index
                .proposed_blocks_restore_from_storage(&storage)
                .expect_err("restore should reject key/hash mismatch");

            assert!(err
                .to_string()
                .contains("PROPOSED_BLOCKS_RESTORE_HASH_MISMATCH"));
            assert!(index.proposed_blocks_snapshot().is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
