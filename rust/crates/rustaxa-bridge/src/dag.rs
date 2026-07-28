use crate::ffi::rustaxa_ffi::{
    DagBlockLookup, DagFrontier, DagHash, DagLevelHashes, DagManagerAnchors,
    DagManagerNonFinalizedSize, DagManagerNonFinalizedSyncPayload, DagOrder,
    DagPersistenceCounters, DagPivotTipsValidation, DagProposerAddBlockReport,
    DagProposerSessionBeginInput, DagProposerSessionStep, DagProposerSignedBlockIntent,
    DagProposerSigningReport, DagProposerStorageTipSelectionInput, DagProposerTipSelectionPlan,
    DagProposerVdfProofReport, DagProposerWorkerCommand, DagProposerWorkerCommandInput,
    DagSyncBlockRlp, DagTransactionHash, DagTransactionRlpLookup, DagVerifyBlockGasReport,
    DagVerifyBlockSessionInput, DagVerifyBlockSessionStep, HashLookup,
    TransactionPackSelectedTransaction,
};
#[cfg(test)]
use crate::ffi::BridgeStorage;
use anyhow::{ensure, Context, Result};
use ethereum_types::H256;
#[cfg(test)]
use rustaxa_consensus::dag::plan_dag_proposer_post_pack;
use rustaxa_consensus::dag::{
    collect_non_finalized_sync_payload_from_storage, construct_dag_vdf_message,
    dag_block_exists_in_storage, dag_manager_block_from_rlp as domain_dag_manager_block_from_rlp,
    dag_persistence_counters_from_storage, decide_dag_verify_vdf_dpos_authorization,
    finalize_dag_proposer_signed_block_intent, load_dag_block_from_storage,
    period_block_hash_from_storage, plan_dag_proposer_attempt,
    plan_dag_proposer_block_construction_from_storage, plan_dag_proposer_block_intent,
    plan_dag_proposer_retry_reset, plan_dag_proposer_stale_proof,
    plan_dag_proposer_tip_selection_from_storage, plan_dag_proposer_vdf_wait,
    plan_dag_proposer_worker_command, plan_dag_verify_transaction_query, save_dag_block_to_storage,
    validate_pivot_tips_metadata, verify_precheck_from_storage,
    DagManagerBlock as DomainDagManagerBlock, DagManagerState,
    DagProposerAttemptInput as DomainDagProposerAttemptInput,
    DagProposerAttemptPlan as DomainDagProposerAttemptPlan,
    DagProposerBlockIntentInput as DomainDagProposerBlockIntentInput,
    DagProposerFrontierFacts as DomainDagProposerFrontierFacts,
    DagProposerSignedBlockIntentInput as DomainDagProposerSignedBlockIntentInput,
    DagProposerStorageBlockConstructionInput as DomainDagProposerStorageBlockConstructionInput,
    DagProposerStorageTipSelectionInput as DomainDagProposerStorageTipSelectionInput,
    DagProposerUnsignedBlockIntent as DomainDagProposerUnsignedBlockIntent,
    DagProposerWorkerCommandInput as DomainDagProposerWorkerCommandInput,
    DagReferenceMetadata as ReferenceMetadata, DagTipGas,
    DagVerifyPrecheckStorageInput as DomainDagVerifyPrecheckStorageInput,
    DagVerifyVdfDposFacts as DomainDagVerifyVdfDposFacts,
};
use rustaxa_consensus::dag_service::{
    DagProposerObservation, DagProposerRetryState, DagProposerSession, DagProposerSessionAction,
    DagProposerSessionBeginInput as DomainDagProposerSessionBeginInput,
    DagProposerTransactionObservation, DagServiceGuard, DagServiceState as DagRuntimeState,
    DagVerifyBlockSession, DagVerifyBlockSessionAction,
};
use rustaxa_consensus::sortition::{SortitionParams, VdfParams, VrfParams};
#[cfg(test)]
use rustaxa_consensus::transaction_packing_service::TransactionPackingSelection;
use rustaxa_storage::Storage;
#[cfg(test)]
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::time::{SystemTime, UNIX_EPOCH};
use tiny_keccak::{Hasher, Keccak};

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

const DAG_VERIFY_SESSION_STATUS_ACTIVE: u8 = 0;
const DAG_VERIFY_SESSION_STATUS_COMPLETE: u8 = 1;
const DAG_VERIFY_SESSION_STATUS_INVALID_REPORT: u8 = 2;
const DAG_VERIFY_SESSION_ACTION_NONE: u8 = 0;
pub(crate) const DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY: u8 = 1;
pub(crate) const DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS: u8 = 2;
pub(crate) const DAG_VERIFY_SESSION_ACTION_VDF_SORTITION: u8 = 3;
pub(crate) const DAG_VERIFY_SESSION_ACTION_GAS: u8 = 4;

/// Private bridge-shaped DAG block used by the retained internal runtime helpers and their tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DagManagerBlock {
    pub hash: [u8; 32],
    pub pivot: [u8; 32],
    pub tips: Vec<DagHash>,
    pub level: u64,
    pub difficulty: u32,
}

const DAG_PROPOSER_SESSION_STATUS_ACTIVE: u8 = 0;
const DAG_PROPOSER_SESSION_STATUS_COMPLETE: u8 = 1;
const DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT: u8 = 2;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_NONE: u8 = 0;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS: u8 = 1;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_START_VDF: u8 = 2;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF: u8 = 3;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_STALE_PROOF_SLEEP: u8 = 4;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK: u8 = 5;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK: u8 = 6;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS: u8 = 7;

/// Private identity and verifier inputs captured from one active VDF action.
///
/// No runtime guard survives the snapshot. Cursor identity, immutable candidate
/// fingerprint, action generation, and normalized counts must match again
/// before the verifier result may advance the session.
pub(crate) type DagVerifyBlockVdfSnapshot =
    rustaxa_consensus::dag_service::DagVerifyBlockVdfSnapshot;

/// Exact proposer identity retained across historical sortition lookups.
///
/// The keyed cursor, requested action, observation fingerprint, and proposal
/// period must all match before the second lookup can plan or advance.
pub(crate) struct DagProposerFinalChainFactsSnapshot {
    pub session_id: u64,
    pub fingerprint: [u8; 32],
    pub proposal_period: u64,
    pub proposal_period_found: bool,
    pub proposer_address: [u8; 20],
}

/// Preparation result preserving existing missing/wrong-action step semantics.
pub(crate) enum DagProposerFinalChainFactsPreparation {
    Snapshot(DagProposerFinalChainFactsSnapshot),
    Step(Box<DagProposerSessionStep>),
}

/// Private pack configuration derived from one live DAG proposer cursor.
#[cfg(test)]
pub(crate) struct DagProposerPackParameters {
    pub proposal_period: u64,
    pub weight_limit: u64,
    pub total_transaction_shards: u16,
    pub node_transaction_shard: u16,
    pub shard_period_interval: u64,
}

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

    /// Loads retained per-tip gas facts directly from Rust storage for DAG block verification.
    ///
    /// Inputs:
    /// - `tips`: candidate tip hashes retained in canonical block order.
    ///
    /// Outputs:
    /// - one domain `DagTipGas` per input hash. Missing tips are returned as
    ///   `found = false` so the Rust verification session can select the
    ///   legacy `MissingTip` status without C++ materializing `DagBlock`
    ///   objects or deriving gas facts from compatibility caches.
    ///
    /// Edge behavior:
    /// - storage backend and decode failures are bridge errors because they
    ///   indicate corrupt or unavailable canonical DAG payloads rather than a
    ///   consensus-invalid missing tip.
    fn dag_manager_runtime_tip_gas_estimations(&self, tips: &[H256]) -> Result<Vec<DagTipGas>> {
        tips.iter()
            .map(|hash| {
                if self
                    .storage
                    .dag()
                    .by_hash_rlp_optional(*hash)
                    .context("DAG_RUNTIME_TIP_GAS_LOOKUP")?
                    .is_none()
                {
                    return Ok(DagTipGas {
                        found: false,
                        gas_estimation: 0,
                    });
                }

                let block = self
                    .storage
                    .dag()
                    .by_hash(*hash)
                    .context("DAG_RUNTIME_TIP_GAS_DECODE")?;
                Ok(DagTipGas {
                    found: true,
                    gas_estimation: block.gas_estimation,
                })
            })
            .collect()
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

    /// Opens a runtime-owned ordered `DagManager::verifyBlock` session.
    ///
    /// The runtime performs storage-backed prechecks immediately, then returns
    /// either a terminal reject/complete step or a transaction-query request.
    /// Later advancement happens only through explicit live-fact reports from
    /// the C++ executor boundary.
    pub fn begin_verify_block_session(&mut self, input: DagVerifyBlockSessionInput) -> Result<()> {
        let fingerprint = input.block_hash;
        let cursor_id = self.next_verify_block_session_id;
        self.next_verify_block_session_id =
            self.next_verify_block_session_id.wrapping_add(1).max(1);
        let tips = input
            .tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect::<Vec<_>>();
        let precheck = verify_precheck_from_storage(
            self.storage.as_ref(),
            DomainDagVerifyPrecheckStorageInput {
                block_level: input.block_level,
                pivot: H256::from(input.pivot),
                tips: tips.clone(),
                dag_expiry_level: self.state.dag_expiry_level(),
            },
        )
        .context("DAG_RUNTIME_VERIFY_SESSION_PRECHECK")?;

        let expected_transactions = input.block_transaction_hashes.len() as u64;
        let action = if precheck.continue_validation {
            let block_transaction_hashes = to_transaction_hashes(input.block_transaction_hashes);
            let supplied_transaction_hashes =
                to_transaction_hashes(input.supplied_transaction_hashes);
            let query_plan = plan_dag_verify_transaction_query(
                &block_transaction_hashes,
                &supplied_transaction_hashes,
            );
            DagVerifyBlockSessionAction::TransactionQuery(query_plan.query_hashes)
        } else {
            DagVerifyBlockSessionAction::Complete
        };

        self.verify_block_session = Some(DagVerifyBlockSession {
            cursor_id,
            fingerprint,
            generation: 1,
            action,
            tips,
            proposal_period: precheck.proposal_period,
            block_rlp: input.block_rlp,
            expected_transactions,
            reject_code: precheck.reject_code,
            sender_eligible_vote_count: 0,
            vdf_sortition_max_vote_count: 0,
            eligibility_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
            error_code: String::new(),
        });

        Ok(())
    }
}

/// Opens a DAG block verification cursor inside the long-lived DAG manager runtime.
///
/// Inputs:
/// - `runtime`: the DAG manager runtime that owns graph state, storage, and the temporary verification cursor.
/// - `input`: compact block facts and supplied transaction hashes for one `DagManager::verifyBlock` call.
///
/// Outputs:
/// - Replaces any previous runtime verification cursor and assigns a new cursor
///   identity. C++ drives it with the composed transaction prepare/completion
///   boundary, `dag_manager_runtime_verify_block_session_next`, and later reports.
///
/// Invariants and edge behavior:
/// - The verification cursor is DAG-manager implementation state and is not exported as a standalone CXX handle.
/// - Starting a new verification replaces any incomplete previous cursor, matching the legacy per-call allocation
///   behavior.
pub fn dag_manager_runtime_begin_verify_block_session(
    runtime: &mut DagRuntimeState,
    input: DagVerifyBlockSessionInput,
) -> Result<()> {
    DagRuntimeAccess(runtime).begin_verify_block_session(input)
}

