//! Native ownership for the DAG manager runtime.
//!
//! The service restores and owns the deterministic DAG graph, durable storage
//! handle, proposer and verifier cursors, retry state, and pending add-block
//! publication. Bridge code may temporarily borrow a short-lived typed guard
//! while FFI-shaped task methods move into this crate; the mutex itself and its
//! poison policy never leave the native owner.

use crate::dag::{
    DAG_PROPOSER_ACTION_CONTINUE, DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
    DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE, DAG_VERIFY_VDF_STATUS_NOT_CHECKED,
    DAG_VERIFY_VDF_STATUS_VALID, DagAddBlockEffectInput, DagAddBlockEffectPlan,
    DagDposAuthorizationFacts, DagHashStorageLookup, DagManagerBlock, DagManagerFinalizationPlan,
    DagManagerSnapshot, DagManagerState, DagPeriodStorageLookup, DagPersistenceCounters,
    DagProposerAttemptInput, DagProposerAttemptPlan, DagProposerBlockIntentInput,
    DagProposerFrontierFacts, DagProposerPostPackInput, DagProposerRetryResetInput,
    DagProposerSignedBlockIntent, DagProposerSignedBlockIntentInput, DagProposerStaleProofInput,
    DagProposerStorageBlockConstructionInput, DagProposerUnsignedBlockIntent,
    DagProposerVdfWaitInput, DagReferenceMetadata, DagTipGas, DagVerifyGasInput,
    DagVerifyPrecheckStorageInput, DagVerifyTransactionAvailabilityInput, DagVerifyVdfDposFacts,
    apply_finalization_cleanup_from_storage, construct_dag_vdf_message,
    dag_block_exists_in_storage, dag_manager_block_from_rlp, dag_persistence_counters_from_storage,
    decide_dag_verify_vdf_dpos_authorization, ensure_proposal_period_mapping,
    finalize_dag_proposer_signed_block_intent, period_block_hash_from_storage,
    plan_dag_add_block_effects, plan_dag_proposer_attempt,
    plan_dag_proposer_block_construction_from_storage, plan_dag_proposer_block_intent,
    plan_dag_proposer_post_pack, plan_dag_proposer_retry_reset, plan_dag_proposer_stale_proof,
    plan_dag_proposer_vdf_wait, plan_dag_verify_transaction_query,
    proposal_period_for_level_from_storage, validate_dag_verify_gas,
    validate_dag_verify_transaction_availability, validate_pivot_tips_metadata,
    verify_precheck_from_storage,
};
use crate::pbft_chain::restore_pbft_chain_from_storage;
use crate::sortition::{SortitionParams, VdfParams, VrfParams};
use crate::transaction_packing_service::TransactionPackingSelection;
use anyhow::{Context, Result, anyhow, ensure};
use ethereum_types::H256;
use rustaxa_storage::Storage;
use rustaxa_types::codec::rlp::dag::DagBlockRlp;
use rustaxa_types::dag::DagBlock;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use tiny_keccak::{Hasher, Keccak};

/// Stable failure identifier returned when the native DAG lock is poisoned.
pub const DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED: &str =
    "DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED";

const DAG_PROPOSER_SESSION_STATUS_ACTIVE: u8 = 0;
const DAG_PROPOSER_SESSION_STATUS_COMPLETE: u8 = 1;
const DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT: u8 = 2;
const DAG_VERIFY_SESSION_STATUS_ACTIVE: u8 = 0;
const DAG_VERIFY_SESSION_STATUS_COMPLETE: u8 = 1;
const DAG_VERIFY_SESSION_STATUS_INVALID_REPORT: u8 = 2;
const DAG_VERIFY_SESSION_ACTION_NONE: u8 = 0;
const DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY: u8 = 1;
const DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS: u8 = 2;
const DAG_VERIFY_SESSION_ACTION_VDF_SORTITION: u8 = 3;
const DAG_VERIFY_SESSION_ACTION_GAS: u8 = 4;

/// Session-local facts consumed by the native proposal-packing application task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DagProposerPackParameters {
    /// Proposal period used for cache and shard selection.
    pub proposal_period: u64,
    /// Maximum cumulative proposal weight.
    pub weight_limit: u64,
    /// Configured transaction shard count.
    pub total_transaction_shards: u16,
    /// Local transaction shard.
    pub node_transaction_shard: u16,
    /// Number of periods between shard rotations.
    pub shard_period_interval: u64,
}

/// Immutable inputs for restoring the native DAG owner.
#[derive(Clone, Copy, Debug)]
pub struct DagServiceConfig {
    /// Nonzero genesis DAG anchor.
    pub genesis_hash: H256,
    /// Number of levels retained behind the live DAG frontier.
    pub dag_expiry_limit: u32,
    /// Initial level whose proposal-period mapping must resolve to period zero.
    pub max_levels_per_period: u64,
}

/// Native equivalent of the CXX proposer-session construction input.
///
/// The bridge converts the public carrier once before retaining these facts in
/// the native cursor. All fields are immutable for the cursor lifetime.
#[derive(Clone)]
pub struct DagProposerSessionBeginInput {
    pub max_non_finalized_transactions: u64,
    pub dag_expiry_level_limit: u64,
    pub wallet_vrf_public_key: [u8; 32],
    pub wallet_vrf_secret: [u8; 64],
    pub proposer_address: [u8; 20],
    pub max_non_finalized_dag_blocks: u64,
    pub max_non_finalized_dag_blocks_low_difficulty: u64,
    pub max_retry_count: u64,
    pub proposal_weight_limit: u64,
    pub total_transaction_shards: u16,
    pub node_transaction_shard: u16,
    pub shard_period_interval: u64,
    pub pbft_gas_limit: u64,
    pub dag_gas_limit: u64,
    pub max_tips: u16,
}

/// Next deterministic external boundary requested by a verification cursor.
#[derive(Clone)]
pub enum DagVerifyBlockSessionAction {
    TransactionQuery(Vec<H256>),
    AuthorizationFacts,
    VdfSortition {
        vote_count: u64,
        max_vote_count: u64,
        vrf_public_key: [u8; 32],
    },
    Gas,
    Complete,
}

/// Ordered native cursor for one DAG block verification call.
pub struct DagVerifyBlockSession {
    pub cursor_id: u64,
    pub fingerprint: [u8; 32],
    pub generation: u64,
    pub action: DagVerifyBlockSessionAction,
    pub tips: Vec<H256>,
    pub proposal_period: u64,
    pub block_rlp: Vec<u8>,
    pub expected_transactions: u64,
    pub reject_code: u32,
    pub sender_eligible_vote_count: u64,
    pub vdf_sortition_max_vote_count: u64,
    pub eligibility_status: u8,
    pub error_code: String,
}

#[derive(Clone, Debug)]
pub struct DagVerifyBlockSessionInput {
    pub block_hash: [u8; 32],
    pub block_level: u64,
    pub pivot: [u8; 32],
    pub tips: Vec<H256>,
    pub block_transaction_hashes: Vec<H256>,
    pub supplied_transaction_hashes: Vec<H256>,
    pub block_rlp: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct DagVerifyBlockSessionStep {
    pub cursor_id: u64,
    pub status: u8,
    pub action: u8,
    pub complete: bool,
    pub proposal_period: u64,
    pub vote_count: u64,
    pub max_vote_count: u64,
    pub reject_code: u32,
    pub error_code: String,
}

#[derive(Clone)]
pub struct DagVerifyBlockTransactionQuery {
    pub cursor_id: u64,
    pub proposal_period: u64,
    pub hashes: Vec<H256>,
    pub expected_transactions: u64,
}

#[derive(Clone)]
pub struct DagVerifyBlockTransactionCompletion {
    pub cursor_id: u64,
    pub proposal_period: u64,
    pub resolved_transactions: u64,
}

#[derive(Clone)]
pub struct DagVerifyBlockAuthorizationSnapshot {
    pub cursor_id: u64,
    pub fingerprint: [u8; 32],
    pub generation: u64,
    pub proposal_period: u64,
    pub block_rlp: Vec<u8>,
}

pub enum DagVerifyBlockAuthorizationPreparation {
    Snapshot(DagVerifyBlockAuthorizationSnapshot),
    Step(DagVerifyBlockSessionStep),
}

#[derive(Clone)]
pub struct DagVerifyBlockVdfSnapshot {
    pub cursor_id: u64,
    pub fingerprint: [u8; 32],
    pub generation: u64,
    pub proposal_period: u64,
    pub vote_count: u64,
    pub max_vote_count: u64,
    pub vrf_public_key: [u8; 32],
}

#[derive(Clone)]
pub struct DagVerifyBlockGasReport {
    pub block_gas_estimation: u64,
    pub estimated_transactions_weight: u64,
    pub dag_gas_limit: u64,
    pub pbft_gas_limit: u64,
}

/// Input facts returned by a stale-proof VDF probe after the external proposer
/// loop attempts proof validation.
#[derive(Clone)]
pub struct DagProposerVdfProofReport {
    /// Whether the external verifier accepted the proof.
    pub proof_ok: bool,
    /// Raw VDF proof payload for the retained proposal.
    pub vdf_rlp: Vec<u8>,
}

/// Input facts returned by the external signing executor.
#[derive(Clone)]
pub struct DagProposerSigningReport {
    /// Canonical 65-byte ECDSA recoverable signature.
    pub signature: Vec<u8>,
}

/// Input facts returned by the external DAG add-block callback.
#[derive(Clone)]
pub struct DagProposerAddBlockReport {
    /// Whether execution accepted the proposed block.
    pub accepted: bool,
    /// Whether execution returned a duplicate for an already-known block.
    pub duplicate: bool,
    /// Whether the block became expired while in the external phase.
    pub expired: bool,
    /// Missing references reported by external execution.
    pub missing_references: Vec<H256>,
}

/// Cursor snapshot retained across FinalChain authorization and sortition lookup.
#[derive(Clone)]
pub(crate) struct DagProposerFinalChainFactsSnapshot {
    /// Exact native proposer cursor identity.
    pub session_id: u64,
    /// Fingerprint of the DAG/frontier observation captured at session begin.
    pub fingerprint: [u8; 32],
    /// Proposal period used for historical authorization and sortition facts.
    pub proposal_period: u64,
    /// Proposer address used for the historical authorization query.
    pub proposer_address: [u8; 20],
    /// Whether native storage resolved the proposal-period mapping.
    pub proposal_period_found: bool,
}

/// Deterministic preparation result before an external FinalChain/sortition lookup.
pub(crate) enum DagProposerFinalChainFactsPreparation {
    Snapshot(DagProposerFinalChainFactsSnapshot),
    Step(Box<DagProposerSessionStep>),
}

/// Next deterministic external boundary requested by a proposer cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DagProposerSessionAction {
    CollectFinalChainFacts,
    PackTransactions,
    StartVdf,
    CancelVdf,
    StaleProofSleep,
    SignBlock,
    AddBlock,
    Complete,
}

/// Owned proposer instruction snapshot returned by native application tasks.
///
/// The snapshot contains only executor-facing facts. It deliberately excludes
/// wallet secrets, retry keys, internal observations, and mutable cursor state.
/// Active snapshots do not advance a cursor. Terminal snapshots are returned
/// only after their retry effects have been published and the cursor removed.
#[derive(Clone, Debug)]
pub struct DagProposerSessionStep {
    /// Native cursor status: active, complete, or invalid report.
    pub status: u8,
    /// Next external boundary selected by the proposer state machine.
    pub action: DagProposerSessionAction,
    /// Stable protocol reason code.
    pub reason_code: u32,
    /// Final proposer return value when the step is terminal.
    pub return_value: bool,
    /// Whether the retained retry cursor was updated.
    pub update_retry_state: bool,
    /// Next last-proposed level for retry publication.
    pub next_last_propose_level: u64,
    /// Next retry count for retry publication.
    pub next_retry_count: u64,
    /// Pivot captured by the proposer attempt.
    pub frontier_pivot: H256,
    /// Candidate proposal level.
    pub proposal_level: u64,
    /// Candidate proposal period.
    pub proposal_period: u64,
    /// Last finalized period used by the attempt.
    pub last_finalized_period: u64,
    /// VRF input passed to the retained external executor.
    pub vrf_input: Vec<u8>,
    /// Sender-eligible vote count.
    pub vote_count: u64,
    /// VDF-sortition maximum vote count.
    pub max_vote_count: u64,
    /// Selected VDF difficulty.
    pub vdf_difficulty: u16,
    /// Exact historical sortition parameters retained by the cursor.
    pub sortition_params: SortitionParams,
    /// Whether the proposal is stale under native policy.
    pub vdf_stale: bool,
    /// Whether the attempt is an old proposal.
    pub old_proposal: bool,
    /// Canonical VDF message for the proof executor.
    pub vdf_message: Vec<u8>,
    /// Selected transaction hashes in proposal order.
    pub selected_transaction_hashes: Vec<H256>,
    /// Selected canonical transaction payloads exposed only at add-block.
    pub selected_transactions: Vec<TransactionPackingSelection>,
    /// Hash passed to the retained signing executor.
    pub signing_hash: H256,
    /// Canonical signed block intent exposed only at add-block.
    pub signed_intent: Option<DagProposerSignedBlockIntent>,
    /// Whether successful add-block execution records the proposed identity.
    pub record_proposed_block: bool,
    /// Stable error identifier for invalid or failed reports.
    pub error_code: String,
}

