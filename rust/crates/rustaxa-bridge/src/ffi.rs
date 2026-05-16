use crate::dag::*;
use crate::final_chain::*;
use crate::gas_pricer::*;
use crate::pbft_chain::*;
use crate::period_data_queue::*;
use crate::pillar_votes::*;
use crate::proposed_blocks::*;
use crate::slashing::*;
use crate::sortition::*;
use crate::storage::*;
use crate::transaction_manager::*;
use crate::transaction_queue::*;
use crate::vdf::*;
use crate::verified_votes::*;
use ethereum_types::H256;
use rustaxa_consensus::dag::{DagGraph, DagManagerState};
use rustaxa_consensus::gas_pricer::GasPriceOracle;
use rustaxa_consensus::pbft_chain::PbftChain;
use rustaxa_consensus::period_data_queue::PeriodDataQueue;
use rustaxa_consensus::proposed_blocks::ProposedBlocks;
use rustaxa_consensus::slashing::SlashingProofPlanner;
use rustaxa_consensus::sortition::SortitionParamsManager;
use rustaxa_consensus::transaction_manager::{
    TransactionManagerSidecar, TransactionPackingPlanner,
};
use rustaxa_consensus::transaction_queue::{TransactionQueue, TransactionQueueEntry};
use rustaxa_consensus::verified_votes::VerifiedVotes;
use rustaxa_consensus::FinalChain;
use rustaxa_consensus::PillarVotes;
use rustaxa_storage::Storage;
use rustaxa_storage::StorageWriteBatch;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

pub struct BridgeStorage(
    pub Arc<Storage>,
    pub Mutex<HashMap<u64, StorageWriteBatch>>,
    pub AtomicU64,
);

pub struct BridgeFinalChain(pub FinalChain);

pub struct BridgeGasPricer(pub Mutex<GasPriceOracle>);

pub struct BridgeDagGraph(pub DagGraph);

/// Storage-free DagManager state wrapper used for in-memory DAG graph/index
/// logic only. Persistence is intentionally handled by `BridgeDagManagerRuntime`.
pub struct BridgeDagManagerState(pub DagManagerState);

/// DagManager runtime wrapper coupling deterministic in-memory state with the
/// shared Rust storage handle used for direct DAG persistence and reads.
pub struct BridgeDagManagerRuntime {
    pub state: DagManagerState,
    pub storage: Arc<Storage>,
}

pub struct BridgePbftChain(pub PbftChain);

pub struct BridgeProposedBlocks(pub ProposedBlocks);

pub struct BridgeSlashingProofPlanner(pub Mutex<SlashingProofPlanner>);

pub struct BridgePeriodDataQueue(pub PeriodDataQueue);

pub struct BridgeVerifiedVotes(pub VerifiedVotes);

pub struct BridgePillarVotes(pub PillarVotes);

pub struct BridgeSortitionParamsManager(pub SortitionParamsManager);

/// Bridge-owned transaction queue handle.
///
/// `queue` owns deterministic queue metadata, queued payload bytes, and the local known-transaction cache.
/// `last_drop_observed` tracks the Rust-mode equivalent of the legacy overflow/drop wall-clock window used by C++
/// callers to tell peers that this node recently rejected or evicted transactions.
pub struct BridgeTransactionQueue {
    pub queue: TransactionQueue,
    pub last_drop_observed: Option<Instant>,
}

/// Bridge-owned TransactionManager runtime handle for Rust-enabled manager paths.
///
/// The runtime combines the manager sidecar state with Rust queue state so the
/// C++ TransactionManager shim can route live admission, lookup, and finalization
/// queue effects through one Rust-owned authority while still materializing
/// legacy `Transaction` objects at the C++ API boundary.
pub struct BridgeTransactionManagerRuntime {
    pub sidecar: TransactionManagerSidecar,
    pub queue: TransactionQueue,
    pub last_drop_observed: Option<Instant>,
    pub transaction_pack_session: Option<TransactionManagerRuntimePackSession>,
}

/// Runtime admission execution script for `saveTransactionsFromDagBlock`.
///
/// `accepted_payloads` is the storage write-set that must persist before any
/// queue/sidecar mutation is committed into runtime live state.
pub struct BridgeTransactionManagerAdmissionExecution {
    pub accepted: Vec<rustaxa_ffi::DagTransactionSaveAccepted>,
    pub accepted_payloads: Vec<rustaxa_ffi::NonFinalizedTransactionPayload>,
    pub target_transaction_count: u64,
}

pub struct BridgeTransactionPackPlanner(pub TransactionPackingPlanner);

/// Runtime-owned state for one TransactionManager proposal-packing pass.
///
/// The session owns the ordered queue candidate snapshot, planner accounting,
/// selected output ordering, and demotion summary. C++ remains responsible only
/// for materializing the current candidate and supplying FinalChain/EVM gas
/// estimates back to the runtime.
pub struct TransactionManagerRuntimePackSession {
    pub planner: TransactionPackingPlanner,
    pub candidates: Vec<TransactionQueueEntry>,
    pub next_index: usize,
    pub current: Option<TransactionQueueEntry>,
    pub selected: Vec<(TransactionQueueEntry, u64)>,
    pub demoted_hashes: Vec<H256>,
    pub stopped: bool,
}

#[cxx::bridge(namespace = "rustaxa")]
pub mod rustaxa_ffi {
    struct BlockPeriod {
        period: u64,
        position: u32,
    }

    struct BlockPeriodLookup {
        found: bool,
        period: u64,
        position: u32,
    }

    struct BlockRlp {
        data: Vec<u8>,
    }

    /// Optional DAG block payload lookup result.
    struct DagBlockLookup {
        found: bool,
        block_rlp: Vec<u8>,
    }

    /// Persisted DAG block/edge counters loaded from storage status fields.
    struct DagPersistenceCounters {
        dag_blocks: u64,
        dag_edges: u64,
    }

    struct LevelBlocks {
        level: u64,
        blocks: Vec<BlockRlp>,
    }

    struct PeriodLookup {
        found: bool,
        period: u64,
    }

    struct PeriodLambda {
        found: bool,
        value: u32,
    }

    struct PeriodRlp {
        period: u64,
        data: Vec<u8>,
    }

    struct TxRlp {
        data: Vec<u8>,
    }

    /// TransactionQueue construction limits.
    struct TransactionQueueConfig {
        max_size: usize,
    }

    /// Queue erase result and metadata for C++ mirror mutation.
    struct TransactionQueueErasePlan {
        removed: bool,
        removed_hash: [u8; 32],
        removed_sender: [u8; 20],
        removed_nonce: [u8; 32],
        removed_gas_price: [u8; 32],
        removed_gas: u64,
        removed_data_size: usize,
        removed_last_block_number: u64,
        removed_proposable: bool,
    }

    /// Hash handle used to map Rust queue decisions back to C++ live transactions.
    struct TransactionQueueHash {
        hash: [u8; 32],
    }

    /// Demote outcome returned by Rust transaction queue metadata.
    struct TransactionQueueDemotePlan {
        status: u8,
        hash: [u8; 32],
        hash_found: bool,
        sender: [u8; 20],
        nonce: [u8; 32],
        gas_price: [u8; 32],
        gas: u64,
        data_size: usize,
        last_block_number: u64,
        proposable_before: bool,
    }

    /// Address handle used by C++ to query FinalChain account state for purge.
    struct TransactionQueueAddress {
        address: [u8; 20],
    }

    /// One finalized account nonce fact consumed by batch purge planning.
    struct TransactionQueueAccountNonceFact {
        sender: [u8; 20],
        account_found: bool,
        account_nonce: [u8; 32],
    }

    /// Proposable transaction hash group returned per sender.
    struct TransactionQueueHashGroup {
        hashes: Vec<TransactionQueueHash>,
    }

    /// C++-originated transaction queue metadata for one insert attempt.
    struct TransactionQueueInsertInput {
        hash: [u8; 32],
        sender: [u8; 20],
        nonce: [u8; 32],
        gas_price: [u8; 32],
        gas: u64,
        data_size: usize,
        tx_rlp: Vec<u8>,
        proposable: bool,
        last_block_number: u64,
    }

    /// Queued transaction payload retained by Rust and materialized by C++.
    struct TransactionQueueStoredTransaction {
        found: bool,
        hash: [u8; 32],
        tx_rlp: Vec<u8>,
    }

    /// Proposable queued transactions returned per sender.
    struct TransactionQueueTransactionGroup {
        transactions: Vec<TransactionQueueStoredTransaction>,
    }

    /// Rust queue insert decision and C++ mirror-update plan.
    struct TransactionQueueInsertOutcome {
        status: u8,
        inserted_hash_found: bool,
        inserted_hash: [u8; 32],
        demoted_hashes: Vec<TransactionQueueHash>,
        overflow_removed_hashes: Vec<TransactionQueueHash>,
    }

    /// Ordered hash read plan with completion metadata.
    struct TransactionQueueOrderedHashesPlan {
        hashes: Vec<TransactionQueueHash>,
        requested_count: u64,
        complete: bool,
    }

    /// Purge-style outcome with removed hashes and count.
    struct TransactionQueuePurgePlan {
        removed_hashes: Vec<TransactionQueueHash>,
        removed_count: usize,
    }

    /// TransactionManager runtime queue cleanup outcome.
    ///
    /// `non_proposable_expired` reports non-proposable entries expired by
    /// finalized block height. `finalized_account_purged` reports proposable
    /// entries removed from C++ supplied FinalChain account nonce facts.
    struct TransactionManagerRuntimeQueueCleanupPlan {
        non_proposable_expired: TransactionQueuePurgePlan,
        finalized_account_purged: TransactionQueuePurgePlan,
    }

    /// Candidate metadata supplied before C++ runs a gas estimate.
    struct TransactionPackCandidateInput {
        hash: [u8; 32],
        declared_gas: u64,
    }

    /// Decision telling C++ whether to estimate a candidate.
    struct TransactionPackCandidateDecision {
        should_estimate: bool,
    }

    /// C++ gas-estimation fact supplied after FinalChain/EVM estimation.
    struct TransactionPackEstimateInput {
        hash: [u8; 32],
        gas_used: u64,
    }

    /// One candidate returned by a Rust-owned runtime packing session.
    struct TransactionPackSessionCandidate {
        found: bool,
        hash: [u8; 32],
        declared_gas: u64,
        tx_rlp: Vec<u8>,
    }

    /// C++ gas-estimation fact supplied for the active runtime packing candidate.
    struct TransactionPackSessionEstimateInput {
        hash: [u8; 32],
        gas_used: u64,
        last_block_number: u64,
    }

    /// One transaction accepted by a Rust-owned runtime packing session.
    struct TransactionPackSelectedTransaction {
        hash: [u8; 32],
        gas_used: u64,
        tx_rlp: Vec<u8>,
    }

    /// Final selected transactions and queue mutation summary for one packing session.
    struct TransactionPackSessionOutcome {
        selected_transactions: Vec<TransactionPackSelectedTransaction>,
        demoted_hashes: Vec<TransactionQueueHash>,
        stopped: bool,
    }

    /// GasPricer construction limits and mode flags supplied by C++ genesis config.
    struct GasPricerConfig {
        percentile: u64,
        minimum_price: [u8; 32],
        history_blocks: usize,
        is_light_node: bool,
        blocks_gas_pricer: bool,
    }

    /// One live or finalized transaction gas-price fact supplied to Rust.
    struct GasPricerGasPrice {
        price: [u8; 32],
    }

    /// One configured wallet/account candidate for slashing proof submission.
    struct SlashingSubmitterFact {
        wallet_index: usize,
        nonce: [u8; 32],
        balance: [u8; 32],
    }

    /// C++-originated facts for planning a double-voting proof transaction.
    struct DoubleVotingProofInput {
        vote_a_hash: [u8; 32],
        vote_b_hash: [u8; 32],
        vote_a_period: u64,
        vote_b_period: u64,
        vote_a_round: u64,
        vote_b_round: u64,
        vote_a_step: u64,
        vote_b_step: u64,
        vote_a_rlp: Vec<u8>,
        vote_b_rlp: Vec<u8>,
        submitters: Vec<SlashingSubmitterFact>,
    }

    /// Rust slashing proof plan consumed by the C++ shim.
    struct DoubleVotingProofPlan {
        status: u8,
        should_submit: bool,
        proof_hash: [u8; 32],
        contract_address: [u8; 20],
        value: [u8; 32],
        gas_limit: u64,
        call_data: Vec<u8>,
        wallet_index: usize,
        nonce: [u8; 32],
    }

    /// Rust decision after consuming a C++ gas estimate.
    struct TransactionPackEstimateOutcome {
        hash: [u8; 32],
        selected: bool,
        demote_to_non_proposable: bool,
        stop: bool,
        gas_used: u64,
    }

    struct HashPeriod {
        hash: [u8; 32],
        period: u64,
    }

    struct VoteRlp {
        data: Vec<u8>,
    }

    struct PbftChainHeadPayload {
        head_hash: [u8; 32],
        size: u64,
        non_empty_size: u64,
        last_pbft_block_hash: [u8; 32],
        last_non_null_anchor_hash: [u8; 32],
    }