/// Returns the next requested action for the runtime-owned DAG verification cursor.
pub fn dag_manager_runtime_verify_block_session_next(
    runtime: &mut DagRuntimeState,
) -> DagVerifyBlockSessionStep {
    native_verify_block_step_to_bridge(DagRuntimeAccess(runtime).verify_block_session_next())
}

/// Applies resolved transaction availability to the runtime-owned DAG verification cursor.
pub(crate) fn dag_manager_runtime_verify_block_session_apply_transaction_resolution(
    runtime: &mut DagRuntimeState,
    resolved_transactions: u64,
) -> DagVerifyBlockSessionStep {
    native_verify_block_step_to_bridge(
        DagRuntimeAccess(runtime)
            .verify_block_session_apply_transaction_resolution(resolved_transactions),
    )
}

/// Private transaction query owned by an active DAG verification session.
///
/// The composed DAG/transaction service consumes this value while holding both
/// runtime locks; hashes are never exposed through CXX.
pub(crate) type DagVerifyBlockTransactionQuery =
    rustaxa_consensus::dag_service::DagVerifyBlockTransactionQuery;

/// Takes a snapshot of the active transaction query without advancing it.
///
/// Missing sessions and calls during another action return stable errors without
/// advancing or invalidating the current session.
pub(crate) fn dag_manager_runtime_verify_block_transaction_query(
    runtime: &DagRuntimeState,
) -> Result<DagVerifyBlockTransactionQuery> {
    DagRuntimeAccess(runtime).verify_block_transaction_query()
}

/// Verifies that a completion targets the still-active prepared query.
///
/// Stale cursor or proposal-period identities and wrong actions return errors
/// without changing the active verification session.
pub(crate) fn dag_manager_runtime_validate_verify_block_transaction_completion(
    runtime: &DagRuntimeState,
    cursor_id: u64,
    proposal_period: u64,
) -> Result<DagVerifyBlockTransactionQuery> {
    DagRuntimeAccess(runtime)
        .verify_block_session_validate_transaction_completion(cursor_id, proposal_period)
}

/// Exact private identity of a DAG verification cursor awaiting FinalChain authorization.
pub(crate) type DagVerifyBlockAuthorizationSnapshot =
    rustaxa_consensus::dag_service::DagVerifyBlockAuthorizationSnapshot;

/// Preparation result preserving the stable missing-session and wrong-action step semantics.
pub(crate) type DagVerifyBlockAuthorizationPreparation =
    rustaxa_consensus::dag_service::DagVerifyBlockAuthorizationPreparation;

/// Snapshots the exact authorization cursor before the lock-free FinalChain query.
pub(crate) fn dag_manager_runtime_prepare_verify_block_authorization(
    runtime: &mut DagRuntimeState,
) -> DagVerifyBlockAuthorizationPreparation {
    DagRuntimeAccess(runtime).prepare_verify_block_authorization()
}

fn verify_block_authorization_snapshot_matches(
    session: &DagVerifyBlockSession,
    snapshot: &DagVerifyBlockAuthorizationSnapshot,
) -> bool {
    session.cursor_id == snapshot.cursor_id
        && session.fingerprint == snapshot.fingerprint
        && session.generation == snapshot.generation
        && session.proposal_period == snapshot.proposal_period
        && session.block_rlp == snapshot.block_rlp
        && matches!(
            session.action,
            DagVerifyBlockSessionAction::AuthorizationFacts
        )
}

/// Removes only the unchanged verification cursor that owned a failed FinalChain query.
pub(crate) fn dag_manager_runtime_cleanup_verify_block_authorization(
    runtime: &mut DagRuntimeState,
    snapshot: &DagVerifyBlockAuthorizationSnapshot,
) -> bool {
    DagRuntimeAccess(runtime).cleanup_verify_block_authorization(snapshot)
}

/// Revalidates and applies Rust FinalChain authorization facts to the exact cursor.
pub(crate) fn dag_manager_runtime_apply_verify_block_authorization(
    runtime: &mut DagRuntimeState,
    snapshot: &DagVerifyBlockAuthorizationSnapshot,
    facts: rustaxa_consensus::dag::DagDposAuthorizationFacts,
) -> Result<DagVerifyBlockSessionStep> {
    DagRuntimeAccess(runtime)
        .apply_verify_block_authorization(snapshot, facts)
        .map(native_verify_block_step_to_bridge)
}

/// Snapshots the active VDF action before historical lookup and proof work.
///
/// Missing sessions and wrong actions are operational errors and leave the
/// cursor unchanged. The returned identity must be revalidated after the
/// lock-free interval.
pub(crate) fn dag_manager_runtime_snapshot_verify_block_vdf(
    runtime: &DagRuntimeState,
    cursor_id: u64,
) -> Result<DagVerifyBlockVdfSnapshot> {
    DagRuntimeAccess(runtime).snapshot_verify_block_vdf(cursor_id)
}

/// Revalidates an unlocked VDF snapshot and advances it exactly once.
///
/// A replacement cursor, changed candidate, advanced action, or changed
/// generation returns an error without mutating the live session.
pub(crate) fn dag_manager_runtime_complete_verify_block_vdf(
    runtime: &mut DagRuntimeState,
    snapshot: &DagVerifyBlockVdfSnapshot,
    vdf_status: u8,
) -> Result<DagVerifyBlockSessionStep> {
    DagRuntimeAccess(runtime)
        .complete_verify_block_vdf(snapshot, vdf_status)
        .map(native_verify_block_step_to_bridge)
}

/// Applies a verified status to the currently validated VDF cursor.
fn apply_verify_block_vdf_status(
    runtime: &mut DagRuntimeState,
    vdf_status: u8,
) -> DagVerifyBlockSessionStep {
    let Some(session) = runtime.verify_block_session.as_mut() else {
        return verify_block_session_not_started_step();
    };
    if !matches!(
        session.action,
        DagVerifyBlockSessionAction::VdfSortition { .. }
    ) {
        return invalid_verify_block_report(session, "DAG_VERIFY_SESSION_UNEXPECTED_VDF_REPORT");
    }

    let dpos_status = if session.eligibility_status
        == rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
    {
        rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
    } else {
        rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_NOT_CHECKED
    };
    let vdf_decision = decide_dag_verify_vdf_dpos_authorization(DomainDagVerifyVdfDposFacts {
        vrf_key_found: true,
        sender_eligible_vote_count: session.sender_eligible_vote_count,
        vdf_sortition_max_vote_count: session.vdf_sortition_max_vote_count,
        vdf_status,
        dpos_status,
    });
    if !vdf_decision.continue_validation {
        return complete_verify_block_session(session, vdf_decision.reject_code);
    }

    let dpos_decision = decide_dag_verify_vdf_dpos_authorization(DomainDagVerifyVdfDposFacts {
        vrf_key_found: true,
        sender_eligible_vote_count: session.sender_eligible_vote_count,
        vdf_sortition_max_vote_count: session.vdf_sortition_max_vote_count,
        vdf_status: rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_VALID,
        dpos_status: session.eligibility_status,
    });
    if !dpos_decision.continue_validation {
        return complete_verify_block_session(session, dpos_decision.reject_code);
    }

    session.action = DagVerifyBlockSessionAction::Gas;
    session.generation = session.generation.wrapping_add(1).max(1);
    verify_block_session_step(session)
}

/// Reports external block/limit gas facts to the runtime-owned DAG verification cursor.
///
/// The active cursor owns canonical tip order. Rust reads tip gas from its
/// private storage only when the legacy block-count condition requires the
/// aggregate check. Missing tips remain typed consensus rejection; storage or
/// decode failures return without advancing the cursor. Calls with no session
/// or the wrong action preserve the existing status-coded report semantics and
/// never perform a tip lookup.
pub fn dag_manager_runtime_verify_block_session_report_gas(
    runtime: &mut DagRuntimeState,
    report: DagVerifyBlockGasReport,
) -> Result<DagVerifyBlockSessionStep> {
    DagRuntimeAccess(runtime)
        .verify_block_session_report_gas(rustaxa_consensus::dag_service::DagVerifyBlockGasReport {
            block_gas_estimation: report.block_gas_estimation,
            estimated_transactions_weight: report.estimated_transactions_weight,
            dag_gas_limit: report.dag_gas_limit,
            pbft_gas_limit: report.pbft_gas_limit,
        })
        .map(native_verify_block_step_to_bridge)
}

fn native_verify_block_step_to_bridge(
    step: rustaxa_consensus::dag_service::DagVerifyBlockSessionStep,
) -> DagVerifyBlockSessionStep {
    DagVerifyBlockSessionStep {
        cursor_id: step.cursor_id,
        status: step.status,
        action: step.action,
        complete: step.complete,
        reject_code: step.reject_code,
        proposal_period: step.proposal_period,
        vote_count: step.vote_count,
        max_vote_count: step.max_vote_count,
        error_code: step.error_code,
    }
}

/// Opens a DAG proposal cursor inside the long-lived DAG manager runtime.
///
/// Inputs:
/// - `runtime`: the DAG manager runtime that owns graph state, storage, and proposal cursors.
/// - `input`: wallet/configuration facts for one attempt.
/// - `transaction_observation`: queue and non-finalized sidecar counts captured by the composed sibling service.
///   Frontier, proposal-period, and transaction-pressure facts are never accepted from C++.
///
/// Outputs:
/// - Returns the runtime-local cursor id that C++ must pass to `dag_manager_runtime_proposer_session_next` and report
///   functions.
///
/// Invariants and edge behavior:
/// - Proposal cursors are DAG-manager implementation state and are not exported as standalone CXX handles.
/// - Multiple wallets may hold active proposal cursors concurrently; each cursor advances only through its returned id.
/// - A changed runtime observation terminates before planner or retry-state mutation.
/// - Terminal cursors are removed after their terminal step is observed.
pub(crate) fn dag_manager_runtime_begin_proposer_session(
    runtime: &mut DagRuntimeState,
    input: DagProposerSessionBeginInput,
    transaction_observation: DagProposerTransactionObservation,
) -> Result<u64> {
    runtime.begin_proposer_session(
        to_domain_proposer_session_begin_input(input),
        transaction_observation,
    )
}

/// Removes a runtime-owned DAG proposal cursor without applying retry-state effects.
///
/// Inputs: `runtime` owns the cursor registry and `session_id` identifies the cursor to remove.
/// Output: `true` only when a live cursor was removed. Missing, already-terminal, and previously aborted ids return
/// `false`, making cleanup idempotent and safe during exception unwinding.
/// Invariants and edge behavior: abort never creates or updates retry state, never runs a planner, and never reports an
/// error.
pub fn dag_manager_runtime_abort_proposer_session(
    runtime: &mut DagRuntimeState,
    session_id: u64,
) -> bool {
    runtime.proposer_sessions.remove(&session_id).is_some()
}

/// Returns the current requested action for a runtime-owned DAG proposal cursor.
///
/// Inputs: `runtime` owns the cursor and `session_id` selects it. Output is a complete executor instruction snapshot;
/// active calls do not advance the cursor. A terminal step removes the cursor, and a missing id returns an
/// `INVALID_REPORT` step with no retry effects.
pub fn dag_manager_runtime_proposer_session_next(
    runtime: &mut DagRuntimeState,
    session_id: u64,
) -> DagProposerSessionStep {
    let Some(session) = runtime.proposer_sessions.get(&session_id) else {
        return dag_proposer_session_not_started_step();
    };
    let step = dag_proposer_session_step(session);
    finish_dag_proposer_session_step(runtime, session_id, step)
}