/// Transaction-pressure snapshot retained by a proposer cursor.
#[derive(Clone, Copy)]
pub struct DagProposerTransactionObservation {
    pub transaction_pool_size: u64,
    pub non_finalized_transaction_count: u64,
}

/// DAG/frontier snapshot retained for cursor revalidation.
#[derive(Clone)]
pub struct DagProposerObservation {
    pub frontier: DagProposerFrontierFacts,
    pub proposal_period_found: bool,
    pub proposal_period: u64,
    pub period_block_hash_found: bool,
    pub period_block_hash: H256,
    pub fingerprint: [u8; 32],
}

/// Ordered native cursor for one DAG proposal attempt.
pub struct DagProposerSession {
    pub action: DagProposerSessionAction,
    pub begin_input: DagProposerSessionBeginInput,
    pub transaction_observation: DagProposerTransactionObservation,
    pub observation: DagProposerObservation,
    pub attempt: DagProposerAttemptPlan,
    pub retry_key: [u8; 32],
    pub minimum_vdf_difficulty: u16,
    pub sortition_params: SortitionParams,
    pub status: u8,
    pub reason_code: u32,
    pub return_value: bool,
    pub update_retry_state: bool,
    pub next_last_propose_level: u64,
    pub next_retry_count: u64,
    pub record_proposed_block: bool,
    pub vdf_message: Vec<u8>,
    pub selected_transaction_hashes: Vec<H256>,
    pub transaction_gas_estimations: Vec<u64>,
    pub selected_transactions: Vec<TransactionPackingSelection>,
    pub vdf_rlp: Vec<u8>,
    pub unsigned_intent: Option<DagProposerUnsignedBlockIntent>,
    pub signed_intent: Option<DagProposerSignedBlockIntent>,
    pub error_code: String,
}

/// Durable retry cursor for one proposer wallet.
pub struct DagProposerRetryState {
    pub last_propose_level: u64,
    pub retry_count: u64,
    pub max_retry_count: u64,
}

/// Transaction payload retained while an add-block cursor is prepared.
#[derive(Clone)]
pub struct DagAddBlockPreparedTransaction {
    pub input_index: u64,
    pub hash: H256,
    pub trx_rlp: Vec<u8>,
    pub transaction_nonce: [u8; 32],
}

/// Copyable add-block effects retained across an unlocked account query.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DagAddBlockStoredPlan {
    pub accepted: bool,
    pub persist_transactions: bool,
    pub persist_block: bool,
    pub add_to_graph: bool,
    pub emit_verified: bool,
    pub gossip: bool,
    pub proposed: bool,
}

/// Pending cursor for one composed accepted-DAG transition.
#[derive(Clone)]
pub struct DagAddBlockSession {
    pub cursor_id: u64,
    pub block: DagManagerBlock,
    pub block_rlp: Vec<u8>,
    pub save: bool,
    pub proposed: bool,
    pub transactions: Vec<DagAddBlockPreparedTransaction>,
    pub plan: DagAddBlockStoredPlan,
}

/// Complete mutable state serialized by [`DagService`].
///
/// Public fields are a temporary CRW-12 bridge escape hatch. Callers must hold
/// [`DagServiceGuard`] and may not retain a reference across an external
/// executor, callback, sleep, thread handoff, or CXX return.
pub struct DagServiceState {
    pub state: DagManagerState,
    pub storage: Arc<Storage>,
    pub next_proposer_session_id: u64,
    pub next_verify_block_session_id: u64,
    pub next_add_block_session_id: u64,
    pub proposer_sessions: BTreeMap<u64, DagProposerSession>,
    pub proposer_retry_states: BTreeMap<[u8; 32], DagProposerRetryState>,
    pub verify_block_session: Option<DagVerifyBlockSession>,
    pub pending_add_block: Option<DagAddBlockSession>,
}

/// Committed native DAG finalization facts consumed by the application root.
pub(crate) struct DagFinalizationCommit {
    pub finalized_count: usize,
    pub expired_hashes: Vec<H256>,
    pub remove_transaction_hashes: Vec<H256>,
}

impl DagServiceState {
    fn restore(storage: Arc<Storage>, config: DagServiceConfig) -> Result<Self> {
        let mut state = Self {
            state: DagManagerState::new(config.genesis_hash, config.dag_expiry_limit)?,
            storage,
            next_proposer_session_id: 1,
            next_verify_block_session_id: 1,
            next_add_block_session_id: 1,
            proposer_sessions: BTreeMap::new(),
            proposer_retry_states: BTreeMap::new(),
            verify_block_session: None,
            pending_add_block: None,
        };
        state.restore_graph()?;
        ensure_proposal_period_mapping(state.storage.as_ref(), config.max_levels_per_period, 0)?;
        Ok(state)
    }

    fn restore_graph(&mut self) -> Result<()> {
        let pbft_restore = restore_pbft_chain_from_storage(self.storage.as_ref())
            .context("DAG_RUNTIME_RESTORE_PBFT_HEAD")?;
        let stored_anchor = pbft_restore.head.last_non_null_pbft_dag_anchor_hash;
        let anchor = if stored_anchor == H256::zero() {
            self.state.anchor()
        } else {
            stored_anchor
        };
        let anchor_level = if stored_anchor == H256::zero() {
            0
        } else {
            self.storage
                .dag()
                .by_hash(anchor)
                .with_context(|| format!("DAG_RUNTIME_RESTORE_ANCHOR_BLOCK: {anchor:?}"))?
                .level
        };

        let mut non_finalized_blocks = Vec::new();
        for (_level, blocks) in self
            .storage
            .dag()
            .non_finalized()
            .context("DAG_RUNTIME_RESTORE_NON_FINALIZED_BLOCKS")?
        {
            for block_rlp in blocks {
                non_finalized_blocks.push(
                    dag_manager_block_from_rlp(&block_rlp)
                        .context("DAG_RUNTIME_RESTORE_NON_FINALIZED_BLOCK_DECODE")?,
                );
            }
        }

        let max_level = non_finalized_blocks
            .iter()
            .map(|block| block.level)
            .chain((stored_anchor != H256::zero()).then_some(anchor_level))
            .max()
            .unwrap_or(0);
        let non_finalized_min_difficulty = non_finalized_blocks
            .iter()
            .map(|block| block.difficulty)
            .min()
            .unwrap_or(u32::MAX);
        let dag_expiry_level = max_level.saturating_sub(u64::from(self.state.dag_expiry_limit()));

        self.state
            .rebuild_from_snapshot(DagManagerSnapshot {
                old_anchor: anchor,
                anchor,
                anchor_level,
                period: pbft_restore.head.size,
                max_level,
                dag_expiry_level,
                non_finalized_min_difficulty,
                non_finalized_blocks,
            })
            .context("DAG_RUNTIME_RESTORE_REBUILD")
    }

    /// Plans one add-block transition from live graph and native storage facts.
    pub(crate) fn plan_add_block(
        &self,
        block: &DagManagerBlock,
        save: bool,
        proposed: bool,
    ) -> Result<DagAddBlockEffectPlan> {
        let block_in_state = self.state.has_vertex(block.hash);
        let block_in_storage = dag_block_exists_in_storage(self.storage.as_ref(), block.hash)
            .context("DAG_RUNTIME_ADD_BLOCK_EXISTS")?;
        let block_exists = if save {
            block_in_storage
        } else {
            block_in_state || block_in_storage
        };
        let pivot_tips = if save
            && !block_in_state
            && !block_exists
            && block.level >= self.state.dag_expiry_level()
        {
            let pivot = self.reference_metadata(block.pivot)?;
            let tips = block
                .tips
                .iter()
                .map(|tip| self.reference_metadata(*tip))
                .collect::<Result<Vec<_>>>()?;
            validate_pivot_tips_metadata(block.level, pivot, &tips)
        } else {
            crate::dag::DagPivotTipsValidation {
                ok: true,
                expected_level: block.level,
                level_matches: true,
                missing_references: Vec::new(),
            }
        };
        let mut plan = plan_dag_add_block_effects(DagAddBlockEffectInput {
            save,
            proposed,
            block_exists,
            block_level: block.level,
            dag_expiry_level: self.state.dag_expiry_level(),
            references_available: pivot_tips.ok,
            missing_references: pivot_tips.missing_references,
        });
        if save && block_in_state && !block_in_storage && plan.accepted && !plan.duplicate {
            plan.add_to_graph = false;
            plan.emit_verified = false;
            plan.gossip = false;
        }
        Ok(plan)
    }

    /// Reads canonical persisted DAG counters from the shared storage owner.
    pub(crate) fn persistence_counters(&self) -> Result<DagPersistenceCounters> {
        dag_persistence_counters_from_storage(self.storage.as_ref())
    }

    /// Applies a finalized order through candidate state and one Rust storage batch.
    ///
    /// The candidate DAG state is published only after all cleanup facts are
    /// preflighted and the durable counter, DAG-row, and transaction-row batch
    /// commits. Empty-anchor periods advance without requiring a stored block.
    pub(crate) fn apply_finalized_order(
        &mut self,
        new_anchor: H256,
        new_period: u64,
        finalized_order: Vec<H256>,
    ) -> Result<DagFinalizationCommit> {
        let mut candidate_state = self.state.clone();
        let plan = if new_anchor == H256::zero() {
            candidate_state
                .advance_empty_period(new_period)
                .context("DAG_RUNTIME_ADVANCE_EMPTY_PERIOD")?;
            DagManagerFinalizationPlan {
                previous_period: self.state.period(),
                new_period,
                previous_anchor: self.state.anchor(),
                current_anchor: self.state.anchor(),
                finalized_count: 0,
                dag_expiry_level: candidate_state.dag_expiry_level(),
                counter_update_hashes: Vec::new(),
                expired_hashes: Vec::new(),
                remaining_hashes: candidate_state
                    .non_finalized_blocks()
                    .values()
                    .flatten()
                    .copied()
                    .collect(),
            }
        } else {
            let anchor_level = self
                .storage
                .dag()
                .by_hash(new_anchor)
                .with_context(|| format!("DAG_RUNTIME_FINALIZATION_ANCHOR_BLOCK: {new_anchor:?}"))?
                .level;
            candidate_state
                .set_finalized_order(new_anchor, new_period, &finalized_order, anchor_level)
                .context("DAG_RUNTIME_SET_FINALIZED_ORDER")?
        };
        let cleanup = apply_finalization_cleanup_from_storage(
            self.storage.as_ref(),
            &plan.counter_update_hashes,
            &plan.expired_hashes,
            &plan.remaining_hashes,
        )
        .context("DAG_RUNTIME_FINALIZATION_STORAGE_APPLY")?;
        self.state = candidate_state;
        Ok(DagFinalizationCommit {
            finalized_count: plan.finalized_count,
            expired_hashes: cleanup.expired_hashes,
            remove_transaction_hashes: cleanup.remove_transaction_hashes,
        })
    }

