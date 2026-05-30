//! Deterministic PBFT finalization intent planning.
//!
//! This module receives plain, C++-computed facts from the execute/finalize
//! boundary and returns a bridge-safe intent that only captures runtime
//! side-effects. No storage mutation, I/O, scheduling, locking, or DB reads are
//! performed in this planner.
//!
//! Inputs:
//! - `block_period`, `block_prev_hash`, `chain_last_hash`, `chain_last_period`:
//!   used only for deterministic candidate acceptance checks.
//! - `block_in_chain`: if true, the candidate was already written previously.
//! - `pivot_dag_anchor_hash`: determines anchored vs null-anchor behavior.
//! - `has_pillar_block` + `pillar_block_finalized`: controls acceptance of
//!   pillar-linked PBFT blocks in Ficus-era hardfork paths.
//! - certified-vote and dynamic-lambda facts let Rust prepare a storage-write
//!   intent that can be executed natively by Rust in the next slice.
//!
//! Outputs:
//! - `finalize_block`: whether the PBFT block should continue through execute/
//!   finalize side-effects.
//! - `anchor`: null-anchor vs anchored classification.
//! - `executed_pbft_block`: intent for setting the manager's executed flag.
//! - `cleanup`: a bounded cleanup intent used by C++ to schedule deterministic
//!   in-memory/storage-facing updates in a fixed order.
//! - `storage_write_intent`: the PBFT persistence command shape. C++ still
//!   applies the writes in this slice, but Rust owns the decision and facts.
//! - `status`: explicit decision status code for metrics/logging/telemetry.
use ethereum_types::H256;
use std::collections::HashSet;

/// Null-anchor / anchored status reported in a planner plan.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftFinalizationAnchor {
    /// PBFT block has `kNullBlockHash` anchor and should follow null-anchor
    /// finalization semantics.
    Null,
    /// PBFT block has a concrete DAG pivot anchor hash.
    Anchored,
    /// Input encoded an unknown anchor code while coming from bridge payloads.
    Unknown,
}

impl PbftFinalizationAnchor {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Anchored => 1,
            Self::Unknown => 255,
        }
    }

    /// Decodes a bridge code from C++.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Null,
            1 => Self::Anchored,
            _ => Self::Unknown,
        }
    }
}

/// Finalization status result codes used by both Rust and C++.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftFinalizationStatus {
    /// The block is accepted for finalization execution.
    Accepted,
    /// Block is already present in chain / storage.
    BlockAlreadyInChain,
    /// Candidate is stale for the current head period and cannot be finalized.
    StalePeriod,
    /// The candidate prev hash mismatches chain head for a non-stale block.
    PreviousHashMismatch,
    /// A pillar-linked PBFT block requires pillar-finalization input that was not provided.
    PillarDependencyMissing,
    /// A non-duplicate finalization path was called without certified votes.
    EmptyCertVotes,
    /// The sample certified vote does not certify the PBFT block hash.
    CertVoteBlockMismatch,
    /// The caller omitted storage payload facts required for accepted writes.
    StorageFactsIncomplete,
    /// Internal contract error or impossible status in transport facts.
    ContractError,
    /// Unknown status code produced from legacy inputs.
    Unknown,
}

impl PbftFinalizationStatus {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Accepted => 0,
            Self::BlockAlreadyInChain => 1,
            Self::StalePeriod => 2,
            Self::PreviousHashMismatch => 3,
            Self::PillarDependencyMissing => 4,
            Self::EmptyCertVotes => 5,
            Self::CertVoteBlockMismatch => 6,
            Self::StorageFactsIncomplete => 7,
            Self::ContractError => 255,
            Self::Unknown => 254,
        }
    }

    /// Decodes a bridge code from C++.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Accepted,
            1 => Self::BlockAlreadyInChain,
            2 => Self::StalePeriod,
            3 => Self::PreviousHashMismatch,
            4 => Self::PillarDependencyMissing,
            5 => Self::EmptyCertVotes,
            6 => Self::CertVoteBlockMismatch,
            7 => Self::StorageFactsIncomplete,
            255 => Self::ContractError,
            _ => Self::Unknown,
        }
    }
}

/// Ordered runtime-side actions for an accepted PBFT finalization path.
///
/// These actions are stable bridge codes. They describe the sequence the C++
/// shim must still execute while ownership is split between Rust-planned
/// persistence and legacy live objects. The list is intentionally explicit so
/// each side effect can be migrated to Rust one action at a time without hiding
/// a legacy fallback.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftFinalizationRuntimeAction {
    /// Apply primary finalized-period storage writes.
    ApplyPrimaryStorage,
    /// Apply reward-vote reset storage writes in the primary batch.
    ApplyRewardVotesResetStorage,
    /// Apply sortition parameter storage writes in the primary batch.
    ApplySortitionStorage,
    /// Commit reward-vote reset metadata to the live vote manager.
    CommitRewardVotesResetRuntime,
    /// Update finalized DAG block ordering in the live DAG manager.
    SetDagBlockOrder,
    /// Update finalized transaction bookkeeping in the live transaction manager.
    UpdateFinalizedTransactions,
    /// Update the live PBFT chain head.
    UpdatePbftChain,
    /// Clear cached anchor DAG order state.
    ClearAnchorDagCache,
    /// Apply dynamic-lambda live state from the Rust dynamic-lambda planner.
    ApplyDynamicLambda,
    /// Dispatch final-chain finalization for the accepted period.
    FinalizeFinalChain,
    /// Persist the executed-PBFT status after final-chain dispatch.
    PersistExecutedStatus,
    /// Mark the live PBFT manager as having executed the block.
    SetExecutedFlag,
    /// Advance the PBFT manager period.
    AdvancePeriod,
    /// Terminal marker reserved for future fully Rust-owned runtimes.
    Complete,
}

impl PbftFinalizationRuntimeAction {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ApplyPrimaryStorage => 0,
            Self::ApplyRewardVotesResetStorage => 1,
            Self::ApplySortitionStorage => 2,
            Self::CommitRewardVotesResetRuntime => 3,
            Self::SetDagBlockOrder => 4,
            Self::UpdateFinalizedTransactions => 5,
            Self::UpdatePbftChain => 6,
            Self::ClearAnchorDagCache => 7,
            Self::ApplyDynamicLambda => 8,
            Self::FinalizeFinalChain => 9,
            Self::PersistExecutedStatus => 10,
            Self::SetExecutedFlag => 11,
            Self::AdvancePeriod => 12,
            Self::Complete => 13,
        }
    }

    /// Decodes a stable bridge action code from C++.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::ApplyPrimaryStorage),
            1 => Some(Self::ApplyRewardVotesResetStorage),
            2 => Some(Self::ApplySortitionStorage),
            3 => Some(Self::CommitRewardVotesResetRuntime),
            4 => Some(Self::SetDagBlockOrder),
            5 => Some(Self::UpdateFinalizedTransactions),
            6 => Some(Self::UpdatePbftChain),
            7 => Some(Self::ClearAnchorDagCache),
            8 => Some(Self::ApplyDynamicLambda),
            9 => Some(Self::FinalizeFinalChain),
            10 => Some(Self::PersistExecutedStatus),
            11 => Some(Self::SetExecutedFlag),
            12 => Some(Self::AdvancePeriod),
            13 => Some(Self::Complete),
            _ => None,
        }
    }
}

/// Ordered runtime plan derived from an already accepted finalization intent.
///
/// Inputs are the accepted/rejected finalization intent. Outputs are bridge
/// action codes in the order the mixed Rust/C++ runtime must execute them. A
/// rejected intent returns no actions and preserves the rejection status.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationRuntimePlan {
    /// True when the candidate should execute finalization side effects.
    pub finalize_block: bool,
    /// Finalization planner status carried through for telemetry.
    pub status: PbftFinalizationStatus,
    /// Ordered side-effect actions for the shim executor.
    pub actions: Vec<PbftFinalizationRuntimeAction>,
}

/// Validation status for Rust-owned PBFT finalization live-mutation reports.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftFinalizationLiveMutationStatus {
    /// The reported live mutation matches the accepted Rust finalization plan.
    Accepted,
    /// The finalization plan is not an accepted executable plan.
    PlanNotAccepted,
    /// The reported action is not part of the Rust-planned runtime script.
    ActionNotPlanned,
    /// The reported action is not one of the live mutations validated here.
    ActionNotLiveMutation,
    /// Reported block period does not match the accepted finalization plan.
    PeriodMismatch,
    /// Reported PBFT block hash does not match the accepted finalization plan.
    BlockHashMismatch,
    /// Reported DAG anchor does not match the accepted finalization plan.
    AnchorMismatch,
    /// DAG finalized-count proof does not match the planned finalized DAG set.
    DagFinalizedCountMismatch,
    /// Transaction finalized-count proof does not match the planned transaction set.
    TransactionFinalizedCountMismatch,
    /// PBFT chain head after mutation does not match the finalized block.
    PbftChainHeadMismatch,
    /// PBFT chain non-null anchor after mutation does not match the finalized anchor.
    PbftChainAnchorMismatch,
    /// Reward-vote live metadata does not match the Rust-planned reset facts.
    RewardVotesMetadataMismatch,
    /// Stale extra reward votes were not cleared after the reset metadata commit.
    RewardVotesExtraVotesNotCleared,
    /// The report carried an unknown action code.
    UnknownAction,
}

impl PbftFinalizationLiveMutationStatus {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Accepted => 0,
            Self::PlanNotAccepted => 1,
            Self::ActionNotPlanned => 2,
            Self::ActionNotLiveMutation => 3,
            Self::PeriodMismatch => 4,
            Self::BlockHashMismatch => 5,
            Self::AnchorMismatch => 6,
            Self::DagFinalizedCountMismatch => 7,
            Self::TransactionFinalizedCountMismatch => 8,
            Self::PbftChainHeadMismatch => 9,
            Self::PbftChainAnchorMismatch => 10,
            Self::RewardVotesMetadataMismatch => 11,
            Self::RewardVotesExtraVotesNotCleared => 12,
            Self::UnknownAction => 255,
        }
    }
}

