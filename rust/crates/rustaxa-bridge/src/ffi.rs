use crate::dag::*;
use crate::dag_transaction_service::*;
use crate::final_chain::*;
use crate::network::*;
use crate::pbft_chain::*;
use crate::pbft_manager::*;
use crate::pbft_period_cleanup::*;
use crate::pbft_sync::*;
use crate::pbft_vote_generation::*;
use crate::pbft_vote_payload::*;
use crate::pbft_vote_validation::*;
use crate::pillar_chain::*;
use crate::pillar_votes::*;
use crate::proposed_blocks::*;
use crate::query::*;
use crate::rewards_stats::*;
use crate::slashing::*;
use crate::sortition::*;
use crate::storage::*;
use crate::transaction::*;
use crate::transaction_manager::*;
use crate::vdf::*;
use ethereum_types::H256;
use rustaxa_consensus::dag::DagManagerState;
use rustaxa_consensus::gas_pricer::GasPriceOracle;
use rustaxa_consensus::pbft_chain::PbftChain;
use rustaxa_consensus::period_data_queue::PeriodDataQueue;
use rustaxa_consensus::slashing::SlashingProofPlanner;
use rustaxa_consensus::sortition::SortitionParamsManager;
use rustaxa_consensus::transaction_manager::{
    TransactionManagerSidecar, TransactionPackingPlanner,
};
use rustaxa_consensus::transaction_queue::{TransactionQueue, TransactionQueueEntry};
use rustaxa_consensus::ConsensusExecutionApi;
use rustaxa_consensus::ConsensusNetworkApi;
use rustaxa_consensus::ConsensusQueryApi;
use rustaxa_consensus::FinalChain;
use rustaxa_consensus::PbftVoteAdmissionRuntime;
use rustaxa_consensus::PillarCurrentAnchor;
use rustaxa_consensus::PillarVotes;
use rustaxa_consensus::RewardsStatsRuntime;
use rustaxa_storage::Storage;
use rustaxa_storage::StorageWriteBatch;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::RwLock;
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

/// Rust-owned external EVM/StateAPI facade.
///
/// The facade is intentionally stateless. C++ passes the live FinalChain and
/// execution-session handles for each call while Rust owns request identity,
/// report validation, publication planning, storage publication, and audit
/// decisions.
pub struct BridgeConsensusExecutionApi(pub ConsensusExecutionApi);

/// Rust-owned public consensus query facade.
///
/// The facade owns only a cloned Rust storage handle and returns stable read
/// DTOs for public adapters. It does not expose consensus managers, storage
/// iterators, or mutable sidecars.
pub struct BridgeConsensusQueryApi(pub ConsensusQueryApi);

/// Rust-owned external network/tarcap facade.
///
/// The facade accepts canonical packet bytes and returns typed network effects
/// without exposing consensus managers or shim-owned compatibility state to the
/// network module.
pub struct BridgeConsensusNetworkApi {
    pub api: Mutex<ConsensusNetworkApi>,
}

/// Private DAG state owned by the application-level DAG/transaction service.
///
/// The state couples deterministic graphs and proposal/verification sessions
/// with the service's shared Rust storage. It never crosses CXX independently;
/// bridge callers reach it only through `BridgeDagTransactionService`.
pub(crate) struct DagRuntimeState {
    pub state: DagManagerState,
    pub storage: Arc<Storage>,
    pub next_proposer_session_id: u64,
    pub proposer_sessions: std::collections::BTreeMap<u64, crate::dag::DagProposerSession>,
    pub proposer_retry_states:
        std::collections::BTreeMap<[u8; 32], crate::dag::DagProposerRetryState>,
    pub verify_block_session: Option<crate::dag::DagVerifyBlockSession>,
}

/// PBFT chain runtime wrapper. Pure state-only instances are used by unit tests
/// and deterministic head transitions; storage-backed instances own the shared
/// Rust storage handle used for PBFT block lookup/materialization.
pub(crate) struct BridgePbftChainState {
    pub state: PbftChain,
    pub initialized_default: bool,
}

/// Rewards-stat runtime wrapper coupling deterministic in-memory state with
/// the shared Rust storage handle used for cache reload, write, and clear
/// operations.
pub struct BridgeRewardsStatsRuntime {
    pub state: RewardsStatsRuntime,
    pub storage: Arc<Storage>,
}

/// Pillar-chain storage wrapper used by the C++ manager shim.
///
/// The wrapper owns a cloned Rust storage handle so production pillar-chain
/// reads and writes do not retain or pass the generic `BridgeStorage` facade
/// after construction.
pub struct BridgePillarChainStorage {
    pub storage: Arc<Storage>,
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
pub(crate) struct BridgePbftManagerRuntimeState {
    pub state: rustaxa_consensus::pbft_manager::PbftManagerRuntime,
    pub storage: Arc<Storage>,
    pub period_data_queue: PeriodDataQueue,
    pub pbft_sync_queue_drain_session: rustaxa_consensus::pbft_sync::PbftSyncQueueDrainSession,
    pub pbft_sync_admission_session: Option<rustaxa_consensus::pbft_sync::PbftSyncAdmissionSession>,
    pub state_action_effect_session:
        Option<rustaxa_consensus::pbft_manager::PbftManagerStateActionEffectSession>,
    pub runtime_session: Option<rustaxa_consensus::pbft_manager::PbftManagerRuntimeSession>,
    pub proposal_session: Option<rustaxa_consensus::pbft_manager::PbftManagerProposalSession>,
    pub finalization_runtime_session:
        Option<rustaxa_consensus::pbft_finalize::PbftFinalizationRuntimeState>,
    pub finalization_runtime_plan: Option<rustaxa_consensus::pbft_finalize::PbftFinalizationPlan>,
    /// Process-local reset proof bound to the active finalization session.
    ///
    /// It is cleared when the session terminates. A same-process resume may
    /// preserve it only while it still matches the shared storage generation;
    /// process restart initializes it to zero and requires a new reset commit.
    pub finalization_reward_votes_reset_generation: u64,
    pub chain: Arc<RwLock<BridgePbftChainState>>,
}

/// Application-owned PBFT service shared by the C++ manager and chain facades.
///
/// The service is the sole Rust owner of PBFT manager/session state, PBFT chain
/// state, proposed-block state, and their common storage handle. Manager
/// commands are serialized by `manager`; chain and proposed-block reads use
/// independent sibling lock domains, and every verified-vote operation is
/// serialized by `verified_votes`. Operations that need both manager and chain
/// acquire `manager` before `chain`; no guard is retained across a C++ executor
/// call. A chain-only compatibility instance has neither manager nor verified-
/// vote state and is held privately by the C++ `PbftChain` adapter. Reaching
/// either unavailable receiver through that adapter is a bridge-wiring bug and
/// returns an explicit error.
pub struct BridgePbftService {
    pub(crate) manager: Mutex<Option<BridgePbftManagerRuntimeState>>,
    pub(crate) chain: Arc<RwLock<BridgePbftChainState>>,
    pub(crate) proposed_blocks: RwLock<rustaxa_consensus::proposed_blocks::ProposedBlocks>,
    pub(crate) verified_votes: Mutex<Option<PbftVoteAdmissionRuntime>>,
    pub(crate) storage: Option<Arc<Storage>>,
    pub(crate) bootstrap_complete: AtomicBool,
}

pub(crate) struct BridgePbftManagerGuard<'a>(MutexGuard<'a, Option<BridgePbftManagerRuntimeState>>);

impl std::ops::Deref for BridgePbftManagerGuard<'_> {
    type Target = BridgePbftManagerRuntimeState;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("PBFT manager operation requires a production service")
    }
}

impl std::ops::DerefMut for BridgePbftManagerGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
            .as_mut()
            .expect("PBFT manager operation requires a production service")
    }
}

impl BridgePbftService {
    pub(crate) fn manager_state(&self) -> BridgePbftManagerGuard<'_> {
        BridgePbftManagerGuard(self.manager.lock().expect("PBFT manager lock poisoned"))
    }

    pub(crate) fn accepts_live_commands(&self) -> bool {
        self.bootstrap_complete.load(Ordering::Acquire)
    }
}

pub struct BridgeSlashingProofPlanner(pub Mutex<SlashingProofPlanner>);

/// Rust-owned pillar-chain runtime used by the C++ PillarChainManager shim.
///
/// The runtime keeps pillar-vote aggregation and typed pillar-chain storage
/// together for operations that need both, avoiding ad hoc bridge-handle
/// composition in live consensus routes.
pub struct BridgePillarChainRuntime {
    pub storage: Arc<Storage>,
    pub votes: PillarVotes,
    /// Lock-protected canonical current-pillar snapshot restored at startup.
    ///
    /// The snapshot is published only after its canonical row is durably
    /// persisted. All runtime consumers bind their work to its generation.
    pub current_anchor: RwLock<PillarChainStateSnapshot>,
    /// One-time single-vote preparations keyed by canonical vote hash.
    ///
    /// Apply removes the entry before mutation. External entries are always
    /// relevance/identity checked again under the current-anchor read lock;
    /// trusted entries can only be created through the explicitly named
    /// local/restart restoration route.
    pub single_vote_preparations: Mutex<SingleVotePreparationRegistry>,
    /// One-time prepared pillar finalization payloads keyed by caller token.
    ///
    /// Entries are bounded, reused for identical prepare requests, and removed
    /// only after acknowledgement authenticates the matching durable row.
    /// Missing or mismatched durable rows retain the token for a safe retry.
    pub pillar_block_finalization_preparations:
        Mutex<HashMap<u64, PillarBlockFinalizationPreparation>>,
    /// Process-local finalization token sequence used by one-time prepare/ack.
    pub next_pillar_block_finalization_preparation_token: u64,
}

/// Runtime-internal one-time preparation state for one canonical pillar vote.
pub struct SingleVotePreparation {
    pub vote_rlp: Vec<u8>,
    pub anchor_generation: u64,
    pub period: u64,
    pub block_hash: H256,
    pub voter: ethereum_types::H160,
    pub needs_threshold: bool,
    pub current_anchor: Option<PillarCurrentAnchor>,
    pub first_pillar_block_period: u64,
    pub pillar_blocks_interval: u64,
    pub trusted_local_or_restore: bool,
}

/// Pending single-vote preparations retained between external DPoS lookup and apply.
pub struct SingleVotePreparationRegistry {
    pub entries: std::collections::BTreeMap<H256, SingleVotePreparation>,
}

/// One-time prepared payload used to acknowledge a pillar-block finalization.
pub struct PillarBlockFinalizationPreparation {
    /// Runtime generation observed when the finalization was prepared.
    pub anchor_generation: u64,
    /// Canonical pillar block period prepared for persistence.
    pub prepared_pillar_block_period: u64,
    /// Canonical pillar block RLP prepared for persistence.
    pub prepared_pillar_block_rlp: Vec<u8>,
    /// Vote-period cutoff for `PillarVotes::erase_votes` after acknowledgement.
    pub matching_vote_cleanup_min_period: u64,
    /// Whether compatibility event emission should run after ack.
    pub should_emit: bool,
}

/// Runtime-owned canonical pillar-chain snapshot.
///
/// The canonical current-data bytes retain validator vote counts for Rust-owned
/// creation planning and for temporary C++ current-block materialization. The
/// latest-finalized block remains decoded alongside its canonical bytes so
/// linkage and finalization never accept C++-supplied parent identity.
#[derive(Debug)]
pub struct PillarChainStateSnapshot {
    pub anchor: Option<PillarCurrentAnchor>,
    pub current_data_rlp: Vec<u8>,
    pub current_block_rlp: Vec<u8>,
    /// Canonical latest-finalized pillar block restored and updated by Rust.
    pub latest_finalized_block: Option<rustaxa_types::pillar::PillarBlock>,
    /// Canonical bytes for compatibility-only C++ materialization.
    pub latest_finalized_block_rlp: Vec<u8>,
    pub generation: u64,
}

/// Bridge wrapper for the Rust sortition parameter manager.
///
/// The manager owns deterministic threshold/runtime state. Production
/// Rust-mode constructors attach native Rust storage inside
/// `SortitionParamsManager`; compatibility constructors may remain storage-free
/// and pass explicit facts for unit-level planner tests.
pub struct BridgeSortitionParamsManager {
    pub manager: SortitionParamsManager,
}

/// Private transaction state owned by the application-level DAG/transaction service.
///
/// The state combines sidecars, queue, gas oracle, packing sessions, and a
/// shared Rust storage handle. It never crosses CXX independently; retained C++
/// facades reach it only through `BridgeDagTransactionService` and materialize
/// legacy `Transaction` objects at explicit public or EVM executor boundaries.
pub(crate) struct TransactionRuntimeState {
    pub sidecar: TransactionManagerSidecar,
    pub queue: TransactionQueue,
    pub gas_price_oracle: GasPriceOracle,
    pub proposal_dag_gas_limit: u64,
    pub storage: Option<Arc<Storage>>,
    pub last_drop_observed: Option<Instant>,
    pub transaction_pack_session: Option<TransactionManagerRuntimePackSession>,
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
    pub pending_estimate_candidates: Vec<TransactionQueueEntry>,
    pub pending_estimate_index: usize,
}

#[cxx::bridge(namespace = "rustaxa")]
pub mod rustaxa_ffi {
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

    /// Public/query JSON view for PBFT block extra data.
    struct PbftBlockExtraDataView {
        found: bool,
        major_version: u16,
        minor_version: u16,
        patch_version: u16,
        net_version: u16,
        node_implementation: String,
        has_pillar_block_hash: bool,
        pillar_block_hash: [u8; 32],
    }

    /// Public/query JSON view for `taraxa_getScheduleBlockByPeriod`.
    struct PbftScheduleBlockView {
        found: bool,
        prev_block_hash: [u8; 32],
        dag_block_hash_as_pivot: [u8; 32],
        order_hash: [u8; 32],
        final_chain_hash: [u8; 32],
        period: u64,
        timestamp: u64,
        block_hash: [u8; 32],
        signature: Vec<u8>,
        beneficiary: [u8; 20],
        reward_votes: Vec<PbftFinalizationHash>,
        has_extra_data: bool,
        extra_data: PbftBlockExtraDataView,
        dag_blocks_order: Vec<PbftFinalizationHash>,
    }

    /// PBFT block author/version facts for `taraxa_getNodeVersions`.
    struct PbftNodeVersionView {
        found: bool,
        beneficiary: [u8; 20],
        major_version: u16,
        minor_version: u16,
        patch_version: u16,
    }

    /// Canonical PBFT vote bytes for public/debug query materialization.
    struct PbftCertVoteRlp {
        vote_rlp: Vec<u8>,
    }

    /// Previous-block PBFT cert votes decoded from finalized period data.
    struct PbftPeriodCertVotesView {
        found: bool,
        period: u64,
        certified_period: u64,
        round: u64,
        step: u64,
        block_hash: [u8; 32],
        votes: Vec<PbftCertVoteRlp>,
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

    /// Storage-backed chain statistics for `taraxa_getChainStats`.
    struct ChainStatsView {
        pbft_period: u64,
        dag_blocks_count: u64,
        transactions_count: u64,
        dag_blocks_executed: u64,
        transactions_executed: u64,
    }

    /// Storage-backed finalized head and DAG index status facts.
    struct ConsensusStatusView {
        final_block_number: u64,
        latest_dag_level: u64,
        latest_dag_period_found: bool,
        latest_dag_period: u64,
    }

    /// Public/query sortition params-change view for Test RPC compatibility.
    struct SortitionParamsChangeView {
        found: bool,
        period: u64,
        interval_efficiency: u16,
        threshold_upper: u16,
        threshold_upper_min: u16,
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

    /// Hash handle used to map Rust queue decisions back to C++ live transactions.
    struct TransactionQueueHash {
        hash: [u8; 32],
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

    /// C++-supplied nonce fact for one proposable account.
    struct TransactionQueueAccountNonceFact {
        sender: [u8; 20],
        account_found: bool,
        account_nonce: [u8; 32],
    }

    /// Proposable account observed from queue state.
    struct TransactionQueueProposableAccountFact {
        sender: [u8; 20],
    }

    /// Result of accepting packet bytes at the Rust consensus ingress boundary.
    struct NetworkIngressReceipt {
        accepted: bool,
        payload_id: u64,
        status: u8,
        error_code: String,
    }

