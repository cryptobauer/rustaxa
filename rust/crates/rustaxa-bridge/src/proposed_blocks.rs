use crate::ffi::rustaxa_ffi::{
    DagHash, ProposedBlockLookup, ProposedBlockMetadataLookup, ProposedBlockPeriodHashes,
    ProposedBlockSnapshotEntry,
};
use crate::ffi::BridgeProposedBlocks;
use crate::ffi::BridgeStorage;
use anyhow::{anyhow, Context};
use ethereum_types::H256;
use rustaxa_consensus::proposed_blocks::{
    cleanup_proposed_blocks_storage, restore_proposed_blocks_from_storage,
    save_proposed_block_storage, ProposedBlockPeriod, ProposedBlocks,
};
use rustaxa_storage::Storage;
use std::sync::Arc;

/// Creates an empty Rust proposed-block index for the C++ PBFT shim.
pub fn create_proposed_blocks_index() -> Box<BridgeProposedBlocks> {
    Box::new(BridgeProposedBlocks {
        index: ProposedBlocks::new(),
        storage: None,
    })
}

/// Creates a Rust proposed-block index bound to a shared Rust storage handle.
///
/// The runtime clones the storage owner from `BridgeStorage` during
/// construction, so C++ can preserve `DbStorage` lifetime ownership without
/// retaining or passing a generic bridge storage pointer for each proposed-block
/// persistence operation.
pub fn create_proposed_blocks_index_from_storage(
    storage: &BridgeStorage,
) -> Box<BridgeProposedBlocks> {
    Box::new(BridgeProposedBlocks {
        index: ProposedBlocks::new(),
        storage: Some(storage.0.clone()),
    })
}