/// Validates and snapshots a proposer cursor before historical parameter lookup.
///
/// Missing cursors retain the existing not-started step behavior. Wrong-action
/// reports remain invalid terminal reports. A successful snapshot does not
/// mutate the keyed cursor.
pub(crate) fn dag_manager_runtime_prepare_proposer_final_chain_facts(
    runtime: &mut DagRuntimeState,
    session_id: u64,
) -> DagProposerFinalChainFactsPreparation {
    let Some(session) = runtime.proposer_sessions.get(&session_id) else {
        return DagProposerFinalChainFactsPreparation::Step(Box::new(
            dag_proposer_session_not_started_step(),
        ));
    };
    if !matches!(
        session.action,
        DagProposerSessionAction::CollectFinalChainFacts
    ) {
        let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
        let step = invalid_dag_proposer_report(
            session,
            "DAG_PROPOSER_SESSION_UNEXPECTED_FINAL_CHAIN_FACTS_REPORT",
        );
        return DagProposerFinalChainFactsPreparation::Step(Box::new(
            finish_dag_proposer_session_step(runtime, session_id, step),
        ));
    }
    DagProposerFinalChainFactsPreparation::Snapshot(DagProposerFinalChainFactsSnapshot {
        session_id,
        fingerprint: session.observation.fingerprint,
        proposal_period: session.observation.proposal_period,
        proposal_period_found: session.observation.proposal_period_found,
        proposer_address: session.begin_input.proposer_address,
    })
}

fn proposer_final_chain_snapshot_matches(
    session: &DagProposerSession,
    snapshot: &DagProposerFinalChainFactsSnapshot,
) -> bool {
    matches!(
        session.action,
        DagProposerSessionAction::CollectFinalChainFacts
    ) && session.observation.fingerprint == snapshot.fingerprint
        && session.observation.proposal_period == snapshot.proposal_period
        && session.observation.proposal_period_found == snapshot.proposal_period_found
        && session.begin_input.proposer_address == snapshot.proposer_address
}

/// Removes only the exact proposer cursor that owned a failed composed lookup.
pub(crate) fn dag_manager_runtime_cleanup_proposer_final_chain_facts(
    runtime: &mut DagRuntimeState,
    snapshot: &DagProposerFinalChainFactsSnapshot,
) -> bool {
    if runtime
        .proposer_sessions
        .get(&snapshot.session_id)
        .is_some_and(|session| proposer_final_chain_snapshot_matches(session, snapshot))
    {
        runtime.proposer_sessions.remove(&snapshot.session_id);
        return true;
    }
    false
}

/// Revalidates and applies FinalChain facts with exact historical parameters.
///
/// The caller holds DAG then sortition locks and has repeated the indexed
/// historical lookup. Cursor/action/fingerprint/period mismatches and changed
/// parameter values return stable errors without advancing any cursor.
pub(crate) fn dag_manager_runtime_apply_proposer_final_chain_facts(
    runtime: &mut DagRuntimeState,
    snapshot: &DagProposerFinalChainFactsSnapshot,
    last_finalized_period: u64,
    authorization_facts: rustaxa_consensus::dag::DagDposAuthorizationFacts,
    sortition_params: SortitionParams,
    initially_loaded_params: SortitionParams,
) -> Result<DagProposerSessionStep> {
    let Some(session) = runtime.proposer_sessions.get(&snapshot.session_id) else {
        anyhow::bail!("DAG_PROPOSER_SESSION_STALE_CURSOR");
    };
    ensure!(
        matches!(
            session.action,
            DagProposerSessionAction::CollectFinalChainFacts
        ),
        "DAG_PROPOSER_SESSION_STALE_ACTION"
    );
    ensure!(
        session.observation.fingerprint == snapshot.fingerprint,
        "DAG_PROPOSER_SESSION_STALE_FINGERPRINT"
    );
    ensure!(
        session.observation.proposal_period == snapshot.proposal_period,
        "DAG_PROPOSER_SESSION_STALE_PROPOSAL_PERIOD"
    );
    ensure!(
        session.observation.proposal_period_found == snapshot.proposal_period_found,
        "DAG_PROPOSER_SESSION_STALE_PROPOSAL_PERIOD_FOUND"
    );
    ensure!(
        session.begin_input.proposer_address == snapshot.proposer_address,
        "DAG_PROPOSER_SESSION_STALE_PROPOSER_ADDRESS"
    );
    ensure!(
        sortition_params == initially_loaded_params,
        "DAG_PROPOSER_SESSION_SORTITION_PARAMS_STALE_RETRY"
    );

    let current = match DagRuntimeAccess(&mut *runtime).proposer_observation() {
        Ok(current) => current,
        Err(error) => {
            runtime.proposer_sessions.remove(&snapshot.session_id);
            return Err(error);
        }
    };
    if current.fingerprint != snapshot.fingerprint {
        let session = runtime
            .proposer_sessions
            .get_mut(&snapshot.session_id)
            .unwrap();
        session.action = DagProposerSessionAction::Complete;
        session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
        session.reason_code = rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION;
        session.error_code = "DAG_PROPOSER_SESSION_STALE_OBSERVATION".to_owned();
        let step = dag_proposer_session_step(session);
        return Ok(finish_dag_proposer_session_step(
            runtime,
            snapshot.session_id,
            step,
        ));
    }

    let session = &runtime.proposer_sessions[&snapshot.session_id];
    let retry = runtime.proposer_retry_states.get(&session.retry_key);
    let last_propose_level = retry.map_or(0, |state| state.last_propose_level);
    let retry_count = retry.map_or(0, |state| state.retry_count);
    let input = domain_attempt_input(
        &session.begin_input,
        session.transaction_observation,
        &session.observation,
        last_finalized_period,
        authorization_facts,
        sortition_params,
        last_propose_level,
        retry_count,
    );
    let minimum_vdf_difficulty = input.sortition_params.vdf.difficulty_min;
    let attempt = match plan_dag_proposer_attempt(input) {
        Ok(attempt) => attempt,
        Err(error) => {
            runtime.proposer_sessions.remove(&snapshot.session_id);
            return Err(error);
        }
    };
    let action = if attempt.action == rustaxa_consensus::dag::DAG_PROPOSER_ACTION_CONTINUE {
        DagProposerSessionAction::PackTransactions
    } else {
        DagProposerSessionAction::Complete
    };
    let session = runtime
        .proposer_sessions
        .get_mut(&snapshot.session_id)
        .unwrap();
    runtime
        .proposer_retry_states
        .entry(session.retry_key)
        .or_insert(DagProposerRetryState {
            last_propose_level,
            retry_count,
            max_retry_count: session.begin_input.max_retry_count,
        })
        .max_retry_count = session.begin_input.max_retry_count;
    session.status = if matches!(action, DagProposerSessionAction::Complete) {
        DAG_PROPOSER_SESSION_STATUS_COMPLETE
    } else {
        DAG_PROPOSER_SESSION_STATUS_ACTIVE
    };
    session.action = action;
    session.reason_code = attempt.reason_code;
    session.update_retry_state = attempt.update_retry_state;
    session.next_last_propose_level = attempt.next_last_propose_level;
    session.next_retry_count = attempt.next_retry_count;
    session.minimum_vdf_difficulty = minimum_vdf_difficulty;
    session.sortition_params = sortition_params;
    session.attempt = attempt;
    let step = dag_proposer_session_step(session);
    Ok(finish_dag_proposer_session_step(
        runtime,
        snapshot.session_id,
        step,
    ))
}

/// Returns private transaction-pack parameters for a live proposer cursor.
#[cfg(test)]
pub(crate) fn dag_manager_runtime_proposer_pack_parameters(
    runtime: &DagRuntimeState,
    session_id: u64,
) -> Result<DagProposerPackParameters> {
    let session = runtime
        .proposer_sessions
        .get(&session_id)
        .context("DAG_PROPOSER_PACK_SESSION_NOT_ACTIVE")?;
    ensure!(
        matches!(session.action, DagProposerSessionAction::PackTransactions),
        "DAG_PROPOSER_PACK_SESSION_WRONG_STAGE"
    );
    Ok(DagProposerPackParameters {
        proposal_period: session.attempt.proposal_period,
        weight_limit: session.attempt.proposal_weight_limit,
        total_transaction_shards: session.attempt.total_transaction_shards,
        node_transaction_shard: session.attempt.node_transaction_shard,
        shard_period_interval: session.attempt.shard_period_interval,
    })
}

/// Applies Rust transaction-pack output directly to its owning DAG cursor.
#[cfg(test)]
pub(crate) fn dag_manager_runtime_apply_proposer_pack(
    runtime: &mut DagRuntimeState,
    session_id: u64,
    network_throttled: bool,
    selected_transactions: Vec<TransactionPackSelectedTransaction>,
) -> Result<DagProposerSessionStep> {
    let session = runtime
        .proposer_sessions
        .get_mut(&session_id)
        .context("DAG_PROPOSER_PACK_SESSION_NOT_ACTIVE")?;
    ensure!(
        matches!(session.action, DagProposerSessionAction::PackTransactions),
        "DAG_PROPOSER_PACK_SESSION_WRONG_STAGE"
    );
    let post_pack = plan_dag_proposer_post_pack(rustaxa_consensus::dag::DagProposerPostPackInput {
        proposal_level: session.attempt.proposal_level,
        network_throttled,
        packed_transaction_count: selected_transactions.len() as u64,
    });
    session.reason_code = post_pack.reason_code;
    session.update_retry_state = post_pack.update_retry_state;
    session.next_last_propose_level = post_pack.next_last_propose_level;
    session.next_retry_count = post_pack.next_retry_count;

    if post_pack.action != rustaxa_consensus::dag::DAG_PROPOSER_ACTION_CONTINUE {
        session.action = DagProposerSessionAction::Complete;
        session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
        session.return_value = false;
    } else {
        session.selected_transaction_hashes = selected_transactions
            .iter()
            .map(|selected| H256::from(selected.hash))
            .collect();
        session.transaction_gas_estimations = selected_transactions
            .iter()
            .map(|selected| selected.gas_used)
            .collect();
        session.vdf_message = construct_dag_vdf_message(
            session.attempt.frontier.pivot,
            &session.selected_transaction_hashes,
        );
        session.selected_transactions = selected_transactions
            .into_iter()
            .map(|selected| TransactionPackingSelection {
                hash: H256::from(selected.hash),
                gas_used: selected.gas_used,
                transaction_rlp: selected.tx_rlp,
            })
            .collect();
        session.action = DagProposerSessionAction::StartVdf;
    }
    let step = dag_proposer_session_step(session);
    Ok(finish_dag_proposer_session_step(runtime, session_id, step))
}

/// Polls whether the runtime-owned proposal cursor should cancel its in-flight VDF.
///
/// The current proposal level is derived from the Rust DAG frontier. A matching active VDF remains active; sufficient
/// frontier advancement returns a terminal cancel step with retry-reset facts. Missing or out-of-order ids return an
/// invalid-report step.
pub fn dag_manager_runtime_proposer_session_poll_vdf(
    runtime: &mut DagRuntimeState,
    session_id: u64,
) -> DagProposerSessionStep {
    let latest_proposal_level = runtime.state.proposer_frontier_facts().propose_level;
    let step = {
        let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
            return dag_proposer_session_not_started_step();
        };
        if !matches!(session.action, DagProposerSessionAction::StartVdf) {
            invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_UNEXPECTED_VDF_WAIT_REPORT")
        } else {
            let wait =
                plan_dag_proposer_vdf_wait(rustaxa_consensus::dag::DagProposerVdfWaitInput {
                    proposal_level: session.attempt.proposal_level,
                    latest_proposal_level,
                    vdf_difficulty: session.attempt.vdf_difficulty,
                    minimum_vdf_difficulty: session.minimum_vdf_difficulty,
                });
            if !wait.cancel_in_flight_proof {
                dag_proposer_session_step(session)
            } else {
                let retry = plan_dag_proposer_retry_reset(
                    rustaxa_consensus::dag::DagProposerRetryResetInput {
                        proposal_level: session.attempt.proposal_level,
                    },
                );
                let mut step = dag_proposer_session_step(session);
                step.action = DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF;
                step.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
                step.return_value = true;
                step.update_retry_state = retry.update_retry_state;
                step.next_last_propose_level = retry.next_last_propose_level;
                step.next_retry_count = retry.next_retry_count;
                step
            }
        }
    };
    finish_dag_proposer_session_step(runtime, session_id, step)
}

