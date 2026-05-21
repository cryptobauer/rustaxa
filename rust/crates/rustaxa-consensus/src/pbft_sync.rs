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