/// C++ executor report for PBFT finalization live mutations.
///
/// Inputs are post-action facts gathered from Rust-backed subsystem shims. Rust
/// validates that the observed mutation matches the accepted finalization plan
/// before the PBFT runtime cursor is advanced.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationLiveMutationReport {
    /// Runtime action being reported.
    pub action: PbftFinalizationRuntimeAction,
    /// Finalized PBFT period observed by the executor.
    pub block_period: u64,
    /// Finalized PBFT block hash observed by the executor.
    pub pbft_block_hash: H256,
    /// DAG anchor hash used for this finalization.
    pub anchor_hash: H256,
    /// Number of unique DAG blocks finalized by the Rust-backed DAG runtime.
    pub dag_finalized_count: u64,
    /// Number of transactions accepted by the Rust-backed finalized-status update.
    pub finalized_transaction_count: u64,
    /// PBFT chain size after the Rust-backed PBFT-chain mutation.
    pub pbft_chain_size: u64,
    /// PBFT chain head hash after the Rust-backed PBFT-chain mutation.
    pub pbft_chain_head_hash: H256,
    /// PBFT chain last non-null anchor after the Rust-backed PBFT-chain mutation.
    pub pbft_chain_last_anchor_hash: H256,
    /// Reward-vote period after the Rust-validated live metadata reset.
    pub reward_votes_period: u64,
    /// Reward-vote round after the Rust-validated live metadata reset.
    pub reward_votes_round: u64,
    /// Reward-vote block hash after the Rust-validated live metadata reset.
    pub reward_votes_block_hash: H256,
    /// Number of stale extra reward-vote sidecars remaining after live reset.
    pub reward_votes_extra_count: u64,
}

/// Result of validating one PBFT finalization live-mutation report.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationLiveMutationValidation {
    /// Whether the report is accepted and the runtime cursor may advance.
    pub accepted: bool,
    /// Stable status code for bridge reporting.
    pub status: PbftFinalizationLiveMutationStatus,
    /// Action that was validated.
    pub action: PbftFinalizationRuntimeAction,
    /// Stable error code, empty on success.
    pub error_code: String,
}

/// Durable resume classification for an already-persisted PBFT block.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftFinalizationResumeStatus {
    /// The candidate is not known to finalized-period storage.
    NotPersisted,
    /// Durable finalization state is complete for this block.
    Complete,
    /// Primary finalized-period storage is present, but FinalChain still needs replay.
    NeedsFinalChainReplay,
    /// FinalChain is caught up, but executed-status persistence is still missing.
    NeedsExecutedStatusPersistence,
    /// Required primary finalized-period facts are missing.
    MissingPrimaryFacts,
    /// Primary finalized-period facts conflict with the expected block payload.
    ConflictingPrimaryFacts,
    /// Dynamic-lambda persistence is required before final-chain replay is safe.
    NeedsDynamicLambdaPersistence,
    /// Internal contract error or impossible status in transport facts.
    ContractError,
    /// Unknown status code produced from bridge inputs.
    Unknown,
}

impl PbftFinalizationResumeStatus {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NotPersisted => 0,
            Self::Complete => 1,
            Self::NeedsFinalChainReplay => 2,
            Self::NeedsExecutedStatusPersistence => 3,
            Self::MissingPrimaryFacts => 4,
            Self::ConflictingPrimaryFacts => 5,
            Self::NeedsDynamicLambdaPersistence => 6,
            Self::ContractError => 255,
            Self::Unknown => 254,
        }
    }
}

/// Storage-derived facts used to classify a duplicate PBFT finalization.
///
/// The facts intentionally describe durable storage only. They do not claim
/// whether C++ live side effects such as DAG manager mutation, transaction
/// sidecars, timers, or period advancement completed, because those do not yet
/// have a durable Rust-owned cursor.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftFinalizationResumeFact {
    /// Whether PBFT block-to-period storage says this block was already persisted.
    pub block_in_chain: bool,
    /// Finalized PBFT block period.
    pub block_period: u64,
    /// Highest FinalChain block durably finalized.
    pub final_chain_last_block: u64,
    /// Whether the PBFT block hash maps to the expected period.
    pub pbft_period_mapping_matches: bool,
    /// Whether the period-data row exists and matches the expected canonical RLP.
    pub period_data_matches: bool,
    /// Whether all planned DAG finalized-position indexes exist with expected values.
    pub dag_positions_match: bool,
    /// Whether all planned transaction finalized-location indexes exist with expected values.
    pub transaction_positions_match: bool,
    /// Whether any required primary finalized-period fact is missing.
    pub missing_primary_facts: bool,
    /// Whether any immutable finalized-period fact conflicts with expected values.
    pub conflicting_primary_facts: bool,
    /// Whether dynamic-lambda persistence is required for this block.
    pub dynamic_lambda_required: bool,
    /// Whether required dynamic-lambda period state already matches storage.
    pub dynamic_lambda_persisted: bool,
    /// Whether executed PBFT status already matches the expected value.
    pub executed_status_persisted: bool,
}

/// Rust classification for an already-persisted PBFT finalization candidate.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationResumePlan {
    /// Stable resume status.
    pub status: PbftFinalizationResumeStatus,
    /// True when the duplicate path should not be treated as a blind duplicate.
    pub duplicate_classified: bool,
    /// True when durable state is complete and no replay is needed.
    pub complete: bool,
    /// Ordered durable replay actions that are safe to reason about from storage facts.
    pub replay_actions: Vec<PbftFinalizationRuntimeAction>,
    /// Stable bridge-visible error code for non-complete classifications.
    pub error_code: String,
}

/// Runtime executor state status for the PBFT finalization action script.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftFinalizationRuntimeStatus {
    /// Runtime is ready to return or accept another action.
    Active,
    /// Runtime completed all planned actions successfully.
    Complete,
    /// Runtime was created from a rejected finalization plan.
    RejectedPlan,
    /// Caller reported an action that does not match the next Rust-planned action.
    ActionMismatch,
    /// Caller reported that the planned action failed.
    ActionFailed,
    /// Runtime state is internally inconsistent.
    ContractError,
    /// Unknown status code decoded from a bridge value.
    Unknown,
}

impl PbftFinalizationRuntimeStatus {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::Complete => 1,
            Self::RejectedPlan => 2,
            Self::ActionMismatch => 3,
            Self::ActionFailed => 4,
            Self::ContractError => 255,
            Self::Unknown => 254,
        }
    }
}

/// Stateful Rust-owned PBFT finalization runtime script.
///
/// C++ owns live side effects for now, but Rust owns the action cursor and
/// validates that the shim reports each side effect in the planned order.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationRuntimeState {
    /// Finalization candidate status carried from the intent planner.
    pub finalization_status: PbftFinalizationStatus,
    /// Runtime executor status.
    pub runtime_status: PbftFinalizationRuntimeStatus,
    /// Ordered side-effect script.
    pub actions: Vec<PbftFinalizationRuntimeAction>,
    /// Index of the next action Rust expects C++ to execute.
    pub next_action_index: u32,
    /// Last successfully reported action, if any.
    pub last_action: Option<PbftFinalizationRuntimeAction>,
    /// Failed or mismatched action, if any.
    pub failed_action: Option<PbftFinalizationRuntimeAction>,
    /// Stable bridge-visible error code for terminal failures.
    pub error_code: String,
}

/// Next runtime action requested by Rust.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationRuntimeStep {
    /// Runtime executor status at the time of the request.
    pub runtime_status: PbftFinalizationRuntimeStatus,
    /// True when `action` is populated and should be executed by C++.
    pub has_action: bool,
    /// Action that C++ should execute next.
    pub action: Option<PbftFinalizationRuntimeAction>,
    /// Index of `action` in the runtime script.
    pub action_index: u32,
    /// True when the runtime has completed all actions.
    pub complete: bool,
    /// Stable bridge-visible error code for terminal failures.
    pub error_code: String,
}

/// Result reported by C++ after executing one runtime action.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationRuntimeActionResult {
    /// Action C++ believes it just executed.
    pub action: PbftFinalizationRuntimeAction,
    /// Whether the action completed successfully.
    pub success: bool,
    /// Optional stable error code supplied by the C++ executor.
    pub error_code: String,
}

/// Cacti dynamic-lambda configuration needed by the Rust planner.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftDynamicLambdaConfig {
    /// Cacti hardfork activation period used by interval checks.
    pub cacti_block_num: u64,
    /// Minimum dynamic lambda for round 1.
    pub lambda_min: u32,
    /// Maximum dynamic lambda for round 1.
    pub lambda_max: u32,
    /// Default lambda used in rounds greater than 1.
    pub lambda_default: u32,
    /// Number of finalized blocks between possible lambda decreases.
    pub lambda_change_interval: u32,
    /// Milliseconds added or subtracted by one adjustment.
    pub lambda_change: u32,
    /// Approximate consensus delay used for blocks-per-year calculation.
    pub consensus_delay: u32,
    /// Pre-Cacti configured blocks-per-year value.
    pub dpos_blocks_per_year: u32,
}

/// Dynamic-lambda planner input for one PBFT finalization.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftDynamicLambdaFact {
    /// Whether the finalized period is in the dynamic-lambda hardfork range.
    pub dynamic_lambda_active: bool,
    /// Finalized PBFT period.
    pub finalized_period: u64,
    /// Certified round for the finalized block.
    pub finalized_round: u64,
    /// Live rounds-count value before applying this finalization.
    pub pre_adjust_rounds_count_dynamic_lambda: u32,
    /// Live dynamic-lambda value before applying this finalization.
    pub pre_adjust_dynamic_lambda: u32,
    /// Cacti dynamic-lambda and reward-rate configuration.
    pub config: PbftDynamicLambdaConfig,
}

/// Rust-computed dynamic-lambda result for one PBFT finalization.
///
/// The result contains the lambda used by the finalized block, the reward-rate
/// blocks-per-year input for final-chain rewards, and the post-adjust live
/// lambda fields that the shim must assign before persisting the lambda stage.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftDynamicLambdaPlan {
    /// Whether dynamic-lambda persistence and live mutation should run.
    pub apply_dynamic_lambda_update: bool,
    /// Lambda used by the finalized block.
    pub period_lambda: u32,
    /// Blocks per year for reward calculation.
    pub blocks_per_year: u32,
    /// Post-adjust rounds-count live state.
    pub rounds_count_dynamic_lambda: u32,
    /// Post-adjust dynamic-lambda live state.
    pub dynamic_lambda: u32,
    /// True when the interval decrease branch changed lambda.
    pub decreased_dynamic_lambda: bool,
    /// True when the slow-round increase branch changed lambda.
    pub increased_dynamic_lambda: bool,
    /// Explicit status for invalid planner inputs.
    pub status: PbftFinalizationStatus,
}

