//! CXX bridge wrappers for deterministic pillar-chain planning.
//!
//! This bridge exposes storage-free pillar-chain helpers to C++ shims. The
//! boundary accepts plain vote-count and linkage facts, converts them into
//! Rust consensus-domain values, and returns stable CXX payloads. C++ remains
//! responsible for FinalChain queries, `PillarBlock` object construction,
//! event emission, and network side effects. Rust consensus owns the storage
//! writes for pillar rows that this shim routes to `rustaxa-storage`.

use crate::ffi::rustaxa_ffi::{
    PillarBlockCreationFact as FfiPillarBlockCreationFact,
    PillarBlockCreationPlan as FfiPillarBlockCreationPlan,
    PillarBlockLinkageFact as FfiPillarBlockLinkageFact,
    PillarBlockLinkagePlan as FfiPillarBlockLinkagePlan,
    PillarValidatorVoteCount as FfiPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as FfiPillarValidatorVoteCountChange,
};
use crate::ffi::{BridgePillarChainStorage, BridgeStorage};
use anyhow::Result;
use ethereum_types::{H160, H256};
use rustaxa_consensus::{
    load_current_pillar_block_data_storage as consensus_load_current_pillar_block_data_storage,
    load_latest_pillar_block_storage as consensus_load_latest_pillar_block_storage,
    load_own_pillar_block_vote_storage as consensus_load_own_pillar_block_vote_storage,
    load_pillar_period_data_storage as consensus_load_pillar_period_data_storage,
    plan_pillar_block_creation as consensus_plan_pillar_block_creation,
    plan_pillar_block_linkage as consensus_plan_pillar_block_linkage,
    plan_pillar_vote_count_changes as consensus_plan_vote_count_changes,
    save_current_pillar_block_data_storage, save_finalized_pillar_block_storage,
    save_own_pillar_block_vote_storage,
    PillarBlockCreationFact as ConsensusPillarBlockCreationFact,
    PillarBlockCreationPlan as ConsensusPillarBlockCreationPlan,
    PillarBlockLinkageFact as ConsensusPillarBlockLinkageFact,
    PillarBlockLinkagePlan as ConsensusPillarBlockLinkagePlan,
    PillarValidatorVoteCount as ConsensusPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as ConsensusPillarValidatorVoteCountChange,
};

/// Creates a typed pillar-chain storage handle from the generic CXX storage
/// facade.
///
/// The returned handle clones the underlying `Arc<rustaxa_storage::Storage>`.
/// Production C++ pillar-chain code should keep this typed handle and use its
/// methods instead of retaining or passing `BridgeStorage` after construction.
pub fn create_pillar_chain_storage(storage: &BridgeStorage) -> Box<BridgePillarChainStorage> {
    Box::new(BridgePillarChainStorage {
        storage: storage.0.clone(),
    })
}

impl BridgePillarChainStorage {
    /// Persists current pillar-block sidecar data through consensus-owned
    /// storage.
    pub fn pillar_chain_storage_apply_current_block_data(&self, data_rlp: Vec<u8>) -> Result<()> {
        save_current_pillar_block_data_storage(self.storage.as_ref(), &data_rlp)
    }

    /// Persists this node's own pillar-block vote through consensus-owned
    /// storage.
    pub fn pillar_chain_storage_apply_own_vote(&self, vote_rlp: Vec<u8>) -> Result<()> {
        save_own_pillar_block_vote_storage(self.storage.as_ref(), &vote_rlp)
    }

    /// Persists a finalized pillar block through consensus-owned storage.
    pub fn pillar_chain_storage_apply_finalized_block(
        &self,
        period: u64,
        pillar_block_rlp: Vec<u8>,
    ) -> Result<()> {
        save_finalized_pillar_block_storage(self.storage.as_ref(), period, &pillar_block_rlp)
    }

    /// Loads this node's own pillar-block vote bytes, returning empty bytes when
    /// no vote is stored.
    pub fn pillar_chain_storage_load_own_vote(&self) -> Result<Vec<u8>> {
        consensus_load_own_pillar_block_vote_storage(self.storage.as_ref())
    }

