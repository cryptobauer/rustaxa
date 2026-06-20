use crate::dag::*;
use crate::final_chain::*;
use crate::gas_pricer::*;
use crate::pbft_chain::*;
use crate::pbft_finalize::*;
use crate::pbft_manager::*;
use crate::pbft_reward_votes::*;
use crate::pbft_sync::*;
use crate::pbft_vote_admission::*;
use crate::pbft_vote_event::*;
use crate::pbft_vote_generation::*;
use crate::pbft_vote_ingress::*;
use crate::pbft_vote_payload::*;
use crate::pbft_vote_pipeline::*;
use crate::pbft_vote_progress::*;
use crate::pbft_vote_validation::*;
use crate::period_data_queue::*;
use crate::pillar_chain::*;
use crate::pillar_votes::*;
use crate::proposed_blocks::*;
use crate::rewards_stats::*;
use crate::slashing::*;
use crate::sortition::*;
use crate::storage::*;
use crate::transaction::*;
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
use rustaxa_consensus::FinalChain;
use rustaxa_consensus::PbftVoteAdmissionRuntime;
use rustaxa_consensus::PillarVotes;
use rustaxa_consensus::RewardsStatsRuntime;
use rustaxa_storage::Storage;
use rustaxa_storage::StorageWriteBatch;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

pub struct BridgeStorage(pub Arc<Storage>);

/// Typed PBFT vote-list query handle for C++ compatibility materializers.
///
/// This wrapper keeps durable vote-list reads grouped under the PBFT storage
/// boundary instead of exposing them as generic `BridgeStorage` methods.
pub struct BridgePbftVoteStorageQueries {
    pub storage: Arc<Storage>,
}

/// Typed PBFT scalar/head query handle for C++ compatibility materializers.
///
/// This wrapper keeps PBFT manager scalar reads, PBFT block existence checks,
/// and PBFT head payload reads grouped under the PBFT storage boundary instead
/// of exposing them as generic `BridgeStorage` methods.
pub struct BridgePbftStorageQueries {
    pub storage: Arc<Storage>,
}

/// Typed metadata/rewards query handle for C++ compatibility materializers.
///
/// This wrapper keeps metadata, status, lambda, sortition, genesis, and block
/// rewards reads grouped under the metadata storage boundary instead of exposing
/// them as generic `BridgeStorage` methods.
pub struct BridgeMetadataStorageQueries {
    pub storage: Arc<Storage>,
}

/// Typed DAG query handle for C++ compatibility materializers.
///
/// This wrapper keeps DAG block, index, period, and proposal-period reads
/// grouped under the DAG storage boundary instead of exposing them as generic
/// `BridgeStorage` methods.
pub struct BridgeDagStorageQueries {
    pub storage: Arc<Storage>,
}

/// Typed transaction query handle for C++ compatibility materializers.
///
/// This wrapper keeps transaction public-read compatibility grouped under the
/// transaction storage boundary instead of exposing those reads as generic
/// `BridgeStorage` methods.
pub struct BridgeTransactionStorageQueries {
    pub storage: Arc<Storage>,
}

/// Typed FinalChain lookup query handle for C++ compatibility materializers.
///
/// This wrapper keeps FinalChain read queries grouped under the FinalChain storage
/// boundary instead of exposing those reads as generic `BridgeStorage` methods.
pub struct BridgeFinalChainStorageQueries {
    pub storage: Arc<Storage>,
}

/// Typed period query handle for C++ compatibility materializers.
///
/// This wrapper keeps period rows grouped under period storage instead of
/// exposing those reads as generic `BridgeStorage` methods.
pub struct BridgePeriodStorageQueries {
    pub storage: Arc<Storage>,
}

/// Rust-owned storage shim batch used to preserve the legacy C++ `Batch&` API
/// while keeping the live write batch inside `rustaxa-storage`.
///
/// C++ shims may stage raw legacy column writes through this object only while
/// public `DbStorage` compatibility methods are being retired. The batch is
/// consumed on commit and silently dropped when the C++ compatibility batch is
/// abandoned.
pub struct BridgeStorageBatch {
    pub storage: Arc<Storage>,
    pub batch: Option<StorageWriteBatch>,
}

pub struct BridgeFinalChain(pub FinalChain);

pub struct BridgeFinalChainExecutionSession {
    pub state: rustaxa_consensus::FinalChainExecutionSession,
}

pub struct BridgeGasPricer(pub Mutex<GasPriceOracle>, pub Option<Arc<Storage>>);

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

pub struct BridgeDagVerifyBlockSession {
    pub state: crate::dag::DagVerifyBlockSession,
}

pub struct BridgeDagProposerSession {
    pub state: crate::dag::DagProposerSession,
}

/// PBFT chain runtime wrapper. Pure state-only instances are used by unit tests
/// and deterministic head transitions; storage-backed instances own the shared
/// Rust storage handle used for PBFT block lookup/materialization.
pub struct BridgePbftChain {
    pub state: PbftChain,
    pub storage: Option<Arc<Storage>>,
    pub initialized_default: bool,
}

pub struct BridgeProposedBlocks {
    pub index: ProposedBlocks,
    pub storage: Option<Arc<Storage>>,
}

/// Rewards-stat runtime wrapper coupling deterministic in-memory state with
/// the shared Rust storage handle used for cache reload, write, and clear
/// operations.
pub struct BridgeRewardsStatsRuntime {
    pub state: RewardsStatsRuntime,
    pub storage: Arc<Storage>,
}

pub struct BridgePbftFinalizationRuntimeSession {
    pub state: rustaxa_consensus::pbft_finalize::PbftFinalizationRuntimeState,
}

pub struct BridgePbftManagerRuntimeSession {
    pub state: rustaxa_consensus::pbft_manager::PbftManagerRuntimeSession,
}

pub struct BridgePbftManagerStateActionEffectSession {
    pub state: rustaxa_consensus::pbft_manager::PbftManagerStateActionEffectSession,
}

/// Pillar-chain storage wrapper used by the C++ manager shim.
///
/// The wrapper owns a cloned Rust storage handle so production pillar-chain
/// reads and writes do not retain or pass the generic `BridgeStorage` facade
/// after construction.
pub struct BridgePillarChainStorage {
    pub storage: Arc<Storage>,
}

pub struct BridgePbftManagerBlockValidationSession {
    pub state: rustaxa_consensus::pbft_manager::PbftManagerBlockValidationSession,
}

pub struct BridgePbftManagerProposalSession {
    pub state: rustaxa_consensus::pbft_manager::PbftManagerProposalSession,
}

/// Long-lived Rust PBFT manager runtime used by the C++ compatibility shim.
///
/// Purpose:
/// - Owns the scalar PBFT manager state machine together with the Rust storage
///   handle required for restart-safe cursor/status persistence.
///
/// Inputs/outputs:
/// - Constructed from a `BridgeStorage` handle during PBFT manager startup.
/// - Consumed by runtime transition APIs that persist through
///   `rustaxa-storage` without requiring C++ to pass storage back for each
///   operation.
///
/// Invariants and edge behavior:
/// - `storage` is the authoritative durable store for PBFT manager fields and
///   statuses while `state` is updated only after Rust storage commits succeed.
/// - C++ callers must update live compatibility mirrors only from snapshots
///   returned by this runtime.
pub struct BridgePbftManagerRuntime {
    pub state: rustaxa_consensus::pbft_manager::PbftManagerRuntime,
    pub storage: Arc<Storage>,
}

pub struct BridgePbftVotePipelineSession {
    pub state: rustaxa_consensus::PbftVotePipelineSession,
    pub context: rustaxa_ffi::PbftVoteProgressContext,
}

pub struct BridgePbftVoteAdmissionSession {
    pub state: rustaxa_consensus::PbftVoteAdmissionSession,
    pub context: rustaxa_ffi::PbftVoteProgressContext,
}

pub struct BridgeSlashingProofPlanner(pub Mutex<SlashingProofPlanner>);

pub struct BridgePeriodDataQueue(pub PeriodDataQueue);

/// Rust-owned verified-votes runtime used by the C++ VoteManager shim.
///
/// Production instances attach a cloned Rust storage handle so VoteManager
/// persistence for own votes, vote-progress bundles, and reward-vote reset
/// finalization stages does not retain or pass the generic `BridgeStorage`
/// facade after construction. Storage-free instances remain for compatibility
/// tests that exercise only in-memory vote admission behavior.
pub struct BridgeVerifiedVotes {
    pub runtime: PbftVoteAdmissionRuntime,
    pub storage: Option<Arc<Storage>>,
}

/// Compatibility runtime for older PBFT vote validation bridge tests.
///
/// Production Rust-mode `VoteManager` routing uses `BridgeVerifiedVotes`, whose
/// `PbftVoteAdmissionRuntime` owns validation replay protection, threshold
/// caching, verified-vote metadata, and retained vote payloads together.
pub struct BridgePbftVoteValidationRuntime {
    pub replay_cache: Mutex<rustaxa_consensus::PbftVoteReplayCache>,
    pub threshold_runtime: Mutex<rustaxa_consensus::PbftTwoTPlusOneThresholdRuntime>,
}

pub struct BridgePillarVotes(pub PillarVotes);

/// Bridge wrapper for the Rust sortition parameter manager.
///
/// The manager owns deterministic threshold/runtime state. Production
/// Rust-mode constructors attach native Rust storage inside
/// `SortitionParamsManager`; compatibility constructors may remain storage-free
/// and pass explicit facts for unit-level planner tests.
pub struct BridgeSortitionParamsManager {
    pub manager: SortitionParamsManager,
}

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
/// legacy `Transaction` objects at the C++ API boundary. Production instances
/// also own a cloned Rust storage handle so C++ does not retain or pass the
/// generic `BridgeStorage` facade for transaction-manager storage operations.
pub struct BridgeTransactionManagerRuntime {
    pub sidecar: TransactionManagerSidecar,
    pub queue: TransactionQueue,
    pub storage: Option<Arc<Storage>>,
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

/// Runtime-owned state for one TransactionManager proposal-packing pass.
///
/// The session owns the ordered queue candidate snapshot, planner accounting,
/// selected output ordering, and demotion summary. C++ remains responsible only
/// for materializing the current candidate and supplying FinalChain/EVM gas
/// estimates back to the runtime.
pub struct TransactionManagerRuntimePackSession {
    pub planner: TransactionPackingPlanner,
    pub proposal_period: u64,
    pub estimate_gas_limit: u64,
    pub last_block_number: u64,
    pub total_shards: u16,
    pub node_shard: u16,
    pub shard_period_interval: u64,
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

    /// Optional canonical hash lookup result.
    ///
    /// `found = false` means the backing storage row was absent. Decode and
    /// backend failures are returned as bridge errors instead.
    struct HashLookup {
        found: bool,
        hash: [u8; 32],
    }

    struct PeriodLambda {
        found: bool,
        value: u32,
    }

    struct PeriodRlp {
        period: u64,
        data: Vec<u8>,
    }

    /// Generic 32-byte hash fact used by rewards-stat bridge payloads.
    struct RewardsHash {
        hash: [u8; 32],
    }

    /// Hardfork and committee configuration for Rust rewards-stat planning.
    struct RewardsStatsConfig {
        committee_size: u32,
        magnolia_period: u64,
        aspen_part_one_period: u64,
    }

    /// Rewards distribution frequency rule active from `from_period` onward.
    struct RewardsFrequencyRule {
        from_period: u64,
        frequency: u32,
    }

    /// Finalized transaction fee fact for one PBFT period.
    struct RewardsTransactionFact {
        hash: [u8; 32],
        gas_price_be: Vec<u8>,
        gas_used: u64,
    }

    /// Finalized DAG block fact for rewards-stat planning.
    struct RewardsDagBlockFact {
        author: [u8; 20],
        difficulty: u16,
        transaction_hashes: Vec<RewardsHash>,
    }

    /// Previous-block cert-vote fact for rewards-stat planning.
    struct RewardsCertVoteFact {
        voter: [u8; 20],
        weight: u64,
        period: u64,
    }

    /// C++-originated fact bundle for one finalized PBFT period.
    struct RewardsStatsProcessFact {
        period: u64,
        block_author: [u8; 20],
        blocks_per_year: u32,
        dpos_eligible_total_vote_count: u64,
        transactions: Vec<RewardsTransactionFact>,
        dag_blocks: Vec<RewardsDagBlockFact>,
        cert_votes: Vec<RewardsCertVoteFact>,
    }

    /// Result from Rust rewards-stat processing.
    ///
    /// Status values:
    /// - `0` - applied
    /// - `1` - rejected
    struct RewardsStatsProcessResult {
        status: u8,
        error_code: String,
        current_period: u64,
        cache_current_period: bool,
        clear_cached_stats: bool,
        current_block_stats_rlp: Vec<u8>,
        distribution_stats: Vec<PeriodRlp>,
    }

    /// Result from appending rewards-stat cache writes to a Rust storage batch.
    struct RewardsStatsApplyResult {
        status: u8,
        current_period: u64,
        wrote_current_period: bool,
        cleared_cached_stats: bool,
        error_code: String,
    }

    struct TxRlp {
        data: Vec<u8>,
    }

    /// Rust-inspected legacy transaction facts.
    ///
    /// `sender_found == false` means regular signature recovery failed. System
    /// transaction inspection always returns the fixed Taraxa system sender.
    struct LegacyTransactionInspection {
        hash: [u8; 32],
        sender_found: bool,
        sender: [u8; 20],
        signature_valid: bool,
        nonce: [u8; 32],
        gas_price: [u8; 32],
        gas_limit: u64,
        receiver_found: bool,
        receiver: [u8; 20],
        value: [u8; 32],
        data: Vec<u8>,
        data_size: usize,
        chain_id: u64,
        intrinsic_gas_covered: bool,
        cost: [u8; 32],
        tx_rlp: Vec<u8>,
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

    /// Gas-estimation request supplied before C++ may call FinalChain/EVM.
    struct TransactionManagerGasEstimationFact {
        hash: [u8; 32],
        declared_gas: u64,
        proposal_period: u64,
        estimate_gas_limit: u64,
    }

    /// Rust-owned cache/orchestration plan for one gas-estimation request.
    struct TransactionManagerGasEstimationPlan {
        use_declared_gas: bool,
        cache_hit: bool,
        requires_evm_call: bool,
        gas_used: u64,
        result_rlp: Vec<u8>,
    }

    /// Opaque C++ gas-estimation result to retain in the Rust runtime cache.
    struct TransactionManagerGasEstimationResult {
        hash: [u8; 32],
        proposal_period: u64,
        gas_used: u64,
        result_rlp: Vec<u8>,
    }

    /// One candidate returned by a Rust-owned runtime packing session.
    struct TransactionPackSessionCandidate {
        found: bool,
        hash: [u8; 32],
        declared_gas: u64,
        sender: [u8; 20],
        nonce: [u8; 32],
        gas_price: [u8; 32],
        gas: u64,
        receiver_found: bool,
        receiver: [u8; 20],
        value: [u8; 32],
        data: Vec<u8>,
    }

    /// C++ gas-estimation fact supplied for the active runtime packing candidate.
    struct TransactionPackSessionEstimateInput {
        hash: [u8; 32],
        gas_used: u64,
        last_block_number: u64,
        result_rlp: Vec<u8>,
    }

    /// One executor step while Rust drives the packTrxs session loop.
    ///
    /// `request_estimate` is true when C++ should estimate `candidate`.
    /// `request_estimate` is false when the session is complete and
    /// `selected_transactions` carries final output.
    struct TransactionPackSessionStep {
        request_estimate: bool,
        candidate: TransactionPackSessionCandidate,
        selected_transactions: Vec<TransactionPackSelectedTransaction>,
        demoted_hashes: Vec<TransactionQueueHash>,
        stopped: bool,
    }

    /// One transaction accepted by a Rust-owned runtime packing session.
    struct TransactionPackSelectedTransaction {
        hash: [u8; 32],
        gas_used: u64,
        tx_rlp: Vec<u8>,
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

    /// PBFT vote payload crossing the CXX bridge for storage persistence.
    ///
    /// `hash` is the RocksDB key and `vote_rlp` is the weighted
    /// `PbftVote::rlp(true, true)` payload. Rust storage treats the bytes as
    /// canonical storage bytes and does not materialize C++ vote objects.
    struct PbftVoteStorageRecord {
        hash: [u8; 32],
        vote_rlp: Vec<u8>,
    }

    /// Lookup result for a retained Rust PBFT vote payload.
    ///
    /// When `found` is true, `vote` carries the weighted
    /// `PbftVote::rlp(true, true)` bytes retained by the Rust admission
    /// runtime. C++ may temporarily materialize a `PbftVote` sidecar from this
    /// payload, but Rust remains the source of truth for payload retention.
    struct PbftVotePayloadLookup {
        found: bool,
        vote: PbftVoteStorageRecord,
    }

    /// Per-family optimized PBFT vote-bundle egress plan.
    ///
    /// Status values are stable bridge codes:
    /// 0 = ready, 1 = not found, 2 = empty request, 3 = unsupported kind,
    /// 4 = mapping mismatch, 5 = requested hash is not in the 2t+1 plan,
    /// 6 = requested hashes are not in plan order, 7 = missing retained
    /// payload, 8 = payload decode error, 9 = payload metadata mismatch.
    struct PbftOptimizedVoteBundlePlan {
        found: bool,
        status: u8,
        error_code: String,
        kind: u8,
        block_hash: [u8; 32],
        period: u64,
        round: u64,
        step: u64,
        vote_hashes: Vec<PbftFinalizationHash>,
    }

    /// Combined get-next response plan for the previous PBFT round.
    ///
    /// Rust owns the next/next-null vote-hash selection from retained
    /// verified-vote metadata. C++ owns peer-known filtering, chunking, tarcap
    /// packet wrapping, and network send policy.
    struct PbftNextVotesBundleEgressPlan {
        status: u8,
        error_code: String,
        period: u64,
        round: u64,
        next_votes: PbftOptimizedVoteBundlePlan,
        next_null_votes: PbftOptimizedVoteBundlePlan,
    }

    /// Request to build one peer-filtered optimized PBFT votes bundle.
    ///
    /// `vote_hashes` must be a non-empty ordered subsequence of the Rust plan
    /// for `kind`; C++ uses this to filter already-known votes and split large
    /// bundles without materializing `PbftVote` objects.
    struct PbftOptimizedVoteBundleBuildRequest {
        kind: u8,
        block_hash: [u8; 32],
        period: u64,
        round: u64,
        step: u64,
        vote_hashes: Vec<PbftFinalizationHash>,
    }

    /// Result of building one optimized PBFT votes bundle.
    ///
    /// On status 0, `votes_bundle_rlp` is the inner
    /// `OptimizedPbftVotesBundle` RLP payload and `vote_hashes` echoes the
    /// included hashes in send order. It is not the tarcap packet wrapper.
    struct PbftOptimizedVoteBundleBuildResult {
        status: u8,
        error_code: String,
        vote_hashes: Vec<PbftFinalizationHash>,
        votes_bundle_rlp: Vec<u8>,
    }

    /// Latest-round 2t+1 vote bundle crossing the CXX bridge for storage persistence.
    ///
    /// `kind` matches C++ `TwoTPlusOneVotedBlockType` discriminants:
    /// soft = 0, cert = 1, next = 2, and next-null = 3. The metadata fields
    /// describe the live VoteManager facts that selected the bundle; the DB key
    /// remains only `kind` to preserve legacy latest-round semantics.
    struct PbftTwoTPlusOneVoteBundle {
        kind: u8,
        period: u64,
        round: u64,
        step: u64,
        block_hash: [u8; 32],
        votes_bundle_rlp: Vec<u8>,
    }

    /// Operation-level VoteManager persistence request for one accepted vote.
    ///
    /// The bridge applies both optional writes through a single Rust storage
    /// batch so replacing a 2t+1 bundle is delete-plus-put atomic.
    struct PbftVoteProgressPersistenceWrite {
        has_extra_reward_vote: bool,
        extra_reward_vote: PbftVoteStorageRecord,
        has_two_t_plus_one_bundle: bool,
        two_t_plus_one_bundle: PbftTwoTPlusOneVoteBundle,
    }

    /// Result for VoteManager PBFT vote persistence bridge operations.
    ///
    /// `status` values are local to the bridge contract: 0 = applied,
    /// 1 = rejected. `applied_writes` counts logical vote-family writes
    /// accepted into the Rust-owned batch or direct operation.
    struct PbftVotePersistenceResult {
        status: u8,
        applied_writes: u64,
        error_code: String,
    }

    struct PbftChainHeadPayload {
        head_hash: [u8; 32],
        size: u64,
        non_empty_size: u64,
        last_pbft_block_hash: [u8; 32],
        last_non_null_anchor_hash: [u8; 32],
    }

    struct PbftChainStorageRestore {
        head: PbftChainHeadPayload,
        initialized_default: bool,
    }

    struct PbftBlockStorageLookup {
        found: bool,
        block_rlp: Vec<u8>,
    }

    /// Warning carried from side-effect-free PBFT sync admission planning.
    struct PbftSyncTransactionWarning {
        hash: [u8; 32],
        kind: u8,
    }

    /// Transaction hash wrapper for CXX bridge vectors.
    struct PbftSyncTransactionHash {
        hash: [u8; 32],
    }

    /// C++-originated PBFT sync transaction references.
    struct PbftSyncTransactionQueryFact {
        dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
    }

    /// Rust-planned finalized transaction lookup work for PBFT sync admission.
    struct PbftSyncTransactionQueryPlan {
        finalized_lookup_hashes: Vec<PbftSyncTransactionHash>,
    }