/// Minimal bounded cleanup intent for deterministic finalize-path side-effects.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftFinalizationCleanupIntent {
    /// Persist selected PBFT block metadata (`pbft_blocks` and PBFT head entry).
    pub persist_pbft_block_metadata: bool,
    /// Persist per-period reward vote state.
    pub reset_reward_votes: bool,
    /// Apply finalized DAG ordering/anchor changes for this block.
    pub set_dag_block_order: bool,
    /// Update sortition parameters for anchored PBFT finalization.
    pub update_sortition_params: bool,
    /// Update transaction manager finalized-transaction bookkeeping.
    pub update_finalized_transactions_status: bool,
    /// Update PBFT head runtime chain state.
    pub update_pbft_chain: bool,
    /// Clear one-period cache of anchored DAG order lookups.
    pub clear_anchor_dag_cache: bool,
    /// Execute final-chain finalize path for the block.
    pub finalize_final_chain: bool,
    /// Persist lambda/period bookkeeping for Cacti-era blocks.
    pub maybe_update_dynamic_lambda: bool,
    /// Advance PBFT manager consensus period.
    pub advance_period: bool,
}

impl PbftFinalizationCleanupIntent {
    const fn reject() -> Self {
        Self {
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
        }
    }
}

/// Hash plus finalized-position metadata for a planned storage index write.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationPositionedHash {
    /// Hash to index in finalized period storage.
    pub hash: H256,
    /// Zero-based position in the accepted period ordering.
    pub position: u32,
}

/// Explicit storage-write intent used as a transition plan before native Rust DB writes are enabled.
///
/// The booleans identify the PBFT persistence operations the caller should
/// execute. The scalar and hash fields carry the exact facts a native Rust DB
/// writer needs next: PBFT head key/value identity, reward-vote reset identity,
/// dynamic-lambda persistence decision, reward calculation block rate, and
/// executed-status value. Opaque legacy payloads such as `PeriodData` are still
/// materialized by C++ in this slice.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationStorageWriteIntent {
    /// Persist selected PBFT block metadata (`pbft_blocks` and PBFT head entry).
    pub persist_pbft_head: bool,
    /// Persist period-data payload (`savePeriodData`).
    pub persist_period_data: bool,
    /// Persist per-period reward vote state update path.
    pub reset_reward_votes: bool,
    /// Persist sortition parameters for anchored PBFT finalization.
    pub update_sortition_params: bool,
    /// Persist lambda/period bookkeeping path for dynamic-lambda-enabled blocks.
    pub apply_dynamic_lambda_update: bool,
    /// Persist period lambda because storage is missing it or contains a different value.
    pub persist_period_lambda: bool,
    /// Persist `PbftMgrStatus::ExecutedBlock`.
    pub persist_executed_pbft_status: bool,
    /// Accepted PBFT block hash.
    pub pbft_block_hash: H256,
    /// PBFT head storage key that should receive the projected head payload.
    pub pbft_head_hash: H256,
    /// Accepted PBFT block period.
    pub block_period: u64,
    /// Whether PBFT-head metadata should encode a null-anchor block.
    pub null_anchor: bool,
    /// Finalized DAG anchor hash, or zero for null-anchor blocks.
    pub anchor_hash: H256,
    /// Certified-vote period used by reward-vote reset.
    pub reward_vote_period: u64,
    /// Certified-vote round used by reward-vote reset.
    pub reward_vote_round: u64,
    /// Certified-vote step used by reward-vote reset.
    pub reward_vote_step: u64,
    /// Certified-vote block hash used by reward-vote reset.
    pub reward_vote_block_hash: H256,
    /// Lambda value to persist when `persist_period_lambda` is true.
    pub period_lambda: u32,
    /// Blocks-per-year value that must be passed to FinalChain finalization.
    pub blocks_per_year: u32,
    /// Executed status value to persist.
    pub executed_pbft_status: bool,
    /// Opaque PBFT head payload using the legacy-compatible JSON encoding.
    ///
    /// The C++ PBFT chain shim still owns this serialization while Rust owns
    /// the storage write-set. Native persistence treats the bytes as canonical
    /// payload and stores them under `pbft_head_hash` without reformatting.
    pub pbft_head_payload: Vec<u8>,
    /// Canonical period-data RLP payload to write.
    pub period_data_rlp: Vec<u8>,
    /// Finalized DAG block period index writes in storage order.
    pub dag_block_period_writes: Vec<PbftFinalizationPositionedHash>,
    /// Finalized transaction location writes in storage order.
    pub transaction_location_writes: Vec<PbftFinalizationPositionedHash>,
}

impl PbftFinalizationStorageWriteIntent {
    fn reject() -> Self {
        Self {
            persist_pbft_head: false,
            persist_period_data: false,
            reset_reward_votes: false,
            update_sortition_params: false,
            apply_dynamic_lambda_update: false,
            persist_period_lambda: false,
            persist_executed_pbft_status: false,
            pbft_block_hash: H256::zero(),
            pbft_head_hash: H256::zero(),
            block_period: 0,
            null_anchor: false,
            anchor_hash: H256::zero(),
            reward_vote_period: 0,
            reward_vote_round: 0,
            reward_vote_step: 0,
            reward_vote_block_hash: H256::zero(),
            period_lambda: 0,
            blocks_per_year: 0,
            executed_pbft_status: false,
            pbft_head_payload: Vec::new(),
            period_data_rlp: Vec::new(),
            dag_block_period_writes: Vec::new(),
            transaction_location_writes: Vec::new(),
        }
    }
}

/// Input facts from C++ execute/finalize path.
#[derive(Debug, Clone)]
pub struct PbftFinalizationIntentFact {
    /// PBFT candidate block hash.
    pub block_hash: H256,
    /// PBFT head storage key used by legacy `addPbftHeadToBatch`.
    pub pbft_head_hash: H256,
    /// PBFT candidate period.
    pub block_period: u64,
    /// PBFT candidate prev hash.
    pub block_prev_hash: H256,
    /// Current chain head hash at intent time.
    pub chain_last_hash: H256,
    /// Current chain last period at intent time.
    pub chain_last_period: u64,
    /// True when `pbftBlock` is already in chain/storage.
    pub block_in_chain: bool,
    /// PBFT block pivot DAG anchor hash.
    pub pivot_dag_anchor_hash: H256,
    /// Block carries a Pillar block hash and therefore requires pillar-chain finalize.
    pub has_pillar_block: bool,
    /// Pillar finalization result supplied by C++ for this candidate.
    pub pillar_block_finalized: bool,
    /// C++ precomputed dynamic-lambda path requirement.
    pub request_dynamic_lambda_update: bool,
    /// Number of certified votes supplied for this non-duplicate finalization.
    pub cert_vote_count: u64,
    /// Sample certified-vote block hash.
    pub sample_cert_vote_block_hash: H256,
    /// Sample certified-vote period.
    pub sample_cert_vote_period: u64,
    /// Sample certified-vote round.
    pub sample_cert_vote_round: u64,
    /// Sample certified-vote step.
    pub sample_cert_vote_step: u64,
    /// Lambda used by this PBFT block round.
    pub block_lambda: u32,
    /// Whether storage already has the previous saved period lambda.
    pub last_saved_period_lambda_found: bool,
    /// Last saved period lambda when present.
    pub last_saved_period_lambda: u32,
    /// C++-computed Cacti-era blocks-per-year value for `block_lambda`.
    pub dynamic_blocks_per_year: u32,
    /// Genesis-configured pre-Cacti blocks-per-year value.
    pub dpos_blocks_per_year: u32,
    /// Legacy-compatible PBFT head payload for native Rust storage writes.
    pub pbft_head_payload: Vec<u8>,
    /// Canonical period-data RLP payload that native Rust storage will write.
    pub period_data_rlp: Vec<u8>,
    /// Ordered finalized DAG block hashes.
    pub ordered_dag_block_hashes: Vec<H256>,
    /// Ordered finalized transaction hashes after legacy nonce reordering.
    pub ordered_transaction_hashes: Vec<H256>,
}

/// Deterministic finalization runtime intent returned to C++ for one certified PBFT
/// block path.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationPlan {
    /// True when this candidate should continue into C++ execute/finalize effects.
    pub finalize_block: bool,
    /// Null-anchor or anchored intent.
    pub anchor: PbftFinalizationAnchor,
    /// `db_->savePbftMgrStatus(PbftMgrStatus::ExecutedBlock, true)` intent.
    pub executed_pbft_block: bool,
    /// Cleanup intent flags for the caller.
    pub cleanup: PbftFinalizationCleanupIntent,
    /// Storage-write intent planned for Rust native persistence.
    pub storage_write_intent: PbftFinalizationStorageWriteIntent,
    /// Explicit status reason for telemetry and error-path handling.
    pub status: PbftFinalizationStatus,
}

impl PbftFinalizationPlan {
    fn accept(anchor: PbftFinalizationAnchor, fact: PbftFinalizationIntentFact) -> Self {
        let anchored = anchor == PbftFinalizationAnchor::Anchored;
        let persist_period_lambda = fact.request_dynamic_lambda_update
            && (!fact.last_saved_period_lambda_found
                || fact.last_saved_period_lambda != fact.block_lambda);
        let blocks_per_year = if fact.request_dynamic_lambda_update {
            fact.dynamic_blocks_per_year
        } else {
            fact.dpos_blocks_per_year
        };
        Self {
            finalize_block: true,
            anchor,
            executed_pbft_block: true,
            cleanup: PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: true,
                reset_reward_votes: true,
                set_dag_block_order: true,
                update_sortition_params: anchored,
                update_finalized_transactions_status: true,
                update_pbft_chain: true,
                clear_anchor_dag_cache: true,
                finalize_final_chain: true,
                maybe_update_dynamic_lambda: fact.request_dynamic_lambda_update,
                advance_period: true,
            },
            storage_write_intent: PbftFinalizationStorageWriteIntent {
                persist_pbft_head: true,
                persist_period_data: true,
                reset_reward_votes: true,
                update_sortition_params: anchored,
                apply_dynamic_lambda_update: fact.request_dynamic_lambda_update,
                persist_period_lambda,
                persist_executed_pbft_status: true,
                pbft_block_hash: fact.block_hash,
                pbft_head_hash: fact.pbft_head_hash,
                block_period: fact.block_period,
                null_anchor: anchor == PbftFinalizationAnchor::Null,
                anchor_hash: fact.pivot_dag_anchor_hash,
                reward_vote_period: fact.sample_cert_vote_period,
                reward_vote_round: fact.sample_cert_vote_round,
                reward_vote_step: fact.sample_cert_vote_step,
                reward_vote_block_hash: fact.sample_cert_vote_block_hash,
                period_lambda: fact.block_lambda,
                blocks_per_year,
                executed_pbft_status: true,
                pbft_head_payload: fact.pbft_head_payload,
                period_data_rlp: fact.period_data_rlp,
                dag_block_period_writes: positioned_hashes(fact.ordered_dag_block_hashes),
                transaction_location_writes: positioned_hashes(fact.ordered_transaction_hashes),
            },
            status: PbftFinalizationStatus::Accepted,
        }
    }

    fn reject(status: PbftFinalizationStatus, anchor: PbftFinalizationAnchor) -> Self {
        Self {
            finalize_block: false,
            anchor,
            executed_pbft_block: false,
            cleanup: PbftFinalizationCleanupIntent::reject(),
            storage_write_intent: PbftFinalizationStorageWriteIntent::reject(),
            status,
        }
    }
}