    struct PbftBlockValidationResult {
        ok: bool,
        code: u8,
        expected_period: u64,
        actual_period: u64,
        expected_prev_hash: [u8; 32],
        actual_prev_hash: [u8; 32],
    }

    struct ProposedBlockLookup {
        found: bool,
        is_valid: bool,
        block_rlp: Vec<u8>,
    }

    struct ProposedBlockPeriodHashes {
        period: u64,
        block_hashes: Vec<DagHash>,
    }

    struct ProposedBlockSnapshotEntry {
        period: u64,
        block_hash: [u8; 32],
        block_rlp: Vec<u8>,
        is_valid: bool,
    }

    struct PeriodDataQueueEntryRef {
        entry_id: u64,
        period: u64,
    }

    struct PeriodDataQueuePushOutcome {
        accepted: bool,
        clear_existing: bool,
        expected_next_period: u64,
        actual_period: u64,
        current_period: u64,
        effective_size: usize,
    }

    struct PeriodDataQueuePopPlan {
        entry_id: u64,
        use_last_block_cert_votes: bool,
        next_entry_id: u64,
        current_period: u64,
        effective_size: usize,
    }

    struct PeriodDataQueueLastEntryLookup {
        found: bool,
        entry_id: u64,
        period: u64,
    }

    struct VerifiedVotePayload {
        vote_hash: [u8; 32],
        block_hash: [u8; 32],
        voter: [u8; 20],
        period: u64,
        round: u64,
        step: u64,
        vote_type: u8,
        weight: u64,
    }

    /// Plain payload for a pillar vote carried across the CXX boundary.
    struct PillarVotePayload {
        vote_hash: [u8; 32],
        block_hash: [u8; 32],
        voter: [u8; 20],
        period: u64,
        weight: u64,
        vote_rlp: Vec<u8>,
    }

    /// Pre-weight pillar-vote identity supplied after Rust signature recovery.
    struct PillarVoteIdentityPayload {
        vote_hash: [u8; 32],
        voter: [u8; 20],
        period: u64,
    }

    /// Result of inspecting one PillarVote RLP payload in Rust.
    struct PillarVoteInspection {
        status: u8,
        period: u64,
        block_hash: [u8; 32],
        vote_hash: [u8; 32],
        voter: [u8; 20],
        signature_valid: bool,
    }

    /// Plain bundle fact consumed by the Rust planner for one planning pass.
    struct PillarVoteBundleFact {
        vote_hash: [u8; 32],
        block_hash: [u8; 32],
        voter: [u8; 20],
        period: u64,
        weight: u64,
        prevalidated: bool,
    }

    /// Result of a uniqueness check for one pillar vote.
    struct PillarVoteUniqueOutcome {
        is_unique: bool,
    }

    /// Result of inserting one pillar vote into Rust-owned aggregation.
    struct PillarVoteInsertOutcome {
        accepted: bool,
        duplicate: bool,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
        block_weight: u64,
    }

    /// Lightweight reference to a Rust-selected pillar vote.
    struct PillarVoteRef {
        vote_hash: [u8; 32],
        weight: u64,
    }

    /// Lightweight reference to a bundle-planned pillar vote.
    /// Includes the vote hash and weight carried from planner input.
    struct PillarVoteBundleAcceptedVote {
        vote_hash: [u8; 32],
        weight: u64,
    }

    /// Lookup result for one pillar block, optionally threshold-filtered.
    struct PillarVotesLookup {
        threshold_met: bool,
        block_weight: u64,
        selected_weight: u64,
        votes: Vec<PillarVoteRef>,
    }

    /// Result of a bundle planning pass.
    ///
    /// `status` values:
    /// - `0` - valid
    /// - `1` - empty bundle
    /// - `2` - vote period mismatch
    /// - `3` - vote block hash mismatch
    /// - `4` - prevalidation failed
    /// - `5` - zero vote weight
    /// - `6` - voter conflict
    /// - `7` - threshold not reached
    /// - `8` - weight overflow
    struct PillarVoteBundlePlan {
        status: u8,
        accepted_votes: Vec<PillarVoteBundleAcceptedVote>,
        block_weight: u64,
        selected_weight: u64,
        first_bad_vote_hash: [u8; 32],
    }

    /// Input facts for one pillar-vote relevance check.
    ///
    /// `has_current_pillar_block` gates whether C++ has provided current pillar
    /// context. When false, `current_pillar_block_period` and
    /// `current_pillar_block_hash` are ignored.
    struct PillarVoteRelevanceFact {
        vote_period: u64,
        vote_block_hash: [u8; 32],
        current_pillar_block_period: u64,
        current_pillar_block_hash: [u8; 32],
        has_current_pillar_block: bool,
        first_pillar_block_period: u64,
        pillar_blocks_interval: u64,
        vote_already_known: bool,
    }

    /// Deterministic relevance decision returned by Rust.
    ///
    /// Status values:
    /// - `0` - relevant
    /// - `1` - vote already known
    /// - `2` - missing current pillar block context
    /// - `3` - vote period mismatch
    /// - `4` - vote hash mismatch for `current_period + 1`
    struct PillarVoteRelevancePlan {
        status: u8,
        is_relevant: bool,
    }

    struct UniqueVoterCheckOutcome {
        is_unique: bool,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
    }

    struct UniqueVoterInsertOutcome {
        accepted: bool,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
        used_secondary_slot: bool,
        duplicate_vote_hash: bool,
    }

    struct VotedValueInsertOutcome {
        inserted: bool,
        total_weight: u64,
        votes_count: usize,
    }

    struct AtomicVoteInsertOutcome {
        inserted: bool,
        total_weight: u64,
        votes_count: usize,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
        used_secondary_slot: bool,
        duplicate_vote_hash: bool,
    }

    struct ThresholdDecisionOutcome {
        t_plus_one_reached: bool,
        network_t_plus_one_step_updated: bool,
        two_t_plus_one_reached: bool,
        two_t_plus_one_kind_found: bool,
        two_t_plus_one_kind: u8,
        two_t_plus_one_round_found: bool,
        two_t_plus_one_inserted: bool,
    }

    struct TwoTPlusOneInsertOutcome {
        round_found: bool,
        inserted: bool,
    }

    struct DetermineNewRoundOutcome {
        found: bool,
        new_round: u64,
        source_round: u64,
        source_kind: u8,
        block_hash: [u8; 32],
        step: u64,
    }

    struct TwoTPlusOneVotedBlockLookup {
        found: bool,
        block_hash: [u8; 32],
        step: u64,
    }

    struct TwoTPlusOneVotesLookup {
        found: bool,
        block_hash: [u8; 32],
        step: u64,
        vote_hashes: Vec<DagHash>,
    }

    struct NetworkTPlusOneStepLookup {
        found: bool,
        step: u64,
    }

    struct TwoTPlusOneSnapshotEntry {
        period: u64,
        round: u64,
        kind: u8,
        block_hash: [u8; 32],
        step: u64,
    }

    struct RoundMarkerSnapshot {
        period: u64,
        round: u64,
        network_t_plus_one_step: u64,
    }

    struct FinalChainBlockNumberLookup {
        found: bool,
        value: u64,
    }

    struct GenesisAccount {
        address: [u8; 20],
        balance: Vec<u8>,
    }

    struct GenesisValidator {
        address: [u8; 20],
        owner: [u8; 20],
        vrf_key: [u8; 32],
        commission: u16,
        description: String,
        endpoint: String,
        total_stake: Vec<u8>,
    }

    struct GenesisDposConfig {
        eligibility_balance_threshold: Vec<u8>,
        vote_eligibility_balance_step: Vec<u8>,
        validator_maximum_stake: Vec<u8>,
        // Exclusive period boundary below which legacy DAG VDF sortition uses
        // the snapshot total eligible vote count as denominator.
        dag_vdf_sortition_total_vote_count_until_period: u64,
    }

    struct AccountLookup {
        found: bool,
        nonce: u64,
        balance: Vec<u8>,
        storage_root_hash: [u8; 32],
        code_hash: [u8; 32],
        code_size: u64,
    }

    struct DposValidatorStake {
        address: [u8; 20],
        stake: Vec<u8>,
    }

    struct DposValidatorVoteCount {
        address: [u8; 20],
        vote_count: u64,
    }

    struct FinalChainCall {
        block_number: u64,
        sender: [u8; 20],
        receiver_found: bool,
        receiver: [u8; 20],
        value: Vec<u8>,
        gas_price: Vec<u8>,
        gas_limit: u64,
        input: Vec<u8>,
    }

    struct FinalChainCallOutcome {
        code_retval: Vec<u8>,
        gas_used: u64,
        code_err: String,
        consensus_err: String,
    }

    struct FinalizationOutcome {
        block_header_rlp: Vec<u8>,
        receipts: Vec<ReceiptRlp>,
    }

    struct FinalizationTransaction {
        hash: [u8; 32],
        sender: [u8; 20],
        receiver_found: bool,
        receiver: [u8; 20],
        nonce: u64,
        value: Vec<u8>,
        gas_price: Vec<u8>,
        gas_limit: u64,
        data: Vec<u8>,
        rlp: Vec<u8>,
    }

    struct ReceiptRlp {
        data: Vec<u8>,
    }

    struct DagHash {
        hash: [u8; 32],
    }

    /// Hash wrapper for transaction lists used by DAG planning payloads.
    struct DagTransactionHash {
        hash: [u8; 32],
    }

    /// Runtime snapshot for non-finalized DAG sync materialization.
    struct DagManagerRuntimeSyncSnapshot {
        period: u64,
        selected_hashes: Vec<DagHash>,
    }

    /// Canonical DAG block RLP selected for non-finalized sync payloads.
    struct DagSyncBlockRlp {
        hash: [u8; 32],
        block_rlp: Vec<u8>,
    }

    /// Rust-storage-backed non-finalized DAG sync payload.
    struct DagManagerNonFinalizedSyncPayload {
        period: u64,
        blocks: Vec<DagSyncBlockRlp>,
        transactions: Vec<DagTransactionRlpLookup>,
    }

    /// Rust-storage-backed transaction lookup result for DAG transaction materialization.
    struct DagTransactionRlpLookup {
        hash: [u8; 32],
        found: bool,
        /// True when the RLP was loaded through finalized transaction location metadata.
        finalized: bool,
        tx_rlp: Vec<u8>,
    }

    /// One ordered transaction lookup request for TransactionManager storage reads.
    ///
    /// `input_index` lets C++ validate and place the result without relying on vector
    /// position alone. `hash` is the canonical transaction hash being resolved.
    struct TransactionManagerStoredTransactionRequest {
        input_index: u64,
        hash: [u8; 32],
    }

    /// One TransactionManager storage lookup result.
    ///
    /// `source` is 0 for missing, 1 for pending/non-finalized storage, 2 for
    /// finalized regular period-data storage, and 3 for finalized system
    /// transaction storage. Missing transactions are data results rather than
    /// errors; malformed storage and backend failures are bridge errors.
    struct TransactionManagerStoredTransactionLookup {
        input_index: u64,
        hash: [u8; 32],
        found: bool,
        source: u8,
        tx_rlp: Vec<u8>,
    }

    /// One non-finalized transaction recovery entry loaded from Rust storage.
    ///
    /// `finalized` identifies stale pending rows that must be removed from
    /// non-finalized storage and must not be materialized into C++ live sidecars.
    struct TransactionManagerRecoveryEntry {
        hash: [u8; 32],
        finalized: bool,
        tx_rlp: Vec<u8>,
    }

    /// One sidecar insertion payload for live non-finalized transaction state.
    struct TransactionManagerSidecarInsertInput {
        hash: [u8; 32],
        trx_rlp: Vec<u8>,
    }

    /// One ordered sidecar lookup request for C++ transaction materialization.
    struct TransactionManagerSidecarLookupRequest {
        input_index: u64,
        hash: [u8; 32],
    }

    /// One sidecar lookup result preserving input ordering metadata.
    struct TransactionManagerSidecarLookup {
        input_index: u64,
        hash: [u8; 32],
        found: bool,
        source: u8,
        trx_rlp: Vec<u8>,
    }

    /// Ordered sidecar lookup plan for C++ materialization.
    struct TransactionManagerSidecarLookupPlan {
        lookups: Vec<TransactionManagerSidecarLookup>,
    }

    /// Canonical hash wrapper for sidecar transition lists.
    struct TransactionManagerSidecarHash {
        hash: [u8; 32],
    }

    /// One finalized transition payload for sidecar mutation.
    struct TransactionManagerSidecarTransitionInput {
        period: u64,
        hashes: Vec<TransactionManagerSidecarHash>,
    }

    /// One recovery insertion payload for sidecar state rebuild.
    struct TransactionManagerSidecarRecoveryInsertInput {
        hash: [u8; 32],
        finalized: bool,
        trx_rlp: Vec<u8>,
    }

    /// Queue-known fact used by Rust-owned TransactionManager known-admission decisions.
    struct TransactionManagerSidecarKnownFact {
        hash: [u8; 32],
        queue_known: bool,
    }

