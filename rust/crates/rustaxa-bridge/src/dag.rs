use crate::ffi::rustaxa_ffi::{
    DagBlockLookup, DagFrontier, DagHash, DagLevelHashes, DagManagerAnchors, DagManagerBlock,
    DagManagerNonFinalizedSize, DagManagerSnapshot, DagOrder, DagPersistenceCounters,
    DagPivotTipsValidation, DagReferenceMetadata, DagVerifyGasInput, DagVerifyGasResult,
    DagVerifyPrecheckBlock, DagVerifyPrecheckResult, DagVerifyTransactionAvailabilityInput,
    DagVerifyTransactionAvailabilityResult,
};
use crate::ffi::{BridgeDagGraph, BridgeDagManagerRuntime, BridgeDagManagerState, BridgeStorage};
use anyhow::{ensure, Context, Result};
use ethereum_types::H256;
use rustaxa_consensus::dag::{
    derive_frontier, validate_dag_verify_gas, validate_dag_verify_precheck,
    validate_dag_verify_transaction_availability, validate_pivot_tips_metadata, DagGraph,
    DagManagerBlock as DomainDagManagerBlock, DagManagerSnapshot as DomainDagManagerSnapshot,
    DagManagerState, DagReferenceMetadata as ReferenceMetadata, DagTipGas,
    DagVerifyGasInput as DomainDagVerifyGasInput, DagVerifyPrecheckInput,
    DagVerifyTransactionAvailabilityInput as DomainDagVerifyTransactionAvailabilityInput,
};
use rustaxa_storage::StatusField;
use std::collections::{BTreeMap, BTreeSet};

pub fn create_dag_graph(genesis: &[u8; 32]) -> Box<BridgeDagGraph> {
    Box::new(BridgeDagGraph(DagGraph::new(to_h256(genesis))))
}

/// Creates a Rust-owned DagManager state bridge rooted at `genesis`.
///
/// The returned state owns deterministic graph/index data only; C++ remains
/// responsible for DB writes, transaction side effects, events, and networking
/// until those domains are explicitly migrated.
pub fn create_dag_manager_state(
    genesis: &[u8; 32],
    dag_expiry_limit: u32,
) -> Result<Box<BridgeDagManagerState>> {
    Ok(Box::new(BridgeDagManagerState(DagManagerState::new(
        to_h256(genesis),
        dag_expiry_limit,
    )?)))
}

/// Creates a Rust-owned DagManager runtime with direct storage access.
///
/// The runtime owns deterministic graph/index state and a cloned Rust storage
/// handle. C++ callers use it for DagManager persistence so the migration path
/// is `DagManager shim -> Rust DagManager runtime -> rustaxa-storage`, without
/// routing through legacy DagManager storage logic.
pub fn create_dag_manager_runtime_from_storage(
    genesis: &[u8; 32],
    dag_expiry_limit: u32,
    storage: &BridgeStorage,
) -> Result<Box<BridgeDagManagerRuntime>> {
    Ok(Box::new(BridgeDagManagerRuntime {
        state: DagManagerState::new(to_h256(genesis), dag_expiry_limit)?,
        storage: storage.0.clone(),
    }))
}

pub fn dag_derive_frontier(ghost_path: Vec<DagHash>, leaves: Vec<DagHash>) -> DagFrontier {
    let ghost_path = ghost_path
        .into_iter()
        .map(|hash| H256::from(hash.hash))
        .collect::<Vec<_>>();
    let leaves = leaves
        .into_iter()
        .map(|hash| H256::from(hash.hash))
        .collect::<Vec<_>>();
    let frontier = derive_frontier(&ghost_path, &leaves);

    DagFrontier {
        pivot: frontier.pivot.into(),
        tips: to_dag_hashes(frontier.tips),
    }
}