impl BridgeProposedBlocks {
    /// Inserts a proposed PBFT block and returns whether it was newly inserted.
    pub fn proposed_blocks_push(
        &mut self,
        period: u64,
        block_hash: &[u8; 32],
        pivot_hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> bool {
        self.index.push(
            period,
            H256::from(*block_hash),
            H256::from(*pivot_hash),
            block_rlp,
        )
    }

    /// Persists a proposed PBFT block through Rust storage, then inserts it into
    /// the Rust-owned live index.
    ///
    /// Storage is committed before live index mutation so failed writes or
    /// sidecar/RLP mismatches cannot leave memory ahead of durable state.
    /// Existing storage rows are overwritten before duplicate detection,
    /// matching the legacy `DbStorage::saveProposedPbftBlock` ordering.
    pub fn proposed_blocks_push_with_storage(
        &mut self,
        period: u64,
        block_hash: &[u8; 32],
        pivot_hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> Result<bool, anyhow::Error> {
        let storage = self.required_storage()?;
        let entry = save_proposed_block_storage(
            storage.as_ref(),
            period,
            H256::from(*block_hash),
            block_rlp.as_slice(),
        )?;
        if entry.pivot_hash != H256::from(*pivot_hash) {
            anyhow::bail!(
                "PROPOSED_BLOCKS_SAVE_PIVOT_MISMATCH: expected {:?}, decoded {:?}",
                H256::from(*pivot_hash),
                entry.pivot_hash
            );
        }
        Ok(self.index.push(
            entry.period,
            entry.block_hash,
            entry.pivot_hash,
            entry.block_rlp,
        ))
    }

    /// Marks an existing proposed PBFT block as valid.
    pub fn proposed_blocks_mark_valid(
        &mut self,
        period: u64,
        block_hash: &[u8; 32],
    ) -> Result<(), anyhow::Error> {
        self.index.mark_valid(period, H256::from(*block_hash))
    }

    /// Looks up a proposed PBFT block and its cached validation flag.
    pub fn proposed_blocks_get(&self, period: u64, block_hash: &[u8; 32]) -> ProposedBlockLookup {
        self.index
            .get(period, H256::from(*block_hash))
            .map(|entry| ProposedBlockLookup {
                found: true,
                is_valid: entry.is_valid,
                pivot_hash: entry.pivot_hash.into(),
                block_rlp: entry.block_rlp,
            })
            .unwrap_or(ProposedBlockLookup {
                found: false,
                is_valid: false,
                pivot_hash: [0; 32],
                block_rlp: Vec::new(),
            })
    }

    /// Looks up compact proposed-block metadata without returning block RLP.
    pub fn proposed_blocks_metadata(
        &self,
        period: u64,
        block_hash: &[u8; 32],
    ) -> ProposedBlockMetadataLookup {
        self.index
            .metadata(period, H256::from(*block_hash))
            .map(|entry| ProposedBlockMetadataLookup {
                found: true,
                is_valid: entry.is_valid,
                pivot_hash: entry.pivot_hash.into(),
            })
            .unwrap_or(ProposedBlockMetadataLookup {
                found: false,
                is_valid: false,
                pivot_hash: [0; 32],
            })
    }

    /// Returns whether a proposed PBFT block is present.
    pub fn proposed_blocks_contains(&self, period: u64, block_hash: &[u8; 32]) -> bool {
        self.index.contains(period, H256::from(*block_hash))
    }

    /// Returns cleanup candidates for all periods lower than `period`.
    pub fn proposed_blocks_cleanup_candidates(
        &self,
        period: u64,
    ) -> Vec<ProposedBlockPeriodHashes> {
        self.index
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
    pub fn proposed_blocks_restore_from_storage(&mut self) -> Result<usize, anyhow::Error> {
        let storage = self.required_storage()?;
        let proposed_entries = restore_proposed_blocks_from_storage(storage.as_ref())?;
        let mut restored = 0;

        for entry in proposed_entries {
            let inserted = self.index.push(
                entry.period,
                entry.block_hash,
                entry.pivot_hash,
                entry.block_rlp,
            );
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
        period: u64,
    ) -> Result<Vec<ProposedBlockPeriodHashes>, anyhow::Error> {
        let removed = self.index.cleanup_candidates(period);
        if removed.is_empty() {
            return Ok(Vec::new());
        }

        let storage = self.required_storage()?;
        cleanup_proposed_blocks_storage(storage.as_ref(), &removed)
            .context("PROPOSED_BLOCKS_CLEANUP_STORAGE")?;

        for period_hashes in &removed {
            self.index.remove_period(period_hashes.period);
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
        self.index.remove_period(period);
    }

    /// Returns the legacy old-blocks diagnostic string.
    pub fn proposed_blocks_old_blocks_message(&self, current_period: u64) -> String {
        self.index
            .old_blocks_message(current_period)
            .unwrap_or_default()
    }

    /// Returns all proposed PBFT block entries with validation flags.
    pub fn proposed_blocks_snapshot_entries(&self) -> Vec<ProposedBlockSnapshotEntry> {
        self.index
            .snapshot_entries()
            .into_iter()
            .map(|entry| ProposedBlockSnapshotEntry {
                period: entry.period,
                block_hash: entry.block_hash.into(),
                pivot_hash: entry.pivot_hash.into(),
                block_rlp: entry.block_rlp,
                is_valid: entry.is_valid,
            })
            .collect()
    }

    /// Returns all proposed PBFT block hashes grouped by period.
    pub fn proposed_blocks_snapshot(&self) -> Vec<ProposedBlockPeriodHashes> {
        self.index.snapshot().into_iter().map(Into::into).collect()
    }

    fn required_storage(&self) -> Result<Arc<Storage>, anyhow::Error> {
        self.storage
            .clone()
            .ok_or_else(|| anyhow!("ProposedBlocks runtime has no Rust storage handle"))
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
    use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
    use rustaxa_types::pbft::PbftBlockLink;
    use std::convert::TryFrom;
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

            let mut index = create_proposed_blocks_index_from_storage(&storage);
            let restored = index
                .proposed_blocks_restore_from_storage()
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
    fn push_with_storage_persists_before_live_index_insert() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_proposed_blocks_push_storage");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let (rlp, link) = proposed_link_and_hash(9, 12_345);
            let mut index = create_proposed_blocks_index_from_storage(&storage);

            let inserted = index
                .proposed_blocks_push_with_storage(
                    link.period,
                    &link.block_hash.0,
                    &link.pivot_dag_block_hash.0,
                    rlp.clone(),
                )
                .expect("push with storage should succeed");
            let duplicate = index
                .proposed_blocks_push_with_storage(
                    link.period,
                    &link.block_hash.0,
                    &link.pivot_dag_block_hash.0,
                    rlp.clone(),
                )
                .expect("duplicate storage put should still succeed");
            let stored = storage
                .0
                .pbft()
                .proposed_rlp()
                .expect("proposed payload should read");

            assert!(inserted);
            assert!(!duplicate);
            assert_eq!(stored, vec![rlp]);
            assert!(index.proposed_blocks_contains(link.period, &link.block_hash.0));
            let metadata = index.proposed_blocks_metadata(link.period, &link.block_hash.0);
            assert!(metadata.found);
            assert_eq!(metadata.pivot_hash, link.pivot_dag_block_hash.0);
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

            let mut index = create_proposed_blocks_index_from_storage(&storage);
            index
                .proposed_blocks_restore_from_storage()
                .expect("restore for baseline");
            let removed = index
                .proposed_blocks_cleanup_with_storage(2)
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
                .proposed_blocks_cleanup_with_storage(3)
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

            let mut index = create_proposed_blocks_index_from_storage(&storage);
            let err = index
                .proposed_blocks_restore_from_storage()
                .expect_err("restore should reject key/hash mismatch");

            assert!(err
                .to_string()
                .contains("PROPOSED_BLOCKS_RESTORE_HASH_MISMATCH"));
            assert!(index.proposed_blocks_snapshot().is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