    /// Input transaction fact for sidecar-aware DAG transaction persistence.
    ///
    /// Rust computes sidecar membership from `BridgeTransactionManagerSidecar`
    /// instead of accepting C++ membership booleans.
    struct DagTransactionSaveSidecarFact {
        input_index: u64,
        hash: [u8; 32],
        trx_rlp: Vec<u8>,
        transaction_nonce: [u8; 32],
        sender_account_nonce: [u8; 32],
    }

    /// One non-finalized transaction payload persisted through Rust storage.
    ///
    /// The bridge caller must supply the canonical C++ transaction hash and RLP.
    /// Rust stores the payload under `hash` and does not re-hash `trx_rlp` at
    /// this storage boundary.
    struct NonFinalizedTransactionPayload {
        hash: [u8; 32],
        trx_rlp: Vec<u8>,
    }

    /// Input transaction fact for Rust planning of `TransactionManager::saveTransactionsFromDagBlock`.
    ///
    /// The caller supplies live C++ cache and FinalChain nonce facts. Rust owns
    /// duplicate filtering, nonce-gated finalized-storage lookup, persistence,
    /// and target count planning.
    struct DagTransactionSaveFact {
        input_index: u64,
        hash: [u8; 32],
        trx_rlp: Vec<u8>,
        transaction_nonce: [u8; 32],
        sender_account_nonce: [u8; 32],
        in_non_finalized_cache: bool,
        in_recently_finalized_cache: bool,
    }

    /// Accepted DAG transaction pointer for C++ live sidecar updates.
    struct DagTransactionSaveAccepted {
        input_index: u64,
        hash: [u8; 32],
        erased_from_queue: bool,
    }

    /// Rust planning outcome for one DAG transaction persistence pass.
    struct DagTransactionSaveOutcome {
        accepted: Vec<DagTransactionSaveAccepted>,
        target_transaction_count: u64,
    }

    /// Input finalized transaction fact for Rust planning of finalized status updates.
    struct FinalizedTransactionStatusFact {
        input_index: u64,
        hash: [u8; 32],
        in_non_finalized_cache: bool,
    }

    /// One finalized transaction action returned from Rust status planning.
    struct FinalizedTransactionStatusAction {
        input_index: u64,
        hash: [u8; 32],
        removed_non_finalized: bool,
        mark_transaction_known: bool,
        erase_from_queue: bool,
        erased_from_queue: bool,
    }

    /// Input finalized transaction payload for sidecar-aware status updates.
    struct FinalizedTransactionStatusSidecarFact {
        input_index: u64,
        hash: [u8; 32],
        trx_rlp: Vec<u8>,
    }

    /// Input for finalized transaction filtering from legacy C++.
    struct TransactionManagerFinalizedFilterFact {
        input_index: u64,
        hash: [u8; 32],
        in_recently_finalized_cache: bool,
    }

    /// Filtered finalized transaction action with preserved index mapping.
    struct TransactionManagerFilterAction {
        input_index: u64,
        hash: [u8; 32],
    }

    /// Finalized-filtering outcome for Rust-only decision logic.
    struct FinalizedTransactionFilterPlan {
        not_finalized: Vec<TransactionManagerFilterAction>,
    }

    /// Input for C++-owned `verifyTransactionsNotFinalized` decisions.
    struct TransactionManagerVerifyNotFinalizedFact {
        input_index: u64,
        hash: [u8; 32],
        transaction_nonce: [u8; 32],
        sender_account_nonce: [u8; 32],
        in_recently_finalized_cache: bool,
    }

    /// Input for Rust-owned sidecar `verifyTransactionsNotFinalized` decisions.
    struct TransactionManagerVerifyNotFinalizedSidecarFact {
        input_index: u64,
        hash: [u8; 32],
        transaction_nonce: [u8; 32],
        sender_account_nonce: [u8; 32],
    }

    /// Decision returned when the first finalized transaction is observed.
    ///
    /// `is_finalized` is false when all inputs are accepted.
    struct TransactionManagerVerifyNotFinalizedOutcome {
        is_finalized: bool,
        input_index: u64,
        hash: [u8; 32],
        source: u8,
    }

    /// Facts extracted by C++ for TransactionManager::verifyTransaction admission checks.
    struct TransactionManagerVerifyTransactionFact {
        /// Transaction hash being evaluated.
        tx_hash: [u8; 32],
        /// Transaction chain id.
        chain_id: u64,
        /// Configured node chain id.
        expected_chain_id: u64,
        /// Gas limit declared in the transaction.
        gas_limit: u64,
        /// Maximum gas limit configured in genesis.
        max_gas_limit: u64,
        /// Last finalized block number; supplied for precomputed hardfork evaluation.
        last_block_number: u64,
        /// Hardfork gate for Cornus is active.
        cornus_active: bool,
        /// `Transaction::intrinsicGasCovered()` result from C++ side.
        intrinsic_gas_covered: bool,
        /// Signature validation result from C++ side.
        signature_valid: bool,
        /// Gas price from the transaction envelope.
        gas_price: [u8; 32],
        /// Minimum gas price from chain policy.
        minimum_gas_price: [u8; 32],
    }

    /// TransactionManager::verifyTransaction plan status for C++.
    struct TransactionManagerVerifyTransactionOutcome {
        status: u8,
    }

    /// Facts extracted by C++ for TransactionManager::insertTransaction admission checks.
    struct TransactionManagerInsertTransactionFact {
        /// Transaction hash being evaluated.
        tx_hash: [u8; 32],
        /// Already known in the live transaction pool.
        hash_known: bool,
        /// Post-queue insertion status as returned by Rust queue adapter.
        queue_status: u8,
        /// Finalized period hint is available.
        has_finalized_period: bool,
        /// Finalized period hint used when `status == AlreadyFinalized`.
        finalized_period: u64,
    }

    /// TransactionManager::insertTransaction plan status for C++.
    struct TransactionManagerInsertTransactionOutcome {
        status: u8,
        finalized_period_known: bool,
        finalized_period: u64,
    }

    /// Facts extracted by C++ before mutating the live transaction queue.
    struct TransactionManagerValidatedInsertFact {
        tx_hash: [u8; 32],
        transaction_nonce: [u8; 32],
        transaction_cost: [u8; 32],
        gas_limit: u64,
        propose_dag_gas_limit: u64,
        insert_non_proposable: bool,
        in_non_finalized_cache: bool,
        in_recently_finalized_cache: bool,
        account_found: bool,
        account_nonce: [u8; 32],
        account_balance: [u8; 32],
    }

    /// Facts extracted by C++ before queue insertion when Rust owns manager sidecars.
    struct TransactionManagerValidatedInsertSidecarFact {
        tx_hash: [u8; 32],
        transaction_nonce: [u8; 32],
        transaction_cost: [u8; 32],
        gas_limit: u64,
        propose_dag_gas_limit: u64,
        insert_non_proposable: bool,
        account_found: bool,
        account_nonce: [u8; 32],
        account_balance: [u8; 32],
    }

    /// Plan for C++ live queue insertion.
    ///
    /// `queue_action`:
    /// - `0`: no queue mutation; return `status` directly
    /// - `1`: insert as proposable
    /// - `2`: insert as non-proposable
    struct TransactionManagerValidatedInsertPlan {
        status: u8,
        queue_action: u8,
        emit_transaction_added: bool,
    }

    /// Runtime-executed insert outcome that includes concrete queue mutations.
    struct TransactionManagerRuntimeValidatedInsertOutcome {
        status: u8,
        emit_transaction_added: bool,
        inserted_hash_found: bool,
        inserted_hash: [u8; 32],
        demoted_hashes: Vec<TransactionQueueHash>,
        overflow_removed_hashes: Vec<TransactionQueueHash>,
    }

    /// Runtime-executed TransactionManager admission outcome.
    ///
    /// Rust owns the validated-admission queue mutation and the public
    /// `insertTransaction` status mapping. C++ supplies verification,
    /// FinalChain account/finalized facts, and executes returned event/logging
    /// side effects.
    struct TransactionManagerRuntimeAdmissionOutcome {
        insert_status: u8,
        transaction_status: u8,
        requires_finalized_lookup: bool,
        finalized_period_known: bool,
        finalized_period: u64,
        emit_transaction_added: bool,
        inserted_hash_found: bool,
        inserted_hash: [u8; 32],
        demoted_hashes: Vec<TransactionQueueHash>,
        overflow_removed_hashes: Vec<TransactionQueueHash>,
    }

    /// Finalized status planning outcome for one finalized period.
    struct FinalizedTransactionStatusPlan {
        accepted: Vec<FinalizedTransactionStatusAction>,
        target_transaction_count: u64,
        stale_period: u64,
        has_stale_period: bool,
        purge_transaction_queue: bool,
    }

    /// Transaction hashes for one DAG block, preserving block-local order.
    struct DagBlockTransactionRefs {
        transaction_hashes: Vec<DagTransactionHash>,
    }

    /// Finalization hint for one transaction referenced by an expired DAG block.
    struct DagExpiredTransactionFact {
        hash: [u8; 32],
        finalized: bool,
    }

    /// Deterministic finalization cleanup payload for expired DAG blocks.
    ///
    /// Callers receive full expired-transaction context to support legacy
    /// storage removals while also receiving compact removal hashes suitable for
    /// direct status updates.
    struct DagExpiredTransactionCleanupPayload {
        /// Transaction facts grouped by discovered order across expired DAG blocks.
        expired_transaction_facts: Vec<DagExpiredTransactionFact>,
        /// Unique hashes that should be removed from non-finalized storage.
        remove_hashes: Vec<DagTransactionHash>,
    }

    /// Query plan returned for additional DAG transaction lookups.
    struct DagTransactionQueryPlan {
        query_hashes: Vec<DagTransactionHash>,
    }

    /// Cleanup plan returned for non-finalized transaction removals.
    struct DagExpiredTransactionCleanupPlan {
        remove_hashes: Vec<DagTransactionHash>,
    }

    struct FinalizationDagBlock {
        author: [u8; 20],
        transaction_hashes: Vec<DagHash>,
    }

    struct DagLevelHashes {
        level: u64,
        hashes: Vec<DagHash>,
    }

    struct DagOrder {
        found: bool,
        hashes: Vec<DagHash>,
    }

    struct DagFrontier {
        pivot: [u8; 32],
        tips: Vec<DagHash>,
    }

    struct DagReferenceMetadata {
        hash: [u8; 32],
        found: bool,
        level: u64,
    }

    struct DagPivotTipsValidation {
        ok: bool,
        expected_level: u64,
        level_matches: bool,
        missing_references: Vec<DagHash>,
    }

    /// C++-originated payload for Rust DAG block verification prechecks.
    struct DagVerifyPrecheckBlock {
        level: u64,
        pivot: [u8; 32],
        tips: Vec<DagHash>,
    }

    /// Rust DAG block verification precheck decision.
    struct DagVerifyPrecheckResult {
        continue_validation: bool,
        reject_code: u32,
        proposal_period_found: bool,
        proposal_period: u64,
    }

    /// Per-tip gas metadata for Rust DAG verification gas decisions.
    struct DagTipGas {
        found: bool,
        gas_estimation: u64,
    }

    /// C++-originated payload for Rust transaction availability decisions.
    struct DagVerifyTransactionAvailabilityInput {
        expected_transactions: u64,
        resolved_transactions: u64,
    }

    /// Rust transaction availability decision.
    struct DagVerifyTransactionAvailabilityResult {
        continue_validation: bool,
        reject_code: u32,
    }

    /// C++-originated payload for Rust VDF verification preparation.
    struct DagVerifyVdfPrepareInput {
        vrf_key_found: bool,
        eligible_vote_count: u64,
        vdf_max_vote_count: u64,
    }

    /// Rust VDF verification preparation result.
    struct DagVerifyVdfPrepareResult {
        continue_validation: bool,
        reject_code: u32,
        reason_code: u32,
        vote_count: u64,
        max_vote_count: u64,
    }

    /// C++-originated payload for Rust authorization decisions.
    struct DagVerifyAuthorizationInput {
        vdf_valid: bool,
        dpos_snapshot_available: bool,
        dpos_eligible: bool,
    }

    /// Rust authorization decision.
    struct DagVerifyAuthorizationResult {
        continue_validation: bool,
        reject_code: u32,
        reason_code: u32,
    }

    /// C++-originated payload for Rust DAG VDF sortition verification.
    struct DagVerifyVdfSortitionInput {
        /// Canonical DAG block RLP bytes.
        block_rlp: Vec<u8>,
        /// VDF message used for Wesolowski proof verification.
        vdf_input: Vec<u8>,
        /// Runtime sortition parameters for this proposal period.
        sortition_params: SortitionRuntimeParams,
        /// Optional legacy path input: precomputed VRF output (64 bytes).
        ///
        /// Rust uses `vrf_public_key` + `vrf_input` when both are provided.
        vrf_output: Vec<u8>,
        /// Embedded VRF public key (32 bytes) for direct Rust verification.
        vrf_public_key: Vec<u8>,
        /// Canonical VRF message used to verify the DAG embedded VRF proof.
        vrf_input: Vec<u8>,
        /// Sender-eligible vote count for threshold normalization.
        sender_eligible_vote_count: u64,
        /// Period-effective maximum vote count for normalization denominator.
        vdf_sortition_max_vote_count: u64,
    }