    /// C++-originated PBFT sync admission fact.
    struct PbftSyncPeriodAdmissionFact {
        block_period: u64,
        block_prev_hash: [u8; 32],
        chain_last_hash: [u8; 32],
        chain_last_period: u64,
        block_in_chain: bool,
        final_chain_hash_status: u8,
        reward_votes_status: u8,
        cert_votes_status: u8,
        missing_transaction_hashes: Vec<PbftSyncTransactionHash>,
        finalized_transaction_hashes: Vec<PbftSyncTransactionHash>,
        contains_finalized_transactions: bool,
        pillar_data_status: u8,
        pillar_votes_status: u8,
    }

    /// Plan outcome for one PBFT sync admission decision.
    struct PbftSyncPeriodAdmissionPlan {
        decision: u8,
        status: u8,
        clear_sync_queue: bool,
        report_malicious_peer: bool,
        wait_for_finalization: bool,
        accept_period_data: bool,
        warnings: Vec<PbftSyncTransactionWarning>,
        contains_finalized_transaction_warning: bool,
    }

    /// Combined PBFT sync runtime decision and transaction lookup plan.
    struct PbftSyncRuntimePlan {
        action: u8,
        period_admission_plan: PbftSyncPeriodAdmissionPlan,
        transaction_query_plan: PbftSyncTransactionQueryPlan,
    }

    /// Storage-backed PBFT sync egress payload for packet materialization.
    struct PbftSyncEgressPayload {
        period_data_rlp: Vec<u8>,
        attach_reward_votes: bool,
    }

    /// C++-originated fact bundle for staged PBFT sync runtime planning.
    struct PbftSyncProcessPeriodDataRuntimeFact {
        block_period: u64,
        block_prev_hash: [u8; 32],
        chain_last_hash: [u8; 32],
        chain_last_period: u64,
        block_in_chain: bool,
        final_chain_hash_status: u8,
        reward_votes_status: u8,
        cert_votes_status: u8,
        transactions_status: u8,
        dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
        missing_transaction_hashes: Vec<PbftSyncTransactionHash>,
        finalized_transaction_hashes: Vec<PbftSyncTransactionHash>,
        contains_finalized_transactions: bool,
        pillar_data_status: u8,
        extra_data_required: bool,
        extra_data_present: bool,
        extra_data_pillar_block_hash_present: bool,
        pillar_votes_required: bool,
        pillar_votes_present: bool,
        pillar_votes_status: u8,
        previous_cert_votes_present: bool,
        previous_cert_first_vote_has_weight: bool,
    }

    /// Staged PBFT sync runtime action for C++ `processPeriodData` execution.
    struct PbftSyncProcessPeriodDataRuntimePlan {
        runtime_action: u8,
        status: u8,
        next_check: u8,
        clear_sync_queue: bool,
        report_malicious_peer: bool,
        wait_for_finalization: bool,
        accept_period_data: bool,
        retry_same_candidate: bool,
        replace_previous_block_cert_votes: bool,
        transaction_query_plan: PbftSyncTransactionQueryPlan,
        warnings: Vec<PbftSyncTransactionWarning>,
        contains_finalized_transaction_warning: bool,
    }

    /// Rust-owned outer drain step for C++ `pushSyncedPbftBlocksIntoChain`.
    struct PbftSyncQueueDrainStep {
        action: u8,
        status: u8,
        clean_before_period: u64,
        can_continue: bool,
        error_code: String,
    }

    /// C++ executor report for one PBFT sync queue-drain step.
    struct PbftSyncQueueDrainReport {
        action: u8,
        success: bool,
        accepted_period_data: bool,
    }

    /// Rust validation result for one queue-drain executor report.
    struct PbftSyncQueueDrainReportResult {
        status: u8,
        can_continue: bool,
        error_code: String,
    }

    /// C++-originated fact bundle for deterministic PBFT finalization intent planning.
    struct PbftFinalizationHash {
        hash: [u8; 32],
    }

    /// Hash plus finalized-position metadata for native-ready storage indexes.
    struct PbftFinalizationPositionedHash {
        hash: [u8; 32],
        position: u32,
    }

    /// C++-originated fact bundle for deterministic PBFT finalization intent planning.
    struct PbftFinalizationIntentFact {
        block_hash: [u8; 32],
        pbft_head_hash: [u8; 32],
        block_period: u64,
        block_prev_hash: [u8; 32],
        chain_last_hash: [u8; 32],
        chain_last_period: u64,
        block_in_chain: bool,
        pivot_dag_anchor_hash: [u8; 32],
        has_pillar_block: bool,
        pillar_block_finalized: bool,
        request_dynamic_lambda_update: bool,
        cert_vote_count: u64,
        sample_cert_vote_block_hash: [u8; 32],
        sample_cert_vote_period: u64,
        sample_cert_vote_round: u64,
        sample_cert_vote_step: u64,
        block_lambda: u32,
        last_saved_period_lambda_found: bool,
        last_saved_period_lambda: u32,
        dynamic_blocks_per_year: u32,
        dpos_blocks_per_year: u32,
        pbft_head_payload: Vec<u8>,
        period_data_rlp: Vec<u8>,
        ordered_dag_block_hashes: Vec<PbftFinalizationHash>,
        ordered_transaction_hashes: Vec<PbftFinalizationHash>,
        process_pillar_block_after_advance: bool,
    }

    /// Rust preflight fact for pillar finalization before PBFT finalization intent bytes are built.
    struct PbftFinalizationPillarPreflightFact {
        pbft_block_hash: [u8; 32],
        block_period: u64,
        block_in_chain: bool,
        pillar_finalization_required: bool,
        has_pillar_block_hash: bool,
        pillar_block_hash: [u8; 32],
        pillar_block_finalized: bool,
    }

    /// Rust-owned pillar preflight plan for the C++ executor.
    struct PbftFinalizationPillarPreflightPlan {
        pbft_block_hash: [u8; 32],
        block_period: u64,
        pillar_block_hash: [u8; 32],
        action: u8,
        finalize_pillar_block: bool,
        accepted: bool,
        status: u8,
        error_code: String,
    }

    /// C++ report for one Rust-planned pillar preflight action.
    struct PbftFinalizationPillarPreflightReport {
        action: u8,
        success: bool,
        status: u8,
        error_code: String,
        block_period: u64,
        pbft_block_hash: [u8; 32],
        pillar_block_hash: [u8; 32],
        pillar_vote_count: u64,
    }

    /// Rust-planned cleanup flags for the PBFT finalization side-effect sequence.
    struct PbftFinalizationCleanupPlan {
        persist_pbft_block_metadata: bool,
        reset_reward_votes: bool,
        set_dag_block_order: bool,
        update_sortition_params: bool,
        update_finalized_transactions_status: bool,
        update_pbft_chain: bool,
        clear_anchor_dag_cache: bool,
        finalize_final_chain: bool,
        maybe_update_dynamic_lambda: bool,
        advance_period: bool,
        process_pillar_block: bool,
    }

    /// Rust-planned storage-write flags for PBFT finalization persistence planning.
    struct PbftFinalizationStorageWritePlan {
        persist_pbft_head: bool,
        persist_period_data: bool,
        reset_reward_votes: bool,
        update_sortition_params: bool,
        apply_dynamic_lambda_update: bool,
        persist_period_lambda: bool,
        persist_executed_pbft_status: bool,
        process_pillar_block: bool,
        pbft_block_hash: [u8; 32],
        pbft_head_hash: [u8; 32],
        block_period: u64,
        null_anchor: bool,
        anchor_hash: [u8; 32],
        reward_vote_period: u64,
        reward_vote_round: u64,
        reward_vote_step: u64,
        reward_vote_block_hash: [u8; 32],
        period_lambda: u32,
        blocks_per_year: u32,
        executed_pbft_status: bool,
        pbft_head_payload: Vec<u8>,
        period_data_rlp: Vec<u8>,
        dag_block_period_writes: Vec<PbftFinalizationPositionedHash>,
        transaction_location_writes: Vec<PbftFinalizationPositionedHash>,
    }

    struct PbftFinalizationStorageWriteStage {
        stage: u8,
        rounds_count_dynamic_lambda: u32,
        dynamic_lambda: u32,
        has_sortition_params_change: bool,
        sortition_params_change_period: u64,
        sortition_params_change_interval_efficiency: u16,
        sortition_params_change_threshold_upper: u16,
        has_reward_votes_reset: bool,
        reward_votes_bundle_rlp: Vec<u8>,
        extra_reward_vote_hashes: Vec<PbftFinalizationHash>,
    }

    /// Cacti dynamic-lambda configuration for Rust PBFT finalization planning.
    struct PbftDynamicLambdaConfig {
        cacti_block_num: u64,
        lambda_min: u32,
        lambda_max: u32,
        lambda_default: u32,
        lambda_change_interval: u32,
        lambda_change: u32,
        consensus_delay: u32,
        dpos_blocks_per_year: u32,
    }

    /// Dynamic-lambda fact bundle for one PBFT finalization.
    struct PbftDynamicLambdaFact {
        dynamic_lambda_active: bool,
        finalized_period: u64,
        finalized_round: u64,
        pre_adjust_rounds_count_dynamic_lambda: u32,
        pre_adjust_dynamic_lambda: u32,
        config: PbftDynamicLambdaConfig,
    }

    /// Rust-planned dynamic-lambda state for one PBFT finalization.
    struct PbftDynamicLambdaPlan {
        apply_dynamic_lambda_update: bool,
        period_lambda: u32,
        blocks_per_year: u32,
        rounds_count_dynamic_lambda: u32,
        dynamic_lambda: u32,
        decreased_dynamic_lambda: bool,
        increased_dynamic_lambda: bool,
        status: u8,
        error_code: String,
    }

    /// Ordered runtime-side actions for PBFT finalization.
    struct PbftFinalizationRuntimePlan {
        finalize_block: bool,
        status: u8,
        actions: Vec<u8>,
        error_code: String,
    }

    /// C++-originated facts for one Rust-owned PBFT manager daemon tick.
    struct PbftManagerRuntimeTickFact {
        tick_id: u64,
        state: u8,
        period: u64,
        round: u64,
        step: u64,
        network_available: bool,
        network_pbft_syncing: bool,
        has_eligible_wallet: bool,
    }

    /// Configuration and current-period facts needed for Rust-owned PBFT
    /// manager startup restore from storage.
    struct PbftManagerStartupFact {
        current_period: u64,
        cacti_active_at_chain_size: bool,
        genesis_lambda_ms: u64,
        cacti_lambda_max_ms: u64,
        cacti_lambda_default_ms: u64,
    }

    /// Rust-owned storage facts for replaying one finalized period during PBFT
    /// manager startup.
    struct PbftManagerStartupReplayPeriod {
        found: bool,
        period_data_rlp: Vec<u8>,
        finalized_dag_hashes: Vec<PbftFinalizationHash>,
        has_period_lambda: bool,
        period_lambda: u32,
    }

    /// Live facts for Rust-owned PBFT manager startup replay range planning.
    struct PbftManagerStartupReplayRangeFact {
        final_chain_last_block: u64,
        pbft_chain_size: u64,
        delegation_delay: u64,
        recently_finalized_factor: u64,
    }

    /// Rust-owned startup replay range plan for C++ executor loops.
    struct PbftManagerStartupReplayRangePlan {
        accepted: bool,
        has_finalization_range: bool,
        finalization_from_period: u64,
        finalization_to_period: u64,
        recent_from_period: u64,
        recent_to_period: u64,
        error_code: String,
    }

    /// Rust-owned PBFT manager period-advance effect plan.
    struct PbftManagerAdvancePeriodPlan {
        accepted: bool,
        finalized_chain_size: u64,
        new_period: u64,
        actions: Vec<u8>,
        error_code: String,
    }

    /// Rust-owned PBFT manager cursor snapshot used by the transitional C++
    /// shim to mirror state after startup or transition commits.
    struct PbftManagerRuntimeSnapshot {
        status: u8,
        state: u8,
        period: u64,
        round: u64,
        step: u64,
        current_round_lambda_ms: u64,
        next_step_time_ms: u64,
        rounds_count_dynamic_lambda: u32,
        dynamic_lambda_ms: u32,
        executed_pbft_block: bool,
        already_next_voted_value: bool,
        already_next_voted_null: bool,
        broadcast_votes_counter: u32,
        rebroadcast_votes_counter: u32,
        broadcast_reward_votes_counter: u32,
        rebroadcast_reward_votes_counter: u32,
        has_cert_voted_block: bool,
        cert_voted_block_period: u64,
        cert_voted_block_round: u64,
        cert_voted_block_hash: [u8; 32],
        persist_normalized_step: bool,
        reset_second_finish_start: bool,
        error_code: String,
    }

    /// One Rust-owned PBFT manager runtime-session step.
    struct PbftManagerRuntimeSessionStep {
        status: u8,
        cursor: u32,
        action: u8,
        has_action: bool,
        complete: bool,
        restart_loop: bool,
        can_continue: bool,
        has_target_round: bool,
        target_round: u64,
        tick_id: u64,
        error_code: String,
    }

    /// Structured PBFT manager action report from C++.
    struct PbftManagerRuntimeActionReport {
        cursor: u32,
        action: u8,
        success: bool,
        result: u8,
        go_finish_state: bool,
        loop_back_finish_state: bool,
        has_eligible_wallet: bool,
        has_new_round: bool,
        new_round: u64,
        error_code: String,
    }

    /// C++-originated deterministic facts for one PBFT manager state action.
    struct PbftManagerStateActionFact {
        state: u8,
        period: u64,
        round: u64,
        step: u64,
        elapsed_round_ms: u64,
        deadline_ms: u64,
        current_round_lambda_ms: u64,
        polling_interval_ms: u64,
        has_previous_round_next_null: bool,
        has_previous_round_next_value: bool,
        previous_round_next_value_hash: [u8; 32],
        has_current_round_soft_value: bool,
        current_round_soft_value_hash: [u8; 32],
        has_cert_voted_block: bool,
        cert_voted_block_hash: [u8; 32],
        already_next_voted_value: bool,
        already_next_voted_null: bool,
    }

    /// Side-effect-free PBFT manager state-action plan for C++ execution.
    struct PbftManagerStateActionPlan {
        status: u8,
        primary_intent: u8,
        primary_hash: [u8; 32],
        secondary_intent: u8,
        secondary_hash: [u8; 32],
        go_finish_state: bool,
        loop_back_finish_state: bool,
        error_code: String,
    }

    /// One ordered PBFT manager state-action effect for C++ execution.
    struct PbftManagerStateActionEffect {
        intent: u8,
        hash: [u8; 32],
    }

    /// Ordered PBFT manager state-action effects planned by Rust.
    struct PbftManagerStateActionEffectPlan {
        status: u8,
        effects: Vec<PbftManagerStateActionEffect>,
        go_finish_state: bool,
        loop_back_finish_state: bool,
        error_code: String,
    }

    /// Report for one C++-executed PBFT manager state-action effect.
    struct PbftManagerStateActionEffectReport {
        cursor: u32,
        intent: u8,
        result: u8,
        error_code: String,
    }

    /// One cursor step from a Rust-owned state-action effect session.
    struct PbftManagerStateActionSessionStep {
        status: u8,
        cursor: u32,
        has_effect: bool,
        effect: PbftManagerStateActionEffect,
        go_finish_state: bool,
        loop_back_finish_state: bool,
        complete: bool,
        can_continue: bool,
        error_code: String,
    }

    /// One local proposer-wallet fact for Rust-owned proposal construction.
    struct PbftManagerProposalWalletFact {
        wallet_index: u64,
        dpos_eligible: bool,
        sortition_valid: bool,
    }

    /// One ordered DAG block gas fact for a Rust-requested proposal anchor.
    struct PbftManagerProposalDagBlockFact {
        hash: [u8; 32],
        gas_estimation: u64,
    }

    /// Initial fact bundle for Rust-owned PBFT proposal construction.
    struct PbftManagerProposalInitialFact {
        period: u64,
        round: u64,
        previous_pbft_block_hash: [u8; 32],
        last_period_dag_anchor_hash: [u8; 32],
        dag_genesis_hash: [u8; 32],
        dag_blocks_size: u64,
        ghost_path_move_back: u64,
        pbft_gas_limit: u64,
        extra_data_required: bool,
        extra_data_available: bool,
        final_chain_hash_valid: bool,
        final_chain_hash: [u8; 32],
        wallets: Vec<PbftManagerProposalWalletFact>,
        ghost_path: Vec<PbftFinalizationHash>,
        has_non_finalized_fallback: bool,
        non_finalized_fallback_hash: [u8; 32],
    }

    /// C++ report for one Rust-requested DAG order.
    struct PbftManagerProposalDagOrderReport {
        anchor_hash: [u8; 32],
        dag_blocks: Vec<PbftManagerProposalDagBlockFact>,
        order_available: bool,
    }

    /// One action or terminal command from a Rust-owned proposal session.
    struct PbftManagerProposalSessionStep {
        action: u8,
        status: u8,
        requested_anchor_hash: [u8; 32],
        previous_pbft_block_hash: [u8; 32],
        anchor_hash: [u8; 32],
        order_hash: [u8; 32],
        final_chain_hash: [u8; 32],
        eligible_wallet_indices: Vec<u64>,
        dag_blocks_included: u64,
        selected_null_anchor: bool,
        error_code: String,
    }

    /// Compact timing and counter facts for Rust-owned broadcast planning.
    struct PbftManagerBroadcastFact {
        round_elapsed_ms: u64,
        period_elapsed_ms: u64,
        current_round_lambda_ms: u64,
        broadcast_lambda_threshold: u32,
        rebroadcast_lambda_threshold: u32,
        broadcast_votes_counter: u32,
        rebroadcast_votes_counter: u32,
        broadcast_reward_votes_counter: u32,
        rebroadcast_reward_votes_counter: u32,
    }

    /// Rust-owned broadcast plan for C++ network execution.
    struct PbftManagerBroadcastPlan {
        status: u8,
        action: u8,
        rebroadcast: bool,
        next_broadcast_votes_counter: u32,
        next_rebroadcast_votes_counter: u32,
        next_broadcast_reward_votes_counter: u32,
        next_rebroadcast_reward_votes_counter: u32,
        error_code: String,
    }

    /// C++ report for one Rust-planned broadcast action.
    struct PbftManagerBroadcastReport {
        action: u8,
        rebroadcast: bool,
        success: bool,
        error_code: String,
    }

    /// Rust validation result for one broadcast report.
    struct PbftManagerBroadcastReportResult {
        status: u8,
        apply_counters: bool,
        broadcast_votes_counter: u32,
        rebroadcast_votes_counter: u32,
        broadcast_reward_votes_counter: u32,
        rebroadcast_reward_votes_counter: u32,
        error_code: String,
    }

    /// C++ live fact bundle for Rust-owned PBFT block-validation orchestration.
    struct PbftManagerBlockValidationFact {
        block_hash: [u8; 32],
        period: u64,
        pivot_hash: [u8; 32],
        pivot_is_null: bool,
        dag_order_cached: bool,
        dag_order_required: bool,
        pillar_block_required: bool,
        dag_weight_check_required: bool,
        pbft_chain_status: u8,
        final_chain_hash_status: u8,
        reward_votes_status: u8,
        extra_data_status: u8,
        pillar_block_status: u8,
        dag_order_status: u8,
        dag_weight_status: u8,
    }

    /// Next PBFT block-validation action requested by Rust.
    struct PbftManagerBlockValidationPlan {
        action: u8,
        status: u8,
        next_check: u8,
        error_code: String,
    }

    /// C++-originated facts for one Rust-owned proposed-block admission attempt.
    struct PbftManagerCandidateAdmissionFact {
        period: u64,
        block_hash: [u8; 32],
        lookup_performed: bool,
        proposed_block_found: bool,
        proposed_block_already_valid: bool,
        validation_status: u8,
    }

    /// Proposed-block admission plan for C++ lookup/validation execution.
    struct PbftManagerCandidateAdmissionPlan {
        action: u8,
        status: u8,
        mark_valid: bool,
        error_code: String,
    }

    /// C++ live lookup and validation facts for one PBFT proposal candidate.
    struct PbftManagerLeaderCandidateInputFact {
        vote_hash: [u8; 32],
        block_hash: [u8; 32],
        period: u64,
        credential: [u8; 64],
        voter_public_key: [u8; 64],
        weight_found: bool,
        weight: u64,
        block_in_chain: bool,
        proposed_block_found: bool,
        block_validation_status: u8,
        pivot_hash: [u8; 32],
    }

    /// Proposed block accepted by Rust candidate planning and ready to mark valid.
    struct PbftManagerLeaderValidBlockCommand {
        period: u64,
        block_hash: [u8; 32],
    }

    /// Grouped PBFT leader-candidate plan for C++ materialization.
    struct PbftManagerLeaderCandidatePlan {
        status: u8,
        selected: bool,
        selected_vote_hash: [u8; 32],
        selected_block_hash: [u8; 32],
        selected_period: u64,
        selected_from_null_anchor: bool,
        valid_blocks: Vec<PbftManagerLeaderValidBlockCommand>,
        error_code: String,
    }

    /// C++-originated facts for one Rust-owned PBFT manager transition.
    struct PbftManagerTransitionFact {
        kind: u8,
        period: u64,
        round: u64,
        step: u64,
        target_round: u64,
        current_round_lambda_ms: u64,
        target_round_lambda_ms: u64,
        default_lambda_ms: u64,
        max_exponential_lambda_ms: u64,
        max_steps: u64,
        network_next_voting_step: u64,
        deadline_ms: u64,
        polling_interval_ms: u64,
        next_step_time_ms: u64,
        cacti_hardfork: bool,
        has_cert_voted_block: bool,
        executed_pbft_block: bool,
    }

