use crate::ffi::rustaxa_ffi::{
    DagBlockLookup, DagBlockTransactionRefs, DagExpiredTransactionCleanupPayload,
    DagExpiredTransactionCleanupPlan, DagExpiredTransactionFact, DagFinalizedCounterUpdate,
    DagFrontier, DagHash, DagLevelHashes, DagManagerAnchors, DagManagerBlock,
    DagManagerFinalizationApplyPayload, DagManagerFinalizationCleanupPayload,
    DagManagerFinalizationPlan, DagManagerNonFinalizedSize, DagManagerNonFinalizedSyncPayload,
    DagManagerRuntimeSyncSnapshot, DagManagerSnapshot, DagOrder, DagPersistenceCounters,
    DagPivotTipsValidation, DagProposerEligibilityDecision, DagProposerEligibilityInput,
    DagProposerTipCandidate, DagProposerTipSelection, DagReferenceMetadata, DagSyncBlockRlp,
    DagTransactionHash, DagTransactionQueryPlan, DagTransactionRlpLookup,
    DagVerifyAuthorizationInput, DagVerifyAuthorizationResult, DagVerifyGasInput,
    DagVerifyGasResult, DagVerifyPrecheckBlock, DagVerifyPrecheckResult,
    DagVerifyTransactionAvailabilityInput, DagVerifyTransactionAvailabilityResult,
    DagVerifyVdfDposDecision, DagVerifyVdfDposFacts, DagVerifyVdfPrepareInput,
    DagVerifyVdfPrepareResult, DagVerifyVdfSortitionFromBlockInput, DagVerifyVdfSortitionInput,
    DagVerifyVdfSortitionResult, HashLookup, PeriodLookup, SortitionRuntimeParams,
};
use crate::ffi::{BridgeDagGraph, BridgeDagManagerRuntime, BridgeDagManagerState, BridgeStorage};
use anyhow::{ensure, Context, Result};
use ethereum_types::H256;
#[cfg(test)]
use rustaxa_consensus::dag::collect_finalization_cleanup_from_storage;
use rustaxa_consensus::dag::{
    apply_finalization_cleanup_from_storage, collect_expired_transaction_cleanup_from_storage,
    collect_non_finalized_sync_payload_from_storage, construct_dag_vdf_message,
    dag_block_exists_in_storage, dag_persistence_counters_from_storage,
    decide_dag_verify_vdf_dpos_authorization, derive_frontier, ensure_proposal_period_mapping,
    load_dag_block_from_storage, period_block_hash_from_storage, plan_dag_verify_transaction_query,
    plan_expired_transaction_cleanup, plan_non_finalized_transaction_query, prepare_dag_verify_vdf,
    proposal_period_for_level_from_storage, save_dag_block_to_storage,
    validate_dag_verify_authorization, validate_dag_verify_gas,
    validate_dag_verify_transaction_availability, validate_pivot_tips_metadata,
    verify_dag_vdf_sortition, verify_dag_vdf_sortition_from_block, verify_precheck_from_storage,
    DagExpiredTransactionFact as DomainDagExpiredTransactionFact, DagGraph,
    DagManagerBlock as DomainDagManagerBlock,
    DagManagerFinalizationCleanupStoragePayload as DomainDagManagerFinalizationCleanupStoragePayload,
    DagManagerFinalizationPlan as DomainDagManagerFinalizationPlan,
    DagManagerSnapshot as DomainDagManagerSnapshot, DagManagerState,
    DagReferenceMetadata as ReferenceMetadata, DagTipGas,
    DagVdfSortitionBlockInput as DomainDagVdfSortitionBlockInput,
    DagVdfSortitionInput as DomainDagVdfSortitionInput,
    DagVerifyAuthorizationInput as DomainDagVerifyAuthorizationInput,
    DagVerifyGasInput as DomainDagVerifyGasInput,
    DagVerifyPrecheckStorageInput as DomainDagVerifyPrecheckStorageInput,
    DagVerifyTransactionAvailabilityInput as DomainDagVerifyTransactionAvailabilityInput,
    DagVerifyVdfDposFacts as DomainDagVerifyVdfDposFacts,
    DagVerifyVdfPrepareInput as DomainDagVerifyVdfPrepareInput,
};
use rustaxa_consensus::sortition::{SortitionParams, VdfParams, VrfParams};
#[cfg(test)]
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
#[cfg(test)]
use rustaxa_types::pbft::PbftBlockLink;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

const DAG_PROPOSER_ACTION_CONTINUE: u8 = 1;
const DAG_PROPOSER_ACTION_SKIP: u8 = 2;
const DAG_PROPOSER_ACTION_RETRY_LATER: u8 = 3;