    /// Loads current pillar-block sidecar bytes, returning empty bytes when
    /// missing.
    pub fn pillar_chain_storage_load_current_block_data(&self) -> Result<Vec<u8>> {
        consensus_load_current_pillar_block_data_storage(self.storage.as_ref())
    }

    /// Loads the latest finalized pillar block bytes, returning empty bytes when
    /// no finalized pillar block is stored.
    pub fn pillar_chain_storage_load_latest_block(&self) -> Result<Vec<u8>> {
        consensus_load_latest_pillar_block_storage(self.storage.as_ref())
    }

    /// Loads raw period data bytes used by temporary C++ pillar-vote
    /// materialization.
    pub fn pillar_chain_storage_load_period_data(&self, period: u64) -> Result<Vec<u8>> {
        consensus_load_pillar_period_data_storage(self.storage.as_ref(), period)
    }
}

/// Computes ordered validator vote-count changes for a pillar block.
///
/// The C++ shim supplies the current DPoS vote-count snapshot and the previous
/// current-pillar snapshot when one exists. Rust returns legacy-compatible
/// signed deltas without constructing a `PillarBlock`.
pub fn plan_pillar_vote_count_changes(
    current_vote_counts: Vec<FfiPillarValidatorVoteCount>,
    previous_vote_counts: Vec<FfiPillarValidatorVoteCount>,
) -> Result<Vec<FfiPillarValidatorVoteCountChange>> {
    let current_vote_counts = current_vote_counts
        .into_iter()
        .map(vote_count_to_consensus)
        .collect::<Vec<_>>();
    let previous_vote_counts = previous_vote_counts
        .into_iter()
        .map(vote_count_to_consensus)
        .collect::<Vec<_>>();

    Ok(
        consensus_plan_vote_count_changes(&current_vote_counts, &previous_vote_counts)?
            .into_iter()
            .map(FfiPillarValidatorVoteCountChange::from)
            .collect(),
    )
}

/// Validates pillar-block parent linkage and returns an explicit status code.
pub fn plan_pillar_block_linkage(
    fact: FfiPillarBlockLinkageFact,
) -> Result<FfiPillarBlockLinkagePlan> {
    Ok(FfiPillarBlockLinkagePlan::from(
        consensus_plan_pillar_block_linkage(linkage_fact_to_consensus(fact))?,
    ))
}

/// Plans the shell fields used by temporary C++ `PillarBlock` materialization.
///
/// Inputs:
/// - `fact`: typed pillar period/config, finalized parent, state root, and
///   bridge root/epoch facts.
///
/// Outputs:
/// - Returns CXX-safe hashes and linkage status that C++ uses to construct the
///   current `PillarBlock` object.
///
/// Invariants and edge behavior:
/// - Bridge root and epoch are consumed by Rust planning before C++ uses them.
/// - Vote-count deltas remain planned separately in this slice.
pub fn plan_pillar_block_creation(
    fact: FfiPillarBlockCreationFact,
) -> Result<FfiPillarBlockCreationPlan> {
    Ok(FfiPillarBlockCreationPlan::from(
        consensus_plan_pillar_block_creation(creation_fact_to_consensus(fact))?,
    ))
}

/// Persists current pillar-block sidecar data through consensus-owned storage.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
/// - `data_rlp`: C++-encoded `CurrentPillarBlockDataDb` bytes.
///
/// Outputs:
/// - Returns success after the current-pillar singleton row is written.
///
/// Invariants and edge behavior:
/// - The bridge performs no storage writes itself; it only forwards DTO bytes
///   to the consensus storage helper.
/// - C++ still owns live manager mirrors and `PillarBlock` materialization for
///   this slice.
pub fn apply_pillar_current_block_data_storage(
    storage: &BridgeStorage,
    data_rlp: Vec<u8>,
) -> Result<()> {
    save_current_pillar_block_data_storage(storage.0.as_ref(), &data_rlp)
}

/// Persists the local node's own pillar-block vote through consensus storage.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
/// - `vote_rlp`: C++-encoded `PillarVote` bytes for the local node's current
///   pillar-block vote.
///
/// Outputs:
/// - Returns success after the own-vote singleton row is written.
///
/// Invariants and edge behavior:
/// - Vote signing, validation, aggregation, and gossip stay in the C++ shim.
/// - Empty or otherwise invalid payloads are rejected by the consensus helper.
pub fn apply_pillar_own_vote_storage(storage: &BridgeStorage, vote_rlp: Vec<u8>) -> Result<()> {
    save_own_pillar_block_vote_storage(storage.0.as_ref(), &vote_rlp)
}