    /// Capacity limits for the external network/tarcap facade.
    struct NetworkApiConfig {
        max_payload_bytes: u64,
        max_retained_payloads: u64,
        max_effects_per_drain: u32,
    }

    /// Canonical packet bytes submitted by network/tarcap.
    struct NetworkIngressPacket {
        packet_type: u32,
        peer_id: [u8; 64],
        payload_bytes: Vec<u8>,
        received_at_mono_ms: u64,
        source_packet_id: u64,
    }

    /// Fixed-size peer id used by network effect payloads.
    struct NetworkPeerId {
        id: [u8; 64],
    }

    /// Executor-visible network effect planned by Rust consensus.
    struct NetworkEffect {
        effect_id: u64,
        source_payload_id: u64,
        kind: u8,
        peer_id: [u8; 64],
        packet_kind: u32,
        payload_bytes: Vec<u8>,
        exclude_peers: Vec<NetworkPeerId>,
        object_kind: u8,
        object_hash: [u8; 32],
        sync_kind: u8,
        sync_start: u64,
        reason_code: u8,
        dependency_id: u64,
        period: u64,
        round: u64,
    }

    /// Ordered network effects returned to network/tarcap for execution.
    struct NetworkEffectBatch {
        status: u8,
        effects: Vec<NetworkEffect>,
        more_available: bool,
        error_code: String,
    }

    /// Network/tarcap executor result for one effect.
    struct NetworkEffectResult {
        effect_id: u64,
        kind: u8,
        peer_id: [u8; 64],
        packet_kind: u32,
        object_kind: u8,
        object_hash: [u8; 32],
        status: u8,
        diagnostic: String,
    }

    /// Summary returned after Rust records network effect results.
    struct NetworkEffectAck {
        status: u8,
        accepted_results: u64,
        failed_results: u64,
        error_code: String,
    }

    /// Scalar context for authoritative PBFT vote ingress through Network/Tarcap.
    struct NetworkPbftVoteIngressContext {
        ingress: PbftVoteIngressContext,
        peer_id: [u8; 64],
        peer_pbft_chain_size: u64,
        source_payload_id: u64,
    }

    /// Packet-specific network ingress decision with queued-effect summary.
    struct NetworkIngressDecision {
        payload_id: u64,
        payload_accepted: bool,
        routed: bool,
        status: u8,
        error_code: String,
        queued_effect_count: u32,
    }

    /// Compact facts for status-triggered network sync planning.
    struct NetworkStatusSyncFacts {
        local_pbft_syncing: bool,
        local_pbft_synced_period: u64,
        local_pbft_period: u64,
        local_pbft_round: u64,
        peer_pbft_chain_size: u64,
        peer_pbft_period: u64,
        peer_pbft_round: u64,
        peer_dag_synced: bool,
        peer_last_status_pbft_chain_size: u64,
    }

    /// Side-effect-free status sync plan for tarcap execution.
    struct NetworkStatusSyncPlan {
        request_pbft_sync: bool,
        request_pending_dag_blocks: bool,
        request_next_votes: bool,
        next_votes_period: u64,
        next_votes_round: u64,
    }

    /// Compact facts needed to shape a local status packet for tarcap egress.
    struct NetworkStatusEgressFacts {
        initial: bool,
        local_chain_id: u64,
        genesis_hash: [u8; 32],
        node_major_version: u32,
        node_minor_version: u32,
        node_patch_version: u32,
        is_light_node: bool,
        light_node_history: u64,
        local_pbft_chain_size: u64,
        local_pbft_round: u64,
        local_dag_level: u64,
        pbft_syncing: bool,
        deep_pbft_syncing: bool,
    }

    /// Side-effect-free local status packet plan for tarcap egress.
    struct NetworkStatusEgressPlan {
        status: u8,
        error_code: String,
        peer_pbft_chain_size: u64,
        peer_pbft_round: u64,
        peer_dag_level: u64,
        peer_syncing: bool,
        include_initial_data: bool,
        chain_id: u64,
        genesis_hash: [u8; 32],
        node_major_version: u32,
        node_minor_version: u32,
        node_patch_version: u32,
        is_light_node: bool,
        light_node_history: u64,
    }

    /// Compact facts needed to validate an initial status packet.
    struct NetworkInitialStatusFacts {
        local_chain_id: u64,
        peer_chain_id: u64,
        expected_genesis_hash: [u8; 32],
        peer_genesis_hash: [u8; 32],
        local_pbft_synced_period: u64,
        peer_pbft_chain_size: u64,
        peer_is_light_node: bool,
        peer_light_node_history: u64,
    }

    /// Side-effect-free initial-status admission plan for tarcap execution.
    struct NetworkInitialStatusPlan {
        status: u8,
        error_code: String,
        accept_peer: bool,
        disconnect_peer: bool,
    }

    /// Compact peer candidate for PBFT sync-start planning.
    struct NetworkPbftSyncPeerCandidate {
        peer_id: [u8; 64],
        pbft_chain_size: u64,
        dag_level: u64,
        is_light_node: bool,
        light_node_history: u64,
        peer_dag_synced: bool,
        peer_dag_syncing: bool,
        dag_sync_allowed: bool,
    }

    /// Compact facts needed to plan PBFT sync start from known peers.
    struct NetworkPbftSyncStartFacts {
        local_pbft_syncing: bool,
        local_pbft_synced_period: u64,
        local_pbft_chain_size: u64,
        candidates: Vec<NetworkPbftSyncPeerCandidate>,
    }

    /// Side-effect-free PBFT sync-start plan for tarcap execution.
    struct NetworkPbftSyncStartPlan {
        status: u8,
        error_code: String,
        start_sync: bool,
        has_peer: bool,
        peer_id: [u8; 64],
        peer_pbft_chain_size: u64,
        request_period: u64,
        enable_snapshot_creation: bool,
    }

    /// Compact facts needed to select the best live network peer.
    struct NetworkPeerSelectionFacts {
        local_pbft_syncing_period: u64,
        candidates: Vec<NetworkPbftSyncPeerCandidate>,
    }

    /// Side-effect-free peer-selection plan for tarcap execution.
    struct NetworkPeerSelectionPlan {
        status: u8,
        error_code: String,
        has_peer: bool,
        peer_id: [u8; 64],
        peer_pbft_chain_size: u64,
    }

    /// Compact facts needed to plan a pending-DAG-block request.
    struct NetworkPendingDagBlocksRequestFacts {
        local_pbft_syncing_period: u64,
        has_explicit_peer: bool,
        explicit_peer: NetworkPbftSyncPeerCandidate,
        candidates: Vec<NetworkPbftSyncPeerCandidate>,
    }

    /// Side-effect-free pending-DAG request plan for tarcap execution.
    struct NetworkPendingDagBlocksRequestPlan {
        status: u8,
        error_code: String,
        request_pending_dag_blocks: bool,
        has_peer: bool,
        peer_id: [u8; 64],
        request_period: u64,
    }

    /// PBFT vote gossip request supplied after admission.
    struct NetworkPbftVoteGossipEffects {
        peer_id: [u8; 64],
        vote_hash: [u8; 32],
        source_payload_id: u64,
        gossip_vote: bool,
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

    /// One-shot packing plan returned before C++ estimates are executed.
    ///
    /// `estimate_requests` contains candidates that require live gas estimation.
    /// `selected_transactions` and `demoted_hashes` already include the
    /// candidates resolved via declared gas or cache hits.
    struct TransactionPackPreparedPlan {
        request_estimates: Vec<TransactionPackSessionCandidate>,
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

    /// C++-originated evidence for planning a double-voting proof transaction.
    ///
    /// The two vote payloads must belong to the shared PBFT slot described by
    /// `period`, `round`, and `step`. Rust consensus still validates the
    /// canonical vote bytes before generating contract call data.
    struct DoubleVotingProofInput {
        vote_a_hash: [u8; 32],
        vote_b_hash: [u8; 32],
        period: u64,
        round: u64,
        step: u64,
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

    /// Executor report after C++ attempts to insert a planned slashing transaction.
    struct DoubleVotingProofSubmissionReport {
        proof_hash: [u8; 32],
        transaction_inserted: bool,
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

    struct RewardVoteCursorSnapshot {
        found: bool,
        period: u64,
        round: u64,
        step: u64,
        block_hash: [u8; 32],
    }

    struct RewardVoteCursorCommitResult {
        status: u8,
        period: u64,
        round: u64,
        step: u64,
        block_hash: [u8; 32],
        reset_generation: u64,
        error_code: String,
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

    struct PbftBlockStorageLookup {
        found: bool,
        block_rlp: Vec<u8>,
    }

    /// Warning carried from side-effect-free PBFT sync admission planning.
    struct PbftSyncTransactionWarning {
        hash: [u8; 32],
        kind: u8,
    }

    /// Compact fact for one synced PBFT cert vote.
    struct PbftSyncCertVoteFact {
        vote_hash: [u8; 32],
        block_hash: [u8; 32],
        period: u64,
        round: u64,
        step: u64,
        vote_type: u8,
        live_vote_valid: bool,
        weight_present: bool,
        weight: u64,
    }

    /// Compact fact bundle for synced PBFT cert-vote validation.
    struct PbftSyncCertVoteBundleFact {
        block_period: u64,
        block_hash: [u8; 32],
        votes: Vec<PbftSyncCertVoteFact>,
        check_weight_threshold: bool,
        two_t_plus_one_found: bool,
        two_t_plus_one: u64,
    }

    /// Rust-owned synced PBFT cert-vote validation result.
    struct PbftSyncCertVoteBundleValidation {
        valid: bool,
        status: u8,
        total_weight: u64,
        two_t_plus_one: u64,
        first_bad_vote_hash: [u8; 32],
    }

    /// Transaction hash wrapper for CXX bridge vectors.
    struct PbftSyncTransactionHash {
        hash: [u8; 32],
    }

    /// Rust-planned finalized transaction lookup work for PBFT sync admission.
    struct PbftSyncTransactionQueryPlan {
        finalized_lookup_hashes: Vec<PbftSyncTransactionHash>,
    }

    /// Storage-backed PBFT sync egress payload for packet materialization.
    struct PbftSyncEgressPayload {
        period_data_rlp: Vec<u8>,
        attach_reward_votes: bool,
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

    /// Immutable synced-candidate facts captured once at admission start.
    struct PbftSyncAdmissionInitialFact {
        block_period: u64,
        block_prev_hash: [u8; 32],
        chain_last_hash: [u8; 32],
        chain_last_period: u64,
        block_in_chain: bool,
        dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
        extra_data_required: bool,
        extra_data_present: bool,
        extra_data_pillar_block_hash_present: bool,
        pillar_votes_required: bool,
        pillar_votes_present: bool,
        previous_cert_votes_present: bool,
        previous_cert_first_vote_has_weight: bool,
    }

    /// Current check or terminal decision from the manager-owned admission cursor.
    struct PbftSyncAdmissionSessionStep {
        status: u8,
        cursor: u32,
        has_check: bool,
        next_check: u8,
        plan: PbftSyncProcessPeriodDataRuntimePlan,
        complete: bool,
        can_continue: bool,
        error_code: String,
    }

    /// Cursor-checked report for final-chain, reward, cert, or pillar validation.
    struct PbftSyncAdmissionStatusReport {
        cursor: u32,
        check: u8,
        status: u8,
    }

    /// Cursor-checked TransactionManager result for synced-period admission.
    struct PbftSyncAdmissionTransactionReport {
        cursor: u32,
        missing_transaction_hashes: Vec<PbftSyncTransactionHash>,
        finalized_transaction_hashes: Vec<PbftSyncTransactionHash>,
        contains_finalized_transactions: bool,
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

    /// Task-specific reward-vote reset storage request.
    struct PbftRewardVotesResetRequest {
        period: u64,
        round: u64,
        step: u64,
        block_hash: [u8; 32],
        sync: bool,
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
        rounds_count_dynamic_lambda: u32,
        dynamic_lambda: u32,
        dpos_blocks_per_year: u32,
        pbft_head_payload: Vec<u8>,
        period_data_rlp: Vec<u8>,
        ordered_dag_block_hashes: Vec<PbftFinalizationHash>,
        ordered_transaction_hashes: Vec<PbftFinalizationHash>,
        process_pillar_block_after_advance: bool,
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
        rounds_count_dynamic_lambda: u32,
        dynamic_lambda: u32,
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
        has_prepared_pillar_block: bool,
        prepared_pillar_block_period: u64,
        prepared_pillar_block_rlp: Vec<u8>,
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

    /// PBFT-manager-owned dynamic-lambda planning output for finalization.
    ///
    /// This extends the pure dynamic-lambda plan with the previous persisted
    /// period-lambda lookup required by finalization intent planning, so C++
    /// does not issue a separate storage query through the bridge.
    struct PbftManagerFinalizationDynamicLambdaPlan {
        apply_dynamic_lambda_update: bool,
        period_lambda: u32,
        blocks_per_year: u32,
        rounds_count_dynamic_lambda: u32,
        dynamic_lambda: u32,
        decreased_dynamic_lambda: bool,
        increased_dynamic_lambda: bool,
        status: u8,
        error_code: String,
        last_saved_period_lambda_found: bool,
        last_saved_period_lambda: u32,
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
        polling_interval_ms: u64,
    }

    /// Immutable application configuration for coherent PBFT service restore.
    ///
    /// The restored chain head supplies the current period and determines
    /// whether Cacti is active; callers cannot inject either derived fact.
    struct PbftServiceConfig {
        genesis_lambda_ms: u64,
        cacti_lambda_max_ms: u64,
        cacti_lambda_default_ms: u64,
        cacti_block: u64,
        max_exponential_lambda_ms: u64,
        max_steps: u64,
        deadline_ms: u64,
        polling_interval_ms: u64,
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

    /// C++ executor report for one Rust-planned PBFT manager period-advance action.
    struct PbftManagerAdvancePeriodActionReport {
        action_index: u64,
        action: u8,
        succeeded: bool,
    }

    /// Validation result for one PBFT manager period-advance action report.
    struct PbftManagerAdvancePeriodActionReportResult {
        accepted: bool,
        status: u8,
        error_code: String,
    }

    /// Atomic verified-vote/proposed-block period cleanup result.
    struct PbftPeriodStateCleanupResult {
        status: u8,
        error_code: String,
        transition_published: bool,
        finalized_chain_size: u64,
        new_period: u64,
        verified_vote_periods_removed: u64,
        verified_votes_removed: u64,
        vote_payloads_removed: u64,
        proposed_block_periods_removed: u64,
        proposed_blocks_removed: u64,
        persistence_required: bool,
        persistence_applied_deletes: u64,
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
        sleep_ms: u64,
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

    /// Rust-owned PBFT manager sleep plan for the C++ condition-variable executor.
    struct PbftManagerSleepPlan {
        accepted: bool,
        should_sleep: bool,
        sleep_ms: u64,
        step: u64,
        error_code: String,
    }

    /// C++-originated facts for PBFT manager startup finalization readiness.
    struct PbftManagerFinalizationWaitFact {
        pbft_chain_size: u64,
        final_chain_last_block: u64,
        delegation_delay: u64,
        polling_interval_ms: u64,
    }

    /// Rust-owned PBFT manager startup finalization wait plan.
    struct PbftManagerFinalizationWaitPlan {
        accepted: bool,
        should_wait: bool,
        sleep_ms: u64,
        error_code: String,
    }

    /// C++-originated facts for eligible-wallet period readiness polling.
    struct PbftManagerEligibleWalletPeriodWaitFact {
        eligible_wallet_period: u64,
        pbft_chain_size: u64,
        polling_interval_ms: u64,
    }

    /// Rust-owned eligible-wallet period readiness wait plan.
    struct PbftManagerEligibleWalletPeriodWaitPlan {
        should_wait: bool,
        sleep_ms: u64,
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

    /// One ordered PBFT manager state-action effect for C++ execution.
    struct PbftManagerStateActionEffect {
        intent: u8,
        hash: [u8; 32],
        request_proposed_block_sidecar: bool,
        proposed_block_sidecar_hash: [u8; 32],
        proposed_block_sidecar_period: u64,
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