    /// Snapshots pack-shaper facts for an active proposer session.
    pub(crate) fn proposer_pack_parameters(
        &self,
        session_id: u64,
    ) -> Result<DagProposerPackParameters> {
        let session = self
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

    /// Applies transaction packing output and advances proposer control flow.
    pub(crate) fn apply_proposer_pack(
        &mut self,
        session_id: u64,
        network_throttled: bool,
        selected_transactions: Vec<TransactionPackingSelection>,
    ) -> Result<DagProposerSessionStep> {
        let session = self
            .proposer_sessions
            .get_mut(&session_id)
            .context("DAG_PROPOSER_PACK_SESSION_NOT_ACTIVE")?;
        ensure!(
            matches!(session.action, DagProposerSessionAction::PackTransactions),
            "DAG_PROPOSER_PACK_SESSION_WRONG_STAGE"
        );
        let post_pack = plan_dag_proposer_post_pack(DagProposerPostPackInput {
            proposal_level: session.attempt.proposal_level,
            network_throttled,
            packed_transaction_count: selected_transactions.len() as u64,
        });
        session.reason_code = post_pack.reason_code;
        session.update_retry_state = post_pack.update_retry_state;
        session.next_last_propose_level = post_pack.next_last_propose_level;
        session.next_retry_count = post_pack.next_retry_count;

        if post_pack.action != DAG_PROPOSER_ACTION_CONTINUE {
            session.action = DagProposerSessionAction::Complete;
            session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
            session.return_value = false;
        } else {
            session.selected_transaction_hashes = selected_transactions
                .iter()
                .map(|selected| selected.hash)
                .collect();
            session.transaction_gas_estimations = selected_transactions
                .iter()
                .map(|selected| selected.gas_used)
                .collect();
            session.vdf_message = construct_dag_vdf_message(
                session.attempt.frontier.pivot,
                &session.selected_transaction_hashes,
            );
            session.selected_transactions = selected_transactions;
            session.action = DagProposerSessionAction::StartVdf;
            session.status = DAG_PROPOSER_SESSION_STATUS_ACTIVE;
        }
        self.finish_proposer_session_step(session_id)
    }

    /// Opens a runtime-owned proposer cursor for one attempt.
    pub(crate) fn begin_proposer_session(
        &mut self,
        input: DagProposerSessionBeginInput,
        transaction_observation: DagProposerTransactionObservation,
    ) -> Result<u64> {
        let retry_key = input.wallet_vrf_public_key;
        let observation = self.proposer_observation()?;
        let attempt = placeholder_attempt(&observation, &input);
        let action = if observation.proposal_period_found {
            DagProposerSessionAction::CollectFinalChainFacts
        } else {
            DagProposerSessionAction::Complete
        };
        let status = if matches!(action, DagProposerSessionAction::Complete) {
            DAG_PROPOSER_SESSION_STATUS_COMPLETE
        } else {
            DAG_PROPOSER_SESSION_STATUS_ACTIVE
        };
        let session_id = self.next_proposer_session_id;
        self.next_proposer_session_id = self.next_proposer_session_id.saturating_add(1).max(1);
        ensure!(
            !self.proposer_sessions.contains_key(&session_id),
            "DAG_PROPOSER_SESSION_ID_COLLISION"
        );
        self.proposer_sessions.insert(
            session_id,
            DagProposerSession {
                action,
                status,
                begin_input: input,
                transaction_observation,
                observation,
                retry_key,
                reason_code: if attempt.proposal_period_found {
                    crate::dag::DAG_PROPOSER_REASON_OK
                } else {
                    crate::dag::DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD
                },
                return_value: false,
                update_retry_state: attempt.update_retry_state,
                next_last_propose_level: attempt.next_last_propose_level,
                next_retry_count: attempt.next_retry_count,
                record_proposed_block: false,
                minimum_vdf_difficulty: 0,
                sortition_params: empty_sortition_params(),
                vdf_message: Vec::new(),
                selected_transaction_hashes: Vec::new(),
                transaction_gas_estimations: Vec::new(),
                selected_transactions: Vec::new(),
                vdf_rlp: Vec::new(),
                unsigned_intent: None,
                signed_intent: None,
                error_code: String::new(),
                attempt,
            },
        );
        Ok(session_id)
    }

    /// Returns the current requested action for a proposer cursor.
    pub(crate) fn proposer_session_next(&mut self, session_id: u64) -> DagProposerSessionStep {
        let Some(session) = self.proposer_sessions.get(&session_id) else {
            return proposer_session_not_started_step();
        };
        let step = proposer_session_step(session);
        finish_proposer_session_step(self, session_id, step)
    }

    /// Validates and snapshots a proposer cursor before external chain/sortition facts.
    pub(crate) fn prepare_proposer_final_chain_facts(
        &mut self,
        session_id: u64,
    ) -> DagProposerFinalChainFactsPreparation {
        let Some(session) = self.proposer_sessions.get(&session_id) else {
            return DagProposerFinalChainFactsPreparation::Step(
                proposer_session_not_started_step_boxed(),
            );
        };
        if !matches!(
            session.action,
            DagProposerSessionAction::CollectFinalChainFacts
        ) {
            let session = self
                .proposer_sessions
                .get_mut(&session_id)
                .expect("session still exists");
            let step = invalid_dag_proposer_report(
                session,
                "DAG_PROPOSER_SESSION_UNEXPECTED_FINAL_CHAIN_FACTS_REPORT",
            );
            return DagProposerFinalChainFactsPreparation::Step(Box::new(
                finish_proposer_session_step(self, session_id, step),
            ));
        }
        DagProposerFinalChainFactsPreparation::Snapshot(DagProposerFinalChainFactsSnapshot {
            session_id,
            fingerprint: session.observation.fingerprint,
            proposal_period: session.observation.proposal_period,
            proposer_address: session.begin_input.proposer_address,
            proposal_period_found: session.observation.proposal_period_found,
        })
    }

    /// Removes only the exact proposer cursor that owned a failed composed lookup.
    pub(crate) fn cleanup_proposer_final_chain_facts(
        &mut self,
        snapshot: &DagProposerFinalChainFactsSnapshot,
    ) -> bool {
        if self
            .proposer_sessions
            .get(&snapshot.session_id)
            .is_some_and(|session| proposer_final_chain_snapshot_matches(session, snapshot))
        {
            self.proposer_sessions.remove(&snapshot.session_id);
            return true;
        }
        false
    }

    /// Revalidates and applies FinalChain/sortition facts into one proposer cursor.
    pub(crate) fn apply_proposer_final_chain_facts(
        &mut self,
        snapshot: &DagProposerFinalChainFactsSnapshot,
        last_finalized_period: u64,
        authorization_facts: DagDposAuthorizationFacts,
        sortition_params: SortitionParams,
        initially_loaded_params: SortitionParams,
    ) -> Result<DagProposerSessionStep> {
        let Some(session) = self.proposer_sessions.get(&snapshot.session_id) else {
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

        let current = match self.proposer_observation() {
            Ok(current) => current,
            Err(error) => {
                self.proposer_sessions.remove(&snapshot.session_id);
                return Err(error);
            }
        };
        if current.fingerprint != snapshot.fingerprint {
            let session = self
                .proposer_sessions
                .get_mut(&snapshot.session_id)
                .expect("snapshot still live");
            session.action = DagProposerSessionAction::Complete;
            session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
            session.reason_code = crate::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION;
            session.error_code = "DAG_PROPOSER_SESSION_STALE_OBSERVATION".to_owned();
            let step = proposer_session_step(session);
            return Ok(finish_proposer_session_step(
                self,
                snapshot.session_id,
                step,
            ));
        }

        let session = &self.proposer_sessions[&snapshot.session_id];
        let retry = self.proposer_retry_states.get(&session.retry_key);
        let last_propose_level = retry.map_or(0, |state| state.last_propose_level);
        let retry_count = retry.map_or(0, |state| state.retry_count);
        let attempt_input = domain_attempt_input(
            &session.begin_input,
            session.transaction_observation,
            &session.observation,
            last_finalized_period,
            authorization_facts,
            sortition_params,
            last_propose_level,
            retry_count,
        );
        let minimum_vdf_difficulty = attempt_input.sortition_params.vdf.difficulty_min;
        let attempt = match plan_dag_proposer_attempt(attempt_input) {
            Ok(attempt) => attempt,
            Err(error) => {
                self.proposer_sessions.remove(&snapshot.session_id);
                return Err(error);
            }
        };
        let action = if attempt.action == DAG_PROPOSER_ACTION_CONTINUE {
            DagProposerSessionAction::PackTransactions
        } else {
            DagProposerSessionAction::Complete
        };
        let session = self
            .proposer_sessions
            .get_mut(&snapshot.session_id)
            .expect("snapshot still live");
        self.proposer_retry_states
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
        let step = proposer_session_step(session);
        Ok(finish_proposer_session_step(
            self,
            snapshot.session_id,
            step,
        ))
    }

    /// Polls proposer VDF work and triggers cancellation when the frontier advanced.
    pub(crate) fn proposer_session_poll_vdf(&mut self, session_id: u64) -> DagProposerSessionStep {
        let latest_proposal_level = self.state.proposer_frontier_facts().propose_level;
        let step = {
            let Some(session) = self.proposer_sessions.get_mut(&session_id) else {
                return proposer_session_not_started_step();
            };
            if !matches!(session.action, DagProposerSessionAction::StartVdf) {
                invalid_dag_proposer_report(
                    session,
                    "DAG_PROPOSER_SESSION_UNEXPECTED_VDF_WAIT_REPORT",
                )
            } else {
                let wait = plan_dag_proposer_vdf_wait(DagProposerVdfWaitInput {
                    proposal_level: session.attempt.proposal_level,
                    latest_proposal_level,
                    vdf_difficulty: session.attempt.vdf_difficulty,
                    minimum_vdf_difficulty: session.minimum_vdf_difficulty,
                });
                if !wait.cancel_in_flight_proof {
                    proposer_session_step(session)
                } else {
                    let retry = plan_dag_proposer_retry_reset(DagProposerRetryResetInput {
                        proposal_level: session.attempt.proposal_level,
                    });
                    let mut step = proposer_session_step(session);
                    session.action = DagProposerSessionAction::CancelVdf;
                    session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
                    step.action = DagProposerSessionAction::CancelVdf;
                    step.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
                    step.return_value = true;
                    step.update_retry_state = retry.update_retry_state;
                    step.next_last_propose_level = retry.next_last_propose_level;
                    step.next_retry_count = retry.next_retry_count;
                    step
                }
            }
        };
        finish_proposer_session_step(self, session_id, step)
    }

    /// Applies VDF proof completion to a proposer cursor.
    pub(crate) fn report_proposer_vdf_proof(
        &mut self,
        session_id: u64,
        report: DagProposerVdfProofReport,
    ) -> Result<DagProposerSessionStep> {
        let Some(session) = self.proposer_sessions.get_mut(&session_id) else {
            return Ok(proposer_session_not_started_step());
        };
        if !matches!(session.action, DagProposerSessionAction::StartVdf) {
            let step = invalid_dag_proposer_report(
                session,
                "DAG_PROPOSER_SESSION_UNEXPECTED_VDF_PROOF_REPORT",
            );
            return Ok(finish_proposer_session_step(self, session_id, step));
        }
        if !report.proof_ok {
            let step =
                invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_VDF_PROOF_FAILED");
            return Ok(finish_proposer_session_step(self, session_id, step));
        }
        if let Some(step) = revalidate_proposer_session_observation(self, session_id)? {
            return Ok(step);
        }
        if self
            .proposer_sessions
            .get(&session_id)
            .expect("session still live")
            .attempt
            .vdf_stale
        {
            let session = self
                .proposer_sessions
                .get_mut(&session_id)
                .expect("session still live");
            session.vdf_rlp = report.vdf_rlp;
            session.action = DagProposerSessionAction::StaleProofSleep;
            return Ok(proposer_session_step(session));
        }
        prepare_proposer_session_signing(self, session_id, report.vdf_rlp)
    }

    /// Resumes a stale-proof cursor after compatibility sleep.
    pub(crate) fn resume_proposer_stale_proof(
        &mut self,
        session_id: u64,
    ) -> Result<DagProposerSessionStep> {
        let latest_proposal_level = self.state.proposer_frontier_facts().propose_level;
        let Some(session) = self.proposer_sessions.get_mut(&session_id) else {
            return Ok(proposer_session_not_started_step());
        };
        if !matches!(session.action, DagProposerSessionAction::StaleProofSleep) {
            let step = invalid_dag_proposer_report(
                session,
                "DAG_PROPOSER_SESSION_UNEXPECTED_STALE_PROOF_REPORT",
            );
            return Ok(finish_proposer_session_step(self, session_id, step));
        }
        if let Some(step) = revalidate_proposer_session_observation(self, session_id)? {
            return Ok(step);
        }
        let session = self
            .proposer_sessions
            .get_mut(&session_id)
            .expect("session still live");
        let stale = plan_dag_proposer_stale_proof(DagProposerStaleProofInput {
            proposal_level: session.attempt.proposal_level,
            latest_proposal_level,
        });
        session.reason_code = stale.reason_code;
        session.update_retry_state = stale.update_retry_state;
        session.next_last_propose_level = stale.next_last_propose_level;
        session.next_retry_count = stale.next_retry_count;
        if stale.action != DAG_PROPOSER_ACTION_CONTINUE {
            session.action = DagProposerSessionAction::Complete;
            session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
            session.return_value = false;
            let step = proposer_session_step(session);
            return Ok(finish_proposer_session_step(self, session_id, step));
        }
        let vdf_rlp = session.vdf_rlp.clone();
        prepare_proposer_session_signing(self, session_id, vdf_rlp)
    }

    /// Reports one recovered signature and advances to add-block.
    pub(crate) fn report_proposer_signing(
        &mut self,
        session_id: u64,
        report: DagProposerSigningReport,
    ) -> Result<DagProposerSessionStep> {
        let Some(session) = self.proposer_sessions.get_mut(&session_id) else {
            return Ok(proposer_session_not_started_step());
        };
        if !matches!(session.action, DagProposerSessionAction::SignBlock) {
            let step = invalid_dag_proposer_report(
                session,
                "DAG_PROPOSER_SESSION_UNEXPECTED_SIGNING_REPORT",
            );
            return Ok(finish_proposer_session_step(self, session_id, step));
        }
        let intent = session
            .unsigned_intent
            .clone()
            .context("DAG_PROPOSER_SIGNING_INTENT_MISSING")?;
        let proposer_address = session.begin_input.proposer_address;
        if report.signature.len() != 65 {
            self.proposer_sessions.remove(&session_id);
            anyhow::bail!("DAG_PROPOSER_SIGNATURE_INVALID_LENGTH");
        }
        let signed = match (|| -> Result<DagProposerSignedBlockIntent> {
            let signed =
                finalize_dag_proposer_signed_block_intent(DagProposerSignedBlockIntentInput {
                    intent,
                    signature: report.signature,
                })?;
            let block = DagBlock::try_from(DagBlockRlp::new(&signed.block_rlp))
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
                self.proposer_sessions.remove(&session_id);
                return Err(error);
            }
        };
        let session = self
            .proposer_sessions
            .get_mut(&session_id)
            .expect("session still live");
        session.signed_intent = Some(signed);
        session.action = DagProposerSessionAction::AddBlock;
        Ok(proposer_session_step(session))
    }

    /// Reports add-block submission and finalizes the proposer cursor.
    pub(crate) fn report_proposer_add_block(
        &mut self,
        session_id: u64,
        report: DagProposerAddBlockReport,
    ) -> DagProposerSessionStep {
        let step = {
            let Some(session) = self.proposer_sessions.get_mut(&session_id) else {
                return proposer_session_not_started_step();
            };
            if !matches!(session.action, DagProposerSessionAction::AddBlock) {
                invalid_dag_proposer_report(
                    session,
                    "DAG_PROPOSER_SESSION_UNEXPECTED_ADD_BLOCK_REPORT",
                )
            } else if ((report.accepted || report.duplicate) && report.expired)
                || (report.accepted && !report.missing_references.is_empty())
            {
                invalid_dag_proposer_report(
                    session,
                    "DAG_PROPOSER_SESSION_INVALID_ADD_BLOCK_REPORT",
                )
            } else {
                let retry = plan_dag_proposer_retry_reset(DagProposerRetryResetInput {
                    proposal_level: session.attempt.proposal_level,
                });
                session.action = DagProposerSessionAction::Complete;
                session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
                session.reason_code = if report.accepted {
                    crate::dag::DAG_PROPOSER_REASON_OK
                } else if report.expired {
                    crate::dag::DAG_PROPOSER_REASON_ADD_BLOCK_EXPIRED
                } else if !report.missing_references.is_empty() {
                    crate::dag::DAG_PROPOSER_REASON_ADD_BLOCK_MISSING_REFERENCES
                } else {
                    crate::dag::DAG_PROPOSER_REASON_ADD_BLOCK_REJECTED
                };
                session.return_value = report.accepted;
                session.update_retry_state = retry.update_retry_state;
                session.next_last_propose_level = retry.next_last_propose_level;
                session.next_retry_count = retry.next_retry_count;
                session.record_proposed_block = report.accepted;
                proposer_session_step(session)
            }
        };
        finish_proposer_session_step(self, session_id, step)
    }

    /// Reads the current proposer observation with proposal-period and fingerprint.
    pub(crate) fn proposer_observation(&self) -> Result<DagProposerObservation> {
        let frontier = self.state.proposer_frontier_facts();
        let proposal_period: DagPeriodStorageLookup =
            proposal_period_for_level_from_storage(self.storage.as_ref(), frontier.propose_level)?;
        let period_block_hash = if proposal_period.found {
            let lookup =
                period_block_hash_from_storage(self.storage.as_ref(), proposal_period.period)?;
            if !lookup.found && proposal_period.period == 0 {
                DagHashStorageLookup {
                    found: true,
                    hash: H256::zero(),
                }
            } else {
                lookup
            }
        } else {
            DagHashStorageLookup {
                found: false,
                hash: H256::zero(),
            }
        };
        let fingerprint = proposer_observation_fingerprint(
            &frontier,
            proposal_period.found,
            proposal_period.period,
            period_block_hash.found,
            period_block_hash.hash,
        );
        Ok(DagProposerObservation {
            frontier,
            proposal_period_found: proposal_period.found,
            proposal_period: proposal_period.period,
            period_block_hash_found: period_block_hash.found,
            period_block_hash: period_block_hash.hash,
            fingerprint,
        })
    }

    /// Opens a verification cursor for one [`DagManager::verifyBlock`] call.
    pub(crate) fn begin_verify_block_session(
        &mut self,
        input: DagVerifyBlockSessionInput,
    ) -> Result<()> {
        let fingerprint = input.block_hash;
        let cursor_id = self.next_verify_block_session_id;
        self.next_verify_block_session_id =
            self.next_verify_block_session_id.wrapping_add(1).max(1);
        let tips = input.tips.clone();
        let precheck = verify_precheck_from_storage(
            self.storage.as_ref(),
            DagVerifyPrecheckStorageInput {
                block_level: input.block_level,
                pivot: H256::from(input.pivot),
                tips: tips.clone(),
                dag_expiry_level: self.state.dag_expiry_level(),
            },
        )
        .context("DAG_RUNTIME_VERIFY_SESSION_PRECHECK")?;

        let expected_transactions = input.block_transaction_hashes.len() as u64;
        let action = if precheck.continue_validation {
            let block_transaction_hashes = input.block_transaction_hashes.clone();
            let supplied_transaction_hashes = input.supplied_transaction_hashes.clone();
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
            eligibility_status: DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
            error_code: String::new(),
        });
        Ok(())
    }

    /// Returns the current requested action for the verification cursor.
    pub(crate) fn verify_block_session_next(&self) -> DagVerifyBlockSessionStep {
        let Some(session) = self.verify_block_session.as_ref() else {
            return verify_block_session_not_started_step();
        };
        verify_block_session_step(session)
    }

    /// Applies resolved transaction availability and advances authorization planning.
    pub(crate) fn verify_block_session_apply_transaction_resolution(
        &mut self,
        resolved_transactions: u64,
    ) -> DagVerifyBlockSessionStep {
        let Some(session) = self.verify_block_session.as_mut() else {
            return verify_block_session_not_started_step();
        };
        if !matches!(
            session.action,
            DagVerifyBlockSessionAction::TransactionQuery(_)
        ) {
            return invalid_verify_block_report(
                session,
                "DAG_VERIFY_SESSION_UNEXPECTED_TRANSACTION_REPORT",
            );
        }

        let availability =
            validate_dag_verify_transaction_availability(DagVerifyTransactionAvailabilityInput {
                expected_transactions: session.expected_transactions,
                resolved_transactions,
            });
        if !availability.continue_validation {
            return complete_verify_block_session(session, availability.reject_code);
        }

        session.action = DagVerifyBlockSessionAction::AuthorizationFacts;
        session.generation = session.generation.wrapping_add(1).max(1);
        verify_block_session_step(session)
    }

    /// Returns the active transaction query without advancing its cursor.
    pub(crate) fn verify_block_transaction_query(&self) -> Result<DagVerifyBlockTransactionQuery> {
        let Some(session) = self.verify_block_session.as_ref() else {
            anyhow::bail!("DAG_VERIFY_SESSION_NOT_STARTED");
        };
        let DagVerifyBlockSessionAction::TransactionQuery(hashes) = &session.action else {
            anyhow::bail!("DAG_VERIFY_SESSION_UNEXPECTED_TRANSACTION_COMPLETION");
        };
        Ok(DagVerifyBlockTransactionQuery {
            cursor_id: session.cursor_id,
            proposal_period: session.proposal_period,
            hashes: hashes.clone(),
            expected_transactions: session.expected_transactions,
        })
    }

    /// Revalidates that a transaction completion still targets the active cursor.
    pub(crate) fn verify_block_session_validate_transaction_completion(
        &self,
        cursor_id: u64,
        proposal_period: u64,
    ) -> Result<DagVerifyBlockTransactionQuery> {
        let query = self.verify_block_transaction_query()?;
        ensure!(
            query.cursor_id == cursor_id,
            "DAG_VERIFY_SESSION_TRANSACTION_CURSOR_MISMATCH"
        );
        ensure!(
            query.proposal_period == proposal_period,
            "DAG_VERIFY_SESSION_TRANSACTION_PERIOD_MISMATCH"
        );
        Ok(query)
    }

    /// Snapshots authorization facts for the exact active verify cursor.
    pub(crate) fn prepare_verify_block_authorization(
        &mut self,
    ) -> DagVerifyBlockAuthorizationPreparation {
        let Some(session) = self.verify_block_session.as_mut() else {
            return DagVerifyBlockAuthorizationPreparation::Step(
                verify_block_session_not_started_step(),
            );
        };
        if !matches!(
            session.action,
            DagVerifyBlockSessionAction::AuthorizationFacts
        ) {
            return DagVerifyBlockAuthorizationPreparation::Step(invalid_verify_block_report(
                session,
                "DAG_VERIFY_SESSION_UNEXPECTED_AUTHORIZATION_REPORT",
            ));
        }

        DagVerifyBlockAuthorizationPreparation::Snapshot(DagVerifyBlockAuthorizationSnapshot {
            cursor_id: session.cursor_id,
            fingerprint: session.fingerprint,
            generation: session.generation,
            proposal_period: session.proposal_period,
            block_rlp: session.block_rlp.clone(),
        })
    }

    /// Removes only the exact authorization cursor that requested facts.
    pub(crate) fn cleanup_verify_block_authorization(
        &mut self,
        snapshot: &DagVerifyBlockAuthorizationSnapshot,
    ) -> bool {
        let matches = self
            .verify_block_session
            .as_ref()
            .is_some_and(|session| verify_block_authorization_snapshot_matches(session, snapshot));
        if matches {
            self.verify_block_session = None;
        }
        matches
    }

    /// Revalidates and applies FinalChain DPoS authorization facts.
    pub(crate) fn apply_verify_block_authorization(
        &mut self,
        snapshot: &DagVerifyBlockAuthorizationSnapshot,
        facts: DagDposAuthorizationFacts,
    ) -> Result<DagVerifyBlockSessionStep> {
        let session = self
            .verify_block_session
            .as_mut()
            .context("DAG_VERIFY_SESSION_NOT_STARTED")?;
        ensure!(
            verify_block_authorization_snapshot_matches(session, snapshot),
            "DAG_VERIFY_SESSION_AUTHORIZATION_CURSOR_MISMATCH"
        );

        session.sender_eligible_vote_count = facts.sender_eligible_vote_count;
        session.vdf_sortition_max_vote_count = facts.vdf_sortition_max_vote_count;
        session.eligibility_status = facts.eligibility_status;

        let dpos_status = if facts.eligibility_status == DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
        {
            DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
        } else {
            DAG_VERIFY_DPOS_STATUS_NOT_CHECKED
        };
        let decision = decide_dag_verify_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: facts.vrf_key_found && facts.vrf_key.is_some(),
            sender_eligible_vote_count: facts.sender_eligible_vote_count,
            vdf_sortition_max_vote_count: facts.vdf_sortition_max_vote_count,
            vdf_status: DAG_VERIFY_VDF_STATUS_NOT_CHECKED,
            dpos_status,
        });
        if !decision.continue_validation {
            return Ok(complete_verify_block_session(session, decision.reject_code));
        }