/// Persists a finalized pillar block through consensus storage.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
/// - `period`: pillar block period used as the storage key.
/// - `pillar_block_rlp`: C++-encoded finalized `PillarBlock` bytes.
///
/// Outputs:
/// - Returns success after the finalized pillar-block row is written.
///
/// Invariants and edge behavior:
/// - Above-threshold vote lookup, event emission, and live finalized/current
///   mirrors remain in the C++ shim for now.
/// - Empty payloads are rejected by the consensus helper.
pub fn apply_finalized_pillar_block_storage(
    storage: &BridgeStorage,
    period: u64,
    pillar_block_rlp: Vec<u8>,
) -> Result<()> {
    save_finalized_pillar_block_storage(storage.0.as_ref(), period, &pillar_block_rlp)
}

/// Loads the local node's own pillar-block vote through consensus storage.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
///
/// Outputs:
/// - Returns C++-decodable `PillarVote` RLP bytes, or empty bytes when missing.
///
/// Invariants and edge behavior:
/// - The bridge performs no storage lookup itself; it forwards the read to the
///   consensus storage helper.
pub fn load_pillar_own_vote_storage(storage: &BridgeStorage) -> Result<Vec<u8>> {
    consensus_load_own_pillar_block_vote_storage(storage.0.as_ref())
}

/// Loads current pillar-block sidecar data through consensus storage.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
///
/// Outputs:
/// - Returns C++-decodable `CurrentPillarBlockDataDb` RLP bytes, or empty bytes
///   when missing.
///
/// Invariants and edge behavior:
/// - C++ remains responsible for decoding and live mirror hydration in this
///   slice.
pub fn load_pillar_current_block_data_storage(storage: &BridgeStorage) -> Result<Vec<u8>> {
    consensus_load_current_pillar_block_data_storage(storage.0.as_ref())
}

/// Loads the latest finalized pillar block through consensus storage.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
///
/// Outputs:
/// - Returns C++-decodable `PillarBlock` RLP bytes, or empty bytes when missing.
///
/// Invariants and edge behavior:
/// - Latest-row ordering is delegated to `rustaxa-storage` via the consensus
///   helper.
pub fn load_latest_pillar_block_storage(storage: &BridgeStorage) -> Result<Vec<u8>> {
    consensus_load_latest_pillar_block_storage(storage.0.as_ref())
}

/// Loads raw period data for pillar-vote recovery through consensus storage.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge handle.
/// - `period`: finalized PBFT period to load.
///
/// Outputs:
/// - Returns raw period-data RLP bytes, or empty bytes when missing.
///
/// Invariants and edge behavior:
/// - Period-data decoding remains in C++ until the surrounding consensus read
///   surface moves to Rust.
pub fn load_pillar_period_data_storage(storage: &BridgeStorage, period: u64) -> Result<Vec<u8>> {
    consensus_load_pillar_period_data_storage(storage.0.as_ref(), period)
}

fn vote_count_to_consensus(
    value: FfiPillarValidatorVoteCount,
) -> ConsensusPillarValidatorVoteCount {
    ConsensusPillarValidatorVoteCount {
        address: H160::from(value.address),
        vote_count: value.vote_count,
    }
}

fn linkage_fact_to_consensus(value: FfiPillarBlockLinkageFact) -> ConsensusPillarBlockLinkageFact {
    ConsensusPillarBlockLinkageFact {
        pillar_block_period: value.pillar_block_period,
        pillar_block_previous_hash: H256::from(value.pillar_block_previous_hash),
        first_pillar_block_period: value.first_pillar_block_period,
        pillar_blocks_interval: value.pillar_blocks_interval,
        last_finalized_period: value
            .has_last_finalized_pillar_block
            .then_some(value.last_finalized_period),
        last_finalized_hash: value
            .has_last_finalized_pillar_block
            .then_some(H256::from(value.last_finalized_hash)),
    }
}