    /// Side-effect-free PBFT manager transition plan for C++ execution.
    struct PbftManagerTransitionPlan {
        status: u8,
        kind: u8,
        new_state: u8,
        new_round: u64,
        new_step: u64,
        current_round_lambda_ms: u64,
        next_step_time_ms: u64,
        persist_round: bool,
        persist_step: bool,
        reset_next_voted_statuses: bool,
        remove_cert_voted_block: bool,
        clear_own_votes: bool,
        clear_broadcasted_votes: bool,
        reset_broadcast_counters: bool,
        reset_executed_block_status: bool,
        set_vote_manager_period_round: bool,
        reset_current_round_start: bool,
        reset_second_finish_start: bool,
        print_cert_step_info: bool,
        print_second_finish_step_info: bool,
        error_code: String,
    }

    /// Result from applying PBFT manager transition storage through the
    /// long-lived Rust runtime handle.
    struct PbftManagerTransitionRuntimeApplyResult {
        status: u8,
        applied_writes: u64,
        snapshot: PbftManagerRuntimeSnapshot,
        error_code: String,
    }

    /// One Rust-owned PBFT finalization runtime-session step.
    struct PbftFinalizationRuntimeSessionStep {
        status: u8,
        cursor: u32,
        action: u8,
        has_action: bool,
        complete: bool,
        can_continue: bool,
        error_code: String,
    }

    /// Structured report for one PBFT finalization runtime action.
    struct PbftFinalizationRuntimeActionReport {
        cursor: u32,
        action: u8,
        success: bool,
        status: u8,
        error_code: String,
    }

    /// Post-action facts for Rust validation of live PBFT finalization mutations.
    struct PbftFinalizationLiveMutationReport {
        action: u8,
        block_period: u64,
        pbft_block_hash: [u8; 32],
        anchor_hash: [u8; 32],
        dag_finalized_count: u64,
        finalized_transaction_count: u64,
        pbft_chain_size: u64,
        pbft_chain_head_hash: [u8; 32],
        pbft_chain_last_anchor_hash: [u8; 32],
        reward_votes_period: u64,
        reward_votes_round: u64,
        reward_votes_block_hash: [u8; 32],
        reward_votes_extra_count: u64,
        sortition_changed: bool,
        sortition_change_period: u64,
        sortition_change_interval_efficiency: u16,
        sortition_change_threshold_upper: u16,
        sortition_current_threshold_upper: u16,
        sortition_params_changes_count: u64,
    }

    /// Result of validating a live PBFT finalization mutation report in Rust.
    struct PbftFinalizationLiveMutationValidation {
        accepted: bool,
        status: u8,
        action: u8,
        error_code: String,
    }

    /// Rust classification of durable PBFT finalization resume state.
    struct PbftFinalizationResumePlan {
        status: u8,
        duplicate_classified: bool,
        complete: bool,
        replay_actions: Vec<u8>,
        error_code: String,
    }

    /// Result from appending Rust-owned finalized-period storage writes to an existing batch.
    struct PbftFinalizedPeriodApplyResult {
        status: u8,
        wrote_pbft_head: bool,
        wrote_period_data: bool,
        dag_index_writes: usize,
        transaction_location_writes: usize,
        block_period: u64,
        pbft_block_hash: [u8; 32],
        error_code: String,
    }

    /// Bridge-safe PBFT finalization intent returned to the C++ shim.
    struct PbftFinalizationIntentPlan {
        finalize_block: bool,
        anchor: u8,
        executed_pbft_block: bool,
        status: u8,
        cleanup: PbftFinalizationCleanupPlan,
        storage_write_intent: PbftFinalizationStorageWritePlan,
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
        pivot_hash: [u8; 32],
        block_rlp: Vec<u8>,
    }

    struct ProposedBlockMetadataLookup {
        found: bool,
        is_valid: bool,
        pivot_hash: [u8; 32],
    }

    struct ProposedBlockPeriodHashes {
        period: u64,
        block_hashes: Vec<DagHash>,
    }

    struct ProposedBlockSnapshotEntry {
        period: u64,
        block_hash: [u8; 32],
        pivot_hash: [u8; 32],
        block_rlp: Vec<u8>,
        is_valid: bool,
    }

    /// Compact transaction identity retained by the Rust period-data queue for
    /// sync finalized-status checks.
    struct PeriodDataQueueTransactionIdentity {
        input_index: u64,
        hash: [u8; 32],
        transaction_nonce: [u8; 32],
        sender: [u8; 20],
    }

    /// Canonical pillar-vote payload retained by the Rust period-data queue
    /// for sync validation without reopening the live C++ `PeriodData`
    /// sidecar.
    struct PeriodDataQueuePillarVotePayload {
        vote_rlp: Vec<u8>,
    }

    /// Canonical PBFT cert-vote payload retained by the Rust period-data queue
    /// for sync validation and finalization without reopening live C++
    /// `PeriodData` vote sidecars as the payload source.
    struct PeriodDataQueuePbftVotePayload {
        vote_rlp: Vec<u8>,
    }

    /// Canonical transaction payload retained by the Rust period-data queue
    /// for finalization materialization without reopening the live C++
    /// `PeriodData` transaction list.
    struct PeriodDataQueueTransactionPayload {
        transaction_rlp: Vec<u8>,
    }

    struct PeriodDataQueueEntryRef {
        entry_id: u64,
        period: u64,
        block_hash: [u8; 32],
        prev_block_hash: [u8; 32],
        pivot_hash: [u8; 32],
        final_chain_hash: [u8; 32],
        reward_vote_hashes: Vec<PbftSyncTransactionHash>,
        pillar_vote_rlps: Vec<PeriodDataQueuePillarVotePayload>,
        transaction_rlps: Vec<PeriodDataQueueTransactionPayload>,
        previous_cert_vote_rlps: Vec<PeriodDataQueuePbftVotePayload>,
        dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
        previous_cert_votes_present: bool,
        previous_cert_first_vote_has_weight: bool,
        pillar_votes_present: bool,
        extra_data_present: bool,
        extra_data_pillar_block_hash_present: bool,
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
        entry_period: u64,
        block_hash: [u8; 32],
        prev_block_hash: [u8; 32],
        pivot_hash: [u8; 32],
        final_chain_hash: [u8; 32],
        reward_vote_hashes: Vec<PbftSyncTransactionHash>,
        pillar_vote_rlps: Vec<PeriodDataQueuePillarVotePayload>,
        transaction_rlps: Vec<PeriodDataQueueTransactionPayload>,
        cert_vote_rlps: Vec<PeriodDataQueuePbftVotePayload>,
        previous_cert_vote_rlps: Vec<PeriodDataQueuePbftVotePayload>,
        dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
        previous_cert_votes_present: bool,
        previous_cert_first_vote_has_weight: bool,
        pillar_votes_present: bool,
        extra_data_present: bool,
        extra_data_pillar_block_hash_present: bool,
        use_last_block_cert_votes: bool,
        next_entry_id: u64,
        current_period: u64,
        effective_size: usize,
    }

    struct PeriodDataQueueLastEntryLookup {
        found: bool,
        entry_id: u64,
        period: u64,
        block_hash: [u8; 32],
        prev_block_hash: [u8; 32],
        pivot_hash: [u8; 32],
        final_chain_hash: [u8; 32],
        reward_vote_hashes: Vec<PbftSyncTransactionHash>,
        pillar_vote_rlps: Vec<PeriodDataQueuePillarVotePayload>,
        transaction_rlps: Vec<PeriodDataQueueTransactionPayload>,
        previous_cert_vote_rlps: Vec<PeriodDataQueuePbftVotePayload>,
        dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
        previous_cert_votes_present: bool,
        previous_cert_first_vote_has_weight: bool,
        pillar_votes_present: bool,
        extra_data_present: bool,
        extra_data_pillar_block_hash_present: bool,
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

    /// Compact facts for one PBFT vote-progress planning pass.
    ///
    /// The vote payload carries canonical consensus identity and weight facts;
    /// booleans carry caller-supplied ingress or validation state. This payload
    /// intentionally does not own packet bytes, live `PbftVote` objects, or
    /// verified-vote state.
    struct PbftVoteProgressFact {
        vote: VerifiedVotePayload,
        vote_already_known: bool,
        carries_proposed_block: bool,
        valid_stale_reward_vote: bool,
    }

    /// Caller-supplied flags for deriving PBFT vote event facts from canonical bytes.
    struct PbftVoteEventFactFlags {
        vote_already_known: bool,
        carries_proposed_block: bool,
        valid_stale_reward_vote: bool,
    }

    /// Compact PBFT vote facts used by Rust-planned ingress gates.
    ///
    /// C++ still owns packet decoding and live vote materialization. This
    /// payload carries only the scalar fields needed to decide relevance,
    /// period/round/step windows, and bundle-shape policy.
    struct PbftVoteIngressFact {
        period: u64,
        round: u64,
        step: u64,
        vote_type: u8,
    }

    /// Local PBFT state and network-window policy for one vote ingress plan.
    ///
    /// Zero max-window values disable the matching upper-bound check, matching
    /// the legacy network DDoS-protection configuration semantics. The
    /// `can_request_*` booleans keep peer-timer throttling in C++ while letting
    /// Rust decide whether the vote shape is eligible to trigger a sync hint.
    struct PbftVoteIngressContext {
        current_period: u64,
        current_round: u64,
        current_step: u64,
        max_future_period_delta: u64,
        max_future_round_delta: u64,
        max_future_step_delta: u64,
        validate_max_round_step: bool,
        source_peer_is_voter: bool,
        can_request_pbft_sync: bool,
        can_request_next_votes_sync: bool,
    }

    /// Side-effect-free PBFT vote ingress plan.
    ///
    /// C++ executes returned sync hints and continues to admission only when
    /// `accepted` is true. `status` matches `PbftVoteIngressStatus::as_u8()` in
    /// `rustaxa-consensus`.
    struct PbftVoteIngressPlan {
        status: u8,
        error_code: String,
        accepted: bool,
        relevant: bool,
        request_pbft_sync: bool,
        request_next_votes_sync: bool,
        checking_round: u64,
        checking_step: u64,
    }

    /// Compact PBFT vote event facts derived from canonical vote bytes.
    ///
    /// Status values:
    /// - `0` - ready
    /// - `1` - malformed RLP
    /// - `2` - invalid signature
    /// - `3` - invalid zero weight
    /// - `4` - validation pending
    /// - `5` - validation rejected
    /// - `6` - accepted validation did not include a calculated weight
    struct PbftVoteEventFact {
        status: u8,
        error_code: String,
        has_progress_fact: bool,
        progress_fact: PbftVoteProgressFact,
    }

    /// Validation-backed PBFT vote fact boundary result.
    struct PbftVoteFactBoundaryResult {
        status: u8,
        error_code: String,
        validation: PbftCanonicalVoteValidation,
        has_progress_fact: bool,
        progress_fact: PbftVoteProgressFact,
    }

    /// Scalar context for one PBFT vote-progress planning pass.
    ///
    /// `has_two_t_plus_one_threshold` gates whether the threshold value should
    /// be passed to the verified-vote executor. `max_future_period_delta`
    /// remains caller-controlled so production routes can preserve legacy
    /// prevalidated-vote behavior while future ingress stages can tighten it.
    struct PbftVoteProgressContext {
        current_period: u64,
        current_round: u64,
        max_future_period_delta: u64,
        has_two_t_plus_one_threshold: bool,
        two_t_plus_one_threshold: u64,
        require_proposed_block_sidecar: bool,
        slashing_enabled: bool,
    }

    /// Pre-mutation PBFT vote-progress decision for C++ executors.
    ///
    /// `status` matches `PbftVoteProgressStatus::as_u8()` in
    /// `rustaxa-consensus`. A true `should_insert_verified_vote` means the shim
    /// should execute exactly one verified-vote insertion mutation, then call
    /// `pbft_vote_progress_plan_after_add`.
    struct PbftVoteProgressPrecheckPlan {
        status: u8,
        error_code: String,
        should_insert_verified_vote: bool,
        has_two_t_plus_one_threshold: bool,
        two_t_plus_one_threshold: u64,
    }

    /// Stable identity for one Rust PBFT vote pipeline transition.
    struct PbftVotePipelineTransitionKey {
        vote_hash: [u8; 32],
        period: u64,
        round: u64,
        step: u64,
        voter: [u8; 20],
    }

    /// Pre-mutation PBFT vote pipeline decision for C++ executors.
    ///
    /// This wraps the existing vote-progress precheck with a transition key and
    /// a Rust-owned session status, so the post-insert report must be returned
    /// to the same Rust vote pipeline session.
    struct PbftVotePipelinePrecheckPlan {
        pipeline_status: u8,
        status: u8,
        error_code: String,
        transition_key: PbftVotePipelineTransitionKey,
        should_insert_verified_vote: bool,
        has_two_t_plus_one_threshold: bool,
        two_t_plus_one_threshold: u64,
        complete: bool,
    }

    /// Post-mutation PBFT vote-progress execution decision for C++ executors.
    ///
    /// This is intentionally operation-specific for `VoteManager::addVerifiedVote`:
    /// Rust owns the protocol decision, while C++ remains the executor for
    /// peer-known marks, proposed-block sidecar handling, gossip, storage,
    /// slashing submission, and PBFT progress dispatch.
    struct PbftVoteProgressExecutionPlan {
        status: u8,
        error_code: String,
        accepted: bool,
        mark_vote_known: bool,
        mark_vote_known_hash: [u8; 32],
        request_proposed_block_sidecar: bool,
        proposed_block_sidecar_hash: [u8; 32],
        proposed_block_sidecar_period: u64,
        gossip_vote: bool,
        gossip_vote_hash: [u8; 32],
        report_slashing: bool,
        slashing_incoming_vote_hash: [u8; 32],
        slashing_conflicting_vote_hash: [u8; 32],
        persist_extra_reward_vote: bool,
        extra_reward_vote_hash: [u8; 32],
        network_t_plus_one_step_updated: bool,
        drive_pbft_progress: bool,
        progress_period: u64,
        progress_round: u64,
        persist_two_t_plus_one_votes: bool,
        two_t_plus_one_kind: u8,
        two_t_plus_one_period: u64,
        two_t_plus_one_round: u64,
        two_t_plus_one_step: u64,
        two_t_plus_one_block_hash: [u8; 32],
    }

    /// Post-mutation PBFT vote pipeline execution decision.
    struct PbftVotePipelineExecutionPlan {
        pipeline_status: u8,
        status: u8,
        error_code: String,
        transition_key: PbftVotePipelineTransitionKey,
        accepted: bool,
        mark_vote_known: bool,
        mark_vote_known_hash: [u8; 32],
        request_proposed_block_sidecar: bool,
        proposed_block_sidecar_hash: [u8; 32],
        proposed_block_sidecar_period: u64,
        gossip_vote: bool,
        gossip_vote_hash: [u8; 32],
        report_slashing: bool,
        slashing_incoming_vote_hash: [u8; 32],
        slashing_conflicting_vote_hash: [u8; 32],
        persist_extra_reward_vote: bool,
        extra_reward_vote_hash: [u8; 32],
        network_t_plus_one_step_updated: bool,
        drive_pbft_progress: bool,
        progress_period: u64,
        progress_round: u64,
        persist_two_t_plus_one_votes: bool,
        two_t_plus_one_kind: u8,
        two_t_plus_one_period: u64,
        two_t_plus_one_round: u64,
        two_t_plus_one_step: u64,
        two_t_plus_one_block_hash: [u8; 32],
        complete: bool,
    }

    /// Pre-mutation PBFT vote admission decision for C++ executors.
    ///
    /// The admission session owns event-fact derivation and the underlying
    /// pipeline precheck. The `progress_fact` payload is Rust-derived and is
    /// provided so temporary C++ live sidecars can be parity-checked before the
    /// executor performs the verified-vote insertion mutation.
    struct PbftVoteAdmissionPrecheckPlan {
        admission_status: u8,
        has_validation: bool,
        validation: PbftCanonicalVoteValidation,
        event_status: u8,
        pipeline_status: u8,
        status: u8,
        error_code: String,
        transition_key: PbftVotePipelineTransitionKey,
        has_progress_fact: bool,
        progress_fact: PbftVoteProgressFact,
        should_insert_verified_vote: bool,
        has_two_t_plus_one_threshold: bool,
        two_t_plus_one_threshold: u64,
        complete: bool,
    }

    /// Post-mutation PBFT vote admission execution decision.
    struct PbftVoteAdmissionExecutionPlan {
        admission_status: u8,
        pipeline_status: u8,
        status: u8,
        error_code: String,
        transition_key: PbftVotePipelineTransitionKey,
        accepted: bool,
        mark_vote_known: bool,
        mark_vote_known_hash: [u8; 32],
        request_proposed_block_sidecar: bool,
        proposed_block_sidecar_hash: [u8; 32],
        proposed_block_sidecar_period: u64,
        gossip_vote: bool,
        gossip_vote_hash: [u8; 32],
        report_slashing: bool,
        slashing_incoming_vote_hash: [u8; 32],
        slashing_conflicting_vote_hash: [u8; 32],
        persist_extra_reward_vote: bool,
        extra_reward_vote_hash: [u8; 32],
        network_t_plus_one_step_updated: bool,
        drive_pbft_progress: bool,
        progress_period: u64,
        progress_round: u64,
        persist_two_t_plus_one_votes: bool,
        two_t_plus_one_kind: u8,
        two_t_plus_one_period: u64,
        two_t_plus_one_round: u64,
        two_t_plus_one_step: u64,
        two_t_plus_one_block_hash: [u8; 32],
        complete: bool,
    }

    /// Runtime-owned PBFT vote admission transition result.
    ///
    /// This is the production-oriented `VoteManager::addVerifiedVote` bridge
    /// result: Rust validates canonical bytes from caller-supplied facts,
    /// mutates the Rust-owned verified-vote index, records vote payload
    /// sidecars, and returns explicit executor effects. C++ still executes
    /// storage writes, slashing transaction submission, logging, and temporary
    /// live sidecar materialization.
    struct PbftVoteAdmissionRuntimeResult {
        status: u8,
        error_code: String,
        accepted: bool,
        rejected: bool,
        has_validation: bool,
        validation: PbftCanonicalVoteValidation,
        replay_should_mark: bool,
        replay_inserted: bool,
        replay_already_present: bool,
        has_vote: bool,
        vote: VerifiedVotePayload,
        has_verified_vote_add: bool,
        verified_vote_add: VerifiedVoteAddOutcome,
        has_storage_vote: bool,
        storage_vote: PbftVoteStorageRecord,
        persist_extra_reward_vote: bool,
        extra_reward_vote: PbftVoteStorageRecord,
        persist_two_t_plus_one_votes: bool,
        two_t_plus_one_bundle: PbftTwoTPlusOneVoteBundle,
        mark_vote_known: bool,
        mark_vote_known_hash: [u8; 32],
        request_proposed_block_sidecar: bool,
        proposed_block_sidecar_hash: [u8; 32],
        proposed_block_sidecar_period: u64,
        gossip_vote: bool,
        gossip_vote_hash: [u8; 32],
        report_slashing: bool,
        slashing_incoming_vote: PbftVoteStorageRecord,
        slashing_conflicting_vote: PbftVoteStorageRecord,
        network_t_plus_one_step_updated: bool,
        drive_pbft_progress: bool,
        progress_period: u64,
        progress_round: u64,
    }

    /// Runtime-owned validation result for callers that validate without
    /// admitting a vote into verified-vote state.
    struct PbftVoteRuntimeValidationResult {
        status: u8,
        error_code: String,
        accepted: bool,
        rejected: bool,
        validation: PbftCanonicalVoteValidation,
        replay_should_mark: bool,
        replay_inserted: bool,
        replay_already_present: bool,
    }

    /// Compact reward-vote membership facts for one PBFT round.
    ///
    /// The C++ VoteManager shim supplies one candidate for the preferred
    /// reward round and a reverse-ordered list of all known period rounds.
    /// `vote_hashes` contains only votes in the cert-vote step bucket for the
    /// expected reward block hash; C++ sidecar objects never cross this bridge.
    struct PbftRewardVoteRoundCandidate {
        round: u64,
        has_cert_step: bool,
        has_reward_block: bool,
        vote_hashes: Vec<PbftFinalizationHash>,
    }

    /// Fact-only input for Rust-planned PBFT reward-vote selection.
    ///
    /// `requested_vote_hashes` are the hashes embedded in the PBFT block being
    /// checked. Rust first evaluates `preferred_round`, then scans
    /// `period_rounds` in caller-supplied order to preserve legacy reverse
    /// round lookup.
    struct PbftRewardVoteSelectionFact {
        block_period: u64,
        reward_period: u64,
        preferred_reward_round: u64,
        reward_block_hash: [u8; 32],
        requested_vote_hashes: Vec<PbftFinalizationHash>,
        has_preferred_round: bool,
        preferred_round: PbftRewardVoteRoundCandidate,
        has_reward_period: bool,
        period_rounds: Vec<PbftRewardVoteRoundCandidate>,
    }

    /// Rust-planned PBFT reward-vote selection output.
    ///
    /// When `accepted` is true, `selected_vote_hashes` preserves the PBFT
    /// block's requested order. C++ maps those hashes back to live `PbftVote`
    /// sidecars only if the caller requested copied votes.
    struct PbftRewardVoteSelectionPlan {
        accepted: bool,
        status: u8,
        error_code: String,
        selected_period: u64,
        selected_round: u64,
        selected_block_hash: [u8; 32],
        selected_vote_hashes: Vec<PbftFinalizationHash>,
        missing_vote_hash: [u8; 32],
    }

