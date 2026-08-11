//! Deterministic PBFT sync-period admission planning.
//!
//! This module owns the side-effect-free decision table for synced PBFT period
//! payloads after C++ has collected the currently available facts from PBFT
//! chain, FinalChain, vote manager, transaction manager, and pillar validation
//! surfaces. It does not log, mutate queues, punish peers, or wait for
//! finalization; callers apply those side effects from the returned plan.
//!
//! Compatibility invariant: missing or already-finalized transaction facts are
//! emitted as warnings only. They intentionally do not reject a synced period
//! payload until the product behavior is explicitly changed.

use ethereum_types::H256;
use std::collections::HashSet;

/// FinalChain state-root validation fact for a synced PBFT block.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncFinalChainHashStatus {
    /// The block's final-chain hash matches local finalized state.
    Valid,
    /// Local finalization is behind and the caller should wait/retry.
    Missing,
    /// The block's final-chain hash conflicts with local finalized state.
    Invalid,
    /// The bridge supplied an unrecognized status code.
    Unknown,
}

impl PbftSyncFinalChainHashStatus {
    /// Stable bridge code used by CXX callers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Valid => 0,
            Self::Missing => 1,
            Self::Invalid => 2,
            Self::Unknown => 255,
        }
    }

    /// Decodes a stable bridge code into a domain status.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Valid,
            1 => Self::Missing,
            2 => Self::Invalid,
            _ => Self::Unknown,
        }
    }
}

/// Generic validation fact status for C++-originated prechecks.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncFactStatus {
    /// The fact was checked and accepted.
    Valid,
    /// The fact was checked and rejected.
    Invalid,
    /// The fact is not required for this block/period.
    NotRequired,
    /// The fact has not been checked yet in the staged C++ flow.
    NotChecked,
    /// The bridge supplied an unrecognized status code.
    Unknown,
}

impl PbftSyncFactStatus {
    /// Stable bridge code used by CXX callers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Valid => 0,
            Self::Invalid => 1,
            Self::NotRequired => 2,
            Self::NotChecked => 3,
            Self::Unknown => 255,
        }
    }

    /// Decodes a stable bridge code into a domain status.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Valid,
            1 => Self::Invalid,
            2 => Self::NotRequired,
            3 => Self::NotChecked,
            _ => Self::Unknown,
        }
    }
}

/// Side-effect intent for one synced PBFT period payload.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncPeriodAdmissionDecision {
    /// Continue processing and eventually accept the period data.
    Accept,
    /// Drop the candidate without clearing the sync queue or reporting a peer.
    Drop,
    /// Wait for FinalChain to catch up, then re-check the same candidate.
    WaitForFinalization,
    /// Clear the sync queue and report the sending peer as malicious.
    ClearAndReportPeer,
}

/// Higher-level admission action that normalizes side-effect intent for runtime
/// use while remaining side-effect-free.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncAdmissionRuntimeAction {
    /// Continue processing and eventually accept the period data.
    Accept,
    /// Drop the candidate without clearing the sync queue or reporting a peer.
    Drop,
    /// Wait for FinalChain to catch up, then re-check the same candidate.
    WaitForFinalization,
    /// Clear the sync queue and report the sending peer as malicious.
    ClearAndReportPeer,
}

impl PbftSyncAdmissionRuntimeAction {
    /// Stable bridge code used by higher-level Rust consumers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Accept => 0,
            Self::Drop => 1,
            Self::WaitForFinalization => 2,
            Self::ClearAndReportPeer => 3,
        }
    }
}

impl From<PbftSyncPeriodAdmissionDecision> for PbftSyncAdmissionRuntimeAction {
    fn from(value: PbftSyncPeriodAdmissionDecision) -> Self {
        match value {
            PbftSyncPeriodAdmissionDecision::Accept => Self::Accept,
            PbftSyncPeriodAdmissionDecision::Drop => Self::Drop,
            PbftSyncPeriodAdmissionDecision::WaitForFinalization => Self::WaitForFinalization,
            PbftSyncPeriodAdmissionDecision::ClearAndReportPeer => Self::ClearAndReportPeer,
        }
    }
}

impl From<PbftSyncAdmissionRuntimeAction> for PbftSyncPeriodAdmissionDecision {
    fn from(value: PbftSyncAdmissionRuntimeAction) -> Self {
        match value {
            PbftSyncAdmissionRuntimeAction::Accept => Self::Accept,
            PbftSyncAdmissionRuntimeAction::Drop => Self::Drop,
            PbftSyncAdmissionRuntimeAction::WaitForFinalization => Self::WaitForFinalization,
            PbftSyncAdmissionRuntimeAction::ClearAndReportPeer => Self::ClearAndReportPeer,
        }
    }
}

impl PbftSyncPeriodAdmissionDecision {
    /// Stable bridge code used by CXX callers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Accept => 0,
            Self::Drop => 1,
            Self::WaitForFinalization => 2,
            Self::ClearAndReportPeer => 3,
        }
    }
}

/// Detailed status for the planner's primary decision.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncPeriodAdmissionStatus {
    /// The candidate is admissible with the supplied facts.
    Accepted,
    /// The block is already present in local PBFT chain state.
    BlockAlreadyInChain,
    /// The candidate does not extend local head and is not ahead of local period.
    StalePeriod,
    /// The candidate does not extend local PBFT head.
    PreviousHashMismatch,
    /// Local FinalChain state has not finalized the needed block yet.
    FinalChainHashMissing,
    /// The candidate's final-chain hash conflicts with local state.
    FinalChainHashInvalid,
    /// Reward votes failed validation.
    RewardVotesInvalid,
    /// Cert votes failed validation.
    CertVotesInvalid,
    /// Pillar data failed validation.
    PillarDataInvalid,
    /// Required pillar votes failed validation.
    PillarVotesInvalid,
    /// Bridge facts used unknown status codes or otherwise invalid values.
    InvalidBridgeFacts,
}

impl PbftSyncPeriodAdmissionStatus {
    /// Stable bridge code used by CXX callers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Accepted => 0,
            Self::BlockAlreadyInChain => 1,
            Self::StalePeriod => 2,
            Self::PreviousHashMismatch => 3,
            Self::FinalChainHashMissing => 4,
            Self::FinalChainHashInvalid => 5,
            Self::RewardVotesInvalid => 6,
            Self::CertVotesInvalid => 7,
            Self::PillarDataInvalid => 10,
            Self::PillarVotesInvalid => 11,
            Self::InvalidBridgeFacts => 12,
        }
    }
}

/// Non-fatal transaction warning classification.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncTransactionWarningKind {
    /// A DAG-referenced transaction was not supplied and was not known finalized.
    MissingTransaction,
    /// A supplied period-data transaction was already finalized locally.
    FinalizedTransaction,
}

impl PbftSyncTransactionWarningKind {
    /// Stable bridge code used by CXX callers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::MissingTransaction => 1,
            Self::FinalizedTransaction => 2,
        }
    }
}

/// Warning entry returned with an otherwise accepted plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncTransactionWarning {
    /// Transaction hash associated with this warning.
    pub hash: H256,
    /// Warning classification.
    pub kind: PbftSyncTransactionWarningKind,
}

/// Compact facts for one cert vote in a synced PBFT period-data bundle.
///
/// Purpose:
/// - Lets Rust own deterministic cert-vote bundle shape and threshold checks
///   without requiring C++ to pass live `PbftVote` objects across the boundary.
///
/// Inputs:
/// - C++ supplies canonical identity fields from the decoded vote sidecar.
/// - `weight_present`, `weight`, and `live_vote_valid` are executor reports
///   from the temporary VoteManager validation path.
///
/// Invariants and edge behavior:
/// - Rust treats missing weight, invalid live validation reports, mismatched
///   period/round/type/step, and wrong block hashes as bundle rejection facts.
/// - Signature, VRF, and DPoS weight calculation remain VoteManager executor
///   effects until Slice 8 moves those ports fully into Rust.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncCertVoteFact {
    /// Canonical vote hash for diagnostics.
    pub vote_hash: H256,
    /// PBFT block hash carried by the vote.
    pub block_hash: H256,
    /// PBFT period carried by the vote.
    pub period: u64,
    /// PBFT round carried by the vote.
    pub round: u64,
    /// PBFT step carried by the vote.
    pub step: u64,
    /// Stable vote-type code carried by the vote.
    pub vote_type: u8,
    /// True when the VoteManager executor accepted the live vote check.
    pub live_vote_valid: bool,
    /// True when `weight` was materialized by the VoteManager executor.
    pub weight_present: bool,
    /// Vote weight reported by the VoteManager executor.
    pub weight: u64,
}

/// Sync cert-vote bundle fact supplied by the PBFT manager shim.
///
/// Inputs:
/// - `block_period` and `block_hash` identify the synced PBFT block.
/// - `votes` carries compact per-vote facts gathered from queued cert-vote
///   sidecars.
/// - `check_weight_threshold` controls whether Rust should enforce
///   `two_t_plus_one`; C++ uses a shape-only precheck before running live
///   VoteManager validation, then a final threshold check after weights exist.
///
/// Outputs are produced by [`validate_pbft_sync_cert_vote_bundle`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncCertVoteBundleFact {
    /// Candidate PBFT block period.
    pub block_period: u64,
    /// Candidate PBFT block hash.
    pub block_hash: H256,
    /// Compact facts for the current-round cert-vote bundle.
    pub votes: Vec<PbftSyncCertVoteFact>,
    /// Whether Rust should validate `two_t_plus_one` and summed weights.
    pub check_weight_threshold: bool,
    /// Whether C++ could load a `2t+1` threshold for the previous period.
    pub two_t_plus_one_found: bool,
    /// Required summed cert-vote weight when `two_t_plus_one_found`.
    pub two_t_plus_one: u64,
}

/// Stable rejection status for sync cert-vote bundle validation.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncCertVoteBundleStatus {
    /// Bundle accepted all requested checks.
    Accepted,
    /// The bundle is empty.
    Empty,
    /// A vote period did not match the synced block period.
    PeriodMismatch,
    /// Votes in the bundle do not all share the first vote round.
    RoundMismatch,
    /// A vote was not a cert vote.
    VoteTypeMismatch,
    /// A vote was not in certify step.
    StepMismatch,
    /// A vote targets a different block hash.
    BlockHashMismatch,
    /// The VoteManager executor rejected a live vote check.
    LiveVoteInvalid,
    /// The VoteManager executor did not materialize a vote weight.
    MissingWeight,
    /// C++ could not load the required `2t+1` threshold.
    ThresholdMissing,
    /// Summed cert-vote weight is below `2t+1`.
    InsufficientWeight,
}

impl PbftSyncCertVoteBundleStatus {
    /// Stable bridge code for CXX callers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Accepted => 0,
            Self::Empty => 1,
            Self::PeriodMismatch => 2,
            Self::RoundMismatch => 3,
            Self::VoteTypeMismatch => 4,
            Self::StepMismatch => 5,
            Self::BlockHashMismatch => 6,
            Self::LiveVoteInvalid => 7,
            Self::MissingWeight => 8,
            Self::ThresholdMissing => 9,
            Self::InsufficientWeight => 10,
        }
    }
}

/// Result of Rust-owned synced cert-vote bundle validation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncCertVoteBundleValidation {
    /// True when the bundle passed all requested checks.
    pub valid: bool,
    /// Detailed deterministic status.
    pub status: PbftSyncCertVoteBundleStatus,
    /// Summed weight of checked cert votes.
    pub total_weight: u64,
    /// Required threshold when it was checked.
    pub two_t_plus_one: u64,
    /// First vote hash that made the bundle invalid, if any.
    pub first_bad_vote_hash: H256,
}