pub fn dag_validate_pivot_tips_metadata(
    block_level: u64,
    pivot: DagReferenceMetadata,
    tips: Vec<DagReferenceMetadata>,
) -> DagPivotTipsValidation {
    let pivot = ReferenceMetadata {
        hash: H256::from(pivot.hash),
        found: pivot.found,
        level: pivot.level,
    };
    let tips = tips
        .into_iter()
        .map(|tip| ReferenceMetadata {
            hash: H256::from(tip.hash),
            found: tip.found,
            level: tip.level,
        })
        .collect::<Vec<_>>();
    let validation = validate_pivot_tips_metadata(block_level, pivot, &tips);

    DagPivotTipsValidation {
        ok: validation.ok,
        expected_level: validation.expected_level,
        level_matches: validation.level_matches,
        missing_references: to_dag_hashes(validation.missing_references),
    }
}

impl BridgeDagGraph {
    pub fn dag_vertex_count(&self) -> usize {
        self.0.vertex_count()
    }

    pub fn dag_edge_count(&self) -> usize {
        self.0.edge_count()
    }

    pub fn dag_has_vertex(&self, vertex: &[u8; 32]) -> bool {
        self.0.has_vertex(to_h256(vertex))
    }

    pub fn dag_add_vertex_edges(
        &mut self,
        new_vertex: &[u8; 32],
        pivot: &[u8; 32],
        tips: Vec<DagHash>,
    ) -> bool {
        let tips = tips
            .iter()
            .map(|tip| H256::from(tip.hash))
            .collect::<Vec<_>>();
        self.0
            .add_vertex_edges(to_h256(new_vertex), to_h256(pivot), &tips)
    }

    pub fn dag_leaves(&self) -> Vec<DagHash> {
        to_dag_hashes(self.0.leaves())
    }

    pub fn dag_ghost_path(&self, root: &[u8; 32]) -> Vec<DagHash> {
        to_dag_hashes(self.0.ghost_path(to_h256(root)))
    }