    /// Rust-owned PBFT reward-vote materialization output.
    ///
    /// This keeps reward-vote selection under the `BridgeVerifiedVotes`
    /// runtime that owns verified-vote metadata and retained weighted payloads.
    /// When `accepted` is true, `selected_records` is ordered exactly like
    /// `selected_vote_hashes`.
    struct PbftRewardVotePayloadSelection {
        accepted: bool,
        status: u8,
        error_code: String,
        selected_period: u64,
        selected_round: u64,
        selected_block_hash: [u8; 32],
        selected_vote_hashes: Vec<PbftFinalizationHash>,
        selected_records: Vec<PbftVoteStorageRecord>,
        missing_vote_hash: [u8; 32],
    }

    /// Explicit caller facts for one Rust-planned PBFT vote validation pass.
    ///
    /// The C++ shim owns live vote materialization, FinalChain/key lookups,
    /// cryptographic checks, and mutable weight calculation for now. Rust owns
    /// the validation decision and replay-marker timing from these facts.
    struct PbftVoteValidationFact {
        vote_type: u8,
        dpos_vote_count_ready: bool,
        dpos_vote_count: u64,
        vrf_key_ready: bool,
        has_vrf_key: bool,
        signature_ready: bool,
        signature_valid: bool,
        vrf_sortition_ready: bool,
        vrf_sortition_valid: bool,
        total_dpos_vote_count_ready: bool,
        total_dpos_vote_count: u64,
        weight_ready: bool,
        weight: u64,
        future_dpos_state: bool,
        unknown_error: bool,
        committee_size: u64,
        number_of_proposers: u64,
    }

    /// Rust PBFT vote validation decision for C++ boundary executors.
    struct PbftVoteValidationPlan {
        status: u8,
        error_code: String,
        accepted: bool,
        rejected: bool,
        mark_validated_replay: bool,
        has_sortition_threshold: bool,
        sortition_threshold: u64,
    }

    /// Caller facts for Rust-owned PBFT `2t+1` threshold lookup and caching.
    struct PbftTwoTPlusOneThresholdFact {
        pbft_period: u64,
        vote_type: u8,
        current_pbft_chain_size: u64,
        committee_size: u64,
        number_of_proposers: u64,
        has_total_dpos_votes_count: bool,
        total_dpos_votes_count: u64,
        future_dpos_state: bool,
        unknown_error: bool,
    }

    /// Rust PBFT `2t+1` threshold lookup result.
    struct PbftTwoTPlusOneThresholdPlan {
        status: u8,
        error_code: String,
        has_threshold: bool,
        threshold: u64,
        sortition_threshold: u64,
        needs_total_dpos_votes: bool,
        cache_hit: bool,
        cached: bool,
    }

    /// Result of inspecting canonical legacy PBFT vote RLP in Rust.
    struct PbftCanonicalVoteInspection {
        status: u8,
        error_code: String,
        vote_hash: [u8; 32],
        signing_hash: [u8; 32],
        block_hash: [u8; 32],
        period: u64,
        round: u64,
        step: u64,
        vote_type: u8,
        recovered_public_key: [u8; 64],
        recovered_voter: [u8; 20],
        signature_valid: bool,
        vrf_proof: [u8; 80],
        has_embedded_weight: bool,
        embedded_weight: u64,
    }

    /// External node-state facts used by Rust canonical PBFT vote validation.
    struct PbftVoteValidationExternalFacts {
        voter_dpos_ready: bool,
        voter_dpos_vote_count: u64,
        total_dpos_ready: bool,
        total_dpos_vote_count: u64,
        future_dpos_state: bool,
        unknown_error: bool,
        vrf_key_ready: bool,
        has_vrf_key: bool,
        vrf_public_key: [u8; 32],
        strict_vrf: bool,
        committee_size: u64,
        number_of_proposers: u64,
    }

    /// Complete Rust result for validating one canonical legacy PBFT vote.
    struct PbftCanonicalVoteValidation {
        status: u8,
        error_code: String,
        accepted: bool,
        rejected: bool,
        mark_validated_replay: bool,
        vote_hash: [u8; 32],
        signing_hash: [u8; 32],
        block_hash: [u8; 32],
        period: u64,
        round: u64,
        step: u64,
        vote_type: u8,
        recovered_voter: [u8; 20],
        recovered_public_key: [u8; 64],
        signature_valid: bool,
        vrf_valid: bool,
        has_sortition_threshold: bool,
        sortition_threshold: u64,
        weight_calculated: bool,
        calculated_weight: u64,
        vrf_output: [u8; 64],
    }

    /// Rust PBFT vote generation input supplied by the C++ VoteManager shim.
    ///
    /// Secrets are ephemeral call inputs only; Rust does not store them in a
    /// runtime handle. Expected identity fields let Rust reject mismatched
    /// wallet material before returning canonical vote bytes.
    struct PbftVoteGenerationInput {
        block_hash: [u8; 32],
        vote_type: u8,
        period: u64,
        round: u64,
        step: u64,
        node_secret: [u8; 32],
        vrf_secret: [u8; 64],
        expected_voter: [u8; 20],
        expected_vrf_public_key: [u8; 32],
    }

    /// DPoS facts used by Rust to embed a legacy PBFT vote weight.
    struct PbftVoteWeightFacts {
        voter_dpos_vote_count: u64,
        total_dpos_vote_count: u64,
        committee_size: u64,
        number_of_proposers: u64,
    }

    /// Rust-generated canonical PBFT vote payload.
    ///
    /// `vote_rlp` is a signed 3-field legacy vote for unweighted generation and
    /// a signed 4-field weighted vote when `has_weight` is true. `vote_hash`
    /// remains the unweighted signed vote hash used as the consensus identity.
    struct PbftGeneratedVote {
        status: u8,
        error_code: String,
        accepted: bool,
        vote_hash: [u8; 32],
        signing_hash: [u8; 32],
        block_hash: [u8; 32],
        voter: [u8; 20],
        voter_public_key: [u8; 64],
        vrf_public_key: [u8; 32],
        vrf_proof: [u8; 80],
        vrf_output: [u8; 64],
        period: u64,
        round: u64,
        step: u64,
        vote_type: u8,
        has_weight: bool,
        weight: u64,
        vote_rlp: Vec<u8>,
    }

    /// Explicit caller facts for locally generated proposer sortition screening.
    struct PbftProposerSortitionFact {
        dpos_vote_count_ready: bool,
        dpos_vote_count: u64,
        total_dpos_vote_count_ready: bool,
        total_dpos_vote_count: u64,
        weight_ready: bool,
        weight: u64,
        future_dpos_state: bool,
        unknown_error: bool,
        number_of_proposers: u64,
    }

    /// Rust screening decision for one locally generated proposer sortition.
    struct PbftProposerSortitionPlan {
        status: u8,
        error_code: String,
        accepted: bool,
        rejected: bool,
        has_sortition_threshold: bool,
        sortition_threshold: u64,
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

    /// Validator vote-count snapshot fact supplied for pillar-block planning.
    struct PillarValidatorVoteCount {
        address: [u8; 20],
        vote_count: u64,
    }

    /// One signed validator vote-count change planned for a pillar block.
    struct PillarValidatorVoteCountChange {
        address: [u8; 20],
        vote_count_change: i32,
    }

    /// Parent-linkage facts for one candidate pillar block.
    struct PillarBlockLinkageFact {
        pillar_block_period: u64,
        pillar_block_previous_hash: [u8; 32],
        first_pillar_block_period: u64,
        pillar_blocks_interval: u64,
        has_last_finalized_pillar_block: bool,
        last_finalized_period: u64,
        last_finalized_hash: [u8; 32],
    }

    /// Result of deterministic pillar-block parent-linkage planning.
    ///
    /// Status values:
    /// - `0` - valid non-first block
    /// - `1` - valid first pillar block
    /// - `2` - missing last finalized pillar block
    /// - `3` - period mismatch
    /// - `4` - previous hash mismatch
    /// - `5` - interval overflow
    struct PillarBlockLinkagePlan {
        status: u8,
        valid: bool,
        expected_previous_period: u64,
    }

    /// Typed facts for Rust-side pillar-block shell planning.
    struct PillarBlockCreationFact {
        pillar_block_period: u64,
        state_root: [u8; 32],
        bridge_root: [u8; 32],
        bridge_epoch: [u8; 32],
        first_pillar_block_period: u64,
        pillar_blocks_interval: u64,
        has_last_finalized_pillar_block: bool,
        last_finalized_period: u64,
        last_finalized_hash: [u8; 32],
    }

    /// Rust-planned shell fields for temporary C++ `PillarBlock` materialization.
    ///
    /// Status values match `PillarBlockLinkagePlan`.
    struct PillarBlockCreationPlan {
        status: u8,
        valid: bool,
        expected_previous_period: u64,
        previous_pillar_block_hash: [u8; 32],
        state_root: [u8; 32],
        bridge_root: [u8; 32],
        bridge_epoch: [u8; 32],
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

    struct VerifiedStepVotesEntry {
        block_hash: [u8; 32],
        total_weight: u64,
        vote_hashes: Vec<DagHash>,
    }

    struct VerifiedStepVotesLookup {
        found: bool,
        entries: Vec<VerifiedStepVotesEntry>,
    }

    struct VerifiedVoteAddOutcome {
        vote: VerifiedVotePayload,
        inserted: bool,
        total_weight: u64,
        votes_count: usize,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
        used_secondary_slot: bool,
        duplicate_vote_hash: bool,
        threshold_applied: bool,
        t_plus_one_reached: bool,
        network_t_plus_one_step_updated: bool,
        two_t_plus_one_reached: bool,
        two_t_plus_one_kind_found: bool,
        two_t_plus_one_kind: u8,
        two_t_plus_one_round_found: bool,
        two_t_plus_one_inserted: bool,
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

    struct TwoTPlusOneVotePayloadsLookup {
        found: bool,
        block_hash: [u8; 32],
        step: u64,
        votes: Vec<PbftVoteStorageRecord>,
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

    struct FinalChainExecutionStatus {
        executed_dag_block_count: u64,
        executed_transaction_count: u64,
    }

    /// One address whose PBFT-facing FinalChain DPoS facts should be collected.
    struct PbftFinalChainFactAddress {
        address: [u8; 20],
    }

    /// PBFT-facing FinalChain fact request.
    ///
    /// `period` is the PBFT period used by existing C++ consensus callers.
    /// `candidate_final_chain_hash` is checked only when
    /// `validate_candidate_final_chain_hash` is true. `collect_final_chain_hash`
    /// requests the proposal-time expected hash without validating a candidate.
    /// Address facts are returned in the same order as `addresses`.
    struct PbftFinalChainFactRequest {
        period: u64,
        candidate_final_chain_hash: [u8; 32],
        collect_final_chain_hash: bool,
        validate_candidate_final_chain_hash: bool,
        collect_total_vote_count: bool,
        collect_address_vote_counts: bool,
        addresses: Vec<PbftFinalChainFactAddress>,
    }

    /// PBFT-facing FinalChain hash lookup or validation result.
    ///
    /// Status values preserve `PbftStateRootValidation` compatibility:
    /// `0` means the expected FinalChain hash was found and, for validation,
    /// matched the candidate; `1` means the required finalized header is not
    /// available yet; `2` means the candidate hash mismatched the Rust-sourced
    /// FinalChain hash.
    struct PbftFinalChainHashResult {
        status: u8,
        expected_hash: [u8; 32],
        actual_hash: [u8; 32],
        error_code: String,
    }

    /// One ordered PBFT-facing FinalChain DPoS address fact.
    ///
    /// `status` is `0` when the vote count and eligibility are available and
    /// `1` when the Rust FinalChain snapshot for the requested period is not
    /// available.
    struct PbftFinalChainAddressFact {
        address: [u8; 20],
        status: u8,
        eligible: bool,
        vote_count: u64,
        error_code: String,
    }

    /// Grouped FinalChain facts sourced by Rust for PBFT manager decisions.
    ///
    /// `status` is `0` when all requested fact groups are ready and `1` when at
    /// least one requested fact group is unavailable as data. Bridge
    /// infrastructure failures still throw.
    struct PbftFinalChainFacts {
        status: u8,
        last_block_number: u64,
        final_chain_hash: PbftFinalChainHashResult,
        total_vote_count_status: u8,
        has_total_vote_count: bool,
        total_vote_count: u64,
        address_facts: Vec<PbftFinalChainAddressFact>,
        error_code: String,
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
        delegations: Vec<GenesisDelegation>,
    }

    struct GenesisDelegation {
        delegator: [u8; 20],
        stake: Vec<u8>,
    }

    struct GenesisDposConfig {
        eligibility_balance_threshold: Vec<u8>,
        vote_eligibility_balance_step: Vec<u8>,
        validator_maximum_stake: Vec<u8>,
        minimum_deposit: Vec<u8>,
        commission_change_delta: u16,
        commission_change_frequency: u32,
        delegation_delay: u64,
        // Exclusive period boundary below which legacy DAG VDF sortition uses
        // the snapshot total eligible vote count as denominator.
        dag_vdf_sortition_total_vote_count_until_period: u64,
    }

    struct FinalChainRewardsConfig {
        committee_size: u32,
        magnolia_period: u64,
        aspen_part_one_period: u64,
        fix_claim_all_block_num: u64,
        aspen_part_two_period: u64,
        max_block_author_reward_percent: u16,
        dag_proposers_reward_percent: u16,
        yield_percentage: u16,
        dpos_blocks_per_year: u32,
        dpos_delegation_locking_period: u64,
        cornus_period: u64,
        cornus_delegation_locking_period: u64,
        genesis_balance_sum: Vec<u8>,
        aspen_max_supply: Vec<u8>,
        aspen_generated_rewards: Vec<u8>,
        cacti_period: u64,
        cacti_delegation_locking_period: u64,
        magnolia_jail_time: u64,
        cacti_jail_time: u64,
        frequency_rules: Vec<RewardsFrequencyRule>,
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

    struct FinalChainExecutionRequest {
        pbft_block_rlp: Vec<u8>,
        transactions: Vec<FinalizationTransaction>,
        finalized_dag_blocks: Vec<FinalizationDagBlock>,
        blocks_per_year: u32,
        cert_votes: Vec<RewardsCertVoteFact>,
        block_gas_limit: u64,
        mode: u8,
    }

    struct FinalChainEvmTransactionInput {
        position: u64,
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
        kind: u8,
        is_system: bool,
    }

    struct FinalChainSystemTransactionRequest {
        request_id: [u8; 32],
        period: u64,
        regular_transaction_count: u64,
    }

    struct FinalChainSystemTransactionReport {
        request_id: [u8; 32],
        period: u64,
        transactions: Vec<TxRlp>,
    }

    struct FinalChainSystemTransactionPlanFact {
        request_id: [u8; 32],
        period: u64,
        is_pillar_block_period: bool,
        bridge_contract_address: [u8; 20],
        bridge_contract_found: bool,
        bridge_contract_has_code: bool,
        should_finalize_epoch: bool,
        system_account_nonce: u64,
        block_gas_limit: u64,
    }

    struct FinalChainSystemTransactionPlan {
        request_id: [u8; 32],
        period: u64,
        transactions: Vec<TxRlp>,
    }

    struct FinalChainEvmExecutionRequest {
        request_id: [u8; 32],
        period: u64,
        block_author: [u8; 20],
        timestamp: u64,
        block_gas_limit: u64,
        transactions: Vec<FinalChainEvmTransactionInput>,
    }

    struct FinalChainEvmLogTopic {
        topic: [u8; 32],
    }

    struct FinalChainEvmLog {
        address: [u8; 20],
        topics: Vec<FinalChainEvmLogTopic>,
        data: Vec<u8>,
    }

    struct FinalChainEvmTransactionResult {
        position: u64,
        hash: [u8; 32],
        status: u8,
        gas_used: u64,
        cumulative_gas_used: u64,
        receipt_rlp: Vec<u8>,
        logs: Vec<FinalChainEvmLog>,
        new_contract_address_found: bool,
        new_contract_address: [u8; 20],
        code_error: String,
        consensus_error: String,
    }

    struct FinalChainEvmExecutionReport {
        request_id: [u8; 32],
        status: u8,
        state_root: [u8; 32],
        cumulative_gas_used: u64,
        results: Vec<FinalChainEvmTransactionResult>,
    }

    struct FinalChainEvmRewardsRequest {
        request_id: [u8; 32],
        period: u64,
        block_author: [u8; 20],
        block_gas_used: u64,
        transaction_gas_used: Vec<u64>,
        transaction_fees: Vec<ReceiptRlp>,
        finalized_dag_block_count: u64,
    }

    struct FinalChainEvmRewardsReport {
        request_id: [u8; 32],
        period: u64,
        status: u8,
        state_root: [u8; 32],
        total_reward: Vec<u8>,
    }

    struct FinalChainExternalEvmCommitPlan {
        request_id: [u8; 32],
        period: u64,
        post_execution_state_root: [u8; 32],
        state_root: [u8; 32],
        total_reward: Vec<u8>,
        transactions_root: [u8; 32],
        receipts_root: [u8; 32],
        header_log_bloom: Vec<u8>,
        indexed_log_bloom: Vec<u8>,
        receipts_rlp: Vec<u8>,
        encoded_receipts: Vec<ReceiptRlp>,
        gas_used: u64,
        executed_dag_blocks: u64,
        executed_transactions: u64,
        regular_transaction_count: u64,
        system_transaction_count: u64,
        error_code: String,
    }

    struct FinalChainExternalEvmTransactionPublication {
        transaction_hash: [u8; 32],
        position: u32,
        is_system: bool,
        receipt_rlp: Vec<u8>,
    }

    struct FinalChainExternalEvmRewardsStatsUpdate {
        current_period: u64,
        cache_current_period: bool,
        clear_cached_stats: bool,
        current_block_stats_rlp: Vec<u8>,
    }

    struct FinalChainProposalPeriodDagLevelUpdate {
        has_update: bool,
        level: u64,
    }

    struct FinalChainExternalEvmPublicationPlan {
        request_id: [u8; 32],
        plan_id: [u8; 32],
        period: u64,
        block_hash: [u8; 32],
        block_header_rlp: Vec<u8>,
        stored_header_rlp: Vec<u8>,
        receipts_rlp: Vec<u8>,
        indexed_log_bloom: Vec<u8>,
        system_transaction_hashes_rlp: Vec<u8>,
        transaction_publications: Vec<FinalChainExternalEvmTransactionPublication>,
        executed_dag_blocks: u64,
        executed_transactions: u64,
        proposal_period_dag_level_update: FinalChainProposalPeriodDagLevelUpdate,
        rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate,
        error_code: String,
    }

    struct FinalChainExternalEvmStateCommitRequest {
        request_id: [u8; 32],
        plan_id: [u8; 32],
        period: u64,
        post_execution_state_root: [u8; 32],
        post_rewards_state_root: [u8; 32],
        publication_block_hash: [u8; 32],
    }

    struct FinalChainExternalEvmStateCommitIntent {
        request_id: [u8; 32],
        plan_id: [u8; 32],
        period: u64,
        publication_block_hash: [u8; 32],
        status: u8,
        error_code: String,
    }

    struct FinalChainExternalEvmStateCommitResult {
        status: u8,
        error_code: String,
    }

    struct FinalChainExternalEvmLifecycleReport {
        request_id: [u8; 32],
        plan_id: [u8; 32],
        period: u64,
        post_execution_state_root: [u8; 32],
        post_rewards_state_root: [u8; 32],
        publication_block_hash: [u8; 32],
        status: u8,
        error_code: String,
    }

    struct FinalChainExternalEvmCommitDecision {
        request_id: [u8; 32],
        plan_id: [u8; 32],
        decision_id: [u8; 32],
        period: u64,
        publication_block_hash: [u8; 32],
        status: u8,
        error_code: String,
    }

    struct FinalChainExternalEvmPublicationReport {
        request_id: [u8; 32],
        plan_id: [u8; 32],
        period: u64,
        block_hash: [u8; 32],
        executed_dag_block_count: u64,
        executed_transaction_count: u64,
        dpos_snapshot_status: u8,
        account_snapshot_status: u8,
        status: u8,
        error_code: String,
    }

    struct FinalChainExecutionStep {
        status: u8,
        action: u8,
        period: u64,
        external_evm_transaction_count: u64,
        evm_request: FinalChainEvmExecutionRequest,
        evm_rewards_request: FinalChainEvmRewardsRequest,
        system_transaction_request: FinalChainSystemTransactionRequest,
        error_code: String,
    }

    struct FinalChainExecutionCommitReport {
        status: u8,
        period: u64,
        block_header_rlp: Vec<u8>,
        receipts: Vec<ReceiptRlp>,
        gas_used: u64,
        executed_dag_blocks: u64,
        executed_transactions: u64,
        error_code: String,
    }

    struct DagHash {
        hash: [u8; 32],
    }

    struct DagCounterUpdate {
        hash: [u8; 32],
        level: u64,
        tips_count: u64,
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
        /// True when a proposal-period account snapshot proved this finalized
        /// transaction nonce is older than the sender account nonce.
        old_finalized: bool,
        tx_rlp: Vec<u8>,
    }

    /// One ordered TransactionManager runtime transaction view request.
    struct TransactionManagerTransactionViewRequest {
        input_index: u64,
        hash: [u8; 32],
    }