/// Transaction references extracted from synced PBFT period data.
///
/// C++ still owns live `DagBlock` and `Transaction` objects while PBFT sync is
/// being migrated. This side-effect-free fact lets Rust own the deterministic
/// set-difference rule for deciding which DAG-referenced transaction hashes
/// must be checked against finalized transaction storage.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncTransactionQueryFact {
    /// Transaction hashes referenced by finalized DAG blocks in period-data order.
    pub dag_transaction_hashes: Vec<H256>,
    /// Transaction hashes supplied in the period data transaction list.
    pub period_data_transaction_hashes: Vec<H256>,
}

/// Rust-planned transaction lookup work for PBFT sync admission.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncTransactionQueryPlan {
    /// Unique DAG-referenced hashes that are absent from the supplied period
    /// data transactions. C++ should query finalized transaction storage for
    /// these hashes and pass non-finalized misses back into the admission fact.
    pub finalized_lookup_hashes: Vec<H256>,
}

/// Combined side-effect-free runtime plan for one PBFT sync pass.
///
/// This value couples the deterministic period-admission plan with the
/// deterministic finalized-transaction lookup plan so `processPeriodData()` can
/// consume a single return object in a Rust-enabled orchestration path.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncRuntimePlan {
    /// Runtime-normalized period-admission decision and low-level decision plan.
    pub period_admission: PbftSyncPeriodAdmissionRuntimePlan,
    /// Planned finalized-transaction lookups for DAG references in period data.
    pub transaction_query: PbftSyncTransactionQueryPlan,
}

impl PbftSyncRuntimePlan {
    /// Returns `true` when the caller may accept the period data.
    pub const fn is_accepted(&self) -> bool {
        self.period_admission.is_accepted()
    }

    /// Whether this runtime result should clear the sync queue.
    pub const fn clear_sync_queue(&self) -> bool {
        self.period_admission.clear_sync_queue()
    }

    /// Whether this runtime result should report the sender peer as malicious.
    pub const fn report_malicious_peer(&self) -> bool {
        self.period_admission.report_malicious_peer()
    }

    /// Whether this runtime result should wait for finalization.
    pub const fn wait_for_finalization(&self) -> bool {
        self.period_admission.wait_for_finalization()
    }

    /// Whether this runtime result may accept the period data.
    pub const fn accept_period_data(&self) -> bool {
        self.period_admission.accept_period_data()
    }

    /// True when finalized-transaction lookup work is required.
    pub fn requires_transaction_lookup(&self) -> bool {
        !self.transaction_query.finalized_lookup_hashes.is_empty()
    }

    /// Decompose into the low-level runtime-planner outputs.
    pub fn into_parts(
        self,
    ) -> (
        PbftSyncPeriodAdmissionRuntimePlan,
        PbftSyncTransactionQueryPlan,
    ) {
        (self.period_admission, self.transaction_query)
    }
}

/// FinalChain fact status used by the staged PBFT sync runtime.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncRuntimeFinalChainHashStatus {
    /// The final-chain hash has not been checked yet.
    NotChecked,
    /// The block's final-chain hash matches local finalized state.
    Valid,
    /// Local finalization is behind and the caller should wait/retry.
    Missing,
    /// The block's final-chain hash conflicts with local finalized state.
    Invalid,
    /// The bridge supplied an unrecognized status code.
    Unknown,
}

impl PbftSyncRuntimeFinalChainHashStatus {
    /// Stable bridge code used by CXX callers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Valid => 0,
            Self::Missing => 1,
            Self::Invalid => 2,
            Self::NotChecked => 3,
            Self::Unknown => 255,
        }
    }

    /// Decodes a stable bridge code into a domain status.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Valid,
            1 => Self::Missing,
            2 => Self::Invalid,
            3 => Self::NotChecked,
            _ => Self::Unknown,
        }
    }
}

/// High-level side-effect intent for the staged `processPeriodData` runtime.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncProcessRuntimeAction {
    /// C++ should run the check named by `next_check` and call Rust again with updated facts.
    RunCheck,
    /// C++ may accept the period data and return it to the PBFT manager caller.
    Accept,
    /// C++ should drop the candidate without peer punishment.
    Drop,
    /// C++ should wait for FinalChain and retry the same candidate.
    WaitForFinalization,
    /// C++ should clear the sync queue and report the sender peer.
    ClearAndReportPeer,
    /// The local C++/Rust bridge supplied invalid status codes or inconsistent facts.
    ContractError,
}

impl PbftSyncProcessRuntimeAction {
    /// Stable bridge code used by CXX callers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::RunCheck => 0,
            Self::Accept => 1,
            Self::Drop => 2,
            Self::WaitForFinalization => 3,
            Self::ClearAndReportPeer => 4,
            Self::ContractError => 5,
        }
    }
}

/// Next C++ live-object check requested by the staged PBFT sync runtime.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncProcessRuntimeNextCheck {
    /// No more live checks are required.
    None,
    /// Run `validateFinalChainHash`.
    ValidateFinalChainHash,
    /// Run reward-vote validation.
    CheckRewardVotes,
    /// Run cert-vote validation.
    ValidateCertVotes,
    /// Run transaction finalized/missing checks.
    CheckTransactions,
    /// Run pillar-vote validation.
    ValidatePillarVotes,
}

impl PbftSyncProcessRuntimeNextCheck {
    /// Stable bridge code used by CXX callers.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::None => 0,
            Self::ValidateFinalChainHash => 1,
            Self::CheckRewardVotes => 2,
            Self::ValidateCertVotes => 3,
            Self::CheckTransactions => 4,
            Self::ValidatePillarVotes => 6,
        }
    }

    /// Decodes a stable bridge check code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::None,
            1 => Self::ValidateFinalChainHash,
            2 => Self::CheckRewardVotes,
            3 => Self::ValidateCertVotes,
            4 => Self::CheckTransactions,
            6 => Self::ValidatePillarVotes,
            _ => Self::None,
        }
    }
}

/// Complete side-effect-free fact bundle for staged PBFT sync runtime planning.
///
/// The runtime fact differs from the low-level admission fact by treating
/// `NotChecked` as a request for the next C++ live-object operation instead of
/// as an accepted precondition. C++ still owns the live checks and calls this
/// planner again with updated facts after each requested operation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncProcessPeriodDataRuntimeFact {
    /// Candidate block period from synced period-data.
    pub block_period: u64,
    /// Candidate block previous hash.
    pub block_prev_hash: H256,
    /// Local PBFT chain head hash at decision time.
    pub chain_last_hash: H256,
    /// Local PBFT chain head period at decision time.
    pub chain_last_period: u64,
    /// True when candidate block hash already exists in local chain state.
    pub block_in_chain: bool,
    /// FinalChain state-root validation status for the candidate.
    pub final_chain_hash_status: PbftSyncRuntimeFinalChainHashStatus,
    /// Reward-vote validation status for the candidate.
    pub reward_votes_status: PbftSyncFactStatus,
    /// Cert-vote validation status for the candidate.
    pub cert_votes_status: PbftSyncFactStatus,
    /// Transaction validation status after C++ performs live TransactionManager checks.
    pub transactions_status: PbftSyncFactStatus,
    /// Transaction hashes referenced by finalized DAG blocks in period-data order.
    pub dag_transaction_hashes: Vec<H256>,
    /// Transaction hashes supplied in the period data transaction list.
    pub period_data_transaction_hashes: Vec<H256>,
    /// DAG-referenced transaction hashes missing from supplied period data and not finalized locally.
    pub missing_transaction_hashes: Vec<H256>,
    /// Supplied transaction hashes that are already finalized locally when exact hashes are known.
    pub finalized_transaction_hashes: Vec<H256>,
    /// True when local checks found at least one supplied transaction already finalized.
    pub contains_finalized_transactions: bool,
    /// Pillar-data validation status for the candidate.
    pub pillar_data_status: PbftSyncFactStatus,
    /// Whether the current hardfork requires PBFT block extra data.
    pub extra_data_required: bool,
    /// Whether the synced PBFT block carried extra data.
    pub extra_data_present: bool,
    /// Whether synced PBFT block extra data carried a pillar block hash.
    pub extra_data_pillar_block_hash_present: bool,
    /// Whether this period requires pillar-vote validation.
    pub pillar_votes_required: bool,
    /// Whether synced period data carried pillar-vote sidecars.
    pub pillar_votes_present: bool,
    /// Pillar-vote validation status for the candidate.
    pub pillar_votes_status: PbftSyncFactStatus,
    /// Whether synced period data carried previous-block cert votes.
    pub previous_cert_votes_present: bool,
    /// Whether the first previous-block cert vote already had a weight.
    pub previous_cert_first_vote_has_weight: bool,
}

/// Staged runtime plan for `processPeriodData` orchestration.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncProcessPeriodDataRuntimePlan {
    /// High-level runtime action requested from C++.
    pub runtime_action: PbftSyncProcessRuntimeAction,
    /// Detailed status explaining `runtime_action`.
    pub status: PbftSyncPeriodAdmissionStatus,
    /// Next C++ live-object check to run when `runtime_action` is `RunCheck`.
    pub next_check: PbftSyncProcessRuntimeNextCheck,
    /// Whether caller should clear the remaining sync queue.
    pub clear_sync_queue: bool,
    /// Whether caller should report the sender as malicious.
    pub report_malicious_peer: bool,
    /// Whether caller should wait for FinalChain finalization and retry.
    pub wait_for_finalization: bool,
    /// Whether caller may accept the period data.
    pub accept_period_data: bool,
    /// Whether the same popped sync candidate should be retried after waiting.
    pub retry_same_candidate: bool,
    /// Whether C++ should replace unweighted previous-block cert votes with checked reward votes.
    pub replace_previous_block_cert_votes: bool,
    /// Planned finalized-transaction lookups for a requested transaction check.
    pub transaction_query: PbftSyncTransactionQueryPlan,
    /// Non-fatal transaction warnings carried with accepted plans.
    pub warnings: Vec<PbftSyncTransactionWarning>,
    /// Non-fatal compatibility signal for finalized transactions when exact hashes are not available.
    pub contains_finalized_transaction_warning: bool,
}

/// Rust-owned action for draining the PBFT sync queue.
///
/// The planner is side-effect-free. Its native service owner consumes queue
/// cleanup internally, while C++ executes only remaining `PeriodData`, network,
/// and PBFT-chain effects.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncQueueDrainAction {
    /// Remove stale queue entries before the drain loop starts.
    CleanOldData,
    /// Pop and validate one queued candidate through the existing period-data runtime.
    PopAndProcess,
    /// Push the accepted period data into the PBFT chain/finalization path.
    PushAccepted,
    /// Publish the post-push PBFT sync state through the network executor.
    UpdateSyncState,
    /// No more drain work should run.
    Stop,
    /// C++ called the session API out of order or supplied an invalid report.
    ContractError,
}

impl PbftSyncQueueDrainAction {
    /// Stable bridge code for queue-drain actions.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::CleanOldData => 0,
            Self::PopAndProcess => 1,
            Self::PushAccepted => 2,
            Self::UpdateSyncState => 3,
            Self::Stop => 4,
            Self::ContractError => 255,
        }
    }

    /// Decodes a stable bridge action code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::CleanOldData,
            1 => Self::PopAndProcess,
            2 => Self::PushAccepted,
            3 => Self::UpdateSyncState,
            4 => Self::Stop,
            _ => Self::ContractError,
        }
    }
}

/// Stable status for the PBFT sync queue-drain session.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncQueueDrainStatus {
    /// The session can continue.
    Active,
    /// The queue is empty or the session intentionally stopped.
    Complete,
    /// The accepted-period push failed and the drain loop must stop.
    PushFailed,
    /// A C++ executor operation failed unexpectedly.
    ExecutorFailed,
    /// C++ called the session API out of order or supplied an invalid report.
    InvalidReport,
}

impl PbftSyncQueueDrainStatus {
    /// Stable bridge code for queue-drain status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::Complete => 1,
            Self::PushFailed => 2,
            Self::ExecutorFailed => 3,
            Self::InvalidReport => 255,
        }
    }
}