/// Builds a deterministic finalization plan from plain facts.
///
/// Ordering and contracts are intentionally side-effect-free:
/// - `block_in_chain` and stale/prev-hash conflicts reject without state change.
/// - pillar-linked blocks require explicit success from C++ pillar-domain checks.
/// - accepted plans mirror legacy non-null anchored cleanup behavior (`sortitionParamsManager`
///   update only for non-null anchors).
pub fn plan_pbft_finalization_intent(fact: PbftFinalizationIntentFact) -> PbftFinalizationPlan {
    let anchor = if fact.pivot_dag_anchor_hash.is_zero() {
        PbftFinalizationAnchor::Null
    } else {
        PbftFinalizationAnchor::Anchored
    };

    if fact.block_in_chain {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::BlockAlreadyInChain, anchor);
    }

    if fact.cert_vote_count == 0 {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::EmptyCertVotes, anchor);
    }

    if fact.sample_cert_vote_block_hash != fact.block_hash {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::CertVoteBlockMismatch, anchor);
    }

    if fact.pbft_head_payload.is_empty() || fact.period_data_rlp.is_empty() {
        return PbftFinalizationPlan::reject(
            PbftFinalizationStatus::StorageFactsIncomplete,
            anchor,
        );
    }

    if fact.block_prev_hash != fact.chain_last_hash && fact.block_period <= fact.chain_last_period {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::StalePeriod, anchor);
    }

    if fact.block_prev_hash != fact.chain_last_hash {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::PreviousHashMismatch, anchor);
    }

    if fact.has_pillar_block && !fact.pillar_block_finalized {
        return PbftFinalizationPlan::reject(
            PbftFinalizationStatus::PillarDependencyMissing,
            anchor,
        );
    }

    PbftFinalizationPlan::accept(anchor, fact)
}

/// Builds the ordered runtime action script for a finalization intent.
///
/// The function is side-effect-free and deliberately mirrors the current shim
/// executor order. Rejected plans produce no actions so callers cannot
/// accidentally perform partial finalization from a failed candidate decision.
pub fn plan_pbft_finalization_runtime(plan: &PbftFinalizationPlan) -> PbftFinalizationRuntimePlan {
    if !plan.finalize_block || plan.status != PbftFinalizationStatus::Accepted {
        return PbftFinalizationRuntimePlan {
            finalize_block: false,
            status: plan.status,
            actions: Vec::new(),
        };
    }

    let mut actions = Vec::new();
    if plan.storage_write_intent.persist_pbft_head || plan.storage_write_intent.persist_period_data
    {
        actions.push(PbftFinalizationRuntimeAction::ApplyPrimaryStorage);
    }
    if plan.cleanup.reset_reward_votes {
        actions.push(PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime);
    }
    if plan.cleanup.set_dag_block_order {
        actions.push(PbftFinalizationRuntimeAction::SetDagBlockOrder);
    }
    if plan.cleanup.update_finalized_transactions_status {
        actions.push(PbftFinalizationRuntimeAction::UpdateFinalizedTransactions);
    }
    if plan.cleanup.update_pbft_chain {
        actions.push(PbftFinalizationRuntimeAction::UpdatePbftChain);
    }
    if plan.cleanup.clear_anchor_dag_cache {
        actions.push(PbftFinalizationRuntimeAction::ClearAnchorDagCache);
    }
    if plan.storage_write_intent.apply_dynamic_lambda_update {
        actions.push(PbftFinalizationRuntimeAction::ApplyDynamicLambda);
    }
    if plan.cleanup.finalize_final_chain {
        actions.push(PbftFinalizationRuntimeAction::FinalizeFinalChain);
    }
    if plan.storage_write_intent.persist_executed_pbft_status {
        actions.push(PbftFinalizationRuntimeAction::PersistExecutedStatus);
    }
    if plan.executed_pbft_block {
        actions.push(PbftFinalizationRuntimeAction::SetExecutedFlag);
    }
    if plan.cleanup.advance_period {
        actions.push(PbftFinalizationRuntimeAction::AdvancePeriod);
    }

    PbftFinalizationRuntimePlan {
        finalize_block: true,
        status: plan.status,
        actions,
    }
}

/// Starts a stateful PBFT finalization runtime executor from a runtime plan.
///
/// The executor owns only ordering state. It never mutates storage or live
/// consensus objects; C++ must call `next_pbft_finalization_runtime_action`,
/// execute the returned action, and report the result through
/// `report_pbft_finalization_runtime_action`.
pub fn start_pbft_finalization_runtime(
    plan: &PbftFinalizationRuntimePlan,
) -> PbftFinalizationRuntimeState {
    if !plan.finalize_block || plan.status != PbftFinalizationStatus::Accepted {
        return PbftFinalizationRuntimeState {
            finalization_status: plan.status,
            runtime_status: PbftFinalizationRuntimeStatus::RejectedPlan,
            actions: Vec::new(),
            next_action_index: 0,
            last_action: None,
            failed_action: None,
            error_code: "PBFT_FINALIZE_RUNTIME_REJECTED_PLAN".to_string(),
        };
    }

    if plan.actions.is_empty() {
        return PbftFinalizationRuntimeState {
            finalization_status: plan.status,
            runtime_status: PbftFinalizationRuntimeStatus::ContractError,
            actions: Vec::new(),
            next_action_index: 0,
            last_action: None,
            failed_action: None,
            error_code: "PBFT_FINALIZE_RUNTIME_EMPTY_SCRIPT".to_string(),
        };
    }

    PbftFinalizationRuntimeState {
        finalization_status: plan.status,
        runtime_status: PbftFinalizationRuntimeStatus::Active,
        actions: plan.actions.clone(),
        next_action_index: 0,
        last_action: None,
        failed_action: None,
        error_code: String::new(),
    }
}

/// Starts a Rust-owned runtime executor from a durable resume plan.
///
/// The resume executor shares the normal finalization cursor/report contract,
/// but its action list is restricted to replay actions derived from durable
/// storage facts. Complete resume plans return a completed runtime with no
/// action. Rejected or unsafe resume classifications return `RejectedPlan` so
/// C++ cannot accidentally execute ambiguous live side effects.
pub fn start_pbft_finalization_resume_runtime(
    plan: &PbftFinalizationResumePlan,
) -> PbftFinalizationRuntimeState {
    if plan.complete && plan.replay_actions.is_empty() {
        return PbftFinalizationRuntimeState {
            finalization_status: PbftFinalizationStatus::Accepted,
            runtime_status: PbftFinalizationRuntimeStatus::Complete,
            actions: Vec::new(),
            next_action_index: 0,
            last_action: None,
            failed_action: None,
            error_code: String::new(),
        };
    }

    if plan.replay_actions.is_empty()
        || !matches!(
            plan.status,
            PbftFinalizationResumeStatus::NeedsFinalChainReplay
                | PbftFinalizationResumeStatus::NeedsExecutedStatusPersistence
        )
    {
        return PbftFinalizationRuntimeState {
            finalization_status: PbftFinalizationStatus::BlockAlreadyInChain,
            runtime_status: PbftFinalizationRuntimeStatus::RejectedPlan,
            actions: Vec::new(),
            next_action_index: 0,
            last_action: None,
            failed_action: None,
            error_code: if plan.error_code.is_empty() {
                "PBFT_FINALIZE_RESUME_RUNTIME_REJECTED_PLAN".to_string()
            } else {
                plan.error_code.clone()
            },
        };
    }

    PbftFinalizationRuntimeState {
        finalization_status: PbftFinalizationStatus::Accepted,
        runtime_status: PbftFinalizationRuntimeStatus::Active,
        actions: plan.replay_actions.clone(),
        next_action_index: 0,
        last_action: None,
        failed_action: None,
        error_code: String::new(),
    }
}

/// Returns the next Rust-planned PBFT finalization action.
///
/// A completed or failed runtime returns no action and carries the terminal
/// status/error. An active runtime that has consumed every action transitions to
/// `Complete` at the returned step boundary.
pub fn next_pbft_finalization_runtime_action(
    state: &PbftFinalizationRuntimeState,
) -> PbftFinalizationRuntimeStep {
    if state.runtime_status != PbftFinalizationRuntimeStatus::Active {
        return PbftFinalizationRuntimeStep {
            runtime_status: state.runtime_status,
            has_action: false,
            action: None,
            action_index: state.next_action_index,
            complete: state.runtime_status == PbftFinalizationRuntimeStatus::Complete,
            error_code: state.error_code.clone(),
        };
    }

    let action_index = state.next_action_index as usize;
    if action_index == state.actions.len() {
        return PbftFinalizationRuntimeStep {
            runtime_status: PbftFinalizationRuntimeStatus::Complete,
            has_action: false,
            action: None,
            action_index: state.next_action_index,
            complete: true,
            error_code: String::new(),
        };
    }

    let Some(action) = state.actions.get(action_index).copied() else {
        return PbftFinalizationRuntimeStep {
            runtime_status: PbftFinalizationRuntimeStatus::ContractError,
            has_action: false,
            action: None,
            action_index: state.next_action_index,
            complete: false,
            error_code: "PBFT_FINALIZE_RUNTIME_CURSOR_OUT_OF_RANGE".to_string(),
        };
    };

    PbftFinalizationRuntimeStep {
        runtime_status: PbftFinalizationRuntimeStatus::Active,
        has_action: true,
        action: Some(action),
        action_index: state.next_action_index,
        complete: false,
        error_code: String::new(),
    }
}

