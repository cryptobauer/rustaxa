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
    PillarBlockCreationWithVoteCountsPlan as FfiPillarBlockCreationWithVoteCountsPlan,
    PillarBlockLinkageFact as FfiPillarBlockLinkageFact,
    PillarBlockLinkagePlan as FfiPillarBlockLinkagePlan,
    PillarValidatorVoteCount as FfiPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as FfiPillarValidatorVoteCountChange,
};
use crate::ffi::{BridgePillarChainRuntime, BridgePillarChainStorage, BridgeStorage};
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

/// Creates a Rust-owned pillar-chain runtime for the C++ PillarChainManager
/// shim.
///
/// The runtime owns both the pillar-vote aggregation state and the typed
/// storage handle needed by finalization, so live pillar-manager routes do not
/// pass one bridge handle into another to execute internal consensus behavior.
pub fn create_pillar_chain_runtime(storage: &BridgeStorage) -> Box<BridgePillarChainRuntime> {
    Box::new(BridgePillarChainRuntime {
        storage: storage.0.clone(),
        votes: rustaxa_consensus::PillarVotes::new(),
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

    /// Loads a finalized pillar block by period, returning empty bytes when no
    /// block is stored for that period.
    pub fn pillar_chain_storage_load_block(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self.storage.pillar().rlp(period)?.unwrap_or_default())
    }
}

impl BridgePillarChainRuntime {
    /// Persists current pillar-block sidecar data through the runtime-owned
    /// native Rust storage handle.
    ///
    /// Inputs:
    /// - `data_rlp` is the canonical legacy `CurrentPillarBlockDataDb` payload
    ///   produced by the temporary C++ pillar-block materializer.
    ///
    /// Outputs:
    /// - Commits the current-block sidecar row used for restart recovery.
    ///
    /// Invariants and edge behavior:
    /// - Empty payloads are rejected by the consensus storage helper.
    /// - This method does not mutate runtime vote state or C++ live mirrors; the
    ///   caller remains responsible for installing temporary C++ sidecars until
    ///   the pillar manager facade is retired.
    pub fn pillar_chain_runtime_apply_current_block_data(&self, data_rlp: Vec<u8>) -> Result<()> {
        save_current_pillar_block_data_storage(self.storage.as_ref(), &data_rlp)
    }

    /// Persists this node's own pillar-block vote through the runtime-owned
    /// native Rust storage handle.
    ///
    /// Inputs:
    /// - `vote_rlp` is the canonical legacy `PillarVote` payload selected by the
    ///   temporary C++ vote materializer.
    ///
    /// Outputs:
    /// - Commits the own-vote row used for restart recovery.
    ///
    /// Invariants and edge behavior:
    /// - Empty payloads are rejected by the consensus storage helper.
    /// - Vote admission into the live runtime index remains a separate operation;
    ///   this API only owns the persistence write that used to route through the
    ///   storage-only bridge handle.
    pub fn pillar_chain_runtime_apply_own_vote(&self, vote_rlp: Vec<u8>) -> Result<()> {
        save_own_pillar_block_vote_storage(self.storage.as_ref(), &vote_rlp)
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

/// Plans the shell fields and ordered vote-count deltas used by temporary C++
/// `PillarBlock` materialization.
///
/// Inputs:
/// - `fact`: typed pillar period/config, finalized parent, state root, and
///   bridge root/epoch facts.
/// - `current_vote_counts`: latest DPoS eligible-vote snapshot.
/// - `previous_vote_counts`: snapshot stored with the current pillar block, or
///   an empty vector for the first pillar block.
///
/// Outputs:
/// - Returns CXX-safe hashes and linkage status that C++ uses to construct the
///   current `PillarBlock` object.
/// - Returns legacy-compatible signed vote-count deltas in deterministic order.
///
/// Invariants and edge behavior:
/// - Bridge root and epoch are consumed by Rust planning before C++ uses them.
/// - Linkage planning and vote-count delta planning either both succeed under
///   this API or the pillar block is not materialized.
pub fn plan_pillar_block_creation_with_vote_counts(
    fact: FfiPillarBlockCreationFact,
    current_vote_counts: Vec<FfiPillarValidatorVoteCount>,
    previous_vote_counts: Vec<FfiPillarValidatorVoteCount>,
) -> Result<FfiPillarBlockCreationWithVoteCountsPlan> {
    let creation_plan = consensus_plan_pillar_block_creation(creation_fact_to_consensus(fact))?;
    let current_vote_counts = current_vote_counts
        .into_iter()
        .map(vote_count_to_consensus)
        .collect::<Vec<_>>();
    let previous_vote_counts = previous_vote_counts
        .into_iter()
        .map(vote_count_to_consensus)
        .collect::<Vec<_>>();
    let vote_count_changes = consensus_plan_vote_count_changes(
        current_vote_counts.as_slice(),
        previous_vote_counts.as_slice(),
    )?
    .into_iter()
    .map(FfiPillarValidatorVoteCountChange::from)
    .collect();
    Ok(
        FfiPillarBlockCreationWithVoteCountsPlan::from_creation_plan(
            creation_plan,
            vote_count_changes,
        ),
    )
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

impl FfiPillarBlockCreationWithVoteCountsPlan {
    fn from_creation_plan(
        value: ConsensusPillarBlockCreationPlan,
        vote_count_changes: Vec<FfiPillarValidatorVoteCountChange>,
    ) -> Self {
        Self {
            status: value.status as u8,
            valid: value.valid,
            expected_previous_period: value.expected_previous_period,
            previous_pillar_block_hash: value.previous_pillar_block_hash.0,
            state_root: value.state_root.0,
            bridge_root: value.bridge_root.0,
            bridge_epoch: value.bridge_epoch.0,
            vote_count_changes,
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
        let plan = plan_pillar_block_creation_with_vote_counts(
            FfiPillarBlockCreationFact {
                pillar_block_period: 18,
                state_root: hash(0xA1),
                bridge_root: hash(0xB2),
                bridge_epoch: hash(0xC3),
                first_pillar_block_period: 10,
                pillar_blocks_interval: 8,
                has_last_finalized_pillar_block: true,
                last_finalized_period: 10,
                last_finalized_hash: hash(0xD4),
            },
            vec![vote_count(3, 5), vote_count(1, 7)],
            vec![vote_count(3, 2), vote_count(2, 4)],
        )
        .expect("creation planning should succeed");

        assert!(plan.valid);
        assert_eq!(plan.status, 0);
        assert_eq!(plan.previous_pillar_block_hash, hash(0xD4));
        assert_eq!(plan.state_root, hash(0xA1));
        assert_eq!(plan.bridge_root, hash(0xB2));
        assert_eq!(plan.bridge_epoch, hash(0xC3));
        assert_eq!(plan.vote_count_changes.len(), 3);
        assert_eq!(plan.vote_count_changes[0].address, addr(1));
        assert_eq!(plan.vote_count_changes[0].vote_count_change, 7);
    }

    #[test]
    fn bridge_plans_first_pillar_block_creation_with_null_parent() {
        let plan = plan_pillar_block_creation_with_vote_counts(
            FfiPillarBlockCreationFact {
                pillar_block_period: 10,
                state_root: hash(0xA1),
                bridge_root: hash(0xB2),
                bridge_epoch: hash(0xC3),
                first_pillar_block_period: 10,
                pillar_blocks_interval: 8,
                has_last_finalized_pillar_block: false,
                last_finalized_period: 0,
                last_finalized_hash: [0; 32],
            },
            vec![vote_count(3, 5), vote_count(1, 7)],
            Vec::new(),
        )
        .expect("first creation planning should succeed");

        assert!(plan.valid);
        assert_eq!(plan.status, 1);
        assert_eq!(plan.previous_pillar_block_hash, [0; 32]);
        assert_eq!(plan.vote_count_changes.len(), 2);
        assert_eq!(plan.vote_count_changes[0].address, addr(3));
        assert_eq!(plan.vote_count_changes[0].vote_count_change, 5);
    }

    #[test]
    fn bridge_rejects_creation_when_vote_count_delta_overflows() {
        let err = match plan_pillar_block_creation_with_vote_counts(
            FfiPillarBlockCreationFact {
                pillar_block_period: 10,
                state_root: hash(0xA1),
                bridge_root: hash(0xB2),
                bridge_epoch: hash(0xC3),
                first_pillar_block_period: 10,
                pillar_blocks_interval: 8,
                has_last_finalized_pillar_block: false,
                last_finalized_period: 0,
                last_finalized_hash: [0; 32],
            },
            vec![vote_count(3, u64::from(i32::MAX as u32) + 1)],
            Vec::new(),
        ) {
            Ok(_) => panic!("oversized first-block vote count should be rejected"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("i32 range"));
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
                pillar_storage
                    .pillar_chain_storage_load_current_block_data()
                    .expect("current pillar data should load"),
                vec![0xC1, 0x01],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_own_vote()
                    .expect("own pillar vote should load"),
                vec![0xC1, 0x02],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_block(42)
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
    fn bridge_runtime_applies_manager_pillar_storage_writes() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_storage_helpers");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);
            let pillar_runtime = create_pillar_chain_runtime(&storage);

            pillar_runtime
                .pillar_chain_runtime_apply_current_block_data(vec![0xC2, 0x01])
                .expect("runtime current pillar data should persist");
            pillar_runtime
                .pillar_chain_runtime_apply_own_vote(vec![0xC2, 0x02])
                .expect("runtime own pillar vote should persist");

            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_current_block_data()
                    .expect("current pillar data should load"),
                vec![0xC2, 0x01],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_own_vote()
                    .expect("own pillar vote should load"),
                vec![0xC2, 0x02],
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
                .pillar_chain_storage_load_block(42)
                .expect("pillar block should read")
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
            let pillar_runtime = create_pillar_chain_runtime(&storage);
            assert!(pillar_runtime
                .pillar_chain_runtime_apply_current_block_data(Vec::new())
                .expect_err("empty runtime current data should reject")
                .to_string()
                .contains("PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD"));
            assert!(pillar_runtime
                .pillar_chain_runtime_apply_own_vote(Vec::new())
                .expect_err("empty runtime own vote should reject")
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