/// One Rust-planned queue-drain step for the C++ executor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncQueueDrainStep {
    /// Action C++ should execute.
    pub action: PbftSyncQueueDrainAction,
    /// Session status associated with this step.
    pub status: PbftSyncQueueDrainStatus,
    /// Current PBFT period to use when `action == CleanOldData`.
    pub clean_before_period: u64,
    /// Whether C++ should continue the drain loop after this step is handled.
    pub can_continue: bool,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

/// C++ executor report for one queue-drain action.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncQueueDrainReport {
    /// Action C++ attempted to execute.
    pub action: PbftSyncQueueDrainAction,
    /// Whether the requested executor operation completed.
    pub success: bool,
    /// Whether `PopAndProcess` produced accepted period data for the push step.
    pub accepted_period_data: bool,
}

/// Rust validation result for one queue-drain executor report.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncQueueDrainReportResult {
    /// Report status selected by Rust.
    pub status: PbftSyncQueueDrainStatus,
    /// Whether C++ may ask Rust for the next queue-drain step.
    pub can_continue: bool,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PbftSyncQueueDrainState {
    Start,
    NeedPop,
    NeedPush,
    NeedSyncStateUpdate,
    Awaiting(PbftSyncQueueDrainAction),
    Complete,
}

/// Rust-owned PBFT sync queue-drain session.
///
/// Purpose:
/// - Owns the outer `pushSyncedPbftBlocksIntoChain` loop decisions: cleanup,
///   pop/process, accepted-period push, sync-state update, retry/continue, and
///   stop.
///
/// Inputs/outputs:
/// - `next` receives the current compatibility queue size and PBFT period from
///   C++ and returns one executor action.
/// - `report` accepts the result of exactly the action Rust last requested.
///
/// Invariants and edge behavior:
/// - C++ may not request a second action before reporting the previous one.
/// - `PushAccepted` is only planned after a `PopAndProcess` report accepted a
///   candidate.
/// - Push failure is a deliberate stop, matching the legacy break behavior.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncQueueDrainSession {
    state: PbftSyncQueueDrainState,
}

impl Default for PbftSyncQueueDrainSession {
    fn default() -> Self {
        Self {
            state: PbftSyncQueueDrainState::Start,
        }
    }
}

fn queue_drain_step(
    action: PbftSyncQueueDrainAction,
    status: PbftSyncQueueDrainStatus,
    clean_before_period: u64,
    can_continue: bool,
    error_code: &'static str,
) -> PbftSyncQueueDrainStep {
    PbftSyncQueueDrainStep {
        action,
        status,
        clean_before_period,
        can_continue,
        error_code,
    }
}

fn queue_drain_report_result(
    status: PbftSyncQueueDrainStatus,
    can_continue: bool,
    error_code: &'static str,
) -> PbftSyncQueueDrainReportResult {
    PbftSyncQueueDrainReportResult {
        status,
        can_continue,
        error_code,
    }
}

/// Creates a Rust-owned PBFT sync queue-drain session.
#[must_use]
pub fn create_pbft_sync_queue_drain_session() -> PbftSyncQueueDrainSession {
    PbftSyncQueueDrainSession::default()
}

/// Returns the next Rust-owned PBFT sync queue-drain step.
///
/// The native service supplies its owned queue size and current period. The
/// planner uses those facts to decide whether another pop is required or the
/// drain is complete.
#[must_use]
pub fn next_pbft_sync_queue_drain_step(
    session: &mut PbftSyncQueueDrainSession,
    queue_size: usize,
    current_period: u64,
) -> PbftSyncQueueDrainStep {
    match session.state {
        PbftSyncQueueDrainState::Start => {
            session.state =
                PbftSyncQueueDrainState::Awaiting(PbftSyncQueueDrainAction::CleanOldData);
            queue_drain_step(
                PbftSyncQueueDrainAction::CleanOldData,
                PbftSyncQueueDrainStatus::Active,
                current_period,
                true,
                "",
            )
        }
        PbftSyncQueueDrainState::NeedPop => {
            if queue_size == 0 {
                session.state = PbftSyncQueueDrainState::Complete;
                queue_drain_step(
                    PbftSyncQueueDrainAction::Stop,
                    PbftSyncQueueDrainStatus::Complete,
                    current_period,
                    false,
                    "PBFT_SYNC_QUEUE_DRAIN_EMPTY",
                )
            } else {
                session.state =
                    PbftSyncQueueDrainState::Awaiting(PbftSyncQueueDrainAction::PopAndProcess);
                queue_drain_step(
                    PbftSyncQueueDrainAction::PopAndProcess,
                    PbftSyncQueueDrainStatus::Active,
                    current_period,
                    true,
                    "",
                )
            }
        }
        PbftSyncQueueDrainState::NeedPush => {
            session.state =
                PbftSyncQueueDrainState::Awaiting(PbftSyncQueueDrainAction::PushAccepted);
            queue_drain_step(
                PbftSyncQueueDrainAction::PushAccepted,
                PbftSyncQueueDrainStatus::Active,
                current_period,
                true,
                "",
            )
        }
        PbftSyncQueueDrainState::NeedSyncStateUpdate => {
            session.state =
                PbftSyncQueueDrainState::Awaiting(PbftSyncQueueDrainAction::UpdateSyncState);
            queue_drain_step(
                PbftSyncQueueDrainAction::UpdateSyncState,
                PbftSyncQueueDrainStatus::Active,
                current_period,
                true,
                "",
            )
        }
        PbftSyncQueueDrainState::Complete => queue_drain_step(
            PbftSyncQueueDrainAction::Stop,
            PbftSyncQueueDrainStatus::Complete,
            current_period,
            false,
            "PBFT_SYNC_QUEUE_DRAIN_COMPLETE",
        ),
        PbftSyncQueueDrainState::Awaiting(_) => queue_drain_step(
            PbftSyncQueueDrainAction::ContractError,
            PbftSyncQueueDrainStatus::InvalidReport,
            current_period,
            false,
            "PBFT_SYNC_QUEUE_DRAIN_NEXT_BEFORE_REPORT",
        ),
    }
}

/// Reports one C++ executor result back to the Rust queue-drain session.
#[must_use]
pub fn report_pbft_sync_queue_drain_step(
    session: &mut PbftSyncQueueDrainSession,
    report: PbftSyncQueueDrainReport,
) -> PbftSyncQueueDrainReportResult {
    let PbftSyncQueueDrainState::Awaiting(expected_action) = session.state else {
        session.state = PbftSyncQueueDrainState::Complete;
        return queue_drain_report_result(
            PbftSyncQueueDrainStatus::InvalidReport,
            false,
            "PBFT_SYNC_QUEUE_DRAIN_REPORT_WITHOUT_ACTION",
        );
    };

    if report.action != expected_action || report.action == PbftSyncQueueDrainAction::ContractError
    {
        session.state = PbftSyncQueueDrainState::Complete;
        return queue_drain_report_result(
            PbftSyncQueueDrainStatus::InvalidReport,
            false,
            "PBFT_SYNC_QUEUE_DRAIN_ACTION_MISMATCH",
        );
    }

    match expected_action {
        PbftSyncQueueDrainAction::CleanOldData => {
            if report.success {
                session.state = PbftSyncQueueDrainState::NeedPop;
                queue_drain_report_result(PbftSyncQueueDrainStatus::Active, true, "")
            } else {
                session.state = PbftSyncQueueDrainState::Complete;
                queue_drain_report_result(
                    PbftSyncQueueDrainStatus::ExecutorFailed,
                    false,
                    "PBFT_SYNC_QUEUE_DRAIN_CLEAN_FAILED",
                )
            }
        }
        PbftSyncQueueDrainAction::PopAndProcess => {
            if !report.success {
                session.state = PbftSyncQueueDrainState::Complete;
                return queue_drain_report_result(
                    PbftSyncQueueDrainStatus::ExecutorFailed,
                    false,
                    "PBFT_SYNC_QUEUE_DRAIN_PROCESS_FAILED",
                );
            }
            session.state = if report.accepted_period_data {
                PbftSyncQueueDrainState::NeedPush
            } else {
                PbftSyncQueueDrainState::NeedPop
            };
            queue_drain_report_result(PbftSyncQueueDrainStatus::Active, true, "")
        }
        PbftSyncQueueDrainAction::PushAccepted => {
            if report.success {
                session.state = PbftSyncQueueDrainState::NeedSyncStateUpdate;
                queue_drain_report_result(PbftSyncQueueDrainStatus::Active, true, "")
            } else {
                session.state = PbftSyncQueueDrainState::Complete;
                queue_drain_report_result(
                    PbftSyncQueueDrainStatus::PushFailed,
                    false,
                    "PBFT_SYNC_QUEUE_DRAIN_PUSH_FAILED",
                )
            }
        }
        PbftSyncQueueDrainAction::UpdateSyncState => {
            if report.success {
                session.state = PbftSyncQueueDrainState::NeedPop;
                queue_drain_report_result(PbftSyncQueueDrainStatus::Active, true, "")
            } else {
                session.state = PbftSyncQueueDrainState::Complete;
                queue_drain_report_result(
                    PbftSyncQueueDrainStatus::ExecutorFailed,
                    false,
                    "PBFT_SYNC_QUEUE_DRAIN_SYNC_STATE_UPDATE_FAILED",
                )
            }
        }
        PbftSyncQueueDrainAction::Stop | PbftSyncQueueDrainAction::ContractError => {
            session.state = PbftSyncQueueDrainState::Complete;
            queue_drain_report_result(
                PbftSyncQueueDrainStatus::InvalidReport,
                false,
                "PBFT_SYNC_QUEUE_DRAIN_TERMINAL_ACTION_REPORTED",
            )
        }
    }
}

/// Input fact for one PBFT sync-period admission request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncPeriodAdmissionFact {
    /// Candidate block period from synced period-data.
    pub block_period: u64,
    /// Candidate block previous hash.
    pub block_prev_hash: H256,
    /// Local PBFT chain head hash at decision time.
    pub chain_last_hash: H256,
    /// Local PBFT chain head period at decision time.
    pub chain_last_period: u64,
    /// True when candidate block hash already exists in local chain state.
    pub block_in_chain: bool,
    /// FinalChain state-root validation status for the candidate.
    pub final_chain_hash_status: PbftSyncFinalChainHashStatus,
    /// Reward-vote validation status for the candidate.
    pub reward_votes_status: PbftSyncFactStatus,
    /// Cert-vote validation status for the candidate.
    pub cert_votes_status: PbftSyncFactStatus,
    /// DAG-referenced transaction hashes missing from supplied period data and not finalized locally.
    pub missing_transaction_hashes: Vec<H256>,
    /// Supplied transaction hashes that are already finalized locally.
    pub finalized_transaction_hashes: Vec<H256>,
    /// True when local checks found at least one supplied transaction already finalized.
    pub contains_finalized_transactions: bool,
    /// Pillar-data validation status for the candidate.
    pub pillar_data_status: PbftSyncFactStatus,
    /// Pillar-vote validation status for the candidate.
    pub pillar_votes_status: PbftSyncFactStatus,
}

/// Output plan for one PBFT sync-period admission request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncPeriodAdmissionPlan {
    /// Primary side-effect intent for this candidate.
    pub decision: PbftSyncPeriodAdmissionDecision,
    /// Detailed status explaining `decision`.
    pub status: PbftSyncPeriodAdmissionStatus,
    /// Whether caller should clear the remaining sync queue.
    pub clear_sync_queue: bool,
    /// Whether caller should report the sender as malicious.
    pub report_malicious_peer: bool,
    /// Whether caller should wait for FinalChain finalization and retry.
    pub wait_for_finalization: bool,
    /// Whether caller may accept the period data.
    pub accept_period_data: bool,
    /// Non-fatal transaction warnings carried with accepted plans.
    pub warnings: Vec<PbftSyncTransactionWarning>,
    /// Non-fatal compatibility signal for finalized transactions when exact hashes are not available.
    pub contains_finalized_transaction_warning: bool,
}

