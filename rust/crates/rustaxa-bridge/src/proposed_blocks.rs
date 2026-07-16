use crate::ffi::rustaxa_ffi::{
    DagHash, ProposedBlockIdentity, ProposedBlockLookup, ProposedBlockMetadataLookup,
    ProposedBlockPeriodHashes, ProposedBlockSnapshotEntry,
};
use crate::ffi::{BridgePbftService, BridgeStorage};
use anyhow::Context;
use ethereum_types::H256;
use rustaxa_consensus::proposed_blocks::{
    cleanup_proposed_blocks_storage, restore_proposed_blocks_from_storage,
    save_proposed_block_storage, ProposedBlockPeriod, ProposedBlocks,
};

impl BridgePbftService {
    /// Persists a proposed PBFT block through Rust storage, then inserts it into
    /// the Rust-owned live index.
    ///
    /// Storage is committed before live index mutation so failed writes or
    /// sidecar/RLP mismatches cannot leave memory ahead of durable state.
    /// Existing storage rows are overwritten before duplicate detection,
    /// matching the legacy `DbStorage::saveProposedPbftBlock` ordering.
    pub fn pbft_service_proposed_blocks_push_with_storage(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        pivot_hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> Result<bool, anyhow::Error> {
        let storage = self
            .storage
            .as_ref()
            .context("PBFT_SERVICE_STORAGE_UNAVAILABLE")?;
        let mut proposed_blocks = self
            .proposed_blocks
            .write()
            .expect("proposed blocks lock poisoned");
        let entry = save_proposed_block_storage(
            storage.as_ref(),
            period,
            H256::from(*block_hash),
            H256::from(*pivot_hash),
            block_rlp.as_slice(),
        )?;
        Ok(proposed_blocks.push(
            entry.period,
            entry.block_hash,
            entry.pivot_hash,
            entry.block_rlp,
        ))
    }

    /// Marks an existing proposed PBFT block as valid after external validation.
    ///
    /// The period and hash identify an entry in the service-owned index. Missing
    /// entries return an error without mutation. This method performs no block
    /// validation itself and must be called only after the C++ validation
    /// executor succeeds.
    pub fn pbft_service_proposed_blocks_mark_valid(
        &self,
        period: u64,
        block_hash: &[u8; 32],
    ) -> Result<(), anyhow::Error> {
        self.proposed_blocks
            .write()
            .expect("proposed blocks lock poisoned")
            .mark_valid(period, H256::from(*block_hash))
    }

    /// Looks up one proposed PBFT block and its cached validation flag.
    ///
    /// The returned carrier owns canonical RLP bytes for C++ compatibility
    /// materialization. Missing entries produce `found = false` with empty
    /// payload and do not mutate service state.
    pub fn pbft_service_proposed_blocks_get(
        &self,
        period: u64,
        block_hash: &[u8; 32],
    ) -> ProposedBlockLookup {
        self.proposed_blocks
            .read()
            .expect("proposed blocks lock poisoned")
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
    ///
    /// The period and hash identify the entry. Missing entries produce
    /// `found = false`; reads acquire only the proposed-block sibling lock and
    /// never expose an internal Rust reference.
    pub fn pbft_service_proposed_blocks_metadata(
        &self,
        period: u64,
        block_hash: &[u8; 32],
    ) -> ProposedBlockMetadataLookup {
        self.proposed_blocks
            .read()
            .expect("proposed blocks lock poisoned")
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

    /// Returns whether the supplied period/hash exists in service-owned state.
    ///
    /// This read is side-effect free, does not access storage, and acquires no
    /// manager or chain lock.
    pub fn pbft_service_proposed_blocks_contains(
        &self,
        period: u64,
        block_hash: &[u8; 32],
    ) -> bool {
        self.proposed_blocks
            .read()
            .expect("proposed blocks lock poisoned")
            .contains(period, H256::from(*block_hash))
    }

    /// Cleans stale proposed PBFT blocks from Rust storage and memory.
    ///
    /// The bridge uses its internally owned shared Rust storage handle for
    /// proposed-block deletes. `period` is the first period to keep; all lower
    /// periods are removed.
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
    pub fn pbft_service_proposed_blocks_cleanup_with_storage(
        &self,
        period: u64,
    ) -> Result<Vec<ProposedBlockPeriodHashes>, anyhow::Error> {
        let mut proposed_blocks = self
            .proposed_blocks
            .write()
            .expect("proposed blocks lock poisoned");
        let removed = proposed_blocks.cleanup_candidates(period);
        if removed.is_empty() {
            return Ok(Vec::new());
        }

        let storage = self
            .storage
            .as_ref()
            .context("PBFT_SERVICE_STORAGE_UNAVAILABLE")?;
        cleanup_proposed_blocks_storage(storage.as_ref(), &removed)
            .context("PROPOSED_BLOCKS_CLEANUP_STORAGE")?;

        for period_hashes in &removed {
            proposed_blocks.remove_period(period_hashes.period);
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

    /// Returns an owned snapshot of all live proposed-block entries.
    ///
    /// Canonical RLP and validation flags are copied for C++ materialization.
    /// The snapshot is point-in-time and subsequent service mutations do not
    /// change it.
    pub fn pbft_service_proposed_blocks_snapshot_entries(&self) -> Vec<ProposedBlockSnapshotEntry> {
        self.proposed_blocks
            .read()
            .expect("proposed blocks lock poisoned")
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
    #[cfg(test)]
    pub fn proposed_blocks_snapshot(&self) -> Vec<ProposedBlockPeriodHashes> {
        self.proposed_blocks
            .read()
            .expect("proposed blocks lock poisoned")
            .snapshot()
            .into_iter()
            .map(Into::into)
            .collect()
    }
}

/// Persists one proposed block without publishing it to a live PBFT service.
///
/// This compatibility boundary exists only for storage-shim callers. The
/// canonical block bytes are decoded and all supplied identity facts are
/// validated before the storage put. A successful put returns `true`; no
/// process-local duplicate decision is possible without a service index.
pub fn proposed_blocks_storage_push_with_storage(
    storage: &BridgeStorage,
    period: u64,
    block_hash: &[u8; 32],
    pivot_hash: &[u8; 32],
    block_rlp: Vec<u8>,
) -> Result<bool, anyhow::Error> {
    save_proposed_block_storage(
        storage.0.as_ref(),
        period,
        H256::from(*block_hash),
        H256::from(*pivot_hash),
        block_rlp.as_slice(),
    )?;
    Ok(true)
}

/// Reads proposed blocks directly from storage without creating live state.
///
/// The storage shim receives owned canonical entries with `is_valid = false`,
/// because validation flags are process-local service state. Decode, iterator,
/// and key/hash consistency failures are returned to the caller.
pub fn proposed_blocks_storage_snapshot_entries(
    storage: &BridgeStorage,
) -> Result<Vec<ProposedBlockSnapshotEntry>, anyhow::Error> {
    Ok(restore_proposed_blocks_from_storage(storage.0.as_ref())?
        .into_iter()
        .map(storage_entry_into_snapshot)
        .collect())
}

/// Looks up ordered identities in a temporary, non-persisted Rust candidate set.
///
/// Candidate entries and requested identities are consumed; one owned lookup is
/// returned for each identity in the same order. Carrier `is_valid` values are
/// intentionally ignored: tentative wallet candidates must pass the external
/// block-validation callback before leader selection. The local index is dropped
/// before return, never acquires a PBFT service lock, and cannot publish into
/// authoritative or durable proposed-block state.
pub fn proposed_blocks_local_candidate_lookups(
    candidates: Vec<ProposedBlockSnapshotEntry>,
    identities: Vec<ProposedBlockIdentity>,
) -> Vec<ProposedBlockLookup> {
    let mut local = ProposedBlocks::new();
    for candidate in candidates {
        let hash = H256::from(candidate.block_hash);
        local.push(
            candidate.period,
            hash,
            H256::from(candidate.pivot_hash),
            candidate.block_rlp,
        );
    }
    identities
        .into_iter()
        .map(|identity| {
            local
                .get(identity.period, H256::from(identity.block_hash))
                .map(proposed_entry_into_lookup)
                .unwrap_or_else(missing_lookup)
        })
        .collect()
}

fn storage_entry_into_snapshot(
    entry: rustaxa_consensus::proposed_blocks::ProposedBlockStorageEntry,
) -> ProposedBlockSnapshotEntry {
    ProposedBlockSnapshotEntry {
        period: entry.period,
        block_hash: entry.block_hash.into(),
        pivot_hash: entry.pivot_hash.into(),
        block_rlp: entry.block_rlp,
        is_valid: false,
    }
}

fn proposed_entry_into_lookup(
    entry: rustaxa_consensus::proposed_blocks::ProposedBlockEntry,
) -> ProposedBlockLookup {
    ProposedBlockLookup {
        found: true,
        is_valid: entry.is_valid,
        pivot_hash: entry.pivot_hash.into(),
        block_rlp: entry.block_rlp,
    }
}

fn missing_lookup() -> ProposedBlockLookup {
    ProposedBlockLookup {
        found: false,
        is_valid: false,
        pivot_hash: [0; 32],
        block_rlp: Vec::new(),
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

    fn persist_proposed_block(storage: &BridgeStorage, rlp: Vec<u8>, link: &PbftBlockLink) {
        proposed_blocks_storage_push_with_storage(
            storage,
            link.period,
            &link.block_hash.0,
            &link.pivot_dag_block_hash.0,
            rlp,
        )
        .expect("proposed block should save");
    }

    fn service(storage: &BridgeStorage) -> Box<BridgePbftService> {
        crate::pbft_manager::create_pbft_service_from_storage(
            storage,
            crate::ffi::rustaxa_ffi::PbftServiceConfig {
                genesis_lambda_ms: 100,
                cacti_lambda_max_ms: 100,
                cacti_lambda_default_ms: 100,
                cacti_block: u64::MAX,
                max_exponential_lambda_ms: 60_000,
                max_steps: 13,
                deadline_ms: 400,
                polling_interval_ms: 100,
            },
        )
        .expect("service should restore")
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

            persist_proposed_block(&storage, rlp_0, &link_0);
            persist_proposed_block(&storage, rlp_1, &link_1);

            let index = service(&storage);
            let snapshot = index.proposed_blocks_snapshot();

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
            let index = service(&storage);

            let inserted = index
                .pbft_service_proposed_blocks_push_with_storage(
                    link.period,
                    &link.block_hash.0,
                    &link.pivot_dag_block_hash.0,
                    rlp.clone(),
                )
                .expect("push with storage should succeed");
            let duplicate = index
                .pbft_service_proposed_blocks_push_with_storage(
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
            assert!(index.pbft_service_proposed_blocks_contains(link.period, &link.block_hash.0));
            let metadata =
                index.pbft_service_proposed_blocks_metadata(link.period, &link.block_hash.0);
            assert!(metadata.found);
            assert_eq!(metadata.pivot_hash, link.pivot_dag_block_hash.0);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn rejected_persistence_input_never_publishes_storage_or_live_state() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_proposed_blocks_atomic_failure");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let (rlp, link) = proposed_link_and_hash(9, 12_345);
            let service = service(&storage);
            let wrong_pivot = H256::from_low_u64_be(999);

            let error = service
                .pbft_service_proposed_blocks_push_with_storage(
                    link.period,
                    &link.block_hash.0,
                    &wrong_pivot.0,
                    rlp,
                )
                .expect_err("pivot mismatch must fail before storage");

            assert!(error
                .to_string()
                .contains("PROPOSED_BLOCKS_SAVE_PIVOT_MISMATCH"));
            assert!(!service.pbft_service_proposed_blocks_contains(link.period, &link.block_hash.0));
            assert!(storage.0.pbft().proposed_rlp().unwrap().is_empty());
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn local_candidate_lookup_does_not_publish_authoritative_state() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_proposed_blocks_local_candidates");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let (rlp, link) = proposed_link_and_hash(17, 12_348);
            let service = service(&storage);

            let lookups = proposed_blocks_local_candidate_lookups(
                vec![ProposedBlockSnapshotEntry {
                    period: link.period,
                    block_hash: link.block_hash.0,
                    pivot_hash: link.pivot_dag_block_hash.0,
                    block_rlp: rlp.clone(),
                    is_valid: true,
                }],
                vec![
                    ProposedBlockIdentity {
                        period: link.period,
                        block_hash: link.block_hash.0,
                    },
                    ProposedBlockIdentity {
                        period: link.period,
                        block_hash: H256::from_low_u64_be(404).0,
                    },
                ],
            );

            assert_eq!(lookups.len(), 2);
            let lookup = &lookups[0];
            assert!(lookup.found);
            assert!(!lookup.is_valid);
            assert_eq!(lookup.block_rlp, rlp);
            assert!(!lookups[1].found);
            assert!(!service.pbft_service_proposed_blocks_contains(link.period, &link.block_hash.0));
            assert!(storage.0.pbft().proposed_rlp().unwrap().is_empty());
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn storage_snapshot_entries_reads_without_mutating_live_index() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_proposed_blocks_storage_snapshot");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let (rlp, link) = proposed_link_and_hash(13, 12_347);

            persist_proposed_block(&storage, rlp.clone(), &link);

            let snapshot = proposed_blocks_storage_snapshot_entries(&storage)
                .expect("storage snapshot should read persisted proposals");

            assert_eq!(snapshot.len(), 1);
            assert_eq!(snapshot[0].period, link.period);
            assert_eq!(snapshot[0].block_hash, link.block_hash.0);
            assert_eq!(snapshot[0].pivot_hash, link.pivot_dag_block_hash.0);
            assert_eq!(snapshot[0].block_rlp, rlp);
            assert!(!snapshot[0].is_valid);
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

            persist_proposed_block(&storage, rlp_old, &old_link);
            persist_proposed_block(&storage, rlp_new, &new_link);

            let index = service(&storage);
            let removed = index
                .pbft_service_proposed_blocks_cleanup_with_storage(2)
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
                .pbft_service_proposed_blocks_cleanup_with_storage(3)
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
                .0
                .pbft()
                .write_proposed(wrong_hash, &rlp)
                .expect("mismatched proposed block key should save");

            let err = crate::pbft_manager::create_pbft_service_from_storage(
                &storage,
                crate::ffi::rustaxa_ffi::PbftServiceConfig {
                    genesis_lambda_ms: 100,
                    cacti_lambda_max_ms: 100,
                    cacti_lambda_default_ms: 100,
                    cacti_block: u64::MAX,
                    max_exponential_lambda_ms: 60_000,
                    max_steps: 13,
                    deadline_ms: 400,
                    polling_interval_ms: 100,
                },
            )
            .err()
            .expect("restore should reject key/hash mismatch");

            assert!(err
                .to_string()
                .contains("PROPOSED_BLOCKS_RESTORE_HASH_MISMATCH"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
