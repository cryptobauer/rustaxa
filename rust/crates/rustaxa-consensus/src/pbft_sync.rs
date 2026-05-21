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
    /// Run pillar-data validation.
    ValidatePillarData,
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
            Self::ValidatePillarData => 5,
            Self::ValidatePillarVotes => 6,
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
    /// Whether this period requires pillar-vote validation.
    pub pillar_votes_required: bool,
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
    if let Some(plan) = staged_fact_status(
        fact.pillar_data_status,
        PbftSyncProcessRuntimeNextCheck::ValidatePillarData,
        PbftSyncPeriodAdmissionStatus::PillarDataInvalid,
        transaction_query.clone(),
    ) {
        return plan;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: u64) -> H256 {
        H256::from_low_u64_be(value)
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
            pillar_votes_required: true,
            pillar_votes_status: PbftSyncFactStatus::NotChecked,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        }
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