impl PbftSyncPeriodAdmissionPlan {
    /// Returns `true` when the caller may accept the period data.
    pub const fn is_accepted(&self) -> bool {
        self.accept_period_data
    }
}

/// Higher-level side-effect-free runtime plan built from the base planner.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncPeriodAdmissionRuntimePlan {
    /// Normalized runtime action for the candidate.
    pub action: PbftSyncAdmissionRuntimeAction,
    /// Full low-level planner output preserved for C++ boundary mapping.
    pub plan: PbftSyncPeriodAdmissionPlan,
}

impl PbftSyncPeriodAdmissionRuntimePlan {
    /// Returns `true` when the caller may accept the period data.
    pub const fn is_accepted(&self) -> bool {
        matches!(self.action, PbftSyncAdmissionRuntimeAction::Accept)
    }

    /// Whether this runtime result should clear the sync queue.
    pub const fn clear_sync_queue(&self) -> bool {
        self.plan.clear_sync_queue
    }

    /// Whether this runtime result should report the sender peer.
    pub const fn report_malicious_peer(&self) -> bool {
        self.plan.report_malicious_peer
    }

    /// Whether this runtime result should wait for finalization.
    pub const fn wait_for_finalization(&self) -> bool {
        self.plan.wait_for_finalization
    }

    /// Whether this runtime result may accept the period data.
    pub const fn accept_period_data(&self) -> bool {
        self.plan.accept_period_data
    }

    /// Expose the underlying low-level plan for bridge conversion.
    pub fn into_plan(self) -> PbftSyncPeriodAdmissionPlan {
        self.plan
    }
}

fn plan(
    decision: PbftSyncPeriodAdmissionDecision,
    status: PbftSyncPeriodAdmissionStatus,
) -> PbftSyncPeriodAdmissionPlan {
    PbftSyncPeriodAdmissionPlan {
        decision,
        status,
        clear_sync_queue: matches!(
            decision,
            PbftSyncPeriodAdmissionDecision::ClearAndReportPeer
        ),
        report_malicious_peer: matches!(
            decision,
            PbftSyncPeriodAdmissionDecision::ClearAndReportPeer
        ),
        wait_for_finalization: matches!(
            decision,
            PbftSyncPeriodAdmissionDecision::WaitForFinalization
        ),
        accept_period_data: matches!(decision, PbftSyncPeriodAdmissionDecision::Accept),
        warnings: Vec::new(),
        contains_finalized_transaction_warning: false,
    }
}

fn invalid_bridge_facts() -> PbftSyncPeriodAdmissionPlan {
    plan(
        PbftSyncPeriodAdmissionDecision::ClearAndReportPeer,
        PbftSyncPeriodAdmissionStatus::InvalidBridgeFacts,
    )
}

fn warn_transactions(
    hash_list: Vec<H256>,
    kind: PbftSyncTransactionWarningKind,
) -> Vec<PbftSyncTransactionWarning> {
    hash_list
        .into_iter()
        .map(|hash| PbftSyncTransactionWarning { hash, kind })
        .collect()
}

/// Plans transaction-finalization lookups for synced PBFT period data.
///
/// The output preserves first-seen DAG transaction order and removes duplicate
/// DAG references. Hashes already supplied in the period data transaction list
/// are skipped. This mirrors the legacy sync check while moving the
/// deterministic set-difference rule out of the C++ shim.
pub fn plan_pbft_sync_transaction_query(
    fact: PbftSyncTransactionQueryFact,
) -> PbftSyncTransactionQueryPlan {
    let supplied_transactions = fact
        .period_data_transaction_hashes
        .into_iter()
        .collect::<HashSet<_>>();
    let mut seen_dag_transactions = HashSet::new();
    let mut finalized_lookup_hashes = Vec::new();

    for hash in fact.dag_transaction_hashes {
        if !seen_dag_transactions.insert(hash) {
            continue;
        }
        if supplied_transactions.contains(&hash) {
            continue;
        }
        finalized_lookup_hashes.push(hash);
    }

    PbftSyncTransactionQueryPlan {
        finalized_lookup_hashes,
    }
}

fn cert_vote_validation_result(
    status: PbftSyncCertVoteBundleStatus,
    total_weight: u64,
    two_t_plus_one: u64,
    first_bad_vote_hash: H256,
) -> PbftSyncCertVoteBundleValidation {
    PbftSyncCertVoteBundleValidation {
        valid: status == PbftSyncCertVoteBundleStatus::Accepted,
        status,
        total_weight,
        two_t_plus_one,
        first_bad_vote_hash,
    }
}

/// Validates deterministic facts for a synced PBFT cert-vote bundle.
///
/// Inputs:
/// - `fact`: compact cert-vote facts gathered by C++ from queued vote sidecars
///   and temporary VoteManager executor reports.
///
/// Outputs:
/// - A side-effect-free validation result with stable status, total weight,
///   checked threshold, and first bad vote hash.
///
/// Invariants and edge behavior:
/// - Cert votes must be non-empty, match the candidate block period/hash, use
///   the cert-vote type code (`3`), use the certify step (`3`), and share the
///   first vote round.
/// - When `check_weight_threshold` is true, Rust also requires every vote to
///   have passed live VoteManager validation, every weight to be present, the
///   threshold to be available, and summed weight to satisfy `2t+1`.
#[must_use]
pub fn validate_pbft_sync_cert_vote_bundle(
    fact: PbftSyncCertVoteBundleFact,
) -> PbftSyncCertVoteBundleValidation {
    const CERT_VOTE_TYPE: u8 = 3;
    const CERTIFY_STEP: u64 = 3;

    if fact.votes.is_empty() {
        return cert_vote_validation_result(
            PbftSyncCertVoteBundleStatus::Empty,
            0,
            fact.two_t_plus_one,
            H256::zero(),
        );
    }

    let first_round = fact.votes[0].round;
    let mut total_weight = 0_u64;
    for vote in &fact.votes {
        if vote.period != fact.block_period {
            return cert_vote_validation_result(
                PbftSyncCertVoteBundleStatus::PeriodMismatch,
                total_weight,
                fact.two_t_plus_one,
                vote.vote_hash,
            );
        }
        if vote.round != first_round {
            return cert_vote_validation_result(
                PbftSyncCertVoteBundleStatus::RoundMismatch,
                total_weight,
                fact.two_t_plus_one,
                vote.vote_hash,
            );
        }
        if vote.vote_type != CERT_VOTE_TYPE {
            return cert_vote_validation_result(
                PbftSyncCertVoteBundleStatus::VoteTypeMismatch,
                total_weight,
                fact.two_t_plus_one,
                vote.vote_hash,
            );
        }
        if vote.step != CERTIFY_STEP {
            return cert_vote_validation_result(
                PbftSyncCertVoteBundleStatus::StepMismatch,
                total_weight,
                fact.two_t_plus_one,
                vote.vote_hash,
            );
        }
        if vote.block_hash != fact.block_hash {
            return cert_vote_validation_result(
                PbftSyncCertVoteBundleStatus::BlockHashMismatch,
                total_weight,
                fact.two_t_plus_one,
                vote.vote_hash,
            );
        }
        if fact.check_weight_threshold {
            if !vote.live_vote_valid {
                return cert_vote_validation_result(
                    PbftSyncCertVoteBundleStatus::LiveVoteInvalid,
                    total_weight,
                    fact.two_t_plus_one,
                    vote.vote_hash,
                );
            }
            if !vote.weight_present {
                return cert_vote_validation_result(
                    PbftSyncCertVoteBundleStatus::MissingWeight,
                    total_weight,
                    fact.two_t_plus_one,
                    vote.vote_hash,
                );
            }
            total_weight = total_weight.saturating_add(vote.weight);
        }
    }

    if fact.check_weight_threshold {
        if !fact.two_t_plus_one_found {
            return cert_vote_validation_result(
                PbftSyncCertVoteBundleStatus::ThresholdMissing,
                total_weight,
                fact.two_t_plus_one,
                H256::zero(),
            );
        }
        if total_weight < fact.two_t_plus_one {
            return cert_vote_validation_result(
                PbftSyncCertVoteBundleStatus::InsufficientWeight,
                total_weight,
                fact.two_t_plus_one,
                H256::zero(),
            );
        }
    }

    cert_vote_validation_result(
        PbftSyncCertVoteBundleStatus::Accepted,
        total_weight,
        fact.two_t_plus_one,
        H256::zero(),
    )
}

fn plan_fact_status_rejection(
    status: PbftSyncFactStatus,
    invalid_status: PbftSyncPeriodAdmissionStatus,
) -> Option<PbftSyncPeriodAdmissionPlan> {
    match status {
        PbftSyncFactStatus::Valid
        | PbftSyncFactStatus::NotRequired
        | PbftSyncFactStatus::NotChecked => None,
        PbftSyncFactStatus::Invalid => Some(plan(
            PbftSyncPeriodAdmissionDecision::ClearAndReportPeer,
            invalid_status,
        )),
        PbftSyncFactStatus::Unknown => Some(invalid_bridge_facts()),
    }
}

/// Computes one deterministic sync-period admission plan.
///
/// The decision precedence intentionally mirrors the current PBFT sync
/// processing order:
/// already-known and stale candidates drop, non-stale previous-hash mismatch
/// clears/reports, missing FinalChain state waits, invalid proofs clear/report,
/// and transaction quality facts stay warnings on accepted plans.
pub fn plan_pbft_sync_period_admission(
    fact: PbftSyncPeriodAdmissionFact,
) -> PbftSyncPeriodAdmissionPlan {
    if fact.block_in_chain {
        return plan(
            PbftSyncPeriodAdmissionDecision::Drop,
            PbftSyncPeriodAdmissionStatus::BlockAlreadyInChain,
        );
    }

    if fact.block_prev_hash != fact.chain_last_hash && fact.block_period <= fact.chain_last_period {
        return plan(
            PbftSyncPeriodAdmissionDecision::Drop,
            PbftSyncPeriodAdmissionStatus::StalePeriod,
        );
    }

    if fact.block_prev_hash != fact.chain_last_hash {
        return plan(
            PbftSyncPeriodAdmissionDecision::ClearAndReportPeer,
            PbftSyncPeriodAdmissionStatus::PreviousHashMismatch,
        );
    }

    match fact.final_chain_hash_status {
        PbftSyncFinalChainHashStatus::Valid => {}
        PbftSyncFinalChainHashStatus::Missing => {
            return plan(
                PbftSyncPeriodAdmissionDecision::WaitForFinalization,
                PbftSyncPeriodAdmissionStatus::FinalChainHashMissing,
            );
        }
        PbftSyncFinalChainHashStatus::Invalid => {
            return plan(
                PbftSyncPeriodAdmissionDecision::ClearAndReportPeer,
                PbftSyncPeriodAdmissionStatus::FinalChainHashInvalid,
            );
        }
        PbftSyncFinalChainHashStatus::Unknown => return invalid_bridge_facts(),
    }

    if let Some(rejection) = plan_fact_status_rejection(
        fact.reward_votes_status,
        PbftSyncPeriodAdmissionStatus::RewardVotesInvalid,
    ) {
        return rejection;
    }
    if let Some(rejection) = plan_fact_status_rejection(
        fact.cert_votes_status,
        PbftSyncPeriodAdmissionStatus::CertVotesInvalid,
    ) {
        return rejection;
    }
    if let Some(rejection) = plan_fact_status_rejection(
        fact.pillar_data_status,
        PbftSyncPeriodAdmissionStatus::PillarDataInvalid,
    ) {
        return rejection;
    }
    if let Some(rejection) = plan_fact_status_rejection(
        fact.pillar_votes_status,
        PbftSyncPeriodAdmissionStatus::PillarVotesInvalid,
    ) {
        return rejection;
    }

    let mut accepted = plan(
        PbftSyncPeriodAdmissionDecision::Accept,
        PbftSyncPeriodAdmissionStatus::Accepted,
    );
    accepted.warnings.extend(warn_transactions(
        fact.missing_transaction_hashes,
        PbftSyncTransactionWarningKind::MissingTransaction,
    ));
    accepted.warnings.extend(warn_transactions(
        fact.finalized_transaction_hashes,
        PbftSyncTransactionWarningKind::FinalizedTransaction,
    ));
    accepted.contains_finalized_transaction_warning = fact.contains_finalized_transactions;
    accepted
}