        session.action = DagVerifyBlockSessionAction::VdfSortition {
            vote_count: decision.vote_count,
            max_vote_count: decision.max_vote_count,
            vrf_public_key: facts
                .vrf_key
                .context("DAG_VERIFY_SESSION_AUTHORIZATION_VRF_KEY_MISSING")?,
        };
        session.generation = session.generation.wrapping_add(1).max(1);
        Ok(verify_block_session_step(session))
    }

    /// Takes a snapshot of the active VDF authorization cursor.
    pub(crate) fn snapshot_verify_block_vdf(
        &self,
        cursor_id: u64,
    ) -> Result<DagVerifyBlockVdfSnapshot> {
        let session = self
            .verify_block_session
            .as_ref()
            .context("DAG_VERIFY_SESSION_NOT_STARTED")?;
        ensure!(
            session.cursor_id == cursor_id,
            "DAG_VERIFY_SESSION_VDF_CURSOR_MISMATCH"
        );
        let DagVerifyBlockSessionAction::VdfSortition {
            vote_count,
            max_vote_count,
            vrf_public_key,
        } = &session.action
        else {
            anyhow::bail!("DAG_VERIFY_SESSION_UNEXPECTED_VDF_ACTION");
        };
        Ok(DagVerifyBlockVdfSnapshot {
            cursor_id: session.cursor_id,
            fingerprint: session.fingerprint,
            generation: session.generation,
            proposal_period: session.proposal_period,
            vote_count: *vote_count,
            max_vote_count: *max_vote_count,
            vrf_public_key: *vrf_public_key,
        })
    }

    /// Revalidates a VDF snapshot and applies the completion result.
    pub(crate) fn complete_verify_block_vdf(
        &mut self,
        snapshot: &DagVerifyBlockVdfSnapshot,
        vdf_status: u8,
    ) -> Result<DagVerifyBlockSessionStep> {
        {
            let session = self
                .verify_block_session
                .as_ref()
                .context("DAG_VERIFY_SESSION_NOT_STARTED")?;
            ensure!(
                session.cursor_id == snapshot.cursor_id,
                "DAG_VERIFY_SESSION_VDF_CURSOR_MISMATCH"
            );
            ensure!(
                session.fingerprint == snapshot.fingerprint,
                "DAG_VERIFY_SESSION_VDF_FINGERPRINT_MISMATCH"
            );
            ensure!(
                session.generation == snapshot.generation,
                "DAG_VERIFY_SESSION_VDF_GENERATION_MISMATCH"
            );
            let DagVerifyBlockSessionAction::VdfSortition {
                vote_count,
                max_vote_count,
                vrf_public_key,
            } = &session.action
            else {
                anyhow::bail!("DAG_VERIFY_SESSION_UNEXPECTED_VDF_ACTION");
            };
            ensure!(
                *vote_count == snapshot.vote_count
                    && *max_vote_count == snapshot.max_vote_count
                    && *vrf_public_key == snapshot.vrf_public_key,
                "DAG_VERIFY_SESSION_VDF_ACTION_MISMATCH"
            );
        };
        let Some(session) = self.verify_block_session.as_mut() else {
            return Ok(verify_block_session_not_started_step());
        };
        Ok(Self::apply_verify_block_vdf_status(session, vdf_status))
    }

    fn apply_verify_block_vdf_status(
        session: &mut DagVerifyBlockSession,
        vdf_status: u8,
    ) -> DagVerifyBlockSessionStep {
        if !matches!(
            session.action,
            DagVerifyBlockSessionAction::VdfSortition { .. }
        ) {
            return invalid_verify_block_report(
                session,
                "DAG_VERIFY_SESSION_UNEXPECTED_VDF_REPORT",
            );
        }

        let dpos_status =
            if session.eligibility_status == DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE {
                DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
            } else {
                DAG_VERIFY_DPOS_STATUS_NOT_CHECKED
            };
        let vdf_decision = decide_dag_verify_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: true,
            sender_eligible_vote_count: session.sender_eligible_vote_count,
            vdf_sortition_max_vote_count: session.vdf_sortition_max_vote_count,
            vdf_status,
            dpos_status,
        });
        if !vdf_decision.continue_validation {
            return complete_verify_block_session(session, vdf_decision.reject_code);
        }

        let dpos_decision = decide_dag_verify_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: true,
            sender_eligible_vote_count: session.sender_eligible_vote_count,
            vdf_sortition_max_vote_count: session.vdf_sortition_max_vote_count,
            vdf_status: DAG_VERIFY_VDF_STATUS_VALID,
            dpos_status: session.eligibility_status,
        });
        if !dpos_decision.continue_validation {
            return complete_verify_block_session(session, dpos_decision.reject_code);
        }

        session.action = DagVerifyBlockSessionAction::Gas;
        session.generation = session.generation.wrapping_add(1).max(1);
        verify_block_session_step(session)
    }

    /// Reports block and tip-gas facts from public EVM estimation into the verify cursor.
    pub(crate) fn verify_block_session_report_gas(
        &mut self,
        report: DagVerifyBlockGasReport,
    ) -> Result<DagVerifyBlockSessionStep> {
        let Some(session) = self.verify_block_session.as_ref() else {
            return Ok(verify_block_session_not_started_step());
        };
        if !matches!(session.action, DagVerifyBlockSessionAction::Gas) {
            let Some(session) = self.verify_block_session.as_mut() else {
                return Ok(verify_block_session_not_started_step());
            };
            return Ok(invalid_verify_block_report(
                session,
                "DAG_VERIFY_SESSION_UNEXPECTED_GAS_REPORT",
            ));
        }

        let tips = session.tips.clone();
        let needs_tip_gas = report.dag_gas_limit == 0
            || (tips.len() as u64).saturating_add(1) > report.pbft_gas_limit / report.dag_gas_limit;
        let tip_gas_estimations = if needs_tip_gas {
            self.verify_block_session_tip_gas_estimations(&tips)?
        } else {
            Vec::new()
        };

        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: report.block_gas_estimation,
            estimated_transactions_weight: report.estimated_transactions_weight,
            dag_gas_limit: report.dag_gas_limit,
            pbft_gas_limit: report.pbft_gas_limit,
            tip_gas_estimations,
        });
        let Some(session) = self.verify_block_session.as_mut() else {
            return Ok(verify_block_session_not_started_step());
        };
        Ok(complete_verify_block_session(session, result.reject_code))
    }

    fn verify_block_session_tip_gas_estimations(&self, tips: &[H256]) -> Result<Vec<DagTipGas>> {
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

    /// Returns the active proposer instruction without advancing its cursor.
    pub(crate) fn proposer_session_step(&self, session_id: u64) -> Result<DagProposerSessionStep> {
        let session = self
            .proposer_sessions
            .get(&session_id)
            .context("DAG_PROPOSER_PACK_SESSION_NOT_ACTIVE")?;
        Ok(proposer_session_step(session))
    }

    fn finish_proposer_session_step(&mut self, session_id: u64) -> Result<DagProposerSessionStep> {
        let step = self.proposer_session_step(session_id)?;
        Ok(finish_proposer_session_step(self, session_id, step))
    }

    /// Removes a proposer session without retry-state mutation.
    pub(crate) fn abort_proposer_session(&mut self, session_id: u64) -> bool {
        self.proposer_sessions.remove(&session_id).is_some()
    }

    fn reference_metadata(&self, hash: H256) -> Result<DagReferenceMetadata> {
        let metadata = self.state.reference_metadata(hash);
        if metadata.found {
            return Ok(metadata);
        }
        if self
            .storage
            .dag()
            .by_hash_rlp_optional(hash)
            .context("DAG_RUNTIME_REFERENCE_STORAGE_LOOKUP")?
            .is_none()
        {
            return Ok(metadata);
        }
        let block = self
            .storage
            .dag()
            .by_hash(hash)
            .context("DAG_RUNTIME_REFERENCE_STORAGE_DECODE")?;
        Ok(DagReferenceMetadata {
            hash,
            found: true,
            level: block.level,
        })
    }
}

