//! Native application ownership for the DAG, sortition, and transaction services.
//!
//! This module is the single supported Rust production composition for the DAG
//! cluster. It restores all sibling services from one storage owner before
//! publication and defines native access to the canonical
//! DAG-then-sortition-then-transaction lock domains. Bridge code may temporarily borrow the typed guards exposed here
//! while FFI-shaped task methods move into native owners; no guard may cross CXX
//! or an external executor boundary.

use crate::dag::{
    DagBlockStorageLookup, DagDposAuthorizationFacts, DagFrontier, DagHashStorageLookup,
    DagPersistenceCounters, DagPivotTipsValidation, DagProposerStorageTipSelectionInput,
    DagProposerTipSelection, DagVdfSortitionBlockInput, dag_block_transaction_hashes,
    dag_manager_block_from_rlp, verify_dag_vdf_sortition_from_block,
};
use crate::dag_service::{
    DagAddBlockPreparedTransaction, DagAddBlockSession, DagAddBlockStoredPlan,
    DagProposerAddBlockReport, DagProposerFinalChainFactsPreparation,
    DagProposerFinalChainFactsSnapshot, DagProposerSessionBeginInput, DagProposerSessionStep,
    DagProposerSigningReport, DagProposerTransactionObservation, DagProposerVdfProofReport,
    DagRuntimeNonFinalizedSyncPayload, DagService, DagServiceConfig, DagServiceGuard,
    DagVerifyBlockAuthorizationPreparation, DagVerifyBlockAuthorizationSnapshot,
    DagVerifyBlockGasReport, DagVerifyBlockSessionInput, DagVerifyBlockSessionStep,
};
use crate::sortition::{SortitionConfig, SortitionService, SortitionServiceGuard};
use crate::transaction_manager::TransactionManagerVerifyTransactionFact;
use crate::transaction_packing_service::{TransactionPackingEstimate, TransactionPackingOwner};
use crate::transaction_queue::TransactionQueueEntry;
use crate::transaction_service::{
    DagTransactionSaveInput, TransactionService, TransactionServiceAccountNonceFact,
    TransactionServiceAdmissionReport, TransactionServiceCompatibilityPackFinalized,
    TransactionServiceCompatibilityPackPrepared, TransactionServiceCompatibilityPackRequest,
    TransactionServiceConfig, TransactionServiceEstimateRequest,
    TransactionServiceFinalChainAdmissionFact, TransactionServiceFinalizedFilterRequest,
    TransactionServiceFinalizedStatusFact, TransactionServiceFinalizedStatusReport,
    TransactionServiceGasEstimationPlan, TransactionServiceGasEstimationRequest,
    TransactionServiceGasEstimationResult, TransactionServiceGuard, TransactionServicePackEstimate,
    TransactionServicePayload, TransactionServiceProposerPackFinalized,
    TransactionServiceProposerPackPrepared, TransactionServicePublicAdmissionReport,
    TransactionServiceTransactionView, TransactionServiceTransactionViewPlan,
    TransactionServiceTransactionViewRequest, TransactionServiceValidatedAdmissionFact,
    TransactionServiceVerifyNotFinalizedFact, TransactionServiceVerifyNotFinalizedOutcome,
    append_prepared_dag_transactions, prepare_dag_transaction_publication,
    prepare_dag_transactions, publish_dag_transactions,
    remove_non_finalized_sidecars_after_dag_commit,
};
use anyhow::{Context, Result, ensure};
use ethereum_types::{H160, H256, U256};
use rustaxa_storage::{Storage, StorageWriteBatch};
use rustaxa_types::LegacyTransactionEnvelope;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Immutable configuration for the native DAG application root.
///
/// Construction restores transaction state first, DAG state second, and
/// sortition state last. This preserves the production startup error precedence
/// while publishing no partially constructed root.
#[derive(Clone, Debug)]
pub struct DagTransactionServiceConfig {
    /// Native transaction runtime configuration.
    pub transaction: TransactionServiceConfig,
    /// Native DAG runtime configuration.
    pub dag: DagServiceConfig,
    /// Native sortition runtime configuration.
    pub sortition: SortitionConfig,
}

/// Native application root for DAG, sortition, and transaction consensus state.
///
/// Every sibling receives the same `Arc<Storage>`. The root owns construction,
/// restoration, lifetime, and cross-domain lock domains. Its lock accessors are
/// a temporary CRW-12 Rust-only escape hatch for bridge task adapters and never
/// expose raw mutexes or guards through CXX.
pub struct DagTransactionService {
    transaction: TransactionService,
    dag: DagService,
    sortition: SortitionService,
}

/// Canonical transaction payload supplied while preparing a DAG block.
#[derive(Clone, Debug)]
pub struct DagAddBlockTransactionPayload {
    /// Expected signed transaction hash.
    pub hash: H256,
    /// Canonical signed transaction RLP.
    pub transaction_rlp: Vec<u8>,
}

/// Native request for one DAG add-block preparation.
#[derive(Debug)]
pub struct DagAddBlockPrepareRequest {
    /// Expected DAG block identity.
    pub expected_hash: H256,
    /// Canonical signed DAG block RLP.
    pub block_rlp: Vec<u8>,
    /// Whether the canonical RLP hash must equal `expected_hash`.
    pub validate_hash: bool,
    /// Whether the accepted block and transactions should be persisted.
    pub save: bool,
    /// Whether the local node proposed the block.
    pub proposed: bool,
    /// Ordered transaction payloads materialized by the retained C++ boundary.
    pub transactions: Vec<DagAddBlockTransactionPayload>,
}

/// Latest-account query requested for one inspected block transaction.
#[derive(Clone, Copy, Debug)]
pub struct DagAddBlockAccountRequest {
    /// Stable index into the request transaction list.
    pub input_index: u64,
    /// Recovered transaction sender.
    pub sender: H160,
}

/// Native non-mutating or cursor-opening add-block preparation.
#[derive(Debug)]
pub struct DagAddBlockPreparation {
    /// Nonzero cursor for an accepted nonterminal transition.
    pub cursor_id: u64,
    /// Candidate block level.
    pub block_level: u64,
    /// Whether the block is accepted.
    pub accepted: bool,
    /// Whether this is an idempotent persisted duplicate.
    pub duplicate: bool,
    /// Whether the block is below the live expiry level.
    pub expired: bool,
    /// Missing pivot/tip identities in deterministic order.
    pub missing_references: Vec<H256>,
    /// External latest-account queries required before completion.
    pub account_requests: Vec<DagAddBlockAccountRequest>,
}

/// Latest sender nonce for one prepared transaction.
#[derive(Clone, Copy, Debug)]
pub struct DagAddBlockAccountNonceFact {
    /// Stable transaction input index.
    pub input_index: u64,
    /// Latest FinalChain account nonce.
    pub account_nonce: U256,
}

/// Cursor-bound completion facts for one prepared add-block transition.
#[derive(Debug)]
pub struct DagAddBlockCompletion {
    /// Prepared cursor identity.
    pub cursor_id: u64,
    /// Latest account nonces for all retained transactions.
    pub account_nonce_facts: Vec<DagAddBlockAccountNonceFact>,
}

/// Durable add-block result and leaf-adapter effects.
#[derive(Debug)]
pub struct DagAddBlockCommitReport {
    /// Always true for a completed accepted cursor.
    pub accepted: bool,
    /// Whether the C++ event adapter should emit verification.
    pub emit_verified: bool,
    /// Whether the transport adapter should gossip the block.
    pub gossip: bool,
    /// Whether the local node proposed the block.
    pub proposed: bool,
    /// Transaction hashes erased from the native pending queue.
    pub queue_erased: Vec<H256>,
    /// Persisted DAG block and edge counters after the transition.
    pub counters: DagPersistenceCounters,
}

/// Native finalization result returned to the retained event adapter.
#[derive(Debug)]
pub struct DagFinalizationReport {
    /// Number of hashes finalized by the transition.
    pub finalized_count: usize,
    /// Expired DAG identities for temporary external seen-block cleanup.
    pub expired_hashes: Vec<H256>,
}

/// Complete native request for one proposer transaction-pack preparation.
#[derive(Clone, Copy, Debug)]
pub struct DagProposerPackPrepareRequest {
    /// DAG proposer cursor identity.
    pub session_id: u64,
    /// Whether transport pressure prevents transaction packing.
    pub network_throttled: bool,
    /// Minimum transaction gas used to bound the queue snapshot.
    pub min_transaction_gas: u64,
    /// Declared-gas ceiling below which EVM estimation is unnecessary.
    pub estimate_gas_limit: u64,
    /// FinalChain head recorded on any queue demotion.
    pub last_block_number: u64,
}

/// Native proposer instruction returned across the unlocked EVM boundary.
#[derive(Clone, Debug)]
pub struct DagProposerPackStep {
    /// Current proposer state-machine instruction.
    pub session: DagProposerSessionStep,
    /// Executor-ready candidates requiring live EVM estimation.
    pub estimate_requests: Vec<TransactionServiceEstimateRequest>,
}

/// External FinalChain facts requested for one exact proposer cursor.
#[derive(Clone)]
pub struct DagProposerFinalChainRequest {
    snapshot: DagProposerFinalChainFactsSnapshot,
    initially_loaded_params: crate::sortition::SortitionParams,
    /// Proposal period queried at the retained FinalChain boundary.
    pub proposal_period: u64,
    /// Whether the native DAG observation resolved a proposal period.
    pub proposal_period_found: bool,
    /// Proposer address queried for historical DPoS authorization.
    pub proposer_address: [u8; 20],
}

/// Proposer preparation either requests unlocked FinalChain facts or returns a terminal step.
pub enum DagProposerFinalChainRequestOrStep {
    Request(DagProposerFinalChainRequest),
    Step(Box<DagProposerSessionStep>),
}

/// FinalChain facts returned across the retained external executor boundary.
#[derive(Clone)]
pub struct DagProposerFinalChainFacts {
    /// Latest finalized period observed by FinalChain.
    pub last_finalized_period: u64,
    /// Historical proposer authorization and vote facts.
    pub authorization_facts: DagDposAuthorizationFacts,
}

/// Ordered transaction views prepared for retained C++ materialization.
#[derive(Clone, Debug)]
pub struct DagVerifyBlockTransactionPreparation {
    /// Exact native verifier cursor identity.
    pub cursor_id: u64,
    /// Proposal period used for finalized sender nonce facts.
    pub proposal_period: u64,
    /// Canonical transaction views in block order.
    pub transactions: Vec<TransactionServiceTransactionView>,
}

/// Cursor-bound completion facts after retained C++ transaction materialization.
#[derive(Clone, Debug)]
pub struct DagVerifyBlockTransactionCompletionReport {
    /// Cursor returned by the matching preparation.
    pub cursor_id: u64,
    /// Proposal period returned by the matching preparation.
    pub proposal_period: u64,
    /// Exact-period sender account facts for finalized storage transactions.
    pub account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
}

/// FinalChain authorization request prepared without retaining a native guard.
#[derive(Clone)]
pub struct DagVerifyBlockAuthorizationRequest {
    snapshot: DagVerifyBlockAuthorizationSnapshot,
    /// Proposal period queried at the retained FinalChain boundary.
    pub proposal_period: u64,
    /// Recovered DAG block sender queried for DPoS and VRF facts.
    pub sender: H160,
}

/// Authorization preparation either requests external facts or returns a status step.
pub enum DagVerifyBlockAuthorizationRequestOrStep {
    Request(DagVerifyBlockAuthorizationRequest),
    Step(DagVerifyBlockSessionStep),
}

/// Proof-bearing facts supplied by the retained C++ verifier adapter.
#[derive(Clone, Debug)]
pub struct DagVerifyBlockVdfRequest {
    /// Exact verifier cursor identity.
    pub cursor_id: u64,
    /// Canonical signed DAG block RLP.
    pub block_rlp: Vec<u8>,
    /// Candidate DAG level.
    pub block_level: u64,
    /// Proposal-period block hash used by VDF sortition.
    pub proposal_period_hash: H256,
}

/// Root used when reading a deterministic GHOST path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DagGhostPathRoot {
    /// Begin at one explicit DAG block.
    Block(H256),
    /// Begin at the current finalized anchor.
    CurrentAnchor,
}

/// Native graph projection requested for diagnostic GraphViz rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DagGraphView {
    /// Render the complete live DAG.
    Complete,
    /// Render only the selected pivot tree.
    PivotTree,
}

/// Pair of finalized DAG anchors observed under one native lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DagAnchors {
    /// Previous finalized anchor.
    pub old: H256,
    /// Current finalized anchor.
    pub current: H256,
}

/// Lock-consistent status of the native DAG graph and finalization head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DagRuntimeStatus {
    /// Number of vertices in the live in-memory graph.
    pub vertex_count: u64,
    /// Number of directed edges in the live in-memory graph.
    pub edge_count: u64,
    /// Highest level represented by the live graph.
    pub max_level: u64,
    /// Latest finalized PBFT period reflected by DAG state.
    pub period: u64,
    /// Previous and current finalized DAG anchors.
    pub anchors: DagAnchors,
    /// Configured count of retained levels behind the frontier.
    pub expiry_limit: u32,
    /// Lowest currently retained DAG level.
    pub expiry_level: u64,
}

/// Non-finalized hashes stored at one DAG level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DagLevelHashes {
    /// DAG level shared by every returned hash.
    pub level: u64,
    /// Deterministically ordered hashes at this level.
    pub hashes: Vec<H256>,
}

/// Complete non-finalized level index observed under one native lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DagNonFinalizedIndex {
    /// Ascending level entries containing ordered block hashes.
    pub levels: Vec<DagLevelHashes>,
}

/// Compact non-finalized pressure facts observed under one native lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DagNonFinalizedSummary {
    /// Number of non-empty levels.
    pub levels: u64,
    /// Total number of non-finalized blocks.
    pub blocks: u64,
    /// Minimum live VDF difficulty, or `u32::MAX` when empty.
    pub min_difficulty: u32,
}

impl DagTransactionService {
    /// Restores all sibling services and publishes one coherent application root.
    ///
    /// Transaction restoration runs before DAG restoration, which runs before
    /// sortition restoration. Any validation, decoding, or storage error returns
    /// without publishing the root. The shared storage owner is cloned only into
    /// the native sibling services.
    pub fn restore(storage: Arc<Storage>, config: DagTransactionServiceConfig) -> Result<Self> {
        let transaction = TransactionService::restore(storage.clone(), config.transaction)?;
        let dag = DagService::restore(storage.clone(), config.dag)?;
        let sortition = SortitionService::restore(config.sortition, storage)?;
        Ok(Self {
            transaction,
            dag,
            sortition,
        })
    }

    /// Returns the transaction owner's queue-aware gas-price bid.
    pub fn transaction_gas_price_bid(&self) -> Result<[u8; 32]> {
        self.transaction.gas_price_bid()
    }

    /// Returns the native declared, cached, or external-EVM gas-estimation decision.
    pub fn transaction_plan_gas_estimation(
        &self,
        request: TransactionServiceGasEstimationRequest,
    ) -> Result<TransactionServiceGasEstimationPlan> {
        self.transaction.plan_gas_estimation(request)
    }