/// Builds a higher-level runtime plan from deterministic planner output.
pub fn plan_pbft_sync_period_admission_runtime(
    fact: PbftSyncPeriodAdmissionFact,
) -> PbftSyncPeriodAdmissionRuntimePlan {
    let plan = plan_pbft_sync_period_admission(fact);
    PbftSyncPeriodAdmissionRuntimePlan {
        action: plan.decision.into(),
        plan,
    }
}

/// Builds one combined side-effect-free runtime plan for processPeriodData orchestration.
///
/// The return value does not perform I/O or mutation. It only composes:
/// - finalized-transaction lookup planning from raw DAG/period transaction refs
/// - period-admission decision planning from pre-checked admission facts
pub fn plan_pbft_sync_runtime(
    admission_fact: PbftSyncPeriodAdmissionFact,
    transaction_query_fact: PbftSyncTransactionQueryFact,
) -> PbftSyncRuntimePlan {
    PbftSyncRuntimePlan {
        period_admission: plan_pbft_sync_period_admission_runtime(admission_fact),
        transaction_query: plan_pbft_sync_transaction_query(transaction_query_fact),
    }
}

fn runtime_plan(
    runtime_action: PbftSyncProcessRuntimeAction,
    status: PbftSyncPeriodAdmissionStatus,
    next_check: PbftSyncProcessRuntimeNextCheck,
    transaction_query: PbftSyncTransactionQueryPlan,
    replace_previous_block_cert_votes: bool,
) -> PbftSyncProcessPeriodDataRuntimePlan {
    PbftSyncProcessPeriodDataRuntimePlan {
        runtime_action,
        status,
        next_check,
        clear_sync_queue: matches!(
            runtime_action,
            PbftSyncProcessRuntimeAction::ClearAndReportPeer
        ),
        report_malicious_peer: matches!(
            runtime_action,
            PbftSyncProcessRuntimeAction::ClearAndReportPeer
        ),
        wait_for_finalization: matches!(
            runtime_action,
            PbftSyncProcessRuntimeAction::WaitForFinalization
        ),
        accept_period_data: matches!(runtime_action, PbftSyncProcessRuntimeAction::Accept),
        retry_same_candidate: matches!(
            runtime_action,
            PbftSyncProcessRuntimeAction::WaitForFinalization
        ),
        replace_previous_block_cert_votes,
        transaction_query,
        warnings: Vec::new(),
        contains_finalized_transaction_warning: false,
    }
}

fn runtime_contract_error(
    transaction_query: PbftSyncTransactionQueryPlan,
) -> PbftSyncProcessPeriodDataRuntimePlan {
    runtime_plan(
        PbftSyncProcessRuntimeAction::ContractError,
        PbftSyncPeriodAdmissionStatus::InvalidBridgeFacts,
        PbftSyncProcessRuntimeNextCheck::None,
        transaction_query,
        false,
    )
}

fn derive_pillar_data_status(fact: &PbftSyncProcessPeriodDataRuntimeFact) -> PbftSyncFactStatus {
    if fact.pillar_data_status != PbftSyncFactStatus::NotChecked {
        return fact.pillar_data_status;
    }

    let extra_data_valid = if fact.extra_data_required {
        fact.extra_data_present
            && if fact.pillar_votes_required {
                fact.extra_data_pillar_block_hash_present
            } else {
                !fact.extra_data_pillar_block_hash_present
            }
    } else {
        !fact.extra_data_present
    };
    let pillar_votes_presence_valid = fact.pillar_votes_present == fact.pillar_votes_required;

    if extra_data_valid && pillar_votes_presence_valid {
        PbftSyncFactStatus::Valid
    } else {
        PbftSyncFactStatus::Invalid
    }
}

/// Plans the next side-effect-free PBFT sync runtime action for `processPeriodData`.
///
/// C++ calls this planner after collecting the facts it already has. When the
/// returned action is `RunCheck`, C++ performs only the named live check,
/// updates the corresponding fact status, and calls the planner again. This
/// keeps C++ side effects staged while moving the branch-ordering contract into
/// Rust.
pub fn plan_pbft_sync_process_period_data_runtime(
    fact: PbftSyncProcessPeriodDataRuntimeFact,
) -> PbftSyncProcessPeriodDataRuntimePlan {
    let pillar_data_status = derive_pillar_data_status(&fact);
    let transaction_query = plan_pbft_sync_transaction_query(PbftSyncTransactionQueryFact {
        dag_transaction_hashes: fact.dag_transaction_hashes,
        period_data_transaction_hashes: fact.period_data_transaction_hashes,
    });
    let replace_previous_block_cert_votes =
        fact.previous_cert_votes_present && !fact.previous_cert_first_vote_has_weight;

    if fact.block_in_chain {
        return runtime_plan(
            PbftSyncProcessRuntimeAction::Drop,
            PbftSyncPeriodAdmissionStatus::BlockAlreadyInChain,
            PbftSyncProcessRuntimeNextCheck::None,
            transaction_query,
            replace_previous_block_cert_votes,
        );
    }

    if fact.block_prev_hash != fact.chain_last_hash && fact.block_period <= fact.chain_last_period {
        return runtime_plan(
            PbftSyncProcessRuntimeAction::Drop,
            PbftSyncPeriodAdmissionStatus::StalePeriod,
            PbftSyncProcessRuntimeNextCheck::None,
            transaction_query,
            replace_previous_block_cert_votes,
        );
    }

    if fact.block_prev_hash != fact.chain_last_hash {
        return runtime_plan(
            PbftSyncProcessRuntimeAction::ClearAndReportPeer,
            PbftSyncPeriodAdmissionStatus::PreviousHashMismatch,
            PbftSyncProcessRuntimeNextCheck::None,
            transaction_query,
            replace_previous_block_cert_votes,
        );
    }

    match fact.final_chain_hash_status {
        PbftSyncRuntimeFinalChainHashStatus::NotChecked => {
            return runtime_plan(
                PbftSyncProcessRuntimeAction::RunCheck,
                PbftSyncPeriodAdmissionStatus::Accepted,
                PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash,
                transaction_query,
                replace_previous_block_cert_votes,
            );
        }
        PbftSyncRuntimeFinalChainHashStatus::Valid => {}
        PbftSyncRuntimeFinalChainHashStatus::Missing => {
            return runtime_plan(
                PbftSyncProcessRuntimeAction::WaitForFinalization,
                PbftSyncPeriodAdmissionStatus::FinalChainHashMissing,
                PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash,
                transaction_query,
                replace_previous_block_cert_votes,
            );
        }
        PbftSyncRuntimeFinalChainHashStatus::Invalid => {
            return runtime_plan(
                PbftSyncProcessRuntimeAction::ClearAndReportPeer,
                PbftSyncPeriodAdmissionStatus::FinalChainHashInvalid,
                PbftSyncProcessRuntimeNextCheck::None,
                transaction_query,
                replace_previous_block_cert_votes,
            );
        }
        PbftSyncRuntimeFinalChainHashStatus::Unknown => {
            return runtime_contract_error(transaction_query);
        }
    }

    let staged_fact_status = |status, next_check, invalid_status, transaction_query| match status {
        PbftSyncFactStatus::NotChecked => Some(runtime_plan(
            PbftSyncProcessRuntimeAction::RunCheck,
            PbftSyncPeriodAdmissionStatus::Accepted,
            next_check,
            transaction_query,
            replace_previous_block_cert_votes,
        )),
        PbftSyncFactStatus::Invalid => Some(runtime_plan(
            PbftSyncProcessRuntimeAction::ClearAndReportPeer,
            invalid_status,
            PbftSyncProcessRuntimeNextCheck::None,
            transaction_query,
            replace_previous_block_cert_votes,
        )),
        PbftSyncFactStatus::Unknown => Some(runtime_contract_error(transaction_query)),
        PbftSyncFactStatus::Valid | PbftSyncFactStatus::NotRequired => None,
    };

    if let Some(plan) = staged_fact_status(
        fact.reward_votes_status,
        PbftSyncProcessRuntimeNextCheck::CheckRewardVotes,
        PbftSyncPeriodAdmissionStatus::RewardVotesInvalid,
        transaction_query.clone(),
    ) {
        return plan;
    }
    if let Some(plan) = staged_fact_status(
        fact.cert_votes_status,
        PbftSyncProcessRuntimeNextCheck::ValidateCertVotes,
        PbftSyncPeriodAdmissionStatus::CertVotesInvalid,
        transaction_query.clone(),
    ) {
        return plan;
    }
    if let Some(plan) = staged_fact_status(
        fact.transactions_status,
        PbftSyncProcessRuntimeNextCheck::CheckTransactions,
        PbftSyncPeriodAdmissionStatus::Accepted,
        transaction_query.clone(),
    ) {
        return plan;
    }
    match pillar_data_status {
        PbftSyncFactStatus::Invalid => {
            return runtime_plan(
                PbftSyncProcessRuntimeAction::ClearAndReportPeer,
                PbftSyncPeriodAdmissionStatus::PillarDataInvalid,
                PbftSyncProcessRuntimeNextCheck::None,
                transaction_query.clone(),
                replace_previous_block_cert_votes,
            );
        }
        PbftSyncFactStatus::Unknown => return runtime_contract_error(transaction_query),
        PbftSyncFactStatus::Valid
        | PbftSyncFactStatus::NotRequired
        | PbftSyncFactStatus::NotChecked => {}
    }

    let pillar_votes_status = if fact.pillar_votes_required {
        fact.pillar_votes_status
    } else {
        PbftSyncFactStatus::NotRequired
    };
    if let Some(plan) = staged_fact_status(
        pillar_votes_status,
        PbftSyncProcessRuntimeNextCheck::ValidatePillarVotes,
        PbftSyncPeriodAdmissionStatus::PillarVotesInvalid,
        transaction_query.clone(),
    ) {
        return plan;
    }

    let mut accepted = runtime_plan(
        PbftSyncProcessRuntimeAction::Accept,
        PbftSyncPeriodAdmissionStatus::Accepted,
        PbftSyncProcessRuntimeNextCheck::None,
        transaction_query,
        replace_previous_block_cert_votes,
    );
    accepted.warnings.extend(warn_transactions(
        fact.missing_transaction_hashes,
        PbftSyncTransactionWarningKind::MissingTransaction,
    ));
    accepted.warnings.extend(warn_transactions(
        fact.finalized_transaction_hashes,
        PbftSyncTransactionWarningKind::FinalizedTransaction,
    ));
    accepted.contains_finalized_transaction_warning = fact.contains_finalized_transactions;
    accepted
}

/// Stable state of one Rust-owned synced-period admission cursor.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftSyncAdmissionSessionStatus {
    /// The cursor is waiting for the named external check.
    Active,
    /// The candidate may be materialized and accepted.
    Accepted,
    /// The candidate was dropped without peer punishment.
    Dropped,
    /// The candidate was rejected and the peer/queue effects must run.
    FailedPeer,
    /// A report did not match the expected cursor/check contract.
    ContractError,
}

impl PbftSyncAdmissionSessionStatus {
    /// Stable bridge code.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::Accepted => 1,
            Self::Dropped => 2,
            Self::FailedPeer => 3,
            Self::ContractError => 4,
        }
    }
}

