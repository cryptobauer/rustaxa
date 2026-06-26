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
    PillarBlockFinalizationFact as FfiPillarBlockFinalizationFact,
    PillarBlockFinalizationPlan as FfiPillarBlockFinalizationPlan,
    PillarBlockLinkageFact as FfiPillarBlockLinkageFact,
    PillarBlockLinkagePlan as FfiPillarBlockLinkagePlan,
    PillarValidatorVoteCount as FfiPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as FfiPillarValidatorVoteCountChange,
};
use crate::ffi::{BridgePillarChainStorage, BridgeStorage};
use anyhow::{Context, Result};
use ethereum_types::{H160, H256};
use rlp::Rlp;
use rustaxa_consensus::{
    load_current_pillar_block_data_storage as consensus_load_current_pillar_block_data_storage,
    load_latest_pillar_block_storage as consensus_load_latest_pillar_block_storage,
    load_own_pillar_block_vote_storage as consensus_load_own_pillar_block_vote_storage,
    load_pillar_period_data_storage as consensus_load_pillar_period_data_storage,
    plan_pillar_block_creation as consensus_plan_pillar_block_creation,
    plan_pillar_block_finalization as consensus_plan_pillar_block_finalization,
    plan_pillar_block_linkage as consensus_plan_pillar_block_linkage,
    plan_pillar_vote_count_changes as consensus_plan_vote_count_changes,
    save_current_pillar_block_data_storage, save_finalized_pillar_block_storage,
    save_own_pillar_block_vote_storage,
    PillarBlockCreationFact as ConsensusPillarBlockCreationFact,
    PillarBlockCreationPlan as ConsensusPillarBlockCreationPlan,
    PillarBlockFinalizationFact as ConsensusPillarBlockFinalizationFact,
    PillarBlockFinalizationPlan as ConsensusPillarBlockFinalizationPlan,
    PillarBlockLinkageFact as ConsensusPillarBlockLinkageFact,
    PillarBlockLinkagePlan as ConsensusPillarBlockLinkagePlan,
    PillarValidatorVoteCount as ConsensusPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as ConsensusPillarValidatorVoteCountChange,
};
use rustaxa_types::pillar::RawPillarBlockData;