    /// Returns the transaction owner's durable transaction count.
    pub fn transaction_count(&self) -> Result<u64> {
        self.transaction.transaction_count()
    }

    /// Returns whether native queue or sidecar state knows `hash`.
    pub fn transaction_is_known(&self, hash: [u8; 32]) -> Result<bool> {
        self.transaction.is_transaction_known(hash)
    }

    /// Returns the current non-finalized sidecar cardinality.
    pub fn transaction_non_finalized_size(&self) -> Result<usize> {
        self.transaction.non_finalized_size()
    }

    /// Returns queue-only transaction views in request order.
    pub fn transaction_queue_views(
        &self,
        requests: Vec<TransactionServiceTransactionViewRequest>,
    ) -> Result<Vec<TransactionServiceTransactionView>> {
        self.transaction.queue_transaction_views(requests)
    }

    /// Returns non-finalized sidecar views in request order.
    pub fn transaction_non_finalized_views(
        &self,
        requests: Vec<TransactionServiceTransactionViewRequest>,
    ) -> Result<Vec<TransactionServiceTransactionView>> {
        self.transaction.non_finalized_transaction_views(requests)
    }

    /// Returns bounded transaction views using native source precedence.
    pub fn transaction_views(
        &self,
        requests: Vec<TransactionServiceTransactionViewRequest>,
        max_count: u64,
    ) -> Result<TransactionServiceTransactionViewPlan> {
        self.transaction.transaction_views(requests, max_count)
    }

    /// Returns proposal-period transaction views with optional nonce facts.
    pub fn proposal_transaction_views(
        &self,
        proposal_period: u64,
        requests: Vec<TransactionServiceTransactionViewRequest>,
        account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
        max_count: u64,
    ) -> Result<TransactionServiceTransactionViewPlan> {
        self.transaction.proposal_transaction_views(
            proposal_period,
            requests,
            account_nonce_facts,
            max_count,
        )
    }

    /// Returns proposer transaction groups ordered by sender and nonce.
    pub fn transaction_queue_groups(&self) -> Result<Vec<Vec<TransactionQueueEntry>>> {
        self.transaction.queue_transaction_groups()
    }

    /// Returns the current proposable transaction count.
    pub fn transaction_queue_size(&self) -> Result<usize> {
        self.transaction.queue_size()
    }

    /// Returns current proposable accounts as owned native addresses.
    pub fn transaction_queue_proposable_accounts(&self) -> Result<Vec<H160>> {
        self.transaction.queue_proposable_accounts()
    }

    /// Returns whether the queue drop-observation window is active.
    pub fn transaction_queue_dropped(&self) -> Result<bool> {
        self.transaction.queue_transactions_dropped()
    }

    /// Returns whether non-proposable queue state reached its configured bound.
    pub fn transaction_queue_non_proposable_over_limit(&self) -> Result<bool> {
        self.transaction.queue_non_proposable_over_limit()
    }

    /// Returns the minimum gas price for inclusion under `limit`.
    pub fn transaction_queue_min_gas_price(&self, limit: u64) -> Result<[u8; 32]> {
        self.transaction
            .queue_min_gas_price_for_block_inclusion(limit)
    }

    /// Updates finalized gas-price facts inside the native transaction owner.
    pub fn transaction_update_gas_prices(&self, gas_prices: Vec<U256>) -> Result<()> {
        self.transaction.update_gas_prices(gas_prices)
    }

    /// Executes validated transaction admission inside the native lock owner.
    pub fn transaction_execute_admission(
        &self,
        fact: TransactionServiceValidatedAdmissionFact,
        final_chain_fact: TransactionServiceFinalChainAdmissionFact,
        entry: TransactionQueueEntry,
    ) -> Result<TransactionServiceAdmissionReport> {
        self.transaction
            .execute_admission(fact, final_chain_fact, entry)
    }

    /// Executes public precheck, verification, and admission inside the native owner.
    pub fn transaction_execute_public_admission(
        &self,
        verify_fact: TransactionManagerVerifyTransactionFact,
        admission_fact: TransactionServiceValidatedAdmissionFact,
        final_chain_fact: TransactionServiceFinalChainAdmissionFact,
        entry: TransactionQueueEntry,
    ) -> Result<TransactionServicePublicAdmissionReport> {
        self.transaction.execute_public_admission(
            verify_fact,
            admission_fact,
            final_chain_fact,
            entry,
        )
    }

    /// Applies finalized status and periodic account purge in the native owner.
    pub fn transaction_update_finalized_status(
        &self,
        period: u64,
        retention_window: u64,
        account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
        facts: Vec<TransactionServiceFinalizedStatusFact>,
    ) -> Result<TransactionServiceFinalizedStatusReport> {
        self.transaction.update_finalized_status(
            period,
            retention_window,
            account_nonce_facts,
            facts,
        )
    }

    /// Starts one compatibility packing cursor and returns unlocked EVM requests.
    pub fn transaction_prepare_compatibility_pack(
        &self,
        request: TransactionServiceCompatibilityPackRequest,
    ) -> Result<TransactionServiceCompatibilityPackPrepared> {
        self.transaction.prepare_compatibility_pack(request)
    }

    /// Finalizes the active compatibility packing cursor.
    pub fn transaction_finalize_compatibility_pack(
        &self,
        estimates: Vec<TransactionServicePackEstimate>,
    ) -> Result<TransactionServiceCompatibilityPackFinalized> {
        self.transaction.finalize_compatibility_pack(estimates)
    }

    /// Aborts only an active compatibility packing cursor.
    pub fn transaction_abort_compatibility_pack(&self) -> Result<bool> {
        self.transaction.abort_compatibility_pack()
    }

    /// Stores one opaque external gas-estimation result.
    pub fn transaction_store_gas_estimation(
        &self,
        result: TransactionServiceGasEstimationResult,
    ) -> Result<bool> {
        self.transaction.store_gas_estimation(result)
    }

    /// Initializes recently-finalized sidecars in one native lock epoch.
    pub fn transaction_initialize_recently_finalized(
        &self,
        period: u64,
        payloads: Vec<TransactionServicePayload>,
    ) -> Result<()> {
        self.transaction
            .initialize_recently_finalized(period, payloads)
    }

    /// Removes selected non-finalized payloads durably before live publication.
    pub fn transaction_remove_non_finalized(&self, hashes: Vec<H256>) -> Result<u64> {
        self.transaction.remove_non_finalized(hashes)
    }

    /// Applies finalized-block expiry to native non-proposable queue state.
    pub fn transaction_queue_block_finalized(&self, block_number: u64) -> Result<Vec<H256>> {
        self.transaction.queue_block_finalized(block_number)
    }

    /// Filters finalized hashes entirely inside the native transaction owner.
    ///
    /// The root forwards owned indices/hashes and returns ordered native
    /// actions. Zero hashes or durable lookup failures propagate without a
    /// transaction guard escaping the call.
    pub fn transaction_filter_non_finalized(
        &self,
        requests: Vec<TransactionServiceFinalizedFilterRequest>,
    ) -> Result<crate::transaction_manager::FinalizedTransactionFilterPlan> {
        self.transaction.filter_non_finalized(requests)
    }

    /// Verifies finalized status with externally supplied sender nonce facts.
    ///
    /// The first recent-sidecar or nonce-gated durable hit is returned with its
    /// source tag. Empty/all-passing input returns the native none outcome;
    /// malformed hashes and storage failures propagate.
    pub fn transaction_verify_not_finalized(
        &self,
        facts: Vec<TransactionServiceVerifyNotFinalizedFact>,
    ) -> Result<TransactionServiceVerifyNotFinalizedOutcome> {
        self.transaction.verify_not_finalized(facts)
    }

    /// Restores non-finalized sidecars from native durable storage.
    ///
    /// The transaction owner removes stale finalized rows, validates every
    /// survivor envelope, and publishes live sidecar state atomically. The
    /// returned count is the number of recovered survivors; validation or
    /// storage errors leave prior live sidecar state intact.
    pub fn transaction_recover_non_finalized(&self) -> Result<u64> {
        self.transaction.recover_non_finalized()
    }

    /// Validates a candidate level against pivot and tip metadata.
    ///
    /// Live graph metadata is preferred and canonical storage is the fallback.
    /// Missing references are returned in deterministic pivot-then-tip order as
    /// a successful rejection result. Lock, storage, or decode failures return
    /// an error and no native guard escapes the call.
    pub fn dag_validate_references(
        &self,
        block_level: u64,
        pivot: H256,
        tips: Vec<H256>,
    ) -> Result<DagPivotTipsValidation> {
        self.dag
            .runtime_validate_pivot_tips(block_level, pivot, tips)
    }

    /// Builds one lock-consistent non-finalized synchronization snapshot.
    ///
    /// `known_hashes` are excluded before canonical block and transaction
    /// payloads are loaded. Block order and first-seen transaction order are
    /// preserved; missing transactions remain explicit successful lookups.
    /// Lock, storage, or decode failures abort the complete snapshot.
    pub fn dag_non_finalized_sync(
        &self,
        known_hashes: Vec<H256>,
    ) -> Result<DagRuntimeNonFinalizedSyncPayload> {
        self.dag.runtime_non_finalized_sync_payload(known_hashes)
    }

    /// Computes deterministic DAG order for one anchor.
    ///
    /// Unknown or non-orderable anchors return `None`; a known empty order is
    /// `Some(Vec::new())`. The returned hashes are owned and the native lock is
    /// released before return.
    pub fn dag_order(&self, anchor: H256) -> Result<Option<Vec<H256>>> {
        self.dag.runtime_compute_order(anchor)
    }

    /// Returns an owned pivot-and-tips frontier from one native lock epoch.
    ///
    /// Tip order is the canonical order maintained by `DagManagerState`.
    /// Poisoned lock state returns the stable native DAG lock error.
    pub fn dag_frontier(&self) -> Result<DagFrontier> {
        self.dag.runtime_frontier()
    }

    /// Returns a GHOST path rooted at an explicit block or current anchor.
    ///
    /// The complete path is owned by the caller after the native lock is
    /// released. Unknown explicit roots preserve `DagManagerState` semantics
    /// and return an empty path rather than a storage error.
    pub fn dag_ghost_path(&self, root: DagGhostPathRoot) -> Result<Vec<H256>> {
        match root {
            DagGhostPathRoot::Block(source) => self.dag.runtime_ghost_path(source),
            DagGhostPathRoot::CurrentAnchor => self.dag.runtime_anchor_ghost_path(),
        }
    }

    /// Renders one deterministic diagnostic graph projection.
    ///
    /// The selected complete or pivot-tree view is rendered while holding the
    /// native lock and returned as owned GraphViz text. The task performs no
    /// storage writes; poisoned lock state is returned as an error.
    pub fn dag_graphviz(&self, view: DagGraphView) -> Result<String> {
        self.dag
            .runtime_graphviz_dot(matches!(view, DagGraphView::PivotTree))
    }

    /// Returns graph, finalization, anchor, and expiry facts from one lock epoch.
    ///
    /// Counts are widened to stable `u64` domain values. Conversion overflow is
    /// reported as an error and no partial status is returned.
    pub fn dag_runtime_status(&self) -> Result<DagRuntimeStatus> {
        let dag = self.lock_dag()?;
        let (old, current) = dag.state.anchors();
        Ok(DagRuntimeStatus {
            vertex_count: u64::try_from(dag.state.vertex_count())
                .context("DAG_RUNTIME_VERTEX_COUNT_OVERFLOW")?,
            edge_count: u64::try_from(dag.state.edge_count())
                .context("DAG_RUNTIME_EDGE_COUNT_OVERFLOW")?,
            max_level: dag.state.max_level(),
            period: dag.state.period(),
            anchors: DagAnchors { old, current },
            expiry_limit: dag.state.dag_expiry_limit(),
            expiry_level: dag.state.dag_expiry_level(),
        })
    }

    /// Returns the complete ordered non-finalized level index from one lock epoch.
    pub fn dag_non_finalized_index(&self) -> Result<DagNonFinalizedIndex> {
        let dag = self.lock_dag()?;
        Ok(DagNonFinalizedIndex {
            levels: dag
                .state
                .non_finalized_blocks()
                .iter()
                .map(|(level, hashes)| DagLevelHashes {
                    level: *level,
                    hashes: hashes.iter().copied().collect(),
                })
                .collect(),
        })
    }

    /// Returns non-finalized pressure counters from one lock epoch.
    pub fn dag_non_finalized_summary(&self) -> Result<DagNonFinalizedSummary> {
        let dag = self.lock_dag()?;
        let (levels, blocks) = dag.state.non_finalized_blocks_size();
        Ok(DagNonFinalizedSummary {
            levels: u64::try_from(levels).context("DAG_RUNTIME_LEVEL_COUNT_OVERFLOW")?,
            blocks: u64::try_from(blocks).context("DAG_RUNTIME_BLOCK_COUNT_OVERFLOW")?,
            min_difficulty: dag.state.non_finalized_min_difficulty(),
        })
    }

    /// Checks block membership in live state or canonical storage.
    ///
    /// Live graph membership returns immediately. Missing storage entries return
    /// `false`; lock or storage failures return an error.
    pub fn dag_is_block_known(&self, hash: H256) -> Result<bool> {
        self.dag.runtime_is_block_known(hash)
    }

    /// Loads one canonical DAG block payload from native storage.
    ///
    /// Missing blocks return `found = false` with an empty payload. Storage or
    /// decode failures return an error.
    pub fn dag_load_block(&self, hash: H256) -> Result<DagBlockStorageLookup> {
        self.dag.runtime_load_block(hash)
    }

    /// Selects proposer tips from storage-backed candidate metadata.
    ///
    /// The native planner owns sender recovery, proposer grouping, level order,
    /// gas and maximum-tip limits, and missing-tip accounting. It returns owned
    /// selected hashes; malformed payloads or storage failures are errors.
    pub fn dag_select_proposer_tips(
        &self,
        input: DagProposerStorageTipSelectionInput,
    ) -> Result<DagProposerTipSelection> {
        self.dag.runtime_plan_proposer_tip_selection(input)
    }

    /// Resolves the canonical PBFT block hash for one finalized period.
    ///
    /// Missing period data returns `found = false`; malformed period data,
    /// storage failures, and lock poison are errors.
    pub fn dag_period_block_hash(&self, period: u64) -> Result<DagHashStorageLookup> {
        self.dag.runtime_period_block_hash(period)
    }

    /// Reads persisted DAG block and edge counters from canonical storage.
    ///
    /// Both counters are returned from one storage read task. Lock or storage
    /// failures return an error and no partial counters are exposed.
    pub fn dag_persistence_counters(&self) -> Result<DagPersistenceCounters> {
        self.dag.runtime_persistence_counters()
    }

