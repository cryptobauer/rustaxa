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
    PillarChainStartupBootstrap as FfiPillarChainStartupBootstrap,
    PillarCurrentAnchorDecisionRequest as FfiPillarCurrentAnchorDecisionRequest,
    PillarCurrentAnchorDecisionResult as FfiPillarCurrentAnchorDecisionResult,
    PillarValidatorVoteCount as FfiPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as FfiPillarValidatorVoteCountChange,
};
use crate::ffi::{
    BridgePillarChainRuntime, BridgePillarChainStorage, BridgeStorage, PillarCurrentAnchorSnapshot,
    SingleVotePreparationRegistry,
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use ethereum_types::{H160, H256};
use rustaxa_consensus::{
    load_current_pillar_block_data_storage as consensus_load_current_pillar_block_data_storage,
    load_latest_pillar_block_storage as consensus_load_latest_pillar_block_storage,
    load_own_pillar_block_vote_storage as consensus_load_own_pillar_block_vote_storage,
    load_pillar_period_data_storage as consensus_load_pillar_period_data_storage,
    plan_pillar_block_creation as consensus_plan_pillar_block_creation,
    plan_pillar_block_linkage as consensus_plan_pillar_block_linkage,
    plan_pillar_consensus_threshold as consensus_plan_pillar_consensus_threshold,
    plan_pillar_current_anchor_decision as consensus_plan_pillar_current_anchor_decision,
    plan_pillar_vote_count_changes as consensus_plan_vote_count_changes,
    save_current_pillar_block_data_storage, save_finalized_pillar_block_storage,
    save_own_pillar_block_vote_storage,
    PillarBlockCreationFact as ConsensusPillarBlockCreationFact,
    PillarBlockCreationPlan as ConsensusPillarBlockCreationPlan,
    PillarBlockLinkageFact as ConsensusPillarBlockLinkageFact,
    PillarBlockLinkagePlan as ConsensusPillarBlockLinkagePlan,
    PillarCurrentAnchor as ConsensusPillarCurrentAnchor,
    PillarCurrentAnchorDecisionRequest as ConsensusPillarCurrentAnchorDecisionRequest,
    PillarValidatorVoteCount as ConsensusPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as ConsensusPillarValidatorVoteCountChange,
};
use rustaxa_types::pillar::{CurrentPillarBlockDataDb, PillarBlock};
use std::sync::RwLock;

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
/// Missing and restored-present snapshots both start at generation zero; zero
/// is a valid process-local baseline, and each successful apply increments it.
/// Malformed persisted current data makes construction fail before a runtime is
/// published.
pub fn create_pillar_chain_runtime(
    storage: &BridgeStorage,
) -> Result<Box<BridgePillarChainRuntime>> {
    let current_data_rlp = consensus_load_current_pillar_block_data_storage(storage.0.as_ref())?;
    let current_anchor = decode_current_anchor_snapshot(current_data_rlp, 0)
        .context("restore current pillar anchor snapshot")?;
    Ok(Box::new(BridgePillarChainRuntime {
        storage: storage.0.clone(),
        votes: rustaxa_consensus::PillarVotes::new(),
        current_anchor: RwLock::new(current_anchor),
        single_vote_preparations: std::sync::Mutex::new(SingleVotePreparationRegistry {
            entries: std::collections::BTreeMap::new(),
        }),
    }))
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
        ensure!(
            !data_rlp.is_empty(),
            "PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD"
        );
        let decoded = CurrentPillarBlockDataDb::decode_rlp(&data_rlp)
            .context("decode current pillar block data before apply")?;
        ensure!(
            decoded.encode_rlp() == data_rlp,
            "current pillar block data must use canonical RLP"
        );
        let current_block_rlp = decoded.pillar_block.encode_rlp();
        let anchor = ConsensusPillarCurrentAnchor {
            period: decoded.pillar_block.period,
            hash: decoded.pillar_block.hash(),
        };
        let mut snapshot = self
            .current_anchor
            .write()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let generation = snapshot
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("current pillar anchor generation overflow"))?;

        // Keep the lock across persistence and publication so no consumer can
        // observe bytes that are not yet durable. A failed write leaves the
        // prior in-memory snapshot unchanged.
        save_current_pillar_block_data_storage(self.storage.as_ref(), &data_rlp)?;
        *snapshot = PillarCurrentAnchorSnapshot {
            anchor: Some(anchor),
            current_data_rlp: data_rlp,
            current_block_rlp,
            generation,
        };
        Ok(())
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

    /// Loads the durable rows required to reconstruct pillar-manager state.
    ///
    /// Inputs:
    /// - Uses the native Rust storage handle owned by this runtime; C++ does not
    ///   choose storage keys or compose a separate storage bridge handle.
    ///
    /// Outputs:
    /// - Returns this node's own vote, current pillar sidecar, latest finalized
    ///   pillar block, and the period-data row following that latest block.
    /// - Missing rows are represented by empty byte vectors.
    ///
    /// Invariants and edge behavior:
    /// - When a latest block exists, Rust decodes its canonical RLP and derives
    ///   the pillar-vote recovery lookup as `latest.period + 1`.
    /// - Malformed latest-block RLP and period overflow are returned as errors,
    ///   preventing startup from silently reconstructing inconsistent state.
    /// - When no latest block exists, no period-data lookup is needed and the
    ///   corresponding output is empty.
    pub fn pillar_chain_runtime_load_startup_bootstrap(
        &self,
    ) -> Result<FfiPillarChainStartupBootstrap> {
        let own_vote_rlp = consensus_load_own_pillar_block_vote_storage(self.storage.as_ref())?;
        let current_block_data_rlp = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?
            .current_data_rlp
            .clone();
        let latest_block_rlp = consensus_load_latest_pillar_block_storage(self.storage.as_ref())?;
        let latest_pillar_votes_period_data_rlp = if latest_block_rlp.is_empty() {
            Vec::new()
        } else {
            let latest_block = PillarBlock::decode_rlp(&latest_block_rlp)?;
            let vote_period = latest_block
                .period
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("latest pillar block period overflow"))?;
            consensus_load_pillar_period_data_storage(self.storage.as_ref(), vote_period)?
        };

        Ok(FfiPillarChainStartupBootstrap {
            own_vote_rlp,
            current_block_data_rlp,
            latest_block_rlp,
            latest_pillar_votes_period_data_rlp,
        })
    }

    /// Plans one operation against the runtime-owned current anchor snapshot.
    ///
    /// The operation tag selects candidate validation, previous-period anchor
    /// selection, or restart-due selection. Unknown tags return an error. The
    /// result includes the exact anchor generation used for the decision.
    pub fn pillar_chain_runtime_plan_current_anchor_decision(
        &self,
        request: FfiPillarCurrentAnchorDecisionRequest,
    ) -> Result<FfiPillarCurrentAnchorDecisionResult> {
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let consensus_request = match request.operation {
            0 => ConsensusPillarCurrentAnchorDecisionRequest::ValidateCandidate {
                candidate_hash: request
                    .has_candidate_hash
                    .then_some(H256::from(request.candidate_hash)),
            },
            1 => ConsensusPillarCurrentAnchorDecisionRequest::SelectPreviousPeriod {
                pbft_period: request.pbft_period,
            },
            2 => ConsensusPillarCurrentAnchorDecisionRequest::RestartPostProcessing {
                pbft_period: request.pbft_period,
                pillar_blocks_interval: request.pillar_blocks_interval,
            },
            operation => bail!("unknown current pillar anchor operation: {operation}"),
        };
        let plan =
            consensus_plan_pillar_current_anchor_decision(snapshot.anchor, consensus_request);
        let (has_current_anchor, current_period, current_hash) = snapshot
            .anchor
            .map(|anchor| (true, anchor.period, anchor.hash.into()))
            .unwrap_or((false, 0, [0; 32]));
        Ok(FfiPillarCurrentAnchorDecisionResult {
            status: plan.status.as_u8(),
            selected: plan.selected,
            has_current_anchor,
            current_period,
            current_hash,
            anchor_generation: snapshot.generation,
        })
    }

    /// Computes the strict-majority threshold from an external total-vote fact.
    pub fn pillar_chain_runtime_consensus_threshold(&self, total_vote_count: u64) -> u64 {
        consensus_plan_pillar_consensus_threshold(total_vote_count)
    }
}