    /// Owned proposal-vote and proposed-block facts captured for external leader validation.
    ///
    /// `vote_record` and `proposed_block_rlp` remain valid after Rust releases
    /// its locks. Presence, cached validity, finalized-chain membership, and
    /// `needs_external_validation` describe the exact prepare-time state and
    /// are included in the enclosing snapshot fingerprint.
    struct PbftLeaderCandidateSnapshot {
        vote_hash: [u8; 32],
        block_hash: [u8; 32],
        vote_record: PbftVoteStorageRecord,
        proposed_block_found: bool,
        proposed_block_is_valid: bool,
        proposed_block_rlp: Vec<u8>,
        pivot_hash: [u8; 32],
        block_in_chain: bool,
        needs_external_validation: bool,
    }

    /// Coherent candidate snapshot returned before external block validation.
    ///
    /// Candidates contain only proposal-step votes for `period`/`round`, are
    /// sorted by vote hash, and are fingerprinted with their owned vote and
    /// proposal payload hashes plus validity and chain-membership state. Empty
    /// input is a successful `NoCandidates` snapshot.
    struct PbftLeaderSelectionSnapshot {
        status: u8,
        error_code: String,
        period: u64,
        round: u64,
        snapshot_fingerprint: [u8; 32],
        candidates: Vec<PbftLeaderCandidateSnapshot>,
    }

    /// External validation result for one candidate from the prepared snapshot.
    ///
    /// Status `1` accepts and status `2` rejects. Finish requires exactly one
    /// report for every candidate whose snapshot requested validation and no
    /// reports for missing, finalized, null, or already-valid candidates.
    struct PbftLeaderCandidateValidation {
        vote_hash: [u8; 32],
        block_hash: [u8; 32],
        status: u8,
    }

    /// Finish request binding validation reports to one exact prepared snapshot.
    ///
    /// `period`, `round`, and `snapshot_fingerprint` must match a freshly
    /// rebuilt snapshot. The request stores no server-side session and is safe
    /// to reject as stale without mutating proposed-block validity.
    struct PbftLeaderSelectionFinishRequest {
        period: u64,
        round: u64,
        snapshot_fingerprint: [u8; 32],
        validations: Vec<PbftLeaderCandidateValidation>,
    }

    /// Final owned leader selection result after snapshot revalidation and planner execution.
    ///
    /// A selected result owns both weighted vote bytes and proposed-block RLP.
    /// Non-selected, stale, and invalid-report outcomes return empty payloads;
    /// only planner-emitted validation commands may mark proposed blocks valid.
    struct PbftLeaderSelectionResult {
        status: u8,
        error_code: String,
        selected: bool,
        selected_vote: PbftVoteStorageRecord,
        selected_block_rlp: Vec<u8>,
    }

    /// Result from applying PBFT manager transition storage through the
    /// long-lived Rust runtime handle.
    struct PbftManagerRuntimeStorageApplyResult {
        status: u8,
        applied_writes: u64,
        snapshot: PbftManagerRuntimeSnapshot,
        error_code: String,
    }

    /// External/configuration inputs for one runtime-owned lifecycle transition.
    struct PbftManagerLifecycleTransitionRequest {
        kind: u8,
        target_period: u64,
        target_round: u64,
        has_network_next_voting_step: bool,
        network_next_voting_step: u64,
    }

    /// Committed runtime snapshot plus temporary C++ sidecar commands.
    struct PbftManagerLifecycleTransitionResult {
        status: u8,
        snapshot: PbftManagerRuntimeSnapshot,
        remove_cert_voted_sidecar: bool,
        clear_broadcasted_vote_sidecars: bool,
        set_vote_manager_period_round: bool,
        reset_current_round_timer: bool,
        reset_second_finish_timer: bool,
        print_cert_step_info: bool,
        print_second_finish_step_info: bool,
        reset_executed_block_follow_up: bool,
        error_code: String,
    }

    /// Request that starts the manager-owned PBFT finalization executor.
    struct PbftFinalizationExecutorStartRequest {
        mode: u8,
        plan: PbftFinalizationIntentPlan,
        primary_stages: Vec<PbftFinalizationStorageWriteStage>,
        sync: bool,
        final_chain_last_block: u64,
    }

    /// Manager-owned PBFT finalization executor state returned to C++.
    struct PbftManagerFinalizationExecutorState {
        status: u8,
        cursor: u32,
        action: u8,
        has_action: bool,
        complete: bool,
        can_continue: bool,
        drained_actions: u32,
        applied_dynamic_lambda: bool,
        persisted_executed_status: bool,
        set_executed_flag: bool,
        cleared_anchor_dag_cache: bool,
        has_snapshot: bool,
        snapshot: PbftManagerRuntimeSnapshot,
        last_storage_status: u8,
        reward_votes_reset_generation: u64,
        error_code: String,
    }

    /// Sortition finalization commit facts reported to the PBFT manager executor.
    struct PbftManagerFinalizationSortitionCommitReport {
        changed: bool,
        change_period: u64,
        change_interval_efficiency: u16,
        change_threshold_upper: u16,
        current_threshold_upper: u16,
        params_changes_count: u64,
    }

    /// Reward-vote reset finalization facts reported to the PBFT manager executor.
    struct PbftManagerFinalizationRewardVotesResetReport {
        period: u64,
        round: u64,
        block_hash: [u8; 32],
        reward_votes_reset_generation: u64,
    }

    /// FinalChain dispatch/replay finalization facts reported to the PBFT manager executor.
    struct PbftManagerFinalizationFinalChainDispatchReport {
        blocks_per_year: u32,
        last_block: u64,
    }

    /// Typed PBFT finalization pillar post-processing report from the C++
    /// executor.
    ///
    /// C++ supplies only the post-processing facts produced after executing
    /// `processPillarBlock`: the finalized PBFT period and the FinalChain
    /// request period used to build pillar inputs. Success/status, manager
    /// period, action identity, and cursor identity are derived by the manager
    /// finalization executor.
    struct PbftManagerFinalizationPillarPostProcessingReport {
        processed_period: u64,
        request_period: u64,
    }

    /// Typed PBFT finalization advance-period report from the C++ executor.
    ///
    /// C++ supplies only the manager period observed after executing the
    /// Rust-planned period advance. Success/status, action identity, and cursor
    /// identity are derived by the manager finalization executor.
    struct PbftManagerFinalizationAdvancePeriodReport {
        manager_period: u64,
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
        reward_votes_reset_generation: u64,
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

    /// Ordered identity queried against a temporary local candidate set.
    ///
    /// `period` and `block_hash` select one caller-supplied candidate. Rust
    /// returns one lookup per input identity in the same order. The carrier
    /// has no validation or persistence authority and is used only by the
    /// non-persisted local PBFT leader-selection adapter.
    struct ProposedBlockIdentity {
        period: u64,
        block_hash: [u8; 32],
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

    /// Runtime-owned PBFT sync period-data queue snapshot for C++ shell reads.
    ///
    /// C++ supplies the current PBFT-chain size and last chain hash facts that
    /// are still owned by the PBFT-chain compatibility facade. Rust returns
    /// the queue-derived view in one call instead of exposing individual queue
    /// metadata getters.
    struct PeriodDataQueueSnapshot {
        period: u64,
        syncing_period: u64,
        last_block_hash_or_chain: [u8; 32],
        size: usize,
        empty: bool,
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

    /// Caller-supplied flags for deriving PBFT vote event facts from canonical bytes.
    struct PbftVoteEventFactFlags {
        vote_already_known: bool,
        carries_proposed_block: bool,
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

    /// Runtime-owned PBFT vote admission transition result.
    ///
    /// This is the production-oriented `VoteManager::addVerifiedVote` bridge
    /// result: Rust validates canonical bytes, applies a bounded in-memory
    /// checkpoint, commits any required vote-progress batch, and publishes the
    /// transition only after persistence succeeds. External effects are valid
    /// only when `transition_published` is true.
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
        persistence_required: bool,
        persistence_status: u8,
        persistence_applied_writes: u64,
        transition_published: bool,
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

    /// Rust-owned PBFT reward-vote materialization output.
    ///
    /// This keeps reward-vote selection under the PBFT service's vote runtime
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
        has_preverified_weight: bool,
        preverified_weight: u64,
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

    /// Result of inspecting one PillarVote RLP payload in Rust.
    struct PillarVoteInspection {
        status: u8,
        period: u64,
        block_hash: [u8; 32],
        vote_hash: [u8; 32],
        voter: [u8; 20],
        signature_valid: bool,
    }

    /// Canonical pillar-vote bytes supplied for one batch inspection pass.
    struct PillarVoteRlpPayload {
        vote_rlp: Vec<u8>,
    }

    /// Canonical pillar-vote bytes paired with externally supplied DPoS weight.
    ///
    /// C++ still owns the external FinalChain/StateAPI read that provides the
    /// weight. Rust owns byte inspection and bundle planning once the weight is
    /// supplied.
    struct PillarVoteWeightedRlpPayload {
        vote_rlp: Vec<u8>,
        weight: u64,
    }

    /// Local context for preparing one pillar-vote admission.
    ///
    /// Rust sources current-pillar anchor facts and owns RLP decoding,
    /// signature recovery, duplicate detection, relevance, and identity
    /// uniqueness checks. C++ supplies only immutable scheduling configuration;
    /// FinalChain DPoS facts remain outside this DTO.
    struct PillarVoteSingleAdmissionContext {
        first_pillar_block_period: u64,
        pillar_blocks_interval: u64,
    }

    /// Configuration for a runtime-owned pillar-vote relevance check.
    ///
    /// Rust sources current-pillar anchor facts and duplicate membership from
    /// runtime-owned state. C++ supplies only immutable pillar scheduling rules.
    struct PillarVoteRuntimeRelevanceContext {
        first_pillar_block_period: u64,
        pillar_blocks_interval: u64,
    }

    /// Prepared identity for one pillar vote before external DPoS lookup.
    ///
    /// Status values match `PillarVoteValidationPlanStatus` in the C++ shim:
    /// `0` is ready for DPoS lookup and `can_query_dpos == true`; non-zero
    /// values identify the deterministic rejection reason.
    /// A ready external result also retains a one-time checked preparation keyed
    /// by `vote_hash`; apply must consume that retained entry.
    struct PillarVoteSingleAdmissionPreparePlan {
        status: u8,
        can_query_dpos: bool,
        needs_threshold: bool,
        period: u64,
        block_hash: [u8; 32],
        vote_hash: [u8; 32],
        voter: [u8; 20],
        anchor_generation: u64,
        has_current_anchor: bool,
        current_period: u64,
        current_hash: [u8; 32],
    }

    /// External DPoS facts needed to apply one prepared pillar vote.
    ///
    /// `vote_hash` selects a one-time runtime preparation created by either the
    /// checked external route or the explicitly trusted local/restart route.
    /// C++ supplies only FinalChain facts; canonical vote bytes, validation
    /// policy, and anchor generation come from the retained preparation.
    struct PillarVoteSingleAdmissionApplyInput {
        vote_hash: [u8; 32],
        validator_vote_count: u64,
        has_threshold: bool,
        threshold: u64,
    }

    /// Preparation result for a synced weighted pillar-vote bundle.
    ///
    /// Status codes are stable: `0` ready, `1` empty bundle, `2` missing
    /// current anchor, `3` current-period mismatch, and `4` inspection failure.
    /// On ready, C++ may query external DPoS weights for `inspections`; apply
    /// must echo `anchor_generation` so Rust can reject stale work.
    struct PillarVoteWeightedBundlePreparePlan {
        status: u8,
        can_query_dpos: bool,
        inspections: Vec<PillarVoteInspection>,
        first_bad_vote_hash: [u8; 32],
        required_votes_period: u64,
        expected_block_hash: [u8; 32],
        anchor_generation: u64,
        has_current_anchor: bool,
        current_period: u64,
        current_hash: [u8; 32],
    }

    /// Generation-bound input for applying one prepared weighted bundle.
    ///
    /// Rust re-derives the expected hash from its current snapshot and rejects
    /// a changed generation before mutating aggregation state.
    struct PillarVoteWeightedBundleApplyInput {
        votes: Vec<PillarVoteWeightedRlpPayload>,
        required_votes_period: u64,
        threshold: u64,
        anchor_generation: u64,
    }

    /// Result of applying one prepared pillar vote to Rust aggregation state.
    /// Status `10` is stale anchor and `11` is missing/consumed/evicted preparation.
    struct PillarVoteSingleAdmissionApplyPlan {
        status: u8,
        accepted: bool,
        duplicate: bool,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
        block_weight: u64,
    }

    /// Pillar vote payload selected for C++ edge materialization.
    ///
    /// Records may come from live Rust runtime state or from a verified stored
    /// `PeriodData` fallback, depending on the runtime lookup API used.
    struct PillarVoteRecord {
        vote_hash: [u8; 32],
        weight: u64,
        vote_rlp: Vec<u8>,
    }

    /// Lookup result with Rust-retained vote payloads for edge materialization.
    struct PillarVotesPayloadLookup {
        threshold_met: bool,
        block_weight: u64,
        selected_weight: u64,
        votes: Vec<PillarVoteRecord>,
    }

    /// Pillar vote hash included in one network-serving bundle.
    struct PillarVoteBundleHash {
        hash: [u8; 32],
    }

    /// Packet-ready optimized pillar-votes bundle payload.
    ///
    /// `votes_bundle_rlp` is the inner `OptimizedPillarVotesBundle` RLP, not
    /// the tarcap packet wrapper. `vote_hashes` mirrors the votes in the same
    /// order so C++ network code can mark them as known without materializing
    /// `PillarVote` objects.
    struct PillarVoteNetworkBundleChunk {
        vote_hashes: Vec<PillarVoteBundleHash>,
        votes_bundle_rlp: Vec<u8>,
    }

    /// Network-facing pillar-vote bundle lookup result.
    struct PillarVoteNetworkBundleLookup {
        from_storage: bool,
        chunks: Vec<PillarVoteNetworkBundleChunk>,
    }

    /// Result of validating and applying canonical pillar-vote bytes.
    ///
    /// C++ supplies DPoS weights from the external FinalChain boundary. Rust
    /// owns byte inspection, weighted bundle planning, period-threshold
    /// initialization, and selected-vote insertion into the pillar-chain
    /// runtime.
    struct PillarVoteBundleApplyPlan {
        status: u8,
        block_weight: u64,
        selected_weight: u64,
        first_bad_vote_hash: [u8; 32],
        insert_failed: bool,
        insert_failed_vote_hash: [u8; 32],
        applied_votes: u64,
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

    /// Public/query JSON view for one pillar validator vote-count delta.
    struct PillarBlockViewVoteCountChange {
        address: [u8; 20],
        vote_count_change: i32,
    }

    /// Public/query JSON view for one compact pillar-vote signature.
    struct PillarBlockViewSignature {
        r: [u8; 32],
        vs: [u8; 32],
    }

    /// Public/query JSON view over stored pillar block data.
    struct PillarBlockDataView {
        found: bool,
        pbft_period: u64,
        state_root: [u8; 32],
        previous_pillar_block_hash: [u8; 32],
        bridge_root: [u8; 32],
        epoch: u64,
        validator_vote_count_changes: Vec<PillarBlockViewVoteCountChange>,
        block_hash: [u8; 32],
        signatures: Vec<PillarBlockViewSignature>,
    }

    /// Durable pillar-chain rows required to reconstruct manager startup state.
    ///
    /// Empty byte vectors represent rows that have not yet been persisted. Rust
    /// owns the latest-finalized block snapshot and derives its following PBFT
    /// period before returning that period's opaque data row.
    struct PillarChainStartupBootstrap {
        own_vote_rlp: Vec<u8>,
        current_block_data_rlp: Vec<u8>,
        latest_pillar_votes_period_data_rlp: Vec<u8>,
    }

    /// Operation-tagged current-pillar decision request.
    ///
    /// Operation values are stable: `0` validate candidate, `1` select the
    /// previous-period anchor, and `2` select restart post-processing due.
    /// Fields unrelated to the selected operation are ignored. Unknown
    /// operation values are rejected as bridge errors.
    struct PillarCurrentAnchorDecisionRequest {
        operation: u8,
        has_candidate_hash: bool,
        candidate_hash: [u8; 32],
        pbft_period: u64,
        pillar_blocks_interval: u64,
    }