    /// Opens one proposer cursor from a single DAG-then-transaction observation.
    ///
    /// CXX supplies only immutable wallet/configuration facts. Queue and
    /// non-finalized pressure are captured from the native transaction sibling
    /// while both state domains remain serialized.
    pub fn begin_proposer_session(&self, input: DagProposerSessionBeginInput) -> Result<u64> {
        let (mut dag, transaction) = self.lock_dag_and_transaction()?;
        let (transaction_pool_size, non_finalized_transaction_count) =
            transaction.dag_proposer_transaction_pressure();
        dag.begin_proposer_session(
            input,
            DagProposerTransactionObservation {
                transaction_pool_size,
                non_finalized_transaction_count,
            },
        )
    }

    /// Returns the current proposer instruction without bridge-owned state transitions.
    pub fn next_proposer_session(&self, session_id: u64) -> Result<DagProposerSessionStep> {
        Ok(self.dag.lock()?.proposer_session_next(session_id))
    }

    /// Idempotently removes the exact DAG cursor and any paired packing cursor.
    pub fn abort_proposer_session(&self, session_id: u64) -> Result<bool> {
        self.abort_proposer_pack(session_id)
    }

    /// Prepares an unlocked FinalChain query for an exact proposer cursor.
    ///
    /// DAG and sortition locks are acquired in separate intervals and both are
    /// released before the request is returned. Sortition lookup failure removes
    /// only the still-matching proposer cursor.
    pub fn prepare_proposer_final_chain_facts(
        &self,
        session_id: u64,
    ) -> Result<DagProposerFinalChainRequestOrStep> {
        let snapshot = {
            let mut dag = self.dag.lock()?;
            match dag.prepare_proposer_final_chain_facts(session_id) {
                DagProposerFinalChainFactsPreparation::Snapshot(snapshot) => snapshot,
                DagProposerFinalChainFactsPreparation::Step(step) => {
                    return Ok(DagProposerFinalChainRequestOrStep::Step(step));
                }
            }
        };
        let initially_loaded_params = match self.sortition.lock().and_then(|sortition| {
            sortition
                .params_for_period_from_storage(snapshot.proposal_period)
                .context("DAG_PROPOSER_SESSION_SORTITION_PARAMS_INITIAL_LOOKUP")
        }) {
            Ok(params) => params,
            Err(error) => {
                self.dag
                    .lock()?
                    .cleanup_proposer_final_chain_facts(&snapshot);
                return Err(error);
            }
        };
        Ok(DagProposerFinalChainRequestOrStep::Request(
            DagProposerFinalChainRequest {
                proposal_period: snapshot.proposal_period,
                proposal_period_found: snapshot.proposal_period_found,
                proposer_address: snapshot.proposer_address,
                snapshot,
                initially_loaded_params,
            },
        ))
    }

    /// Revalidates one prepared proposer cursor and applies unlocked FinalChain facts.
    pub fn complete_proposer_final_chain_facts(
        &self,
        request: &DagProposerFinalChainRequest,
        facts: DagProposerFinalChainFacts,
    ) -> Result<DagProposerSessionStep> {
        let mut dag = self.dag.lock()?;
        let sortition = match self.sortition.lock() {
            Ok(sortition) => sortition,
            Err(error) => {
                dag.cleanup_proposer_final_chain_facts(&request.snapshot);
                return Err(error);
            }
        };
        let revalidated_params = match sortition
            .params_for_period_from_storage(request.proposal_period)
            .context("DAG_PROPOSER_SESSION_SORTITION_PARAMS_REVALIDATION_LOOKUP")
        {
            Ok(params) => params,
            Err(error) => {
                dag.cleanup_proposer_final_chain_facts(&request.snapshot);
                return Err(error);
            }
        };
        dag.apply_proposer_final_chain_facts(
            &request.snapshot,
            facts.last_finalized_period,
            facts.authorization_facts,
            revalidated_params,
            request.initially_loaded_params,
        )
    }

    /// Removes only the unchanged cursor that owned a failed FinalChain request.
    pub fn abort_proposer_final_chain_facts(
        &self,
        request: &DagProposerFinalChainRequest,
    ) -> Result<bool> {
        Ok(self
            .dag
            .lock()?
            .cleanup_proposer_final_chain_facts(&request.snapshot))
    }

    /// Polls the active native VDF stage without retaining a guard across executor work.
    pub fn poll_proposer_vdf(&self, session_id: u64) -> Result<DagProposerSessionStep> {
        Ok(self.dag.lock()?.proposer_session_poll_vdf(session_id))
    }

    /// Applies an external VDF proof report to the exact native cursor.
    pub fn report_proposer_vdf_proof(
        &self,
        session_id: u64,
        report: DagProposerVdfProofReport,
    ) -> Result<DagProposerSessionStep> {
        self.dag
            .lock()?
            .report_proposer_vdf_proof(session_id, report)
    }

    /// Resumes one stale-proof cursor after the retained external sleep.
    pub fn resume_proposer_stale_proof(&self, session_id: u64) -> Result<DagProposerSessionStep> {
        self.dag.lock()?.resume_proposer_stale_proof(session_id)
    }

    /// Applies an external signing report to the exact native cursor.
    pub fn report_proposer_signing(
        &self,
        session_id: u64,
        report: DagProposerSigningReport,
    ) -> Result<DagProposerSessionStep> {
        self.dag.lock()?.report_proposer_signing(session_id, report)
    }

    /// Applies retained add-block executor facts and completes the native cursor.
    pub fn report_proposer_add_block(
        &self,
        session_id: u64,
        report: DagProposerAddBlockReport,
    ) -> Result<DagProposerSessionStep> {
        Ok(self
            .dag
            .lock()?
            .report_proposer_add_block(session_id, report))
    }