fn creation_fact_to_consensus(
    value: FfiPillarBlockCreationFact,
) -> ConsensusPillarBlockCreationFact {
    ConsensusPillarBlockCreationFact {
        pillar_block_period: value.pillar_block_period,
        state_root: H256::from(value.state_root),
        bridge_root: H256::from(value.bridge_root),
        bridge_epoch: H256::from(value.bridge_epoch),
        first_pillar_block_period: value.first_pillar_block_period,
        pillar_blocks_interval: value.pillar_blocks_interval,
        last_finalized_period: value
            .has_last_finalized_pillar_block
            .then_some(value.last_finalized_period),
        last_finalized_hash: value
            .has_last_finalized_pillar_block
            .then_some(H256::from(value.last_finalized_hash)),
    }
}

impl From<ConsensusPillarValidatorVoteCountChange> for FfiPillarValidatorVoteCountChange {
    fn from(value: ConsensusPillarValidatorVoteCountChange) -> Self {
        Self {
            address: value.address.into(),
            vote_count_change: value.vote_count_change,
        }
    }
}

impl From<ConsensusPillarBlockLinkagePlan> for FfiPillarBlockLinkagePlan {
    fn from(value: ConsensusPillarBlockLinkagePlan) -> Self {
        Self {
            status: value.status.as_u8(),
            valid: value.valid,
            expected_previous_period: value.expected_previous_period,
        }
    }
}

impl From<ConsensusPillarBlockCreationPlan> for FfiPillarBlockCreationPlan {
    fn from(value: ConsensusPillarBlockCreationPlan) -> Self {
        Self {
            status: value.status as u8,
            valid: value.valid,
            expected_previous_period: value.expected_previous_period,
            previous_pillar_block_hash: value.previous_pillar_block_hash.0,
            state_root: value.state_root.0,
            bridge_root: value.bridge_root.0,
            bridge_epoch: value.bridge_epoch.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::create_storage;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn addr(value: u8) -> [u8; 20] {
        [value; 20]
    }

    fn hash(value: u64) -> [u8; 32] {
        H256::from_low_u64_be(value).into()
    }

    fn vote_count(address: u8, vote_count: u64) -> FfiPillarValidatorVoteCount {
        FfiPillarValidatorVoteCount {
            address: addr(address),
            vote_count,
        }
    }

    #[test]
    fn bridge_plans_vote_count_changes() {
        let changes = plan_pillar_vote_count_changes(
            vec![vote_count(3, 5), vote_count(1, 7)],
            vec![vote_count(3, 2), vote_count(2, 4)],
        )
        .unwrap();

        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].address, addr(1));
        assert_eq!(changes[0].vote_count_change, 7);
        assert_eq!(changes[1].address, addr(2));
        assert_eq!(changes[1].vote_count_change, -4);
        assert_eq!(changes[2].address, addr(3));
        assert_eq!(changes[2].vote_count_change, 3);
    }

    #[test]
    fn bridge_plans_pillar_block_linkage() {
        let valid = plan_pillar_block_linkage(FfiPillarBlockLinkageFact {
            pillar_block_period: 8,
            pillar_block_previous_hash: hash(44),
            first_pillar_block_period: 4,
            pillar_blocks_interval: 4,
            has_last_finalized_pillar_block: true,
            last_finalized_period: 4,
            last_finalized_hash: hash(44),
        })
        .unwrap();

        assert!(valid.valid);
        assert_eq!(valid.status, 0);

        let invalid = plan_pillar_block_linkage(FfiPillarBlockLinkageFact {
            pillar_block_period: 8,
            pillar_block_previous_hash: hash(45),
            first_pillar_block_period: 4,
            pillar_blocks_interval: 4,
            has_last_finalized_pillar_block: true,
            last_finalized_period: 4,
            last_finalized_hash: hash(44),
        })
        .unwrap();

        assert!(!invalid.valid);
        assert_eq!(invalid.status, 4);
    }