    /// One source-ordered TransactionManager runtime payload view entry.
    struct TransactionManagerTransactionView {
        input_index: u64,
        hash: [u8; 32],
        found: bool,
        /// Source precedence is queue / live sidecars / storage in one API surface.
        source: u8,
        /// True when a proposal-period account snapshot filtered a finalized tx as old.
        old_finalized: bool,
        tx_rlp: Vec<u8>,
    }

    /// Bounded payload-view plan preserving caller ordering semantics.
    struct TransactionManagerTransactionViewPlan {
        requested_count: u64,
        complete: bool,
        views: Vec<TransactionManagerTransactionView>,
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

    /// Input transaction fact for runtime DAG persistence with sender account
    /// facts sourced by Rust from latest FinalChain state.
    struct DagTransactionSaveRuntimeFact {
        input_index: u64,
        hash: [u8; 32],
        trx_rlp: Vec<u8>,
        transaction_nonce: [u8; 32],
        sender: [u8; 20],
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

    /// Filtered finalized transaction action with preserved index mapping.
    struct TransactionManagerFilterAction {
        input_index: u64,
        hash: [u8; 32],
    }

    /// Finalized-filtering outcome for Rust-only decision logic.
    struct FinalizedTransactionFilterPlan {
        not_finalized: Vec<TransactionManagerFilterAction>,
    }

    /// Input for Rust runtime `verifyTransactionsNotFinalized` decisions with
    /// sender account facts sourced from FinalChain.
    struct TransactionManagerVerifyNotFinalizedRuntimeFact {
        input_index: u64,
        hash: [u8; 32],
        transaction_nonce: [u8; 32],
        sender: [u8; 20],
    }

    /// Input for `verifyTransactionsNotFinalized` when sender nonce facts are
    /// supplied by the C++ external-EVM boundary.
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

    /// Facts for runtime validated insert with account facts sourced from
    /// FinalChain at execution time.
    struct TransactionManagerValidatedInsertRuntimeFact {
        tx_hash: [u8; 32],
        sender: [u8; 20],
        transaction_nonce: [u8; 32],
        transaction_cost: [u8; 32],
        gas_limit: u64,
        propose_dag_gas_limit: u64,
        insert_non_proposable: bool,
    }

    /// FinalChain facts collected by C++ when account state is still owned by
    /// the external EVM adapter.
    struct TransactionManagerFinalChainAdmissionFact {
        account_found: bool,
        account_nonce: [u8; 32],
        account_balance: [u8; 32],
        finalized_period_known: bool,
        finalized_period: u64,
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

    /// One direct transaction hash in a typed TransactionManager command report.
    ///
    /// Direct hashes are emitted for runtime queue or sidecar effects that are
    /// no longer tied to a current C++ input vector.
    struct TransactionManagerHashCommand {
        hash: [u8; 32],
    }

    /// Typed command report for DAG-block transaction persistence.
    ///
    /// Rust has already persisted storage, updated sidecars, erased queued
    /// transactions, and updated the authoritative runtime count. C++ consumes
    /// this report only for logging.
    struct TransactionManagerDagSaveCommandReport {
        queue_erased: Vec<TransactionManagerHashCommand>,
    }

    /// Typed command report for finalized transaction status updates.
    ///
    /// Rust has already applied storage updates, live sidecar transitions,
    /// queue erasure, optional finalized-account queue purge, and runtime count
    /// changes. C++ consumes the buckets for existing logs only and reads the
    /// authoritative count from the runtime when callers ask for it.
    struct TransactionManagerFinalizedStatusCommandReport {
        removed_non_finalized: Vec<TransactionManagerHashCommand>,
        queue_erased: Vec<TransactionManagerHashCommand>,
        finalized_account_purged: Vec<TransactionManagerHashCommand>,
        accepted_count: u64,
        purge_transaction_queue: bool,
    }

    /// Typed admission result attached to admission command reports.
    struct TransactionManagerAdmissionResult {
        present: bool,
        insert_status: u8,
        transaction_status: u8,
        finalized_period_known: bool,
        finalized_period: u64,
        requires_finalized_lookup: bool,
    }

    /// Typed command report for TransactionManager admission.
    ///
    /// Rust has already completed validated queue mutation and public status
    /// mapping. C++ consumes this report only for legacy logging/event dispatch
    /// mechanics and public status conversion.
    struct TransactionManagerAdmissionCommandReport {
        inserted_hash_found: bool,
        inserted_hash: [u8; 32],
        transaction_added_hash_found: bool,
        transaction_added_hash: [u8; 32],
        admission: TransactionManagerAdmissionResult,
    }

    /// Legacy public insert result selected by Rust.
    ///
    /// `accepted` and `message` map directly to the C++ public
    /// `TransactionManager::insertTransaction` return value.
    struct TransactionManagerPublicInsertResult {
        accepted: bool,
        message: String,
    }

    /// Typed command report for public `insertTransaction` admission.
    ///
    /// Rust owns known-fast-path precheck, verification status decision, account
    /// fact sourcing, finalized-location lookup, queue mutation, and admission
    /// status mapping plus legacy public result text.
    struct TransactionManagerPublicAdmissionCommandReport {
        verification_status: u8,
        verification_chain_id: u64,
        verification_expected_chain_id: u64,
        public_result: TransactionManagerPublicInsertResult,
        admission: TransactionManagerAdmissionCommandReport,
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
        difficulty: u16,
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

    struct DagProposerFrontierFacts {
        pivot: [u8; 32],
        tips: Vec<DagHash>,
        propose_level: u64,
        anchor: [u8; 32],
        non_finalized_block_count: u64,
        non_finalized_min_difficulty: u32,
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

    /// Compact block and caller-supplied transaction facts used to open one
    /// Rust-owned `DagManager::verifyBlock` runtime session.
    struct DagVerifyBlockSessionInput {
        block_level: u64,
        pivot: [u8; 32],
        tips: Vec<DagHash>,
        block_transaction_hashes: Vec<DagTransactionHash>,
        supplied_transaction_hashes: Vec<DagTransactionHash>,
    }

    /// One requested Rust-owned `verifyBlock` session step.
    struct DagVerifyBlockSessionStep {
        status: u8,
        action: u8,
        complete: bool,
        reject_code: u32,
        proposal_period: u64,
        query_hashes: Vec<DagTransactionHash>,
        vote_count: u64,
        max_vote_count: u64,
        error_code: String,
    }

    /// Live transaction-materialization report for one `verifyBlock` session.
    struct DagVerifyBlockTransactionReport {
        resolved_transactions: u64,
    }

    /// Live DPoS/VRF authorization report for one `verifyBlock` session.
    struct DagVerifyBlockAuthorizationReport {
        vrf_key_found: bool,
        sender_eligible_vote_count: u64,
        vdf_sortition_max_vote_count: u64,
        eligibility_status: u8,
    }

    /// Live VDF verifier report for one `verifyBlock` session.
    struct DagVerifyBlockVdfReport {
        vdf_status: u8,
    }

    /// Live gas-estimation report for one `verifyBlock` session.
    struct DagVerifyBlockGasReport {
        block_gas_estimation: u64,
        estimated_transactions_weight: u64,
        dag_gas_limit: u64,
        pbft_gas_limit: u64,
        tip_gas_estimations: Vec<DagTipGas>,
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

    /// C++-originated facts for Rust DAG proposal-attempt planning.
    struct DagProposerAttemptInput {
        transaction_pool_size: u64,
        non_finalized_transaction_count: u64,
        max_non_finalized_transactions: u64,
        frontier_facts: DagProposerFrontierFacts,
        proposal_period_found: bool,
        proposal_period: u64,
        last_finalized_period: u64,
        dag_expiry_level_limit: u64,
        wallet_vrf_public_key: [u8; 32],
        wallet_vrf_secret: [u8; 64],
        authorization_facts: DagDposAuthorizationFacts,
        sortition_params: SortitionRuntimeParams,
        max_non_finalized_dag_blocks: u64,
        max_non_finalized_dag_blocks_low_difficulty: u64,
        last_propose_level: u64,
        retry_count: u64,
        max_retry_count: u64,
        proposal_weight_limit: u64,
        total_transaction_shards: u16,
        node_transaction_shard: u16,
        shard_period_interval: u64,
    }

    /// Rust-planned transaction packing request for a DAG proposal attempt.
    struct DagProposerTransactionPackRequest {
        proposal_period: u64,
        weight_limit: u64,
        total_transaction_shards: u16,
        node_transaction_shard: u16,
        shard_period_interval: u64,
    }

    /// Rust-owned DAG proposal-attempt decision.
    struct DagProposerAttemptPlan {
        action: u8,
        reason_code: u32,
        frontier_pivot: [u8; 32],
        frontier_tips: Vec<DagHash>,
        anchor: [u8; 32],
        proposal_level: u64,
        proposal_period_found: bool,
        proposal_period: u64,
        last_finalized_period: u64,
        period_block_hash_found: bool,
        period_block_hash: [u8; 32],
        vrf_input: Vec<u8>,
        vote_count: u64,
        max_vote_count: u64,
        vdf_difficulty: u16,
        vdf_stale: bool,
        old_proposal: bool,
        update_retry_state: bool,
        next_last_propose_level: u64,
        next_retry_count: u64,
        transaction_request: DagProposerTransactionPackRequest,
    }

    /// Step returned by the Rust-owned DAG proposer session cursor.
    struct DagProposerSessionStep {
        status: u8,
        action: u8,
        reason_code: u32,
        return_value: bool,
        update_retry_state: bool,
        next_last_propose_level: u64,
        next_retry_count: u64,
        frontier_pivot: [u8; 32],
        frontier_tips: Vec<DagHash>,
        proposal_level: u64,
        proposal_period: u64,
        last_finalized_period: u64,
        vrf_input: Vec<u8>,
        vote_count: u64,
        max_vote_count: u64,
        vdf_difficulty: u16,
        vdf_stale: bool,
        old_proposal: bool,
        vdf_message: Vec<u8>,
        selected_transaction_hashes: Vec<DagHash>,
        transaction_gas_estimations: Vec<u64>,
        transaction_request: DagProposerTransactionPackRequest,
        error_code: String,
    }

    /// Report from the live transaction-packing executor boundary.
    struct DagProposerTransactionPackReport {
        network_throttled: bool,
        transaction_hashes: Vec<DagHash>,
        transaction_gas_estimations: Vec<u64>,
    }

    /// Report from a live in-flight VDF wait observation.
    struct DagProposerVdfWaitReport {
        latest_proposal_level: u64,
    }

    /// Report that a live VDF proof executor boundary has completed.
    struct DagProposerVdfProofReport {
        proof_ok: bool,
    }

    /// Report after the compatibility stale-proof sleep observes latest level.
    struct DagProposerStaleProofReport {
        latest_proposal_level: u64,
    }

    /// Report after C++ materializes/signs/adds the proposed DAG block.
    struct DagProposerAddBlockReport {
        accepted: bool,
    }

    /// Rust-runtime DAG block construction facts for storage-backed tip metadata planning.
    struct DagProposerStorageBlockConstructionInput {
        frontier_tips: Vec<DagHash>,
        transaction_gas_estimations: Vec<u64>,
        pbft_gas_limit: u64,
        dag_gas_limit: u64,
        max_tips: u16,
    }

    /// Rust-runtime DAG proposer tip-selection facts for the legacy compatibility API.
    struct DagProposerStorageTipSelectionInput {
        frontier_tips: Vec<DagHash>,
        gas_limit: u64,
        max_tips: u16,
    }

    /// Rust producer-side DAG block construction plan.
    struct DagProposerBlockConstructionPlan {
        selected_tips: Vec<DagHash>,
        block_gas_estimation: u64,
    }

    /// Rust producer-side DAG tip-selection plan.
    struct DagProposerTipSelectionPlan {
        selected_tips: Vec<DagHash>,
        skipped_missing_tips: u64,
    }

    /// Final unsigned DAG block fields selected by Rust before temporary C++ signing.
    struct DagProposerBlockIntentInput {
        pivot: [u8; 32],
        level: u64,
        timestamp: u64,
        vdf_rlp: Vec<u8>,
        selected_tips: Vec<DagHash>,
        transaction_hashes: Vec<DagHash>,
        block_gas_estimation: u64,
    }

    /// Final unsigned DAG block fields whose timestamp is selected by Rust.
    struct DagProposerBlockIntentNowInput {
        pivot: [u8; 32],
        level: u64,
        vdf_rlp: Vec<u8>,
        selected_tips: Vec<DagHash>,
        transaction_hashes: Vec<DagHash>,
        block_gas_estimation: u64,
    }

    /// Unsigned DAG block intent with the legacy signing hash C++ must sign temporarily.
    struct DagProposerUnsignedBlockIntent {
        pivot: [u8; 32],
        level: u64,
        timestamp: u64,
        vdf_rlp: Vec<u8>,
        selected_tips: Vec<DagHash>,
        transaction_hashes: Vec<DagHash>,
        block_gas_estimation: u64,
        signing_hash: [u8; 32],
    }

    /// Recoverable signature supplied for a Rust-planned unsigned DAG block intent.
    struct DagProposerSignedBlockIntentInput {
        intent: DagProposerUnsignedBlockIntent,
        signature: Vec<u8>,
    }

    /// Canonical signed DAG block bytes and hash returned by Rust.
    struct DagProposerSignedBlockIntent {
        block_rlp: Vec<u8>,
        block_hash: [u8; 32],
    }

    /// Facts used by Rust to plan one DAG add-block execution.
    struct DagAddBlockEffectInput {
        save: bool,
        proposed: bool,
        block_exists: bool,
        block_level: u64,
        dag_expiry_level: u64,
        references_available: bool,
        missing_references: Vec<DagHash>,
    }

    /// Compact block facts used by the Rust DAG manager runtime to plan one
    /// add-block execution from runtime-owned graph state.
    struct DagAddBlockRuntimeInput {
        save: bool,
        proposed: bool,
        block_hash: [u8; 32],
        pivot: [u8; 32],
        tips: Vec<DagHash>,
        block_level: u64,
    }