fn decode_current_anchor_snapshot(
    current_data_rlp: Vec<u8>,
    generation: u64,
) -> Result<PillarCurrentAnchorSnapshot> {
    if current_data_rlp.is_empty() {
        return Ok(PillarCurrentAnchorSnapshot {
            anchor: None,
            current_data_rlp,
            current_block_rlp: Vec::new(),
            generation,
        });
    }
    let decoded = CurrentPillarBlockDataDb::decode_rlp(&current_data_rlp)?;
    ensure!(
        decoded.encode_rlp() == current_data_rlp,
        "current pillar block data must use canonical RLP"
    );
    let current_block_rlp = decoded.pillar_block.encode_rlp();
    Ok(PillarCurrentAnchorSnapshot {
        anchor: Some(ConsensusPillarCurrentAnchor {
            period: decoded.pillar_block.period,
            hash: decoded.pillar_block.hash(),
        }),
        current_data_rlp,
        current_block_rlp,
        generation,
    })
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

    fn canonical_current_data(period: u64) -> Vec<u8> {
        CurrentPillarBlockDataDb {
            pillar_block: PillarBlock {
                period,
                state_root: H256::from_low_u64_be(1),
                previous_pillar_block_hash: H256::from_low_u64_be(2),
                bridge_root: H256::from_low_u64_be(3),
                epoch: 4,
                validator_vote_count_changes: Vec::new(),
            },
            vote_counts: Vec::new(),
        }
        .encode_rlp()
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
            let pillar_runtime =
                create_pillar_chain_runtime(&storage).expect("pillar runtime should initialize");
            let current_data = canonical_current_data(41);

            pillar_runtime
                .pillar_chain_runtime_apply_current_block_data(current_data.clone())
                .expect("runtime current pillar data should persist");
            pillar_runtime
                .pillar_chain_runtime_apply_own_vote(vec![0xC2, 0x02])
                .expect("runtime own pillar vote should persist");

            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_current_block_data()
                    .expect("current pillar data should load"),
                current_data,
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
    fn bridge_runtime_loads_pillar_startup_bootstrap_after_restart() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_startup_bootstrap");
        let latest_block = PillarBlock {
            period: 42,
            state_root: H256::from_low_u64_be(1),
            previous_pillar_block_hash: H256::from_low_u64_be(2),
            bridge_root: H256::from_low_u64_be(3),
            epoch: 4,
            validator_vote_count_changes: Vec::new(),
        }
        .encode_rlp();
        let current_data = canonical_current_data(41);

        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let runtime =
                create_pillar_chain_runtime(&storage).expect("pillar runtime should initialize");
            runtime
                .pillar_chain_runtime_apply_current_block_data(current_data.clone())
                .expect("current pillar data should persist");
            runtime
                .pillar_chain_runtime_apply_own_vote(vec![0xC3, 0x02])
                .expect("own vote should persist");
            save_finalized_pillar_block_storage(storage.0.as_ref(), 42, &latest_block)
                .expect("latest pillar block should persist");
            storage
                .0
                .period()
                .write(43, &[0xC3, 0x04])
                .expect("following period data should persist");
        }

        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should reopen");
            let runtime =
                create_pillar_chain_runtime(&storage).expect("pillar runtime should initialize");
            let bootstrap = runtime
                .pillar_chain_runtime_load_startup_bootstrap()
                .expect("runtime should load restart bootstrap");

            assert_eq!(bootstrap.own_vote_rlp, vec![0xC3, 0x02]);
            assert_eq!(bootstrap.current_block_data_rlp, current_data);
            assert_eq!(bootstrap.latest_block_rlp, latest_block);
            assert_eq!(
                bootstrap.latest_pillar_votes_period_data_rlp,
                vec![0xC3, 0x04]
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_startup_bootstrap_is_empty_for_new_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_empty_bootstrap");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let runtime =
                create_pillar_chain_runtime(&storage).expect("pillar runtime should initialize");
            let bootstrap = runtime
                .pillar_chain_runtime_load_startup_bootstrap()
                .expect("empty bootstrap should load");

            assert!(bootstrap.own_vote_rlp.is_empty());
            assert!(bootstrap.current_block_data_rlp.is_empty());
            assert!(bootstrap.latest_block_rlp.is_empty());
            assert!(bootstrap.latest_pillar_votes_period_data_rlp.is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_restores_anchor_and_preserves_decisions_across_restart() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_anchor_restart");
        let current_data = canonical_current_data(41);
        let before;
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_chain_runtime(&storage).unwrap();
            runtime
                .pillar_chain_runtime_apply_current_block_data(current_data.clone())
                .unwrap();
            before = runtime
                .pillar_chain_runtime_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 1,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 42,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();
            assert!(before.selected);
            assert_eq!(before.anchor_generation, 1);
        }
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_chain_runtime(&storage).unwrap();
            let after = runtime
                .pillar_chain_runtime_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 1,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 42,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();
            assert_eq!(after.status, before.status);
            assert_eq!(after.selected, before.selected);
            assert_eq!(after.current_period, before.current_period);
            assert_eq!(after.current_hash, before.current_hash);
            assert_eq!(after.anchor_generation, 0);
            assert_eq!(
                runtime
                    .pillar_chain_runtime_load_startup_bootstrap()
                    .unwrap()
                    .current_block_data_rlp,
                current_data
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn malformed_persisted_current_data_rejects_runtime_factory() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_malformed_current_restore");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            save_current_pillar_block_data_storage(storage.0.as_ref(), &[0xC1, 0x01]).unwrap();
            let error = match create_pillar_chain_runtime(&storage) {
                Ok(_) => panic!("malformed current data must reject construction"),
                Err(error) => error,
            };
            assert!(error
                .to_string()
                .contains("restore current pillar anchor snapshot"));
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn malformed_current_apply_leaves_storage_and_snapshot_unchanged() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_malformed_current_apply");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_chain_runtime(&storage).unwrap();
            let current_data = canonical_current_data(41);
            runtime
                .pillar_chain_runtime_apply_current_block_data(current_data.clone())
                .unwrap();
            let before = runtime
                .pillar_chain_runtime_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 1,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 42,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();

            assert!(runtime
                .pillar_chain_runtime_apply_current_block_data(vec![0xC1, 0x01])
                .is_err());
            let after = runtime
                .pillar_chain_runtime_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 1,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 42,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();
            assert_eq!(after.anchor_generation, before.anchor_generation);
            assert_eq!(after.current_hash, before.current_hash);
            assert_eq!(
                consensus_load_current_pillar_block_data_storage(storage.0.as_ref()).unwrap(),
                current_data
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn current_anchor_bridge_rejects_unknown_tag_and_maps_threshold() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_anchor_tags");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_chain_runtime(&storage).unwrap();
            let missing = runtime
                .pillar_chain_runtime_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 0,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 0,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();
            assert_eq!(missing.status, 1);
            assert!(!missing.has_current_anchor);
            assert!(runtime
                .pillar_chain_runtime_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 99,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 0,
                        pillar_blocks_interval: 0,
                    },
                )
                .is_err());
            assert_eq!(runtime.pillar_chain_runtime_consensus_threshold(0), 1);
            assert_eq!(
                runtime.pillar_chain_runtime_consensus_threshold(u64::MAX),
                (u64::MAX / 2) + 1
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_startup_bootstrap_rejects_malformed_latest_block() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_malformed_bootstrap");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            save_finalized_pillar_block_storage(storage.0.as_ref(), 42, &[0xC1, 0x01])
                .expect("malformed latest bytes should persist opaquely");
            let runtime =
                create_pillar_chain_runtime(&storage).expect("pillar runtime should initialize");

            let error = match runtime.pillar_chain_runtime_load_startup_bootstrap() {
                Ok(_) => panic!("malformed latest block should reject startup"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("six items"));
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
            let pillar_runtime =
                create_pillar_chain_runtime(&storage).expect("pillar runtime should initialize");
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