    /// Current-pillar decision plus the snapshot facts used to make it.
    ///
    /// Status codes are defined by native `PillarCurrentAnchorDecisionStatus`.
    /// `selected` has operation-specific meaning; the anchor/generation fields
    /// are always returned for diagnostics and stale-work binding.
    struct PillarCurrentAnchorDecisionResult {
        status: u8,
        selected: bool,
        has_current_anchor: bool,
        current_period: u64,
        current_hash: [u8; 32],
        anchor_generation: u64,
    }

    /// Public FinalChain block view returned by `ConsensusQueryApi`.
    ///
    /// The view is read-only and contains stable scalar/hash facts plus the
    /// canonical stored header bytes for compatibility formatters. It does not
    /// expose FinalChain, PBFT, storage, or manager objects.
    struct FinalChainBlockView {
        found: bool,
        number: u64,
        hash: [u8; 32],
        parent_hash: [u8; 32],
        author: [u8; 20],
        state_root: [u8; 32],
        transactions_root: [u8; 32],
        receipts_root: [u8; 32],
        log_bloom: Vec<u8>,
        gas_used: u64,
        total_reward: [u8; 32],
        stored_header_rlp: Vec<u8>,
        has_pbft_hash: bool,
        pbft_block_hash: [u8; 32],
    }

    /// Candidate facts for runtime-owned pillar-block parent-linkage planning.
    struct PillarBlockLinkageRequest {
        pillar_block_period: u64,
        pillar_block_previous_hash: [u8; 32],
        first_pillar_block_period: u64,
        pillar_blocks_interval: u64,
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

    /// External facts for runtime-owned pillar-block shell planning.
    struct PillarBlockCreationRequest {
        pillar_block_period: u64,
        state_root: [u8; 32],
        bridge_root: [u8; 32],
        bridge_epoch: [u8; 32],
        first_pillar_block_period: u64,
        pillar_blocks_interval: u64,
    }

    /// Rust-planned shell fields and validator deltas for temporary C++
    /// `PillarBlock` materialization.
    ///
    /// Status values match `PillarBlockLinkagePlan`.
    struct PillarBlockCreationWithVoteCountsPlan {
        status: u8,
        valid: bool,
        expected_previous_period: u64,
        previous_pillar_block_hash: [u8; 32],
        state_root: [u8; 32],
        bridge_root: [u8; 32],
        bridge_epoch: [u8; 32],
        vote_count_changes: Vec<PillarValidatorVoteCountChange>,
    }

    /// Compact request for Rust-owned pillar-block finalization execution.
    ///
    /// C++ supplies only the requested hash. Rust derives current and
    /// latest-finalized identity from its runtime snapshot and owns verified-
    /// vote lookup and planning. The prepared canonical row is persisted in
    /// the PBFT primary batch, then Rust authenticates it during acknowledgement
    /// before publishing the latest snapshot and cleaning votes.
    struct PillarBlockFinalizationRequest {
        requested_pillar_block_hash: [u8; 32],
    }

    /// Result of Rust-owned pillar-block finalization prepare.
    ///
    /// Status values match the native pillar finalization planner: `0` ready,
    /// `1` missing current block, `2` current hash mismatch, `3` missing
    /// votes, and `4` already finalized.
    ///
    /// In the ready path, Rust emits the canonical pillar block RLP plus a
    /// one-time preparation token so C++ can attach it to the existing PBFT
    /// primary persistence stage instead of mutating storage here.
    struct PillarBlockFinalizationPrepareResult {
        status: u8,
        success: bool,
        should_request_votes: bool,
        has_request_votes_period: bool,
        request_votes_period: u64,
        should_emit: bool,
        current_period: u64,
        current_hash: [u8; 32],
        block_weight: u64,
        selected_weight: u64,
        selected_vote_count: u64,
        prepared_pillar_block_period: u64,
        prepared_pillar_block_rlp: Vec<u8>,
        has_prepared_pillar_block: bool,
        preparation_anchor_generation: u64,
        preparation_token: u64,
        votes: Vec<PillarVoteRecord>,
    }

    /// Request to acknowledge one prepared pillar-block finalization.
    ///
    /// The generation + token pair is required before persistence side-effects
    /// can be applied. This keeps replay and stale acknowledge paths explicit.
    struct PillarBlockFinalizationAcknowledgeRequest {
        anchor_generation: u64,
        preparation_token: u64,
    }

    /// Result after acknowledging one prepared pillar-block finalization.
    ///
    /// Returns the latest finalized identity that is now mirrored into the runtime
    /// snapshot so compatibility event emission can use one canonical source.
    #[derive(Debug)]
    struct PillarBlockFinalizationAcknowledgeResult {
        should_emit: bool,
        latest_finalized_period: u64,
        latest_finalized_hash: [u8; 32],
    }

    struct UniqueVoterInsertOutcome {
        accepted: bool,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
        conflicting_vote_found: bool,
        conflicting_vote: PbftVoteStorageRecord,
        bucket_found: bool,
        bucket: VerifiedStepVotePayloadEntry,
        used_secondary_slot: bool,
        duplicate_vote_hash: bool,
    }

    struct VotedValueInsertOutcome {
        inserted: bool,
        total_weight: u64,
        votes_count: usize,
        conflicting_vote_found: bool,
        conflicting_vote: PbftVoteStorageRecord,
        bucket_found: bool,
        bucket: VerifiedStepVotePayloadEntry,
    }

    struct VerifiedVoteAddOutcome {
        vote: VerifiedVotePayload,
        inserted: bool,
        total_weight: u64,
        votes_count: usize,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
        conflicting_vote_found: bool,
        conflicting_vote: PbftVoteStorageRecord,
        bucket_found: bool,
        bucket: VerifiedStepVotePayloadEntry,
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
        conflicting_vote_found: bool,
        conflicting_vote: PbftVoteStorageRecord,
        bucket_found: bool,
        bucket: VerifiedStepVotePayloadEntry,
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

    struct TwoTPlusOneVotePayloadsLookup {
        found: bool,
        block_hash: [u8; 32],
        step: u64,
        votes: Vec<PbftVoteStorageRecord>,
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

    /// One metadata vote paired with its authoritative retained weighted payload.
    struct VerifiedVoteStateSnapshotEntry {
        vote: VerifiedVotePayload,
        weighted_vote: PbftVoteStorageRecord,
    }

    /// Coherent owned snapshot of every verified-vote state family materialized by C++.
    struct VerifiedVotesStateSnapshot {
        votes: Vec<VerifiedVoteStateSnapshotEntry>,
        round_markers: Vec<RoundMarkerSnapshot>,
        two_t_plus_one: Vec<TwoTPlusOneSnapshotEntry>,
    }

    /// One voted-value bucket with owned payload records in canonical vote-hash order.
    struct VerifiedStepVotePayloadEntry {
        block_hash: [u8; 32],
        total_weight: u64,
        votes: Vec<PbftVoteStorageRecord>,
    }

    struct VerifiedStepVotePayloadsLookup {
        found: bool,
        entries: Vec<VerifiedStepVotePayloadEntry>,
    }

    /// Coherent reward cursor and the retained weighted payloads selected by that cursor.
    struct RewardVotePayloadSnapshot {
        cursor: RewardVoteCursorSnapshot,
        records: Vec<PbftVoteStorageRecord>,
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

    /// Minimal CXX report after external reward execution is accepted by Rust.
    ///
    /// The full external-EVM commit plan remains session-owned Rust state; C++
    /// only needs correlation fields and any validation error before calling
    /// the one-shot state-commit preparation API.
    struct FinalChainExternalEvmCommitReport {
        request_id: [u8; 32],
        period: u64,
        error_code: String,
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

    /// Public/query JSON view for one finalized DAG block.
    struct DagBlockPublicView {
        found: bool,
        pivot: [u8; 32],
        level: u64,
        tips: Vec<DagHash>,
        transactions: Vec<DagHash>,
        trx_estimations: u64,
        signature: Vec<u8>,
        block_rlp: Vec<u8>,
        hash: [u8; 32],
        sender: [u8; 20],
        timestamp: u64,
        finalized_period_found: bool,
        finalized_period: u64,
        finalized_position: u32,
        has_vdf: bool,
        vdf_proof: Vec<u8>,
        vdf_sol1: Vec<u8>,
        vdf_sol2: Vec<u8>,
        vdf_difficulty: u16,
    }

    /// Public/query transaction payload view.
    struct TransactionPublicView {
        found: bool,
        hash: [u8; 32],
        source: u8,
        location_found: bool,
        block_number: u64,
        transaction_index: u32,
        is_system: bool,
        block_hash_found: bool,
        block_hash: [u8; 32],
        transaction_rlp: Vec<u8>,
    }

    /// Public/query transaction receipt payload view.
    struct TransactionReceiptPublicView {
        found: bool,
        transaction_hash: [u8; 32],
        transaction_source: u8,
        transaction_rlp: Vec<u8>,
        receipt_rlp: Vec<u8>,
        block_number: u64,
        transaction_index: u32,
        is_system: bool,
        block_hash_found: bool,
        block_hash: [u8; 32],
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

    /// Input transaction fact for runtime-owned DAG transaction persistence.
    ///
    /// Rust computes sidecar membership from `TransactionRuntimeState`
    /// instead of accepting C++ membership booleans.
    struct DagTransactionSaveSidecarFact {
        input_index: u64,
        hash: [u8; 32],
        trx_rlp: Vec<u8>,
        transaction_nonce: [u8; 32],
        sender_account_nonce: [u8; 32],
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

    /// One Rust-authored admission shell side effect.
    ///
    /// Rust selects these intents after queue mutation and public-status
    /// planning. C++ only realizes the requested legacy shell effect while app
    /// event/log infrastructure remains C++ hosted.
    struct TransactionManagerAdmissionShellIntent {
        kind: u8,
        hash: [u8; 32],
    }

    /// Typed command report for TransactionManager admission.
    ///
    /// Rust has already completed validated queue mutation and public status
    /// mapping, then selected the shell logging/event intents. C++ consumes this
    /// report only for legacy shell realization and public status conversion.
    struct TransactionManagerAdmissionCommandReport {
        inserted_hash_found: bool,
        inserted_hash: [u8; 32],
        transaction_added_hash_found: bool,
        transaction_added_hash: [u8; 32],
        shell_intents: Vec<TransactionManagerAdmissionShellIntent>,
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

    struct DagPivotTipsValidation {
        ok: bool,
        expected_level: u64,
        level_matches: bool,
        missing_references: Vec<DagHash>,
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

    /// Rust-collected DPoS and VRF facts for DAG authorization.
    struct DagDposAuthorizationFacts {
        vrf_key_found: bool,
        vrf_key: Vec<u8>,
        sender_eligible_vote_count: u64,
        vdf_sortition_max_vote_count: u64,
        eligibility_status: u8,
    }

    /// Rust-collected FinalChain facts needed to start one DAG proposer attempt.
    struct DagProposerFinalChainFacts {
        last_finalized_period: u64,
        authorization_facts: DagDposAuthorizationFacts,
    }

    /// External/configured facts used to open one runtime-owned proposal session.
    ///
    /// The caller supplies trusted wallet identity (VRF keys and proposer address), transaction pressure, packing
    /// limits, and block-construction gas/tip limits. Rust derives the DAG frontier, proposal level/period/hash, and
    /// observation fingerprint atomically from the runtime. Invalid storage state is returned as an error; a missing
    /// proposal-period mapping produces a terminal session step instead. Identity and limits are retained by the cursor
    /// and cannot be replaced at signing time.
    struct DagProposerSessionBeginInput {
        transaction_pool_size: u64,
        non_finalized_transaction_count: u64,
        max_non_finalized_transactions: u64,
        dag_expiry_level_limit: u64,
        wallet_vrf_public_key: [u8; 32],
        wallet_vrf_secret: [u8; 64],
        proposer_address: [u8; 20],
        max_non_finalized_dag_blocks: u64,
        max_non_finalized_dag_blocks_low_difficulty: u64,
        max_retry_count: u64,
        proposal_weight_limit: u64,
        total_transaction_shards: u16,
        node_transaction_shard: u16,
        shard_period_interval: u64,
        pbft_gas_limit: u64,
        dag_gas_limit: u64,
        max_tips: u16,
    }

    /// FinalChain and sortition facts collected for the runtime-derived proposal period.
    ///
    /// These facts answer the session's collect action and must correspond to the `proposal_period` in that step. Rust
    /// revalidates its observation before consuming them; stale observations terminate without retry mutation, while
    /// storage/decode/planner failures return an error and remove the session.
    struct DagProposerExternalProposalFactsReport {
        last_finalized_period: u64,
        authorization_facts: DagDposAuthorizationFacts,
        sortition_params: SortitionRuntimeParams,
    }

    /// Rust-planned transaction packing request for a DAG proposal attempt.
    struct DagProposerTransactionPackRequest {
        proposal_period: u64,
        weight_limit: u64,
        total_transaction_shards: u16,
        node_transaction_shard: u16,
        shard_period_interval: u64,
    }

    /// Complete instruction/result snapshot returned by the Rust-owned DAG proposer cursor.
    ///
    /// `status` distinguishes active, complete, and invalid-report outcomes; `action` selects the one external boundary
    /// to execute. Retry fields are authoritative only when `update_retry_state` is true. Vectors may be empty when the
    /// selected action does not consume them. Action 5 exposes only the Rust-owned intent's `signing_hash`; action 6
    /// exposes the canonical `signed_block` executor payload. Terminal steps remove the cursor after construction.
    struct DagProposerSessionStep {
        status: u8,
        action: u8,
        reason_code: u32,
        return_value: bool,
        update_retry_state: bool,
        next_last_propose_level: u64,
        next_retry_count: u64,
        frontier_pivot: [u8; 32],
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
        transaction_request: DagProposerTransactionPackRequest,
        signing_hash: [u8; 32],
        signed_block: DagProposerSignedBlockIntent,
        record_proposed_block: bool,
        vdf_poll_interval_ms: u64,
        stale_proof_sleep_ms: u64,
        error_code: String,
    }

    /// Report from the live transaction-packing executor boundary.
    struct DagProposerTransactionPackReport {
        network_throttled: bool,
        transaction_hashes: Vec<DagHash>,
        transaction_gas_estimations: Vec<u64>,
    }

    /// Result of the external VDF executor boundary.
    ///
    /// `vdf_rlp` is consumed only when `proof_ok` is true and becomes the canonical proof field of the session-owned
    /// unsigned intent. The caller cannot supply any other block field. Malformed storage during subsequent construction
    /// returns an error and removes the session.
    struct DagProposerVdfProofReport {
        proof_ok: bool,
        vdf_rlp: Vec<u8>,
    }

    /// Recoverable signature returned by the external signer.
    ///
    /// The signature must be exactly 65 bytes over the signing hash from action 5 and recover to the trusted proposer
    /// address captured at begin. Rust combines it only with the stored unsigned intent; malformed or wrong-key
    /// signatures return an error and remove the session without retry mutation.
    struct DagProposerSigningReport {
        signature: Vec<u8>,
    }

    /// Typed report after C++ materializes/signs/adds the proposed DAG block.
    ///
    /// C++ remains the executor for compatibility side effects, but Rust consumes
    /// the complete executor outcome before advancing the proposer session.
    struct DagProposerAddBlockReport {
        accepted: bool,
        duplicate: bool,
        expired: bool,
        missing_references: Vec<DagHash>,
    }

    /// Live facts for one Rust-owned DAG proposer worker-loop command decision.
    struct DagProposerWorkerCommandInput {
        pbft_syncing: bool,
        packet_queue_over_limit: bool,
        has_attempt_result: bool,
        attempt_returned_proposed: bool,
    }

    /// Command C++ executes for one DAG proposer worker-loop tick.
    struct DagProposerWorkerCommand {
        attempt_proposal: bool,
        sleep_after_tick: bool,
        sleep_ms: u64,
        reason_code: u32,
    }

    /// Rust-runtime DAG proposer tip-selection facts for the legacy compatibility API.
    struct DagProposerStorageTipSelectionInput {
        frontier_tips: Vec<DagHash>,
        gas_limit: u64,
        max_tips: u16,
    }

    /// Rust producer-side DAG tip-selection plan.
    struct DagProposerTipSelectionPlan {
        selected_tips: Vec<DagHash>,
        skipped_missing_tips: u64,
    }