    pub fn dag_compute_order(
        &self,
        anchor: &[u8; 32],
        non_finalized_blocks: Vec<DagLevelHashes>,
    ) -> DagOrder {
        let non_finalized_blocks = non_finalized_blocks
            .into_iter()
            .map(|level_hashes| {
                (
                    level_hashes.level,
                    level_hashes
                        .hashes
                        .into_iter()
                        .map(|hash| H256::from(hash.hash))
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();

        match self.0.compute_order(to_h256(anchor), &non_finalized_blocks) {
            Some(hashes) => DagOrder {
                found: true,
                hashes: to_dag_hashes(hashes),
            },
            None => DagOrder {
                found: false,
                hashes: Vec::new(),
            },
        }
    }

    pub fn dag_clear(&mut self) {
        self.0.clear();
    }

    pub fn dag_graphviz_dot(&self) -> String {
        self.0.graphviz_dot()
    }
}

impl BridgeDagManagerState {
    /// Rebuilds the in-memory DAG state from a caller-provided snapshot.
    pub fn dag_manager_rebuild(&mut self, snapshot: DagManagerSnapshot) -> Result<()> {
        self.0.rebuild_from_snapshot(to_domain_snapshot(snapshot))
    }

    /// Adds one accepted DAG block to the in-memory Rust state.
    pub fn dag_manager_add_block(&mut self, block: DagManagerBlock) -> Result<()> {
        self.0.add_block(to_domain_block(block))
    }

    /// Validates pivot/tip availability against the current in-memory DAG state.
    pub fn dag_manager_validate_pivot_tips(
        &self,
        block_level: u64,
        pivot: &[u8; 32],
        tips: Vec<DagHash>,
    ) -> DagPivotTipsValidation {
        let tips = tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect::<Vec<_>>();
        let validation = self
            .0
            .validate_pivot_tips(block_level, to_h256(pivot), &tips);

        DagPivotTipsValidation {
            ok: validation.ok,
            expected_level: validation.expected_level,
            level_matches: validation.level_matches,
            missing_references: to_dag_hashes(validation.missing_references),
        }
    }

    pub fn dag_manager_compute_order(&self, anchor: &[u8; 32]) -> DagOrder {
        match self.0.compute_order(to_h256(anchor)) {
            Some(hashes) => DagOrder {
                found: true,
                hashes: to_dag_hashes(hashes),
            },
            None => DagOrder {
                found: false,
                hashes: Vec::new(),
            },
        }
    }

    pub fn dag_manager_frontier(&self) -> DagFrontier {
        to_bridge_frontier(self.0.frontier())
    }

    pub fn dag_manager_ghost_path(&self, source: &[u8; 32]) -> Vec<DagHash> {
        to_dag_hashes(self.0.ghost_path(to_h256(source)))
    }

    pub fn dag_manager_anchor_ghost_path(&self) -> Vec<DagHash> {
        to_dag_hashes(self.0.anchor_ghost_path())
    }

    pub fn dag_manager_graphviz_dot(&self, pivot_tree: bool) -> String {
        self.0.graphviz_dot(pivot_tree)
    }

    pub fn dag_manager_vertex_count(&self) -> usize {
        self.0.vertex_count()
    }

    pub fn dag_manager_edge_count(&self) -> usize {
        self.0.edge_count()
    }

    pub fn dag_manager_max_level(&self) -> u64 {
        self.0.max_level()
    }

    pub fn dag_manager_latest_period(&self) -> u64 {
        self.0.period()
    }

    pub fn dag_manager_anchors(&self) -> DagManagerAnchors {
        let (old_anchor, anchor) = self.0.anchors();
        DagManagerAnchors {
            old_anchor: old_anchor.into(),
            anchor: anchor.into(),
        }
    }

    pub fn dag_manager_dag_expiry_limit(&self) -> u32 {
        self.0.dag_expiry_limit()
    }

    pub fn dag_manager_dag_expiry_level(&self) -> u64 {
        self.0.dag_expiry_level()
    }

    pub fn dag_manager_non_finalized_blocks(&self) -> Vec<DagLevelHashes> {
        self.0
            .non_finalized_blocks()
            .iter()
            .map(|(level, hashes)| DagLevelHashes {
                level: *level,
                hashes: to_dag_hashes(hashes.iter().copied().collect()),
            })
            .collect()
    }

    pub fn dag_manager_non_finalized_blocks_size(&self) -> DagManagerNonFinalizedSize {
        let (levels, blocks) = self.0.non_finalized_blocks_size();
        DagManagerNonFinalizedSize {
            levels: levels as u64,
            blocks: blocks as u64,
        }
    }

    pub fn dag_manager_non_finalized_min_difficulty(&self) -> u32 {
        self.0.non_finalized_min_difficulty()
    }
}

impl BridgeDagManagerRuntime {
    /// Rebuilds the in-memory DAG state from a caller-provided snapshot.
    pub fn dag_manager_runtime_rebuild(&mut self, snapshot: DagManagerSnapshot) -> Result<()> {
        self.state
            .rebuild_from_snapshot(to_domain_snapshot(snapshot))
    }

    /// Adds one accepted DAG block to the in-memory Rust state.
    pub fn dag_manager_runtime_add_block(&mut self, block: DagManagerBlock) -> Result<()> {
        self.state.add_block(to_domain_block(block))
    }

    /// Computes deterministic DAG order for a target anchor.
    pub fn dag_manager_runtime_compute_order(&self, anchor: &[u8; 32]) -> DagOrder {
        match self.state.compute_order(to_h256(anchor)) {
            Some(hashes) => DagOrder {
                found: true,
                hashes: to_dag_hashes(hashes),
            },
            None => DagOrder {
                found: false,
                hashes: Vec::new(),
            },
        }
    }

    /// Returns the current Rust-owned DAG frontier.
    pub fn dag_manager_runtime_frontier(&self) -> DagFrontier {
        to_bridge_frontier(self.state.frontier())
    }

    /// Returns the ghost path from a source block.
    pub fn dag_manager_runtime_ghost_path(&self, source: &[u8; 32]) -> Vec<DagHash> {
        to_dag_hashes(self.state.ghost_path(to_h256(source)))
    }

    /// Returns the ghost path rooted at the current anchor.
    pub fn dag_manager_runtime_anchor_ghost_path(&self) -> Vec<DagHash> {
        to_dag_hashes(self.state.anchor_ghost_path())
    }

    /// Renders the selected Rust-owned DAG graph as GraphViz dot text.
    pub fn dag_manager_runtime_graphviz_dot(&self, pivot_tree: bool) -> String {
        self.state.graphviz_dot(pivot_tree)
    }

    /// Returns current in-memory DAG vertex count.
    pub fn dag_manager_runtime_vertex_count(&self) -> usize {
        self.state.vertex_count()
    }

    /// Returns current in-memory DAG edge count.
    pub fn dag_manager_runtime_edge_count(&self) -> usize {
        self.state.edge_count()
    }

    /// Returns current max DAG level mirrored in Rust state.
    pub fn dag_manager_runtime_max_level(&self) -> u64 {
        self.state.max_level()
    }

    /// Returns latest finalized period mirrored in Rust state.
    pub fn dag_manager_runtime_latest_period(&self) -> u64 {
        self.state.period()
    }

    /// Returns old/current anchors mirrored in Rust state.
    pub fn dag_manager_runtime_anchors(&self) -> DagManagerAnchors {
        let (old_anchor, anchor) = self.state.anchors();
        DagManagerAnchors {
            old_anchor: old_anchor.into(),
            anchor: anchor.into(),
        }
    }

    /// Returns configured DAG expiry limit.
    pub fn dag_manager_runtime_dag_expiry_limit(&self) -> u32 {
        self.state.dag_expiry_limit()
    }

    /// Returns current DAG expiry level.
    pub fn dag_manager_runtime_dag_expiry_level(&self) -> u64 {
        self.state.dag_expiry_level()
    }

    /// Returns current non-finalized DAG block index by level.
    pub fn dag_manager_runtime_non_finalized_blocks(&self) -> Vec<DagLevelHashes> {
        self.state
            .non_finalized_blocks()
            .iter()
            .map(|(level, hashes)| DagLevelHashes {
                level: *level,
                hashes: to_dag_hashes(hashes.iter().copied().collect()),
            })
            .collect()
    }

    /// Returns non-finalized level and block counts.
    pub fn dag_manager_runtime_non_finalized_blocks_size(&self) -> DagManagerNonFinalizedSize {
        let (levels, blocks) = self.state.non_finalized_blocks_size();
        DagManagerNonFinalizedSize {
            levels: levels as u64,
            blocks: blocks as u64,
        }
    }

    /// Returns current non-finalized minimum difficulty.
    pub fn dag_manager_runtime_non_finalized_min_difficulty(&self) -> u32 {
        self.state.non_finalized_min_difficulty()
    }

    /// Returns whether Rust storage contains a DAG block in non-finalized or
    /// finalized storage.
    pub fn dag_manager_runtime_block_exists(&self, hash: &[u8; 32]) -> Result<bool> {
        self.storage
            .dag()
            .exists(to_h256(hash))
            .context("DAG_STORAGE_BLOCK_EXISTS")
    }

    /// Loads canonical DAG block RLP from Rust storage.
    pub fn dag_manager_runtime_load_block(&self, hash: &[u8; 32]) -> Result<DagBlockLookup> {
        let block_rlp = self
            .storage
            .dag()
            .by_hash_rlp_optional(to_h256(hash))
            .context("DAG_STORAGE_BLOCK_LOAD")?;
        Ok(match block_rlp {
            Some(block_rlp) => DagBlockLookup {
                found: true,
                block_rlp,
            },
            None => DagBlockLookup {
                found: false,
                block_rlp: Vec::new(),
            },
        })
    }

    /// Persists one non-finalized DAG block through Rust storage and updates
    /// persistent DAG counters atomically.
    pub fn dag_manager_runtime_save_block(
        &self,
        hash: &[u8; 32],
        level: u64,
        tips_count: u64,
        block_rlp: Vec<u8>,
    ) -> Result<()> {
        self.storage
            .dag()
            .write(to_h256(hash), level, tips_count, &block_rlp)
            .context("DAG_STORAGE_BLOCK_SAVE")
    }

    /// Ensures the proposal-period mapping exists for `level`.
    ///
    /// Returns true when a mapping write was required and false when the
    /// existing lookup already resolves to `period`.
    pub fn dag_manager_runtime_ensure_proposal_period_mapping(
        &self,
        level: u64,
        period: u64,
    ) -> Result<bool> {
        let dag = self.storage.dag();
        if dag
            .proposal_period_at_level(level)
            .context("DAG_PROPOSAL_PERIOD_LOOKUP")?
            == Some(period)
        {
            return Ok(false);
        }
        dag.write_proposal_period_at_level(level, period)
            .context("DAG_PROPOSAL_PERIOD_WRITE")?;
        Ok(true)
    }

    /// Reads persisted DAG counters directly from Rust storage.
    pub fn dag_manager_runtime_persistence_counters(&self) -> Result<DagPersistenceCounters> {
        let metadata = self.storage.metadata();
        Ok(DagPersistenceCounters {
            dag_blocks: metadata
                .status_field(StatusField::DagBlkCount as u8)
                .context("DAG_STORAGE_COUNTERS")?,
            dag_edges: metadata
                .status_field(StatusField::DagEdgeCount as u8)
                .context("DAG_STORAGE_COUNTERS")?,
        })
    }

    /// Runs deterministic DAG block verification prechecks against Rust state
    /// and storage.
    ///
    /// This bridge method intentionally does not perform transaction, VDF,
    /// DPOS, gas-estimation, event, or networking work. A successful result only
    /// tells C++ to continue the remaining verification stages.
    pub fn dag_manager_runtime_verify_precheck(
        &self,
        block: DagVerifyPrecheckBlock,
    ) -> Result<DagVerifyPrecheckResult> {
        let proposal_period = self
            .storage
            .dag()
            .proposal_period_at_level(block.level)
            .context("DAG_PROPOSAL_PERIOD_LOOKUP")?;
        let tips = block
            .tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect::<Vec<_>>();
        let precheck = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: block.level,
            pivot: to_h256(&block.pivot),
            tips,
            proposal_period_found: proposal_period.is_some(),
            proposal_period: proposal_period.unwrap_or(0),
            dag_expiry_level: self.state.dag_expiry_level(),
        });

        Ok(DagVerifyPrecheckResult {
            continue_validation: precheck.continue_validation,
            reject_code: precheck.reject_code,
            proposal_period_found: precheck.proposal_period_found,
            proposal_period: precheck.proposal_period,
        })
    }
}

/// Runs deterministic transaction availability decisions for DAG block
/// verification.
///
/// C++ supplies live transaction lookup counts. This bridge converts those
/// plain values into the Rust consensus policy result used by the DagManager
/// shim.
pub fn dag_verify_transaction_availability(
    input: DagVerifyTransactionAvailabilityInput,
) -> DagVerifyTransactionAvailabilityResult {
    let result =
        validate_dag_verify_transaction_availability(DomainDagVerifyTransactionAvailabilityInput {
            expected_transactions: input.expected_transactions,
            resolved_transactions: input.resolved_transactions,
        });
    DagVerifyTransactionAvailabilityResult {
        continue_validation: result.continue_validation,
        reject_code: result.reject_code,
    }
}

/// Runs deterministic gas decisions for DAG block verification.
///
/// C++ supplies live gas-estimation outputs. This bridge converts those plain
/// values into the Rust consensus policy result used by the DagManager shim.
pub fn dag_verify_gas(input: DagVerifyGasInput) -> Result<DagVerifyGasResult> {
    ensure!(input.dag_gas_limit != 0, "DAG_GAS_LIMIT_ZERO");
    let result = validate_dag_verify_gas(DomainDagVerifyGasInput {
        block_gas_estimation: input.block_gas_estimation,
        estimated_transactions_weight: input.estimated_transactions_weight,
        dag_gas_limit: input.dag_gas_limit,
        pbft_gas_limit: input.pbft_gas_limit,
        tip_gas_estimations: input
            .tip_gas_estimations
            .into_iter()
            .map(|tip| DagTipGas {
                found: tip.found,
                gas_estimation: tip.gas_estimation,
            })
            .collect(),
    });
    Ok(DagVerifyGasResult {
        continue_validation: result.continue_validation,
        reject_code: result.reject_code,
    })
}

fn to_h256(hash: &[u8; 32]) -> H256 {
    H256::from(*hash)
}

fn to_dag_hashes(hashes: Vec<H256>) -> Vec<DagHash> {
    hashes
        .into_iter()
        .map(|hash| DagHash { hash: hash.0 })
        .collect()
}

fn to_bridge_frontier(frontier: &rustaxa_consensus::dag::DagFrontier) -> DagFrontier {
    DagFrontier {
        pivot: frontier.pivot.into(),
        tips: to_dag_hashes(frontier.tips.clone()),
    }
}

fn to_domain_block(block: DagManagerBlock) -> DomainDagManagerBlock {
    DomainDagManagerBlock {
        hash: H256::from(block.hash),
        pivot: H256::from(block.pivot),
        tips: block
            .tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect(),
        level: block.level,
        difficulty: block.difficulty,
    }
}

fn to_domain_snapshot(snapshot: DagManagerSnapshot) -> DomainDagManagerSnapshot {
    DomainDagManagerSnapshot {
        old_anchor: H256::from(snapshot.old_anchor),
        anchor: H256::from(snapshot.anchor),
        anchor_level: snapshot.anchor_level,
        period: snapshot.period,
        max_level: snapshot.max_level,
        dag_expiry_level: snapshot.dag_expiry_level,
        non_finalized_min_difficulty: snapshot.non_finalized_min_difficulty,
        non_finalized_blocks: snapshot
            .non_finalized_blocks
            .into_iter()
            .map(to_domain_block)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::create_storage;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after UNIX_EPOCH")
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, now_ns, id))
    }

    #[test]
    fn dag_manager_runtime_persists_and_loads_blocks() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_storage");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            let hash = [7u8; 32];
            let block_rlp = vec![0xAA, 0xBB, 0xCC];

            assert!(!runtime
                .dag_manager_runtime_block_exists(&hash)
                .expect("existence lookup should succeed"));

            runtime
                .dag_manager_runtime_save_block(&hash, 11, 2, block_rlp.clone())
                .expect("save should succeed");

            assert!(runtime
                .dag_manager_runtime_block_exists(&hash)
                .expect("existence lookup should succeed"));

            let loaded = runtime
                .dag_manager_runtime_load_block(&hash)
                .expect("load should succeed");
            assert!(loaded.found);
            assert_eq!(loaded.block_rlp, block_rlp);

            let counters = runtime
                .dag_manager_runtime_persistence_counters()
                .expect("counter lookup should succeed");
            assert_eq!(counters.dag_blocks, 1);
            assert_eq!(counters.dag_edges, 3);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_ensures_proposal_period_for_mismatched_lookup() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposal_mapping");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            assert!(runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(200, 5)
                .expect("initial mapping write should succeed"));

            // Level 100 resolves to period 5 via the later (200 -> 5) mapping.
            let before = storage
                .get_proposal_period_for_dag_level(100)
                .expect("lookup should succeed");
            assert!(before.found);
            assert_eq!(before.period, 5);

            // Ensure path must still write because the resolved value mismatches
            // the expected period for this level.
            assert!(runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(100, 0)
                .expect("mismatch correction should succeed"));

            let after = storage
                .get_proposal_period_for_dag_level(100)
                .expect("lookup should succeed");
            assert!(after.found);
            assert_eq!(after.period, 0);

            assert!(!runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(100, 0)
                .expect("idempotent ensure should succeed"));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_verify_precheck_uses_storage_period_and_expiry() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_verify_precheck");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_rebuild(DagManagerSnapshot {
                    old_anchor: [1u8; 32],
                    anchor: [1u8; 32],
                    anchor_level: 8,
                    period: 5,
                    max_level: 8,
                    dag_expiry_level: 4,
                    non_finalized_min_difficulty: u32::MAX,
                    non_finalized_blocks: vec![],
                })
                .expect("rebuild should succeed");

            let missing_period = runtime
                .dag_manager_runtime_verify_precheck(DagVerifyPrecheckBlock {
                    level: 3,
                    pivot: [1u8; 32],
                    tips: vec![],
                })
                .expect("precheck should succeed");
            assert!(!missing_period.continue_validation);
            assert_eq!(
                missing_period.reject_code,
                rustaxa_consensus::dag::DAG_VERIFY_REJECT_AHEAD_BLOCK
            );

            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(3, 5)
                .expect("mapping write should succeed");
            let expired = runtime
                .dag_manager_runtime_verify_precheck(DagVerifyPrecheckBlock {
                    level: 3,
                    pivot: [1u8; 32],
                    tips: vec![],
                })
                .expect("precheck should succeed");
            assert!(!expired.continue_validation);
            assert_eq!(
                expired.reject_code,
                rustaxa_consensus::dag::DAG_VERIFY_REJECT_EXPIRED_BLOCK
            );

            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(4, 5)
                .expect("mapping write should succeed");
            let continues = runtime
                .dag_manager_runtime_verify_precheck(DagVerifyPrecheckBlock {
                    level: 4,
                    pivot: [1u8; 32],
                    tips: vec![],
                })
                .expect("precheck should succeed");
            assert!(continues.continue_validation);
            assert_eq!(continues.reject_code, 0);
            assert!(continues.proposal_period_found);
            assert_eq!(continues.proposal_period, 5);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_verify_transaction_availability_and_gas_bridge_decisions() {
        let missing = dag_verify_transaction_availability(DagVerifyTransactionAvailabilityInput {
            expected_transactions: 2,
            resolved_transactions: 1,
        });
        assert!(!missing.continue_validation);
        assert_eq!(
            missing.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_MISSING_TRANSACTION
        );

        let block_too_big = dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 101,
            estimated_transactions_weight: 101,
            dag_gas_limit: 100,
            pbft_gas_limit: 500,
            tip_gas_estimations: vec![],
        })
        .expect("gas decision should succeed");
        assert!(!block_too_big.continue_validation);
        assert_eq!(
            block_too_big.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_BLOCK_TOO_BIG
        );

        let continues = dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 100,
            estimated_transactions_weight: 100,
            dag_gas_limit: 100,
            pbft_gas_limit: 300,
            tip_gas_estimations: vec![
                crate::ffi::rustaxa_ffi::DagTipGas {
                    found: true,
                    gas_estimation: 50,
                },
                crate::ffi::rustaxa_ffi::DagTipGas {
                    found: true,
                    gas_estimation: 50,
                },
            ],
        })
        .expect("gas decision should succeed");
        assert!(continues.continue_validation);
        assert_eq!(continues.reject_code, 0);
    }
}