/// Immutable facts captured when one synced period-data candidate is popped.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncAdmissionInitialFact {
    pub block_period: u64,
    pub block_prev_hash: H256,
    pub chain_last_hash: H256,
    pub chain_last_period: u64,
    pub block_in_chain: bool,
    pub dag_transaction_hashes: Vec<H256>,
    pub period_data_transaction_hashes: Vec<H256>,
    pub extra_data_required: bool,
    pub extra_data_present: bool,
    pub extra_data_pillar_block_hash_present: bool,
    pub pillar_votes_required: bool,
    pub pillar_votes_present: bool,
    pub previous_cert_votes_present: bool,
    pub previous_cert_first_vote_has_weight: bool,
}

/// Cursor-checked report for the transaction-manager admission check.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncAdmissionTransactionReport {
    pub missing_transaction_hashes: Vec<H256>,
    pub finalized_transaction_hashes: Vec<H256>,
    pub contains_finalized_transactions: bool,
}

/// One cursor step with the complete terminal admission plan when finished.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncAdmissionSessionStep {
    pub status: PbftSyncAdmissionSessionStatus,
    pub cursor: u32,
    pub has_check: bool,
    pub next_check: PbftSyncProcessRuntimeNextCheck,
    pub plan: PbftSyncProcessPeriodDataRuntimePlan,
    pub complete: bool,
    pub can_continue: bool,
    pub error_code: String,
}

/// Stateful wrapper around the synced-period decision table.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftSyncAdmissionSession {
    fact: PbftSyncProcessPeriodDataRuntimeFact,
    cursor: u32,
    contract_error: Option<String>,
}

/// Creates an admission cursor with every external validation fact unchecked.
pub fn create_pbft_sync_admission_session(
    initial: PbftSyncAdmissionInitialFact,
) -> PbftSyncAdmissionSession {
    PbftSyncAdmissionSession {
        fact: PbftSyncProcessPeriodDataRuntimeFact {
            block_period: initial.block_period,
            block_prev_hash: initial.block_prev_hash,
            chain_last_hash: initial.chain_last_hash,
            chain_last_period: initial.chain_last_period,
            block_in_chain: initial.block_in_chain,
            final_chain_hash_status: PbftSyncRuntimeFinalChainHashStatus::NotChecked,
            reward_votes_status: PbftSyncFactStatus::NotChecked,
            cert_votes_status: PbftSyncFactStatus::NotChecked,
            transactions_status: PbftSyncFactStatus::NotChecked,
            dag_transaction_hashes: initial.dag_transaction_hashes,
            period_data_transaction_hashes: initial.period_data_transaction_hashes,
            missing_transaction_hashes: Vec::new(),
            finalized_transaction_hashes: Vec::new(),
            contains_finalized_transactions: false,
            pillar_data_status: PbftSyncFactStatus::NotChecked,
            extra_data_required: initial.extra_data_required,
            extra_data_present: initial.extra_data_present,
            extra_data_pillar_block_hash_present: initial.extra_data_pillar_block_hash_present,
            pillar_votes_required: initial.pillar_votes_required,
            pillar_votes_present: initial.pillar_votes_present,
            pillar_votes_status: if initial.pillar_votes_required {
                PbftSyncFactStatus::NotChecked
            } else {
                PbftSyncFactStatus::NotRequired
            },
            previous_cert_votes_present: initial.previous_cert_votes_present,
            previous_cert_first_vote_has_weight: initial.previous_cert_first_vote_has_weight,
        },
        cursor: 0,
        contract_error: None,
    }
}

fn sync_admission_step(session: &PbftSyncAdmissionSession) -> PbftSyncAdmissionSessionStep {
    if let Some(error_code) = &session.contract_error {
        let transaction_query = plan_pbft_sync_transaction_query(PbftSyncTransactionQueryFact {
            dag_transaction_hashes: session.fact.dag_transaction_hashes.clone(),
            period_data_transaction_hashes: session.fact.period_data_transaction_hashes.clone(),
        });
        return PbftSyncAdmissionSessionStep {
            status: PbftSyncAdmissionSessionStatus::ContractError,
            cursor: session.cursor,
            has_check: false,
            next_check: PbftSyncProcessRuntimeNextCheck::None,
            plan: runtime_contract_error(transaction_query),
            complete: true,
            can_continue: false,
            error_code: error_code.clone(),
        };
    }
    let plan = plan_pbft_sync_process_period_data_runtime(session.fact.clone());
    let (status, complete, can_continue) = match plan.runtime_action {
        PbftSyncProcessRuntimeAction::RunCheck => {
            (PbftSyncAdmissionSessionStatus::Active, false, true)
        }
        PbftSyncProcessRuntimeAction::Accept => {
            (PbftSyncAdmissionSessionStatus::Accepted, true, true)
        }
        PbftSyncProcessRuntimeAction::Drop => (PbftSyncAdmissionSessionStatus::Dropped, true, true),
        PbftSyncProcessRuntimeAction::WaitForFinalization => {
            (PbftSyncAdmissionSessionStatus::Active, false, true)
        }
        PbftSyncProcessRuntimeAction::ClearAndReportPeer => {
            (PbftSyncAdmissionSessionStatus::FailedPeer, true, true)
        }
        PbftSyncProcessRuntimeAction::ContractError => {
            (PbftSyncAdmissionSessionStatus::ContractError, true, false)
        }
    };
    PbftSyncAdmissionSessionStep {
        status,
        cursor: session.cursor,
        has_check: matches!(
            plan.runtime_action,
            PbftSyncProcessRuntimeAction::RunCheck
                | PbftSyncProcessRuntimeAction::WaitForFinalization
        ),
        next_check: if plan.runtime_action == PbftSyncProcessRuntimeAction::WaitForFinalization {
            PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash
        } else {
            plan.next_check
        },
        complete,
        can_continue,
        error_code: if plan.runtime_action == PbftSyncProcessRuntimeAction::ContractError {
            "PBFT_SYNC_ADMISSION_INVALID_BRIDGE_FACTS".to_string()
        } else {
            String::new()
        },
        plan,
    }
}

/// Returns the current admission check or terminal decision without advancing.
pub fn next_pbft_sync_admission_session(
    session: &PbftSyncAdmissionSession,
) -> PbftSyncAdmissionSessionStep {
    sync_admission_step(session)
}

/// Returns the exact pillar-vote request currently pending on an admission cursor.
///
/// The pair is `(cursor, required_votes_period)`. Non-pillar stages and
/// terminal sessions return `None`; callers may use the pair only as a bounded
/// identity and must revalidate it before reporting after external work.
pub(crate) fn pbft_sync_admission_pillar_request(
    session: &PbftSyncAdmissionSession,
) -> Option<(u32, u64)> {
    let step = sync_admission_step(session);
    (step.has_check && step.next_check == PbftSyncProcessRuntimeNextCheck::ValidatePillarVotes)
        .then_some((step.cursor, session.fact.block_period))
}

fn validate_sync_admission_report(
    session: &mut PbftSyncAdmissionSession,
    cursor: u32,
    check: PbftSyncProcessRuntimeNextCheck,
) -> bool {
    let step = sync_admission_step(session);
    if !step.has_check || step.cursor != cursor || step.next_check != check {
        session.contract_error = Some("PBFT_SYNC_ADMISSION_REPORT_MISMATCH".to_string());
        return false;
    }
    true
}

/// Reports one non-transaction external validation status.
pub fn report_pbft_sync_admission_status(
    session: &mut PbftSyncAdmissionSession,
    cursor: u32,
    check: PbftSyncProcessRuntimeNextCheck,
    final_chain_status: PbftSyncRuntimeFinalChainHashStatus,
    fact_status: PbftSyncFactStatus,
) -> PbftSyncAdmissionSessionStep {
    if !validate_sync_admission_report(session, cursor, check) {
        return sync_admission_step(session);
    }
    match check {
        PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash => {
            session.fact.final_chain_hash_status = final_chain_status;
        }
        PbftSyncProcessRuntimeNextCheck::CheckRewardVotes => {
            session.fact.reward_votes_status = fact_status;
        }
        PbftSyncProcessRuntimeNextCheck::ValidateCertVotes => {
            session.fact.cert_votes_status = fact_status;
        }
        PbftSyncProcessRuntimeNextCheck::ValidatePillarVotes => {
            session.fact.pillar_votes_status = fact_status;
        }
        _ => {
            session.contract_error =
                Some("PBFT_SYNC_ADMISSION_STATUS_REPORT_WRONG_CHECK".to_string());
            return sync_admission_step(session);
        }
    }
    session.cursor = session.cursor.saturating_add(1);
    sync_admission_step(session)
}

/// Reports transaction-manager lookup results for the requested candidate.
pub fn report_pbft_sync_admission_transactions(
    session: &mut PbftSyncAdmissionSession,
    cursor: u32,
    report: PbftSyncAdmissionTransactionReport,
) -> PbftSyncAdmissionSessionStep {
    if !validate_sync_admission_report(
        session,
        cursor,
        PbftSyncProcessRuntimeNextCheck::CheckTransactions,
    ) {
        return sync_admission_step(session);
    }
    session.fact.transactions_status = PbftSyncFactStatus::Valid;
    session.fact.missing_transaction_hashes = report.missing_transaction_hashes;
    session.fact.finalized_transaction_hashes = report.finalized_transaction_hashes;
    session.fact.contains_finalized_transactions = report.contains_finalized_transactions;
    session.cursor = session.cursor.saturating_add(1);
    sync_admission_step(session)
}

