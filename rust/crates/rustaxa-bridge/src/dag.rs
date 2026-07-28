#[cfg(test)]
use crate::ffi::rustaxa_ffi::DagTransactionHash;
use crate::ffi::rustaxa_ffi::{
    DagBlockLookup, DagFrontier, DagHash, DagLevelHashes, DagManagerAnchors,
    DagManagerNonFinalizedSize, DagManagerNonFinalizedSyncPayload, DagOrder,
    DagPersistenceCounters, DagPivotTipsValidation, DagProposerStorageTipSelectionInput,
    DagProposerTipSelectionPlan, DagProposerWorkerCommand, DagProposerWorkerCommandInput,
    DagSyncBlockRlp, DagTransactionRlpLookup, HashLookup,
};
#[cfg(test)]
use crate::ffi::BridgeStorage;
use anyhow::{Context, Result};
use ethereum_types::H256;
use rustaxa_consensus::dag::{
    collect_non_finalized_sync_payload_from_storage, construct_dag_vdf_message,
    dag_block_exists_in_storage, dag_manager_block_from_rlp as domain_dag_manager_block_from_rlp,
    dag_persistence_counters_from_storage, load_dag_block_from_storage,
    period_block_hash_from_storage, plan_dag_proposer_tip_selection_from_storage,
    plan_dag_proposer_worker_command, save_dag_block_to_storage, validate_pivot_tips_metadata,
    DagManagerBlock as DomainDagManagerBlock, DagManagerState,
    DagProposerStorageTipSelectionInput as DomainDagProposerStorageTipSelectionInput,
    DagProposerWorkerCommandInput as DomainDagProposerWorkerCommandInput,
    DagReferenceMetadata as ReferenceMetadata,
};
use rustaxa_consensus::dag_service::{DagServiceGuard, DagServiceState as DagRuntimeState};
use rustaxa_consensus::sortition::{SortitionParams, VdfParams, VrfParams};
use rustaxa_storage::Storage;
#[cfg(test)]
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

/// Temporary FFI adapter over a short-lived native DAG guard or owned test state.
pub(crate) struct DagRuntimeAccess<T>(pub(crate) T);

impl<T> Deref for DagRuntimeAccess<T>
where
    T: Deref<Target = DagRuntimeState>,
{
    type Target = DagRuntimeState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for DagRuntimeAccess<T>
where
    T: DerefMut<Target = DagRuntimeState>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub(crate) type DagRuntimeGuard<'a> = DagRuntimeAccess<DagServiceGuard<'a>>;

/// Module-private deterministic plan for advancing DAG finalization state.
///
/// The carrier is produced from a validated anchor transition and contains the
/// finalized count plus the block sets needed to derive Rust-storage counter
/// updates and cleanup. Hash vectors retain domain ordering and may be empty for
/// an empty period. The plan performs no I/O, never crosses CXX, and is returned
/// only after state-transition validation succeeds.
#[cfg(test)]
struct DagManagerFinalizationPlan {
    finalized_count: u64,
    counter_update_hashes: Vec<DagHash>,
    expired_hashes: Vec<DagHash>,
    remaining_hashes: Vec<DagHash>,
}

/// Private bridge-shaped DAG block used by the retained internal runtime helpers and their tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DagManagerBlock {
    pub hash: [u8; 32],
    pub pivot: [u8; 32],
    pub tips: Vec<DagHash>,
    pub level: u64,
    pub difficulty: u32,
}

pub(crate) const DAG_PROPOSER_SESSION_ACTION_NONE: u8 = 0;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS: u8 = 1;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_START_VDF: u8 = 2;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF: u8 = 3;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_STALE_PROOF_SLEEP: u8 = 4;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK: u8 = 5;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK: u8 = 6;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS: u8 = 7;

struct DagManagerRuntimeSyncSnapshot {
    period: u64,
    selected_hashes: Vec<DagHash>,
}