/// Advances the PBFT finalization runtime after C++ executes one action.
///
/// The reported action must exactly match the next Rust-planned action. Failed
/// actions put the runtime into a terminal `ActionFailed` state; mismatches put
/// it into `ActionMismatch`. Successful reports advance the cursor and mark the
/// runtime complete when the final action has been reported.
pub fn report_pbft_finalization_runtime_action(
    mut state: PbftFinalizationRuntimeState,
    result: PbftFinalizationRuntimeActionResult,
) -> PbftFinalizationRuntimeState {
    if state.runtime_status != PbftFinalizationRuntimeStatus::Active {
        state.runtime_status = PbftFinalizationRuntimeStatus::ContractError;
        state.failed_action = Some(result.action);
        if state.error_code.is_empty() {
            state.error_code = "PBFT_FINALIZE_RUNTIME_NOT_ACTIVE".to_string();
        }
        return state;
    }

    let step = next_pbft_finalization_runtime_action(&state);
    if step.runtime_status != PbftFinalizationRuntimeStatus::Active || !step.has_action {
        state.runtime_status = PbftFinalizationRuntimeStatus::ContractError;
        state.failed_action = Some(result.action);
        state.error_code = if step.error_code.is_empty() {
            "PBFT_FINALIZE_RUNTIME_NO_ACTION".to_string()
        } else {
            step.error_code
        };
        return state;
    }

    let expected_action = step
        .action
        .expect("active runtime step with has_action must carry action");
    if result.action != expected_action {
        state.runtime_status = PbftFinalizationRuntimeStatus::ActionMismatch;
        state.failed_action = Some(result.action);
        state.error_code = "PBFT_FINALIZE_RUNTIME_ACTION_MISMATCH".to_string();
        return state;
    }

    if !result.success {
        state.runtime_status = PbftFinalizationRuntimeStatus::ActionFailed;
        state.failed_action = Some(result.action);
        state.error_code = if result.error_code.is_empty() {
            "PBFT_FINALIZE_RUNTIME_ACTION_FAILED".to_string()
        } else {
            result.error_code
        };
        return state;
    }

    state.last_action = Some(result.action);
    state.next_action_index += 1;
    if state.next_action_index as usize == state.actions.len() {
        state.runtime_status = PbftFinalizationRuntimeStatus::Complete;
    }
    state
}

/// Validates a Rust-backed live-mutation report before the PBFT runtime cursor advances.
///
/// The report must match an accepted finalization plan and one of the live
/// mutation actions still executed through subsystem shims. This gives Rust
/// ownership of the action proof while C++ remains the temporary object
/// materializer and lock owner.
pub fn validate_pbft_finalization_live_mutation_report(
    plan: &PbftFinalizationPlan,
    report: PbftFinalizationLiveMutationReport,
) -> PbftFinalizationLiveMutationValidation {
    let reject = |status: PbftFinalizationLiveMutationStatus, error_code: &str| {
        PbftFinalizationLiveMutationValidation {
            accepted: false,
            status,
            action: report.action,
            error_code: error_code.to_string(),
        }
    };

    if !plan.finalize_block || plan.status != PbftFinalizationStatus::Accepted {
        return reject(
            PbftFinalizationLiveMutationStatus::PlanNotAccepted,
            "PBFT_FINALIZE_LIVE_MUTATION_PLAN_NOT_ACCEPTED",
        );
    }

    let runtime_plan = plan_pbft_finalization_runtime(plan);
    if !runtime_plan.actions.contains(&report.action) {
        return reject(
            PbftFinalizationLiveMutationStatus::ActionNotPlanned,
            "PBFT_FINALIZE_LIVE_MUTATION_ACTION_NOT_PLANNED",
        );
    }

    if !matches!(
        report.action,
        PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime
            | PbftFinalizationRuntimeAction::SetDagBlockOrder
            | PbftFinalizationRuntimeAction::UpdateFinalizedTransactions
            | PbftFinalizationRuntimeAction::UpdatePbftChain
    ) {
        return reject(
            PbftFinalizationLiveMutationStatus::ActionNotLiveMutation,
            "PBFT_FINALIZE_LIVE_MUTATION_UNSUPPORTED_ACTION",
        );
    }

    if report.block_period != plan.storage_write_intent.block_period {
        return reject(
            PbftFinalizationLiveMutationStatus::PeriodMismatch,
            "PBFT_FINALIZE_LIVE_MUTATION_PERIOD_MISMATCH",
        );
    }
    if report.pbft_block_hash != plan.storage_write_intent.pbft_block_hash {
        return reject(
            PbftFinalizationLiveMutationStatus::BlockHashMismatch,
            "PBFT_FINALIZE_LIVE_MUTATION_BLOCK_HASH_MISMATCH",
        );
    }
    if report.anchor_hash != plan.storage_write_intent.anchor_hash {
        return reject(
            PbftFinalizationLiveMutationStatus::AnchorMismatch,
            "PBFT_FINALIZE_LIVE_MUTATION_ANCHOR_MISMATCH",
        );
    }

    match report.action {
        PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime => {
            if report.reward_votes_period != plan.storage_write_intent.reward_vote_period
                || report.reward_votes_round != plan.storage_write_intent.reward_vote_round
                || report.reward_votes_block_hash
                    != plan.storage_write_intent.reward_vote_block_hash
            {
                return reject(
                    PbftFinalizationLiveMutationStatus::RewardVotesMetadataMismatch,
                    "PBFT_FINALIZE_LIVE_MUTATION_REWARD_VOTES_METADATA_MISMATCH",
                );
            }
            if report.reward_votes_extra_count != 0 {
                return reject(
                    PbftFinalizationLiveMutationStatus::RewardVotesExtraVotesNotCleared,
                    "PBFT_FINALIZE_LIVE_MUTATION_REWARD_VOTES_EXTRA_NOT_CLEARED",
                );
            }
        }
        PbftFinalizationRuntimeAction::SetDagBlockOrder => {
            let expected =
                unique_positioned_hash_count(&plan.storage_write_intent.dag_block_period_writes);
            if report.dag_finalized_count != expected {
                return reject(
                    PbftFinalizationLiveMutationStatus::DagFinalizedCountMismatch,
                    "PBFT_FINALIZE_LIVE_MUTATION_DAG_COUNT_MISMATCH",
                );
            }
        }
        PbftFinalizationRuntimeAction::UpdateFinalizedTransactions => {
            let expected = unique_positioned_hash_count(
                &plan.storage_write_intent.transaction_location_writes,
            );
            if report.finalized_transaction_count != expected {
                return reject(
                    PbftFinalizationLiveMutationStatus::TransactionFinalizedCountMismatch,
                    "PBFT_FINALIZE_LIVE_MUTATION_TRANSACTION_COUNT_MISMATCH",
                );
            }
        }
        PbftFinalizationRuntimeAction::UpdatePbftChain => {
            if report.pbft_chain_size != plan.storage_write_intent.block_period
                || report.pbft_chain_head_hash != plan.storage_write_intent.pbft_block_hash
            {
                return reject(
                    PbftFinalizationLiveMutationStatus::PbftChainHeadMismatch,
                    "PBFT_FINALIZE_LIVE_MUTATION_PBFT_CHAIN_HEAD_MISMATCH",
                );
            }
            if !plan.storage_write_intent.null_anchor
                && report.pbft_chain_last_anchor_hash != plan.storage_write_intent.anchor_hash
            {
                return reject(
                    PbftFinalizationLiveMutationStatus::PbftChainAnchorMismatch,
                    "PBFT_FINALIZE_LIVE_MUTATION_PBFT_CHAIN_ANCHOR_MISMATCH",
                );
            }
        }
        _ => unreachable!("live mutation action checked above"),
    }

    PbftFinalizationLiveMutationValidation {
        accepted: true,
        status: PbftFinalizationLiveMutationStatus::Accepted,
        action: report.action,
        error_code: String::new(),
    }
}

/// Classifies durable PBFT finalization state for an already-persisted block.
///
/// This planner is deliberately separate from normal finalization admission:
/// `plan_pbft_finalization_intent` still rejects `block_in_chain=true`, while
/// this function inspects storage-derived facts for duplicate/restart paths.
/// It only returns actions whose need can be inferred from durable state. Live
/// DAG, transaction, vote, timer, and period-reset replay remain explicit debt
/// until those side effects have Rust-owned durable cursors.
pub fn plan_pbft_finalization_resume(
    fact: PbftFinalizationResumeFact,
) -> PbftFinalizationResumePlan {
    if !fact.block_in_chain {
        return PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::NotPersisted,
            duplicate_classified: false,
            complete: false,
            replay_actions: Vec::new(),
            error_code: "PBFT_FINALIZE_RESUME_NOT_PERSISTED".to_string(),
        };
    }

    if fact.conflicting_primary_facts {
        return PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::ConflictingPrimaryFacts,
            duplicate_classified: true,
            complete: false,
            replay_actions: Vec::new(),
            error_code: "PBFT_FINALIZE_RESUME_CONFLICTING_PRIMARY_FACTS".to_string(),
        };
    }

    let primary_complete = fact.pbft_period_mapping_matches
        && fact.period_data_matches
        && fact.dag_positions_match
        && fact.transaction_positions_match
        && !fact.missing_primary_facts;
    if !primary_complete {
        return PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::MissingPrimaryFacts,
            duplicate_classified: true,
            complete: false,
            replay_actions: Vec::new(),
            error_code: "PBFT_FINALIZE_RESUME_MISSING_PRIMARY_FACTS".to_string(),
        };
    }

    if fact.dynamic_lambda_required && !fact.dynamic_lambda_persisted {
        return PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::NeedsDynamicLambdaPersistence,
            duplicate_classified: true,
            complete: false,
            replay_actions: vec![PbftFinalizationRuntimeAction::ApplyDynamicLambda],
            error_code: "PBFT_FINALIZE_RESUME_NEEDS_DYNAMIC_LAMBDA".to_string(),
        };
    }

    if fact.final_chain_last_block < fact.block_period {
        let mut replay_actions = Vec::new();
        replay_actions.push(PbftFinalizationRuntimeAction::FinalizeFinalChain);
        if !fact.executed_status_persisted {
            replay_actions.push(PbftFinalizationRuntimeAction::PersistExecutedStatus);
        }
        replay_actions.push(PbftFinalizationRuntimeAction::SetExecutedFlag);
        replay_actions.push(PbftFinalizationRuntimeAction::AdvancePeriod);
        return PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::NeedsFinalChainReplay,
            duplicate_classified: true,
            complete: false,
            replay_actions,
            error_code: "PBFT_FINALIZE_RESUME_NEEDS_FINAL_CHAIN_REPLAY".to_string(),
        };
    }

    if !fact.executed_status_persisted {
        return PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::NeedsExecutedStatusPersistence,
            duplicate_classified: true,
            complete: false,
            replay_actions: vec![PbftFinalizationRuntimeAction::PersistExecutedStatus],
            error_code: "PBFT_FINALIZE_RESUME_NEEDS_EXECUTED_STATUS".to_string(),
        };
    }

    PbftFinalizationResumePlan {
        status: PbftFinalizationResumeStatus::Complete,
        duplicate_classified: true,
        complete: true,
        replay_actions: Vec::new(),
        error_code: String::new(),
    }
}