fn proposer_session_step(session: &DagProposerSession) -> DagProposerSessionStep {
    DagProposerSessionStep {
        status: session.status,
        action: session.action,
        reason_code: session.reason_code,
        return_value: session.return_value,
        update_retry_state: session.update_retry_state,
        next_last_propose_level: session.next_last_propose_level,
        next_retry_count: session.next_retry_count,
        frontier_pivot: session.attempt.frontier.pivot,
        proposal_level: session.attempt.proposal_level,
        proposal_period: session.attempt.proposal_period,
        last_finalized_period: session.attempt.last_finalized_period,
        vrf_input: session.attempt.vrf_input.clone(),
        vote_count: session.attempt.vote_count,
        max_vote_count: session.attempt.max_vote_count,
        vdf_difficulty: session.attempt.vdf_difficulty,
        sortition_params: session.sortition_params,
        vdf_stale: session.attempt.vdf_stale,
        old_proposal: session.attempt.old_proposal,
        vdf_message: session.vdf_message.clone(),
        selected_transaction_hashes: session.selected_transaction_hashes.clone(),
        selected_transactions: if matches!(session.action, DagProposerSessionAction::AddBlock) {
            session.selected_transactions.clone()
        } else {
            Vec::new()
        },
        signing_hash: session
            .unsigned_intent
            .as_ref()
            .map_or_else(H256::zero, |intent| intent.signing_hash),
        signed_intent: session.signed_intent.clone(),
        record_proposed_block: session.record_proposed_block,
        error_code: session.error_code.clone(),
    }
}

fn proposer_session_not_started_step() -> DagProposerSessionStep {
    DagProposerSessionStep {
        status: DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT,
        action: DagProposerSessionAction::Complete,
        reason_code: crate::dag::DAG_PROPOSER_REASON_OK,
        return_value: false,
        update_retry_state: false,
        next_last_propose_level: 0,
        next_retry_count: 0,
        frontier_pivot: H256::zero(),
        proposal_level: 0,
        proposal_period: 0,
        last_finalized_period: 0,
        vrf_input: Vec::new(),
        vote_count: 0,
        max_vote_count: 0,
        vdf_difficulty: 0,
        sortition_params: empty_sortition_params(),
        vdf_stale: false,
        old_proposal: false,
        vdf_message: Vec::new(),
        selected_transaction_hashes: Vec::new(),
        selected_transactions: Vec::new(),
        signing_hash: H256::zero(),
        signed_intent: None,
        record_proposed_block: false,
        error_code: "DAG_PROPOSER_SESSION_NOT_STARTED".to_string(),
    }
}

fn proposer_session_not_started_step_boxed() -> Box<DagProposerSessionStep> {
    Box::new(proposer_session_not_started_step())
}

fn finish_proposer_session_step(
    state: &mut DagServiceState,
    session_id: u64,
    step: DagProposerSessionStep,
) -> DagProposerSessionStep {
    if step.status == DAG_PROPOSER_SESSION_STATUS_ACTIVE {
        return step;
    }
    if step.update_retry_state
        && let Some(session) = state.proposer_sessions.get(&session_id)
        && let Some(retry_state) = state.proposer_retry_states.get_mut(&session.retry_key)
    {
        retry_state.last_propose_level = step.next_last_propose_level;
        retry_state.retry_count = step.next_retry_count;
    }
    state.proposer_sessions.remove(&session_id);
    step
}