    /// Typed side effects C++ executes for a Rust-planned DAG add-block attempt.
    struct DagAddBlockEffectPlan {
        accepted: bool,
        duplicate: bool,
        expired: bool,
        persist_transactions: bool,
        persist_block: bool,
        add_to_graph: bool,
        mirror_legacy_graph: bool,
        emit_verified: bool,
        gossip: bool,
        proposed: bool,
        missing_references: Vec<DagHash>,
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
        /// Plans one add-block execution from Rust-owned runtime graph state.
        pub fn dag_manager_runtime_plan_add_block(
            self: &BridgeDagManagerRuntime,
            input: DagAddBlockRuntimeInput,
        ) -> Result<DagAddBlockEffectPlan>;
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
        pub fn dag_manager_runtime_proposer_frontier_facts(
            self: &BridgeDagManagerRuntime,
        ) -> DagProposerFrontierFacts;
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
        pub fn dag_manager_runtime_plan_proposal_block_construction(
            self: &BridgeDagManagerRuntime,
            input: DagProposerStorageBlockConstructionInput,
        ) -> Result<DagProposerBlockConstructionPlan>;
        pub fn dag_manager_runtime_plan_proposal_tip_selection(
            self: &BridgeDagManagerRuntime,
            input: DagProposerStorageTipSelectionInput,
        ) -> Result<DagProposerTipSelectionPlan>;
        pub fn dag_manager_runtime_plan_proposal_attempt(
            self: &BridgeDagManagerRuntime,
            input: DagProposerAttemptInput,
        ) -> Result<DagProposerAttemptPlan>;
        pub fn dag_manager_runtime_create_proposer_session(
            self: &BridgeDagManagerRuntime,
            input: DagProposerAttemptInput,
        ) -> Result<Box<BridgeDagProposerSession>>;
        pub fn dag_manager_runtime_ensure_proposal_period_mapping(
            self: &BridgeDagManagerRuntime,
            level: u64,
            period: u64,
        ) -> Result<bool>;
        pub fn dag_manager_runtime_proposal_period_for_level(
            self: &BridgeDagManagerRuntime,
            level: u64,
        ) -> Result<PeriodLookup>;
        pub fn dag_manager_runtime_period_block_hash(
            self: &BridgeDagManagerRuntime,
            period: u64,
        ) -> Result<HashLookup>;
        pub fn dag_manager_runtime_persistence_counters(
            self: &BridgeDagManagerRuntime,
        ) -> Result<DagPersistenceCounters>;
        pub fn dag_manager_runtime_verify_precheck(
            self: &BridgeDagManagerRuntime,
            block: DagVerifyPrecheckBlock,
        ) -> Result<DagVerifyPrecheckResult>;
        pub fn dag_manager_runtime_create_verify_block_session(
            self: &BridgeDagManagerRuntime,
            input: DagVerifyBlockSessionInput,
        ) -> Result<Box<BridgeDagVerifyBlockSession>>;
        type BridgeDagVerifyBlockSession;
        pub fn dag_verify_block_session_next(
            self: &BridgeDagVerifyBlockSession,
        ) -> DagVerifyBlockSessionStep;
        pub fn dag_verify_block_session_report_transactions(
            self: &mut BridgeDagVerifyBlockSession,
            report: DagVerifyBlockTransactionReport,
        ) -> DagVerifyBlockSessionStep;
        pub fn dag_verify_block_session_report_authorization(
            self: &mut BridgeDagVerifyBlockSession,
            report: DagVerifyBlockAuthorizationReport,
        ) -> DagVerifyBlockSessionStep;
        pub fn dag_verify_block_session_report_vdf(
            self: &mut BridgeDagVerifyBlockSession,
            report: DagVerifyBlockVdfReport,
        ) -> DagVerifyBlockSessionStep;
        pub fn dag_verify_block_session_report_gas(
            self: &mut BridgeDagVerifyBlockSession,
            report: DagVerifyBlockGasReport,
        ) -> DagVerifyBlockSessionStep;
        type BridgeDagProposerSession;
        pub fn dag_proposer_session_next(self: &BridgeDagProposerSession)
            -> DagProposerSessionStep;
        pub fn dag_proposer_session_report_transactions(
            self: &mut BridgeDagProposerSession,
            report: DagProposerTransactionPackReport,
        ) -> DagProposerSessionStep;
        pub fn dag_proposer_session_report_vdf_wait(
            self: &BridgeDagProposerSession,
            report: DagProposerVdfWaitReport,
        ) -> DagProposerSessionStep;
        pub fn dag_proposer_session_report_vdf_proof(
            self: &mut BridgeDagProposerSession,
            report: DagProposerVdfProofReport,
        ) -> DagProposerSessionStep;
        pub fn dag_proposer_session_report_stale_proof(
            self: &mut BridgeDagProposerSession,
            report: DagProposerStaleProofReport,
        ) -> DagProposerSessionStep;
        pub fn dag_proposer_session_report_add_block(
            self: &mut BridgeDagProposerSession,
            report: DagProposerAddBlockReport,
        ) -> DagProposerSessionStep;
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
        pub fn dag_proposer_plan_block_intent(
            input: DagProposerBlockIntentInput,
        ) -> Result<DagProposerUnsignedBlockIntent>;
        pub fn dag_proposer_plan_block_intent_with_current_timestamp(
            input: DagProposerBlockIntentNowInput,
        ) -> Result<DagProposerUnsignedBlockIntent>;
        pub fn dag_proposer_finalize_signed_block_intent(
            input: DagProposerSignedBlockIntentInput,
        ) -> Result<DagProposerSignedBlockIntent>;
        pub fn dag_manager_block_from_rlp(block_rlp: Vec<u8>) -> Result<DagManagerBlock>;
        pub fn dag_plan_add_block_effects(input: DagAddBlockEffectInput) -> DagAddBlockEffectPlan;
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
        pub fn create_pbft_chain_with_storage(
            storage: &BridgeStorage,
            head: PbftChainHeadPayload,
        ) -> Result<Box<BridgePbftChain>>;
        pub fn create_pbft_chain_from_storage(
            storage: &BridgeStorage,
        ) -> Result<Box<BridgePbftChain>>;
        pub fn restore_pbft_chain_storage(
            storage: &BridgeStorage,
        ) -> Result<PbftChainStorageRestore>;
        pub fn pbft_chain_block_exists(
            storage: &BridgeStorage,
            block_hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn pbft_chain_block_rlp(
            storage: &BridgeStorage,
            block_hash: &[u8; 32],
        ) -> Result<PbftBlockStorageLookup>;
        pub fn pbft_chain_initialized_default(self: &BridgePbftChain) -> bool;
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
        pub fn pbft_chain_block_exists(
            self: &BridgePbftChain,
            block_hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn pbft_chain_block_rlp(
            self: &BridgePbftChain,
            block_hash: &[u8; 32],
        ) -> Result<PbftBlockStorageLookup>;
        pub fn pbft_chain_validate_block(
            self: &BridgePbftChain,
            period: u64,
            prev_hash: &[u8; 32],
        ) -> PbftBlockValidationResult;
        pub fn plan_pbft_sync_period_admission(
            fact: PbftSyncPeriodAdmissionFact,
        ) -> PbftSyncPeriodAdmissionPlan;
        pub fn plan_pbft_sync_transaction_query(
            fact: PbftSyncTransactionQueryFact,
        ) -> PbftSyncTransactionQueryPlan;
        pub fn plan_pbft_sync_runtime(
            period_admission_fact: PbftSyncPeriodAdmissionFact,
            transaction_query_fact: PbftSyncTransactionQueryFact,
        ) -> PbftSyncRuntimePlan;
        pub fn load_pbft_sync_egress_payload(
            runtime: &BridgePbftManagerRuntime,
            block_period: u64,
            last_block: bool,
            pbft_chain_synced: bool,
            reward_votes_present: bool,
            reward_votes_period: u64,
        ) -> Result<PbftSyncEgressPayload>;
        pub fn plan_pbft_sync_process_period_data_runtime(
            fact: PbftSyncProcessPeriodDataRuntimeFact,
        ) -> PbftSyncProcessPeriodDataRuntimePlan;
        type BridgePbftSyncQueueDrainSession;
        pub fn create_pbft_sync_queue_drain_session() -> Box<BridgePbftSyncQueueDrainSession>;
        pub fn pbft_sync_queue_drain_session_next(
            session: &mut BridgePbftSyncQueueDrainSession,
            queue_size: usize,
            current_period: u64,
        ) -> PbftSyncQueueDrainStep;
        pub fn pbft_sync_queue_drain_session_report(
            session: &mut BridgePbftSyncQueueDrainSession,
            report: PbftSyncQueueDrainReport,
        ) -> PbftSyncQueueDrainReportResult;
        pub fn plan_pbft_finalization_intent(
            fact: PbftFinalizationIntentFact,
        ) -> PbftFinalizationIntentPlan;
        pub fn plan_pbft_finalization_pillar_preflight(
            fact: PbftFinalizationPillarPreflightFact,
        ) -> PbftFinalizationPillarPreflightPlan;
        pub fn report_pbft_finalization_pillar_preflight(
            plan: &PbftFinalizationPillarPreflightPlan,
            report: PbftFinalizationPillarPreflightReport,
        ) -> PbftFinalizationPillarPreflightPlan;
        pub fn plan_pbft_finalization_runtime(
            plan: &PbftFinalizationIntentPlan,
        ) -> PbftFinalizationRuntimePlan;
        type BridgePbftFinalizationRuntimeSession;
        pub fn create_pbft_finalization_runtime_session(
            plan: &PbftFinalizationIntentPlan,
        ) -> Box<BridgePbftFinalizationRuntimeSession>;
        type BridgePbftManagerRuntime;
        pub fn create_pbft_manager_runtime_from_storage(
            storage: &BridgeStorage,
            fact: PbftManagerStartupFact,
        ) -> Result<Box<BridgePbftManagerRuntime>>;
        pub fn pbft_manager_runtime_load_startup_replay_period(
            runtime: &BridgePbftManagerRuntime,
            period: u64,
            load_period_lambda: bool,
        ) -> Result<PbftManagerStartupReplayPeriod>;
        pub fn pbft_manager_runtime_snapshot(
            runtime: &BridgePbftManagerRuntime,
        ) -> PbftManagerRuntimeSnapshot;
        pub fn plan_pbft_manager_startup_replay_ranges(
            fact: PbftManagerStartupReplayRangeFact,
        ) -> PbftManagerStartupReplayRangePlan;
        pub fn plan_pbft_manager_advance_period(
            pbft_chain_size: u64,
            transition_plan: &PbftManagerTransitionPlan,
        ) -> PbftManagerAdvancePeriodPlan;
        pub fn pbft_manager_runtime_apply_period_advance(
            runtime: &mut BridgePbftManagerRuntime,
            new_period: u64,
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_apply_broadcast_counters(
            runtime: &mut BridgePbftManagerRuntime,
            broadcast_votes_counter: u32,
            rebroadcast_votes_counter: u32,
            broadcast_reward_votes_counter: u32,
            rebroadcast_reward_votes_counter: u32,
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_cert_voted_block_in_round(
            runtime: &BridgePbftManagerRuntime,
        ) -> Result<Vec<u8>>;
        pub fn pbft_manager_runtime_save_cert_voted_block_in_round(
            runtime: &mut BridgePbftManagerRuntime,
            period: u64,
            round: u32,
            block_hash: [u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<PbftManagerRuntimeSnapshot>;
        pub fn pbft_manager_runtime_apply_cert_voted_block_metadata(
            runtime: &mut BridgePbftManagerRuntime,
            period: u64,
            round: u32,
            block_hash: [u8; 32],
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_has_cached_anchor_dag_order(
            runtime: &BridgePbftManagerRuntime,
            anchor_hash: &[u8; 32],
        ) -> bool;
        pub fn pbft_manager_runtime_record_cached_anchor_dag_order(
            runtime: &mut BridgePbftManagerRuntime,
            anchor_hash: [u8; 32],
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_remove_cached_anchor_dag_order(
            runtime: &mut BridgePbftManagerRuntime,
            anchor_hash: [u8; 32],
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_clear_cached_anchor_dag_order(
            runtime: &mut BridgePbftManagerRuntime,
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_own_pillar_block_vote(
            runtime: &BridgePbftManagerRuntime,
        ) -> Result<Vec<u8>>;
        pub fn pbft_manager_runtime_apply_transition_storage_write(
            runtime: &mut BridgePbftManagerRuntime,
            plan: PbftManagerTransitionPlan,
            own_vote_hashes: Vec<PbftFinalizationHash>,
        ) -> Result<PbftManagerTransitionRuntimeApplyResult>;
        pub fn pbft_manager_runtime_apply_executed_block_reset(
            runtime: &mut BridgePbftManagerRuntime,
        ) -> Result<PbftManagerTransitionRuntimeApplyResult>;
        pub fn pbft_manager_runtime_apply_next_voted_status(
            runtime: &mut BridgePbftManagerRuntime,
            status: u8,
        ) -> Result<PbftManagerRuntimeSnapshot>;
        pub fn pbft_manager_runtime_apply_cursor_field(
            runtime: &mut BridgePbftManagerRuntime,
            field: u8,
            value: u32,
        ) -> Result<PbftManagerRuntimeSnapshot>;
        pub fn pbft_manager_runtime_apply_dynamic_lambda(
            runtime: &mut BridgePbftManagerRuntime,
            rounds_count_dynamic_lambda: u32,
            dynamic_lambda_ms: u32,
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_dag_block_period(
            runtime: &BridgePbftManagerRuntime,
            hash: &[u8; 32],
        ) -> Result<BlockPeriodLookup>;
        pub fn pbft_manager_runtime_pbft_block_in_db(
            runtime: &BridgePbftManagerRuntime,
            hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn pbft_manager_runtime_load_finalization_last_period_lambda(
            runtime: &BridgePbftManagerRuntime,
            period: u64,
        ) -> Result<PeriodLambda>;
        pub fn pbft_manager_runtime_inspect_finalization_resume(
            runtime: &BridgePbftManagerRuntime,
            write_set: &PbftFinalizationStorageWritePlan,
            final_chain_last_block: u64,
        ) -> Result<PbftFinalizationResumePlan>;
        pub fn pbft_manager_runtime_apply_finalization_storage_writes(
            runtime: &BridgePbftManagerRuntime,
            write_set: &PbftFinalizationStorageWritePlan,
            stages: Vec<PbftFinalizationStorageWriteStage>,
            sync: bool,
        ) -> Result<PbftFinalizedPeriodApplyResult>;
        type BridgePbftManagerRuntimeSession;
        pub fn create_pbft_manager_runtime_session(
            fact: PbftManagerRuntimeTickFact,
        ) -> Box<BridgePbftManagerRuntimeSession>;
        pub fn plan_pbft_manager_state_action(
            fact: PbftManagerStateActionFact,
        ) -> PbftManagerStateActionPlan;
        pub fn plan_pbft_manager_state_action_effects(
            fact: PbftManagerStateActionFact,
        ) -> PbftManagerStateActionEffectPlan;
        type BridgePbftManagerStateActionEffectSession;
        pub fn create_pbft_manager_state_action_effect_session(
            fact: PbftManagerStateActionFact,
        ) -> Box<BridgePbftManagerStateActionEffectSession>;
        pub fn pbft_manager_state_action_effect_session_next(
            session: &mut BridgePbftManagerStateActionEffectSession,
        ) -> PbftManagerStateActionSessionStep;
        pub fn pbft_manager_state_action_effect_session_report(
            session: &mut BridgePbftManagerStateActionEffectSession,
            report: PbftManagerStateActionEffectReport,
        ) -> PbftManagerStateActionSessionStep;
        pub fn abort_pbft_manager_state_action_effect_session(
            session: &mut BridgePbftManagerStateActionEffectSession,
        );
        type BridgePbftManagerProposalSession;
        pub fn create_pbft_manager_proposal_session(
            fact: PbftManagerProposalInitialFact,
        ) -> Box<BridgePbftManagerProposalSession>;
        pub fn pbft_manager_proposal_session_next(
            session: &mut BridgePbftManagerProposalSession,
        ) -> PbftManagerProposalSessionStep;
        pub fn pbft_manager_proposal_session_report_dag_order(
            session: &mut BridgePbftManagerProposalSession,
            report: PbftManagerProposalDagOrderReport,
        ) -> PbftManagerProposalSessionStep;
        pub fn abort_pbft_manager_proposal_session(
            session: &mut BridgePbftManagerProposalSession,
        ) -> PbftManagerProposalSessionStep;
        pub fn plan_pbft_manager_broadcast(
            fact: PbftManagerBroadcastFact,
        ) -> PbftManagerBroadcastPlan;
        pub fn report_pbft_manager_broadcast(
            plan: PbftManagerBroadcastPlan,
            report: PbftManagerBroadcastReport,
        ) -> PbftManagerBroadcastReportResult;
        pub fn plan_pbft_manager_block_validation(
            fact: PbftManagerBlockValidationFact,
        ) -> PbftManagerBlockValidationPlan;
        type BridgePbftManagerBlockValidationSession;
        pub fn create_pbft_manager_block_validation_session(
            fact: PbftManagerBlockValidationFact,
        ) -> Box<BridgePbftManagerBlockValidationSession>;
        pub fn pbft_manager_block_validation_session_next(
            session: &mut BridgePbftManagerBlockValidationSession,
        ) -> PbftManagerBlockValidationPlan;
        pub fn pbft_manager_block_validation_session_report(
            session: &mut BridgePbftManagerBlockValidationSession,
            status: u8,
            dag_weight_check_required: bool,
        ) -> PbftManagerBlockValidationPlan;
        pub fn plan_pbft_manager_candidate_admission(
            fact: PbftManagerCandidateAdmissionFact,
        ) -> PbftManagerCandidateAdmissionPlan;
        pub fn plan_pbft_manager_leader_candidates(
            candidates: Vec<PbftManagerLeaderCandidateInputFact>,
        ) -> PbftManagerLeaderCandidatePlan;
        pub fn plan_pbft_manager_transition(
            fact: PbftManagerTransitionFact,
        ) -> PbftManagerTransitionPlan;
        pub fn create_pbft_finalization_resume_runtime_session(
            plan: &PbftFinalizationResumePlan,
        ) -> Box<BridgePbftFinalizationRuntimeSession>;
        pub fn pbft_finalization_runtime_session_next(
            self: &mut BridgePbftFinalizationRuntimeSession,
        ) -> PbftFinalizationRuntimeSessionStep;
        pub fn pbft_finalization_runtime_session_report(
            self: &mut BridgePbftFinalizationRuntimeSession,
            cursor: u32,
            action: u8,
            success: bool,
            action_status: u8,
        ) -> PbftFinalizationRuntimeSessionStep;
        pub fn pbft_finalization_runtime_session_report_action(
            self: &mut BridgePbftFinalizationRuntimeSession,
            report: PbftFinalizationRuntimeActionReport,
        ) -> PbftFinalizationRuntimeSessionStep;
        pub fn pbft_manager_runtime_session_next(
            self: &mut BridgePbftManagerRuntimeSession,
        ) -> PbftManagerRuntimeSessionStep;
        pub fn pbft_manager_runtime_session_report(
            self: &mut BridgePbftManagerRuntimeSession,
            report: PbftManagerRuntimeActionReport,
        ) -> PbftManagerRuntimeSessionStep;
        pub fn abort_pbft_finalization_runtime_session(
            self: &mut BridgePbftFinalizationRuntimeSession,
        );
        pub fn abort_pbft_manager_runtime_session(self: &mut BridgePbftManagerRuntimeSession);
        pub fn pbft_manager_state_action_effect_session_next(
            self: &mut BridgePbftManagerStateActionEffectSession,
        ) -> PbftManagerStateActionSessionStep;
        pub fn pbft_manager_state_action_effect_session_report(
            self: &mut BridgePbftManagerStateActionEffectSession,
            report: PbftManagerStateActionEffectReport,
        ) -> PbftManagerStateActionSessionStep;
        pub fn abort_pbft_manager_state_action_effect_session(
            self: &mut BridgePbftManagerStateActionEffectSession,
        );
        pub fn pbft_manager_proposal_session_next(
            self: &mut BridgePbftManagerProposalSession,
        ) -> PbftManagerProposalSessionStep;
        pub fn pbft_manager_proposal_session_report_dag_order(
            self: &mut BridgePbftManagerProposalSession,
            report: PbftManagerProposalDagOrderReport,
        ) -> PbftManagerProposalSessionStep;
        pub fn abort_pbft_manager_proposal_session(
            self: &mut BridgePbftManagerProposalSession,
        ) -> PbftManagerProposalSessionStep;
        pub fn pbft_manager_block_validation_session_next(
            self: &mut BridgePbftManagerBlockValidationSession,
        ) -> PbftManagerBlockValidationPlan;
        pub fn pbft_manager_block_validation_session_report(
            self: &mut BridgePbftManagerBlockValidationSession,
            status: u8,
            dag_weight_check_required: bool,
        ) -> PbftManagerBlockValidationPlan;
        pub fn validate_pbft_finalization_live_mutation_report(
            plan: &PbftFinalizationIntentPlan,
            report: PbftFinalizationLiveMutationReport,
        ) -> PbftFinalizationLiveMutationValidation;
        pub fn inspect_pbft_finalization_resume(
            storage: &BridgeStorage,
            write_set: &PbftFinalizationStorageWritePlan,
            final_chain_last_block: u64,
        ) -> Result<PbftFinalizationResumePlan>;
        pub fn plan_pbft_dynamic_lambda(fact: PbftDynamicLambdaFact) -> PbftDynamicLambdaPlan;
        pub fn load_pbft_finalization_last_period_lambda_storage(
            storage: &BridgeStorage,
            period: u64,
        ) -> Result<PeriodLambda>;
        pub fn apply_pbft_finalization_storage_writes(
            storage: &BridgeStorage,
            write_set: &PbftFinalizationStorageWritePlan,
            stages: Vec<PbftFinalizationStorageWriteStage>,
            sync: bool,
        ) -> Result<PbftFinalizedPeriodApplyResult>;

        // Consensus proposed PBFT blocks

        type BridgeProposedBlocks;

        pub fn create_proposed_blocks_index() -> Box<BridgeProposedBlocks>;
        pub fn create_proposed_blocks_index_from_storage(
            storage: &BridgeStorage,
        ) -> Box<BridgeProposedBlocks>;
        pub fn proposed_blocks_push(
            self: &mut BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
            pivot_hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> bool;
        pub fn proposed_blocks_push_with_storage(
            self: &mut BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
            pivot_hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<bool>;
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
        pub fn proposed_blocks_metadata(
            self: &BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
        ) -> ProposedBlockMetadataLookup;
        pub fn proposed_blocks_contains(
            self: &BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
        ) -> bool;
        pub fn proposed_blocks_cleanup_candidates(
            self: &BridgeProposedBlocks,
            period: u64,
        ) -> Vec<ProposedBlockPeriodHashes>;
        pub fn proposed_blocks_restore_from_storage(
            self: &mut BridgeProposedBlocks,
        ) -> Result<usize>;
        pub fn proposed_blocks_storage_snapshot_entries(
            self: &BridgeProposedBlocks,
        ) -> Result<Vec<ProposedBlockSnapshotEntry>>;
        pub fn proposed_blocks_cleanup_with_storage(
            self: &mut BridgeProposedBlocks,
            period: u64,
        ) -> Result<Vec<ProposedBlockPeriodHashes>>;
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

        // Consensus rewards stats

        type BridgeRewardsStatsRuntime;

        pub fn create_rewards_stats_runtime(
            storage: &BridgeStorage,
            config: RewardsStatsConfig,
            frequency_rules: Vec<RewardsFrequencyRule>,
            last_block_number: u64,
        ) -> Result<Box<BridgeRewardsStatsRuntime>>;
        pub fn process_finalized_period_rewards_stats(
            self: &mut BridgeRewardsStatsRuntime,
            fact: RewardsStatsProcessFact,
        ) -> RewardsStatsProcessResult;
        pub fn rewards_stats_runtime_clear_committed(
            self: &mut BridgeRewardsStatsRuntime,
            current_period: u64,
        );
        pub fn rewards_stats_runtime_cached_stats(
            self: &BridgeRewardsStatsRuntime,
        ) -> Vec<PeriodRlp>;
        pub fn rewards_stats_runtime_apply_storage_writes(
            self: &BridgeRewardsStatsRuntime,
            plan: &RewardsStatsProcessResult,
            sync: bool,
        ) -> Result<RewardsStatsApplyResult>;
        pub fn rewards_stats_runtime_clear_storage_and_state(
            self: &mut BridgeRewardsStatsRuntime,
            current_period: u64,
            sync: bool,
        ) -> Result<RewardsStatsApplyResult>;
        pub fn apply_rewards_stats_storage_writes(
            storage: &BridgeStorage,
            plan: &RewardsStatsProcessResult,
            sync: bool,
        ) -> Result<RewardsStatsApplyResult>;

        // Consensus period-data queue

        type BridgePeriodDataQueue;

        pub fn create_period_data_queue() -> Box<BridgePeriodDataQueue>;
        pub fn period_data_queue_period(self: &BridgePeriodDataQueue) -> u64;
        pub fn period_data_queue_syncing_period(
            self: &BridgePeriodDataQueue,
            pbft_chain_size: u64,
        ) -> u64;
        pub fn period_data_queue_last_block_hash_or_chain(
            self: &BridgePeriodDataQueue,
            current_period: u64,
            chain_last_hash: [u8; 32],
        ) -> [u8; 32];
        pub fn period_data_queue_size(self: &BridgePeriodDataQueue) -> usize;
        pub fn period_data_queue_empty(self: &BridgePeriodDataQueue) -> bool;
        pub fn period_data_queue_clear(self: &mut BridgePeriodDataQueue);
        pub fn period_data_queue_push(
            self: &mut BridgePeriodDataQueue,
            entry_id: u64,
            period: u64,
            block_hash: [u8; 32],
            prev_block_hash: [u8; 32],
            pivot_hash: [u8; 32],
            final_chain_hash: [u8; 32],
            reward_vote_hashes: Vec<PbftSyncTransactionHash>,
            pillar_vote_rlps: Vec<PeriodDataQueuePillarVotePayload>,
            transaction_rlps: Vec<PeriodDataQueueTransactionPayload>,
            previous_cert_vote_rlps: Vec<PeriodDataQueuePbftVotePayload>,
            dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
            period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
            period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
            previous_cert_votes_present: bool,
            previous_cert_first_vote_has_weight: bool,
            pillar_votes_present: bool,
            extra_data_present: bool,
            extra_data_pillar_block_hash_present: bool,
            max_pbft_size: u64,
            current_block_cert_vote_rlps: Vec<PeriodDataQueuePbftVotePayload>,
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
        pub fn transaction_queue_purge_with_final_chain(
            self: &mut BridgeTransactionQueue,
            final_chain: &BridgeFinalChain,
        ) -> Result<TransactionQueuePurgePlan>;
        pub fn transaction_queue_non_proposable_over_limit(self: &BridgeTransactionQueue) -> bool;
        pub fn transaction_queue_min_gas_price_for_block_inclusion(
            self: &BridgeTransactionQueue,
            limit: u64,
        ) -> [u8; 32];

        // Consensus gas pricer

        type BridgeGasPricer;

        pub fn create_gas_pricer(config: GasPricerConfig) -> Result<Box<BridgeGasPricer>>;
        pub fn create_gas_pricer_from_storage(
            config: GasPricerConfig,
            storage: &BridgeStorage,
        ) -> Result<Box<BridgeGasPricer>>;
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

        type BridgeTransactionManagerSidecar;
        type BridgeTransactionManagerRuntime;
        type BridgeTransactionManagerAdmissionExecution;

        pub fn create_transaction_manager_sidecar(
            initial_transaction_count: u64,
        ) -> Box<BridgeTransactionManagerSidecar>;
        pub fn create_transaction_manager_runtime(
            initial_transaction_count: u64,
            config: TransactionQueueConfig,
        ) -> Box<BridgeTransactionManagerRuntime>;
        pub fn create_transaction_manager_runtime_from_storage(
            storage: &BridgeStorage,
            initial_transaction_count: u64,
            config: TransactionQueueConfig,
        ) -> Box<BridgeTransactionManagerRuntime>;
        pub fn transaction_manager_runtime_pack_begin(
            self: &mut BridgeTransactionManagerRuntime,
            weight_limit: u64,
            min_transaction_gas: u64,
            proposal_period: u64,
            estimate_gas_limit: u64,
            last_block_number: u64,
        ) -> Result<()>;
        pub fn transaction_manager_runtime_pack_begin_sharded(
            self: &mut BridgeTransactionManagerRuntime,
            weight_limit: u64,
            min_transaction_gas: u64,
            proposal_period: u64,
            estimate_gas_limit: u64,
            last_block_number: u64,
            total_shards: u16,
            node_shard: u16,
            shard_period_interval: u64,
        ) -> Result<()>;
        pub fn transaction_manager_runtime_pack_request_next(
            self: &mut BridgeTransactionManagerRuntime,
        ) -> Result<TransactionPackSessionStep>;
        pub fn transaction_manager_runtime_pack_record_estimate_step(
            self: &mut BridgeTransactionManagerRuntime,
            input: TransactionPackSessionEstimateInput,
        ) -> Result<TransactionPackSessionStep>;
        pub fn transaction_manager_runtime_pack_abort(
            self: &mut BridgeTransactionManagerRuntime,
        ) -> bool;
        pub fn transaction_manager_runtime_plan_gas_estimation(
            self: &BridgeTransactionManagerRuntime,
            fact: TransactionManagerGasEstimationFact,
        ) -> Result<TransactionManagerGasEstimationPlan>;
        pub fn transaction_manager_runtime_store_gas_estimation(
            self: &mut BridgeTransactionManagerRuntime,
            result: TransactionManagerGasEstimationResult,
        ) -> Result<bool>;
        pub fn transaction_manager_runtime_gas_estimation_cache_size(
            self: &BridgeTransactionManagerRuntime,
        ) -> usize;
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
        pub fn transaction_manager_runtime_insert_transaction_precheck(
            self: &BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> Result<TransactionManagerInsertTransactionOutcome>;
        pub fn transaction_manager_runtime_finish_insert_transaction(
            self: &BridgeTransactionManagerRuntime,
            fact: TransactionManagerInsertTransactionFact,
        ) -> Result<TransactionManagerInsertTransactionOutcome>;
        /// Executes FinalChain-backed admission and returns a typed command report.
        pub fn transaction_manager_runtime_execute_transaction_admission_with_final_chain_command_report(
            self: &mut BridgeTransactionManagerRuntime,
            final_chain: &BridgeFinalChain,
            fact: TransactionManagerValidatedInsertRuntimeFact,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionManagerAdmissionCommandReport>;
        /// Executes admission with FinalChain facts supplied by the C++ external-EVM boundary.
        pub fn transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts_command_report(
            self: &mut BridgeTransactionManagerRuntime,
            fact: TransactionManagerValidatedInsertRuntimeFact,
            final_chain_fact: TransactionManagerFinalChainAdmissionFact,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionManagerAdmissionCommandReport>;
        /// Executes public insertTransaction verification and admission as one Rust-owned command.
        pub fn transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_command_report(
            self: &mut BridgeTransactionManagerRuntime,
            final_chain: &BridgeFinalChain,
            verify_fact: TransactionManagerVerifyTransactionFact,
            admission_fact: TransactionManagerValidatedInsertRuntimeFact,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionManagerPublicAdmissionCommandReport>;
        /// Executes public insertTransaction verification and fact-backed admission as one Rust-owned command.
        pub fn transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_facts_command_report(
            self: &mut BridgeTransactionManagerRuntime,
            verify_fact: TransactionManagerVerifyTransactionFact,
            admission_fact: TransactionManagerValidatedInsertRuntimeFact,
            final_chain_fact: TransactionManagerFinalChainAdmissionFact,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionManagerPublicAdmissionCommandReport>;
        pub fn transaction_manager_runtime_queue_erase(
            self: &mut BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> bool;
        pub fn transaction_manager_runtime_queue_get_transaction(
            self: &BridgeTransactionManagerRuntime,
            hash: &[u8; 32],
        ) -> TransactionQueueStoredTransaction;
        /// Resolves requested hashes against Rust-owned live queue payloads only.
        pub fn transaction_manager_runtime_queue_lookup_transaction_views(
            self: &BridgeTransactionManagerRuntime,
            requests: Vec<TransactionManagerTransactionViewRequest>,
        ) -> Result<Vec<TransactionManagerTransactionView>>;
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
        pub fn transaction_manager_runtime_queue_cleanup_with_final_chain(
            self: &mut BridgeTransactionManagerRuntime,
            final_chain: &BridgeFinalChain,
            apply_block_finalized: bool,
            block_number: u64,
        ) -> Result<TransactionManagerRuntimeQueueCleanupPlan>;
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
        /// Resolves requested hashes against non-finalized/recently-finalized sidecars.
        pub fn transaction_manager_runtime_lookup_non_finalized_transaction_views(
            self: &BridgeTransactionManagerRuntime,
            requests: Vec<TransactionManagerTransactionViewRequest>,
        ) -> Result<Vec<TransactionManagerTransactionView>>;
        /// Resolves requested hashes through queue, sidecars, then Rust storage.
        pub fn transaction_manager_runtime_lookup_transaction_views(
            self: &BridgeTransactionManagerRuntime,
            requests: Vec<TransactionManagerTransactionViewRequest>,
            max_count: u64,
        ) -> Result<TransactionManagerTransactionViewPlan>;
        /// Resolves requested hashes through queue, sidecars, then proposal-filtered Rust storage.
        pub fn transaction_manager_runtime_lookup_proposal_transaction_views(
            self: &BridgeTransactionManagerRuntime,
            final_chain: &BridgeFinalChain,
            proposal_period: u64,
            requests: Vec<TransactionManagerTransactionViewRequest>,
            max_count: u64,
        ) -> Result<TransactionManagerTransactionViewPlan>;
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
            facts: Vec<DagTransactionSaveSidecarFact>,
        ) -> Result<DagTransactionSaveOutcome>;
        /// Applies DAG transaction persistence and returns a typed command report.
        pub fn save_transactions_from_dag_block_command_report_with_runtime(
            runtime: &mut BridgeTransactionManagerRuntime,
            facts: Vec<DagTransactionSaveSidecarFact>,
        ) -> Result<TransactionManagerDagSaveCommandReport>;
        pub fn save_transactions_from_dag_block_with_runtime_and_final_chain(
            runtime: &mut BridgeTransactionManagerRuntime,
            final_chain: &BridgeFinalChain,
            facts: Vec<DagTransactionSaveRuntimeFact>,
        ) -> Result<DagTransactionSaveOutcome>;
        /// Applies DAG transaction persistence and returns a typed command report.
        pub fn save_transactions_from_dag_block_command_report_with_runtime_and_final_chain(
            runtime: &mut BridgeTransactionManagerRuntime,
            final_chain: &BridgeFinalChain,
            facts: Vec<DagTransactionSaveRuntimeFact>,
        ) -> Result<TransactionManagerDagSaveCommandReport>;
        /// Executes runtime admission planning and returns an explicit commit script.
        pub fn transaction_manager_runtime_execute_admission(
            runtime: &BridgeTransactionManagerRuntime,
            facts: Vec<DagTransactionSaveSidecarFact>,
        ) -> Result<Box<BridgeTransactionManagerAdmissionExecution>>;
        /// Commits one runtime admission script with storage-first ordering.
        pub fn transaction_manager_runtime_commit_admission(
            runtime: &mut BridgeTransactionManagerRuntime,
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
            period: u64,
            retention_window: u64,
            facts: Vec<FinalizedTransactionStatusSidecarFact>,
        ) -> Result<FinalizedTransactionStatusPlan>;
        /// Applies finalized status updates and returns a typed command report.
        pub fn update_finalized_transactions_status_command_report_with_runtime(
            runtime: &mut BridgeTransactionManagerRuntime,
            period: u64,
            retention_window: u64,
            facts: Vec<FinalizedTransactionStatusSidecarFact>,
        ) -> Result<TransactionManagerFinalizedStatusCommandReport>;
        /// Applies finalized status updates plus periodic purge and returns a typed command report.
        pub fn update_finalized_transactions_status_command_report_with_runtime_and_final_chain(
            runtime: &mut BridgeTransactionManagerRuntime,
            final_chain: &BridgeFinalChain,
            period: u64,
            retention_window: u64,
            facts: Vec<FinalizedTransactionStatusSidecarFact>,
        ) -> Result<TransactionManagerFinalizedStatusCommandReport>;
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
        pub fn transaction_manager_filter_non_finalized_with_runtime(
            runtime: &BridgeTransactionManagerRuntime,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<FinalizedTransactionFilterPlan>;
        pub fn transaction_manager_verify_not_finalized_with_runtime_and_final_chain(
            runtime: &BridgeTransactionManagerRuntime,
            final_chain: &BridgeFinalChain,
            facts: Vec<TransactionManagerVerifyNotFinalizedRuntimeFact>,
        ) -> Result<TransactionManagerVerifyNotFinalizedOutcome>;
        pub fn transaction_manager_verify_not_finalized_with_runtime(
            runtime: &BridgeTransactionManagerRuntime,
            facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
        ) -> Result<TransactionManagerVerifyNotFinalizedOutcome>;
        /// Resolves transaction hashes through TransactionManager storage rules.
        pub fn transaction_manager_load_stored_transactions(
            storage: &BridgeStorage,
            requests: Vec<TransactionManagerStoredTransactionRequest>,
        ) -> Result<Vec<TransactionManagerStoredTransactionLookup>>;
        /// Resolves storage transactions and applies proposal-period FinalChain account filtering.
        pub fn transaction_manager_load_proposal_transactions_with_final_chain(
            storage: &BridgeStorage,
            final_chain: &BridgeFinalChain,
            proposal_period: u64,
            requests: Vec<TransactionManagerStoredTransactionRequest>,
        ) -> Result<Vec<TransactionManagerStoredTransactionLookup>>;
        /// Returns persisted non-finalized transaction payloads for TransactionManager recovery.
        pub fn transaction_manager_load_nonfinalized_recovery(
            storage: &BridgeStorage,
        ) -> Result<Vec<TransactionManagerRecoveryEntry>>;
        /// Returns Rust-validated sidecar recovery inputs for TransactionManager startup recovery.
        pub fn transaction_manager_load_nonfinalized_recovery_inputs(
            storage: &BridgeStorage,
        ) -> Result<Vec<TransactionManagerSidecarRecoveryInsertInput>>;
        /// Rebuilds runtime recovery sidecars from Rust-backed storage.
        pub fn transaction_manager_recover_nonfinalized_with_runtime(
            runtime: &mut BridgeTransactionManagerRuntime,
        ) -> Result<()>;

        // Consensus verified votes

        type BridgeVerifiedVotes;

        pub fn create_verified_votes_index() -> Box<BridgeVerifiedVotes>;
        pub fn verified_votes_attach_storage(
            self: &mut BridgeVerifiedVotes,
            storage: &BridgeStorage,
        );
        pub fn verified_votes_size(self: &BridgeVerifiedVotes) -> u64;
        pub fn verified_votes_replay_contains(
            self: &BridgeVerifiedVotes,
            vote_hash: &[u8; 32],
        ) -> bool;
        pub fn verified_votes_replay_insert(
            self: &mut BridgeVerifiedVotes,
            vote_hash: &[u8; 32],
        ) -> bool;
        pub fn verified_votes_two_t_plus_one_threshold(
            self: &mut BridgeVerifiedVotes,
            fact: PbftTwoTPlusOneThresholdFact,
        ) -> PbftTwoTPlusOneThresholdPlan;
        pub fn verified_votes_validate_canonical_vote(
            self: &mut BridgeVerifiedVotes,
            canonical_vote_rlp: &[u8],
            validation_facts: PbftVoteValidationExternalFacts,
        ) -> Result<PbftVoteRuntimeValidationResult>;
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
        pub fn verified_votes_get_two_t_plus_one_voted_block_payloads(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
            kind: u8,
        ) -> Result<TwoTPlusOneVotePayloadsLookup>;
        pub fn verified_votes_plan_next_votes_bundle_egress(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
        ) -> PbftNextVotesBundleEgressPlan;
        pub fn verified_votes_build_optimized_votes_bundle_egress(
            self: &BridgeVerifiedVotes,
            request: PbftOptimizedVoteBundleBuildRequest,
        ) -> PbftOptimizedVoteBundleBuildResult;
        pub fn verified_votes_get_step_votes(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
            step: u64,
        ) -> VerifiedStepVotesLookup;
        pub fn verified_votes_cleanup_votes_by_period(
            self: &mut BridgeVerifiedVotes,
            pbft_period: u64,
        );
        pub fn verified_votes_add_verified_vote(
            self: &mut BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
            two_t_plus_one_threshold: u64,
            apply_threshold_decision: bool,
        ) -> Result<VerifiedVoteAddOutcome>;
        pub fn verified_votes_admit_validated_vote(
            self: &mut BridgeVerifiedVotes,
            canonical_vote_rlp: &[u8],
            validation_facts: PbftVoteValidationExternalFacts,
            flags: PbftVoteEventFactFlags,
            context: PbftVoteProgressContext,
        ) -> Result<PbftVoteAdmissionRuntimeResult>;
        pub fn verified_votes_snapshot_votes(
            self: &BridgeVerifiedVotes,
        ) -> Vec<VerifiedVotePayload>;
        pub fn verified_votes_snapshot_weighted_payloads(
            self: &BridgeVerifiedVotes,
        ) -> Vec<PbftVoteStorageRecord>;
        pub fn verified_votes_weighted_payload(
            self: &BridgeVerifiedVotes,
            vote_hash: &[u8; 32],
        ) -> PbftVotePayloadLookup;
        pub fn verified_votes_select_reward_vote_payloads(
            self: &BridgeVerifiedVotes,
            block_period: u64,
            reward_period: u64,
            preferred_reward_round: u64,
            reward_block_hash: &[u8; 32],
            requested_vote_hashes: Vec<PbftFinalizationHash>,
        ) -> Result<PbftRewardVotePayloadSelection>;
        pub fn verified_votes_snapshot_two_t_plus_one(
            self: &BridgeVerifiedVotes,
        ) -> Vec<TwoTPlusOneSnapshotEntry>;
        pub fn verified_votes_snapshot_round_markers(
            self: &BridgeVerifiedVotes,
        ) -> Vec<RoundMarkerSnapshot>;
        pub fn verified_votes_save_own_verified_vote(
            self: &BridgeVerifiedVotes,
            record: PbftVoteStorageRecord,
        ) -> Result<PbftVotePersistenceResult>;
        pub fn verified_votes_clear_own_verified_votes(
            self: &BridgeVerifiedVotes,
            hashes: Vec<PbftFinalizationHash>,
        ) -> Result<PbftVotePersistenceResult>;
        pub fn verified_votes_persist_pbft_vote_progress(
            self: &BridgeVerifiedVotes,
            write: PbftVoteProgressPersistenceWrite,
        ) -> Result<PbftVotePersistenceResult>;
        pub fn verified_votes_apply_pbft_finalization_storage_writes(
            self: &BridgeVerifiedVotes,
            write_intent: &PbftFinalizationStorageWritePlan,
            stages: Vec<PbftFinalizationStorageWriteStage>,
            sync: bool,
        ) -> Result<PbftFinalizedPeriodApplyResult>;

        // PBFT vote-progress protocol planner

        pub fn pbft_vote_progress_plan_precheck(
            fact: PbftVoteProgressFact,
            context: PbftVoteProgressContext,
        ) -> Result<PbftVoteProgressPrecheckPlan>;
        pub fn pbft_vote_progress_plan_after_add(
            fact: PbftVoteProgressFact,
            context: PbftVoteProgressContext,
            add_vote_outcome: VerifiedVoteAddOutcome,
        ) -> Result<PbftVoteProgressExecutionPlan>;
        pub fn pbft_vote_ingress_plan(
            fact: PbftVoteIngressFact,
            context: PbftVoteIngressContext,
        ) -> Result<PbftVoteIngressPlan>;
        pub fn pbft_vote_bundle_ingress_plan(
            reference: PbftVoteIngressFact,
            vote: PbftVoteIngressFact,
            context: PbftVoteIngressContext,
        ) -> Result<PbftVoteIngressPlan>;
        type BridgePbftVotePipelineSession;
        pub fn create_pbft_vote_pipeline_session(
            fact: PbftVoteProgressFact,
            context: PbftVoteProgressContext,
        ) -> Result<Box<BridgePbftVotePipelineSession>>;
        pub fn pbft_vote_pipeline_precheck(
            self: &mut BridgePbftVotePipelineSession,
        ) -> PbftVotePipelinePrecheckPlan;
        pub fn pbft_vote_pipeline_complete(
            self: &mut BridgePbftVotePipelineSession,
            add_vote_outcome: VerifiedVoteAddOutcome,
        ) -> PbftVotePipelineExecutionPlan;
        type BridgePbftVoteAdmissionSession;
        pub fn create_pbft_vote_admission_session(
            canonical_vote_rlp: &[u8],
            weight: u64,
            flags: PbftVoteEventFactFlags,
            context: PbftVoteProgressContext,
        ) -> Result<Box<BridgePbftVoteAdmissionSession>>;
        pub fn create_pbft_vote_admission_session_from_validation_facts(
            canonical_vote_rlp: &[u8],
            validation_facts: PbftVoteValidationExternalFacts,
            flags: PbftVoteEventFactFlags,
            context: PbftVoteProgressContext,
        ) -> Result<Box<BridgePbftVoteAdmissionSession>>;
        pub fn pbft_vote_admission_precheck(
            self: &mut BridgePbftVoteAdmissionSession,
        ) -> PbftVoteAdmissionPrecheckPlan;
        pub fn pbft_vote_admission_complete(
            self: &mut BridgePbftVoteAdmissionSession,
            add_vote_outcome: VerifiedVoteAddOutcome,
        ) -> PbftVoteAdmissionExecutionPlan;

        // PBFT reward-vote selection planner

        pub fn pbft_reward_votes_plan(
            fact: PbftRewardVoteSelectionFact,
        ) -> PbftRewardVoteSelectionPlan;

        // PBFT vote validation planner

        pub fn pbft_vote_sortition_threshold_for_bridge(
            total_dpos_vote_count: u64,
            vote_type: u8,
            committee_size: u64,
            number_of_proposers: u64,
        ) -> Result<u64>;
        type BridgePbftVoteValidationRuntime;
        pub fn create_pbft_vote_validation_runtime(
            max_size: usize,
            delete_step: usize,
        ) -> Box<BridgePbftVoteValidationRuntime>;
        pub fn pbft_vote_replay_contains(
            self: &BridgePbftVoteValidationRuntime,
            vote_hash: &[u8; 32],
        ) -> bool;
        pub fn pbft_vote_replay_insert(
            self: &BridgePbftVoteValidationRuntime,
            vote_hash: &[u8; 32],
        ) -> bool;
        pub fn pbft_two_t_plus_one_threshold(
            self: &BridgePbftVoteValidationRuntime,
            fact: PbftTwoTPlusOneThresholdFact,
        ) -> PbftTwoTPlusOneThresholdPlan;
        pub fn pbft_vote_validation_plan(
            fact: PbftVoteValidationFact,
        ) -> Result<PbftVoteValidationPlan>;
        pub fn pbft_inspect_canonical_vote(vote_rlp: &[u8]) -> Result<PbftCanonicalVoteInspection>;
        pub fn pbft_validate_canonical_vote(
            vote_rlp: &[u8],
            facts: PbftVoteValidationExternalFacts,
        ) -> Result<PbftCanonicalVoteValidation>;
        pub fn pbft_vote_event_fact_from_canonical_vote(
            canonical_vote_rlp: &[u8],
            weight: u64,
            flags: PbftVoteEventFactFlags,
        ) -> Result<PbftVoteEventFact>;
        pub fn pbft_derive_vote_progress_fact_from_canonical_vote(
            canonical_vote_rlp: &[u8],
            validation_facts: PbftVoteValidationExternalFacts,
            flags: PbftVoteEventFactFlags,
        ) -> Result<PbftVoteFactBoundaryResult>;
        pub fn pbft_generate_signed_vote(
            input: PbftVoteGenerationInput,
        ) -> Result<PbftGeneratedVote>;
        pub fn pbft_generate_signed_vote_with_weight(
            input: PbftVoteGenerationInput,
            facts: PbftVoteWeightFacts,
        ) -> Result<PbftGeneratedVote>;
        pub fn pbft_vote_weighted_payload_from_canonical_vote(
            canonical_vote_rlp: &[u8],
            weight: u64,
        ) -> Result<PbftVoteStorageRecord>;
        pub fn pbft_vote_slashing_payload_from_canonical_vote(
            canonical_vote_rlp: &[u8],
        ) -> Result<PbftVoteStorageRecord>;
        pub fn pbft_vote_bundle_payload_from_records(
            records: Vec<PbftVoteStorageRecord>,
        ) -> Result<Vec<u8>>;
        pub fn pbft_proposer_sortition_plan(
            fact: PbftProposerSortitionFact,
        ) -> Result<PbftProposerSortitionPlan>;

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

        /// Computes ordered validator vote-count changes for a pillar block.
        pub fn plan_pillar_vote_count_changes(
            current_vote_counts: Vec<PillarValidatorVoteCount>,
            previous_vote_counts: Vec<PillarValidatorVoteCount>,
        ) -> Result<Vec<PillarValidatorVoteCountChange>>;

        /// Validates pillar-block parent linkage.
        pub fn plan_pillar_block_linkage(
            fact: PillarBlockLinkageFact,
        ) -> Result<PillarBlockLinkagePlan>;
        pub fn plan_pillar_block_creation(
            fact: PillarBlockCreationFact,
        ) -> Result<PillarBlockCreationPlan>;

        type BridgePillarChainStorage;

        pub fn create_pillar_chain_storage(
            storage: &BridgeStorage,
        ) -> Box<BridgePillarChainStorage>;
        pub fn pillar_chain_storage_apply_current_block_data(
            self: &BridgePillarChainStorage,
            data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn pillar_chain_storage_apply_own_vote(
            self: &BridgePillarChainStorage,
            vote_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn pillar_chain_storage_apply_finalized_block(
            self: &BridgePillarChainStorage,
            period: u64,
            pillar_block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn pillar_chain_storage_load_own_vote(
            self: &BridgePillarChainStorage,
        ) -> Result<Vec<u8>>;
        pub fn pillar_chain_storage_load_current_block_data(
            self: &BridgePillarChainStorage,
        ) -> Result<Vec<u8>>;
        pub fn pillar_chain_storage_load_latest_block(
            self: &BridgePillarChainStorage,
        ) -> Result<Vec<u8>>;
        pub fn pillar_chain_storage_load_period_data(
            self: &BridgePillarChainStorage,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn pillar_chain_storage_load_block(
            self: &BridgePillarChainStorage,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn pillar_chain_storage_block_data_rlp(
            self: &BridgePillarChainStorage,
            period: u64,
        ) -> Result<Vec<u8>>;

        // Consensus sortition

        type BridgeSortitionParamsManager;

        pub fn create_sortition_params_manager(
            config: SortitionRuntimeConfig,
            params_changes: Vec<SortitionParamsChangePayload>,
        ) -> Result<Box<BridgeSortitionParamsManager>>;
        pub fn create_sortition_params_manager_from_storage(
            config: SortitionRuntimeConfig,
            storage: &BridgeStorage,
        ) -> Result<Box<BridgeSortitionParamsManager>>;
        pub fn sortition_current_params(
            self: &BridgeSortitionParamsManager,
        ) -> SortitionRuntimeParams;
        pub fn sortition_params_for_period(
            self: &BridgeSortitionParamsManager,
            found: bool,
            change: SortitionParamsChangePayload,
        ) -> SortitionRuntimeParams;
        pub fn sortition_params_for_period_from_storage(
            self: &BridgeSortitionParamsManager,
            period: u64,
        ) -> Result<SortitionRuntimeParams>;
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
        pub fn sortition_record_finalized_period_and_persist(
            self: &mut BridgeSortitionParamsManager,
            period: u64,
            has_pivot: bool,
            unique_transactions: u64,
            total_dag_transaction_refs: u64,
            non_empty_pbft_chain_size: u64,
        ) -> Result<SortitionParamsChangeResult>;
        pub fn sortition_preview_finalized_period(
            self: &BridgeSortitionParamsManager,
            period: u64,
            has_pivot: bool,
            unique_transactions: u64,
            total_dag_transaction_refs: u64,
            non_empty_pbft_chain_size: u64,
        ) -> Result<SortitionParamsChangeResult>;
        pub fn sortition_commit_finalized_period(
            self: &mut BridgeSortitionParamsManager,
            period: u64,
            has_pivot: bool,
            unique_transactions: u64,
            total_dag_transaction_refs: u64,
            non_empty_pbft_chain_size: u64,
            expected_changed: bool,
            expected_change: SortitionParamsChangePayload,
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
        type BridgeDagStorageQueries;
        type BridgeMetadataStorageQueries;
        type BridgePbftStorageQueries;
        type BridgePbftVoteStorageQueries;
        type BridgeTransactionStorageQueries;
        type BridgeFinalChainStorageQueries;
        type BridgePeriodStorageQueries;
        type BridgeStorageBatch;

        pub fn create_storage(path: &str) -> Result<Box<BridgeStorage>>;
        pub fn create_pbft_storage_queries(
            storage: &BridgeStorage,
        ) -> Box<BridgePbftStorageQueries>;
        pub fn create_metadata_storage_queries(
            storage: &BridgeStorage,
        ) -> Box<BridgeMetadataStorageQueries>;
        pub fn create_dag_storage_queries(storage: &BridgeStorage) -> Box<BridgeDagStorageQueries>;
        pub fn create_pbft_vote_storage_queries(
            storage: &BridgeStorage,
        ) -> Box<BridgePbftVoteStorageQueries>;
        pub fn create_transaction_storage_queries(
            storage: &BridgeStorage,
        ) -> Box<BridgeTransactionStorageQueries>;
        pub fn create_final_chain_storage_queries(
            storage: &BridgeStorage,
        ) -> Box<BridgeFinalChainStorageQueries>;
        pub fn create_period_storage_queries(
            storage: &BridgeStorage,
        ) -> Box<BridgePeriodStorageQueries>;
        pub fn create_storage_shim_batch(storage: &BridgeStorage) -> Box<BridgeStorageBatch>;
        pub fn storage_shim_save_status_field(
            batch: &mut BridgeStorageBatch,
            field: u8,
            value: u64,
        ) -> Result<()>;
        pub fn storage_shim_save_sortition_params_change(
            batch: &mut BridgeStorageBatch,
            period: u64,
            params_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn storage_shim_save_period_lambda(
            batch: &mut BridgeStorageBatch,
            period: u64,
            period_lambda: u32,
        ) -> Result<()>;
        pub fn storage_shim_save_rounds_count_dynamic_lambda(
            batch: &mut BridgeStorageBatch,
            rounds_count: u32,
        ) -> Result<()>;
        pub fn storage_shim_save_block_rewards_stats(
            batch: &mut BridgeStorageBatch,
            period: u64,
            stats_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn storage_shim_save_pbft_mgr_field(
            batch: &mut BridgeStorageBatch,
            field: u8,
            value: u32,
        ) -> Result<()>;
        pub fn storage_shim_save_pbft_mgr_status(
            batch: &mut BridgeStorageBatch,
            field: u8,
            value: bool,
        ) -> Result<()>;
        pub fn storage_shim_remove_cert_voted_block_in_round(
            batch: &mut BridgeStorageBatch,
        ) -> Result<()>;
        pub fn storage_shim_save_pbft_head(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
            head: Vec<u8>,
        ) -> Result<()>;
        pub fn storage_shim_remove_own_verified_vote(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
        ) -> Result<()>;
        pub fn storage_shim_replace_two_t_plus_one_votes(
            batch: &mut BridgeStorageBatch,
            vote_type: u8,
            votes_bundle_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn storage_shim_remove_extra_reward_vote(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
        ) -> Result<()>;
        pub fn storage_shim_save_pbft_block_period(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
            period: u64,
        ) -> Result<()>;
        pub fn storage_shim_save_dag_block_period(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
            period: u64,
            position: u32,
        ) -> Result<()>;
        pub fn storage_shim_save_dag_block(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
            level: u64,
            block_rlp: Vec<u8>,
            dag_blocks_count: u64,
            dag_edge_count: u64,
        ) -> Result<()>;
        pub fn storage_shim_update_dag_block_counters(
            batch: &mut BridgeStorageBatch,
            updates: Vec<DagCounterUpdate>,
            dag_blocks_count: u64,
            dag_edge_count: u64,
        ) -> Result<()>;
        pub fn storage_shim_remove_dag_block(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
        ) -> Result<()>;
        pub fn storage_shim_save_period_data(
            batch: &mut BridgeStorageBatch,
            period: u64,
            period_data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn storage_shim_remove_proposed_pbft_block(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
        ) -> Result<()>;
        pub fn storage_shim_save_proposal_period_dag_level(
            batch: &mut BridgeStorageBatch,
            level: u64,
            period: u64,
        ) -> Result<()>;
        pub fn storage_shim_save_transaction_location(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
            period: u64,
            position: u32,
            is_system: bool,
        ) -> Result<()>;
        pub fn storage_shim_save_transaction(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
            trx_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn storage_shim_remove_transaction(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
        ) -> Result<()>;
        pub fn storage_shim_save_system_transaction(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
            trx_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn storage_shim_save_period_system_transactions_hashes(
            batch: &mut BridgeStorageBatch,
            period: u64,
            hashes_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn storage_shim_commit_batch(batch: Box<BridgeStorageBatch>, sync: bool) -> Result<()>;

        pub fn save_dag_block(
            self: &BridgeStorage,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
            block_rlp: Vec<u8>,
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
        pub fn dag_block_in_db(self: &BridgeDagStorageQueries, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_dag_block(self: &BridgeDagStorageQueries, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_dag_block_period(
            self: &BridgeDagStorageQueries,
            hash: &[u8; 32],
        ) -> Result<BlockPeriod>;
        pub fn get_dag_block_period_lookup(
            self: &BridgeDagStorageQueries,
            hash: &[u8; 32],
        ) -> Result<BlockPeriodLookup>;
        pub fn get_last_blocks_level(self: &BridgeDagStorageQueries) -> Result<u64>;
        pub fn get_blocks_by_level(self: &BridgeDagStorageQueries, level: u64) -> Result<Vec<u8>>;
        pub fn get_dag_blocks_at_level(
            self: &BridgeDagStorageQueries,
            level: u64,
            number_of_levels: u32,
        ) -> Result<Vec<BlockRlp>>;
        pub fn get_nonfinalized_dag_blocks(
            self: &BridgeDagStorageQueries,
        ) -> Result<Vec<LevelBlocks>>;
        pub fn get_proposal_period_for_dag_level(
            self: &BridgeDagStorageQueries,
            level: u64,
        ) -> Result<PeriodLookup>;

        /// Typed period reads (preferred for typed query surfaces).
        pub fn get_period_data_raw(
            self: &BridgePeriodStorageQueries,
            period: u64,
        ) -> Result<Vec<u8>>;
        /// Typed period-by-PBFT-block hash lookup.
        pub fn get_period_from_pbft_hash(
            self: &BridgePeriodStorageQueries,
            hash: &[u8; 32],
        ) -> Result<PeriodLookup>;
        /// Typed by-period receipts lookup.
        pub fn get_block_receipt(self: &BridgePeriodStorageQueries, period: u64)
            -> Result<Vec<u8>>;
        pub fn seed_final_chain_conformance_lookup_rows(
            self: &BridgeStorage,
            meta_key: u32,
            meta_value: Vec<u8>,
            block_number: u64,
            block_hash: &[u8; 32],
            block_header_rlp: Vec<u8>,
            receipt_hash: &[u8; 32],
            receipt_rlp: Vec<u8>,
            blooms_chunk: &[u8; 32],
            blooms_rlp: Vec<u8>,
            receipt_period: u64,
            receipts_rlp: Vec<u8>,
        ) -> Result<()>;
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
        pub fn set_genesis_hash(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn save_status_field(self: &BridgeStorage, field: u8, value: u64) -> Result<()>;
        pub fn save_sortition_params_change(
            self: &BridgeStorage,
            period: u64,
            params_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_period_lambda(
            self: &BridgeStorage,
            period: u64,
            period_lambda: u32,
        ) -> Result<()>;
        pub fn save_rounds_count_dynamic_lambda(
            self: &BridgeStorage,
            rounds_count: u32,
        ) -> Result<()>;
        pub fn save_block_rewards_stats(
            self: &BridgeStorage,
            period: u64,
            stats_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn clear_block_rewards_stats(self: &BridgeStorage) -> Result<()>;

        pub fn get_genesis_hash(self: &BridgeMetadataStorageQueries) -> Result<Vec<u8>>;
        pub fn get_last_sortition_params(
            self: &BridgeMetadataStorageQueries,
            count: u64,
        ) -> Result<Vec<BlockRlp>>;
        pub fn get_params_change_for_period(
            self: &BridgeMetadataStorageQueries,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_status_field(self: &BridgeMetadataStorageQueries, field: u8) -> Result<u64>;
        pub fn get_period_lambda(
            self: &BridgeMetadataStorageQueries,
            period: u64,
            find_closest: bool,
        ) -> Result<PeriodLambda>;
        pub fn get_rounds_count_dynamic_lambda(self: &BridgeMetadataStorageQueries) -> Result<u32>;
        pub fn get_blocks_rewards_stats(
            self: &BridgeMetadataStorageQueries,
        ) -> Result<Vec<PeriodRlp>>;
        pub fn pbft_block_in_db(self: &BridgePbftStorageQueries, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_pbft_mgr_field(self: &BridgePbftStorageQueries, field: u8) -> Result<u32>;
        pub fn get_pbft_mgr_status(self: &BridgePbftStorageQueries, field: u8) -> Result<bool>;
        pub fn get_pbft_head(self: &BridgePbftStorageQueries, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_cert_voted_block_in_round(self: &BridgePbftStorageQueries) -> Result<Vec<u8>>;
        pub fn get_own_verified_votes(self: &BridgePbftVoteStorageQueries) -> Result<Vec<VoteRlp>>;
        pub fn get_all_two_t_plus_one_votes(
            self: &BridgePbftVoteStorageQueries,
        ) -> Result<Vec<VoteRlp>>;
        pub fn get_reward_votes(self: &BridgePbftVoteStorageQueries) -> Result<Vec<VoteRlp>>;
        pub fn save_cert_voted_block_in_round(
            self: &BridgeStorage,
            round: u64,
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
        pub fn persist_pbft_vote_progress(
            self: &BridgeStorage,
            write: PbftVoteProgressPersistenceWrite,
        ) -> Result<PbftVotePersistenceResult>;
        pub fn clear_own_verified_votes(
            self: &BridgeStorage,
            vote_hashes: Vec<PbftFinalizationHash>,
        ) -> Result<PbftVotePersistenceResult>;

        pub fn transaction_in_db(
            self: &BridgeTransactionStorageQueries,
            hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn transaction_finalized(
            self: &BridgeTransactionStorageQueries,
            hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn get_transaction_location(
            self: &BridgeTransactionStorageQueries,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_transaction(
            self: &BridgeTransactionStorageQueries,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_by_period_position(
            self: &BridgeTransactionStorageQueries,
            period: u64,
            position: u32,
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(
            self: &BridgeTransactionStorageQueries,
            period: u64,
        ) -> Result<u64>;
        pub fn get_system_transaction(
            self: &BridgeTransactionStorageQueries,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_all_nonfinalized_transactions(
            self: &BridgeTransactionStorageQueries,
        ) -> Result<Vec<TxRlp>>;
        /// Batch-fetches transaction RLP payloads by hash from Rust storage.
        pub fn get_transaction_rlps_by_hashes(
            self: &BridgeTransactionStorageQueries,
            hashes: Vec<DagTransactionHash>,
        ) -> Result<Vec<DagTransactionRlpLookup>>;
        pub fn get_all_transaction_period(
            self: &BridgeTransactionStorageQueries,
        ) -> Result<Vec<HashPeriod>>;
        pub fn get_period_system_transactions_hashes(
            self: &BridgeTransactionStorageQueries,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_meta_value(
            self: &BridgeFinalChainStorageQueries,
            key: u32,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_header(
            self: &BridgeFinalChainStorageQueries,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_hash_by_number(
            self: &BridgeFinalChainStorageQueries,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_number_by_hash(
            self: &BridgeFinalChainStorageQueries,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_log_blooms_chunk(
            self: &BridgeFinalChainStorageQueries,
            chunk_id: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_receipt_by_trx_hash(
            self: &BridgeFinalChainStorageQueries,
            trx_hash: &[u8; 32],
        ) -> Result<Vec<u8>>;

        // Transaction envelope

        pub fn inspect_legacy_transaction_rlp(
            tx_rlp: Vec<u8>,
            source: u8,
        ) -> Result<LegacyTransactionInspection>;

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

        pub fn create_final_chain_with_rewards_config(
            storage: &BridgeStorage,
            block_gas_limit: u64,
            genesis_timestamp: u64,
            genesis_accounts: Vec<GenesisAccount>,
            genesis_validators: Vec<GenesisValidator>,
            genesis_dpos_config: GenesisDposConfig,
            rewards_config: FinalChainRewardsConfig,
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
        pub fn get_execution_status(self: &BridgeFinalChain) -> Result<FinalChainExecutionStatus>;
        pub fn get_blocks_with_bloom(
            self: &BridgeFinalChain,
            bloom: &[u8; 256],
            from: u64,
            to: u64,
        ) -> Result<Vec<u64>>;
        pub fn get_account(self: &BridgeFinalChain, address: &[u8; 20]) -> Result<AccountLookup>;
        pub fn get_account_at_block(
            self: &BridgeFinalChain,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<AccountLookup>;
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
        pub fn get_dpos_total_amount_delegated(
            self: &BridgeFinalChain,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_dpos_yield(self: &BridgeFinalChain, block_number: u64) -> Result<u64>;
        pub fn get_dpos_total_supply(self: &BridgeFinalChain, block_number: u64)
            -> Result<Vec<u8>>;
        pub fn get_dpos_validators_eligible_vote_counts(
            self: &BridgeFinalChain,
            block_number: u64,
        ) -> Result<Vec<DposValidatorVoteCount>>;
        pub fn get_vrf_key(self: &BridgeFinalChain, address: &[u8; 20]) -> Result<Vec<u8>>;
        pub fn get_vrf_key_at_block(
            self: &BridgeFinalChain,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<Vec<u8>>;
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
        pub fn finalize_block_with_rewards_context(
            self: &BridgeFinalChain,
            pbft_block_rlp: Vec<u8>,
            transactions: Vec<FinalizationTransaction>,
            finalized_dag_blocks: Vec<FinalizationDagBlock>,
            blocks_per_year: u32,
        ) -> Result<FinalizationOutcome>;
        pub fn finalize_block_with_rewards_facts(
            self: &BridgeFinalChain,
            pbft_block_rlp: Vec<u8>,
            transactions: Vec<FinalizationTransaction>,
            finalized_dag_blocks: Vec<FinalizationDagBlock>,
            blocks_per_year: u32,
            cert_votes: Vec<RewardsCertVoteFact>,
        ) -> Result<FinalizationOutcome>;
        type BridgeFinalChainExecutionSession;
        pub fn create_final_chain_execution_session(
            final_chain: &BridgeFinalChain,
            request: FinalChainExecutionRequest,
        ) -> Result<Box<BridgeFinalChainExecutionSession>>;
        pub fn final_chain_execution_session_next(
            self: &mut BridgeFinalChainExecutionSession,
        ) -> Result<FinalChainExecutionStep>;
        pub fn final_chain_execution_session_report_evm(
            self: &mut BridgeFinalChainExecutionSession,
            report: FinalChainEvmExecutionReport,
        ) -> Result<FinalChainExecutionStep>;
        pub fn final_chain_execution_session_report_system_transactions(
            self: &mut BridgeFinalChainExecutionSession,
            report: FinalChainSystemTransactionReport,
        ) -> Result<FinalChainExecutionStep>;
        pub fn plan_external_evm_system_transactions(
            fact: FinalChainSystemTransactionPlanFact,
        ) -> Result<FinalChainSystemTransactionPlan>;
        pub fn final_chain_execution_session_plan_external_evm_commit(
            self: &mut BridgeFinalChainExecutionSession,
            rewards_report: FinalChainEvmRewardsReport,
        ) -> Result<FinalChainExternalEvmCommitPlan>;
        pub fn final_chain_execution_session_attach_external_evm_rewards_stats(
            self: &mut BridgeFinalChainExecutionSession,
            rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate,
        ) -> Result<FinalChainExternalEvmPublicationPlan>;
        pub fn final_chain_execution_session_attach_external_evm_proposal_period_dag_level(
            self: &mut BridgeFinalChainExecutionSession,
            update: FinalChainProposalPeriodDagLevelUpdate,
        ) -> Result<FinalChainExternalEvmPublicationPlan>;
        pub fn final_chain_execution_session_plan_external_evm_publication(
            final_chain: &BridgeFinalChain,
            session: &mut BridgeFinalChainExecutionSession,
        ) -> Result<FinalChainExternalEvmPublicationPlan>;
        pub fn final_chain_execution_session_publish_external_evm_publication(
            final_chain: &BridgeFinalChain,
            session: &mut BridgeFinalChainExecutionSession,
        ) -> Result<FinalChainExternalEvmPublicationReport>;
        pub fn final_chain_execution_session_persist_external_evm_pending_publication(
            final_chain: &BridgeFinalChain,
            session: &mut BridgeFinalChainExecutionSession,
        ) -> Result<FinalChainExternalEvmPublicationReport>;
        pub fn final_chain_execution_session_request_external_evm_state_commit(
            self: &mut BridgeFinalChainExecutionSession,
            request: FinalChainExternalEvmStateCommitRequest,
        ) -> Result<FinalChainExternalEvmStateCommitIntent>;
        pub fn final_chain_execution_session_report_external_evm_state_commit_result(
            final_chain: &BridgeFinalChain,
            session: &mut BridgeFinalChainExecutionSession,
            result: FinalChainExternalEvmStateCommitResult,
        ) -> Result<FinalChainExternalEvmCommitDecision>;
        pub fn final_chain_execution_session_report_external_evm_lifecycle(
            self: &mut BridgeFinalChainExecutionSession,
            report: FinalChainExternalEvmLifecycleReport,
        ) -> Result<FinalChainExternalEvmCommitDecision>;
        pub fn publish_external_evm_publication(
            self: &BridgeFinalChain,
            plan: FinalChainExternalEvmPublicationPlan,
            decision: FinalChainExternalEvmCommitDecision,
        ) -> Result<FinalChainExternalEvmPublicationReport>;
        pub fn recover_external_evm_pending_publication(
            self: &BridgeFinalChain,
            committed_period: u64,
            committed_state_root: &[u8; 32],
        ) -> Result<FinalChainExternalEvmPublicationReport>;
        pub fn final_chain_execution_session_commit(
            final_chain: &BridgeFinalChain,
            session: Box<BridgeFinalChainExecutionSession>,
        ) -> Result<FinalChainExecutionCommitReport>;
        pub fn abort_final_chain_execution_session(session: Box<BridgeFinalChainExecutionSession>);
        pub fn get_transaction_rlps(self: &BridgeFinalChain, period: u64) -> Result<Vec<TxRlp>>;
        pub fn get_transaction_receipt(
            self: &BridgeFinalChain,
            period: u64,
            position: u64,
        ) -> Result<Vec<u8>>;
        pub fn collect_pbft_final_chain_facts(
            self: &BridgeFinalChain,
            request: PbftFinalChainFactRequest,
        ) -> Result<PbftFinalChainFacts>;
    }
}