    /// Locks the transaction sibling for one short-lived native task.
    ///
    /// Coupled tasks must use [`Self::lock_dag_and_transaction`] or explicitly
    /// acquire DAG and sortition first. The guard must not cross an external
    /// executor, callback, thread handoff, asynchronous boundary, or CXX return.
    #[doc(hidden)]
    pub fn lock_transaction(&self) -> Result<TransactionServiceGuard<'_>> {
        self.transaction.lock()
    }

    /// Persists and publishes one standalone DAG-block transaction set.
    ///
    /// The transaction sibling owns locking, durable batching, and post-commit
    /// publication. Facts are owned native values, so no guard or state
    /// reference crosses the application-root boundary.
    pub fn transaction_save_dag_transactions(
        &self,
        facts: Vec<crate::transaction_service::DagTransactionSaveInput>,
    ) -> Result<crate::transaction_service::DagTransactionSaveOutcome> {
        self.transaction.save_dag_transactions(facts)
    }

    /// Locks the DAG sibling for one short-lived native task.
    ///
    /// This is the first lock in every coupled operation. The guard must not
    /// cross an external executor, callback, sleep, thread handoff, asynchronous
    /// boundary, or CXX return.
    #[doc(hidden)]
    pub(crate) fn lock_dag(&self) -> Result<DagServiceGuard<'_>> {
        self.dag.lock()
    }

    /// Locks the sortition sibling for one short-lived native task.
    ///
    /// Coupled tasks acquire this only after DAG and before transaction. A
    /// standalone sortition task may acquire it directly.
    #[doc(hidden)]
    pub fn lock_sortition(&self) -> Result<SortitionServiceGuard<'_>> {
        self.sortition.lock()
    }

    /// Locks DAG and transaction state in their canonical relative order.
    ///
    /// Sortition is not part of this operation. A three-domain operation must
    /// acquire sortition between these two locks. If transaction locking fails,
    /// the DAG guard is dropped with the returned error.
    #[doc(hidden)]
    pub(crate) fn lock_dag_and_transaction(
        &self,
    ) -> Result<(DagServiceGuard<'_>, TransactionServiceGuard<'_>)> {
        let dag = self.dag.lock()?;
        let transaction = self.transaction.lock()?;
        Ok((dag, transaction))
    }

    /// Opens a native DAG verification cursor from canonical block facts.
    ///
    /// Storage-backed prechecks and cursor replacement are completed while the
    /// DAG owner is locked. No CXX carrier or external executor participates.
    pub fn begin_verify_block_session(&self, input: DagVerifyBlockSessionInput) -> Result<()> {
        self.dag.lock()?.begin_verify_block_session(input)
    }

    /// Returns the current native verifier instruction without advancing it.
    pub fn next_verify_block_session(&self) -> Result<DagVerifyBlockSessionStep> {
        Ok(self.dag.lock()?.verify_block_session_next())
    }

    /// Resolves the active transaction query from native queue, sidecar, and storage state.
    ///
    /// Locks are acquired DAG then transaction. The cursor does not advance;
    /// C++ may materialize the returned canonical payloads before reporting
    /// exact-period account facts.
    pub fn prepare_verify_block_transactions(
        &self,
    ) -> Result<DagVerifyBlockTransactionPreparation> {
        let (dag, transaction) = self.lock_dag_and_transaction()?;
        let query = dag.verify_block_transaction_query()?;
        let requests = query
            .hashes
            .iter()
            .enumerate()
            .map(
                |(input_index, hash)| TransactionServiceTransactionViewRequest {
                    input_index: input_index as u64,
                    hash: hash.to_fixed_bytes(),
                },
            )
            .collect();
        let plan = transaction.lookup_transaction_views(requests, query.hashes.len() as u64)?;
        Ok(DagVerifyBlockTransactionPreparation {
            cursor_id: query.cursor_id,
            proposal_period: query.proposal_period,
            transactions: plan.views,
        })
    }

    /// Revalidates a prepared transaction query and advances only when every view remains usable.
    ///
    /// Finalized storage payloads require explicit sender facts. Identity,
    /// lookup, or fact errors leave the active cursor unchanged for retry.
    pub fn complete_verify_block_transactions(
        &self,
        report: DagVerifyBlockTransactionCompletionReport,
    ) -> Result<DagVerifyBlockSessionStep> {
        let (mut dag, transaction) = self.lock_dag_and_transaction()?;
        let query = dag.verify_block_session_validate_transaction_completion(
            report.cursor_id,
            report.proposal_period,
        )?;
        let requests = query
            .hashes
            .iter()
            .enumerate()
            .map(
                |(input_index, hash)| TransactionServiceTransactionViewRequest {
                    input_index: input_index as u64,
                    hash: hash.to_fixed_bytes(),
                },
            )
            .collect();
        let plan = transaction.lookup_proposal_transaction_views_requiring_account_nonce_facts(
            query.proposal_period,
            requests,
            report.account_nonce_facts,
            query.hashes.len() as u64,
        )?;
        let all_resolved = plan.complete
            && plan.views.len() == query.hashes.len()
            && plan
                .views
                .iter()
                .all(|view| view.found && !view.old_finalized);
        let resolved_transactions = if all_resolved {
            query.expected_transactions
        } else {
            0
        };
        Ok(dag.verify_block_session_apply_transaction_resolution(resolved_transactions))
    }

    /// Prepares an unlocked FinalChain authorization query for the exact cursor.
    ///
    /// Sender decoding and recovery happen after releasing the DAG lock. A
    /// decode or recovery failure removes only the unchanged owning cursor.
    pub fn prepare_verify_block_authorization(
        &self,
    ) -> Result<DagVerifyBlockAuthorizationRequestOrStep> {
        let snapshot = {
            let mut dag = self.dag.lock()?;
            match dag.prepare_verify_block_authorization() {
                DagVerifyBlockAuthorizationPreparation::Snapshot(snapshot) => snapshot,
                DagVerifyBlockAuthorizationPreparation::Step(step) => {
                    return Ok(DagVerifyBlockAuthorizationRequestOrStep::Step(step));
                }
            }
        };
        let sender = match rustaxa_types::dag::DagBlock::try_from(
            rustaxa_types::codec::rlp::dag::DagBlockRlp::new(&snapshot.block_rlp),
        )
        .context("DAG_VERIFY_SESSION_AUTHORIZATION_BLOCK_DECODE")
        .and_then(|block| {
            block
                .recover_sender()
                .context("DAG_VERIFY_SESSION_AUTHORIZATION_SENDER_RECOVERY")
        }) {
            Ok(sender) => H160::from(sender.0),
            Err(error) => {
                self.dag
                    .lock()?
                    .cleanup_verify_block_authorization(&snapshot);
                return Err(error);
            }
        };
        Ok(DagVerifyBlockAuthorizationRequestOrStep::Request(
            DagVerifyBlockAuthorizationRequest {
                proposal_period: snapshot.proposal_period,
                sender,
                snapshot,
            },
        ))
    }

    /// Applies authorization facts to the exact prepared cursor.
    pub fn complete_verify_block_authorization(
        &self,
        request: &DagVerifyBlockAuthorizationRequest,
        facts: DagDposAuthorizationFacts,
    ) -> Result<DagVerifyBlockSessionStep> {
        self.dag
            .lock()?
            .apply_verify_block_authorization(&request.snapshot, facts)
    }

    /// Removes only the unchanged cursor that owned a failed authorization query.
    pub fn abort_verify_block_authorization(
        &self,
        request: &DagVerifyBlockAuthorizationRequest,
    ) -> Result<bool> {
        Ok(self
            .dag
            .lock()?
            .cleanup_verify_block_authorization(&request.snapshot))
    }

    /// Runs native VDF sortition verification between cursor snapshot and revalidation.
    ///
    /// DAG and sortition guards are released before proof work. Completion
    /// advances only the exact cursor/fingerprint/generation/action snapshot.
    pub fn verify_block_vdf(
        &self,
        request: DagVerifyBlockVdfRequest,
    ) -> Result<DagVerifyBlockSessionStep> {
        let snapshot = self
            .dag
            .lock()?
            .snapshot_verify_block_vdf(request.cursor_id)?;
        ensure!(
            keccak256_bytes(&request.block_rlp) == snapshot.fingerprint,
            "DAG_VERIFY_SESSION_VDF_REQUEST_FINGERPRINT_MISMATCH"
        );
        let decoded_block = dag_manager_block_from_rlp(&request.block_rlp);
        if let Ok(block) = decoded_block.as_ref() {
            ensure!(
                block.level == request.block_level,
                "DAG_VERIFY_SESSION_VDF_REQUEST_LEVEL_MISMATCH"
            );
        }
        let sortition_params = self
            .sortition
            .lock()?
            .params_for_period_from_storage(snapshot.proposal_period)
            .context("DAG_VERIFY_SESSION_VDF_SORTITION_PARAMS")?;
        let vdf_status = match decoded_block.and_then(|_| {
            verify_dag_vdf_sortition_from_block(DagVdfSortitionBlockInput {
                block_rlp: request.block_rlp,
                block_level: request.block_level,
                proposal_period_hash: request.proposal_period_hash,
                vrf_public_key: snapshot.vrf_public_key,
                sortition_params,
                sender_eligible_vote_count: snapshot.vote_count,
                vdf_sortition_max_vote_count: snapshot.max_vote_count,
            })
        }) {
            Ok(result) => result.vdf_status,
            Err(_) => crate::dag::DAG_VERIFY_VDF_STATUS_INVALID,
        };
        self.dag
            .lock()?
            .complete_verify_block_vdf(&snapshot, vdf_status)
    }

    /// Applies retained EVM gas facts to the active native verifier cursor.
    pub fn report_verify_block_gas(
        &self,
        report: DagVerifyBlockGasReport,
    ) -> Result<DagVerifyBlockSessionStep> {
        self.dag.lock()?.verify_block_session_report_gas(report)
    }

    /// Prepares one add-block transition without mutating live or durable state.
    ///
    /// The method acquires DAG then transaction, decodes and validates canonical
    /// payloads, and publishes at most one pending native cursor. Terminal
    /// duplicate, expired, or missing-reference outcomes return cursor zero.
    /// No external account lookup is performed while locks are held.
    pub fn prepare_add_block(
        &self,
        request: DagAddBlockPrepareRequest,
    ) -> Result<DagAddBlockPreparation> {
        let (mut dag, _transaction) = self.lock_dag_and_transaction()?;
        ensure!(
            dag.pending_add_block.is_none(),
            "DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"
        );
        let mut block = dag_manager_block_from_rlp(&request.block_rlp)
            .context("DAG_ADD_BLOCK_PREPARE_DECODE")?;
        if request.validate_hash {
            ensure!(
                block.hash == request.expected_hash,
                "DAG_ADD_BLOCK_PREPARE_HASH_MISMATCH"
            );
        } else {
            block.hash = request.expected_hash;
        }
        let plan = dag.plan_add_block(&block, request.save, request.proposed)?;
        if !plan.accepted || plan.duplicate || plan.expired {
            return Ok(DagAddBlockPreparation {
                cursor_id: 0,
                block_level: block.level,
                accepted: plan.accepted,
                duplicate: plan.duplicate,
                expired: plan.expired,
                missing_references: plan.missing_references,
                account_requests: Vec::new(),
            });
        }

        let mut transactions = Vec::new();
        let mut account_requests = Vec::new();
        if plan.persist_transactions {
            let expected_hashes = if request.validate_hash {
                let block_hashes = dag_block_transaction_hashes(&request.block_rlp)
                    .context("DAG_ADD_BLOCK_PREPARE_TRANSACTION_HASHES")?;
                ensure!(
                    block_hashes.len() == request.transactions.len(),
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_COUNT_MISMATCH"
                );
                block_hashes
            } else {
                request
                    .transactions
                    .iter()
                    .map(|payload| payload.hash)
                    .collect()
            };
            for (input_index, (expected_hash, payload)) in expected_hashes
                .into_iter()
                .zip(request.transactions)
                .enumerate()
            {
                ensure!(
                    expected_hash == payload.hash,
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_ORDER_MISMATCH"
                );
                let envelope = LegacyTransactionEnvelope::decode(&payload.transaction_rlp)
                    .context("DAG_ADD_BLOCK_PREPARE_TRANSACTION_DECODE")?;
                ensure!(
                    envelope.hash == payload.hash,
                    "DAG_ADD_BLOCK_PREPARE_TRANSACTION_HASH_MISMATCH"
                );
                let sender = envelope
                    .sender
                    .context("DAG_ADD_BLOCK_PREPARE_TRANSACTION_SENDER_MISSING")?;
                transactions.push(DagAddBlockPreparedTransaction {
                    input_index: input_index as u64,
                    hash: expected_hash,
                    trx_rlp: payload.transaction_rlp,
                    transaction_nonce: envelope.nonce.to_big_endian(),
                });
                account_requests.push(DagAddBlockAccountRequest {
                    input_index: input_index as u64,
                    sender,
                });
            }
        }

        let cursor_id = dag.next_add_block_session_id;
        dag.next_add_block_session_id = cursor_id.wrapping_add(1).max(1);
        let block_level = block.level;
        dag.pending_add_block = Some(DagAddBlockSession {
            cursor_id,
            block,
            block_rlp: request.block_rlp,
            save: request.save,
            proposed: request.proposed,
            transactions,
            plan: stored_add_block_plan(&plan),
        });
        Ok(DagAddBlockPreparation {
            cursor_id,
            block_level,
            accepted: true,
            duplicate: false,
            expired: false,
            missing_references: Vec::new(),
            account_requests,
        })
    }

    /// Completes one prepared add-block through a single shared durable batch.
    ///
    /// DAG and transaction next states are fully prevalidated before persistence.
    /// The pending cursor is retained when the batch commit fails. Neither live
    /// state is published before commit; after success both are published
    /// infallibly while DAG and transaction locks remain held.
    pub fn complete_add_block(
        &self,
        completion: DagAddBlockCompletion,
    ) -> Result<DagAddBlockCommitReport> {
        self.complete_add_block_with_commit(completion, |storage, batch| {
            storage.commit_write_batch_with_sync(batch, false)
        })
    }

    /// Prepares one owner-bound proposer transaction-pack stage.
    ///
    /// A throttled request validates and advances only the DAG cursor. Other
    /// requests acquire DAG then transaction, derive private proposal/shard
    /// parameters, snapshot the queue and cache, and either return
    /// executor-ready estimate requests or advance the DAG cursor immediately.
    /// Every guard is released before return. Any failure removes the matching
    /// DAG cursor and aborts only the matching transaction-packing owner.
    pub fn prepare_proposer_pack(
        &self,
        request: DagProposerPackPrepareRequest,
    ) -> Result<DagProposerPackStep> {
        if request.network_throttled {
            let mut dag = self.lock_dag()?;
            let result = (|| {
                let _ = dag.proposer_pack_parameters(request.session_id)?;
                let session = dag.apply_proposer_pack(request.session_id, true, Vec::new())?;
                Ok(DagProposerPackStep {
                    session,
                    estimate_requests: Vec::new(),
                })
            })();
            if result.is_err() {
                dag.abort_proposer_session(request.session_id);
            }
            return result;
        }

        let mut dag = self.lock_dag()?;
        let mut transaction = match self.lock_transaction() {
            Ok(transaction) => transaction,
            Err(error) => {
                dag.abort_proposer_session(request.session_id);
                return Err(error);
            }
        };
        let owner = TransactionPackingOwner::DagProposer(request.session_id);
        let result = (|| {
            let params = dag.proposer_pack_parameters(request.session_id)?;
            let prepared: TransactionServiceProposerPackPrepared = transaction
                .prepare_proposer_pack(
                    owner,
                    params,
                    request.min_transaction_gas,
                    request.estimate_gas_limit,
                    request.last_block_number,
                )?;
            let session = if prepared.request_estimates.is_empty() {
                dag.apply_proposer_pack(request.session_id, false, prepared.selected_transactions)?
            } else {
                dag.proposer_session_step(request.session_id)?
            };
            Ok(DagProposerPackStep {
                session,
                estimate_requests: prepared.request_estimates,
            })
        })();
        match result {
            Ok(report) => Ok(report),
            Err(error) => {
                let _ = transaction.abort_proposer_pack(owner);
                dag.abort_proposer_session(request.session_id);
                Err(error)
            }
        }
    }

    /// Finalizes one proposer pack after the unlocked EVM interval.
    ///
    /// Estimates must match the retained owner-bound candidate sequence exactly.
    /// The method acquires DAG then transaction, validates the DAG stage before
    /// transaction mutation, atomically publishes queue/cache effects, transfers
    /// selected canonical payloads into the DAG cursor, and returns its next
    /// instruction. Count, hash, owner, cache, or DAG errors clean both matching
    /// cursors while preserving any compatibility-owned packing session.
    pub fn finalize_proposer_pack(
        &self,
        session_id: u64,
        estimates: Vec<TransactionPackingEstimate>,
    ) -> Result<DagProposerPackStep> {
        let owner = TransactionPackingOwner::DagProposer(session_id);
        let mut dag = match self.lock_dag() {
            Ok(dag) => dag,
            Err(error) => {
                self.abort_transaction_pack_after_dag_lock_failure(owner);
                return Err(error);
            }
        };
        let mut transaction = match self.lock_transaction() {
            Ok(transaction) => transaction,
            Err(error) => {
                dag.abort_proposer_session(session_id);
                return Err(error);
            }
        };
        let result = (|| {
            let _ = dag.proposer_pack_parameters(session_id)?;
            let finalized: TransactionServiceProposerPackFinalized =
                transaction.finalize_proposer_pack(owner, estimates)?;
            let session =
                dag.apply_proposer_pack(session_id, false, finalized.selected_transactions)?;
            Ok(DagProposerPackStep {
                session,
                estimate_requests: Vec::new(),
            })
        })();
        match result {
            Ok(report) => Ok(report),
            Err(error) => {
                let _ = transaction.abort_proposer_pack(owner);
                let _ = dag.abort_proposer_session(session_id);
                Err(error)
            }
        }
    }

    /// Idempotently aborts one owner-bound proposer pack.
    ///
    /// DAG is locked before transaction. The return is true when either matching
    /// cursor was removed. Other transaction-packing owners are preserved. Lock
    /// poison is returned as a stable error; the DAG cursor is still removed when
    /// transaction locking fails.
    pub fn abort_proposer_pack(&self, session_id: u64) -> Result<bool> {
        let owner = TransactionPackingOwner::DagProposer(session_id);
        let mut dag = match self.lock_dag() {
            Ok(dag) => dag,
            Err(error) => {
                self.abort_transaction_pack_after_dag_lock_failure(owner);
                return Err(error);
            }
        };
        let mut transaction = match self.lock_transaction() {
            Ok(transaction) => transaction,
            Err(error) => {
                dag.abort_proposer_session(session_id);
                return Err(error);
            }
        };
        let transaction_result = transaction.abort_proposer_pack(owner);
        let dag_aborted = dag.abort_proposer_session(session_id);
        let transaction_aborted = transaction_result?;
        Ok(transaction_aborted || dag_aborted)
    }

    /// Best-effort cleanup for a transaction-packing cursor whose DAG sibling
    /// can no longer be locked.
    ///
    /// The caller retains and returns the original DAG poison error. Cleanup is
    /// owner-scoped so a compatibility cursor or another proposer session is
    /// never removed while the DAG domain is unavailable.
    fn abort_transaction_pack_after_dag_lock_failure(&self, owner: TransactionPackingOwner) {
        if let Ok(mut transaction) = self.lock_transaction() {
            let _ = transaction.abort_proposer_pack(owner);
        }
    }

    fn complete_add_block_with_commit(
        &self,
        completion: DagAddBlockCompletion,
        commit: impl FnOnce(&Storage, StorageWriteBatch) -> Result<()>,
    ) -> Result<DagAddBlockCommitReport> {
        let (mut dag, mut transaction) = self.lock_dag_and_transaction()?;
        let session = dag
            .pending_add_block
            .as_ref()
            .context("DAG_ADD_BLOCK_SESSION_NOT_STARTED")?
            .clone();
        ensure!(
            session.cursor_id == completion.cursor_id,
            "DAG_ADD_BLOCK_SESSION_CURSOR_MISMATCH"
        );
        let current_plan = dag.plan_add_block(&session.block, session.save, session.proposed)?;
        ensure!(
            stored_add_block_plan(&current_plan) == session.plan
                && current_plan.accepted
                && !current_plan.duplicate
                && !current_plan.expired,
            "DAG_ADD_BLOCK_SESSION_STALE_PLAN"
        );

        let mut nonce_facts = BTreeMap::new();
        for fact in completion.account_nonce_facts {
            ensure!(
                nonce_facts
                    .insert(fact.input_index, fact.account_nonce)
                    .is_none(),
                "DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_DUPLICATE"
            );
        }
        ensure!(
            nonce_facts.len() == session.transactions.len(),
            "DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_COUNT_MISMATCH"
        );
        let transaction_facts = session
            .transactions
            .iter()
            .map(|input| {
                Ok(DagTransactionSaveInput {
                    input_index: input.input_index,
                    hash: input.hash,
                    transaction_rlp: input.trx_rlp.clone(),
                    transaction_nonce: U256::from_big_endian(&input.transaction_nonce),
                    sender_account_nonce: *nonce_facts
                        .get(&input.input_index)
                        .context("DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_MISSING")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let prepared_transactions = session
            .plan
            .persist_transactions
            .then(|| prepare_dag_transactions(&transaction, transaction_facts))
            .transpose()?;
        let transaction_publication = prepared_transactions
            .as_ref()
            .map(|prepared| prepare_dag_transaction_publication(&transaction, prepared))
            .transpose()?;
        let mut next_dag_state = dag.state.clone();
        if session.plan.add_to_graph {
            next_dag_state
                .add_block(session.block.clone())
                .context("DAG_ADD_BLOCK_GRAPH_PREVALIDATE")?;
        }

        ensure!(
            Arc::ptr_eq(&dag.storage, &transaction.storage),
            "DAG_ADD_BLOCK_STORAGE_OWNER_MISMATCH"
        );
        let counters;
        let mut pending_batch = None;
        if session.plan.persist_block {
            let mut batch = dag.storage.create_write_batch();
            if let Some(prepared) = prepared_transactions.as_ref() {
                append_prepared_dag_transactions(dag.storage.as_ref(), &mut batch, prepared)?;
            }
            let (dag_blocks, dag_edges) = dag.storage.dag().append_write_to_batch(
                &mut batch,
                session.block.hash,
                session.block.level,
                session.block.tips.len() as u64,
                &session.block_rlp,
            )?;
            counters = DagPersistenceCounters {
                dag_blocks,
                dag_edges,
            };
            pending_batch = Some(batch);
        } else {
            counters = dag.persistence_counters()?;
        }

        let removed_session = dag
            .pending_add_block
            .take()
            .context("DAG_ADD_BLOCK_SESSION_DISAPPEARED_BEFORE_COMMIT")?;
        let commit_result = pending_batch
            .map(|batch| commit(dag.storage.as_ref(), batch))
            .transpose();
        if let Err(error) = commit_result {
            dag.pending_add_block = Some(removed_session);
            return Err(error).context("DAG_ADD_BLOCK_BATCH_COMMIT");
        }
        dag.state = next_dag_state;
        let queue_erased = transaction_publication
            .map(|publication| publish_dag_transactions(&mut transaction, publication))
            .into_iter()
            .flat_map(|outcome| outcome.accepted)
            .filter(|accepted| accepted.erased_from_queue)
            .map(|accepted| accepted.hash)
            .collect();
        Ok(DagAddBlockCommitReport {
            accepted: true,
            emit_verified: session.plan.emit_verified,
            gossip: session.plan.gossip,
            proposed: session.plan.proposed,
            queue_erased,
            counters,
        })
    }

    /// Idempotently aborts only the matching pending add-block cursor.
    pub fn abort_add_block(&self, cursor_id: u64) -> Result<bool> {
        let mut dag = self.lock_dag()?;
        if dag
            .pending_add_block
            .as_ref()
            .is_some_and(|session| session.cursor_id == cursor_id)
        {
            dag.pending_add_block = None;
            return Ok(true);
        }
        Ok(false)
    }

    /// Commits DAG finalization and clears sibling transaction sidecars.
    ///
    /// DAG cleanup owns the complete durable batch. The transaction sibling is
    /// mutated only after that commit succeeds, while both locks remain held in
    /// DAG-then-transaction order. Only retained external event facts escape.
    pub fn apply_finalized_order(
        &self,
        new_anchor: H256,
        new_period: u64,
        finalized_order: Vec<H256>,
    ) -> Result<DagFinalizationReport> {
        let (mut dag, mut transaction) = self.lock_dag_and_transaction()?;
        let committed = dag.apply_finalized_order(new_anchor, new_period, finalized_order)?;
        remove_non_finalized_sidecars_after_dag_commit(
            &mut transaction,
            &committed.remove_transaction_hashes,
        );
        Ok(DagFinalizationReport {
            finalized_count: committed.finalized_count,
            expired_hashes: committed.expired_hashes,
        })
    }
}

fn stored_add_block_plan(plan: &crate::dag::DagAddBlockEffectPlan) -> DagAddBlockStoredPlan {
    DagAddBlockStoredPlan {
        accepted: plan.accepted,
        persist_transactions: plan.persist_transactions,
        persist_block: plan.persist_block,
        add_to_graph: plan.add_to_graph,
        emit_verified: plan.emit_verified,
        gossip: plan.gossip,
        proposed: plan.proposed,
    }
}

fn keccak256_bytes(bytes: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut hash = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut hash);
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::{
        DAG_PROPOSER_ACTION_CONTINUE, DAG_VERIFY_REJECT_MISSING_TRANSACTION, DagFrontier,
        DagManagerBlock, DagProposerAttemptPlan, DagProposerFrontierFacts,
        ensure_proposal_period_mapping, save_dag_block_to_storage,
    };
    use crate::dag_service::{
        DagProposerObservation, DagProposerRetryState, DagProposerSession,
        DagProposerSessionAction, DagProposerSessionBeginInput, DagProposerTransactionObservation,
        DagVerifyBlockSession, DagVerifyBlockSessionAction,
    };
    use crate::gas_pricer::GasPricerConfig;
    use crate::sortition::{HUNDRED_PERCENT, SortitionParams, VdfParams, VrfParams};
    use crate::transaction_queue::TransactionQueueEntry;
    use crate::transaction_service::{
        TM_TRANSACTION_VIEW_SOURCE_NON_FINALIZED_SIDECAR, TM_TRANSACTION_VIEW_SOURCE_QUEUE,
    };
    use anyhow::{Result, anyhow};
    use ethereum_types::{H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::Config;
    use std::path::PathBuf;
    use std::sync::Barrier;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tiny_keccak::{Hasher, Keccak};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn service_config() -> DagTransactionServiceConfig {
        DagTransactionServiceConfig {
            transaction: TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: U256::one(),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
            dag: DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
            sortition: SortitionConfig {
                params: SortitionParams {
                    vrf: VrfParams {
                        threshold_upper: 0x100,
                    },
                    vdf: VdfParams {
                        difficulty_min: 1,
                        difficulty_max: 10,
                        difficulty_stale: 5,
                        lambda_bound: 100,
                    },
                },
                changes_count_for_average: 8,
                dag_efficiency_targets: (HUNDRED_PERCENT / 2, HUNDRED_PERCENT),
                changing_interval: 10,
                computation_interval: 5,
            },
        }
    }

    fn keccak256(bytes: &[u8]) -> H256 {
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(bytes);
        hasher.finalize(&mut output);
        H256(output)
    }

    fn signed_legacy_transaction_rlp(signing_key: &SigningKey) -> Vec<u8> {
        let chain_id = 2999_u64;
        let mut unsigned = RlpStream::new_list(9);
        unsigned.append(&U256::from(1));
        unsigned.append(&U256::from(2));
        unsigned.append(&21_000_u64);
        unsigned.append(&H160::repeat_byte(0x44));
        unsigned.append(&U256::from(3));
        unsigned.append(&Vec::<u8>::new());
        unsigned.append(&U256::from(chain_id));
        unsigned.append(&U256::zero());
        unsigned.append(&U256::zero());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(keccak256(&unsigned.out()).as_bytes())
            .expect("test transaction signing must succeed");
        let signature = signature.to_bytes();
        let mut signed = RlpStream::new_list(9);
        signed.append(&U256::from(1));
        signed.append(&U256::from(2));
        signed.append(&21_000_u64);
        signed.append(&H160::repeat_byte(0x44));
        signed.append(&U256::from(3));
        signed.append(&Vec::<u8>::new());
        signed.append(&U256::from(
            chain_id * 2 + 35 + u64::from(recovery_id.to_byte()),
        ));
        signed.append(&U256::from_big_endian(&signature[..32]));
        signed.append(&U256::from_big_endian(&signature[32..]));
        signed.out().to_vec()
    }

    fn composed_add_block_rlp(pivot: H256, level: u64, transactions: &[H256]) -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11_u8; 80]);
        vdf.append(&vec![0x22_u8]);
        vdf.append(&vec![0x33_u8]);
        vdf.append(&1_u16);
        let mut block = RlpStream::new_list(8);
        block.append(&pivot);
        block.append(&level);
        block.append(&0_u64);
        block.append(&vdf.out().to_vec());
        block.begin_list(0);
        block.begin_list(transactions.len());
        for hash in transactions {
            block.append(hash);
        }
        block.append(&&[0_u8; 65][..]);
        block.append(&0_u64);
        block.out().to_vec()
    }

    fn add_block_request(block_rlp: Vec<u8>, save: bool) -> DagAddBlockPrepareRequest {
        DagAddBlockPrepareRequest {
            expected_hash: keccak256(&block_rlp),
            block_rlp,
            validate_hash: true,
            save,
            proposed: false,
            transactions: Vec::new(),
        }
    }

    fn period_data_with_transaction_rlps(transactions: &[Vec<u8>]) -> Vec<u8> {
        let mut txs = RlpStream::new_list(transactions.len());
        for tx in transactions {
            txs.append_raw(tx, 1);
        }
        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.out().to_vec()
    }

    fn install_pack_session(root: &DagTransactionService, session_id: u64) -> Result<()> {
        let sortition_params = service_config().sortition.params;
        let frontier = DagFrontier {
            pivot: H256::repeat_byte(1),
            tips: Vec::new(),
        };
        let frontier_facts = DagProposerFrontierFacts {
            frontier: frontier.clone(),
            propose_level: 1,
            anchor: H256::repeat_byte(1),
            non_finalized_block_count: 0,
            non_finalized_min_difficulty: u32::MAX,
        };
        root.lock_dag()?.proposer_sessions.insert(
            session_id,
            DagProposerSession {
                action: DagProposerSessionAction::PackTransactions,
                begin_input: DagProposerSessionBeginInput {
                    max_non_finalized_transactions: 100,
                    dag_expiry_level_limit: 100,
                    wallet_vrf_public_key: [2; 32],
                    wallet_vrf_secret: [3; 64],
                    proposer_address: [4; 20],
                    max_non_finalized_dag_blocks: 100,
                    max_non_finalized_dag_blocks_low_difficulty: 50,
                    max_retry_count: 20,
                    proposal_weight_limit: 100_000,
                    total_transaction_shards: 1,
                    node_transaction_shard: 0,
                    shard_period_interval: 10,
                    pbft_gas_limit: 1_000_000,
                    dag_gas_limit: 100_000,
                    max_tips: 16,
                },
                transaction_observation: DagProposerTransactionObservation {
                    transaction_pool_size: 1,
                    non_finalized_transaction_count: 0,
                },
                observation: DagProposerObservation {
                    frontier: frontier_facts.clone(),
                    proposal_period_found: true,
                    proposal_period: 0,
                    period_block_hash_found: true,
                    period_block_hash: H256::zero(),
                    fingerprint: [5; 32],
                },
                attempt: DagProposerAttemptPlan {
                    action: DAG_PROPOSER_ACTION_CONTINUE,
                    reason_code: 0,
                    frontier,
                    anchor: H256::repeat_byte(1),
                    proposal_level: 1,
                    proposal_period_found: true,
                    proposal_period: 0,
                    last_finalized_period: 0,
                    period_block_hash_found: true,
                    period_block_hash: H256::zero(),
                    vrf_input: Vec::new(),
                    vote_count: 1,
                    max_vote_count: 1,
                    vdf_difficulty: 1,
                    vdf_stale: false,
                    old_proposal: false,
                    update_retry_state: false,
                    next_last_propose_level: 0,
                    next_retry_count: 0,
                    proposal_weight_limit: 100_000,
                    total_transaction_shards: 1,
                    node_transaction_shard: 0,
                    shard_period_interval: 10,
                },
                retry_key: [2; 32],
                minimum_vdf_difficulty: 1,
                sortition_params,
                status: 0,
                reason_code: 0,
                return_value: false,
                update_retry_state: false,
                next_last_propose_level: 0,
                next_retry_count: 0,
                record_proposed_block: false,
                vdf_message: Vec::new(),
                selected_transaction_hashes: Vec::new(),
                transaction_gas_estimations: Vec::new(),
                selected_transactions: Vec::new(),
                vdf_rlp: Vec::new(),
                unsigned_intent: None,
                signed_intent: None,
                error_code: String::new(),
            },
        );
        Ok(())
    }

    fn insert_pack_transaction(
        root: &DagTransactionService,
        signing_key: &SigningKey,
    ) -> Result<LegacyTransactionEnvelope> {
        let transaction_rlp = signed_legacy_transaction_rlp(signing_key);
        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)?;
        root.lock_transaction()?.queue.insert(
            TransactionQueueEntry {
                hash: envelope.hash,
                sender: envelope.sender.context("test transaction sender")?,
                nonce: envelope.nonce,
                gas_price: envelope.gas_price,
                gas: envelope.gas,
                data_size: envelope.data.len() as u64,
                rlp: transaction_rlp,
                last_block_number: 0,
            },
            true,
        )?;
        Ok(envelope)
    }

    fn insert_verify_transaction(
        root: &DagTransactionService,
        signing_key: &SigningKey,
    ) -> Result<LegacyTransactionEnvelope> {
        let transaction_rlp = signed_legacy_transaction_rlp(signing_key);
        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)?;
        let sender = envelope.sender.context("test transaction sender")?;
        root.lock_transaction()?.queue.insert(
            TransactionQueueEntry {
                hash: envelope.hash,
                sender,
                nonce: envelope.nonce,
                gas_price: envelope.gas_price,
                gas: envelope.gas,
                data_size: envelope.data.len() as u64,
                rlp: transaction_rlp,
                last_block_number: 0,
            },
            true,
        )?;
        Ok(envelope)
    }

    #[test]
    fn restore_publishes_all_siblings_with_one_storage_owner() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_root");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;

        let (dag, transaction) = root.lock_dag_and_transaction()?;
        assert!(Arc::ptr_eq(&dag.storage, &storage));
        assert!(Arc::ptr_eq(&transaction.storage, &storage));
        assert_eq!(dag.state.vertex_count(), 1);
        assert_eq!(transaction.sidecar.transaction_count(), 0);
        drop(transaction);
        drop(dag);
        assert_eq!(
            root.lock_sortition()?.current_params().vrf.threshold_upper,
            0x100
        );

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn dag_query_tasks_return_owned_lock_consistent_domain_snapshots() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_query_tasks");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let genesis = service_config().dag.genesis_hash;

        let status = root.dag_runtime_status()?;
        assert_eq!(status.vertex_count, 1);
        assert_eq!(status.edge_count, 0);
        assert_eq!(status.max_level, 0);
        assert_eq!(status.period, 0);
        assert_eq!(status.anchors.current, genesis);
        assert_eq!(status.expiry_limit, 32);

        let non_finalized = root.dag_non_finalized_index()?;
        let summary = root.dag_non_finalized_summary()?;
        assert!(non_finalized.levels.is_empty());
        assert_eq!(summary.levels, 0);
        assert_eq!(summary.blocks, 0);
        assert_eq!(summary.min_difficulty, u32::MAX);

        let validation = root.dag_validate_references(1, genesis, Vec::new())?;
        assert!(validation.ok);
        assert_eq!(validation.expected_level, 1);
        assert!(root.dag_is_block_known(genesis)?);

        let sync = root.dag_non_finalized_sync(Vec::new())?;
        assert_eq!(sync.period, 0);
        assert!(sync.storage.blocks.is_empty());
        assert!(sync.storage.transactions.is_empty());
        assert!(
            root.dag_graphviz(DagGraphView::Complete)?
                .contains("digraph")
        );

        let malformed_hash = H256::repeat_byte(3);
        root.lock_dag()?.state.add_block(DagManagerBlock {
            hash: malformed_hash,
            pivot: genesis,
            tips: Vec::new(),
            level: 1,
            difficulty: 1,
        })?;
        save_dag_block_to_storage(storage.as_ref(), malformed_hash, 1, 0, &[0x80])?;
        let error = root
            .dag_non_finalized_sync(Vec::new())
            .expect_err("malformed selected block must fail the complete sync snapshot");
        assert!(
            error
                .to_string()
                .contains("DAG_RUNTIME_SYNC_STORAGE_PAYLOAD")
        );

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_is_restart_safe_and_keeps_initial_mapping_idempotent() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_restart");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);

        for _ in 0..2 {
            let root = DagTransactionService::restore(storage.clone(), service_config())?;
            assert_eq!(root.lock_dag()?.state.vertex_count(), 1);
            assert!(!ensure_proposal_period_mapping(storage.as_ref(), 100, 0)?);
            drop(root);
        }

        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_preserves_transaction_then_dag_then_sortition_error_precedence() -> Result<()> {
        let transaction_path =
            unique_temp_dir("rustaxa_consensus_dag_transaction_order_transaction");
        let transaction_storage = Arc::new(Storage::new(Config::new(transaction_path.clone()))?);
        let mut invalid_transaction = service_config();
        invalid_transaction.transaction.gas_pricer_config.percentile = 101;
        invalid_transaction.dag.genesis_hash = H256::zero();
        invalid_transaction.sortition.computation_interval = 11;
        let transaction_error =
            DagTransactionService::restore(transaction_storage.clone(), invalid_transaction)
                .err()
                .expect("transaction configuration must fail first");
        assert!(transaction_error.to_string().contains("percentile"));
        drop(transaction_storage);
        std::fs::remove_dir_all(transaction_path)?;

        let dag_path = unique_temp_dir("rustaxa_consensus_dag_transaction_order_dag");
        let dag_storage = Arc::new(Storage::new(Config::new(dag_path.clone()))?);
        let mut invalid_dag = service_config();
        invalid_dag.dag.genesis_hash = H256::zero();
        invalid_dag.sortition.computation_interval = 11;
        let dag_error = DagTransactionService::restore(dag_storage.clone(), invalid_dag)
            .err()
            .expect("DAG configuration must fail before sortition");
        assert!(dag_error.to_string().contains("nonzero genesis"));
        drop(dag_storage);
        std::fs::remove_dir_all(dag_path)?;

        let sortition_path = unique_temp_dir("rustaxa_consensus_dag_transaction_order_sortition");
        let sortition_storage = Arc::new(Storage::new(Config::new(sortition_path.clone()))?);
        let mut invalid_sortition = service_config();
        invalid_sortition.sortition.computation_interval = 11;
        let sortition_error =
            DagTransactionService::restore(sortition_storage.clone(), invalid_sortition)
                .err()
                .expect("sortition configuration must fail last");
        assert!(
            sortition_error
                .to_string()
                .contains("SORTITION_STORAGE_CREATE_RUNTIME")
        );
        drop(sortition_storage);
        std::fs::remove_dir_all(sortition_path)?;
        Ok(())
    }

    #[test]
    fn add_block_commits_once_and_restores_native_state() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_add_block");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let block_rlp = composed_add_block_rlp(H256::repeat_byte(1), 1, &[]);

        let preparation = root.prepare_add_block(add_block_request(block_rlp.clone(), true))?;
        assert!(preparation.accepted);
        assert_ne!(preparation.cursor_id, 0);
        assert!(preparation.account_requests.is_empty());
        let report = root.complete_add_block(DagAddBlockCompletion {
            cursor_id: preparation.cursor_id,
            account_nonce_facts: Vec::new(),
        })?;
        assert!(report.accepted);
        assert!(report.emit_verified);
        assert!(report.gossip);
        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 2);

        let duplicate = root.prepare_add_block(add_block_request(block_rlp, true))?;
        assert!(duplicate.accepted);
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.cursor_id, 0);
        drop(root);
        drop(storage);

        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let restored = DagTransactionService::restore(storage.clone(), service_config())?;
        assert_eq!(restored.lock_dag()?.state.vertex_count(), 2);
        assert_eq!(restored.lock_dag()?.persistence_counters()?.dag_blocks, 1);
        drop(restored);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_commit_failure_retains_cursor_and_publishes_neither_state() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_commit_failure");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let transaction_rlp = signed_legacy_transaction_rlp(
            &SigningKey::from_slice(&[0x42; 32]).expect("valid test signing key"),
        );
        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)?;
        let sender = envelope.sender.context("test transaction sender")?;
        root.lock_transaction()?.queue.insert(
            TransactionQueueEntry {
                hash: envelope.hash,
                sender,
                nonce: envelope.nonce,
                gas_price: envelope.gas_price,
                gas: envelope.gas,
                data_size: envelope.data.len() as u64,
                rlp: transaction_rlp.clone(),
                last_block_number: 0,
            },
            true,
        )?;
        let block_rlp = composed_add_block_rlp(
            H256::repeat_byte(1),
            1,
            std::slice::from_ref(&envelope.hash),
        );
        let preparation = root.prepare_add_block(DagAddBlockPrepareRequest {
            expected_hash: keccak256(&block_rlp),
            block_rlp,
            validate_hash: true,
            save: true,
            proposed: false,
            transactions: vec![DagAddBlockTransactionPayload {
                hash: envelope.hash,
                transaction_rlp: transaction_rlp.clone(),
            }],
        })?;
        assert_eq!(preparation.account_requests.len(), 1);
        assert_eq!(preparation.account_requests[0].sender, sender);

        let error = root
            .complete_add_block_with_commit(
                DagAddBlockCompletion {
                    cursor_id: preparation.cursor_id,
                    account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                        input_index: 0,
                        account_nonce: U256::zero(),
                    }],
                },
                |_storage, _batch| Err(anyhow!("injected commit failure")),
            )
            .expect_err("the injected durable commit must fail");
        assert!(error.to_string().contains("DAG_ADD_BLOCK_BATCH_COMMIT"));
        {
            let (dag, transaction) = root.lock_dag_and_transaction()?;
            assert_eq!(dag.state.vertex_count(), 1);
            assert_eq!(dag.persistence_counters()?.dag_blocks, 0);
            assert_eq!(
                dag.pending_add_block
                    .as_ref()
                    .map(|session| session.cursor_id),
                Some(preparation.cursor_id)
            );
            assert_eq!(transaction.sidecar.transaction_count(), 0);
            assert!(transaction.queue.contains(envelope.hash));
            assert!(!transaction.sidecar.contains_non_finalized(envelope.hash));
        }
        assert!(storage.transaction().rlp(envelope.hash)?.is_none());

        let report = root.complete_add_block(DagAddBlockCompletion {
            cursor_id: preparation.cursor_id,
            account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                input_index: 0,
                account_nonce: U256::zero(),
            }],
        })?;
        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(report.queue_erased, vec![envelope.hash]);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 2);
        {
            let transaction_state = root.lock_transaction()?;
            assert_eq!(transaction_state.sidecar.transaction_count(), 1);
            assert!(!transaction_state.queue.contains(envelope.hash));
            assert!(
                transaction_state
                    .sidecar
                    .contains_non_finalized(envelope.hash)
            );
        }
        assert_eq!(
            storage.transaction().rlp(envelope.hash)?,
            Some(transaction_rlp)
        );
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_cursor_rejects_overlap_and_abort_is_stale_safe() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_cursor");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let block_rlp = composed_add_block_rlp(H256::repeat_byte(1), 1, &[]);
        let first = root.prepare_add_block(add_block_request(block_rlp.clone(), true))?;

        let overlap = root
            .prepare_add_block(add_block_request(block_rlp.clone(), false))
            .expect_err("a second prepare must not replace the active cursor");
        assert!(
            overlap
                .to_string()
                .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE")
        );
        let terminal = root
            .prepare_add_block(DagAddBlockPrepareRequest {
                expected_hash: H256::repeat_byte(0x77),
                block_rlp: composed_add_block_rlp(H256::repeat_byte(0x77), 1, &[]),
                validate_hash: true,
                save: true,
                proposed: false,
                transactions: Vec::new(),
            })
            .expect_err("a terminal second prepare must preserve the active cursor");
        assert!(
            terminal
                .to_string()
                .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE")
        );
        let malformed = root
            .prepare_add_block(DagAddBlockPrepareRequest {
                expected_hash: H256::zero(),
                block_rlp: vec![0x80],
                validate_hash: true,
                save: true,
                proposed: false,
                transactions: Vec::new(),
            })
            .expect_err("malformed second prepare must preserve the active cursor");
        assert!(
            malformed
                .to_string()
                .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE")
        );
        assert!(!root.abort_add_block(first.cursor_id + 1)?);
        assert!(root.abort_add_block(first.cursor_id)?);
        assert!(!root.abort_add_block(first.cursor_id)?);

        let second = root.prepare_add_block(add_block_request(block_rlp, false))?;
        assert_ne!(second.cursor_id, first.cursor_id);
        let stale = root
            .complete_add_block(DagAddBlockCompletion {
                cursor_id: first.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .expect_err("a stale cursor must not consume the active session");
        assert!(
            stale
                .to_string()
                .contains("DAG_ADD_BLOCK_SESSION_CURSOR_MISMATCH")
        );
        assert!(root.abort_add_block(second.cursor_id)?);
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_object_identity_persists_only_supplied_transactions() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_add_block_object_identity");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let omitted_transaction_hash = H256::repeat_byte(0xBD);
        let canonical_block_rlp = composed_add_block_rlp(
            H256::repeat_byte(1),
            1,
            std::slice::from_ref(&omitted_transaction_hash),
        );
        let canonical_hash = H256::from(keccak256(&canonical_block_rlp));
        let object_hash = H256::repeat_byte(0xBE);
        let preparation = root.prepare_add_block(DagAddBlockPrepareRequest {
            expected_hash: object_hash,
            validate_hash: false,
            block_rlp: canonical_block_rlp.clone(),
            save: true,
            proposed: false,
            transactions: Vec::new(),
        })?;
        assert!(preparation.accepted);
        let report = root.complete_add_block(DagAddBlockCompletion {
            cursor_id: preparation.cursor_id,
            account_nonce_facts: Vec::new(),
        })?;
        assert_eq!(report.counters.dag_blocks, 1);
        assert!(root.dag_is_block_known(object_hash)?);
        assert!(!root.dag_is_block_known(canonical_hash)?);
        assert!(root.dag_load_block(object_hash)?.block_rlp == canonical_block_rlp);
        assert!(!root.dag_load_block(canonical_hash)?.found);
        assert_eq!(root.lock_transaction()?.sidecar.transaction_count(), 0);
        assert!(
            storage
                .transaction()
                .rlp(omitted_transaction_hash)?
                .is_none()
        );
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_completion_rejects_duplicate_nonce_facts() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_add_block_nonce_duplicates");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let key_0 = SigningKey::from_slice(&[0x70; 32]).expect("test signing key");
        let key_1 = SigningKey::from_slice(&[0x71; 32]).expect("test signing key");
        let transaction_0 = signed_legacy_transaction_rlp(&key_0);
        let transaction_1 = signed_legacy_transaction_rlp(&key_1);
        let envelope_0 = LegacyTransactionEnvelope::decode(&transaction_0)?;
        let envelope_1 = LegacyTransactionEnvelope::decode(&transaction_1)?;
        let block_rlp =
            composed_add_block_rlp(H256::repeat_byte(1), 1, &[envelope_0.hash, envelope_1.hash]);
        let block_hash = keccak256(&block_rlp);
        let preparation = root.prepare_add_block(DagAddBlockPrepareRequest {
            expected_hash: block_hash,
            block_rlp,
            validate_hash: true,
            save: true,
            proposed: false,
            transactions: vec![
                DagAddBlockTransactionPayload {
                    hash: envelope_0.hash,
                    transaction_rlp: transaction_0,
                },
                DagAddBlockTransactionPayload {
                    hash: envelope_1.hash,
                    transaction_rlp: transaction_1,
                },
            ],
        })?;
        assert!(preparation.accepted);
        let error = root
            .complete_add_block(DagAddBlockCompletion {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: vec![
                    DagAddBlockAccountNonceFact {
                        input_index: 0,
                        account_nonce: U256::zero(),
                    },
                    DagAddBlockAccountNonceFact {
                        input_index: 0,
                        account_nonce: U256::zero(),
                    },
                ],
            })
            .expect_err("duplicate nonce facts must be rejected");
        assert!(
            error
                .to_string()
                .contains("DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_DUPLICATE")
        );
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_completion_rejects_nonce_fact_count_mismatch() -> Result<()> {
        let path =
            unique_temp_dir("rustaxa_consensus_dag_transaction_add_block_nonce_count_mismatch");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let key_0 = SigningKey::from_slice(&[0x72; 32]).expect("test signing key");
        let key_1 = SigningKey::from_slice(&[0x73; 32]).expect("test signing key");
        let transaction_0 = signed_legacy_transaction_rlp(&key_0);
        let transaction_1 = signed_legacy_transaction_rlp(&key_1);
        let envelope_0 = LegacyTransactionEnvelope::decode(&transaction_0)?;
        let envelope_1 = LegacyTransactionEnvelope::decode(&transaction_1)?;
        let block_rlp =
            composed_add_block_rlp(H256::repeat_byte(1), 1, &[envelope_0.hash, envelope_1.hash]);
        let block_hash = keccak256(&block_rlp);
        let preparation = root.prepare_add_block(DagAddBlockPrepareRequest {
            expected_hash: block_hash,
            block_rlp,
            validate_hash: true,
            save: true,
            proposed: false,
            transactions: vec![
                DagAddBlockTransactionPayload {
                    hash: envelope_0.hash,
                    transaction_rlp: transaction_0,
                },
                DagAddBlockTransactionPayload {
                    hash: envelope_1.hash,
                    transaction_rlp: transaction_1,
                },
            ],
        })?;
        assert!(preparation.accepted);
        let error = root
            .complete_add_block(DagAddBlockCompletion {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                    input_index: 0,
                    account_nonce: U256::zero(),
                }],
            })
            .expect_err("mismatched nonce fact counts must be rejected");
        assert!(
            error
                .to_string()
                .contains("DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_COUNT_MISMATCH")
        );
        assert_eq!(root.lock_dag()?.state.vertex_count(), 1);
        assert_eq!(root.lock_transaction()?.sidecar.transaction_count(), 0);
        assert!(!root.dag_load_block(block_hash)?.found);
        assert!(storage.transaction().rlp(envelope_0.hash)?.is_none());
        assert!(storage.transaction().rlp(envelope_1.hash)?.is_none());
        let report = root.complete_add_block(DagAddBlockCompletion {
            cursor_id: preparation.cursor_id,
            account_nonce_facts: vec![
                DagAddBlockAccountNonceFact {
                    input_index: 0,
                    account_nonce: U256::zero(),
                },
                DagAddBlockAccountNonceFact {
                    input_index: 1,
                    account_nonce: U256::zero(),
                },
            ],
        })?;
        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 2);
        assert_eq!(root.lock_transaction()?.sidecar.transaction_count(), 2);
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_finalized_sender_nonce_skips_transaction_publication() -> Result<()> {
        let path =
            unique_temp_dir("rustaxa_consensus_dag_transaction_finalized_sender_nonce_filter");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let key = SigningKey::from_slice(&[0x62; 32]).expect("test signing key");
        let transaction_rlp = signed_legacy_transaction_rlp(&key);
        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)?;
        storage
            .transaction()
            .write_location(envelope.hash, 1, 0, false)?;
        storage.period().write(
            1,
            &period_data_with_transaction_rlps(std::slice::from_ref(&transaction_rlp)),
        )?;
        let block_rlp = composed_add_block_rlp(
            H256::repeat_byte(1),
            1,
            std::slice::from_ref(&envelope.hash),
        );
        let preparation = root.prepare_add_block(DagAddBlockPrepareRequest {
            expected_hash: keccak256(&block_rlp),
            block_rlp,
            validate_hash: true,
            save: true,
            proposed: false,
            transactions: vec![DagAddBlockTransactionPayload {
                hash: envelope.hash,
                transaction_rlp: transaction_rlp.clone(),
            }],
        })?;
        let report = root.complete_add_block(DagAddBlockCompletion {
            cursor_id: preparation.cursor_id,
            account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                input_index: 0,
                account_nonce: U256::from(2_u64),
            }],
        })?;
        assert!(report.accepted);
        assert_eq!(report.queue_erased.len(), 0);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 2);
        {
            let transaction = root.lock_transaction()?;
            assert!(!transaction.queue.contains(envelope.hash));
            assert!(!transaction.sidecar.contains_non_finalized(envelope.hash));
        }
        assert!(storage.transaction().rlp(envelope.hash)?.is_none());
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_block_transaction_completion_rejects_stale_request_and_advances_when_complete()
    -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_verify_completion_stale");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let not_started = root
            .prepare_verify_block_transactions()
            .expect_err("transaction preparation must require a verifier session");
        assert!(
            not_started
                .to_string()
                .contains("DAG_VERIFY_SESSION_NOT_STARTED")
        );
        let envelope = insert_verify_transaction(
            &root,
            &SigningKey::from_slice(&[0x11; 32]).expect("valid test signing key"),
        )?;
        root.begin_verify_block_session(DagVerifyBlockSessionInput {
            block_hash: keccak256(&[1_u8]).to_fixed_bytes(),
            block_level: 1,
            pivot: [1; 32],
            tips: Vec::new(),
            block_transaction_hashes: vec![envelope.hash],
            supplied_transaction_hashes: vec![envelope.hash],
            block_rlp: Vec::new(),
        })?;

        let initial_step = root.next_verify_block_session()?;
        assert_eq!(initial_step.complete, false);
        let plan = root.prepare_verify_block_transactions()?;
        assert!(plan.transactions.is_empty());
        assert_eq!(
            root.next_verify_block_session()?.action,
            initial_step.action
        );
        let stale_step = root
            .complete_verify_block_transactions(DagVerifyBlockTransactionCompletionReport {
                cursor_id: plan.cursor_id + 1,
                proposal_period: plan.proposal_period,
                account_nonce_facts: Vec::new(),
            })
            .expect_err("stale cursor must fail without advancing");
        assert!(
            stale_step
                .to_string()
                .contains("DAG_VERIFY_SESSION_TRANSACTION_CURSOR_MISMATCH")
        );
        let replacement = root.next_verify_block_session()?;
        assert_eq!(replacement.action, initial_step.action);

        let wrong_period = root
            .complete_verify_block_transactions(DagVerifyBlockTransactionCompletionReport {
                cursor_id: plan.cursor_id,
                proposal_period: plan.proposal_period + 1,
                account_nonce_facts: Vec::new(),
            })
            .expect_err("wrong proposal period must fail without advancing");
        assert!(
            wrong_period
                .to_string()
                .contains("DAG_VERIFY_SESSION_TRANSACTION_PERIOD_MISMATCH")
        );
        assert_eq!(
            root.next_verify_block_session()?.action,
            initial_step.action
        );

        let resolved =
            root.complete_verify_block_transactions(DagVerifyBlockTransactionCompletionReport {
                cursor_id: plan.cursor_id,
                proposal_period: plan.proposal_period,
                account_nonce_facts: Vec::new(),
            })?;
        assert_eq!(resolved.complete, false);
        assert_eq!(resolved.action, 2);

        let wrong_stage = root
            .prepare_verify_block_transactions()
            .expect_err("transaction preparation must reject the authorization stage");
        assert!(
            wrong_stage
                .to_string()
                .contains("DAG_VERIFY_SESSION_UNEXPECTED_TRANSACTION_COMPLETION")
        );

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_block_transactions_resolve_queue_sidecar_and_missing_in_canonical_order() -> Result<()>
    {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_verify_sources");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let queue_key =
            SigningKey::from_slice(&[0x49; 32]).expect("valid queue transaction signing key");
        let queue_rlp = signed_legacy_transaction_rlp(&queue_key);
        let queue_envelope = insert_verify_transaction(&root, &queue_key)?;
        let sidecar_rlp = signed_legacy_transaction_rlp(
            &SigningKey::from_slice(&[0x4c; 32]).expect("valid sidecar transaction signing key"),
        );
        let sidecar_hash = keccak256(&sidecar_rlp);
        root.lock_transaction()?
            .sidecar
            .insert_non_finalized(sidecar_hash, sidecar_rlp.clone())?;
        let supplied_hash = H256::repeat_byte(0x08);
        root.begin_verify_block_session(DagVerifyBlockSessionInput {
            block_hash: keccak256(&[2_u8]).to_fixed_bytes(),
            block_level: 1,
            pivot: [1; 32],
            tips: Vec::new(),
            block_transaction_hashes: vec![
                supplied_hash,
                queue_envelope.hash,
                sidecar_hash,
                queue_envelope.hash,
            ],
            supplied_transaction_hashes: vec![supplied_hash],
            block_rlp: Vec::new(),
        })?;

        let preparation = root.prepare_verify_block_transactions()?;
        assert_eq!(preparation.transactions.len(), 2);
        assert_eq!(preparation.transactions[0].input_index, 0);
        assert_eq!(preparation.transactions[0].hash, queue_envelope.hash.0);
        assert_eq!(
            preparation.transactions[0].source,
            TM_TRANSACTION_VIEW_SOURCE_QUEUE
        );
        assert_eq!(preparation.transactions[0].tx_rlp, queue_rlp);
        assert_eq!(preparation.transactions[1].input_index, 1);
        assert_eq!(preparation.transactions[1].hash, sidecar_hash.0);
        assert_eq!(
            preparation.transactions[1].source,
            TM_TRANSACTION_VIEW_SOURCE_NON_FINALIZED_SIDECAR
        );
        assert_eq!(preparation.transactions[1].tx_rlp, sidecar_rlp);
        let resolved =
            root.complete_verify_block_transactions(DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            })?;
        assert!(!resolved.complete);
        assert_eq!(resolved.action, 2);

        let missing_hash = H256::repeat_byte(0x0a);
        root.begin_verify_block_session(DagVerifyBlockSessionInput {
            block_hash: keccak256(&[3_u8]).to_fixed_bytes(),
            block_level: 1,
            pivot: [1; 32],
            tips: Vec::new(),
            block_transaction_hashes: vec![missing_hash],
            supplied_transaction_hashes: Vec::new(),
            block_rlp: Vec::new(),
        })?;
        let missing = root.prepare_verify_block_transactions()?;
        assert_eq!(missing.transactions.len(), 1);
        assert_eq!(missing.transactions[0].hash, missing_hash.0);
        assert!(!missing.transactions[0].found);
        let rejected =
            root.complete_verify_block_transactions(DagVerifyBlockTransactionCompletionReport {
                cursor_id: missing.cursor_id,
                proposal_period: missing.proposal_period,
                account_nonce_facts: Vec::new(),
            })?;
        assert!(rejected.complete);
        assert_eq!(rejected.reject_code, DAG_VERIFY_REJECT_MISSING_TRANSACTION);

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_block_transactions_require_nonce_facts_and_reject_old_finalized() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_verify_finalized_nonce");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let signing_key =
            SigningKey::from_slice(&[0x4a; 32]).expect("valid finalized transaction signing key");
        let transaction_rlp = signed_legacy_transaction_rlp(&signing_key);
        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)?;
        let sender = envelope.sender.context("finalized transaction sender")?;
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        storage
            .transaction()
            .write_location(envelope.hash, 1, 0, false)?;
        storage
            .period()
            .write(1, &period_data_with_transaction_rlps(&[transaction_rlp]))?;
        root.begin_verify_block_session(DagVerifyBlockSessionInput {
            block_hash: keccak256(&[4_u8]).to_fixed_bytes(),
            block_level: 1,
            pivot: [1; 32],
            tips: Vec::new(),
            block_transaction_hashes: vec![envelope.hash],
            supplied_transaction_hashes: Vec::new(),
            block_rlp: Vec::new(),
        })?;

        let preparation = root.prepare_verify_block_transactions()?;
        assert_eq!(preparation.transactions.len(), 1);
        assert!(preparation.transactions[0].found);
        let missing_fact = root
            .complete_verify_block_transactions(DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            })
            .expect_err("finalized transaction must require an account nonce fact");
        assert!(
            missing_fact
                .to_string()
                .contains("TM_PROPOSAL_FINALIZED_ACCOUNT_NONCE_FACT_MISSING")
        );
        assert_eq!(root.next_verify_block_session()?.action, 1);

        let rejected =
            root.complete_verify_block_transactions(DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: vec![TransactionServiceAccountNonceFact {
                    sender: sender.0,
                    account_found: true,
                    account_nonce: (envelope.nonce + U256::one()).to_big_endian(),
                }],
            })?;
        assert!(rejected.complete);
        assert_eq!(rejected.reject_code, DAG_VERIFY_REJECT_MISSING_TRANSACTION);

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_block_authorization_stale_snapshot_rejects_and_preserves_replacement() -> Result<()> {
        let path =
            unique_temp_dir("rustaxa_consensus_dag_transaction_verify_authorization_snapshot");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let envelope = insert_verify_transaction(
            &root,
            &SigningKey::from_slice(&[0x22; 32]).expect("valid test signing key"),
        )?;
        root.begin_verify_block_session(DagVerifyBlockSessionInput {
            block_hash: keccak256(&[2_u8]).to_fixed_bytes(),
            block_level: 1,
            pivot: [1; 32],
            tips: Vec::new(),
            block_transaction_hashes: vec![envelope.hash],
            supplied_transaction_hashes: vec![envelope.hash],
            block_rlp: Vec::new(),
        })?;
        let plan = root.prepare_verify_block_transactions()?;
        let authorization =
            root.complete_verify_block_transactions(DagVerifyBlockTransactionCompletionReport {
                cursor_id: plan.cursor_id,
                proposal_period: plan.proposal_period,
                account_nonce_facts: Vec::new(),
            })?;
        assert_eq!(authorization.action, 2);

        let request = {
            let dag = root.lock_dag()?;
            let session = dag
                .verify_block_session
                .as_ref()
                .expect("verification session must be active for authorization snapshot");
            let proposal_period = session.proposal_period;
            let snapshot = DagVerifyBlockAuthorizationSnapshot {
                cursor_id: session.cursor_id,
                fingerprint: session.fingerprint,
                generation: session.generation,
                proposal_period: session.proposal_period,
                block_rlp: session.block_rlp.clone(),
            };
            DagVerifyBlockAuthorizationRequest {
                snapshot,
                proposal_period,
                sender: H160::repeat_byte(0x33),
            }
        };
        root.begin_verify_block_session(DagVerifyBlockSessionInput {
            block_hash: keccak256(&[3_u8]).to_fixed_bytes(),
            block_level: 1,
            pivot: [1; 32],
            tips: Vec::new(),
            block_transaction_hashes: vec![],
            supplied_transaction_hashes: vec![],
            block_rlp: Vec::new(),
        })?;
        let stale = root
            .complete_verify_block_authorization(
                &request,
                crate::dag::DagDposAuthorizationFacts {
                    vrf_key: None,
                    vrf_key_found: false,
                    sender_eligible_vote_count: 0,
                    vdf_sortition_max_vote_count: 0,
                    eligibility_status: crate::dag::DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
                },
            )
            .expect_err("replacement session must mismatch stale authorization request");
        assert!(
            stale
                .to_string()
                .contains("DAG_VERIFY_SESSION_AUTHORIZATION_CURSOR_MISMATCH")
        );
        let replacement = root.next_verify_block_session()?;
        assert_eq!(replacement.status, 0);
        assert_eq!(replacement.action, 1);

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_block_vdf_mismatch_fingerprint_rejects_without_advancing() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_verify_vdf_mismatch");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        {
            let mut dag = root.lock_dag()?;
            dag.verify_block_session = Some(DagVerifyBlockSession {
                cursor_id: 1,
                fingerprint: [0x11; 32],
                generation: 1,
                action: DagVerifyBlockSessionAction::VdfSortition {
                    vote_count: 4,
                    max_vote_count: 4,
                    vrf_public_key: [0x22; 32],
                },
                tips: Vec::new(),
                proposal_period: 0,
                block_rlp: Vec::new(),
                expected_transactions: 0,
                reject_code: 0,
                sender_eligible_vote_count: 0,
                vdf_sortition_max_vote_count: 4,
                eligibility_status: crate::dag::DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
                error_code: String::new(),
            });
        }

        let mismatch = root
            .verify_block_vdf(DagVerifyBlockVdfRequest {
                cursor_id: 1,
                block_rlp: vec![0x99],
                block_level: 1,
                proposal_period_hash: H256::zero(),
            })
            .expect_err("fingerprint mismatch must fail before completion");
        assert!(
            mismatch
                .to_string()
                .contains("DAG_VERIFY_SESSION_VDF_REQUEST_FINGERPRINT_MISMATCH")
        );
        let step = root.next_verify_block_session()?;
        assert!(!step.complete);
        assert_eq!(step.action, 3);

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn verify_block_gas_wrong_stage_returns_invalid_report_and_leaves_session() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_verify_wrong_stage_gas");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        root.begin_verify_block_session(DagVerifyBlockSessionInput {
            block_hash: keccak256(&[4_u8]).to_fixed_bytes(),
            block_level: 1,
            pivot: [1; 32],
            tips: Vec::new(),
            block_transaction_hashes: Vec::new(),
            supplied_transaction_hashes: Vec::new(),
            block_rlp: Vec::new(),
        })?;
        let step = root
            .report_verify_block_gas(DagVerifyBlockGasReport {
                block_gas_estimation: 0,
                estimated_transactions_weight: 0,
                dag_gas_limit: 0,
                pbft_gas_limit: 0,
            })
            .expect("wrong-stage gas report should return explicit invalid step");
        assert!(step.complete);
        assert_eq!(step.status, 2);
        let query = root.next_verify_block_session()?;
        assert_eq!(query.complete, true);

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn add_block_terminal_and_save_false_paths_do_not_persist() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_terminal");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;

        let missing = root.prepare_add_block(add_block_request(
            composed_add_block_rlp(H256::repeat_byte(9), 1, &[]),
            true,
        ))?;
        assert!(!missing.accepted);
        assert_eq!(missing.cursor_id, 0);
        assert_eq!(missing.missing_references, vec![H256::repeat_byte(9)]);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 1);

        let transient = root.prepare_add_block(add_block_request(
            composed_add_block_rlp(H256::repeat_byte(1), 1, &[]),
            false,
        ))?;
        let report = root.complete_add_block(DagAddBlockCompletion {
            cursor_id: transient.cursor_id,
            account_nonce_facts: Vec::new(),
        })?;
        assert_eq!(report.counters.dag_blocks, 0);
        assert_eq!(root.lock_dag()?.state.vertex_count(), 2);
        drop(root);
        drop(storage);

        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let restored = DagTransactionService::restore(storage.clone(), service_config())?;
        assert_eq!(restored.lock_dag()?.state.vertex_count(), 1);
        drop(restored);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn concurrent_add_block_prepares_publish_exactly_one_cursor() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_concurrent");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = Arc::new(DagTransactionService::restore(
            storage.clone(),
            service_config(),
        )?);
        let block_rlp = composed_add_block_rlp(H256::repeat_byte(1), 1, &[]);
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let root = root.clone();
            let block_rlp = block_rlp.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                root.prepare_add_block(add_block_request(block_rlp, false))
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| worker.join().expect("prepare worker must not panic"))
            .collect::<Vec<_>>();
        let cursors = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().ok().map(|prepared| prepared.cursor_id))
            .collect::<Vec<_>>();
        assert_eq!(cursors.len(), 1);
        assert_ne!(cursors[0], 0);
        let errors = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().err())
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0]
                .to_string()
                .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE")
        );
        assert!(root.abort_add_block(cursors[0])?);
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn proposer_pack_estimate_interval_is_unlocked_and_finalizes_natively() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_pack_estimate");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;
        let session_id = 41;
        install_pack_session(&root, session_id)?;
        let envelope = insert_pack_transaction(
            &root,
            &SigningKey::from_slice(&[0x47; 32]).expect("valid test signing key"),
        )?;

        let prepared = root.prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id,
            network_throttled: false,
            min_transaction_gas: 21_000,
            estimate_gas_limit: 0,
            last_block_number: 10,
        })?;
        assert_eq!(
            prepared.session.action,
            DagProposerSessionAction::PackTransactions
        );
        assert_eq!(prepared.estimate_requests.len(), 1);
        assert_eq!(prepared.estimate_requests[0].hash, envelope.hash);
        assert_eq!(
            prepared.estimate_requests[0].sender,
            envelope.sender.expect("test sender")
        );

        // The external EVM interval owns no DAG, transaction, or packing guard.
        assert!(root.lock_transaction()?.transaction_packing.is_active()?);

        let completed = root.finalize_proposer_pack(
            session_id,
            vec![TransactionPackingEstimate {
                hash: envelope.hash,
                gas_used: 21_000,
                last_block_number: 10,
                result_rlp: vec![0xC0],
            }],
        )?;
        assert_eq!(completed.session.action, DagProposerSessionAction::StartVdf);
        assert_eq!(
            completed.session.selected_transaction_hashes,
            vec![envelope.hash]
        );
        {
            let transaction = root.lock_transaction()?;
            assert!(!transaction.transaction_packing.is_active()?);
            assert_eq!(
                transaction
                    .sidecar
                    .gas_estimation_cache_get(envelope.hash, 0)?
                    .expect("estimate cache entry")
                    .gas_used,
                21_000
            );
        }
        assert!(root.abort_proposer_pack(session_id)?);

        install_pack_session(&root, 42)?;
        let cached = root.prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id: 42,
            network_throttled: false,
            min_transaction_gas: 21_000,
            estimate_gas_limit: 0,
            last_block_number: 10,
        })?;
        assert!(cached.estimate_requests.is_empty());
        assert_eq!(cached.session.action, DagProposerSessionAction::StartVdf);
        assert_eq!(
            cached.session.selected_transaction_hashes,
            vec![envelope.hash]
        );
        assert!(!root.lock_transaction()?.transaction_packing.is_active()?);
        assert!(root.abort_proposer_pack(42)?);
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn proposer_pack_terminal_and_failure_paths_clean_exact_cursors() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_pack_cleanup");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;

        install_pack_session(&root, 51)?;
        let throttled = root.prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id: 51,
            network_throttled: true,
            min_transaction_gas: 21_000,
            estimate_gas_limit: 0,
            last_block_number: 0,
        })?;
        assert_eq!(throttled.session.action, DagProposerSessionAction::Complete);
        assert!(!root.lock_transaction()?.transaction_packing.is_active()?);
        assert!(!root.abort_proposer_pack(51)?);

        install_pack_session(&root, 52)?;
        root.lock_dag()?.proposer_retry_states.insert(
            [2; 32],
            DagProposerRetryState {
                last_propose_level: 99,
                retry_count: 7,
                max_retry_count: 20,
            },
        );
        let empty = root.prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id: 52,
            network_throttled: false,
            min_transaction_gas: 21_000,
            estimate_gas_limit: 0,
            last_block_number: 0,
        })?;
        assert_eq!(empty.session.action, DagProposerSessionAction::Complete);
        assert!(empty.estimate_requests.is_empty());
        assert!(!root.lock_transaction()?.transaction_packing.is_active()?);
        {
            let dag = root.lock_dag()?;
            let retry = dag
                .proposer_retry_states
                .get(&[2; 32])
                .expect("matching proposer retry state");
            assert_eq!(retry.last_propose_level, 1);
            assert_eq!(retry.retry_count, 0);
            assert!(!dag.proposer_sessions.contains_key(&52));
        }

        install_pack_session(&root, 53)?;
        insert_pack_transaction(
            &root,
            &SigningKey::from_slice(&[0x48; 32]).expect("valid test signing key"),
        )?;
        let prepared = root.prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id: 53,
            network_throttled: false,
            min_transaction_gas: 21_000,
            estimate_gas_limit: 0,
            last_block_number: 0,
        })?;
        assert_eq!(prepared.estimate_requests.len(), 1);
        let error = root
            .finalize_proposer_pack(53, Vec::new())
            .expect_err("estimate count mismatch must terminate the composed task");
        assert!(error.to_string().contains("TM_RUNTIME_PACK_FINALIZE"));
        assert!(!root.lock_transaction()?.transaction_packing.is_active()?);
        assert!(!root.abort_proposer_pack(53)?);

        install_pack_session(&root, 54)?;
        let prepared = root.prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id: 54,
            network_throttled: false,
            min_transaction_gas: 21_000,
            estimate_gas_limit: 0,
            last_block_number: 0,
        })?;
        let wrong_hash = root
            .finalize_proposer_pack(
                54,
                vec![TransactionPackingEstimate {
                    hash: H256::repeat_byte(0x54),
                    gas_used: 21_000,
                    last_block_number: 0,
                    result_rlp: vec![0xC0],
                }],
            )
            .expect_err("estimate hash mismatch must terminate the composed task");
        assert_eq!(prepared.estimate_requests.len(), 1);
        assert!(format!("{wrong_hash:#}").contains("TM_RUNTIME_PACK_FINALIZE_HASH_MISMATCH"));
        assert!(!root.lock_transaction()?.transaction_packing.is_active()?);
        assert!(!root.abort_proposer_pack(54)?);

        install_pack_session(&root, 55)?;
        root.lock_dag()?
            .proposer_sessions
            .get_mut(&55)
            .expect("installed proposer cursor")
            .action = DagProposerSessionAction::CollectFinalChainFacts;
        let wrong_stage = root
            .prepare_proposer_pack(DagProposerPackPrepareRequest {
                session_id: 55,
                network_throttled: true,
                min_transaction_gas: 21_000,
                estimate_gas_limit: 0,
                last_block_number: 0,
            })
            .expect_err("wrong DAG stage must remove only its cursor");
        assert!(
            wrong_stage
                .to_string()
                .contains("DAG_PROPOSER_PACK_SESSION_WRONG_STAGE")
        );
        assert!(!root.abort_proposer_pack(55)?);

        install_pack_session(&root, 56)?;
        let declared = root.prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id: 56,
            network_throttled: false,
            min_transaction_gas: 21_000,
            estimate_gas_limit: 21_000,
            last_block_number: 0,
        })?;
        assert!(declared.estimate_requests.is_empty());
        assert_eq!(declared.session.action, DagProposerSessionAction::StartVdf);
        assert!(!root.lock_transaction()?.transaction_packing.is_active()?);
        assert!(root.abort_proposer_pack(56)?);
        assert!(!root.abort_proposer_pack(999)?);

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn proposer_pack_dag_poison_cleans_only_the_matching_transaction_owner() -> Result<()> {
        fn poison_dag(root: &DagTransactionService) {
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        let _guard = root.lock_dag().expect("DAG lock before poisoning");
                        panic!("poison DAG lock");
                    })
                    .join()
                    .expect_err("DAG poison thread must panic");
            });
        }

        let finalize_path = unique_temp_dir("rustaxa_consensus_pack_finalize_dag_poison");
        let finalize_storage = Arc::new(Storage::new(Config::new(finalize_path.clone()))?);
        let finalize_root =
            DagTransactionService::restore(finalize_storage.clone(), service_config())?;
        install_pack_session(&finalize_root, 71)?;
        insert_pack_transaction(
            &finalize_root,
            &SigningKey::from_slice(&[0x71; 32]).expect("valid test signing key"),
        )?;
        let prepared = finalize_root.prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id: 71,
            network_throttled: false,
            min_transaction_gas: 21_000,
            estimate_gas_limit: 0,
            last_block_number: 0,
        })?;
        assert_eq!(prepared.estimate_requests.len(), 1);
        poison_dag(&finalize_root);
        let finalize_error = finalize_root
            .finalize_proposer_pack(71, Vec::new())
            .expect_err("poisoned DAG must reject finalize");
        assert!(
            finalize_error
                .to_string()
                .contains("DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED")
        );
        assert!(
            !finalize_root
                .lock_transaction()?
                .transaction_packing
                .is_active()?
        );
        drop(finalize_root);
        drop(finalize_storage);
        std::fs::remove_dir_all(finalize_path)?;

        let abort_path = unique_temp_dir("rustaxa_consensus_pack_abort_dag_poison");
        let abort_storage = Arc::new(Storage::new(Config::new(abort_path.clone()))?);
        let abort_root = DagTransactionService::restore(abort_storage.clone(), service_config())?;
        install_pack_session(&abort_root, 72)?;
        insert_pack_transaction(
            &abort_root,
            &SigningKey::from_slice(&[0x72; 32]).expect("valid test signing key"),
        )?;
        let prepared = abort_root.prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id: 72,
            network_throttled: false,
            min_transaction_gas: 21_000,
            estimate_gas_limit: 0,
            last_block_number: 0,
        })?;
        assert_eq!(prepared.estimate_requests.len(), 1);
        poison_dag(&abort_root);
        let abort_error = abort_root
            .abort_proposer_pack(72)
            .expect_err("poisoned DAG must reject abort");
        assert!(
            abort_error
                .to_string()
                .contains("DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED")
        );
        assert!(
            !abort_root
                .lock_transaction()?
                .transaction_packing
                .is_active()?
        );
        drop(abort_root);
        drop(abort_storage);
        std::fs::remove_dir_all(abort_path)?;

        let compatibility_path = unique_temp_dir("rustaxa_consensus_pack_compatibility_dag_poison");
        let compatibility_storage =
            Arc::new(Storage::new(Config::new(compatibility_path.clone()))?);
        let compatibility_root =
            DagTransactionService::restore(compatibility_storage.clone(), service_config())?;
        install_pack_session(&compatibility_root, 73)?;
        insert_pack_transaction(
            &compatibility_root,
            &SigningKey::from_slice(&[0x73; 32]).expect("valid test signing key"),
        )?;
        {
            let params = compatibility_root
                .lock_dag()?
                .proposer_pack_parameters(73)?;
            compatibility_root
                .lock_transaction()?
                .prepare_proposer_pack(
                    TransactionPackingOwner::Compatibility,
                    params,
                    21_000,
                    0,
                    0,
                )?;
        }
        poison_dag(&compatibility_root);
        compatibility_root
            .abort_proposer_pack(73)
            .expect_err("poisoned DAG must reject compatibility collision cleanup");
        {
            let mut transaction = compatibility_root.lock_transaction()?;
            assert!(transaction.transaction_packing.is_active()?);
            assert!(transaction.abort_proposer_pack(TransactionPackingOwner::Compatibility)?);
        }
        drop(compatibility_root);
        drop(compatibility_storage);
        std::fs::remove_dir_all(compatibility_path)?;
        Ok(())
    }

    #[test]
    fn proposer_pack_preserves_compatibility_owner_and_rejects_malformed_queue_rlp() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_pack_owner");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let root = DagTransactionService::restore(storage.clone(), service_config())?;

        install_pack_session(&root, 61)?;
        insert_pack_transaction(
            &root,
            &SigningKey::from_slice(&[0x61; 32]).expect("valid test signing key"),
        )?;
        {
            let params = root.lock_dag()?.proposer_pack_parameters(61)?;
            root.lock_transaction()?.prepare_proposer_pack(
                TransactionPackingOwner::Compatibility,
                params,
                21_000,
                0,
                0,
            )?;
        }
        let collision = root
            .prepare_proposer_pack(DagProposerPackPrepareRequest {
                session_id: 61,
                network_throttled: false,
                min_transaction_gas: 21_000,
                estimate_gas_limit: 0,
                last_block_number: 0,
            })
            .expect_err("compatibility owner must prevent DAG packing");
        assert!(
            format!("{collision:#}").contains("TM_RUNTIME_PACK_SESSION_ALREADY_ACTIVE"),
            "unexpected collision error: {collision:#}"
        );
        {
            let mut transaction = root.lock_transaction()?;
            assert!(transaction.transaction_packing.is_active()?);
            assert!(transaction.abort_proposer_pack(TransactionPackingOwner::Compatibility)?);
        }
        assert!(!root.abort_proposer_pack(61)?);

        install_pack_session(&root, 62)?;
        root.lock_transaction()?.queue.insert(
            TransactionQueueEntry {
                hash: H256::repeat_byte(0x62),
                sender: H160::repeat_byte(0x62),
                nonce: U256::zero(),
                gas_price: U256::one(),
                gas: 21_000,
                data_size: 0,
                rlp: vec![0xFF],
                last_block_number: 0,
            },
            true,
        )?;
        let malformed = root
            .prepare_proposer_pack(DagProposerPackPrepareRequest {
                session_id: 62,
                network_throttled: false,
                min_transaction_gas: 21_000,
                estimate_gas_limit: 0,
                last_block_number: 0,
            })
            .expect_err("malformed queue payload must fail before returning an executor request");
        assert!(
            malformed
                .to_string()
                .contains("TM_RUNTIME_PACK_CANDIDATE_ENVELOPE_INSPECT_FAILED")
        );
        assert!(!root.lock_transaction()?.transaction_packing.is_active()?);
        assert!(!root.abort_proposer_pack(62)?);

        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn finalized_order_clears_transaction_sidecars_only_after_dag_commit() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_dag_transaction_finalization");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let mut config = service_config();
        config.dag.dag_expiry_limit = 1;
        let root = DagTransactionService::restore(storage.clone(), config)?;
        let expired_hash = H256::repeat_byte(3);
        let anchor_hash = H256::repeat_byte(8);
        let transaction_hash = H256::repeat_byte(7);
        {
            let mut dag = root.lock_dag()?;
            dag.state.add_block(DagManagerBlock {
                hash: expired_hash,
                pivot: H256::repeat_byte(1),
                tips: Vec::new(),
                level: 3,
                difficulty: 90,
            })?;
        }
        save_dag_block_to_storage(
            storage.as_ref(),
            expired_hash,
            3,
            0,
            &composed_add_block_rlp(
                H256::repeat_byte(1),
                3,
                std::slice::from_ref(&transaction_hash),
            ),
        )?;
        save_dag_block_to_storage(
            storage.as_ref(),
            anchor_hash,
            5,
            0,
            &composed_add_block_rlp(H256::repeat_byte(1), 5, &[]),
        )?;
        storage.transaction().write(transaction_hash, &[0xA7])?;
        root.lock_transaction()?
            .sidecar
            .insert_non_finalized(transaction_hash, vec![0xA7])?;

        let error = root
            .apply_finalized_order(anchor_hash, 2, vec![anchor_hash])
            .expect_err("an invalid period must fail before sidecar publication");
        assert!(
            error
                .to_string()
                .contains("DAG_RUNTIME_SET_FINALIZED_ORDER")
        );
        assert!(
            root.lock_transaction()?
                .sidecar
                .contains_non_finalized(transaction_hash)
        );

        let report = root.apply_finalized_order(anchor_hash, 1, vec![anchor_hash])?;
        assert_eq!(report.finalized_count, 1);
        assert_eq!(report.expired_hashes, vec![expired_hash]);
        assert!(
            !root
                .lock_transaction()?
                .sidecar
                .contains_non_finalized(transaction_hash)
        );
        assert!(
            storage
                .transaction()
                .rlp(transaction_hash)?
                .unwrap_or_default()
                .is_empty()
        );
        drop(root);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }
}