const DAG_PROPOSER_REASON_OK: u32 = 0;
const DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD: u32 = 1;
const DAG_PROPOSER_REASON_MISSING_VRF_KEY: u32 = 2;
const DAG_PROPOSER_REASON_VRF_KEY_MISMATCH: u32 = 3;
const DAG_PROPOSER_REASON_DPOS_UNAVAILABLE: u32 = 4;
const DAG_PROPOSER_REASON_NOT_ELIGIBLE: u32 = 5;
const DAG_PROPOSER_REASON_ZERO_DENOMINATOR: u32 = 6;

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

    /// Applies one finalized DAG order directly to Rust state and advances period/anchor.
    ///
    /// Inputs:
    /// - `new_anchor`: hash of the new anchor block, or zero for an empty
    ///   PBFT period without a DAG anchor transition.
    /// - `new_anchor_level`: storage-resolved anchor level for non-empty anchors.
    /// - `new_period`: expected to be `state.period + 1`.
    /// - `finalized_order`: hashes finalized in this order transition.
    ///
    /// Output:
    /// - deterministic finalization plan including unique finalized count and side-effect hashes.
    #[cfg(test)]
    pub fn dag_manager_runtime_set_finalized_order(
        &mut self,
        new_anchor: [u8; 32],
        new_anchor_level: u64,
        new_period: u64,
        finalized_order: Vec<DagHash>,
    ) -> Result<DagManagerFinalizationPlan> {
        let new_anchor = to_h256(&new_anchor);
        if new_anchor == H256::zero() {
            self.state
                .advance_empty_period(new_period)
                .context("DAG_RUNTIME_ADVANCE_EMPTY_PERIOD")?;
            return Ok(DagManagerFinalizationPlan {
                finalized_count: 0,
                counter_update_hashes: Vec::new(),
                expired_hashes: Vec::new(),
                remaining_hashes: to_dag_hashes(
                    self.state
                        .non_finalized_blocks()
                        .values()
                        .flatten()
                        .copied()
                        .collect(),
                ),
                remove_transaction_hashes: Vec::new(),
            });
        }

        let finalized_order = finalized_order
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        self.state
            .set_finalized_order(new_anchor, new_period, &finalized_order, new_anchor_level)
            .context("DAG_RUNTIME_SET_FINALIZED_ORDER")
            .map(to_bridge_finalization_plan)
    }

    /// Builds storage-backed cleanup facts for a finalized DAG order plan.
    ///
    /// This method is a narrow convenience wrapper over
    /// `dag_manager_runtime_expired_transaction_cleanup_payload` for callers that
    /// already have a full `DagManagerFinalizationPlan`.
    #[cfg(test)]
    pub fn dag_manager_runtime_finalization_cleanup_payload(
        &self,
        plan: DagManagerFinalizationPlan,
    ) -> Result<DagManagerFinalizationCleanupPayload> {
        let DagManagerFinalizationPlan {
            finalized_count: _,
            counter_update_hashes,
            remove_transaction_hashes: _,
            expired_hashes,
            remaining_hashes,
        } = plan;

        let counter_update_hashes = counter_update_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let expired_hashes = expired_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let remaining_hashes = remaining_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let payload = collect_finalization_cleanup_from_storage(
            self.storage.as_ref(),
            &counter_update_hashes,
            &expired_hashes,
            &remaining_hashes,
        )
        .context("DAG_RUNTIME_FINALIZATION_CLEANUP_BUILD_FAILED")?;

        Ok(to_bridge_finalization_cleanup_payload(payload))
    }

    /// Applies one finalized DAG order through Rust state and Rust storage.
    ///
    /// Inputs:
    /// - `new_anchor`: the new finalized DAG anchor, or zero for an empty PBFT
    ///   period without a DAG anchor transition.
    /// - `new_period`: expected to be the next runtime period.
    /// - `finalized_order`: finalized DAG block hashes in legacy order.
    ///
    /// Output:
    /// - finalized block count plus the live C++ side-effect facts that cannot
    ///   move yet: expired block hashes for `seen_blocks_` cleanup and expired
    ///   transaction hashes for transaction-manager sidecar cleanup. Returned
    ///   transaction hashes have already been removed from Rust-owned storage.
    ///
    /// Behavior:
    /// - resolves the anchor level from Rust storage when the anchor is nonzero
    /// - computes finalization on a candidate state before mutating this runtime
    /// - preflights storage-backed cleanup facts before persistent writes
    /// - updates Rust DAG counters, removes expired DAG blocks, and removes
    ///   expired non-finalized transaction payloads through `rustaxa-storage`
    /// - commits the candidate state only after the Rust-owned storage writes
    ///   complete
    pub fn dag_manager_runtime_apply_finalized_order(
        &mut self,
        new_anchor: [u8; 32],
        new_period: u64,
        finalized_order: Vec<DagHash>,
    ) -> Result<DagManagerFinalizationApplyPayload> {
        let new_anchor = H256::from(new_anchor);
        let mut candidate_state = self.state.clone();

        let plan = if new_anchor == H256::zero() {
            candidate_state
                .advance_empty_period(new_period)
                .context("DAG_RUNTIME_ADVANCE_EMPTY_PERIOD")?;
            DagManagerFinalizationPlan {
                finalized_count: 0,
                counter_update_hashes: Vec::new(),
                expired_hashes: Vec::new(),
                remaining_hashes: to_dag_hashes(
                    candidate_state
                        .non_finalized_blocks()
                        .values()
                        .flatten()
                        .copied()
                        .collect(),
                ),
                remove_transaction_hashes: Vec::new(),
            }
        } else {
            let anchor_level = self
                .storage
                .dag()
                .by_hash(new_anchor)
                .with_context(|| format!("DAG_RUNTIME_FINALIZATION_ANCHOR_BLOCK: {new_anchor:?}"))?
                .level;
            let finalized_order = finalized_order
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect::<Vec<_>>();
            candidate_state
                .set_finalized_order(new_anchor, new_period, &finalized_order, anchor_level)
                .context("DAG_RUNTIME_SET_FINALIZED_ORDER")
                .map(to_bridge_finalization_plan)?
        };

        let finalized_count = plan.finalized_count;
        let counter_update_hashes = plan
            .counter_update_hashes
            .iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let expired_hashes = plan
            .expired_hashes
            .iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let remaining_hashes = plan
            .remaining_hashes
            .iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let cleanup = apply_finalization_cleanup_from_storage(
            self.storage.as_ref(),
            &counter_update_hashes,
            &expired_hashes,
            &remaining_hashes,
        )
        .context("DAG_RUNTIME_FINALIZATION_STORAGE_APPLY")?;

        self.state = candidate_state;
        let cleanup = to_bridge_finalization_cleanup_payload(cleanup);

        Ok(DagManagerFinalizationApplyPayload {
            finalized_count,
            expired_hashes: cleanup.expired_hashes,
            remove_transaction_hashes: cleanup.remove_transaction_hashes,
        })
    }

    /// Returns a one-shot sync snapshot containing the current period and the
    /// deterministic selection of non-finalized block hashes that are not in
    /// `known_hashes`.
    pub fn dag_manager_runtime_non_finalized_sync_snapshot(
        &self,
        known_hashes: Vec<DagHash>,
    ) -> DagManagerRuntimeSyncSnapshot {
        let known_hashes = known_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        DagManagerRuntimeSyncSnapshot {
            period: self.state.period(),
            selected_hashes: to_dag_hashes(
                self.state
                    .select_non_finalized_hashes_excluding_known(&known_hashes),
            ),
        }
    }

    /// Builds one-shot non-finalized DAG sync materialization data through
    /// Rust storage only.
    ///
    /// Returns selected block RLP payloads plus a de-duplicated transaction lookup
    /// list that preserves the sync snapshot block order and per-block
    /// transaction order.
    pub fn dag_manager_runtime_non_finalized_sync_payload(
        &self,
        known_hashes: Vec<DagHash>,
    ) -> Result<DagManagerNonFinalizedSyncPayload> {
        let snapshot = self.dag_manager_runtime_non_finalized_sync_snapshot(known_hashes);
        let selected_hashes = snapshot
            .selected_hashes
            .iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let payload = collect_non_finalized_sync_payload_from_storage(
            self.storage.as_ref(),
            &selected_hashes,
        )
        .context("DAG_RUNTIME_SYNC_STORAGE_PAYLOAD")?;

        Ok(DagManagerNonFinalizedSyncPayload {
            period: snapshot.period,
            blocks: to_bridge_sync_blocks(payload.blocks),
            transactions: to_bridge_transaction_rlp_lookups(payload.transactions),
        })
    }

    /// Builds finalization cleanup facts and removals from storage-backed block inputs.
    ///
    /// Inputs:
    /// - `expired_hashes`: hashes of non-finalized DAG blocks removed by this
    ///   finalized order transition.
    /// - `remaining_hashes`: hashes of DAG blocks that remain in the non-finalized
    ///   graph after the transition.
    ///
    /// Output:
    /// - `expired_transaction_facts`: transaction references observed in expired blocks
    ///   with `finalized` flags resolved from Rust storage.
    /// - `remove_hashes`: a compact set computed by
    ///   `plan_expired_transaction_cleanup`.
    pub fn dag_manager_runtime_expired_transaction_cleanup_payload(
        &self,
        expired_hashes: Vec<DagHash>,
        remaining_hashes: Vec<DagHash>,
    ) -> Result<DagExpiredTransactionCleanupPayload> {
        let expired_hashes = expired_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let remaining_hashes = remaining_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let payload = collect_expired_transaction_cleanup_from_storage(
            self.storage.as_ref(),
            &expired_hashes,
            &remaining_hashes,
        )
        .context("DAG_RUNTIME_FINALIZATION_CLEANUP_STORAGE")?;

        let expired_transaction_facts = payload
            .expired_transaction_facts
            .iter()
            .map(|candidate| DagExpiredTransactionFact {
                hash: candidate.hash.0,
                finalized: candidate.finalized,
            })
            .collect();

        Ok(DagExpiredTransactionCleanupPayload {
            expired_transaction_facts,
            remove_hashes: to_bridge_transaction_hashes(payload.remove_hashes),
        })
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

    /// Returns non-finalized DAG block hashes excluding already-known hashes.
    ///
    /// This method applies the deterministic `DagManagerState` selection helper at
    /// the runtime boundary so C++ can request next-sync candidates without
    /// reordering responsibility.
    pub fn dag_manager_runtime_select_non_finalized_hashes(
        &self,
        known_hashes: Vec<DagHash>,
    ) -> Vec<DagHash> {
        let known_hashes = known_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        to_dag_hashes(
            self.state
                .select_non_finalized_hashes_excluding_known(&known_hashes),
        )
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
        dag_block_exists_in_storage(self.storage.as_ref(), to_h256(hash))
    }

    /// Loads canonical DAG block RLP from Rust storage.
    pub fn dag_manager_runtime_load_block(&self, hash: &[u8; 32]) -> Result<DagBlockLookup> {
        let lookup = load_dag_block_from_storage(self.storage.as_ref(), to_h256(hash))?;
        Ok(DagBlockLookup {
            found: lookup.found,
            block_rlp: lookup.block_rlp,
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
        save_dag_block_to_storage(
            self.storage.as_ref(),
            to_h256(hash),
            level,
            tips_count,
            &block_rlp,
        )
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
        ensure_proposal_period_mapping(self.storage.as_ref(), level, period)
    }

    /// Resolves the finalized proposal period for a DAG level through the
    /// runtime-owned Rust storage handle.
    ///
    /// Inputs and outputs mirror `DbStorage::getProposalPeriodForDagLevel`:
    /// Rust storage returns the first persisted `(level -> period)` row at or
    /// after the requested level. Missing rows are reported as `found = false`
    /// instead of errors, while malformed storage/backend failures are errors.
    pub fn dag_manager_runtime_proposal_period_for_level(
        &self,
        level: u64,
    ) -> Result<PeriodLookup> {
        let lookup = proposal_period_for_level_from_storage(self.storage.as_ref(), level)?;
        Ok(PeriodLookup {
            found: lookup.found,
            period: lookup.period,
        })
    }

    /// Returns the canonical PBFT block hash for finalized `period`.
    ///
    /// The hash is derived from item 0 of the canonical `PeriodData` RLP stored
    /// in Rust storage, matching legacy `DbStorage::getPeriodBlockHash`. Missing
    /// period data returns `found = false`; corrupt period data is a bridge
    /// error so C++ verification can reject rather than silently use bad facts.
    pub fn dag_manager_runtime_period_block_hash(&self, period: u64) -> Result<HashLookup> {
        let lookup = period_block_hash_from_storage(self.storage.as_ref(), period)?;
        Ok(HashLookup {
            found: lookup.found,
            hash: lookup.hash.into(),
        })
    }

    /// Reads persisted DAG counters directly from Rust storage.
    pub fn dag_manager_runtime_persistence_counters(&self) -> Result<DagPersistenceCounters> {
        let counters = dag_persistence_counters_from_storage(self.storage.as_ref())?;
        Ok(DagPersistenceCounters {
            dag_blocks: counters.dag_blocks,
            dag_edges: counters.dag_edges,
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
        let tips = block
            .tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect::<Vec<_>>();
        let precheck = verify_precheck_from_storage(
            self.storage.as_ref(),
            DomainDagVerifyPrecheckStorageInput {
                block_level: block.level,
                pivot: to_h256(&block.pivot),
                tips,
                dag_expiry_level: self.state.dag_expiry_level(),
            },
        )?;

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

/// Builds a deterministic plan of additional transaction hashes required for
/// `DagManager::verifyBlock`.
///
/// Inputs:
/// - `block_transaction_hashes`: all hashes in block order.
/// - `supplied_transaction_hashes`: hashes already provided by the caller.
///
/// Outputs preserve first-seen block order and dedupe duplicates.
pub fn dag_plan_verify_transaction_query(
    block_transaction_hashes: Vec<DagTransactionHash>,
    supplied_transaction_hashes: Vec<DagTransactionHash>,
) -> DagTransactionQueryPlan {
    let block_transaction_hashes = to_transaction_hashes(block_transaction_hashes);
    let supplied_transaction_hashes = to_transaction_hashes(supplied_transaction_hashes);
    let plan =
        plan_dag_verify_transaction_query(&block_transaction_hashes, &supplied_transaction_hashes);
    DagTransactionQueryPlan {
        query_hashes: to_bridge_transaction_hashes(plan.query_hashes),
    }
}

/// Builds a deterministic unique list of transaction hashes referenced by
/// non-finalized DAG blocks, preserving first-seen order.
pub fn dag_plan_non_finalized_transaction_query(
    blocks: Vec<DagBlockTransactionRefs>,
) -> DagTransactionQueryPlan {
    let blocks = blocks
        .into_iter()
        .map(|block| to_transaction_hashes(block.transaction_hashes))
        .collect::<Vec<_>>();
    let plan = plan_non_finalized_transaction_query(&blocks);
    DagTransactionQueryPlan {
        query_hashes: to_bridge_transaction_hashes(plan.query_hashes),
    }
}

/// Builds a deterministic cleanup plan for non-finalized transaction state after
/// expired DAG block finalization.
///
/// Inputs:
/// - `expired_candidates`: candidate hashes from expired DAG blocks with finality
///   flags.
/// - `retained_transaction_refs`: hashes still referenced by non-finalized DAG
///   blocks and therefore not removable.
pub fn dag_plan_expired_transaction_cleanup(
    expired_candidates: Vec<DagExpiredTransactionFact>,
    retained_transaction_refs: Vec<DagTransactionHash>,
) -> DagExpiredTransactionCleanupPlan {
    let expired_candidates = expired_candidates
        .into_iter()
        .map(|candidate| DomainDagExpiredTransactionFact {
            hash: H256::from(candidate.hash),
            finalized: candidate.finalized,
        })
        .collect::<Vec<_>>();
    let retained_transaction_refs = to_transaction_hashes(retained_transaction_refs);
    let plan = plan_expired_transaction_cleanup(&expired_candidates, &retained_transaction_refs);
    DagExpiredTransactionCleanupPlan {
        remove_hashes: to_bridge_transaction_hashes(plan.remove_hashes),
    }
}

/// Prepares deterministic VDF verification inputs for DAG block verification.
///
/// C++ supplies live VRF-key and DPoS vote-count data. Rust returns the
/// legacy-compatible missing-key reject or the vote counts C++ must pass to
/// the current C++ VDF verifier.
pub fn dag_verify_vdf_prepare(input: DagVerifyVdfPrepareInput) -> DagVerifyVdfPrepareResult {
    let result = prepare_dag_verify_vdf(DomainDagVerifyVdfPrepareInput {
        vrf_key_found: input.vrf_key_found,
        eligible_vote_count: input.eligible_vote_count,
        vdf_max_vote_count: input.vdf_max_vote_count,
    });
    DagVerifyVdfPrepareResult {
        continue_validation: result.continue_validation,
        reject_code: result.reject_code,
        reason_code: result.reason_code,
        vote_count: result.vote_count,
        max_vote_count: result.max_vote_count,
    }
}

/// Runs deterministic authorization decisions for DAG block verification.
///
/// C++ supplies outcomes from VDF verification and DPoS eligibility reads. Rust
/// applies consensus reject ordering and returns legacy-compatible codes.
pub fn dag_verify_authorization(
    input: DagVerifyAuthorizationInput,
) -> DagVerifyAuthorizationResult {
    let result = validate_dag_verify_authorization(DomainDagVerifyAuthorizationInput {
        vdf_valid: input.vdf_valid,
        dpos_snapshot_available: input.dpos_snapshot_available,
        dpos_eligible: input.dpos_eligible,
    });
    DagVerifyAuthorizationResult {
        continue_validation: result.continue_validation,
        reject_code: result.reject_code,
        reason_code: result.reason_code,
    }
}

/// Runs deterministic VDF and DPoS authorization over explicit facts.
///
/// C++ supplies current live VDF and DPoS lookup outcomes. Rust applies the
/// complete consensus reject ordering and returns legacy-compatible codes plus
/// the vote counts used for diagnostics and parity checks.
pub fn dag_decide_vdf_dpos_authorization(facts: DagVerifyVdfDposFacts) -> DagVerifyVdfDposDecision {
    let decision = decide_dag_verify_vdf_dpos_authorization(DomainDagVerifyVdfDposFacts {
        vrf_key_found: facts.vrf_key_found,
        sender_eligible_vote_count: facts.sender_eligible_vote_count,
        vdf_sortition_max_vote_count: facts.vdf_sortition_max_vote_count,
        vdf_status: facts.vdf_status,
        dpos_status: facts.dpos_status,
    });
    DagVerifyVdfDposDecision {
        continue_validation: decision.continue_validation,
        reject_code: decision.reject_code,
        reason_code: decision.reason_code,
        vote_count: decision.vote_count,
        max_vote_count: decision.max_vote_count,
    }
}

/// Verifies DAG VDF proof and difficulty using either:
/// - direct embedded VRF verification (`vrf_public_key` + `vrf_input`), or
/// - legacy precomputed `vrf_output` compatibility path.
///
/// Invalid peer proof/data returns `vdf_status = INVALID`; malformed bridge
/// payloads such as wrong precomputed-output shape return `Err` because the
/// Rust/bridge contract is itself not satisfiable.
pub fn dag_verify_vdf_sortition(
    input: DagVerifyVdfSortitionInput,
) -> Result<DagVerifyVdfSortitionResult> {
    let has_embedded_vrf = !(input.vrf_public_key.is_empty() && input.vrf_input.is_empty());
    ensure!(
        !has_embedded_vrf || (!input.vrf_public_key.is_empty() && !input.vrf_input.is_empty()),
        "embedded VRF verification requires vrf_public_key and vrf_input"
    );

    let result = if has_embedded_vrf {
        verify_dag_vdf_sortition(DomainDagVdfSortitionInput {
            block_rlp: input.block_rlp,
            vdf_input: input.vdf_input,
            sortition_params: to_domain_sortition_params(input.sortition_params),
            vrf_output: [0_u8; 64],
            vrf_public_key: input.vrf_public_key,
            vrf_input: input.vrf_input,
            sender_eligible_vote_count: input.sender_eligible_vote_count,
            vdf_sortition_max_vote_count: input.vdf_sortition_max_vote_count,
        })?
    } else {
        let vrf_output = to_vrf_output(input.vrf_output)?;
        verify_dag_vdf_sortition(DomainDagVdfSortitionInput {
            block_rlp: input.block_rlp,
            vdf_input: input.vdf_input,
            sortition_params: to_domain_sortition_params(input.sortition_params),
            vrf_output,
            vrf_public_key: Vec::new(),
            vrf_input: Vec::new(),
            sender_eligible_vote_count: input.sender_eligible_vote_count,
            vdf_sortition_max_vote_count: input.vdf_sortition_max_vote_count,
        })?
    };

    Ok(DagVerifyVdfSortitionResult {
        vdf_status: result.vdf_status,
        difficulty: result.difficulty,
        expected_difficulty: result.expected_difficulty,
    })
}

/// Verifies DAG VDF sortition after building canonical legacy messages in Rust.
///
/// C++ passes only the block payload and sortition context; Rust rebuilds:
/// - `vrf_input`: sequential RLP items `block_level`, `proposal_period_hash`
/// - `vdf_input`: sequential RLP items `pivot`, then each transaction hash
///
/// It then verifies the embedded proof using `vrf_public_key`.
pub fn dag_verify_vdf_sortition_from_block(
    input: DagVerifyVdfSortitionFromBlockInput,
) -> Result<DagVerifyVdfSortitionResult> {
    let result = verify_dag_vdf_sortition_from_block(DomainDagVdfSortitionBlockInput {
        block_rlp: input.block_rlp,
        block_level: input.block_level,
        proposal_period_hash: H256::from(input.proposal_period_hash),
        vrf_public_key: input.vrf_public_key,
        sortition_params: to_domain_sortition_params(input.sortition_params),
        sender_eligible_vote_count: input.sender_eligible_vote_count,
        vdf_sortition_max_vote_count: input.vdf_sortition_max_vote_count,
    })?;

    Ok(DagVerifyVdfSortitionResult {
        vdf_status: result.vdf_status,
        difficulty: result.difficulty,
        expected_difficulty: result.expected_difficulty,
    })
}

/// Builds the legacy DAG proposer VRF input.
///
/// The returned bytes are the canonical sequential RLP encoding of
/// `(block_level, proposal_period_hash)`. This is the producer-side counterpart
/// to Rust DAG VDF verification, which reconstructs the same bytes from block
/// context.
pub fn dag_vrf_input(block_level: u64, proposal_period_hash: &[u8; 32]) -> Vec<u8> {
    rustaxa_consensus::dag::construct_dag_vrf_input(block_level, H256::from(*proposal_period_hash))
}

/// Builds the legacy DAG VDF message for a pivot and ordered transaction hashes.
///
/// This bridge is used by the C++ DagManager shim to preserve the public
/// `DagManager::getVdfMessage` API while moving the consensus byte construction
/// into Rust. The output is a sequence of RLP items, matching legacy C++
/// `dev::RLPStream << pivot << tx_hash...` behavior.
pub fn dag_vdf_message(pivot: &[u8; 32], transaction_hashes: Vec<DagHash>) -> Vec<u8> {
    let hashes = transaction_hashes
        .into_iter()
        .map(|hash| H256::from(hash.hash))
        .collect::<Vec<_>>();
    construct_dag_vdf_message(H256::from(*pivot), &hashes)
}

/// Checks DAG block proposer eligibility from Rust-owned DPoS/VRF facts.
///
/// Inputs are plain bridge values collected by the C++ proposer shim: whether
/// the level has a proposal period, the local wallet VRF public key, and the
/// Rust FinalChain authorization facts for `(proposal_period, proposer)`.
/// Expected proposal skips are returned as status data; bridge contract
/// violations are not represented here because all fields have fixed shapes.
pub fn dag_proposer_check_eligibility(
    input: DagProposerEligibilityInput,
) -> DagProposerEligibilityDecision {
    if !input.proposal_period_found {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_SKIP,
            DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD,
            0,
            0,
        );
    }
    if !input.authorization_facts.vrf_key_found {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_SKIP,
            DAG_PROPOSER_REASON_MISSING_VRF_KEY,
            0,
            0,
        );
    }
    if input.authorization_facts.vrf_key != input.wallet_vrf_public_key {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_SKIP,
            DAG_PROPOSER_REASON_VRF_KEY_MISMATCH,
            0,
            0,
        );
    }
    if input.authorization_facts.eligibility_status
        == rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
    {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_RETRY_LATER,
            DAG_PROPOSER_REASON_DPOS_UNAVAILABLE,
            0,
            0,
        );
    }
    if input.authorization_facts.eligibility_status
        != rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE
    {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_SKIP,
            DAG_PROPOSER_REASON_NOT_ELIGIBLE,
            0,
            0,
        );
    }
    if input.authorization_facts.vdf_sortition_max_vote_count == 0 {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_SKIP,
            DAG_PROPOSER_REASON_ZERO_DENOMINATOR,
            input.authorization_facts.sender_eligible_vote_count,
            0,
        );
    }

    dag_proposer_decision(
        DAG_PROPOSER_ACTION_CONTINUE,
        DAG_PROPOSER_REASON_OK,
        input.authorization_facts.sender_eligible_vote_count,
        input.authorization_facts.vdf_sortition_max_vote_count,
    )
}