    #[test]
    fn bridge_plans_pillar_block_creation_with_bridge_facts() {
        let plan = plan_pillar_block_creation(FfiPillarBlockCreationFact {
            pillar_block_period: 18,
            state_root: hash(0xA1),
            bridge_root: hash(0xB2),
            bridge_epoch: hash(0xC3),
            first_pillar_block_period: 10,
            pillar_blocks_interval: 8,
            has_last_finalized_pillar_block: true,
            last_finalized_period: 10,
            last_finalized_hash: hash(0xD4),
        })
        .expect("creation planning should succeed");

        assert!(plan.valid);
        assert_eq!(plan.status, 0);
        assert_eq!(plan.previous_pillar_block_hash, hash(0xD4));
        assert_eq!(plan.state_root, hash(0xA1));
        assert_eq!(plan.bridge_root, hash(0xB2));
        assert_eq!(plan.bridge_epoch, hash(0xC3));
    }

    #[test]
    fn bridge_plans_first_pillar_block_creation_with_null_parent() {
        let plan = plan_pillar_block_creation(FfiPillarBlockCreationFact {
            pillar_block_period: 10,
            state_root: hash(0xA1),
            bridge_root: hash(0xB2),
            bridge_epoch: hash(0xC3),
            first_pillar_block_period: 10,
            pillar_blocks_interval: 8,
            has_last_finalized_pillar_block: false,
            last_finalized_period: 0,
            last_finalized_hash: [0; 32],
        })
        .expect("first creation planning should succeed");

        assert!(plan.valid);
        assert_eq!(plan.status, 1);
        assert_eq!(plan.previous_pillar_block_hash, [0; 32]);
    }

    #[test]
    fn bridge_applies_pillar_storage_writes_through_consensus() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_storage_helpers");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);

            pillar_storage
                .pillar_chain_storage_apply_current_block_data(vec![0xC1, 0x01])
                .expect("current pillar data should persist");
            pillar_storage
                .pillar_chain_storage_apply_own_vote(vec![0xC1, 0x02])
                .expect("own pillar vote should persist");
            pillar_storage
                .pillar_chain_storage_apply_finalized_block(42, vec![0xC1, 0x03])
                .expect("finalized pillar block should persist");

            assert_eq!(
                storage
                    .get_current_pillar_block_data()
                    .expect("current pillar data should load"),
                vec![0xC1, 0x01],
            );
            assert_eq!(
                storage
                    .get_own_pillar_block_vote()
                    .expect("own pillar vote should load"),
                vec![0xC1, 0x02],
            );
            assert_eq!(
                storage
                    .get_pillar_block(42)
                    .expect("pillar block should load"),
                vec![0xC1, 0x03],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_current_block_data()
                    .expect("current pillar data should read"),
                vec![0xC1, 0x01],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_own_vote()
                    .expect("own pillar vote should read"),
                vec![0xC1, 0x02],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_latest_block()
                    .expect("latest pillar block should read"),
                vec![0xC1, 0x03],
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_pillar_storage_reads_return_empty_when_missing() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_storage_missing_reads");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);

            assert!(pillar_storage
                .pillar_chain_storage_load_current_block_data()
                .expect("current pillar data should read")
                .is_empty());
            assert!(pillar_storage
                .pillar_chain_storage_load_own_vote()
                .expect("own pillar vote should read")
                .is_empty());
            assert!(pillar_storage
                .pillar_chain_storage_load_latest_block()
                .expect("latest pillar block should read")
                .is_empty());
            assert!(pillar_storage
                .pillar_chain_storage_load_period_data(42)
                .expect("period data should read")
                .is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_rejects_empty_pillar_storage_payloads() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_storage_empty_payloads");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);

            assert!(pillar_storage
                .pillar_chain_storage_apply_current_block_data(Vec::new())
                .expect_err("empty current data should reject")
                .to_string()
                .contains("PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD"));
            assert!(pillar_storage
                .pillar_chain_storage_apply_own_vote(Vec::new())
                .expect_err("empty own vote should reject")
                .to_string()
                .contains("PILLAR_OWN_VOTE_EMPTY_PAYLOAD"));
            assert!(pillar_storage
                .pillar_chain_storage_apply_finalized_block(42, Vec::new())
                .expect_err("empty pillar block should reject")
                .to_string()
                .contains("PILLAR_FINALIZED_BLOCK_EMPTY_PAYLOAD"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