/// Builds private DAG service state with direct storage access.
///
/// The returned state owns deterministic graph/index data and a cloned Rust
/// storage handle. It remains private to `BridgeDagTransactionService`; callers
/// cannot publish or pass it as a standalone bridge handle. Construction does
/// not restore persisted state, so the service factory must restore both sibling
/// domains before publishing the composed service.
#[cfg(test)]
pub(crate) fn build_dag_state_from_storage(
    genesis: &[u8; 32],
    dag_expiry_limit: u32,
    storage: &BridgeStorage,
) -> Result<Box<DagRuntimeAccess<Box<DagRuntimeState>>>> {
    Ok(Box::new(DagRuntimeAccess(Box::new(DagRuntimeState {
        state: DagManagerState::new(to_h256(genesis), dag_expiry_limit)?,
        storage: storage.0.clone(),
        next_proposer_session_id: 1,
        next_verify_block_session_id: 1,
        next_add_block_session_id: 1,
        proposer_sessions: BTreeMap::new(),
        proposer_retry_states: BTreeMap::new(),
        verify_block_session: None,
        pending_add_block: None,
    }))))
}

impl<T> DagRuntimeAccess<T>
where
    T: DerefMut<Target = DagRuntimeState>,
{
    /// Adds one accepted DAG block to the in-memory Rust state.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn dag_manager_runtime_add_block(&mut self, block: DagManagerBlock) -> Result<()> {
        self.state.add_block(to_domain_block(block))
    }

    /// Validates pivot/tip availability from Rust runtime state and storage.
    ///
    /// Inputs:
    /// - `block_level`: level declared by the candidate DAG block.
    /// - `pivot` and `tips`: candidate references in legacy block order.
    ///
    /// Output:
    /// - compact reference availability and expected-level facts. Missing
    ///   references are returned as data, not errors, so compatibility callers
    ///   can preserve the public `(bool, missing_hashes)` API without
    ///   materializing C++ `DagBlock` objects.
    ///
    /// Edge behavior:
    /// - storage backend or payload decode failures are bridge errors because
    ///   canonical DAG storage is the authoritative source for persisted block
    ///   metadata in Rust mode.
    pub fn dag_manager_runtime_validate_pivot_tips(
        &self,
        block_level: u64,
        pivot: &[u8; 32],
        tips: Vec<DagHash>,
    ) -> Result<DagPivotTipsValidation> {
        let pivot = dag_reference_metadata_from_runtime_or_storage(
            &self.state,
            self.storage.as_ref(),
            to_h256(pivot),
        )?;
        let tips = tips
            .into_iter()
            .map(|tip| {
                dag_reference_metadata_from_runtime_or_storage(
                    &self.state,
                    self.storage.as_ref(),
                    H256::from(tip.hash),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let validation = validate_pivot_tips_metadata(block_level, pivot, &tips);

        Ok(DagPivotTipsValidation {
            ok: validation.ok,
            expected_level: validation.expected_level,
            level_matches: validation.level_matches,
            missing_references: to_dag_hashes(validation.missing_references),
        })
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
    fn dag_manager_runtime_set_finalized_order(
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

    /// Returns a one-shot sync snapshot containing the current period and the
    /// deterministic selection of non-finalized block hashes that are not in
    /// `known_hashes`.
    fn dag_manager_runtime_non_finalized_sync_snapshot(
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
    #[cfg(test)]
    fn dag_manager_runtime_select_non_finalized_hashes(
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

    /// Returns whether the Rust DAG runtime knows a block in live graph state
    /// or canonical Rust storage.
    ///
    /// This is the Rust-mode authority for `DagManager::isDagBlockKnown`.
    /// Compatibility caches may still retain materialized `DagBlock` sidecars
    /// for public/test/event edges, but they do not decide membership.
    pub fn dag_manager_runtime_is_block_known(&self, hash: &[u8; 32]) -> Result<bool> {
        let hash = to_h256(hash);
        Ok(
            self.state.has_vertex(hash)
                || dag_block_exists_in_storage(self.storage.as_ref(), hash)?,
        )
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
    #[cfg_attr(not(test), allow(dead_code))]
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

    /// Selects proposer tips with tip metadata loaded from Rust storage.
    ///
    /// This backs the legacy `DagBlockProposer::selectDagBlockTips` compatibility API. Rust owns storage metadata
    /// loading, sender recovery, missing-tip skipping, proposer grouping, level ordering, gas-limit enforcement, and
    /// max-tip enforcement. C++ only supplies candidate hashes and materializes the returned hash list.
    pub fn dag_manager_runtime_plan_proposal_tip_selection(
        &self,
        input: DagProposerStorageTipSelectionInput,
    ) -> Result<DagProposerTipSelectionPlan> {
        let plan = plan_dag_proposer_tip_selection_from_storage(
            self.storage.as_ref(),
            DomainDagProposerStorageTipSelectionInput {
                frontier_tips: input
                    .frontier_tips
                    .into_iter()
                    .map(|hash| H256::from(hash.hash))
                    .collect(),
                gas_limit: input.gas_limit,
                max_tips: input.max_tips,
            },
        )?;
        Ok(DagProposerTipSelectionPlan {
            selected_tips: plan
                .selected
                .into_iter()
                .map(|hash| DagHash { hash: hash.0 })
                .collect(),
            skipped_missing_tips: plan.skipped_missing,
        })
    }

    /// Ensures the proposal-period mapping exists for `level`.
    ///
    /// Returns true when a mapping write was required and false when the
    /// existing lookup already resolves to `period`.
    #[cfg(test)]
    pub fn dag_manager_runtime_ensure_proposal_period_mapping(
        &self,
        level: u64,
        period: u64,
    ) -> Result<bool> {
        rustaxa_consensus::dag::ensure_proposal_period_mapping(self.storage.as_ref(), level, period)
    }

    /// Resolves the finalized proposal period for a DAG level through the
    /// runtime-owned Rust storage handle.
    ///
    /// Inputs and outputs mirror `DbStorage::getProposalPeriodForDagLevel`:
    /// Rust storage returns the first persisted `(level -> period)` row at or
    /// after the requested level. Missing rows are reported as `found = false`
    /// instead of errors, while malformed storage/backend failures are errors.
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
}

/// Plans one DAG proposer worker-loop command from live executor facts.
///
/// C++ still owns the worker thread, network object, and timer. Rust owns the
/// command choice so scheduling policy does not live in the proposer shell.
pub fn dag_plan_proposer_worker_command(
    input: DagProposerWorkerCommandInput,
) -> DagProposerWorkerCommand {
    let command = plan_dag_proposer_worker_command(DomainDagProposerWorkerCommandInput {
        pbft_syncing: input.pbft_syncing,
        packet_queue_over_limit: input.packet_queue_over_limit,
        has_attempt_result: input.has_attempt_result,
        attempt_returned_proposed: input.attempt_returned_proposed,
    });
    DagProposerWorkerCommand {
        attempt_proposal: command.attempt_proposal,
        sleep_after_tick: command.sleep_after_tick,
        sleep_ms: command.sleep_ms,
        reason_code: command.reason_code,
    }
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

#[cfg_attr(not(test), allow(dead_code))]
pub fn dag_manager_block_from_rlp(block_rlp: Vec<u8>) -> Result<DagManagerBlock> {
    let block = domain_dag_manager_block_from_rlp(&block_rlp)?;
    Ok(DagManagerBlock {
        hash: block.hash.into(),
        pivot: block.pivot.into(),
        tips: to_dag_hashes(block.tips),
        level: block.level,
        difficulty: block.difficulty,
    })
}

fn to_h256(hash: &[u8; 32]) -> H256 {
    H256::from(*hash)
}

pub(crate) fn empty_sortition_params() -> SortitionParams {
    SortitionParams {
        vrf: VrfParams { threshold_upper: 0 },
        vdf: VdfParams {
            difficulty_min: 0,
            difficulty_max: 0,
            difficulty_stale: 0,
            lambda_bound: 0,
        },
    }
}

pub(crate) fn legacy_sortition_params(
    params: SortitionParams,
) -> crate::ffi::rustaxa_ffi::LegacySortitionParams {
    crate::ffi::rustaxa_ffi::LegacySortitionParams {
        vrf_threshold_upper: params.vrf.threshold_upper,
        vdf_difficulty_min: params.vdf.difficulty_min,
        vdf_difficulty_max: params.vdf.difficulty_max,
        vdf_difficulty_stale: params.vdf.difficulty_stale,
        vdf_lambda_bound: params.vdf.lambda_bound,
    }
}

fn to_dag_hashes(hashes: Vec<H256>) -> Vec<DagHash> {
    hashes
        .into_iter()
        .map(|hash| DagHash { hash: hash.0 })
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

#[cfg(test)]
fn to_bridge_finalization_plan(
    plan: rustaxa_consensus::dag::DagManagerFinalizationPlan,
) -> DagManagerFinalizationPlan {
    DagManagerFinalizationPlan {
        finalized_count: plan.finalized_count as u64,
        counter_update_hashes: to_dag_hashes(plan.counter_update_hashes),
        expired_hashes: to_dag_hashes(plan.expired_hashes),
        remaining_hashes: to_dag_hashes(plan.remaining_hashes),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
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

fn dag_reference_metadata_from_runtime_or_storage(
    state: &DagManagerState,
    storage: &Storage,
    hash: H256,
) -> Result<ReferenceMetadata> {
    let metadata = state.reference_metadata(hash);
    if metadata.found {
        return Ok(metadata);
    }

    if storage
        .dag()
        .by_hash_rlp_optional(hash)
        .context("DAG_RUNTIME_REFERENCE_STORAGE_LOOKUP")?
        .is_none()
    {
        return Ok(metadata);
    }

    let block = storage
        .dag()
        .by_hash(hash)
        .context("DAG_RUNTIME_REFERENCE_STORAGE_DECODE")?;
    Ok(ReferenceMetadata {
        hash,
        found: true,
        level: block.level,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{BridgeDagStorageQueries, BridgeStorage};
    use crate::storage::{create_dag_storage_queries, create_storage};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
    use rustaxa_types::pbft::PbftBlockLink;
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

    fn dag_queries(storage: &BridgeStorage) -> Box<BridgeDagStorageQueries> {
        create_dag_storage_queries(storage)
    }

    fn signed_dag_block_rlp(seed: u8, level: u64, gas_estimation: u64) -> Vec<u8> {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("signing key");
        let mut block = rustaxa_types::dag::DagBlock {
            pivot: H256::from([1u8; 32]),
            level,
            timestamp: 123,
            vdf: vec![1, 2, 3],
            tips: vec![],
            transactions: vec![H256::from([9u8; 32])],
            signature: [0; 65],
            gas_estimation,
        };
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(block.signing_hash().as_bytes())
            .expect("sign dag block");
        block.signature[..64].copy_from_slice(&signature.to_bytes());
        block.signature[64] = recovery_id.to_byte();

        let mut stream = RlpStream::new_list(8);
        stream.append(&block.pivot);
        stream.append(&block.level);
        stream.append(&block.timestamp);
        stream.append(&block.vdf);
        stream.append_list(&block.tips);
        stream.append_list(&block.transactions);
        stream.append(&block.signature.as_ref());
        stream.append(&block.gas_estimation);
        stream.out().to_vec()
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
        signed_pbft_block_with_pivot(period, timestamp, H256::from_low_u64_be(11))
    }

    fn signed_pbft_block_with_pivot(period: u64, timestamp: u64, pivot: H256) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from_low_u64_be(10));
        block.append(&pivot);
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

    #[test]
    fn dag_manager_runtime_persists_and_loads_blocks() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_storage");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            let hash = [7u8; 32];
            let block_rlp = vec![0xAA, 0xBB, 0xCC];

            runtime
                .dag_manager_runtime_save_block(&hash, 11, 2, block_rlp.clone())
                .expect("save should succeed");

            assert!(runtime
                .dag_manager_runtime_is_block_known(&hash)
                .expect("known lookup should succeed"));

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
    fn dag_manager_runtime_plans_proposal_tip_selection_from_storage_tips() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposal_tip_selection");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_save_block(
                    &[10u8; 32],
                    3,
                    0,
                    signed_dag_block_rlp(0x61, 3, 100),
                )
                .expect("save lower tip");
            runtime
                .dag_manager_runtime_save_block(
                    &[20u8; 32],
                    5,
                    0,
                    signed_dag_block_rlp(0x62, 5, 100),
                )
                .expect("save higher tip");

            let plan = runtime
                .dag_manager_runtime_plan_proposal_tip_selection(
                    DagProposerStorageTipSelectionInput {
                        frontier_tips: vec![
                            DagHash { hash: [10u8; 32] },
                            DagHash { hash: [20u8; 32] },
                            DagHash { hash: [30u8; 32] },
                        ],
                        gas_limit: 150,
                        max_tips: 16,
                    },
                )
                .expect("plan");

            assert_eq!(plan.selected_tips.len(), 1);
            assert_eq!(plan.selected_tips[0].hash, [20u8; 32]);
            assert_eq!(plan.skipped_missing_tips, 1);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_ensures_proposal_period_for_mismatched_lookup() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposal_mapping");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            assert!(runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(200, 5)
                .expect("initial mapping write should succeed"));

            // Level 100 resolves to period 5 via the later (200 -> 5) mapping.
            let before = dag_queries(&storage)
                .get_proposal_period_for_dag_level(100)
                .expect("lookup should succeed");
            assert!(before.found);
            assert_eq!(before.period, 5);

            // Ensure path must still write because the resolved value mismatches
            // the expected period for this level.
            assert!(runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(100, 0)
                .expect("mismatch correction should succeed"));

            let after = dag_queries(&storage)
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
    fn dag_manager_runtime_period_block_hash_uses_rust_period_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_period_hash");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
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
                .0
                .period()
                .write(7, &period_data_with_pbft_block(&pbft_block))
                .expect("period data save should succeed");

            let found = runtime
                .dag_manager_runtime_period_block_hash(7)
                .expect("period hash lookup should succeed");
            assert!(found.found);
            assert_eq!(found.hash, expected_hash);

            storage
                .0
                .period()
                .write(8, &vec![0x80])
                .expect("corrupt period data save should succeed");
            assert!(runtime.dag_manager_runtime_period_block_hash(8).is_err());
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_proposer_worker_command_plans_attempts_and_backoff() {
        let attempt = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: false,
            packet_queue_over_limit: false,
            has_attempt_result: false,
            attempt_returned_proposed: false,
        });
        assert!(attempt.attempt_proposal);
        assert!(!attempt.sleep_after_tick);

        let throttle = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: true,
            packet_queue_over_limit: false,
            has_attempt_result: false,
            attempt_returned_proposed: false,
        });
        assert!(!throttle.attempt_proposal);
        assert!(throttle.sleep_after_tick);
        assert_eq!(throttle.sleep_ms, 100);

        let no_block = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: false,
            packet_queue_over_limit: false,
            has_attempt_result: true,
            attempt_returned_proposed: false,
        });
        assert!(!no_block.attempt_proposal);
        assert!(no_block.sleep_after_tick);
        assert_eq!(no_block.sleep_ms, 100);

        let proposed = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: false,
            packet_queue_over_limit: false,
            has_attempt_result: true,
            attempt_returned_proposed: true,
        });
        assert!(!proposed.attempt_proposal);
        assert!(!proposed.sleep_after_tick);
    }

    #[test]
    fn dag_manager_runtime_set_finalized_order_updates_graph_state() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_set_finalized_order");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
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
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 2, &storage)
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
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
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
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
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
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
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
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
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
                .0
                .transaction()
                .write(H256::from([1u8; 32]), &[0xA1, 0x01])
                .expect("persist pending transaction 1");
            storage
                .0
                .transaction()
                .write(H256::from([2u8; 32]), &[0xA2, 0x02])
                .expect("persist pending transaction 2");
            storage
                .0
                .transaction()
                .write(H256::from([3u8; 32]), &[0xA3, 0x03])
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
    fn dag_manager_block_from_rlp_bridge_decodes_hash_level_tips_and_difficulty() {
        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&vec![0x11u8; 80]);
        vdf_payload.append(&vec![0x22u8]);
        vdf_payload.append(&vec![0x33u8]);
        vdf_payload.append(&7u16);
        let block_rlp = dag_block_with_level_and_transaction_hashes(
            9,
            vdf_payload.out().to_vec(),
            &[DagTransactionHash { hash: [0x44; 32] }],
        );

        let facts = dag_manager_block_from_rlp(block_rlp).expect("manager facts");

        assert_ne!(facts.hash, [0; 32]);
        assert_eq!(facts.pivot, [0u8; 32]);
        assert_eq!(facts.level, 9);
        assert_eq!(facts.tips.len(), 0);
        assert_eq!(facts.difficulty, 7);
    }
}