/// Selects DAG block proposer tips from caller-provided tip metadata.
///
/// C++ owns live `DagBlock` lookup and passes flat candidate records. Rust owns
/// deterministic ordering and gas-limit policy:
/// - missing candidates are skipped and counted
/// - found candidates from unique proposers are considered before duplicate
///   proposer candidates
/// - each group is ordered by descending level with stable input-order ties
/// - selection stops before exceeding `gas_limit` or `max_tips`
pub fn dag_proposer_select_tips(
    candidates: Vec<DagProposerTipCandidate>,
    gas_limit: u64,
    max_tips: u16,
) -> DagProposerTipSelection {
    let skipped_missing = candidates
        .iter()
        .filter(|candidate| !candidate.found)
        .count() as u64;
    let found = candidates
        .into_iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.found)
        .collect::<Vec<_>>();

    let mut proposer_counts = BTreeMap::<[u8; 20], usize>::new();
    for (_, candidate) in &found {
        *proposer_counts.entry(candidate.sender).or_default() += 1;
    }

    let mut unique = Vec::new();
    let mut duplicate = Vec::new();
    for candidate in found {
        if proposer_counts
            .get(&candidate.1.sender)
            .copied()
            .unwrap_or_default()
            > 1
        {
            duplicate.push(candidate);
        } else {
            unique.push(candidate);
        }
    }

    unique.sort_by_key(|(position, candidate)| (Reverse(candidate.level), *position));
    duplicate.sort_by_key(|(position, candidate)| (Reverse(candidate.level), *position));

    let mut selected = Vec::new();
    let mut gas_used = 0_u64;
    for (_, candidate) in unique.into_iter().chain(duplicate) {
        gas_used = gas_used.saturating_add(candidate.gas_estimation);
        if gas_used > gas_limit || selected.len() == usize::from(max_tips) {
            break;
        }
        selected.push(DagHash {
            hash: candidate.hash,
        });
    }

    DagProposerTipSelection {
        selected,
        skipped_missing,
    }
}