/// Computes the block lambda and post-finalization dynamic-lambda state.
///
/// This is the Rust source of truth for the Cacti dynamic-lambda algorithm. It
/// does not read storage or mutate live state. Invalid config that would make
/// interval or reward-rate arithmetic undefined returns `ContractError`.
pub fn plan_pbft_dynamic_lambda(fact: PbftDynamicLambdaFact) -> PbftDynamicLambdaPlan {
    if !fact.dynamic_lambda_active {
        return PbftDynamicLambdaPlan {
            apply_dynamic_lambda_update: false,
            period_lambda: 0,
            blocks_per_year: fact.config.dpos_blocks_per_year,
            rounds_count_dynamic_lambda: fact.pre_adjust_rounds_count_dynamic_lambda,
            dynamic_lambda: fact.pre_adjust_dynamic_lambda,
            decreased_dynamic_lambda: false,
            increased_dynamic_lambda: false,
            status: PbftFinalizationStatus::Accepted,
        };
    }

    if fact.finalized_round == 0 || fact.config.lambda_change_interval == 0 {
        return dynamic_lambda_contract_error(fact);
    }

    let period_lambda = if fact.finalized_round == 1 {
        fact.pre_adjust_dynamic_lambda
    } else {
        fact.config.lambda_default
    };
    let Some(blocks_per_year) = calc_blocks_per_year(period_lambda, fact.config.consensus_delay)
    else {
        return dynamic_lambda_contract_error(fact);
    };
    if fact.finalized_round > u32::MAX as u64 {
        return dynamic_lambda_contract_error(fact);
    }
    let Some(mut rounds_count_dynamic_lambda) = fact
        .pre_adjust_rounds_count_dynamic_lambda
        .checked_add(fact.finalized_round as u32)
    else {
        return dynamic_lambda_contract_error(fact);
    };

    let mut dynamic_lambda = fact.pre_adjust_dynamic_lambda;
    let mut decreased_dynamic_lambda = false;
    let mut increased_dynamic_lambda = false;

    if is_dynamic_lambda_change_interval(
        fact.finalized_period,
        fact.config.cacti_block_num,
        fact.config.lambda_change_interval,
    ) {
        if rounds_count_dynamic_lambda == fact.config.lambda_change_interval
            && dynamic_lambda > fact.config.lambda_min
        {
            dynamic_lambda = dynamic_lambda
                .saturating_sub(fact.config.lambda_change)
                .max(fact.config.lambda_min);
            decreased_dynamic_lambda = true;
        }
        rounds_count_dynamic_lambda = 0;
    }

    if fact.finalized_round > 1 && dynamic_lambda < fact.config.lambda_max {
        dynamic_lambda = dynamic_lambda
            .saturating_add(fact.config.lambda_change)
            .min(fact.config.lambda_max);
        increased_dynamic_lambda = true;
    }

    PbftDynamicLambdaPlan {
        apply_dynamic_lambda_update: true,
        period_lambda,
        blocks_per_year,
        rounds_count_dynamic_lambda,
        dynamic_lambda,
        decreased_dynamic_lambda,
        increased_dynamic_lambda,
        status: PbftFinalizationStatus::Accepted,
    }
}

fn dynamic_lambda_contract_error(fact: PbftDynamicLambdaFact) -> PbftDynamicLambdaPlan {
    PbftDynamicLambdaPlan {
        apply_dynamic_lambda_update: fact.dynamic_lambda_active,
        period_lambda: 0,
        blocks_per_year: fact.config.dpos_blocks_per_year,
        rounds_count_dynamic_lambda: fact.pre_adjust_rounds_count_dynamic_lambda,
        dynamic_lambda: fact.pre_adjust_dynamic_lambda,
        decreased_dynamic_lambda: false,
        increased_dynamic_lambda: false,
        status: PbftFinalizationStatus::ContractError,
    }
}

fn is_dynamic_lambda_change_interval(
    block_number: u64,
    cacti_block_num: u64,
    lambda_change_interval: u32,
) -> bool {
    lambda_change_interval == 1
        || (block_number > cacti_block_num
            && block_number.is_multiple_of(u64::from(lambda_change_interval)))
}

fn calc_blocks_per_year(lambda_ms: u32, delay_ms: u32) -> Option<u32> {
    let expected_block_time = u64::from(lambda_ms)
        .checked_mul(2)?
        .checked_add(u64::from(delay_ms))?;
    if expected_block_time == 0 {
        return None;
    }
    let year_ms = 365_u64 * 24 * 60 * 60 * 1000;
    u32::try_from(year_ms / expected_block_time).ok()
}

fn positioned_hashes(hashes: Vec<H256>) -> Vec<PbftFinalizationPositionedHash> {
    hashes
        .into_iter()
        .enumerate()
        .map(|(position, hash)| PbftFinalizationPositionedHash {
            hash,
            position: position as u32,
        })
        .collect()
}