    /// Rust DAG VDF sortition verification result.
    struct DagVerifyVdfSortitionResult {
        vdf_status: u8,
        difficulty: u16,
        expected_difficulty: u16,
    }

    /// C++-originated payload to build legacy VRF/VDF messages from block RLP
    /// and verify embedded sortition proof.
    struct DagVerifyVdfSortitionFromBlockInput {
        /// Canonical DAG block RLP bytes.
        block_rlp: Vec<u8>,
        /// DAG block level used in legacy VRF message construction.
        block_level: u64,
        /// Legacy proposal-period hash used in legacy VRF message construction.
        proposal_period_hash: [u8; 32],
        /// Runtime sortition parameters for this proposal period.
        sortition_params: SortitionRuntimeParams,
        /// Embedded VRF public key (32 bytes) for direct Rust verification.
        vrf_public_key: [u8; 32],
        /// Sender-eligible vote count for threshold normalization.
        sender_eligible_vote_count: u64,
        /// Period-effective maximum vote count for normalization denominator.
        vdf_sortition_max_vote_count: u64,
    }

    /// C++-originated VDF and DPoS facts for Rust authorization decisions.
    struct DagVerifyVdfDposFacts {
        vrf_key_found: bool,
        sender_eligible_vote_count: u64,
        vdf_sortition_max_vote_count: u64,
        vdf_status: u8,
        dpos_status: u8,
    }

    /// Rust-collected DPoS and VRF facts for DAG authorization.
    struct DagDposAuthorizationFacts {
        vrf_key_found: bool,
        vrf_key: Vec<u8>,
        sender_eligible_vote_count: u64,
        vdf_sortition_max_vote_count: u64,
        eligibility_status: u8,
    }

    /// C++-originated proposer eligibility facts.
    struct DagProposerEligibilityInput {
        proposal_period_found: bool,
        wallet_vrf_public_key: [u8; 32],
        authorization_facts: DagDposAuthorizationFacts,
    }

    /// Rust producer-side proposer eligibility decision.
    struct DagProposerEligibilityDecision {
        action: u8,
        reason_code: u32,
        vote_count: u64,
        max_vote_count: u64,
    }

    /// C++-originated tip candidate facts for Rust proposer tip selection.
    struct DagProposerTipCandidate {
        hash: [u8; 32],
        found: bool,
        sender: [u8; 20],
        level: u64,
        gas_estimation: u64,
    }

    /// Rust producer-side tip selection result.
    struct DagProposerTipSelection {
        selected: Vec<DagHash>,
        skipped_missing: u64,
    }

    /// Rust VDF and DPoS authorization decision.
    struct DagVerifyVdfDposDecision {
        continue_validation: bool,
        reject_code: u32,
        reason_code: u32,
        vote_count: u64,
        max_vote_count: u64,
    }

    /// C++-originated payload for Rust gas verification decisions.
    struct DagVerifyGasInput {
        block_gas_estimation: u64,
        estimated_transactions_weight: u64,
        dag_gas_limit: u64,
        pbft_gas_limit: u64,
        tip_gas_estimations: Vec<DagTipGas>,
    }

    /// Rust gas verification decision.
    struct DagVerifyGasResult {
        continue_validation: bool,
        reject_code: u32,
    }

    struct DagManagerBlock {
        hash: [u8; 32],
        pivot: [u8; 32],
        tips: Vec<DagHash>,
        level: u64,
        difficulty: u32,
    }

    struct DagManagerSnapshot {
        old_anchor: [u8; 32],
        anchor: [u8; 32],
        anchor_level: u64,
        period: u64,
        max_level: u64,
        dag_expiry_level: u64,
        non_finalized_min_difficulty: u32,
        non_finalized_blocks: Vec<DagManagerBlock>,
    }

    struct DagManagerAnchors {
        old_anchor: [u8; 32],
        anchor: [u8; 32],
    }

    struct DagManagerFinalizationPlan {
        finalized_count: u64,
        counter_update_hashes: Vec<DagHash>,
        expired_hashes: Vec<DagHash>,
        remaining_hashes: Vec<DagHash>,
        /// Transaction hashes that can be removed after this finalized transition.
        ///
        /// Plan payloads are pre-apply facts. Apply payloads return the same hashes
        /// after Rust has removed them from non-finalized storage, so C++ must use
        /// them only for live sidecar cleanup.
        remove_transaction_hashes: Vec<DagTransactionHash>,
    }

    /// Storage-derived counter update fact for a finalized DAG block.
    struct DagFinalizedCounterUpdate {
        hash: [u8; 32],
        level: u64,
        tips_count: u64,
    }

    /// Rust-storage-backed cleanup payload after applying a finalized DAG order.
    struct DagManagerFinalizationCleanupPayload {
        counter_updates: Vec<DagFinalizedCounterUpdate>,
        expired_hashes: Vec<DagHash>,
        /// Expired transaction hashes selected for Rust-owned storage deletion.
        remove_transaction_hashes: Vec<DagTransactionHash>,
    }

    /// Rust-applied finalized DAG order result for C++ live side effects.
    struct DagManagerFinalizationApplyPayload {
        finalized_count: u64,
        expired_hashes: Vec<DagHash>,
        /// Expired transaction hashes already removed from Rust-owned
        /// non-finalized storage. C++ must only clear live sidecars for them.
        remove_transaction_hashes: Vec<DagTransactionHash>,
    }

    struct DagManagerNonFinalizedSize {
        levels: u64,
        blocks: u64,
    }

    struct SortitionRuntimeConfig {
        threshold_upper: u16,
        difficulty_min: u16,
        difficulty_max: u16,
        difficulty_stale: u16,
        lambda_bound: u16,
        changes_count_for_average: u16,
        dag_efficiency_target_low: u16,
        dag_efficiency_target_high: u16,
        changing_interval: u16,
        computation_interval: u16,
    }

    struct SortitionRuntimeParams {
        threshold_upper: u16,
        difficulty_min: u16,
        difficulty_max: u16,
        difficulty_stale: u16,
        lambda_bound: u16,
    }

    struct SortitionParamsChangePayload {
        period: u64,
        interval_efficiency: u16,
        threshold_upper: u16,
    }

    struct SortitionParamsChangeResult {
        changed: bool,
        period: u64,
        interval_efficiency: u16,
        threshold_upper: u16,
    }

    struct SortitionEfficiencyResult {
        ok: bool,
        value: u16,
        error: String,
    }

    struct LegacySortitionParams {
        vrf_threshold_upper: u16,
        vdf_difficulty_min: u16,
        vdf_difficulty_max: u16,
        vdf_difficulty_stale: u16,
        vdf_lambda_bound: u16,
    }

    struct VrfVerifyResult {
        ok: bool,
        status: u8,
        error: String,
        output: [u8; 64],
        threshold: u16,
    }

    struct VrfProofResult {
        ok: bool,
        status: u8,
        error: String,
        public_key: [u8; 32],
        proof: [u8; 80],
        output: [u8; 64],
        threshold: u16,
    }

    struct VrfVerifyOutput {
        is_valid: bool,
        output: Vec<u8>,
    }

    struct VdfSortitionVerifyResult {
        ok: bool,
        status: u8,
        error: String,
        vrf_output: [u8; 64],
        vrf_threshold: u16,
        expected_difficulty: u16,
        actual_difficulty: u16,
    }

    struct VdfSortitionPayload {
        vrf_proof: [u8; 80],
        vdf_solution_proof: Vec<u8>,
        vdf_solution_output: Vec<u8>,
        difficulty: u16,
    }

    struct VdfSortitionVerifyConfig {
        threshold_upper: u16,
        difficulty_min: u16,
        difficulty_max: u16,
        difficulty_stale: u16,
        lambda_bound: u16,
    }

    struct VdfSortitionPayloadVerifyResult {
        vdf_status: u8,
        difficulty: u16,
        expected_difficulty: u16,
    }

    struct VdfSortitionProofResult {
        ok: bool,
        status: u8,
        error: String,
        vrf_proof: [u8; 80],
        vrf_output: [u8; 64],
        vrf_threshold: u16,
        difficulty: u16,
        vdf_proof: Vec<u8>,
        vdf_output: Vec<u8>,
    }