const PILLAR_VOTES_POS_IN_PERIOD_DATA: usize = 4;

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

    /// Loads a finalized pillar block by period, returning empty bytes when no
    /// block is stored for that period.
    pub fn pillar_chain_storage_load_block(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self.storage.pillar().rlp(period)?.unwrap_or_default())
    }

    /// Returns canonical `PillarBlockData` RLP for RPC/query materialization.
    ///
    /// Inputs:
    /// - `period`: pillar block period requested by the caller.
    ///
    /// Outputs:
    /// - Empty bytes when either the pillar block or the following period's
    ///   finalized pillar-vote bundle is absent.
    /// - Otherwise `[pillar_block_rlp, optimized_pillar_votes_bundle_rlp]`
    ///   encoded with the compatibility shape used by C++ `PillarBlockData`.
    ///
    /// Invariants and edge behavior:
    /// - Reads go directly through the typed pillar storage handle; no broad
    ///   `BridgeStorage` query method participates after handle construction.
    /// - Vote payloads are preserved as canonical bytes and decoded only by the
    ///   RPC materialization boundary.
    pub fn pillar_chain_storage_block_data_rlp(&self, period: u64) -> Result<Vec<u8>> {
        let Some(pillar_block_rlp) = self.storage.pillar().rlp(period)? else {
            return Ok(Vec::new());
        };
        let period_data = self
            .storage
            .period()
            .data_raw(period + 1)
            .context("PILLAR_BLOCK_DATA_PERIOD_DATA")?;
        if period_data.is_empty() {
            return Ok(Vec::new());
        }

        let period_rlp = Rlp::new(&period_data);
        if period_rlp.item_count()? <= PILLAR_VOTES_POS_IN_PERIOD_DATA {
            return Ok(Vec::new());
        }
        let votes = period_rlp
            .at(PILLAR_VOTES_POS_IN_PERIOD_DATA)
            .context("PILLAR_BLOCK_DATA_VOTES")?;
        if votes.item_count()? == 0 {
            return Ok(Vec::new());
        }

        RawPillarBlockData {
            pillar_block_rlp,
            pillar_votes_bundle_rlp: votes.as_raw().to_vec(),
        }
        .encode_rlp()
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

/// Plans one pillar-block finalization attempt from compact manager facts.
///
/// Inputs:
/// - `fact`: current-block hash/period, selected vote count, requested hash,
///   and latest-finalized hash facts supplied by the C++ executor.
///
/// Outputs:
/// - Stable status and effect booleans. C++ performs network request, storage
///   persistence, cleanup, and event emission only when Rust requests them.
pub fn plan_pillar_block_finalization(
    fact: FfiPillarBlockFinalizationFact,
) -> Result<FfiPillarBlockFinalizationPlan> {
    Ok(FfiPillarBlockFinalizationPlan::from(
        consensus_plan_pillar_block_finalization(finalization_fact_to_consensus(fact)),
    ))
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

fn finalization_fact_to_consensus(
    value: FfiPillarBlockFinalizationFact,
) -> ConsensusPillarBlockFinalizationFact {
    ConsensusPillarBlockFinalizationFact {
        requested_pillar_block_hash: H256::from(value.requested_pillar_block_hash),
        has_current_pillar_block: value.has_current_pillar_block,
        current_period: value.current_period,
        current_hash: H256::from(value.current_hash),
        threshold_met: value.threshold_met,
        block_weight: value.block_weight,
        selected_weight: value.selected_weight,
        selected_vote_count: value.selected_vote_count,
        has_last_finalized_pillar_block: value.has_last_finalized_pillar_block,
        last_finalized_hash: H256::from(value.last_finalized_hash),
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

impl From<ConsensusPillarBlockFinalizationPlan> for FfiPillarBlockFinalizationPlan {
    fn from(value: ConsensusPillarBlockFinalizationPlan) -> Self {
        Self {
            status: value.status.as_u8(),
            return_votes: value.return_votes,
            should_request_votes: value.should_request_votes,
            should_persist: value.should_persist,
            should_emit: value.should_emit,
            current_period: value.current_period,
            block_weight: value.block_weight,
            selected_weight: value.selected_weight,
            selected_vote_count: value.selected_vote_count,
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

    fn period_data_with_pillar_votes_rlp(votes_bundle_rlp: &[u8]) -> Vec<u8> {
        let mut period_data = rlp::RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(votes_bundle_rlp, 1);
        period_data.out().to_vec()
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
    fn bridge_plans_pillar_block_finalization_effects() {
        let requested = hash(0xAA);
        let ready = plan_pillar_block_finalization(FfiPillarBlockFinalizationFact {
            requested_pillar_block_hash: requested,
            has_current_pillar_block: true,
            current_period: 24,
            current_hash: requested,
            threshold_met: true,
            block_weight: 9,
            selected_weight: 7,
            selected_vote_count: 5,
            has_last_finalized_pillar_block: false,
            last_finalized_hash: [0; 32],
        })
        .expect("finalization plan should be built");
        assert_eq!(ready.status, 0);
        assert!(ready.return_votes);
        assert!(ready.should_persist);
        assert!(ready.should_emit);

        let already_finalized = plan_pillar_block_finalization(FfiPillarBlockFinalizationFact {
            requested_pillar_block_hash: requested,
            has_current_pillar_block: true,
            current_period: 24,
            current_hash: requested,
            threshold_met: true,
            block_weight: 9,
            selected_weight: 7,
            selected_vote_count: 5,
            has_last_finalized_pillar_block: true,
            last_finalized_hash: requested,
        })
        .expect("already-finalized plan should be built");
        assert_eq!(already_finalized.status, 4);
        assert!(already_finalized.return_votes);
        assert!(!already_finalized.should_persist);
        assert!(!already_finalized.should_emit);
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
    fn bridge_pillar_block_data_query_reads_raw_components_from_typed_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_block_data");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);

            assert!(pillar_storage
                .pillar_chain_storage_block_data_rlp(10)
                .expect("missing query should succeed")
                .is_empty());

            let pillar_block_rlp = vec![0xC1, 0xA1];
            let mut votes_bundle = rlp::RlpStream::new_list(1);
            votes_bundle.append(&vec![0xB0]);
            let votes_bundle_rlp = votes_bundle.out().to_vec();

            pillar_storage
                .pillar_chain_storage_apply_finalized_block(10, pillar_block_rlp.clone())
                .expect("pillar block should persist");
            storage
                .save_period_data(11, period_data_with_pillar_votes_rlp(&votes_bundle_rlp))
                .expect("period data should persist");

            let encoded = pillar_storage
                .pillar_chain_storage_block_data_rlp(10)
                .expect("query should succeed");
            let decoded = RawPillarBlockData::decode_rlp(&encoded).expect("wrapper should decode");
            assert_eq!(decoded.pillar_block_rlp, pillar_block_rlp);
            assert_eq!(decoded.pillar_votes_bundle_rlp, votes_bundle_rlp);
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
            assert!(pillar_storage
                .pillar_chain_storage_apply_finalized_block(42, Vec::new())
                .expect_err("empty pillar block should reject")
                .to_string()
                .contains("PILLAR_FINALIZED_BLOCK_EMPTY_PAYLOAD"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