    /// Canonical signed DAG block executor payload returned by action 6.
    ///
    /// `block_rlp` is the complete eight-field canonical block encoding assembled from session-owned facts and the
    /// external signature. `block_hash` is its Keccak hash. Both fields are empty/zero on non-add-block actions.
    struct DagProposerSignedBlockIntent {
        block_rlp: Vec<u8>,
        block_hash: [u8; 32],
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
        emit_verified: bool,
        gossip: bool,
        proposed: bool,
        missing_references: Vec<DagHash>,
    }

    struct DagManagerBlock {
        hash: [u8; 32],
        pivot: [u8; 32],
        tips: Vec<DagHash>,
        level: u64,
        difficulty: u32,
    }

    struct DagManagerAnchors {
        old_anchor: [u8; 32],
        anchor: [u8; 32],
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

    struct VrfProofResult {
        ok: bool,
        status: u8,
        error: String,
        public_key: [u8; 32],
        proof: [u8; 80],
        output: [u8; 64],
        threshold: u16,
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
        type BridgeConsensusQueryApi;

        pub fn create_consensus_query_api(storage: &BridgeStorage) -> Box<BridgeConsensusQueryApi>;
        pub fn consensus_query_pbft_block_hash_by_period(
            self: &BridgeConsensusQueryApi,
            period: u64,
        ) -> Result<HashLookup>;
        pub fn consensus_query_final_chain_block_by_number(
            self: &BridgeConsensusQueryApi,
            number: u64,
        ) -> Result<FinalChainBlockView>;
        pub fn consensus_query_final_chain_block_number_by_hash(
            self: &BridgeConsensusQueryApi,
            block_hash: &[u8; 32],
        ) -> Result<FinalChainBlockNumberLookup>;
        pub fn consensus_query_final_chain_last_block_number(
            self: &BridgeConsensusQueryApi,
        ) -> Result<u64>;
        pub fn consensus_query_period_lambda_by_period(
            self: &BridgeConsensusQueryApi,
            period: u64,
        ) -> Result<PeriodLambda>;
        pub fn consensus_query_proposal_period_for_dag_level(
            self: &BridgeConsensusQueryApi,
            level: u64,
        ) -> Result<FinalChainBlockNumberLookup>;
        pub fn consensus_query_chain_stats(
            self: &BridgeConsensusQueryApi,
        ) -> Result<ChainStatsView>;
        pub fn consensus_query_status(
            self: &BridgeConsensusQueryApi,
        ) -> Result<ConsensusStatusView>;
        pub fn consensus_query_sortition_params_change_by_period(
            self: &BridgeConsensusQueryApi,
            period: u64,
        ) -> Result<SortitionParamsChangeView>;
        pub fn consensus_query_final_chain_blocks_with_bloom(
            self: &BridgeConsensusQueryApi,
            bloom: &[u8; 256],
            from: u64,
            to: u64,
        ) -> Result<Vec<u64>>;
        pub fn consensus_query_pbft_schedule_block_by_period(
            self: &BridgeConsensusQueryApi,
            period: u64,
        ) -> Result<PbftScheduleBlockView>;
        pub fn consensus_query_pbft_node_version_by_period(
            self: &BridgeConsensusQueryApi,
            period: u64,
        ) -> Result<PbftNodeVersionView>;
        pub fn consensus_query_pbft_previous_block_cert_votes_by_period(
            self: &BridgeConsensusQueryApi,
            period: u64,
        ) -> Result<PbftPeriodCertVotesView>;
        pub fn consensus_query_pillar_block_data_by_period(
            self: &BridgeConsensusQueryApi,
            period: u64,
        ) -> Result<PillarBlockDataView>;
        pub fn consensus_query_dag_block_by_hash(
            self: &BridgeConsensusQueryApi,
            hash: &[u8; 32],
        ) -> Result<DagBlockPublicView>;
        pub fn consensus_query_dag_blocks_by_level(
            self: &BridgeConsensusQueryApi,
            level: u64,
            number_of_levels: u32,
        ) -> Result<Vec<DagBlockPublicView>>;
        pub fn consensus_query_finalized_dag_blocks_by_period(
            self: &BridgeConsensusQueryApi,
            period: u64,
        ) -> Result<Vec<DagBlockPublicView>>;
        pub fn consensus_query_transaction_by_hash(
            self: &BridgeConsensusQueryApi,
            hash: &[u8; 32],
        ) -> Result<TransactionPublicView>;
        pub fn consensus_query_transaction_by_block_number_and_index(
            self: &BridgeConsensusQueryApi,
            block_number: u64,
            transaction_index: u64,
        ) -> Result<TransactionPublicView>;
        pub fn consensus_query_transaction_by_block_hash_and_index(
            self: &BridgeConsensusQueryApi,
            block_hash: &[u8; 32],
            transaction_index: u64,
        ) -> Result<TransactionPublicView>;
        pub fn consensus_query_transaction_count_by_block_number(
            self: &BridgeConsensusQueryApi,
            block_number: u64,
        ) -> Result<u64>;
        pub fn consensus_query_transaction_count_by_block_hash(
            self: &BridgeConsensusQueryApi,
            block_hash: &[u8; 32],
        ) -> Result<u64>;
        pub fn consensus_query_transaction_receipt_by_hash(
            self: &BridgeConsensusQueryApi,
            hash: &[u8; 32],
        ) -> Result<TransactionReceiptPublicView>;
        pub fn consensus_query_transaction_receipts_by_block_number(
            self: &BridgeConsensusQueryApi,
            block_number: u64,
        ) -> Result<Vec<TransactionReceiptPublicView>>;
    }