fn dag_proposer_decision(
    action: u8,
    reason_code: u32,
    vote_count: u64,
    max_vote_count: u64,
) -> DagProposerEligibilityDecision {
    DagProposerEligibilityDecision {
        action,
        reason_code,
        vote_count,
        max_vote_count,
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

fn to_domain_sortition_params(params: SortitionRuntimeParams) -> SortitionParams {
    SortitionParams {
        vrf: VrfParams {
            threshold_upper: params.threshold_upper,
        },
        vdf: VdfParams {
            difficulty_min: params.difficulty_min,
            difficulty_max: params.difficulty_max,
            difficulty_stale: params.difficulty_stale,
            lambda_bound: params.lambda_bound,
        },
    }
}

fn to_vrf_output(vrf_output: Vec<u8>) -> Result<[u8; 64]> {
    const VRF_OUTPUT_BYTES: usize = 64;

    ensure!(
        vrf_output.len() == VRF_OUTPUT_BYTES,
        "VRF output must be 64 bytes"
    );
    let mut out = [0_u8; VRF_OUTPUT_BYTES];
    out.copy_from_slice(&vrf_output);
    Ok(out)
}

fn to_dag_hashes(hashes: Vec<H256>) -> Vec<DagHash> {
    hashes
        .into_iter()
        .map(|hash| DagHash { hash: hash.0 })
        .collect()
}

fn to_transaction_hashes(hashes: Vec<DagTransactionHash>) -> Vec<H256> {
    hashes.into_iter().map(|hash| hash.hash.into()).collect()
}

fn to_bridge_transaction_hashes(hashes: Vec<H256>) -> Vec<DagTransactionHash> {
    hashes
        .into_iter()
        .map(|hash| DagTransactionHash { hash: hash.0 })
        .collect()
}

fn to_bridge_sync_blocks(
    blocks: Vec<rustaxa_consensus::dag::DagSyncBlockRlp>,
) -> Vec<DagSyncBlockRlp> {
    blocks
        .into_iter()
        .map(|block| DagSyncBlockRlp {
            hash: block.hash.into(),
            block_rlp: block.block_rlp,
        })
        .collect()
}

fn to_bridge_transaction_rlp_lookups(
    lookups: Vec<rustaxa_consensus::dag::DagTransactionStorageLookup>,
) -> Vec<DagTransactionRlpLookup> {
    lookups
        .into_iter()
        .map(|lookup| DagTransactionRlpLookup {
            hash: lookup.hash.into(),
            found: lookup.found,
            finalized: lookup.finalized,
            tx_rlp: lookup.tx_rlp,
        })
        .collect()
}

fn to_bridge_frontier(frontier: &rustaxa_consensus::dag::DagFrontier) -> DagFrontier {
    DagFrontier {
        pivot: frontier.pivot.into(),
        tips: to_dag_hashes(frontier.tips.clone()),
    }
}

fn to_bridge_finalization_plan(
    plan: DomainDagManagerFinalizationPlan,
) -> DagManagerFinalizationPlan {
    DagManagerFinalizationPlan {
        finalized_count: plan.finalized_count as u64,
        counter_update_hashes: to_dag_hashes(plan.counter_update_hashes),
        expired_hashes: to_dag_hashes(plan.expired_hashes),
        remaining_hashes: to_dag_hashes(plan.remaining_hashes),
        remove_transaction_hashes: Vec::new(),
    }
}

fn to_bridge_finalization_cleanup_payload(
    payload: DomainDagManagerFinalizationCleanupStoragePayload,
) -> DagManagerFinalizationCleanupPayload {
    DagManagerFinalizationCleanupPayload {
        counter_updates: payload
            .counter_updates
            .into_iter()
            .map(|update| DagFinalizedCounterUpdate {
                hash: update.hash.into(),
                level: update.level,
                tips_count: update.tips_count,
            })
            .collect(),
        expired_hashes: to_dag_hashes(payload.expired_hashes),
        remove_transaction_hashes: to_bridge_transaction_hashes(payload.remove_transaction_hashes),
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
    use crate::ffi::rustaxa_ffi::DagDposAuthorizationFacts;
    use crate::storage::create_storage;
    use rlp::RlpStream;
    use rustaxa_consensus::dag;
    use rustaxa_vdf::prover::CancellationToken;
    use rustaxa_vdf::sortition::{self, LegacySortitionParams};
    use rustaxa_vdf::vrf::public_key_from_secret;
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

    fn dag_block_with_vdf_payload(vdf_payload: Vec<u8>) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&&[0u8; 32][..]);
        block.append(&1u64);
        block.append(&0u64);
        block.append(&vdf_payload);
        block.begin_list(0);
        block.begin_list(0);
        block.append(&&[0u8; 65][..]);
        block.append(&123u64);
        block.out().to_vec()
    }

    fn dag_block_with_vdf_payload_and_transaction_hashes(
        vdf_payload: Vec<u8>,
        transaction_hashes: &[DagTransactionHash],
    ) -> Vec<u8> {
        dag_block_with_level_and_transaction_hashes(1, vdf_payload, transaction_hashes)
    }

    fn dag_block_with_level_and_transaction_hashes(
        level: u64,
        vdf_payload: Vec<u8>,
        transaction_hashes: &[DagTransactionHash],
    ) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&&[0u8; 32][..]);
        block.append(&level);
        block.append(&0u64);
        block.append(&vdf_payload);
        block.begin_list(0);
        block.begin_list(transaction_hashes.len());
        for hash in transaction_hashes {
            block.append(&&hash.hash[..]);
        }
        block.append(&&[0u8; 65][..]);
        block.append(&123u64);
        block.out().to_vec()
    }

    fn tx_hash(byte: u8) -> DagTransactionHash {
        DagTransactionHash { hash: [byte; 32] }
    }

    fn signed_pbft_block(period: u64, timestamp: u64) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from_low_u64_be(10));
        block.append(&H256::from_low_u64_be(11));
        block.append(&H256::from_low_u64_be(12));
        block.append(&H256::from_low_u64_be(13));
        block.append(&period);
        block.append(&timestamp);
        block.begin_list(0);
        block.append(&vec![0u8; 65]);
        block.out().to_vec()
    }

    fn period_data_with_pbft_block(pbft_block: &[u8]) -> Vec<u8> {
        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(pbft_block, 1);
        period_data.append_empty_data();
        period_data.append_empty_data();
        period_data.begin_list(0);
        period_data.out().to_vec()
    }

    const SECRET_KEY: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

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

            let runtime_lookup = runtime
                .dag_manager_runtime_proposal_period_for_level(100)
                .expect("runtime lookup should succeed");
            assert!(runtime_lookup.found);
            assert_eq!(runtime_lookup.period, 0);

            assert!(!runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(100, 0)
                .expect("idempotent ensure should succeed"));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_period_block_hash_uses_rust_period_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_period_hash");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            let missing = runtime
                .dag_manager_runtime_period_block_hash(7)
                .expect("missing period lookup should succeed");
            assert!(!missing.found);
            assert_eq!(missing.hash, [0; 32]);

            let pbft_block = signed_pbft_block(7, 99);
            let expected_hash: [u8; 32] =
                PbftBlockLink::try_from(SignedPbftBlockRlp::new(&pbft_block))
                    .expect("test PBFT block should decode")
                    .block_hash
                    .into();
            storage
                .save_period_data(7, period_data_with_pbft_block(&pbft_block))
                .expect("period data save should succeed");

            let found = runtime
                .dag_manager_runtime_period_block_hash(7)
                .expect("period hash lookup should succeed");
            assert!(found.found);
            assert_eq!(found.hash, expected_hash);

            storage
                .save_period_data(8, vec![0x80])
                .expect("corrupt period data save should succeed");
            assert!(runtime.dag_manager_runtime_period_block_hash(8).is_err());
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
    fn dag_manager_runtime_set_finalized_order_updates_graph_state() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_set_finalized_order");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block 2");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [2u8; 32],
                    tips: vec![DagHash { hash: [1u8; 32] }],
                    level: 3,
                    difficulty: 80,
                })
                .expect("add block 3");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [2u8; 32],
                    tips: vec![DagHash { hash: [3u8; 32] }],
                    level: 4,
                    difficulty: 60,
                })
                .expect("add block 4");

            let removed = runtime
                .dag_manager_runtime_set_finalized_order(
                    [4u8; 32],
                    4,
                    1,
                    vec![
                        DagHash { hash: [2u8; 32] },
                        DagHash { hash: [3u8; 32] },
                        DagHash { hash: [4u8; 32] },
                    ],
                )
                .expect("set finalized order should succeed");
            assert_eq!(removed.finalized_count, 3);
            assert!(removed.counter_update_hashes.is_empty());
            assert!(removed.expired_hashes.is_empty());
            assert!(removed.remaining_hashes.is_empty());

            let anchors = runtime.dag_manager_runtime_anchors();
            assert_eq!(anchors.old_anchor, [1u8; 32]);
            assert_eq!(anchors.anchor, [4u8; 32]);
            assert_eq!(runtime.dag_manager_runtime_latest_period(), 1);

            let non_finalized = runtime.dag_manager_runtime_non_finalized_blocks();
            assert!(non_finalized.is_empty());
            assert_eq!(
                runtime.dag_manager_runtime_non_finalized_min_difficulty(),
                u32::MAX
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_set_finalized_order_reports_expiry_plan() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_finalized_order_expiry");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 2, &storage)
                .expect("runtime should initialize");

            for block in [
                DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                },
                DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [2u8; 32],
                    tips: vec![],
                    level: 3,
                    difficulty: 90,
                },
                DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 80,
                },
                DagManagerBlock {
                    hash: [5u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 1,
                    difficulty: 70,
                },
                DagManagerBlock {
                    hash: [6u8; 32],
                    pivot: [5u8; 32],
                    tips: vec![],
                    level: 6,
                    difficulty: 60,
                },
            ] {
                runtime
                    .dag_manager_runtime_add_block(block)
                    .expect("add block");
            }

            let plan = runtime
                .dag_manager_runtime_set_finalized_order(
                    [4u8; 32],
                    4,
                    1,
                    vec![
                        DagHash { hash: [2u8; 32] },
                        DagHash { hash: [3u8; 32] },
                        DagHash { hash: [4u8; 32] },
                    ],
                )
                .expect("set finalized order should succeed");

            assert_eq!(
                plan.expired_hashes
                    .iter()
                    .map(|hash| hash.hash)
                    .collect::<Vec<_>>(),
                vec![[5u8; 32], [6u8; 32]]
            );
            assert!(plan.remaining_hashes.is_empty());
            assert_eq!(runtime.dag_manager_runtime_dag_expiry_level(), 2);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_select_non_finalized_hashes_excludes_known_hashes() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_select_hashes");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 3,
                    difficulty: 90,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [6u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 85,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 80,
                })
                .expect("add block");

            let selected = runtime.dag_manager_runtime_select_non_finalized_hashes(vec![
                DagHash { hash: [2u8; 32] },
                DagHash { hash: [9u8; 32] },
                DagHash { hash: [2u8; 32] },
            ]);
            let selected = selected.iter().map(|hash| hash.hash).collect::<Vec<_>>();
            assert_eq!(selected, vec![[3u8; 32], [4u8; 32], [6u8; 32]]);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_empty_period_preserves_anchors() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_empty_period");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block");

            let finalized_count = runtime
                .dag_manager_runtime_set_finalized_order([0u8; 32], 0, 1, vec![])
                .expect("empty period should advance");
            assert_eq!(finalized_count.finalized_count, 0);

            let anchors = runtime.dag_manager_runtime_anchors();
            assert_eq!(anchors.old_anchor, [0u8; 32]);
            assert_eq!(anchors.anchor, [1u8; 32]);
            assert_eq!(runtime.dag_manager_runtime_latest_period(), 1);
            assert_eq!(
                runtime
                    .dag_manager_runtime_non_finalized_blocks_size()
                    .blocks,
                1
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_non_finalized_sync_snapshot_includes_period_and_selected_hashes() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_sync_snapshot");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 3,
                    difficulty: 90,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [6u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 85,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 80,
                })
                .expect("add block");

            let snapshot = runtime.dag_manager_runtime_non_finalized_sync_snapshot(vec![
                DagHash { hash: [2u8; 32] },
                DagHash { hash: [9u8; 32] },
                DagHash { hash: [2u8; 32] },
            ]);
            let selected = snapshot
                .selected_hashes
                .into_iter()
                .map(|hash| hash.hash)
                .collect::<Vec<_>>();

            assert_eq!(snapshot.period, 0);
            assert_eq!(selected, vec![[3u8; 32], [4u8; 32], [6u8; 32]]);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_non_finalized_sync_payload_uses_storage_and_dedupes_transactions() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_sync_payload");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block 2");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [2u8; 32],
                    tips: vec![DagHash { hash: [2u8; 32] }],
                    level: 3,
                    difficulty: 90,
                })
                .expect("add block 3");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![DagHash { hash: [3u8; 32] }],
                    level: 4,
                    difficulty: 80,
                })
                .expect("add block 4");

            let tx_block_3 = dag_block_with_vdf_payload_and_transaction_hashes(
                vec![0x11],
                &[tx_hash(1), tx_hash(2)],
            );
            let tx_block_4 = dag_block_with_vdf_payload_and_transaction_hashes(
                vec![0x22],
                &[tx_hash(2), tx_hash(4)],
            );

            runtime
                .dag_manager_runtime_save_block(&[3u8; 32], 3, 2, tx_block_3.clone())
                .expect("persist block 3");
            runtime
                .dag_manager_runtime_save_block(&[4u8; 32], 4, 2, tx_block_4.clone())
                .expect("persist block 4");

            storage
                .save_transaction(&[1u8; 32], vec![0xA1, 0x01])
                .expect("persist pending transaction 1");
            storage
                .save_transaction(&[2u8; 32], vec![0xA2, 0x02])
                .expect("persist pending transaction 2");
            storage
                .save_transaction(&[3u8; 32], vec![0xA3, 0x03])
                .expect("persist pending transaction 3");

            let payload = runtime
                .dag_manager_runtime_non_finalized_sync_payload(vec![DagHash { hash: [2u8; 32] }])
                .expect("sync payload should materialize");

            assert_eq!(payload.period, 0);
            assert_eq!(payload.blocks.len(), 2);
            assert_eq!(payload.blocks[0].hash, [3u8; 32]);
            assert_eq!(payload.blocks[0].block_rlp, tx_block_3);
            assert_eq!(payload.blocks[1].hash, [4u8; 32]);
            assert_eq!(payload.blocks[1].block_rlp, tx_block_4);

            assert_eq!(payload.transactions.len(), 3);
            assert_eq!(payload.transactions[0].hash, tx_hash(1).hash);
            assert!(payload.transactions[0].found);
            assert_eq!(payload.transactions[0].tx_rlp, vec![0xA1, 0x01]);
            assert_eq!(payload.transactions[1].hash, tx_hash(2).hash);
            assert!(payload.transactions[1].found);
            assert_eq!(payload.transactions[1].tx_rlp, vec![0xA2, 0x02]);
            assert_eq!(payload.transactions[2].hash, tx_hash(4).hash);
            assert!(!payload.transactions[2].found);
            assert!(payload.transactions[2].tx_rlp.is_empty());
            assert!(!payload.transactions[2].finalized);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_expired_transaction_cleanup_payload_checks_finalized_and_retained_refs()
    {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_finalization_cleanup_payload");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            let expired_block_a = dag_block_with_vdf_payload_and_transaction_hashes(
                vec![0x11],
                &[tx_hash(1), tx_hash(2), tx_hash(1)],
            );
            let expired_block_b =
                dag_block_with_vdf_payload_and_transaction_hashes(vec![0x22], &[tx_hash(3)]);
            let remaining_block =
                dag_block_with_vdf_payload_and_transaction_hashes(vec![0x33], &[tx_hash(3)]);

            runtime
                .dag_manager_runtime_save_block(&[3u8; 32], 3, 3, expired_block_a)
                .expect("persist expired block a");
            runtime
                .dag_manager_runtime_save_block(&[4u8; 32], 4, 1, expired_block_b)
                .expect("persist expired block b");
            runtime
                .dag_manager_runtime_save_block(&[6u8; 32], 6, 1, remaining_block)
                .expect("persist remaining block");

            storage
                .save_transaction_location(&[2u8; 32], 7, 0, false)
                .expect("mark tx2 as finalized");

            let payload = runtime
                .dag_manager_runtime_expired_transaction_cleanup_payload(
                    vec![DagHash { hash: [3u8; 32] }, DagHash { hash: [4u8; 32] }],
                    vec![DagHash { hash: [6u8; 32] }],
                )
                .expect("finalization cleanup payload should compute");

            let facts = payload.expired_transaction_facts;
            assert_eq!(facts.len(), 4);
            assert_eq!(facts[0].hash, [1u8; 32]);
            assert!(!facts[0].finalized);
            assert_eq!(facts[1].hash, [2u8; 32]);
            assert!(facts[1].finalized);
            assert_eq!(facts[2].hash, [1u8; 32]);
            assert!(!facts[2].finalized);
            assert_eq!(facts[3].hash, [3u8; 32]);
            assert!(!facts[3].finalized);

            let remove_hashes = payload.remove_hashes;
            assert_eq!(remove_hashes.len(), 1);
            assert_eq!(remove_hashes[0].hash, [1u8; 32]);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_finalization_cleanup_payload_returns_storage_backed_side_effects() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_finalization_payload");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_save_block(
                    &[8u8; 32],
                    1,
                    0,
                    dag_block_with_vdf_payload(vec![0x88]),
                )
                .expect("persist finalized block needing counter update");
            runtime
                .dag_manager_runtime_save_block(
                    &[3u8; 32],
                    3,
                    3,
                    dag_block_with_vdf_payload_and_transaction_hashes(
                        vec![0x11],
                        &[tx_hash(1), tx_hash(2), tx_hash(1)],
                    ),
                )
                .expect("persist expired block a");
            runtime
                .dag_manager_runtime_save_block(
                    &[4u8; 32],
                    4,
                    1,
                    dag_block_with_vdf_payload_and_transaction_hashes(vec![0x22], &[tx_hash(3)]),
                )
                .expect("persist expired block b");
            runtime
                .dag_manager_runtime_save_block(
                    &[6u8; 32],
                    6,
                    1,
                    dag_block_with_vdf_payload_and_transaction_hashes(vec![0x33], &[tx_hash(3)]),
                )
                .expect("persist remaining block");

            storage
                .save_transaction_location(&[2u8; 32], 7, 0, false)
                .expect("mark tx2 as finalized");

            let payload = runtime
                .dag_manager_runtime_finalization_cleanup_payload(DagManagerFinalizationPlan {
                    finalized_count: 2,
                    counter_update_hashes: vec![DagHash { hash: [8u8; 32] }],
                    expired_hashes: vec![DagHash { hash: [3u8; 32] }, DagHash { hash: [4u8; 32] }],
                    remaining_hashes: vec![DagHash { hash: [6u8; 32] }],
                    remove_transaction_hashes: vec![],
                })
                .expect("finalization cleanup payload should compute");

            assert_eq!(payload.counter_updates.len(), 1);
            assert_eq!(payload.counter_updates[0].hash, [8u8; 32]);
            assert_eq!(payload.counter_updates[0].level, 1);
            assert_eq!(payload.counter_updates[0].tips_count, 0);

            assert_eq!(payload.expired_hashes.len(), 2);
            assert_eq!(payload.expired_hashes[0].hash, [3u8; 32]);
            assert_eq!(payload.expired_hashes[1].hash, [4u8; 32]);

            assert_eq!(payload.remove_transaction_hashes.len(), 1);
            assert_eq!(payload.remove_transaction_hashes[0].hash, [1u8; 32]);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_apply_finalized_order_writes_storage_and_commits_state() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_apply_finalized_order");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 1, &storage)
                .expect("runtime should initialize");

            for block in [
                DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 3,
                    difficulty: 90,
                },
                DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 80,
                },
                DagManagerBlock {
                    hash: [6u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 6,
                    difficulty: 70,
                },
            ] {
                runtime
                    .dag_manager_runtime_add_block(block)
                    .expect("add non-finalized block");
            }

            runtime
                .dag_manager_runtime_save_block(
                    &[8u8; 32],
                    5,
                    0,
                    dag_block_with_level_and_transaction_hashes(5, vec![0x88], &[]),
                )
                .expect("persist finalized anchor block");
            runtime
                .dag_manager_runtime_save_block(
                    &[3u8; 32],
                    3,
                    0,
                    dag_block_with_level_and_transaction_hashes(
                        3,
                        vec![0x11],
                        &[tx_hash(1), tx_hash(2)],
                    ),
                )
                .expect("persist expired block a");
            runtime
                .dag_manager_runtime_save_block(
                    &[4u8; 32],
                    4,
                    0,
                    dag_block_with_level_and_transaction_hashes(4, vec![0x22], &[tx_hash(3)]),
                )
                .expect("persist expired dependent block");
            runtime
                .dag_manager_runtime_save_block(
                    &[6u8; 32],
                    6,
                    0,
                    dag_block_with_level_and_transaction_hashes(6, vec![0x33], &[tx_hash(3)]),
                )
                .expect("persist remaining block");

            storage
                .save_transaction_location(&[2u8; 32], 7, 0, false)
                .expect("mark tx2 as finalized");
            storage
                .save_transaction(&[1u8; 32], vec![0xA1])
                .expect("persist expired pending tx1");
            storage
                .save_transaction(&[2u8; 32], vec![0xA2])
                .expect("persist finalized pending tx2");
            storage
                .save_transaction(&[3u8; 32], vec![0xA3])
                .expect("persist retained pending tx3");

            let payload = runtime
                .dag_manager_runtime_apply_finalized_order(
                    [8u8; 32],
                    1,
                    vec![DagHash { hash: [8u8; 32] }],
                )
                .expect("apply finalized order");

            assert_eq!(payload.finalized_count, 1);
            assert_eq!(payload.expired_hashes.len(), 2);
            assert_eq!(payload.expired_hashes[0].hash, [3u8; 32]);
            assert_eq!(payload.expired_hashes[1].hash, [4u8; 32]);
            assert_eq!(payload.remove_transaction_hashes.len(), 1);
            assert_eq!(payload.remove_transaction_hashes[0].hash, [1u8; 32]);
            assert_eq!(
                storage
                    .get_transaction(&[1u8; 32])
                    .expect("load removed pending tx1"),
                Vec::<u8>::new()
            );
            assert_eq!(
                storage
                    .get_transaction(&[2u8; 32])
                    .expect("load finalized pending tx2"),
                vec![0xA2]
            );
            assert_eq!(
                storage
                    .get_transaction(&[3u8; 32])
                    .expect("load retained pending tx3"),
                vec![0xA3]
            );

            assert_eq!(runtime.dag_manager_runtime_latest_period(), 1);
            assert_eq!(runtime.dag_manager_runtime_anchors().anchor, [8u8; 32]);
            assert!(
                !runtime
                    .dag_manager_runtime_load_block(&[3u8; 32])
                    .expect("load removed block")
                    .found
            );
            assert!(
                !runtime
                    .dag_manager_runtime_load_block(&[4u8; 32])
                    .expect("load removed dependent block")
                    .found
            );
            assert!(
                runtime
                    .dag_manager_runtime_load_block(&[6u8; 32])
                    .expect("load remaining block")
                    .found
            );

            let counters = runtime
                .dag_manager_runtime_persistence_counters()
                .expect("load counters");
            assert_eq!(counters.dag_blocks, 5);
            assert_eq!(counters.dag_edges, 5);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_apply_finalized_order_requires_anchor_in_storage_before_state_commit() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_apply_missing_anchor");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 1, &storage)
                .expect("runtime should initialize");

            let err = match runtime.dag_manager_runtime_apply_finalized_order(
                [8u8; 32],
                1,
                vec![DagHash { hash: [8u8; 32] }],
            ) {
                Ok(_) => panic!("missing anchor should fail"),
                Err(err) => err,
            };

            assert!(err
                .to_string()
                .contains("DAG_RUNTIME_FINALIZATION_ANCHOR_BLOCK"));
            assert_eq!(runtime.dag_manager_runtime_latest_period(), 0);
            assert_eq!(runtime.dag_manager_runtime_anchors().anchor, [1u8; 32]);
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

    #[test]
    fn dag_plan_verify_transaction_query_preserves_missing_block_order() {
        let plan = dag_plan_verify_transaction_query(
            vec![tx_hash(1), tx_hash(2), tx_hash(3), tx_hash(1), tx_hash(2)],
            vec![tx_hash(2), tx_hash(9)],
        );
        assert_eq!(plan.query_hashes.len(), 2);
        assert_eq!(plan.query_hashes[0].hash, [1u8; 32]);
        assert_eq!(plan.query_hashes[1].hash, [3u8; 32]);
    }

    #[test]
    fn dag_plan_non_finalized_transaction_query_deduplicates_first_seen_order() {
        let plan = dag_plan_non_finalized_transaction_query(vec![
            DagBlockTransactionRefs {
                transaction_hashes: vec![tx_hash(1), tx_hash(2), tx_hash(1)],
            },
            DagBlockTransactionRefs {
                transaction_hashes: vec![tx_hash(3), tx_hash(2)],
            },
        ]);
        assert_eq!(plan.query_hashes.len(), 3);
        assert_eq!(plan.query_hashes[0].hash, [1u8; 32]);
        assert_eq!(plan.query_hashes[1].hash, [2u8; 32]);
        assert_eq!(plan.query_hashes[2].hash, [3u8; 32]);
    }

    #[test]
    fn dag_plan_expired_transaction_cleanup_skips_finalized_and_retained_refs() {
        let plan = dag_plan_expired_transaction_cleanup(
            vec![
                DagExpiredTransactionFact {
                    hash: [1u8; 32],
                    finalized: false,
                },
                DagExpiredTransactionFact {
                    hash: [2u8; 32],
                    finalized: true,
                },
                DagExpiredTransactionFact {
                    hash: [3u8; 32],
                    finalized: false,
                },
                DagExpiredTransactionFact {
                    hash: [1u8; 32],
                    finalized: false,
                },
            ],
            vec![tx_hash(3)],
        );

        assert_eq!(plan.remove_hashes.len(), 1);
        assert_eq!(plan.remove_hashes[0].hash, [1u8; 32]);
    }

    #[test]
    fn dag_verify_vdf_prepare_and_authorization_bridge_decisions() {
        let missing_vrf = dag_verify_vdf_prepare(DagVerifyVdfPrepareInput {
            vrf_key_found: false,
            eligible_vote_count: 12,
            vdf_max_vote_count: 77,
        });
        assert!(!missing_vrf.continue_validation);
        assert_eq!(
            missing_vrf.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );
        assert_eq!(
            missing_vrf.reason_code,
            rustaxa_consensus::dag::DAG_VERIFY_REASON_MISSING_VRF_KEY
        );

        let prepared = dag_verify_vdf_prepare(DagVerifyVdfPrepareInput {
            vrf_key_found: true,
            eligible_vote_count: 12,
            vdf_max_vote_count: 77,
        });
        assert!(prepared.continue_validation);
        assert_eq!(prepared.reject_code, 0);
        assert_eq!(prepared.vote_count, 12);
        assert_eq!(prepared.max_vote_count, 77);

        let future_snapshot = dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: true,
            dpos_snapshot_available: false,
            dpos_eligible: false,
        });
        assert!(!future_snapshot.continue_validation);
        assert_eq!(
            future_snapshot.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_FUTURE_BLOCK
        );
        assert_eq!(
            future_snapshot.reason_code,
            rustaxa_consensus::dag::DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT
        );

        let not_eligible = dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: true,
            dpos_snapshot_available: true,
            dpos_eligible: false,
        });
        assert!(!not_eligible.continue_validation);
        assert_eq!(
            not_eligible.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_NOT_ELIGIBLE
        );

        let continues = dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: true,
            dpos_snapshot_available: true,
            dpos_eligible: true,
        });
        assert!(continues.continue_validation);
        assert_eq!(continues.reject_code, 0);

        let combined_missing_vrf = dag_decide_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: false,
            sender_eligible_vote_count: 12,
            vdf_sortition_max_vote_count: 77,
            vdf_status: rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_INVALID,
            dpos_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
        });
        assert!(!combined_missing_vrf.continue_validation);
        assert_eq!(
            combined_missing_vrf.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );
        assert_eq!(
            combined_missing_vrf.reason_code,
            rustaxa_consensus::dag::DAG_VERIFY_REASON_MISSING_VRF_KEY
        );
        assert_eq!(combined_missing_vrf.vote_count, 12);
        assert_eq!(combined_missing_vrf.max_vote_count, 77);

        let combined_invalid_vdf = dag_decide_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: true,
            sender_eligible_vote_count: 12,
            vdf_sortition_max_vote_count: 77,
            vdf_status: rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_INVALID,
            dpos_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
        });
        assert!(!combined_invalid_vdf.continue_validation);
        assert_eq!(
            combined_invalid_vdf.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );
        assert_eq!(
            combined_invalid_vdf.reason_code,
            rustaxa_consensus::dag::DAG_VERIFY_REASON_INVALID_VDF
        );

        let combined_future_snapshot = dag_decide_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: true,
            sender_eligible_vote_count: 12,
            vdf_sortition_max_vote_count: 77,
            vdf_status: rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_NOT_CHECKED,
            dpos_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
        });
        assert!(!combined_future_snapshot.continue_validation);
        assert_eq!(
            combined_future_snapshot.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_FUTURE_BLOCK
        );
        assert_eq!(
            combined_future_snapshot.reason_code,
            rustaxa_consensus::dag::DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT
        );

        let combined_not_eligible = dag_decide_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: true,
            sender_eligible_vote_count: 12,
            vdf_sortition_max_vote_count: 77,
            vdf_status: rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_VALID,
            dpos_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_NOT_ELIGIBLE,
        });
        assert!(!combined_not_eligible.continue_validation);
        assert_eq!(
            combined_not_eligible.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_NOT_ELIGIBLE
        );
        assert_eq!(
            combined_not_eligible.reason_code,
            rustaxa_consensus::dag::DAG_VERIFY_REASON_NOT_ELIGIBLE
        );

        let combined_continues = dag_decide_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: true,
            sender_eligible_vote_count: 12,
            vdf_sortition_max_vote_count: 77,
            vdf_status: rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_VALID,
            dpos_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
        });
        assert!(combined_continues.continue_validation);
        assert_eq!(combined_continues.reject_code, 0);
        assert_eq!(combined_continues.vote_count, 12);
        assert_eq!(combined_continues.max_vote_count, 77);
    }

    #[test]
    fn dag_vdf_sortition_legacy_output_bridge_verification() {
        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&vec![0x11u8; 80]);
        vdf_payload.append(&vec![0x22u8]);
        vdf_payload.append(&vec![0x33u8]);
        vdf_payload.append(&1u16);

        let block_rlp = dag_block_with_vdf_payload(vdf_payload.out().to_vec());

        let result = dag_verify_vdf_sortition(DagVerifyVdfSortitionInput {
            block_rlp,
            vdf_input: vec![0x01],
            sortition_params: SortitionRuntimeParams {
                threshold_upper: 1000,
                difficulty_min: 1,
                difficulty_max: 1,
                difficulty_stale: 1,
                lambda_bound: 6,
            },
            vrf_output: vec![0u8; 64],
            vrf_public_key: Vec::new(),
            vrf_input: Vec::new(),
            sender_eligible_vote_count: 100,
            vdf_sortition_max_vote_count: 100,
        })
        .expect("verification should return a result");

        assert_eq!(result.vdf_status, dag::DAG_VERIFY_VDF_STATUS_INVALID);
        assert_eq!(result.difficulty, 1);
        assert_eq!(result.expected_difficulty, 1);
    }

    #[test]
    fn dag_vdf_sortition_verifies_embedded_vrf_proof() {
        let sortition_input = LegacySortitionParams {
            vrf_threshold_upper: 0x5ff,
            vdf_difficulty_min: 5,
            vdf_difficulty_max: 10,
            vdf_difficulty_stale: 9,
            vdf_lambda_bound: 64,
        };
        let vrf_input = vec![0xA1, 0x02, 0x03];
        let vdf_input = vec![0xB1, 0x04];
        let proof = sortition::prove_legacy_vdf_sortition(
            sortition_input,
            &SECRET_KEY,
            &vrf_input,
            &vdf_input,
            1,
            1,
            &CancellationToken::new(),
        )
        .expect("proof generation should succeed");
        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&&proof.vrf_proof[..]);
        vdf_payload.append(&proof.vdf_proof);
        vdf_payload.append(&proof.vdf_output);
        vdf_payload.append(&proof.difficulty);
        let block_rlp = dag_block_with_vdf_payload(vdf_payload.out().to_vec());
        let vrf_public_key =
            public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");

        let result = dag_verify_vdf_sortition(DagVerifyVdfSortitionInput {
            block_rlp,
            vdf_input,
            sortition_params: SortitionRuntimeParams {
                threshold_upper: 0x5ff,
                difficulty_min: 5,
                difficulty_max: 10,
                difficulty_stale: 9,
                lambda_bound: 64,
            },
            vrf_output: Vec::new(),
            vrf_public_key: vrf_public_key.to_vec(),
            vrf_input,
            sender_eligible_vote_count: 1,
            vdf_sortition_max_vote_count: 1,
        })
        .expect("verification should return a result");

        assert_eq!(result.vdf_status, dag::DAG_VERIFY_VDF_STATUS_VALID);
        assert_eq!(result.difficulty, result.expected_difficulty);
    }

    #[test]
    fn dag_verify_vdf_sortition_from_block_constructs_and_verifies_embedded_inputs() {
        let sortition_input = LegacySortitionParams {
            vrf_threshold_upper: 0x5ff,
            vdf_difficulty_min: 5,
            vdf_difficulty_max: 10,
            vdf_difficulty_stale: 9,
            vdf_lambda_bound: 64,
        };
        let proposal_period_hash = [9u8; 32];
        let block_level = 1;
        let vrf_input = dag::construct_dag_vrf_input(block_level, H256::from(proposal_period_hash));
        let block_rlp = dag_block_with_vdf_payload(vec![0; 0]);
        let vdf_input = dag::construct_dag_vdf_message_from_block_rlp(&block_rlp)
            .expect("VDF input should build");

        let proof = sortition::prove_legacy_vdf_sortition(
            sortition_input,
            &SECRET_KEY,
            &vrf_input,
            &vdf_input,
            1,
            1,
            &CancellationToken::new(),
        )
        .expect("proof generation should succeed");

        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&&proof.vrf_proof[..]);
        vdf_payload.append(&proof.vdf_proof);
        vdf_payload.append(&proof.vdf_output);
        vdf_payload.append(&proof.difficulty);

        let block_rlp = dag_block_with_vdf_payload(vdf_payload.out().to_vec());
        let vrf_public_key =
            public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");

        let result = dag_verify_vdf_sortition_from_block(DagVerifyVdfSortitionFromBlockInput {
            block_rlp,
            block_level,
            proposal_period_hash,
            sortition_params: SortitionRuntimeParams {
                threshold_upper: 0x5ff,
                difficulty_min: 5,
                difficulty_max: 10,
                difficulty_stale: 9,
                lambda_bound: 64,
            },
            vrf_public_key,
            sender_eligible_vote_count: 1,
            vdf_sortition_max_vote_count: 1,
        })
        .expect("embedded bridge verification should succeed");

        assert_eq!(result.vdf_status, dag::DAG_VERIFY_VDF_STATUS_VALID);
        assert_eq!(result.difficulty, result.expected_difficulty);
    }

    #[test]
    fn dag_vdf_message_bridge_uses_legacy_pivot_and_transaction_rlp() {
        let pivot = [0x11_u8; 32];
        let tx_hashes = vec![
            DagHash {
                hash: [0x22_u8; 32],
            },
            DagHash {
                hash: [0x33_u8; 32],
            },
        ];

        let mut expected = RlpStream::new();
        expected.append(&H256::from(pivot));
        expected.append(&H256::from(tx_hashes[0].hash));
        expected.append(&H256::from(tx_hashes[1].hash));

        assert_eq!(dag_vdf_message(&pivot, tx_hashes), expected.out().to_vec());
    }

    #[test]
    fn dag_vrf_input_bridge_uses_legacy_level_and_period_hash_rlp() {
        let block_level = 7;
        let proposal_period_hash = [0x44_u8; 32];

        let mut expected = RlpStream::new();
        expected.append(&block_level);
        expected.append(&H256::from(proposal_period_hash));

        assert_eq!(
            dag_vrf_input(block_level, &proposal_period_hash),
            expected.out().to_vec()
        );
    }

    #[test]
    fn dag_proposer_eligibility_returns_status_decisions() {
        let wallet_vrf_public_key = [0x55_u8; 32];
        let continues = dag_proposer_check_eligibility(DagProposerEligibilityInput {
            proposal_period_found: true,
            wallet_vrf_public_key,
            authorization_facts: DagDposAuthorizationFacts {
                vrf_key_found: true,
                vrf_key: wallet_vrf_public_key.to_vec(),
                sender_eligible_vote_count: 12,
                vdf_sortition_max_vote_count: 30,
                eligibility_status: dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
            },
        });
        assert_eq!(continues.action, DAG_PROPOSER_ACTION_CONTINUE);
        assert_eq!(continues.reason_code, DAG_PROPOSER_REASON_OK);
        assert_eq!(continues.vote_count, 12);
        assert_eq!(continues.max_vote_count, 30);

        let mismatch = dag_proposer_check_eligibility(DagProposerEligibilityInput {
            proposal_period_found: true,
            wallet_vrf_public_key,
            authorization_facts: DagDposAuthorizationFacts {
                vrf_key_found: true,
                vrf_key: vec![0x66; 32],
                sender_eligible_vote_count: 12,
                vdf_sortition_max_vote_count: 30,
                eligibility_status: dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
            },
        });
        assert_eq!(mismatch.action, DAG_PROPOSER_ACTION_SKIP);
        assert_eq!(mismatch.reason_code, DAG_PROPOSER_REASON_VRF_KEY_MISMATCH);

        let unavailable = dag_proposer_check_eligibility(DagProposerEligibilityInput {
            proposal_period_found: true,
            wallet_vrf_public_key,
            authorization_facts: DagDposAuthorizationFacts {
                vrf_key_found: true,
                vrf_key: wallet_vrf_public_key.to_vec(),
                sender_eligible_vote_count: 0,
                vdf_sortition_max_vote_count: 0,
                eligibility_status: dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
            },
        });
        assert_eq!(unavailable.action, DAG_PROPOSER_ACTION_RETRY_LATER);
        assert_eq!(
            unavailable.reason_code,
            DAG_PROPOSER_REASON_DPOS_UNAVAILABLE
        );
    }

    #[test]
    fn dag_proposer_tip_selection_skips_missing_and_prefers_unique_higher_levels() {
        let candidates = vec![
            DagProposerTipCandidate {
                hash: [0x01; 32],
                found: true,
                sender: [0xA1; 20],
                level: 1,
                gas_estimation: 100,
            },
            DagProposerTipCandidate {
                hash: [0x02; 32],
                found: false,
                sender: [0; 20],
                level: 0,
                gas_estimation: 0,
            },
            DagProposerTipCandidate {
                hash: [0x03; 32],
                found: true,
                sender: [0xB1; 20],
                level: 2,
                gas_estimation: 100,
            },
            DagProposerTipCandidate {
                hash: [0x04; 32],
                found: true,
                sender: [0xB1; 20],
                level: 3,
                gas_estimation: 100,
            },
            DagProposerTipCandidate {
                hash: [0x05; 32],
                found: true,
                sender: [0xC1; 20],
                level: 1,
                gas_estimation: 100,
            },
        ];

        let selection = dag_proposer_select_tips(candidates, 250, 10);
        assert_eq!(selection.skipped_missing, 1);
        let selected = selection
            .selected
            .into_iter()
            .map(|hash| hash.hash)
            .collect::<Vec<_>>();
        assert_eq!(selected, vec![[0x01; 32], [0x05; 32]]);
    }

    #[test]
    fn dag_verify_vdf_sortition_rejects_invalid_vrf_output_shape() {
        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&vec![0x11u8; 80]);
        vdf_payload.append(&vec![0x22u8]);
        vdf_payload.append(&vec![0x33u8]);
        vdf_payload.append(&1u16);

        let err = match dag_verify_vdf_sortition(DagVerifyVdfSortitionInput {
            block_rlp: dag_block_with_vdf_payload(vdf_payload.out().to_vec()),
            vdf_input: vec![0x01],
            sortition_params: SortitionRuntimeParams {
                threshold_upper: 1000,
                difficulty_min: 1,
                difficulty_max: 1,
                difficulty_stale: 1,
                lambda_bound: 6,
            },
            vrf_output: vec![0u8; 63],
            vrf_public_key: Vec::new(),
            vrf_input: Vec::new(),
            sender_eligible_vote_count: 100,
            vdf_sortition_max_vote_count: 100,
        }) {
            Ok(_) => panic!("invalid VRF output shape should fail"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("VRF output must be 64 bytes"),
            "unexpected error: {err}"
        );
    }
}