/// Aborts an admission cursor after an external executor exception.
pub fn abort_pbft_sync_admission_session(
    session: &mut PbftSyncAdmissionSession,
) -> PbftSyncAdmissionSessionStep {
    session.contract_error = Some("PBFT_SYNC_ADMISSION_SESSION_ABORTED".to_string());
    sync_admission_step(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: u64) -> H256 {
        H256::from_low_u64_be(value)
    }

    fn admission_initial(pillar_votes_required: bool) -> PbftSyncAdmissionInitialFact {
        PbftSyncAdmissionInitialFact {
            block_period: 10,
            block_prev_hash: hash(9),
            chain_last_hash: hash(9),
            chain_last_period: 9,
            block_in_chain: false,
            dag_transaction_hashes: vec![hash(1)],
            period_data_transaction_hashes: vec![hash(1)],
            extra_data_required: pillar_votes_required,
            extra_data_present: pillar_votes_required,
            extra_data_pillar_block_hash_present: pillar_votes_required,
            pillar_votes_required,
            pillar_votes_present: pillar_votes_required,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        }
    }

    #[test]
    fn admission_session_owns_full_accepted_check_order() {
        let mut session = create_pbft_sync_admission_session(admission_initial(true));
        let expected = [
            PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash,
            PbftSyncProcessRuntimeNextCheck::CheckRewardVotes,
            PbftSyncProcessRuntimeNextCheck::ValidateCertVotes,
            PbftSyncProcessRuntimeNextCheck::CheckTransactions,
            PbftSyncProcessRuntimeNextCheck::ValidatePillarVotes,
        ];

        for check in expected {
            let step = next_pbft_sync_admission_session(&session);
            assert_eq!(step.status, PbftSyncAdmissionSessionStatus::Active);
            assert_eq!(step.next_check, check);
            if check == PbftSyncProcessRuntimeNextCheck::CheckTransactions {
                report_pbft_sync_admission_transactions(
                    &mut session,
                    step.cursor,
                    PbftSyncAdmissionTransactionReport {
                        missing_transaction_hashes: Vec::new(),
                        finalized_transaction_hashes: Vec::new(),
                        contains_finalized_transactions: false,
                    },
                );
            } else {
                report_pbft_sync_admission_status(
                    &mut session,
                    step.cursor,
                    check,
                    PbftSyncRuntimeFinalChainHashStatus::Valid,
                    PbftSyncFactStatus::Valid,
                );
            }
        }

        let accepted = next_pbft_sync_admission_session(&session);
        assert_eq!(accepted.status, PbftSyncAdmissionSessionStatus::Accepted);
        assert!(accepted.complete);
        assert!(accepted.plan.accept_period_data);
        assert!(accepted.plan.replace_previous_block_cert_votes);
    }

    #[test]
    fn admission_session_omits_optional_pillar_check_and_rejects_wrong_report() {
        let mut session = create_pbft_sync_admission_session(admission_initial(false));
        let first = next_pbft_sync_admission_session(&session);
        let failed = report_pbft_sync_admission_status(
            &mut session,
            first.cursor,
            PbftSyncProcessRuntimeNextCheck::ValidateCertVotes,
            PbftSyncRuntimeFinalChainHashStatus::Valid,
            PbftSyncFactStatus::Valid,
        );
        assert_eq!(failed.status, PbftSyncAdmissionSessionStatus::ContractError);
        assert!(!failed.can_continue);
        assert_eq!(failed.error_code, "PBFT_SYNC_ADMISSION_REPORT_MISMATCH");
    }

    #[test]
    fn admission_session_terminalizes_peer_failure_and_abort() {
        let mut session = create_pbft_sync_admission_session(admission_initial(false));
        let first = next_pbft_sync_admission_session(&session);
        let rejected = report_pbft_sync_admission_status(
            &mut session,
            first.cursor,
            first.next_check,
            PbftSyncRuntimeFinalChainHashStatus::Invalid,
            PbftSyncFactStatus::Invalid,
        );
        assert_eq!(rejected.status, PbftSyncAdmissionSessionStatus::FailedPeer);
        assert!(rejected.plan.clear_sync_queue);
        assert!(rejected.plan.report_malicious_peer);

        let mut aborted = create_pbft_sync_admission_session(admission_initial(false));
        let step = abort_pbft_sync_admission_session(&mut aborted);
        assert_eq!(step.status, PbftSyncAdmissionSessionStatus::ContractError);
        assert_eq!(step.error_code, "PBFT_SYNC_ADMISSION_SESSION_ABORTED");
    }

    #[test]
    fn admission_session_waits_then_rechecks_final_chain_for_same_candidate() {
        let mut session = create_pbft_sync_admission_session(admission_initial(false));
        let first = next_pbft_sync_admission_session(&session);
        let retry = report_pbft_sync_admission_status(
            &mut session,
            first.cursor,
            first.next_check,
            PbftSyncRuntimeFinalChainHashStatus::Missing,
            PbftSyncFactStatus::NotChecked,
        );
        assert_eq!(retry.status, PbftSyncAdmissionSessionStatus::Active);
        assert!(!retry.complete);
        assert!(retry.has_check);
        assert!(retry.plan.wait_for_finalization);
        assert_eq!(
            retry.next_check,
            PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash
        );

        let reward = report_pbft_sync_admission_status(
            &mut session,
            retry.cursor,
            retry.next_check,
            PbftSyncRuntimeFinalChainHashStatus::Valid,
            PbftSyncFactStatus::Valid,
        );
        assert_eq!(
            reward.next_check,
            PbftSyncProcessRuntimeNextCheck::CheckRewardVotes
        );
    }

    fn fact() -> PbftSyncPeriodAdmissionFact {
        PbftSyncPeriodAdmissionFact {
            block_period: 101,
            block_prev_hash: hash(10_000),
            chain_last_hash: hash(10_000),
            chain_last_period: 100,
            block_in_chain: false,
            final_chain_hash_status: PbftSyncFinalChainHashStatus::Valid,
            reward_votes_status: PbftSyncFactStatus::Valid,
            cert_votes_status: PbftSyncFactStatus::Valid,
            missing_transaction_hashes: vec![],
            finalized_transaction_hashes: vec![],
            contains_finalized_transactions: false,
            pillar_data_status: PbftSyncFactStatus::Valid,
            pillar_votes_status: PbftSyncFactStatus::NotRequired,
        }
    }

    fn runtime_fact() -> PbftSyncProcessPeriodDataRuntimeFact {
        PbftSyncProcessPeriodDataRuntimeFact {
            block_period: 101,
            block_prev_hash: hash(10_000),
            chain_last_hash: hash(10_000),
            chain_last_period: 100,
            block_in_chain: false,
            final_chain_hash_status: PbftSyncRuntimeFinalChainHashStatus::NotChecked,
            reward_votes_status: PbftSyncFactStatus::NotChecked,
            cert_votes_status: PbftSyncFactStatus::NotChecked,
            transactions_status: PbftSyncFactStatus::NotChecked,
            dag_transaction_hashes: vec![hash(1), hash(2), hash(1)],
            period_data_transaction_hashes: vec![hash(2)],
            missing_transaction_hashes: vec![],
            finalized_transaction_hashes: vec![],
            contains_finalized_transactions: false,
            pillar_data_status: PbftSyncFactStatus::NotChecked,
            extra_data_required: true,
            extra_data_present: true,
            extra_data_pillar_block_hash_present: true,
            pillar_votes_required: true,
            pillar_votes_present: true,
            pillar_votes_status: PbftSyncFactStatus::NotChecked,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        }
    }

    fn cert_vote(weight: u64) -> PbftSyncCertVoteFact {
        PbftSyncCertVoteFact {
            vote_hash: hash(weight),
            block_hash: hash(9),
            period: 101,
            round: 2,
            step: 3,
            vote_type: 3,
            live_vote_valid: true,
            weight_present: true,
            weight,
        }
    }

    fn cert_vote_bundle(weights: Vec<u64>, threshold: u64) -> PbftSyncCertVoteBundleFact {
        PbftSyncCertVoteBundleFact {
            block_period: 101,
            block_hash: hash(9),
            votes: weights.into_iter().map(cert_vote).collect(),
            check_weight_threshold: true,
            two_t_plus_one_found: true,
            two_t_plus_one: threshold,
        }
    }

    #[test]
    fn cert_vote_bundle_accepts_shape_and_threshold() {
        let result = validate_pbft_sync_cert_vote_bundle(cert_vote_bundle(vec![2, 3], 5));

        assert!(result.valid);
        assert_eq!(result.status, PbftSyncCertVoteBundleStatus::Accepted);
        assert_eq!(result.total_weight, 5);
        assert_eq!(result.two_t_plus_one, 5);
    }

    #[test]
    fn cert_vote_bundle_rejects_bad_shape_and_threshold() {
        let mut fact = cert_vote_bundle(vec![2, 2], 5);
        fact.votes[1].block_hash = hash(10);
        let result = validate_pbft_sync_cert_vote_bundle(fact);
        assert!(!result.valid);
        assert_eq!(
            result.status,
            PbftSyncCertVoteBundleStatus::BlockHashMismatch
        );
        assert_eq!(result.first_bad_vote_hash, hash(2));

        let result = validate_pbft_sync_cert_vote_bundle(cert_vote_bundle(vec![2, 2], 5));
        assert!(!result.valid);
        assert_eq!(
            result.status,
            PbftSyncCertVoteBundleStatus::InsufficientWeight
        );
        assert_eq!(result.total_weight, 4);
    }

    #[test]
    fn accepts_and_carries_transaction_fact_warnings() {
        let mut f = fact();
        f.missing_transaction_hashes = vec![hash(1), hash(2)];
        f.finalized_transaction_hashes = vec![hash(3)];
        f.contains_finalized_transactions = true;

        let plan = plan_pbft_sync_period_admission(f);
        assert!(plan.is_accepted());
        assert_eq!(plan.decision, PbftSyncPeriodAdmissionDecision::Accept);
        assert_eq!(plan.status, PbftSyncPeriodAdmissionStatus::Accepted);
        assert_eq!(plan.warnings.len(), 3);
        assert_eq!(
            plan.warnings[0].kind,
            PbftSyncTransactionWarningKind::MissingTransaction
        );
        assert_eq!(plan.warnings[0].hash, hash(1));
        assert_eq!(
            plan.warnings[2].kind,
            PbftSyncTransactionWarningKind::FinalizedTransaction
        );
        assert_eq!(plan.warnings[2].hash, hash(3));
        assert!(plan.contains_finalized_transaction_warning);
    }

    #[test]
    fn runtime_plan_wraps_base_plan_without_behavior_change() {
        let runtime_plan = plan_pbft_sync_period_admission_runtime(fact());
        assert_eq!(runtime_plan.action, PbftSyncAdmissionRuntimeAction::Accept);
        assert!(runtime_plan.is_accepted());
        assert!(!runtime_plan.clear_sync_queue());
        assert!(!runtime_plan.report_malicious_peer());
        assert!(!runtime_plan.wait_for_finalization());
        assert!(runtime_plan.accept_period_data());
    }

    #[test]
    fn runtime_plan_wraps_transaction_query_and_admission() {
        let runtime_plan = plan_pbft_sync_runtime(
            fact(),
            PbftSyncTransactionQueryFact {
                dag_transaction_hashes: vec![
                    H256::from_low_u64_be(1),
                    H256::from_low_u64_be(2),
                    H256::from_low_u64_be(1),
                ],
                period_data_transaction_hashes: vec![H256::from_low_u64_be(2)],
            },
        );

        assert_eq!(
            runtime_plan.period_admission.action,
            PbftSyncAdmissionRuntimeAction::Accept
        );
        assert!(runtime_plan.is_accepted());
        assert_eq!(
            runtime_plan.transaction_query.finalized_lookup_hashes,
            vec![H256::from_low_u64_be(1)]
        );
        assert!(runtime_plan.requires_transaction_lookup());
    }

    #[test]
    fn process_period_runtime_requests_checks_in_legacy_order() {
        let plan = plan_pbft_sync_process_period_data_runtime(runtime_fact());
        assert_eq!(plan.runtime_action, PbftSyncProcessRuntimeAction::RunCheck);
        assert_eq!(
            plan.next_check,
            PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash
        );
        assert!(plan.replace_previous_block_cert_votes);

        let mut f = runtime_fact();
        f.final_chain_hash_status = PbftSyncRuntimeFinalChainHashStatus::Valid;
        let plan = plan_pbft_sync_process_period_data_runtime(f);
        assert_eq!(
            plan.next_check,
            PbftSyncProcessRuntimeNextCheck::CheckRewardVotes
        );

        let mut f = runtime_fact();
        f.final_chain_hash_status = PbftSyncRuntimeFinalChainHashStatus::Valid;
        f.reward_votes_status = PbftSyncFactStatus::Valid;
        let plan = plan_pbft_sync_process_period_data_runtime(f);
        assert_eq!(
            plan.next_check,
            PbftSyncProcessRuntimeNextCheck::ValidateCertVotes
        );

        let mut f = runtime_fact();
        f.final_chain_hash_status = PbftSyncRuntimeFinalChainHashStatus::Valid;
        f.reward_votes_status = PbftSyncFactStatus::Valid;
        f.cert_votes_status = PbftSyncFactStatus::Valid;
        let plan = plan_pbft_sync_process_period_data_runtime(f);
        assert_eq!(
            plan.next_check,
            PbftSyncProcessRuntimeNextCheck::CheckTransactions
        );
        assert_eq!(
            plan.transaction_query.finalized_lookup_hashes,
            vec![hash(1)]
        );
    }

    #[test]
    fn process_period_runtime_waits_rejects_and_accepts_with_warnings() {
        let mut f = runtime_fact();
        f.final_chain_hash_status = PbftSyncRuntimeFinalChainHashStatus::Missing;
        let plan = plan_pbft_sync_process_period_data_runtime(f);
        assert_eq!(
            plan.runtime_action,
            PbftSyncProcessRuntimeAction::WaitForFinalization
        );
        assert!(plan.retry_same_candidate);

        let mut f = runtime_fact();
        f.block_prev_hash = hash(9);
        let plan = plan_pbft_sync_process_period_data_runtime(f);
        assert_eq!(
            plan.runtime_action,
            PbftSyncProcessRuntimeAction::ClearAndReportPeer
        );
        assert_eq!(
            plan.status,
            PbftSyncPeriodAdmissionStatus::PreviousHashMismatch
        );

        let mut f = runtime_fact();
        f.final_chain_hash_status = PbftSyncRuntimeFinalChainHashStatus::Valid;
        f.reward_votes_status = PbftSyncFactStatus::Valid;
        f.cert_votes_status = PbftSyncFactStatus::Valid;
        f.transactions_status = PbftSyncFactStatus::Valid;
        f.missing_transaction_hashes = vec![hash(1)];
        f.contains_finalized_transactions = true;
        f.pillar_data_status = PbftSyncFactStatus::Valid;
        f.pillar_votes_status = PbftSyncFactStatus::Valid;
        let plan = plan_pbft_sync_process_period_data_runtime(f);
        assert_eq!(plan.runtime_action, PbftSyncProcessRuntimeAction::Accept);
        assert!(plan.accept_period_data);
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.contains_finalized_transaction_warning);
    }

    #[test]
    fn process_period_runtime_derives_pillar_data_from_queue_metadata() {
        let mut f = runtime_fact();
        f.final_chain_hash_status = PbftSyncRuntimeFinalChainHashStatus::Valid;
        f.reward_votes_status = PbftSyncFactStatus::Valid;
        f.cert_votes_status = PbftSyncFactStatus::Valid;
        f.transactions_status = PbftSyncFactStatus::Valid;
        f.pillar_votes_status = PbftSyncFactStatus::NotRequired;
        f.extra_data_pillar_block_hash_present = false;
        let plan = plan_pbft_sync_process_period_data_runtime(f);
        assert_eq!(
            plan.status,
            PbftSyncPeriodAdmissionStatus::PillarDataInvalid
        );
        assert_eq!(
            plan.runtime_action,
            PbftSyncProcessRuntimeAction::ClearAndReportPeer
        );

        let mut f = runtime_fact();
        f.final_chain_hash_status = PbftSyncRuntimeFinalChainHashStatus::Valid;
        f.reward_votes_status = PbftSyncFactStatus::Valid;
        f.cert_votes_status = PbftSyncFactStatus::Valid;
        f.transactions_status = PbftSyncFactStatus::Valid;
        f.extra_data_required = false;
        f.extra_data_present = true;
        f.pillar_votes_required = false;
        f.pillar_votes_present = false;
        f.pillar_votes_status = PbftSyncFactStatus::NotRequired;
        let plan = plan_pbft_sync_process_period_data_runtime(f);
        assert_eq!(
            plan.status,
            PbftSyncPeriodAdmissionStatus::PillarDataInvalid
        );
    }

    #[test]
    fn transaction_query_plans_unique_missing_dag_transactions_in_order() {
        let plan = plan_pbft_sync_transaction_query(PbftSyncTransactionQueryFact {
            dag_transaction_hashes: vec![
                H256::from_low_u64_be(1),
                H256::from_low_u64_be(2),
                H256::from_low_u64_be(1),
                H256::from_low_u64_be(3),
                H256::from_low_u64_be(4),
            ],
            period_data_transaction_hashes: vec![
                H256::from_low_u64_be(2),
                H256::from_low_u64_be(4),
            ],
        });

        assert_eq!(
            plan.finalized_lookup_hashes,
            vec![H256::from_low_u64_be(1), H256::from_low_u64_be(3)]
        );
    }

    #[test]
    fn queue_drain_session_orders_clean_pop_push_update_and_stop() {
        let mut session = create_pbft_sync_queue_drain_session();

        let clean = next_pbft_sync_queue_drain_step(&mut session, 2, 10);
        assert_eq!(clean.action, PbftSyncQueueDrainAction::CleanOldData);
        assert_eq!(clean.clean_before_period, 10);
        assert!(
            report_pbft_sync_queue_drain_step(
                &mut session,
                PbftSyncQueueDrainReport {
                    action: PbftSyncQueueDrainAction::CleanOldData,
                    success: true,
                    accepted_period_data: false,
                }
            )
            .can_continue
        );

        let pop = next_pbft_sync_queue_drain_step(&mut session, 1, 10);
        assert_eq!(pop.action, PbftSyncQueueDrainAction::PopAndProcess);
        assert!(
            report_pbft_sync_queue_drain_step(
                &mut session,
                PbftSyncQueueDrainReport {
                    action: PbftSyncQueueDrainAction::PopAndProcess,
                    success: true,
                    accepted_period_data: true,
                }
            )
            .can_continue
        );

        let push = next_pbft_sync_queue_drain_step(&mut session, 1, 10);
        assert_eq!(push.action, PbftSyncQueueDrainAction::PushAccepted);
        assert!(
            report_pbft_sync_queue_drain_step(
                &mut session,
                PbftSyncQueueDrainReport {
                    action: PbftSyncQueueDrainAction::PushAccepted,
                    success: true,
                    accepted_period_data: false,
                }
            )
            .can_continue
        );

        let update = next_pbft_sync_queue_drain_step(&mut session, 1, 11);
        assert_eq!(update.action, PbftSyncQueueDrainAction::UpdateSyncState);
        assert!(
            report_pbft_sync_queue_drain_step(
                &mut session,
                PbftSyncQueueDrainReport {
                    action: PbftSyncQueueDrainAction::UpdateSyncState,
                    success: true,
                    accepted_period_data: false,
                }
            )
            .can_continue
        );

        let stop = next_pbft_sync_queue_drain_step(&mut session, 0, 11);
        assert_eq!(stop.action, PbftSyncQueueDrainAction::Stop);
        assert_eq!(stop.status, PbftSyncQueueDrainStatus::Complete);
        assert!(!stop.can_continue);
    }

    #[test]
    fn queue_drain_session_continues_after_dropped_candidate() {
        let mut session = create_pbft_sync_queue_drain_session();
        let _ = next_pbft_sync_queue_drain_step(&mut session, 2, 10);
        let _ = report_pbft_sync_queue_drain_step(
            &mut session,
            PbftSyncQueueDrainReport {
                action: PbftSyncQueueDrainAction::CleanOldData,
                success: true,
                accepted_period_data: false,
            },
        );

        let pop = next_pbft_sync_queue_drain_step(&mut session, 2, 10);
        assert_eq!(pop.action, PbftSyncQueueDrainAction::PopAndProcess);
        let result = report_pbft_sync_queue_drain_step(
            &mut session,
            PbftSyncQueueDrainReport {
                action: PbftSyncQueueDrainAction::PopAndProcess,
                success: true,
                accepted_period_data: false,
            },
        );
        assert_eq!(result.status, PbftSyncQueueDrainStatus::Active);

        let next_pop = next_pbft_sync_queue_drain_step(&mut session, 1, 10);
        assert_eq!(next_pop.action, PbftSyncQueueDrainAction::PopAndProcess);
    }

    #[test]
    fn queue_drain_session_stops_on_push_failure_and_invalid_reports() {
        let mut session = create_pbft_sync_queue_drain_session();
        let _ = next_pbft_sync_queue_drain_step(&mut session, 1, 10);
        let _ = report_pbft_sync_queue_drain_step(
            &mut session,
            PbftSyncQueueDrainReport {
                action: PbftSyncQueueDrainAction::CleanOldData,
                success: true,
                accepted_period_data: false,
            },
        );
        let _ = next_pbft_sync_queue_drain_step(&mut session, 1, 10);
        let _ = report_pbft_sync_queue_drain_step(
            &mut session,
            PbftSyncQueueDrainReport {
                action: PbftSyncQueueDrainAction::PopAndProcess,
                success: true,
                accepted_period_data: true,
            },
        );
        let _ = next_pbft_sync_queue_drain_step(&mut session, 1, 10);
        let failed = report_pbft_sync_queue_drain_step(
            &mut session,
            PbftSyncQueueDrainReport {
                action: PbftSyncQueueDrainAction::PushAccepted,
                success: false,
                accepted_period_data: false,
            },
        );
        assert_eq!(failed.status, PbftSyncQueueDrainStatus::PushFailed);
        assert!(!failed.can_continue);

        let mut invalid = create_pbft_sync_queue_drain_session();
        let _ = next_pbft_sync_queue_drain_step(&mut invalid, 1, 10);
        let mismatch = report_pbft_sync_queue_drain_step(
            &mut invalid,
            PbftSyncQueueDrainReport {
                action: PbftSyncQueueDrainAction::PopAndProcess,
                success: true,
                accepted_period_data: false,
            },
        );
        assert_eq!(mismatch.status, PbftSyncQueueDrainStatus::InvalidReport);
    }

    #[test]
    fn drops_known_and_stale_blocks_without_peer_penalty() {
        let mut f = fact();
        f.block_in_chain = true;
        let plan = plan_pbft_sync_period_admission(f);
        assert_eq!(plan.decision, PbftSyncPeriodAdmissionDecision::Drop);
        assert_eq!(
            plan.status,
            PbftSyncPeriodAdmissionStatus::BlockAlreadyInChain
        );
        assert!(!plan.clear_sync_queue);
        assert!(!plan.report_malicious_peer);

        let mut f = fact();
        f.block_period = 100;
        f.block_prev_hash = hash(1);
        let plan = plan_pbft_sync_period_admission(f);
        assert_eq!(plan.decision, PbftSyncPeriodAdmissionDecision::Drop);
        assert_eq!(plan.status, PbftSyncPeriodAdmissionStatus::StalePeriod);
        assert!(!plan.clear_sync_queue);
        assert!(!plan.report_malicious_peer);
    }

    #[test]
    fn rejects_non_stale_previous_hash_mismatch() {
        let mut f = fact();
        f.block_prev_hash = hash(1);

        let plan = plan_pbft_sync_period_admission(f);
        assert_eq!(
            plan.decision,
            PbftSyncPeriodAdmissionDecision::ClearAndReportPeer
        );
        assert_eq!(
            plan.status,
            PbftSyncPeriodAdmissionStatus::PreviousHashMismatch
        );
        assert!(plan.clear_sync_queue);
        assert!(plan.report_malicious_peer);
    }

    #[test]
    fn waits_for_missing_final_chain_without_peer_penalty() {
        let mut f = fact();
        f.final_chain_hash_status = PbftSyncFinalChainHashStatus::Missing;

        let plan = plan_pbft_sync_period_admission(f);
        assert_eq!(
            plan.decision,
            PbftSyncPeriodAdmissionDecision::WaitForFinalization
        );
        assert_eq!(
            plan.status,
            PbftSyncPeriodAdmissionStatus::FinalChainHashMissing
        );
        assert!(plan.wait_for_finalization);
        assert!(!plan.clear_sync_queue);
        assert!(!plan.report_malicious_peer);
    }

    #[test]
    fn invalid_vote_and_pillar_facts_clear_and_report() {
        let mut f = fact();
        f.reward_votes_status = PbftSyncFactStatus::Invalid;
        let plan = plan_pbft_sync_period_admission(f);
        assert_eq!(
            plan.status,
            PbftSyncPeriodAdmissionStatus::RewardVotesInvalid
        );
        assert!(plan.clear_sync_queue);

        let mut f = fact();
        f.cert_votes_status = PbftSyncFactStatus::Invalid;
        assert_eq!(
            plan_pbft_sync_period_admission(f).status,
            PbftSyncPeriodAdmissionStatus::CertVotesInvalid
        );

        let mut f = fact();
        f.pillar_data_status = PbftSyncFactStatus::Invalid;
        assert_eq!(
            plan_pbft_sync_period_admission(f).status,
            PbftSyncPeriodAdmissionStatus::PillarDataInvalid
        );

        let mut f = fact();
        f.pillar_votes_status = PbftSyncFactStatus::Invalid;
        assert_eq!(
            plan_pbft_sync_period_admission(f).status,
            PbftSyncPeriodAdmissionStatus::PillarVotesInvalid
        );
    }

    #[test]
    fn not_checked_and_not_required_facts_do_not_reject() {
        let mut f = fact();
        f.reward_votes_status = PbftSyncFactStatus::NotChecked;
        f.cert_votes_status = PbftSyncFactStatus::NotChecked;
        f.pillar_data_status = PbftSyncFactStatus::NotChecked;
        f.pillar_votes_status = PbftSyncFactStatus::NotRequired;

        assert!(plan_pbft_sync_period_admission(f).is_accepted());
    }
}