    extern "Rust" {
        type WesolowskiVdf;
        type CancellationToken;
        type Solution;

        pub fn make_vdf(
            lambda: u32,
            time_bits: u32,
            input: &[u8],
            modulus: &[u8],
        ) -> Box<WesolowskiVdf>;

        pub fn make_solution(proof: &[u8], output: &[u8]) -> Box<Solution>;

        pub fn make_cancellation_token() -> Box<CancellationToken>;
        pub unsafe fn make_cancellation_token_with_atomic(
            atomic_ptr: *const bool,
        ) -> Box<CancellationToken>;
        pub fn cancellation_token_cancel(token: &CancellationToken);

        pub fn prove(vdf: &WesolowskiVdf, cancelled: &CancellationToken) -> Box<Solution>;
        pub fn verify(vdf: &WesolowskiVdf, solution: &Solution) -> bool;

        pub fn solution_get_proof(solution: &Solution) -> &[u8];
        pub fn solution_get_output(solution: &Solution) -> &[u8];

        pub fn vdf_sortition_payload_encode(payload: &VdfSortitionPayload) -> Vec<u8>;

        pub fn vdf_sortition_payload_decode(payload: &[u8]) -> Result<VdfSortitionPayload>;

        pub fn vdf_sortition_payload_verify(
            payload: &VdfSortitionPayload,
            vdf_input: &[u8],
            config: VdfSortitionVerifyConfig,
            vrf_output: &[u8],
            sender_eligible_vote_count: u64,
            vdf_sortition_max_vote_count: u64,
        ) -> Result<VdfSortitionPayloadVerifyResult>;

        pub fn vdf_sortition_payload_verify_with_modulus(
            payload: &VdfSortitionPayload,
            vdf_input: &[u8],
            config: VdfSortitionVerifyConfig,
            vrf_output: &[u8],
            sender_eligible_vote_count: u64,
            vdf_sortition_max_vote_count: u64,
            modulus: &[u8],
        ) -> Result<VdfSortitionPayloadVerifyResult>;

        pub fn vdf_sortition_threshold_from_output(
            vrf_output: &[u8],
            vote_count: u16,
        ) -> Result<u16>;

        pub fn vdf_sortition_normalize_vote_count(
            sender_eligible_vote_count: u64,
            vdf_sortition_max_vote_count: u64,
        ) -> Result<u16>;

        pub fn vdf_sortition_difficulty(
            config: VdfSortitionVerifyConfig,
            threshold: u16,
        ) -> Result<u16>;

        pub fn vdf_sortition_legacy_modulus() -> Vec<u8>;

        pub fn vrf_verify_output(
            vrf_public_key: &[u8],
            vrf_proof: &[u8],
            message: &[u8],
        ) -> Result<VrfVerifyOutput>;

        pub fn vrf_proof_to_hash(vrf_proof: &[u8]) -> Result<Vec<u8>>;

        pub fn vrf_prove_output(vrf_secret_key: &[u8], message: &[u8]) -> Result<Vec<u8>>;

        pub fn verify_legacy_vrf_sortition(
            public_key: &[u8; 32],
            proof: &[u8; 80],
            message: &[u8],
            vote_count: u16,
            strict: bool,
        ) -> VrfVerifyResult;

        pub fn prove_legacy_vrf_sortition(
            secret_key: &[u8; 64],
            message: &[u8],
            vote_count: u16,
        ) -> VrfProofResult;

        pub fn prove_legacy_vdf_sortition(
            params: LegacySortitionParams,
            secret_key: &[u8; 64],
            vrf_input: &[u8],
            vdf_input: &[u8],
            vote_count: u64,
            total_vote_count: u64,
            cancellation_token: &CancellationToken,
        ) -> VdfSortitionProofResult;

        pub fn verify_legacy_vdf_sortition(
            params: LegacySortitionParams,
            public_key: &[u8; 32],
            sortition_rlp: &[u8],
            vrf_input: &[u8],
            vdf_input: &[u8],
            vote_count: u64,
            total_vote_count: u64,
        ) -> VdfSortitionVerifyResult;

        // Consensus DAG

        type BridgeDagGraph;

        pub fn create_dag_graph(genesis: &[u8; 32]) -> Box<BridgeDagGraph>;
        pub fn dag_vertex_count(self: &BridgeDagGraph) -> usize;
        pub fn dag_edge_count(self: &BridgeDagGraph) -> usize;
        pub fn dag_has_vertex(self: &BridgeDagGraph, vertex: &[u8; 32]) -> bool;
        pub fn dag_add_vertex_edges(
            self: &mut BridgeDagGraph,
            new_vertex: &[u8; 32],
            pivot: &[u8; 32],
            tips: Vec<DagHash>,
        ) -> bool;
        pub fn dag_leaves(self: &BridgeDagGraph) -> Vec<DagHash>;
        pub fn dag_ghost_path(self: &BridgeDagGraph, root: &[u8; 32]) -> Vec<DagHash>;
        pub fn dag_compute_order(
            self: &BridgeDagGraph,
            anchor: &[u8; 32],
            non_finalized_blocks: Vec<DagLevelHashes>,
        ) -> DagOrder;
        pub fn dag_derive_frontier(ghost_path: Vec<DagHash>, leaves: Vec<DagHash>) -> DagFrontier;
        pub fn dag_validate_pivot_tips_metadata(
            block_level: u64,
            pivot: DagReferenceMetadata,
            tips: Vec<DagReferenceMetadata>,
        ) -> DagPivotTipsValidation;
        pub fn dag_clear(self: &mut BridgeDagGraph);
        pub fn dag_graphviz_dot(self: &BridgeDagGraph) -> String;

        type BridgeDagManagerState;

        pub fn create_dag_manager_state(
            genesis: &[u8; 32],
            dag_expiry_limit: u32,
        ) -> Result<Box<BridgeDagManagerState>>;
        pub fn dag_manager_rebuild(
            self: &mut BridgeDagManagerState,
            snapshot: DagManagerSnapshot,
        ) -> Result<()>;
        pub fn dag_manager_add_block(
            self: &mut BridgeDagManagerState,
            block: DagManagerBlock,
        ) -> Result<()>;
        pub fn dag_manager_validate_pivot_tips(
            self: &BridgeDagManagerState,
            block_level: u64,
            pivot: &[u8; 32],
            tips: Vec<DagHash>,
        ) -> DagPivotTipsValidation;
        pub fn dag_manager_compute_order(
            self: &BridgeDagManagerState,
            anchor: &[u8; 32],
        ) -> DagOrder;
        pub fn dag_manager_frontier(self: &BridgeDagManagerState) -> DagFrontier;
        pub fn dag_manager_ghost_path(
            self: &BridgeDagManagerState,
            source: &[u8; 32],
        ) -> Vec<DagHash>;
        pub fn dag_manager_anchor_ghost_path(self: &BridgeDagManagerState) -> Vec<DagHash>;
        pub fn dag_manager_graphviz_dot(self: &BridgeDagManagerState, pivot_tree: bool) -> String;
        pub fn dag_manager_vertex_count(self: &BridgeDagManagerState) -> usize;
        pub fn dag_manager_edge_count(self: &BridgeDagManagerState) -> usize;
        pub fn dag_manager_max_level(self: &BridgeDagManagerState) -> u64;
        pub fn dag_manager_latest_period(self: &BridgeDagManagerState) -> u64;
        pub fn dag_manager_anchors(self: &BridgeDagManagerState) -> DagManagerAnchors;
        pub fn dag_manager_dag_expiry_limit(self: &BridgeDagManagerState) -> u32;
        pub fn dag_manager_dag_expiry_level(self: &BridgeDagManagerState) -> u64;
        pub fn dag_manager_non_finalized_blocks(
            self: &BridgeDagManagerState,
        ) -> Vec<DagLevelHashes>;
        pub fn dag_manager_non_finalized_blocks_size(
            self: &BridgeDagManagerState,
        ) -> DagManagerNonFinalizedSize;
        pub fn dag_manager_non_finalized_min_difficulty(self: &BridgeDagManagerState) -> u32;

        type BridgeDagManagerRuntime;

        pub fn create_dag_manager_runtime_from_storage(
            genesis: &[u8; 32],
            dag_expiry_limit: u32,
            storage: &BridgeStorage,
        ) -> Result<Box<BridgeDagManagerRuntime>>;
        pub fn dag_manager_runtime_rebuild(
            self: &mut BridgeDagManagerRuntime,
            snapshot: DagManagerSnapshot,
        ) -> Result<()>;
        pub fn dag_manager_runtime_add_block(
            self: &mut BridgeDagManagerRuntime,
            block: DagManagerBlock,
        ) -> Result<()>;
        /// Applies finalized DAG order using Rust state and Rust storage.
        pub fn dag_manager_runtime_apply_finalized_order(
            self: &mut BridgeDagManagerRuntime,
            new_anchor: [u8; 32],
            new_period: u64,
            finalized_order: Vec<DagHash>,
        ) -> Result<DagManagerFinalizationApplyPayload>;
        /// Returns current runtime sync snapshot for non-finalized materialization.
        pub fn dag_manager_runtime_non_finalized_sync_snapshot(
            self: &BridgeDagManagerRuntime,
            known_hashes: Vec<DagHash>,
        ) -> DagManagerRuntimeSyncSnapshot;
        /// Returns non-finalized sync DAG block RLPs and referenced transaction
        /// RLPs through Rust-owned storage access.
        pub fn dag_manager_runtime_non_finalized_sync_payload(
            self: &BridgeDagManagerRuntime,
            known_hashes: Vec<DagHash>,
        ) -> Result<DagManagerNonFinalizedSyncPayload>;
        pub fn dag_manager_runtime_compute_order(
            self: &BridgeDagManagerRuntime,
            anchor: &[u8; 32],
        ) -> DagOrder;
        pub fn dag_manager_runtime_select_non_finalized_hashes(
            self: &BridgeDagManagerRuntime,
            known_hashes: Vec<DagHash>,
        ) -> Vec<DagHash>;
        pub fn dag_manager_runtime_frontier(self: &BridgeDagManagerRuntime) -> DagFrontier;
        pub fn dag_manager_runtime_ghost_path(
            self: &BridgeDagManagerRuntime,
            source: &[u8; 32],
        ) -> Vec<DagHash>;
        pub fn dag_manager_runtime_anchor_ghost_path(
            self: &BridgeDagManagerRuntime,
        ) -> Vec<DagHash>;
        pub fn dag_manager_runtime_graphviz_dot(
            self: &BridgeDagManagerRuntime,
            pivot_tree: bool,
        ) -> String;
        pub fn dag_manager_runtime_vertex_count(self: &BridgeDagManagerRuntime) -> usize;
        pub fn dag_manager_runtime_edge_count(self: &BridgeDagManagerRuntime) -> usize;
        pub fn dag_manager_runtime_max_level(self: &BridgeDagManagerRuntime) -> u64;
        pub fn dag_manager_runtime_latest_period(self: &BridgeDagManagerRuntime) -> u64;
        pub fn dag_manager_runtime_anchors(self: &BridgeDagManagerRuntime) -> DagManagerAnchors;
        pub fn dag_manager_runtime_dag_expiry_limit(self: &BridgeDagManagerRuntime) -> u32;
        pub fn dag_manager_runtime_dag_expiry_level(self: &BridgeDagManagerRuntime) -> u64;
        pub fn dag_manager_runtime_non_finalized_blocks(
            self: &BridgeDagManagerRuntime,
        ) -> Vec<DagLevelHashes>;
        pub fn dag_manager_runtime_non_finalized_blocks_size(
            self: &BridgeDagManagerRuntime,
        ) -> DagManagerNonFinalizedSize;
        pub fn dag_manager_runtime_non_finalized_min_difficulty(
            self: &BridgeDagManagerRuntime,
        ) -> u32;
        pub fn dag_manager_runtime_block_exists(
            self: &BridgeDagManagerRuntime,
            hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn dag_manager_runtime_load_block(
            self: &BridgeDagManagerRuntime,
            hash: &[u8; 32],
        ) -> Result<DagBlockLookup>;
        pub fn dag_manager_runtime_save_block(
            self: &BridgeDagManagerRuntime,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn dag_manager_runtime_ensure_proposal_period_mapping(
            self: &BridgeDagManagerRuntime,
            level: u64,
            period: u64,
        ) -> Result<bool>;
        pub fn dag_manager_runtime_persistence_counters(
            self: &BridgeDagManagerRuntime,
        ) -> Result<DagPersistenceCounters>;
        pub fn dag_manager_runtime_verify_precheck(
            self: &BridgeDagManagerRuntime,
            block: DagVerifyPrecheckBlock,
        ) -> Result<DagVerifyPrecheckResult>;
        pub fn dag_verify_transaction_availability(
            input: DagVerifyTransactionAvailabilityInput,
        ) -> DagVerifyTransactionAvailabilityResult;
        /// Plans verifyBlock transaction queries from block hashes and already-supplied
        /// hashes.
        pub fn dag_plan_verify_transaction_query(
            block_transaction_hashes: Vec<DagTransactionHash>,
            supplied_transaction_hashes: Vec<DagTransactionHash>,
        ) -> DagTransactionQueryPlan;
        /// Plans unique transaction hashes needed from non-finalized DAG blocks.
        pub fn dag_plan_non_finalized_transaction_query(
            blocks: Vec<DagBlockTransactionRefs>,
        ) -> DagTransactionQueryPlan;
        /// Plans non-finalized transaction removals after expired DAG block
        /// finalization, excluding finalized and still-retained hashes.
        pub fn dag_plan_expired_transaction_cleanup(
            expired_candidates: Vec<DagExpiredTransactionFact>,
            retained_transaction_refs: Vec<DagTransactionHash>,
        ) -> DagExpiredTransactionCleanupPlan;
        /// Builds a compact finalization cleanup payload from plan candidates.
        pub fn dag_manager_runtime_expired_transaction_cleanup_payload(
            self: &BridgeDagManagerRuntime,
            expired_hashes: Vec<DagHash>,
            remaining_hashes: Vec<DagHash>,
        ) -> Result<DagExpiredTransactionCleanupPayload>;
        pub fn dag_verify_vdf_prepare(input: DagVerifyVdfPrepareInput)
            -> DagVerifyVdfPrepareResult;
        pub fn dag_verify_vdf_sortition(
            input: DagVerifyVdfSortitionInput,
        ) -> Result<DagVerifyVdfSortitionResult>;
        pub fn dag_verify_vdf_sortition_from_block(
            input: DagVerifyVdfSortitionFromBlockInput,
        ) -> Result<DagVerifyVdfSortitionResult>;
        pub fn dag_vrf_input(block_level: u64, proposal_period_hash: &[u8; 32]) -> Vec<u8>;
        pub fn dag_vdf_message(pivot: &[u8; 32], transaction_hashes: Vec<DagHash>) -> Vec<u8>;
        pub fn dag_proposer_check_eligibility(
            input: DagProposerEligibilityInput,
        ) -> DagProposerEligibilityDecision;
        pub fn dag_proposer_select_tips(
            candidates: Vec<DagProposerTipCandidate>,
            gas_limit: u64,
            max_tips: u16,
        ) -> DagProposerTipSelection;
        pub fn dag_verify_authorization(
            input: DagVerifyAuthorizationInput,
        ) -> DagVerifyAuthorizationResult;
        pub fn dag_decide_vdf_dpos_authorization(
            facts: DagVerifyVdfDposFacts,
        ) -> DagVerifyVdfDposDecision;
        pub fn dag_verify_gas(input: DagVerifyGasInput) -> Result<DagVerifyGasResult>;

        // Consensus PBFT chain

        type BridgePbftChain;

        pub fn create_pbft_chain(head: PbftChainHeadPayload) -> Result<Box<BridgePbftChain>>;
        pub fn pbft_chain_head(self: &BridgePbftChain) -> PbftChainHeadPayload;
        pub fn pbft_chain_project_update(
            self: &BridgePbftChain,
            block_hash: &[u8; 32],
            anchor_hash: &[u8; 32],
        ) -> Result<PbftChainHeadPayload>;
        pub fn pbft_chain_project_legacy_json_head(
            self: &BridgePbftChain,
            block_hash: &[u8; 32],
            increments_non_empty_size: bool,
        ) -> Result<PbftChainHeadPayload>;
        pub fn pbft_chain_update(
            self: &mut BridgePbftChain,
            block_hash: &[u8; 32],
            anchor_hash: &[u8; 32],
        ) -> Result<PbftChainHeadPayload>;
        pub fn pbft_chain_validate_block(
            self: &BridgePbftChain,
            period: u64,
            prev_hash: &[u8; 32],
        ) -> PbftBlockValidationResult;

        // Consensus proposed PBFT blocks

        type BridgeProposedBlocks;

        pub fn create_proposed_blocks_index() -> Box<BridgeProposedBlocks>;
        pub fn proposed_blocks_push(
            self: &mut BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> bool;
        pub fn proposed_blocks_mark_valid(
            self: &mut BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
        ) -> Result<()>;
        pub fn proposed_blocks_get(
            self: &BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
        ) -> ProposedBlockLookup;
        pub fn proposed_blocks_contains(
            self: &BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
        ) -> bool;
        pub fn proposed_blocks_cleanup_candidates(
            self: &BridgeProposedBlocks,
            period: u64,
        ) -> Vec<ProposedBlockPeriodHashes>;
        pub fn proposed_blocks_remove_period(self: &mut BridgeProposedBlocks, period: u64);
        pub fn proposed_blocks_old_blocks_message(
            self: &BridgeProposedBlocks,
            current_period: u64,
        ) -> String;
        pub fn proposed_blocks_snapshot_entries(
            self: &BridgeProposedBlocks,
        ) -> Vec<ProposedBlockSnapshotEntry>;
        pub fn proposed_blocks_snapshot(
            self: &BridgeProposedBlocks,
        ) -> Vec<ProposedBlockPeriodHashes>;

        // Consensus period-data queue

        type BridgePeriodDataQueue;

        pub fn create_period_data_queue() -> Box<BridgePeriodDataQueue>;
        pub fn period_data_queue_period(self: &BridgePeriodDataQueue) -> u64;
        pub fn period_data_queue_size(self: &BridgePeriodDataQueue) -> usize;
        pub fn period_data_queue_empty(self: &BridgePeriodDataQueue) -> bool;
        pub fn period_data_queue_clear(self: &mut BridgePeriodDataQueue);
        pub fn period_data_queue_push(
            self: &mut BridgePeriodDataQueue,
            entry_id: u64,
            period: u64,
            max_pbft_size: u64,
            current_block_cert_votes_count: usize,
        ) -> Result<PeriodDataQueuePushOutcome>;
        pub fn period_data_queue_pop(
            self: &mut BridgePeriodDataQueue,
        ) -> Result<PeriodDataQueuePopPlan>;
        pub fn period_data_queue_last_entry(
            self: &BridgePeriodDataQueue,
        ) -> PeriodDataQueueLastEntryLookup;
        pub fn period_data_queue_clean_old_data(
            self: &mut BridgePeriodDataQueue,
            period: u64,
        ) -> Vec<PeriodDataQueueEntryRef>;

        // Consensus transaction queue

        type BridgeTransactionQueue;

        pub fn create_transaction_queue(
            config: TransactionQueueConfig,
        ) -> Box<BridgeTransactionQueue>;
        pub fn transaction_queue_insert(
            self: &mut BridgeTransactionQueue,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionQueueInsertOutcome>;
        pub fn transaction_queue_erase_plan(
            self: &mut BridgeTransactionQueue,
            hash: &[u8; 32],
        ) -> TransactionQueueErasePlan;
        pub fn transaction_queue_erase(self: &mut BridgeTransactionQueue, hash: &[u8; 32]) -> bool;
        pub fn transaction_queue_contains(self: &BridgeTransactionQueue, hash: &[u8; 32]) -> bool;
        pub fn transaction_queue_mark_transaction_known(
            self: &mut BridgeTransactionQueue,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_queue_is_transaction_known(
            self: &BridgeTransactionQueue,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_queue_transactions_dropped(self: &BridgeTransactionQueue) -> bool;
        pub fn transaction_queue_get_transaction(
            self: &BridgeTransactionQueue,
            hash: &[u8; 32],
        ) -> TransactionQueueStoredTransaction;
        pub fn transaction_queue_size(self: &BridgeTransactionQueue) -> usize;
        pub fn transaction_queue_ordered_hashes(
            self: &BridgeTransactionQueue,
            count: u64,
        ) -> Vec<TransactionQueueHash>;
        pub fn transaction_queue_ordered_transactions(
            self: &BridgeTransactionQueue,
            count: u64,
        ) -> Vec<TransactionQueueStoredTransaction>;
        pub fn transaction_queue_ordered_hashes_plan(
            self: &BridgeTransactionQueue,
            count: u64,
        ) -> TransactionQueueOrderedHashesPlan;
        pub fn transaction_queue_all_hash_groups(
            self: &BridgeTransactionQueue,
        ) -> Vec<TransactionQueueHashGroup>;
        pub fn transaction_queue_all_transaction_groups(
            self: &BridgeTransactionQueue,
        ) -> Vec<TransactionQueueTransactionGroup>;
        pub fn transaction_queue_block_finalized(
            self: &mut BridgeTransactionQueue,
            block_number: u64,
        ) -> Vec<TransactionQueueHash>;
        pub fn transaction_queue_block_finalized_plan(
            self: &mut BridgeTransactionQueue,
            block_number: u64,
        ) -> TransactionQueuePurgePlan;
        pub fn transaction_queue_proposable_accounts(
            self: &BridgeTransactionQueue,
        ) -> Vec<TransactionQueueAddress>;
        pub fn transaction_queue_purge_account(
            self: &mut BridgeTransactionQueue,
            sender: &[u8; 20],
            account_nonce: &[u8; 32],
        ) -> Vec<TransactionQueueHash>;
        pub fn transaction_queue_purge_account_plan(
            self: &mut BridgeTransactionQueue,
            sender: &[u8; 20],
            account_nonce: &[u8; 32],
        ) -> TransactionQueuePurgePlan;
        pub fn transaction_queue_purge_accounts_plan(
            self: &mut BridgeTransactionQueue,
            facts: Vec<TransactionQueueAccountNonceFact>,
        ) -> TransactionQueuePurgePlan;
        pub fn transaction_queue_non_proposable_over_limit(self: &BridgeTransactionQueue) -> bool;
        pub fn transaction_queue_min_gas_price_for_block_inclusion(
            self: &BridgeTransactionQueue,
            limit: u64,
        ) -> [u8; 32];
        pub fn transaction_queue_demote_to_non_proposable(
            self: &mut BridgeTransactionQueue,
            hash: &[u8; 32],
            last_block_number: u64,
        ) -> TransactionQueueDemotePlan;

        // Consensus gas pricer

        type BridgeGasPricer;

        pub fn create_gas_pricer(config: GasPricerConfig) -> Result<Box<BridgeGasPricer>>;
        pub fn gas_pricer_bid(self: &BridgeGasPricer) -> Result<[u8; 32]>;
        pub fn gas_pricer_bid_from_pool(
            self: &BridgeGasPricer,
            pool_price: &[u8; 32],
        ) -> Result<[u8; 32]>;
        pub fn gas_pricer_update(
            self: &BridgeGasPricer,
            gas_prices: Vec<GasPricerGasPrice>,
        ) -> Result<()>;
        pub fn gas_pricer_init_from_storage(
            self: &BridgeGasPricer,
            storage: &BridgeStorage,
        ) -> Result<()>;

        // Consensus slashing proof planner

        type BridgeSlashingProofPlanner;

        pub fn create_slashing_proof_planner(
            report_malicious_behaviour: bool,
        ) -> Result<Box<BridgeSlashingProofPlanner>>;
        pub fn slashing_plan_double_voting_proof(
            self: &BridgeSlashingProofPlanner,
            input: DoubleVotingProofInput,
        ) -> Result<DoubleVotingProofPlan>;
        pub fn slashing_mark_double_voting_proof_submission(
            self: &BridgeSlashingProofPlanner,
            proof_hash: &[u8; 32],
        ) -> Result<bool>;

        // Consensus transaction manager planning

        type BridgeTransactionPackPlanner;
        type BridgeTransactionManagerSidecar;
        type BridgeTransactionManagerRuntime;
        type BridgeTransactionManagerAdmissionExecution;

        pub fn create_transaction_pack_planner(
            weight_limit: u64,
            min_transaction_gas: u64,
        ) -> Result<Box<BridgeTransactionPackPlanner>>;
        pub fn transaction_pack_max_candidate_count(self: &BridgeTransactionPackPlanner) -> u64;
        pub fn transaction_pack_consider_candidate(
            self: &BridgeTransactionPackPlanner,
            input: TransactionPackCandidateInput,
        ) -> Result<TransactionPackCandidateDecision>;
        pub fn transaction_pack_record_estimate(
            self: &mut BridgeTransactionPackPlanner,
            input: TransactionPackEstimateInput,
        ) -> Result<TransactionPackEstimateOutcome>;
        pub fn create_transaction_manager_sidecar(
            initial_transaction_count: u64,
        ) -> Box<BridgeTransactionManagerSidecar>;
        pub fn create_transaction_manager_runtime(
            initial_transaction_count: u64,
            config: TransactionQueueConfig,
        ) -> Box<BridgeTransactionManagerRuntime>;
        pub fn transaction_manager_runtime_pack_begin(
            self: &mut BridgeTransactionManagerRuntime,
            weight_limit: u64,
            min_transaction_gas: u64,
        ) -> Result<()>;
        pub fn transaction_manager_runtime_pack_next_candidate(
            self: &mut BridgeTransactionManagerRuntime,
        ) -> Result<TransactionPackSessionCandidate>;
        pub fn transaction_manager_runtime_pack_record_estimate(
            self: &mut BridgeTransactionManagerRuntime,
            input: TransactionPackSessionEstimateInput,
        ) -> Result<TransactionPackEstimateOutcome>;
        pub fn transaction_manager_runtime_pack_finalize(
            self: &mut BridgeTransactionManagerRuntime,
        ) -> Result<TransactionPackSessionOutcome>;
        pub fn transaction_manager_runtime_transaction_count(
            self: &BridgeTransactionManagerRuntime,
        ) -> u64;
        pub fn transaction_manager_runtime_is_transaction_known(
            self: &BridgeTransactionManagerRuntime,
            fact: TransactionManagerSidecarKnownFact,
        ) -> Result<bool>;
        pub fn transaction_manager_runtime_insert_non_finalized(
            self: &mut BridgeTransactionManagerRuntime,
            input: TransactionManagerSidecarInsertInput,
        ) -> Result<()>;
        pub fn transaction_manager_runtime_contains_non_finalized(
            self: &BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_manager_runtime_contains_recently_finalized(
            self: &BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_manager_runtime_lookup_ordered_payloads(
            self: &BridgeTransactionManagerRuntime,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<TransactionManagerSidecarLookupPlan>;
        pub fn transaction_manager_runtime_non_finalized_size(
            self: &BridgeTransactionManagerRuntime,
        ) -> usize;
        pub fn transaction_manager_runtime_remove_non_finalized(
            self: &mut BridgeTransactionManagerRuntime,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<u64>;
        pub fn transaction_manager_runtime_apply_finalized_transition(
            self: &mut BridgeTransactionManagerRuntime,
            transition: TransactionManagerSidecarTransitionInput,
        ) -> Result<()>;
        pub fn transaction_manager_runtime_evict_stale_recently_finalized(
            self: &mut BridgeTransactionManagerRuntime,
            stale_period: u64,
        ) -> u64;
        pub fn transaction_manager_runtime_insert_recovery_entries(
            self: &mut BridgeTransactionManagerRuntime,
            entries: Vec<TransactionManagerSidecarRecoveryInsertInput>,
        ) -> Result<u64>;
        pub fn transaction_manager_runtime_queue_insert(
            self: &mut BridgeTransactionManagerRuntime,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionQueueInsertOutcome>;
        pub fn transaction_manager_runtime_insert_validated_transaction(
            self: &mut BridgeTransactionManagerRuntime,
            fact: TransactionManagerValidatedInsertSidecarFact,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionManagerRuntimeValidatedInsertOutcome>;
        pub fn transaction_manager_runtime_insert_transaction_precheck(
            self: &BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> Result<TransactionManagerInsertTransactionOutcome>;
        pub fn transaction_manager_runtime_finish_insert_transaction(
            self: &BridgeTransactionManagerRuntime,
            fact: TransactionManagerInsertTransactionFact,
        ) -> Result<TransactionManagerInsertTransactionOutcome>;
        pub fn transaction_manager_runtime_execute_transaction_admission(
            self: &mut BridgeTransactionManagerRuntime,
            fact: TransactionManagerValidatedInsertSidecarFact,
            input: TransactionQueueInsertInput,
            has_finalized_period: bool,
            finalized_period: u64,
        ) -> Result<TransactionManagerRuntimeAdmissionOutcome>;
        pub fn transaction_manager_runtime_queue_erase(
            self: &mut BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_manager_runtime_queue_get_transaction(
            self: &BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> TransactionQueueStoredTransaction;
        pub fn transaction_manager_runtime_queue_ordered_transactions(
            self: &BridgeTransactionManagerRuntime,
            count: u64,
        ) -> Vec<TransactionQueueStoredTransaction>;
        pub fn transaction_manager_runtime_queue_all_transaction_groups(
            self: &BridgeTransactionManagerRuntime,
        ) -> Vec<TransactionQueueTransactionGroup>;
        pub fn transaction_manager_runtime_queue_contains(
            self: &BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_manager_runtime_queue_size(
            self: &BridgeTransactionManagerRuntime,
        ) -> usize;
        pub fn transaction_manager_runtime_queue_block_finalized(
            self: &mut BridgeTransactionManagerRuntime,
            block_number: u64,
        ) -> Vec<TransactionQueueHash>;
        pub fn transaction_manager_runtime_queue_proposable_accounts(
            self: &BridgeTransactionManagerRuntime,
        ) -> Vec<TransactionQueueAddress>;
        pub fn transaction_manager_runtime_queue_purge_accounts_plan(
            self: &mut BridgeTransactionManagerRuntime,
            facts: Vec<TransactionQueueAccountNonceFact>,
        ) -> TransactionQueuePurgePlan;
        pub fn transaction_manager_runtime_queue_cleanup(
            self: &mut BridgeTransactionManagerRuntime,
            apply_block_finalized: bool,
            block_number: u64,
            facts: Vec<TransactionQueueAccountNonceFact>,
        ) -> TransactionManagerRuntimeQueueCleanupPlan;
        pub fn transaction_manager_runtime_queue_mark_transaction_known(
            self: &mut BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_manager_runtime_queue_transactions_dropped(
            self: &BridgeTransactionManagerRuntime,
        ) -> bool;
        pub fn transaction_manager_runtime_queue_non_proposable_over_limit(
            self: &BridgeTransactionManagerRuntime,
        ) -> bool;
        pub fn transaction_manager_runtime_queue_min_gas_price_for_block_inclusion(
            self: &BridgeTransactionManagerRuntime,
            limit: u64,
        ) -> [u8; 32];
        pub fn transaction_manager_runtime_queue_demote_to_non_proposable(
            self: &mut BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
            last_block_number: u64,
        ) -> TransactionQueueDemotePlan;
        pub fn transaction_manager_sidecar_transaction_count(
            self: &BridgeTransactionManagerSidecar,
        ) -> u64;
        pub fn transaction_manager_sidecar_is_transaction_known(
            self: &BridgeTransactionManagerSidecar,
            fact: TransactionManagerSidecarKnownFact,
        ) -> Result<bool>;
        pub fn transaction_manager_sidecar_insert_non_finalized(
            self: &mut BridgeTransactionManagerSidecar,
            input: TransactionManagerSidecarInsertInput,
        ) -> Result<()>;
        pub fn transaction_manager_sidecar_contains_non_finalized(
            self: &BridgeTransactionManagerSidecar,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_manager_sidecar_contains_recently_finalized(
            self: &BridgeTransactionManagerSidecar,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_manager_sidecar_non_finalized_size(
            self: &BridgeTransactionManagerSidecar,
        ) -> usize;
        pub fn transaction_manager_sidecar_lookup_ordered_payloads(
            self: &BridgeTransactionManagerSidecar,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<TransactionManagerSidecarLookupPlan>;
        pub fn transaction_manager_sidecar_remove_non_finalized(
            self: &mut BridgeTransactionManagerSidecar,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<u64>;
        pub fn transaction_manager_sidecar_apply_finalized_transition(
            self: &mut BridgeTransactionManagerSidecar,
            transition: TransactionManagerSidecarTransitionInput,
        ) -> Result<()>;
        pub fn transaction_manager_sidecar_evict_stale_recently_finalized(
            self: &mut BridgeTransactionManagerSidecar,
            stale_period: u64,
        ) -> u64;
        pub fn transaction_manager_sidecar_insert_recovery_entries(
            self: &mut BridgeTransactionManagerSidecar,
            entries: Vec<TransactionManagerSidecarRecoveryInsertInput>,
        ) -> Result<u64>;
        pub fn save_transactions_from_dag_block_with_sidecar(
            sidecar: &mut BridgeTransactionManagerSidecar,
            storage: &BridgeStorage,
            facts: Vec<DagTransactionSaveSidecarFact>,
        ) -> Result<DagTransactionSaveOutcome>;
        pub fn save_transactions_from_dag_block_with_runtime(
            runtime: &mut BridgeTransactionManagerRuntime,
            storage: &BridgeStorage,
            facts: Vec<DagTransactionSaveSidecarFact>,
        ) -> Result<DagTransactionSaveOutcome>;
        /// Executes runtime admission planning and returns an explicit commit script.
        pub fn transaction_manager_runtime_execute_admission(
            runtime: &BridgeTransactionManagerRuntime,
            storage: &BridgeStorage,
            facts: Vec<DagTransactionSaveSidecarFact>,
        ) -> Result<Box<BridgeTransactionManagerAdmissionExecution>>;
        /// Commits one runtime admission script with storage-first ordering.
        pub fn transaction_manager_runtime_commit_admission(
            runtime: &mut BridgeTransactionManagerRuntime,
            storage: &BridgeStorage,
            execution: Box<BridgeTransactionManagerAdmissionExecution>,
        ) -> Result<DagTransactionSaveOutcome>;
        pub fn save_transactions_from_dag_block(
            storage: &BridgeStorage,
            current_transaction_count: u64,
            facts: Vec<DagTransactionSaveFact>,
        ) -> Result<DagTransactionSaveOutcome>;
        pub fn update_finalized_transactions_status_with_sidecar(
            sidecar: &mut BridgeTransactionManagerSidecar,
            storage: &BridgeStorage,
            period: u64,
            retention_window: u64,
            facts: Vec<FinalizedTransactionStatusSidecarFact>,
        ) -> Result<FinalizedTransactionStatusPlan>;
        pub fn update_finalized_transactions_status_with_runtime(
            runtime: &mut BridgeTransactionManagerRuntime,
            storage: &BridgeStorage,
            period: u64,
            retention_window: u64,
            facts: Vec<FinalizedTransactionStatusSidecarFact>,
        ) -> Result<FinalizedTransactionStatusPlan>;
        pub fn update_finalized_transactions_status(
            storage: &BridgeStorage,
            period: u64,
            retention_window: u64,
            current_transaction_count: u64,
            facts: Vec<FinalizedTransactionStatusFact>,
        ) -> Result<FinalizedTransactionStatusPlan>;
        /// Builds deterministic TransactionManager::verifyTransaction admission plan.
        pub fn transaction_manager_verify_transaction(
            fact: TransactionManagerVerifyTransactionFact,
        ) -> Result<TransactionManagerVerifyTransactionOutcome>;
        /// Builds deterministic TransactionManager::insertTransaction admission plan.
        pub fn transaction_manager_insert_transaction(
            fact: TransactionManagerInsertTransactionFact,
        ) -> Result<TransactionManagerInsertTransactionOutcome>;
        /// Builds deterministic TransactionManager::insertTransaction plan using Rust sidecars.
        pub fn transaction_manager_insert_transaction_with_sidecar(
            sidecar: &BridgeTransactionManagerSidecar,
            fact: TransactionManagerInsertTransactionFact,
        ) -> Result<TransactionManagerInsertTransactionOutcome>;
        /// Builds deterministic TransactionManager::insertTransaction plan using Rust runtime state.
        pub fn transaction_manager_insert_transaction_with_runtime(
            runtime: &BridgeTransactionManagerRuntime,
            fact: TransactionManagerInsertTransactionFact,
        ) -> Result<TransactionManagerInsertTransactionOutcome>;
        /// Builds deterministic TransactionManager::insertValidatedTransaction plan.
        pub fn transaction_manager_plan_validated_insert(
            fact: TransactionManagerValidatedInsertFact,
        ) -> Result<TransactionManagerValidatedInsertPlan>;
        /// Builds deterministic TransactionManager::insertValidatedTransaction plan using Rust sidecars.
        pub fn transaction_manager_plan_validated_insert_with_sidecar(
            sidecar: &BridgeTransactionManagerSidecar,
            fact: TransactionManagerValidatedInsertSidecarFact,
        ) -> Result<TransactionManagerValidatedInsertPlan>;
        /// Builds deterministic TransactionManager::insertValidatedTransaction plan using Rust runtime state.
        pub fn transaction_manager_plan_validated_insert_with_runtime(
            runtime: &BridgeTransactionManagerRuntime,
            fact: TransactionManagerValidatedInsertSidecarFact,
        ) -> Result<TransactionManagerValidatedInsertPlan>;
        /// Determines which hash inputs are not finalized in-memory and in storage.
        pub fn transaction_manager_filter_non_finalized(
            storage: &BridgeStorage,
            facts: Vec<TransactionManagerFinalizedFilterFact>,
        ) -> Result<FinalizedTransactionFilterPlan>;
        /// Determines which hash inputs are not finalized using Rust-owned sidecars and storage.
        pub fn transaction_manager_filter_non_finalized_with_sidecar(
            sidecar: &BridgeTransactionManagerSidecar,
            storage: &BridgeStorage,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<FinalizedTransactionFilterPlan>;
        pub fn transaction_manager_filter_non_finalized_with_runtime(
            runtime: &BridgeTransactionManagerRuntime,
            storage: &BridgeStorage,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<FinalizedTransactionFilterPlan>;
        /// Verifies a transaction sequence has no finalized entries.
        pub fn transaction_manager_verify_not_finalized(
            storage: &BridgeStorage,
            facts: Vec<TransactionManagerVerifyNotFinalizedFact>,
        ) -> Result<TransactionManagerVerifyNotFinalizedOutcome>;
        /// Verifies a transaction sequence has no finalized entries using Rust-owned sidecars.
        pub fn transaction_manager_verify_not_finalized_with_sidecar(
            sidecar: &BridgeTransactionManagerSidecar,
            storage: &BridgeStorage,
            facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
        ) -> Result<TransactionManagerVerifyNotFinalizedOutcome>;
        pub fn transaction_manager_verify_not_finalized_with_runtime(
            runtime: &BridgeTransactionManagerRuntime,
            storage: &BridgeStorage,
            facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
        ) -> Result<TransactionManagerVerifyNotFinalizedOutcome>;
        /// Resolves transaction hashes through TransactionManager storage rules.
        pub fn transaction_manager_load_stored_transactions(
            storage: &BridgeStorage,
            requests: Vec<TransactionManagerStoredTransactionRequest>,
        ) -> Result<Vec<TransactionManagerStoredTransactionLookup>>;
        /// Returns persisted non-finalized transaction payloads for TransactionManager recovery.
        pub fn transaction_manager_load_nonfinalized_recovery(
            storage: &BridgeStorage,
        ) -> Result<Vec<TransactionManagerRecoveryEntry>>;

        // Consensus verified votes

        type BridgeVerifiedVotes;

        pub fn create_verified_votes_index() -> Box<BridgeVerifiedVotes>;
        pub fn verified_votes_size(self: &BridgeVerifiedVotes) -> u64;
        pub fn verified_votes_check_unique_voter(
            self: &BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
        ) -> Result<UniqueVoterCheckOutcome>;
        pub fn verified_votes_insert_unique_voter(
            self: &mut BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
        ) -> Result<UniqueVoterInsertOutcome>;
        pub fn verified_votes_insert_voted_value(
            self: &mut BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
        ) -> Result<VotedValueInsertOutcome>;
        pub fn verified_votes_insert_vote_atomic(
            self: &mut BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
        ) -> Result<AtomicVoteInsertOutcome>;
        pub fn verified_votes_apply_threshold_decision(
            self: &mut BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
            total_weight: u64,
            two_t_plus_one_threshold: u64,
        ) -> Result<ThresholdDecisionOutcome>;
        pub fn verified_votes_vote_in_verified_map(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
            step: u64,
            block_hash: &[u8; 32],
            vote_hash: &[u8; 32],
        ) -> bool;
        pub fn verified_votes_set_network_t_plus_one_step(
            self: &mut BridgeVerifiedVotes,
            period: u64,
            round: u64,
            step: u64,
        ) -> bool;
        pub fn verified_votes_get_network_t_plus_one_step(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
        ) -> NetworkTPlusOneStepLookup;
        pub fn verified_votes_determine_new_round(
            self: &BridgeVerifiedVotes,
            period: u64,
            current_round: u64,
        ) -> DetermineNewRoundOutcome;
        pub fn verified_votes_insert_two_t_plus_one_voted_block(
            self: &mut BridgeVerifiedVotes,
            period: u64,
            round: u64,
            kind: u8,
            block_hash: &[u8; 32],
            step: u64,
        ) -> Result<TwoTPlusOneInsertOutcome>;
        pub fn verified_votes_get_two_t_plus_one_voted_block(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
            kind: u8,
        ) -> Result<TwoTPlusOneVotedBlockLookup>;
        pub fn verified_votes_get_two_t_plus_one_voted_block_votes(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
            kind: u8,
        ) -> Result<TwoTPlusOneVotesLookup>;
        pub fn verified_votes_cleanup_votes_by_period(
            self: &mut BridgeVerifiedVotes,
            pbft_period: u64,
        );
        pub fn verified_votes_snapshot_votes(
            self: &BridgeVerifiedVotes,
        ) -> Vec<VerifiedVotePayload>;
        pub fn verified_votes_snapshot_two_t_plus_one(
            self: &BridgeVerifiedVotes,
        ) -> Vec<TwoTPlusOneSnapshotEntry>;
        pub fn verified_votes_snapshot_round_markers(
            self: &BridgeVerifiedVotes,
        ) -> Vec<RoundMarkerSnapshot>;

        // Consensus pillar votes

        type BridgePillarVotes;

        pub fn create_pillar_votes_index() -> Box<BridgePillarVotes>;
        pub fn pillar_votes_period_data_initialized(self: &BridgePillarVotes, period: u64) -> bool;
        pub fn pillar_votes_init_period_data(
            self: &mut BridgePillarVotes,
            period: u64,
            threshold: u64,
        ) -> bool;
        pub fn pillar_votes_vote_exists(
            self: &BridgePillarVotes,
            vote: PillarVotePayload,
        ) -> Result<bool>;
        pub fn pillar_vote_inspect(vote_rlp: &[u8]) -> Result<PillarVoteInspection>;
        pub fn pillar_votes_is_unique_identity(
            self: &BridgePillarVotes,
            vote: PillarVoteIdentityPayload,
        ) -> Result<PillarVoteUniqueOutcome>;
        pub fn pillar_votes_is_unique_vote(
            self: &BridgePillarVotes,
            vote: PillarVotePayload,
        ) -> Result<PillarVoteUniqueOutcome>;
        pub fn pillar_votes_insert_vote(
            self: &mut BridgePillarVotes,
            vote: PillarVotePayload,
        ) -> Result<PillarVoteInsertOutcome>;
        pub fn pillar_votes_get_verified_votes(
            self: &BridgePillarVotes,
            period: u64,
            block_hash: &[u8; 32],
            above_threshold: bool,
        ) -> PillarVotesLookup;
        pub fn pillar_votes_cleanup_votes_by_period(self: &mut BridgePillarVotes, min_period: u64);
        pub fn pillar_votes_snapshot_refs(self: &BridgePillarVotes) -> Vec<PillarVoteRef>;

        pub fn plan_pillar_vote_bundle(
            facts: Vec<PillarVoteBundleFact>,
            expected_period: u64,
            expected_block_hash: &[u8; 32],
            threshold: u64,
        ) -> Result<PillarVoteBundlePlan>;

        /// Evaluates one pillar-vote relevance query.
        pub fn plan_pillar_vote_relevance(
            fact: PillarVoteRelevanceFact,
        ) -> Result<PillarVoteRelevancePlan>;

        // Consensus sortition

        type BridgeSortitionParamsManager;

        pub fn create_sortition_params_manager(
            config: SortitionRuntimeConfig,
            params_changes: Vec<SortitionParamsChangePayload>,
        ) -> Result<Box<BridgeSortitionParamsManager>>;
        pub fn sortition_current_params(
            self: &BridgeSortitionParamsManager,
        ) -> SortitionRuntimeParams;
        pub fn sortition_params_for_period(
            self: &BridgeSortitionParamsManager,
            found: bool,
            change: SortitionParamsChangePayload,
        ) -> SortitionRuntimeParams;
        pub fn sortition_restore_finalized_period(
            self: &mut BridgeSortitionParamsManager,
            has_pivot: bool,
            unique_transactions: u64,
            total_dag_transaction_refs: u64,
        ) -> Result<()>;
        pub fn sortition_record_finalized_period(
            self: &mut BridgeSortitionParamsManager,
            period: u64,
            has_pivot: bool,
            unique_transactions: u64,
            total_dag_transaction_refs: u64,
            non_empty_pbft_chain_size: u64,
        ) -> Result<SortitionParamsChangeResult>;
        pub fn sortition_average_dag_efficiency(self: &BridgeSortitionParamsManager)
            -> Result<u16>;
        pub fn sortition_params_changes(
            self: &BridgeSortitionParamsManager,
        ) -> Vec<SortitionParamsChangePayload>;
        pub fn sortition_calculate_dag_efficiency(
            self: &BridgeSortitionParamsManager,
            unique_transactions: u64,
            total_dag_transaction_refs: u64,
        ) -> SortitionEfficiencyResult;

        // Storage

        type BridgeStorage;

        pub fn create_storage(path: &str) -> Result<Box<BridgeStorage>>;
        pub fn create_write_batch(self: &BridgeStorage) -> Result<u64>;
        pub fn batch_put(
            self: &BridgeStorage,
            batch_id: u64,
            column: u8,
            key: Vec<u8>,
            value: Vec<u8>,
        ) -> Result<()>;
        pub fn batch_delete(
            self: &BridgeStorage,
            batch_id: u64,
            column: u8,
            key: Vec<u8>,
        ) -> Result<()>;
        pub fn commit_write_batch(self: &BridgeStorage, batch_id: u64, sync: bool) -> Result<()>;
        pub fn drop_write_batch(self: &BridgeStorage, batch_id: u64) -> Result<()>;

        pub fn dag_block_in_db(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_dag_block(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_dag_block_period(self: &BridgeStorage, hash: &[u8; 32]) -> Result<BlockPeriod>;
        pub fn get_dag_block_period_lookup(
            self: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<BlockPeriodLookup>;
        pub fn get_last_blocks_level(self: &BridgeStorage) -> Result<u64>;
        pub fn get_blocks_by_level(self: &BridgeStorage, level: u64) -> Result<Vec<u8>>;
        pub fn get_dag_blocks_at_level(
            self: &BridgeStorage,
            level: u64,
            number_of_levels: u32,
        ) -> Result<Vec<BlockRlp>>;
        pub fn get_nonfinalized_dag_blocks(self: &BridgeStorage) -> Result<Vec<LevelBlocks>>;
        pub fn get_proposal_period_for_dag_level(
            self: &BridgeStorage,
            level: u64,
        ) -> Result<PeriodLookup>;
        pub fn save_dag_block(
            self: &BridgeStorage,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn update_dag_block_counter(
            self: &BridgeStorage,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
        ) -> Result<()>;
        pub fn remove_dag_block(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn save_proposal_period_dag_levels_map(
            self: &BridgeStorage,
            level: u64,
            period: u64,
        ) -> Result<()>;
        pub fn save_dag_block_period(
            self: &BridgeStorage,
            hash: &[u8; 32],
            period: u64,
            position: u32,
        ) -> Result<()>;

        pub fn get_period_data_raw(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_period_from_pbft_hash(
            self: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<PeriodLookup>;
        pub fn get_block_receipt(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_final_chain_meta_value(self: &BridgeStorage, key: u32) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_header(
            self: &BridgeStorage,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_hash_by_number(
            self: &BridgeStorage,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_number_by_hash(
            self: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_log_blooms_chunk(
            self: &BridgeStorage,
            chunk_id: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_receipt_by_trx_hash(
            self: &BridgeStorage,
            trx_hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn save_period_data(
            self: &BridgeStorage,
            period: u64,
            period_data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_pbft_block_period(
            self: &BridgeStorage,
            hash: &[u8; 32],
            period: u64,
        ) -> Result<()>;
        pub fn get_pillar_block(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_latest_pillar_block(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn get_own_pillar_block_vote(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn get_current_pillar_block_data(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn save_pillar_block(
            self: &BridgeStorage,
            period: u64,
            pillar_block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_own_pillar_block_vote(self: &BridgeStorage, vote_rlp: Vec<u8>) -> Result<()>;
        pub fn save_current_pillar_block_data(
            self: &BridgeStorage,
            data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn get_genesis_hash(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn set_genesis_hash(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn get_last_sortition_params(self: &BridgeStorage, count: u64)
            -> Result<Vec<BlockRlp>>;
        pub fn get_params_change_for_period(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_status_field(self: &BridgeStorage, field: u8) -> Result<u64>;
        pub fn save_status_field(self: &BridgeStorage, field: u8, value: u64) -> Result<()>;
        pub fn save_sortition_params_change(
            self: &BridgeStorage,
            period: u64,
            params_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn get_period_lambda(
            self: &BridgeStorage,
            period: u64,
            find_closest: bool,
        ) -> Result<PeriodLambda>;
        pub fn save_period_lambda(
            self: &BridgeStorage,
            period: u64,
            period_lambda: u32,
        ) -> Result<()>;
        pub fn get_rounds_count_dynamic_lambda(self: &BridgeStorage) -> Result<u32>;
        pub fn save_rounds_count_dynamic_lambda(
            self: &BridgeStorage,
            rounds_count: u32,
        ) -> Result<()>;
        pub fn get_blocks_rewards_stats(self: &BridgeStorage) -> Result<Vec<PeriodRlp>>;
        pub fn save_block_rewards_stats(
            self: &BridgeStorage,
            period: u64,
            stats_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn clear_block_rewards_stats(self: &BridgeStorage) -> Result<()>;

        pub fn pbft_block_in_db(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_pbft_mgr_field(self: &BridgeStorage, field: u8) -> Result<u32>;
        pub fn get_pbft_mgr_status(self: &BridgeStorage, field: u8) -> Result<bool>;
        pub fn get_cert_voted_block_in_round(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn get_proposed_pbft_blocks(self: &BridgeStorage) -> Result<Vec<BlockRlp>>;
        pub fn get_pbft_head(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_own_verified_votes(self: &BridgeStorage) -> Result<Vec<VoteRlp>>;
        pub fn get_all_two_t_plus_one_votes(self: &BridgeStorage) -> Result<Vec<VoteRlp>>;
        pub fn get_reward_votes(self: &BridgeStorage) -> Result<Vec<VoteRlp>>;
        pub fn save_cert_voted_block_in_round(
            self: &BridgeStorage,
            round: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_proposed_pbft_block(
            self: &BridgeStorage,
            hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_pbft_mgr_field(self: &BridgeStorage, field: u8, value: u32) -> Result<()>;
        pub fn save_pbft_mgr_status(self: &BridgeStorage, field: u8, value: bool) -> Result<()>;
        pub fn save_pbft_head(self: &BridgeStorage, hash: &[u8; 32], head: Vec<u8>) -> Result<()>;
        pub fn save_own_verified_vote(
            self: &BridgeStorage,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn remove_cert_voted_block_in_round(self: &BridgeStorage) -> Result<()>;
        pub fn remove_proposed_pbft_block(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn remove_own_verified_vote(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn remove_extra_reward_vote(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn replace_two_t_plus_one_votes(
            self: &BridgeStorage,
            vote_type: u8,
            votes_bundle_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_extra_reward_vote(
            self: &BridgeStorage,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
        ) -> Result<()>;

        pub fn transaction_in_db(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn transaction_finalized(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_transaction_location(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_transaction(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_transaction_by_period_position(
            self: &BridgeStorage,
            period: u64,
            position: u32,
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(self: &BridgeStorage, period: u64) -> Result<u64>;
        pub fn get_system_transaction(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_all_nonfinalized_transactions(self: &BridgeStorage) -> Result<Vec<TxRlp>>;
        /// Batch-fetches transaction RLP payloads by hash from Rust storage.
        pub fn get_transaction_rlps_by_hashes(
            self: &BridgeStorage,
            hashes: Vec<DagTransactionHash>,
        ) -> Result<Vec<DagTransactionRlpLookup>>;
        pub fn get_all_transaction_period(self: &BridgeStorage) -> Result<Vec<HashPeriod>>;
        pub fn get_period_system_transactions_hashes(
            self: &BridgeStorage,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn save_transaction(
            self: &BridgeStorage,
            hash: &[u8; 32],
            trx_rlp: Vec<u8>,
        ) -> Result<()>;
        /// Persists TransactionManager-accepted non-finalized transactions in one
        /// storage batch and writes the manager-owned `StatusDbField::TrxCount`.
        pub fn save_non_finalized_transactions(
            self: &BridgeStorage,
            transactions: Vec<NonFinalizedTransactionPayload>,
            transaction_count: u64,
        ) -> Result<()>;
        pub fn remove_transaction(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn save_transaction_location(
            self: &BridgeStorage,
            hash: &[u8; 32],
            period: u64,
            position: u32,
            is_system: bool,
        ) -> Result<()>;
        pub fn save_system_transaction(
            self: &BridgeStorage,
            hash: &[u8; 32],
            trx_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_period_system_transactions_hashes(
            self: &BridgeStorage,
            period: u64,
            hashes_rlp: Vec<u8>,
        ) -> Result<()>;

        // FinalChain

        type BridgeFinalChain;

        pub fn create_final_chain(
            storage: &BridgeStorage,
            block_gas_limit: u64,
            genesis_timestamp: u64,
            genesis_accounts: Vec<GenesisAccount>,
            genesis_validators: Vec<GenesisValidator>,
            genesis_dpos_config: GenesisDposConfig,
        ) -> Result<Box<BridgeFinalChain>>;

        pub fn get_last_block_number(self: &BridgeFinalChain) -> Result<u64>;
        pub fn get_block_number(
            self: &BridgeFinalChain,
            hash: &[u8; 32],
        ) -> Result<FinalChainBlockNumberLookup>;
        pub fn get_block_hash(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>>;
        pub fn get_block_header(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>>;
        pub fn get_transaction_location(
            self: &BridgeFinalChain,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(self: &BridgeFinalChain, period: u64) -> Result<u64>;
        pub fn get_account(self: &BridgeFinalChain, address: &[u8; 20]) -> Result<AccountLookup>;
        pub fn get_dpos_eligible_vote_count(
            self: &BridgeFinalChain,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<u64>;
        pub fn get_dpos_eligible_total_vote_count(
            self: &BridgeFinalChain,
            block_number: u64,
        ) -> Result<u64>;
        pub fn get_dpos_is_eligible(
            self: &BridgeFinalChain,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<bool>;
        pub fn get_dag_dpos_authorization_facts(
            self: &BridgeFinalChain,
            block_number: u64,
            sender: &[u8; 20],
        ) -> Result<DagDposAuthorizationFacts>;
        pub fn get_dpos_validators_total_stakes(
            self: &BridgeFinalChain,
            block_number: u64,
        ) -> Result<Vec<DposValidatorStake>>;
        pub fn get_dpos_validators_eligible_vote_counts(
            self: &BridgeFinalChain,
            block_number: u64,
        ) -> Result<Vec<DposValidatorVoteCount>>;
        pub fn get_vrf_key(self: &BridgeFinalChain, address: &[u8; 20]) -> Result<Vec<u8>>;
        pub fn estimate_call_gas(self: &BridgeFinalChain, gas_limit: u64) -> Result<u64>;
        pub fn call(
            self: &BridgeFinalChain,
            request: FinalChainCall,
        ) -> Result<FinalChainCallOutcome>;
        pub fn finalize_block(
            self: &BridgeFinalChain,
            pbft_block_rlp: Vec<u8>,
            transactions: Vec<FinalizationTransaction>,
            finalized_dag_blocks: Vec<FinalizationDagBlock>,
        ) -> Result<FinalizationOutcome>;
        pub fn get_transaction_rlps(self: &BridgeFinalChain, period: u64) -> Result<Vec<TxRlp>>;
        pub fn get_transaction_receipt(
            self: &BridgeFinalChain,
            period: u64,
            position: u64,
        ) -> Result<Vec<u8>>;
    }
}