fn invalid_dag_proposer_report(
    session: &mut DagProposerSession,
    error_code: &str,
) -> DagProposerSessionStep {
    session.action = DagProposerSessionAction::Complete;
    session.status = DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT;
    session.return_value = false;
    session.error_code = error_code.to_string();
    proposer_session_step(session)
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

fn revalidate_proposer_session_observation(
    state: &mut DagServiceState,
    session_id: u64,
) -> Result<Option<DagProposerSessionStep>> {
    let current = match state.proposer_observation() {
        Ok(current) => current,
        Err(error) => {
            state.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    if current.fingerprint == state.proposer_sessions[&session_id].observation.fingerprint {
        return Ok(None);
    }
    let session = state
        .proposer_sessions
        .get_mut(&session_id)
        .expect("session still live");
    session.action = DagProposerSessionAction::Complete;
    session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
    session.reason_code = crate::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION;
    session.return_value = false;
    session.update_retry_state = false;
    session.error_code = "DAG_PROPOSER_SESSION_STALE_OBSERVATION".to_owned();
    let step = proposer_session_step(session);
    Ok(Some(finish_proposer_session_step(state, session_id, step)))
}

fn prepare_proposer_session_signing(
    state: &mut DagServiceState,
    session_id: u64,
    vdf_rlp: Vec<u8>,
) -> Result<DagProposerSessionStep> {
    let session = &state.proposer_sessions[&session_id];
    let frontier_tips = session.observation.frontier.frontier.tips.clone();
    let transaction_gas_estimations = session.transaction_gas_estimations.clone();
    let pbft_gas_limit = session.begin_input.pbft_gas_limit;
    let dag_gas_limit = session.begin_input.dag_gas_limit;
    let max_tips = session.begin_input.max_tips;
    let pivot = session.observation.frontier.frontier.pivot;
    let proposal_level = session.attempt.proposal_level;
    let transaction_hashes = session.selected_transaction_hashes.clone();

    let prepared = (|| -> Result<DagProposerUnsignedBlockIntent> {
        let construction = plan_dag_proposer_block_construction_from_storage(
            state.storage.as_ref(),
            DagProposerStorageBlockConstructionInput {
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
            DagProposerBlockIntentInput {
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
            state.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    let session = state
        .proposer_sessions
        .get_mut(&session_id)
        .expect("session still live");
    session.vdf_rlp = intent.vdf_rlp.clone();
    session.unsigned_intent = Some(intent);
    session.action = DagProposerSessionAction::SignBlock;
    Ok(proposer_session_step(session))
}

#[allow(clippy::too_many_arguments)]
fn domain_attempt_input(
    input: &DagProposerSessionBeginInput,
    transaction_observation: DagProposerTransactionObservation,
    observation: &DagProposerObservation,
    last_finalized_period: u64,
    authorization_facts: DagDposAuthorizationFacts,
    sortition_params: SortitionParams,
    last_propose_level: u64,
    retry_count: u64,
) -> DagProposerAttemptInput {
    DagProposerAttemptInput {
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
        authorization_facts: DagDposAuthorizationFacts {
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
    input: &DagProposerSessionBeginInput,
) -> DagProposerAttemptPlan {
    DagProposerAttemptPlan {
        action: crate::dag::DAG_PROPOSER_ACTION_SKIP,
        reason_code: crate::dag::DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD,
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

fn proposer_observation_fingerprint(
    frontier: &DagProposerFrontierFacts,
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

fn empty_sortition_params() -> SortitionParams {
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

fn complete_verify_block_session(
    session: &mut DagVerifyBlockSession,
    reject_code: u32,
) -> DagVerifyBlockSessionStep {
    session.reject_code = reject_code;
    session.action = DagVerifyBlockSessionAction::Complete;
    session.generation = session.generation.wrapping_add(1).max(1);
    verify_block_session_step(session)
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

/// Native owner of DAG construction, restoration, sessions, and locking.
pub struct DagService {
    state: Mutex<DagServiceState>,
}

impl DagService {
    /// Restores all DAG state before publishing the mutex-owning service.
    pub fn restore(storage: Arc<Storage>, config: DagServiceConfig) -> Result<Self> {
        Ok(Self {
            state: Mutex::new(DagServiceState::restore(storage, config)?),
        })
    }

    /// Locks the complete DAG serialization domain.
    pub fn lock(&self) -> Result<DagServiceGuard<'_>> {
        Ok(DagServiceGuard(self.state.lock().map_err(|_| {
            anyhow!(DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED)
        })?))
    }
}

/// Exclusive short-lived guard over the native DAG runtime.
pub struct DagServiceGuard<'a>(MutexGuard<'a, DagServiceState>);

impl Deref for DagServiceGuard<'_> {
    type Target = DagServiceState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for DagServiceGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::Config;
    use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
    use rustaxa_types::pbft::PbftBlockLink;
    use rustaxa_vdf::vrf::public_key_from_secret;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn signed_pbft_block(period: u64, pivot: H256) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from_low_u64_be(10));
        block.append(&pivot);
        block.append(&H256::from_low_u64_be(12));
        block.append(&H256::from_low_u64_be(13));
        block.append(&period);
        block.append(&123u64);
        block.begin_list(0);
        block.append(&vec![0u8; 65]);
        block.out().to_vec()
    }

    fn period_data(pbft_block: &[u8]) -> Vec<u8> {
        let mut data = RlpStream::new_list(4);
        data.append_raw(pbft_block, 1);
        data.append_empty_data();
        data.append_empty_data();
        data.begin_list(0);
        data.out().to_vec()
    }

    fn seed_pbft_head(storage: &Storage, period: u64, pivot: H256) -> Result<()> {
        let pbft_block = signed_pbft_block(period, pivot);
        let pbft_link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&pbft_block))?;
        storage.period().write(period, &period_data(&pbft_block))?;
        storage
            .period()
            .write_pbft_period(pbft_link.block_hash, period)?;
        storage.pbft().write_head(
            H256::zero(),
            format!(
                r#"{{"head_hash":"0x{:064x}","size":{},"non_empty_size":{},"last_pbft_block_hash":"0x{:064x}"}}"#,
                0, period, period, pbft_link.block_hash
            )
            .as_bytes(),
        )?;
        Ok(())
    }

    fn dag_block(pivot: H256, level: u64, difficulty: u16) -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11u8; 80]);
        vdf.append(&vec![0x22u8]);
        vdf.append(&vec![0x33u8]);
        vdf.append(&difficulty);

        let mut block = RlpStream::new_list(8);
        block.append(&pivot);
        block.append(&level);
        block.append(&0u64);
        block.append(&vdf.out().to_vec());
        block.begin_list(0);
        block.begin_list(0);
        block.append(&&[0u8; 65][..]);
        block.append(&123u64);
        block.out().to_vec()
    }

    fn signed_dag_block_rlp(level: u64, tip_gas: u64) -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11u8; 80]);
        vdf.append(&vec![0x22u8]);
        vdf.append(&vec![0x33u8]);
        vdf.append(&1u16);

        let mut block = RlpStream::new_list(8);
        block.append(&H256::zero());
        block.append(&level);
        block.append(&123u64);
        block.append(&vdf.out().to_vec());
        block.begin_list(0);
        block.begin_list(0);
        block.append(&&[0u8; 65][..]);
        block.append(&tip_gas);
        block.out().to_vec()
    }

    fn begin_verify_block_session_with_mapping(
        dag: &mut DagServiceGuard<'_>,
        proposal_period: u64,
        block_level: u64,
        tips: Vec<H256>,
        block_transaction_hashes: Vec<H256>,
        supplied_transaction_hashes: Vec<H256>,
    ) -> Result<()> {
        ensure_proposal_period_mapping(&dag.storage, block_level, proposal_period)?;
        dag.begin_verify_block_session(DagVerifyBlockSessionInput {
            block_hash: [0u8; 32],
            block_level,
            pivot: [1u8; 32],
            tips,
            block_transaction_hashes,
            supplied_transaction_hashes,
            block_rlp: Vec::new(),
        })?;
        Ok(())
    }

    fn begin_verify_session_for_gas(runtime: &mut DagServiceGuard<'_>, tip: H256) -> Result<()> {
        begin_verify_block_session_with_mapping(runtime, 7, 5, vec![tip], Vec::new(), Vec::new())?;
        let auth = runtime.verify_block_session_apply_transaction_resolution(0);
        assert_eq!(
            auth.action, DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS,
            "initial verification query must advance to authorization"
        );
        let snapshot = match runtime.prepare_verify_block_authorization() {
            DagVerifyBlockAuthorizationPreparation::Snapshot(snapshot) => snapshot,
            DagVerifyBlockAuthorizationPreparation::Step(_) => {
                anyhow::bail!("verification cursor must await authorization")
            }
        };
        let step = runtime.apply_verify_block_authorization(
            &snapshot,
            DagDposAuthorizationFacts {
                vrf_key: Some([0x44; 32]),
                vrf_key_found: true,
                sender_eligible_vote_count: 11,
                vdf_sortition_max_vote_count: 33,
                eligibility_status: crate::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
            },
        )?;
        assert_eq!(step.action, DAG_VERIFY_SESSION_ACTION_VDF_SORTITION);
        let vdf_snapshot = runtime.snapshot_verify_block_vdf(step.cursor_id)?;
        let gas_step = runtime
            .complete_verify_block_vdf(&vdf_snapshot, crate::dag::DAG_VERIFY_VDF_STATUS_VALID)?;
        assert_eq!(
            gas_step.action, DAG_VERIFY_SESSION_ACTION_GAS,
            "verification cursor should advance to gas check"
        );
        Ok(())
    }

    const TEST_VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn proposer_seed_address(seed: u8) -> [u8; 20] {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("proposer seed");
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let mut hash = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&encoded.as_bytes()[1..]);
        hasher.finalize(&mut hash);
        hash[12..]
            .try_into()
            .expect("proposer address slice has fixed length")
    }

    fn sign_dag_hash(seed: u8, signing_hash: [u8; 32]) -> Vec<u8> {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("signing seed");
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&signing_hash)
            .expect("sign proposer intent");
        let mut bytes = signature.to_bytes().to_vec();
        bytes.push(recovery_id.to_byte());
        bytes
    }

    fn proposer_begin_input(vrf_key: [u8; 32]) -> DagProposerSessionBeginInput {
        DagProposerSessionBeginInput {
            max_non_finalized_transactions: 100,
            dag_expiry_level_limit: 100,
            wallet_vrf_public_key: vrf_key,
            wallet_vrf_secret: TEST_VRF_SECRET,
            proposer_address: proposer_seed_address(0x44),
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

    fn proposer_transaction_observation() -> DagProposerTransactionObservation {
        DagProposerTransactionObservation {
            transaction_pool_size: 1,
            non_finalized_transaction_count: 0,
        }
    }

    fn proposer_authorization_facts(vrf_key: [u8; 32]) -> DagDposAuthorizationFacts {
        DagDposAuthorizationFacts {
            vrf_key: Some(vrf_key),
            vrf_key_found: true,
            sender_eligible_vote_count: 10,
            vdf_sortition_max_vote_count: 20,
            eligibility_status: crate::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
        }
    }

    fn proposer_sortition_params() -> SortitionParams {
        SortitionParams {
            vrf: VrfParams {
                threshold_upper: u16::MAX,
            },
            vdf: VdfParams {
                difficulty_min: 3,
                difficulty_max: 3,
                difficulty_stale: 9,
                lambda_bound: 128,
            },
        }
    }

    fn proposer_sortition_params_stale() -> SortitionParams {
        SortitionParams {
            vrf: VrfParams {
                threshold_upper: u16::MAX,
            },
            vdf: VdfParams {
                difficulty_min: 9,
                difficulty_max: 9,
                difficulty_stale: 9,
                lambda_bound: 128,
            },
        }
    }

    fn proposer_vrf_key() -> [u8; 32] {
        public_key_from_secret(&TEST_VRF_SECRET).expect("VRF key from test secret")
    }

    fn ensure_period_mapping_for_frontier(runtime: &DagServiceState) -> u64 {
        let frontier_level = runtime.state.proposer_frontier_facts().propose_level;
        ensure_proposal_period_mapping(&runtime.storage, frontier_level, 0)
            .expect("frontier mapping should persist");
        frontier_level
    }

    fn apply_proposer_final_chain_facts(
        runtime: &mut DagServiceGuard<'_>,
        session_id: u64,
        vrf_key: [u8; 32],
        sortition_params: SortitionParams,
    ) -> Result<DagProposerSessionStep> {
        let snapshot = match runtime.prepare_proposer_final_chain_facts(session_id) {
            DagProposerFinalChainFactsPreparation::Snapshot(snapshot) => snapshot,
            DagProposerFinalChainFactsPreparation::Step(step) => return Ok(*step),
        };
        runtime.apply_proposer_final_chain_facts(
            &snapshot,
            0,
            proposer_authorization_facts(vrf_key),
            sortition_params,
            sortition_params,
        )
    }

    fn begin_proposer_vdf_session(
        runtime: &mut DagServiceGuard<'_>,
        sortition_params: SortitionParams,
        tx_hash: H256,
    ) -> Result<u64> {
        let vrf_key = proposer_vrf_key();
        ensure_period_mapping_for_frontier(&runtime);
        let session_id = runtime.begin_proposer_session(
            proposer_begin_input(vrf_key),
            proposer_transaction_observation(),
        )?;
        assert_eq!(
            runtime.proposer_session_next(session_id).action,
            DagProposerSessionAction::CollectFinalChainFacts
        );
        let attempt =
            apply_proposer_final_chain_facts(runtime, session_id, vrf_key, sortition_params)?;
        assert_eq!(
            attempt.action,
            DagProposerSessionAction::PackTransactions,
            "attempt action should request packing, got {:?} reason {:?}",
            attempt.action,
            attempt.reason_code
        );
        assert_eq!(
            runtime
                .apply_proposer_pack(
                    session_id,
                    false,
                    vec![TransactionPackingSelection {
                        hash: tx_hash,
                        gas_used: 100,
                        transaction_rlp: vec![tx_hash.as_bytes()[0]],
                    }],
                )?
                .action,
            runtime.proposer_session_next(session_id).action,
            "pack should advance"
        );
        Ok(session_id)
    }

    fn add_frontier_blocks(runtime: &mut DagServiceGuard<'_>) -> Result<()> {
        runtime.state.add_block(DagManagerBlock {
            hash: H256::from([2u8; 32]),
            pivot: H256::from([1u8; 32]),
            tips: Vec::new(),
            level: 2,
            difficulty: 100,
        })?;
        runtime.state.add_block(DagManagerBlock {
            hash: H256::from([3u8; 32]),
            pivot: H256::from([1u8; 32]),
            tips: vec![H256::from([2u8; 32])],
            level: 3,
            difficulty: 80,
        })?;
        Ok(())
    }

    #[test]
    fn fresh_restore_publishes_complete_empty_session_owner() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_fresh");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        let runtime = service.lock()?;
        assert!(Arc::ptr_eq(&runtime.storage, &storage));
        assert_eq!(runtime.state.anchor(), H256::repeat_byte(1));
        assert_eq!(runtime.next_proposer_session_id, 1);
        assert_eq!(runtime.next_verify_block_session_id, 1);
        assert_eq!(runtime.next_add_block_session_id, 1);
        assert!(runtime.proposer_sessions.is_empty());
        assert!(runtime.proposer_retry_states.is_empty());
        assert!(runtime.verify_block_session.is_none());
        assert!(runtime.pending_add_block.is_none());
        drop(runtime);
        drop(service);
        let restarted = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        assert_eq!(restarted.lock()?.state.anchor(), H256::repeat_byte(1));
        drop(restarted);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn poisoned_lock_returns_stable_identifier() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_poison");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = Arc::new(DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(2),
                dag_expiry_limit: 8,
                max_levels_per_period: 10,
            },
        )?);
        let poison_owner = service.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_owner.lock().expect("lock before poisoning");
            panic!("poison dag service");
        })
        .join();
        let error = service.lock().err().expect("poisoned lock must fail");
        assert_eq!(error.to_string(), DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED);
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_rebuilds_persisted_head_and_non_finalized_graph() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_restore");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let genesis = H256::repeat_byte(1);
        let anchor_rlp = dag_block(genesis, 3, 3);
        let anchor = dag_manager_block_from_rlp(&anchor_rlp)?;
        let live_rlp = dag_block(anchor.hash, 4, 4);
        let live = dag_manager_block_from_rlp(&live_rlp)?;

        seed_pbft_head(storage.as_ref(), 1, anchor.hash)?;
        storage.dag().write(
            anchor.hash,
            anchor.level,
            anchor.tips.len() as u64,
            &anchor_rlp,
        )?;
        storage
            .dag()
            .write(live.hash, live.level, live.tips.len() as u64, &live_rlp)?;

        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: genesis,
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        let runtime = service.lock()?;
        assert_eq!(runtime.state.period(), 1);
        assert_eq!(runtime.state.anchor(), anchor.hash);
        assert!(runtime.state.has_vertex(live.hash));
        assert_eq!(runtime.state.max_level(), 4);
        assert_eq!(runtime.state.non_finalized_min_difficulty(), 3);
        assert_eq!(runtime.state.non_finalized_blocks_size().1, 2);
        drop(runtime);
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_fails_before_publication_when_persisted_anchor_is_missing() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_missing_anchor");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        seed_pbft_head(storage.as_ref(), 1, H256::repeat_byte(9))?;
        let error = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )
        .err()
        .expect("missing anchor must reject restoration");
        assert!(
            error
                .to_string()
                .contains("DAG_RUNTIME_RESTORE_ANCHOR_BLOCK")
        );
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_fails_before_publication_on_malformed_non_finalized_payload() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_malformed");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        storage.dag().write(H256::repeat_byte(7), 1, 0, &[0xc0])?;
        let error = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )
        .err()
        .expect("malformed DAG payload must reject restoration");
        assert!(
            error
                .to_string()
                .contains("DAG_RUNTIME_RESTORE_NON_FINALIZED_BLOCKS"),
            "{error:#}"
        );
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_block_session_orders_live_queries_then_gas() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_verify_session");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            begin_verify_block_session_with_mapping(
                &mut runtime,
                7,
                5,
                Vec::new(),
                vec![H256::from([2u8; 32]), H256::from([3u8; 32])],
                vec![H256::from([3u8; 32])],
            )?;
            let initial = runtime.verify_block_session_next();
            assert_eq!(initial.status, DAG_VERIFY_SESSION_STATUS_ACTIVE);
            assert_eq!(initial.action, DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY);
            assert_eq!(initial.proposal_period, 7);

            let query = runtime.verify_block_transaction_query()?;
            assert_eq!(query.hashes, vec![H256::from([2u8; 32])]);

            let authorization = runtime.verify_block_session_apply_transaction_resolution(2);
            assert_eq!(
                authorization.action,
                DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
            );
            let snapshot = match runtime.prepare_verify_block_authorization() {
                DagVerifyBlockAuthorizationPreparation::Snapshot(snapshot) => snapshot,
                DagVerifyBlockAuthorizationPreparation::Step(_) => {
                    anyhow::bail!("authorization step must be requested after tx resolution")
                }
            };
            let vdf_step = runtime.apply_verify_block_authorization(
                &snapshot,
                DagDposAuthorizationFacts {
                    vrf_key: Some([0x44; 32]),
                    vrf_key_found: true,
                    sender_eligible_vote_count: 11,
                    vdf_sortition_max_vote_count: 33,
                    eligibility_status: crate::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
                },
            )?;
            assert_eq!(vdf_step.action, DAG_VERIFY_SESSION_ACTION_VDF_SORTITION);
            let vdf_snapshot = runtime.snapshot_verify_block_vdf(vdf_step.cursor_id)?;
            let gas_step = runtime.complete_verify_block_vdf(
                &vdf_snapshot,
                crate::dag::DAG_VERIFY_VDF_STATUS_VALID,
            )?;
            assert_eq!(gas_step.action, DAG_VERIFY_SESSION_ACTION_GAS);

            let complete = runtime.verify_block_session_report_gas(DagVerifyBlockGasReport {
                block_gas_estimation: 10,
                estimated_transactions_weight: 10,
                dag_gas_limit: 20,
                pbft_gas_limit: 100,
            })?;
            assert!(complete.complete);
            assert_eq!(complete.status, DAG_VERIFY_SESSION_STATUS_COMPLETE);
            assert_eq!(complete.reject_code, 0);

            begin_verify_block_session_with_mapping(
                &mut runtime,
                7,
                5,
                Vec::new(),
                vec![H256::from([4u8; 32])],
                Vec::new(),
            )?;
            let missing = runtime.verify_block_session_apply_transaction_resolution(0);
            assert!(missing.complete);
            assert_eq!(
                missing.reject_code,
                crate::dag::DAG_VERIFY_REJECT_MISSING_TRANSACTION
            );
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_block_vdf_stale_snapshot_preserves_replacement_cursor() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_verify_vdf_stale_snapshot");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            begin_verify_block_session_with_mapping(
                &mut runtime,
                7,
                5,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )?;
            let authorization = runtime.verify_block_session_apply_transaction_resolution(0);
            assert_eq!(
                authorization.action,
                DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
            );
            let authorization_snapshot = match runtime.prepare_verify_block_authorization() {
                DagVerifyBlockAuthorizationPreparation::Snapshot(snapshot) => snapshot,
                DagVerifyBlockAuthorizationPreparation::Step(_) => {
                    anyhow::bail!("verification cursor must await authorization")
                }
            };
            let vdf = runtime.apply_verify_block_authorization(
                &authorization_snapshot,
                DagDposAuthorizationFacts {
                    vrf_key: Some([0x44; 32]),
                    vrf_key_found: true,
                    sender_eligible_vote_count: 1,
                    vdf_sortition_max_vote_count: 1,
                    eligibility_status: crate::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
                },
            )?;
            let stale_snapshot = runtime.snapshot_verify_block_vdf(vdf.cursor_id)?;

            begin_verify_block_session_with_mapping(
                &mut runtime,
                7,
                5,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )?;
            let error = runtime
                .complete_verify_block_vdf(&stale_snapshot, crate::dag::DAG_VERIFY_VDF_STATUS_VALID)
                .expect_err("a replacement cursor must reject the stale VDF completion");
            assert!(
                error
                    .to_string()
                    .contains("DAG_VERIFY_SESSION_VDF_CURSOR_MISMATCH")
            );
            let replacement = runtime.verify_block_session_next();
            assert_eq!(replacement.status, DAG_VERIFY_SESSION_STATUS_ACTIVE);
            assert_eq!(
                replacement.action,
                DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY
            );
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_gas_skips_tip_lookup_when_count_policy_does_not_require_it() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_verify_gas_no_tip_lookup");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let tip = H256::from([0x71u8; 32]);
        storage
            .dag()
            .write(tip, 4, 0, &signed_dag_block_rlp(4, 25))?;

        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            begin_verify_session_for_gas(&mut runtime, tip)?;
            storage.dag().remove(tip)?;
            let complete = runtime.verify_block_session_report_gas(DagVerifyBlockGasReport {
                block_gas_estimation: 10,
                estimated_transactions_weight: 10,
                dag_gas_limit: 20,
                pbft_gas_limit: 100,
            })?;
            assert!(complete.complete);
            assert_eq!(complete.reject_code, 0);
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_gas_loads_retained_tips_only_when_count_policy_requires_it() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_verify_gas_required_tip_lookup");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let tip = H256::from([0x72u8; 32]);
        storage
            .dag()
            .write(tip, 4, 0, &signed_dag_block_rlp(4, 25))?;

        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            begin_verify_session_for_gas(&mut runtime, tip)?;
            let complete = runtime.verify_block_session_report_gas(DagVerifyBlockGasReport {
                block_gas_estimation: 10,
                estimated_transactions_weight: 10,
                dag_gas_limit: 20,
                pbft_gas_limit: 30,
            })?;
            assert!(complete.complete);
            assert_eq!(
                complete.reject_code,
                crate::dag::DAG_VERIFY_REJECT_BLOCK_TOO_BIG
            );

            begin_verify_session_for_gas(&mut runtime, tip)?;
            storage.dag().remove(tip)?;
            let missing = runtime.verify_block_session_report_gas(DagVerifyBlockGasReport {
                block_gas_estimation: 10,
                estimated_transactions_weight: 10,
                dag_gas_limit: 20,
                pbft_gas_limit: 30,
            })?;
            assert_eq!(
                missing.reject_code,
                crate::dag::DAG_VERIFY_REJECT_MISSING_TIP
            );
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_gas_wrong_stage_rejects_before_retained_tip_lookup() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_verify_gas_wrong_stage");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let tip = H256::from([0x73u8; 32]);
        storage
            .dag()
            .write(tip, 4, 0, &signed_dag_block_rlp(4, 25))?;

        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            begin_verify_session_for_gas(&mut runtime, tip)?;
            begin_verify_block_session_with_mapping(
                &mut runtime,
                7,
                5,
                vec![tip],
                Vec::new(),
                Vec::new(),
            )?;
            storage.dag().remove(tip)?;
            let invalid = runtime.verify_block_session_report_gas(DagVerifyBlockGasReport {
                block_gas_estimation: 10,
                estimated_transactions_weight: 10,
                dag_gas_limit: 20,
                pbft_gas_limit: 30,
            })?;
            assert_eq!(invalid.status, DAG_VERIFY_SESSION_STATUS_INVALID_REPORT);
            assert_eq!(
                invalid.error_code,
                "DAG_VERIFY_SESSION_UNEXPECTED_GAS_REPORT"
            );
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn dag_service_state_proposer_session_orders_the_happy_path() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_proposer_happy_path");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            let session_id = begin_proposer_vdf_session(
                &mut runtime,
                proposer_sortition_params(),
                H256::from([0x31u8; 32]),
            )?;
            let waiting = runtime.proposer_session_poll_vdf(session_id);
            assert_eq!(waiting.action, DagProposerSessionAction::StartVdf);

            let sign = runtime.report_proposer_vdf_proof(
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )?;
            assert_eq!(sign.action, DagProposerSessionAction::SignBlock);
            assert_ne!(sign.signing_hash, H256::zero());

            let add = runtime.report_proposer_signing(
                session_id,
                DagProposerSigningReport {
                    signature: sign_dag_hash(0x44, sign.signing_hash.into()),
                },
            )?;
            assert_eq!(add.action, DagProposerSessionAction::AddBlock);

            let complete = runtime.report_proposer_add_block(
                session_id,
                DagProposerAddBlockReport {
                    accepted: true,
                    duplicate: false,
                    expired: false,
                    missing_references: Vec::new(),
                },
            );
            assert_eq!(complete.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(complete.action, DagProposerSessionAction::Complete);
            assert!(complete.return_value);
            assert!(complete.record_proposed_block);
            assert!(complete.update_retry_state);
            assert_eq!(complete.next_last_propose_level, 1);
            assert_eq!(complete.next_retry_count, 0);

            let retry = runtime
                .proposer_retry_states
                .get(&proposer_vrf_key())
                .expect("retry state should exist");
            assert_eq!(retry.last_propose_level, 1);
            assert_eq!(retry.retry_count, 0);
            assert_eq!(
                runtime.proposer_session_next(session_id).status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn dag_service_state_proposer_session_handles_missing_invalid_out_of_order_and_abort_idempotent()
    -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_proposer_invalid_reports");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 0,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            let vrf_key = proposer_vrf_key();
            let missing = runtime.begin_proposer_session(
                proposer_begin_input(vrf_key),
                proposer_transaction_observation(),
            )?;
            let missing_next = runtime.proposer_session_next(missing);
            assert_eq!(missing_next.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(
                missing_next.reason_code,
                crate::dag::DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD
            );

            ensure_period_mapping_for_frontier(&runtime);
            let invalid_id = runtime.begin_proposer_session(
                proposer_begin_input(vrf_key),
                proposer_transaction_observation(),
            )?;
            let invalid = runtime.apply_proposer_pack(invalid_id, false, Vec::new());
            assert!(invalid.is_err_and(|error| {
                error
                    .to_string()
                    .contains("DAG_PROPOSER_PACK_SESSION_WRONG_STAGE")
            }));
            assert!(runtime.abort_proposer_session(invalid_id));
            let step = match runtime.prepare_proposer_final_chain_facts(invalid_id) {
                DagProposerFinalChainFactsPreparation::Snapshot(_) => {
                    anyhow::bail!("invalid report should invalidate the session")
                }
                DagProposerFinalChainFactsPreparation::Step(step) => *step,
            };
            assert_eq!(step.status, DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT);

            let out_of_order_id = runtime.begin_proposer_session(
                proposer_begin_input(vrf_key),
                proposer_transaction_observation(),
            )?;
            let snapshot = match runtime.prepare_proposer_final_chain_facts(out_of_order_id) {
                DagProposerFinalChainFactsPreparation::Snapshot(snapshot) => snapshot,
                DagProposerFinalChainFactsPreparation::Step(step) => {
                    panic!("expected fact snapshot, got {:?}", step);
                }
            };
            runtime
                .apply_proposer_final_chain_facts(
                    &snapshot,
                    0,
                    proposer_authorization_facts(vrf_key),
                    proposer_sortition_params(),
                    proposer_sortition_params(),
                )
                .expect("facts should advance to packing");
            runtime
                .apply_proposer_pack(
                    out_of_order_id,
                    false,
                    vec![TransactionPackingSelection {
                        hash: H256::from([9u8; 32]),
                        gas_used: 100,
                        transaction_rlp: vec![0x99],
                    }],
                )
                .expect("pack should move to VDF");
            let out_of_order = runtime
                .report_proposer_signing(
                    out_of_order_id,
                    DagProposerSigningReport {
                        signature: vec![0; 65],
                    },
                )
                .expect("out-of-order report should return terminal step");
            assert_eq!(
                out_of_order.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );

            let duplicate_vdf_id = begin_proposer_vdf_session(
                &mut runtime,
                proposer_sortition_params(),
                H256::from([0x0au8; 32]),
            )?;
            let sign = runtime.report_proposer_vdf_proof(
                duplicate_vdf_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )?;
            assert_eq!(sign.action, DagProposerSessionAction::SignBlock);
            let duplicate_vdf = runtime.report_proposer_vdf_proof(
                duplicate_vdf_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )?;
            assert_eq!(
                duplicate_vdf.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );

            let duplicate_signing_id = begin_proposer_vdf_session(
                &mut runtime,
                proposer_sortition_params(),
                H256::from([0x0bu8; 32]),
            )?;
            let sign = runtime.report_proposer_vdf_proof(
                duplicate_signing_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )?;
            let signature = sign_dag_hash(0x44, sign.signing_hash.into());
            let add_block = runtime.report_proposer_signing(
                duplicate_signing_id,
                DagProposerSigningReport {
                    signature: signature.clone(),
                },
            )?;
            assert_eq!(add_block.action, DagProposerSessionAction::AddBlock);
            let duplicate_signing = runtime.report_proposer_signing(
                duplicate_signing_id,
                DagProposerSigningReport { signature },
            )?;
            assert_eq!(
                duplicate_signing.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );

            let aborted = runtime.begin_proposer_session(
                proposer_begin_input(vrf_key),
                proposer_transaction_observation(),
            )?;
            assert!(runtime.abort_proposer_session(aborted));
            assert!(!runtime.abort_proposer_session(aborted));
            assert!(!runtime.abort_proposer_session(u64::MAX));
            let _ = runtime.proposer_session_next(aborted).status;
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn dag_service_state_proposer_session_tracks_stale_observation_before_and_after_vdf()
    -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_proposer_stale_observation");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            let vrf_key = proposer_vrf_key();
            ensure_period_mapping_for_frontier(&runtime);
            let before_id = runtime.begin_proposer_session(
                proposer_begin_input(vrf_key),
                proposer_transaction_observation(),
            )?;
            assert_eq!(
                runtime.proposer_session_next(before_id).action,
                DagProposerSessionAction::CollectFinalChainFacts
            );
            runtime.state.add_block(DagManagerBlock {
                hash: H256::from([2u8; 32]),
                pivot: H256::from([1u8; 32]),
                tips: Vec::new(),
                level: 2,
                difficulty: 100,
            })?;
            let stale = apply_proposer_final_chain_facts(
                &mut runtime,
                before_id,
                proposer_vrf_key(),
                proposer_sortition_params(),
            )?;
            assert_eq!(stale.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(
                stale.reason_code,
                crate::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION
            );
            assert_eq!(stale.error_code, "DAG_PROPOSER_SESSION_STALE_OBSERVATION");
            assert!(!runtime.proposer_retry_states.contains_key(&vrf_key));

            let after_id = begin_proposer_vdf_session(
                &mut runtime,
                proposer_sortition_params(),
                H256::from([0x22u8; 32]),
            )?;
            runtime.state.add_block(DagManagerBlock {
                hash: H256::from([3u8; 32]),
                pivot: H256::from([1u8; 32]),
                tips: Vec::new(),
                level: 4,
                difficulty: 90,
            })?;
            let stale = runtime.report_proposer_vdf_proof(
                after_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )?;
            assert_eq!(stale.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(
                stale.reason_code,
                crate::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION
            );
            assert!(!stale.update_retry_state);
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("retry state should persist");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn dag_service_state_proposer_session_rejects_bad_signatures_and_corrupt_tips() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_proposer_bad_signatures");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            let malformed_id = begin_proposer_vdf_session(
                &mut runtime,
                proposer_sortition_params(),
                H256::from([0x30u8; 32]),
            )?;
            let _malformed_step = runtime.report_proposer_vdf_proof(
                malformed_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )?;
            let malformed = runtime
                .report_proposer_signing(
                    malformed_id,
                    DagProposerSigningReport {
                        signature: vec![0; 65],
                    },
                )
                .err()
                .expect("malformed signature should fail");
            assert!(
                malformed
                    .to_string()
                    .contains("DAG_PROPOSER_SIGNATURE_RECOVERY")
            );
            assert!(!runtime.proposer_sessions.contains_key(&malformed_id));

            let wrong_key_id = begin_proposer_vdf_session(
                &mut runtime,
                proposer_sortition_params(),
                H256::from([0x31u8; 32]),
            )?;
            let signing = runtime.report_proposer_vdf_proof(
                wrong_key_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )?;
            let wrong_key = runtime
                .report_proposer_signing(
                    wrong_key_id,
                    DagProposerSigningReport {
                        signature: sign_dag_hash(0x45, signing.signing_hash.into()),
                    },
                )
                .err()
                .expect("wrong-key signature should be rejected");
            assert!(
                wrong_key
                    .to_string()
                    .contains("DAG_PROPOSER_SIGNATURE_PROPOSER_MISMATCH")
            );
            assert!(!runtime.proposer_sessions.contains_key(&wrong_key_id));
        }
        {
            let mut runtime = service.lock()?;
            let vrf_key = proposer_vrf_key();
            ensure_period_mapping_for_frontier(&runtime);
            add_frontier_blocks(&mut runtime)?;
            let tip_session_id = begin_proposer_vdf_session(
                &mut runtime,
                proposer_sortition_params(),
                H256::from([0x40u8; 32]),
            )?;
            let tip = runtime
                .state
                .proposer_frontier_facts()
                .frontier
                .tips
                .first()
                .cloned()
                .expect("frontier should expose a tip for tip storage checks");
            runtime.storage.dag().write(tip, 2, 0, &[0x80])?;
            let corrupt = runtime.report_proposer_vdf_proof(
                tip_session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            );
            assert!(
                corrupt.is_err(),
                "corrupt tip storage should reject signing intent preparation"
            );
            assert!(!runtime.proposer_sessions.contains_key(&tip_session_id));
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("retry state should be initialized before signing");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
            assert_eq!(retry.max_retry_count, 20);
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn dag_service_state_proposer_session_vdf_cancel_and_stale_resume() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_proposer_vdf_cancel_resume");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            let cancel_id = begin_proposer_vdf_session(
                &mut runtime,
                proposer_sortition_params(),
                H256::from([0x50u8; 32]),
            )?;
            let lowered_minimum = runtime
                .proposer_sessions
                .get(&cancel_id)
                .expect("cancel session should exist")
                .attempt
                .vdf_difficulty
                .saturating_sub(1);
            runtime
                .proposer_sessions
                .get_mut(&cancel_id)
                .expect("cancel session should remain")
                .minimum_vdf_difficulty = lowered_minimum;
            runtime.state.add_block(DagManagerBlock {
                hash: H256::from([5u8; 32]),
                pivot: H256::from([1u8; 32]),
                tips: Vec::new(),
                level: 3,
                difficulty: 100,
            })?;
            let cancel = runtime.proposer_session_poll_vdf(cancel_id);
            assert_eq!(cancel.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(cancel.action, DagProposerSessionAction::CancelVdf);

            let resume_level = runtime.state.proposer_frontier_facts().propose_level;
            runtime.proposer_retry_states.insert(
                proposer_vrf_key(),
                DagProposerRetryState {
                    last_propose_level: resume_level,
                    retry_count: 20,
                    max_retry_count: 20,
                },
            );
            let resume_id = begin_proposer_vdf_session(
                &mut runtime,
                proposer_sortition_params_stale(),
                H256::from([0x51u8; 32]),
            )?;
            let sleep = runtime.report_proposer_vdf_proof(
                resume_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )?;
            assert_eq!(sleep.action, DagProposerSessionAction::StaleProofSleep);

            runtime.state.add_block(DagManagerBlock {
                hash: H256::from([6u8; 32]),
                pivot: H256::from([1u8; 32]),
                tips: Vec::new(),
                level: 4,
                difficulty: 120,
            })?;
            let resumed = runtime.resume_proposer_stale_proof(resume_id)?;
            assert_eq!(resumed.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(resumed.action, DagProposerSessionAction::Complete);
            assert_eq!(
                resumed.reason_code,
                crate::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION
            );
            assert!(!resumed.update_retry_state);
            assert!(!runtime.proposer_sessions.contains_key(&resume_id));
        }
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }
}