fn unique_positioned_hash_count(writes: &[PbftFinalizationPositionedHash]) -> u64 {
    writes
        .iter()
        .map(|write| write.hash)
        .collect::<HashSet<_>>()
        .len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    fn accepted_fact() -> PbftFinalizationIntentFact {
        PbftFinalizationIntentFact {
            block_hash: hash(99),
            pbft_head_hash: hash(88),
            block_period: 10,
            block_prev_hash: hash(42),
            chain_last_hash: hash(42),
            chain_last_period: 9,
            block_in_chain: false,
            pivot_dag_anchor_hash: hash(123),
            has_pillar_block: false,
            pillar_block_finalized: false,
            request_dynamic_lambda_update: true,
            cert_vote_count: 3,
            sample_cert_vote_block_hash: hash(99),
            sample_cert_vote_period: 10,
            sample_cert_vote_round: 2,
            sample_cert_vote_step: 5,
            block_lambda: 1_500,
            last_saved_period_lambda_found: false,
            last_saved_period_lambda: 0,
            dynamic_blocks_per_year: 1_000,
            dpos_blocks_per_year: 500,
            pbft_head_payload: br#"{"head":true}"#.to_vec(),
            period_data_rlp: vec![0xc0],
            ordered_dag_block_hashes: vec![hash(1), hash(2)],
            ordered_transaction_hashes: vec![hash(3), hash(4)],
        }
    }

    fn dynamic_lambda_fact() -> PbftDynamicLambdaFact {
        PbftDynamicLambdaFact {
            dynamic_lambda_active: true,
            finalized_period: 20,
            finalized_round: 1,
            pre_adjust_rounds_count_dynamic_lambda: 9,
            pre_adjust_dynamic_lambda: 1_500,
            config: PbftDynamicLambdaConfig {
                cacti_block_num: 10,
                lambda_min: 500,
                lambda_max: 1_500,
                lambda_default: 2_000,
                lambda_change_interval: 10,
                lambda_change: 10,
                consensus_delay: 400,
                dpos_blocks_per_year: 500,
            },
        }
    }

    fn live_report(action: PbftFinalizationRuntimeAction) -> PbftFinalizationLiveMutationReport {
        PbftFinalizationLiveMutationReport {
            action,
            block_period: 10,
            pbft_block_hash: hash(99),
            anchor_hash: hash(123),
            dag_finalized_count: 0,
            finalized_transaction_count: 0,
            pbft_chain_size: 0,
            pbft_chain_head_hash: H256::zero(),
            pbft_chain_last_anchor_hash: H256::zero(),
            reward_votes_period: 10,
            reward_votes_round: 2,
            reward_votes_block_hash: hash(99),
            reward_votes_extra_count: 0,
        }
    }

    #[test]
    fn accepts_anchored_block_and_raises_expected_cleanup_intent() {
        let fact = accepted_fact();
        let plan = plan_pbft_finalization_intent(fact);

        assert!(plan.finalize_block);
        assert_eq!(plan.anchor, PbftFinalizationAnchor::Anchored);
        assert!(plan.executed_pbft_block);
        assert_eq!(plan.status, PbftFinalizationStatus::Accepted);
        assert!(plan.cleanup.persist_pbft_block_metadata);
        assert!(plan.storage_write_intent.persist_pbft_head);
        assert!(plan.storage_write_intent.persist_period_data);
        assert!(plan.storage_write_intent.reset_reward_votes);
        assert!(plan.cleanup.update_sortition_params);
        assert!(plan.storage_write_intent.update_sortition_params);
        assert!(plan.storage_write_intent.persist_period_lambda);
        assert!(plan.storage_write_intent.persist_executed_pbft_status);
        assert_eq!(plan.storage_write_intent.pbft_block_hash, hash(99));
        assert_eq!(plan.storage_write_intent.pbft_head_hash, hash(88));
        assert_eq!(plan.storage_write_intent.block_period, 10);
        assert!(!plan.storage_write_intent.null_anchor);
        assert_eq!(plan.storage_write_intent.reward_vote_period, 10);
        assert_eq!(plan.storage_write_intent.reward_vote_round, 2);
        assert_eq!(plan.storage_write_intent.reward_vote_step, 5);
        assert_eq!(plan.storage_write_intent.reward_vote_block_hash, hash(99));
        assert_eq!(plan.storage_write_intent.period_lambda, 1_500);
        assert_eq!(plan.storage_write_intent.blocks_per_year, 1_000);
        assert!(plan.storage_write_intent.executed_pbft_status);
        assert_eq!(
            plan.storage_write_intent.pbft_head_payload,
            br#"{"head":true}"#.to_vec()
        );
        assert_eq!(plan.storage_write_intent.period_data_rlp, vec![0xc0]);
        assert_eq!(
            plan.storage_write_intent.dag_block_period_writes,
            vec![
                PbftFinalizationPositionedHash {
                    hash: hash(1),
                    position: 0
                },
                PbftFinalizationPositionedHash {
                    hash: hash(2),
                    position: 1
                }
            ]
        );
        assert_eq!(
            plan.storage_write_intent.transaction_location_writes,
            vec![
                PbftFinalizationPositionedHash {
                    hash: hash(3),
                    position: 0
                },
                PbftFinalizationPositionedHash {
                    hash: hash(4),
                    position: 1
                }
            ]
        );
        assert!(plan.cleanup.finalize_final_chain);
        assert!(plan.cleanup.advance_period);
        assert!(plan.cleanup.set_dag_block_order);
        assert!(plan.cleanup.update_finalized_transactions_status);
        assert!(plan.storage_write_intent.apply_dynamic_lambda_update);
    }

    #[test]
    fn null_anchor_is_skipped_from_sortition_update_cleanup() {
        let mut fact = accepted_fact();
        fact.pivot_dag_anchor_hash = H256::zero();
        fact.request_dynamic_lambda_update = false;
        let plan = plan_pbft_finalization_intent(fact);

        assert!(plan.finalize_block);
        assert_eq!(plan.anchor, PbftFinalizationAnchor::Null);
        assert!(!plan.cleanup.update_sortition_params);
        assert!(!plan.storage_write_intent.update_sortition_params);
        assert!(!plan.storage_write_intent.apply_dynamic_lambda_update);
        assert!(!plan.storage_write_intent.persist_period_lambda);
        assert!(plan.storage_write_intent.null_anchor);
        assert_eq!(plan.storage_write_intent.blocks_per_year, 500);
    }

    #[test]
    fn rejects_duplicate_blocks() {
        let mut fact = accepted_fact();
        fact.block_in_chain = true;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::BlockAlreadyInChain);
        assert!(!plan.executed_pbft_block);
        assert!(!plan.cleanup.advance_period);
        assert!(!plan.storage_write_intent.persist_pbft_head);
        assert!(!plan.storage_write_intent.persist_period_data);
        assert!(!plan.storage_write_intent.reset_reward_votes);
        assert!(!plan.storage_write_intent.persist_executed_pbft_status);
    }

    #[test]
    fn resume_classifier_distinguishes_complete_replay_and_corrupt_storage() {
        let complete = PbftFinalizationResumeFact {
            block_in_chain: true,
            block_period: 10,
            final_chain_last_block: 10,
            pbft_period_mapping_matches: true,
            period_data_matches: true,
            dag_positions_match: true,
            transaction_positions_match: true,
            missing_primary_facts: false,
            conflicting_primary_facts: false,
            dynamic_lambda_required: true,
            dynamic_lambda_persisted: true,
            executed_status_persisted: true,
        };
        let plan = plan_pbft_finalization_resume(complete);
        assert_eq!(plan.status, PbftFinalizationResumeStatus::Complete);
        assert!(plan.complete);
        assert!(plan.replay_actions.is_empty());

        let mut needs_final_chain = complete;
        needs_final_chain.final_chain_last_block = 9;
        needs_final_chain.executed_status_persisted = false;
        let plan = plan_pbft_finalization_resume(needs_final_chain);
        assert_eq!(
            plan.status,
            PbftFinalizationResumeStatus::NeedsFinalChainReplay
        );
        assert_eq!(
            plan.replay_actions,
            vec![
                PbftFinalizationRuntimeAction::FinalizeFinalChain,
                PbftFinalizationRuntimeAction::PersistExecutedStatus,
                PbftFinalizationRuntimeAction::SetExecutedFlag,
                PbftFinalizationRuntimeAction::AdvancePeriod,
            ]
        );

        let mut missing = complete;
        missing.period_data_matches = false;
        missing.missing_primary_facts = true;
        let plan = plan_pbft_finalization_resume(missing);
        assert_eq!(
            plan.status,
            PbftFinalizationResumeStatus::MissingPrimaryFacts
        );
        assert!(plan.replay_actions.is_empty());

        let mut conflicting = complete;
        conflicting.conflicting_primary_facts = true;
        let plan = plan_pbft_finalization_resume(conflicting);
        assert_eq!(
            plan.status,
            PbftFinalizationResumeStatus::ConflictingPrimaryFacts
        );
        assert!(plan.replay_actions.is_empty());
    }

    #[test]
    fn resume_classifier_requires_dynamic_lambda_before_final_chain_replay() {
        let plan = plan_pbft_finalization_resume(PbftFinalizationResumeFact {
            block_in_chain: true,
            block_period: 10,
            final_chain_last_block: 9,
            pbft_period_mapping_matches: true,
            period_data_matches: true,
            dag_positions_match: true,
            transaction_positions_match: true,
            missing_primary_facts: false,
            conflicting_primary_facts: false,
            dynamic_lambda_required: true,
            dynamic_lambda_persisted: false,
            executed_status_persisted: false,
        });

        assert_eq!(
            plan.status,
            PbftFinalizationResumeStatus::NeedsDynamicLambdaPersistence
        );
        assert_eq!(
            plan.replay_actions,
            vec![PbftFinalizationRuntimeAction::ApplyDynamicLambda]
        );
    }

    #[test]
    fn resume_runtime_session_uses_only_replay_actions() {
        let resume = PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::NeedsFinalChainReplay,
            duplicate_classified: true,
            complete: false,
            replay_actions: vec![
                PbftFinalizationRuntimeAction::FinalizeFinalChain,
                PbftFinalizationRuntimeAction::PersistExecutedStatus,
                PbftFinalizationRuntimeAction::SetExecutedFlag,
                PbftFinalizationRuntimeAction::AdvancePeriod,
            ],
            error_code: "PBFT_FINALIZE_RESUME_NEEDS_FINAL_CHAIN_REPLAY".to_string(),
        };
        let mut state = start_pbft_finalization_resume_runtime(&resume);

        let mut seen = Vec::new();
        loop {
            let step = next_pbft_finalization_runtime_action(&state);
            if step.complete {
                assert_eq!(step.runtime_status, PbftFinalizationRuntimeStatus::Complete);
                break;
            }
            let action = step.action.expect("active step should carry action");
            seen.push(action);
            state = report_pbft_finalization_runtime_action(
                state,
                PbftFinalizationRuntimeActionResult {
                    action,
                    success: true,
                    error_code: String::new(),
                },
            );
        }

        assert_eq!(seen, resume.replay_actions);
    }

    #[test]
    fn resume_runtime_rejects_unsafe_or_complete_plans_without_actions() {
        let complete = PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::Complete,
            duplicate_classified: true,
            complete: true,
            replay_actions: Vec::new(),
            error_code: String::new(),
        };
        let state = start_pbft_finalization_resume_runtime(&complete);
        assert_eq!(
            state.runtime_status,
            PbftFinalizationRuntimeStatus::Complete
        );

        let unsafe_plan = PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::MissingPrimaryFacts,
            duplicate_classified: true,
            complete: false,
            replay_actions: Vec::new(),
            error_code: "PBFT_FINALIZE_RESUME_MISSING_PRIMARY_FACTS".to_string(),
        };
        let state = start_pbft_finalization_resume_runtime(&unsafe_plan);
        assert_eq!(
            state.runtime_status,
            PbftFinalizationRuntimeStatus::RejectedPlan
        );
        assert_eq!(
            state.error_code,
            "PBFT_FINALIZE_RESUME_MISSING_PRIMARY_FACTS"
        );

        let dynamic_gap = PbftFinalizationResumePlan {
            status: PbftFinalizationResumeStatus::NeedsDynamicLambdaPersistence,
            duplicate_classified: true,
            complete: false,
            replay_actions: vec![PbftFinalizationRuntimeAction::ApplyDynamicLambda],
            error_code: "PBFT_FINALIZE_RESUME_NEEDS_DYNAMIC_LAMBDA".to_string(),
        };
        let state = start_pbft_finalization_resume_runtime(&dynamic_gap);
        assert_eq!(
            state.runtime_status,
            PbftFinalizationRuntimeStatus::RejectedPlan
        );
        assert_eq!(
            state.error_code,
            "PBFT_FINALIZE_RESUME_NEEDS_DYNAMIC_LAMBDA"
        );
    }

    #[test]
    fn rejects_missing_and_mismatched_cert_vote_facts() {
        let mut fact = accepted_fact();
        fact.cert_vote_count = 0;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::EmptyCertVotes);
        assert!(!plan.storage_write_intent.persist_pbft_head);

        fact = accepted_fact();
        fact.sample_cert_vote_block_hash = hash(100);

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::CertVoteBlockMismatch);
        assert!(!plan.storage_write_intent.persist_period_data);
    }

    #[test]
    fn skips_period_lambda_storage_when_existing_value_matches() {
        let mut fact = accepted_fact();
        fact.last_saved_period_lambda_found = true;
        fact.last_saved_period_lambda = fact.block_lambda;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(plan.finalize_block);
        assert!(plan.storage_write_intent.apply_dynamic_lambda_update);
        assert!(!plan.storage_write_intent.persist_period_lambda);
    }

    #[test]
    fn rejects_missing_storage_payload_facts_for_accepted_blocks() {
        let mut fact = accepted_fact();
        fact.period_data_rlp.clear();

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::StorageFactsIncomplete);
        assert!(plan.storage_write_intent.period_data_rlp.is_empty());
        assert!(plan.storage_write_intent.dag_block_period_writes.is_empty());

        let mut fact = accepted_fact();
        fact.pbft_head_payload.clear();

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::StorageFactsIncomplete);
        assert!(plan.storage_write_intent.pbft_head_payload.is_empty());
    }

    #[test]
    fn rejects_pillar_blocks_without_finalized_pillar() {
        let mut fact = accepted_fact();
        fact.has_pillar_block = true;
        fact.pillar_block_finalized = false;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::PillarDependencyMissing);
    }

    #[test]
    fn rejects_stale_prev_hash_conflicts() {
        let mut fact = accepted_fact();
        fact.block_prev_hash = hash(41);
        fact.chain_last_period = 12;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::StalePeriod);
    }

    #[test]
    fn rejects_non_stale_prev_hash_mismatch_with_previous_hash_status() {
        let mut fact = accepted_fact();
        fact.block_prev_hash = hash(41);
        fact.chain_last_period = 9;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::PreviousHashMismatch);
    }

    #[test]
    fn finalization_runtime_orders_accepted_side_effects() {
        let plan = plan_pbft_finalization_intent(accepted_fact());

        let runtime = plan_pbft_finalization_runtime(&plan);

        assert!(runtime.finalize_block);
        assert_eq!(runtime.status, PbftFinalizationStatus::Accepted);
        assert_eq!(
            runtime.actions,
            vec![
                PbftFinalizationRuntimeAction::ApplyPrimaryStorage,
                PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime,
                PbftFinalizationRuntimeAction::SetDagBlockOrder,
                PbftFinalizationRuntimeAction::UpdateFinalizedTransactions,
                PbftFinalizationRuntimeAction::UpdatePbftChain,
                PbftFinalizationRuntimeAction::ClearAnchorDagCache,
                PbftFinalizationRuntimeAction::ApplyDynamicLambda,
                PbftFinalizationRuntimeAction::FinalizeFinalChain,
                PbftFinalizationRuntimeAction::PersistExecutedStatus,
                PbftFinalizationRuntimeAction::SetExecutedFlag,
                PbftFinalizationRuntimeAction::AdvancePeriod,
            ]
        );
    }

    #[test]
    fn finalization_runtime_session_advances_only_after_matching_reports() {
        let plan = plan_pbft_finalization_intent(accepted_fact());
        let runtime = plan_pbft_finalization_runtime(&plan);
        let mut state = start_pbft_finalization_runtime(&runtime);

        let step = next_pbft_finalization_runtime_action(&state);
        assert_eq!(step.runtime_status, PbftFinalizationRuntimeStatus::Active);
        assert!(step.has_action);
        assert_eq!(
            step.action,
            Some(PbftFinalizationRuntimeAction::ApplyPrimaryStorage)
        );
        assert_eq!(step.action_index, 0);

        state = report_pbft_finalization_runtime_action(
            state,
            PbftFinalizationRuntimeActionResult {
                action: PbftFinalizationRuntimeAction::ApplyPrimaryStorage,
                success: true,
                error_code: String::new(),
            },
        );

        let step = next_pbft_finalization_runtime_action(&state);
        assert_eq!(step.runtime_status, PbftFinalizationRuntimeStatus::Active);
        assert_eq!(
            step.action,
            Some(PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime)
        );
        assert_eq!(step.action_index, 1);
    }

    #[test]
    fn finalization_runtime_session_rejects_mismatched_and_failed_reports() {
        let plan = plan_pbft_finalization_intent(accepted_fact());
        let runtime = plan_pbft_finalization_runtime(&plan);
        let state = start_pbft_finalization_runtime(&runtime);

        let mismatch = report_pbft_finalization_runtime_action(
            state.clone(),
            PbftFinalizationRuntimeActionResult {
                action: PbftFinalizationRuntimeAction::FinalizeFinalChain,
                success: true,
                error_code: String::new(),
            },
        );
        assert_eq!(
            mismatch.runtime_status,
            PbftFinalizationRuntimeStatus::ActionMismatch
        );
        assert_eq!(mismatch.error_code, "PBFT_FINALIZE_RUNTIME_ACTION_MISMATCH");

        let failed = report_pbft_finalization_runtime_action(
            state,
            PbftFinalizationRuntimeActionResult {
                action: PbftFinalizationRuntimeAction::ApplyPrimaryStorage,
                success: false,
                error_code: "PRIMARY_FAILED".to_string(),
            },
        );
        assert_eq!(
            failed.runtime_status,
            PbftFinalizationRuntimeStatus::ActionFailed
        );
        assert_eq!(failed.next_action_index, 0);
        assert_eq!(failed.error_code, "PRIMARY_FAILED");
    }

    #[test]
    fn finalization_runtime_session_completes_after_last_action() {
        let plan = plan_pbft_finalization_intent(accepted_fact());
        let runtime = plan_pbft_finalization_runtime(&plan);
        let mut state = start_pbft_finalization_runtime(&runtime);

        for action in runtime.actions {
            state = report_pbft_finalization_runtime_action(
                state,
                PbftFinalizationRuntimeActionResult {
                    action,
                    success: true,
                    error_code: String::new(),
                },
            );
        }

        assert_eq!(
            state.runtime_status,
            PbftFinalizationRuntimeStatus::Complete
        );
        let step = next_pbft_finalization_runtime_action(&state);
        assert_eq!(step.runtime_status, PbftFinalizationRuntimeStatus::Complete);
        assert!(step.complete);
        assert!(!step.has_action);
    }

    #[test]
    fn live_mutation_report_accepts_rust_backed_dag_transaction_and_chain_proofs() {
        let plan = plan_pbft_finalization_intent(accepted_fact());

        let dag = validate_pbft_finalization_live_mutation_report(
            &plan,
            PbftFinalizationLiveMutationReport {
                dag_finalized_count: 2,
                ..live_report(PbftFinalizationRuntimeAction::SetDagBlockOrder)
            },
        );
        assert!(dag.accepted);
        assert_eq!(dag.status, PbftFinalizationLiveMutationStatus::Accepted);

        let reward_votes = validate_pbft_finalization_live_mutation_report(
            &plan,
            live_report(PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime),
        );
        assert!(reward_votes.accepted);

        let transactions = validate_pbft_finalization_live_mutation_report(
            &plan,
            PbftFinalizationLiveMutationReport {
                finalized_transaction_count: 2,
                ..live_report(PbftFinalizationRuntimeAction::UpdateFinalizedTransactions)
            },
        );
        assert!(transactions.accepted);

        let chain = validate_pbft_finalization_live_mutation_report(
            &plan,
            PbftFinalizationLiveMutationReport {
                pbft_chain_size: 10,
                pbft_chain_head_hash: hash(99),
                pbft_chain_last_anchor_hash: hash(123),
                ..live_report(PbftFinalizationRuntimeAction::UpdatePbftChain)
            },
        );
        assert!(chain.accepted);
    }

    #[test]
    fn live_mutation_report_rejects_unplanned_and_mismatched_proofs() {
        let mut fact = accepted_fact();
        fact.ordered_dag_block_hashes = vec![hash(1), hash(1), hash(2)];
        let plan = plan_pbft_finalization_intent(fact);

        let dag = validate_pbft_finalization_live_mutation_report(
            &plan,
            PbftFinalizationLiveMutationReport {
                dag_finalized_count: 3,
                ..live_report(PbftFinalizationRuntimeAction::SetDagBlockOrder)
            },
        );
        assert!(!dag.accepted);
        assert_eq!(
            dag.status,
            PbftFinalizationLiveMutationStatus::DagFinalizedCountMismatch
        );

        let chain = validate_pbft_finalization_live_mutation_report(
            &plan,
            PbftFinalizationLiveMutationReport {
                pbft_chain_size: 9,
                pbft_chain_head_hash: hash(99),
                pbft_chain_last_anchor_hash: hash(123),
                ..live_report(PbftFinalizationRuntimeAction::UpdatePbftChain)
            },
        );
        assert_eq!(
            chain.status,
            PbftFinalizationLiveMutationStatus::PbftChainHeadMismatch
        );

        let unsupported = validate_pbft_finalization_live_mutation_report(
            &plan,
            live_report(PbftFinalizationRuntimeAction::FinalizeFinalChain),
        );
        assert_eq!(
            unsupported.status,
            PbftFinalizationLiveMutationStatus::ActionNotLiveMutation
        );

        let reward_votes = validate_pbft_finalization_live_mutation_report(
            &plan,
            PbftFinalizationLiveMutationReport {
                reward_votes_extra_count: 1,
                ..live_report(PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime)
            },
        );
        assert_eq!(
            reward_votes.status,
            PbftFinalizationLiveMutationStatus::RewardVotesExtraVotesNotCleared
        );
    }

    #[test]
    fn finalization_runtime_omits_sortition_and_lambda_for_null_anchor_without_cacti() {
        let mut fact = accepted_fact();
        fact.pivot_dag_anchor_hash = H256::zero();
        fact.request_dynamic_lambda_update = false;
        let plan = plan_pbft_finalization_intent(fact);

        let runtime = plan_pbft_finalization_runtime(&plan);

        assert!(runtime.finalize_block);
        assert!(
            !runtime
                .actions
                .contains(&PbftFinalizationRuntimeAction::ApplySortitionStorage)
        );
        assert!(
            !runtime
                .actions
                .contains(&PbftFinalizationRuntimeAction::ApplyDynamicLambda)
        );
        assert!(
            runtime
                .actions
                .contains(&PbftFinalizationRuntimeAction::FinalizeFinalChain)
        );
    }

    #[test]
    fn finalization_runtime_rejected_plan_has_no_actions() {
        let mut fact = accepted_fact();
        fact.block_in_chain = true;
        let plan = plan_pbft_finalization_intent(fact);

        let runtime = plan_pbft_finalization_runtime(&plan);

        assert!(!runtime.finalize_block);
        assert_eq!(runtime.status, PbftFinalizationStatus::BlockAlreadyInChain);
        assert!(runtime.actions.is_empty());
    }

    #[test]
    fn dynamic_lambda_decreases_on_exact_interval_and_round_one() {
        let plan = plan_pbft_dynamic_lambda(dynamic_lambda_fact());

        assert_eq!(plan.status, PbftFinalizationStatus::Accepted);
        assert!(plan.apply_dynamic_lambda_update);
        assert_eq!(plan.period_lambda, 1_500);
        assert_eq!(plan.blocks_per_year, 9_275_294);
        assert_eq!(plan.rounds_count_dynamic_lambda, 0);
        assert_eq!(plan.dynamic_lambda, 1_490);
        assert!(plan.decreased_dynamic_lambda);
        assert!(!plan.increased_dynamic_lambda);
    }

    #[test]
    fn dynamic_lambda_increases_for_late_round_and_clamps_to_max() {
        let mut fact = dynamic_lambda_fact();
        fact.finalized_period = 21;
        fact.finalized_round = 2;
        fact.pre_adjust_rounds_count_dynamic_lambda = 3;
        fact.pre_adjust_dynamic_lambda = 1_495;

        let plan = plan_pbft_dynamic_lambda(fact);

        assert_eq!(plan.status, PbftFinalizationStatus::Accepted);
        assert_eq!(plan.period_lambda, 2_000);
        assert_eq!(plan.rounds_count_dynamic_lambda, 5);
        assert_eq!(plan.dynamic_lambda, 1_500);
        assert!(!plan.decreased_dynamic_lambda);
        assert!(plan.increased_dynamic_lambda);
    }

    #[test]
    fn dynamic_lambda_decrease_then_increase_preserves_legacy_order() {
        let mut fact = dynamic_lambda_fact();
        fact.finalized_round = 2;
        fact.pre_adjust_rounds_count_dynamic_lambda = 8;
        fact.pre_adjust_dynamic_lambda = 1_000;

        let plan = plan_pbft_dynamic_lambda(fact);

        assert_eq!(plan.rounds_count_dynamic_lambda, 0);
        assert_eq!(plan.dynamic_lambda, 1_000);
        assert!(plan.decreased_dynamic_lambda);
        assert!(plan.increased_dynamic_lambda);
    }

    #[test]
    fn dynamic_lambda_disabled_uses_dpos_rate_without_mutation() {
        let mut fact = dynamic_lambda_fact();
        fact.dynamic_lambda_active = false;

        let plan = plan_pbft_dynamic_lambda(fact);

        assert_eq!(plan.status, PbftFinalizationStatus::Accepted);
        assert!(!plan.apply_dynamic_lambda_update);
        assert_eq!(plan.period_lambda, 0);
        assert_eq!(plan.blocks_per_year, 500);
        assert_eq!(plan.rounds_count_dynamic_lambda, 9);
        assert_eq!(plan.dynamic_lambda, 1_500);
    }

    #[test]
    fn dynamic_lambda_rejects_invalid_interval() {
        let mut fact = dynamic_lambda_fact();
        fact.config.lambda_change_interval = 0;

        let plan = plan_pbft_dynamic_lambda(fact);

        assert_eq!(plan.status, PbftFinalizationStatus::ContractError);
        assert!(plan.apply_dynamic_lambda_update);
        assert_eq!(plan.dynamic_lambda, 1_500);
    }
}