    extern "Rust" {
        type BridgeConsensusNetworkApi;

        pub fn create_consensus_network_api(
            config: NetworkApiConfig,
        ) -> Box<BridgeConsensusNetworkApi>;
        pub fn consensus_network_ingest_packet(
            self: &BridgeConsensusNetworkApi,
            packet: NetworkIngressPacket,
        ) -> Result<NetworkIngressReceipt>;
        pub fn consensus_network_drain_work(
            self: &BridgeConsensusNetworkApi,
            budget: u32,
        ) -> Result<NetworkEffectBatch>;
        pub fn consensus_network_report_effect_results(
            self: &BridgeConsensusNetworkApi,
            results: Vec<NetworkEffectResult>,
        ) -> Result<NetworkEffectAck>;
        pub fn consensus_network_plan_pbft_vote_ingress(
            self: &BridgeConsensusNetworkApi,
            fact: PbftVoteIngressFact,
            context: PbftVoteIngressContext,
        ) -> Result<PbftVoteIngressPlan>;
        pub fn consensus_network_plan_pbft_vote_bundle_ingress(
            self: &BridgeConsensusNetworkApi,
            reference: PbftVoteIngressFact,
            vote: PbftVoteIngressFact,
            context: PbftVoteIngressContext,
        ) -> Result<PbftVoteIngressPlan>;
        pub fn consensus_network_ingest_pbft_vote(
            self: &BridgeConsensusNetworkApi,
            fact: PbftVoteIngressFact,
            context: NetworkPbftVoteIngressContext,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_ingest_pbft_vote_bundle_member(
            self: &BridgeConsensusNetworkApi,
            reference: PbftVoteIngressFact,
            vote: PbftVoteIngressFact,
            context: NetworkPbftVoteIngressContext,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_plan_pillar_vote_relevance(
            self: &BridgeConsensusNetworkApi,
            fact: PillarVoteRelevanceFact,
        ) -> Result<PillarVoteRelevancePlan>;
        pub fn consensus_network_plan_status_sync(
            self: &BridgeConsensusNetworkApi,
            facts: NetworkStatusSyncFacts,
        ) -> Result<NetworkStatusSyncPlan>;
        pub fn consensus_network_plan_status_egress(
            self: &BridgeConsensusNetworkApi,
            facts: NetworkStatusEgressFacts,
        ) -> Result<NetworkStatusEgressPlan>;
        pub fn consensus_network_plan_initial_status(
            self: &BridgeConsensusNetworkApi,
            facts: NetworkInitialStatusFacts,
        ) -> Result<NetworkInitialStatusPlan>;
        pub fn consensus_network_plan_pbft_sync_start(
            self: &BridgeConsensusNetworkApi,
            facts: NetworkPbftSyncStartFacts,
        ) -> Result<NetworkPbftSyncStartPlan>;
        pub fn consensus_network_plan_max_chain_peer_selection(
            self: &BridgeConsensusNetworkApi,
            facts: NetworkPeerSelectionFacts,
        ) -> Result<NetworkPeerSelectionPlan>;
        pub fn consensus_network_plan_pending_dag_blocks_request(
            self: &BridgeConsensusNetworkApi,
            facts: NetworkPendingDagBlocksRequestFacts,
        ) -> Result<NetworkPendingDagBlocksRequestPlan>;
        pub fn consensus_network_gossip_pbft_vote(
            self: &BridgeConsensusNetworkApi,
            effects: NetworkPbftVoteGossipEffects,
        ) -> Result<NetworkIngressDecision>;

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

        pub unsafe fn make_cancellation_token_with_atomic(
            atomic_ptr: *const bool,
        ) -> Box<CancellationToken>;

        pub fn prove(vdf: &WesolowskiVdf, cancelled: &CancellationToken) -> Box<Solution>;
        pub fn verify(vdf: &WesolowskiVdf, solution: &Solution) -> bool;

        pub fn solution_get_proof(solution: &Solution) -> &[u8];
        pub fn solution_get_output(solution: &Solution) -> &[u8];

        pub fn vdf_sortition_payload_encode(payload: &VdfSortitionPayload) -> Vec<u8>;

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

        type BridgeDagTransactionService;

        pub fn create_dag_transaction_service_from_storage(
            storage: &BridgeStorage,
            genesis: &[u8; 32],
            dag_expiry_limit: u32,
            max_levels_per_period: u64,
            transaction_queue_config: TransactionQueueConfig,
            gas_pricer_config: GasPricerConfig,
            proposal_dag_gas_limit: u64,
        ) -> Result<Box<BridgeDagTransactionService>>;
        pub fn create_dag_transaction_service_for_transaction_manager(
            storage: &BridgeStorage,
            transaction_queue_config: TransactionQueueConfig,
            gas_pricer_config: GasPricerConfig,
            proposal_dag_gas_limit: u64,
        ) -> Result<Box<BridgeDagTransactionService>>;
        pub fn create_dag_transaction_service_for_gas_pricer(
            gas_pricer_config: GasPricerConfig,
        ) -> Result<Box<BridgeDagTransactionService>>;
        /// Rebuilds the DAG runtime snapshot from Rust PBFT/DAG storage without
        /// using the legacy C++ graph mirror.
        pub fn dag_manager_runtime_restore_from_storage(
            self: &BridgeDagTransactionService,
        ) -> Result<()>;
        pub fn dag_manager_runtime_add_block(
            self: &BridgeDagTransactionService,
            block: DagManagerBlock,
        ) -> Result<()>;
        /// Plans one add-block execution from Rust-owned runtime graph state.
        pub fn dag_manager_runtime_plan_add_block(
            self: &BridgeDagTransactionService,
            input: DagAddBlockRuntimeInput,
        ) -> Result<DagAddBlockEffectPlan>;
        /// Validates candidate pivot/tip references from Rust runtime state and
        /// storage without C++ `DagBlock` materialization.
        pub fn dag_manager_runtime_validate_pivot_tips(
            self: &BridgeDagTransactionService,
            block_level: u64,
            pivot: &[u8; 32],
            tips: Vec<DagHash>,
        ) -> Result<DagPivotTipsValidation>;
        /// Applies finalized DAG order using Rust state and Rust storage.
        pub fn dag_manager_runtime_apply_finalized_order(
            self: &BridgeDagTransactionService,
            new_anchor: [u8; 32],
            new_period: u64,
            finalized_order: Vec<DagHash>,
        ) -> Result<DagManagerFinalizationApplyPayload>;
        /// Returns non-finalized sync DAG block RLPs and referenced transaction
        /// RLPs through Rust-owned storage access.
        pub fn dag_manager_runtime_non_finalized_sync_payload(
            self: &BridgeDagTransactionService,
            known_hashes: Vec<DagHash>,
        ) -> Result<DagManagerNonFinalizedSyncPayload>;
        pub fn dag_manager_runtime_compute_order(
            self: &BridgeDagTransactionService,
            anchor: &[u8; 32],
        ) -> Result<DagOrder>;
        pub fn dag_manager_runtime_frontier(
            self: &BridgeDagTransactionService,
        ) -> Result<DagFrontier>;
        pub fn dag_manager_runtime_ghost_path(
            self: &BridgeDagTransactionService,
            source: &[u8; 32],
        ) -> Result<Vec<DagHash>>;
        pub fn dag_manager_runtime_anchor_ghost_path(
            self: &BridgeDagTransactionService,
        ) -> Result<Vec<DagHash>>;
        pub fn dag_manager_runtime_graphviz_dot(
            self: &BridgeDagTransactionService,
            pivot_tree: bool,
        ) -> Result<String>;
        pub fn dag_manager_runtime_vertex_count(
            self: &BridgeDagTransactionService,
        ) -> Result<usize>;
        pub fn dag_manager_runtime_edge_count(self: &BridgeDagTransactionService) -> Result<usize>;
        pub fn dag_manager_runtime_max_level(self: &BridgeDagTransactionService) -> Result<u64>;
        pub fn dag_manager_runtime_latest_period(self: &BridgeDagTransactionService)
            -> Result<u64>;
        pub fn dag_manager_runtime_anchors(
            self: &BridgeDagTransactionService,
        ) -> Result<DagManagerAnchors>;
        pub fn dag_manager_runtime_dag_expiry_limit(
            self: &BridgeDagTransactionService,
        ) -> Result<u32>;
        pub fn dag_manager_runtime_dag_expiry_level(
            self: &BridgeDagTransactionService,
        ) -> Result<u64>;
        pub fn dag_manager_runtime_non_finalized_blocks(
            self: &BridgeDagTransactionService,
        ) -> Result<Vec<DagLevelHashes>>;
        pub fn dag_manager_runtime_non_finalized_blocks_size(
            self: &BridgeDagTransactionService,
        ) -> Result<DagManagerNonFinalizedSize>;
        pub fn dag_manager_runtime_non_finalized_min_difficulty(
            self: &BridgeDagTransactionService,
        ) -> Result<u32>;
        /// Returns DAG block membership from Rust graph state plus canonical
        /// Rust storage without consulting C++ compatibility caches.
        pub fn dag_manager_runtime_is_block_known(
            self: &BridgeDagTransactionService,
            hash: &[u8; 32],
        ) -> Result<bool>;
        /// Loads per-tip gas facts from canonical Rust DAG storage for
        /// verification gas checks without C++ `DagBlock` materialization.
        pub fn dag_manager_runtime_tip_gas_estimations(
            self: &BridgeDagTransactionService,
            tips: Vec<DagHash>,
        ) -> Result<Vec<DagTipGas>>;
        pub fn dag_manager_runtime_load_block(
            self: &BridgeDagTransactionService,
            hash: &[u8; 32],
        ) -> Result<DagBlockLookup>;
        pub fn dag_manager_runtime_save_block(
            self: &BridgeDagTransactionService,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn dag_manager_runtime_plan_proposal_tip_selection(
            self: &BridgeDagTransactionService,
            input: DagProposerStorageTipSelectionInput,
        ) -> Result<DagProposerTipSelectionPlan>;
        /// Opens a runtime-owned proposer cursor from wallet/configuration and transaction-pressure inputs.
        /// Returns a unique cursor id or a storage/decode error; Rust derives all DAG observation facts, and callers
        /// must eventually consume a terminal step or call the idempotent abort function.
        #[rust_name = "service_dag_manager_runtime_begin_proposer_session"]
        pub fn dag_manager_runtime_begin_proposer_session(
            runtime: &BridgeDagTransactionService,
            input: DagProposerSessionBeginInput,
        ) -> Result<u64>;
        /// Idempotently removes a live proposer cursor without planner or retry effects.
        /// Returns true only when this call removed the cursor; missing or already-removed ids return false.
        #[rust_name = "service_dag_manager_runtime_abort_proposer_session"]
        pub fn dag_manager_runtime_abort_proposer_session(
            runtime: &BridgeDagTransactionService,
            session_id: u64,
        ) -> Result<bool>;
        pub fn dag_manager_runtime_ensure_proposal_period_mapping(
            self: &BridgeDagTransactionService,
            level: u64,
            period: u64,
        ) -> Result<bool>;
        pub fn dag_manager_runtime_period_block_hash(
            self: &BridgeDagTransactionService,
            period: u64,
        ) -> Result<HashLookup>;
        pub fn dag_manager_runtime_persistence_counters(
            self: &BridgeDagTransactionService,
        ) -> Result<DagPersistenceCounters>;
        #[rust_name = "service_dag_manager_runtime_begin_verify_block_session"]
        pub fn dag_manager_runtime_begin_verify_block_session(
            runtime: &BridgeDagTransactionService,
            input: DagVerifyBlockSessionInput,
        ) -> Result<()>;
        #[rust_name = "service_dag_manager_runtime_verify_block_session_next"]
        pub fn dag_manager_runtime_verify_block_session_next(
            runtime: &BridgeDagTransactionService,
        ) -> Result<DagVerifyBlockSessionStep>;
        #[rust_name = "service_dag_manager_runtime_verify_block_session_report_transactions"]
        pub fn dag_manager_runtime_verify_block_session_report_transactions(
            runtime: &BridgeDagTransactionService,
            report: DagVerifyBlockTransactionReport,
        ) -> Result<DagVerifyBlockSessionStep>;
        #[rust_name = "service_dag_manager_runtime_verify_block_session_report_authorization"]
        pub fn dag_manager_runtime_verify_block_session_report_authorization(
            runtime: &BridgeDagTransactionService,
            report: DagVerifyBlockAuthorizationReport,
        ) -> Result<DagVerifyBlockSessionStep>;
        #[rust_name = "service_dag_manager_runtime_verify_block_session_report_vdf"]
        pub fn dag_manager_runtime_verify_block_session_report_vdf(
            runtime: &BridgeDagTransactionService,
            report: DagVerifyBlockVdfReport,
        ) -> Result<DagVerifyBlockSessionStep>;
        #[rust_name = "service_dag_manager_runtime_verify_block_session_report_gas"]
        pub fn dag_manager_runtime_verify_block_session_report_gas(
            runtime: &BridgeDagTransactionService,
            report: DagVerifyBlockGasReport,
        ) -> Result<DagVerifyBlockSessionStep>;
        /// Reads the cursor's current executor instruction; terminal reads remove it.
        /// Missing ids return an invalid-report step and do not mutate retry state.
        #[rust_name = "service_dag_manager_runtime_proposer_session_next"]
        pub fn dag_manager_runtime_proposer_session_next(
            runtime: &BridgeDagTransactionService,
            session_id: u64,
        ) -> Result<DagProposerSessionStep>;
        /// Supplies the requested FinalChain/sortition facts after Rust revalidates its observation.
        /// Any returned error removes the cursor, so callers may also invoke abort safely during generic cleanup.
        #[rust_name = "service_dag_manager_runtime_proposer_session_report_external_facts"]
        pub fn dag_manager_runtime_proposer_session_report_external_facts(
            runtime: &BridgeDagTransactionService,
            session_id: u64,
            report: DagProposerExternalProposalFactsReport,
        ) -> Result<DagProposerSessionStep>;
        #[rust_name = "service_dag_manager_runtime_proposer_session_report_transactions"]
        pub fn dag_manager_runtime_proposer_session_report_transactions(
            runtime: &BridgeDagTransactionService,
            session_id: u64,
            report: DagProposerTransactionPackReport,
        ) -> Result<DagProposerSessionStep>;
        /// Polls VDF cancellation using the current Rust-derived proposal frontier level.
        /// Missing/out-of-order ids return an invalid-report step; cancellation returns a terminal cancel action.
        #[rust_name = "service_dag_manager_runtime_proposer_session_poll_vdf"]
        pub fn dag_manager_runtime_proposer_session_poll_vdf(
            runtime: &BridgeDagTransactionService,
            session_id: u64,
        ) -> Result<DagProposerSessionStep>;
        /// Supplies proof success and canonical VDF RLP, revalidates the observation, and constructs the unsigned intent.
        /// Success returns signing action 5; stale observations terminate without retry mutation, and construction
        /// errors remove the cursor before throwing across CXX.
        #[rust_name = "service_dag_manager_runtime_proposer_session_report_vdf_proof"]
        pub fn dag_manager_runtime_proposer_session_report_vdf_proof(
            runtime: &BridgeDagTransactionService,
            session_id: u64,
            report: DagProposerVdfProofReport,
        ) -> Result<DagProposerSessionStep>;
        /// Rechecks a stale proof after compatibility sleep and revalidates the complete Rust observation.
        /// An unchanged observation constructs the stored proof's unsigned intent and returns signing action 5; stale
        /// observations terminate without retry mutation, and construction errors remove the cursor.
        #[rust_name = "service_dag_manager_runtime_proposer_session_resume_stale_proof"]
        pub fn dag_manager_runtime_proposer_session_resume_stale_proof(
            runtime: &BridgeDagTransactionService,
            session_id: u64,
        ) -> Result<DagProposerSessionStep>;
        /// Finalizes the stored unsigned intent with a 65-byte recoverable signature.
        /// Success returns add-block action 6 with canonical RLP/hash; malformed signatures and finalization errors remove
        /// the cursor, while missing/out-of-order ids return an invalid terminal step.
        #[rust_name = "service_dag_manager_runtime_proposer_session_report_signing"]
        pub fn dag_manager_runtime_proposer_session_report_signing(
            runtime: &BridgeDagTransactionService,
            session_id: u64,
            report: DagProposerSigningReport,
        ) -> Result<DagProposerSessionStep>;
        #[rust_name = "service_dag_manager_runtime_proposer_session_report_add_block"]
        pub fn dag_manager_runtime_proposer_session_report_add_block(
            runtime: &BridgeDagTransactionService,
            session_id: u64,
            report: DagProposerAddBlockReport,
        ) -> Result<DagProposerSessionStep>;
        pub fn dag_plan_proposer_worker_command(
            input: DagProposerWorkerCommandInput,
        ) -> DagProposerWorkerCommand;
        pub fn dag_verify_vdf_sortition_from_block(
            input: DagVerifyVdfSortitionFromBlockInput,
        ) -> Result<DagVerifyVdfSortitionResult>;
        pub fn dag_vdf_message(pivot: &[u8; 32], transaction_hashes: Vec<DagHash>) -> Vec<u8>;
        pub fn dag_manager_block_from_rlp(block_rlp: Vec<u8>) -> Result<DagManagerBlock>;

        // Consensus PBFT chain

        type BridgePbftService;

        pub fn create_pbft_chain_service_from_storage(
            storage: &BridgeStorage,
        ) -> Result<Box<BridgePbftService>>;
        pub fn pbft_chain_initialized_default(self: &BridgePbftService) -> bool;
        pub fn pbft_chain_head(self: &BridgePbftService) -> PbftChainHeadPayload;
        pub fn pbft_chain_project_legacy_json_head(
            self: &BridgePbftService,
            block_hash: &[u8; 32],
            increments_non_empty_size: bool,
        ) -> Result<PbftChainHeadPayload>;
        pub fn pbft_chain_update(
            self: &BridgePbftService,
            block_hash: &[u8; 32],
            anchor_hash: &[u8; 32],
        ) -> Result<PbftChainHeadPayload>;
        pub fn pbft_chain_block_exists(
            self: &BridgePbftService,
            block_hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn pbft_chain_block_rlp(
            self: &BridgePbftService,
            block_hash: &[u8; 32],
        ) -> Result<PbftBlockStorageLookup>;
        pub fn pbft_chain_validate_block(
            self: &BridgePbftService,
            period: u64,
            prev_hash: &[u8; 32],
        ) -> PbftBlockValidationResult;
        pub fn load_pbft_sync_egress_payload(
            runtime: &BridgePbftService,
            block_period: u64,
            last_block: bool,
            pbft_chain_synced: bool,
            reward_votes_present: bool,
            reward_votes_period: u64,
        ) -> Result<PbftSyncEgressPayload>;
        pub fn pbft_manager_runtime_begin_pbft_sync_admission(
            runtime: &BridgePbftService,
            fact: PbftSyncAdmissionInitialFact,
        );
        pub fn pbft_manager_runtime_pbft_sync_admission_next(
            runtime: &BridgePbftService,
        ) -> PbftSyncAdmissionSessionStep;
        pub fn pbft_manager_runtime_pbft_sync_admission_report_status(
            runtime: &BridgePbftService,
            report: PbftSyncAdmissionStatusReport,
        ) -> PbftSyncAdmissionSessionStep;
        pub fn pbft_manager_runtime_pbft_sync_admission_report_transactions(
            runtime: &BridgePbftService,
            report: PbftSyncAdmissionTransactionReport,
        ) -> PbftSyncAdmissionSessionStep;
        pub fn abort_pbft_manager_runtime_pbft_sync_admission(
            runtime: &BridgePbftService,
        ) -> PbftSyncAdmissionSessionStep;
        pub fn validate_pbft_sync_cert_vote_bundle(
            fact: PbftSyncCertVoteBundleFact,
        ) -> PbftSyncCertVoteBundleValidation;

        pub fn create_pbft_service_from_storage(
            storage: &BridgeStorage,
            config: PbftServiceConfig,
        ) -> Result<Box<BridgePbftService>>;
        pub fn pbft_service_complete_bootstrap(service: &BridgePbftService) -> Result<()>;
        pub fn pbft_manager_runtime_load_startup_replay_period(
            runtime: &BridgePbftService,
            period: u64,
            load_period_lambda: bool,
        ) -> Result<PbftManagerStartupReplayPeriod>;
        pub fn pbft_manager_runtime_snapshot(
            runtime: &BridgePbftService,
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_period_data_queue_snapshot(
            runtime: &BridgePbftService,
            pbft_chain_size: u64,
            current_period: u64,
            chain_last_hash: [u8; 32],
        ) -> PeriodDataQueueSnapshot;
        pub fn pbft_manager_runtime_period_data_queue_clear(runtime: &BridgePbftService);
        pub fn pbft_manager_runtime_period_data_queue_push(
            runtime: &BridgePbftService,
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
        pub fn pbft_manager_runtime_period_data_queue_pop(
            runtime: &BridgePbftService,
        ) -> Result<PeriodDataQueuePopPlan>;
        pub fn pbft_manager_runtime_period_data_queue_clean_old_data(
            runtime: &BridgePbftService,
            period: u64,
        ) -> Vec<PeriodDataQueueEntryRef>;
        pub fn pbft_manager_runtime_begin_pbft_sync_queue_drain(runtime: &BridgePbftService);
        pub fn pbft_manager_runtime_pbft_sync_queue_drain_next(
            runtime: &BridgePbftService,
            queue_size: usize,
            current_period: u64,
        ) -> PbftSyncQueueDrainStep;
        pub fn pbft_manager_runtime_pbft_sync_queue_drain_report(
            runtime: &BridgePbftService,
            report: PbftSyncQueueDrainReport,
        ) -> PbftSyncQueueDrainReportResult;
        pub fn plan_pbft_manager_startup_replay_ranges(
            fact: PbftManagerStartupReplayRangeFact,
        ) -> PbftManagerStartupReplayRangePlan;
        pub fn pbft_manager_runtime_plan_advance_period_after_reset(
            runtime: &BridgePbftService,
            pbft_chain_size: u64,
        ) -> PbftManagerAdvancePeriodPlan;
        pub fn validate_pbft_manager_advance_period_action_report(
            plan: &PbftManagerAdvancePeriodPlan,
            report: PbftManagerAdvancePeriodActionReport,
        ) -> PbftManagerAdvancePeriodActionReportResult;
        pub fn pbft_manager_runtime_apply_period_advance(
            runtime: &BridgePbftService,
            new_period: u64,
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_service_cleanup_period_state(
            service: &BridgePbftService,
            finalized_chain_size: u64,
            new_period: u64,
        ) -> Result<PbftPeriodStateCleanupResult>;
        pub fn pbft_manager_runtime_apply_broadcast_counters(
            runtime: &BridgePbftService,
            broadcast_votes_counter: u32,
            rebroadcast_votes_counter: u32,
            broadcast_reward_votes_counter: u32,
            rebroadcast_reward_votes_counter: u32,
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_cert_voted_block_in_round(
            runtime: &BridgePbftService,
        ) -> Result<Vec<u8>>;
        pub fn pbft_manager_runtime_save_cert_voted_block_in_round(
            runtime: &BridgePbftService,
            period: u64,
            round: u32,
            block_hash: [u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<PbftManagerRuntimeSnapshot>;
        pub fn pbft_manager_runtime_apply_cert_voted_block_metadata(
            runtime: &BridgePbftService,
            period: u64,
            round: u32,
            block_hash: [u8; 32],
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_has_cached_anchor_dag_order(
            runtime: &BridgePbftService,
            anchor_hash: &[u8; 32],
        ) -> bool;
        pub fn pbft_manager_runtime_record_cached_anchor_dag_order(
            runtime: &BridgePbftService,
            anchor_hash: [u8; 32],
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_remove_cached_anchor_dag_order(
            runtime: &BridgePbftService,
            anchor_hash: [u8; 32],
        ) -> PbftManagerRuntimeSnapshot;
        pub fn pbft_manager_runtime_own_pillar_block_vote(
            runtime: &BridgePbftService,
        ) -> Result<Vec<u8>>;
        pub fn pbft_manager_runtime_execute_lifecycle_transition(
            runtime: &BridgePbftService,
            request: PbftManagerLifecycleTransitionRequest,
        ) -> Result<PbftManagerLifecycleTransitionResult>;
        pub fn pbft_manager_runtime_apply_executed_block_reset(
            runtime: &BridgePbftService,
        ) -> Result<PbftManagerRuntimeStorageApplyResult>;
        pub fn pbft_manager_runtime_apply_next_voted_status(
            runtime: &BridgePbftService,
            status: u8,
        ) -> Result<PbftManagerRuntimeSnapshot>;
        pub fn pbft_manager_runtime_apply_cursor_field(
            runtime: &BridgePbftService,
            field: u8,
            value: u32,
        ) -> Result<PbftManagerRuntimeSnapshot>;
        pub fn pbft_manager_runtime_dag_block_period(
            runtime: &BridgePbftService,
            hash: &[u8; 32],
        ) -> Result<BlockPeriodLookup>;
        pub fn pbft_manager_runtime_pbft_block_in_db(
            runtime: &BridgePbftService,
            hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn pbft_manager_runtime_plan_finalization_dynamic_lambda(
            runtime: &BridgePbftService,
            fact: PbftDynamicLambdaFact,
        ) -> Result<PbftManagerFinalizationDynamicLambdaPlan>;
        pub fn pbft_manager_runtime_plan_finalization_intent(
            runtime: &BridgePbftService,
            fact: PbftFinalizationIntentFact,
        ) -> PbftFinalizationIntentPlan;
        pub fn pbft_manager_runtime_start_finalization_executor(
            runtime: &BridgePbftService,
            request: PbftFinalizationExecutorStartRequest,
        ) -> Result<PbftManagerFinalizationExecutorState>;
        pub fn pbft_manager_runtime_fail_finalization_external_effect(
            runtime: &BridgePbftService,
            cursor: u32,
            status: u8,
            error_code: String,
        ) -> Result<PbftManagerFinalizationExecutorState>;
        pub fn pbft_manager_runtime_advance_finalization_transaction_status(
            runtime: &BridgePbftService,
            cursor: u32,
            report: TransactionManagerFinalizedStatusCommandReport,
        ) -> Result<PbftManagerFinalizationExecutorState>;
        pub fn pbft_manager_runtime_advance_finalization_dag_order(
            runtime: &BridgePbftService,
            cursor: u32,
            finalized_count: u64,
        ) -> Result<PbftManagerFinalizationExecutorState>;
        pub fn pbft_manager_runtime_advance_finalization_sortition_commit(
            runtime: &BridgePbftService,
            cursor: u32,
            report: PbftManagerFinalizationSortitionCommitReport,
        ) -> Result<PbftManagerFinalizationExecutorState>;
        pub fn pbft_manager_runtime_advance_finalization_reward_votes_reset(
            runtime: &BridgePbftService,
            cursor: u32,
            report: PbftManagerFinalizationRewardVotesResetReport,
        ) -> Result<PbftManagerFinalizationExecutorState>;
        pub fn pbft_manager_runtime_advance_finalization_final_chain_dispatch(
            runtime: &BridgePbftService,
            cursor: u32,
            report: PbftManagerFinalizationFinalChainDispatchReport,
        ) -> Result<PbftManagerFinalizationExecutorState>;
        pub fn pbft_manager_runtime_advance_finalization_pillar_post_processing(
            runtime: &BridgePbftService,
            cursor: u32,
            report: PbftManagerFinalizationPillarPostProcessingReport,
        ) -> Result<PbftManagerFinalizationExecutorState>;
        pub fn pbft_manager_runtime_advance_finalization_advance_period(
            runtime: &BridgePbftService,
            cursor: u32,
            report: PbftManagerFinalizationAdvancePeriodReport,
        ) -> Result<PbftManagerFinalizationExecutorState>;
        pub fn pbft_manager_runtime_begin_session(
            runtime: &BridgePbftService,
            fact: PbftManagerRuntimeTickFact,
        );
        pub fn plan_pbft_manager_runtime_sleep_until_next_step(
            runtime: &BridgePbftService,
            round_elapsed_ms: i64,
        ) -> PbftManagerSleepPlan;
        pub fn plan_pbft_manager_finalization_wait(
            fact: PbftManagerFinalizationWaitFact,
        ) -> PbftManagerFinalizationWaitPlan;
        pub fn plan_pbft_manager_eligible_wallet_period_wait(
            fact: PbftManagerEligibleWalletPeriodWaitFact,
        ) -> PbftManagerEligibleWalletPeriodWaitPlan;
        pub fn pbft_manager_runtime_begin_state_action_effect_session(
            runtime: &BridgePbftService,
            fact: PbftManagerStateActionFact,
        );
        pub fn pbft_manager_runtime_state_action_effect_session_next(
            runtime: &BridgePbftService,
        ) -> PbftManagerStateActionSessionStep;
        pub fn pbft_manager_runtime_state_action_effect_session_report(
            runtime: &BridgePbftService,
            report: PbftManagerStateActionEffectReport,
        ) -> PbftManagerStateActionSessionStep;
        pub fn pbft_manager_runtime_begin_proposal_session(
            runtime: &BridgePbftService,
            fact: PbftManagerProposalInitialFact,
        );
        pub fn pbft_manager_proposal_session_next(
            runtime: &BridgePbftService,
        ) -> PbftManagerProposalSessionStep;
        pub fn pbft_manager_proposal_session_report_dag_order(
            runtime: &BridgePbftService,
            report: PbftManagerProposalDagOrderReport,
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
        pub fn plan_pbft_manager_candidate_admission(
            fact: PbftManagerCandidateAdmissionFact,
        ) -> PbftManagerCandidateAdmissionPlan;
        pub fn plan_pbft_manager_leader_candidates(
            candidates: Vec<PbftManagerLeaderCandidateInputFact>,
        ) -> PbftManagerLeaderCandidatePlan;
        pub fn pbft_manager_runtime_session_next(
            runtime: &BridgePbftService,
        ) -> PbftManagerRuntimeSessionStep;
        pub fn pbft_manager_runtime_session_report(
            runtime: &BridgePbftService,
            report: PbftManagerRuntimeActionReport,
        ) -> PbftManagerRuntimeSessionStep;
        pub fn abort_pbft_manager_runtime_session(runtime: &BridgePbftService);
        // Consensus proposed PBFT blocks

        pub fn pbft_service_proposed_blocks_push_with_storage(
            self: &BridgePbftService,
            period: u64,
            block_hash: &[u8; 32],
            pivot_hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<bool>;
        pub fn pbft_service_proposed_blocks_mark_valid(
            self: &BridgePbftService,
            period: u64,
            block_hash: &[u8; 32],
        ) -> Result<()>;
        pub fn pbft_service_proposed_blocks_get(
            self: &BridgePbftService,
            period: u64,
            block_hash: &[u8; 32],
        ) -> ProposedBlockLookup;
        pub fn pbft_service_proposed_blocks_metadata(
            self: &BridgePbftService,
            period: u64,
            block_hash: &[u8; 32],
        ) -> ProposedBlockMetadataLookup;
        pub fn pbft_service_proposed_blocks_contains(
            self: &BridgePbftService,
            period: u64,
            block_hash: &[u8; 32],
        ) -> bool;
        pub fn pbft_service_proposed_blocks_cleanup_with_storage(
            self: &BridgePbftService,
            period: u64,
        ) -> Result<Vec<ProposedBlockPeriodHashes>>;
        pub fn pbft_service_proposed_blocks_snapshot_entries(
            self: &BridgePbftService,
        ) -> Vec<ProposedBlockSnapshotEntry>;
        pub fn proposed_blocks_storage_push_with_storage(
            storage: &BridgeStorage,
            period: u64,
            block_hash: &[u8; 32],
            pivot_hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<bool>;
        pub fn proposed_blocks_storage_snapshot_entries(
            storage: &BridgeStorage,
        ) -> Result<Vec<ProposedBlockSnapshotEntry>>;
        pub fn proposed_blocks_local_candidate_lookups(
            candidates: Vec<ProposedBlockSnapshotEntry>,
            identities: Vec<ProposedBlockIdentity>,
        ) -> Vec<ProposedBlockLookup>;

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
        pub fn preview_finalized_period_rewards_stats(
            self: &BridgeRewardsStatsRuntime,
            fact: RewardsStatsProcessFact,
        ) -> RewardsStatsProcessResult;
        pub fn rewards_stats_runtime_commit_process_result(
            self: &mut BridgeRewardsStatsRuntime,
            plan: &RewardsStatsProcessResult,
        ) -> Result<RewardsStatsApplyResult>;
        pub fn rewards_stats_runtime_clear_committed(
            self: &mut BridgeRewardsStatsRuntime,
            current_period: u64,
        );
        pub fn rewards_stats_runtime_cached_stats(
            self: &BridgeRewardsStatsRuntime,
        ) -> Vec<PeriodRlp>;
        pub fn rewards_stats_append_storage_writes_to_batch(
            batch: &mut BridgeStorageBatch,
            plan: &RewardsStatsProcessResult,
        ) -> Result<RewardsStatsApplyResult>;
        pub fn rewards_stats_runtime_clear_storage_and_state(
            self: &mut BridgeRewardsStatsRuntime,
            current_period: u64,
            sync: bool,
        ) -> Result<RewardsStatsApplyResult>;
        // Consensus slashing proof planner

        type BridgeSlashingProofPlanner;

        pub fn create_slashing_proof_planner(
            report_malicious_behaviour: bool,
            magnolia_activation_period: u64,
        ) -> Result<Box<BridgeSlashingProofPlanner>>;
        pub fn slashing_plan_double_voting_proof(
            self: &BridgeSlashingProofPlanner,
            input: DoubleVotingProofInput,
        ) -> Result<DoubleVotingProofPlan>;
        pub fn slashing_report_double_voting_proof_submission(
            self: &BridgeSlashingProofPlanner,
            report: DoubleVotingProofSubmissionReport,
        ) -> Result<bool>;

        // Consensus transaction manager planning

        pub fn transaction_manager_runtime_gas_price_bid(
            self: &BridgeDagTransactionService,
        ) -> [u8; 32];
        pub fn transaction_manager_runtime_gas_price_update(
            self: &BridgeDagTransactionService,
            gas_prices: Vec<GasPricerGasPrice>,
        );
        pub fn transaction_manager_runtime_pack_prepare_sharded(
            self: &BridgeDagTransactionService,
            weight_limit: u64,
            min_transaction_gas: u64,
            proposal_period: u64,
            estimate_gas_limit: u64,
            last_block_number: u64,
            total_shards: u16,
            node_shard: u16,
            shard_period_interval: u64,
        ) -> Result<TransactionPackPreparedPlan>;
        pub fn transaction_manager_runtime_pack_finalize_with_estimates(
            self: &BridgeDagTransactionService,
            inputs: Vec<TransactionPackSessionEstimateInput>,
        ) -> Result<TransactionPackSessionStep>;
        pub fn transaction_manager_runtime_pack_abort(self: &BridgeDagTransactionService) -> bool;
        pub fn transaction_manager_runtime_plan_gas_estimation(
            self: &BridgeDagTransactionService,
            fact: TransactionManagerGasEstimationFact,
        ) -> Result<TransactionManagerGasEstimationPlan>;
        pub fn transaction_manager_runtime_store_gas_estimation(
            self: &BridgeDagTransactionService,
            result: TransactionManagerGasEstimationResult,
        ) -> Result<bool>;
        pub fn transaction_manager_runtime_transaction_count(
            self: &BridgeDagTransactionService,
        ) -> u64;
        /// Returns Rust's known-transaction decision from runtime-owned queue and sidecar state.
        pub fn transaction_manager_runtime_is_transaction_known_hash(
            self: &BridgeDagTransactionService,
            hash: &[u8; 32],
        ) -> Result<bool>;
        /// Inserts payloads and moves them into recently-finalized sidecar state in one Rust command.
        pub fn transaction_manager_runtime_initialize_recently_finalized_payloads(
            self: &BridgeDagTransactionService,
            period: u64,
            payloads: Vec<TransactionManagerSidecarInsertInput>,
        ) -> Result<()>;
        pub fn transaction_manager_runtime_non_finalized_size(
            self: &BridgeDagTransactionService,
        ) -> usize;
        pub fn transaction_manager_runtime_remove_non_finalized(
            self: &BridgeDagTransactionService,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<u64>;
        /// Executes admission with FinalChain facts supplied by the C++ external-EVM boundary.
        pub fn transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts_command_report(
            self: &BridgeDagTransactionService,
            fact: TransactionManagerValidatedInsertRuntimeFact,
            final_chain_fact: TransactionManagerFinalChainAdmissionFact,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionManagerAdmissionCommandReport>;
        /// Executes public insertTransaction verification and fact-backed admission as one Rust-owned command.
        pub fn transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_facts_command_report(
            self: &BridgeDagTransactionService,
            verify_fact: TransactionManagerVerifyTransactionFact,
            admission_fact: TransactionManagerValidatedInsertRuntimeFact,
            final_chain_fact: TransactionManagerFinalChainAdmissionFact,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionManagerPublicAdmissionCommandReport>;
        /// Resolves requested hashes against Rust-owned live queue payloads only.
        pub fn transaction_manager_runtime_queue_lookup_transaction_views(
            self: &BridgeDagTransactionService,
            requests: Vec<TransactionManagerTransactionViewRequest>,
        ) -> Result<Vec<TransactionManagerTransactionView>>;
        pub fn transaction_manager_runtime_queue_all_transaction_groups(
            self: &BridgeDagTransactionService,
        ) -> Vec<TransactionQueueTransactionGroup>;
        pub fn transaction_manager_runtime_queue_size(self: &BridgeDagTransactionService) -> usize;
        pub fn transaction_manager_runtime_queue_proposable_accounts(
            self: &BridgeDagTransactionService,
        ) -> Vec<TransactionQueueProposableAccountFact>;
        pub fn transaction_manager_runtime_queue_block_finalized(
            self: &BridgeDagTransactionService,
            block_number: u64,
        ) -> Vec<TransactionQueueHash>;
        pub fn transaction_manager_runtime_queue_transactions_dropped(
            self: &BridgeDagTransactionService,
        ) -> bool;
        pub fn transaction_manager_runtime_queue_non_proposable_over_limit(
            self: &BridgeDagTransactionService,
        ) -> bool;
        pub fn transaction_manager_runtime_queue_min_gas_price_for_block_inclusion(
            self: &BridgeDagTransactionService,
            limit: u64,
        ) -> [u8; 32];
        /// Resolves requested hashes against non-finalized/recently-finalized sidecars.
        pub fn transaction_manager_runtime_lookup_non_finalized_transaction_views(
            self: &BridgeDagTransactionService,
            requests: Vec<TransactionManagerTransactionViewRequest>,
        ) -> Result<Vec<TransactionManagerTransactionView>>;
        /// Resolves requested hashes through queue, sidecars, then Rust storage.
        pub fn transaction_manager_runtime_lookup_transaction_views(
            self: &BridgeDagTransactionService,
            requests: Vec<TransactionManagerTransactionViewRequest>,
            max_count: u64,
        ) -> Result<TransactionManagerTransactionViewPlan>;
        /// Resolves requested hashes through queue, sidecars, then proposal-filtered Rust storage.
        pub fn transaction_manager_runtime_lookup_proposal_transaction_views_with_account_nonce_facts(
            self: &BridgeDagTransactionService,
            proposal_period: u64,
            requests: Vec<TransactionManagerTransactionViewRequest>,
            account_nonce_facts: Vec<TransactionQueueAccountNonceFact>,
            max_count: u64,
        ) -> Result<TransactionManagerTransactionViewPlan>;
        /// Applies DAG transaction persistence and returns a typed command report.
        #[rust_name = "service_save_transactions_from_dag_block_command_report_with_runtime"]
        pub fn save_transactions_from_dag_block_command_report_with_runtime(
            runtime: &BridgeDagTransactionService,
            facts: Vec<DagTransactionSaveSidecarFact>,
        ) -> Result<TransactionManagerDagSaveCommandReport>;
        /// Applies finalized status updates plus periodic purge and returns a typed command report.
        #[rust_name = "service_update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts"]
        pub fn update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts(
            runtime: &BridgeDagTransactionService,
            period: u64,
            retention_window: u64,
            account_nonce_facts: Vec<TransactionQueueAccountNonceFact>,
            facts: Vec<FinalizedTransactionStatusSidecarFact>,
        ) -> Result<TransactionManagerFinalizedStatusCommandReport>;
        /// Builds deterministic TransactionManager::verifyTransaction admission plan.
        pub fn transaction_manager_verify_transaction(
            fact: TransactionManagerVerifyTransactionFact,
        ) -> Result<TransactionManagerVerifyTransactionOutcome>;
        #[rust_name = "service_transaction_manager_filter_non_finalized_with_runtime"]
        pub fn transaction_manager_filter_non_finalized_with_runtime(
            runtime: &BridgeDagTransactionService,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<FinalizedTransactionFilterPlan>;
        #[rust_name = "service_transaction_manager_verify_not_finalized_with_runtime"]
        pub fn transaction_manager_verify_not_finalized_with_runtime(
            runtime: &BridgeDagTransactionService,
            facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
        ) -> Result<TransactionManagerVerifyNotFinalizedOutcome>;
        /// Rebuilds runtime recovery sidecars from Rust-backed storage.
        #[rust_name = "service_transaction_manager_recover_nonfinalized_with_runtime"]
        pub fn transaction_manager_recover_nonfinalized_with_runtime(
            runtime: &BridgeDagTransactionService,
        ) -> Result<()>;

        // Consensus verified votes

        pub fn pbft_service_verified_votes_own_vote_records(
            self: &BridgePbftService,
        ) -> Result<Vec<PbftVoteStorageRecord>>;
        pub fn pbft_service_verified_votes_size(self: &BridgePbftService) -> Result<u64>;
        pub fn pbft_service_verified_votes_replay_contains(
            self: &BridgePbftService,
            vote_hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn pbft_service_verified_votes_replay_insert(
            self: &BridgePbftService,
            vote_hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn pbft_service_verified_votes_two_t_plus_one_threshold(
            self: &BridgePbftService,
            fact: PbftTwoTPlusOneThresholdFact,
        ) -> Result<PbftTwoTPlusOneThresholdPlan>;
        pub fn pbft_service_verified_votes_validate_canonical_vote(
            self: &BridgePbftService,
            canonical_vote_rlp: &[u8],
            validation_facts: PbftVoteValidationExternalFacts,
        ) -> Result<PbftVoteRuntimeValidationResult>;
        pub fn pbft_service_verified_votes_insert_unique_voter(
            self: &BridgePbftService,
            vote: VerifiedVotePayload,
            weighted_vote: PbftVoteStorageRecord,
        ) -> Result<UniqueVoterInsertOutcome>;
        pub fn pbft_service_verified_votes_insert_voted_value(
            self: &BridgePbftService,
            vote: VerifiedVotePayload,
            weighted_vote: PbftVoteStorageRecord,
        ) -> Result<VotedValueInsertOutcome>;
        pub fn pbft_service_verified_votes_insert_vote_atomic(
            self: &BridgePbftService,
            vote: VerifiedVotePayload,
            weighted_vote: PbftVoteStorageRecord,
        ) -> Result<AtomicVoteInsertOutcome>;
        pub fn pbft_service_verified_votes_apply_threshold_decision(
            self: &BridgePbftService,
            vote: VerifiedVotePayload,
            total_weight: u64,
            two_t_plus_one_threshold: u64,
        ) -> Result<ThresholdDecisionOutcome>;
        pub fn pbft_service_verified_votes_set_network_t_plus_one_step(
            self: &BridgePbftService,
            period: u64,
            round: u64,
            step: u64,
        ) -> Result<bool>;
        pub fn pbft_service_verified_votes_determine_new_round(
            self: &BridgePbftService,
            period: u64,
            current_round: u64,
        ) -> Result<DetermineNewRoundOutcome>;
        pub fn pbft_service_verified_votes_insert_two_t_plus_one_voted_block(
            self: &BridgePbftService,
            period: u64,
            round: u64,
            kind: u8,
            block_hash: &[u8; 32],
            step: u64,
        ) -> Result<TwoTPlusOneInsertOutcome>;
        pub fn pbft_service_verified_votes_get_two_t_plus_one_voted_block(
            self: &BridgePbftService,
            period: u64,
            round: u64,
            kind: u8,
        ) -> Result<TwoTPlusOneVotedBlockLookup>;
        pub fn pbft_service_verified_votes_get_two_t_plus_one_voted_block_payloads(
            self: &BridgePbftService,
            period: u64,
            round: u64,
            kind: u8,
        ) -> Result<TwoTPlusOneVotePayloadsLookup>;
        pub fn pbft_service_verified_votes_plan_next_votes_bundle_egress(
            self: &BridgePbftService,
            period: u64,
            round: u64,
        ) -> Result<PbftNextVotesBundleEgressPlan>;
        pub fn pbft_service_verified_votes_build_optimized_votes_bundle_egress(
            self: &BridgePbftService,
            request: PbftOptimizedVoteBundleBuildRequest,
        ) -> Result<PbftOptimizedVoteBundleBuildResult>;
        pub fn pbft_service_verified_votes_cleanup_votes_by_period(
            self: &BridgePbftService,
            pbft_period: u64,
        ) -> Result<()>;
        pub fn pbft_service_verified_votes_add_verified_vote(
            self: &BridgePbftService,
            vote: VerifiedVotePayload,
            weighted_vote: PbftVoteStorageRecord,
            two_t_plus_one_threshold: u64,
            apply_threshold_decision: bool,
        ) -> Result<VerifiedVoteAddOutcome>;
        pub fn pbft_service_verified_votes_admit_and_persist(
            self: &BridgePbftService,
            canonical_vote_rlp: &[u8],
            validation_facts: PbftVoteValidationExternalFacts,
            flags: PbftVoteEventFactFlags,
            context: PbftVoteProgressContext,
        ) -> Result<PbftVoteAdmissionRuntimeResult>;
        pub fn pbft_service_verified_votes_weighted_payload(
            self: &BridgePbftService,
            vote_hash: &[u8; 32],
        ) -> Result<PbftVotePayloadLookup>;
        pub fn pbft_service_verified_votes_select_reward_vote_payloads(
            self: &BridgePbftService,
            block_period: u64,
            requested_vote_hashes: Vec<PbftFinalizationHash>,
        ) -> Result<PbftRewardVotePayloadSelection>;
        pub fn pbft_service_verified_votes_reward_vote_cursor(
            self: &BridgePbftService,
        ) -> Result<RewardVoteCursorSnapshot>;
        pub fn pbft_service_verified_votes_reward_vote_period(
            self: &BridgePbftService,
        ) -> Result<u64>;
        pub fn pbft_service_verified_votes_commit_reward_vote_cursor(
            self: &BridgePbftService,
            write_intent: &PbftFinalizationStorageWritePlan,
            reset_generation: u64,
        ) -> Result<RewardVoteCursorCommitResult>;
        pub fn pbft_service_verified_votes_state_snapshot(
            self: &BridgePbftService,
        ) -> Result<VerifiedVotesStateSnapshot>;
        pub fn pbft_service_verified_votes_step_payloads(
            self: &BridgePbftService,
            period: u64,
            round: u64,
            step: u64,
        ) -> Result<VerifiedStepVotePayloadsLookup>;
        pub fn pbft_service_verified_votes_current_reward_snapshot(
            self: &BridgePbftService,
        ) -> Result<RewardVotePayloadSnapshot>;
        pub fn pbft_service_verified_votes_save_own_verified_vote(
            self: &BridgePbftService,
            record: PbftVoteStorageRecord,
        ) -> Result<PbftVotePersistenceResult>;
        pub fn pbft_service_verified_votes_clear_own_verified_votes(
            self: &BridgePbftService,
        ) -> Result<PbftVotePersistenceResult>;
        pub fn pbft_service_verified_votes_persist_pbft_vote_progress(
            self: &BridgePbftService,
            write: PbftVoteProgressPersistenceWrite,
        ) -> Result<PbftVotePersistenceResult>;
        pub fn pbft_service_verified_votes_apply_pbft_finalization_storage_writes(
            self: &BridgePbftService,
            write_intent: &PbftFinalizationStorageWritePlan,
            stages: Vec<PbftFinalizationStorageWriteStage>,
            sync: bool,
        ) -> Result<PbftFinalizedPeriodApplyResult>;
        pub fn pbft_service_verified_votes_prepare_reward_votes_reset_stage(
            self: &BridgePbftService,
            write_intent: &PbftFinalizationStorageWritePlan,
        ) -> Result<PbftFinalizationStorageWriteStage>;
        pub fn pbft_service_verified_votes_apply_reward_votes_reset(
            self: &BridgePbftService,
            request: PbftRewardVotesResetRequest,
        ) -> Result<PbftFinalizedPeriodApplyResult>;
        pub fn pbft_service_prepare_leader_selection(
            self: &BridgePbftService,
            period: u64,
            round: u64,
        ) -> Result<PbftLeaderSelectionSnapshot>;
        pub fn pbft_service_finish_leader_selection(
            self: &BridgePbftService,
            request: PbftLeaderSelectionFinishRequest,
        ) -> Result<PbftLeaderSelectionResult>;

        pub fn pbft_inspect_canonical_vote(vote_rlp: &[u8]) -> Result<PbftCanonicalVoteInspection>;
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
        pub fn pbft_vote_bundle_payload_from_records(
            records: Vec<PbftVoteStorageRecord>,
        ) -> Result<Vec<u8>>;
        pub fn pbft_proposer_sortition_plan(
            fact: PbftProposerSortitionFact,
        ) -> Result<PbftProposerSortitionPlan>;

        // Consensus pillar votes

        pub fn pillar_vote_inspect(vote_rlp: &[u8]) -> Result<PillarVoteInspection>;
        pub fn pillar_chain_runtime_plan_block_creation(
            self: &BridgePillarChainRuntime,
            request: PillarBlockCreationRequest,
            current_vote_counts: Vec<PillarValidatorVoteCount>,
        ) -> Result<PillarBlockCreationWithVoteCountsPlan>;
        pub fn pillar_chain_runtime_plan_block_linkage(
            self: &BridgePillarChainRuntime,
            request: PillarBlockLinkageRequest,
        ) -> Result<PillarBlockLinkagePlan>;
        pub fn pillar_chain_runtime_latest_finalized_block_rlp(
            self: &BridgePillarChainRuntime,
        ) -> Result<Vec<u8>>;

        type BridgePillarChainStorage;
        type BridgePillarChainRuntime;

        pub fn create_pillar_chain_storage(
            storage: &BridgeStorage,
        ) -> Box<BridgePillarChainStorage>;
        pub fn create_pillar_chain_runtime(
            storage: &BridgeStorage,
        ) -> Result<Box<BridgePillarChainRuntime>>;
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
        pub fn pillar_chain_storage_load_block(
            self: &BridgePillarChainStorage,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn pillar_chain_runtime_apply_current_block_data(
            self: &BridgePillarChainRuntime,
            data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn pillar_chain_runtime_apply_own_vote(
            self: &BridgePillarChainRuntime,
            vote_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn pillar_chain_runtime_load_startup_bootstrap(
            self: &BridgePillarChainRuntime,
        ) -> Result<PillarChainStartupBootstrap>;
        pub fn pillar_chain_runtime_plan_current_anchor_decision(
            self: &BridgePillarChainRuntime,
            request: PillarCurrentAnchorDecisionRequest,
        ) -> Result<PillarCurrentAnchorDecisionResult>;
        pub fn pillar_chain_runtime_consensus_threshold(
            self: &BridgePillarChainRuntime,
            total_vote_count: u64,
        ) -> u64;
        pub fn pillar_chain_runtime_prepare_single_vote_admission(
            self: &BridgePillarChainRuntime,
            vote_rlp: Vec<u8>,
            context: PillarVoteSingleAdmissionContext,
        ) -> Result<PillarVoteSingleAdmissionPreparePlan>;
        pub fn pillar_chain_runtime_prepare_trusted_single_vote_admission(
            self: &BridgePillarChainRuntime,
            vote_rlp: Vec<u8>,
        ) -> Result<PillarVoteSingleAdmissionPreparePlan>;
        pub fn pillar_chain_runtime_plan_vote_relevance(
            self: &BridgePillarChainRuntime,
            vote_rlp: Vec<u8>,
            context: PillarVoteRuntimeRelevanceContext,
        ) -> Result<PillarVoteRelevancePlan>;
        pub fn pillar_chain_runtime_apply_prepared_single_vote_admission(
            self: &mut BridgePillarChainRuntime,
            input: PillarVoteSingleAdmissionApplyInput,
        ) -> Result<PillarVoteSingleAdmissionApplyPlan>;
        pub fn pillar_chain_runtime_prepare_weighted_rlp_bundle(
            self: &BridgePillarChainRuntime,
            vote_rlps: Vec<PillarVoteRlpPayload>,
            required_votes_period: u64,
        ) -> Result<PillarVoteWeightedBundlePreparePlan>;
        pub fn pillar_chain_runtime_apply_weighted_rlp_bundle(
            self: &mut BridgePillarChainRuntime,
            input: PillarVoteWeightedBundleApplyInput,
        ) -> Result<PillarVoteBundleApplyPlan>;
        pub fn pillar_chain_runtime_get_verified_vote_payloads(
            self: &BridgePillarChainRuntime,
            period: u64,
            block_hash: &[u8; 32],
            above_threshold: bool,
        ) -> Result<PillarVotesPayloadLookup>;
        pub fn pillar_chain_runtime_build_verified_vote_network_bundles(
            self: &BridgePillarChainRuntime,
            period: u64,
            block_hash: &[u8; 32],
            max_votes_per_bundle: usize,
        ) -> Result<PillarVoteNetworkBundleLookup>;
        pub fn pillar_chain_runtime_prepare_finalized_block_for_pbft(
            self: &mut BridgePillarChainRuntime,
            request: PillarBlockFinalizationRequest,
        ) -> Result<PillarBlockFinalizationPrepareResult>;
        pub fn pillar_chain_runtime_ack_finalize_block_for_pbft(
            self: &mut BridgePillarChainRuntime,
            request: PillarBlockFinalizationAcknowledgeRequest,
        ) -> Result<PillarBlockFinalizationAcknowledgeResult>;

        // Consensus sortition

        type BridgeSortitionParamsManager;

        pub fn create_sortition_params_manager_from_storage(
            config: SortitionRuntimeConfig,
            storage: &BridgeStorage,
        ) -> Result<Box<BridgeSortitionParamsManager>>;
        pub fn sortition_current_params(
            self: &BridgeSortitionParamsManager,
        ) -> SortitionRuntimeParams;
        pub fn sortition_params_for_period_from_storage(
            self: &BridgeSortitionParamsManager,
            period: u64,
        ) -> Result<SortitionRuntimeParams>;
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
        pub fn storage_shim_clear_block_rewards_stats(storage: &BridgeStorage) -> Result<()>;
        pub fn storage_shim_set_genesis_hash(
            storage: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<()>;
        #[allow(clippy::too_many_arguments)]
        pub fn storage_shim_seed_final_chain_conformance_lookup_rows(
            storage: &BridgeStorage,
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
        pub fn storage_shim_save_cert_voted_block_in_round(
            batch: &mut BridgeStorageBatch,
            round: u64,
            block_rlp: Vec<u8>,
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
        pub fn storage_shim_save_own_verified_vote(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
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
        pub fn storage_shim_save_extra_reward_vote(
            batch: &mut BridgeStorageBatch,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
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

        pub fn dag_block_in_db(self: &BridgeDagStorageQueries, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_dag_block(self: &BridgeDagStorageQueries, hash: &[u8; 32]) -> Result<Vec<u8>>;
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

        // FinalChain

        type BridgeFinalChain;

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
        pub fn get_dag_proposer_final_chain_facts(
            self: &BridgeFinalChain,
            proposal_period_found: bool,
            proposal_period: u64,
            sender: &[u8; 20],
        ) -> Result<DagProposerFinalChainFacts>;
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
        pub fn get_vrf_key_at_block(
            self: &BridgeFinalChain,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<Vec<u8>>;
        pub fn call(
            self: &BridgeFinalChain,
            request: FinalChainCall,
        ) -> Result<FinalChainCallOutcome>;
        type BridgeFinalChainExecutionSession;
        pub fn create_final_chain_execution_session(
            request: FinalChainExecutionRequest,
        ) -> Result<Box<BridgeFinalChainExecutionSession>>;
        pub fn recover_external_evm_pending_publication(
            self: &BridgeFinalChain,
            committed_period: u64,
            committed_state_root: &[u8; 32],
        ) -> Result<FinalChainExternalEvmPublicationReport>;
        type BridgeConsensusExecutionApi;
        pub fn create_consensus_execution_api() -> Result<Box<BridgeConsensusExecutionApi>>;
        pub fn consensus_execution_plan_system_transactions(
            self: &BridgeConsensusExecutionApi,
            fact: FinalChainSystemTransactionPlanFact,
        ) -> Result<FinalChainSystemTransactionPlan>;
        pub fn consensus_execution_commit_session(
            self: &BridgeConsensusExecutionApi,
            final_chain: &BridgeFinalChain,
            session: Box<BridgeFinalChainExecutionSession>,
        ) -> Result<FinalChainExecutionCommitReport>;
        pub fn consensus_execution_next_execution_request(
            self: &BridgeConsensusExecutionApi,
            session: &mut BridgeFinalChainExecutionSession,
        ) -> Result<FinalChainExecutionStep>;
        pub fn consensus_execution_report_execution_result(
            self: &BridgeConsensusExecutionApi,
            session: &mut BridgeFinalChainExecutionSession,
            report: FinalChainEvmExecutionReport,
        ) -> Result<FinalChainExecutionStep>;
        pub fn consensus_execution_report_system_transactions(
            self: &BridgeConsensusExecutionApi,
            session: &mut BridgeFinalChainExecutionSession,
            report: FinalChainSystemTransactionReport,
        ) -> Result<FinalChainExecutionStep>;
        pub fn consensus_execution_report_rewards_result(
            self: &BridgeConsensusExecutionApi,
            session: &mut BridgeFinalChainExecutionSession,
            report: FinalChainEvmRewardsReport,
        ) -> Result<FinalChainExternalEvmCommitReport>;
        pub fn consensus_execution_prepare_external_evm_state_commit(
            self: &BridgeConsensusExecutionApi,
            final_chain: &BridgeFinalChain,
            session: &mut BridgeFinalChainExecutionSession,
            rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate,
            proposal_period_update: FinalChainProposalPeriodDagLevelUpdate,
        ) -> Result<FinalChainExternalEvmStateCommitIntent>;
        pub fn consensus_execution_report_state_commit_result(
            self: &BridgeConsensusExecutionApi,
            final_chain: &BridgeFinalChain,
            session: &mut BridgeFinalChainExecutionSession,
            result: FinalChainExternalEvmStateCommitResult,
        ) -> Result<FinalChainExternalEvmCommitDecision>;
        pub fn consensus_execution_publish_state_commit(
            self: &BridgeConsensusExecutionApi,
            final_chain: &BridgeFinalChain,
            session: &mut BridgeFinalChainExecutionSession,
        ) -> Result<FinalChainExternalEvmPublicationReport>;
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
