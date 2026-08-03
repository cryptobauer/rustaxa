//! Deterministic PBFT manager runtime planning and storage-backed startup restore.
//!
//! This module owns Rust-side PBFT manager orchestration: storage-backed startup
//! restoration for the long-lived runtime plus the ordered control-flow script
//! for one daemon tick. Tick planning is intentionally side-effect-free. C++
//! supplies already-collected live facts, executes each requested action against
//! the existing manager shell, then reports the result before Rust advances the
//! cursor. Eligible-wallet state is reported after the pre-state cert/round
//! checks so the runtime preserves the legacy branch order.
//!
//! Inputs are a compact `PbftManagerRuntimeTickFact`: current PBFT state,
//! period/round/step telemetry, network availability, sync status, and whether
//! any local wallet is eligible for the current period. Outputs are stable
//! action/status codes and a cursor-managed session.
//!
//! Invariants:
//! - Rust decides the order of manager actions for the tick.
//! - C++ remains the temporary owner of live objects, network dispatch, sleeps,
//!   and non-migrated state mutation in this slice.
//! - Storage-backed startup reads and step normalization use
//!   `rustaxa_storage::Storage` directly inside Rust.
//! - Early-progress actions such as cert-block push complete the session with
//!   `restart_loop = true`, matching the old `continue` path.
//! - Round-advance candidates are reported as facts. Rust validates them and
//!   emits an explicit `ResetConsensus` effect with the target round.
//! - The active-state vs ineligible-sleep branch is selected from the
//!   `has_eligible_wallet` report supplied after `TryAdvanceRound`.
//! - Branches after `run_certify` and `run_second_finish` are selected only from
//!   explicit report flags returned by the C++ executor.

use anyhow::{Context, Result, anyhow};
use ethereum_types::H256;
use rlp::RlpStream;
use rustaxa_storage::{Column, Storage, StorageWriteBatch};
use rustaxa_types::codec::rlp::dag::FinalizedDagBlockBundleRlp;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};
use tiny_keccak::{Hasher, Keccak};

const PBFT_MGR_FIELD_ROUND: u8 = 0;
const PBFT_MGR_FIELD_STEP: u8 = 1;
const PBFT_MGR_FIELD_LAMBDA: u8 = 2;
const PBFT_MGR_STATUS_EXECUTED_BLOCK: u8 = 0;
const PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE: u8 = 2;
const PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH: u8 = 3;

/// Task-oriented port for publishing the reward-vote cursor retained by PBFT finalization.
///
/// The production implementation is [`crate::pbft_vote_runtime::PbftVerifiedVotesService`].
/// Tests may supply a deterministic implementation to exercise manager cursor
/// and error semantics without recreating verified-vote admission state.
pub trait PbftRewardVoteCursorCommitPort {
    /// Publishes or revalidates the exact durable reward-vote cursor.
    fn commit_reward_vote_cursor(
        &self,
        request: crate::pbft_vote_runtime::RewardVoteCursorCommitRequest,
    ) -> Result<crate::pbft_vote_runtime::RewardVoteCursorCommitResult>;
}

impl PbftRewardVoteCursorCommitPort for crate::pbft_vote_runtime::PbftVerifiedVotesService {
    fn commit_reward_vote_cursor(
        &self,
        request: crate::pbft_vote_runtime::RewardVoteCursorCommitRequest,
    ) -> Result<crate::pbft_vote_runtime::RewardVoteCursorCommitResult> {
        self.commit_reward_vote_cursor(request)
    }
}

/// Locked task port for preparing the reward-vote stage of PBFT finalization.
///
/// The production implementation is [`crate::pbft_vote_runtime::PbftVerifiedVotesService`].
/// The guard remains held from canonical bundle derivation through the primary
/// storage commit so concurrent admission cannot invalidate durable bytes.
pub(crate) trait PbftRewardVoteResetStageGuard {
    /// Builds the canonical reward-reset stage for one accepted finalization.
    fn prepare_reward_votes_reset_stage(
        &self,
        request: crate::pbft_vote_runtime::RewardVoteResetPrepareRequest,
    ) -> Result<crate::pbft_finalize::PbftFinalizationStorageWriteStage>;
}

impl PbftRewardVoteResetStageGuard for crate::pbft_vote_runtime::PbftVerifiedVotesServiceGuard<'_> {
    fn prepare_reward_votes_reset_stage(
        &self,
        request: crate::pbft_vote_runtime::RewardVoteResetPrepareRequest,
    ) -> Result<crate::pbft_finalize::PbftFinalizationStorageWriteStage> {
        crate::pbft_vote_runtime::PbftVoteAdmissionRuntime::prepare_reward_votes_reset_stage(
            self, request,
        )
    }
}

/// Task-oriented owner that locks verified votes for finalization preparation.
pub(crate) trait PbftRewardVoteResetStagePort {
    /// Guard held across canonical stage derivation and the primary commit.
    type Guard<'a>: PbftRewardVoteResetStageGuard
    where
        Self: 'a;

    /// Acquires the verified-vote serialization domain.
    fn lock_reward_votes(&self) -> Result<Self::Guard<'_>>;
}

impl PbftRewardVoteResetStagePort for crate::pbft_vote_runtime::PbftVerifiedVotesService {
    type Guard<'a> = crate::pbft_vote_runtime::PbftVerifiedVotesServiceGuard<'a>;

    fn lock_reward_votes(&self) -> Result<Self::Guard<'_>> {
        self.lock()
    }
}

/// Native port for applying the finalized transaction set retained by PBFT finalization.
///
/// The production implementation is
/// [`crate::dag_transaction_service::DagTransactionService`]. The port accepts
/// only canonical period bytes plus narrow external account-nonce facts; it
/// returns native mutation facts and never exposes queue/sidecar effects to C++.
pub trait PbftFinalizedTransactionStatusPort {
    /// Applies storage, sidecar, queue, and account-purge effects for one finalized period.
    fn update_finalized_transactions_from_period_data(
        &self,
        period: u64,
        retention_window: u64,
        account_nonce_facts: Vec<crate::transaction_service::TransactionServiceAccountNonceFact>,
        period_data_rlp: &[u8],
    ) -> Result<crate::transaction_service::TransactionServiceFinalizedStatusReport>;
}

impl PbftFinalizedTransactionStatusPort for crate::dag_transaction_service::DagTransactionService {
    fn update_finalized_transactions_from_period_data(
        &self,
        period: u64,
        retention_window: u64,
        account_nonce_facts: Vec<crate::transaction_service::TransactionServiceAccountNonceFact>,
        period_data_rlp: &[u8],
    ) -> Result<crate::transaction_service::TransactionServiceFinalizedStatusReport> {
        self.transaction_update_finalized_status_from_period_data(
            period,
            retention_window,
            account_nonce_facts,
            period_data_rlp,
        )
    }
}

/// Native outcome after draining PBFT-manager-owned finalization actions.
///
/// The manager executes every consecutive action whose state and persistence
/// it owns, then returns the first external/subsystem action or terminal step.
/// The two booleans are leaf effects consumed by the temporary C++ shell:
/// whether its anchor compatibility cache must be mirrored clear and whether
/// its scalar snapshot must be refreshed. `error_code` carries a stable
/// operation-level rejection label when the runtime step contains the more
/// specific validation or storage status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftFinalizationOwnedActionDrain {
    /// Whether the retained C++ anchor-order compatibility cache must be cleared.
    pub cleared_anchor_dag_cache: bool,
    /// Whether native scalar manager state changed during this drain.
    pub has_snapshot: bool,
    /// Expired DAG hashes that should be removed from C++ compatibility state.
    pub expired_dag_hashes: Vec<H256>,
    /// Whether C++ should refresh its DAG counter mirrors from runtime state.
    pub refresh_dag_counters: bool,
    /// First action not owned by this drain, or its terminal runtime step.
    pub next_step: crate::pbft_finalize::PbftFinalizationRuntimeStep,
    /// Stable high-level rejection label, empty on success.
    pub error_code: String,
}

/// Native start mode for one PBFT finalization executor session.
///
/// Fresh mode applies the primary storage transaction before exposing an
/// external action. Resume mode derives the bounded replay tail from durable
/// storage without reapplying primary writes. Unknown mode preserves the
/// fail-closed CXX boundary behavior while keeping numeric FFI decoding out of
/// the manager task itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PbftFinalizationExecutorStartMode {
    /// Start a newly accepted plan with its complete primary storage batch.
    Fresh {
        /// Caller-prepared primary stages; native sortition preparation may
        /// append its exact stage before the atomic commit.
        primary_stages: Vec<crate::pbft_finalize::PbftFinalizationStorageWriteStage>,
        /// Whether primary persistence must use a synchronous commit.
        sync: bool,
    },
    /// Resume an already-persisted plan from the current FinalChain height.
    Resume {
        /// FinalChain height used only for durable replay classification.
        final_chain_last_block: u64,
    },
    /// Reject an unrecognized boundary mode after clearing stale session state.
    Unknown,
}

/// Complete native input for starting or resuming PBFT finalization.
///
/// The accepted plan and typed mode are domain values converted once by the
/// bridge. Impossible fresh/resume field mixtures cannot reach the manager.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftFinalizationExecutorStartRequest {
    /// Accepted finalization intent retained for validation and replay.
    pub plan: crate::pbft_finalize::PbftFinalizationPlan,
    /// Typed fresh, resume, or fail-closed unknown operation.
    pub mode: PbftFinalizationExecutorStartMode,
}

/// Native executor boundary returned after starting or resuming finalization.
///
/// The step is the first remaining external action or a terminal runtime
/// outcome. Compatibility cache and snapshot effects are captured while the
/// manager lock is still held, so the bridge never reacquires native state to
/// materialize a potentially incoherent result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftFinalizationExecutorBoundary {
    /// First external action or terminal finalization step.
    pub next_step: crate::pbft_finalize::PbftFinalizationRuntimeStep,
    /// Whether C++ must mirror-clear its temporary anchor DAG cache.
    pub cleared_anchor_dag_cache: bool,
    /// Whether scalar compatibility state changed during native draining.
    pub has_snapshot: bool,
    /// Expired DAG hashes from the native DAG finalization action.
    pub expired_dag_hashes: Vec<H256>,
    /// Whether C++ should refresh its DAG counter mirrors from runtime state.
    pub refresh_dag_counters: bool,
    /// Lock-coherent manager snapshot captured before returning the boundary.
    pub snapshot: PbftManagerRuntimeSnapshot,
    /// Stable operation-level error label, empty on success.
    pub error_code: String,
}

pub(crate) fn base_owned_finalization_live_report(
    action: crate::pbft_finalize::PbftFinalizationRuntimeAction,
    write_set: &crate::pbft_finalize::PbftFinalizationStorageWriteIntent,
) -> crate::pbft_finalize::PbftFinalizationLiveMutationReport {
    crate::pbft_finalize::PbftFinalizationLiveMutationReport {
        action,
        block_period: write_set.block_period,
        pbft_block_hash: write_set.pbft_block_hash,
        anchor_hash: write_set.anchor_hash,
        dag_finalized_count: 0,
        finalized_transaction_count: 0,
        pbft_chain_size: 0,
        pbft_chain_head_hash: H256::zero(),
        pbft_chain_last_anchor_hash: H256::zero(),
        reward_votes_period: 0,
        reward_votes_round: 0,
        reward_votes_block_hash: H256::zero(),
        reward_votes_reset_provenance_valid: false,
        sortition_changed: false,
        sortition_change_period: 0,
        sortition_change_interval_efficiency: 0,
        sortition_change_threshold_upper: 0,
        sortition_current_threshold_upper: 0,
        sortition_params_changes_count: 0,
        rounds_count_dynamic_lambda: 0,
        dynamic_lambda: 0,
        executed_pbft_block: false,
        manager_period: 0,
        pillar_processed_period: 0,
        pillar_request_period: 0,
        anchor_dag_cache_count: 0,
        final_chain_dispatched: false,
        final_chain_blocks_per_year: 0,
        final_chain_last_block: 0,
    }
}

fn owned_finalization_drain_outcome(
    cleared_anchor_dag_cache: bool,
    has_snapshot: bool,
    next_step: crate::pbft_finalize::PbftFinalizationRuntimeStep,
    error_code: &str,
) -> PbftFinalizationOwnedActionDrain {
    PbftFinalizationOwnedActionDrain {
        cleared_anchor_dag_cache,
        has_snapshot,
        expired_dag_hashes: Vec::new(),
        refresh_dag_counters: false,
        next_step,
        error_code: error_code.to_string(),
    }
}

/// Prepares and publishes the sortition portion of one PBFT finalization write set.
///
/// The caller must already hold the manager serialization guard. The task
/// rejects caller-supplied sortition stages, validates the accepted period and
/// canonical period-data payload against the manager-owned chain head, previews
/// the change through the supplied native DAG/transaction service, appends the
/// exact storage stage when needed, and retains the preview for the later live
/// commit. PBFT and DAG roots may be distinct compatible instances.
///
/// Manager, chain, then sortition lock order is preserved. Validation failures
/// do not append a stage or publish a preparation and retain the stable
/// `PBFT_FINALIZE_SORTITION_*` error codes used by the CXX boundary.
pub fn prepare_pbft_finalization_sortition(
    runtime: &mut PbftManagerRuntimeState,
    dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
    write_set: &crate::pbft_finalize::PbftFinalizationStorageWriteIntent,
    stages: &mut Vec<crate::pbft_finalize::PbftFinalizationStorageWriteStage>,
) -> Result<()> {
    const SORTITION_STAGE: u8 = 3;

    anyhow::ensure!(
        stages.iter().all(|stage| {
            stage.stage != SORTITION_STAGE
                && !stage.has_sortition_params_change
                && stage.sortition_params_change_period == 0
                && stage.sortition_params_change_interval_efficiency == 0
                && stage.sortition_params_change_threshold_upper == 0
        }),
        "PBFT_FINALIZE_SORTITION_STAGE_CALLER_OWNED"
    );
    if !write_set.update_sortition_params {
        return Ok(());
    }

    let non_empty_pbft_chain_size = {
        let chain = runtime.chain.read().expect("PBFT chain lock poisoned");
        let head = chain.state.head();
        let expected_period = head
            .size
            .checked_add(1)
            .ok_or_else(|| anyhow!("PBFT_FINALIZE_SORTITION_CHAIN_SIZE_OVERFLOW"))?;
        anyhow::ensure!(
            expected_period == write_set.block_period,
            "PBFT_FINALIZE_SORTITION_CHAIN_HEAD_PERIOD_MISMATCH"
        );
        head.non_empty_size
            .checked_add(1)
            .ok_or_else(|| anyhow!("PBFT_FINALIZE_SORTITION_NON_EMPTY_SIZE_OVERFLOW"))?
    };

    let counts = crate::sortition::decode_period_efficiency_counts(&write_set.period_data_rlp)
        .context("PBFT_FINALIZE_SORTITION_PERIOD_DATA")?;
    anyhow::ensure!(
        counts.has_pivot == !write_set.null_anchor,
        "PBFT_FINALIZE_SORTITION_PIVOT_MISMATCH"
    );
    let finalized_period =
        crate::dag_transaction_service::DagTransactionSortitionFinalizationRequest {
            period: write_set.block_period,
            efficiency_counts: counts,
            non_empty_pbft_chain_size,
        };
    let commit_request =
        crate::dag_transaction_service::DagTransactionSortitionFinalizationCommitRequest {
            finalized_period,
            expected_change: dag_transaction_service.preview_finalized_period(finalized_period)?,
        };
    if let Some(change) = commit_request.expected_change {
        stages.push(crate::pbft_finalize::PbftFinalizationStorageWriteStage {
            stage: SORTITION_STAGE,
            has_sortition_params_change: true,
            sortition_params_change_period: change.period,
            sortition_params_change_interval_efficiency: change.interval_efficiency,
            sortition_params_change_threshold_upper: change.threshold_upper,
            ..Default::default()
        });
    }
    runtime.finalization_sortition_commit_request = Some(commit_request);
    Ok(())
}

/// Rejects caller-owned reward-vote stages before fresh finalization.
///
/// The caller already holds the manager serialization guard. Any stage code,
/// payload gate, payload bytes, or delete keys associated with reward reset are
/// rejected. After this check, fresh start acquires verified votes, derives the
/// canonical stage, and retains that guard through sortition preparation and
/// the primary storage commit.
pub(crate) fn reject_caller_owned_reward_vote_stages(
    stages: &[crate::pbft_finalize::PbftFinalizationStorageWriteStage],
) -> Result<()> {
    const REWARD_VOTES_RESET_STAGE: u8 = 4;

    anyhow::ensure!(
        stages.iter().all(|stage| {
            stage.stage != REWARD_VOTES_RESET_STAGE
                && !stage.has_reward_votes_reset
                && stage.reward_votes_bundle_rlp.is_empty()
                && stage.extra_reward_vote_hashes.is_empty()
        }),
        "PBFT_FINALIZE_REWARD_VOTES_STAGE_CALLER_OWNED"
    );
    Ok(())
}

fn prepare_pbft_finalization_reward_votes<G: PbftRewardVoteResetStageGuard>(
    verified_votes: &G,
    write_set: &crate::pbft_finalize::PbftFinalizationStorageWriteIntent,
    stages: &mut Vec<crate::pbft_finalize::PbftFinalizationStorageWriteStage>,
) -> Result<()> {
    stages.push(verified_votes.prepare_reward_votes_reset_stage(
        crate::pbft_vote_runtime::RewardVoteResetPrepareRequest {
            requested: true,
            cursor: crate::pbft_vote_runtime::RewardVoteCursor {
                period: write_set.reward_vote_period,
                round: write_set.reward_vote_round,
                step: write_set.reward_vote_step,
                block_hash: write_set.reward_vote_block_hash,
            },
        },
    )?);
    Ok(())
}

/// Lock-protected mutable state owned by the native PBFT manager service.
///
/// The state groups the deterministic manager runtime, its in-flight sessions,
/// and the PBFT chain/storage dependencies that must be mutated under the same
/// serialization domain. Callers access it only through
/// [`PbftManagerService::lock`]; no state guard may be retained across an
/// external executor, network, or C++ callback.
pub struct PbftManagerRuntimeState {
    /// Deterministic PBFT manager scalar state restored from durable storage.
    pub state: PbftManagerRuntime,
    /// Shared native storage used by manager persistence and startup replay.
    pub storage: Arc<Storage>,
    /// Ordered finalized-period queue consumed by PBFT synchronization.
    pub(crate) period_data_queue: crate::period_data_queue::PeriodDataQueue,
    /// Active queue-drain cursor, reset whenever a new drain begins.
    pub(crate) pbft_sync_queue_drain_session: crate::pbft_sync::PbftSyncQueueDrainSession,
    /// Optional admission cursor for the PBFT sync item currently being checked.
    pub pbft_sync_admission_session: Option<crate::pbft_sync::PbftSyncAdmissionSession>,
    /// Optional cursor for applying one manager state-action effect sequence.
    pub(crate) state_action_effect_session: Option<PbftManagerStateActionEffectSession>,
    /// Optional daemon-tick planning cursor.
    pub(crate) runtime_session: Option<PbftManagerRuntimeSession>,
    /// Optional PBFT block-proposal planning cursor.
    pub(crate) proposal_session: Option<PbftManagerProposalSession>,
    /// Optional finalization executor state for the current PBFT block.
    pub finalization_runtime_session: Option<crate::pbft_finalize::PbftFinalizationRuntimeState>,
    /// Optional immutable plan paired with the active finalization executor.
    pub finalization_runtime_plan: Option<crate::pbft_finalize::PbftFinalizationPlan>,
    /// Exact native sortition commit request retained until finalization commits.
    pub finalization_sortition_commit_request:
        Option<crate::dag_transaction_service::DagTransactionSortitionFinalizationCommitRequest>,
    /// Process-local reset proof bound to the active finalization session.
    pub finalization_reward_votes_reset_generation: u64,
    /// Native PBFT chain owner used by manager operations in the same lock domain.
    pub chain: crate::pbft_chain::PbftChainService,
}

/// Native owner of the PBFT manager serialization and runtime-state domain.
///
/// Construction consumes a restored manager runtime, shared storage, and PBFT
/// chain service. [`Self::lock`] serializes every manager/session mutation and
/// returns a guard that dereferences to [`PbftManagerRuntimeState`]. A poisoned
/// lock is treated as an unrecoverable consensus invariant failure.
#[derive(Clone)]
pub struct PbftManagerService {
    runtime: Arc<Mutex<PbftManagerRuntimeState>>,
}

/// Exclusive native PBFT manager runtime guard.
///
/// The guard is produced by [`PbftManagerService::lock`], provides immutable
/// and mutable field access through `Deref`/`DerefMut`, and releases the native
/// manager serialization domain when dropped.
pub struct PbftManagerGuard<'a>(MutexGuard<'a, PbftManagerRuntimeState>);

impl Deref for PbftManagerGuard<'_> {
    type Target = PbftManagerRuntimeState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for PbftManagerGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl PbftManagerGuard<'_> {
    fn clear_finalization_executor_runtime(&mut self) {
        self.finalization_runtime_session = None;
        self.finalization_runtime_plan = None;
        self.finalization_reward_votes_reset_generation = 0;
        self.finalization_sortition_commit_request = None;
    }

    /// Authenticates a retained sortition preview against the committed primary batch.
    ///
    /// Only a preview that emitted a concrete parameter change is recoverable:
    /// its exact RLP row supplies durable proof that primary storage committed.
    /// No-change previews intentionally return `None` because replaying them
    /// could advance hidden efficiency-window state without a durable cursor.
    fn authenticated_resume_sortition_request(
        &self,
        plan: &crate::pbft_finalize::PbftFinalizationPlan,
    ) -> Result<
        Option<crate::dag_transaction_service::DagTransactionSortitionFinalizationCommitRequest>,
    > {
        let Some(request) = self.finalization_sortition_commit_request else {
            return Ok(None);
        };
        if !plan.cleanup.update_sortition_params
            || !plan.storage_write_intent.update_sortition_params
            || request.finalized_period.period != plan.storage_write_intent.block_period
            || request.finalized_period.efficiency_counts.has_pivot
                != !plan.storage_write_intent.null_anchor
        {
            return Ok(None);
        }
        let Some(change) = request.expected_change else {
            return Ok(None);
        };
        let durable = self
            .storage
            .get_raw(Column::SortitionParamsChange, &change.period.to_le_bytes())?;
        Ok((durable.as_deref() == Some(change.to_rlp_bytes().as_slice())).then_some(request))
    }

    /// Applies terminal/error cleanup to one complete executor operation.
    ///
    /// Active external boundaries retain their plan and cursor. Complete,
    /// rejected, mismatched, failed, or operational-error outcomes clear every
    /// retained session, sortition preparation, and reset-generation proof.
    pub(crate) fn finish_finalization_executor(
        &mut self,
        result: Result<PbftFinalizationOwnedActionDrain>,
    ) -> Result<PbftFinalizationOwnedActionDrain> {
        match result {
            Ok(drain) => {
                if drain.next_step.complete
                    || drain.next_step.runtime_status
                        != crate::pbft_finalize::PbftFinalizationRuntimeStatus::Active
                {
                    self.clear_finalization_executor_runtime();
                }
                Ok(drain)
            }
            Err(error) => {
                self.clear_finalization_executor_runtime();
                Err(error)
            }
        }
    }

    /// Starts or resumes finalization and advances to the first external action.
    ///
    /// The task clears any stale executor, preserves only a storage-proven
    /// reward-reset generation for resume, derives the native cursor, prepares
    /// sortition under manager-before-sortition lock order, atomically applies
    /// fresh primary storage, reports that action, and drains consecutive
    /// manager-owned actions. Resume derives its durable tail from native
    /// storage and never reapplies primary writes. It prepends a concrete
    /// sortition change only when the retained preview matches its exact durable
    /// row, then prepends reward-cursor publication only when both the accepted
    /// plan and current storage reset generation authenticate that recovery
    /// action. No-change sortition previews remain non-replayable.
    ///
    /// The returned drain contains the first DAG, transaction, vote, FinalChain,
    /// pillar, period-advance, or network action for the C++ leaf executor.
    /// Rejected/complete sessions and all errors clear retained executor state;
    /// active external boundaries retain the plan and cursor. Unknown mode,
    /// storage failures, malformed preparation, and invalid retained invariants
    /// fail closed without publishing sortition state.
    pub(crate) fn start_finalization_executor<V: PbftRewardVoteResetStagePort>(
        &mut self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        verified_votes: &V,
        request: PbftFinalizationExecutorStartRequest,
    ) -> Result<PbftFinalizationOwnedActionDrain> {
        let resume_mode = matches!(
            request.mode,
            PbftFinalizationExecutorStartMode::Resume { .. }
        );
        let resume_generation = if resume_mode
            && self.finalization_reward_votes_reset_generation != 0
            && self.finalization_reward_votes_reset_generation
                == self.storage.extra_reward_votes_reset_generation()
        {
            self.finalization_reward_votes_reset_generation
        } else {
            0
        };
        let resume_sortition_request = if resume_mode {
            self.authenticated_resume_sortition_request(&request.plan)
        } else {
            Ok(None)
        };
        self.clear_finalization_executor_runtime();
        let resume_sortition_request = resume_sortition_request?;
        if resume_mode {
            self.finalization_reward_votes_reset_generation = resume_generation;
            self.finalization_sortition_commit_request = resume_sortition_request;
        }

        let result = self.start_finalization_executor_inner(
            dag_transaction_service,
            verified_votes,
            request,
        );
        self.finish_finalization_executor(result)
    }

    /// Drains native manager actions after one subsystem or leaf action.
    ///
    /// Active steps enter the owned drain; terminal steps are projected without
    /// further mutation. Cleanup remains the caller's responsibility through
    /// [`Self::finish_finalization_executor`].
    pub(crate) fn continue_finalization_executor_from_step(
        &mut self,
        step: crate::pbft_finalize::PbftFinalizationRuntimeStep,
    ) -> Result<PbftFinalizationOwnedActionDrain> {
        if step.runtime_status == crate::pbft_finalize::PbftFinalizationRuntimeStatus::Active
            && step.has_action
        {
            self.drain_finalization_owned_actions()
        } else {
            Ok(owned_finalization_drain_outcome(false, false, step, ""))
        }
    }

    fn current_finalization_executor_step(
        &self,
    ) -> crate::pbft_finalize::PbftFinalizationRuntimeStep {
        use crate::pbft_finalize::{
            PbftFinalizationRuntimeStatus, PbftFinalizationRuntimeStep,
            next_pbft_finalization_runtime_action,
        };

        self.finalization_runtime_session
            .as_ref()
            .map(next_pbft_finalization_runtime_action)
            .unwrap_or_else(|| PbftFinalizationRuntimeStep {
                runtime_status: PbftFinalizationRuntimeStatus::ActionMismatch,
                has_action: false,
                action: None,
                action_index: 0,
                complete: false,
                error_code: "PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED".to_string(),
            })
    }

    fn reject_finalization_cursor(
        &mut self,
        error_code: &str,
    ) -> crate::pbft_finalize::PbftFinalizationRuntimeStep {
        use crate::pbft_finalize::{
            PbftFinalizationRuntimeStatus, next_pbft_finalization_runtime_action,
        };

        let session = self
            .finalization_runtime_session
            .as_mut()
            .expect("active finalization step requires a session");
        session.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
        session.error_code = error_code.to_string();
        next_pbft_finalization_runtime_action(session)
    }

    fn report_finalization_live_mutation(
        &mut self,
        report: crate::pbft_finalize::PbftFinalizationLiveMutationReport,
    ) -> Result<crate::pbft_finalize::PbftFinalizationRuntimeStep> {
        use crate::pbft_finalize::{
            PbftFinalizationRuntimeActionResult, next_pbft_finalization_runtime_action,
            report_pbft_finalization_runtime_action,
            validate_pbft_finalization_live_mutation_report,
        };

        let plan = self
            .finalization_runtime_plan
            .as_ref()
            .ok_or_else(|| anyhow!("PBFT_FINALIZE_RUNTIME_PLAN_NOT_STARTED"))?;
        let validation = validate_pbft_finalization_live_mutation_report(plan, report);
        let session = self
            .finalization_runtime_session
            .as_mut()
            .expect("active finalization step requires a session");
        *session = report_pbft_finalization_runtime_action(
            session.clone(),
            PbftFinalizationRuntimeActionResult {
                action: validation.action,
                success: validation.accepted,
                status: validation.status.as_u8(),
                error_code: validation.error_code,
            },
        );
        Ok(next_pbft_finalization_runtime_action(session))
    }

    /// Validates one typed external-leaf success against the retained plan.
    ///
    /// The echoed cursor must identify the current native action. The supplied
    /// builder receives that typed action and immutable storage intent, returns
    /// the narrow observed facts, and is never invoked for stale cursors or
    /// terminal sessions. Validation reports through the native runtime cursor.
    pub(crate) fn advance_finalization_live_mutation(
        &mut self,
        cursor: u32,
        build_report: impl FnOnce(
            crate::pbft_finalize::PbftFinalizationRuntimeAction,
            &crate::pbft_finalize::PbftFinalizationStorageWriteIntent,
        ) -> crate::pbft_finalize::PbftFinalizationLiveMutationReport,
    ) -> Result<crate::pbft_finalize::PbftFinalizationRuntimeStep> {
        use crate::pbft_finalize::PbftFinalizationRuntimeStatus;

        let current_step = self.current_finalization_executor_step();
        if current_step.runtime_status != PbftFinalizationRuntimeStatus::Active
            || !current_step.has_action
        {
            return Ok(current_step);
        }
        if cursor != current_step.action_index {
            return Ok(self.reject_finalization_cursor("PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH"));
        }
        let action = current_step
            .action
            .expect("active finalization step carries an action");
        let plan = self
            .finalization_runtime_plan
            .as_ref()
            .ok_or_else(|| anyhow!("PBFT_FINALIZE_RUNTIME_PLAN_NOT_STARTED"))?;
        let report = build_report(action, &plan.storage_write_intent);
        self.report_finalization_live_mutation(report)
    }

    /// Records failure of the current external finalization leaf.
    ///
    /// Cursor mismatch fails closed without accepting the external status.
    /// Otherwise the current typed action is reported failed with the supplied
    /// stable status and error. The returned terminal step is cleaned up by the
    /// enclosing application-service operation.
    pub(crate) fn fail_finalization_external_effect(
        &mut self,
        cursor: u32,
        status: u8,
        error_code: String,
    ) -> Result<crate::pbft_finalize::PbftFinalizationRuntimeStep> {
        use crate::pbft_finalize::{
            PbftFinalizationRuntimeActionResult, PbftFinalizationRuntimeStatus,
            next_pbft_finalization_runtime_action, report_pbft_finalization_runtime_action,
        };

        let current_step = self.current_finalization_executor_step();
        if current_step.runtime_status != PbftFinalizationRuntimeStatus::Active
            || !current_step.has_action
        {
            return Ok(current_step);
        }
        if cursor != current_step.action_index {
            return Ok(self.reject_finalization_cursor("PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH"));
        }
        let action = current_step
            .action
            .expect("active finalization step carries an action");
        let session = self
            .finalization_runtime_session
            .as_mut()
            .expect("active finalization step requires a session");
        *session = report_pbft_finalization_runtime_action(
            session.clone(),
            PbftFinalizationRuntimeActionResult {
                action,
                success: false,
                status,
                error_code,
            },
        );
        Ok(next_pbft_finalization_runtime_action(session))
    }

    fn start_finalization_executor_inner<V: PbftRewardVoteResetStagePort>(
        &mut self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        verified_votes: &V,
        request: PbftFinalizationExecutorStartRequest,
    ) -> Result<PbftFinalizationOwnedActionDrain> {
        use crate::pbft_finalize::{
            inspect_pbft_finalization_resume, start_pbft_finalization_resume_runtime,
        };

        match request.mode {
            PbftFinalizationExecutorStartMode::Resume {
                final_chain_last_block,
            } => {
                let resume = inspect_pbft_finalization_resume(
                    self.storage.as_ref(),
                    &request.plan.storage_write_intent,
                    final_chain_last_block,
                )?;
                let replay_reward_cursor = request.plan.cleanup.reset_reward_votes
                    && request.plan.storage_write_intent.reset_reward_votes
                    && self.finalization_reward_votes_reset_generation != 0;
                let replay_sortition = self.finalization_sortition_commit_request.is_some();
                self.finalization_runtime_plan = Some(request.plan);
                let mut session = start_pbft_finalization_resume_runtime(&resume);
                if replay_reward_cursor
                    && matches!(
                        session.runtime_status,
                        crate::pbft_finalize::PbftFinalizationRuntimeStatus::Active
                            | crate::pbft_finalize::PbftFinalizationRuntimeStatus::Complete
                    )
                    && !session.actions.contains(
                        &crate::pbft_finalize::PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime,
                    )
                {
                    session.runtime_status =
                        crate::pbft_finalize::PbftFinalizationRuntimeStatus::Active;
                    session.actions.insert(
                        0,
                        crate::pbft_finalize::PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime,
                    );
                    session.next_action_index = 0;
                    session.last_action = None;
                }
                if replay_sortition
                    && matches!(
                        session.runtime_status,
                        crate::pbft_finalize::PbftFinalizationRuntimeStatus::Active
                            | crate::pbft_finalize::PbftFinalizationRuntimeStatus::Complete
                    )
                    && !session.actions.contains(
                        &crate::pbft_finalize::PbftFinalizationRuntimeAction::CommitSortitionRuntime,
                    )
                {
                    session.runtime_status =
                        crate::pbft_finalize::PbftFinalizationRuntimeStatus::Active;
                    session.actions.insert(
                        0,
                        crate::pbft_finalize::PbftFinalizationRuntimeAction::CommitSortitionRuntime,
                    );
                    session.next_action_index = 0;
                    session.last_action = None;
                }
                self.finalization_runtime_session = Some(session);
                self.drain_finalization_owned_actions()
            }
            PbftFinalizationExecutorStartMode::Fresh {
                primary_stages,
                sync,
            } => self.start_fresh_finalization_executor(
                dag_transaction_service,
                verified_votes,
                request.plan,
                primary_stages,
                sync,
            ),
            PbftFinalizationExecutorStartMode::Unknown => {
                Err(anyhow!("PBFT_FINALIZE_EXECUTOR_UNKNOWN_MODE"))
            }
        }
    }

    fn start_fresh_finalization_executor<V: PbftRewardVoteResetStagePort>(
        &mut self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        verified_votes: &V,
        plan: crate::pbft_finalize::PbftFinalizationPlan,
        mut primary_stages: Vec<crate::pbft_finalize::PbftFinalizationStorageWriteStage>,
        sync: bool,
    ) -> Result<PbftFinalizationOwnedActionDrain> {
        use crate::pbft_finalize::{
            PbftFinalizationRuntimeAction, PbftFinalizationRuntimeActionResult,
            PbftFinalizationRuntimeStatus, apply_pbft_finalization_storage_writes,
            next_pbft_finalization_runtime_action, plan_pbft_finalization_runtime,
            report_pbft_finalization_runtime_action, start_pbft_finalization_runtime,
        };

        let runtime_plan = plan_pbft_finalization_runtime(&plan);
        self.finalization_runtime_session = Some(start_pbft_finalization_runtime(&runtime_plan));
        self.finalization_runtime_plan = Some(plan);
        let current_step = next_pbft_finalization_runtime_action(
            self.finalization_runtime_session
                .as_ref()
                .expect("fresh finalization session installed"),
        );
        if current_step.runtime_status != PbftFinalizationRuntimeStatus::Active
            || current_step.action != Some(PbftFinalizationRuntimeAction::ApplyPrimaryStorage)
        {
            return Ok(owned_finalization_drain_outcome(
                false,
                false,
                current_step,
                "PBFT_FINALIZE_PRIMARY_STORAGE_ACTION_MISSING",
            ));
        }

        let write_set = self
            .finalization_runtime_plan
            .as_ref()
            .expect("fresh finalization plan installed")
            .storage_write_intent
            .clone();
        reject_caller_owned_reward_vote_stages(&primary_stages)?;
        let reward_votes_guard = if write_set.reset_reward_votes {
            Some(verified_votes.lock_reward_votes()?)
        } else {
            None
        };
        if let Some(reward_votes_guard) = reward_votes_guard.as_ref() {
            prepare_pbft_finalization_reward_votes(
                reward_votes_guard,
                &write_set,
                &mut primary_stages,
            )?;
        }
        prepare_pbft_finalization_sortition(
            self,
            dag_transaction_service,
            &write_set,
            &mut primary_stages,
        )?;
        let apply_result = apply_pbft_finalization_storage_writes(
            self.storage.as_ref(),
            &write_set,
            primary_stages,
            sync,
        )?;
        drop(reward_votes_guard);
        let accepted = apply_result.status.is_success();
        self.finalization_reward_votes_reset_generation =
            apply_result.reward_votes_reset_generation;
        let session = self
            .finalization_runtime_session
            .take()
            .expect("fresh finalization session installed");
        self.finalization_runtime_session = Some(report_pbft_finalization_runtime_action(
            session,
            PbftFinalizationRuntimeActionResult {
                action: PbftFinalizationRuntimeAction::ApplyPrimaryStorage,
                success: accepted,
                status: apply_result.status.as_u8(),
                error_code: apply_result.error_code,
            },
        ));
        if !accepted {
            let next_step = next_pbft_finalization_runtime_action(
                self.finalization_runtime_session
                    .as_ref()
                    .expect("fresh finalization session retained"),
            );
            return Ok(owned_finalization_drain_outcome(
                false,
                false,
                next_step,
                "PBFT_FINALIZE_PRIMARY_STORAGE_REJECTED",
            ));
        }

        self.drain_finalization_owned_actions()
    }

    /// Commits the retained finalization sortition request and advances its runtime cursor.
    ///
    /// The lock-held task validates the current cursor and required
    /// `CommitSortitionRuntime` action, checks the retained native commit request
    /// against the accepted finalization plan, commits through the supplied
    /// DAG/transaction service, validates the resulting live facts, and reports
    /// the result to the manager-owned finalization runtime. Cursor and action
    /// failures return terminal runtime steps; missing or inconsistent retained
    /// state and sortition commit failures retain the stable
    /// `PBFT_FINALIZE_POST_STORAGE_SORTITION_INVARIANT` fatal prefix.
    ///
    /// The manager guard remains held while the DAG service acquires sortition,
    /// preserving manager-before-sortition lock order. A successful commit
    /// clears the retained request exactly once before runtime reporting.
    pub fn advance_finalization_sortition_commit(
        &mut self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
    ) -> Result<crate::pbft_finalize::PbftFinalizationRuntimeStep> {
        use crate::pbft_finalize::{
            PbftFinalizationLiveMutationReport, PbftFinalizationRuntimeAction,
            PbftFinalizationRuntimeActionResult, PbftFinalizationRuntimeStatus,
            PbftFinalizationRuntimeStep, next_pbft_finalization_runtime_action,
            report_pbft_finalization_runtime_action,
            validate_pbft_finalization_live_mutation_report,
        };

        let Some(session) = self.finalization_runtime_session.as_ref() else {
            return Ok(PbftFinalizationRuntimeStep {
                runtime_status: PbftFinalizationRuntimeStatus::ActionMismatch,
                has_action: false,
                action: None,
                action_index: 0,
                complete: false,
                error_code: "PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED".to_string(),
            });
        };
        let current_step = next_pbft_finalization_runtime_action(session);
        if current_step.runtime_status != PbftFinalizationRuntimeStatus::Active
            || !current_step.has_action
        {
            return Ok(current_step);
        }
        if cursor != current_step.action_index {
            let session = self
                .finalization_runtime_session
                .as_mut()
                .expect("finalization session checked above");
            session.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
            session.error_code = "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH".to_string();
            return Ok(next_pbft_finalization_runtime_action(session));
        }
        if current_step.action != Some(PbftFinalizationRuntimeAction::CommitSortitionRuntime) {
            let action = current_step
                .action
                .expect("active finalization step must carry an action");
            let session = self
                .finalization_runtime_session
                .as_mut()
                .expect("finalization session checked above");
            *session = report_pbft_finalization_runtime_action(
                session.clone(),
                PbftFinalizationRuntimeActionResult {
                    action,
                    success: false,
                    status: PbftFinalizationRuntimeStatus::ActionMismatch.as_u8(),
                    error_code: "PBFT_FINALIZE_RUNTIME_ACTION_MISMATCH".to_string(),
                },
            );
            return Ok(next_pbft_finalization_runtime_action(session));
        }

        let plan = self
            .finalization_runtime_plan
            .clone()
            .ok_or_else(|| anyhow!("PBFT_FINALIZE_RUNTIME_PLAN_NOT_STARTED"))?;
        let commit_request = self.finalization_sortition_commit_request.ok_or_else(|| {
            anyhow!("PBFT_FINALIZE_POST_STORAGE_SORTITION_INVARIANT:MISSING_PREPARATION")
        })?;
        if commit_request.finalized_period.period != plan.storage_write_intent.block_period
            || commit_request.finalized_period.efficiency_counts.has_pivot
                != !plan.storage_write_intent.null_anchor
        {
            return Err(anyhow!(
                "PBFT_FINALIZE_POST_STORAGE_SORTITION_INVARIANT:PREPARATION_PLAN_MISMATCH"
            ));
        }
        let committed = dag_transaction_service
            .commit_finalized_period_with_live_snapshot(commit_request)
            .map_err(|err| anyhow!("PBFT_FINALIZE_POST_STORAGE_SORTITION_INVARIANT:{err}"))?;
        self.finalization_sortition_commit_request = None;

        let mut report = PbftFinalizationLiveMutationReport {
            action: PbftFinalizationRuntimeAction::CommitSortitionRuntime,
            block_period: plan.storage_write_intent.block_period,
            pbft_block_hash: plan.storage_write_intent.pbft_block_hash,
            anchor_hash: plan.storage_write_intent.anchor_hash,
            dag_finalized_count: 0,
            finalized_transaction_count: 0,
            pbft_chain_size: 0,
            pbft_chain_head_hash: H256::zero(),
            pbft_chain_last_anchor_hash: H256::zero(),
            reward_votes_period: 0,
            reward_votes_round: 0,
            reward_votes_block_hash: H256::zero(),
            reward_votes_reset_provenance_valid: false,
            sortition_changed: false,
            sortition_change_period: 0,
            sortition_change_interval_efficiency: 0,
            sortition_change_threshold_upper: 0,
            sortition_current_threshold_upper: committed.current_threshold_upper,
            sortition_params_changes_count: committed.params_changes_count,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            executed_pbft_block: false,
            manager_period: 0,
            pillar_processed_period: 0,
            pillar_request_period: 0,
            anchor_dag_cache_count: 0,
            final_chain_dispatched: false,
            final_chain_blocks_per_year: 0,
            final_chain_last_block: 0,
        };
        if let Some(change) = committed.change {
            report.sortition_changed = true;
            report.sortition_change_period = change.period;
            report.sortition_change_interval_efficiency = change.interval_efficiency;
            report.sortition_change_threshold_upper = change.threshold_upper;
        }
        let validation = validate_pbft_finalization_live_mutation_report(&plan, report);
        let session = self
            .finalization_runtime_session
            .as_mut()
            .expect("finalization session checked above");
        *session = report_pbft_finalization_runtime_action(
            session.clone(),
            PbftFinalizationRuntimeActionResult {
                action: validation.action,
                success: validation.accepted,
                status: validation.status.as_u8(),
                error_code: validation.error_code,
            },
        );
        Ok(next_pbft_finalization_runtime_action(session))
    }

    /// Applies one finalized-DAG order update and advances finalization cursor.
    ///
    /// This validates the cursor and required `SetDagBlockOrder` action before
    /// resolving the accepted finalization write-set into ordered hashes and
    /// applying them through the DAG/transaction runtime. It then validates the
    /// derived finalized-count fact and advances the local finalization cursor.
    ///
    /// The returned tuple includes Rust-side compatibility effects and is
    /// populated only when the DAG mutation is attempted.
    pub fn advance_finalization_set_dag_order(
        &mut self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
    ) -> Result<(
        crate::pbft_finalize::PbftFinalizationRuntimeStep,
        Vec<H256>,
        bool,
    )> {
        use crate::pbft_finalize::{
            PbftFinalizationLiveMutationReport, PbftFinalizationRuntimeAction,
            PbftFinalizationRuntimeActionResult, PbftFinalizationRuntimeStatus,
            PbftFinalizationRuntimeStep, next_pbft_finalization_runtime_action,
            report_pbft_finalization_runtime_action,
            validate_pbft_finalization_live_mutation_report,
        };

        let Some(session) = self.finalization_runtime_session.as_ref() else {
            return Ok((
                PbftFinalizationRuntimeStep {
                    runtime_status: PbftFinalizationRuntimeStatus::ActionMismatch,
                    has_action: false,
                    action: None,
                    action_index: 0,
                    complete: false,
                    error_code: "PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED".to_string(),
                },
                Vec::new(),
                false,
            ));
        };

        let current_step = next_pbft_finalization_runtime_action(session);
        if current_step.runtime_status != PbftFinalizationRuntimeStatus::Active
            || !current_step.has_action
        {
            return Ok((current_step, Vec::new(), false));
        }

        if cursor != current_step.action_index {
            let session = self
                .finalization_runtime_session
                .as_mut()
                .expect("finalization session checked above");
            session.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
            session.error_code = "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH".to_string();
            return Ok((
                next_pbft_finalization_runtime_action(session),
                Vec::new(),
                false,
            ));
        }

        let action = current_step
            .action
            .expect("active finalization step must carry an action");
        if action != PbftFinalizationRuntimeAction::SetDagBlockOrder {
            let session = self
                .finalization_runtime_session
                .as_mut()
                .expect("finalization session checked above");
            *session = report_pbft_finalization_runtime_action(
                session.clone(),
                PbftFinalizationRuntimeActionResult {
                    action,
                    success: false,
                    status: PbftFinalizationRuntimeStatus::ActionMismatch.as_u8(),
                    error_code: "PBFT_FINALIZE_RUNTIME_ACTION_MISMATCH".to_string(),
                },
            );
            return Ok((
                next_pbft_finalization_runtime_action(session),
                Vec::new(),
                false,
            ));
        }

        let plan = self
            .finalization_runtime_plan
            .clone()
            .ok_or_else(|| anyhow!("PBFT_FINALIZE_RUNTIME_PLAN_NOT_STARTED"))?;
        let finalized_order = plan
            .storage_write_intent
            .dag_block_period_writes
            .iter()
            .map(|entry| entry.hash)
            .collect::<Vec<_>>();
        let finalized = dag_transaction_service.apply_finalized_order(
            plan.storage_write_intent.anchor_hash,
            plan.storage_write_intent.block_period,
            finalized_order,
        )?;
        let report = PbftFinalizationLiveMutationReport {
            action,
            block_period: plan.storage_write_intent.block_period,
            pbft_block_hash: plan.storage_write_intent.pbft_block_hash,
            anchor_hash: plan.storage_write_intent.anchor_hash,
            dag_finalized_count: u64::try_from(finalized.finalized_count)
                .context("PBFT_FINALIZE_DAG_FINALIZED_COUNT_OVERFLOW")?,
            finalized_transaction_count: 0,
            pbft_chain_size: 0,
            pbft_chain_head_hash: H256::zero(),
            pbft_chain_last_anchor_hash: H256::zero(),
            reward_votes_period: 0,
            reward_votes_round: 0,
            reward_votes_block_hash: H256::zero(),
            reward_votes_reset_provenance_valid: false,
            sortition_changed: false,
            sortition_change_period: 0,
            sortition_change_interval_efficiency: 0,
            sortition_change_threshold_upper: 0,
            sortition_current_threshold_upper: 0,
            sortition_params_changes_count: 0,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            executed_pbft_block: false,
            manager_period: 0,
            pillar_processed_period: 0,
            pillar_request_period: 0,
            anchor_dag_cache_count: 0,
            final_chain_dispatched: false,
            final_chain_blocks_per_year: 0,
            final_chain_last_block: 0,
        };
        let validation = validate_pbft_finalization_live_mutation_report(&plan, report);
        let session = self
            .finalization_runtime_session
            .as_mut()
            .expect("finalization session checked above");
        *session = report_pbft_finalization_runtime_action(
            session.clone(),
            PbftFinalizationRuntimeActionResult {
                action: validation.action,
                success: validation.accepted,
                status: validation.status.as_u8(),
                error_code: validation.error_code,
            },
        );

        Ok((
            next_pbft_finalization_runtime_action(session),
            finalized.expired_hashes,
            true,
        ))
    }

    /// Commits the retained reward-vote cursor and advances its finalization cursor.
    ///
    /// The task owns cursor/action classification, derives the exact certified
    /// vote identity from the accepted finalization plan, publishes that cursor
    /// through the native verified-vote owner, verifies the nonzero reset
    /// generation against both this session and shared storage, then builds and
    /// validates the complete live-mutation report. Rejected cursor publication
    /// is a fatal post-storage invariant; no C++ vote-manager report participates
    /// in the operation.
    pub fn advance_finalization_reward_votes_reset<V: PbftRewardVoteCursorCommitPort>(
        &mut self,
        verified_votes: &V,
        cursor: u32,
    ) -> Result<crate::pbft_finalize::PbftFinalizationRuntimeStep> {
        use crate::pbft_finalize::{
            PbftFinalizationLiveMutationReport, PbftFinalizationRuntimeAction,
            PbftFinalizationRuntimeActionResult, PbftFinalizationRuntimeStatus,
            PbftFinalizationRuntimeStep, next_pbft_finalization_runtime_action,
            report_pbft_finalization_runtime_action,
            validate_pbft_finalization_live_mutation_report,
        };

        let Some(session) = self.finalization_runtime_session.as_ref() else {
            return Ok(PbftFinalizationRuntimeStep {
                runtime_status: PbftFinalizationRuntimeStatus::ActionMismatch,
                has_action: false,
                action: None,
                action_index: 0,
                complete: false,
                error_code: "PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED".to_string(),
            });
        };
        let current_step = next_pbft_finalization_runtime_action(session);
        if current_step.runtime_status != PbftFinalizationRuntimeStatus::Active
            || !current_step.has_action
        {
            return Ok(current_step);
        }
        if cursor != current_step.action_index {
            let session = self
                .finalization_runtime_session
                .as_mut()
                .expect("finalization session checked above");
            session.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
            session.error_code = "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH".to_string();
            return Ok(next_pbft_finalization_runtime_action(session));
        }

        let action = current_step
            .action
            .expect("active finalization step must carry an action");
        if action != PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime {
            let session = self
                .finalization_runtime_session
                .as_mut()
                .expect("finalization session checked above");
            *session = report_pbft_finalization_runtime_action(
                session.clone(),
                PbftFinalizationRuntimeActionResult {
                    action,
                    success: false,
                    status: PbftFinalizationRuntimeStatus::ActionMismatch.as_u8(),
                    error_code: "PBFT_FINALIZE_RUNTIME_ACTION_MISMATCH".to_string(),
                },
            );
            return Ok(next_pbft_finalization_runtime_action(session));
        }
        let plan = self
            .finalization_runtime_plan
            .clone()
            .ok_or_else(|| anyhow!("PBFT_FINALIZE_RUNTIME_PLAN_NOT_STARTED"))?;
        let reset_generation = self.finalization_reward_votes_reset_generation;
        let committed = verified_votes
            .commit_reward_vote_cursor(crate::pbft_vote_runtime::RewardVoteCursorCommitRequest {
                cursor: crate::pbft_vote_runtime::RewardVoteCursor {
                    period: plan.storage_write_intent.reward_vote_period,
                    round: plan.storage_write_intent.reward_vote_round,
                    step: plan.storage_write_intent.reward_vote_step,
                    block_hash: plan.storage_write_intent.reward_vote_block_hash,
                },
                reset_generation,
            })
            .map_err(|error| {
                anyhow!("PBFT_FINALIZE_POST_STORAGE_REWARD_VOTES_INVARIANT:{error}")
            })?;
        if committed.status == crate::pbft_vote_runtime::RewardVoteCursorCommitStatus::Rejected {
            return Err(anyhow!(
                "PBFT_FINALIZE_POST_STORAGE_REWARD_VOTES_INVARIANT:{}",
                committed.error_code
            ));
        }
        let reset_provenance_valid = committed.reset_generation != 0
            && committed.reset_generation == reset_generation
            && committed.reset_generation == self.storage.extra_reward_votes_reset_generation();
        let live_report = PbftFinalizationLiveMutationReport {
            action,
            block_period: plan.storage_write_intent.block_period,
            pbft_block_hash: plan.storage_write_intent.pbft_block_hash,
            anchor_hash: plan.storage_write_intent.anchor_hash,
            dag_finalized_count: 0,
            finalized_transaction_count: 0,
            pbft_chain_size: 0,
            pbft_chain_head_hash: H256::zero(),
            pbft_chain_last_anchor_hash: H256::zero(),
            reward_votes_period: committed.cursor.period,
            reward_votes_round: committed.cursor.round,
            reward_votes_block_hash: committed.cursor.block_hash,
            reward_votes_reset_provenance_valid: reset_provenance_valid,
            sortition_changed: false,
            sortition_change_period: 0,
            sortition_change_interval_efficiency: 0,
            sortition_change_threshold_upper: 0,
            sortition_current_threshold_upper: 0,
            sortition_params_changes_count: 0,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            executed_pbft_block: false,
            manager_period: 0,
            pillar_processed_period: 0,
            pillar_request_period: 0,
            anchor_dag_cache_count: 0,
            final_chain_dispatched: false,
            final_chain_blocks_per_year: 0,
            final_chain_last_block: 0,
        };
        let validation = validate_pbft_finalization_live_mutation_report(&plan, live_report);
        let session = self
            .finalization_runtime_session
            .as_mut()
            .expect("finalization session checked above");
        *session = report_pbft_finalization_runtime_action(
            session.clone(),
            PbftFinalizationRuntimeActionResult {
                action: validation.action,
                success: validation.accepted,
                status: validation.status.as_u8(),
                error_code: validation.error_code,
            },
        );
        Ok(next_pbft_finalization_runtime_action(session))
    }

    /// Applies finalized transaction status and advances its finalization cursor.
    ///
    /// The manager derives the finalized period and canonical transaction bytes
    /// from its accepted plan, calls the native transaction owner while holding
    /// manager-before-transaction lock order, validates the accepted count, and
    /// reports the action through the native runtime. C++ supplies only account
    /// nonce facts read through the retained external EVM query boundary.
    /// Decode, storage, or transaction-state failures are fatal post-storage
    /// invariants and do not advance the cursor.
    pub fn advance_finalization_transaction_status<V: PbftFinalizedTransactionStatusPort>(
        &mut self,
        transactions: &V,
        cursor: u32,
        retention_window: u64,
        account_nonce_facts: Vec<crate::transaction_service::TransactionServiceAccountNonceFact>,
    ) -> Result<crate::pbft_finalize::PbftFinalizationRuntimeStep> {
        use crate::pbft_finalize::{
            PbftFinalizationLiveMutationReport, PbftFinalizationRuntimeAction,
            PbftFinalizationRuntimeActionResult, PbftFinalizationRuntimeStatus,
            PbftFinalizationRuntimeStep, next_pbft_finalization_runtime_action,
            report_pbft_finalization_runtime_action,
            validate_pbft_finalization_live_mutation_report,
        };

        let Some(session) = self.finalization_runtime_session.as_ref() else {
            return Ok(PbftFinalizationRuntimeStep {
                runtime_status: PbftFinalizationRuntimeStatus::ActionMismatch,
                has_action: false,
                action: None,
                action_index: 0,
                complete: false,
                error_code: "PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED".to_string(),
            });
        };
        let current_step = next_pbft_finalization_runtime_action(session);
        if current_step.runtime_status != PbftFinalizationRuntimeStatus::Active
            || !current_step.has_action
        {
            return Ok(current_step);
        }
        if cursor != current_step.action_index {
            let session = self
                .finalization_runtime_session
                .as_mut()
                .expect("finalization session checked above");
            session.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
            session.error_code = "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH".to_string();
            return Ok(next_pbft_finalization_runtime_action(session));
        }

        let action = current_step
            .action
            .expect("active finalization step must carry an action");
        if action != PbftFinalizationRuntimeAction::UpdateFinalizedTransactions {
            let session = self
                .finalization_runtime_session
                .as_mut()
                .expect("finalization session checked above");
            *session = report_pbft_finalization_runtime_action(
                session.clone(),
                PbftFinalizationRuntimeActionResult {
                    action,
                    success: false,
                    status: PbftFinalizationRuntimeStatus::ActionMismatch.as_u8(),
                    error_code: "PBFT_FINALIZE_RUNTIME_ACTION_MISMATCH".to_string(),
                },
            );
            return Ok(next_pbft_finalization_runtime_action(session));
        }

        let plan = self
            .finalization_runtime_plan
            .clone()
            .ok_or_else(|| anyhow!("PBFT_FINALIZE_RUNTIME_PLAN_NOT_STARTED"))?;
        let report = transactions
            .update_finalized_transactions_from_period_data(
                plan.storage_write_intent.block_period,
                retention_window,
                account_nonce_facts,
                &plan.storage_write_intent.period_data_rlp,
            )
            .map_err(|error| {
                anyhow!("PBFT_FINALIZE_POST_STORAGE_TRANSACTION_STATUS_INVARIANT:{error}")
            })?;
        let validation = validate_pbft_finalization_live_mutation_report(
            &plan,
            PbftFinalizationLiveMutationReport {
                action,
                block_period: plan.storage_write_intent.block_period,
                pbft_block_hash: plan.storage_write_intent.pbft_block_hash,
                anchor_hash: plan.storage_write_intent.anchor_hash,
                dag_finalized_count: 0,
                finalized_transaction_count: report.accepted_count,
                pbft_chain_size: 0,
                pbft_chain_head_hash: H256::zero(),
                pbft_chain_last_anchor_hash: H256::zero(),
                reward_votes_period: 0,
                reward_votes_round: 0,
                reward_votes_block_hash: H256::zero(),
                reward_votes_reset_provenance_valid: false,
                sortition_changed: false,
                sortition_change_period: 0,
                sortition_change_interval_efficiency: 0,
                sortition_change_threshold_upper: 0,
                sortition_current_threshold_upper: 0,
                sortition_params_changes_count: 0,
                rounds_count_dynamic_lambda: 0,
                dynamic_lambda: 0,
                executed_pbft_block: false,
                manager_period: 0,
                pillar_processed_period: 0,
                pillar_request_period: 0,
                anchor_dag_cache_count: 0,
                final_chain_dispatched: false,
                final_chain_blocks_per_year: 0,
                final_chain_last_block: 0,
            },
        );
        let session = self
            .finalization_runtime_session
            .as_mut()
            .expect("finalization session checked above");
        *session = report_pbft_finalization_runtime_action(
            session.clone(),
            PbftFinalizationRuntimeActionResult {
                action: validation.action,
                success: validation.accepted,
                status: validation.status.as_u8(),
                error_code: validation.error_code,
            },
        );
        Ok(next_pbft_finalization_runtime_action(session))
    }

    /// Drains every consecutive PBFT-manager-owned finalization action.
    ///
    /// This task owns PBFT-chain publication, dynamic-lambda persistence and
    /// live publication, executed-status persistence/live publication, anchor
    /// cache clearing, validation, and cursor reporting. It stops before DAG,
    /// transaction, sortition, vote, FinalChain/EVM, pillar, period-advance, or
    /// network work. Storage commits precede corresponding live-state updates,
    /// and manager-before-chain lock order is preserved.
    ///
    /// Missing plans and storage/chain failures return `Err`. A missing runtime
    /// session preserves the executor contract by returning an `ActionMismatch`
    /// step. Deterministic storage or live-state rejections are reported to the
    /// runtime and returned as a terminal drain outcome with a stable high-level
    /// error label.
    pub fn drain_finalization_owned_actions(&mut self) -> Result<PbftFinalizationOwnedActionDrain> {
        use crate::pbft_finalize::{
            PbftFinalizationRuntimeAction, PbftFinalizationRuntimeActionResult,
            PbftFinalizationRuntimeStatus, PbftFinalizationStorageWriteStage,
            apply_pbft_finalization_storage_writes, next_pbft_finalization_runtime_action,
            report_pbft_finalization_runtime_action,
            validate_pbft_finalization_live_mutation_report,
        };

        const DYNAMIC_LAMBDA_STAGE: u8 = 1;
        const EXECUTED_STATUS_STAGE: u8 = 2;

        let plan = self
            .finalization_runtime_plan
            .clone()
            .ok_or_else(|| anyhow!("PBFT_FINALIZE_RUNTIME_PLAN_NOT_STARTED"))?;
        let write_set = plan.storage_write_intent.clone();
        let mut cleared_anchor_dag_cache = false;
        let mut has_snapshot = false;

        loop {
            let Some(session) = self.finalization_runtime_session.as_ref() else {
                return Ok(owned_finalization_drain_outcome(
                    cleared_anchor_dag_cache,
                    has_snapshot,
                    crate::pbft_finalize::PbftFinalizationRuntimeStep {
                        runtime_status: PbftFinalizationRuntimeStatus::ActionMismatch,
                        has_action: false,
                        action: None,
                        action_index: 0,
                        complete: false,
                        error_code: "PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED".to_string(),
                    },
                    "",
                ));
            };
            let current_step = next_pbft_finalization_runtime_action(session);
            if current_step.runtime_status != PbftFinalizationRuntimeStatus::Active
                || !current_step.has_action
            {
                return Ok(owned_finalization_drain_outcome(
                    cleared_anchor_dag_cache,
                    has_snapshot,
                    current_step,
                    "",
                ));
            }
            let action = current_step
                .action
                .expect("active finalization step must carry an action");

            let (success, status, error_code, rejection_code) = match action {
                PbftFinalizationRuntimeAction::UpdatePbftChain => {
                    let mut chain = self.chain.write().expect("PBFT chain lock poisoned");
                    let head = chain
                        .state
                        .project_update(write_set.pbft_block_hash, write_set.anchor_hash)?;
                    let validation = validate_pbft_finalization_live_mutation_report(
                        &plan,
                        crate::pbft_finalize::PbftFinalizationLiveMutationReport {
                            pbft_chain_size: head.size,
                            pbft_chain_head_hash: head.last_pbft_block_hash,
                            pbft_chain_last_anchor_hash: head.last_non_null_pbft_dag_anchor_hash,
                            ..base_owned_finalization_live_report(action, &write_set)
                        },
                    );
                    if validation.accepted {
                        chain
                            .state
                            .update(write_set.pbft_block_hash, write_set.anchor_hash)?;
                    }
                    (
                        validation.accepted,
                        validation.status.as_u8(),
                        validation.error_code,
                        "PBFT_FINALIZE_CHAIN_LIVE_REJECTED",
                    )
                }
                PbftFinalizationRuntimeAction::ApplyDynamicLambda => {
                    let apply_result = apply_pbft_finalization_storage_writes(
                        self.storage.as_ref(),
                        &write_set,
                        vec![PbftFinalizationStorageWriteStage {
                            stage: DYNAMIC_LAMBDA_STAGE,
                            rounds_count_dynamic_lambda: write_set.rounds_count_dynamic_lambda,
                            dynamic_lambda: write_set.dynamic_lambda,
                            ..Default::default()
                        }],
                        false,
                    )?;
                    if !apply_result.status.is_success() {
                        (
                            false,
                            apply_result.status.as_u8(),
                            apply_result.error_code,
                            "PBFT_FINALIZE_DYNAMIC_LAMBDA_STORAGE_REJECTED",
                        )
                    } else {
                        self.state.apply_committed_dynamic_lambda(
                            write_set.rounds_count_dynamic_lambda,
                            write_set.dynamic_lambda,
                        );
                        has_snapshot = true;
                        let snapshot = self.state.snapshot();
                        let validation = validate_pbft_finalization_live_mutation_report(
                            &plan,
                            crate::pbft_finalize::PbftFinalizationLiveMutationReport {
                                rounds_count_dynamic_lambda: snapshot.rounds_count_dynamic_lambda,
                                dynamic_lambda: snapshot.dynamic_lambda_ms,
                                ..base_owned_finalization_live_report(action, &write_set)
                            },
                        );
                        (
                            validation.accepted,
                            if validation.accepted {
                                apply_result.status.as_u8()
                            } else {
                                validation.status.as_u8()
                            },
                            validation.error_code,
                            "PBFT_FINALIZE_DYNAMIC_LAMBDA_LIVE_REJECTED",
                        )
                    }
                }
                PbftFinalizationRuntimeAction::PersistExecutedStatus => {
                    let apply_result = apply_pbft_finalization_storage_writes(
                        self.storage.as_ref(),
                        &write_set,
                        vec![PbftFinalizationStorageWriteStage {
                            stage: EXECUTED_STATUS_STAGE,
                            ..Default::default()
                        }],
                        false,
                    )?;
                    (
                        apply_result.status.is_success(),
                        apply_result.status.as_u8(),
                        apply_result.error_code,
                        "PBFT_FINALIZE_EXECUTED_STATUS_STORAGE_REJECTED",
                    )
                }
                PbftFinalizationRuntimeAction::SetExecutedFlag => {
                    self.state.apply_committed_finalization_executed_status(
                        write_set.executed_pbft_status,
                    );
                    has_snapshot = true;
                    let validation = validate_pbft_finalization_live_mutation_report(
                        &plan,
                        crate::pbft_finalize::PbftFinalizationLiveMutationReport {
                            executed_pbft_block: self.state.snapshot().executed_pbft_block,
                            ..base_owned_finalization_live_report(action, &write_set)
                        },
                    );
                    (
                        validation.accepted,
                        validation.status.as_u8(),
                        validation.error_code,
                        "PBFT_FINALIZE_EXECUTED_FLAG_LIVE_REJECTED",
                    )
                }
                PbftFinalizationRuntimeAction::ClearAnchorDagCache => {
                    self.state.clear_cached_anchor_dag_order();
                    has_snapshot = true;
                    let validation = validate_pbft_finalization_live_mutation_report(
                        &plan,
                        crate::pbft_finalize::PbftFinalizationLiveMutationReport {
                            anchor_dag_cache_count: self.state.cached_anchor_dag_order_count(),
                            ..base_owned_finalization_live_report(action, &write_set)
                        },
                    );
                    if validation.accepted {
                        cleared_anchor_dag_cache = true;
                    }
                    (
                        validation.accepted,
                        validation.status.as_u8(),
                        validation.error_code,
                        "PBFT_FINALIZE_ANCHOR_DAG_CACHE_LIVE_REJECTED",
                    )
                }
                _ => {
                    return Ok(owned_finalization_drain_outcome(
                        cleared_anchor_dag_cache,
                        has_snapshot,
                        current_step,
                        "",
                    ));
                }
            };

            let session = self
                .finalization_runtime_session
                .as_mut()
                .expect("finalization session checked above");
            *session = report_pbft_finalization_runtime_action(
                session.clone(),
                PbftFinalizationRuntimeActionResult {
                    action,
                    success,
                    status,
                    error_code,
                },
            );
            let next_step = next_pbft_finalization_runtime_action(session);
            if !success {
                return Ok(owned_finalization_drain_outcome(
                    cleared_anchor_dag_cache,
                    has_snapshot,
                    next_step,
                    rejection_code,
                ));
            }
        }
    }
}

impl PbftManagerService {
    /// Creates the native PBFT manager owner.
    ///
    /// The supplied runtime, storage, and chain are installed atomically into
    /// one initially idle runtime state. All session slots start empty, queue
    /// draining starts from a fresh cursor, and the reward-vote reset
    /// generation starts at zero.
    pub fn new(
        state: PbftManagerRuntime,
        storage: Arc<Storage>,
        chain: crate::pbft_chain::PbftChainService,
    ) -> Self {
        Self {
            runtime: Arc::new(Mutex::new(PbftManagerRuntimeState {
                state,
                storage,
                period_data_queue: crate::period_data_queue::PeriodDataQueue::new(),
                pbft_sync_queue_drain_session:
                    crate::pbft_sync::create_pbft_sync_queue_drain_session(),
                pbft_sync_admission_session: None,
                state_action_effect_session: None,
                runtime_session: None,
                proposal_session: None,
                finalization_runtime_session: None,
                finalization_runtime_plan: None,
                finalization_sortition_commit_request: None,
                finalization_reward_votes_reset_generation: 0,
                chain,
            })),
        }
    }

    /// Locks the complete native PBFT manager runtime state.
    ///
    /// The returned guard permits field-level access for bridge adapters while
    /// keeping lock ownership in `rustaxa-consensus`. Lock poisoning panics
    /// because continuing after a panic during consensus mutation could expose
    /// a partially updated in-memory state.
    pub fn lock(&self) -> PbftManagerGuard<'_> {
        PbftManagerGuard(self.runtime.lock().expect("PBFT manager lock poisoned"))
    }

    /// Starts a synced-period admission session under the manager lock.
    ///
    /// The supplied immutable candidate facts replace any stale admission
    /// cursor. Subsequent checks and reports must enter through the task
    /// methods below; callers never receive mutable session state.
    pub fn begin_pbft_sync_admission(&self, fact: crate::pbft_sync::PbftSyncAdmissionInitialFact) {
        self.lock().pbft_sync_admission_session =
            Some(crate::pbft_sync::create_pbft_sync_admission_session(fact));
    }

    /// Returns the current synced-period admission step without advancing it.
    ///
    /// Terminal or failed steps consume the manager-owned session. `None`
    /// means no admission cursor is active.
    pub fn pbft_sync_admission_next(
        &self,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        let mut runtime = self.lock();
        let step = runtime
            .pbft_sync_admission_session
            .as_ref()
            .map(crate::pbft_sync::next_pbft_sync_admission_session)?;
        clear_terminal_pbft_sync_admission(&mut runtime, &step);
        Some(step)
    }

    /// Reports one non-transaction synced-period validation result.
    ///
    /// Cursor/check mismatches become terminal contract errors and consume the
    /// session. `None` means no admission cursor is active.
    pub fn report_pbft_sync_admission_status(
        &self,
        cursor: u32,
        check: crate::pbft_sync::PbftSyncProcessRuntimeNextCheck,
        final_chain_status: crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus,
        fact_status: crate::pbft_sync::PbftSyncFactStatus,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        let mut runtime = self.lock();
        let step = crate::pbft_sync::report_pbft_sync_admission_status(
            runtime.pbft_sync_admission_session.as_mut()?,
            cursor,
            check,
            final_chain_status,
            fact_status,
        );
        clear_terminal_pbft_sync_admission(&mut runtime, &step);
        Some(step)
    }

    /// Reports the transaction lookup result requested by the active cursor.
    ///
    /// The manager validates the expected transaction-check stage and consumes
    /// every terminal or failed session before returning.
    pub fn report_pbft_sync_admission_transactions(
        &self,
        cursor: u32,
        report: crate::pbft_sync::PbftSyncAdmissionTransactionReport,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        let mut runtime = self.lock();
        let step = crate::pbft_sync::report_pbft_sync_admission_transactions(
            runtime.pbft_sync_admission_session.as_mut()?,
            cursor,
            report,
        );
        clear_terminal_pbft_sync_admission(&mut runtime, &step);
        Some(step)
    }

    /// Aborts and consumes the current synced-period admission cursor.
    ///
    /// The returned terminal step preserves the native abort diagnostic.
    /// `None` means no session was active.
    pub fn abort_pbft_sync_admission(
        &self,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        let mut runtime = self.lock();
        let step = crate::pbft_sync::abort_pbft_sync_admission_session(
            runtime.pbft_sync_admission_session.as_mut()?,
        );
        runtime.pbft_sync_admission_session = None;
        Some(step)
    }
}

fn clear_terminal_pbft_sync_admission(
    runtime: &mut PbftManagerRuntimeState,
    step: &crate::pbft_sync::PbftSyncAdmissionSessionStep,
) {
    if step.complete || !step.can_continue {
        runtime.pbft_sync_admission_session = None;
    }
}

/// Stable PBFT manager state codes used by the CXX bridge.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeStateCode {
    /// Value-proposal state.
    ValueProposal,
    /// Filtering / identify-leader state.
    Filter,
    /// Certifying state.
    Certify,
    /// First finish state.
    Finish,
    /// Second finish / polling state.
    FinishPolling,
    /// Unknown bridge state.
    Unknown,
}

/// Live-object availability status for one proposed PBFT leader candidate.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerLeaderCandidateStatus {
    /// The candidate block resolved, passed current validation, and can be selected.
    Ready,
    /// The proposal vote pointed at the null PBFT block hash and must be ignored.
    NullVoteBlockHash,
    /// The candidate PBFT block is already present in the local PBFT chain.
    BlockInChain,
    /// C++ could not resolve or validate the proposed block for the vote.
    BlockMissingOrInvalid,
    /// The vote did not carry a positive proposer weight.
    InvalidVoteWeight,
    /// Unknown bridge status.
    Unknown,
}

/// Validation result for a proposed PBFT block candidate.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerLeaderBlockValidationStatus {
    /// The block was already marked valid in the proposed-block sidecar.
    AlreadyValid,
    /// C++ live validation accepted the block.
    Validated,
    /// C++ live validation rejected the block.
    Rejected,
    /// Unknown bridge status.
    Unknown,
}

/// Live validation status for one proposed-block admission attempt.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerCandidateAdmissionValidationStatus {
    /// Rust has not requested live block validation yet.
    NotChecked,
    /// C++ live validation accepted the proposed block.
    Valid,
    /// C++ live validation rejected the proposed block.
    Invalid,
    /// Unknown bridge status.
    Unknown,
}

impl PbftManagerCandidateAdmissionValidationStatus {
    /// Stable bridge code for proposed-block admission validation status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NotChecked => 0,
            Self::Valid => 1,
            Self::Invalid => 2,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge status code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::NotChecked,
            1 => Self::Valid,
            2 => Self::Invalid,
            _ => Self::Unknown,
        }
    }
}

/// Runtime action for Rust-owned proposed-block admission.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerCandidateAdmissionAction {
    /// C++ must lookup the live proposed block sidecar before retrying.
    RequestLookup,
    /// C++ must validate the found block and report the result.
    RequestValidation,
    /// The block is accepted for use by the caller.
    Accept,
    /// The block is rejected by supplied facts.
    Reject,
    /// The local proposed-block sidecar is missing and execution should retry
    /// when the block arrives through existing network/sync boundaries.
    DeferMissingBlock,
    /// Supplied bridge facts violate the admission contract.
    ContractError,
}

impl PbftManagerCandidateAdmissionAction {
    /// Stable bridge code for proposed-block admission action.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::RequestLookup => 0,
            Self::RequestValidation => 1,
            Self::Accept => 2,
            Self::Reject => 3,
            Self::DeferMissingBlock => 4,
            Self::ContractError => 255,
        }
    }
}

/// Final proposed-block admission status selected by Rust.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerCandidateAdmissionStatus {
    /// The proposed-block sidecar lookup is still needed.
    LookupRequired,
    /// Live block validation is still needed.
    ValidationRequired,
    /// The block was already marked valid and is accepted.
    AcceptedAlreadyValid,
    /// The block was newly validated and is accepted.
    AcceptedNewlyValidated,
    /// The proposed-block sidecar did not contain the requested block.
    BlockMissing,
    /// Live block validation rejected the candidate.
    ValidationRejected,
    /// Supplied bridge facts violate the admission contract.
    InvalidBridgeFacts,
}

impl PbftManagerCandidateAdmissionStatus {
    /// Stable bridge code for proposed-block admission status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::LookupRequired => 0,
            Self::ValidationRequired => 1,
            Self::AcceptedAlreadyValid => 2,
            Self::AcceptedNewlyValidated => 3,
            Self::BlockMissing => 4,
            Self::ValidationRejected => 5,
            Self::InvalidBridgeFacts => 255,
        }
    }
}

impl PbftManagerLeaderBlockValidationStatus {
    /// Stable bridge code for proposed-block validation status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::AlreadyValid => 0,
            Self::Validated => 1,
            Self::Rejected => 2,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge status code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::AlreadyValid,
            1 => Self::Validated,
            2 => Self::Rejected,
            _ => Self::Unknown,
        }
    }
}

impl PbftManagerLeaderCandidateStatus {
    /// Stable bridge code for the candidate status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::NullVoteBlockHash => 1,
            Self::BlockInChain => 2,
            Self::BlockMissingOrInvalid => 3,
            Self::InvalidVoteWeight => 4,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge status code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Ready,
            1 => Self::NullVoteBlockHash,
            2 => Self::BlockInChain,
            3 => Self::BlockMissingOrInvalid,
            4 => Self::InvalidVoteWeight,
            _ => Self::Unknown,
        }
    }
}

/// Live facts for one proposal vote before Rust derives candidate status.
///
/// Inputs:
/// - `vote_hash`, `block_hash`, `period`, `credential`, and
///   `voter_public_key` identify and rank the proposal vote.
/// - `weight_found` and `weight` describe the validated proposer weight.
/// - `block_in_chain`, `proposed_block_found`, and `block_validation_status`
///   summarize C++ live sidecar/PBFT-chain/DAG validation without deciding
///   candidate eligibility in C++.
/// - `pivot_hash` is the proposed block pivot hash when the block was found.
///
/// Outputs are produced by `plan_pbft_manager_leader_candidates`.
///
/// Invariants:
/// - Rust owns the legacy candidate-status derivation and ranking.
/// - C++ remains responsible for live object lookup and block validation until
///   those dependencies move into Rust.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderCandidateInputFact {
    /// Signed proposal vote hash.
    pub vote_hash: H256,
    /// Proposed PBFT block hash from the proposal vote.
    pub block_hash: H256,
    /// Vote period used for live block lookup.
    pub period: u64,
    /// Proposal vote VRF output.
    pub credential: [u8; 64],
    /// Recovered voter public key.
    pub voter_public_key: [u8; 64],
    /// True when proposer weight was present on the live vote.
    pub weight_found: bool,
    /// Validated proposer vote weight.
    pub weight: u64,
    /// True when the proposed block is already in the PBFT chain.
    pub block_in_chain: bool,
    /// True when the proposed-block sidecar resolved the block.
    pub proposed_block_found: bool,
    /// Proposed-block validation status.
    pub block_validation_status: PbftManagerLeaderBlockValidationStatus,
    /// Pivot DAG hash for a found proposed block.
    pub pivot_hash: H256,
}

/// C++-originated facts for one proposed PBFT block admission attempt.
///
/// Inputs:
/// - `period` and `block_hash` identify the candidate the caller wants to use.
/// - `lookup_performed`, `proposed_block_found`, and
///   `proposed_block_already_valid` report the proposed-block sidecar lookup.
/// - `validation_status` reports the live validation result only after Rust
///   asks for validation.
///
/// Outputs are produced by `plan_pbft_manager_candidate_admission`.
///
/// Invariants and edge behavior:
/// - Rust owns the admission state machine and mark-valid decision.
/// - C++ owns the live sidecar lookup, block validation checks, and sidecar
///   mutation requested by the final plan.
/// - Missing blocks and failed validation are explicit rejections; malformed
///   fact order returns a contract error.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerCandidateAdmissionFact {
    /// Candidate PBFT period.
    pub period: u64,
    /// Candidate PBFT block hash.
    pub block_hash: H256,
    /// True once C++ has looked up the proposed-block sidecar.
    pub lookup_performed: bool,
    /// True when the proposed-block sidecar resolved the candidate block.
    pub proposed_block_found: bool,
    /// True when the resolved proposed block was already marked valid.
    pub proposed_block_already_valid: bool,
    /// Live validation result supplied after Rust requests validation.
    pub validation_status: PbftManagerCandidateAdmissionValidationStatus,
}

/// Side-effect-free proposed-block admission plan for C++ execution.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerCandidateAdmissionPlan {
    /// Runtime action C++ must take.
    pub action: PbftManagerCandidateAdmissionAction,
    /// Current admission status.
    pub status: PbftManagerCandidateAdmissionStatus,
    /// True when C++ must mark the proposed block valid before returning it.
    pub mark_valid: bool,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

/// Rust-owned outcome for PBFT leader candidate selection.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerLeaderSelectionStatus {
    /// A leader block/vote pair was selected.
    Selected,
    /// No proposal vote facts were supplied.
    Empty,
    /// Candidate facts were present, but none were selectable.
    NoEligibleCandidate,
    /// One or more candidate facts were malformed.
    InvalidFact,
}

impl PbftManagerLeaderSelectionStatus {
    /// Stable bridge code for the selection status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Selected => 0,
            Self::Empty => 1,
            Self::NoEligibleCandidate => 2,
            Self::InvalidFact => 3,
        }
    }
}

/// Candidate facts for deterministic PBFT leader selection.
///
/// Inputs:
/// - `vote_hash` and `block_hash` identify the live C++ objects to materialize
///   after Rust selection.
/// - `credential` is the 64-byte VRF output from the proposal vote.
/// - `voter_public_key` is the 64-byte secp256k1 public key recovered from the
///   vote signature.
/// - `weight` is the already-validated proposer vote weight.
/// - `status` and `pivot_hash` summarize C++ live-object resolution and
///   candidate validation. Rust uses these facts only after applying legacy
///   proposal ranking.
///
/// Outputs are produced by `plan_pbft_manager_leader_selection`.
///
/// Invariants:
/// - Candidate ordering is computed from the legacy minimum of
///   `sha3(rlp([credential, voter_public_key, i]))` for `i = 1..=weight`.
/// - Duplicate rank hashes retain the last input candidate, matching legacy
///   `std::map<h256, vote>` assignment behavior.
/// - Null-anchor candidates are eligible only as a fallback when no non-null
///   candidate wins.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderCandidateFact {
    /// Signed proposal vote hash.
    pub vote_hash: H256,
    /// Proposed PBFT block hash.
    pub block_hash: H256,
    /// Vote period used for live block lookup.
    pub period: u64,
    /// Proposal vote VRF output.
    pub credential: [u8; 64],
    /// Recovered voter public key.
    pub voter_public_key: [u8; 64],
    /// Validated proposal vote weight.
    pub weight: u64,
    /// Candidate live-object/validation status.
    pub status: PbftManagerLeaderCandidateStatus,
    /// Pivot DAG hash for a ready candidate block.
    pub pivot_hash: H256,
}

/// Side-effect-free PBFT leader selection plan.
///
/// The selected hashes identify the C++ live vote/block pair to return from the
/// shim. Empty and rejected plans return zero hashes and `selected = false`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderSelectionPlan {
    /// Selection status.
    pub status: PbftManagerLeaderSelectionStatus,
    /// True when `selected_vote_hash` and `selected_block_hash` are meaningful.
    pub selected: bool,
    /// Selected proposal vote hash.
    pub selected_vote_hash: H256,
    /// Selected PBFT block hash.
    pub selected_block_hash: H256,
    /// Selected vote period.
    pub selected_period: u64,
    /// True when the selected block is the null-anchor fallback.
    pub selected_from_null_anchor: bool,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

/// Proposed-block command emitted by grouped PBFT candidate planning.
///
/// C++ applies this command to mark a proposed PBFT block valid only after Rust
/// has accepted the corresponding validation report.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderValidBlockCommand {
    /// PBFT period for the proposed block.
    pub period: u64,
    /// Proposed PBFT block hash to mark valid.
    pub block_hash: H256,
}

/// Grouped PBFT leader-candidate plan.
///
/// The selection fields mirror `PbftManagerLeaderSelectionPlan`. The
/// `valid_blocks` commands are emitted for unmarked candidate blocks whose live
/// validation was reported as accepted, keeping proposed-block status mutation
/// under the Rust-planned route.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderCandidatePlan {
    /// Selection status.
    pub status: PbftManagerLeaderSelectionStatus,
    /// True when `selected_vote_hash` and `selected_block_hash` are meaningful.
    pub selected: bool,
    /// Selected proposal vote hash.
    pub selected_vote_hash: H256,
    /// Selected PBFT block hash.
    pub selected_block_hash: H256,
    /// Selected vote period.
    pub selected_period: u64,
    /// True when the selected block is the null-anchor fallback.
    pub selected_from_null_anchor: bool,
    /// Proposed blocks that C++ should mark valid.
    pub valid_blocks: Vec<PbftManagerLeaderValidBlockCommand>,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

/// Tri-state fact status for Rust-owned PBFT block validation orchestration.
///
/// C++ reports each live-object check with this status after Rust asks for the
/// next check. `Missing` is distinct from `Invalid` for FinalChain lag and DAG
/// order availability, where the caller may choose to retry or delay instead of
/// treating the peer/block as malicious.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBlockValidationFactStatus {
    /// The fact has not been supplied yet.
    NotChecked,
    /// The live check accepted the fact.
    Valid,
    /// The live check rejected the fact.
    Invalid,
    /// The live check could not resolve required data.
    Missing,
    /// The check is not required for this block/context.
    NotRequired,
    /// Unknown bridge status.
    Unknown,
}

impl PbftManagerBlockValidationFactStatus {
    /// Stable bridge code for a PBFT block-validation fact status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NotChecked => 0,
            Self::Valid => 1,
            Self::Invalid => 2,
            Self::Missing => 3,
            Self::NotRequired => 4,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge status code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::NotChecked,
            1 => Self::Valid,
            2 => Self::Invalid,
            3 => Self::Missing,
            4 => Self::NotRequired,
            _ => Self::Unknown,
        }
    }
}

/// Next live check requested by Rust PBFT block validation planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBlockValidationNextCheck {
    /// No further live check is needed.
    None,
    /// Validate the block's previous PBFT hash against the current PBFT chain.
    CheckPbftChain,
    /// Validate the PBFT block FinalChain/state-root hash.
    ValidateFinalChainHash,
    /// Check reward-vote availability/validity for the candidate block.
    CheckRewardVotes,
    /// Validate PBFT block extra-data shape for the active hardfork.
    ValidateExtraData,
    /// Compare the embedded pillar block hash against the local pillar block.
    ValidatePillarBlock,
    /// Resolve and verify DAG order for the candidate pivot.
    CheckDagOrder,
    /// Check DAG block weight after Rust requested and C++ cached the order.
    CheckDagWeight,
    /// Unknown bridge status.
    Unknown,
}

impl PbftManagerBlockValidationNextCheck {
    /// Stable bridge code for the next requested check.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::None => 255,
            Self::CheckPbftChain => 0,
            Self::ValidateFinalChainHash => 1,
            Self::CheckRewardVotes => 2,
            Self::ValidateExtraData => 3,
            Self::ValidatePillarBlock => 4,
            Self::CheckDagOrder => 5,
            Self::CheckDagWeight => 6,
            Self::Unknown => 254,
        }
    }
}

/// PBFT block-validation runtime action selected by Rust.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBlockValidationAction {
    /// C++ must execute `next_check` and call the planner again with the result.
    RunCheck,
    /// The block is accepted by all required checks.
    Accept,
    /// The block is rejected by a supplied fact.
    Reject,
    /// The FinalChain/state-root fact is missing and the caller may wait/retry.
    WaitForFinalization,
    /// Supplied bridge facts violate the validation contract.
    ContractError,
}

impl PbftManagerBlockValidationAction {
    /// Stable bridge code for a PBFT block-validation action.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::RunCheck => 0,
            Self::Accept => 1,
            Self::Reject => 2,
            Self::WaitForFinalization => 3,
            Self::ContractError => 255,
        }
    }
}

/// Final PBFT block-validation status selected by Rust.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBlockValidationStatus {
    /// Validation is still waiting for C++ to run a requested check.
    Pending,
    /// All required checks accepted the PBFT block.
    Accepted,
    /// Previous PBFT hash/chain validation failed.
    PbftChainInvalid,
    /// FinalChain/state-root validation is behind execution.
    FinalChainHashMissing,
    /// FinalChain/state-root validation rejected the block.
    FinalChainHashInvalid,
    /// Reward votes rejected the block.
    RewardVotesInvalid,
    /// Extra-data shape rejected the block.
    ExtraDataInvalid,
    /// Embedded/local pillar block facts rejected the block.
    PillarBlockInvalid,
    /// DAG order could not be resolved.
    DagOrderMissing,
    /// DAG order hash rejected the block.
    DagOrderInvalid,
    /// DAG block weight rejected the block.
    DagWeightInvalid,
    /// Supplied bridge facts violate the validation contract.
    InvalidBridgeFacts,
}

impl PbftManagerBlockValidationStatus {
    /// Stable bridge code for a PBFT block-validation status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Accepted => 1,
            Self::PbftChainInvalid => 2,
            Self::FinalChainHashMissing => 3,
            Self::FinalChainHashInvalid => 4,
            Self::RewardVotesInvalid => 5,
            Self::ExtraDataInvalid => 6,
            Self::PillarBlockInvalid => 7,
            Self::DagOrderMissing => 8,
            Self::DagOrderInvalid => 9,
            Self::DagWeightInvalid => 10,
            Self::InvalidBridgeFacts => 255,
        }
    }
}

/// Compact fact bundle for Rust-owned PBFT block validation orchestration.
///
/// Inputs:
/// - Block identity fields let C++ correlate diagnostics and cached DAG state.
/// - `*_status` fields report the result of live checks only after Rust asks for
///   the corresponding `next_check`.
/// - `pivot_is_null`, `dag_order_cached`, `dag_order_required`,
///   `pillar_block_required`, and `dag_weight_check_required` encode
///   deterministic branch conditions that C++ can derive from existing sidecars
///   without deciding final acceptance.
///
/// Outputs are produced by `plan_pbft_manager_block_validation`.
///
/// Invariants and edge behavior:
/// - Rust owns the ordering of all validation checks.
/// - C++ owns live PBFT chain, FinalChain, reward-vote, pillar, and DAG queries.
/// - Missing FinalChain hash facts return `WaitForFinalization`; proposal paths
///   may treat that as rejection, while sync paths can wait and retry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerBlockValidationFact {
    /// Candidate PBFT block hash.
    pub block_hash: H256,
    /// Candidate PBFT period.
    pub period: u64,
    /// Candidate pivot DAG block hash.
    pub pivot_hash: H256,
    /// True when the pivot hash is the null DAG anchor.
    pub pivot_is_null: bool,
    /// True when the C++ DAG-order sidecar already has cached order for pivot.
    pub dag_order_cached: bool,
    /// True when this validation context requires DAG order/hash validation.
    pub dag_order_required: bool,
    /// True when hardfork rules require local pillar-block hash comparison.
    pub pillar_block_required: bool,
    /// True when the resolved DAG order must pass the weight check.
    pub dag_weight_check_required: bool,
    /// PBFT-chain previous-hash validation status.
    pub pbft_chain_status: PbftManagerBlockValidationFactStatus,
    /// FinalChain/state-root validation status.
    pub final_chain_hash_status: PbftManagerBlockValidationFactStatus,
    /// Reward-vote validation status.
    pub reward_votes_status: PbftManagerBlockValidationFactStatus,
    /// Extra-data validation status.
    pub extra_data_status: PbftManagerBlockValidationFactStatus,
    /// Pillar block validation status.
    pub pillar_block_status: PbftManagerBlockValidationFactStatus,
    /// DAG order lookup/hash validation status.
    pub dag_order_status: PbftManagerBlockValidationFactStatus,
    /// DAG weight validation status.
    pub dag_weight_status: PbftManagerBlockValidationFactStatus,
}

/// Side-effect-free PBFT block-validation plan for C++ execution.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerBlockValidationPlan {
    /// Runtime action C++ must take.
    pub action: PbftManagerBlockValidationAction,
    /// Current validation status.
    pub status: PbftManagerBlockValidationStatus,
    /// Next live-object check requested when `action == RunCheck`.
    pub next_check: PbftManagerBlockValidationNextCheck,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

/// Stateful PBFT block-validation session owned by Rust.
///
/// Purpose:
/// - Wraps `plan_pbft_manager_block_validation` so proposal and sync callers
///   share one Rust-owned validation cursor instead of mutating bridge facts in
///   separate C++ loops.
///
/// Inputs/outputs:
/// - Constructed from the initial compact validation fact bundle.
/// - `next_pbft_manager_block_validation_session` returns the next requested
///   check or terminal plan.
/// - `report_pbft_manager_block_validation_session_check` applies the result
///   of the requested live check and immediately returns the next plan.
///
/// Invariants and edge behavior:
/// - C++ may only report a status for the check Rust most recently requested.
/// - DAG-order reports may update `dag_weight_check_required` because that
///   fact is discovered while executing the live DAG order check.
/// - Reporting `NotChecked` is only accepted as a retry reset for the pending
///   FinalChain hash check after a wait-for-finalization outcome.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerBlockValidationSession {
    /// Current accumulated validation facts.
    pub fact: PbftManagerBlockValidationFact,
    pending_check: Option<PbftManagerBlockValidationNextCheck>,
}

/// Stable proposal-construction session status selected by Rust.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerProposalStatus {
    /// More executor facts are required before Rust can produce a command.
    Active,
    /// Rust returned a build command for the C++ compatibility executor.
    BuildReady,
    /// No local wallet can propose for this period/round.
    NoEligibleWallet,
    /// FinalChain hash was not available, so proposal must be skipped.
    MissingFinalChainHash,
    /// Hardfork rules require extra data that C++ could not materialize.
    MissingExtraData,
    /// C++ reported that the requested DAG order could not be loaded.
    MissingDagOrder,
    /// Supplied facts or reports violate the bridge contract.
    InvalidBridgeFacts,
}

impl PbftManagerProposalStatus {
    /// Stable bridge code for proposal-construction status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::BuildReady => 1,
            Self::NoEligibleWallet => 2,
            Self::MissingFinalChainHash => 3,
            Self::MissingExtraData => 4,
            Self::MissingDagOrder => 5,
            Self::InvalidBridgeFacts => 255,
        }
    }
}

/// FinalChain-hash composition status for PBFT proposal and sync validation.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerFinalChainHashStatus {
    /// Candidate hash matches the local FinalChain block hash for the period.
    Valid,
    /// FinalChain has not produced a finalized block for the requested period.
    Missing,
    /// Candidate hash does not match the local FinalChain block hash.
    Invalid,
    /// Bridge supplied an unsupported status code.
    Unknown,
}

impl PbftManagerFinalChainHashStatus {
    /// Stable bridge code for FinalChain hash validation status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Valid => 0,
            Self::Missing => 1,
            Self::Invalid => 2,
            Self::Unknown => 255,
        }
    }

    /// Decodes a stable bridge status code into a domain status.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Valid,
            1 => Self::Missing,
            2 => Self::Invalid,
            _ => Self::Unknown,
        }
    }
}

/// Result of validating a candidate FinalChain hash for one PBFT period.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerFinalChainHashValidationResult {
    /// Validation status after comparing candidate and local FinalChain hash.
    pub status: PbftManagerFinalChainHashStatus,
    /// Expected FinalChain hash when status is `Valid` or `Invalid`.
    pub expected_hash: H256,
    /// Stable error code for the validation outcome.
    pub error_code: String,
}

/// Stable proposal-construction session action for the C++ executor.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerProposalAction {
    /// C++ must load DAG order and gas facts for `requested_anchor_hash`.
    RequestDagOrder,
    /// C++ can materialize the PBFT block from the returned command fields.
    BuildProposal,
    /// No proposal should be produced.
    SkipProposal,
    /// Supplied facts or reports violate the bridge contract.
    ContractError,
}

impl PbftManagerProposalAction {
    /// Stable bridge code for proposal-construction action.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::RequestDagOrder => 0,
            Self::BuildProposal => 1,
            Self::SkipProposal => 2,
            Self::ContractError => 255,
        }
    }
}

/// Wallet eligibility fact supplied to Rust proposal construction.
///
/// C++ still executes DPoS and VRF/sortition checks against live subsystems, but
/// Rust owns final filtering from those facts. `wallet_index` is an index into
/// the local wallet vector retained by the C++ compatibility executor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerProposalWalletFact {
    /// Stable index of the candidate wallet in the C++ local wallet vector.
    pub wallet_index: u64,
    /// Whether DPoS eligibility accepted this wallet for the proposal period.
    pub dpos_eligible: bool,
    /// Whether VRF sortition accepted this wallet for the proposal round.
    pub sortition_valid: bool,
}

/// One ordered DAG block fact supplied for a requested anchor.
///
/// Inputs:
/// - `hash` preserves canonical DAG order.
/// - `gas_estimation` is the block gas estimate projected to the configured
///   PBFT gas-limit domain for proposal clipping.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerProposalDagBlockFact {
    /// DAG block hash.
    pub hash: H256,
    /// Gas estimate used for PBFT block clipping.
    pub gas_estimation: u64,
}

/// Initial fact bundle for Rust-owned PBFT proposal construction.
///
/// Purpose:
/// - Move deterministic proposer eligibility, null-anchor fallback, DAG anchor
///   selection, FinalChain/extra-data skip status, gas clipping, and order-hash
///   calculation into Rust.
///
/// Invariants and edge behavior:
/// - C++ supplies live facts and materializes the returned build command.
/// - FinalChain/EVM, DAG storage, key-manager signing, vote sidecars, and
///   network effects remain executor boundaries.
/// - DAG order is requested through the session so Rust can ask for a recompute
///   when gas clipping selects a closer anchor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerProposalInitialFact {
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Hash of the previous PBFT block.
    pub previous_pbft_block_hash: H256,
    /// Last non-null DAG anchor from the PBFT chain, normalized by C++ when the
    /// previous anchor is null.
    pub last_period_dag_anchor_hash: H256,
    /// DAG genesis hash used by the null-anchor rule.
    pub dag_genesis_hash: H256,
    /// Configured maximum DAG block window used by legacy anchor selection.
    pub dag_blocks_size: u64,
    /// Configured GHOST move-back distance.
    pub ghost_path_move_back: u64,
    /// PBFT gas limit for the current proposal period.
    pub pbft_gas_limit: u64,
    /// Whether hardfork rules require PBFT block extra data.
    pub extra_data_required: bool,
    /// Whether C++ successfully materialized required extra data.
    pub extra_data_available: bool,
    /// Whether Rust FinalChain supplied the hash for this proposal period.
    pub final_chain_hash_valid: bool,
    /// FinalChain hash to embed in the PBFT block when valid.
    pub final_chain_hash: H256,
    /// Local wallet eligibility facts.
    pub wallets: Vec<PbftManagerProposalWalletFact>,
    /// GHOST path from the last period DAG anchor.
    pub ghost_path: Vec<H256>,
    /// Whether a non-finalized fallback anchor is available.
    pub has_non_finalized_fallback: bool,
    /// Fallback anchor selected from non-finalized DAG blocks when GHOST has no
    /// new anchor after the previous period anchor.
    pub non_finalized_fallback_hash: H256,
}

/// C++ report for one DAG-order request from the proposal session.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerProposalDagOrderReport {
    /// Anchor hash Rust requested.
    pub anchor_hash: H256,
    /// Ordered DAG block facts for the requested anchor.
    pub dag_blocks: Vec<PbftManagerProposalDagBlockFact>,
    /// True when C++ successfully loaded the order.
    pub order_available: bool,
}

/// One Rust-owned proposal-construction session step.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerProposalSessionStep {
    /// Action requested from C++.
    pub action: PbftManagerProposalAction,
    /// Current proposal status.
    pub status: PbftManagerProposalStatus,
    /// Anchor requested when `action == RequestDagOrder`.
    pub requested_anchor_hash: H256,
    /// Previous PBFT block hash for the build command.
    pub previous_pbft_block_hash: H256,
    /// DAG anchor hash for the build command.
    pub anchor_hash: H256,
    /// Canonical order hash for the build command.
    pub order_hash: H256,
    /// FinalChain hash for the build command.
    pub final_chain_hash: H256,
    /// Wallet indices selected by Rust for proposal materialization.
    pub eligible_wallet_indices: Vec<u64>,
    /// Number of DAG blocks included before gas clipping for telemetry.
    pub dag_blocks_included: u64,
    /// True when Rust selected the null-anchor proposal rule.
    pub selected_null_anchor: bool,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: String,
}

/// Rust-owned PBFT proposal-construction cursor.
///
/// The session chooses proposer candidates and initial anchor immediately from
/// supplied facts. For non-null anchors it asks C++ for ordered DAG block gas
/// facts. If gas clipping selects a closer anchor, Rust requests that order and
/// only returns a build command after it can compute the final order hash.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerProposalSession {
    /// Initial proposal facts.
    pub fact: PbftManagerProposalInitialFact,
    eligible_wallet_indices: Vec<u64>,
    current_anchor: H256,
    requested_anchor: Option<H256>,
    build_step: Option<PbftManagerProposalSessionStep>,
    terminal_status: Option<PbftManagerProposalStatus>,
    error_code: String,
}

/// Broadcast action family selected by Rust for one `broadcastVotes()` tick.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBroadcastAction {
    /// No broadcast threshold has been reached.
    Noop,
    /// Broadcast reward, own PBFT, and own pillar votes.
    PeriodVotes,
    /// Broadcast period votes plus current/previous round 2t+1 bundles.
    RoundVotes,
    /// Unknown bridge action.
    Unknown,
}

impl PbftManagerBroadcastAction {
    /// Stable bridge code for the broadcast action.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Noop => 0,
            Self::PeriodVotes => 1,
            Self::RoundVotes => 2,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge action code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Noop,
            1 => Self::PeriodVotes,
            2 => Self::RoundVotes,
            _ => Self::Unknown,
        }
    }
}

/// Broadcast plan status selected by Rust.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBroadcastStatus {
    /// The plan is valid and ready for optional C++ execution.
    Ready,
    /// Supplied facts violate the broadcast planner contract.
    InvalidFact,
    /// C++ reported an executor failure.
    ExecutorFailed,
    /// C++ reported an unknown or mismatched action.
    InvalidReport,
}

impl PbftManagerBroadcastStatus {
    /// Stable bridge code for the broadcast status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidFact => 1,
            Self::ExecutorFailed => 2,
            Self::InvalidReport => 3,
        }
    }
}

/// Compact timing/counter facts for Rust-owned PBFT vote broadcast planning.
///
/// Inputs:
/// - elapsed times and lambda are supplied as milliseconds.
/// - counters are the current C++ compatibility mirrors.
/// - thresholds are passed from the manager constants so tests and future
///   configuration can validate the same planner without hardcoding globals.
///
/// Outputs are produced by `plan_pbft_manager_broadcast`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerBroadcastFact {
    /// Elapsed time since the current round started.
    pub round_elapsed_ms: u64,
    /// Elapsed time since the current period started.
    pub period_elapsed_ms: u64,
    /// Current round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Broadcast threshold multiplier.
    pub broadcast_lambda_threshold: u32,
    /// Rebroadcast threshold multiplier.
    pub rebroadcast_lambda_threshold: u32,
    /// Counter for normal round broadcasts.
    pub broadcast_votes_counter: u32,
    /// Counter for round rebroadcasts.
    pub rebroadcast_votes_counter: u32,
    /// Counter for normal period/reward broadcasts.
    pub broadcast_reward_votes_counter: u32,
    /// Counter for period/reward rebroadcasts.
    pub rebroadcast_reward_votes_counter: u32,
}

/// Rust-owned broadcast plan for one `broadcastVotes()` call.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerBroadcastPlan {
    /// Plan status.
    pub status: PbftManagerBroadcastStatus,
    /// Vote family C++ should broadcast.
    pub action: PbftManagerBroadcastAction,
    /// Whether C++ should use rebroadcast network send semantics.
    pub rebroadcast: bool,
    /// Counter value to apply after Rust accepts a successful executor report.
    pub next_broadcast_votes_counter: u32,
    /// Counter value to apply after Rust accepts a successful executor report.
    pub next_rebroadcast_votes_counter: u32,
    /// Counter value to apply after Rust accepts a successful executor report.
    pub next_broadcast_reward_votes_counter: u32,
    /// Counter value to apply after Rust accepts a successful executor report.
    pub next_rebroadcast_reward_votes_counter: u32,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: String,
}

/// C++ executor report for one Rust-planned vote broadcast.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerBroadcastReport {
    /// Action C++ attempted to execute.
    pub action: PbftManagerBroadcastAction,
    /// Whether C++ used rebroadcast network send semantics.
    pub rebroadcast: bool,
    /// Whether the network executor completed the requested action.
    pub success: bool,
    /// Optional executor diagnostic.
    pub error_code: String,
}

/// Result of a Rust-accepted or rejected broadcast executor report.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerBroadcastReportResult {
    /// Report validation status.
    pub status: PbftManagerBroadcastStatus,
    /// True when C++ may apply the returned counter mirrors.
    pub apply_counters: bool,
    /// Counter value to apply when `apply_counters` is true.
    pub broadcast_votes_counter: u32,
    /// Counter value to apply when `apply_counters` is true.
    pub rebroadcast_votes_counter: u32,
    /// Counter value to apply when `apply_counters` is true.
    pub broadcast_reward_votes_counter: u32,
    /// Counter value to apply when `apply_counters` is true.
    pub rebroadcast_reward_votes_counter: u32,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: String,
}

/// Computes the legacy PBFT proposer ranking hash for one vote index.
///
/// Inputs are the proposal vote VRF output, recovered voter public key, and
/// one-based vote-weight index. The output matches C++ `getVoterIndexHash`:
/// Keccak256 over RLP list `[credential, voter_public_key, index]`.
#[must_use]
pub fn pbft_manager_voter_index_hash(
    credential: [u8; 64],
    voter_public_key: [u8; 64],
    index: u64,
) -> H256 {
    let mut stream = RlpStream::new_list(3);
    stream.append(&credential.as_slice());
    stream.append(&voter_public_key.as_slice());
    stream.append(&index);
    keccak256(&stream.out())
}

/// Computes the legacy proposal rank for a weighted PBFT proposal vote.
///
/// The rank is the lowest voter-index hash across the vote's positive weight.
/// A zero weight has no valid rank and returns `None` so callers can surface an
/// explicit invalid fact instead of silently selecting the vote.
#[must_use]
pub fn pbft_manager_proposal_rank_hash(
    credential: [u8; 64],
    voter_public_key: [u8; 64],
    weight: u64,
) -> Option<H256> {
    if weight == 0 {
        return None;
    }

    let mut lowest_hash = pbft_manager_voter_index_hash(credential, voter_public_key, 1);
    for index in 2..=weight {
        let candidate = pbft_manager_voter_index_hash(credential, voter_public_key, index);
        if lowest_hash > candidate {
            lowest_hash = candidate;
        }
    }
    Some(lowest_hash)
}

/// Plans one proposed PBFT block admission attempt.
///
/// C++ supplies live sidecar lookup and validation facts in the order Rust
/// requests them. Rust decides whether the block is missing, needs validation,
/// should be returned immediately, should be marked valid, or must be rejected.
/// The planner does not materialize or mutate proposed blocks.
#[must_use]
pub fn plan_pbft_manager_candidate_admission(
    fact: PbftManagerCandidateAdmissionFact,
) -> PbftManagerCandidateAdmissionPlan {
    if fact.block_hash == H256::zero() {
        return pbft_manager_candidate_admission_contract_error(
            "PBFT_MANAGER_CANDIDATE_ADMISSION_ZERO_BLOCK_HASH",
        );
    }
    if !fact.lookup_performed {
        if fact.proposed_block_found
            || fact.proposed_block_already_valid
            || fact.validation_status != PbftManagerCandidateAdmissionValidationStatus::NotChecked
        {
            return pbft_manager_candidate_admission_contract_error(
                "PBFT_MANAGER_CANDIDATE_ADMISSION_PRELOOKUP_FACTS",
            );
        }
        return PbftManagerCandidateAdmissionPlan {
            action: PbftManagerCandidateAdmissionAction::RequestLookup,
            status: PbftManagerCandidateAdmissionStatus::LookupRequired,
            mark_valid: false,
            error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_LOOKUP_REQUIRED",
        };
    }

    if !fact.proposed_block_found {
        if fact.proposed_block_already_valid
            || fact.validation_status != PbftManagerCandidateAdmissionValidationStatus::NotChecked
        {
            return pbft_manager_candidate_admission_contract_error(
                "PBFT_MANAGER_CANDIDATE_ADMISSION_MISSING_BLOCK_FACTS",
            );
        }
        return PbftManagerCandidateAdmissionPlan {
            action: PbftManagerCandidateAdmissionAction::DeferMissingBlock,
            status: PbftManagerCandidateAdmissionStatus::BlockMissing,
            mark_valid: false,
            error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_BLOCK_MISSING",
        };
    }

    if fact.proposed_block_already_valid {
        if fact.validation_status != PbftManagerCandidateAdmissionValidationStatus::NotChecked {
            return pbft_manager_candidate_admission_contract_error(
                "PBFT_MANAGER_CANDIDATE_ADMISSION_ALREADY_VALID_WITH_REPORT",
            );
        }
        return PbftManagerCandidateAdmissionPlan {
            action: PbftManagerCandidateAdmissionAction::Accept,
            status: PbftManagerCandidateAdmissionStatus::AcceptedAlreadyValid,
            mark_valid: false,
            error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_ALREADY_VALID",
        };
    }

    match fact.validation_status {
        PbftManagerCandidateAdmissionValidationStatus::NotChecked => {
            PbftManagerCandidateAdmissionPlan {
                action: PbftManagerCandidateAdmissionAction::RequestValidation,
                status: PbftManagerCandidateAdmissionStatus::ValidationRequired,
                mark_valid: false,
                error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_VALIDATION_REQUIRED",
            }
        }
        PbftManagerCandidateAdmissionValidationStatus::Valid => PbftManagerCandidateAdmissionPlan {
            action: PbftManagerCandidateAdmissionAction::Accept,
            status: PbftManagerCandidateAdmissionStatus::AcceptedNewlyValidated,
            mark_valid: true,
            error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_VALIDATED",
        },
        PbftManagerCandidateAdmissionValidationStatus::Invalid => {
            PbftManagerCandidateAdmissionPlan {
                action: PbftManagerCandidateAdmissionAction::Reject,
                status: PbftManagerCandidateAdmissionStatus::ValidationRejected,
                mark_valid: false,
                error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_VALIDATION_REJECTED",
            }
        }
        PbftManagerCandidateAdmissionValidationStatus::Unknown => {
            pbft_manager_candidate_admission_contract_error(
                "PBFT_MANAGER_CANDIDATE_ADMISSION_UNKNOWN_VALIDATION_STATUS",
            )
        }
    }
}

/// Selects the PBFT leader candidate from C++-collected proposal facts.
///
/// C++ supplies live-object and validation status facts; Rust owns the
/// deterministic rank ordering, duplicate-rank overwrite rule, in-chain/invalid
/// skipping, and null-anchor fallback. The function does not materialize or
/// mutate blocks or votes.
#[must_use]
pub fn plan_pbft_manager_leader_selection(
    candidates: Vec<PbftManagerLeaderCandidateFact>,
) -> PbftManagerLeaderSelectionPlan {
    if candidates.is_empty() {
        return pbft_manager_leader_no_selection(
            PbftManagerLeaderSelectionStatus::Empty,
            "PBFT_MANAGER_LEADER_EMPTY",
        );
    }

    let mut ranked_candidates = BTreeMap::<H256, PbftManagerLeaderCandidateFact>::new();
    for candidate in candidates {
        if candidate.status == PbftManagerLeaderCandidateStatus::Unknown {
            return pbft_manager_leader_no_selection(
                PbftManagerLeaderSelectionStatus::InvalidFact,
                "PBFT_MANAGER_LEADER_UNKNOWN_CANDIDATE_STATUS",
            );
        }
        if candidate.weight == 0 {
            ranked_candidates.insert(
                candidate.vote_hash,
                PbftManagerLeaderCandidateFact {
                    status: PbftManagerLeaderCandidateStatus::InvalidVoteWeight,
                    ..candidate
                },
            );
            continue;
        }
        let Some(rank) = pbft_manager_proposal_rank_hash(
            candidate.credential,
            candidate.voter_public_key,
            candidate.weight,
        ) else {
            return pbft_manager_leader_no_selection(
                PbftManagerLeaderSelectionStatus::InvalidFact,
                "PBFT_MANAGER_LEADER_INVALID_WEIGHT",
            );
        };
        ranked_candidates.insert(rank, candidate);
    }

    let mut null_anchor_fallback = None;
    for candidate in ranked_candidates.into_values() {
        if candidate.status != PbftManagerLeaderCandidateStatus::Ready {
            continue;
        }
        let from_null_anchor = candidate.pivot_hash == H256::zero();
        if from_null_anchor {
            if null_anchor_fallback.is_none() {
                null_anchor_fallback = Some(candidate);
            }
            continue;
        }
        return pbft_manager_leader_selected(candidate, false);
    }

    if let Some(candidate) = null_anchor_fallback {
        return pbft_manager_leader_selected(candidate, true);
    }

    pbft_manager_leader_no_selection(
        PbftManagerLeaderSelectionStatus::NoEligibleCandidate,
        "PBFT_MANAGER_LEADER_NO_ELIGIBLE_CANDIDATE",
    )
}

/// Derives PBFT proposal candidate statuses and selects the leader.
///
/// C++ supplies compact live lookup and validation facts for every proposal
/// vote. Rust derives candidate status in the legacy order, emits mark-valid
/// commands for accepted but previously unmarked blocks, and then applies the
/// Rust-owned leader ranking/null-anchor fallback rules.
#[must_use]
pub fn plan_pbft_manager_leader_candidates(
    candidates: Vec<PbftManagerLeaderCandidateInputFact>,
) -> PbftManagerLeaderCandidatePlan {
    let mut valid_blocks = Vec::new();
    let mut selection_candidates = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let mut status = PbftManagerLeaderCandidateStatus::Ready;
        let weight = if !candidate.weight_found || candidate.weight == 0 {
            status = PbftManagerLeaderCandidateStatus::InvalidVoteWeight;
            0
        } else {
            candidate.weight
        };

        if status == PbftManagerLeaderCandidateStatus::Ready {
            if candidate.block_hash == H256::zero() {
                status = PbftManagerLeaderCandidateStatus::NullVoteBlockHash;
            } else if candidate.block_in_chain {
                status = PbftManagerLeaderCandidateStatus::BlockInChain;
            } else if !candidate.proposed_block_found {
                status = PbftManagerLeaderCandidateStatus::BlockMissingOrInvalid;
            } else {
                match candidate.block_validation_status {
                    PbftManagerLeaderBlockValidationStatus::AlreadyValid => {}
                    PbftManagerLeaderBlockValidationStatus::Validated => {
                        valid_blocks.push(PbftManagerLeaderValidBlockCommand {
                            period: candidate.period,
                            block_hash: candidate.block_hash,
                        });
                    }
                    PbftManagerLeaderBlockValidationStatus::Rejected => {
                        status = PbftManagerLeaderCandidateStatus::BlockMissingOrInvalid;
                    }
                    PbftManagerLeaderBlockValidationStatus::Unknown => {
                        return pbft_manager_candidate_plan_from_selection(
                            pbft_manager_leader_no_selection(
                                PbftManagerLeaderSelectionStatus::InvalidFact,
                                "PBFT_MANAGER_LEADER_UNKNOWN_BLOCK_VALIDATION_STATUS",
                            ),
                            valid_blocks,
                        );
                    }
                }
            }
        }

        selection_candidates.push(PbftManagerLeaderCandidateFact {
            vote_hash: candidate.vote_hash,
            block_hash: candidate.block_hash,
            period: candidate.period,
            credential: candidate.credential,
            voter_public_key: candidate.voter_public_key,
            weight,
            status,
            pivot_hash: candidate.pivot_hash,
        });
    }

    let selection = plan_pbft_manager_leader_selection(selection_candidates);
    pbft_manager_candidate_plan_from_selection(selection, valid_blocks)
}

/// Plans the next step of PBFT block validation.
///
/// The planner is a side-effect-free state machine: C++ supplies the latest
/// validation fact bundle, Rust requests the next live check, and C++ reports
/// that result back into the next call. The accepted/rejected outcome is
/// therefore Rust-owned even while live PBFT chain, FinalChain, reward-vote,
/// pillar, and DAG objects remain outside Rust.
#[must_use]
pub fn plan_pbft_manager_block_validation(
    fact: PbftManagerBlockValidationFact,
) -> PbftManagerBlockValidationPlan {
    if fact.pbft_chain_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.final_chain_hash_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.reward_votes_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.extra_data_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.pillar_block_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.dag_order_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.dag_weight_status == PbftManagerBlockValidationFactStatus::Unknown
    {
        return pbft_manager_block_validation_contract_error(
            "PBFT_MANAGER_BLOCK_VALIDATION_UNKNOWN_FACT_STATUS",
        );
    }

    match fact.pbft_chain_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::CheckPbftChain,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Invalid
        | PbftManagerBlockValidationFactStatus::Missing => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::PbftChainInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_PBFT_CHAIN_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_PBFT_CHAIN_STATUS_INVALID",
            );
        }
    }

    match fact.final_chain_hash_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::ValidateFinalChainHash,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Missing => {
            return PbftManagerBlockValidationPlan {
                action: PbftManagerBlockValidationAction::WaitForFinalization,
                status: PbftManagerBlockValidationStatus::FinalChainHashMissing,
                next_check: PbftManagerBlockValidationNextCheck::ValidateFinalChainHash,
                error_code: "PBFT_MANAGER_BLOCK_VALIDATION_FINAL_CHAIN_HASH_MISSING",
            };
        }
        PbftManagerBlockValidationFactStatus::Invalid => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::FinalChainHashInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_FINAL_CHAIN_HASH_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_FINAL_CHAIN_STATUS_INVALID",
            );
        }
    }

    match fact.reward_votes_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::CheckRewardVotes,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Invalid
        | PbftManagerBlockValidationFactStatus::Missing => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::RewardVotesInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_REWARD_VOTES_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_REWARD_VOTES_STATUS_INVALID",
            );
        }
    }

    match fact.extra_data_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::ValidateExtraData,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Invalid
        | PbftManagerBlockValidationFactStatus::Missing => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::ExtraDataInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_EXTRA_DATA_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_EXTRA_DATA_STATUS_INVALID",
            );
        }
    }

    if fact.pillar_block_required {
        match fact.pillar_block_status {
            PbftManagerBlockValidationFactStatus::NotChecked => {
                return pbft_manager_block_validation_run_check(
                    PbftManagerBlockValidationNextCheck::ValidatePillarBlock,
                );
            }
            PbftManagerBlockValidationFactStatus::Valid => {}
            PbftManagerBlockValidationFactStatus::Invalid
            | PbftManagerBlockValidationFactStatus::Missing => {
                return pbft_manager_block_validation_reject(
                    PbftManagerBlockValidationStatus::PillarBlockInvalid,
                    "PBFT_MANAGER_BLOCK_VALIDATION_PILLAR_BLOCK_INVALID",
                );
            }
            PbftManagerBlockValidationFactStatus::NotRequired
            | PbftManagerBlockValidationFactStatus::Unknown => {
                return pbft_manager_block_validation_contract_error(
                    "PBFT_MANAGER_BLOCK_VALIDATION_PILLAR_BLOCK_STATUS_INVALID",
                );
            }
        }
    } else if fact.pillar_block_status == PbftManagerBlockValidationFactStatus::NotChecked {
        // Normalize not-required checks so the C++ executor does not need to
        // report unused facts for non-pillar periods.
    } else if fact.pillar_block_status != PbftManagerBlockValidationFactStatus::NotRequired {
        return pbft_manager_block_validation_contract_error(
            "PBFT_MANAGER_BLOCK_VALIDATION_UNEXPECTED_PILLAR_BLOCK_STATUS",
        );
    }

    if fact.pivot_is_null || fact.dag_order_cached || !fact.dag_order_required {
        return pbft_manager_block_validation_accept();
    }

    match fact.dag_order_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::CheckDagOrder,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Missing => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::DagOrderMissing,
                "PBFT_MANAGER_BLOCK_VALIDATION_DAG_ORDER_MISSING",
            );
        }
        PbftManagerBlockValidationFactStatus::Invalid => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::DagOrderInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_DAG_ORDER_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_DAG_ORDER_STATUS_INVALID",
            );
        }
    }

    if !fact.dag_weight_check_required {
        return pbft_manager_block_validation_accept();
    }

    match fact.dag_weight_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::CheckDagWeight,
            )
        }
        PbftManagerBlockValidationFactStatus::Valid => pbft_manager_block_validation_accept(),
        PbftManagerBlockValidationFactStatus::Invalid
        | PbftManagerBlockValidationFactStatus::Missing => pbft_manager_block_validation_reject(
            PbftManagerBlockValidationStatus::DagWeightInvalid,
            "PBFT_MANAGER_BLOCK_VALIDATION_DAG_WEIGHT_INVALID",
        ),
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_DAG_WEIGHT_STATUS_INVALID",
            )
        }
    }
}

/// Creates a Rust-owned PBFT block-validation session from initial facts.
#[must_use]
pub fn create_pbft_manager_block_validation_session(
    fact: PbftManagerBlockValidationFact,
) -> PbftManagerBlockValidationSession {
    PbftManagerBlockValidationSession {
        fact,
        pending_check: None,
    }
}

/// Returns the next plan for a Rust-owned PBFT block-validation session.
#[must_use]
pub fn next_pbft_manager_block_validation_session(
    session: &mut PbftManagerBlockValidationSession,
) -> PbftManagerBlockValidationPlan {
    let plan = plan_pbft_manager_block_validation(session.fact.clone());
    session.pending_check = match plan.action {
        PbftManagerBlockValidationAction::RunCheck
        | PbftManagerBlockValidationAction::WaitForFinalization => Some(plan.next_check),
        _ => None,
    };
    plan
}

/// Applies one live-check report and returns the next PBFT block-validation plan.
#[must_use]
pub fn report_pbft_manager_block_validation_session_check(
    session: &mut PbftManagerBlockValidationSession,
    status: PbftManagerBlockValidationFactStatus,
    dag_weight_check_required: bool,
) -> PbftManagerBlockValidationPlan {
    let Some(pending_check) = session.pending_check else {
        return pbft_manager_block_validation_contract_error(
            "PBFT_MANAGER_BLOCK_VALIDATION_SESSION_NO_PENDING_CHECK",
        );
    };

    if status == PbftManagerBlockValidationFactStatus::Unknown {
        return pbft_manager_block_validation_contract_error(
            "PBFT_MANAGER_BLOCK_VALIDATION_SESSION_UNKNOWN_STATUS",
        );
    }
    if status == PbftManagerBlockValidationFactStatus::NotRequired {
        return pbft_manager_block_validation_contract_error(
            "PBFT_MANAGER_BLOCK_VALIDATION_SESSION_NOT_REQUIRED_REPORT",
        );
    }
    if status == PbftManagerBlockValidationFactStatus::NotChecked
        && pending_check != PbftManagerBlockValidationNextCheck::ValidateFinalChainHash
    {
        return pbft_manager_block_validation_contract_error(
            "PBFT_MANAGER_BLOCK_VALIDATION_SESSION_INVALID_RETRY_RESET",
        );
    }

    match pending_check {
        PbftManagerBlockValidationNextCheck::CheckPbftChain => {
            session.fact.pbft_chain_status = status;
        }
        PbftManagerBlockValidationNextCheck::ValidateFinalChainHash => {
            session.fact.final_chain_hash_status = status;
        }
        PbftManagerBlockValidationNextCheck::CheckRewardVotes => {
            session.fact.reward_votes_status = status;
        }
        PbftManagerBlockValidationNextCheck::ValidateExtraData => {
            session.fact.extra_data_status = status;
        }
        PbftManagerBlockValidationNextCheck::ValidatePillarBlock => {
            session.fact.pillar_block_status = status;
        }
        PbftManagerBlockValidationNextCheck::CheckDagOrder => {
            session.fact.dag_order_status = status;
            if status == PbftManagerBlockValidationFactStatus::Valid {
                session.fact.dag_weight_check_required = dag_weight_check_required;
            }
        }
        PbftManagerBlockValidationNextCheck::CheckDagWeight => {
            session.fact.dag_weight_status = status;
        }
        PbftManagerBlockValidationNextCheck::None
        | PbftManagerBlockValidationNextCheck::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_SESSION_INVALID_PENDING_CHECK",
            );
        }
    }

    session.pending_check = None;
    next_pbft_manager_block_validation_session(session)
}

fn pbft_manager_order_hash(dag_blocks: &[PbftManagerProposalDagBlockFact]) -> H256 {
    if dag_blocks.is_empty() {
        return H256::zero();
    }

    let mut stream = RlpStream::new_list(1);
    stream.begin_list(dag_blocks.len());
    for block in dag_blocks {
        let hash_bytes: &[u8] = block.hash.as_bytes();
        stream.append(&hash_bytes);
    }
    keccak256(&stream.out())
}

fn pbft_manager_proposal_contract_error(
    error_code: impl Into<String>,
) -> PbftManagerProposalSessionStep {
    PbftManagerProposalSessionStep {
        action: PbftManagerProposalAction::ContractError,
        status: PbftManagerProposalStatus::InvalidBridgeFacts,
        requested_anchor_hash: H256::zero(),
        previous_pbft_block_hash: H256::zero(),
        anchor_hash: H256::zero(),
        order_hash: H256::zero(),
        final_chain_hash: H256::zero(),
        eligible_wallet_indices: Vec::new(),
        dag_blocks_included: 0,
        selected_null_anchor: false,
        error_code: error_code.into(),
    }
}

fn pbft_manager_proposal_skip(
    fact: &PbftManagerProposalInitialFact,
    status: PbftManagerProposalStatus,
    error_code: impl Into<String>,
) -> PbftManagerProposalSessionStep {
    PbftManagerProposalSessionStep {
        action: PbftManagerProposalAction::SkipProposal,
        status,
        requested_anchor_hash: H256::zero(),
        previous_pbft_block_hash: fact.previous_pbft_block_hash,
        anchor_hash: H256::zero(),
        order_hash: H256::zero(),
        final_chain_hash: fact.final_chain_hash,
        eligible_wallet_indices: Vec::new(),
        dag_blocks_included: 0,
        selected_null_anchor: false,
        error_code: error_code.into(),
    }
}

fn pbft_manager_proposal_build(
    fact: &PbftManagerProposalInitialFact,
    anchor_hash: H256,
    order_hash: H256,
    eligible_wallet_indices: Vec<u64>,
    dag_blocks_included: u64,
    error_code: impl Into<String>,
) -> PbftManagerProposalSessionStep {
    PbftManagerProposalSessionStep {
        action: PbftManagerProposalAction::BuildProposal,
        status: PbftManagerProposalStatus::BuildReady,
        requested_anchor_hash: H256::zero(),
        previous_pbft_block_hash: fact.previous_pbft_block_hash,
        anchor_hash,
        order_hash,
        final_chain_hash: fact.final_chain_hash,
        eligible_wallet_indices,
        dag_blocks_included,
        selected_null_anchor: anchor_hash == H256::zero(),
        error_code: error_code.into(),
    }
}

fn pbft_manager_proposal_request_order(anchor_hash: H256) -> PbftManagerProposalSessionStep {
    PbftManagerProposalSessionStep {
        action: PbftManagerProposalAction::RequestDagOrder,
        status: PbftManagerProposalStatus::Active,
        requested_anchor_hash: anchor_hash,
        previous_pbft_block_hash: H256::zero(),
        anchor_hash: H256::zero(),
        order_hash: H256::zero(),
        final_chain_hash: H256::zero(),
        eligible_wallet_indices: Vec::new(),
        dag_blocks_included: 0,
        selected_null_anchor: false,
        error_code: String::new(),
    }
}

fn pbft_manager_proposal_initial_anchor(fact: &PbftManagerProposalInitialFact) -> H256 {
    if fact.ghost_path.is_empty() {
        return H256::zero();
    }

    let mut dag_block_hash = if fact.ghost_path.len() as u64 <= fact.dag_blocks_size {
        let move_back = fact.ghost_path_move_back.saturating_add(1);
        let mut ghost_index = if fact.ghost_path.len() as u64 >= move_back {
            fact.ghost_path.len() - move_back as usize
        } else {
            0
        };
        while ghost_index < fact.ghost_path.len() - 1
            && fact.ghost_path[ghost_index] == fact.last_period_dag_anchor_hash
        {
            ghost_index += 1;
        }
        fact.ghost_path[ghost_index]
    } else {
        fact.ghost_path[(fact.dag_blocks_size - 1) as usize]
    };

    if dag_block_hash == fact.dag_genesis_hash {
        return H256::zero();
    }

    if dag_block_hash == fact.last_period_dag_anchor_hash {
        if fact.has_non_finalized_fallback {
            dag_block_hash = fact.non_finalized_fallback_hash;
        } else {
            return H256::zero();
        }
    }

    dag_block_hash
}

fn pbft_manager_proposal_closest_anchor(
    ghost_path: &[H256],
    dag_blocks: &[PbftManagerProposalDagBlockFact],
    included: usize,
) -> Option<H256> {
    for block in dag_blocks.iter().take(included).rev() {
        if ghost_path.contains(&block.hash) {
            return Some(block.hash);
        }
    }
    ghost_path.get(1).copied()
}

fn pbft_manager_proposal_clip(
    dag_blocks: &[PbftManagerProposalDagBlockFact],
    pbft_gas_limit: u64,
) -> usize {
    let mut total_weight = 0_u64;
    let mut included = 0_usize;
    for block in dag_blocks {
        let Some(next_weight) = total_weight.checked_add(block.gas_estimation) else {
            break;
        };
        if next_weight > pbft_gas_limit {
            break;
        }
        total_weight = next_weight;
        included += 1;
    }
    included
}

/// Creates a Rust-owned PBFT proposal-construction session.
#[must_use]
pub fn create_pbft_manager_proposal_session(
    fact: PbftManagerProposalInitialFact,
) -> PbftManagerProposalSession {
    let eligible_wallet_indices = fact
        .wallets
        .iter()
        .filter(|wallet| wallet.dpos_eligible && wallet.sortition_valid)
        .map(|wallet| wallet.wallet_index)
        .collect::<Vec<_>>();
    let current_anchor = pbft_manager_proposal_initial_anchor(&fact);

    PbftManagerProposalSession {
        fact,
        eligible_wallet_indices,
        current_anchor,
        requested_anchor: None,
        build_step: None,
        terminal_status: None,
        error_code: String::new(),
    }
}

/// Returns the next action for a Rust-owned proposal-construction session.
#[must_use]
pub fn next_pbft_manager_proposal_session(
    session: &mut PbftManagerProposalSession,
) -> PbftManagerProposalSessionStep {
    if let Some(step) = &session.build_step {
        return step.clone();
    }

    if let Some(status) = session.terminal_status {
        return match status {
            PbftManagerProposalStatus::NoEligibleWallet
            | PbftManagerProposalStatus::MissingFinalChainHash
            | PbftManagerProposalStatus::MissingExtraData
            | PbftManagerProposalStatus::MissingDagOrder => {
                pbft_manager_proposal_skip(&session.fact, status, session.error_code.clone())
            }
            PbftManagerProposalStatus::InvalidBridgeFacts => {
                pbft_manager_proposal_contract_error(session.error_code.clone())
            }
            PbftManagerProposalStatus::Active | PbftManagerProposalStatus::BuildReady => {
                pbft_manager_proposal_contract_error(
                    "PBFT_MANAGER_PROPOSAL_INVALID_TERMINAL_STATUS",
                )
            }
        };
    }

    if session.fact.period == 0 || session.fact.round == 0 {
        session.terminal_status = Some(PbftManagerProposalStatus::InvalidBridgeFacts);
        session.error_code = "PBFT_MANAGER_PROPOSAL_INVALID_PERIOD_OR_ROUND".to_string();
        return pbft_manager_proposal_contract_error(session.error_code.clone());
    }
    if session.fact.dag_blocks_size == 0 {
        session.terminal_status = Some(PbftManagerProposalStatus::InvalidBridgeFacts);
        session.error_code = "PBFT_MANAGER_PROPOSAL_ZERO_DAG_BLOCKS_SIZE".to_string();
        return pbft_manager_proposal_contract_error(session.error_code.clone());
    }
    if session.eligible_wallet_indices.is_empty() {
        session.terminal_status = Some(PbftManagerProposalStatus::NoEligibleWallet);
        session.error_code = "PBFT_MANAGER_PROPOSAL_NO_ELIGIBLE_WALLET".to_string();
        return pbft_manager_proposal_skip(
            &session.fact,
            PbftManagerProposalStatus::NoEligibleWallet,
            session.error_code.clone(),
        );
    }
    if session.fact.extra_data_required && !session.fact.extra_data_available {
        session.terminal_status = Some(PbftManagerProposalStatus::MissingExtraData);
        session.error_code = "PBFT_MANAGER_PROPOSAL_MISSING_EXTRA_DATA".to_string();
        return pbft_manager_proposal_skip(
            &session.fact,
            PbftManagerProposalStatus::MissingExtraData,
            session.error_code.clone(),
        );
    }
    if !session.fact.final_chain_hash_valid {
        session.terminal_status = Some(PbftManagerProposalStatus::MissingFinalChainHash);
        session.error_code = "PBFT_MANAGER_PROPOSAL_MISSING_FINAL_CHAIN_HASH".to_string();
        return pbft_manager_proposal_skip(
            &session.fact,
            PbftManagerProposalStatus::MissingFinalChainHash,
            session.error_code.clone(),
        );
    }

    if session.current_anchor == H256::zero() {
        let step = pbft_manager_proposal_build(
            &session.fact,
            H256::zero(),
            H256::zero(),
            session.eligible_wallet_indices.clone(),
            0,
            "PBFT_MANAGER_PROPOSAL_NULL_ANCHOR",
        );
        session.build_step = Some(step.clone());
        return step;
    }

    session.requested_anchor = Some(session.current_anchor);
    pbft_manager_proposal_request_order(session.current_anchor)
}

/// Reports one DAG-order response and returns the next proposal step.
#[must_use]
pub fn report_pbft_manager_proposal_dag_order(
    session: &mut PbftManagerProposalSession,
    report: PbftManagerProposalDagOrderReport,
) -> PbftManagerProposalSessionStep {
    let Some(requested_anchor) = session.requested_anchor else {
        session.terminal_status = Some(PbftManagerProposalStatus::InvalidBridgeFacts);
        session.error_code = "PBFT_MANAGER_PROPOSAL_NO_PENDING_DAG_ORDER".to_string();
        return pbft_manager_proposal_contract_error(session.error_code.clone());
    };
    if report.anchor_hash != requested_anchor {
        session.terminal_status = Some(PbftManagerProposalStatus::InvalidBridgeFacts);
        session.error_code = "PBFT_MANAGER_PROPOSAL_DAG_ORDER_ANCHOR_MISMATCH".to_string();
        return pbft_manager_proposal_contract_error(session.error_code.clone());
    }
    if !report.order_available || report.dag_blocks.is_empty() {
        session.terminal_status = Some(PbftManagerProposalStatus::MissingDagOrder);
        session.error_code = "PBFT_MANAGER_PROPOSAL_MISSING_DAG_ORDER".to_string();
        return pbft_manager_proposal_skip(
            &session.fact,
            PbftManagerProposalStatus::MissingDagOrder,
            session.error_code.clone(),
        );
    }

    let included = pbft_manager_proposal_clip(&report.dag_blocks, session.fact.pbft_gas_limit);
    if included != report.dag_blocks.len() {
        let Some(closest_anchor) = pbft_manager_proposal_closest_anchor(
            &session.fact.ghost_path,
            &report.dag_blocks,
            included,
        ) else {
            session.terminal_status = Some(PbftManagerProposalStatus::InvalidBridgeFacts);
            session.error_code = "PBFT_MANAGER_PROPOSAL_CLOSEST_ANCHOR_MISSING".to_string();
            return pbft_manager_proposal_contract_error(session.error_code.clone());
        };
        if closest_anchor != requested_anchor {
            session.current_anchor = closest_anchor;
            session.requested_anchor = Some(closest_anchor);
            return pbft_manager_proposal_request_order(closest_anchor);
        }
    }

    session.requested_anchor = None;
    let step = pbft_manager_proposal_build(
        &session.fact,
        requested_anchor,
        pbft_manager_order_hash(&report.dag_blocks),
        session.eligible_wallet_indices.clone(),
        included as u64,
        "PBFT_MANAGER_PROPOSAL_READY",
    );
    session.build_step = Some(step.clone());
    step
}

/// Aborts a proposal session with a stable contract-error status.
#[must_use]
pub fn abort_pbft_manager_proposal_session(
    session: &mut PbftManagerProposalSession,
) -> PbftManagerProposalSessionStep {
    session.terminal_status = Some(PbftManagerProposalStatus::InvalidBridgeFacts);
    session.error_code = "PBFT_MANAGER_PROPOSAL_SESSION_ABORTED".to_string();
    pbft_manager_proposal_contract_error(session.error_code.clone())
}

fn pbft_manager_broadcast_invalid(
    fact: PbftManagerBroadcastFact,
    error_code: impl Into<String>,
) -> PbftManagerBroadcastPlan {
    PbftManagerBroadcastPlan {
        status: PbftManagerBroadcastStatus::InvalidFact,
        action: PbftManagerBroadcastAction::Noop,
        rebroadcast: false,
        next_broadcast_votes_counter: fact.broadcast_votes_counter,
        next_rebroadcast_votes_counter: fact.rebroadcast_votes_counter,
        next_broadcast_reward_votes_counter: fact.broadcast_reward_votes_counter,
        next_rebroadcast_reward_votes_counter: fact.rebroadcast_reward_votes_counter,
        error_code: error_code.into(),
    }
}

fn pbft_manager_broadcast_ready(
    action: PbftManagerBroadcastAction,
    rebroadcast: bool,
    next_broadcast_votes_counter: u32,
    next_rebroadcast_votes_counter: u32,
    next_broadcast_reward_votes_counter: u32,
    next_rebroadcast_reward_votes_counter: u32,
) -> PbftManagerBroadcastPlan {
    PbftManagerBroadcastPlan {
        status: PbftManagerBroadcastStatus::Ready,
        action,
        rebroadcast,
        next_broadcast_votes_counter,
        next_rebroadcast_votes_counter,
        next_broadcast_reward_votes_counter,
        next_rebroadcast_reward_votes_counter,
        error_code: if action == PbftManagerBroadcastAction::Noop {
            "PBFT_MANAGER_BROADCAST_NOOP".to_string()
        } else {
            String::new()
        },
    }
}

fn ratio_threshold_exceeded(elapsed_ms: u64, lambda_ms: u64, threshold: u32, counter: u32) -> bool {
    elapsed_ms / lambda_ms > u64::from(threshold).saturating_mul(u64::from(counter))
}

fn pbft_manager_counter_increment(value: u32) -> Option<u32> {
    value.checked_add(1)
}

/// Plans one Rust-owned PBFT vote broadcast decision.
///
/// Rust owns the threshold comparisons, branch priority, rebroadcast flag, and
/// post-success counter values. C++ remains the executor for resolving retained
/// vote payloads/sidecars and calling network gossip APIs.
#[must_use]
pub fn plan_pbft_manager_broadcast(fact: PbftManagerBroadcastFact) -> PbftManagerBroadcastPlan {
    if fact.current_round_lambda_ms == 0 {
        return pbft_manager_broadcast_invalid(fact, "PBFT_MANAGER_BROADCAST_ZERO_LAMBDA");
    }
    if fact.broadcast_lambda_threshold == 0 || fact.rebroadcast_lambda_threshold == 0 {
        return pbft_manager_broadcast_invalid(fact, "PBFT_MANAGER_BROADCAST_ZERO_THRESHOLD");
    }
    if fact.broadcast_votes_counter == 0
        || fact.rebroadcast_votes_counter == 0
        || fact.broadcast_reward_votes_counter == 0
        || fact.rebroadcast_reward_votes_counter == 0
    {
        return pbft_manager_broadcast_invalid(fact, "PBFT_MANAGER_BROADCAST_ZERO_COUNTER");
    }

    if ratio_threshold_exceeded(
        fact.round_elapsed_ms,
        fact.current_round_lambda_ms,
        fact.rebroadcast_lambda_threshold,
        fact.rebroadcast_votes_counter,
    ) {
        let Some(next_broadcast_votes_counter) =
            pbft_manager_counter_increment(fact.broadcast_votes_counter)
        else {
            return pbft_manager_broadcast_invalid(fact, "PBFT_MANAGER_BROADCAST_COUNTER_OVERFLOW");
        };
        let Some(next_rebroadcast_votes_counter) =
            pbft_manager_counter_increment(fact.rebroadcast_votes_counter)
        else {
            return pbft_manager_broadcast_invalid(fact, "PBFT_MANAGER_BROADCAST_COUNTER_OVERFLOW");
        };
        return pbft_manager_broadcast_ready(
            PbftManagerBroadcastAction::RoundVotes,
            true,
            next_broadcast_votes_counter,
            next_rebroadcast_votes_counter,
            fact.broadcast_reward_votes_counter,
            fact.rebroadcast_reward_votes_counter,
        );
    }

    if ratio_threshold_exceeded(
        fact.round_elapsed_ms,
        fact.current_round_lambda_ms,
        fact.broadcast_lambda_threshold,
        fact.broadcast_votes_counter,
    ) {
        let Some(next_broadcast_votes_counter) =
            pbft_manager_counter_increment(fact.broadcast_votes_counter)
        else {
            return pbft_manager_broadcast_invalid(fact, "PBFT_MANAGER_BROADCAST_COUNTER_OVERFLOW");
        };
        return pbft_manager_broadcast_ready(
            PbftManagerBroadcastAction::RoundVotes,
            false,
            next_broadcast_votes_counter,
            fact.rebroadcast_votes_counter,
            fact.broadcast_reward_votes_counter,
            fact.rebroadcast_reward_votes_counter,
        );
    }

    if ratio_threshold_exceeded(
        fact.period_elapsed_ms,
        fact.current_round_lambda_ms,
        fact.rebroadcast_lambda_threshold,
        fact.rebroadcast_reward_votes_counter,
    ) {
        let Some(next_broadcast_reward_votes_counter) =
            pbft_manager_counter_increment(fact.broadcast_reward_votes_counter)
        else {
            return pbft_manager_broadcast_invalid(fact, "PBFT_MANAGER_BROADCAST_COUNTER_OVERFLOW");
        };
        let Some(next_rebroadcast_reward_votes_counter) =
            pbft_manager_counter_increment(fact.rebroadcast_reward_votes_counter)
        else {
            return pbft_manager_broadcast_invalid(fact, "PBFT_MANAGER_BROADCAST_COUNTER_OVERFLOW");
        };
        return pbft_manager_broadcast_ready(
            PbftManagerBroadcastAction::PeriodVotes,
            true,
            fact.broadcast_votes_counter,
            fact.rebroadcast_votes_counter,
            next_broadcast_reward_votes_counter,
            next_rebroadcast_reward_votes_counter,
        );
    }

    if ratio_threshold_exceeded(
        fact.period_elapsed_ms,
        fact.current_round_lambda_ms,
        fact.broadcast_lambda_threshold,
        fact.broadcast_reward_votes_counter,
    ) {
        let Some(next_broadcast_reward_votes_counter) =
            pbft_manager_counter_increment(fact.broadcast_reward_votes_counter)
        else {
            return pbft_manager_broadcast_invalid(fact, "PBFT_MANAGER_BROADCAST_COUNTER_OVERFLOW");
        };
        return pbft_manager_broadcast_ready(
            PbftManagerBroadcastAction::PeriodVotes,
            false,
            fact.broadcast_votes_counter,
            fact.rebroadcast_votes_counter,
            next_broadcast_reward_votes_counter,
            fact.rebroadcast_reward_votes_counter,
        );
    }

    pbft_manager_broadcast_ready(
        PbftManagerBroadcastAction::Noop,
        false,
        fact.broadcast_votes_counter,
        fact.rebroadcast_votes_counter,
        fact.broadcast_reward_votes_counter,
        fact.rebroadcast_reward_votes_counter,
    )
}

/// Validates a C++ executor report before counter mirrors are updated.
#[must_use]
pub fn report_pbft_manager_broadcast(
    plan: PbftManagerBroadcastPlan,
    report: PbftManagerBroadcastReport,
) -> PbftManagerBroadcastReportResult {
    if plan.status != PbftManagerBroadcastStatus::Ready {
        return PbftManagerBroadcastReportResult {
            status: plan.status,
            apply_counters: false,
            broadcast_votes_counter: plan.next_broadcast_votes_counter,
            rebroadcast_votes_counter: plan.next_rebroadcast_votes_counter,
            broadcast_reward_votes_counter: plan.next_broadcast_reward_votes_counter,
            rebroadcast_reward_votes_counter: plan.next_rebroadcast_reward_votes_counter,
            error_code: plan.error_code,
        };
    }

    if report.action == PbftManagerBroadcastAction::Unknown
        || report.action != plan.action
        || report.rebroadcast != plan.rebroadcast
    {
        return PbftManagerBroadcastReportResult {
            status: PbftManagerBroadcastStatus::InvalidReport,
            apply_counters: false,
            broadcast_votes_counter: plan.next_broadcast_votes_counter,
            rebroadcast_votes_counter: plan.next_rebroadcast_votes_counter,
            broadcast_reward_votes_counter: plan.next_broadcast_reward_votes_counter,
            rebroadcast_reward_votes_counter: plan.next_rebroadcast_reward_votes_counter,
            error_code: "PBFT_MANAGER_BROADCAST_REPORT_ACTION_MISMATCH".to_string(),
        };
    }

    if !report.success {
        return PbftManagerBroadcastReportResult {
            status: PbftManagerBroadcastStatus::ExecutorFailed,
            apply_counters: false,
            broadcast_votes_counter: plan.next_broadcast_votes_counter,
            rebroadcast_votes_counter: plan.next_rebroadcast_votes_counter,
            broadcast_reward_votes_counter: plan.next_broadcast_reward_votes_counter,
            rebroadcast_reward_votes_counter: plan.next_rebroadcast_reward_votes_counter,
            error_code: if report.error_code.is_empty() {
                "PBFT_MANAGER_BROADCAST_EXECUTOR_FAILED".to_string()
            } else {
                report.error_code
            },
        };
    }

    PbftManagerBroadcastReportResult {
        status: PbftManagerBroadcastStatus::Ready,
        apply_counters: plan.action != PbftManagerBroadcastAction::Noop,
        broadcast_votes_counter: plan.next_broadcast_votes_counter,
        rebroadcast_votes_counter: plan.next_rebroadcast_votes_counter,
        broadcast_reward_votes_counter: plan.next_broadcast_reward_votes_counter,
        rebroadcast_reward_votes_counter: plan.next_rebroadcast_reward_votes_counter,
        error_code: String::new(),
    }
}

fn pbft_manager_block_validation_run_check(
    next_check: PbftManagerBlockValidationNextCheck,
) -> PbftManagerBlockValidationPlan {
    PbftManagerBlockValidationPlan {
        action: PbftManagerBlockValidationAction::RunCheck,
        status: PbftManagerBlockValidationStatus::Pending,
        next_check,
        error_code: "",
    }
}

fn pbft_manager_block_validation_accept() -> PbftManagerBlockValidationPlan {
    PbftManagerBlockValidationPlan {
        action: PbftManagerBlockValidationAction::Accept,
        status: PbftManagerBlockValidationStatus::Accepted,
        next_check: PbftManagerBlockValidationNextCheck::None,
        error_code: "",
    }
}

fn pbft_manager_block_validation_reject(
    status: PbftManagerBlockValidationStatus,
    error_code: &'static str,
) -> PbftManagerBlockValidationPlan {
    PbftManagerBlockValidationPlan {
        action: PbftManagerBlockValidationAction::Reject,
        status,
        next_check: PbftManagerBlockValidationNextCheck::None,
        error_code,
    }
}

fn pbft_manager_block_validation_contract_error(
    error_code: &'static str,
) -> PbftManagerBlockValidationPlan {
    PbftManagerBlockValidationPlan {
        action: PbftManagerBlockValidationAction::ContractError,
        status: PbftManagerBlockValidationStatus::InvalidBridgeFacts,
        next_check: PbftManagerBlockValidationNextCheck::None,
        error_code,
    }
}

fn pbft_manager_candidate_admission_contract_error(
    error_code: &'static str,
) -> PbftManagerCandidateAdmissionPlan {
    PbftManagerCandidateAdmissionPlan {
        action: PbftManagerCandidateAdmissionAction::ContractError,
        status: PbftManagerCandidateAdmissionStatus::InvalidBridgeFacts,
        mark_valid: false,
        error_code,
    }
}

fn pbft_manager_candidate_plan_from_selection(
    selection: PbftManagerLeaderSelectionPlan,
    valid_blocks: Vec<PbftManagerLeaderValidBlockCommand>,
) -> PbftManagerLeaderCandidatePlan {
    PbftManagerLeaderCandidatePlan {
        status: selection.status,
        selected: selection.selected,
        selected_vote_hash: selection.selected_vote_hash,
        selected_block_hash: selection.selected_block_hash,
        selected_period: selection.selected_period,
        selected_from_null_anchor: selection.selected_from_null_anchor,
        valid_blocks,
        error_code: selection.error_code,
    }
}

fn pbft_manager_leader_selected(
    candidate: PbftManagerLeaderCandidateFact,
    selected_from_null_anchor: bool,
) -> PbftManagerLeaderSelectionPlan {
    PbftManagerLeaderSelectionPlan {
        status: PbftManagerLeaderSelectionStatus::Selected,
        selected: true,
        selected_vote_hash: candidate.vote_hash,
        selected_block_hash: candidate.block_hash,
        selected_period: candidate.period,
        selected_from_null_anchor,
        error_code: "",
    }
}

fn pbft_manager_leader_no_selection(
    status: PbftManagerLeaderSelectionStatus,
    error_code: &'static str,
) -> PbftManagerLeaderSelectionPlan {
    PbftManagerLeaderSelectionPlan {
        status,
        selected: false,
        selected_vote_hash: H256::zero(),
        selected_block_hash: H256::zero(),
        selected_period: 0,
        selected_from_null_anchor: false,
        error_code,
    }
}

fn keccak256(data: &[u8]) -> H256 {
    let mut output = [0_u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut output);
    H256::from(output)
}

impl PbftManagerRuntimeStateCode {
    /// Stable bridge code for the state.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ValueProposal => 0,
            Self::Filter => 1,
            Self::Certify => 2,
            Self::Finish => 3,
            Self::FinishPolling => 4,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::ValueProposal,
            1 => Self::Filter,
            2 => Self::Certify,
            3 => Self::Finish,
            4 => Self::FinishPolling,
            _ => Self::Unknown,
        }
    }
}

/// Runtime status for one Rust-owned PBFT manager tick session.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeStatus {
    /// Session is ready to execute or report the next action.
    Active,
    /// Session completed all actions.
    Complete,
    /// The tick facts were rejected before execution.
    RejectedTick,
    /// Reported action does not match the current cursor.
    ActionMismatch,
    /// The C++ executor reported action failure.
    ActionFailed,
    /// The report was malformed for the current action.
    InvalidReport,
    /// Internal invariant or unknown bridge-code failure.
    ContractError,
    /// Unknown bridge status.
    Unknown,
}

impl PbftManagerRuntimeStatus {
    /// Stable bridge code for the status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::Complete => 1,
            Self::RejectedTick => 2,
            Self::ActionMismatch => 3,
            Self::ActionFailed => 4,
            Self::InvalidReport => 5,
            Self::ContractError => 255,
            Self::Unknown => 254,
        }
    }
}

/// Stable action codes for one PBFT manager daemon tick.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeAction {
    /// Drain synced PBFT blocks into the local chain path.
    ProcessSyncedPbftBlocks,
    /// Broadcast or rebroadcast votes according to existing C++ timers.
    MaybeBroadcastVotes,
    /// Try to push a cert-voted PBFT block into the chain.
    TryPushCertVotesBlock,
    /// Try to advance to a higher round from next-vote facts.
    TryAdvanceRound,
    /// Reset consensus to a Rust-selected target round.
    ResetConsensus,
    /// Sleep briefly when the node has no eligible wallet for active steps.
    SleepIneligiblePollingInterval,
    /// Execute value proposal behavior.
    RunValueProposal,
    /// Transition from value proposal to filter state.
    TransitionToFilter,
    /// Execute filtering / leader-identification behavior.
    RunFilter,
    /// Transition from filter to certify state.
    TransitionToCertify,
    /// Execute certifying behavior.
    RunCertify,
    /// Transition from certify to first finish state.
    TransitionToFinish,
    /// Delay certify polling without changing state.
    DelayCertifyPoll,
    /// Execute first finish behavior.
    RunFirstFinish,
    /// Transition from first finish to finish-polling state.
    TransitionToFinishPolling,
    /// Execute second finish / polling behavior.
    RunSecondFinish,
    /// Loop from finish-polling back to first finish.
    LoopBackFinish,
    /// Delay finish-polling without changing state.
    DelayFinishPoll,
    /// Sleep until the next planned step time.
    SleepUntilNextStep,
    /// Unknown bridge action code.
    Unknown,
}

impl PbftManagerRuntimeAction {
    /// Stable bridge code for the action.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ProcessSyncedPbftBlocks => 0,
            Self::MaybeBroadcastVotes => 1,
            Self::TryPushCertVotesBlock => 2,
            Self::TryAdvanceRound => 3,
            Self::SleepIneligiblePollingInterval => 4,
            Self::RunValueProposal => 5,
            Self::TransitionToFilter => 6,
            Self::RunFilter => 7,
            Self::TransitionToCertify => 8,
            Self::RunCertify => 9,
            Self::TransitionToFinish => 10,
            Self::DelayCertifyPoll => 11,
            Self::RunFirstFinish => 12,
            Self::TransitionToFinishPolling => 13,
            Self::RunSecondFinish => 14,
            Self::LoopBackFinish => 15,
            Self::DelayFinishPoll => 16,
            Self::SleepUntilNextStep => 17,
            Self::ResetConsensus => 18,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge action code.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::ProcessSyncedPbftBlocks),
            1 => Some(Self::MaybeBroadcastVotes),
            2 => Some(Self::TryPushCertVotesBlock),
            3 => Some(Self::TryAdvanceRound),
            4 => Some(Self::SleepIneligiblePollingInterval),
            5 => Some(Self::RunValueProposal),
            6 => Some(Self::TransitionToFilter),
            7 => Some(Self::RunFilter),
            8 => Some(Self::TransitionToCertify),
            9 => Some(Self::RunCertify),
            10 => Some(Self::TransitionToFinish),
            11 => Some(Self::DelayCertifyPoll),
            12 => Some(Self::RunFirstFinish),
            13 => Some(Self::TransitionToFinishPolling),
            14 => Some(Self::RunSecondFinish),
            15 => Some(Self::LoopBackFinish),
            16 => Some(Self::DelayFinishPoll),
            17 => Some(Self::SleepUntilNextStep),
            18 => Some(Self::ResetConsensus),
            _ => Some(Self::Unknown),
        }
    }
}

/// Stable PBFT manager effect catalog for the Rust-mode C++ executor.
///
/// Purpose:
/// - Names every larger live action boundary that remains around the
///   Rust-owned PBFT manager runtime.
/// - Gives follow-up slices a single vocabulary for replacing branch-local
///   C++ helper calls with Rust-planned ordered effects.
///
/// Inputs/outputs:
/// - Values are emitted or referenced by PBFT manager planners and sessions.
/// - C++ executors resolve compatibility sidecars, execute the requested live
///   action, and report the result back before Rust advances.
///
/// Invariants and edge behavior:
/// - Numeric codes are stable for bridge and transcript-test use.
/// - The enum catalogs executor boundaries only; it does not perform I/O,
///   mutate storage, send network messages, or materialize C++ objects.
/// - `Unknown` is reserved for rejected bridge values and must never be emitted
///   by Rust planners as an executable effect.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerEffectKind {
    /// Drain synced period data from the PBFT sync queue.
    ProcessSyncedPbftBlocks,
    /// Decide and execute PBFT vote/reward/pillar rebroadcasts.
    BroadcastVotes,
    /// Try to push the current cert-voted PBFT block into the chain.
    TryPushCertVotesBlock,
    /// Query VoteManager for a higher round candidate.
    DetermineNewRound,
    /// Apply a Rust-planned manager cursor transition.
    ApplyManagerTransition,
    /// Sleep because the local node has no eligible wallet for the active phase.
    SleepIneligiblePollingInterval,
    /// Sleep until the next planned PBFT step.
    SleepUntilNextStep,
    /// Construct a new PBFT proposal candidate.
    ConstructProposal,
    /// Resolve and validate a proposed PBFT block sidecar.
    ValidateProposedBlock,
    /// Rank proposal votes and resolve the selected leader block.
    ResolveLeaderBlock,
    /// Generate a local PBFT vote from Rust-owned vote bytes.
    GenerateVote,
    /// Insert a Rust-accepted vote into live compatibility sidecars.
    PlaceVote,
    /// Gossip a single vote or vote bundle through the network executor.
    GossipVote,
    /// Query FinalChain facts or wait for FinalChain progress.
    FinalChainFactOrWait,
    /// Query DAG ordering, block, weight, or cleanup facts.
    DagFactOrMutation,
    /// Query or mutate transaction manager finalization state.
    TransactionFactOrMutation,
    /// Validate, finalize, or post-process pillar chain data.
    PillarFactOrMutation,
    /// Apply PBFT finalization storage writes through Rust storage.
    ApplyFinalizationStorage,
    /// Dispatch FinalChain finalization outside the PBFT manager runtime.
    FinalizeFinalChain,
    /// Apply dynamic-lambda live state selected by Rust.
    ApplyDynamicLambda,
    /// Update live PBFT-chain compatibility state.
    UpdatePbftChain,
    /// Advance the PBFT period and related compatibility mirrors.
    AdvancePeriod,
    /// Report a malicious or invalid sync peer through the network executor.
    ReportPeer,
    /// Clear sync/proposed-block/anchor caches or other compatibility sidecars.
    ClearCompatibilityCache,
    /// Unknown bridge effect code.
    Unknown,
}

impl PbftManagerEffectKind {
    /// Stable bridge and transcript code for the PBFT manager effect.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ProcessSyncedPbftBlocks => 0,
            Self::BroadcastVotes => 1,
            Self::TryPushCertVotesBlock => 2,
            Self::DetermineNewRound => 3,
            Self::ApplyManagerTransition => 4,
            Self::SleepIneligiblePollingInterval => 5,
            Self::SleepUntilNextStep => 6,
            Self::ConstructProposal => 7,
            Self::ValidateProposedBlock => 8,
            Self::ResolveLeaderBlock => 9,
            Self::GenerateVote => 10,
            Self::PlaceVote => 11,
            Self::GossipVote => 12,
            Self::FinalChainFactOrWait => 13,
            Self::DagFactOrMutation => 14,
            Self::TransactionFactOrMutation => 15,
            Self::PillarFactOrMutation => 16,
            Self::ApplyFinalizationStorage => 17,
            Self::FinalizeFinalChain => 18,
            Self::ApplyDynamicLambda => 19,
            Self::UpdatePbftChain => 20,
            Self::AdvancePeriod => 21,
            Self::ReportPeer => 22,
            Self::ClearCompatibilityCache => 23,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge or transcript effect code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::ProcessSyncedPbftBlocks,
            1 => Self::BroadcastVotes,
            2 => Self::TryPushCertVotesBlock,
            3 => Self::DetermineNewRound,
            4 => Self::ApplyManagerTransition,
            5 => Self::SleepIneligiblePollingInterval,
            6 => Self::SleepUntilNextStep,
            7 => Self::ConstructProposal,
            8 => Self::ValidateProposedBlock,
            9 => Self::ResolveLeaderBlock,
            10 => Self::GenerateVote,
            11 => Self::PlaceVote,
            12 => Self::GossipVote,
            13 => Self::FinalChainFactOrWait,
            14 => Self::DagFactOrMutation,
            15 => Self::TransactionFactOrMutation,
            16 => Self::PillarFactOrMutation,
            17 => Self::ApplyFinalizationStorage,
            18 => Self::FinalizeFinalChain,
            19 => Self::ApplyDynamicLambda,
            20 => Self::UpdatePbftChain,
            21 => Self::AdvancePeriod,
            22 => Self::ReportPeer,
            23 => Self::ClearCompatibilityCache,
            _ => Self::Unknown,
        }
    }

    /// Returns true when the effect is intentionally outside the PBFT manager
    /// breakthrough boundary and must remain a C++ executor action for now.
    pub const fn is_external_boundary(self) -> bool {
        matches!(
            self,
            Self::BroadcastVotes
                | Self::GossipVote
                | Self::FinalChainFactOrWait
                | Self::FinalizeFinalChain
                | Self::ReportPeer
                | Self::SleepIneligiblePollingInterval
                | Self::SleepUntilNextStep
        )
    }
}

/// Stable result code for one C++-executed manager action.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeActionResultCode {
    /// Action succeeded and session should continue normally.
    NoProgressContinue,
    /// Action made progress and the manager loop must restart immediately.
    ProgressRestartLoop,
    /// State action completed.
    StateActionDone,
    /// State transition completed.
    TransitionApplied,
    /// Sleep action completed.
    SleepApplied,
    /// C++ executor reported an error.
    ExecutorError,
    /// Unknown bridge result.
    Unknown,
}

impl PbftManagerRuntimeActionResultCode {
    /// Stable bridge code for action report results.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NoProgressContinue => 0,
            Self::ProgressRestartLoop => 1,
            Self::StateActionDone => 2,
            Self::TransitionApplied => 3,
            Self::SleepApplied => 4,
            Self::ExecutorError => 255,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge result code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::NoProgressContinue,
            1 => Self::ProgressRestartLoop,
            2 => Self::StateActionDone,
            3 => Self::TransitionApplied,
            4 => Self::SleepApplied,
            255 => Self::ExecutorError,
            _ => Self::Unknown,
        }
    }
}

/// C++-originated facts for one PBFT manager daemon tick.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerRuntimeTickFact {
    /// Monotonic caller-local tick id for telemetry.
    pub tick_id: u64,
    /// Current PBFT manager state.
    pub state: PbftManagerRuntimeStateCode,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Whether the network handle is currently available.
    pub network_available: bool,
    /// Whether the network reports PBFT sync mode.
    pub network_pbft_syncing: bool,
    /// Initial eligibility snapshot for telemetry. The runtime branch uses the
    /// post-prestate value reported by C++ after `TryAdvanceRound`.
    pub has_eligible_wallet: bool,
    /// Polling sleep duration in milliseconds for ineligible-wallet ticks.
    ///
    /// C++ supplies the configured executor interval, but Rust carries it on
    /// the selected sleep command so the shell does not own the scheduling
    /// decision.
    pub polling_interval_ms: u64,
}

/// Stable action-intent codes for deterministic PBFT state actions.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStateActionIntent {
    /// No consensus-side work should be executed for this state action.
    Noop,
    /// Build and propose a fresh PBFT block for the current period/round.
    ProposeNewBlock,
    /// Re-propose the previous round's 2t+1 next-voted value.
    ReproposePreviousRoundNextValue,
    /// Identify the current round leader block and soft-vote it if present.
    IdentifyLeaderAndSoftVote,
    /// Soft-vote the previous round's 2t+1 next-voted value.
    SoftVotePreviousRoundNextValue,
    /// Cert-vote the current round's 2t+1 soft-voted value.
    CertVoteCurrentSoftValue,
    /// Move from certify polling to the finish state.
    GoFinish,
    /// Next-vote the block this node cert-voted in the current round.
    NextVoteCertVotedBlock,
    /// Next-vote the null block hash.
    NextVoteNullBlock,
    /// Next-vote the previous round's 2t+1 next-voted value.
    NextVotePreviousRoundValue,
    /// Next-vote the current round's 2t+1 soft-voted value.
    NextVoteCurrentSoftValue,
}

impl PbftManagerStateActionIntent {
    /// Stable bridge code for the state-action intent.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Noop => 0,
            Self::ProposeNewBlock => 1,
            Self::ReproposePreviousRoundNextValue => 2,
            Self::IdentifyLeaderAndSoftVote => 3,
            Self::SoftVotePreviousRoundNextValue => 4,
            Self::CertVoteCurrentSoftValue => 5,
            Self::GoFinish => 6,
            Self::NextVoteCertVotedBlock => 7,
            Self::NextVoteNullBlock => 8,
            Self::NextVotePreviousRoundValue => 9,
            Self::NextVoteCurrentSoftValue => 10,
        }
    }

    /// Decodes a stable bridge state-action code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::ProposeNewBlock,
            2 => Self::ReproposePreviousRoundNextValue,
            3 => Self::IdentifyLeaderAndSoftVote,
            4 => Self::SoftVotePreviousRoundNextValue,
            5 => Self::CertVoteCurrentSoftValue,
            6 => Self::GoFinish,
            7 => Self::NextVoteCertVotedBlock,
            8 => Self::NextVoteNullBlock,
            9 => Self::NextVotePreviousRoundValue,
            10 => Self::NextVoteCurrentSoftValue,
            _ => Self::Noop,
        }
    }
}

/// Stable status codes for PBFT manager state-action planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStateActionStatus {
    /// The plan is usable by the C++ executor.
    Ready,
    /// The supplied state is unknown or unsupported.
    InvalidState,
    /// The supplied fact bundle is internally inconsistent.
    InvalidFact,
}

impl PbftManagerStateActionStatus {
    /// Stable bridge code for the state-action status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidState => 1,
            Self::InvalidFact => 2,
        }
    }
}

/// C++-originated facts for one PBFT manager state action.
///
/// The fact bundle is intentionally compact and contains only deterministic
/// branch inputs. C++ remains responsible for sourcing those facts from live
/// vote/proposed-block sidecars, executing returned intents, materializing
/// blocks and votes, writing storage, and emitting network effects.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionFact {
    /// State being executed.
    pub state: PbftManagerRuntimeStateCode,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Elapsed milliseconds in the current round.
    pub elapsed_round_ms: u64,
    /// PBFT deadline for the current round in milliseconds.
    pub deadline_ms: u64,
    /// Current round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Polling interval used by the legacy manager loop.
    pub polling_interval_ms: u64,
    /// Whether the previous round has 2t+1 next votes for null.
    pub has_previous_round_next_null: bool,
    /// Whether the previous round has 2t+1 next votes for a block value.
    pub has_previous_round_next_value: bool,
    /// Previous round 2t+1 next-voted block value, when present.
    pub previous_round_next_value_hash: [u8; 32],
    /// Whether the current round has 2t+1 soft votes for a block value.
    pub has_current_round_soft_value: bool,
    /// Current round 2t+1 soft-voted block value, when present.
    pub current_round_soft_value_hash: [u8; 32],
    /// Whether this node already cert-voted a block in this round.
    pub has_cert_voted_block: bool,
    /// Current round cert-voted block hash, when present.
    pub cert_voted_block_hash: [u8; 32],
    /// Whether this node already emitted a next vote for a soft-voted value.
    pub already_next_voted_value: bool,
    /// Whether this node already emitted a null-block next vote.
    pub already_next_voted_null: bool,
}

/// Side-effect-free PBFT manager state-action plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionPlan {
    /// Planning status.
    pub status: PbftManagerStateActionStatus,
    /// Primary action intent for the C++ executor.
    pub primary_intent: PbftManagerStateActionIntent,
    /// Hash argument for the primary intent, if applicable.
    pub primary_hash: [u8; 32],
    /// Secondary action intent for states that can emit two vote attempts.
    pub secondary_intent: PbftManagerStateActionIntent,
    /// Hash argument for the secondary intent, if applicable.
    pub secondary_hash: [u8; 32],
    /// Planned value for `go_finish_state_`.
    pub go_finish_state: bool,
    /// Planned value for `loop_back_finish_state_`.
    pub loop_back_finish_state: bool,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

/// One ordered PBFT manager state-action effect for the C++ executor.
///
/// Inputs:
/// - `intent` names the live action C++ must execute.
/// - `hash` carries the block hash argument for intents that need one.
/// - `request_proposed_block_sidecar` and the sidecar identity fields are set
///   for effects whose executor must resolve a proposed PBFT block before it
///   can generate a vote or re-proposal.
///
/// Invariants:
/// - Effects are emitted in the order Rust expects them to run.
/// - C++ must not reorder effects or infer extra branch work outside this list.
/// - Rust owns which effects require proposed-block sidecar materialization.
///   C++ remains the executor for materialization, vote generation, storage
///   mutation, and gossip until those dependencies move to Rust.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionEffect {
    /// Effect intent for the C++ executor.
    pub intent: PbftManagerStateActionIntent,
    /// Hash argument for the effect, if applicable.
    pub hash: [u8; 32],
    /// True when C++ must materialize/admit the proposed-block sidecar.
    pub request_proposed_block_sidecar: bool,
    /// Proposed-block sidecar hash requested by Rust.
    pub proposed_block_sidecar_hash: [u8; 32],
    /// Proposed-block sidecar period requested by Rust.
    pub proposed_block_sidecar_period: u64,
}

/// Ordered PBFT manager state-action effect plan.
///
/// This is the effect-oriented successor surface for
/// `plan_pbft_manager_state_action`. It keeps the same deterministic branch
/// decisions but returns an ordered effect vector so the C++ shim can use one
/// executor loop for value proposal, filter, certify, first finish, and finish
/// polling. Empty `effects` is a valid no-op plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionEffectPlan {
    /// Planning status.
    pub status: PbftManagerStateActionStatus,
    /// Ordered live effects to execute.
    pub effects: Vec<PbftManagerStateActionEffect>,
    /// Planned value for `go_finish_state_`.
    pub go_finish_state: bool,
    /// Planned value for `loop_back_finish_state_`.
    pub loop_back_finish_state: bool,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

/// Status for a Rust-owned PBFT manager state-action effect session.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStateActionSessionStatus {
    /// The session is active and may yield more effects.
    Active,
    /// All planned effects completed successfully.
    Complete,
    /// The original fact bundle was rejected by the planner.
    RejectedFact,
    /// C++ reported an effect that did not match the pending cursor/intent.
    EffectMismatch,
    /// The report used an unknown result code.
    InvalidReport,
    /// C++ reported a live check or sidecar failure for the pending effect.
    EffectFailed,
    /// C++ reported an executor or bridge contract error.
    ContractError,
}

impl PbftManagerStateActionSessionStatus {
    /// Stable bridge code for the session status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::Complete => 1,
            Self::RejectedFact => 2,
            Self::EffectMismatch => 3,
            Self::InvalidReport => 4,
            Self::EffectFailed => 5,
            Self::ContractError => 6,
        }
    }
}

/// Result code reported by C++ after executing one state-action effect.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStateActionEffectResultCode {
    /// The executor applied the effect or completed its no-progress live check.
    Applied,
    /// The effect was valid but produced no mutation.
    SkippedNoWork,
    /// A required live block, vote, or sidecar was unavailable.
    SkippedMissingLiveObject,
    /// A live compatibility check rejected the effect.
    RejectedLiveCheck,
    /// Unknown bridge result code.
    Unknown,
    /// The executor hit an unsupported effect, exception, or contract error.
    ExecutorError,
}

impl PbftManagerStateActionEffectResultCode {
    /// Stable bridge code for effect reports.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Applied => 0,
            Self::SkippedNoWork => 1,
            Self::SkippedMissingLiveObject => 2,
            Self::RejectedLiveCheck => 3,
            Self::Unknown => 254,
            Self::ExecutorError => 255,
        }
    }

    /// Decodes a stable bridge code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Applied,
            1 => Self::SkippedNoWork,
            2 => Self::SkippedMissingLiveObject,
            3 => Self::RejectedLiveCheck,
            254 => Self::Unknown,
            255 => Self::ExecutorError,
            _ => Self::Unknown,
        }
    }
}

/// Report supplied by C++ after executing a Rust-planned state-action effect.
///
/// Inputs:
/// - `cursor` and `intent` must match the pending effect returned by Rust.
/// - `result` reports whether the live executor accepted the effect.
/// - `error_code` carries executor diagnostics for rejected effects.
///
/// Invariants:
/// - Rust validates report ordering before advancing the effect cursor.
/// - Reports do not carry live objects; C++ remains the temporary owner of
///   sidecar materialization and mutation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionEffectReport {
    /// Cursor returned with the pending effect.
    pub cursor: u32,
    /// Effect intent C++ attempted to execute.
    pub intent: PbftManagerStateActionIntent,
    /// Executor result.
    pub result: PbftManagerStateActionEffectResultCode,
    /// Optional executor diagnostic for rejected effects.
    pub error_code: String,
}

/// One step from a Rust-owned PBFT manager state-action effect session.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionSessionStep {
    /// Session status after the last report.
    pub status: PbftManagerStateActionSessionStatus,
    /// Monotonic effect cursor.
    pub cursor: u32,
    /// True when `effect` contains work for the C++ executor.
    pub has_effect: bool,
    /// Pending effect for C++ execution.
    pub effect: PbftManagerStateActionEffect,
    /// Planned value for `go_finish_state_`.
    pub go_finish_state: bool,
    /// Planned value for `loop_back_finish_state_`.
    pub loop_back_finish_state: bool,
    /// True when the session reached a terminal status.
    pub complete: bool,
    /// True when the C++ caller may continue with follow-up manager routing.
    pub can_continue: bool,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: String,
}

/// Rust-owned cursor for ordered PBFT manager state-action effects.
///
/// The session wraps `PbftManagerStateActionEffectPlan` and exposes one effect
/// at a time. C++ must report each effect before Rust advances. This keeps
/// state-action ordering in Rust while leaving live side effects outside the
/// PBFT manager migration boundary.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionEffectSession {
    /// Planned effects and state flags.
    pub plan: PbftManagerStateActionEffectPlan,
    cursor: usize,
    status: PbftManagerStateActionSessionStatus,
    pending: Option<PbftManagerStateActionEffect>,
}

/// Stable transition-kind codes for PBFT manager cursor mutation planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerTransitionKind {
    /// Reset the consensus cursor for a new round.
    ResetConsensus,
    /// Move from value-proposal to filtering.
    ToFilter,
    /// Move from filtering to certifying.
    ToCertify,
    /// Move from certifying to first finish.
    ToFinish,
    /// Move from first finish to finish polling.
    ToFinishPolling,
    /// Loop from finish polling back to first finish.
    LoopBackFinish,
    /// Delay certify polling without changing phase.
    DelayCertifyPoll,
    /// Delay finish polling without changing phase.
    DelayFinishPoll,
    /// Unknown bridge transition code.
    Unknown,
}

impl PbftManagerTransitionKind {
    /// Stable bridge code for this transition kind.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ResetConsensus => 0,
            Self::ToFilter => 1,
            Self::ToCertify => 2,
            Self::ToFinish => 3,
            Self::ToFinishPolling => 4,
            Self::LoopBackFinish => 5,
            Self::DelayCertifyPoll => 6,
            Self::DelayFinishPoll => 7,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge transition code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::ResetConsensus,
            1 => Self::ToFilter,
            2 => Self::ToCertify,
            3 => Self::ToFinish,
            4 => Self::ToFinishPolling,
            5 => Self::LoopBackFinish,
            6 => Self::DelayCertifyPoll,
            7 => Self::DelayFinishPoll,
            _ => Self::Unknown,
        }
    }
}

/// Stable status codes for PBFT manager transition planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerTransitionStatus {
    /// The transition plan is usable by the C++ executor.
    Ready,
    /// The supplied transition kind is unknown.
    InvalidKind,
    /// The supplied facts are internally inconsistent.
    InvalidFact,
}

impl PbftManagerTransitionStatus {
    /// Stable bridge code for the transition status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidKind => 1,
            Self::InvalidFact => 2,
        }
    }
}

/// C++-originated facts for one PBFT manager cursor/status transition.
///
/// The fact bundle contains only scalar state, timing, and already-sourced
/// network vote progress. Rust decides the resulting manager cursor, lambda,
/// next-step deadline, manager-status resets, durable storage commit, and
/// runtime publication. C++ remains the executor only for returned
/// VoteManager, live-sidecar, timer, and compatibility logging effects.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerTransitionFact {
    /// Transition kind requested by the runtime cursor.
    pub kind: PbftManagerTransitionKind,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Target round for `ResetConsensus`; ignored by phase transitions.
    pub target_round: u64,
    /// Current round lambda before the transition.
    pub current_round_lambda_ms: u64,
    /// Lambda calculated for the target round under Cacti.
    pub target_round_lambda_ms: u64,
    /// Genesis/default lambda used before Cacti and for exponential reset.
    pub default_lambda_ms: u64,
    /// Maximum exponential lambda.
    pub max_exponential_lambda_ms: u64,
    /// Odd step where exponential backoff starts.
    pub max_steps: u64,
    /// Greatest network t+1 next-voting step already sourced by C++.
    pub network_next_voting_step: u64,
    /// PBFT deadline for the current round in milliseconds.
    pub deadline_ms: u64,
    /// Polling interval used by finish polling.
    pub polling_interval_ms: u64,
    /// Current `next_step_time_ms_`.
    pub next_step_time_ms: u64,
    /// Whether the target period is on the Cacti hardfork.
    pub cacti_hardfork: bool,
    /// Whether a cert-voted block sidecar exists and may need removal.
    pub has_cert_voted_block: bool,
    /// Whether an executed PBFT block flag is set and requires executor reset.
    pub executed_pbft_block: bool,
}

/// External/configuration inputs for a runtime-derived lifecycle transition.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerLifecycleTransitionRequest {
    pub kind: PbftManagerTransitionKind,
    pub target_period: u64,
    pub target_round: u64,
    pub has_network_next_voting_step: bool,
    pub network_next_voting_step: u64,
}

/// Side-effect-free plan for one PBFT manager cursor/status transition.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerTransitionPlan {
    /// Planning status.
    pub status: PbftManagerTransitionStatus,
    /// Transition kind echoed back for executor validation.
    pub kind: PbftManagerTransitionKind,
    /// Planned PBFT state after the transition.
    pub new_state: PbftManagerRuntimeStateCode,
    /// Planned round after the transition.
    pub new_round: u64,
    /// Planned step after the transition.
    pub new_step: u64,
    /// Planned current-round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Planned next-step deadline in milliseconds.
    pub next_step_time_ms: u64,
    /// Persist the planned round field.
    pub persist_round: bool,
    /// Persist the planned step field.
    pub persist_step: bool,
    /// Reset next-voted manager status bits and live flags.
    pub reset_next_voted_statuses: bool,
    /// Remove the saved cert-voted block if present.
    pub remove_cert_voted_block: bool,
    /// Clear local own-vote records through the VoteManager executor.
    pub clear_own_votes: bool,
    /// Clear current-round broadcast bookkeeping.
    pub clear_broadcasted_votes: bool,
    /// Reset current-round broadcast counters.
    pub reset_broadcast_counters: bool,
    /// Reset the executed-block manager status after period finalization.
    pub reset_executed_block_status: bool,
    /// Update the VoteManager period/round executor boundary.
    pub set_vote_manager_period_round: bool,
    /// Reset current round start time in C++.
    pub reset_current_round_start: bool,
    /// Reset second-finish polling start time in C++.
    pub reset_second_finish_start: bool,
    /// Set the certify-step log flag.
    pub print_cert_step_info: bool,
    /// Set the second-finish-step log flag.
    pub print_second_finish_step_info: bool,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

/// Durable storage result for one PBFT manager transition commit.
///
/// Inputs:
/// - Produced only by Rust-owned PBFT manager transition storage helpers.
///
/// Outputs:
/// - `status` records whether the storage commit applied or was rejected.
/// - `applied_writes` reports the number of manager/status/vote rows written
///   or removed before commit.
/// - `error_code` is stable bridge-facing detail for rejected plans, overflow,
///   storage write failure, or commit failure.
///
/// Invariants and edge behavior:
/// - Rejected results are returned before the Rust runtime cursor is advanced.
/// - Rejected write batches are dropped by ownership; callers must not update
///   C++ mirrors or runtime snapshots for rejected results.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerTransitionStorageResult {
    /// Stable storage-apply status.
    pub status: PbftManagerTransitionStorageStatus,
    /// Number of durable writes/deletes requested by the accepted commit.
    pub applied_writes: u64,
    /// Stable rejection detail, empty for applied commits.
    pub error_code: String,
}

/// Stable storage-apply status for PBFT manager transition commits.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerTransitionStorageStatus {
    /// The Rust-owned storage batch committed.
    Applied,
    /// The plan or storage operation was rejected without advancing runtime
    /// state.
    Rejected,
}

impl PbftManagerTransitionStorageStatus {
    /// Stable bridge code for the transition storage result.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Applied => 0,
            Self::Rejected => 1,
        }
    }
}

/// Stable status codes for PBFT manager runtime startup restore.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStartupRestoreStatus {
    /// Startup facts are valid and the runtime snapshot is usable.
    Ready,
    /// Startup facts are internally inconsistent or represent corrupted
    /// persisted manager state.
    InvalidFact,
}

impl PbftManagerStartupRestoreStatus {
    /// Stable bridge code for the startup restore status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidFact => 1,
        }
    }
}

/// Persisted and configuration facts used to restore the PBFT manager runtime.
///
/// The fact bundle is deliberately scalar-only. Storage and bridge code read
/// persisted DB values, then Rust decides the normalized PBFT cursor and live
/// startup flags without materializing PBFT blocks, votes, network handles, or
/// FinalChain objects.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStartupRestoreFact {
    /// Current PBFT period at startup.
    pub current_period: u64,
    /// Persisted manager round, defaulted by storage compatibility to `1` when
    /// absent.
    pub persisted_round: u64,
    /// Persisted manager step, defaulted by storage compatibility to `1` when
    /// absent.
    pub persisted_step: u64,
    /// Whether the Cacti dynamic-lambda rules are active for
    /// `current_period - 1`.
    pub cacti_active_at_chain_size: bool,
    /// Persisted rounds-count dynamic-lambda accumulator.
    pub rounds_count_dynamic_lambda: u32,
    /// Persisted dynamic lambda manager field, defaulted by storage
    /// compatibility to `1` when absent.
    pub persisted_dynamic_lambda_ms: u32,
    /// Genesis PBFT lambda used before Cacti.
    pub genesis_lambda_ms: u32,
    /// Cacti maximum lambda used as live default before any finalized Cacti
    /// period has saved a dynamic lambda.
    pub cacti_lambda_max_ms: u32,
    /// Cacti non-round-one lambda.
    pub cacti_lambda_default_ms: u32,
    /// Persisted executed-block manager status.
    pub executed_pbft_block: bool,
    /// Persisted next-voted-value manager status.
    pub already_next_voted_value: bool,
    /// Persisted next-voted-null manager status.
    pub already_next_voted_null: bool,
}

/// Storage-backed PBFT manager startup configuration.
///
/// Purpose:
/// - Carries only non-storage startup configuration into the native Rust
///   storage restore path. Persisted manager fields and statuses are read
///   directly from `rustaxa-storage` by
///   `create_pbft_manager_runtime_from_storage`.
///
/// Inputs:
/// - `current_period` is the PBFT period observed by the C++ compatibility
///   shell at startup.
/// - Cacti and lambda fields are configuration facts that are not stored in
///   the PBFT manager storage columns.
///
/// Invariants and edge behavior:
/// - Lambda values must be nonzero. Invalid or corrupted persisted storage
///   facts are rejected with stable error labels from
///   `restore_pbft_manager_runtime`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerStorageStartupFact {
    /// Current PBFT period at startup.
    pub current_period: u64,
    /// Whether the Cacti dynamic-lambda rules are active for
    /// `current_period - 1`.
    pub cacti_active_at_chain_size: bool,
    /// Genesis PBFT lambda used before Cacti.
    pub genesis_lambda_ms: u32,
    /// Cacti maximum lambda used as live default before any finalized Cacti
    /// period has saved a dynamic lambda.
    pub cacti_lambda_max_ms: u32,
    /// Cacti non-round-one lambda.
    pub cacti_lambda_default_ms: u32,
    pub cacti_block: u64,
    pub max_exponential_lambda_ms: u64,
    pub max_steps: u64,
    pub deadline_ms: u64,
    pub polling_interval_ms: u64,
}

/// Runtime cursor and live scalar facts restored for the PBFT manager shim.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeSnapshot {
    /// Restore/apply status.
    pub status: PbftManagerStartupRestoreStatus,
    /// Current PBFT manager state.
    pub state: PbftManagerRuntimeStateCode,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Current-round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Next-step deadline in milliseconds.
    pub next_step_time_ms: u64,
    /// Live dynamic-lambda accumulator restored from storage.
    pub rounds_count_dynamic_lambda: u32,
    /// Live dynamic lambda in milliseconds.
    pub dynamic_lambda_ms: u32,
    /// Live executed-block flag.
    pub executed_pbft_block: bool,
    /// Live next-voted-value flag.
    pub already_next_voted_value: bool,
    /// Live next-voted-null flag.
    pub already_next_voted_null: bool,
    /// Live round-vote broadcast counter.
    pub broadcast_votes_counter: u32,
    /// Live round-vote rebroadcast counter.
    pub rebroadcast_votes_counter: u32,
    /// Live reward-vote broadcast counter.
    pub broadcast_reward_votes_counter: u32,
    /// Live reward-vote rebroadcast counter.
    pub rebroadcast_reward_votes_counter: u32,
    /// Whether Rust has an active cert-voted PBFT block metadata record.
    pub has_cert_voted_block: bool,
    /// PBFT period of the active cert-voted block metadata.
    pub cert_voted_block_period: u64,
    /// PBFT round that produced the active cert-voted block metadata.
    pub cert_voted_block_round: u64,
    /// Hash of the active cert-voted PBFT block.
    pub cert_voted_block_hash: H256,
    /// Whether startup normalized persisted step and must persist the new step
    /// before C++ mirrors are updated.
    pub persist_normalized_step: bool,
    /// Whether C++ should reset the second-finish polling timestamp.
    pub reset_second_finish_start: bool,
    /// Stable error detail for rejected startup facts.
    pub error_code: String,
}

/// Deterministic facts required to plan the PBFT manager sleep-before-next-step wait.
///
/// Inputs:
/// - `next_step_time_ms`: Rust-owned next-step deadline measured from the
///   current round start.
/// - `round_elapsed_ms`: C++-observed elapsed wall-clock time for the current
///   round. C++ still owns the clock and condition-variable wait while the app
///   host remains in C++.
/// - `step`: Current PBFT step, echoed into the plan so the compatibility shell
///   can log the Rust-planned wait without choosing consensus behavior.
///
/// Invariants and edge behavior:
/// - This planner is side-effect free and does not sleep.
/// - If the deadline has already passed, the plan tells the executor not to
///   sleep.
/// - Negative elapsed time is preserved for legacy wall-clock behavior: it
///   lengthens the computed wait instead of being clamped.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerSleepFact {
    /// Next-step deadline in milliseconds from the current round start.
    pub next_step_time_ms: i64,
    /// Elapsed milliseconds in the current PBFT round.
    pub round_elapsed_ms: i64,
    /// Current PBFT step.
    pub step: u64,
}

/// Rust-owned PBFT manager sleep-before-next-step plan for the C++ executor.
///
/// Outputs:
/// - `accepted` is false only when the supplied facts cannot be represented as
///   a C++ wait duration.
/// - `should_sleep` tells C++ whether to execute the condition-variable wait.
/// - `sleep_ms` is the wait duration when `should_sleep` is true.
/// - `step` echoes the input step for compatibility logging.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerSleepPlan {
    /// Whether the fact bundle was accepted.
    pub accepted: bool,
    /// Whether C++ should wait before re-entering the PBFT manager loop.
    pub should_sleep: bool,
    /// Wait duration in milliseconds when `should_sleep` is true.
    pub sleep_ms: u64,
    /// Current PBFT step from the accepted fact bundle.
    pub step: u64,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

/// Plans the PBFT manager wait before entering the next PBFT step.
///
/// Purpose:
/// - Moves the deterministic timer decision out of the PBFT manager shim while
///   leaving OS sleep and wakeup mechanics in C++ until the app host migrates.
///
/// Inputs:
/// - `fact`: deadline and elapsed-time facts for the current PBFT round.
///
/// Outputs:
/// - A side-effect-free sleep plan. C++ must execute the returned wait
///   mechanically and must not recompute the deadline comparison.
pub fn plan_pbft_manager_sleep_until_next_step(fact: PbftManagerSleepFact) -> PbftManagerSleepPlan {
    let remaining_ms = fact.next_step_time_ms.saturating_sub(fact.round_elapsed_ms);
    if remaining_ms <= 0 {
        return PbftManagerSleepPlan {
            accepted: true,
            should_sleep: false,
            sleep_ms: 0,
            step: fact.step,
            error_code: String::new(),
        };
    }

    match u64::try_from(remaining_ms) {
        Ok(sleep_ms) => PbftManagerSleepPlan {
            accepted: true,
            should_sleep: true,
            sleep_ms,
            step: fact.step,
            error_code: String::new(),
        },
        Err(_) => PbftManagerSleepPlan {
            accepted: false,
            should_sleep: false,
            sleep_ms: 0,
            step: fact.step,
            error_code: "PBFT_MANAGER_SLEEP_DURATION_OVERFLOW".to_string(),
        },
    }
}

/// Plans the PBFT manager wait from a Rust-owned runtime snapshot.
///
/// Purpose:
/// - Lets bridge callers keep timer authority with the Rust runtime: C++ only
///   supplies the elapsed wall-clock fact and executes the returned wait.
///
/// Inputs:
/// - `snapshot`: current Rust-owned PBFT manager runtime cursor and deadline.
/// - `round_elapsed_ms`: C++-observed elapsed milliseconds for the current
///   round.
///
/// Outputs:
/// - A side-effect-free sleep plan. Rejected snapshots or unrepresentable
///   deadlines return `accepted = false` with a stable error code.
pub fn plan_pbft_manager_runtime_sleep_until_next_step(
    snapshot: &PbftManagerRuntimeSnapshot,
    round_elapsed_ms: i64,
) -> PbftManagerSleepPlan {
    if snapshot.status != PbftManagerStartupRestoreStatus::Ready {
        return PbftManagerSleepPlan {
            accepted: false,
            should_sleep: false,
            sleep_ms: 0,
            step: snapshot.step,
            error_code: if snapshot.error_code.is_empty() {
                "PBFT_MANAGER_SLEEP_RUNTIME_SNAPSHOT_NOT_READY".to_string()
            } else {
                snapshot.error_code.clone()
            },
        };
    }

    let Ok(next_step_time_ms) = i64::try_from(snapshot.next_step_time_ms) else {
        return PbftManagerSleepPlan {
            accepted: false,
            should_sleep: false,
            sleep_ms: 0,
            step: snapshot.step,
            error_code: "PBFT_MANAGER_SLEEP_DEADLINE_OVERFLOW".to_string(),
        };
    };

    plan_pbft_manager_sleep_until_next_step(PbftManagerSleepFact {
        next_step_time_ms,
        round_elapsed_ms,
        step: snapshot.step,
    })
}

/// Facts required to decide whether PBFT manager startup must wait for FinalChain finalization.
///
/// Inputs:
/// - `pbft_chain_size`: current PBFT-chain size observed by the C++ shell.
/// - `final_chain_last_block`: latest finalized block number observed at the
///   accepted FinalChain boundary.
/// - `delegation_delay`: configured FinalChain DPoS delegation delay.
/// - `polling_interval_ms`: executor sleep duration that C++ will run when
///   Rust says the wait must continue.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerFinalizationWaitFact {
    /// Current PBFT-chain size.
    pub pbft_chain_size: u64,
    /// Latest finalized FinalChain block number.
    pub final_chain_last_block: u64,
    /// Configured delegation delay.
    pub delegation_delay: u64,
    /// Polling sleep duration in milliseconds.
    pub polling_interval_ms: u64,
}

/// Rust-owned startup finalization wait plan for the C++ sleep executor.
///
/// Outputs:
/// - `should_wait` is true when PBFT is ahead of FinalChain plus delegation
///   delay and the shell should execute `sleep_ms` before checking again.
/// - `accepted` is false only when the readiness threshold cannot be computed.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerFinalizationWaitPlan {
    /// Whether the fact bundle was accepted.
    pub accepted: bool,
    /// Whether C++ should continue waiting.
    pub should_wait: bool,
    /// Wait duration in milliseconds when `should_wait` is true.
    pub sleep_ms: u64,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

/// Plans the PBFT manager startup wait for FinalChain finalization readiness.
///
/// Purpose:
/// - Moves the deterministic readiness comparison out of the PBFT manager shim
///   while C++ keeps the startup loop and sleep mechanics.
pub fn plan_pbft_manager_finalization_wait(
    fact: PbftManagerFinalizationWaitFact,
) -> PbftManagerFinalizationWaitPlan {
    let Some(ready_height) = fact
        .final_chain_last_block
        .checked_add(fact.delegation_delay)
    else {
        return PbftManagerFinalizationWaitPlan {
            accepted: false,
            should_wait: false,
            sleep_ms: 0,
            error_code: "PBFT_MANAGER_FINALIZATION_WAIT_READY_HEIGHT_OVERFLOW".to_string(),
        };
    };

    PbftManagerFinalizationWaitPlan {
        accepted: true,
        should_wait: fact.pbft_chain_size > ready_height,
        sleep_ms: if fact.pbft_chain_size > ready_height {
            fact.polling_interval_ms
        } else {
            0
        },
        error_code: String::new(),
    }
}

/// Facts required to decide whether PBFT manager vote-count queries must wait for eligible-wallet period readiness.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerEligibleWalletPeriodWaitFact {
    /// Period currently loaded by the eligible-wallet runtime.
    pub eligible_wallet_period: u64,
    /// Current PBFT-chain size.
    pub pbft_chain_size: u64,
    /// Polling sleep duration in milliseconds.
    pub polling_interval_ms: u64,
}

/// Rust-owned eligible-wallet period readiness plan for the C++ polling executor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerEligibleWalletPeriodWaitPlan {
    /// Whether C++ should keep waiting.
    pub should_wait: bool,
    /// Wait duration in milliseconds when `should_wait` is true.
    pub sleep_ms: u64,
}

/// Plans whether PBFT manager vote-count queries should wait for eligible-wallet period readiness.
///
/// Purpose:
/// - Moves the deterministic eligible-wallet-period readiness comparison out of
///   the PBFT manager shell while C++ keeps the public-query polling loop.
pub fn plan_pbft_manager_eligible_wallet_period_wait(
    fact: PbftManagerEligibleWalletPeriodWaitFact,
) -> PbftManagerEligibleWalletPeriodWaitPlan {
    let should_wait = fact.eligible_wallet_period != fact.pbft_chain_size;
    PbftManagerEligibleWalletPeriodWaitPlan {
        should_wait,
        sleep_ms: if should_wait {
            fact.polling_interval_ms
        } else {
            0
        },
    }
}

/// Long-lived PBFT manager runtime cursor owned by Rust.
///
/// This runtime owns the scalar PBFT manager cursor restored from storage and
/// updated after accepted transition storage commits. It does not own timers,
/// network effects, FinalChain/EVM execution, or live C++ PBFT object
/// materialization.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntime {
    snapshot: PbftManagerRuntimeSnapshot,
    cached_anchor_dag_order_hashes: BTreeSet<H256>,
    policy: PbftManagerLifecyclePolicy,
    last_committed_reset: Option<PbftManagerCommittedReset>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
struct PbftManagerLifecyclePolicy {
    cacti_block: u64,
    genesis_lambda_ms: u64,
    cacti_lambda_default_ms: u64,
    max_exponential_lambda_ms: u64,
    max_steps: u64,
    deadline_ms: u64,
    polling_interval_ms: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct PbftManagerCommittedReset {
    target_period: u64,
    reset_executed_block_follow_up: bool,
    set_vote_manager_period_round: bool,
    reset_current_round_timer: bool,
}

/// Storage-backed facts for replaying one finalized period during PBFT manager startup.
///
/// Inputs:
/// - Loaded from `rustaxa-storage` by `load_pbft_manager_startup_replay_period`.
///
/// Outputs:
/// - `period_data_rlp` is the canonical legacy `PeriodData` payload.
/// - `finalized_dag_hashes` preserves the finalized DAG block order encoded in
///   the period data.
/// - `period_lambda` is the closest persisted dynamic lambda when requested by
///   the startup replay path.
///
/// Invariants and edge behavior:
/// - `found = false` means no period data was present; all payload fields are
///   empty/default.
/// - Malformed period data returns an error rather than falling back to C++
///   storage decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftManagerStartupReplayPeriod {
    /// Whether the requested period data exists in storage.
    pub found: bool,
    /// Canonical legacy `PeriodData` RLP bytes for C++ temporary sidecar materialization.
    pub period_data_rlp: Vec<u8>,
    /// Finalized DAG block hashes in persisted order.
    pub finalized_dag_hashes: Vec<H256>,
    /// Closest persisted dynamic lambda for this period, when requested and present.
    pub period_lambda: Option<u32>,
}

/// Startup replay range facts supplied by the compatibility shell.
///
/// Purpose:
/// - Moves startup range selection out of the PBFT manager overlay while
///   keeping FinalChain height, PBFT-chain size, and delegation-delay sourcing
///   at their current executor boundaries.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerStartupReplayRangeFact {
    /// Last block finalized by FinalChain at PBFT manager startup.
    pub final_chain_last_block: u64,
    /// Current PBFT chain size at startup.
    pub pbft_chain_size: u64,
    /// FinalChain delegation delay used for recently-finalized transaction hydration.
    pub delegation_delay: u64,
    /// Legacy multiplier for recently-finalized transaction replay coverage.
    pub recently_finalized_factor: u64,
}

/// Rust-owned startup replay range plan.
///
/// Outputs:
/// - `finalization_*` covers finalized PBFT periods that FinalChain must replay.
/// - `recent_*` covers periods used to hydrate recently-finalized transaction
///   compatibility sidecars.
///
/// Invariants and edge behavior:
/// - Empty finalization ranges are represented by
///   `has_finalization_range = false`.
/// - Recent replay always has a bounded inclusive range when PBFT chain size is
///   nonzero, preserving legacy period `1` as the minimum.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStartupReplayRangePlan {
    /// Whether the plan is usable.
    pub accepted: bool,
    /// Whether FinalChain replay has at least one period.
    pub has_finalization_range: bool,
    /// Inclusive first period for FinalChain replay.
    pub finalization_from_period: u64,
    /// Inclusive last period for FinalChain replay.
    pub finalization_to_period: u64,
    /// Inclusive first period for recently-finalized transaction hydration.
    pub recent_from_period: u64,
    /// Inclusive last period for recently-finalized transaction hydration.
    pub recent_to_period: u64,
    /// Stable error code, empty on success.
    pub error_code: String,
}

/// Ordered effects for `PbftManager::advancePeriod`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerAdvancePeriodAction {
    /// Apply the delayed executed-block reset after waiting for finalization.
    ApplyExecutedBlockReset,
    /// Update VoteManager period/round after the reset transition.
    SetVoteManagerPeriodRound,
    /// Reset current-round timer in the compatibility shell.
    ResetCurrentRoundTimer,
    /// Reset reward-vote broadcast counters.
    ResetRewardVoteCounters,
    /// Reset current-period timer in the compatibility shell.
    ResetPeriodTimer,
    /// Update wallet eligibility after reset/wait-for-finalization.
    UpdateWalletEligibility,
}

impl PbftManagerAdvancePeriodAction {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ApplyExecutedBlockReset => 1,
            Self::SetVoteManagerPeriodRound => 2,
            Self::ResetCurrentRoundTimer => 3,
            Self::ResetRewardVoteCounters => 4,
            Self::ResetPeriodTimer => 5,
            Self::UpdateWalletEligibility => 6,
        }
    }

    /// Decodes a stable bridge code from C++.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::ApplyExecutedBlockReset),
            2 => Some(Self::SetVoteManagerPeriodRound),
            3 => Some(Self::ResetCurrentRoundTimer),
            4 => Some(Self::ResetRewardVoteCounters),
            5 => Some(Self::ResetPeriodTimer),
            6 => Some(Self::UpdateWalletEligibility),
            _ => None,
        }
    }
}

/// Facts for planning one PBFT manager period advance command.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvancePeriodFact {
    /// PBFT chain size after the just-finalized block was pushed.
    pub pbft_chain_size: u64,
    /// Existing reset-consensus transition fact for target round one.
    pub transition_fact: PbftManagerTransitionFact,
}

/// Rust-owned advance-period effect plan for the transitional C++ executor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvancePeriodPlan {
    /// Whether C++ may execute the ordered effects.
    pub accepted: bool,
    /// PBFT chain size that was just finalized and should be used for cleanup.
    pub finalized_chain_size: u64,
    /// PBFT period after advancement.
    pub new_period: u64,
    /// Ordered effect script for the C++ executor.
    pub actions: Vec<PbftManagerAdvancePeriodAction>,
    /// Stable error code, empty on success.
    pub error_code: String,
}

/// Executor report for one Rust-planned PBFT period-advance action.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvancePeriodActionReport {
    /// Zero-based action position reported by the compatibility executor.
    pub action_index: u64,
    /// Stable action code that C++ claims to have executed.
    pub action: u8,
    /// Whether the compatibility executor completed the action successfully.
    pub succeeded: bool,
}

/// Stable validation status for one PBFT period-advance executor report.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerAdvancePeriodActionReportStatus {
    /// The report matches the accepted Rust plan and may advance the executor cursor.
    Accepted,
    /// The report references a plan Rust already rejected.
    PlanRejected,
    /// The reported action code is not part of the Rust action enum.
    UnknownAction,
    /// The reported action index is outside the planned script.
    ActionIndexOutOfRange,
    /// The reported action does not match the planned action at that index.
    ActionMismatch,
    /// The compatibility executor reported action failure.
    ExecutorRejected,
}

impl PbftManagerAdvancePeriodActionReportStatus {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Accepted => 0,
            Self::PlanRejected => 1,
            Self::UnknownAction => 2,
            Self::ActionIndexOutOfRange => 3,
            Self::ActionMismatch => 4,
            Self::ExecutorRejected => 5,
        }
    }
}

/// Result of validating one PBFT period-advance executor report.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvancePeriodActionReportResult {
    /// Whether the action report is accepted by the Rust-owned script cursor.
    pub accepted: bool,
    /// Stable validation status for C++ logging and tests.
    pub status: PbftManagerAdvancePeriodActionReportStatus,
    /// Stable error code, empty on success.
    pub error_code: String,
}

impl PbftManagerRuntime {
    /// Creates a runtime from an already accepted startup snapshot.
    pub fn new(snapshot: PbftManagerRuntimeSnapshot) -> Self {
        Self {
            snapshot,
            cached_anchor_dag_order_hashes: BTreeSet::new(),
            policy: PbftManagerLifecyclePolicy::default(),
            last_committed_reset: None,
        }
    }

    /// Returns the current Rust-owned scalar snapshot.
    pub fn snapshot(&self) -> PbftManagerRuntimeSnapshot {
        self.snapshot.clone()
    }

    #[cfg(test)]
    /// Sets the scalar period for cross-module application-boundary fixtures.
    pub(crate) fn set_period_for_test(&mut self, period: u64) {
        self.snapshot.period = period;
    }

    /// Plans a lifecycle transition from authoritative runtime cursor fields.
    pub fn plan_lifecycle_transition(
        &self,
        request: PbftManagerLifecycleTransitionRequest,
    ) -> PbftManagerTransitionPlan {
        let next_step = match request.kind {
            PbftManagerTransitionKind::ResetConsensus => 1,
            PbftManagerTransitionKind::DelayCertifyPoll
            | PbftManagerTransitionKind::DelayFinishPoll => self.snapshot.step,
            _ => self.snapshot.step.saturating_add(1),
        };
        let needs_network_step = next_step >= self.policy.max_steps && next_step % 2 == 1;
        if needs_network_step != request.has_network_next_voting_step {
            return reject_transition_plan(
                PbftManagerTransitionStatus::InvalidFact,
                request.kind,
                "PBFT_MANAGER_TRANSITION_NETWORK_STEP_PRESENCE_MISMATCH",
            );
        }
        let cacti_hardfork = request.target_period >= self.policy.cacti_block;
        let target_round_lambda_ms = if request.target_round == 1 {
            u64::from(self.snapshot.dynamic_lambda_ms)
        } else {
            self.policy.cacti_lambda_default_ms
        };
        plan_pbft_manager_transition(PbftManagerTransitionFact {
            kind: request.kind,
            period: request.target_period,
            round: self.snapshot.round,
            step: self.snapshot.step,
            target_round: request.target_round,
            current_round_lambda_ms: self.snapshot.current_round_lambda_ms,
            target_round_lambda_ms,
            default_lambda_ms: self.policy.genesis_lambda_ms,
            max_exponential_lambda_ms: self.policy.max_exponential_lambda_ms,
            max_steps: self.policy.max_steps,
            network_next_voting_step: request.network_next_voting_step,
            deadline_ms: self.policy.deadline_ms,
            polling_interval_ms: self.policy.polling_interval_ms,
            next_step_time_ms: self.snapshot.next_step_time_ms,
            cacti_hardfork,
            has_cert_voted_block: self.snapshot.has_cert_voted_block,
            executed_pbft_block: self.snapshot.executed_pbft_block,
        })
    }

    /// Returns whether Rust currently tracks materialized DAG-order sidecar data for an anchor.
    ///
    /// Inputs:
    /// - `anchor_hash`: PBFT pivot DAG block hash used as the materialized DAG-order cache key.
    ///
    /// Outputs:
    /// - `true` when the C++ compatibility shell has reported a materialized DAG-order sidecar
    ///   for the anchor and has not reported its removal or period-scoped cleanup.
    ///
    /// Invariants and edge behavior:
    /// - Rust owns compact cache-membership metadata only. C++ remains the temporary owner of
    ///   the live `DagBlock` vector sidecar until FinalChain/finalization object materialization
    ///   moves behind Rust-owned effect payloads.
    /// - The zero hash is treated like any other hash so this helper mirrors reported executor
    ///   state exactly; callers should avoid reporting null-anchor DAG-order caches.
    pub fn has_cached_anchor_dag_order(&self, anchor_hash: H256) -> bool {
        self.cached_anchor_dag_order_hashes.contains(&anchor_hash)
    }

    /// Returns the number of Rust-tracked materialized DAG-order anchor sidecars.
    ///
    /// Outputs:
    /// - Count of anchor hashes reported by the C++ compatibility shell and
    ///   still retained in the Rust runtime metadata.
    ///
    /// Invariants and edge behavior:
    /// - This is live metadata only. It is used by finalization report
    ///   validation to prove period-scoped DAG-order cache cleanup completed on
    ///   the Rust runtime as well as the temporary C++ sidecar map.
    pub fn cached_anchor_dag_order_count(&self) -> u64 {
        self.cached_anchor_dag_order_hashes.len() as u64
    }

    /// Records that the compatibility shell materialized DAG-order data for an anchor.
    ///
    /// Inputs:
    /// - `anchor_hash`: anchor whose DAG-order `DagBlock` vector is now available to the C++
    ///   FinalChain/finalization executor.
    ///
    /// Outputs:
    /// - Returns the current runtime snapshot for bridge consistency. Scalar PBFT manager fields
    ///   are unchanged.
    ///
    /// Invariants and edge behavior:
    /// - This is live runtime metadata, not durable storage.
    /// - Re-recording the same anchor is idempotent.
    pub fn record_cached_anchor_dag_order(
        &mut self,
        anchor_hash: H256,
    ) -> PbftManagerRuntimeSnapshot {
        self.cached_anchor_dag_order_hashes.insert(anchor_hash);
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.error_code.clear();
        self.snapshot.clone()
    }

    /// Removes Rust-owned DAG-order cache metadata for one anchor.
    ///
    /// Inputs:
    /// - `anchor_hash`: anchor whose materialized C++ DAG-order sidecar was erased or rejected.
    ///
    /// Outputs:
    /// - Returns the current runtime snapshot for bridge consistency. Scalar PBFT manager fields
    ///   are unchanged.
    ///
    /// Invariants and edge behavior:
    /// - Removing a missing anchor is idempotent.
    pub fn remove_cached_anchor_dag_order(
        &mut self,
        anchor_hash: H256,
    ) -> PbftManagerRuntimeSnapshot {
        self.cached_anchor_dag_order_hashes.remove(&anchor_hash);
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.error_code.clear();
        self.snapshot.clone()
    }

    /// Clears all Rust-owned DAG-order cache metadata.
    ///
    /// Outputs:
    /// - Returns the current runtime snapshot for bridge consistency. Scalar PBFT manager fields
    ///   are unchanged.
    ///
    /// Invariants and edge behavior:
    /// - C++ calls this when the period-scoped materialized DAG-order sidecar cache is cleared
    ///   after finalization cleanup.
    pub fn clear_cached_anchor_dag_order(&mut self) -> PbftManagerRuntimeSnapshot {
        self.cached_anchor_dag_order_hashes.clear();
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.error_code.clear();
        self.snapshot.clone()
    }

    /// Advances the Rust-owned scalar cursor after transition storage commits.
    ///
    /// The caller must only invoke this after the corresponding Rust storage
    /// batch has been committed. Rejected plans are ignored so storage failure
    /// cannot move the in-memory Rust cursor ahead of durable state.
    pub fn apply_committed_transition(&mut self, plan: &PbftManagerTransitionPlan) {
        if plan.status != PbftManagerTransitionStatus::Ready {
            return;
        }

        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.state = plan.new_state;
        self.snapshot.round = plan.new_round;
        self.snapshot.step = plan.new_step;
        self.snapshot.current_round_lambda_ms = plan.current_round_lambda_ms;
        self.snapshot.next_step_time_ms = plan.next_step_time_ms;
        self.snapshot.persist_normalized_step = false;
        self.snapshot.reset_second_finish_start = plan.reset_second_finish_start;
        self.snapshot.error_code.clear();
        if plan.reset_next_voted_statuses {
            self.snapshot.already_next_voted_value = false;
            self.snapshot.already_next_voted_null = false;
        }
        if plan.reset_broadcast_counters {
            self.snapshot.broadcast_votes_counter = 1;
            self.snapshot.rebroadcast_votes_counter = 1;
        }
        if plan.remove_cert_voted_block {
            self.snapshot.has_cert_voted_block = false;
            self.snapshot.cert_voted_block_period = 0;
            self.snapshot.cert_voted_block_round = 0;
            self.snapshot.cert_voted_block_hash = H256::zero();
        }
    }

    /// Records provenance and follow-up flags for a just-committed reset transition.
    pub fn record_committed_reset(&mut self, target_period: u64, plan: &PbftManagerTransitionPlan) {
        self.last_committed_reset = (plan.status == PbftManagerTransitionStatus::Ready
            && plan.kind == PbftManagerTransitionKind::ResetConsensus
            && target_period > self.snapshot.period)
            .then_some(PbftManagerCommittedReset {
                target_period,
                reset_executed_block_follow_up: plan.reset_executed_block_status,
                set_vote_manager_period_round: plan.set_vote_manager_period_round,
                reset_current_round_timer: plan.reset_current_round_start,
            });
    }

    /// Plans post-reset period advancement only from immediately committed reset provenance.
    pub fn plan_advance_period_after_reset(
        &self,
        finalized_chain_size: u64,
    ) -> PbftManagerAdvancePeriodPlan {
        let Some(reset) = self.last_committed_reset else {
            return rejected_advance_period_plan("PBFT_MANAGER_ADVANCE_PERIOD_RESET_NOT_COMMITTED");
        };
        if finalized_chain_size == 0 {
            return rejected_advance_period_plan("PBFT_MANAGER_ADVANCE_PERIOD_EMPTY_CHAIN");
        }
        if reset.target_period != finalized_chain_size.saturating_add(1) {
            return rejected_advance_period_plan(
                "PBFT_MANAGER_ADVANCE_PERIOD_RESET_PERIOD_MISMATCH",
            );
        }
        plan_pbft_manager_advance_period_after_reset(
            finalized_chain_size,
            reset.reset_executed_block_follow_up,
            reset.set_vote_manager_period_round,
            reset.reset_current_round_timer,
        )
    }

    /// Records the delayed executed-block status reset after persistence.
    ///
    /// Reset-consensus plans keep the legacy wait-for-finalization ordering for
    /// the durable `ExecutedBlock` manager status. The bridge calls this only
    /// after that Rust storage write succeeds, so later C++ mirror updates are
    /// sourced from an authoritative Rust runtime snapshot instead of a stale
    /// pre-reset flag.
    pub fn apply_committed_executed_block_reset(&mut self) {
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.executed_pbft_block = false;
        self.snapshot.error_code.clear();
    }

    /// Records the executed-PBFT status selected by an accepted finalization plan.
    ///
    /// Inputs:
    /// - `executed_pbft_status`: final live manager flag from the Rust
    ///   finalization storage-write intent.
    ///
    /// Outputs:
    /// - Returns the updated Rust-owned runtime snapshot for compatibility
    ///   mirror hydration.
    ///
    /// Invariants and edge behavior:
    /// - This does not write storage. The finalization runtime must persist the
    ///   executed-status stage before reporting the `SetExecutedFlag` action.
    /// - C++ must not derive this flag from sidecar state; the accepted Rust
    ///   finalization intent is the source of truth.
    pub fn apply_committed_finalization_executed_status(
        &mut self,
        executed_pbft_status: bool,
    ) -> PbftManagerRuntimeSnapshot {
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.executed_pbft_block = executed_pbft_status;
        self.snapshot.error_code.clear();
        self.snapshot.clone()
    }

    /// Records a committed next-vote status after Rust storage persistence.
    ///
    /// Inputs:
    /// - `status`: stable PBFT manager status id for the next-voted soft value
    ///   or next-voted null-block-hash flag.
    ///
    /// Outputs:
    /// - Updates the matching runtime snapshot flag and clears the restore
    ///   error code.
    ///
    /// Invariants and edge behavior:
    /// - Callers must persist the matching status row before invoking this
    ///   method, so the long-lived runtime never advances ahead of durable
    ///   storage.
    /// - Unsupported status ids are ignored here because
    ///   `apply_next_voted_status_storage` rejects them before the bridge calls
    ///   this method.
    pub fn apply_committed_next_voted_status(&mut self, status: u8) {
        match status {
            PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE => {
                self.snapshot.already_next_voted_value = true;
            }
            PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH => {
                self.snapshot.already_next_voted_null = true;
            }
            _ => return,
        }
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.error_code.clear();
    }

    /// Records a committed PBFT manager cursor field after Rust storage persistence.
    ///
    /// Inputs:
    /// - `field`: stable PBFT manager field id for round or step.
    /// - `value`: durable cursor value that was just written to storage.
    ///
    /// Outputs:
    /// - Updates the matching runtime snapshot field and clears the restore
    ///   error code.
    ///
    /// Invariants and edge behavior:
    /// - Callers must persist the matching field row before invoking this
    ///   method, so the long-lived runtime never advances ahead of durable
    ///   storage.
    /// - Unsupported fields are ignored here because
    ///   `apply_pbft_manager_cursor_field_storage` rejects them before the
    ///   bridge calls this method.
    pub fn apply_committed_cursor_field(&mut self, field: u8, value: u32) {
        match field {
            PBFT_MGR_FIELD_ROUND => self.snapshot.round = u64::from(value),
            PBFT_MGR_FIELD_STEP => self.snapshot.step = u64::from(value),
            _ => return,
        }
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.error_code.clear();
    }

    /// Records a committed dynamic-lambda storage stage in the runtime snapshot.
    ///
    /// Inputs:
    /// - `rounds_count_dynamic_lambda` is the durable post-adjust accumulator
    ///   written by the Rust finalization storage stage.
    /// - `dynamic_lambda_ms` is the durable post-adjust PBFT manager lambda
    ///   written by the same stage.
    ///
    /// Outputs:
    /// - Updates the runtime snapshot so later transition facts and public
    ///   compatibility mirrors source dynamic-lambda state from Rust runtime
    ///   state rather than C++ mirror fields.
    ///
    /// Invariants and edge behavior:
    /// - Callers must invoke this only after the Rust-owned finalization
    ///   storage stage has been accepted, so the runtime snapshot never
    ///   advances ahead of durable dynamic-lambda state.
    /// - Zero dynamic lambda values are ignored because startup restore rejects
    ///   missing Cacti dynamic-lambda state and transition lambda calculations
    ///   require a nonzero round-one lambda.
    pub fn apply_committed_dynamic_lambda(
        &mut self,
        rounds_count_dynamic_lambda: u32,
        dynamic_lambda_ms: u32,
    ) -> PbftManagerRuntimeSnapshot {
        if dynamic_lambda_ms == 0 {
            let mut rejected = self.snapshot.clone();
            rejected.status = PbftManagerStartupRestoreStatus::InvalidFact;
            rejected.error_code = "PBFT_MANAGER_DYNAMIC_LAMBDA_ZERO".to_string();
            return rejected;
        }
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.rounds_count_dynamic_lambda = rounds_count_dynamic_lambda;
        self.snapshot.dynamic_lambda_ms = dynamic_lambda_ms;
        self.snapshot.error_code.clear();
        self.snapshot.clone()
    }

    /// Records committed broadcast counter state in the runtime snapshot.
    ///
    /// Inputs:
    /// - The four counters are the next values produced by the Rust broadcast
    ///   planner/report contract or by Rust-planned compatibility reset effects
    ///   such as force-broadcast and reward-vote counter reset.
    ///
    /// Outputs:
    /// - Updates the runtime snapshot and returns it for C++ compatibility
    ///   mirror hydration.
    ///
    /// Invariants and edge behavior:
    /// - Counters are live runtime state, not durable PBFT manager storage.
    /// - Zero counters are rejected because broadcast planning treats zero as
    ///   malformed input and legacy counters are one-based.
    /// - Rejected updates leave the previous runtime snapshot unchanged and
    ///   return an invalid snapshot with a stable error code.
    pub fn apply_committed_broadcast_counters(
        &mut self,
        broadcast_votes_counter: u32,
        rebroadcast_votes_counter: u32,
        broadcast_reward_votes_counter: u32,
        rebroadcast_reward_votes_counter: u32,
    ) -> PbftManagerRuntimeSnapshot {
        if broadcast_votes_counter == 0
            || rebroadcast_votes_counter == 0
            || broadcast_reward_votes_counter == 0
            || rebroadcast_reward_votes_counter == 0
        {
            let mut rejected = self.snapshot.clone();
            rejected.status = PbftManagerStartupRestoreStatus::InvalidFact;
            rejected.error_code = "PBFT_MANAGER_BROADCAST_COUNTER_ZERO".to_string();
            return rejected;
        }
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.broadcast_votes_counter = broadcast_votes_counter;
        self.snapshot.rebroadcast_votes_counter = rebroadcast_votes_counter;
        self.snapshot.broadcast_reward_votes_counter = broadcast_reward_votes_counter;
        self.snapshot.rebroadcast_reward_votes_counter = rebroadcast_reward_votes_counter;
        self.snapshot.error_code.clear();
        self.snapshot.clone()
    }

    /// Records committed cert-voted block metadata in the runtime snapshot.
    ///
    /// Inputs:
    /// - `period` and `round` identify the PBFT manager cursor that produced
    ///   the cert vote.
    /// - `block_hash` identifies the live compatibility block sidecar C++ may
    ///   still materialize for vote placement and proposed-block APIs.
    ///
    /// Outputs:
    /// - Updates the runtime snapshot and returns it for compatibility mirror
    ///   hydration.
    ///
    /// Invariants and edge behavior:
    /// - The durable cert-voted recovery payload must be written before this
    ///   method is called for newly produced cert votes.
    /// - The runtime owns only compact metadata; C++ remains the temporary
    ///   owner of `PbftBlock` materialization until proposed-block sidecars
    ///   move to Rust.
    /// - Zero period or round values are rejected and leave the runtime
    ///   unchanged.
    pub fn apply_committed_cert_voted_block(
        &mut self,
        period: u64,
        round: u64,
        block_hash: H256,
    ) -> PbftManagerRuntimeSnapshot {
        if period == 0 || round == 0 {
            let mut rejected = self.snapshot.clone();
            rejected.status = PbftManagerStartupRestoreStatus::InvalidFact;
            rejected.error_code = "PBFT_MANAGER_CERT_VOTED_METADATA_INVALID_CURSOR".to_string();
            return rejected;
        }
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.has_cert_voted_block = true;
        self.snapshot.cert_voted_block_period = period;
        self.snapshot.cert_voted_block_round = round;
        self.snapshot.cert_voted_block_hash = block_hash;
        self.snapshot.error_code.clear();
        self.snapshot.clone()
    }

    /// Records a completed Rust-planned period advance.
    ///
    /// Inputs:
    /// - `new_period`: PBFT period produced by the runtime-owned
    ///   `plan_advance_period_after_reset` operation.
    ///
    /// Outputs:
    /// - Updates the Rust-owned runtime period after the C++ executor has
    ///   completed the ordered advance-period effects.
    ///
    /// Invariants and edge behavior:
    /// - `new_period` must be strictly greater than the current runtime period;
    ///   invalid reports leave the snapshot unchanged and return an invalid
    ///   snapshot with a stable error code.
    pub fn apply_committed_period_advance(
        &mut self,
        new_period: u64,
    ) -> PbftManagerRuntimeSnapshot {
        if new_period <= self.snapshot.period {
            let mut rejected = self.snapshot.clone();
            rejected.status = PbftManagerStartupRestoreStatus::InvalidFact;
            rejected.error_code = "PBFT_MANAGER_ADVANCE_PERIOD_NON_INCREASING_PERIOD".to_string();
            return rejected;
        }
        let Some(reset) = self.last_committed_reset else {
            let mut rejected = self.snapshot.clone();
            rejected.status = PbftManagerStartupRestoreStatus::InvalidFact;
            rejected.error_code = "PBFT_MANAGER_ADVANCE_PERIOD_RESET_NOT_COMMITTED".to_string();
            return rejected;
        };
        if reset.target_period != new_period {
            let mut rejected = self.snapshot.clone();
            rejected.status = PbftManagerStartupRestoreStatus::InvalidFact;
            rejected.error_code = "PBFT_MANAGER_ADVANCE_PERIOD_RESET_PERIOD_MISMATCH".to_string();
            return rejected;
        }
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.period = new_period;
        self.snapshot.error_code.clear();
        self.last_committed_reset = None;
        self.snapshot.clone()
    }
}

fn reject_startup_restore(error_code: &str) -> PbftManagerRuntimeSnapshot {
    PbftManagerRuntimeSnapshot {
        status: PbftManagerStartupRestoreStatus::InvalidFact,
        state: PbftManagerRuntimeStateCode::Unknown,
        period: 0,
        round: 0,
        step: 0,
        current_round_lambda_ms: 0,
        next_step_time_ms: 0,
        rounds_count_dynamic_lambda: 0,
        dynamic_lambda_ms: 0,
        executed_pbft_block: false,
        already_next_voted_value: false,
        already_next_voted_null: false,
        broadcast_votes_counter: 0,
        rebroadcast_votes_counter: 0,
        broadcast_reward_votes_counter: 0,
        rebroadcast_reward_votes_counter: 0,
        has_cert_voted_block: false,
        cert_voted_block_period: 0,
        cert_voted_block_round: 0,
        cert_voted_block_hash: H256::zero(),
        persist_normalized_step: false,
        reset_second_finish_start: false,
        error_code: error_code.to_string(),
    }
}

/// Restores the Rust-owned PBFT manager runtime cursor from persisted facts.
///
/// The restored snapshot mirrors legacy startup semantics: missing round/step
/// default to one, steps below four restart in first-finish at step four, even
/// steps restart in first-finish, and odd steps restart in finish-polling. Cacti
/// dynamic lambda is restored from the persisted manager field after at least
/// one Cacti period has finalized; a default `1` value in that case is rejected
/// as corrupted storage to preserve the legacy safety check.
pub fn restore_pbft_manager_runtime(
    fact: PbftManagerStartupRestoreFact,
) -> PbftManagerRuntimeSnapshot {
    if fact.current_period == 0 || fact.persisted_round == 0 || fact.persisted_step == 0 {
        return reject_startup_restore("PBFT_MANAGER_STARTUP_INVALID_CURSOR");
    }
    if fact.genesis_lambda_ms == 0
        || fact.cacti_lambda_max_ms == 0
        || fact.cacti_lambda_default_ms == 0
    {
        return reject_startup_restore("PBFT_MANAGER_STARTUP_INVALID_LAMBDA_CONFIG");
    }

    let chain_size = fact.current_period.saturating_sub(1);
    let dynamic_lambda_ms = if fact.cacti_active_at_chain_size {
        if chain_size >= 1 {
            if fact.persisted_dynamic_lambda_ms == 1 {
                return reject_startup_restore("PBFT_MANAGER_STARTUP_MISSING_DYNAMIC_LAMBDA");
            }
            fact.persisted_dynamic_lambda_ms
        } else {
            fact.cacti_lambda_max_ms
        }
    } else {
        fact.cacti_lambda_max_ms
    };

    let current_round_lambda_ms = if fact.cacti_active_at_chain_size {
        if fact.persisted_round == 1 {
            dynamic_lambda_ms
        } else {
            fact.cacti_lambda_default_ms
        }
    } else {
        fact.genesis_lambda_ms
    };

    let (state, step, persist_normalized_step, reset_second_finish_start) =
        if fact.persisted_round == 1 && fact.persisted_step == 1 {
            (PbftManagerRuntimeStateCode::ValueProposal, 1, false, false)
        } else if fact.persisted_step < 4 {
            (PbftManagerRuntimeStateCode::Finish, 4, true, false)
        } else if fact.persisted_step % 2 == 0 {
            (
                PbftManagerRuntimeStateCode::Finish,
                fact.persisted_step,
                false,
                false,
            )
        } else {
            (
                PbftManagerRuntimeStateCode::FinishPolling,
                fact.persisted_step,
                false,
                true,
            )
        };

    PbftManagerRuntimeSnapshot {
        status: PbftManagerStartupRestoreStatus::Ready,
        state,
        period: fact.current_period,
        round: fact.persisted_round,
        step,
        current_round_lambda_ms: u64::from(current_round_lambda_ms),
        next_step_time_ms: 0,
        rounds_count_dynamic_lambda: if fact.cacti_active_at_chain_size {
            fact.rounds_count_dynamic_lambda
        } else {
            0
        },
        dynamic_lambda_ms,
        executed_pbft_block: fact.executed_pbft_block,
        already_next_voted_value: fact.already_next_voted_value,
        already_next_voted_null: fact.already_next_voted_null,
        broadcast_votes_counter: 1,
        rebroadcast_votes_counter: 1,
        broadcast_reward_votes_counter: 1,
        rebroadcast_reward_votes_counter: 1,
        has_cert_voted_block: false,
        cert_voted_block_period: 0,
        cert_voted_block_round: 0,
        cert_voted_block_hash: H256::zero(),
        persist_normalized_step,
        reset_second_finish_start,
        error_code: String::new(),
    }
}

/// Creates a PBFT manager runtime from `rustaxa-storage` directly.
///
/// Purpose:
/// - Makes `rustaxa-consensus` the owner of PBFT manager startup storage
///   reads and normalization. The bridge may temporarily pass the shared
///   storage handle, but it no longer decides which storage rows form the
///   runtime snapshot.
///
/// Inputs:
/// - `storage` is the native Rust storage module.
/// - `fact` contains only live/config facts that are not stored in PBFT manager
///   columns.
///
/// Outputs:
/// - A Rust-owned PBFT manager runtime seeded from durable storage.
///
/// Invariants and edge behavior:
/// - Missing round/step/lambda fields preserve legacy compatibility defaults.
/// - If persisted step normalization is required, the normalized step is
///   written through `rustaxa-storage` before the returned runtime clears the
///   `persist_normalized_step` flag.
/// - Invalid startup facts return a stable error label and do not fall back to
///   C++ storage behavior.
pub fn create_pbft_manager_runtime_from_storage(
    storage: &Storage,
    fact: PbftManagerStorageStartupFact,
) -> Result<PbftManagerRuntime> {
    let pbft = storage.pbft();
    let mut snapshot = restore_pbft_manager_runtime(PbftManagerStartupRestoreFact {
        current_period: fact.current_period,
        persisted_round: u64::from(pbft.manager_field(PBFT_MGR_FIELD_ROUND)?.unwrap_or(1)),
        persisted_step: u64::from(pbft.manager_field(PBFT_MGR_FIELD_STEP)?.unwrap_or(1)),
        cacti_active_at_chain_size: fact.cacti_active_at_chain_size,
        rounds_count_dynamic_lambda: storage.metadata().rounds_count_dynamic_lambda()?,
        persisted_dynamic_lambda_ms: pbft.manager_field(PBFT_MGR_FIELD_LAMBDA)?.unwrap_or(1),
        genesis_lambda_ms: fact.genesis_lambda_ms,
        cacti_lambda_max_ms: fact.cacti_lambda_max_ms,
        cacti_lambda_default_ms: fact.cacti_lambda_default_ms,
        executed_pbft_block: pbft
            .manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)?
            .unwrap_or(false),
        already_next_voted_value: pbft
            .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE)?
            .unwrap_or(false),
        already_next_voted_null: pbft
            .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH)?
            .unwrap_or(false),
    });

    if snapshot.status != PbftManagerStartupRestoreStatus::Ready {
        return Err(anyhow!(snapshot.error_code.clone()));
    }
    if snapshot.persist_normalized_step {
        pbft.write_manager_field(
            PBFT_MGR_FIELD_STEP,
            u32::try_from(snapshot.step)
                .map_err(|_| anyhow!("PBFT_MANAGER_STARTUP_NORMALIZED_STEP_OVERFLOW"))?,
        )?;
        snapshot.persist_normalized_step = false;
    }

    let mut runtime = PbftManagerRuntime::new(snapshot);
    runtime.policy = PbftManagerLifecyclePolicy {
        cacti_block: fact.cacti_block,
        genesis_lambda_ms: u64::from(fact.genesis_lambda_ms),
        cacti_lambda_default_ms: u64::from(fact.cacti_lambda_default_ms),
        max_exponential_lambda_ms: fact.max_exponential_lambda_ms,
        max_steps: fact.max_steps,
        deadline_ms: fact.deadline_ms,
        polling_interval_ms: fact.polling_interval_ms,
    };
    Ok(runtime)
}

/// Persists one PBFT manager cursor field through Rust-owned storage.
///
/// Inputs:
/// - `storage`: shared Rust storage handle owned by the PBFT manager runtime.
/// - `field`: stable PBFT manager field id for round or step.
/// - `value`: absolute cursor value to persist.
///
/// Outputs:
/// - Writes the field to `pbft_mgr_round_step` and returns success after the
///   durable write completes.
///
/// Invariants and edge behavior:
/// - This is intentionally not a generic manager-field bridge. Dynamic lambda
///   is written by the finalization/dynamic-lambda storage paths that own that
///   state transition.
/// - Unsupported fields return an error without writing storage.
pub fn apply_pbft_manager_cursor_field_storage(
    storage: &Storage,
    field: u8,
    value: u32,
) -> Result<()> {
    match field {
        PBFT_MGR_FIELD_ROUND | PBFT_MGR_FIELD_STEP => {
            storage.pbft().write_manager_field(field, value)
        }
        _ => Err(anyhow!(
            "unsupported PBFT manager cursor field for runtime storage write: {field}"
        )),
    }
}

/// Persists the PBFT manager's latest cert-voted block through Rust storage.
///
/// Inputs:
/// - `storage`: shared Rust storage handle owned by the PBFT manager runtime.
/// - `round`: PBFT round that produced the cert vote.
/// - `block_rlp`: canonical signed PBFT block RLP payload.
///
/// Outputs:
/// - Stores the legacy `[round, block_rlp]` row in
///   `cert_voted_block_in_round` and returns after the write completes.
///
/// Invariants and edge behavior:
/// - Empty block payloads are rejected before storage writes because restart
///   recovery cannot materialize a PBFT block from an empty sidecar.
/// - The row is overwritten on each successful cert vote, matching legacy
///   RocksDB put semantics.
pub fn save_cert_voted_block_in_round_storage(
    storage: &Storage,
    round: u64,
    block_rlp: &[u8],
) -> Result<()> {
    if block_rlp.is_empty() {
        return Err(anyhow!("PBFT_MANAGER_CERT_VOTED_BLOCK_EMPTY_PAYLOAD"));
    }
    storage
        .pbft()
        .write_cert_voted_block_in_round(round, block_rlp)
}

/// Loads one finalized period needed by the PBFT manager startup replay from
/// native Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `period`: finalized PBFT period to replay.
/// - `load_period_lambda`: whether the caller needs the closest persisted
///   dynamic-lambda value for Cacti reward replay.
///
/// Outputs:
/// - `found = false` if the period data row is missing.
/// - Otherwise the canonical period data RLP, finalized DAG hashes decoded from
///   the period data, and optional period lambda.
///
/// Invariants and edge behavior:
/// - The helper only reads storage and derives hashes from canonical stored
///   bytes; C++ may still materialize temporary `PeriodData` objects from the
///   returned RLP while the live replay boundary remains transitional.
/// - Malformed period data is reported as an error so startup does not silently
///   route through legacy `DbStorage` decoding.
pub fn load_pbft_manager_startup_replay_period(
    storage: &Storage,
    period: u64,
    load_period_lambda: bool,
) -> Result<PbftManagerStartupReplayPeriod> {
    let period_data_rlp = storage.period().data_raw(period)?;
    if period_data_rlp.is_empty() {
        return Ok(PbftManagerStartupReplayPeriod {
            found: false,
            period_data_rlp,
            finalized_dag_hashes: Vec::new(),
            period_lambda: None,
        });
    }

    let finalized_dag_hashes = finalized_dag_hashes_from_period_data(&period_data_rlp)
        .with_context(|| format!("PBFT_MANAGER_STARTUP_PERIOD_DATA_DAG_HASHES_INVALID:{period}"))?;
    let period_lambda = if load_period_lambda {
        storage.metadata().period_lambda(period, true)?
    } else {
        None
    };

    Ok(PbftManagerStartupReplayPeriod {
        found: true,
        period_data_rlp,
        finalized_dag_hashes,
        period_lambda,
    })
}

/// Plans the PBFT manager startup replay ranges from explicit live facts.
///
/// The C++ overlay still executes FinalChain replay and transaction-manager
/// hydration, but Rust now owns the range arithmetic and corrupted-fact
/// rejection. This keeps startup replay decisions with the long-lived PBFT
/// manager runtime rather than duplicating them in the compatibility shell.
pub fn plan_pbft_manager_startup_replay_ranges(
    fact: PbftManagerStartupReplayRangeFact,
) -> PbftManagerStartupReplayRangePlan {
    if fact.final_chain_last_block > fact.pbft_chain_size {
        return PbftManagerStartupReplayRangePlan {
            accepted: false,
            has_finalization_range: false,
            finalization_from_period: 0,
            finalization_to_period: 0,
            recent_from_period: 0,
            recent_to_period: 0,
            error_code: "PBFT_MANAGER_STARTUP_REPLAY_FINAL_CHAIN_AHEAD".to_string(),
        };
    }

    if fact.pbft_chain_size == 0 {
        return PbftManagerStartupReplayRangePlan {
            accepted: true,
            has_finalization_range: false,
            finalization_from_period: 0,
            finalization_to_period: 0,
            recent_from_period: 1,
            recent_to_period: 0,
            error_code: String::new(),
        };
    }

    let finalization_from_period = fact.final_chain_last_block.saturating_add(1);
    let has_finalization_range = finalization_from_period <= fact.pbft_chain_size;
    let coverage = fact
        .recently_finalized_factor
        .saturating_mul(fact.delegation_delay);
    let recent_from_period = if fact.pbft_chain_size > coverage {
        fact.pbft_chain_size - coverage
    } else {
        1
    };

    PbftManagerStartupReplayRangePlan {
        accepted: true,
        has_finalization_range,
        finalization_from_period: if has_finalization_range {
            finalization_from_period
        } else {
            0
        },
        finalization_to_period: if has_finalization_range {
            fact.pbft_chain_size
        } else {
            0
        },
        recent_from_period,
        recent_to_period: fact.pbft_chain_size,
        error_code: String::new(),
    }
}

/// Plans the ordered effects for advancing the PBFT manager period.
///
/// C++ remains the executor for timers, counters, wallet eligibility, and
/// logging. Rust owns the action order and period arithmetic; after those
/// external effects are reported, the application service atomically commits
/// vote/proposed-block cleanup and publishes the manager period.
pub fn plan_pbft_manager_advance_period_after_reset(
    pbft_chain_size: u64,
    reset_executed_block_follow_up: bool,
    set_vote_manager_period_round: bool,
    reset_current_round_timer: bool,
) -> PbftManagerAdvancePeriodPlan {
    if pbft_chain_size == 0 {
        return PbftManagerAdvancePeriodPlan {
            accepted: false,
            finalized_chain_size: 0,
            new_period: 0,
            actions: Vec::new(),
            error_code: "PBFT_MANAGER_ADVANCE_PERIOD_EMPTY_CHAIN".to_string(),
        };
    }
    let mut actions = Vec::new();
    if reset_executed_block_follow_up {
        actions.push(PbftManagerAdvancePeriodAction::ApplyExecutedBlockReset);
    }
    if set_vote_manager_period_round {
        actions.push(PbftManagerAdvancePeriodAction::SetVoteManagerPeriodRound);
    }
    if reset_current_round_timer {
        actions.push(PbftManagerAdvancePeriodAction::ResetCurrentRoundTimer);
    }
    actions.push(PbftManagerAdvancePeriodAction::ResetRewardVoteCounters);
    actions.push(PbftManagerAdvancePeriodAction::ResetPeriodTimer);
    actions.push(PbftManagerAdvancePeriodAction::UpdateWalletEligibility);

    let Some(new_period) = pbft_chain_size.checked_add(1) else {
        return rejected_advance_period_plan("PBFT_MANAGER_ADVANCE_PERIOD_OVERFLOW");
    };

    PbftManagerAdvancePeriodPlan {
        accepted: true,
        finalized_chain_size: pbft_chain_size,
        new_period,
        actions,
        error_code: String::new(),
    }
}

fn rejected_advance_period_plan(error_code: &str) -> PbftManagerAdvancePeriodPlan {
    PbftManagerAdvancePeriodPlan {
        accepted: false,
        finalized_chain_size: 0,
        new_period: 0,
        actions: Vec::new(),
        error_code: error_code.to_string(),
    }
}

/// Validates one compatibility-executor report against a Rust period-advance plan.
///
/// Inputs:
/// - `plan`: accepted Rust-owned period-advance script.
/// - `report`: one C++ executor action result with zero-based script position.
///
/// Outputs:
/// - Accepted only when the plan is accepted, the action index is in range, the
///   reported action matches the Rust script at that index, and the executor
///   reports success.
///
/// Invariants and edge behavior:
/// - This helper is side-effect free. It intentionally does not mutate the
///   manager runtime period; callers must apply the final period commit only
///   after every planned action report has been accepted.
/// - Unknown action codes and out-of-order reports are rejected with stable
///   labels so the C++ shell cannot silently skip or reorder non-EVM
///   finalization cleanup effects.
pub fn validate_pbft_manager_advance_period_action_report(
    plan: &PbftManagerAdvancePeriodPlan,
    report: PbftManagerAdvancePeriodActionReport,
) -> PbftManagerAdvancePeriodActionReportResult {
    if !plan.accepted {
        return PbftManagerAdvancePeriodActionReportResult {
            accepted: false,
            status: PbftManagerAdvancePeriodActionReportStatus::PlanRejected,
            error_code: "PBFT_MANAGER_ADVANCE_PERIOD_REPORT_PLAN_REJECTED".to_string(),
        };
    }
    let Some(reported_action) = PbftManagerAdvancePeriodAction::from_u8(report.action) else {
        return PbftManagerAdvancePeriodActionReportResult {
            accepted: false,
            status: PbftManagerAdvancePeriodActionReportStatus::UnknownAction,
            error_code: "PBFT_MANAGER_ADVANCE_PERIOD_REPORT_UNKNOWN_ACTION".to_string(),
        };
    };
    let Ok(action_index) = usize::try_from(report.action_index) else {
        return PbftManagerAdvancePeriodActionReportResult {
            accepted: false,
            status: PbftManagerAdvancePeriodActionReportStatus::ActionIndexOutOfRange,
            error_code: "PBFT_MANAGER_ADVANCE_PERIOD_REPORT_INDEX_OVERFLOW".to_string(),
        };
    };
    let Some(expected_action) = plan.actions.get(action_index) else {
        return PbftManagerAdvancePeriodActionReportResult {
            accepted: false,
            status: PbftManagerAdvancePeriodActionReportStatus::ActionIndexOutOfRange,
            error_code: "PBFT_MANAGER_ADVANCE_PERIOD_REPORT_INDEX_OUT_OF_RANGE".to_string(),
        };
    };
    if *expected_action != reported_action {
        return PbftManagerAdvancePeriodActionReportResult {
            accepted: false,
            status: PbftManagerAdvancePeriodActionReportStatus::ActionMismatch,
            error_code: "PBFT_MANAGER_ADVANCE_PERIOD_REPORT_ACTION_MISMATCH".to_string(),
        };
    }
    if !report.succeeded {
        return PbftManagerAdvancePeriodActionReportResult {
            accepted: false,
            status: PbftManagerAdvancePeriodActionReportStatus::ExecutorRejected,
            error_code: "PBFT_MANAGER_ADVANCE_PERIOD_REPORT_EXECUTOR_REJECTED".to_string(),
        };
    }
    PbftManagerAdvancePeriodActionReportResult {
        accepted: true,
        status: PbftManagerAdvancePeriodActionReportStatus::Accepted,
        error_code: String::new(),
    }
}

fn finalized_dag_hashes_from_period_data(period_data_rlp: &[u8]) -> Result<Vec<H256>> {
    let period_data = rlp::Rlp::new(period_data_rlp);
    let dag_blocks_data = period_data.at(2)?;
    let bundle = FinalizedDagBlockBundleRlp::new(dag_blocks_data.as_raw());
    let mut hashes = Vec::with_capacity(dag_blocks_data.at(2)?.item_count()?);
    for position in 0..dag_blocks_data.at(2)?.item_count()? {
        hashes.push(keccak256(&bundle.canonical_block_rlp(position)?));
    }
    Ok(hashes)
}

/// Persists the delayed executed-block manager-status reset.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
///
/// Outputs:
/// - Writes `PbftMgrStatus::ExecutedBlock = false` through `rustaxa-storage`.
///
/// Invariants and edge behavior:
/// - This owns only the durable status row. Callers must update live/runtime
///   mirrors only after this function returns success.
/// - The post-`waitForPeriodFinalization()` ordering remains owned by the
///   PBFT manager runtime/shim boundary until that executor moves to Rust.
pub fn apply_executed_block_reset_storage(storage: &Storage) -> Result<()> {
    storage
        .pbft()
        .write_manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK, false)
        .context("PBFT_MANAGER_EXECUTED_BLOCK_RESET_WRITE")
}

/// Persists a successful next-vote manager status through Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `status`: PBFT manager status field. Only `NextVotedSoftValue` and
///   `NextVotedNullBlockHash` are accepted.
///
/// Outputs:
/// - Writes the accepted status row as `true`.
///
/// Invariants and edge behavior:
/// - This helper owns only the durable status row. Vote generation, vote
///   gossip, and live C++ mirror flags remain executor-side boundaries until
///   the state-action executor moves to Rust.
/// - Any status outside the next-voted family is rejected so this cannot become
///   a generic PBFT manager status bridge.
pub fn apply_next_voted_status_storage(storage: &Storage, status: u8) -> Result<()> {
    match status {
        PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE | PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH => {
            storage
                .pbft()
                .write_manager_status(status, true)
                .context("PBFT_MANAGER_NEXT_VOTED_STATUS_WRITE")
        }
        _ => Err(anyhow!("PBFT_MANAGER_NEXT_VOTED_STATUS_UNSUPPORTED")),
    }
}

fn transition_storage_applied(applied_writes: u64) -> PbftManagerTransitionStorageResult {
    PbftManagerTransitionStorageResult {
        status: PbftManagerTransitionStorageStatus::Applied,
        applied_writes,
        error_code: String::new(),
    }
}

fn transition_storage_rejected(error_code: &str) -> PbftManagerTransitionStorageResult {
    PbftManagerTransitionStorageResult {
        status: PbftManagerTransitionStorageStatus::Rejected,
        applied_writes: 0,
        error_code: error_code.to_string(),
    }
}

fn to_manager_u32(
    value: u64,
    error_code: &str,
) -> std::result::Result<u32, PbftManagerTransitionStorageResult> {
    u32::try_from(value).map_err(|_| transition_storage_rejected(error_code))
}

fn append_transition_storage_to_batch(
    storage: &Storage,
    batch: &mut StorageWriteBatch,
    plan: &PbftManagerTransitionPlan,
) -> std::result::Result<u64, PbftManagerTransitionStorageResult> {
    let mut applied_writes = 0;
    let pbft = storage.pbft();

    if plan.persist_round {
        let round = to_manager_u32(
            plan.new_round,
            "PBFT_MANAGER_TRANSITION_STORAGE_ROUND_OVERFLOW",
        )?;
        if pbft
            .write_manager_field_in_batch(batch, PBFT_MGR_FIELD_ROUND, round)
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 1;
    }

    if plan.persist_step {
        let step = to_manager_u32(
            plan.new_step,
            "PBFT_MANAGER_TRANSITION_STORAGE_STEP_OVERFLOW",
        )?;
        if pbft
            .write_manager_field_in_batch(batch, PBFT_MGR_FIELD_STEP, step)
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 1;
    }

    if plan.reset_next_voted_statuses {
        if pbft
            .write_manager_status_in_batch(batch, PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH, false)
            .and_then(|_| {
                pbft.write_manager_status_in_batch(
                    batch,
                    PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE,
                    false,
                )
            })
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 2;
    }

    if plan.remove_cert_voted_block {
        if pbft
            .remove_cert_voted_block_in_round_in_batch(batch)
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 1;
    }

    Ok(applied_writes)
}

/// Applies PBFT manager transition persistence in one Rust-owned storage batch.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `plan`: accepted transition plan from Rust PBFT manager planning/runtime.
/// - `own_vote_hashes`: latest-round own-vote keys to delete when
///   `plan.clear_own_votes` is set.
/// - `sync`: RocksDB write-sync setting for the committed batch.
///
/// Outputs:
/// - A storage result with stable status, applied write count, and rejection
///   code.
///
/// Invariants and edge behavior:
/// - This owns the full durable commit for manager cursor/status transitions
///   and latest-round own-vote cleanup.
/// - Callers must advance Rust runtime state and C++ mirrors only after an
///   `Applied` result.
/// - Executed-block reset remains outside this batch to preserve the
///   post-`waitForPeriodFinalization()` ordering until the executor moves to
///   Rust.
pub fn apply_pbft_manager_transition_storage(
    storage: &Storage,
    plan: &PbftManagerTransitionPlan,
    own_vote_hashes: &[H256],
    sync: bool,
) -> Result<PbftManagerTransitionStorageResult> {
    if plan.status != PbftManagerTransitionStatus::Ready {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_PLAN_NOT_READY",
        ));
    }
    if !plan.clear_own_votes && !own_vote_hashes.is_empty() {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_UNEXPECTED_OWN_VOTE_HASHES",
        ));
    }

    let mut batch = storage.create_write_batch();
    let mut applied_writes = match append_transition_storage_to_batch(storage, &mut batch, plan) {
        Ok(applied_writes) => applied_writes,
        Err(result) => return Ok(result),
    };

    if plan.clear_own_votes {
        for hash in own_vote_hashes {
            if storage
                .pbft()
                .remove_own_verified_vote_in_batch(&mut batch, *hash)
                .is_err()
            {
                return Ok(transition_storage_rejected(
                    "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
                ));
            }
        }
        applied_writes += own_vote_hashes.len() as u64;
    }

    if storage.commit_write_batch_with_sync(batch, sync).is_err() {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_COMMIT_FAILURE",
        ));
    }

    Ok(transition_storage_applied(applied_writes))
}

/// C++-originated facts for deciding whether PBFT can advance to a new round.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvanceRoundFact {
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub current_round: u64,
    /// Whether C++/VoteManager found a candidate new round.
    pub has_new_round: bool,
    /// Candidate new round when present.
    pub new_round: u64,
}

/// Side-effect-free plan for PBFT round advancement.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvanceRoundPlan {
    /// Planning status.
    pub status: PbftManagerTransitionStatus,
    /// Whether C++ should apply a reset transition to `target_round`.
    pub should_advance: bool,
    /// Planned target round when `should_advance` is true.
    pub target_round: u64,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

fn ready_state_action_plan(
    primary_intent: PbftManagerStateActionIntent,
    primary_hash: [u8; 32],
    secondary_intent: PbftManagerStateActionIntent,
    secondary_hash: [u8; 32],
    go_finish_state: bool,
    loop_back_finish_state: bool,
) -> PbftManagerStateActionPlan {
    PbftManagerStateActionPlan {
        status: PbftManagerStateActionStatus::Ready,
        primary_intent,
        primary_hash,
        secondary_intent,
        secondary_hash,
        go_finish_state,
        loop_back_finish_state,
        error_code: String::new(),
    }
}

fn reject_state_action_plan(
    status: PbftManagerStateActionStatus,
    error_code: &str,
) -> PbftManagerStateActionPlan {
    PbftManagerStateActionPlan {
        status,
        primary_intent: PbftManagerStateActionIntent::Noop,
        primary_hash: [0; 32],
        secondary_intent: PbftManagerStateActionIntent::Noop,
        secondary_hash: [0; 32],
        go_finish_state: false,
        loop_back_finish_state: false,
        error_code: error_code.to_string(),
    }
}

fn reject_transition_plan(
    status: PbftManagerTransitionStatus,
    kind: PbftManagerTransitionKind,
    error_code: &str,
) -> PbftManagerTransitionPlan {
    PbftManagerTransitionPlan {
        status,
        kind,
        new_state: PbftManagerRuntimeStateCode::Unknown,
        new_round: 0,
        new_step: 0,
        current_round_lambda_ms: 0,
        next_step_time_ms: 0,
        persist_round: false,
        persist_step: false,
        reset_next_voted_statuses: false,
        remove_cert_voted_block: false,
        clear_own_votes: false,
        clear_broadcasted_votes: false,
        reset_broadcast_counters: false,
        reset_executed_block_status: false,
        set_vote_manager_period_round: false,
        reset_current_round_start: false,
        reset_second_finish_start: false,
        print_cert_step_info: false,
        print_second_finish_step_info: false,
        error_code: error_code.to_string(),
    }
}

fn transition_base_plan(
    fact: &PbftManagerTransitionFact,
    new_state: PbftManagerRuntimeStateCode,
    new_round: u64,
    new_step: u64,
    current_round_lambda_ms: u64,
    next_step_time_ms: u64,
) -> PbftManagerTransitionPlan {
    PbftManagerTransitionPlan {
        status: PbftManagerTransitionStatus::Ready,
        kind: fact.kind,
        new_state,
        new_round,
        new_step,
        current_round_lambda_ms,
        next_step_time_ms,
        persist_round: false,
        persist_step: true,
        reset_next_voted_statuses: false,
        remove_cert_voted_block: false,
        clear_own_votes: false,
        clear_broadcasted_votes: false,
        reset_broadcast_counters: false,
        reset_executed_block_status: false,
        set_vote_manager_period_round: false,
        reset_current_round_start: false,
        reset_second_finish_start: false,
        print_cert_step_info: false,
        print_second_finish_step_info: false,
        error_code: String::new(),
    }
}

fn planned_lambda_for_step(fact: &PbftManagerTransitionFact, new_step: u64) -> u64 {
    if new_step >= fact.max_steps && new_step % 2 == 1 {
        let mut lambda = if new_step == fact.max_steps {
            fact.default_lambda_ms
        } else {
            fact.current_round_lambda_ms
        };
        let catch_up_delay = fact.max_steps.saturating_sub(4);
        if fact.network_next_voting_step > new_step
            && fact.network_next_voting_step - new_step >= catch_up_delay
        {
            fact.default_lambda_ms
        } else if lambda < fact.max_exponential_lambda_ms {
            lambda = lambda.saturating_mul(2).min(fact.max_exponential_lambda_ms);
            lambda
        } else {
            lambda
        }
    } else {
        fact.current_round_lambda_ms
    }
}

fn validate_transition_fact(fact: &PbftManagerTransitionFact) -> Option<&'static str> {
    if fact.kind == PbftManagerTransitionKind::Unknown {
        return Some("PBFT_MANAGER_TRANSITION_UNKNOWN_KIND");
    }
    if fact.period == 0 || fact.round == 0 || fact.step == 0 {
        return Some("PBFT_MANAGER_TRANSITION_INVALID_CURSOR");
    }
    if fact.current_round_lambda_ms == 0
        || fact.default_lambda_ms == 0
        || fact.max_exponential_lambda_ms == 0
        || fact.max_steps == 0
    {
        return Some("PBFT_MANAGER_TRANSITION_INVALID_TIMING_FACTS");
    }
    if fact.kind == PbftManagerTransitionKind::ResetConsensus && fact.target_round == 0 {
        return Some("PBFT_MANAGER_TRANSITION_INVALID_TARGET_ROUND");
    }
    None
}

/// Plans one PBFT manager cursor/status transition from explicit protocol facts.
///
/// The plan is side-effect-free. It owns the deterministic state/round/step,
/// lambda, next-step timing, and manager-status reset decisions. C++ consumes
/// the plan as an executor by persisting fields, updating live compatibility
/// state, clearing sidecars, and setting timestamps.
pub fn plan_pbft_manager_transition(fact: PbftManagerTransitionFact) -> PbftManagerTransitionPlan {
    if let Some(error) = validate_transition_fact(&fact) {
        let status = if fact.kind == PbftManagerTransitionKind::Unknown {
            PbftManagerTransitionStatus::InvalidKind
        } else {
            PbftManagerTransitionStatus::InvalidFact
        };
        return reject_transition_plan(status, fact.kind, error);
    }

    match fact.kind {
        PbftManagerTransitionKind::ResetConsensus => {
            let lambda = if fact.cacti_hardfork {
                fact.target_round_lambda_ms
            } else {
                fact.default_lambda_ms
            };
            if lambda == 0 {
                return reject_transition_plan(
                    PbftManagerTransitionStatus::InvalidFact,
                    fact.kind,
                    "PBFT_MANAGER_TRANSITION_INVALID_RESET_LAMBDA",
                );
            }
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::ValueProposal,
                fact.target_round,
                1,
                lambda,
                fact.next_step_time_ms,
            );
            plan.persist_round = true;
            plan.reset_next_voted_statuses = true;
            plan.remove_cert_voted_block = fact.has_cert_voted_block;
            plan.clear_own_votes = true;
            plan.clear_broadcasted_votes = true;
            plan.reset_broadcast_counters = true;
            plan.reset_executed_block_status = fact.executed_pbft_block;
            plan.set_vote_manager_period_round = true;
            plan.reset_current_round_start = true;
            plan
        }
        PbftManagerTransitionKind::ToFilter => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Filter,
                fact.round,
                new_step,
                lambda,
                lambda.saturating_mul(2),
            )
        }
        PbftManagerTransitionKind::ToCertify => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Certify,
                fact.round,
                new_step,
                lambda,
                lambda.saturating_mul(2),
            );
            plan.print_cert_step_info = true;
            plan
        }
        PbftManagerTransitionKind::ToFinish => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Finish,
                fact.round,
                new_step,
                lambda,
                fact.deadline_ms,
            )
        }
        PbftManagerTransitionKind::ToFinishPolling => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::FinishPolling,
                fact.round,
                new_step,
                lambda,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.reset_next_voted_statuses = true;
            plan.reset_second_finish_start = true;
            plan.print_second_finish_step_info = true;
            plan
        }
        PbftManagerTransitionKind::LoopBackFinish => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Finish,
                fact.round,
                new_step,
                lambda,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.reset_next_voted_statuses = true;
            plan
        }
        PbftManagerTransitionKind::DelayCertifyPoll => {
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Certify,
                fact.round,
                fact.step,
                fact.current_round_lambda_ms,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.persist_step = false;
            plan
        }
        PbftManagerTransitionKind::DelayFinishPoll => {
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::FinishPolling,
                fact.round,
                fact.step,
                fact.current_round_lambda_ms,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.persist_step = false;
            plan
        }
        PbftManagerTransitionKind::Unknown => unreachable!("unknown transition rejected above"),
    }
}

/// Plans whether a PBFT manager round-advance candidate should reset consensus.
///
/// C++ still sources the candidate from the VoteManager/verified-votes runtime,
/// but Rust validates the protocol condition that advancement requires a
/// strictly greater round before C++ applies the reset transition.
pub fn plan_pbft_manager_advance_round(
    fact: PbftManagerAdvanceRoundFact,
) -> PbftManagerAdvanceRoundPlan {
    if fact.period == 0 || fact.current_round == 0 {
        return PbftManagerAdvanceRoundPlan {
            status: PbftManagerTransitionStatus::InvalidFact,
            should_advance: false,
            target_round: 0,
            error_code: "PBFT_MANAGER_ADVANCE_ROUND_INVALID_CURSOR".to_string(),
        };
    }
    if !fact.has_new_round {
        return PbftManagerAdvanceRoundPlan {
            status: PbftManagerTransitionStatus::Ready,
            should_advance: false,
            target_round: 0,
            error_code: String::new(),
        };
    }
    if fact.new_round <= fact.current_round {
        return PbftManagerAdvanceRoundPlan {
            status: PbftManagerTransitionStatus::InvalidFact,
            should_advance: false,
            target_round: 0,
            error_code: "PBFT_MANAGER_ADVANCE_ROUND_NON_INCREASING_ROUND".to_string(),
        };
    }
    PbftManagerAdvanceRoundPlan {
        status: PbftManagerTransitionStatus::Ready,
        should_advance: true,
        target_round: fact.new_round,
        error_code: String::new(),
    }
}

/// Plans one PBFT manager state action from explicit protocol facts.
///
/// The plan is side-effect-free. It deliberately does not validate or
/// materialize PBFT blocks, generate votes, write storage, sleep, gossip, or
/// execute FinalChain/EVM logic. Those remain executor responsibilities around
/// the Rust-owned protocol branch decision.
pub fn plan_pbft_manager_state_action(
    fact: PbftManagerStateActionFact,
) -> PbftManagerStateActionPlan {
    if fact.state == PbftManagerRuntimeStateCode::Unknown {
        return reject_state_action_plan(
            PbftManagerStateActionStatus::InvalidState,
            "PBFT_MANAGER_STATE_ACTION_UNKNOWN_STATE",
        );
    }
    if fact.period == 0 || fact.round == 0 || fact.step == 0 {
        return reject_state_action_plan(
            PbftManagerStateActionStatus::InvalidFact,
            "PBFT_MANAGER_STATE_ACTION_INVALID_CURSOR",
        );
    }

    match fact.state {
        PbftManagerRuntimeStateCode::ValueProposal => plan_value_proposal_state_action(&fact),
        PbftManagerRuntimeStateCode::Filter => plan_filter_state_action(&fact),
        PbftManagerRuntimeStateCode::Certify => plan_certify_state_action(&fact),
        PbftManagerRuntimeStateCode::Finish => plan_first_finish_state_action(&fact),
        PbftManagerRuntimeStateCode::FinishPolling => plan_second_finish_state_action(&fact),
        PbftManagerRuntimeStateCode::Unknown => unreachable!("unknown state rejected above"),
    }
}

/// Plans one PBFT manager state action as ordered effects.
///
/// This function is side-effect-free. It preserves the same deterministic
/// decision table as `plan_pbft_manager_state_action`, then converts non-noop
/// primary and secondary intents into ordered effects for a shared C++ executor
/// loop. C++ remains responsible for resolving live block/vote sidecars,
/// generating and placing votes, persisting compatibility state, and gossiping
/// generated votes.
pub fn plan_pbft_manager_state_action_effects(
    fact: PbftManagerStateActionFact,
) -> PbftManagerStateActionEffectPlan {
    let plan = plan_pbft_manager_state_action(fact.clone());
    let mut effects = Vec::with_capacity(2);
    if plan.primary_intent != PbftManagerStateActionIntent::Noop {
        effects.push(pbft_manager_state_action_effect(
            &fact,
            plan.primary_intent,
            plan.primary_hash,
        ));
    }
    if plan.secondary_intent != PbftManagerStateActionIntent::Noop {
        effects.push(pbft_manager_state_action_effect(
            &fact,
            plan.secondary_intent,
            plan.secondary_hash,
        ));
    }

    PbftManagerStateActionEffectPlan {
        status: plan.status,
        effects,
        go_finish_state: plan.go_finish_state,
        loop_back_finish_state: plan.loop_back_finish_state,
        error_code: plan.error_code,
    }
}

fn pbft_manager_state_action_effect(
    fact: &PbftManagerStateActionFact,
    intent: PbftManagerStateActionIntent,
    hash: [u8; 32],
) -> PbftManagerStateActionEffect {
    let request_proposed_block_sidecar =
        pbft_manager_state_action_intent_requires_proposed_block_sidecar(intent);
    PbftManagerStateActionEffect {
        intent,
        hash,
        request_proposed_block_sidecar,
        proposed_block_sidecar_hash: if request_proposed_block_sidecar {
            hash
        } else {
            [0; 32]
        },
        proposed_block_sidecar_period: if request_proposed_block_sidecar {
            fact.period
        } else {
            0
        },
    }
}

fn pbft_manager_state_action_intent_requires_proposed_block_sidecar(
    intent: PbftManagerStateActionIntent,
) -> bool {
    matches!(
        intent,
        PbftManagerStateActionIntent::ReproposePreviousRoundNextValue
            | PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue
            | PbftManagerStateActionIntent::CertVoteCurrentSoftValue
            | PbftManagerStateActionIntent::NextVoteCertVotedBlock
            | PbftManagerStateActionIntent::NextVotePreviousRoundValue
            | PbftManagerStateActionIntent::NextVoteCurrentSoftValue
    )
}

fn state_action_session_step(
    status: PbftManagerStateActionSessionStatus,
    cursor: usize,
    effect: Option<PbftManagerStateActionEffect>,
    plan: &PbftManagerStateActionEffectPlan,
    error_code: String,
) -> PbftManagerStateActionSessionStep {
    PbftManagerStateActionSessionStep {
        status,
        cursor: u32::try_from(cursor).unwrap_or(u32::MAX),
        has_effect: effect.is_some(),
        effect: effect.unwrap_or(PbftManagerStateActionEffect {
            intent: PbftManagerStateActionIntent::Noop,
            hash: [0; 32],
            request_proposed_block_sidecar: false,
            proposed_block_sidecar_hash: [0; 32],
            proposed_block_sidecar_period: 0,
        }),
        go_finish_state: plan.go_finish_state,
        loop_back_finish_state: plan.loop_back_finish_state,
        complete: status != PbftManagerStateActionSessionStatus::Active,
        can_continue: matches!(
            status,
            PbftManagerStateActionSessionStatus::Active
                | PbftManagerStateActionSessionStatus::Complete
        ),
        error_code,
    }
}

/// Creates a Rust-owned state-action effect session from compact C++ facts.
///
/// The session owns the ordered effect cursor. Rejected fact bundles produce a
/// terminal session whose first `next` call returns `RejectedFact`.
pub fn create_pbft_manager_state_action_effect_session(
    fact: PbftManagerStateActionFact,
) -> PbftManagerStateActionEffectSession {
    let plan = plan_pbft_manager_state_action_effects(fact);
    let status = if plan.status == PbftManagerStateActionStatus::Ready {
        PbftManagerStateActionSessionStatus::Active
    } else {
        PbftManagerStateActionSessionStatus::RejectedFact
    };
    PbftManagerStateActionEffectSession {
        plan,
        cursor: 0,
        status,
        pending: None,
    }
}

/// Returns the next state-action effect requested by Rust.
///
/// Edge behavior:
/// - A no-op plan completes immediately.
/// - Calling `next` while an effect is pending returns the same pending effect
///   until C++ reports it.
/// - Rejected or executor-failed sessions return terminal steps.
pub fn next_pbft_manager_state_action_effect_session(
    session: &mut PbftManagerStateActionEffectSession,
) -> PbftManagerStateActionSessionStep {
    if session.status != PbftManagerStateActionSessionStatus::Active {
        return state_action_session_step(
            session.status,
            session.cursor,
            None,
            &session.plan,
            session.plan.error_code.clone(),
        );
    }
    if let Some(effect) = session.pending.clone() {
        return state_action_session_step(
            PbftManagerStateActionSessionStatus::Active,
            session.cursor,
            Some(effect),
            &session.plan,
            String::new(),
        );
    }
    if session.cursor >= session.plan.effects.len() {
        session.status = PbftManagerStateActionSessionStatus::Complete;
        return state_action_session_step(
            session.status,
            session.cursor,
            None,
            &session.plan,
            String::new(),
        );
    }

    let effect = session.plan.effects[session.cursor].clone();
    session.pending = Some(effect.clone());
    state_action_session_step(
        PbftManagerStateActionSessionStatus::Active,
        session.cursor,
        Some(effect),
        &session.plan,
        String::new(),
    )
}

/// Reports one C++-executed state-action effect and advances the Rust cursor.
///
/// Rust validates that the report matches the pending cursor and intent before
/// accepting it. Executor rejection is terminal; successful reports advance to
/// the next effect or complete the session.
pub fn report_pbft_manager_state_action_effect_session(
    session: &mut PbftManagerStateActionEffectSession,
    report: PbftManagerStateActionEffectReport,
) -> PbftManagerStateActionSessionStep {
    if session.status != PbftManagerStateActionSessionStatus::Active {
        return state_action_session_step(
            session.status,
            session.cursor,
            None,
            &session.plan,
            session.plan.error_code.clone(),
        );
    }
    let Some(pending) = session.pending.clone() else {
        session.status = PbftManagerStateActionSessionStatus::EffectMismatch;
        return state_action_session_step(
            session.status,
            session.cursor,
            None,
            &session.plan,
            "PBFT_MANAGER_STATE_ACTION_EFFECT_REPORT_WITHOUT_PENDING_EFFECT".to_string(),
        );
    };
    if report.cursor as usize != session.cursor || report.intent != pending.intent {
        session.status = PbftManagerStateActionSessionStatus::EffectMismatch;
        return state_action_session_step(
            session.status,
            session.cursor,
            None,
            &session.plan,
            "PBFT_MANAGER_STATE_ACTION_EFFECT_REPORT_MISMATCH".to_string(),
        );
    }
    if report.result == PbftManagerStateActionEffectResultCode::Unknown {
        session.status = PbftManagerStateActionSessionStatus::InvalidReport;
        return state_action_session_step(
            session.status,
            session.cursor,
            None,
            &session.plan,
            "PBFT_MANAGER_STATE_ACTION_EFFECT_UNKNOWN_RESULT".to_string(),
        );
    }
    session.pending = None;
    if report.result == PbftManagerStateActionEffectResultCode::ExecutorError {
        session.status = PbftManagerStateActionSessionStatus::ContractError;
        return state_action_session_step(
            session.status,
            session.cursor,
            None,
            &session.plan,
            if report.error_code.is_empty() {
                "PBFT_MANAGER_STATE_ACTION_EFFECT_EXECUTOR_ERROR".to_string()
            } else {
                report.error_code
            },
        );
    }
    if matches!(
        report.result,
        PbftManagerStateActionEffectResultCode::SkippedMissingLiveObject
            | PbftManagerStateActionEffectResultCode::RejectedLiveCheck
    ) {
        session.status = PbftManagerStateActionSessionStatus::EffectFailed;
        return state_action_session_step(
            session.status,
            session.cursor,
            None,
            &session.plan,
            if report.error_code.is_empty() {
                "PBFT_MANAGER_STATE_ACTION_EFFECT_FAILED".to_string()
            } else {
                report.error_code
            },
        );
    }
    session.cursor += 1;
    next_pbft_manager_state_action_effect_session(session)
}

fn previous_round_starts_from_null(fact: &PbftManagerStateActionFact) -> bool {
    fact.round == 1 || fact.has_previous_round_next_null
}

fn plan_value_proposal_state_action(
    fact: &PbftManagerStateActionFact,
) -> PbftManagerStateActionPlan {
    if previous_round_starts_from_null(fact) {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::ProposeNewBlock,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_previous_round_next_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::ReproposePreviousRoundNextValue,
            fact.previous_round_next_value_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    reject_state_action_plan(
        PbftManagerStateActionStatus::InvalidFact,
        "PBFT_MANAGER_VALUE_PROPOSAL_MISSING_PREVIOUS_ROUND_STARTING_VALUE",
    )
}

fn plan_filter_state_action(fact: &PbftManagerStateActionFact) -> PbftManagerStateActionPlan {
    if previous_round_starts_from_null(fact) {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::IdentifyLeaderAndSoftVote,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_previous_round_next_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue,
            fact.previous_round_next_value_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    ready_state_action_plan(
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        false,
        false,
    )
}

fn plan_certify_state_action(fact: &PbftManagerStateActionFact) -> PbftManagerStateActionPlan {
    let finish_deadline_ms = fact.deadline_ms.saturating_sub(fact.polling_interval_ms);
    let go_finish_state = fact.elapsed_round_ms > finish_deadline_ms;
    if go_finish_state {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::GoFinish,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            true,
            false,
        );
    }

    if fact.elapsed_round_ms < fact.current_round_lambda_ms.saturating_mul(2) {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_cert_voted_block || !fact.has_current_round_soft_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    ready_state_action_plan(
        PbftManagerStateActionIntent::CertVoteCurrentSoftValue,
        fact.current_round_soft_value_hash,
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        false,
        false,
    )
}

fn plan_first_finish_state_action(fact: &PbftManagerStateActionFact) -> PbftManagerStateActionPlan {
    if fact.has_cert_voted_block {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::NextVoteCertVotedBlock,
            fact.cert_voted_block_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.round >= 2 && fact.has_previous_round_next_null {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::NextVoteNullBlock,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_previous_round_next_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::NextVotePreviousRoundValue,
            fact.previous_round_next_value_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    ready_state_action_plan(
        PbftManagerStateActionIntent::NextVoteNullBlock,
        [0; 32],
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        false,
        false,
    )
}

fn plan_second_finish_state_action(
    fact: &PbftManagerStateActionFact,
) -> PbftManagerStateActionPlan {
    let primary = if !fact.already_next_voted_value && fact.has_current_round_soft_value {
        PbftManagerStateActionIntent::NextVoteCurrentSoftValue
    } else {
        PbftManagerStateActionIntent::Noop
    };
    let primary_hash = if primary == PbftManagerStateActionIntent::NextVoteCurrentSoftValue {
        fact.current_round_soft_value_hash
    } else {
        [0; 32]
    };

    let secondary = if !fact.has_cert_voted_block
        && !fact.already_next_voted_null
        && fact.round >= 2
        && fact.has_previous_round_next_null
    {
        PbftManagerStateActionIntent::NextVoteNullBlock
    } else {
        PbftManagerStateActionIntent::Noop
    };

    let loop_back_finish_state = fact.elapsed_round_ms
        > fact
            .current_round_lambda_ms
            .saturating_sub(fact.polling_interval_ms)
            .saturating_mul(2);

    ready_state_action_plan(
        primary,
        primary_hash,
        secondary,
        [0; 32],
        false,
        loop_back_finish_state,
    )
}

/// One C++ action report for the Rust-owned PBFT manager runtime cursor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeActionReport {
    /// Cursor returned by the previous session step.
    pub cursor: u32,
    /// Action that C++ executed.
    pub action: PbftManagerRuntimeAction,
    /// Whether the action call itself succeeded.
    pub success: bool,
    /// Stable action result code.
    pub result: PbftManagerRuntimeActionResultCode,
    /// `go_finish_state_` observed after `RunCertify`.
    pub go_finish_state: bool,
    /// `loop_back_finish_state_` observed after `RunSecondFinish`.
    pub loop_back_finish_state: bool,
    /// Current eligible-wallet state after the reported action.
    pub has_eligible_wallet: bool,
    /// Whether C++/VoteManager found a candidate new round for
    /// `TryAdvanceRound`.
    pub has_new_round: bool,
    /// Candidate new round reported for `TryAdvanceRound`, when present.
    pub new_round: u64,
    /// Optional error detail from the C++ executor.
    pub error_code: String,
}

/// One Rust-owned session step for C++ execution.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeSessionStep {
    /// Session status.
    pub status: PbftManagerRuntimeStatus,
    /// Cursor for the returned action.
    pub cursor: u32,
    /// Action to execute, if any.
    pub action: Option<PbftManagerRuntimeAction>,
    /// Whether `action` is valid.
    pub has_action: bool,
    /// Whether the session completed all actions.
    pub complete: bool,
    /// Whether C++ should restart the daemon loop immediately.
    pub restart_loop: bool,
    /// Whether this step carries a target round for a reset-consensus effect.
    pub has_target_round: bool,
    /// Target round for `ResetConsensus` when `has_target_round` is true.
    pub target_round: u64,
    /// Rust-planned sleep duration for sleep actions.
    pub sleep_ms: u64,
    /// Caller-local tick id.
    pub tick_id: u64,
    /// Stable error detail.
    pub error_code: String,
}

/// Stateful Rust cursor for one PBFT manager daemon tick.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeSession {
    /// Immutable tick facts.
    pub fact: PbftManagerRuntimeTickFact,
    /// Current session status.
    pub status: PbftManagerRuntimeStatus,
    /// Pending actions that have not yet been handed to C++.
    pub pending: VecDeque<PbftManagerRuntimeAction>,
    /// Cursor of the next action.
    pub cursor: u32,
    /// Completed restart-loop signal.
    pub restart_loop: bool,
    /// Target round attached to a pending `ResetConsensus` action.
    pub reset_target_round: Option<u64>,
    /// Stable error detail.
    pub error_code: String,
}

fn reject_session(fact: PbftManagerRuntimeTickFact, error_code: &str) -> PbftManagerRuntimeSession {
    PbftManagerRuntimeSession {
        fact,
        status: PbftManagerRuntimeStatus::RejectedTick,
        pending: VecDeque::new(),
        cursor: 0,
        restart_loop: false,
        reset_target_round: None,
        error_code: error_code.to_string(),
    }
}

fn append_state_script(
    actions: &mut VecDeque<PbftManagerRuntimeAction>,
    state: PbftManagerRuntimeStateCode,
) {
    match state {
        PbftManagerRuntimeStateCode::ValueProposal => {
            actions.push_back(PbftManagerRuntimeAction::RunValueProposal);
            actions.push_back(PbftManagerRuntimeAction::TransitionToFilter);
            actions.push_back(PbftManagerRuntimeAction::SleepUntilNextStep);
        }
        PbftManagerRuntimeStateCode::Filter => {
            actions.push_back(PbftManagerRuntimeAction::RunFilter);
            actions.push_back(PbftManagerRuntimeAction::TransitionToCertify);
            actions.push_back(PbftManagerRuntimeAction::SleepUntilNextStep);
        }
        PbftManagerRuntimeStateCode::Certify => {
            actions.push_back(PbftManagerRuntimeAction::RunCertify);
        }
        PbftManagerRuntimeStateCode::Finish => {
            actions.push_back(PbftManagerRuntimeAction::RunFirstFinish);
            actions.push_back(PbftManagerRuntimeAction::TransitionToFinishPolling);
            actions.push_back(PbftManagerRuntimeAction::SleepUntilNextStep);
        }
        PbftManagerRuntimeStateCode::FinishPolling => {
            actions.push_back(PbftManagerRuntimeAction::RunSecondFinish);
        }
        PbftManagerRuntimeStateCode::Unknown => {}
    }
}

/// Creates a Rust-owned PBFT manager runtime session for one daemon tick.
pub fn create_pbft_manager_runtime_session(
    fact: PbftManagerRuntimeTickFact,
) -> PbftManagerRuntimeSession {
    if fact.state == PbftManagerRuntimeStateCode::Unknown {
        return reject_session(fact, "PBFT_MANAGER_RUNTIME_UNKNOWN_STATE");
    }

    if fact.period == 0 || fact.round == 0 || fact.step == 0 {
        return reject_session(fact, "PBFT_MANAGER_RUNTIME_INVALID_CURSOR");
    }

    let mut pending = VecDeque::new();
    pending.push_back(PbftManagerRuntimeAction::ProcessSyncedPbftBlocks);
    if fact.network_available && !fact.network_pbft_syncing {
        pending.push_back(PbftManagerRuntimeAction::MaybeBroadcastVotes);
        pending.push_back(PbftManagerRuntimeAction::TryPushCertVotesBlock);
    }
    pending.push_back(PbftManagerRuntimeAction::TryAdvanceRound);

    PbftManagerRuntimeSession {
        fact,
        status: PbftManagerRuntimeStatus::Active,
        pending,
        cursor: 0,
        restart_loop: false,
        reset_target_round: None,
        error_code: String::new(),
    }
}

/// Returns the next action for a PBFT manager runtime session.
pub fn next_pbft_manager_runtime_action(
    session: &PbftManagerRuntimeSession,
) -> PbftManagerRuntimeSessionStep {
    if session.status != PbftManagerRuntimeStatus::Active {
        return PbftManagerRuntimeSessionStep {
            status: session.status,
            cursor: session.cursor,
            action: None,
            has_action: false,
            complete: session.status == PbftManagerRuntimeStatus::Complete,
            restart_loop: session.restart_loop,
            has_target_round: false,
            target_round: 0,
            sleep_ms: 0,
            tick_id: session.fact.tick_id,
            error_code: session.error_code.clone(),
        };
    }

    match session.pending.front().copied() {
        Some(action) => {
            let target_round = if action == PbftManagerRuntimeAction::ResetConsensus {
                session.reset_target_round.unwrap_or(0)
            } else {
                0
            };
            PbftManagerRuntimeSessionStep {
                status: PbftManagerRuntimeStatus::Active,
                cursor: session.cursor,
                action: Some(action),
                has_action: true,
                complete: false,
                restart_loop: false,
                has_target_round: action == PbftManagerRuntimeAction::ResetConsensus,
                target_round,
                sleep_ms: if action == PbftManagerRuntimeAction::SleepIneligiblePollingInterval {
                    session.fact.polling_interval_ms
                } else {
                    0
                },
                tick_id: session.fact.tick_id,
                error_code: String::new(),
            }
        }
        None => PbftManagerRuntimeSessionStep {
            status: PbftManagerRuntimeStatus::Complete,
            cursor: session.cursor,
            action: None,
            has_action: false,
            complete: true,
            restart_loop: session.restart_loop,
            has_target_round: false,
            target_round: 0,
            sleep_ms: 0,
            tick_id: session.fact.tick_id,
            error_code: String::new(),
        },
    }
}

fn fail_session(
    mut session: PbftManagerRuntimeSession,
    status: PbftManagerRuntimeStatus,
    error_code: String,
) -> PbftManagerRuntimeSession {
    session.status = status;
    session.pending.clear();
    session.reset_target_round = None;
    session.error_code = error_code;
    session
}

fn report_error(report: &PbftManagerRuntimeActionReport, fallback: &str) -> String {
    if report.error_code.is_empty() {
        fallback.to_string()
    } else {
        report.error_code.clone()
    }
}

fn valid_action_result(
    action: PbftManagerRuntimeAction,
    result: PbftManagerRuntimeActionResultCode,
) -> bool {
    match action {
        PbftManagerRuntimeAction::TryPushCertVotesBlock => matches!(
            result,
            PbftManagerRuntimeActionResultCode::NoProgressContinue
                | PbftManagerRuntimeActionResultCode::ProgressRestartLoop
        ),
        PbftManagerRuntimeAction::TryAdvanceRound => {
            result == PbftManagerRuntimeActionResultCode::NoProgressContinue
        }
        PbftManagerRuntimeAction::ResetConsensus => {
            result == PbftManagerRuntimeActionResultCode::TransitionApplied
        }
        PbftManagerRuntimeAction::TransitionToFilter
        | PbftManagerRuntimeAction::TransitionToCertify
        | PbftManagerRuntimeAction::TransitionToFinish
        | PbftManagerRuntimeAction::TransitionToFinishPolling
        | PbftManagerRuntimeAction::LoopBackFinish => {
            result == PbftManagerRuntimeActionResultCode::TransitionApplied
        }
        PbftManagerRuntimeAction::SleepIneligiblePollingInterval
        | PbftManagerRuntimeAction::DelayCertifyPoll
        | PbftManagerRuntimeAction::DelayFinishPoll
        | PbftManagerRuntimeAction::SleepUntilNextStep => {
            result == PbftManagerRuntimeActionResultCode::SleepApplied
        }
        PbftManagerRuntimeAction::ProcessSyncedPbftBlocks
        | PbftManagerRuntimeAction::MaybeBroadcastVotes
        | PbftManagerRuntimeAction::RunValueProposal
        | PbftManagerRuntimeAction::RunFilter
        | PbftManagerRuntimeAction::RunCertify
        | PbftManagerRuntimeAction::RunFirstFinish
        | PbftManagerRuntimeAction::RunSecondFinish => {
            result == PbftManagerRuntimeActionResultCode::StateActionDone
        }
        PbftManagerRuntimeAction::Unknown => false,
    }
}

/// Reports a C++-executed manager action and advances the Rust cursor.
pub fn report_pbft_manager_runtime_action(
    mut session: PbftManagerRuntimeSession,
    report: PbftManagerRuntimeActionReport,
) -> PbftManagerRuntimeSession {
    if session.status != PbftManagerRuntimeStatus::Active {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ContractError,
            "PBFT_MANAGER_RUNTIME_NOT_ACTIVE".to_string(),
        );
    }

    if report.cursor != session.cursor {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ActionMismatch,
            "PBFT_MANAGER_RUNTIME_CURSOR_MISMATCH".to_string(),
        );
    }

    let Some(expected_action) = session.pending.pop_front() else {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ContractError,
            "PBFT_MANAGER_RUNTIME_MISSING_ACTION".to_string(),
        );
    };

    if report.action != expected_action {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ActionMismatch,
            "PBFT_MANAGER_RUNTIME_ACTION_MISMATCH".to_string(),
        );
    }

    if !report.success || report.result == PbftManagerRuntimeActionResultCode::ExecutorError {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ActionFailed,
            report_error(&report, "PBFT_MANAGER_RUNTIME_ACTION_FAILED"),
        );
    }

    if !valid_action_result(expected_action, report.result) {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::InvalidReport,
            "PBFT_MANAGER_RUNTIME_RESULT_MISMATCH".to_string(),
        );
    }

    match expected_action {
        PbftManagerRuntimeAction::TryPushCertVotesBlock => {
            if report.result == PbftManagerRuntimeActionResultCode::ProgressRestartLoop {
                session.status = PbftManagerRuntimeStatus::Complete;
                session.pending.clear();
                session.restart_loop = true;
                session.cursor = session.cursor.saturating_add(1);
                return session;
            }
        }
        PbftManagerRuntimeAction::TryAdvanceRound => {
            let advance_plan = plan_pbft_manager_advance_round(PbftManagerAdvanceRoundFact {
                period: session.fact.period,
                current_round: session.fact.round,
                has_new_round: report.has_new_round,
                new_round: report.new_round,
            });
            if advance_plan.status != PbftManagerTransitionStatus::Ready {
                return fail_session(
                    session,
                    PbftManagerRuntimeStatus::InvalidReport,
                    advance_plan.error_code,
                );
            }
            if advance_plan.should_advance {
                session.reset_target_round = Some(advance_plan.target_round);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::ResetConsensus);
            } else if report.has_eligible_wallet {
                append_state_script(&mut session.pending, session.fact.state);
            } else {
                session
                    .pending
                    .push_back(PbftManagerRuntimeAction::SleepIneligiblePollingInterval);
            }
        }
        PbftManagerRuntimeAction::ResetConsensus => {
            session.status = PbftManagerRuntimeStatus::Complete;
            session.pending.clear();
            session.reset_target_round = None;
            session.restart_loop = true;
            session.cursor = session.cursor.saturating_add(1);
            return session;
        }
        PbftManagerRuntimeAction::SleepIneligiblePollingInterval => {
            if report.result != PbftManagerRuntimeActionResultCode::SleepApplied {
                return fail_session(
                    session,
                    PbftManagerRuntimeStatus::InvalidReport,
                    "PBFT_MANAGER_RUNTIME_INELIGIBLE_SLEEP_REPORT_MISMATCH".to_string(),
                );
            }
            session.status = PbftManagerRuntimeStatus::Complete;
            session.pending.clear();
            session.restart_loop = true;
            session.cursor = session.cursor.saturating_add(1);
            return session;
        }
        PbftManagerRuntimeAction::RunCertify => {
            if report.go_finish_state {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::TransitionToFinish);
            } else {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::DelayCertifyPoll);
            }
        }
        PbftManagerRuntimeAction::RunSecondFinish => {
            if report.loop_back_finish_state {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::LoopBackFinish);
            } else {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::DelayFinishPoll);
            }
        }
        _ => {}
    }

    session.cursor = session.cursor.saturating_add(1);
    if session.pending.is_empty() {
        session.status = PbftManagerRuntimeStatus::Complete;
    }
    session
}

/// Marks a PBFT manager runtime session as aborted.
pub fn abort_pbft_manager_runtime_session(
    mut session: PbftManagerRuntimeSession,
) -> PbftManagerRuntimeSession {
    session.status = PbftManagerRuntimeStatus::ContractError;
    session.pending.clear();
    session.restart_loop = false;
    session.reset_target_round = None;
    session.error_code = "PBFT_MANAGER_RUNTIME_ABORTED".to_string();
    session
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::{DagManagerBlock, save_dag_block_to_storage};
    use crate::dag_service::DagServiceConfig;
    use crate::dag_transaction_service::{
        DagTransactionService, DagTransactionServiceConfig,
        DagTransactionSortitionFinalizationCommitRequest,
    };
    use crate::gas_pricer::GasPricerConfig;
    use crate::pbft_finalize::{
        PbftFinalizationAnchor, PbftFinalizationCleanupIntent, PbftFinalizationPlan,
        PbftFinalizationPositionedHash, PbftFinalizationRuntimeAction, PbftFinalizationRuntimePlan,
        PbftFinalizationRuntimeStatus, PbftFinalizationStatus, PbftFinalizationStorageWriteIntent,
        PbftFinalizationStorageWriteStage, start_pbft_finalization_runtime,
    };
    use crate::sortition::{SortitionConfig, SortitionParams, VdfParams, VrfParams};
    use crate::transaction_service::TransactionServiceConfig;
    use ethereum_types::U256;
    use rustaxa_storage::{Config, Storage};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct RewardVoteCursorCommitStub {
        result: crate::pbft_vote_runtime::RewardVoteCursorCommitResult,
        request: Mutex<Option<crate::pbft_vote_runtime::RewardVoteCursorCommitRequest>>,
    }

    struct RewardVoteResetStageStub;

    struct BlockingRewardVoteResetStagePort {
        prepared: std::sync::mpsc::Sender<()>,
        dropped: std::sync::mpsc::Sender<()>,
    }

    struct BlockingRewardVoteResetStageGuard {
        prepared: std::sync::mpsc::Sender<()>,
        dropped: std::sync::mpsc::Sender<()>,
    }

    impl Drop for BlockingRewardVoteResetStageGuard {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }

    type FinalizedTransactionRequest = (
        u64,
        u64,
        Vec<crate::transaction_service::TransactionServiceAccountNonceFact>,
        Vec<u8>,
    );

    struct FinalizedTransactionStatusStub {
        accepted_count: u64,
        error: Option<&'static str>,
        request: Mutex<Option<FinalizedTransactionRequest>>,
    }

    impl PbftFinalizedTransactionStatusPort for FinalizedTransactionStatusStub {
        fn update_finalized_transactions_from_period_data(
            &self,
            period: u64,
            retention_window: u64,
            account_nonce_facts: Vec<
                crate::transaction_service::TransactionServiceAccountNonceFact,
            >,
            period_data_rlp: &[u8],
        ) -> Result<crate::transaction_service::TransactionServiceFinalizedStatusReport> {
            *self.request.lock().expect("request lock") = Some((
                period,
                retention_window,
                account_nonce_facts,
                period_data_rlp.to_vec(),
            ));
            if let Some(error) = self.error {
                return Err(anyhow!(error));
            }
            Ok(
                crate::transaction_service::TransactionServiceFinalizedStatusReport {
                    removed_non_finalized: Vec::new(),
                    queue_erased: Vec::new(),
                    finalized_account_purged: Vec::new(),
                    accepted_count: self.accepted_count,
                },
            )
        }
    }

    impl PbftRewardVoteCursorCommitPort for RewardVoteCursorCommitStub {
        fn commit_reward_vote_cursor(
            &self,
            request: crate::pbft_vote_runtime::RewardVoteCursorCommitRequest,
        ) -> Result<crate::pbft_vote_runtime::RewardVoteCursorCommitResult> {
            *self.request.lock().expect("request lock") = Some(request);
            Ok(self.result.clone())
        }
    }

    impl PbftRewardVoteResetStageGuard for RewardVoteResetStageStub {
        fn prepare_reward_votes_reset_stage(
            &self,
            _request: crate::pbft_vote_runtime::RewardVoteResetPrepareRequest,
        ) -> Result<PbftFinalizationStorageWriteStage> {
            Err(anyhow!("PBFT_TEST_UNEXPECTED_REWARD_VOTE_STAGE_REQUEST"))
        }
    }

    impl PbftRewardVoteResetStagePort for RewardVoteResetStageStub {
        type Guard<'a> = RewardVoteResetStageStub;

        fn lock_reward_votes(&self) -> Result<Self::Guard<'_>> {
            Ok(RewardVoteResetStageStub)
        }
    }

    impl PbftRewardVoteResetStageGuard for BlockingRewardVoteResetStageGuard {
        fn prepare_reward_votes_reset_stage(
            &self,
            _request: crate::pbft_vote_runtime::RewardVoteResetPrepareRequest,
        ) -> Result<PbftFinalizationStorageWriteStage> {
            self.prepared.send(()).expect("preparation signal sends");
            Ok(PbftFinalizationStorageWriteStage {
                stage: 4,
                has_reward_votes_reset: true,
                reward_votes_bundle_rlp: vec![0xc1, 0x01],
                ..Default::default()
            })
        }
    }

    impl PbftRewardVoteResetStagePort for BlockingRewardVoteResetStagePort {
        type Guard<'a> = BlockingRewardVoteResetStageGuard;

        fn lock_reward_votes(&self) -> Result<Self::Guard<'_>> {
            Ok(BlockingRewardVoteResetStageGuard {
                prepared: self.prepared.clone(),
                dropped: self.dropped.clone(),
            })
        }
    }

    fn reward_vote_cursor_commit_stub(
        cursor: crate::pbft_vote_runtime::RewardVoteCursor,
        reset_generation: u64,
    ) -> RewardVoteCursorCommitStub {
        RewardVoteCursorCommitStub {
            result: crate::pbft_vote_runtime::RewardVoteCursorCommitResult {
                status: crate::pbft_vote_runtime::RewardVoteCursorCommitStatus::Applied,
                cursor,
                reset_generation,
                error_code: "",
            },
            request: Mutex::new(None),
        }
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn finalization_sortition_services(
        name: &str,
    ) -> (PathBuf, PbftManagerService, DagTransactionService) {
        finalization_sortition_services_with_interval(name, 10)
    }

    fn finalization_sortition_services_with_interval(
        name: &str,
        changing_interval: u16,
    ) -> (PathBuf, PbftManagerService, DagTransactionService) {
        let path = unique_temp_dir(name);
        let storage = Arc::new(Storage::new(Config::new(path.clone())).expect("storage opens"));
        let (manager, dag) =
            finalization_sortition_services_from_storage(storage, changing_interval);
        (path, manager, dag)
    }

    fn finalization_sortition_services_from_storage(
        storage: Arc<Storage>,
        changing_interval: u16,
    ) -> (PbftManagerService, DagTransactionService) {
        let runtime =
            create_pbft_manager_runtime_from_storage(storage.as_ref(), storage_startup_fact())
                .expect("manager runtime restores");
        let chain = crate::pbft_chain::PbftChainService::restore(storage.clone())
            .expect("PBFT chain restores");
        let manager = PbftManagerService::new(runtime, storage.clone(), chain);
        let dag = DagTransactionService::restore(
            storage,
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
                    dag_efficiency_targets: (5_000, 10_000),
                    changing_interval,
                    computation_interval: changing_interval.min(5),
                },
            },
        )
        .expect("DAG transaction service restores");
        (manager, dag)
    }

    fn install_sortition_commit_runtime(
        runtime: &mut PbftManagerRuntimeState,
        write_set: PbftFinalizationStorageWriteIntent,
    ) {
        runtime.finalization_runtime_plan = Some(PbftFinalizationPlan {
            finalize_block: true,
            anchor: PbftFinalizationAnchor::Anchored,
            executed_pbft_block: true,
            cleanup: PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: true,
                reset_reward_votes: true,
                set_dag_block_order: true,
                update_sortition_params: true,
                update_finalized_transactions_status: true,
                update_pbft_chain: true,
                clear_anchor_dag_cache: true,
                finalize_final_chain: true,
                maybe_update_dynamic_lambda: false,
                advance_period: true,
                process_pillar_block: false,
            },
            storage_write_intent: write_set,
            status: PbftFinalizationStatus::Accepted,
        });
        runtime.finalization_runtime_session = Some(start_pbft_finalization_runtime(
            &PbftFinalizationRuntimePlan {
                finalize_block: true,
                status: PbftFinalizationStatus::Accepted,
                actions: vec![
                    PbftFinalizationRuntimeAction::CommitSortitionRuntime,
                    PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime,
                ],
            },
        ));
    }

    fn install_reward_votes_reset_runtime(
        runtime: &mut PbftManagerRuntimeState,
        write_set: PbftFinalizationStorageWriteIntent,
    ) {
        runtime.finalization_runtime_plan = Some(PbftFinalizationPlan {
            finalize_block: true,
            anchor: PbftFinalizationAnchor::Anchored,
            executed_pbft_block: true,
            cleanup: PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: true,
                reset_reward_votes: true,
                set_dag_block_order: true,
                update_sortition_params: true,
                update_finalized_transactions_status: true,
                update_pbft_chain: true,
                clear_anchor_dag_cache: true,
                finalize_final_chain: true,
                maybe_update_dynamic_lambda: false,
                advance_period: true,
                process_pillar_block: false,
            },
            storage_write_intent: write_set,
            status: PbftFinalizationStatus::Accepted,
        });
        runtime.finalization_runtime_session = Some(start_pbft_finalization_runtime(
            &PbftFinalizationRuntimePlan {
                finalize_block: true,
                status: PbftFinalizationStatus::Accepted,
                actions: vec![
                    PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime,
                    PbftFinalizationRuntimeAction::SetDagBlockOrder,
                ],
            },
        ));
    }

    fn install_transaction_status_runtime(
        runtime: &mut PbftManagerRuntimeState,
        write_set: PbftFinalizationStorageWriteIntent,
    ) {
        runtime.finalization_runtime_plan = Some(PbftFinalizationPlan {
            finalize_block: true,
            anchor: PbftFinalizationAnchor::Anchored,
            executed_pbft_block: true,
            cleanup: PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: true,
                reset_reward_votes: true,
                set_dag_block_order: true,
                update_sortition_params: true,
                update_finalized_transactions_status: true,
                update_pbft_chain: true,
                clear_anchor_dag_cache: true,
                finalize_final_chain: true,
                maybe_update_dynamic_lambda: false,
                advance_period: true,
                process_pillar_block: false,
            },
            storage_write_intent: write_set,
            status: PbftFinalizationStatus::Accepted,
        });
        runtime.finalization_runtime_session = Some(start_pbft_finalization_runtime(
            &PbftFinalizationRuntimePlan {
                finalize_block: true,
                status: PbftFinalizationStatus::Accepted,
                actions: vec![
                    PbftFinalizationRuntimeAction::UpdateFinalizedTransactions,
                    PbftFinalizationRuntimeAction::UpdatePbftChain,
                ],
            },
        ));
    }

    fn commit_reward_votes_reset_generation(runtime: &PbftManagerRuntimeState) -> u64 {
        let guard = runtime
            .storage
            .lock_extra_reward_votes()
            .expect("reward-vote storage lock remains healthy");
        guard
            .commit_reset_batch(runtime.storage.create_write_batch(), false)
            .expect("reward-vote reset commits")
    }

    fn finalization_period_data(
        pivot: H256,
        unique_transactions: usize,
        dag_transaction_ref_counts: &[usize],
    ) -> Vec<u8> {
        let mut pbft_block = RlpStream::new_list(8);
        pbft_block.append(&H256::from_low_u64_be(1));
        pbft_block.append(&pivot);
        pbft_block.append(&H256::from_low_u64_be(2));
        pbft_block.append(&H256::from_low_u64_be(3));
        pbft_block.append(&1_u64);
        pbft_block.append(&123_u64);
        pbft_block.begin_list(0);
        pbft_block.append(&vec![0_u8; 65]);

        let ordered_transaction_hashes = RlpStream::new_list(0);
        let mut transaction_indexes = RlpStream::new_list(dag_transaction_ref_counts.len());
        for count in dag_transaction_ref_counts {
            transaction_indexes.begin_list(*count);
            for index in 0..*count {
                transaction_indexes.append(&index);
            }
        }
        let compact_blocks = RlpStream::new_list(0);
        let mut bundle = RlpStream::new_list(3);
        bundle.append_raw(&ordered_transaction_hashes.out(), 1);
        bundle.append_raw(&transaction_indexes.out(), 1);
        bundle.append_raw(&compact_blocks.out(), 1);

        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(&pbft_block.out(), 1);
        period_data.append_empty_data();
        period_data.append_raw(&bundle.out(), 1);
        period_data.begin_list(unique_transactions);
        for _ in 0..unique_transactions {
            period_data.append_empty_data();
        }
        period_data.out().to_vec()
    }

    fn finalization_sortition_write_set(
        block_period: u64,
        null_anchor: bool,
        period_data_rlp: Vec<u8>,
    ) -> PbftFinalizationStorageWriteIntent {
        PbftFinalizationStorageWriteIntent {
            persist_pbft_head: true,
            persist_period_data: true,
            reset_reward_votes: false,
            update_sortition_params: true,
            apply_dynamic_lambda_update: false,
            persist_period_lambda: false,
            persist_executed_pbft_status: false,
            process_pillar_block: false,
            pbft_block_hash: H256::repeat_byte(7),
            pbft_head_hash: H256::repeat_byte(8),
            block_period,
            null_anchor,
            anchor_hash: if null_anchor {
                H256::zero()
            } else {
                H256::repeat_byte(4)
            },
            reward_vote_period: 0,
            reward_vote_round: 0,
            reward_vote_step: 0,
            reward_vote_block_hash: H256::zero(),
            period_lambda: 0,
            blocks_per_year: 0,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            executed_pbft_status: false,
            pbft_head_payload: Vec::new(),
            period_data_rlp,
            dag_block_period_writes: Vec::<PbftFinalizationPositionedHash>::new(),
            transaction_location_writes: Vec::<PbftFinalizationPositionedHash>::new(),
        }
    }

    fn finalization_dag_block_rlp(pivot: H256, level: u64) -> Vec<u8> {
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
        block.begin_list(0);
        block.append(&&[0_u8; 65][..]);
        block.append(&0_u64);
        block.out().to_vec()
    }

    fn install_owned_finalization_runtime(
        runtime: &mut PbftManagerRuntimeState,
        write_set: PbftFinalizationStorageWriteIntent,
        cleanup: PbftFinalizationCleanupIntent,
        executed_pbft_block: bool,
        actions: Option<Vec<PbftFinalizationRuntimeAction>>,
    ) {
        let plan = PbftFinalizationPlan {
            finalize_block: true,
            anchor: if write_set.null_anchor {
                PbftFinalizationAnchor::Null
            } else {
                PbftFinalizationAnchor::Anchored
            },
            executed_pbft_block,
            cleanup,
            storage_write_intent: write_set,
            status: PbftFinalizationStatus::Accepted,
        };
        let runtime_plan = actions
            .map(|actions| PbftFinalizationRuntimePlan {
                finalize_block: true,
                status: PbftFinalizationStatus::Accepted,
                actions,
            })
            .unwrap_or_else(|| crate::pbft_finalize::plan_pbft_finalization_runtime(&plan));
        runtime.finalization_runtime_session = Some(start_pbft_finalization_runtime(&runtime_plan));
        runtime.finalization_runtime_plan = Some(plan);
    }

    fn finalization_start_plan(
        write_set: PbftFinalizationStorageWriteIntent,
    ) -> PbftFinalizationPlan {
        PbftFinalizationPlan {
            finalize_block: true,
            anchor: if write_set.null_anchor {
                PbftFinalizationAnchor::Null
            } else {
                PbftFinalizationAnchor::Anchored
            },
            executed_pbft_block: false,
            cleanup: PbftFinalizationCleanupIntent {
                update_sortition_params: true,
                ..empty_finalization_cleanup()
            },
            storage_write_intent: write_set,
            status: PbftFinalizationStatus::Accepted,
        }
    }

    fn empty_finalization_cleanup() -> PbftFinalizationCleanupIntent {
        PbftFinalizationCleanupIntent {
            persist_pbft_block_metadata: false,
            reset_reward_votes: false,
            set_dag_block_order: false,
            update_sortition_params: false,
            update_finalized_transactions_status: false,
            update_pbft_chain: false,
            clear_anchor_dag_cache: false,
            finalize_final_chain: false,
            maybe_update_dynamic_lambda: false,
            advance_period: false,
            process_pillar_block: false,
        }
    }

    #[test]
    fn finalization_start_fresh_applies_primary_and_retains_external_boundary() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_start_fresh");
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 3, &[2, 4]),
        );
        write_set.pbft_head_payload = vec![0xde, 0xad, 0xbe, 0xef];
        let expected_period_data = write_set.period_data_rlp.clone();
        let mut runtime = manager.lock();

        let boundary = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: finalization_start_plan(write_set),
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                        sync: false,
                    },
                },
            )
            .expect("fresh finalization start succeeds");

        assert_eq!(
            boundary.next_step.action,
            Some(PbftFinalizationRuntimeAction::CommitSortitionRuntime)
        );
        assert!(runtime.finalization_runtime_session.is_some());
        assert!(runtime.finalization_runtime_plan.is_some());
        assert!(runtime.finalization_sortition_commit_request.is_some());
        assert_eq!(
            runtime
                .storage
                .period()
                .data_raw(1)
                .expect("primary period data reads"),
            expected_period_data
        );

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_rejects_caller_sortition_and_clears_runtime() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_start_sortition_reject");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        let error = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: finalization_start_plan(write_set),
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage {
                            stage: 3,
                            has_sortition_params_change: true,
                            sortition_params_change_period: 1,
                            sortition_params_change_interval_efficiency: 5_000,
                            sortition_params_change_threshold_upper: 1_100,
                            ..Default::default()
                        }],
                        sync: false,
                    },
                },
            )
            .expect_err("caller-owned sortition stage rejects");

        assert_eq!(
            error.to_string(),
            "PBFT_FINALIZE_SORTITION_STAGE_CALLER_OWNED"
        );
        assert!(runtime.finalization_runtime_session.is_none());
        assert!(runtime.finalization_runtime_plan.is_none());
        assert!(runtime.finalization_sortition_commit_request.is_none());
        assert_eq!(runtime.finalization_reward_votes_reset_generation, 0);
        assert!(
            runtime
                .storage
                .period()
                .data_raw(1)
                .expect("rejected period data reads")
                .is_empty()
        );

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_rejects_caller_reward_stage_and_clears_runtime() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_start_reward_reject");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        let error = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: finalization_start_plan(write_set),
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage {
                            stage: 4,
                            has_reward_votes_reset: true,
                            reward_votes_bundle_rlp: vec![0xc1, 0x01],
                            ..Default::default()
                        }],
                        sync: false,
                    },
                },
            )
            .expect_err("caller-owned reward stage rejects");

        assert_eq!(
            error.to_string(),
            "PBFT_FINALIZE_REWARD_VOTES_STAGE_CALLER_OWNED"
        );
        assert!(runtime.finalization_runtime_session.is_none());
        assert!(runtime.finalization_runtime_plan.is_none());
        assert!(runtime.finalization_sortition_commit_request.is_none());
        assert_eq!(runtime.finalization_reward_votes_reset_generation, 0);
        assert!(
            runtime
                .storage
                .period()
                .data_raw(1)
                .expect("rejected period data reads")
                .is_empty()
        );

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_holds_reward_guard_through_primary_commit() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_reward_guard_commit");
        let storage = manager.lock().storage.clone();
        let storage_guard = storage.lock_extra_reward_votes().unwrap();
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        write_set.pbft_head_payload = vec![0xde, 0xad, 0xbe, 0xef];
        write_set.reset_reward_votes = true;
        write_set.reward_vote_period = 1;
        write_set.reward_vote_round = 2;
        write_set.reward_vote_step = 3;
        write_set.reward_vote_block_hash = H256::repeat_byte(7);
        let mut plan = finalization_start_plan(write_set);
        plan.cleanup.reset_reward_votes = true;
        let (prepared_tx, prepared_rx) = std::sync::mpsc::channel();
        let (dropped_tx, dropped_rx) = std::sync::mpsc::channel();
        let port = BlockingRewardVoteResetStagePort {
            prepared: prepared_tx,
            dropped: dropped_tx,
        };
        let manager = Arc::new(manager);
        let dag = Arc::new(dag);
        let worker_manager = manager.clone();
        let worker_dag = dag.clone();
        let worker = std::thread::spawn(move || {
            let mut runtime = worker_manager.lock();
            runtime.start_finalization_executor(
                worker_dag.as_ref(),
                &port,
                PbftFinalizationExecutorStartRequest {
                    plan,
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                        sync: false,
                    },
                },
            )
        });

        prepared_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("reward stage prepares before storage lock");
        assert!(
            dropped_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "reward guard must remain held while primary storage is blocked"
        );
        drop(storage_guard);
        worker
            .join()
            .expect("finalization worker joins")
            .expect("primary storage commits");
        dropped_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("reward guard drops after primary storage commit");

        drop(manager);
        drop(dag);
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_primary_rejection_discards_prepared_sortition() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_start_primary_reject");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let sortition_before = {
            let sortition = dag.lock_sortition().expect("sortition locks");
            (
                sortition.current_params(),
                sortition.params_changes().clone(),
            )
        };
        let mut runtime = manager.lock();

        let boundary = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: finalization_start_plan(write_set),
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                        sync: false,
                    },
                },
            )
            .expect("primary rejection returns a terminal boundary");

        assert_eq!(
            boundary.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionFailed
        );
        assert_eq!(
            boundary.error_code,
            "PBFT_FINALIZE_PRIMARY_STORAGE_REJECTED"
        );
        assert!(runtime.finalization_runtime_session.is_none());
        assert!(runtime.finalization_runtime_plan.is_none());
        assert!(runtime.finalization_sortition_commit_request.is_none());
        assert_eq!(runtime.finalization_reward_votes_reset_generation, 0);
        assert!(
            runtime
                .storage
                .period()
                .data_raw(1)
                .expect("rejected period data reads")
                .is_empty()
        );
        drop(runtime);

        let sortition_after = {
            let sortition = dag.lock_sortition().expect("sortition locks");
            (
                sortition.current_params(),
                sortition.params_changes().clone(),
            )
        };
        assert_eq!(sortition_after, sortition_before);

        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_unknown_mode_clears_stale_runtime() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_start_unknown");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        install_owned_finalization_runtime(
            &mut runtime,
            write_set.clone(),
            empty_finalization_cleanup(),
            false,
            Some(vec![PbftFinalizationRuntimeAction::UpdatePbftChain]),
        );
        runtime.finalization_reward_votes_reset_generation = 99;
        let error = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: finalization_start_plan(write_set),
                    mode: PbftFinalizationExecutorStartMode::Unknown,
                },
            )
            .expect_err("unknown start mode fails closed");

        assert_eq!(error.to_string(), "PBFT_FINALIZE_EXECUTOR_UNKNOWN_MODE");
        assert!(runtime.finalization_runtime_session.is_none());
        assert!(runtime.finalization_runtime_plan.is_none());
        assert!(runtime.finalization_sortition_commit_request.is_none());
        assert_eq!(runtime.finalization_reward_votes_reset_generation, 0);

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_resume_missing_primary_clears_runtime() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_start_resume_missing");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();

        let boundary = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: finalization_start_plan(write_set),
                    mode: PbftFinalizationExecutorStartMode::Resume {
                        final_chain_last_block: 0,
                    },
                },
            )
            .expect("missing durable facts produce a rejected resume boundary");

        assert_eq!(
            boundary.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::RejectedPlan
        );
        assert_eq!(
            boundary.next_step.error_code,
            "PBFT_FINALIZE_RESUME_NOT_PERSISTED"
        );
        assert!(runtime.finalization_runtime_session.is_none());
        assert!(runtime.finalization_runtime_plan.is_none());
        assert!(runtime.finalization_sortition_commit_request.is_none());
        assert_eq!(runtime.finalization_reward_votes_reset_generation, 0);

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_resume_authenticates_reset_generation() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_start_resume_generation");
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        write_set.pbft_head_payload = vec![0xde, 0xad, 0xbe, 0xef];
        let plan = finalization_start_plan(write_set);
        let mut runtime = manager.lock();
        runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: plan.clone(),
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                        sync: false,
                    },
                },
            )
            .expect("fresh primary storage persists");

        let generation = commit_reward_votes_reset_generation(&runtime);
        runtime.finalization_reward_votes_reset_generation = generation;
        let mut resume_plan = plan.clone();
        resume_plan.cleanup.reset_reward_votes = true;
        resume_plan.storage_write_intent.reset_reward_votes = true;
        let matching = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: resume_plan.clone(),
                    mode: PbftFinalizationExecutorStartMode::Resume {
                        final_chain_last_block: 0,
                    },
                },
            )
            .expect("matching generation resumes");
        assert_eq!(
            matching.next_step.action,
            Some(PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime)
        );
        assert_eq!(
            runtime.finalization_reward_votes_reset_generation,
            generation
        );

        runtime.finalization_reward_votes_reset_generation = generation + 1;
        let stale = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: resume_plan,
                    mode: PbftFinalizationExecutorStartMode::Resume {
                        final_chain_last_block: 0,
                    },
                },
            )
            .expect("stale generation still resumes without reset authority");
        assert_eq!(
            stale.next_step.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
        assert_eq!(runtime.finalization_reward_votes_reset_generation, 0);

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_resume_replays_durable_sortition_change() {
        let (path, manager, dag) = finalization_sortition_services_with_interval(
            "pbft_manager_finalization_start_resume_sortition",
            1,
        );
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 3, &[2, 4]),
        );
        write_set.pbft_head_payload = vec![0xde, 0xad, 0xbe, 0xef];
        let plan = finalization_start_plan(write_set);
        let mut runtime = manager.lock();

        let fresh = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: plan.clone(),
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                        sync: false,
                    },
                },
            )
            .expect("fresh primary storage persists sortition change");
        assert_eq!(
            fresh.next_step.action,
            Some(PbftFinalizationRuntimeAction::CommitSortitionRuntime)
        );
        assert!(
            runtime
                .finalization_sortition_commit_request
                .and_then(|request| request.expected_change)
                .is_some()
        );

        let resumed = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan,
                    mode: PbftFinalizationExecutorStartMode::Resume {
                        final_chain_last_block: 0,
                    },
                },
            )
            .expect("durable sortition change authenticates resume");
        assert_eq!(
            resumed.next_step.action,
            Some(PbftFinalizationRuntimeAction::CommitSortitionRuntime)
        );

        let advanced = runtime
            .advance_finalization_sortition_commit(&dag, resumed.next_step.action_index)
            .expect("authenticated sortition request publishes");
        assert_eq!(
            advanced.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
        assert!(runtime.finalization_sortition_commit_request.is_none());

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_resume_rejects_sortition_preview_without_durable_match() {
        let (path, manager, dag) = finalization_sortition_services_with_interval(
            "pbft_manager_finalization_start_resume_sortition_mismatch",
            1,
        );
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 3, &[2, 4]),
        );
        write_set.pbft_head_payload = vec![0xde, 0xad, 0xbe, 0xef];
        let plan = finalization_start_plan(write_set);
        let mut runtime = manager.lock();
        runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: plan.clone(),
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                        sync: false,
                    },
                },
            )
            .expect("fresh primary storage persists sortition change");
        let request = runtime
            .finalization_sortition_commit_request
            .as_mut()
            .expect("sortition preview retained");
        request
            .expected_change
            .as_mut()
            .expect("interval emits change")
            .threshold_upper ^= 1;

        let resumed = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan,
                    mode: PbftFinalizationExecutorStartMode::Resume {
                        final_chain_last_block: 0,
                    },
                },
            )
            .expect("mismatched preview resumes without sortition authority");
        assert_eq!(
            resumed.next_step.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
        assert!(runtime.finalization_sortition_commit_request.is_none());

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_full_restart_does_not_replay_sortition_change() {
        let (path, manager, dag) = finalization_sortition_services_with_interval(
            "pbft_manager_finalization_start_restart_sortition",
            1,
        );
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 3, &[2, 4]),
        );
        write_set.pbft_head_payload = vec![0xde, 0xad, 0xbe, 0xef];
        let plan = finalization_start_plan(write_set);
        let mut runtime = manager.lock();
        runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: plan.clone(),
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                        sync: false,
                    },
                },
            )
            .expect("fresh primary storage persists sortition change");
        let storage = runtime.storage.clone();
        drop(runtime);
        drop(manager);
        drop(dag);

        let (restarted_manager, restarted_dag) =
            finalization_sortition_services_from_storage(storage, 1);
        let mut restarted = restarted_manager.lock();
        let resumed = restarted
            .start_finalization_executor(
                &restarted_dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan,
                    mode: PbftFinalizationExecutorStartMode::Resume {
                        final_chain_last_block: 0,
                    },
                },
            )
            .expect("full restart resumes from durable tail only");
        assert_eq!(
            resumed.next_step.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
        assert!(restarted.finalization_sortition_commit_request.is_none());

        drop(restarted);
        drop(restarted_manager);
        drop(restarted_dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_start_resume_excludes_sortition_preview_without_durable_change() {
        let (path, manager, dag) = finalization_sortition_services(
            "pbft_manager_finalization_start_resume_sortition_no_change",
        );
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 3, &[2, 4]),
        );
        write_set.pbft_head_payload = vec![0xde, 0xad, 0xbe, 0xef];
        let plan = finalization_start_plan(write_set);
        let mut runtime = manager.lock();
        runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan: plan.clone(),
                    mode: PbftFinalizationExecutorStartMode::Fresh {
                        primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                        sync: false,
                    },
                },
            )
            .expect("fresh primary storage retains no-change preview");
        assert!(
            runtime
                .finalization_sortition_commit_request
                .is_some_and(|request| request.expected_change.is_none())
        );

        let resumed = runtime
            .start_finalization_executor(
                &dag,
                &RewardVoteResetStageStub,
                PbftFinalizationExecutorStartRequest {
                    plan,
                    mode: PbftFinalizationExecutorStartMode::Resume {
                        final_chain_last_block: 0,
                    },
                },
            )
            .expect("no-change preview resumes without replay authority");
        assert_eq!(
            resumed.next_step.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
        assert!(runtime.finalization_sortition_commit_request.is_none());

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_advancement_validates_dag_and_reaches_next_leaf() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_advance_dag");
        let first_hash = H256::repeat_byte(2);
        let anchor_hash = H256::repeat_byte(3);
        {
            let mut dag_state = dag.lock_dag().unwrap();
            dag_state
                .state
                .add_block(DagManagerBlock {
                    hash: first_hash,
                    pivot: H256::repeat_byte(1),
                    tips: Vec::new(),
                    level: 1,
                    difficulty: 90,
                })
                .unwrap();
            dag_state
                .state
                .add_block(DagManagerBlock {
                    hash: anchor_hash,
                    pivot: first_hash,
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 90,
                })
                .unwrap();
        }
        let storage = manager.lock().storage.clone();
        save_dag_block_to_storage(
            storage.as_ref(),
            first_hash,
            1,
            0,
            &finalization_dag_block_rlp(H256::repeat_byte(1), 1),
        )
        .unwrap();
        save_dag_block_to_storage(
            storage.as_ref(),
            anchor_hash,
            2,
            0,
            &finalization_dag_block_rlp(first_hash, 2),
        )
        .unwrap();
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        write_set.anchor_hash = anchor_hash;
        write_set.dag_block_period_writes = vec![
            PbftFinalizationPositionedHash {
                hash: first_hash,
                position: 0,
            },
            PbftFinalizationPositionedHash {
                hash: anchor_hash,
                position: 1,
            },
        ];
        let mut runtime = manager.lock();
        install_owned_finalization_runtime(
            &mut runtime,
            write_set,
            PbftFinalizationCleanupIntent {
                set_dag_block_order: true,
                finalize_final_chain: true,
                ..empty_finalization_cleanup()
            },
            false,
            Some(vec![
                PbftFinalizationRuntimeAction::SetDagBlockOrder,
                PbftFinalizationRuntimeAction::FinalizeFinalChain,
            ]),
        );

        let (step, expired_hashes, refresh_counters) = runtime
            .advance_finalization_set_dag_order(&dag, 0)
            .expect("native DAG order validates");
        assert!(expired_hashes.is_empty());
        assert!(refresh_counters);
        let dag_state = dag.lock_dag().unwrap();
        assert_eq!(dag_state.state.period(), 1);
        assert_eq!(dag_state.state.anchor(), anchor_hash);
        let drain = runtime
            .continue_finalization_executor_from_step(step)
            .expect("owned drain reaches the next external leaf");
        let drain = runtime
            .finish_finalization_executor(Ok(drain))
            .expect("active boundary remains retained");

        assert_eq!(
            drain.next_step.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
        assert!(runtime.finalization_runtime_session.is_some());
        assert!(runtime.finalization_runtime_plan.is_some());

        drop(runtime);
        drop(manager);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_dag_advancement_rejects_stale_cursor_before_mutation() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_finalization_advance_dag_stale");
        let write_set = finalization_sortition_write_set(
            1,
            true,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let initial_period = dag.lock_dag().unwrap().state.period();
        let mut runtime = manager.lock();
        install_owned_finalization_runtime(
            &mut runtime,
            write_set,
            PbftFinalizationCleanupIntent {
                set_dag_block_order: true,
                ..empty_finalization_cleanup()
            },
            false,
            Some(vec![PbftFinalizationRuntimeAction::SetDagBlockOrder]),
        );

        let (step, expired_hashes, refresh_counters) = runtime
            .advance_finalization_set_dag_order(&dag, 1)
            .expect("stale cursor returns a terminal step");
        assert_eq!(
            step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionMismatch
        );
        assert!(expired_hashes.is_empty());
        assert!(!refresh_counters);
        assert_eq!(dag.lock_dag().unwrap().state.period(), initial_period);

        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_advancement_failure_and_stale_cursor_clear_runtime() {
        for (name, stale_cursor) in [("failure", false), ("stale", true)] {
            let (path, manager, _dag) = finalization_sortition_services(&format!(
                "pbft_manager_finalization_advance_{name}"
            ));
            let write_set = finalization_sortition_write_set(
                1,
                false,
                finalization_period_data(H256::repeat_byte(4), 0, &[]),
            );
            let mut runtime = manager.lock();
            install_owned_finalization_runtime(
                &mut runtime,
                write_set,
                PbftFinalizationCleanupIntent {
                    finalize_final_chain: true,
                    ..empty_finalization_cleanup()
                },
                false,
                Some(vec![PbftFinalizationRuntimeAction::FinalizeFinalChain]),
            );

            let step = runtime
                .fail_finalization_external_effect(
                    u32::from(stale_cursor),
                    77,
                    "PBFT_FINALIZE_TEST_EXTERNAL_FAILURE".to_string(),
                )
                .expect("failure report returns a terminal step");
            let drain = runtime
                .continue_finalization_executor_from_step(step)
                .expect("terminal step projects");
            let drain = runtime
                .finish_finalization_executor(Ok(drain))
                .expect("terminal boundary clears");

            assert_eq!(
                drain.next_step.runtime_status,
                if stale_cursor {
                    PbftFinalizationRuntimeStatus::ActionMismatch
                } else {
                    PbftFinalizationRuntimeStatus::ActionFailed
                }
            );
            assert_eq!(
                drain.next_step.error_code,
                if stale_cursor {
                    "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH"
                } else {
                    "PBFT_FINALIZE_TEST_EXTERNAL_FAILURE"
                }
            );
            assert!(runtime.finalization_runtime_session.is_none());
            assert!(runtime.finalization_runtime_plan.is_none());

            drop(runtime);
            drop(manager);
            let _ = fs::remove_dir_all(path);
        }
    }

    #[test]
    fn finalization_advancement_validates_final_chain_facts() {
        for (name, last_block, accepted) in [("accepted", 1, true), ("mismatch", 0, false)] {
            let (path, manager, _dag) = finalization_sortition_services(&format!(
                "pbft_manager_finalization_advance_final_chain_{name}"
            ));
            let write_set = finalization_sortition_write_set(
                1,
                false,
                finalization_period_data(H256::repeat_byte(4), 0, &[]),
            );
            let mut runtime = manager.lock();
            install_owned_finalization_runtime(
                &mut runtime,
                write_set,
                PbftFinalizationCleanupIntent {
                    finalize_final_chain: true,
                    advance_period: true,
                    ..empty_finalization_cleanup()
                },
                false,
                Some(vec![
                    PbftFinalizationRuntimeAction::FinalizeFinalChain,
                    PbftFinalizationRuntimeAction::AdvancePeriod,
                ]),
            );

            let step = runtime
                .advance_finalization_live_mutation(0, |action, write_set| {
                    let mut report = base_owned_finalization_live_report(action, write_set);
                    report.final_chain_dispatched = true;
                    report.final_chain_blocks_per_year = write_set.blocks_per_year;
                    report.final_chain_last_block = last_block;
                    report
                })
                .expect("FinalChain report validates");
            let drain = runtime
                .continue_finalization_executor_from_step(step)
                .expect("FinalChain result projects");
            let drain = runtime
                .finish_finalization_executor(Ok(drain))
                .expect("FinalChain boundary finishes");

            if accepted {
                assert_eq!(
                    drain.next_step.action,
                    Some(PbftFinalizationRuntimeAction::AdvancePeriod)
                );
                assert!(runtime.finalization_runtime_session.is_some());
            } else {
                assert_eq!(
                    drain.next_step.runtime_status,
                    PbftFinalizationRuntimeStatus::ActionFailed
                );
                assert_eq!(
                    drain.next_step.error_code,
                    "PBFT_FINALIZE_LIVE_MUTATION_FINAL_CHAIN_LAST_BLOCK_MISMATCH"
                );
                assert!(runtime.finalization_runtime_session.is_none());
            }

            drop(runtime);
            drop(manager);
            let _ = fs::remove_dir_all(path);
        }
    }

    #[test]
    fn finalization_owned_drain_updates_chain_and_clears_anchor_cache() {
        let (path, manager, _dag) =
            finalization_sortition_services("pbft_manager_owned_drain_chain");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        runtime
            .state
            .record_cached_anchor_dag_order(H256::repeat_byte(42));
        install_owned_finalization_runtime(
            &mut runtime,
            write_set.clone(),
            PbftFinalizationCleanupIntent {
                update_pbft_chain: true,
                clear_anchor_dag_cache: true,
                finalize_final_chain: true,
                ..empty_finalization_cleanup()
            },
            false,
            Some(vec![
                PbftFinalizationRuntimeAction::UpdatePbftChain,
                PbftFinalizationRuntimeAction::ClearAnchorDagCache,
                PbftFinalizationRuntimeAction::FinalizeFinalChain,
            ]),
        );

        let drained = runtime
            .drain_finalization_owned_actions()
            .expect("owned chain/cache drain succeeds");

        assert!(drained.cleared_anchor_dag_cache);
        assert!(drained.has_snapshot);
        assert_eq!(
            drained.next_step.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
        assert_eq!(runtime.state.cached_anchor_dag_order_count(), 0);
        let head = runtime
            .chain
            .read()
            .expect("PBFT chain lock remains healthy")
            .state
            .head();
        assert_eq!(head.size, 1);
        assert_eq!(head.last_pbft_block_hash, write_set.pbft_block_hash);
        assert_eq!(
            head.last_non_null_pbft_dag_anchor_hash,
            write_set.anchor_hash
        );

        drop(runtime);
        drop(manager);
        drop(_dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_owned_drain_persists_and_publishes_dynamic_lambda() {
        let (path, manager, _dag) =
            finalization_sortition_services("pbft_manager_owned_drain_dynamic_lambda");
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        write_set.apply_dynamic_lambda_update = true;
        write_set.persist_period_lambda = true;
        write_set.period_lambda = 1_500;
        write_set.rounds_count_dynamic_lambda = 12;
        write_set.dynamic_lambda = 1_250;
        let mut runtime = manager.lock();
        install_owned_finalization_runtime(
            &mut runtime,
            write_set.clone(),
            PbftFinalizationCleanupIntent {
                finalize_final_chain: true,
                ..empty_finalization_cleanup()
            },
            false,
            Some(vec![
                PbftFinalizationRuntimeAction::ApplyDynamicLambda,
                PbftFinalizationRuntimeAction::FinalizeFinalChain,
            ]),
        );

        let drained = runtime
            .drain_finalization_owned_actions()
            .expect("owned dynamic-lambda drain succeeds");

        assert!(drained.has_snapshot);
        assert_eq!(
            drained.next_step.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
        assert_eq!(
            runtime
                .storage
                .metadata()
                .period_lambda(1, false)
                .expect("period lambda reads"),
            Some(1_500)
        );
        assert_eq!(
            runtime
                .storage
                .metadata()
                .rounds_count_dynamic_lambda()
                .expect("dynamic-lambda rounds read"),
            12
        );
        let snapshot = runtime.state.snapshot();
        assert_eq!(snapshot.rounds_count_dynamic_lambda, 12);
        assert_eq!(snapshot.dynamic_lambda_ms, 1_250);

        install_owned_finalization_runtime(
            &mut runtime,
            write_set,
            PbftFinalizationCleanupIntent {
                finalize_final_chain: true,
                ..empty_finalization_cleanup()
            },
            false,
            Some(vec![
                PbftFinalizationRuntimeAction::ApplyDynamicLambda,
                PbftFinalizationRuntimeAction::FinalizeFinalChain,
            ]),
        );
        let replay = runtime
            .drain_finalization_owned_actions()
            .expect("idempotent dynamic-lambda replay succeeds");
        assert_eq!(
            replay.next_step.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );

        drop(runtime);
        drop(manager);
        drop(_dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_owned_drain_replays_executed_status_tail() {
        let (path, manager, _dag) =
            finalization_sortition_services("pbft_manager_owned_drain_executed");
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        write_set.persist_executed_pbft_status = true;
        write_set.executed_pbft_status = true;
        let mut runtime = manager.lock();
        install_owned_finalization_runtime(
            &mut runtime,
            write_set,
            empty_finalization_cleanup(),
            true,
            Some(vec![
                PbftFinalizationRuntimeAction::PersistExecutedStatus,
                PbftFinalizationRuntimeAction::SetExecutedFlag,
            ]),
        );

        let drained = runtime
            .drain_finalization_owned_actions()
            .expect("owned executed-status tail succeeds");

        assert!(drained.has_snapshot);
        assert!(drained.next_step.complete);
        assert_eq!(
            drained.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::Complete
        );
        assert_eq!(
            runtime
                .storage
                .pbft()
                .manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)
                .expect("executed status reads"),
            Some(true)
        );
        assert!(runtime.state.snapshot().executed_pbft_block);

        drop(runtime);
        drop(manager);
        drop(_dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_owned_drain_stops_around_final_chain_boundary() {
        let (path, manager, _dag) =
            finalization_sortition_services("pbft_manager_owned_drain_final_chain_boundary");
        let mut write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        write_set.persist_executed_pbft_status = true;
        write_set.executed_pbft_status = true;
        let mut runtime = manager.lock();
        install_owned_finalization_runtime(
            &mut runtime,
            write_set,
            PbftFinalizationCleanupIntent {
                clear_anchor_dag_cache: true,
                finalize_final_chain: true,
                advance_period: true,
                ..empty_finalization_cleanup()
            },
            true,
            Some(vec![
                PbftFinalizationRuntimeAction::ClearAnchorDagCache,
                PbftFinalizationRuntimeAction::FinalizeFinalChain,
                PbftFinalizationRuntimeAction::PersistExecutedStatus,
                PbftFinalizationRuntimeAction::SetExecutedFlag,
                PbftFinalizationRuntimeAction::AdvancePeriod,
            ]),
        );

        let before_external = runtime
            .drain_finalization_owned_actions()
            .expect("pre-FinalChain owned actions drain");
        assert_eq!(
            before_external.next_step.action,
            Some(PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
        assert_eq!(
            runtime
                .storage
                .pbft()
                .manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)
                .expect("executed status reads"),
            None
        );

        let session = runtime
            .finalization_runtime_session
            .take()
            .expect("finalization session remains active");
        runtime.finalization_runtime_session = Some(
            crate::pbft_finalize::report_pbft_finalization_runtime_action(
                session,
                crate::pbft_finalize::PbftFinalizationRuntimeActionResult {
                    action: PbftFinalizationRuntimeAction::FinalizeFinalChain,
                    success: true,
                    status: 0,
                    error_code: String::new(),
                },
            ),
        );
        let after_external = runtime
            .drain_finalization_owned_actions()
            .expect("post-FinalChain owned actions drain");
        assert_eq!(
            after_external.next_step.action,
            Some(PbftFinalizationRuntimeAction::AdvancePeriod)
        );
        assert_eq!(
            runtime
                .storage
                .pbft()
                .manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)
                .expect("executed status reads"),
            Some(true)
        );
        assert!(runtime.state.snapshot().executed_pbft_block);

        drop(runtime);
        drop(manager);
        drop(_dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_owned_drain_reports_rejected_storage_action() {
        let (path, manager, _dag) =
            finalization_sortition_services("pbft_manager_owned_drain_rejected");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        install_owned_finalization_runtime(
            &mut runtime,
            write_set,
            empty_finalization_cleanup(),
            false,
            Some(vec![PbftFinalizationRuntimeAction::PersistExecutedStatus]),
        );

        let drained = runtime
            .drain_finalization_owned_actions()
            .expect("rejected owned storage action reports through the runtime");

        assert_eq!(
            drained.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionFailed
        );
        assert!(!drained.next_step.complete);
        assert_eq!(
            drained.error_code,
            "PBFT_FINALIZE_EXECUTED_STATUS_STORAGE_REJECTED"
        );

        drop(runtime);
        drop(manager);
        drop(_dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_owned_drain_reports_missing_session() {
        let (path, manager, _dag) =
            finalization_sortition_services("pbft_manager_owned_drain_missing_session");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        install_owned_finalization_runtime(
            &mut runtime,
            write_set,
            empty_finalization_cleanup(),
            false,
            Some(vec![PbftFinalizationRuntimeAction::UpdatePbftChain]),
        );
        runtime.finalization_runtime_session = None;

        let drained = runtime
            .drain_finalization_owned_actions()
            .expect("missing session reports through the runtime contract");

        assert_eq!(
            drained.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionMismatch
        );
        assert!(!drained.next_step.has_action);
        assert_eq!(
            drained.next_step.error_code,
            "PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED"
        );
        assert!(drained.error_code.is_empty());

        drop(runtime);
        drop(manager);
        drop(_dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_owned_drain_rejects_chain_projection_without_publication() {
        let (path, manager, _dag) =
            finalization_sortition_services("pbft_manager_owned_drain_chain_rejected");
        let write_set = finalization_sortition_write_set(
            2,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        let original_head = runtime
            .chain
            .read()
            .expect("PBFT chain lock remains healthy")
            .state
            .head();
        install_owned_finalization_runtime(
            &mut runtime,
            write_set,
            PbftFinalizationCleanupIntent {
                update_pbft_chain: true,
                ..empty_finalization_cleanup()
            },
            false,
            Some(vec![PbftFinalizationRuntimeAction::UpdatePbftChain]),
        );

        let drained = runtime
            .drain_finalization_owned_actions()
            .expect("invalid chain projection reports through the runtime");

        assert_eq!(
            drained.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionFailed
        );
        assert_eq!(drained.error_code, "PBFT_FINALIZE_CHAIN_LIVE_REJECTED");
        assert_eq!(
            runtime
                .chain
                .read()
                .expect("PBFT chain lock remains healthy")
                .state
                .head(),
            original_head
        );

        drop(runtime);
        drop(manager);
        drop(_dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_sortition_preparation_publishes_canonical_commit_request() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_sortition_prepare");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 3, &[2, 4]),
        );
        let mut stages = vec![PbftFinalizationStorageWriteStage::default()];
        let mut runtime = manager.lock();

        prepare_pbft_finalization_sortition(&mut runtime, &dag, &write_set, &mut stages)
            .expect("canonical preparation succeeds");

        let request = runtime
            .finalization_sortition_commit_request
            .expect("commit request is published");
        assert_eq!(
            request,
            DagTransactionSortitionFinalizationCommitRequest {
                finalized_period:
                    crate::dag_transaction_service::DagTransactionSortitionFinalizationRequest {
                        period: 1,
                        efficiency_counts: crate::sortition::PeriodEfficiencyCounts {
                            has_pivot: true,
                            unique_transactions: 3,
                            total_dag_transaction_refs: 6,
                        },
                        non_empty_pbft_chain_size: 1,
                    },
                expected_change: None,
            }
        );
        assert_eq!(stages, vec![PbftFinalizationStorageWriteStage::default()]);
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_sortition_preparation_rejects_invalid_facts_without_publication() {
        for (case, mutate, expected) in [
            ("malformed", 0_u8, "PBFT_FINALIZE_SORTITION_PERIOD_DATA"),
            (
                "head_mismatch",
                1,
                "PBFT_FINALIZE_SORTITION_CHAIN_HEAD_PERIOD_MISMATCH",
            ),
            (
                "pivot_mismatch",
                2,
                "PBFT_FINALIZE_SORTITION_PIVOT_MISMATCH",
            ),
            (
                "head_overflow",
                3,
                "PBFT_FINALIZE_SORTITION_CHAIN_SIZE_OVERFLOW",
            ),
        ] {
            let (path, manager, dag) =
                finalization_sortition_services(&format!("pbft_manager_sortition_{case}"));
            let mut write_set = finalization_sortition_write_set(
                1,
                false,
                finalization_period_data(H256::repeat_byte(4), 0, &[]),
            );
            let mut runtime = manager.lock();
            match mutate {
                0 => write_set.period_data_rlp = vec![0xc0],
                1 => write_set.block_period = 2,
                2 => write_set.null_anchor = true,
                3 => {
                    runtime
                        .chain
                        .write()
                        .expect("PBFT chain lock remains healthy")
                        .state =
                        crate::pbft_chain::PbftChain::new(crate::pbft_chain::PbftChainHead {
                            head_hash: H256::repeat_byte(8),
                            size: u64::MAX,
                            non_empty_size: u64::MAX,
                            last_pbft_block_hash: H256::repeat_byte(3),
                            last_non_null_pbft_dag_anchor_hash: H256::repeat_byte(2),
                        })
                        .expect("maximum head is structurally valid");
                }
                _ => unreachable!(),
            }
            let mut stages = vec![PbftFinalizationStorageWriteStage::default()];

            let error =
                prepare_pbft_finalization_sortition(&mut runtime, &dag, &write_set, &mut stages)
                    .expect_err("invalid preparation facts reject");
            assert!(error.to_string().contains(expected), "{case}: {error}");
            assert!(runtime.finalization_sortition_commit_request.is_none());
            assert_eq!(stages, vec![PbftFinalizationStorageWriteStage::default()]);
            drop(runtime);
            drop(manager);
            drop(dag);
            let _ = fs::remove_dir_all(path);
        }
    }

    #[test]
    fn finalization_sortition_commit_advances_native_runtime_and_clears_request() {
        let (path, manager, dag) = finalization_sortition_services("pbft_manager_sortition_commit");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 3, &[2, 4]),
        );
        let mut runtime = manager.lock();
        prepare_pbft_finalization_sortition(
            &mut runtime,
            &dag,
            &write_set,
            &mut vec![PbftFinalizationStorageWriteStage::default()],
        )
        .expect("preparation succeeds");
        install_sortition_commit_runtime(&mut runtime, write_set);

        let step = runtime
            .advance_finalization_sortition_commit(&dag, 0)
            .expect("native commit advances");

        assert_eq!(step.runtime_status, PbftFinalizationRuntimeStatus::Active);
        assert_eq!(
            step.action,
            Some(PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime)
        );
        assert!(runtime.finalization_sortition_commit_request.is_none());
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_sortition_commit_rejects_stale_cursor_before_publication() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_sortition_commit_stale");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        prepare_pbft_finalization_sortition(
            &mut runtime,
            &dag,
            &write_set,
            &mut vec![PbftFinalizationStorageWriteStage::default()],
        )
        .expect("preparation succeeds");
        install_sortition_commit_runtime(&mut runtime, write_set);

        let step = runtime
            .advance_finalization_sortition_commit(&dag, 1)
            .expect("stale cursor returns a runtime step");

        assert_eq!(
            step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionMismatch
        );
        assert_eq!(step.error_code, "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH");
        assert!(runtime.finalization_sortition_commit_request.is_some());
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_sortition_commit_preserves_fatal_prefix_on_preview_drift() {
        let (path, manager, dag) =
            finalization_sortition_services_with_interval("pbft_manager_sortition_commit_drift", 1);
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        prepare_pbft_finalization_sortition(
            &mut runtime,
            &dag,
            &write_set,
            &mut vec![PbftFinalizationStorageWriteStage::default()],
        )
        .expect("preparation succeeds");
        let mut retained = runtime
            .finalization_sortition_commit_request
            .expect("commit request retained");
        assert!(retained.expected_change.is_some());
        retained.expected_change = None;
        runtime.finalization_sortition_commit_request = Some(retained);
        install_sortition_commit_runtime(&mut runtime, write_set);

        let error = runtime
            .advance_finalization_sortition_commit(&dag, 0)
            .expect_err("drift is a fatal invariant");

        assert!(
            error
                .to_string()
                .starts_with("PBFT_FINALIZE_POST_STORAGE_SORTITION_INVARIANT")
        );
        assert!(runtime.finalization_sortition_commit_request.is_some());
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_reward_votes_reset_advances_with_matching_generation() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_reward_reset_advance");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        install_reward_votes_reset_runtime(&mut runtime, write_set);
        let generation = commit_reward_votes_reset_generation(&runtime);
        runtime.finalization_reward_votes_reset_generation = generation;
        let verified_votes = reward_vote_cursor_commit_stub(
            crate::pbft_vote_runtime::RewardVoteCursor {
                period: 0,
                round: 0,
                step: 0,
                block_hash: H256::zero(),
            },
            generation,
        );

        let step = runtime
            .advance_finalization_reward_votes_reset(&verified_votes, 0)
            .expect("matching reset report advances");

        assert_eq!(step.runtime_status, PbftFinalizationRuntimeStatus::Active);
        assert_eq!(
            step.action,
            Some(PbftFinalizationRuntimeAction::SetDagBlockOrder)
        );
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_reward_votes_reset_rejects_invalid_provenance() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_reward_reset_provenance");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        install_reward_votes_reset_runtime(&mut runtime, write_set);
        let generation = commit_reward_votes_reset_generation(&runtime);
        runtime.finalization_reward_votes_reset_generation = generation;
        let verified_votes = reward_vote_cursor_commit_stub(
            crate::pbft_vote_runtime::RewardVoteCursor {
                period: 0,
                round: 0,
                step: 0,
                block_hash: H256::zero(),
            },
            generation.wrapping_add(1),
        );

        let step = runtime
            .advance_finalization_reward_votes_reset(&verified_votes, 0)
            .expect("invalid provenance returns failed step");

        assert_eq!(
            step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionFailed
        );
        assert_eq!(
            step.error_code,
            "PBFT_FINALIZE_LIVE_MUTATION_REWARD_VOTES_RESET_PROVENANCE_MISMATCH"
        );
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_reward_votes_reset_rejects_wrong_action_before_publication() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_reward_reset_wrong_action");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        install_reward_votes_reset_runtime(&mut runtime, write_set);
        runtime
            .finalization_runtime_session
            .as_mut()
            .expect("reward runtime installed")
            .actions[0] = PbftFinalizationRuntimeAction::SetDagBlockOrder;
        let verified_votes = reward_vote_cursor_commit_stub(
            crate::pbft_vote_runtime::RewardVoteCursor {
                period: 0,
                round: 0,
                step: 0,
                block_hash: H256::zero(),
            },
            1,
        );

        let step = runtime
            .advance_finalization_reward_votes_reset(&verified_votes, 0)
            .expect("wrong action returns a terminal step");

        assert_eq!(
            step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionFailed
        );
        assert_eq!(step.error_code, "PBFT_FINALIZE_RUNTIME_ACTION_MISMATCH");
        assert!(
            verified_votes
                .request
                .lock()
                .expect("request lock")
                .is_none()
        );
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_reward_votes_reset_rejects_metadata_mismatch() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_reward_reset_metadata");
        let write_set = finalization_sortition_write_set(
            1,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        install_reward_votes_reset_runtime(&mut runtime, write_set);
        let generation = commit_reward_votes_reset_generation(&runtime);
        runtime.finalization_reward_votes_reset_generation = generation;
        let verified_votes = reward_vote_cursor_commit_stub(
            crate::pbft_vote_runtime::RewardVoteCursor {
                period: 0,
                round: 0,
                step: 0,
                block_hash: H256::repeat_byte(9),
            },
            generation,
        );

        let step = runtime
            .advance_finalization_reward_votes_reset(&verified_votes, 0)
            .expect("metadata mismatch returns failed step");

        assert_eq!(
            step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionFailed
        );
        assert_eq!(
            step.error_code,
            "PBFT_FINALIZE_LIVE_MUTATION_REWARD_VOTES_METADATA_MISMATCH"
        );
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_transaction_status_advances_from_retained_period_data() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_transaction_status_advance");
        let mut write_set = finalization_sortition_write_set(
            7,
            false,
            finalization_period_data(H256::repeat_byte(4), 1, &[]),
        );
        write_set.transaction_location_writes = vec![PbftFinalizationPositionedHash {
            hash: H256::repeat_byte(9),
            position: 0,
        }];
        let retained_period_data = write_set.period_data_rlp.clone();
        let mut runtime = manager.lock();
        install_transaction_status_runtime(&mut runtime, write_set);
        let account_facts = vec![
            crate::transaction_service::TransactionServiceAccountNonceFact {
                sender: ethereum_types::H160::repeat_byte(3).0,
                account_found: true,
                account_nonce: [4; 32],
            },
        ];
        let transactions = FinalizedTransactionStatusStub {
            accepted_count: 1,
            error: None,
            request: Mutex::new(None),
        };

        let step = runtime
            .advance_finalization_transaction_status(&transactions, 0, 42, account_facts.clone())
            .expect("native transaction status advances");

        assert_eq!(step.runtime_status, PbftFinalizationRuntimeStatus::Active);
        assert_eq!(
            step.action,
            Some(PbftFinalizationRuntimeAction::UpdatePbftChain)
        );
        assert_eq!(
            *transactions.request.lock().expect("request lock"),
            Some((7, 42, account_facts, retained_period_data))
        );
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_transaction_status_rejects_count_mismatch() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_transaction_status_count");
        let mut write_set = finalization_sortition_write_set(
            7,
            false,
            finalization_period_data(H256::repeat_byte(4), 1, &[]),
        );
        write_set.transaction_location_writes = vec![PbftFinalizationPositionedHash {
            hash: H256::repeat_byte(9),
            position: 0,
        }];
        let mut runtime = manager.lock();
        install_transaction_status_runtime(&mut runtime, write_set);
        let transactions = FinalizedTransactionStatusStub {
            accepted_count: 0,
            error: None,
            request: Mutex::new(None),
        };

        let step = runtime
            .advance_finalization_transaction_status(&transactions, 0, 42, Vec::new())
            .expect("count mismatch returns a failed runtime step");

        assert_eq!(
            step.runtime_status,
            PbftFinalizationRuntimeStatus::ActionFailed
        );
        assert_eq!(
            step.error_code,
            "PBFT_FINALIZE_LIVE_MUTATION_TRANSACTION_COUNT_MISMATCH"
        );
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_transaction_status_preserves_fatal_native_error_prefix() {
        let (path, manager, dag) =
            finalization_sortition_services("pbft_manager_transaction_status_error");
        let write_set = finalization_sortition_write_set(
            7,
            false,
            finalization_period_data(H256::repeat_byte(4), 0, &[]),
        );
        let mut runtime = manager.lock();
        install_transaction_status_runtime(&mut runtime, write_set);
        let transactions = FinalizedTransactionStatusStub {
            accepted_count: 0,
            error: Some("decode failed"),
            request: Mutex::new(None),
        };

        let error = runtime
            .advance_finalization_transaction_status(&transactions, 0, 42, Vec::new())
            .expect_err("native status failure is a fatal post-storage invariant");

        assert_eq!(
            error.to_string(),
            "PBFT_FINALIZE_POST_STORAGE_TRANSACTION_STATUS_INVARIANT:decode failed"
        );
        let step = crate::pbft_finalize::next_pbft_finalization_runtime_action(
            runtime
                .finalization_runtime_session
                .as_ref()
                .expect("session remains retained"),
        );
        assert_eq!(step.action_index, 0);
        assert_eq!(
            step.action,
            Some(PbftFinalizationRuntimeAction::UpdateFinalizedTransactions)
        );
        drop(runtime);
        drop(manager);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    fn fact(state: PbftManagerRuntimeStateCode) -> PbftManagerRuntimeTickFact {
        PbftManagerRuntimeTickFact {
            tick_id: 42,
            state,
            period: 10,
            round: 2,
            step: 3,
            network_available: true,
            network_pbft_syncing: false,
            has_eligible_wallet: true,
            polling_interval_ms: 100,
        }
    }

    fn report(cursor: u32, action: PbftManagerRuntimeAction) -> PbftManagerRuntimeActionReport {
        PbftManagerRuntimeActionReport {
            cursor,
            action,
            success: true,
            result: PbftManagerRuntimeActionResultCode::StateActionDone,
            go_finish_state: false,
            loop_back_finish_state: false,
            has_eligible_wallet: true,
            has_new_round: false,
            new_round: 0,
            error_code: String::new(),
        }
    }

    fn state_fact(state: PbftManagerRuntimeStateCode) -> PbftManagerStateActionFact {
        PbftManagerStateActionFact {
            state,
            period: 10,
            round: 2,
            step: 3,
            elapsed_round_ms: 250,
            deadline_ms: 1_000,
            current_round_lambda_ms: 100,
            polling_interval_ms: 100,
            has_previous_round_next_null: false,
            has_previous_round_next_value: false,
            previous_round_next_value_hash: [0x11; 32],
            has_current_round_soft_value: false,
            current_round_soft_value_hash: [0x22; 32],
            has_cert_voted_block: false,
            cert_voted_block_hash: [0x33; 32],
            already_next_voted_value: false,
            already_next_voted_null: false,
        }
    }

    fn transition_fact(kind: PbftManagerTransitionKind) -> PbftManagerTransitionFact {
        PbftManagerTransitionFact {
            kind,
            period: 10,
            round: 2,
            step: 3,
            target_round: 4,
            current_round_lambda_ms: 100,
            target_round_lambda_ms: 400,
            default_lambda_ms: 100,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            network_next_voting_step: 0,
            deadline_ms: 1_000,
            polling_interval_ms: 100,
            next_step_time_ms: 900,
            cacti_hardfork: true,
            has_cert_voted_block: true,
            executed_pbft_block: true,
        }
    }

    fn startup_fact(round: u64, step: u64) -> PbftManagerStartupRestoreFact {
        PbftManagerStartupRestoreFact {
            current_period: 10,
            persisted_round: round,
            persisted_step: step,
            cacti_active_at_chain_size: true,
            rounds_count_dynamic_lambda: 7,
            persisted_dynamic_lambda_ms: 1_500,
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
            executed_pbft_block: true,
            already_next_voted_value: true,
            already_next_voted_null: false,
        }
    }

    fn storage_startup_fact() -> PbftManagerStorageStartupFact {
        PbftManagerStorageStartupFact {
            current_period: 10,
            cacti_active_at_chain_size: false,
            genesis_lambda_ms: 1_000,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
            cacti_block: 1,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            deadline_ms: 1_000,
            polling_interval_ms: 100,
        }
    }

    fn proposal_fact() -> PbftManagerProposalInitialFact {
        PbftManagerProposalInitialFact {
            period: 10,
            round: 2,
            previous_pbft_block_hash: H256::from_low_u64_be(100),
            last_period_dag_anchor_hash: H256::from_low_u64_be(1),
            dag_genesis_hash: H256::from_low_u64_be(1),
            dag_blocks_size: 10,
            ghost_path_move_back: 0,
            pbft_gas_limit: 100,
            extra_data_required: false,
            extra_data_available: false,
            final_chain_hash_valid: true,
            final_chain_hash: H256::from_low_u64_be(200),
            wallets: vec![
                PbftManagerProposalWalletFact {
                    wallet_index: 0,
                    dpos_eligible: false,
                    sortition_valid: true,
                },
                PbftManagerProposalWalletFact {
                    wallet_index: 1,
                    dpos_eligible: true,
                    sortition_valid: false,
                },
                PbftManagerProposalWalletFact {
                    wallet_index: 2,
                    dpos_eligible: true,
                    sortition_valid: true,
                },
            ],
            ghost_path: vec![
                H256::from_low_u64_be(1),
                H256::from_low_u64_be(2),
                H256::from_low_u64_be(3),
            ],
            has_non_finalized_fallback: false,
            non_finalized_fallback_hash: H256::zero(),
        }
    }

    fn dag_block(hash: u64, gas_estimation: u64) -> PbftManagerProposalDagBlockFact {
        PbftManagerProposalDagBlockFact {
            hash: H256::from_low_u64_be(hash),
            gas_estimation,
        }
    }

    fn proposal_report(
        anchor: u64,
        blocks: Vec<PbftManagerProposalDagBlockFact>,
    ) -> PbftManagerProposalDagOrderReport {
        PbftManagerProposalDagOrderReport {
            anchor_hash: H256::from_low_u64_be(anchor),
            dag_blocks: blocks,
            order_available: true,
        }
    }

    fn broadcast_fact(round_elapsed_ms: u64, period_elapsed_ms: u64) -> PbftManagerBroadcastFact {
        PbftManagerBroadcastFact {
            round_elapsed_ms,
            period_elapsed_ms,
            current_round_lambda_ms: 100,
            broadcast_lambda_threshold: 20,
            rebroadcast_lambda_threshold: 60,
            broadcast_votes_counter: 1,
            rebroadcast_votes_counter: 1,
            broadcast_reward_votes_counter: 1,
            rebroadcast_reward_votes_counter: 1,
        }
    }

    #[test]
    fn pbft_manager_effect_catalog_has_stable_codes() {
        let effects = [
            PbftManagerEffectKind::ProcessSyncedPbftBlocks,
            PbftManagerEffectKind::BroadcastVotes,
            PbftManagerEffectKind::TryPushCertVotesBlock,
            PbftManagerEffectKind::DetermineNewRound,
            PbftManagerEffectKind::ApplyManagerTransition,
            PbftManagerEffectKind::SleepIneligiblePollingInterval,
            PbftManagerEffectKind::SleepUntilNextStep,
            PbftManagerEffectKind::ConstructProposal,
            PbftManagerEffectKind::ValidateProposedBlock,
            PbftManagerEffectKind::ResolveLeaderBlock,
            PbftManagerEffectKind::GenerateVote,
            PbftManagerEffectKind::PlaceVote,
            PbftManagerEffectKind::GossipVote,
            PbftManagerEffectKind::FinalChainFactOrWait,
            PbftManagerEffectKind::DagFactOrMutation,
            PbftManagerEffectKind::TransactionFactOrMutation,
            PbftManagerEffectKind::PillarFactOrMutation,
            PbftManagerEffectKind::ApplyFinalizationStorage,
            PbftManagerEffectKind::FinalizeFinalChain,
            PbftManagerEffectKind::ApplyDynamicLambda,
            PbftManagerEffectKind::UpdatePbftChain,
            PbftManagerEffectKind::AdvancePeriod,
            PbftManagerEffectKind::ReportPeer,
            PbftManagerEffectKind::ClearCompatibilityCache,
        ];

        for (expected_code, effect) in effects.into_iter().enumerate() {
            assert_eq!(effect.as_u8(), expected_code as u8);
            assert_eq!(PbftManagerEffectKind::from_u8(expected_code as u8), effect);
        }
        assert_eq!(PbftManagerEffectKind::Unknown.as_u8(), 254);
        assert_eq!(
            PbftManagerEffectKind::from_u8(200),
            PbftManagerEffectKind::Unknown
        );
    }

    #[test]
    fn pbft_manager_effect_catalog_marks_external_boundaries() {
        for effect in [
            PbftManagerEffectKind::BroadcastVotes,
            PbftManagerEffectKind::GossipVote,
            PbftManagerEffectKind::FinalChainFactOrWait,
            PbftManagerEffectKind::FinalizeFinalChain,
            PbftManagerEffectKind::ReportPeer,
            PbftManagerEffectKind::SleepIneligiblePollingInterval,
            PbftManagerEffectKind::SleepUntilNextStep,
        ] {
            assert!(
                effect.is_external_boundary(),
                "{effect:?} should stay external"
            );
        }

        for effect in [
            PbftManagerEffectKind::ConstructProposal,
            PbftManagerEffectKind::ValidateProposedBlock,
            PbftManagerEffectKind::ApplyFinalizationStorage,
            PbftManagerEffectKind::ApplyDynamicLambda,
            PbftManagerEffectKind::AdvancePeriod,
        ] {
            assert!(
                !effect.is_external_boundary(),
                "{effect:?} should remain in PBFT ownership scope"
            );
        }
    }

    #[test]
    fn proposal_session_builds_null_anchor_when_ghost_is_empty() {
        let mut fact = proposal_fact();
        fact.ghost_path.clear();
        let mut session = create_pbft_manager_proposal_session(fact);

        let step = next_pbft_manager_proposal_session(&mut session);

        assert_eq!(step.action, PbftManagerProposalAction::BuildProposal);
        assert_eq!(step.status, PbftManagerProposalStatus::BuildReady);
        assert!(step.selected_null_anchor);
        assert_eq!(step.anchor_hash, H256::zero());
        assert_eq!(step.order_hash, H256::zero());
        assert_eq!(step.eligible_wallet_indices, vec![2]);
    }

    #[test]
    fn proposal_session_skips_when_no_wallet_is_eligible() {
        let mut fact = proposal_fact();
        for wallet in &mut fact.wallets {
            wallet.sortition_valid = false;
        }
        let mut session = create_pbft_manager_proposal_session(fact);

        let step = next_pbft_manager_proposal_session(&mut session);

        assert_eq!(step.action, PbftManagerProposalAction::SkipProposal);
        assert_eq!(step.status, PbftManagerProposalStatus::NoEligibleWallet);
    }

    #[test]
    fn proposal_session_skips_missing_required_facts() {
        let mut final_chain_fact = proposal_fact();
        final_chain_fact.final_chain_hash_valid = false;
        let mut final_chain_session = create_pbft_manager_proposal_session(final_chain_fact);
        assert_eq!(
            next_pbft_manager_proposal_session(&mut final_chain_session).status,
            PbftManagerProposalStatus::MissingFinalChainHash
        );

        let mut extra_data_fact = proposal_fact();
        extra_data_fact.extra_data_required = true;
        extra_data_fact.extra_data_available = false;
        let mut extra_data_session = create_pbft_manager_proposal_session(extra_data_fact);
        assert_eq!(
            next_pbft_manager_proposal_session(&mut extra_data_session).status,
            PbftManagerProposalStatus::MissingExtraData
        );
    }

    #[test]
    fn proposal_session_requests_dag_order_and_computes_order_hash() {
        let mut session = create_pbft_manager_proposal_session(proposal_fact());

        let request = next_pbft_manager_proposal_session(&mut session);
        assert_eq!(request.action, PbftManagerProposalAction::RequestDagOrder);
        assert_eq!(request.requested_anchor_hash, H256::from_low_u64_be(3));

        let build = report_pbft_manager_proposal_dag_order(
            &mut session,
            proposal_report(3, vec![dag_block(2, 10), dag_block(3, 10)]),
        );

        assert_eq!(build.action, PbftManagerProposalAction::BuildProposal);
        assert_eq!(build.anchor_hash, H256::from_low_u64_be(3));
        assert_eq!(build.dag_blocks_included, 2);
        assert_ne!(build.order_hash, H256::zero());
        assert_eq!(build.final_chain_hash, H256::from_low_u64_be(200));
    }

    #[test]
    fn proposal_session_recomputes_order_when_gas_clipping_changes_anchor() {
        let mut fact = proposal_fact();
        fact.pbft_gas_limit = 50;
        let mut session = create_pbft_manager_proposal_session(fact);

        let request = next_pbft_manager_proposal_session(&mut session);
        assert_eq!(request.requested_anchor_hash, H256::from_low_u64_be(3));

        let recompute = report_pbft_manager_proposal_dag_order(
            &mut session,
            proposal_report(3, vec![dag_block(2, 40), dag_block(3, 40)]),
        );
        assert_eq!(recompute.action, PbftManagerProposalAction::RequestDagOrder);
        assert_eq!(recompute.requested_anchor_hash, H256::from_low_u64_be(2));

        let build = report_pbft_manager_proposal_dag_order(
            &mut session,
            proposal_report(2, vec![dag_block(2, 40)]),
        );
        assert_eq!(build.action, PbftManagerProposalAction::BuildProposal);
        assert_eq!(build.anchor_hash, H256::from_low_u64_be(2));
        assert_eq!(build.dag_blocks_included, 1);
    }

    #[test]
    fn proposal_session_rejects_missing_or_mismatched_dag_order() {
        let mut missing_session = create_pbft_manager_proposal_session(proposal_fact());
        let request = next_pbft_manager_proposal_session(&mut missing_session);
        let missing = report_pbft_manager_proposal_dag_order(
            &mut missing_session,
            PbftManagerProposalDagOrderReport {
                anchor_hash: request.requested_anchor_hash,
                dag_blocks: Vec::new(),
                order_available: false,
            },
        );
        assert_eq!(missing.status, PbftManagerProposalStatus::MissingDagOrder);

        let mut mismatch_session = create_pbft_manager_proposal_session(proposal_fact());
        let _ = next_pbft_manager_proposal_session(&mut mismatch_session);
        let mismatch = report_pbft_manager_proposal_dag_order(
            &mut mismatch_session,
            proposal_report(9, vec![dag_block(9, 1)]),
        );
        assert_eq!(
            mismatch.status,
            PbftManagerProposalStatus::InvalidBridgeFacts
        );
    }

    #[test]
    fn broadcast_planner_selects_round_broadcast() {
        let plan = plan_pbft_manager_broadcast(broadcast_fact(2_100, 0));

        assert_eq!(plan.status, PbftManagerBroadcastStatus::Ready);
        assert_eq!(plan.action, PbftManagerBroadcastAction::RoundVotes);
        assert!(!plan.rebroadcast);
        assert_eq!(plan.next_broadcast_votes_counter, 2);
        assert_eq!(plan.next_rebroadcast_votes_counter, 1);
    }

    #[test]
    fn broadcast_planner_prioritizes_round_rebroadcast() {
        let plan = plan_pbft_manager_broadcast(broadcast_fact(6_100, 10_000));

        assert_eq!(plan.status, PbftManagerBroadcastStatus::Ready);
        assert_eq!(plan.action, PbftManagerBroadcastAction::RoundVotes);
        assert!(plan.rebroadcast);
        assert_eq!(plan.next_broadcast_votes_counter, 2);
        assert_eq!(plan.next_rebroadcast_votes_counter, 2);
        assert_eq!(plan.next_broadcast_reward_votes_counter, 1);
    }

    #[test]
    fn broadcast_planner_selects_period_vote_branches() {
        let rebroadcast = plan_pbft_manager_broadcast(broadcast_fact(0, 6_100));
        assert_eq!(rebroadcast.action, PbftManagerBroadcastAction::PeriodVotes);
        assert!(rebroadcast.rebroadcast);
        assert_eq!(rebroadcast.next_broadcast_reward_votes_counter, 2);
        assert_eq!(rebroadcast.next_rebroadcast_reward_votes_counter, 2);

        let broadcast = plan_pbft_manager_broadcast(broadcast_fact(0, 2_100));
        assert_eq!(broadcast.action, PbftManagerBroadcastAction::PeriodVotes);
        assert!(!broadcast.rebroadcast);
        assert_eq!(broadcast.next_broadcast_reward_votes_counter, 2);
        assert_eq!(broadcast.next_rebroadcast_reward_votes_counter, 1);
    }

    #[test]
    fn broadcast_planner_noops_and_rejects_invalid_facts() {
        let noop = plan_pbft_manager_broadcast(broadcast_fact(2_000, 2_000));
        assert_eq!(noop.status, PbftManagerBroadcastStatus::Ready);
        assert_eq!(noop.action, PbftManagerBroadcastAction::Noop);

        let mut invalid = broadcast_fact(10_000, 10_000);
        invalid.current_round_lambda_ms = 0;
        let rejected = plan_pbft_manager_broadcast(invalid);
        assert_eq!(rejected.status, PbftManagerBroadcastStatus::InvalidFact);

        let mut overflow = broadcast_fact(10_000, 0);
        overflow.broadcast_votes_counter = u32::MAX;
        let rejected = plan_pbft_manager_broadcast(overflow);
        assert_eq!(rejected.status, PbftManagerBroadcastStatus::InvalidFact);
        assert_eq!(
            rejected.error_code,
            "PBFT_MANAGER_BROADCAST_COUNTER_OVERFLOW"
        );
    }

    #[test]
    fn broadcast_report_gates_counter_updates() {
        let plan = plan_pbft_manager_broadcast(broadcast_fact(2_100, 0));
        let accepted = report_pbft_manager_broadcast(
            plan.clone(),
            PbftManagerBroadcastReport {
                action: PbftManagerBroadcastAction::RoundVotes,
                rebroadcast: false,
                success: true,
                error_code: String::new(),
            },
        );
        assert_eq!(accepted.status, PbftManagerBroadcastStatus::Ready);
        assert!(accepted.apply_counters);
        assert_eq!(accepted.broadcast_votes_counter, 2);

        let failed = report_pbft_manager_broadcast(
            plan.clone(),
            PbftManagerBroadcastReport {
                action: PbftManagerBroadcastAction::RoundVotes,
                rebroadcast: false,
                success: false,
                error_code: "NETWORK_DOWN".to_string(),
            },
        );
        assert_eq!(failed.status, PbftManagerBroadcastStatus::ExecutorFailed);
        assert!(!failed.apply_counters);

        let mismatch = report_pbft_manager_broadcast(
            plan,
            PbftManagerBroadcastReport {
                action: PbftManagerBroadcastAction::PeriodVotes,
                rebroadcast: false,
                success: true,
                error_code: String::new(),
            },
        );
        assert_eq!(mismatch.status, PbftManagerBroadcastStatus::InvalidReport);
        assert!(!mismatch.apply_counters);
    }

    fn finalized_dag_bundle_rlp() -> (Vec<u8>, Vec<H256>) {
        let mut compact_block = RlpStream::new_list(7);
        compact_block.append(&H256::from_low_u64_be(1));
        compact_block.append(&7u64);
        compact_block.append(&123u64);
        compact_block.append(&vec![0x44, 0x55]);
        compact_block.append_list(&vec![H256::from_low_u64_be(2)]);
        compact_block.append(&vec![0x66; 65]);
        compact_block.append(&99u64);

        let mut canonical_block = RlpStream::new_list(8);
        canonical_block.append(&H256::from_low_u64_be(1));
        canonical_block.append(&7u64);
        canonical_block.append(&123u64);
        canonical_block.append(&vec![0x44, 0x55]);
        canonical_block.append_list(&vec![H256::from_low_u64_be(2)]);
        let empty_transactions: Vec<H256> = Vec::new();
        canonical_block.append_list(&empty_transactions);
        canonical_block.append(&vec![0x66; 65]);
        canonical_block.append(&99u64);
        let expected_hash = keccak256(&canonical_block.out());

        let ordered_transaction_hashes = RlpStream::new_list(0);
        let mut transaction_indexes = RlpStream::new_list(1);
        transaction_indexes.begin_list(0);
        let mut compact_blocks = RlpStream::new_list(1);
        compact_blocks.append_raw(&compact_block.out(), 1);

        let mut bundle = RlpStream::new_list(3);
        bundle.append_raw(&ordered_transaction_hashes.out(), 1);
        bundle.append_raw(&transaction_indexes.out(), 1);
        bundle.append_raw(&compact_blocks.out(), 1);

        (bundle.out().to_vec(), vec![expected_hash])
    }

    fn period_data_with_dag_bundle(bundle: &[u8]) -> Vec<u8> {
        let mut period_data = RlpStream::new_list(4);
        period_data.append_empty_data();
        period_data.append_empty_data();
        period_data.append_raw(bundle, 1);
        period_data.begin_list(0);
        period_data.out().to_vec()
    }

    fn drain_actions(mut session: PbftManagerRuntimeSession) -> Vec<PbftManagerRuntimeAction> {
        let mut actions = Vec::new();
        loop {
            let step = next_pbft_manager_runtime_action(&session);
            if !step.has_action {
                break;
            }
            let action = step.action.expect("action is present");
            actions.push(action);
            let mut action_report = report(step.cursor, action);
            action_report.result = match action {
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                | PbftManagerRuntimeAction::TryAdvanceRound => {
                    PbftManagerRuntimeActionResultCode::NoProgressContinue
                }
                PbftManagerRuntimeAction::TransitionToFilter
                | PbftManagerRuntimeAction::TransitionToCertify
                | PbftManagerRuntimeAction::TransitionToFinish
                | PbftManagerRuntimeAction::TransitionToFinishPolling
                | PbftManagerRuntimeAction::LoopBackFinish
                | PbftManagerRuntimeAction::ResetConsensus => {
                    PbftManagerRuntimeActionResultCode::TransitionApplied
                }
                PbftManagerRuntimeAction::SleepUntilNextStep
                | PbftManagerRuntimeAction::SleepIneligiblePollingInterval
                | PbftManagerRuntimeAction::DelayCertifyPoll
                | PbftManagerRuntimeAction::DelayFinishPoll => {
                    PbftManagerRuntimeActionResultCode::SleepApplied
                }
                _ => PbftManagerRuntimeActionResultCode::StateActionDone,
            };
            session = report_pbft_manager_runtime_action(session, action_report);
        }
        actions
    }

    #[test]
    fn value_proposal_tick_orders_prestate_state_transition_and_sleep() {
        let actions = drain_actions(create_pbft_manager_runtime_session(fact(
            PbftManagerRuntimeStateCode::ValueProposal,
        )));

        assert_eq!(
            actions,
            vec![
                PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
                PbftManagerRuntimeAction::MaybeBroadcastVotes,
                PbftManagerRuntimeAction::TryPushCertVotesBlock,
                PbftManagerRuntimeAction::TryAdvanceRound,
                PbftManagerRuntimeAction::RunValueProposal,
                PbftManagerRuntimeAction::TransitionToFilter,
                PbftManagerRuntimeAction::SleepUntilNextStep,
            ]
        );
    }

    #[test]
    fn sleep_until_next_step_returns_wait_before_deadline() {
        let plan = plan_pbft_manager_sleep_until_next_step(PbftManagerSleepFact {
            next_step_time_ms: 1_500,
            round_elapsed_ms: 900,
            step: 2,
        });

        assert!(plan.accepted);
        assert!(plan.should_sleep);
        assert_eq!(plan.sleep_ms, 600);
        assert_eq!(plan.step, 2);
        assert!(plan.error_code.is_empty());
    }

    #[test]
    fn sleep_until_next_step_returns_no_wait_after_deadline() {
        let plan = plan_pbft_manager_sleep_until_next_step(PbftManagerSleepFact {
            next_step_time_ms: 1_500,
            round_elapsed_ms: 1_500,
            step: 3,
        });

        assert!(plan.accepted);
        assert!(!plan.should_sleep);
        assert_eq!(plan.sleep_ms, 0);
        assert_eq!(plan.step, 3);
        assert!(plan.error_code.is_empty());
    }

    #[test]
    fn sleep_until_next_step_preserves_negative_elapsed_wait() {
        let plan = plan_pbft_manager_sleep_until_next_step(PbftManagerSleepFact {
            next_step_time_ms: 1_500,
            round_elapsed_ms: -100,
            step: 4,
        });

        assert!(plan.accepted);
        assert!(plan.should_sleep);
        assert_eq!(plan.sleep_ms, 1_600);
        assert_eq!(plan.step, 4);
        assert!(plan.error_code.is_empty());
    }

    #[test]
    fn finalization_wait_planner_waits_until_delegation_delay_is_covered() {
        let wait = plan_pbft_manager_finalization_wait(PbftManagerFinalizationWaitFact {
            pbft_chain_size: 20,
            final_chain_last_block: 14,
            delegation_delay: 5,
            polling_interval_ms: 100,
        });
        assert!(wait.accepted);
        assert!(wait.should_wait);
        assert_eq!(wait.sleep_ms, 100);
        assert!(wait.error_code.is_empty());

        let ready = plan_pbft_manager_finalization_wait(PbftManagerFinalizationWaitFact {
            pbft_chain_size: 19,
            final_chain_last_block: 14,
            delegation_delay: 5,
            polling_interval_ms: 100,
        });
        assert!(ready.accepted);
        assert!(!ready.should_wait);
        assert_eq!(ready.sleep_ms, 0);
        assert!(ready.error_code.is_empty());
    }

    #[test]
    fn eligible_wallet_period_wait_planner_waits_until_period_matches_chain_size() {
        let wait = plan_pbft_manager_eligible_wallet_period_wait(
            PbftManagerEligibleWalletPeriodWaitFact {
                eligible_wallet_period: 8,
                pbft_chain_size: 10,
                polling_interval_ms: 10,
            },
        );
        assert!(wait.should_wait);
        assert_eq!(wait.sleep_ms, 10);

        let ready = plan_pbft_manager_eligible_wallet_period_wait(
            PbftManagerEligibleWalletPeriodWaitFact {
                eligible_wallet_period: 10,
                pbft_chain_size: 10,
                polling_interval_ms: 10,
            },
        );
        assert!(!ready.should_wait);
        assert_eq!(ready.sleep_ms, 0);
    }

    #[test]
    fn network_syncing_skips_broadcast_and_cert_push_but_keeps_round_check() {
        let mut tick = fact(PbftManagerRuntimeStateCode::Filter);
        tick.network_pbft_syncing = true;
        let actions = drain_actions(create_pbft_manager_runtime_session(tick));

        assert_eq!(
            actions,
            vec![
                PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
                PbftManagerRuntimeAction::TryAdvanceRound,
                PbftManagerRuntimeAction::RunFilter,
                PbftManagerRuntimeAction::TransitionToCertify,
                PbftManagerRuntimeAction::SleepUntilNextStep,
            ]
        );
    }

    #[test]
    fn cert_push_progress_completes_with_restart_loop() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        for expected in [
            PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
            PbftManagerRuntimeAction::MaybeBroadcastVotes,
        ] {
            let step = next_pbft_manager_runtime_action(&session);
            assert_eq!(step.action, Some(expected));
            session = report_pbft_manager_runtime_action(session, report(step.cursor, expected));
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(
            step.action,
            Some(PbftManagerRuntimeAction::TryPushCertVotesBlock)
        );
        let mut action_report =
            report(step.cursor, PbftManagerRuntimeAction::TryPushCertVotesBlock);
        action_report.result = PbftManagerRuntimeActionResultCode::ProgressRestartLoop;
        session = report_pbft_manager_runtime_action(session, action_report);

        let final_step = next_pbft_manager_runtime_action(&session);
        assert!(final_step.complete);
        assert!(final_step.restart_loop);
    }

    #[test]
    fn advance_round_candidate_emits_reset_effect_and_restarts_after_report() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        for expected in [
            PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
            PbftManagerRuntimeAction::MaybeBroadcastVotes,
            PbftManagerRuntimeAction::TryPushCertVotesBlock,
        ] {
            let step = next_pbft_manager_runtime_action(&session);
            assert_eq!(step.action, Some(expected));
            let mut action_report = report(step.cursor, expected);
            if expected == PbftManagerRuntimeAction::TryPushCertVotesBlock {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(step.action, Some(PbftManagerRuntimeAction::TryAdvanceRound));
        let mut action_report = report(step.cursor, PbftManagerRuntimeAction::TryAdvanceRound);
        action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
        action_report.has_new_round = true;
        action_report.new_round = 5;
        session = report_pbft_manager_runtime_action(session, action_report);

        let reset = next_pbft_manager_runtime_action(&session);
        assert_eq!(reset.action, Some(PbftManagerRuntimeAction::ResetConsensus));
        assert!(reset.has_target_round);
        assert_eq!(reset.target_round, 5);

        let mut reset_report = report(reset.cursor, PbftManagerRuntimeAction::ResetConsensus);
        reset_report.result = PbftManagerRuntimeActionResultCode::TransitionApplied;
        session = report_pbft_manager_runtime_action(session, reset_report);
        let complete = next_pbft_manager_runtime_action(&session);
        assert!(complete.complete);
        assert!(complete.restart_loop);
    }

    #[test]
    fn advance_round_rejects_non_increasing_candidate() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        for expected in [
            PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
            PbftManagerRuntimeAction::MaybeBroadcastVotes,
            PbftManagerRuntimeAction::TryPushCertVotesBlock,
        ] {
            let step = next_pbft_manager_runtime_action(&session);
            assert_eq!(step.action, Some(expected));
            let mut action_report = report(step.cursor, expected);
            if expected == PbftManagerRuntimeAction::TryPushCertVotesBlock {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        let mut action_report = report(step.cursor, PbftManagerRuntimeAction::TryAdvanceRound);
        action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
        action_report.has_new_round = true;
        action_report.new_round = 2;
        session = report_pbft_manager_runtime_action(session, action_report);

        let failed = next_pbft_manager_runtime_action(&session);
        assert_eq!(failed.status, PbftManagerRuntimeStatus::InvalidReport);
        assert_eq!(
            failed.error_code,
            "PBFT_MANAGER_ADVANCE_ROUND_NON_INCREASING_ROUND"
        );
    }

    #[test]
    fn ineligible_wallet_path_sleeps_and_restarts_without_state_action() {
        let mut tick = fact(PbftManagerRuntimeStateCode::ValueProposal);
        tick.has_eligible_wallet = false;
        let mut session = create_pbft_manager_runtime_session(tick);

        loop {
            let step = next_pbft_manager_runtime_action(&session);
            if step.action == Some(PbftManagerRuntimeAction::SleepIneligiblePollingInterval) {
                assert_eq!(step.sleep_ms, tick.polling_interval_ms);
                let mut action_report = report(
                    step.cursor,
                    PbftManagerRuntimeAction::SleepIneligiblePollingInterval,
                );
                action_report.result = PbftManagerRuntimeActionResultCode::SleepApplied;
                session = report_pbft_manager_runtime_action(session, action_report);
                break;
            }
            let action = step.action.expect("action");
            let mut action_report = report(step.cursor, action);
            if matches!(
                action,
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                    | PbftManagerRuntimeAction::TryAdvanceRound
            ) {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            if action == PbftManagerRuntimeAction::TryAdvanceRound {
                action_report.has_eligible_wallet = false;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let final_step = next_pbft_manager_runtime_action(&session);
        assert!(final_step.complete);
        assert!(final_step.restart_loop);
    }

    #[test]
    fn certify_branch_uses_reported_go_finish_flag() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Certify));
        loop {
            let step = next_pbft_manager_runtime_action(&session);
            let action = step.action.expect("action");
            if action == PbftManagerRuntimeAction::RunCertify {
                let mut action_report = report(step.cursor, action);
                action_report.go_finish_state = true;
                session = report_pbft_manager_runtime_action(session, action_report);
                break;
            }
            let mut action_report = report(step.cursor, action);
            if matches!(
                action,
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                    | PbftManagerRuntimeAction::TryAdvanceRound
            ) {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(
            step.action,
            Some(PbftManagerRuntimeAction::TransitionToFinish)
        );
    }

    #[test]
    fn second_finish_branch_uses_reported_loopback_flag() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::FinishPolling));
        loop {
            let step = next_pbft_manager_runtime_action(&session);
            let action = step.action.expect("action");
            if action == PbftManagerRuntimeAction::RunSecondFinish {
                let mut action_report = report(step.cursor, action);
                action_report.loop_back_finish_state = true;
                session = report_pbft_manager_runtime_action(session, action_report);
                break;
            }
            let mut action_report = report(step.cursor, action);
            if matches!(
                action,
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                    | PbftManagerRuntimeAction::TryAdvanceRound
            ) {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(step.action, Some(PbftManagerRuntimeAction::LoopBackFinish));
    }

    #[test]
    fn cursor_mismatch_and_unknown_state_are_explicit_errors() {
        let session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Unknown));
        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(step.status, PbftManagerRuntimeStatus::RejectedTick);
        assert_eq!(step.error_code, "PBFT_MANAGER_RUNTIME_UNKNOWN_STATE");

        let session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        let step = next_pbft_manager_runtime_action(&session);
        let mut bad_report = report(step.cursor + 1, step.action.expect("action"));
        bad_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
        let session = report_pbft_manager_runtime_action(session, bad_report);
        assert_eq!(session.status, PbftManagerRuntimeStatus::ActionMismatch);
    }

    #[test]
    fn runtime_rejects_unknown_action_and_result_codes() {
        let session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        let step = next_pbft_manager_runtime_action(&session);

        let mut bad_action = report(step.cursor, PbftManagerRuntimeAction::Unknown);
        bad_action.result = PbftManagerRuntimeActionResultCode::StateActionDone;
        let failed = report_pbft_manager_runtime_action(session.clone(), bad_action);
        assert_eq!(failed.status, PbftManagerRuntimeStatus::ActionMismatch);

        let mut bad_result = report(step.cursor, step.action.expect("action"));
        bad_result.result = PbftManagerRuntimeActionResultCode::Unknown;
        let failed = report_pbft_manager_runtime_action(session, bad_result);
        assert_eq!(failed.status, PbftManagerRuntimeStatus::InvalidReport);
        assert_eq!(failed.error_code, "PBFT_MANAGER_RUNTIME_RESULT_MISMATCH");
    }

    #[test]
    fn state_action_planner_selects_value_proposal_starting_value() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::ValueProposal);
        fact.has_previous_round_next_null = true;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(plan.status, PbftManagerStateActionStatus::Ready);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::ProposeNewBlock
        );

        fact.has_previous_round_next_null = false;
        fact.has_previous_round_next_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::ReproposePreviousRoundNextValue
        );
        assert_eq!(plan.primary_hash, [0x11; 32]);
    }

    #[test]
    fn state_action_planner_selects_filter_branches() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Filter);
        fact.round = 1;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::IdentifyLeaderAndSoftVote
        );

        fact.round = 2;
        fact.has_previous_round_next_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue
        );
        assert_eq!(plan.primary_hash, [0x11; 32]);
    }

    #[test]
    fn state_action_planner_selects_certify_timeout_and_vote() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Certify);
        fact.elapsed_round_ms = 950;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(plan.primary_intent, PbftManagerStateActionIntent::GoFinish);
        assert!(plan.go_finish_state);

        fact.elapsed_round_ms = 250;
        fact.has_current_round_soft_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::CertVoteCurrentSoftValue
        );
        assert_eq!(plan.primary_hash, [0x22; 32]);
    }

    #[test]
    fn state_action_planner_selects_finish_votes() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Finish);
        fact.has_cert_voted_block = true;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVoteCertVotedBlock
        );
        assert_eq!(plan.primary_hash, [0x33; 32]);

        fact.has_cert_voted_block = false;
        fact.has_previous_round_next_null = true;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVoteNullBlock
        );

        fact.has_previous_round_next_null = false;
        fact.has_previous_round_next_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVotePreviousRoundValue
        );
        assert_eq!(plan.primary_hash, [0x11; 32]);
    }

    #[test]
    fn state_action_planner_selects_second_finish_primary_secondary_and_loopback() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::FinishPolling);
        fact.current_round_lambda_ms = 1_000;
        fact.has_current_round_soft_value = true;
        fact.has_previous_round_next_null = true;
        fact.elapsed_round_ms = 50;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVoteCurrentSoftValue
        );
        assert_eq!(
            plan.secondary_intent,
            PbftManagerStateActionIntent::NextVoteNullBlock
        );
        assert!(!plan.loop_back_finish_state);

        fact.elapsed_round_ms = 2_000;
        fact.already_next_voted_value = true;
        fact.already_next_voted_null = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(plan.primary_intent, PbftManagerStateActionIntent::Noop);
        assert_eq!(plan.secondary_intent, PbftManagerStateActionIntent::Noop);
        assert!(plan.loop_back_finish_state);
    }

    #[test]
    fn state_action_effect_plan_preserves_single_effect_ordering() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Filter);
        fact.round = 2;
        fact.has_previous_round_next_value = true;

        let plan = plan_pbft_manager_state_action_effects(fact);

        assert_eq!(plan.status, PbftManagerStateActionStatus::Ready);
        assert_eq!(plan.effects.len(), 1);
        assert_eq!(
            plan.effects[0].intent,
            PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue
        );
        assert_eq!(plan.effects[0].hash, [0x11; 32]);
        assert!(plan.effects[0].request_proposed_block_sidecar);
        assert_eq!(plan.effects[0].proposed_block_sidecar_hash, [0x11; 32]);
        assert_eq!(plan.effects[0].proposed_block_sidecar_period, 10);
    }

    #[test]
    fn state_action_effect_plan_preserves_primary_then_secondary_order() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::FinishPolling);
        fact.current_round_lambda_ms = 1_000;
        fact.has_current_round_soft_value = true;
        fact.has_previous_round_next_null = true;

        let plan = plan_pbft_manager_state_action_effects(fact);

        assert_eq!(plan.status, PbftManagerStateActionStatus::Ready);
        assert_eq!(plan.effects.len(), 2);
        assert_eq!(
            plan.effects[0].intent,
            PbftManagerStateActionIntent::NextVoteCurrentSoftValue
        );
        assert!(plan.effects[0].request_proposed_block_sidecar);
        assert_eq!(plan.effects[0].proposed_block_sidecar_hash, [0x22; 32]);
        assert_eq!(plan.effects[0].proposed_block_sidecar_period, 10);
        assert_eq!(
            plan.effects[1].intent,
            PbftManagerStateActionIntent::NextVoteNullBlock
        );
        assert!(!plan.effects[1].request_proposed_block_sidecar);
        assert_eq!(plan.effects[1].proposed_block_sidecar_hash, [0; 32]);
        assert_eq!(plan.effects[1].proposed_block_sidecar_period, 0);
    }

    #[test]
    fn state_action_effect_plan_allows_noop_with_flags() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::FinishPolling);
        fact.current_round_lambda_ms = 1_000;
        fact.already_next_voted_value = true;
        fact.already_next_voted_null = true;
        fact.elapsed_round_ms = 2_000;

        let plan = plan_pbft_manager_state_action_effects(fact);

        assert_eq!(plan.status, PbftManagerStateActionStatus::Ready);
        assert!(plan.effects.is_empty());
        assert!(plan.loop_back_finish_state);
    }

    #[test]
    fn state_action_effect_session_advances_only_after_reports() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::FinishPolling);
        fact.current_round_lambda_ms = 1_000;
        fact.has_current_round_soft_value = true;
        fact.has_previous_round_next_null = true;
        let mut session = create_pbft_manager_state_action_effect_session(fact);

        let first = next_pbft_manager_state_action_effect_session(&mut session);
        assert_eq!(first.status, PbftManagerStateActionSessionStatus::Active);
        assert_eq!(
            first.effect.intent,
            PbftManagerStateActionIntent::NextVoteCurrentSoftValue
        );

        let repeated = next_pbft_manager_state_action_effect_session(&mut session);
        assert_eq!(repeated.cursor, first.cursor);
        assert_eq!(repeated.effect, first.effect);

        let second = report_pbft_manager_state_action_effect_session(
            &mut session,
            PbftManagerStateActionEffectReport {
                cursor: first.cursor,
                intent: first.effect.intent,
                result: PbftManagerStateActionEffectResultCode::Applied,
                error_code: String::new(),
            },
        );
        assert_eq!(second.status, PbftManagerStateActionSessionStatus::Active);
        assert_eq!(
            second.effect.intent,
            PbftManagerStateActionIntent::NextVoteNullBlock
        );

        let done = report_pbft_manager_state_action_effect_session(
            &mut session,
            PbftManagerStateActionEffectReport {
                cursor: second.cursor,
                intent: second.effect.intent,
                result: PbftManagerStateActionEffectResultCode::Applied,
                error_code: String::new(),
            },
        );
        assert_eq!(done.status, PbftManagerStateActionSessionStatus::Complete);
        assert!(done.complete);
        assert!(!done.has_effect);
    }

    #[test]
    fn state_action_effect_session_completes_noop_plan() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::FinishPolling);
        fact.current_round_lambda_ms = 1_000;
        fact.already_next_voted_value = true;
        fact.already_next_voted_null = true;
        fact.elapsed_round_ms = 2_000;
        let mut session = create_pbft_manager_state_action_effect_session(fact);

        let step = next_pbft_manager_state_action_effect_session(&mut session);

        assert_eq!(step.status, PbftManagerStateActionSessionStatus::Complete);
        assert!(!step.has_effect);
        assert!(step.loop_back_finish_state);
    }

    #[test]
    fn state_action_effect_session_rejects_mismatched_report() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Filter);
        fact.round = 2;
        fact.has_previous_round_next_value = true;
        let mut session = create_pbft_manager_state_action_effect_session(fact);
        let step = next_pbft_manager_state_action_effect_session(&mut session);

        let failed = report_pbft_manager_state_action_effect_session(
            &mut session,
            PbftManagerStateActionEffectReport {
                cursor: step.cursor + 1,
                intent: step.effect.intent,
                result: PbftManagerStateActionEffectResultCode::Applied,
                error_code: String::new(),
            },
        );

        assert_eq!(
            failed.status,
            PbftManagerStateActionSessionStatus::EffectMismatch
        );
        assert_eq!(
            failed.error_code,
            "PBFT_MANAGER_STATE_ACTION_EFFECT_REPORT_MISMATCH"
        );
    }

    #[test]
    fn state_action_effect_session_stops_on_live_rejection() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Filter);
        fact.round = 2;
        fact.has_previous_round_next_value = true;
        let mut session = create_pbft_manager_state_action_effect_session(fact);
        let step = next_pbft_manager_state_action_effect_session(&mut session);

        let failed = report_pbft_manager_state_action_effect_session(
            &mut session,
            PbftManagerStateActionEffectReport {
                cursor: step.cursor,
                intent: step.effect.intent,
                result: PbftManagerStateActionEffectResultCode::RejectedLiveCheck,
                error_code: "EXECUTOR_NO_BLOCK".to_string(),
            },
        );

        assert_eq!(
            failed.status,
            PbftManagerStateActionSessionStatus::EffectFailed
        );
        assert_eq!(failed.error_code, "EXECUTOR_NO_BLOCK");
    }

    #[test]
    fn state_action_effect_session_treats_no_work_skip_as_success() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::ValueProposal);
        fact.has_previous_round_next_null = true;
        let mut session = create_pbft_manager_state_action_effect_session(fact);
        let step = next_pbft_manager_state_action_effect_session(&mut session);

        let done = report_pbft_manager_state_action_effect_session(
            &mut session,
            PbftManagerStateActionEffectReport {
                cursor: step.cursor,
                intent: step.effect.intent,
                result: PbftManagerStateActionEffectResultCode::SkippedNoWork,
                error_code: String::new(),
            },
        );

        assert_eq!(done.status, PbftManagerStateActionSessionStatus::Complete);
        assert!(done.can_continue);
    }

    #[test]
    fn startup_restore_normalizes_cursor_and_restores_status_flags() {
        let snapshot = restore_pbft_manager_runtime(startup_fact(2, 2));

        assert_eq!(snapshot.status, PbftManagerStartupRestoreStatus::Ready);
        assert_eq!(snapshot.state, PbftManagerRuntimeStateCode::Finish);
        assert_eq!(snapshot.round, 2);
        assert_eq!(snapshot.step, 4);
        assert_eq!(snapshot.current_round_lambda_ms, 500);
        assert_eq!(snapshot.rounds_count_dynamic_lambda, 7);
        assert_eq!(snapshot.dynamic_lambda_ms, 1_500);
        assert!(snapshot.executed_pbft_block);
        assert!(snapshot.already_next_voted_value);
        assert!(!snapshot.already_next_voted_null);
        assert!(snapshot.persist_normalized_step);
        assert_eq!(snapshot.broadcast_votes_counter, 1);
        assert_eq!(snapshot.rebroadcast_votes_counter, 1);
        assert_eq!(snapshot.broadcast_reward_votes_counter, 1);
        assert_eq!(snapshot.rebroadcast_reward_votes_counter, 1);
        assert!(!snapshot.has_cert_voted_block);
        assert_eq!(snapshot.cert_voted_block_hash, H256::zero());
    }

    #[test]
    fn startup_restore_maps_scratch_and_finish_polling_states() {
        let scratch = restore_pbft_manager_runtime(startup_fact(1, 1));
        assert_eq!(scratch.state, PbftManagerRuntimeStateCode::ValueProposal);
        assert_eq!(scratch.step, 1);
        assert!(!scratch.persist_normalized_step);
        assert!(!scratch.reset_second_finish_start);

        let polling = restore_pbft_manager_runtime(startup_fact(4, 5));
        assert_eq!(polling.state, PbftManagerRuntimeStateCode::FinishPolling);
        assert_eq!(polling.step, 5);
        assert!(polling.reset_second_finish_start);
    }

    #[test]
    fn startup_replay_range_planner_selects_final_chain_and_recent_ranges() {
        let plan = plan_pbft_manager_startup_replay_ranges(PbftManagerStartupReplayRangeFact {
            final_chain_last_block: 8,
            pbft_chain_size: 12,
            delegation_delay: 3,
            recently_finalized_factor: 2,
        });

        assert!(plan.accepted);
        assert!(plan.has_finalization_range);
        assert_eq!(plan.finalization_from_period, 9);
        assert_eq!(plan.finalization_to_period, 12);
        assert_eq!(plan.recent_from_period, 6);
        assert_eq!(plan.recent_to_period, 12);

        let caught_up =
            plan_pbft_manager_startup_replay_ranges(PbftManagerStartupReplayRangeFact {
                final_chain_last_block: 12,
                pbft_chain_size: 12,
                delegation_delay: 100,
                recently_finalized_factor: 2,
            });
        assert!(caught_up.accepted);
        assert!(!caught_up.has_finalization_range);
        assert_eq!(caught_up.recent_from_period, 1);
        assert_eq!(caught_up.recent_to_period, 12);
    }

    #[test]
    fn startup_replay_range_planner_rejects_corrupted_heights() {
        let empty = plan_pbft_manager_startup_replay_ranges(PbftManagerStartupReplayRangeFact {
            final_chain_last_block: 0,
            pbft_chain_size: 0,
            delegation_delay: 1,
            recently_finalized_factor: 1,
        });
        assert!(empty.accepted);
        assert!(!empty.has_finalization_range);
        assert_eq!(empty.recent_from_period, 1);
        assert_eq!(empty.recent_to_period, 0);
        assert!(empty.error_code.is_empty());

        let ahead = plan_pbft_manager_startup_replay_ranges(PbftManagerStartupReplayRangeFact {
            final_chain_last_block: 13,
            pbft_chain_size: 12,
            delegation_delay: 1,
            recently_finalized_factor: 1,
        });
        assert!(!ahead.accepted);
        assert_eq!(
            ahead.error_code,
            "PBFT_MANAGER_STARTUP_REPLAY_FINAL_CHAIN_AHEAD"
        );
    }

    #[test]
    fn advance_period_planner_orders_executor_effects_and_runtime_period_commit() {
        let plan = plan_pbft_manager_advance_period_after_reset(12, true, true, true);

        assert!(plan.accepted);
        assert_eq!(plan.finalized_chain_size, 12);
        assert_eq!(plan.new_period, 13);
        assert_eq!(
            plan.actions,
            vec![
                PbftManagerAdvancePeriodAction::ApplyExecutedBlockReset,
                PbftManagerAdvancePeriodAction::SetVoteManagerPeriodRound,
                PbftManagerAdvancePeriodAction::ResetCurrentRoundTimer,
                PbftManagerAdvancePeriodAction::ResetRewardVoteCounters,
                PbftManagerAdvancePeriodAction::ResetPeriodTimer,
                PbftManagerAdvancePeriodAction::UpdateWalletEligibility,
            ]
        );

        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(1, 1)));
        let mut reset_fact = transition_fact(PbftManagerTransitionKind::ResetConsensus);
        reset_fact.target_round = 1;
        let reset_plan = plan_pbft_manager_transition(reset_fact);
        runtime.record_committed_reset(plan.new_period, &reset_plan);

        let wrong_period = runtime.apply_committed_period_advance(plan.new_period + 1);
        assert_eq!(
            wrong_period.status,
            PbftManagerStartupRestoreStatus::InvalidFact
        );
        assert_eq!(runtime.snapshot().period, 10);
        assert!(runtime.plan_advance_period_after_reset(12).accepted);

        let snapshot = runtime.apply_committed_period_advance(plan.new_period);
        assert_eq!(snapshot.status, PbftManagerStartupRestoreStatus::Ready);
        assert_eq!(snapshot.period, 13);
        let consumed = runtime.plan_advance_period_after_reset(12);
        assert!(!consumed.accepted);
        assert_eq!(
            consumed.error_code,
            "PBFT_MANAGER_ADVANCE_PERIOD_RESET_NOT_COMMITTED"
        );

        let rejected = runtime.apply_committed_period_advance(13);
        assert_eq!(
            rejected.status,
            PbftManagerStartupRestoreStatus::InvalidFact
        );
        assert_eq!(
            rejected.error_code,
            "PBFT_MANAGER_ADVANCE_PERIOD_NON_INCREASING_PERIOD"
        );
        assert_eq!(runtime.snapshot().period, 13);
    }

    #[test]
    fn advance_period_action_reports_validate_against_rust_script() {
        let plan = plan_pbft_manager_advance_period_after_reset(12, true, true, true);

        for (action_index, action) in plan.actions.iter().enumerate() {
            let result = validate_pbft_manager_advance_period_action_report(
                &plan,
                PbftManagerAdvancePeriodActionReport {
                    action_index: action_index as u64,
                    action: action.as_u8(),
                    succeeded: true,
                },
            );
            assert_eq!(
                result.status,
                PbftManagerAdvancePeriodActionReportStatus::Accepted
            );
            assert!(result.accepted);
            assert!(result.error_code.is_empty());
        }

        let skipped = validate_pbft_manager_advance_period_action_report(
            &plan,
            PbftManagerAdvancePeriodActionReport {
                action_index: 1,
                action: PbftManagerAdvancePeriodAction::ResetCurrentRoundTimer.as_u8(),
                succeeded: true,
            },
        );
        assert_eq!(
            skipped.status,
            PbftManagerAdvancePeriodActionReportStatus::ActionMismatch
        );
        assert_eq!(
            skipped.error_code,
            "PBFT_MANAGER_ADVANCE_PERIOD_REPORT_ACTION_MISMATCH"
        );

        let failed = validate_pbft_manager_advance_period_action_report(
            &plan,
            PbftManagerAdvancePeriodActionReport {
                action_index: 0,
                action: PbftManagerAdvancePeriodAction::ApplyExecutedBlockReset.as_u8(),
                succeeded: false,
            },
        );
        assert_eq!(
            failed.status,
            PbftManagerAdvancePeriodActionReportStatus::ExecutorRejected
        );

        let out_of_range = validate_pbft_manager_advance_period_action_report(
            &plan,
            PbftManagerAdvancePeriodActionReport {
                action_index: plan.actions.len() as u64,
                action: PbftManagerAdvancePeriodAction::UpdateWalletEligibility.as_u8(),
                succeeded: true,
            },
        );
        assert_eq!(
            out_of_range.status,
            PbftManagerAdvancePeriodActionReportStatus::ActionIndexOutOfRange
        );

        assert!(PbftManagerAdvancePeriodAction::from_u8(7).is_none());
        let removed_action = validate_pbft_manager_advance_period_action_report(
            &plan,
            PbftManagerAdvancePeriodActionReport {
                action_index: (plan.actions.len() - 1) as u64,
                action: 8,
                succeeded: true,
            },
        );
        assert_eq!(
            removed_action.status,
            PbftManagerAdvancePeriodActionReportStatus::UnknownAction
        );
    }

    #[test]
    fn runtime_records_committed_dynamic_lambda_after_storage_acceptance() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(1, 1)));

        let snapshot = runtime.apply_committed_dynamic_lambda(12, 1_250);
        assert_eq!(snapshot.status, PbftManagerStartupRestoreStatus::Ready);
        assert_eq!(snapshot.rounds_count_dynamic_lambda, 12);
        assert_eq!(snapshot.dynamic_lambda_ms, 1_250);
        assert_eq!(runtime.snapshot().dynamic_lambda_ms, 1_250);

        let rejected = runtime.apply_committed_dynamic_lambda(99, 0);
        assert_eq!(
            rejected.status,
            PbftManagerStartupRestoreStatus::InvalidFact
        );
        assert_eq!(rejected.error_code, "PBFT_MANAGER_DYNAMIC_LAMBDA_ZERO");
        assert_eq!(runtime.snapshot().rounds_count_dynamic_lambda, 12);
        assert_eq!(runtime.snapshot().dynamic_lambda_ms, 1_250);
    }

    #[test]
    fn startup_restore_rejects_missing_cacti_dynamic_lambda() {
        let mut fact = startup_fact(1, 1);
        fact.persisted_dynamic_lambda_ms = 1;
        let snapshot = restore_pbft_manager_runtime(fact);

        assert_eq!(
            snapshot.status,
            PbftManagerStartupRestoreStatus::InvalidFact
        );
        assert_eq!(
            snapshot.error_code,
            "PBFT_MANAGER_STARTUP_MISSING_DYNAMIC_LAMBDA"
        );
    }

    #[test]
    fn storage_startup_restore_reads_rust_storage_and_persists_normalized_step() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_storage_startup");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 2)
                .expect("round should persist");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_STEP, 2)
                .expect("step should persist");
            storage
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK, true)
                .expect("executed status should persist");
            storage
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE, true)
                .expect("next-voted status should persist");

            let runtime =
                create_pbft_manager_runtime_from_storage(&storage, storage_startup_fact())
                    .expect("runtime should restore from Rust storage");
            let snapshot = runtime.snapshot();

            assert_eq!(snapshot.status, PbftManagerStartupRestoreStatus::Ready);
            assert_eq!(snapshot.state, PbftManagerRuntimeStateCode::Finish);
            assert_eq!(snapshot.round, 2);
            assert_eq!(snapshot.step, 4);
            assert_eq!(snapshot.current_round_lambda_ms, 1_000);
            assert!(snapshot.executed_pbft_block);
            assert!(snapshot.already_next_voted_value);
            assert!(!snapshot.already_next_voted_null);
            assert!(!snapshot.persist_normalized_step);
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_STEP)
                    .expect("normalized step should load"),
                Some(4),
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn native_service_owns_manager_runtime_lock_domain() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_native_service");
        {
            let storage = Arc::new(
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize"),
            );
            let runtime =
                create_pbft_manager_runtime_from_storage(storage.as_ref(), storage_startup_fact())
                    .expect("runtime should restore from Rust storage");
            let chain = crate::pbft_chain::PbftChainService::restore(storage.clone())
                .expect("PBFT chain should restore");
            let service = PbftManagerService::new(runtime, storage, chain);

            {
                let mut state = service.lock();
                state.finalization_reward_votes_reset_generation = 9;
                state.runtime_session = Some(create_pbft_manager_runtime_session(fact(
                    PbftManagerRuntimeStateCode::Filter,
                )));
            }

            let state = service.lock();
            assert_eq!(state.finalization_reward_votes_reset_generation, 9);
            assert!(state.runtime_session.is_some());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_cursor_field_storage_persists_round_and_step_only() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_cursor_field");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 1)
                .expect("round seed should persist");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_STEP, 1)
                .expect("step seed should persist");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_LAMBDA, 1_500)
                .expect("lambda seed should persist");
            let mut runtime =
                create_pbft_manager_runtime_from_storage(&storage, storage_startup_fact())
                    .expect("runtime should restore from Rust storage");

            apply_pbft_manager_cursor_field_storage(&storage, PBFT_MGR_FIELD_ROUND, 7)
                .expect("round cursor should persist");
            runtime.apply_committed_cursor_field(PBFT_MGR_FIELD_ROUND, 7);
            apply_pbft_manager_cursor_field_storage(&storage, PBFT_MGR_FIELD_STEP, 9)
                .expect("step cursor should persist");
            runtime.apply_committed_cursor_field(PBFT_MGR_FIELD_STEP, 9);
            let err = apply_pbft_manager_cursor_field_storage(&storage, PBFT_MGR_FIELD_LAMBDA, 1)
                .expect_err("dynamic lambda should not use cursor field API");

            let snapshot = runtime.snapshot();
            assert_eq!(snapshot.round, 7);
            assert_eq!(snapshot.step, 9);
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_ROUND)
                    .expect("round should load"),
                Some(7),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_STEP)
                    .expect("step should load"),
                Some(9),
            );
            assert!(
                err.to_string()
                    .contains("unsupported PBFT manager cursor field")
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn cert_voted_block_storage_write_persists_legacy_payload() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_cert_voted_write");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");

            save_cert_voted_block_in_round_storage(&storage, 5, &[0xC0])
                .expect("cert-voted block should persist");
            let err = save_cert_voted_block_in_round_storage(&storage, 6, &[])
                .expect_err("empty PBFT block payload should reject");

            let payload = storage
                .pbft()
                .cert_voted_block_in_round_rlp()
                .expect("cert-voted block should load")
                .expect("cert-voted block should exist");
            let rlp = rlp::Rlp::new(&payload);
            assert_eq!(rlp.item_count().unwrap(), 2);
            assert_eq!(rlp.at(0).unwrap().as_val::<u64>().unwrap(), 5);
            assert_eq!(rlp.at(1).unwrap().as_raw(), &[0xC0]);
            assert_eq!(
                err.to_string(),
                "PBFT_MANAGER_CERT_VOTED_BLOCK_EMPTY_PAYLOAD"
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn startup_replay_period_loader_reads_period_lambda_and_finalized_dag_hashes() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_startup_replay");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            let (bundle, expected_hashes) = finalized_dag_bundle_rlp();
            let period_data = period_data_with_dag_bundle(&bundle);
            storage
                .period()
                .write(12, &period_data)
                .expect("period data should persist");
            storage
                .metadata()
                .write_period_lambda(11, 1_234)
                .expect("period lambda should persist");

            let replay = load_pbft_manager_startup_replay_period(&storage, 12, true)
                .expect("startup replay period should load");

            assert!(replay.found);
            assert_eq!(replay.period_data_rlp, period_data);
            assert_eq!(replay.finalized_dag_hashes, expected_hashes);
            assert_eq!(replay.period_lambda, Some(1_234));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn startup_replay_period_loader_reports_missing_period_data_without_fallback() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_startup_replay_missing");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");

            let replay = load_pbft_manager_startup_replay_period(&storage, 99, true)
                .expect("missing startup replay period should be explicit");

            assert!(!replay.found);
            assert!(replay.period_data_rlp.is_empty());
            assert!(replay.finalized_dag_hashes.is_empty());
            assert_eq!(replay.period_lambda, None);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn storage_startup_restore_rejects_corrupt_rust_storage() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_storage_corrupt_startup");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 0)
                .expect("corrupt round should persist");

            let err = create_pbft_manager_runtime_from_storage(&storage, storage_startup_fact())
                .expect_err("corrupt cursor should reject startup");
            assert_eq!(err.to_string(), "PBFT_MANAGER_STARTUP_INVALID_CURSOR");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn storage_startup_restore_rejects_missing_cacti_lambda_without_mutation() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_missing_cacti_lambda");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 1)
                .expect("round seed should persist");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_STEP, 1)
                .expect("step seed should persist");
            let mut fact = storage_startup_fact();
            fact.cacti_active_at_chain_size = true;

            let error = create_pbft_manager_runtime_from_storage(&storage, fact)
                .expect_err("missing Cacti lambda should reject startup");

            assert!(
                error
                    .to_string()
                    .contains("PBFT_MANAGER_STARTUP_MISSING_DYNAMIC_LAMBDA")
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_STEP)
                    .expect("step should load"),
                Some(1),
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn transition_storage_apply_commits_manager_status_and_own_vote_cleanup() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_transition_storage");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            let own_hash = H256::from([0xAB; 32]);
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 1)
                .expect("round seed should persist");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_STEP, 1)
                .expect("step seed should persist");
            storage
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE, true)
                .expect("soft next status should persist");
            storage
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH, true)
                .expect("null next status should persist");
            storage
                .pbft()
                .write_cert_voted_block_in_round(1, &[0xC0])
                .expect("cert-voted seed should persist");
            storage
                .pbft()
                .write_own_verified_vote(own_hash, &[0xC1])
                .expect("own vote should persist");

            let mut plan = plan_pbft_manager_transition(transition_fact(
                PbftManagerTransitionKind::ResetConsensus,
            ));
            plan.remove_cert_voted_block = true;
            let result = apply_pbft_manager_transition_storage(&storage, &plan, &[own_hash], false)
                .expect("transition storage should return a result");

            assert_eq!(result.status, PbftManagerTransitionStorageStatus::Applied);
            assert_eq!(result.applied_writes, 6);
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_ROUND)
                    .expect("round should load"),
                Some(4),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_STEP)
                    .expect("step should load"),
                Some(1),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE)
                    .expect("soft next status should load"),
                Some(false),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH)
                    .expect("null next status should load"),
                Some(false),
            );
            assert!(
                storage
                    .pbft()
                    .cert_voted_block_in_round_rlp()
                    .expect("cert-voted block should load")
                    .is_none()
            );
            assert!(
                storage
                    .pbft()
                    .own_verified_votes_rlp()
                    .expect("own votes should load")
                    .is_empty()
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn transition_storage_rejects_unexpected_own_vote_hash_without_mutation() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_transition_storage_reject");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 3)
                .expect("round seed should persist");

            let mut plan =
                plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFilter));
            plan.clear_own_votes = false;
            let result = apply_pbft_manager_transition_storage(
                &storage,
                &plan,
                &[H256::from([0xCD; 32])],
                false,
            )
            .expect("transition storage should return a rejection");

            assert_eq!(result.status, PbftManagerTransitionStorageStatus::Rejected);
            assert_eq!(
                result.error_code,
                "PBFT_MANAGER_TRANSITION_STORAGE_UNEXPECTED_OWN_VOTE_HASHES"
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_ROUND)
                    .expect("round should load"),
                Some(3),
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn next_voted_status_storage_persists_only_next_vote_family() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_next_voted_status");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            let mut runtime =
                create_pbft_manager_runtime_from_storage(&storage, storage_startup_fact())
                    .expect("runtime should restore from Rust storage");

            apply_next_voted_status_storage(&storage, PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE)
                .expect("soft next-voted status should persist");
            runtime.apply_committed_next_voted_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE);
            apply_next_voted_status_storage(&storage, PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH)
                .expect("null next-voted status should persist");
            runtime.apply_committed_next_voted_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH);
            let err = apply_next_voted_status_storage(&storage, PBFT_MGR_STATUS_EXECUTED_BLOCK)
                .expect_err("generic PBFT manager status should reject");

            let snapshot = runtime.snapshot();
            assert!(snapshot.already_next_voted_value);
            assert!(snapshot.already_next_voted_null);
            assert_eq!(
                storage
                    .pbft()
                    .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE)
                    .expect("soft status should load"),
                Some(true),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH)
                    .expect("null status should load"),
                Some(true),
            );
            assert_eq!(
                err.to_string(),
                "PBFT_MANAGER_NEXT_VOTED_STATUS_UNSUPPORTED"
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn executed_block_reset_persists_before_runtime_publication() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_executed_reset");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK, true)
                .expect("executed status should persist");
            let mut runtime =
                create_pbft_manager_runtime_from_storage(&storage, storage_startup_fact())
                    .expect("runtime should restore from Rust storage");
            assert!(runtime.snapshot().executed_pbft_block);

            apply_executed_block_reset_storage(&storage)
                .expect("executed-block reset should persist");
            runtime.apply_committed_executed_block_reset();

            assert!(!runtime.snapshot().executed_pbft_block);
            assert_eq!(
                storage
                    .pbft()
                    .manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)
                    .expect("executed status should load"),
                Some(false),
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_snapshot_advances_only_after_committed_transition_report() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(2, 4)));
        let before = runtime.snapshot();
        let rejected = reject_transition_plan(
            PbftManagerTransitionStatus::InvalidFact,
            PbftManagerTransitionKind::ToFilter,
            "rejected",
        );
        runtime.apply_committed_transition(&rejected);
        assert_eq!(runtime.snapshot(), before);

        let plan =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFilter));
        runtime.apply_committed_transition(&plan);
        let after = runtime.snapshot();
        assert_eq!(after.state, PbftManagerRuntimeStateCode::Filter);
        assert_eq!(after.round, 2);
        assert_eq!(after.step, 4);
        assert_eq!(after.current_round_lambda_ms, 100);
        assert_eq!(after.next_step_time_ms, 200);
    }

    #[test]
    fn runtime_records_committed_broadcast_counters() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(1, 1)));

        let snapshot = runtime.apply_committed_broadcast_counters(2, 3, 4, 5);

        assert_eq!(snapshot.status, PbftManagerStartupRestoreStatus::Ready);
        assert_eq!(snapshot.broadcast_votes_counter, 2);
        assert_eq!(snapshot.rebroadcast_votes_counter, 3);
        assert_eq!(snapshot.broadcast_reward_votes_counter, 4);
        assert_eq!(snapshot.rebroadcast_reward_votes_counter, 5);

        let rejected = runtime.apply_committed_broadcast_counters(0, 1, 1, 1);
        assert_eq!(
            rejected.status,
            PbftManagerStartupRestoreStatus::InvalidFact
        );
        assert_eq!(rejected.error_code, "PBFT_MANAGER_BROADCAST_COUNTER_ZERO");
        assert_eq!(runtime.snapshot().broadcast_votes_counter, 2);

        let mut reset_fact = transition_fact(PbftManagerTransitionKind::ResetConsensus);
        reset_fact.target_round = 1;
        let reset_plan = plan_pbft_manager_transition(reset_fact);
        assert!(reset_plan.reset_broadcast_counters);
        runtime.apply_committed_transition(&reset_plan);
        let reset_snapshot = runtime.snapshot();
        assert_eq!(reset_snapshot.broadcast_votes_counter, 1);
        assert_eq!(reset_snapshot.rebroadcast_votes_counter, 1);
        assert_eq!(reset_snapshot.broadcast_reward_votes_counter, 4);
        assert_eq!(reset_snapshot.rebroadcast_reward_votes_counter, 5);
    }

    #[test]
    fn runtime_records_committed_cert_voted_block_metadata() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(1, 1)));
        assert!(!runtime.snapshot().has_cert_voted_block);

        let block_hash = H256::from_low_u64_be(0xC377);
        let snapshot = runtime.apply_committed_cert_voted_block(10, 2, block_hash);

        assert_eq!(snapshot.status, PbftManagerStartupRestoreStatus::Ready);
        assert!(snapshot.has_cert_voted_block);
        assert_eq!(snapshot.cert_voted_block_period, 10);
        assert_eq!(snapshot.cert_voted_block_round, 2);
        assert_eq!(snapshot.cert_voted_block_hash, block_hash);

        let rejected = runtime.apply_committed_cert_voted_block(0, 2, H256::zero());
        assert_eq!(
            rejected.status,
            PbftManagerStartupRestoreStatus::InvalidFact
        );
        assert_eq!(
            rejected.error_code,
            "PBFT_MANAGER_CERT_VOTED_METADATA_INVALID_CURSOR"
        );
        assert_eq!(runtime.snapshot().cert_voted_block_hash, block_hash);

        let mut reset_fact = transition_fact(PbftManagerTransitionKind::ResetConsensus);
        reset_fact.target_round = 1;
        let reset_plan = plan_pbft_manager_transition(reset_fact);
        assert!(reset_plan.remove_cert_voted_block);
        runtime.apply_committed_transition(&reset_plan);
        let reset_snapshot = runtime.snapshot();
        assert!(!reset_snapshot.has_cert_voted_block);
        assert_eq!(reset_snapshot.cert_voted_block_period, 0);
        assert_eq!(reset_snapshot.cert_voted_block_round, 0);
        assert_eq!(reset_snapshot.cert_voted_block_hash, H256::zero());
    }

    #[test]
    fn runtime_records_cached_anchor_dag_order_metadata() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(1, 1)));
        let first_anchor = H256::from_low_u64_be(0xDA60);
        let second_anchor = H256::from_low_u64_be(0xDA61);

        assert!(!runtime.has_cached_anchor_dag_order(first_anchor));

        let record_snapshot = runtime.record_cached_anchor_dag_order(first_anchor);
        assert_eq!(
            record_snapshot.status,
            PbftManagerStartupRestoreStatus::Ready
        );
        assert!(runtime.has_cached_anchor_dag_order(first_anchor));
        assert!(!runtime.has_cached_anchor_dag_order(second_anchor));

        let remove_snapshot = runtime.remove_cached_anchor_dag_order(first_anchor);
        assert_eq!(
            remove_snapshot.status,
            PbftManagerStartupRestoreStatus::Ready
        );
        assert!(!runtime.has_cached_anchor_dag_order(first_anchor));

        runtime.record_cached_anchor_dag_order(first_anchor);
        runtime.record_cached_anchor_dag_order(second_anchor);
        assert!(runtime.has_cached_anchor_dag_order(first_anchor));
        assert!(runtime.has_cached_anchor_dag_order(second_anchor));
        assert_eq!(runtime.cached_anchor_dag_order_count(), 2);

        let clear_snapshot = runtime.clear_cached_anchor_dag_order();
        assert_eq!(
            clear_snapshot.status,
            PbftManagerStartupRestoreStatus::Ready
        );
        assert_eq!(runtime.cached_anchor_dag_order_count(), 0);
        assert!(!runtime.has_cached_anchor_dag_order(first_anchor));
        assert!(!runtime.has_cached_anchor_dag_order(second_anchor));
    }

    #[test]
    fn runtime_snapshot_records_committed_executed_block_reset() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(2, 4)));
        assert!(runtime.snapshot().executed_pbft_block);

        runtime.apply_committed_executed_block_reset();
        let after = runtime.snapshot();

        assert_eq!(after.status, PbftManagerStartupRestoreStatus::Ready);
        assert!(!after.executed_pbft_block);
        assert!(after.error_code.is_empty());
    }

    #[test]
    fn runtime_snapshot_records_finalization_executed_status_from_intent() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(2, 4)));
        runtime.apply_committed_executed_block_reset();

        let after = runtime.apply_committed_finalization_executed_status(true);

        assert_eq!(after.status, PbftManagerStartupRestoreStatus::Ready);
        assert!(after.executed_pbft_block);
        assert!(after.error_code.is_empty());
    }

    #[test]
    fn transition_planner_selects_phase_targets_and_timing() {
        let filter =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFilter));
        assert_eq!(filter.status, PbftManagerTransitionStatus::Ready);
        assert_eq!(filter.new_state, PbftManagerRuntimeStateCode::Filter);
        assert_eq!(filter.new_round, 2);
        assert_eq!(filter.new_step, 4);
        assert_eq!(filter.current_round_lambda_ms, 100);
        assert_eq!(filter.next_step_time_ms, 200);
        assert!(filter.persist_step);

        let certify =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToCertify));
        assert_eq!(certify.new_state, PbftManagerRuntimeStateCode::Certify);
        assert!(certify.print_cert_step_info);

        let finish =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFinish));
        assert_eq!(finish.new_state, PbftManagerRuntimeStateCode::Finish);
        assert_eq!(finish.next_step_time_ms, 1_000);

        let finish_polling = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::ToFinishPolling,
        ));
        assert_eq!(
            finish_polling.new_state,
            PbftManagerRuntimeStateCode::FinishPolling
        );
        assert_eq!(finish_polling.next_step_time_ms, 1_000);
        assert!(finish_polling.reset_next_voted_statuses);
        assert!(finish_polling.reset_second_finish_start);
        assert!(finish_polling.print_second_finish_step_info);

        let delay_certify = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::DelayCertifyPoll,
        ));
        assert_eq!(
            delay_certify.new_state,
            PbftManagerRuntimeStateCode::Certify
        );
        assert_eq!(delay_certify.new_step, 3);
        assert_eq!(delay_certify.next_step_time_ms, 1_000);
        assert!(!delay_certify.persist_step);

        let delay_finish = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::DelayFinishPoll,
        ));
        assert_eq!(
            delay_finish.new_state,
            PbftManagerRuntimeStateCode::FinishPolling
        );
        assert_eq!(delay_finish.new_step, 3);
        assert_eq!(delay_finish.next_step_time_ms, 1_000);
        assert!(!delay_finish.persist_step);
    }

    #[test]
    fn transition_planner_selects_reset_effects() {
        let reset = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::ResetConsensus,
        ));
        assert_eq!(reset.status, PbftManagerTransitionStatus::Ready);
        assert_eq!(reset.new_state, PbftManagerRuntimeStateCode::ValueProposal);
        assert_eq!(reset.new_round, 4);
        assert_eq!(reset.new_step, 1);
        assert_eq!(reset.current_round_lambda_ms, 400);
        assert!(reset.persist_round);
        assert!(reset.persist_step);
        assert!(reset.reset_next_voted_statuses);
        assert!(reset.remove_cert_voted_block);
        assert!(reset.clear_own_votes);
        assert!(reset.clear_broadcasted_votes);
        assert!(reset.reset_broadcast_counters);
        assert!(reset.reset_executed_block_status);
        assert!(reset.set_vote_manager_period_round);
        assert!(reset.reset_current_round_start);
    }

    #[test]
    fn transition_planner_applies_finish_loopback_and_lambda_backoff() {
        let mut fact = transition_fact(PbftManagerTransitionKind::LoopBackFinish);
        fact.step = 12;
        fact.current_round_lambda_ms = 100;
        fact.next_step_time_ms = 900;
        let plan = plan_pbft_manager_transition(fact);

        assert_eq!(plan.status, PbftManagerTransitionStatus::Ready);
        assert_eq!(plan.new_state, PbftManagerRuntimeStateCode::Finish);
        assert_eq!(plan.new_step, 13);
        assert_eq!(plan.current_round_lambda_ms, 200);
        assert_eq!(plan.next_step_time_ms, 1_000);
        assert!(plan.reset_next_voted_statuses);
    }

    #[test]
    fn transition_planner_resets_lambda_when_network_is_far_ahead() {
        let mut fact = transition_fact(PbftManagerTransitionKind::LoopBackFinish);
        fact.step = 14;
        fact.current_round_lambda_ms = 800;
        fact.network_next_voting_step = 24;
        let plan = plan_pbft_manager_transition(fact);

        assert_eq!(plan.new_step, 15);
        assert_eq!(plan.current_round_lambda_ms, 100);
    }

    #[test]
    fn transition_and_advance_planners_reject_invalid_facts() {
        let mut invalid = transition_fact(PbftManagerTransitionKind::ToFilter);
        invalid.step = 0;
        let plan = plan_pbft_manager_transition(invalid);
        assert_eq!(plan.status, PbftManagerTransitionStatus::InvalidFact);
        assert_eq!(plan.error_code, "PBFT_MANAGER_TRANSITION_INVALID_CURSOR");

        let no_candidate = plan_pbft_manager_advance_round(PbftManagerAdvanceRoundFact {
            period: 10,
            current_round: 2,
            has_new_round: false,
            new_round: 0,
        });
        assert_eq!(no_candidate.status, PbftManagerTransitionStatus::Ready);
        assert!(!no_candidate.should_advance);

        let invalid_round = plan_pbft_manager_advance_round(PbftManagerAdvanceRoundFact {
            period: 10,
            current_round: 2,
            has_new_round: true,
            new_round: 2,
        });
        assert_eq!(
            invalid_round.status,
            PbftManagerTransitionStatus::InvalidFact
        );
        assert_eq!(
            invalid_round.error_code,
            "PBFT_MANAGER_ADVANCE_ROUND_NON_INCREASING_ROUND"
        );
    }

    #[test]
    fn runtime_rejects_unneeded_network_step_without_mutation() {
        let runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(1, 1)));
        let before = runtime.snapshot();

        let plan = runtime.plan_lifecycle_transition(PbftManagerLifecycleTransitionRequest {
            kind: PbftManagerTransitionKind::ToFilter,
            target_period: before.period,
            target_round: before.round,
            has_network_next_voting_step: true,
            network_next_voting_step: 7,
        });

        assert_eq!(plan.status, PbftManagerTransitionStatus::InvalidFact);
        assert_eq!(
            plan.error_code,
            "PBFT_MANAGER_TRANSITION_NETWORK_STEP_PRESENCE_MISMATCH"
        );
        assert_eq!(runtime.snapshot(), before);
    }

    #[test]
    fn leader_selection_prefers_lowest_ranked_non_null_candidate() {
        let mut high_rank = leader_candidate(1, 1, PbftManagerLeaderCandidateStatus::Ready, 9);
        high_rank.weight = 2;
        let low_rank = leader_candidate(2, 2, PbftManagerLeaderCandidateStatus::Ready, 10);
        let null_anchor = leader_candidate(3, 3, PbftManagerLeaderCandidateStatus::Ready, 0);

        let plan = plan_pbft_manager_leader_selection(vec![
            high_rank.clone(),
            low_rank.clone(),
            null_anchor,
        ]);
        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert!(plan.selected);
        assert!(!plan.selected_from_null_anchor);

        let high_rank_hash = pbft_manager_proposal_rank_hash(
            high_rank.credential,
            high_rank.voter_public_key,
            high_rank.weight,
        )
        .unwrap();
        let low_rank_hash =
            pbft_manager_proposal_rank_hash(low_rank.credential, low_rank.voter_public_key, 1)
                .unwrap();
        let expected = if high_rank_hash < low_rank_hash {
            high_rank
        } else {
            low_rank
        };
        assert_eq!(plan.selected_vote_hash, expected.vote_hash);
        assert_eq!(plan.selected_block_hash, expected.block_hash);
    }

    #[test]
    fn leader_selection_uses_null_anchor_only_as_fallback() {
        let invalid = leader_candidate(
            1,
            1,
            PbftManagerLeaderCandidateStatus::BlockMissingOrInvalid,
            8,
        );
        let in_chain = leader_candidate(2, 2, PbftManagerLeaderCandidateStatus::BlockInChain, 9);
        let null_anchor = leader_candidate(3, 3, PbftManagerLeaderCandidateStatus::Ready, 0);

        let plan = plan_pbft_manager_leader_selection(vec![invalid, null_anchor.clone(), in_chain]);
        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert!(plan.selected_from_null_anchor);
        assert_eq!(plan.selected_vote_hash, null_anchor.vote_hash);
    }

    #[test]
    fn leader_selection_keeps_last_duplicate_rank_candidate() {
        let first = leader_candidate(1, 1, PbftManagerLeaderCandidateStatus::Ready, 5);
        let mut second = leader_candidate(2, 2, PbftManagerLeaderCandidateStatus::Ready, 6);
        second.credential = first.credential;
        second.voter_public_key = first.voter_public_key;
        second.weight = first.weight;

        let plan = plan_pbft_manager_leader_selection(vec![first, second.clone()]);
        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert_eq!(plan.selected_vote_hash, second.vote_hash);
    }

    #[test]
    fn leader_selection_rejects_unknown_status_and_skips_invalid_weight() {
        let unknown = leader_candidate(1, 1, PbftManagerLeaderCandidateStatus::Unknown, 1);
        let plan = plan_pbft_manager_leader_selection(vec![unknown]);
        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::InvalidFact);

        let zero_weight = leader_candidate(2, 2, PbftManagerLeaderCandidateStatus::Ready, 2);
        let plan = plan_pbft_manager_leader_selection(vec![PbftManagerLeaderCandidateFact {
            weight: 0,
            ..zero_weight
        }]);
        assert_eq!(
            plan.status,
            PbftManagerLeaderSelectionStatus::NoEligibleCandidate
        );
    }

    #[test]
    fn leader_candidate_planner_derives_statuses_and_mark_valid_commands() {
        let invalid_weight = leader_candidate_input(1, 1);
        let in_chain = PbftManagerLeaderCandidateInputFact {
            block_in_chain: true,
            ..leader_candidate_input(2, 2)
        };
        let missing = PbftManagerLeaderCandidateInputFact {
            proposed_block_found: false,
            ..leader_candidate_input(3, 3)
        };
        let valid = PbftManagerLeaderCandidateInputFact {
            block_validation_status: PbftManagerLeaderBlockValidationStatus::Validated,
            pivot_hash: H256::from([9; 32]),
            ..leader_candidate_input(4, 4)
        };
        let mut invalid_weight = invalid_weight;
        invalid_weight.weight_found = false;

        let plan =
            plan_pbft_manager_leader_candidates(vec![invalid_weight, in_chain, missing, valid]);

        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert!(plan.selected);
        assert_eq!(plan.selected_vote_hash, H256::from([4; 32]));
        assert_eq!(plan.selected_block_hash, H256::from([4; 32]));
        assert_eq!(
            plan.valid_blocks,
            vec![PbftManagerLeaderValidBlockCommand {
                period: 7,
                block_hash: H256::from([4; 32]),
            }]
        );
    }

    #[test]
    fn leader_candidate_planner_keeps_already_valid_blocks_out_of_mark_commands() {
        let fallback = PbftManagerLeaderCandidateInputFact {
            block_validation_status: PbftManagerLeaderBlockValidationStatus::AlreadyValid,
            pivot_hash: H256::zero(),
            ..leader_candidate_input(1, 1)
        };
        let selected = PbftManagerLeaderCandidateInputFact {
            block_validation_status: PbftManagerLeaderBlockValidationStatus::AlreadyValid,
            pivot_hash: H256::from([8; 32]),
            ..leader_candidate_input(2, 2)
        };

        let plan = plan_pbft_manager_leader_candidates(vec![fallback, selected]);

        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert!(!plan.selected_from_null_anchor);
        assert!(plan.valid_blocks.is_empty());
        assert_eq!(plan.selected_block_hash, H256::from([2; 32]));
    }

    #[test]
    fn leader_candidate_planner_rejects_unknown_validation_status() {
        let plan = plan_pbft_manager_leader_candidates(vec![PbftManagerLeaderCandidateInputFact {
            block_validation_status: PbftManagerLeaderBlockValidationStatus::Unknown,
            ..leader_candidate_input(1, 1)
        }]);

        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::InvalidFact);
        assert_eq!(
            plan.error_code,
            "PBFT_MANAGER_LEADER_UNKNOWN_BLOCK_VALIDATION_STATUS"
        );
    }

    #[test]
    fn candidate_admission_plans_lookup_validation_and_mark_valid() {
        let mut fact = candidate_admission_fact();

        let plan = plan_pbft_manager_candidate_admission(fact.clone());
        assert_eq!(
            plan.action,
            PbftManagerCandidateAdmissionAction::RequestLookup
        );
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::LookupRequired
        );
        assert!(!plan.mark_valid);

        fact.lookup_performed = true;
        fact.proposed_block_found = true;
        let plan = plan_pbft_manager_candidate_admission(fact.clone());
        assert_eq!(
            plan.action,
            PbftManagerCandidateAdmissionAction::RequestValidation
        );
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::ValidationRequired
        );

        fact.validation_status = PbftManagerCandidateAdmissionValidationStatus::Valid;
        let plan = plan_pbft_manager_candidate_admission(fact);
        assert_eq!(plan.action, PbftManagerCandidateAdmissionAction::Accept);
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::AcceptedNewlyValidated
        );
        assert!(plan.mark_valid);
    }

    #[test]
    fn candidate_admission_accepts_already_valid_and_rejects_missing() {
        let already_valid = PbftManagerCandidateAdmissionFact {
            lookup_performed: true,
            proposed_block_found: true,
            proposed_block_already_valid: true,
            ..candidate_admission_fact()
        };
        let plan = plan_pbft_manager_candidate_admission(already_valid);
        assert_eq!(plan.action, PbftManagerCandidateAdmissionAction::Accept);
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::AcceptedAlreadyValid
        );
        assert!(!plan.mark_valid);

        let missing = PbftManagerCandidateAdmissionFact {
            lookup_performed: true,
            proposed_block_found: false,
            ..candidate_admission_fact()
        };
        let plan = plan_pbft_manager_candidate_admission(missing);
        assert_eq!(
            plan.action,
            PbftManagerCandidateAdmissionAction::DeferMissingBlock
        );
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::BlockMissing
        );
    }

    #[test]
    fn candidate_admission_rejects_bad_fact_order() {
        let bad = PbftManagerCandidateAdmissionFact {
            lookup_performed: false,
            proposed_block_found: true,
            ..candidate_admission_fact()
        };
        let plan = plan_pbft_manager_candidate_admission(bad);
        assert_eq!(
            plan.action,
            PbftManagerCandidateAdmissionAction::ContractError
        );
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::InvalidBridgeFacts
        );
    }

    #[test]
    fn block_validation_planner_drives_live_checks_in_legacy_order() {
        let mut fact = block_validation_fact();

        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(plan.action, PbftManagerBlockValidationAction::RunCheck);
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckPbftChain
        );

        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidateFinalChainHash
        );

        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckRewardVotes
        );

        fact.reward_votes_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidateExtraData
        );

        fact.extra_data_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckDagOrder
        );

        fact.dag_order_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.dag_weight_check_required = true;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckDagWeight
        );

        fact.dag_weight_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact);
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Accept);
        assert_eq!(plan.status, PbftManagerBlockValidationStatus::Accepted);
    }

    #[test]
    fn block_validation_session_drives_live_checks_in_legacy_order() {
        let mut session = create_pbft_manager_block_validation_session(block_validation_fact());

        let plan = next_pbft_manager_block_validation_session(&mut session);
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckPbftChain
        );

        let plan = report_pbft_manager_block_validation_session_check(
            &mut session,
            PbftManagerBlockValidationFactStatus::Valid,
            false,
        );
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidateFinalChainHash
        );

        let plan = report_pbft_manager_block_validation_session_check(
            &mut session,
            PbftManagerBlockValidationFactStatus::Valid,
            false,
        );
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckRewardVotes
        );

        let plan = report_pbft_manager_block_validation_session_check(
            &mut session,
            PbftManagerBlockValidationFactStatus::Valid,
            false,
        );
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidateExtraData
        );

        let plan = report_pbft_manager_block_validation_session_check(
            &mut session,
            PbftManagerBlockValidationFactStatus::Valid,
            false,
        );
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckDagOrder
        );

        let plan = report_pbft_manager_block_validation_session_check(
            &mut session,
            PbftManagerBlockValidationFactStatus::Valid,
            true,
        );
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckDagWeight
        );

        let plan = report_pbft_manager_block_validation_session_check(
            &mut session,
            PbftManagerBlockValidationFactStatus::Valid,
            false,
        );
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Accept);
        assert_eq!(plan.status, PbftManagerBlockValidationStatus::Accepted);
    }

    #[test]
    fn block_validation_session_supports_final_chain_wait_retry() {
        let mut session = create_pbft_manager_block_validation_session(block_validation_fact());
        let _ = next_pbft_manager_block_validation_session(&mut session);
        let plan = report_pbft_manager_block_validation_session_check(
            &mut session,
            PbftManagerBlockValidationFactStatus::Valid,
            false,
        );
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidateFinalChainHash
        );

        let plan = report_pbft_manager_block_validation_session_check(
            &mut session,
            PbftManagerBlockValidationFactStatus::Missing,
            false,
        );
        assert_eq!(
            plan.action,
            PbftManagerBlockValidationAction::WaitForFinalization
        );

        let plan = report_pbft_manager_block_validation_session_check(
            &mut session,
            PbftManagerBlockValidationFactStatus::NotChecked,
            false,
        );
        assert_eq!(plan.action, PbftManagerBlockValidationAction::RunCheck);
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidateFinalChainHash
        );
    }

    #[test]
    fn block_validation_planner_handles_final_chain_wait_and_rejections() {
        let mut fact = block_validation_fact();
        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Missing;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.action,
            PbftManagerBlockValidationAction::WaitForFinalization
        );
        assert_eq!(
            plan.status,
            PbftManagerBlockValidationStatus::FinalChainHashMissing
        );

        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Invalid;
        let plan = plan_pbft_manager_block_validation(fact);
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Reject);
        assert_eq!(
            plan.status,
            PbftManagerBlockValidationStatus::FinalChainHashInvalid
        );
    }

    #[test]
    fn block_validation_planner_accepts_null_or_cached_anchor_without_dag_checks() {
        let mut fact = block_validation_fact();
        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.reward_votes_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.extra_data_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.pivot_is_null = true;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Accept);

        fact.pivot_is_null = false;
        fact.dag_order_cached = true;
        let plan = plan_pbft_manager_block_validation(fact);
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Accept);
    }

    #[test]
    fn block_validation_planner_can_skip_dag_order_for_sync_context() {
        let mut fact = block_validation_fact();
        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.reward_votes_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.extra_data_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.dag_order_required = false;

        let plan = plan_pbft_manager_block_validation(fact);

        assert_eq!(plan.action, PbftManagerBlockValidationAction::Accept);
        assert_eq!(plan.status, PbftManagerBlockValidationStatus::Accepted);
    }

    #[test]
    fn block_validation_planner_requires_pillar_block_only_when_configured() {
        let mut fact = block_validation_fact();
        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.reward_votes_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.extra_data_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.pillar_block_required = true;
        fact.pillar_block_status = PbftManagerBlockValidationFactStatus::NotChecked;

        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidatePillarBlock
        );

        fact.pillar_block_status = PbftManagerBlockValidationFactStatus::Invalid;
        let plan = plan_pbft_manager_block_validation(fact);
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Reject);
        assert_eq!(
            plan.status,
            PbftManagerBlockValidationStatus::PillarBlockInvalid
        );
    }

    fn leader_candidate(
        id: u8,
        block: u8,
        status: PbftManagerLeaderCandidateStatus,
        pivot: u8,
    ) -> PbftManagerLeaderCandidateFact {
        PbftManagerLeaderCandidateFact {
            vote_hash: H256::from([id; 32]),
            block_hash: H256::from([block; 32]),
            period: 7,
            credential: [id; 64],
            voter_public_key: [id.wrapping_add(11); 64],
            weight: 1,
            status,
            pivot_hash: H256::from([pivot; 32]),
        }
    }

    fn leader_candidate_input(id: u8, block: u8) -> PbftManagerLeaderCandidateInputFact {
        PbftManagerLeaderCandidateInputFact {
            vote_hash: H256::from([id; 32]),
            block_hash: H256::from([block; 32]),
            period: 7,
            credential: [id; 64],
            voter_public_key: [id.wrapping_add(11); 64],
            weight_found: true,
            weight: 1,
            block_in_chain: false,
            proposed_block_found: true,
            block_validation_status: PbftManagerLeaderBlockValidationStatus::Validated,
            pivot_hash: H256::from([block.wrapping_add(20); 32]),
        }
    }

    fn candidate_admission_fact() -> PbftManagerCandidateAdmissionFact {
        PbftManagerCandidateAdmissionFact {
            period: 7,
            block_hash: H256::from([1; 32]),
            lookup_performed: false,
            proposed_block_found: false,
            proposed_block_already_valid: false,
            validation_status: PbftManagerCandidateAdmissionValidationStatus::NotChecked,
        }
    }

    fn block_validation_fact() -> PbftManagerBlockValidationFact {
        PbftManagerBlockValidationFact {
            block_hash: H256::from([1; 32]),
            period: 7,
            pivot_hash: H256::from([2; 32]),
            pivot_is_null: false,
            dag_order_cached: false,
            dag_order_required: true,
            pillar_block_required: false,
            dag_weight_check_required: false,
            pbft_chain_status: PbftManagerBlockValidationFactStatus::NotChecked,
            final_chain_hash_status: PbftManagerBlockValidationFactStatus::NotChecked,
            reward_votes_status: PbftManagerBlockValidationFactStatus::NotChecked,
            extra_data_status: PbftManagerBlockValidationFactStatus::NotChecked,
            pillar_block_status: PbftManagerBlockValidationFactStatus::NotRequired,
            dag_order_status: PbftManagerBlockValidationFactStatus::NotChecked,
            dag_weight_status: PbftManagerBlockValidationFactStatus::NotChecked,
        }
    }
}
