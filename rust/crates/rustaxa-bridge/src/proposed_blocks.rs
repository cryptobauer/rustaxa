use crate::ffi::rustaxa_ffi::{
    ProposedBlockIdentity, ProposedBlockLookup, ProposedBlockSnapshotEntry,
};
use crate::ffi::{BridgePbftService, BridgeStorage};
use ethereum_types::H256;
use rustaxa_consensus::proposed_blocks::{
    restore_proposed_blocks_from_storage, save_proposed_block_storage, ProposedBlocks,
};

impl BridgePbftService {
    /// Publishes a proposed PBFT block through the native PBFT service.
    ///
    /// Storage is committed before live index mutation so failed writes or
    /// sidecar/RLP mismatches cannot leave memory ahead of durable state.
    /// Existing live entries return `false` after their durable row is
    /// overwritten. The native service holds one write guard across the
    /// unconditional storage write and live duplicate detection, preserving
    /// the legacy durability and repair ordering.
    pub fn pbft_service_publish_proposed_block(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        pivot_hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> Result<bool, anyhow::Error> {
        self.proposed_blocks().push_with_storage(
            period,
            H256::from(*block_hash),
            H256::from(*pivot_hash),
            block_rlp,
        )
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
        self.proposed_blocks()
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
        self.proposed_blocks()
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

    /// Returns an owned snapshot of all live proposed-block entries.
    ///
    /// Canonical RLP and validation flags are copied for C++ materialization.
    /// The snapshot is point-in-time and subsequent service mutations do not
    /// change it.
    pub fn pbft_service_proposed_blocks_snapshot_entries(&self) -> Vec<ProposedBlockSnapshotEntry> {
        self.proposed_blocks()
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
                report_malicious_behaviour: false,
                magnolia_activation_period: 0,
            },
        )
        .expect("service should restore")
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
            assert!(
                !service
                    .pbft_service_proposed_blocks_get(link.period, &link.block_hash.0)
                    .found
            );
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
}