fn revalidate_proposer_session_observation(
    runtime: &mut DagRuntimeState,
    session_id: u64,
) -> Result<Option<DagProposerSessionStep>> {
    let current = match DagRuntimeAccess(&mut *runtime).proposer_observation() {
        Ok(current) => current,
        Err(error) => {
            runtime.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    if current.fingerprint
        == runtime.proposer_sessions[&session_id]
            .observation
            .fingerprint
    {
        return Ok(None);
    }
    let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
    session.action = DagProposerSessionAction::Complete;
    session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
    session.reason_code = rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION;
    session.return_value = false;
    session.update_retry_state = false;
    session.error_code = "DAG_PROPOSER_SESSION_STALE_OBSERVATION".to_owned();
    let step = dag_proposer_session_step(session);
    Ok(Some(finish_dag_proposer_session_step(
        runtime, session_id, step,
    )))
}

fn prepare_proposer_session_signing(
    runtime: &mut DagRuntimeState,
    session_id: u64,
    vdf_rlp: Vec<u8>,
) -> Result<DagProposerSessionStep> {
    let session = &runtime.proposer_sessions[&session_id];
    let frontier_tips = session.observation.frontier.frontier.tips.clone();
    let transaction_gas_estimations = session.transaction_gas_estimations.clone();
    let pbft_gas_limit = session.begin_input.pbft_gas_limit;
    let dag_gas_limit = session.begin_input.dag_gas_limit;
    let max_tips = session.begin_input.max_tips;
    let pivot = session.observation.frontier.frontier.pivot;
    let proposal_level = session.attempt.proposal_level;
    let transaction_hashes = session.selected_transaction_hashes.clone();

    let prepared = (|| -> Result<DomainDagProposerUnsignedBlockIntent> {
        let construction = plan_dag_proposer_block_construction_from_storage(
            runtime.storage.as_ref(),
            DomainDagProposerStorageBlockConstructionInput {
                frontier_tips,
                transaction_gas_estimations,
                pbft_gas_limit,
                dag_gas_limit,
                max_tips,
            },
        )?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("DAG_PROPOSER_CURRENT_TIMESTAMP")?
            .as_secs();
        Ok(plan_dag_proposer_block_intent(
            DomainDagProposerBlockIntentInput {
                pivot,
                level: proposal_level,
                timestamp,
                vdf_rlp,
                selected_tips: construction.selected_tips,
                transaction_hashes,
                block_gas_estimation: construction.block_gas_estimation,
            },
        ))
    })();
    let intent = match prepared {
        Ok(intent) => intent,
        Err(error) => {
            runtime.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
    session.vdf_rlp = intent.vdf_rlp.clone();
    session.unsigned_intent = Some(intent);
    session.action = DagProposerSessionAction::SignBlock;
    Ok(dag_proposer_session_step(session))
}

/// Consumes VDF proof completion and constructs the session-owned unsigned block intent.
///
/// Inputs are only proof success and canonical VDF RLP; all other block fields come from the cursor. Rust revalidates
/// the complete observation, performs storage-backed tip construction, chooses the timestamp, and returns signing action
/// 5 with the canonical signing hash. Stale observations terminate without retry mutation. Storage, timestamp, and
/// planning errors remove the cursor before returning `Err`; missing/out-of-order ids return invalid terminal steps.
pub fn dag_manager_runtime_proposer_session_report_vdf_proof(
    runtime: &mut DagRuntimeState,
    session_id: u64,
    report: DagProposerVdfProofReport,
) -> Result<DagProposerSessionStep> {
    let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
        return Ok(dag_proposer_session_not_started_step());
    };
    if !matches!(session.action, DagProposerSessionAction::StartVdf) {
        let step = invalid_dag_proposer_report(
            session,
            "DAG_PROPOSER_SESSION_UNEXPECTED_VDF_PROOF_REPORT",
        );
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    if !report.proof_ok {
        let step = invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_VDF_PROOF_FAILED");
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    if let Some(step) = revalidate_proposer_session_observation(runtime, session_id)? {
        return Ok(step);
    }
    if runtime.proposer_sessions[&session_id].attempt.vdf_stale {
        let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
        session.vdf_rlp = report.vdf_rlp;
        session.action = DagProposerSessionAction::StaleProofSleep;
        return Ok(dag_proposer_session_step(session));
    }
    prepare_proposer_session_signing(runtime, session_id, report.vdf_rlp)
}

/// Resumes a stale-proof cursor after the external compatibility sleep.
///
/// Rust revalidates the complete observation before using the stored proof. An unchanged observation constructs the
/// unsigned intent and returns signing action 5; a stale observation terminates without retry mutation. Construction
/// errors remove the cursor before returning `Err`, and missing/out-of-order ids return invalid terminal steps.
pub fn dag_manager_runtime_proposer_session_resume_stale_proof(
    runtime: &mut DagRuntimeState,
    session_id: u64,
) -> Result<DagProposerSessionStep> {
    let latest_proposal_level = runtime.state.proposer_frontier_facts().propose_level;
    let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
        return Ok(dag_proposer_session_not_started_step());
    };
    if !matches!(session.action, DagProposerSessionAction::StaleProofSleep) {
        let step = invalid_dag_proposer_report(
            session,
            "DAG_PROPOSER_SESSION_UNEXPECTED_STALE_PROOF_REPORT",
        );
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    if let Some(step) = revalidate_proposer_session_observation(runtime, session_id)? {
        return Ok(step);
    }
    let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
    let stale = plan_dag_proposer_stale_proof(rustaxa_consensus::dag::DagProposerStaleProofInput {
        proposal_level: session.attempt.proposal_level,
        latest_proposal_level,
    });
    session.reason_code = stale.reason_code;
    session.update_retry_state = stale.update_retry_state;
    session.next_last_propose_level = stale.next_last_propose_level;
    session.next_retry_count = stale.next_retry_count;
    if stale.action != rustaxa_consensus::dag::DAG_PROPOSER_ACTION_CONTINUE {
        session.action = DagProposerSessionAction::Complete;
        session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
        session.return_value = false;
        let step = dag_proposer_session_step(session);
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    let vdf_rlp = session.vdf_rlp.clone();
    prepare_proposer_session_signing(runtime, session_id, vdf_rlp)
}

/// Finalizes the cursor's unsigned intent with an external recoverable signature.
///
/// The report contains only the 65-byte recoverable signature over the previously returned signing hash. Rust requires
/// recovery to match the trusted proposer address captured at begin, then assembles/stores canonical signed RLP/hash and
/// returns add-block action 6. Malformed or wrong-key signatures and finalization errors remove the cursor before
/// returning `Err`; missing/out-of-order reports return invalid terminal steps.
pub fn dag_manager_runtime_proposer_session_report_signing(
    runtime: &mut DagRuntimeState,
    session_id: u64,
    report: DagProposerSigningReport,
) -> Result<DagProposerSessionStep> {
    let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
        return Ok(dag_proposer_session_not_started_step());
    };
    if !matches!(session.action, DagProposerSessionAction::SignBlock) {
        let step =
            invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_UNEXPECTED_SIGNING_REPORT");
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    let intent = session
        .unsigned_intent
        .clone()
        .expect("signing action must own an unsigned intent");
    let proposer_address = session.begin_input.proposer_address;
    let signed = match (|| -> Result<rustaxa_consensus::dag::DagProposerSignedBlockIntent> {
        let signed =
            finalize_dag_proposer_signed_block_intent(DomainDagProposerSignedBlockIntentInput {
                intent,
                signature: report.signature,
            })?;
        let block = rustaxa_types::dag::DagBlock::try_from(
            rustaxa_types::codec::rlp::dag::DagBlockRlp::new(&signed.block_rlp),
        )
        .context("DAG_PROPOSER_SIGNED_BLOCK_DECODE")?;
        let recovered = block
            .recover_sender()
            .context("DAG_PROPOSER_SIGNATURE_RECOVERY")?;
        ensure!(
            recovered.0 == proposer_address,
            "DAG_PROPOSER_SIGNATURE_PROPOSER_MISMATCH"
        );
        Ok(signed)
    })() {
        Ok(signed) => signed,
        Err(error) => {
            runtime.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
    session.signed_intent = Some(signed);
    session.action = DagProposerSessionAction::AddBlock;
    Ok(dag_proposer_session_step(session))
}

/// Reports `DagManager::addDagBlock` execution to the runtime-owned DAG proposal cursor.
pub fn dag_manager_runtime_proposer_session_report_add_block(
    runtime: &mut DagRuntimeState,
    session_id: u64,
    report: DagProposerAddBlockReport,
) -> DagProposerSessionStep {
    let step = {
        let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
            return dag_proposer_session_not_started_step();
        };
        if !matches!(session.action, DagProposerSessionAction::AddBlock) {
            invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_UNEXPECTED_ADD_BLOCK_REPORT")
        } else if ((report.accepted || report.duplicate) && report.expired)
            || (report.accepted && !report.missing_references.is_empty())
        {
            invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_INVALID_ADD_BLOCK_REPORT")
        } else {
            let retry =
                plan_dag_proposer_retry_reset(rustaxa_consensus::dag::DagProposerRetryResetInput {
                    proposal_level: session.attempt.proposal_level,
                });
            session.action = DagProposerSessionAction::Complete;
            session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
            session.reason_code = if report.accepted {
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_OK
            } else if report.expired {
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_ADD_BLOCK_EXPIRED
            } else if !report.missing_references.is_empty() {
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_ADD_BLOCK_MISSING_REFERENCES
            } else {
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_ADD_BLOCK_REJECTED
            };
            session.return_value = report.accepted;
            session.update_retry_state = retry.update_retry_state;
            session.next_last_propose_level = retry.next_last_propose_level;
            session.next_retry_count = retry.next_retry_count;
            session.record_proposed_block = report.accepted;
            dag_proposer_session_step(session)
        }
    };
    finish_dag_proposer_session_step(runtime, session_id, step)
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

fn domain_attempt_input(
    input: &DomainDagProposerSessionBeginInput,
    transaction_observation: DagProposerTransactionObservation,
    observation: &DagProposerObservation,
    last_finalized_period: u64,
    authorization_facts: rustaxa_consensus::dag::DagDposAuthorizationFacts,
    sortition_params: SortitionParams,
    last_propose_level: u64,
    retry_count: u64,
) -> DomainDagProposerAttemptInput {
    DomainDagProposerAttemptInput {
        transaction_pool_size: transaction_observation.transaction_pool_size,
        non_finalized_transaction_count: transaction_observation.non_finalized_transaction_count,
        max_non_finalized_transactions: input.max_non_finalized_transactions,
        frontier: observation.frontier.clone(),
        proposal_period_found: observation.proposal_period_found,
        proposal_period: observation.proposal_period,
        last_finalized_period,
        dag_expiry_level_limit: input.dag_expiry_level_limit,
        period_block_hash_found: observation.period_block_hash_found,
        period_block_hash: observation.period_block_hash,
        wallet_vrf_public_key: input.wallet_vrf_public_key,
        wallet_vrf_secret: input.wallet_vrf_secret,
        authorization_facts: rustaxa_consensus::dag::DagDposAuthorizationFacts {
            vrf_key: authorization_facts.vrf_key,
            vrf_key_found: authorization_facts.vrf_key_found,
            sender_eligible_vote_count: authorization_facts.sender_eligible_vote_count,
            vdf_sortition_max_vote_count: authorization_facts.vdf_sortition_max_vote_count,
            eligibility_status: authorization_facts.eligibility_status,
        },
        sortition_params,
        max_non_finalized_dag_blocks: input.max_non_finalized_dag_blocks,
        max_non_finalized_dag_blocks_low_difficulty: input
            .max_non_finalized_dag_blocks_low_difficulty,
        last_propose_level,
        retry_count,
        max_retry_count: input.max_retry_count,
        proposal_weight_limit: input.proposal_weight_limit,
        total_transaction_shards: input.total_transaction_shards,
        node_transaction_shard: input.node_transaction_shard,
        shard_period_interval: input.shard_period_interval,
    }
}

fn placeholder_attempt(
    observation: &DagProposerObservation,
    input: &DomainDagProposerSessionBeginInput,
) -> DomainDagProposerAttemptPlan {
    DomainDagProposerAttemptPlan {
        action: rustaxa_consensus::dag::DAG_PROPOSER_ACTION_SKIP,
        reason_code: rustaxa_consensus::dag::DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD,
        frontier: observation.frontier.frontier.clone(),
        anchor: observation.frontier.anchor,
        proposal_level: observation.frontier.propose_level,
        proposal_period_found: observation.proposal_period_found,
        proposal_period: observation.proposal_period,
        last_finalized_period: 0,
        period_block_hash_found: observation.period_block_hash_found,
        period_block_hash: observation.period_block_hash,
        vrf_input: Vec::new(),
        vote_count: 0,
        max_vote_count: 0,
        vdf_difficulty: 0,
        vdf_stale: false,
        old_proposal: false,
        update_retry_state: false,
        next_last_propose_level: 0,
        next_retry_count: 0,
        proposal_weight_limit: input.proposal_weight_limit,
        total_transaction_shards: input.total_transaction_shards,
        node_transaction_shard: input.node_transaction_shard,
        shard_period_interval: input.shard_period_interval,
    }
}

fn to_domain_proposer_session_begin_input(
    input: DagProposerSessionBeginInput,
) -> DomainDagProposerSessionBeginInput {
    DomainDagProposerSessionBeginInput {
        max_non_finalized_transactions: input.max_non_finalized_transactions,
        dag_expiry_level_limit: input.dag_expiry_level_limit,
        wallet_vrf_public_key: input.wallet_vrf_public_key,
        wallet_vrf_secret: input.wallet_vrf_secret,
        proposer_address: input.proposer_address,
        max_non_finalized_dag_blocks: input.max_non_finalized_dag_blocks,
        max_non_finalized_dag_blocks_low_difficulty: input
            .max_non_finalized_dag_blocks_low_difficulty,
        max_retry_count: input.max_retry_count,
        proposal_weight_limit: input.proposal_weight_limit,
        total_transaction_shards: input.total_transaction_shards,
        node_transaction_shard: input.node_transaction_shard,
        shard_period_interval: input.shard_period_interval,
        pbft_gas_limit: input.pbft_gas_limit,
        dag_gas_limit: input.dag_gas_limit,
        max_tips: input.max_tips,
    }
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

fn proposer_observation_fingerprint(
    frontier: &DomainDagProposerFrontierFacts,
    proposal_period_found: bool,
    proposal_period: u64,
    period_block_hash_found: bool,
    period_block_hash: H256,
) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(frontier.frontier.pivot.as_bytes());
    for tip in &frontier.frontier.tips {
        hasher.update(tip.as_bytes());
    }
    hasher.update(&frontier.propose_level.to_be_bytes());
    hasher.update(frontier.anchor.as_bytes());
    hasher.update(&(frontier.non_finalized_block_count as u64).to_be_bytes());
    hasher.update(&frontier.non_finalized_min_difficulty.to_be_bytes());
    hasher.update(&[u8::from(proposal_period_found)]);
    hasher.update(&proposal_period.to_be_bytes());
    hasher.update(&[u8::from(period_block_hash_found)]);
    hasher.update(period_block_hash.as_bytes());
    let mut output = [0_u8; 32];
    hasher.finalize(&mut output);
    output
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

fn dag_proposer_session_step(session: &DagProposerSession) -> DagProposerSessionStep {
    let action = match session.action {
        DagProposerSessionAction::CollectFinalChainFacts => {
            DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
        }
        DagProposerSessionAction::PackTransactions => DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS,
        DagProposerSessionAction::StartVdf => DAG_PROPOSER_SESSION_ACTION_START_VDF,
        DagProposerSessionAction::CancelVdf => DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF,
        DagProposerSessionAction::StaleProofSleep => DAG_PROPOSER_SESSION_ACTION_STALE_PROOF_SLEEP,
        DagProposerSessionAction::SignBlock => DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK,
        DagProposerSessionAction::AddBlock => DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK,
        DagProposerSessionAction::Complete => DAG_PROPOSER_SESSION_ACTION_NONE,
    };
    DagProposerSessionStep {
        status: session.status,
        action,
        reason_code: session.reason_code,
        return_value: session.return_value,
        update_retry_state: session.update_retry_state,
        next_last_propose_level: session.next_last_propose_level,
        next_retry_count: session.next_retry_count,
        frontier_pivot: session.attempt.frontier.pivot.into(),
        proposal_level: session.attempt.proposal_level,
        proposal_period: session.attempt.proposal_period,
        last_finalized_period: session.attempt.last_finalized_period,
        vrf_input: session.attempt.vrf_input.clone(),
        vote_count: session.attempt.vote_count,
        max_vote_count: session.attempt.max_vote_count,
        vdf_difficulty: session.attempt.vdf_difficulty,
        vdf_sortition_params: if matches!(session.action, DagProposerSessionAction::StartVdf) {
            legacy_sortition_params(session.sortition_params)
        } else {
            legacy_sortition_params(empty_sortition_params())
        },
        vdf_stale: session.attempt.vdf_stale,
        old_proposal: session.attempt.old_proposal,
        vdf_message: session.vdf_message.clone(),
        selected_transaction_hashes: to_dag_hashes(session.selected_transaction_hashes.clone()),
        transaction_estimate_requests: Vec::new(),
        selected_transactions: if matches!(session.action, DagProposerSessionAction::AddBlock) {
            session
                .selected_transactions
                .iter()
                .map(|selected| TransactionPackSelectedTransaction {
                    hash: selected.hash.0,
                    gas_used: selected.gas_used,
                    tx_rlp: selected.transaction_rlp.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
        signing_hash: session
            .unsigned_intent
            .as_ref()
            .map_or([0; 32], |intent| intent.signing_hash.0),
        signed_block: session.signed_intent.as_ref().map_or(
            DagProposerSignedBlockIntent {
                block_rlp: Vec::new(),
                block_hash: [0; 32],
            },
            |intent| DagProposerSignedBlockIntent {
                block_rlp: intent.block_rlp.clone(),
                block_hash: intent.block_hash.0,
            },
        ),
        record_proposed_block: session.record_proposed_block,
        vdf_poll_interval_ms: rustaxa_consensus::dag::DAG_PROPOSER_VDF_POLL_INTERVAL_MS,
        stale_proof_sleep_ms: rustaxa_consensus::dag::DAG_PROPOSER_STALE_PROOF_SLEEP_MS,
        error_code: session.error_code.clone(),
    }
}

fn invalid_dag_proposer_report(
    session: &mut DagProposerSession,
    error_code: &str,
) -> DagProposerSessionStep {
    session.action = DagProposerSessionAction::Complete;
    session.status = DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT;
    session.return_value = false;
    session.error_code = error_code.to_string();
    dag_proposer_session_step(session)
}

fn finish_dag_proposer_session_step(
    runtime: &mut DagRuntimeState,
    session_id: u64,
    step: DagProposerSessionStep,
) -> DagProposerSessionStep {
    if !dag_proposer_session_step_is_terminal(&step) {
        return step;
    }

    if step.update_retry_state {
        if let Some(session) = runtime.proposer_sessions.get(&session_id) {
            if let Some(retry_state) = runtime.proposer_retry_states.get_mut(&session.retry_key) {
                retry_state.last_propose_level = step.next_last_propose_level;
                retry_state.retry_count = step.next_retry_count;
            }
        }
    }
    runtime.proposer_sessions.remove(&session_id);
    step
}

fn dag_proposer_session_step_is_terminal(step: &DagProposerSessionStep) -> bool {
    step.status != DAG_PROPOSER_SESSION_STATUS_ACTIVE
}

fn dag_proposer_session_not_started_step() -> DagProposerSessionStep {
    DagProposerSessionStep {
        status: DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT,
        action: DAG_PROPOSER_SESSION_ACTION_NONE,
        reason_code: rustaxa_consensus::dag::DAG_PROPOSER_REASON_OK,
        return_value: false,
        update_retry_state: false,
        next_last_propose_level: 0,
        next_retry_count: 0,
        frontier_pivot: [0; 32],
        proposal_level: 0,
        proposal_period: 0,
        last_finalized_period: 0,
        vrf_input: Vec::new(),
        vote_count: 0,
        max_vote_count: 0,
        vdf_difficulty: 0,
        vdf_sortition_params: legacy_sortition_params(empty_sortition_params()),
        vdf_stale: false,
        old_proposal: false,
        vdf_message: Vec::new(),
        selected_transaction_hashes: Vec::new(),
        transaction_estimate_requests: Vec::new(),
        selected_transactions: Vec::new(),
        signing_hash: [0; 32],
        signed_block: DagProposerSignedBlockIntent {
            block_rlp: Vec::new(),
            block_hash: [0; 32],
        },
        record_proposed_block: false,
        vdf_poll_interval_ms: rustaxa_consensus::dag::DAG_PROPOSER_VDF_POLL_INTERVAL_MS,
        stale_proof_sleep_ms: rustaxa_consensus::dag::DAG_PROPOSER_STALE_PROOF_SLEEP_MS,
        error_code: "DAG_PROPOSER_SESSION_NOT_STARTED".to_string(),
    }
}

fn verify_block_session_step(session: &DagVerifyBlockSession) -> DagVerifyBlockSessionStep {
    match &session.action {
        DagVerifyBlockSessionAction::TransactionQuery(_) => DagVerifyBlockSessionStep {
            cursor_id: session.cursor_id,
            status: DAG_VERIFY_SESSION_STATUS_ACTIVE,
            action: DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY,
            complete: false,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            vote_count: 0,
            max_vote_count: 0,
            error_code: session.error_code.clone(),
        },
        DagVerifyBlockSessionAction::AuthorizationFacts => DagVerifyBlockSessionStep {
            cursor_id: session.cursor_id,
            status: DAG_VERIFY_SESSION_STATUS_ACTIVE,
            action: DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS,
            complete: false,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            vote_count: 0,
            max_vote_count: 0,
            error_code: session.error_code.clone(),
        },
        DagVerifyBlockSessionAction::VdfSortition {
            vote_count,
            max_vote_count,
            ..
        } => DagVerifyBlockSessionStep {
            cursor_id: session.cursor_id,
            status: DAG_VERIFY_SESSION_STATUS_ACTIVE,
            action: DAG_VERIFY_SESSION_ACTION_VDF_SORTITION,
            complete: false,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            vote_count: *vote_count,
            max_vote_count: *max_vote_count,
            error_code: session.error_code.clone(),
        },
        DagVerifyBlockSessionAction::Gas => DagVerifyBlockSessionStep {
            cursor_id: session.cursor_id,
            status: DAG_VERIFY_SESSION_STATUS_ACTIVE,
            action: DAG_VERIFY_SESSION_ACTION_GAS,
            complete: false,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            vote_count: 0,
            max_vote_count: 0,
            error_code: session.error_code.clone(),
        },
        DagVerifyBlockSessionAction::Complete => DagVerifyBlockSessionStep {
            cursor_id: session.cursor_id,
            status: DAG_VERIFY_SESSION_STATUS_COMPLETE,
            action: DAG_VERIFY_SESSION_ACTION_NONE,
            complete: true,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            vote_count: 0,
            max_vote_count: 0,
            error_code: session.error_code.clone(),
        },
    }
}

fn invalid_verify_block_report(
    session: &mut DagVerifyBlockSession,
    error_code: &str,
) -> DagVerifyBlockSessionStep {
    session.action = DagVerifyBlockSessionAction::Complete;
    session.generation = session.generation.wrapping_add(1).max(1);
    session.error_code = error_code.to_string();
    DagVerifyBlockSessionStep {
        cursor_id: session.cursor_id,
        status: DAG_VERIFY_SESSION_STATUS_INVALID_REPORT,
        action: DAG_VERIFY_SESSION_ACTION_NONE,
        complete: true,
        reject_code: session.reject_code,
        proposal_period: session.proposal_period,
        vote_count: 0,
        max_vote_count: 0,
        error_code: session.error_code.clone(),
    }
}

fn verify_block_session_not_started_step() -> DagVerifyBlockSessionStep {
    DagVerifyBlockSessionStep {
        cursor_id: 0,
        status: DAG_VERIFY_SESSION_STATUS_INVALID_REPORT,
        action: DAG_VERIFY_SESSION_ACTION_NONE,
        complete: true,
        reject_code: 0,
        proposal_period: 0,
        vote_count: 0,
        max_vote_count: 0,
        error_code: "DAG_VERIFY_SESSION_NOT_STARTED".to_string(),
    }
}

fn complete_verify_block_session(
    session: &mut DagVerifyBlockSession,
    reject_code: u32,
) -> DagVerifyBlockSessionStep {
    session.reject_code = reject_code;
    session.action = DagVerifyBlockSessionAction::Complete;
    session.generation = session.generation.wrapping_add(1).max(1);
    verify_block_session_step(session)
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
    use crate::ffi::rustaxa_ffi::DagProposerSessionBeginInput;
    use crate::ffi::{BridgeDagStorageQueries, BridgeStorage};
    use crate::storage::{create_dag_storage_queries, create_storage};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_consensus::dag;
    use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
    use rustaxa_types::pbft::PbftBlockLink;
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

    fn advance_verify_session_to_gas(runtime: &mut DagRuntimeState, tip: H256) {
        dag_manager_runtime_begin_verify_block_session(
            runtime,
            DagVerifyBlockSessionInput {
                block_hash: [0u8; 32],
                block_level: 5,
                pivot: [1u8; 32],
                tips: vec![DagHash { hash: tip.0 }],
                block_transaction_hashes: vec![],
                supplied_transaction_hashes: vec![],
                block_rlp: Vec::new(),
            },
        )
        .expect("verify session should initialize");
        let authorization =
            dag_manager_runtime_verify_block_session_apply_transaction_resolution(runtime, 0);
        assert_eq!(
            authorization.action,
            DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
        );
        let vdf = apply_test_verify_authorization(runtime);
        assert_eq!(vdf.action, DAG_VERIFY_SESSION_ACTION_VDF_SORTITION);
        let gas = apply_verify_block_vdf_status(
            runtime,
            rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_VALID,
        );
        assert_eq!(gas.action, DAG_VERIFY_SESSION_ACTION_GAS);
    }

    fn apply_test_verify_authorization(runtime: &mut DagRuntimeState) -> DagVerifyBlockSessionStep {
        let snapshot = match dag_manager_runtime_prepare_verify_block_authorization(runtime) {
            DagVerifyBlockAuthorizationPreparation::Snapshot(snapshot) => snapshot,
            DagVerifyBlockAuthorizationPreparation::Step(_) => {
                panic!("verification cursor should await authorization")
            }
        };
        dag_manager_runtime_apply_verify_block_authorization(
            runtime,
            &snapshot,
            rustaxa_consensus::dag::DagDposAuthorizationFacts {
                vrf_key: Some([0x44; 32]),
                vrf_key_found: true,
                sender_eligible_vote_count: 11,
                vdf_sortition_max_vote_count: 33,
                eligibility_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
            },
        )
        .expect("authorization should apply")
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

    fn proposer_session_begin_input(vrf_key: [u8; 32]) -> DagProposerSessionBeginInput {
        DagProposerSessionBeginInput {
            max_non_finalized_transactions: 100,
            dag_expiry_level_limit: 100,
            wallet_vrf_public_key: vrf_key,
            wallet_vrf_secret: SECRET_KEY,
            proposer_address: proposer_address_for_seed(0x44),
            max_non_finalized_dag_blocks: 100,
            max_non_finalized_dag_blocks_low_difficulty: 50,
            max_retry_count: 20,
            proposal_weight_limit: 1_000,
            total_transaction_shards: 4,
            node_transaction_shard: 2,
            shard_period_interval: 10,
            pbft_gas_limit: 10_000,
            dag_gas_limit: 1_000,
            max_tips: 16,
        }
    }

    fn begin_test_proposer_session(
        runtime: &mut DagRuntimeState,
        input: DagProposerSessionBeginInput,
    ) -> Result<u64> {
        dag_manager_runtime_begin_proposer_session(
            runtime,
            input,
            DagProposerTransactionObservation {
                transaction_pool_size: 1,
                non_finalized_transaction_count: 0,
            },
        )
    }

    struct ProposerFinalChainTestFacts {
        last_finalized_period: u64,
        authorization_facts: dag::DagDposAuthorizationFacts,
        sortition_params: SortitionParams,
    }

    fn proposer_final_chain_facts(vrf_key: [u8; 32]) -> ProposerFinalChainTestFacts {
        ProposerFinalChainTestFacts {
            last_finalized_period: 7,
            authorization_facts: dag::DagDposAuthorizationFacts {
                vrf_key: Some(vrf_key),
                vrf_key_found: true,
                sender_eligible_vote_count: 10,
                vdf_sortition_max_vote_count: 20,
                eligibility_status: dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
            },
            sortition_params: SortitionParams {
                vrf: VrfParams {
                    threshold_upper: u16::MAX,
                },
                vdf: VdfParams {
                    difficulty_min: 3,
                    difficulty_max: 3,
                    difficulty_stale: 9,
                    lambda_bound: 128,
                },
            },
        }
    }

    fn apply_test_proposer_final_chain_facts(
        runtime: &mut DagRuntimeState,
        session_id: u64,
        facts: ProposerFinalChainTestFacts,
    ) -> Result<DagProposerSessionStep> {
        let snapshot =
            match dag_manager_runtime_prepare_proposer_final_chain_facts(runtime, session_id) {
                DagProposerFinalChainFactsPreparation::Snapshot(snapshot) => snapshot,
                DagProposerFinalChainFactsPreparation::Step(step) => return Ok(*step),
            };
        dag_manager_runtime_apply_proposer_final_chain_facts(
            runtime,
            &snapshot,
            facts.last_finalized_period,
            facts.authorization_facts,
            facts.sortition_params,
            facts.sortition_params,
        )
    }

    fn proposer_address_for_seed(seed: u8) -> [u8; 20] {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("signing key");
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let mut public_key_hash = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&encoded.as_bytes()[1..]);
        hasher.finalize(&mut public_key_hash);
        public_key_hash[12..]
            .try_into()
            .expect("address slice has fixed length")
    }

    fn sign_proposer_hash_with_seed(signing_hash: [u8; 32], seed: u8) -> Vec<u8> {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("signing key");
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&signing_hash)
            .expect("sign proposer intent");
        let mut bytes = signature.to_bytes().to_vec();
        bytes.push(recovery_id.to_byte());
        bytes
    }

    fn sign_proposer_hash(signing_hash: [u8; 32]) -> Vec<u8> {
        sign_proposer_hash_with_seed(signing_hash, 0x44)
    }

    fn begin_proposer_vdf_session(
        runtime: &mut DagRuntimeState,
        vrf_key: [u8; 32],
        transaction_hash: [u8; 32],
    ) -> u64 {
        let session_id =
            begin_test_proposer_session(runtime, proposer_session_begin_input(vrf_key))
                .expect("session should open");
        assert_eq!(
            apply_test_proposer_final_chain_facts(
                runtime,
                session_id,
                proposer_final_chain_facts(vrf_key),
            )
            .expect("external facts should succeed")
            .action,
            DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS
        );
        assert_eq!(
            dag_manager_runtime_apply_proposer_pack(
                runtime,
                session_id,
                false,
                vec![TransactionPackSelectedTransaction {
                    hash: transaction_hash,
                    gas_used: 100,
                    tx_rlp: vec![transaction_hash[0]],
                }],
            )
            .expect("pack should apply")
            .action,
            DAG_PROPOSER_SESSION_ACTION_START_VDF
        );
        session_id
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
    fn dag_manager_runtime_proposer_session_orders_executor_reports() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_session");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 7)
                .expect("proposal-period mapping should save");
            let pbft_block = signed_pbft_block(7, 99);
            storage
                .0
                .period()
                .write(7, &period_data_with_pbft_block(&pbft_block))
                .expect("period data save should succeed");

            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("session should open");

            let first = dag_manager_runtime_proposer_session_next(&mut runtime, session_id);
            assert_eq!(first.status, DAG_PROPOSER_SESSION_STATUS_ACTIVE);
            assert_eq!(
                first.action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );
            assert_eq!(first.proposal_period, 7);

            let pack = apply_test_proposer_final_chain_facts(
                &mut runtime,
                session_id,
                proposer_final_chain_facts(vrf_key),
            )
            .expect("external facts should be accepted");
            assert_eq!(pack.action, DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS);
            assert_eq!(first.vdf_poll_interval_ms, 100);
            assert_eq!(first.stale_proof_sleep_ms, 1_000);
            assert!(!pack.vrf_input.is_empty());

            let start_vdf = dag_manager_runtime_apply_proposer_pack(
                &mut runtime,
                session_id,
                false,
                vec![TransactionPackSelectedTransaction {
                    hash: [2u8; 32],
                    gas_used: 100,
                    tx_rlp: vec![0xC0],
                }],
            )
            .expect("pack should apply");
            assert_eq!(start_vdf.action, DAG_PROPOSER_SESSION_ACTION_START_VDF);
            assert_eq!(start_vdf.selected_transaction_hashes.len(), 1);
            assert_eq!(
                start_vdf.vdf_message,
                dag_vdf_message(&[1u8; 32], vec![DagHash { hash: [2u8; 32] }])
            );

            let still_waiting =
                dag_manager_runtime_proposer_session_poll_vdf(&mut runtime, session_id);
            assert_eq!(still_waiting.action, DAG_PROPOSER_SESSION_ACTION_START_VDF);

            let build = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("block construction should succeed");
            assert_eq!(build.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);
            assert_ne!(build.signing_hash, [0; 32]);

            let add = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                session_id,
                DagProposerSigningReport {
                    signature: sign_proposer_hash(build.signing_hash),
                },
            )
            .expect("signed intent should finalize");
            assert_eq!(add.action, DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK);
            assert!(!add.signed_block.block_rlp.is_empty());
            assert_ne!(add.signed_block.block_hash, [0; 32]);
            let decoded = rustaxa_types::dag::DagBlock::try_from(
                rustaxa_types::codec::rlp::dag::DagBlockRlp::new(&add.signed_block.block_rlp),
            )
            .expect("canonical signed block should decode");
            assert_eq!(decoded.pivot, H256::from([1u8; 32]));
            assert_eq!(decoded.level, 1);
            assert_eq!(decoded.vdf, vec![0xC0]);
            assert_eq!(decoded.transactions, vec![H256::from([2u8; 32])]);
            let mut expected_hash = [0u8; 32];
            let mut hasher = Keccak::v256();
            hasher.update(&add.signed_block.block_rlp);
            hasher.finalize(&mut expected_hash);
            assert_eq!(add.signed_block.block_hash, expected_hash);

            let complete = dag_manager_runtime_proposer_session_report_add_block(
                &mut runtime,
                session_id,
                DagProposerAddBlockReport {
                    accepted: true,
                    duplicate: false,
                    expired: false,
                    missing_references: Vec::new(),
                },
            );
            assert_eq!(complete.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(complete.action, DAG_PROPOSER_SESSION_ACTION_NONE);
            assert!(complete.return_value);
            assert!(complete.record_proposed_block);
            assert!(complete.update_retry_state);
            assert_eq!(complete.next_last_propose_level, 1);
            assert_eq!(complete.next_retry_count, 0);
            let retry_state = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("terminal step should persist retry state");
            assert_eq!(retry_state.last_propose_level, 1);
            assert_eq!(retry_state.retry_count, 0);
            assert_eq!(retry_state.max_retry_count, 20);

            let after_complete =
                dag_manager_runtime_proposer_session_next(&mut runtime, session_id);
            assert_eq!(
                after_complete.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_sessions_are_independent() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_keyed_sessions");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 7)
                .expect("first proposal-period mapping should save");
            let pbft_block = signed_pbft_block(7, 99);
            storage
                .0
                .period()
                .write(7, &period_data_with_pbft_block(&pbft_block))
                .expect("period data save should succeed");

            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let first_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("first session should open");
            let second_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("second session should open");
            assert_ne!(first_id, second_id);

            let second_first_step =
                dag_manager_runtime_proposer_session_next(&mut runtime, second_id);
            assert_eq!(second_first_step.status, DAG_PROPOSER_SESSION_STATUS_ACTIVE);
            assert_eq!(
                second_first_step.action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );
            assert_eq!(second_first_step.proposal_level, 1);

            let first_first_step =
                dag_manager_runtime_proposer_session_next(&mut runtime, first_id);
            assert_eq!(first_first_step.status, DAG_PROPOSER_SESSION_STATUS_ACTIVE);
            assert_eq!(
                first_first_step.action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );
            assert_eq!(first_first_step.proposal_level, 1);

            let second_pack = apply_test_proposer_final_chain_facts(
                &mut runtime,
                second_id,
                proposer_final_chain_facts(vrf_key),
            )
            .expect("second report should succeed");
            assert_eq!(
                second_pack.action,
                DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS
            );

            let second_start_vdf = dag_manager_runtime_apply_proposer_pack(
                &mut runtime,
                second_id,
                false,
                vec![TransactionPackSelectedTransaction {
                    hash: [4u8; 32],
                    gas_used: 200,
                    tx_rlp: vec![4],
                }],
            )
            .expect("second pack should apply");
            assert_eq!(
                second_start_vdf.action,
                DAG_PROPOSER_SESSION_ACTION_START_VDF
            );
            assert_eq!(second_start_vdf.proposal_level, 1);
            assert_eq!(
                second_start_vdf.vdf_message,
                dag_vdf_message(&[1u8; 32], vec![DagHash { hash: [4u8; 32] }])
            );

            let first_still_waiting =
                dag_manager_runtime_proposer_session_next(&mut runtime, first_id);
            assert_eq!(
                first_still_waiting.action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );
            assert_eq!(first_still_waiting.proposal_level, 1);

            let first_pack = apply_test_proposer_final_chain_facts(
                &mut runtime,
                first_id,
                proposer_final_chain_facts(vrf_key),
            )
            .expect("first report should succeed");
            assert_eq!(
                first_pack.action,
                DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS
            );

            let first_start_vdf = dag_manager_runtime_apply_proposer_pack(
                &mut runtime,
                first_id,
                false,
                vec![TransactionPackSelectedTransaction {
                    hash: [2u8; 32],
                    gas_used: 100,
                    tx_rlp: vec![2],
                }],
            )
            .expect("first pack should apply");
            assert_eq!(
                first_start_vdf.action,
                DAG_PROPOSER_SESSION_ACTION_START_VDF
            );
            assert_eq!(first_start_vdf.proposal_level, 1);
            assert_eq!(
                first_start_vdf.vdf_message,
                dag_vdf_message(&[1u8; 32], vec![DagHash { hash: [2u8; 32] }])
            );

            let second_sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                second_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC1, 0x02],
                },
            )
            .expect("second intent should build");
            let first_sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                first_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC1, 0x01],
                },
            )
            .expect("first intent should build");
            assert_eq!(second_sign.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);
            assert_eq!(first_sign.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);
            assert_ne!(second_sign.signing_hash, first_sign.signing_hash);
            assert_eq!(
                runtime.proposer_sessions[&second_id]
                    .unsigned_intent
                    .as_ref()
                    .expect("second intent")
                    .transaction_hashes,
                vec![H256::from([4u8; 32])]
            );
            assert_eq!(
                runtime.proposer_sessions[&first_id]
                    .unsigned_intent
                    .as_ref()
                    .expect("first intent")
                    .transaction_hashes,
                vec![H256::from([2u8; 32])]
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_handles_missing_period_and_invalid_reports() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_invalid");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");

            let missing_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("missing-period session should open");
            let missing = dag_manager_runtime_proposer_session_next(&mut runtime, missing_id);
            assert_eq!(missing.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(
                missing.reason_code,
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD
            );

            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let invalid_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("session should open");
            let invalid = dag_manager_runtime_apply_proposer_pack(
                &mut runtime,
                invalid_id,
                false,
                Vec::new(),
            )
            .err()
            .expect("out-of-order pack must fail");
            assert!(invalid
                .to_string()
                .contains("DAG_PROPOSER_PACK_SESSION_WRONG_STAGE"));
            assert!(dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                invalid_id
            ));

            let after_invalid = apply_test_proposer_final_chain_facts(
                &mut runtime,
                invalid_id,
                proposer_final_chain_facts(vrf_key),
            )
            .expect("unknown-session report should return a step");
            assert_eq!(
                after_invalid.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_abort_proposer_session_is_idempotent() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_abort");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("session should open");
            assert!(runtime.proposer_sessions.contains_key(&session_id));

            assert!(dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                u64::MAX
            ));
            assert!(!runtime.proposer_retry_states.contains_key(&vrf_key));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_final_chain_facts_error_removes_proposer_session() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_report_error");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 7)
                .expect("proposal-period mapping should save");
            storage
                .0
                .period()
                .write(7, &period_data_with_pbft_block(&signed_pbft_block(7, 99)))
                .expect("valid period data should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("session should open");
            assert!(runtime.proposer_sessions.contains_key(&session_id));

            storage
                .0
                .period()
                .write(7, &vec![0x80])
                .expect("corrupt period data should save");
            assert!(apply_test_proposer_final_chain_facts(
                &mut runtime,
                session_id,
                proposer_final_chain_facts(vrf_key),
            )
            .is_err());
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            assert!(!runtime.proposer_retry_states.contains_key(&vrf_key));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_rejects_stale_observation_before_retry_mutation() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_stale_observation");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("session should open");
            assert_eq!(
                dag_manager_runtime_proposer_session_next(&mut runtime, session_id).action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                })
                .expect("frontier mutation should succeed");
            let stale = apply_test_proposer_final_chain_facts(
                &mut runtime,
                session_id,
                proposer_final_chain_facts(vrf_key),
            )
            .expect("stale report should return a terminal step");
            assert_eq!(stale.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(
                stale.reason_code,
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION
            );
            assert_eq!(stale.error_code, "DAG_PROPOSER_SESSION_STALE_OBSERVATION");
            assert!(!runtime.proposer_retry_states.contains_key(&vrf_key));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_rejects_stale_observation_after_vdf() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_stale_after_vdf");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [2u8; 32]);

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                })
                .expect("frontier mutation should succeed");
            let stale = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("stale observation should return a terminal step");
            assert_eq!(stale.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(
                stale.reason_code,
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION
            );
            assert!(!stale.update_retry_state);
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("external facts initialize retry state");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_cleans_up_malformed_signature() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_bad_signature");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [2u8; 32]);
            let sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("construction should succeed");
            assert_eq!(sign.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);

            let error = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                session_id,
                DagProposerSigningReport {
                    signature: vec![0; 65],
                },
            )
            .err()
            .expect("structurally invalid signature must fail recovery");
            assert!(error
                .to_string()
                .contains("DAG_PROPOSER_SIGNATURE_RECOVERY"));
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("external facts initialize retry state");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_rejects_valid_wrong_key_signature() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_wrong_signer");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [2u8; 32]);
            let sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("construction should succeed");

            let error = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                session_id,
                DagProposerSigningReport {
                    signature: sign_proposer_hash_with_seed(sign.signing_hash, 0x45),
                },
            )
            .err()
            .expect("wrong-key signature must be rejected");
            assert!(error
                .to_string()
                .contains("DAG_PROPOSER_SIGNATURE_PROPOSER_MISMATCH"));
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("external facts initialize retry state");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_cleans_up_corrupt_tip_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_corrupt_tip");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            for block in [
                DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                },
                DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 80,
                },
            ] {
                runtime
                    .dag_manager_runtime_add_block(block)
                    .expect("graph branch should add");
            }
            let frontier = runtime.state.proposer_frontier_facts();
            assert!(!frontier.frontier.tips.is_empty());
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(frontier.propose_level, 0)
                .expect("bootstrap proposal-period mapping should save");
            let corrupt_tip = frontier.frontier.tips[0];
            runtime
                .dag_manager_runtime_save_block(&corrupt_tip.0, 2, 0, vec![0x80])
                .expect("corrupt canonical row should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [4u8; 32]);

            assert!(dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .is_err());
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("external facts initialize retry state");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_rejects_duplicate_and_out_of_order_reports() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_report_order");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");

            let out_of_order_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [2u8; 32]);
            let out_of_order = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                out_of_order_id,
                DagProposerSigningReport {
                    signature: vec![0; 65],
                },
            )
            .expect("out-of-order report should return a step");
            assert_eq!(
                out_of_order.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );

            let duplicate_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [3u8; 32]);
            let sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                duplicate_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("construction should succeed");
            assert_eq!(sign.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);
            let duplicate = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                duplicate_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("duplicate report should return a step");
            assert_eq!(duplicate.status, DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT);

            let duplicate_signing_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [4u8; 32]);
            let sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                duplicate_signing_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("construction should succeed");
            let add = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                duplicate_signing_id,
                DagProposerSigningReport {
                    signature: sign_proposer_hash(sign.signing_hash),
                },
            )
            .expect("signing should succeed");
            assert_eq!(add.action, DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK);
            let duplicate_signing = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                duplicate_signing_id,
                DagProposerSigningReport {
                    signature: sign_proposer_hash(sign.signing_hash),
                },
            )
            .expect("duplicate signing should return a step");
            assert_eq!(
                duplicate_signing.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_uses_runtime_frontier_for_vdf_cancel() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_vdf_cancel");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("session should open");
            let pack = apply_test_proposer_final_chain_facts(
                &mut runtime,
                session_id,
                proposer_final_chain_facts(vrf_key),
            )
            .expect("external facts should be accepted");
            assert_eq!(pack.action, DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS);
            let start = dag_manager_runtime_apply_proposer_pack(
                &mut runtime,
                session_id,
                false,
                vec![TransactionPackSelectedTransaction {
                    hash: [2u8; 32],
                    gas_used: 100,
                    tx_rlp: vec![2],
                }],
            )
            .expect("pack should apply");
            assert_eq!(start.action, DAG_PROPOSER_SESSION_ACTION_START_VDF);
            runtime
                .proposer_sessions
                .get_mut(&session_id)
                .expect("session should remain active")
                .minimum_vdf_difficulty = start.vdf_difficulty.saturating_sub(1);

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                })
                .expect("frontier mutation should succeed");
            let cancelled = dag_manager_runtime_proposer_session_poll_vdf(&mut runtime, session_id);
            assert_eq!(cancelled.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(cancelled.action, DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_resumes_stale_proof_from_runtime_frontier() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_stale_resume");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            runtime.proposer_retry_states.insert(
                vrf_key,
                DagProposerRetryState {
                    last_propose_level: 1,
                    retry_count: 20,
                    max_retry_count: 20,
                },
            );
            let session_id =
                begin_test_proposer_session(&mut runtime, proposer_session_begin_input(vrf_key))
                    .expect("session should open");
            let mut facts = proposer_final_chain_facts(vrf_key);
            facts.sortition_params.vdf.difficulty_min = 9;
            facts.sortition_params.vdf.difficulty_max = 9;
            facts.sortition_params.vdf.difficulty_stale = 9;
            let pack = apply_test_proposer_final_chain_facts(&mut runtime, session_id, facts)
                .expect("external facts should be accepted");
            assert!(pack.vdf_stale);
            assert_eq!(pack.action, DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS);
            assert_eq!(
                dag_manager_runtime_apply_proposer_pack(
                    &mut runtime,
                    session_id,
                    false,
                    vec![TransactionPackSelectedTransaction {
                        hash: [2u8; 32],
                        gas_used: 100,
                        tx_rlp: vec![2],
                    }],
                )
                .expect("pack should apply")
                .action,
                DAG_PROPOSER_SESSION_ACTION_START_VDF
            );
            let sleep = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("stale proof should request sleep");
            assert_eq!(sleep.action, DAG_PROPOSER_SESSION_ACTION_STALE_PROOF_SLEEP);

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                })
                .expect("frontier mutation should succeed");

            let resumed =
                dag_manager_runtime_proposer_session_resume_stale_proof(&mut runtime, session_id)
                    .expect("stale resume should produce a terminal step");
            assert_eq!(resumed.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(resumed.action, DAG_PROPOSER_SESSION_ACTION_NONE);
            assert_eq!(
                resumed.reason_code,
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION
            );
            assert!(!resumed.update_retry_state);
            assert_eq!(resumed.proposal_level, 1);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_verify_block_session_orders_live_reports() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_verify_session");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(5, 7)
                .expect("mapping write should succeed");

            dag_manager_runtime_begin_verify_block_session(
                &mut runtime,
                DagVerifyBlockSessionInput {
                    block_hash: [0u8; 32],
                    block_level: 5,
                    pivot: [1u8; 32],
                    tips: vec![],
                    block_transaction_hashes: vec![
                        DagTransactionHash { hash: [2u8; 32] },
                        DagTransactionHash { hash: [3u8; 32] },
                    ],
                    supplied_transaction_hashes: vec![DagTransactionHash { hash: [3u8; 32] }],
                    block_rlp: Vec::new(),
                },
            )
            .expect("session should initialize");

            let first = dag_manager_runtime_verify_block_session_next(&mut runtime);
            assert_eq!(first.status, DAG_VERIFY_SESSION_STATUS_ACTIVE);
            assert_eq!(first.action, DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY);
            assert_eq!(first.proposal_period, 7);
            let query = match dag_manager_runtime_verify_block_transaction_query(&mut runtime) {
                Ok(query) => query,
                Err(_) => panic!("transaction query should remain Rust-private"),
            };
            assert_eq!(query.hashes, vec![H256::from([2u8; 32])]);

            let auth = dag_manager_runtime_verify_block_session_apply_transaction_resolution(
                &mut runtime,
                2,
            );
            assert_eq!(auth.action, DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS);

            let vdf = apply_test_verify_authorization(&mut runtime);
            assert_eq!(vdf.action, DAG_VERIFY_SESSION_ACTION_VDF_SORTITION);
            assert_eq!(vdf.vote_count, 11);
            assert_eq!(vdf.max_vote_count, 33);

            let gas = apply_verify_block_vdf_status(
                &mut runtime,
                rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_VALID,
            );
            assert_eq!(gas.action, DAG_VERIFY_SESSION_ACTION_GAS);

            let complete = dag_manager_runtime_verify_block_session_report_gas(
                &mut runtime,
                DagVerifyBlockGasReport {
                    block_gas_estimation: 10,
                    estimated_transactions_weight: 10,
                    dag_gas_limit: 20,
                    pbft_gas_limit: 100,
                },
            )
            .expect("gas report should resolve without tip lookup");
            assert!(complete.complete);
            assert_eq!(complete.status, DAG_VERIFY_SESSION_STATUS_COMPLETE);
            assert_eq!(complete.reject_code, 0);

            dag_manager_runtime_begin_verify_block_session(
                &mut runtime,
                DagVerifyBlockSessionInput {
                    block_hash: [0u8; 32],
                    block_level: 5,
                    pivot: [1u8; 32],
                    tips: vec![],
                    block_transaction_hashes: vec![DagTransactionHash { hash: [4u8; 32] }],
                    supplied_transaction_hashes: vec![],
                    block_rlp: Vec::new(),
                },
            )
            .expect("missing session should initialize");
            let _ = dag_manager_runtime_verify_block_session_next(&mut runtime);
            let missing = dag_manager_runtime_verify_block_session_apply_transaction_resolution(
                &mut runtime,
                0,
            );
            assert!(missing.complete);
            assert_eq!(
                missing.reject_code,
                rustaxa_consensus::dag::DAG_VERIFY_REJECT_MISSING_TRANSACTION
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn verify_gas_skips_stale_tip_lookup_when_count_policy_does_not_require_it() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_verify_gas_no_tip_lookup");
        let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
            .expect("storage should initialize");
        let tip = H256::from([0x71; 32]);
        storage
            .0
            .dag()
            .write(tip, 4, 0, &signed_dag_block_rlp(0x71, 4, 25))
            .expect("tip should persist for verify precheck");
        let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
            .expect("runtime should initialize");
        runtime
            .dag_manager_runtime_ensure_proposal_period_mapping(5, 7)
            .expect("mapping write should succeed");
        advance_verify_session_to_gas(&mut runtime, tip);
        storage
            .0
            .dag()
            .remove(tip)
            .expect("tip removal should simulate a stale storage view");

        let complete = dag_manager_runtime_verify_block_session_report_gas(
            &mut runtime,
            DagVerifyBlockGasReport {
                block_gas_estimation: 10,
                estimated_transactions_weight: 10,
                dag_gas_limit: 20,
                pbft_gas_limit: 100,
            },
        )
        .expect("non-required tip gas must not touch stale storage");
        assert!(complete.complete);
        assert_eq!(complete.reject_code, 0);

        drop(runtime);
        drop(storage);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn verify_gas_loads_retained_tips_only_when_count_policy_requires_it() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_verify_gas_required_tip_lookup");
        let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
            .expect("storage should initialize");
        let tip = H256::from([0x72; 32]);
        storage
            .0
            .dag()
            .write(tip, 4, 0, &signed_dag_block_rlp(0x72, 4, 25))
            .expect("tip should persist for aggregate gas lookup");
        let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
            .expect("runtime should initialize");
        runtime
            .dag_manager_runtime_ensure_proposal_period_mapping(5, 7)
            .expect("mapping write should succeed");
        advance_verify_session_to_gas(&mut runtime, tip);

        let complete = dag_manager_runtime_verify_block_session_report_gas(
            &mut runtime,
            DagVerifyBlockGasReport {
                block_gas_estimation: 10,
                estimated_transactions_weight: 10,
                dag_gas_limit: 20,
                pbft_gas_limit: 30,
            },
        )
        .expect("required tip gas should load from private Rust storage");
        assert!(complete.complete);
        assert_eq!(
            complete.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_BLOCK_TOO_BIG
        );

        advance_verify_session_to_gas(&mut runtime, tip);
        storage
            .0
            .dag()
            .remove(tip)
            .expect("tip removal should simulate stale required metadata");
        let missing = dag_manager_runtime_verify_block_session_report_gas(
            &mut runtime,
            DagVerifyBlockGasReport {
                block_gas_estimation: 10,
                estimated_transactions_weight: 10,
                dag_gas_limit: 20,
                pbft_gas_limit: 30,
            },
        )
        .expect("a missing retained tip is a typed consensus rejection");
        assert_eq!(
            missing.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_MISSING_TIP
        );

        drop(runtime);
        drop(storage);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn verify_gas_wrong_action_rejects_before_retained_tip_lookup() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_verify_gas_wrong_action");
        let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
            .expect("storage should initialize");
        let tip = H256::from([0x73; 32]);
        storage
            .0
            .dag()
            .write(tip, 4, 0, &signed_dag_block_rlp(0x73, 4, 25))
            .expect("tip should persist for verify precheck");
        let mut runtime = build_dag_state_from_storage(&[1u8; 32], 32, &storage)
            .expect("runtime should initialize");
        runtime
            .dag_manager_runtime_ensure_proposal_period_mapping(5, 7)
            .expect("mapping write should succeed");
        advance_verify_session_to_gas(&mut runtime, tip);
        dag_manager_runtime_begin_verify_block_session(
            &mut runtime,
            DagVerifyBlockSessionInput {
                block_hash: [0u8; 32],
                block_level: 5,
                pivot: [1u8; 32],
                tips: vec![DagHash { hash: tip.0 }],
                block_transaction_hashes: vec![],
                supplied_transaction_hashes: vec![],
                block_rlp: Vec::new(),
            },
        )
        .expect("replacement session should initialize");
        storage
            .0
            .dag()
            .remove(tip)
            .expect("tip removal should expose an accidental lookup");

        let invalid = dag_manager_runtime_verify_block_session_report_gas(
            &mut runtime,
            DagVerifyBlockGasReport {
                block_gas_estimation: 10,
                estimated_transactions_weight: 10,
                dag_gas_limit: 20,
                pbft_gas_limit: 30,
            },
        )
        .expect("wrong-action report should remain status coded");
        assert_eq!(invalid.status, DAG_VERIFY_SESSION_STATUS_INVALID_REPORT);
        assert_eq!(
            invalid.error_code,
            "DAG_VERIFY_SESSION_UNEXPECTED_GAS_REPORT"
        );

        drop(runtime);
        drop(storage);
        let _ = fs::remove_dir_all(temp_dir);
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
