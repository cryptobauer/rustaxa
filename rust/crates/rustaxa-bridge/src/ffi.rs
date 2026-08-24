use crate::consensus_host_ports::*;
use crate::dag::*;
pub(crate) use crate::dag_transaction_service::BridgeApp;
pub use crate::dag_transaction_service::BridgeConsensusApplication;
use crate::dag_transaction_service::*;
use crate::final_chain::*;
use crate::network::*;
use crate::network_slashing::*;
use crate::query::*;
use crate::storage::*;
use crate::transaction::*;
use crate::transaction_manager::*;
use crate::vdf::*;
use rustaxa_consensus::ConsensusExecutionApi;
use rustaxa_consensus::ConsensusQueryApi;
use rustaxa_storage::Storage;
use rustaxa_storage::StorageWriteBatch;
use std::sync::Arc;

/// Read-only storage compatibility handle for legacy C++ materializers.
///
/// Operation-shaped methods remain grouped by name while the storage shim is
/// retired; separate per-domain opaque handles are no longer part of the ABI.
pub struct BridgeStorageQueries {
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

/// Opaque state for one in-progress FinalChain execution session.
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
/// Production instances combine a cloned Rust storage handle with a live PBFT
/// read handle owned by the application root. Storage-only instances remain
/// available for isolated query fixtures. The facade returns stable read DTOs
/// without exposing consensus managers, storage iterators, or mutable sidecars.
pub struct BridgeConsensusQueryApi(pub ConsensusQueryApi);

/// Thin CXX adapter over the PBFT-root-owned native network service.
///
/// Cloning the service shares the root's ordered effect queue and sibling
/// protocol owners; this bridge owns no mutex, configuration, or standalone
/// consensus runtime.
pub struct BridgeConsensusNetworkApi {
    pub(crate) network: rustaxa_consensus::ConsensusNetworkService,
    pub(crate) pbft: Arc<rustaxa_consensus::PbftService>,
    pub(crate) final_chain: Arc<rustaxa_consensus::FinalChain>,
}

/// Thin CXX adapter over the CXX-free native PBFT application root.
///
/// The bridge owns no sibling protocol state, storage handle, mutex, or
/// readiness flag. It maps stable CXX inputs and outputs around the native
/// composition without changing the native siblings' lock domains.
#[cxx::bridge(namespace = "rustaxa")]
pub mod rustaxa_ffi {
    struct BlockPeriodLookup {
        found: bool,
        period: u64,
        position: u32,
    }

    /// Canonical encoded bytes shared by storage and host-port list payloads.
    struct CanonicalBytes {
        data: Vec<u8>,
    }

    /// Optional canonical block RLP lookup shared by DAG and PBFT query adapters.
    struct BlockRlpLookup {
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
        blocks: Vec<CanonicalBytes>,
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
        reward_votes: Vec<DagHash>,
        has_extra_data: bool,
        extra_data: PbftBlockExtraDataView,
        dag_blocks_order: Vec<DagHash>,
    }

    /// PBFT block author/version facts for `taraxa_getNodeVersions`.
    struct PbftNodeVersionView {
        found: bool,
        beneficiary: [u8; 20],
        major_version: u16,
        minor_version: u16,
        patch_version: u16,
    }

    /// Canonical signed PBFT certificate-vote bytes shared by query and queue boundaries.
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

    /// Client-oriented live PBFT progress and persisted chain statistics for
    /// `taraxa_getChainStats`.
    struct ChainStatsView {
        pbft_period: u64,
        non_empty_pbft_periods: u64,
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

    /// Rewards distribution frequency rule active from `from_period` onward.
    struct RewardsFrequencyRule {
        from_period: u64,
        frequency: u32,
    }

    /// Previous-block cert-vote fact for rewards-stat planning.
    struct RewardsCertVoteFact {
        voter: [u8; 20],
        weight: u64,
        period: u64,
    }

    struct TxRlp {
        data: Vec<u8>,
        is_system: bool,
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

    /// Fixed-size peer id used by network effect payloads.
    struct NetworkPeerId {
        id: [u8; 64],
    }

    /// Executor-visible network effect planned by Rust consensus.
    struct NetworkEffect {
        effect_id: u64,
        source_payload_id: u64,
        transport_lane: u32,
        kind: u8,
        peer_id: [u8; 64],
        packet_kind: u32,
        payload_bytes: Vec<u8>,
        related_payload_bytes: Vec<u8>,
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
        admission_accepted: bool,
        admission_already_present: bool,
        admission_mark_vote_known: bool,
        admission_gossip_vote: bool,
        admission_report_slashing: bool,
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
        transport_lane: u32,
        peer_id: [u8; 64],
        peer_pbft_chain_size: u64,
        source_payload_id: u64,
        enqueue_admission: bool,
        allow_gossip: bool,
        vote_hash: [u8; 32],
        vote_rlp: Vec<u8>,
        pbft_block_rlp: Vec<u8>,
        pbft_block_hash: [u8; 32],
        pbft_block_period: u64,
    }

    /// Scalar context for authoritative pillar-vote ingress through Network/Tarcap.
    struct NetworkPillarVoteIngressContext {
        transport_lane: u32,
        peer_id: [u8; 64],
        source_payload_id: u64,
        ficus_activation_period: u64,
        allow_gossip: bool,
    }

    /// Operation-specific canonical get-PBFT-sync ingress request.
    struct NetworkGetPbftSyncRequest {
        tarcap_version: u32,
        peer_id: [u8; 64],
        request_rlp: Vec<u8>,
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
        application_effect_id: u64,
    }

    /// One PBFT vote packet after native routing and root-owned admission.
    ///
    /// `has_admission` is false for routing rejection and for a bundle member
    /// cancelled by an earlier slashing conflict. Transport follow-ups are
    /// already queued; the optional runtime result exposes only terminal
    /// admission facts and the named slashing transaction leaf.
    struct NetworkPbftVoteAdmissionOutcome {
        decision: NetworkIngressDecision,
        has_admission: bool,
        accepted: bool,
        already_present: bool,
        mark_vote_known: bool,
        gossip_vote: bool,
        report_slashing: bool,
        has_slashing_transaction_effect: bool,
        slashing_transaction_effect: SlashingTransactionEffect,
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
        demoted_hashes: Vec<DagHash>,
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
        demoted_hashes: Vec<DagHash>,
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

    /// Bootstrap identity or live concrete-EVM slashing account fact in stable wallet order.
    struct SlashingSubmitterIdentity {
        wallet_index: usize,
        address: [u8; 20],
        nonce: [u8; 32],
        balance: [u8; 32],
    }

    /// Typed Rust-owned slashing transaction effect.
    /// Status zero is the only executable effect. Non-zero statuses describe
    /// why a published double-vote conflict produced no transaction. Raw vote
    /// evidence never crosses this boundary.
    struct SlashingTransactionEffect {
        status: u8,
        proof_hash: [u8; 32],
        wallet_index: usize,
        nonce: [u8; 32],
        contract_address: [u8; 20],
        value: [u8; 32],
        gas_limit: u64,
        call_data: Vec<u8>,
    }

    struct HashPeriod {
        hash: [u8; 32],
        period: u64,
    }

    /// Immutable application configuration for coherent PBFT service restore.
    ///
    /// The restored chain head supplies the current period and determines
    /// whether Cacti is active; callers cannot inject either derived fact.
    /// Slashing/hardfork facts and PBFT-sync service limits are copied into the
    /// service-owned planners and cannot change during the service lifetime.
    /// `sync_level_size` must be nonzero.
    struct PbftServiceConfig {
        genesis_lambda_ms: u64,
        cacti_lambda_max_ms: u64,
        cacti_lambda_default_ms: u64,
        cacti_block: u64,
        max_exponential_lambda_ms: u64,
        max_steps: u64,
        deadline_ms: u64,
        polling_interval_ms: u64,
        report_malicious_behaviour: bool,
        magnolia_activation_period: u64,
        ficus_activation_period: u64,
        pillar_blocks_interval: u64,
        sync_level_size: u64,
        is_light_node: bool,
        light_node_history: u64,
        committee_size: u64,
        number_of_proposers: u64,
        dag_blocks_size: u64,
        ghost_path_move_back: u64,
        node_version_major: u16,
        node_version_minor: u16,
        node_version_patch: u16,
        node_version_network: u16,
        node_version_suffix: Vec<u8>,
        default_pbft_gas_limit: u64,
        cornus_activation_period: u64,
        cornus_pbft_gas_limit: u64,
        lambda_min_ms: u64,
        lambda_change_interval: u64,
        lambda_change_ms: u64,
        consensus_delay_ms: u64,
        dpos_blocks_per_year: u64,
        recently_finalized_factor: u64,
        chain_id: u64,
    }

    /// One terminal or slashing-resumable native PBFT-sync ingress step.
    struct PbftSyncIngressStep {
        action: u8,
        error_code: String,
        source_payload_id: u64,
        block_hash: [u8; 32],
        period: u64,
        max_dag_level: u64,
        last_block: bool,
        current_cert_present: bool,
        has_slashing_transaction_effect: bool,
        slashing_transaction_effect: SlashingTransactionEffect,
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

    /// Public/query PBFT `2t+1` threshold lookup result.
    struct PbftTwoTPlusOneThresholdPlan {
        status: u8,
        error_code: String,
        has_threshold: bool,
        threshold: u64,
    }

    /// Canonical pillar-vote bytes shared by batch inspection and period-data pop boundaries.
    struct PillarVoteRlpPayload {
        vote_rlp: Vec<u8>,
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

    /// CXX-visible result of composed pillar-vote validation.
    ///
    /// Status values match `PillarVoteValidationPlanStatus` in the C++ shim:
    /// `0` is valid and non-zero values identify deterministic rejection. Rust
    /// owns generation-bound preparation and deterministic validation; C++
    /// supplies exact external-EVM DPoS facts and receives only compatibility
    /// result fields.
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

    /// Non-mutating validation result for one exact retained preparation.
    struct PillarVoteSingleAdmissionValidationPlan {
        status: u8,
        period: u64,
        vote_hash: [u8; 32],
        voter: [u8; 20],
    }

    /// External DPoS facts used to consume one retained pillar-vote preparation.
    struct PillarVoteSingleAdmissionApplyInput {
        vote_hash: [u8; 32],
        validator_vote_count: u64,
        has_total_eligible_vote_count: bool,
        total_eligible_vote_count: u64,
    }

    /// Mutation result for one exact retained pillar-vote preparation.
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

    /// Compatibility lookup for the externally visible pillar threshold API.
    struct PillarConsensusThresholdLookup {
        available: bool,
        threshold: u64,
        error_code: String,
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

    /// One signed validator vote-count change shared by pillar planning and query views.
    struct PillarValidatorVoteCountChange {
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
        epoch: [u8; 32],
        validator_vote_count_changes: Vec<PillarValidatorVoteCountChange>,
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
    /// Status values match native pillar-block linkage planning.
    struct PillarBlockCreationWithVoteCountsPlan {
        status: u8,
        valid: bool,
        expected_previous_period: u64,
        previous_pillar_block_hash: [u8; 32],
        state_root: [u8; 32],
        bridge_root: [u8; 32],
        bridge_epoch: [u8; 32],
        vote_count_changes: Vec<PillarValidatorVoteCountChange>,
        current_vote_counts: Vec<PillarValidatorVoteCount>,
        anchor_generation: u64,
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

    struct FinalChainBlockNumberLookup {
        found: bool,
        value: u64,
    }

    struct FinalChainExecutionStatus {
        executed_dag_block_count: u64,
        executed_transaction_count: u64,
    }

    /// Genesis account carried across the CXX bootstrap boundary.
    ///
    /// `address` identifies the account and `balance` is its validated U256 byte representation.
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

    struct RedelegationCorrection {
        validator: [u8; 20],
        delegator: [u8; 20],
        amount: Vec<u8>,
    }

    struct FinalChainRewardsConfig {
        committee_size: u32,
        magnolia_period: u64,
        phalaenopsis_period: u64,
        aspen_part_one_period: u64,
        fix_claim_all_block_num: u64,
        fix_redelegate_block_num: u64,
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
        redelegations: Vec<RedelegationCorrection>,
    }

    struct AccountLookup {
        found: bool,
        /// Canonical minimal big-endian account nonce (empty means zero).
        nonce: Vec<u8>,
        balance: Vec<u8>,
        storage_root_hash: [u8; 32],
        code_hash: [u8; 32],
        code_size: u64,
    }

    struct DposValidatorStake {
        address: [u8; 20],
        stake: Vec<u8>,
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
        logs: Vec<FinalChainEvmLog>,
        gas_used: u64,
        code_err: String,
        consensus_err: String,
    }

    struct FinalizationTransaction {
        hash: [u8; 32],
        sender: [u8; 20],
        receiver_found: bool,
        receiver: [u8; 20],
        /// Canonical minimal big-endian transaction nonce (empty means zero).
        nonce: Vec<u8>,
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
        /// Canonical minimal big-endian transaction nonce (empty means zero).
        nonce: Vec<u8>,
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
        /// Canonical minimal big-endian system-account nonce (empty means zero).
        system_account_nonce: Vec<u8>,
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
        distribution_stats: Vec<PeriodRlp>,
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
    /// Native `TransactionService` computes sidecar membership instead of
    /// accepting C++ membership booleans.
    struct DagTransactionSaveSidecarFact {
        input_index: u64,
        hash: [u8; 32],
        trx_rlp: Vec<u8>,
        transaction_nonce: [u8; 32],
        sender_account_nonce: [u8; 32],
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

    /// Typed command report for DAG-block transaction persistence.
    ///
    /// Rust has already persisted storage, updated sidecars, erased queued
    /// transactions, and updated the authoritative runtime count. C++ consumes
    /// this report only for logging.
    struct TransactionManagerDagSaveCommandReport {
        queue_erased: Vec<DagHash>,
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
        /// Canonical Keccak-256 hash of the complete signed block RLP.
        block_hash: [u8; 32],
        block_level: u64,
        pivot: [u8; 32],
        tips: Vec<DagHash>,
        block_transaction_hashes: Vec<DagTransactionHash>,
        supplied_transaction_hashes: Vec<DagTransactionHash>,
        /// Canonical signed block bytes retained for authorization-stage sender recovery.
        block_rlp: Vec<u8>,
    }

    /// One requested Rust-owned `verifyBlock` session step.
    struct DagVerifyBlockSessionStep {
        /// Identity of the active cursor; zero only when no cursor exists.
        cursor_id: u64,
        status: u8,
        action: u8,
        complete: bool,
        reject_code: u32,
        proposal_period: u64,
        vote_count: u64,
        max_vote_count: u64,
        error_code: String,
    }

    /// Non-advancing Rust transaction preparation for one `verifyBlock` session.
    ///
    /// `transactions` preserves the private query's canonical order. C++ must
    /// materialize and validate every found payload before completing this exact
    /// cursor with proposal-period account facts.
    struct DagVerifyBlockTransactionPreparation {
        /// Identity of the active Rust verification cursor.
        cursor_id: u64,
        /// Proposal period used for FinalChain account lookup and completion.
        proposal_period: u64,
        /// Ordered TransactionManager views for C++ transaction materialization.
        transactions: Vec<TransactionManagerTransactionView>,
    }

    /// Cursor-bound completion facts for prepared verify-block transactions.
    struct DagVerifyBlockTransactionCompletionReport {
        /// Cursor returned by the matching preparation.
        cursor_id: u64,
        /// Proposal period returned by the matching preparation.
        proposal_period: u64,
        /// Per-sender account facts read at `proposal_period` after materialization.
        account_nonce_facts: Vec<TransactionQueueAccountNonceFact>,
    }

    /// Proof-bearing facts needed to verify the active Rust-owned DAG cursor.
    /// Session identity, proposal period, normalized vote counts, and
    /// historical sortition parameters remain private Rust state.
    struct DagVerifyBlockVdfRequest {
        /// Cursor identity returned by the VDF action step.
        cursor_id: u64,
        block_rlp: Vec<u8>,
        block_level: u64,
        proposal_period_hash: [u8; 32],
    }

    /// External gas facts for one `verifyBlock` session.
    ///
    /// Rust retains canonical tips in the active cursor and derives any
    /// required per-tip gas metadata from private DAG storage.
    struct DagVerifyBlockGasReport {
        block_gas_estimation: u64,
        estimated_transactions_weight: u64,
        dag_gas_limit: u64,
        pbft_gas_limit: u64,
    }

    /// External/configured facts used to open one runtime-owned proposal session.
    ///
    /// The caller supplies trusted wallet identity (VRF keys and proposer address), packing limits, and
    /// block-construction gas/tip limits. The composed Rust service derives transaction queue and non-finalized sidecar
    /// counts from its sibling TransactionManager, while the DAG runtime derives frontier, proposal level/period/hash,
    /// and observation fingerprint. Invalid storage state is returned as an error; a missing proposal-period mapping
    /// produces a terminal session step instead. Identity, limits, and derived observations are retained by the cursor.
    struct DagProposerSessionBeginInput {
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

    /// Complete instruction/result snapshot returned by the Rust-owned DAG proposer cursor.
    ///
    /// `status` distinguishes active, complete, and invalid-report outcomes; `action` selects the one external boundary
    /// to execute. Retry fields are authoritative only when `update_retry_state` is true. Vectors may be empty when the
    /// selected action does not consume them. Action 1 exposes only EVM candidates in `transaction_estimate_requests`;
    /// canonical selected RLP payloads remain private until action 6 exposes `selected_transactions` beside the signed
    /// block. Action 5 exposes only the Rust-owned intent's `signing_hash`. Terminal steps remove the cursor after
    /// construction.
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
        /// Exact historical parameters for StartVdf; zeroed for every other action.
        vdf_sortition_params: LegacySortitionParams,
        vdf_stale: bool,
        old_proposal: bool,
        vdf_message: Vec<u8>,
        selected_transaction_hashes: Vec<DagHash>,
        transaction_estimate_requests: Vec<TransactionPackSessionCandidate>,
        selected_transactions: Vec<TransactionPackSelectedTransaction>,
        signing_hash: [u8; 32],
        signed_block: DagProposerSignedBlockIntent,
        record_proposed_block: bool,
        vdf_poll_interval_ms: u64,
        stale_proof_sleep_ms: u64,
        error_code: String,
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

    /// One canonical transaction payload supplied with an accepted DAG block.
    struct DagAddBlockTransactionPayload {
        hash: [u8; 32],
        trx_rlp: Vec<u8>,
    }

    /// Canonical add-block input inspected by the composed DAG/transaction service.
    struct DagAddBlockPrepareInput {
        expected_block_hash: [u8; 32],
        /// Whether the canonical RLP hash must match `expected_block_hash`.
        /// Object-backed compatibility calls disable this and retain the object's
        /// externally supplied identity while still decoding all other RLP facts.
        validate_block_hash: bool,
        block_rlp: Vec<u8>,
        save: bool,
        proposed: bool,
        transactions: Vec<DagAddBlockTransactionPayload>,
    }

    /// Latest-account request for one inspected block transaction.
    struct DagAddBlockAccountRequest {
        input_index: u64,
        sender: [u8; 20],
    }

    /// Non-mutating add-block preparation or terminal admission result.
    struct DagAddBlockPreparation {
        cursor_id: u64,
        block_level: u64,
        accepted: bool,
        duplicate: bool,
        expired: bool,
        missing_references: Vec<DagHash>,
        account_requests: Vec<DagAddBlockAccountRequest>,
    }

    /// Indexed latest account nonce returned after preparation.
    struct DagAddBlockAccountNonceFact {
        input_index: u64,
        account_nonce: [u8; 32],
    }

    /// Cursor-bound completion facts for one prepared add-block transition.
    struct DagAddBlockCompletionInput {
        cursor_id: u64,
        account_nonce_facts: Vec<DagAddBlockAccountNonceFact>,
    }

    /// Durable add-block result and retained C++ shell effects.
    struct DagAddBlockCommitReport {
        accepted: bool,
        emit_verified: bool,
        gossip: bool,
        proposed: bool,
        queue_erased: Vec<DagHash>,
        counters: DagPersistenceCounters,
    }

    struct DagManagerAnchors {
        old_anchor: [u8; 32],
        anchor: [u8; 32],
    }

    /// Rust-applied finalized DAG order result for C++ live side effects.
    struct DagManagerFinalizationApplyPayload {
        finalized_count: u64,
        expired_hashes: Vec<DagHash>,
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

    /// Public identity of one configured signing wallet.
    ///
    /// The stable index selects host-held key material for later effects;
    /// only public address and verification keys enter native configuration.
    struct SigningIdentity {
        wallet_index: u64,
        address: [u8; 20],
        node_public_key: [u8; 64],
        vrf_public_key: [u8; 32],
    }

    /// Hot application-root PBFT status used by App scheduling and networking.
    struct HostConsensusLiveStatus {
        period: u64,
        round: u64,
        step: u64,
        finalized_chain_size: u64,
        syncing_period: u64,
        sync_queue_size: u64,
        has_current_node_votes: bool,
        current_node_votes: u64,
        has_total_eligible_votes: bool,
        total_eligible_votes: u64,
    }

    /// One validator's eligible DPoS vote count returned by FinalChain queries.
    struct HostValidatorVoteCount {
        address: [u8; 20],
        vote_count: u64,
    }

    extern "Rust" {
        type BridgeConsensusQueryApi;

        pub fn create_consensus_query_api(
            runtime: &BridgeConsensusApplication,
        ) -> Box<BridgeConsensusQueryApi>;
        pub fn consensus_query_pbft_block_hash_by_period(
            self: &BridgeConsensusQueryApi,
            period: u64,
        ) -> Result<HashLookup>;
        pub fn consensus_query_pbft_sync_block_exists(
            self: &BridgeConsensusQueryApi,
            block_hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn consensus_query_verified_vote_count(self: &BridgeConsensusQueryApi) -> Result<u64>;
        pub fn consensus_query_pbft_vote_threshold(
            self: &BridgeConsensusQueryApi,
            period: u64,
            vote_type: u8,
        ) -> Result<PbftTwoTPlusOneThresholdPlan>;
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
            service: &BridgeConsensusApplication,
        ) -> Box<BridgeConsensusNetworkApi>;
        pub fn consensus_network_drain_work(
            self: &BridgeConsensusNetworkApi,
            transport_lane: u32,
            source_payload_id: u64,
            source_scoped: bool,
            budget: u32,
        ) -> Result<NetworkEffectBatch>;
        pub fn consensus_network_report_effect_results(
            self: &BridgeConsensusNetworkApi,
            results: Vec<NetworkEffectResult>,
        ) -> Result<NetworkEffectAck>;
        pub fn consensus_network_admit_pbft_vote(
            self: &BridgeConsensusNetworkApi,
            fact: PbftVoteIngressFact,
            context: NetworkPbftVoteIngressContext,
            slashing_submitters: Vec<SlashingSubmitterIdentity>,
        ) -> Result<NetworkPbftVoteAdmissionOutcome>;
        pub fn consensus_network_admit_pbft_vote_bundle(
            self: &BridgeConsensusNetworkApi,
            reference: PbftVoteIngressFact,
            votes: Vec<PbftVoteIngressFact>,
            contexts: Vec<NetworkPbftVoteIngressContext>,
            slashing_submitters: Vec<SlashingSubmitterIdentity>,
        ) -> Result<Vec<NetworkPbftVoteAdmissionOutcome>>;
        pub fn consensus_network_ingest_pillar_vote_bundle(
            self: &BridgeConsensusNetworkApi,
            context: NetworkPillarVoteIngressContext,
            votes: Vec<PillarVoteRlpPayload>,
        ) -> Result<Vec<NetworkIngressDecision>>;
        pub fn consensus_network_ingest_pbft_next_votes_bundle_request(
            self: &BridgeConsensusNetworkApi,
            transport_lane: u32,
            peer_id: [u8; 64],
            peer_period: u64,
            peer_round: u64,
            source_payload_id: u64,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_ingest_pillar_votes_bundle_request(
            self: &BridgeConsensusNetworkApi,
            transport_lane: u32,
            peer_id: [u8; 64],
            period: u64,
            pillar_block_hash: [u8; 32],
            source_payload_id: u64,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_ingest_get_pbft_sync_request(
            self: &BridgeConsensusNetworkApi,
            request: NetworkGetPbftSyncRequest,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_ingest_pbft_blocks_bundle(
            self: &BridgeConsensusNetworkApi,
            runtime: &BridgeConsensusApplication,
            packet_rlp: Vec<u8>,
            source_payload_id: u64,
        ) -> Result<NetworkIngressDecision>;
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

        type BridgeConsensusApplication;

        pub fn create_consensus_application(
            storage_path: &str,
            schema_major: u32,
            schema_minor: u32,
            storage_genesis: &[u8; 32],
            dag_genesis: &[u8; 32],
            dag_expiry_limit: u32,
            max_levels_per_period: u64,
            sortition_config: SortitionRuntimeConfig,
            transaction_queue_config: TransactionQueueConfig,
            gas_pricer_config: GasPricerConfig,
            proposal_dag_gas_limit: u64,
            pbft_config: PbftServiceConfig,
            signing_identities: Vec<SigningIdentity>,
            final_chain_block_gas_limit: u64,
            final_chain_genesis_timestamp: u64,
            final_chain_genesis_accounts: Vec<GenesisAccount>,
            final_chain_genesis_validators: Vec<GenesisValidator>,
            final_chain_genesis_dpos_config: GenesisDposConfig,
            final_chain_rewards_config: FinalChainRewardsConfig,
        ) -> Result<Box<BridgeConsensusApplication>>;
        /// Returns one lock-coherent hot status snapshot without exposing the
        /// native PBFT manager or its internal executor cursor.
        pub fn consensus_application_live_status(
            application: &BridgeConsensusApplication,
        ) -> Result<HostConsensusLiveStatus>;
        /// Prunes native FinalChain lookup indexes below the retained block.
        pub fn prune_final_chain_before(
            self: &BridgeConsensusApplication,
            first_to_keep: u64,
        ) -> Result<u64>;
        /// Prepares one canonical add-block transition without mutation.
        pub fn dag_transaction_service_prepare_add_block(
            self: &BridgeConsensusApplication,
            input: DagAddBlockPrepareInput,
        ) -> Result<DagAddBlockPreparation>;
        /// Atomically persists and publishes one prepared add-block transition.
        pub fn dag_transaction_service_complete_add_block(
            self: &BridgeConsensusApplication,
            input: DagAddBlockCompletionInput,
        ) -> Result<DagAddBlockCommitReport>;
        /// Idempotently aborts only the matching prepared add-block cursor.
        pub fn dag_transaction_service_abort_add_block(
            self: &BridgeConsensusApplication,
            cursor_id: u64,
        ) -> Result<bool>;
        /// Validates candidate pivot/tip references from Rust runtime state and
        /// storage without C++ `DagBlock` materialization.
        pub fn dag_manager_runtime_validate_pivot_tips(
            self: &BridgeConsensusApplication,
            block_level: u64,
            pivot: &[u8; 32],
            tips: Vec<DagHash>,
        ) -> Result<DagPivotTipsValidation>;
        /// Applies finalized DAG order using Rust state and Rust storage.
        pub fn dag_manager_runtime_apply_finalized_order(
            self: &BridgeConsensusApplication,
            new_anchor: [u8; 32],
            new_period: u64,
            finalized_order: Vec<DagHash>,
        ) -> Result<DagManagerFinalizationApplyPayload>;
        /// Returns non-finalized sync DAG block RLPs and referenced transaction
        /// RLPs through Rust-owned storage access.
        pub fn dag_manager_runtime_non_finalized_sync_payload(
            self: &BridgeConsensusApplication,
            known_hashes: Vec<DagHash>,
        ) -> Result<DagManagerNonFinalizedSyncPayload>;
        pub fn dag_manager_runtime_compute_order(
            self: &BridgeConsensusApplication,
            anchor: &[u8; 32],
        ) -> Result<DagOrder>;
        pub fn dag_manager_runtime_frontier(
            self: &BridgeConsensusApplication,
        ) -> Result<DagFrontier>;
        pub fn dag_manager_runtime_ghost_path(
            self: &BridgeConsensusApplication,
            source: &[u8; 32],
        ) -> Result<Vec<DagHash>>;
        pub fn dag_manager_runtime_anchor_ghost_path(
            self: &BridgeConsensusApplication,
        ) -> Result<Vec<DagHash>>;
        pub fn dag_manager_runtime_graphviz_dot(
            self: &BridgeConsensusApplication,
            pivot_tree: bool,
        ) -> Result<String>;
        pub fn dag_manager_runtime_vertex_count(self: &BridgeConsensusApplication)
            -> Result<usize>;
        pub fn dag_manager_runtime_edge_count(self: &BridgeConsensusApplication) -> Result<usize>;
        pub fn dag_manager_runtime_max_level(self: &BridgeConsensusApplication) -> Result<u64>;
        pub fn dag_manager_runtime_latest_period(self: &BridgeConsensusApplication) -> Result<u64>;
        pub fn dag_manager_runtime_anchors(
            self: &BridgeConsensusApplication,
        ) -> Result<DagManagerAnchors>;
        pub fn dag_manager_runtime_dag_expiry_level(
            self: &BridgeConsensusApplication,
        ) -> Result<u64>;
        pub fn dag_manager_runtime_non_finalized_blocks(
            self: &BridgeConsensusApplication,
        ) -> Result<Vec<DagLevelHashes>>;
        pub fn dag_manager_runtime_non_finalized_blocks_size(
            self: &BridgeConsensusApplication,
        ) -> Result<DagManagerNonFinalizedSize>;
        /// Returns DAG block membership from Rust graph state plus canonical
        /// Rust storage without consulting C++ compatibility caches.
        pub fn dag_manager_runtime_is_block_known(
            self: &BridgeConsensusApplication,
            hash: &[u8; 32],
        ) -> Result<bool>;
        /// Loads one canonical DAG block payload from Rust storage.
        pub fn dag_manager_runtime_load_block(
            self: &BridgeConsensusApplication,
            hash: &[u8; 32],
        ) -> Result<BlockRlpLookup>;
        pub fn dag_manager_runtime_plan_proposal_tip_selection(
            self: &BridgeConsensusApplication,
            input: DagProposerStorageTipSelectionInput,
        ) -> Result<DagProposerTipSelectionPlan>;
        /// Opens a runtime-owned proposer cursor from wallet/configuration input.
        /// Returns a unique cursor id or a storage/decode error. Rust derives DAG observations plus queue and sidecar
        /// pressure; callers must eventually consume a terminal step or call the idempotent abort function.
        #[rust_name = "service_dag_manager_runtime_begin_proposer_session"]
        pub fn dag_manager_runtime_begin_proposer_session(
            runtime: &BridgeConsensusApplication,
            input: DagProposerSessionBeginInput,
        ) -> Result<u64>;
        /// Idempotently removes a live proposer cursor without planner or retry effects.
        /// Returns true only when this call removed the cursor; missing or already-removed ids return false.
        #[rust_name = "service_dag_manager_runtime_abort_proposer_session"]
        pub fn dag_manager_runtime_abort_proposer_session(
            runtime: &BridgeConsensusApplication,
            session_id: u64,
        ) -> Result<bool>;
        pub fn dag_manager_runtime_period_block_hash(
            self: &BridgeConsensusApplication,
            period: u64,
        ) -> Result<HashLookup>;
        pub fn dag_manager_runtime_persistence_counters(
            self: &BridgeConsensusApplication,
        ) -> Result<DagPersistenceCounters>;
        #[rust_name = "service_dag_manager_runtime_begin_verify_block_session"]
        pub fn dag_manager_runtime_begin_verify_block_session(
            runtime: &BridgeConsensusApplication,
            input: DagVerifyBlockSessionInput,
        ) -> Result<()>;
        #[rust_name = "service_dag_manager_runtime_verify_block_session_next"]
        pub fn dag_manager_runtime_verify_block_session_next(
            runtime: &BridgeConsensusApplication,
        ) -> Result<DagVerifyBlockSessionStep>;
        /// Resolves the active private transaction query without advancing it.
        ///
        /// Rust reads query hashes and proposal period, locks DAG then
        /// TransactionManager, preserves duplicate/caller-supplied semantics, and
        /// returns ordered payload views plus cursor identity for completion.
        #[rust_name = "service_dag_manager_runtime_verify_block_session_prepare_transactions"]
        pub fn dag_manager_runtime_verify_block_session_prepare_transactions(
            runtime: &BridgeConsensusApplication,
        ) -> Result<DagVerifyBlockTransactionPreparation>;
        /// Applies proposal-period account facts after successful C++ materialization.
        /// Rejects stale cursor/period identities without advancing the active session.
        #[rust_name = "service_dag_manager_runtime_verify_block_session_complete_transactions"]
        pub fn dag_manager_runtime_verify_block_session_complete_transactions(
            runtime: &BridgeConsensusApplication,
            report: DagVerifyBlockTransactionCompletionReport,
        ) -> Result<DagVerifyBlockSessionStep>;
        /// Collects DPoS/VRF facts from the borrowed Rust FinalChain for the
        /// active authorization cursor. Missing or wrong-stage cursors return
        /// the stable invalid-step carrier. The DAG lock is released during
        /// sender recovery and FinalChain lookup; Rust then revalidates the
        /// exact cursor before applying facts. Decode, recovery, storage, or
        /// FinalChain failures remove only the unchanged owning cursor and
        /// propagate as bridge errors.
        #[rust_name = "service_dag_manager_runtime_verify_block_session_report_authorization"]
        pub fn dag_manager_runtime_verify_block_session_report_authorization(
            runtime: &BridgeConsensusApplication,
        ) -> Result<DagVerifyBlockSessionStep>;
        /// Verifies the active VDF action through isolated DAG and sortition
        /// lock intervals, then advances only the unchanged cursor.
        #[rust_name = "service_dag_transaction_service_verify_block_session_vdf"]
        pub fn dag_transaction_service_verify_block_session_vdf(
            runtime: &BridgeConsensusApplication,
            request: DagVerifyBlockVdfRequest,
        ) -> Result<DagVerifyBlockSessionStep>;
        #[rust_name = "service_dag_manager_runtime_verify_block_session_report_gas"]
        pub fn dag_manager_runtime_verify_block_session_report_gas(
            runtime: &BridgeConsensusApplication,
            report: DagVerifyBlockGasReport,
        ) -> Result<DagVerifyBlockSessionStep>;
        /// Reads the cursor's current executor instruction; terminal reads remove it.
        /// Missing ids return an invalid-report step and do not mutate retry state.
        #[rust_name = "service_dag_manager_runtime_proposer_session_next"]
        pub fn dag_manager_runtime_proposer_session_next(
            runtime: &BridgeConsensusApplication,
            session_id: u64,
        ) -> Result<DagProposerSessionStep>;
        /// Supplies requested FinalChain facts; Rust loads and revalidates exact
        /// historical sortition parameters inside the composed service.
        /// Any returned error removes the cursor, so callers may also invoke abort safely during generic cleanup.
        #[rust_name = "service_dag_manager_runtime_proposer_session_report_final_chain_facts"]
        pub fn dag_manager_runtime_proposer_session_report_final_chain_facts(
            runtime: &BridgeConsensusApplication,
            session_id: u64,
        ) -> Result<DagProposerSessionStep>;
        /// Prepares a DAG-owned transaction pack from private cursor configuration.
        /// Estimate-needed results keep action 1 and expose only `transaction_estimate_requests`; declared/cache-only,
        /// empty, and throttled results advance immediately. No Rust lock crosses the external EVM interval.
        pub fn dag_transaction_service_proposer_pack_prepare(
            self: &BridgeConsensusApplication,
            session_id: u64,
            network_throttled: bool,
            min_transaction_gas: u64,
            estimate_gas_limit: u64,
            last_block_number: u64,
        ) -> Result<DagProposerSessionStep>;
        /// Finalizes the matching owner-bound transaction cursor and transfers canonical selected payloads directly into
        /// the DAG cursor. Wrong-owner or malformed estimates abort both matching cursors before returning an error.
        pub fn dag_transaction_service_proposer_pack_finalize(
            self: &BridgeConsensusApplication,
            session_id: u64,
            estimates: Vec<TransactionPackSessionEstimateInput>,
        ) -> Result<DagProposerSessionStep>;
        /// Idempotently aborts matching proposer/transaction cursors. Transaction-only services fail before transaction
        /// mutation; a wrong-owner transaction cursor is never removed.
        pub fn dag_transaction_service_proposer_pack_abort(
            self: &BridgeConsensusApplication,
            session_id: u64,
        ) -> Result<bool>;
        /// Polls VDF cancellation using the current Rust-derived proposal frontier level.
        /// Missing/out-of-order ids return an invalid-report step; cancellation returns a terminal cancel action.
        #[rust_name = "service_dag_manager_runtime_proposer_session_poll_vdf"]
        pub fn dag_manager_runtime_proposer_session_poll_vdf(
            runtime: &BridgeConsensusApplication,
            session_id: u64,
        ) -> Result<DagProposerSessionStep>;
        /// Supplies proof success and canonical VDF RLP, revalidates the observation, and constructs the unsigned intent.
        /// Success returns signing action 5; stale observations terminate without retry mutation, and construction
        /// errors remove the cursor before throwing across CXX.
        #[rust_name = "service_dag_manager_runtime_proposer_session_report_vdf_proof"]
        pub fn dag_manager_runtime_proposer_session_report_vdf_proof(
            runtime: &BridgeConsensusApplication,
            session_id: u64,
            report: DagProposerVdfProofReport,
        ) -> Result<DagProposerSessionStep>;
        /// Rechecks a stale proof after compatibility sleep and revalidates the complete Rust observation.
        /// An unchanged observation constructs the stored proof's unsigned intent and returns signing action 5; stale
        /// observations terminate without retry mutation, and construction errors remove the cursor.
        #[rust_name = "service_dag_manager_runtime_proposer_session_resume_stale_proof"]
        pub fn dag_manager_runtime_proposer_session_resume_stale_proof(
            runtime: &BridgeConsensusApplication,
            session_id: u64,
        ) -> Result<DagProposerSessionStep>;
        /// Finalizes the stored unsigned intent with a 65-byte recoverable signature.
        /// Success returns add-block action 6 with canonical RLP/hash; malformed signatures and finalization errors remove
        /// the cursor, while missing/out-of-order ids return an invalid terminal step.
        #[rust_name = "service_dag_manager_runtime_proposer_session_report_signing"]
        pub fn dag_manager_runtime_proposer_session_report_signing(
            runtime: &BridgeConsensusApplication,
            session_id: u64,
            report: DagProposerSigningReport,
        ) -> Result<DagProposerSessionStep>;
        #[rust_name = "service_dag_manager_runtime_proposer_session_report_add_block"]
        pub fn dag_manager_runtime_proposer_session_report_add_block(
            runtime: &BridgeConsensusApplication,
            session_id: u64,
            report: DagProposerAddBlockReport,
        ) -> Result<DagProposerSessionStep>;
        pub fn dag_plan_proposer_worker_command(
            input: DagProposerWorkerCommandInput,
        ) -> DagProposerWorkerCommand;
        pub fn dag_vdf_message(pivot: &[u8; 32], transaction_hashes: Vec<DagHash>) -> Vec<u8>;

        // Network-owned PBFT ingress leaves retained for tarcap clients.
        pub fn pbft_service_begin_pbft_sync_ingress(
            service: &BridgeConsensusApplication,
            packet_rlp: &[u8],
            source_payload_id: u64,
            source_peer_id: [u8; 64],
            slashing_submitters: Vec<SlashingSubmitterIdentity>,
        ) -> Result<PbftSyncIngressStep>;
        pub fn pbft_service_report_pbft_sync_ingress_slashing(
            service: &BridgeConsensusApplication,
            proof_hash: [u8; 32],
            transaction_inserted: bool,
        ) -> Result<PbftSyncIngressStep>;

        // Network-owned proposed-block publication leaf.
        pub fn pbft_service_publish_proposed_block_effect(
            self: &BridgeConsensusApplication,
            canonical_signed_block_rlp: Vec<u8>,
        ) -> Result<bool>;
        // Consensus transaction manager planning

        pub fn transaction_manager_runtime_gas_price_bid(
            self: &BridgeConsensusApplication,
        ) -> [u8; 32];
        pub fn transaction_manager_runtime_gas_price_update(
            self: &BridgeConsensusApplication,
            gas_prices: Vec<GasPricerGasPrice>,
        );
        pub fn transaction_manager_runtime_pack_prepare_sharded(
            self: &BridgeConsensusApplication,
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
            self: &BridgeConsensusApplication,
            inputs: Vec<TransactionPackSessionEstimateInput>,
        ) -> Result<TransactionPackSessionStep>;
        pub fn transaction_manager_runtime_pack_abort(self: &BridgeConsensusApplication) -> bool;
        pub fn transaction_manager_runtime_plan_gas_estimation(
            self: &BridgeConsensusApplication,
            fact: TransactionManagerGasEstimationFact,
        ) -> Result<TransactionManagerGasEstimationPlan>;
        pub fn transaction_manager_runtime_store_gas_estimation(
            self: &BridgeConsensusApplication,
            result: TransactionManagerGasEstimationResult,
        ) -> Result<bool>;
        pub fn transaction_manager_runtime_transaction_count(
            self: &BridgeConsensusApplication,
        ) -> u64;
        /// Returns Rust's known-transaction decision from runtime-owned queue and sidecar state.
        pub fn transaction_manager_runtime_is_transaction_known_hash(
            self: &BridgeConsensusApplication,
            hash: &[u8; 32],
        ) -> Result<bool>;
        /// Inserts payloads and moves them into recently-finalized sidecar state in one Rust command.
        pub fn transaction_manager_runtime_initialize_recently_finalized_payloads(
            self: &BridgeConsensusApplication,
            period: u64,
            payloads: Vec<TransactionManagerSidecarInsertInput>,
        ) -> Result<()>;
        pub fn transaction_manager_runtime_non_finalized_size(
            self: &BridgeConsensusApplication,
        ) -> usize;
        pub fn transaction_manager_runtime_remove_non_finalized(
            self: &BridgeConsensusApplication,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<u64>;
        /// Executes admission with FinalChain facts supplied by the C++ external-EVM boundary.
        pub fn transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts_command_report(
            self: &BridgeConsensusApplication,
            fact: TransactionManagerValidatedInsertRuntimeFact,
            final_chain_fact: TransactionManagerFinalChainAdmissionFact,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionManagerAdmissionCommandReport>;
        /// Executes public insertTransaction verification and fact-backed admission as one Rust-owned command.
        pub fn transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_facts_command_report(
            self: &BridgeConsensusApplication,
            verify_fact: TransactionManagerVerifyTransactionFact,
            admission_fact: TransactionManagerValidatedInsertRuntimeFact,
            final_chain_fact: TransactionManagerFinalChainAdmissionFact,
            input: TransactionQueueInsertInput,
        ) -> Result<TransactionManagerPublicAdmissionCommandReport>;
        /// Resolves requested hashes against Rust-owned live queue payloads only.
        pub fn transaction_manager_runtime_queue_lookup_transaction_views(
            self: &BridgeConsensusApplication,
            requests: Vec<TransactionManagerTransactionViewRequest>,
        ) -> Result<Vec<TransactionManagerTransactionView>>;
        pub fn transaction_manager_runtime_queue_all_transaction_groups(
            self: &BridgeConsensusApplication,
        ) -> Vec<TransactionQueueTransactionGroup>;
        pub fn transaction_manager_runtime_queue_size(self: &BridgeConsensusApplication) -> usize;
        pub fn transaction_manager_runtime_queue_proposable_accounts(
            self: &BridgeConsensusApplication,
        ) -> Vec<TransactionQueueProposableAccountFact>;
        pub fn transaction_manager_runtime_queue_block_finalized(
            self: &BridgeConsensusApplication,
            block_number: u64,
        ) -> Vec<DagHash>;
        pub fn transaction_manager_runtime_queue_transactions_dropped(
            self: &BridgeConsensusApplication,
        ) -> bool;
        pub fn transaction_manager_runtime_queue_non_proposable_over_limit(
            self: &BridgeConsensusApplication,
        ) -> bool;
        pub fn transaction_manager_runtime_queue_min_gas_price_for_block_inclusion(
            self: &BridgeConsensusApplication,
            limit: u64,
        ) -> [u8; 32];
        /// Resolves requested hashes against non-finalized/recently-finalized sidecars.
        pub fn transaction_manager_runtime_lookup_non_finalized_transaction_views(
            self: &BridgeConsensusApplication,
            requests: Vec<TransactionManagerTransactionViewRequest>,
        ) -> Result<Vec<TransactionManagerTransactionView>>;
        /// Resolves requested hashes through queue, sidecars, then Rust storage.
        pub fn transaction_manager_runtime_lookup_transaction_views(
            self: &BridgeConsensusApplication,
            requests: Vec<TransactionManagerTransactionViewRequest>,
            max_count: u64,
        ) -> Result<TransactionManagerTransactionViewPlan>;
        /// Resolves requested hashes through queue, sidecars, then proposal-filtered Rust storage.
        pub fn transaction_manager_runtime_lookup_proposal_transaction_views_with_account_nonce_facts(
            self: &BridgeConsensusApplication,
            proposal_period: u64,
            requests: Vec<TransactionManagerTransactionViewRequest>,
            account_nonce_facts: Vec<TransactionQueueAccountNonceFact>,
            max_count: u64,
        ) -> Result<TransactionManagerTransactionViewPlan>;
        /// Applies DAG transaction persistence and returns a typed command report.
        #[rust_name = "service_save_transactions_from_dag_block_command_report_with_runtime"]
        pub fn save_transactions_from_dag_block_command_report_with_runtime(
            runtime: &BridgeConsensusApplication,
            facts: Vec<DagTransactionSaveSidecarFact>,
        ) -> Result<TransactionManagerDagSaveCommandReport>;
        pub fn service_update_finalized_transactions_status_from_transaction_list(
            service: &BridgeConsensusApplication,
            period: u64,
            retention_window: u64,
            account_nonce_facts: Vec<TransactionQueueAccountNonceFact>,
            transaction_list_rlp: Vec<u8>,
        ) -> Result<()>;
        /// Builds deterministic TransactionManager::verifyTransaction admission plan.
        pub fn transaction_manager_verify_transaction(
            fact: TransactionManagerVerifyTransactionFact,
        ) -> Result<TransactionManagerVerifyTransactionOutcome>;
        #[rust_name = "service_transaction_manager_filter_non_finalized_with_runtime"]
        pub fn transaction_manager_filter_non_finalized_with_runtime(
            runtime: &BridgeConsensusApplication,
            requests: Vec<TransactionManagerSidecarLookupRequest>,
        ) -> Result<FinalizedTransactionFilterPlan>;
        #[rust_name = "service_transaction_manager_verify_not_finalized_with_runtime"]
        pub fn transaction_manager_verify_not_finalized_with_runtime(
            runtime: &BridgeConsensusApplication,
            facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
        ) -> Result<TransactionManagerVerifyNotFinalizedOutcome>;
        /// Rebuilds runtime recovery sidecars from Rust-backed storage.
        #[rust_name = "service_transaction_manager_recover_nonfinalized_with_runtime"]
        pub fn transaction_manager_recover_nonfinalized_with_runtime(
            runtime: &BridgeConsensusApplication,
        ) -> Result<()>;

        // Network-owned verified-vote slashing acknowledgement leaf.
        pub fn pbft_service_verified_votes_report_slashing_transaction_submission(
            self: &BridgeConsensusApplication,
            proof_hash: &[u8; 32],
            transaction_inserted: bool,
        ) -> Result<bool>;
        // Consensus pillar votes

        pub fn pbft_service_pillar_plan_block_creation_with_final_chain(
            self: &BridgeConsensusApplication,
            request: PillarBlockCreationRequest,
        ) -> Result<PillarBlockCreationWithVoteCountsPlan>;
        pub fn pbft_service_pillar_latest_finalized_block_rlp(
            self: &BridgeConsensusApplication,
        ) -> Result<Vec<u8>>;
        pub fn pbft_service_pillar_current_block_rlp(
            self: &BridgeConsensusApplication,
        ) -> Result<Vec<u8>>;

        pub fn pbft_service_pillar_ready(self: &BridgeConsensusApplication) -> bool;
        pub fn pbft_service_complete_pillar_bootstrap(
            self: &BridgeConsensusApplication,
        ) -> Result<()>;
        pub fn pillar_chain_storage_apply_current_block_data(
            self: &BridgeConsensusApplication,
            data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn pillar_chain_storage_apply_own_vote(
            self: &BridgeConsensusApplication,
            vote_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn pillar_chain_storage_apply_finalized_block(
            self: &BridgeConsensusApplication,
            period: u64,
            pillar_block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn pillar_chain_storage_load_own_vote(
            self: &BridgeConsensusApplication,
        ) -> Result<Vec<u8>>;
        pub fn pillar_chain_storage_load_current_block_data(
            self: &BridgeConsensusApplication,
        ) -> Result<Vec<u8>>;
        pub fn pillar_chain_storage_load_latest_block(
            self: &BridgeConsensusApplication,
        ) -> Result<Vec<u8>>;
        pub fn pillar_chain_storage_load_block(
            self: &BridgeConsensusApplication,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_genesis_hash(self: &BridgeConsensusApplication) -> Result<Vec<u8>>;
        pub fn get_last_sortition_params(
            self: &BridgeConsensusApplication,
            count: u64,
        ) -> Result<Vec<CanonicalBytes>>;
        pub fn get_params_change_for_period(
            self: &BridgeConsensusApplication,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_status_field(self: &BridgeConsensusApplication, field: u8) -> Result<u64>;
        pub fn get_period_lambda(
            self: &BridgeConsensusApplication,
            period: u64,
            find_closest: bool,
        ) -> Result<PeriodLambda>;
        pub fn get_rounds_count_dynamic_lambda(self: &BridgeConsensusApplication) -> Result<u32>;
        pub fn get_blocks_rewards_stats(
            self: &BridgeConsensusApplication,
        ) -> Result<Vec<PeriodRlp>>;
        pub fn pbft_service_pillar_apply_planned_current_block_data(
            self: &BridgeConsensusApplication,
            data_rlp: Vec<u8>,
            expected_anchor_generation: u64,
        ) -> Result<()>;
        pub fn pbft_service_pillar_apply_own_vote(
            self: &BridgeConsensusApplication,
            vote_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn pbft_service_pillar_load_startup_bootstrap(
            self: &BridgeConsensusApplication,
        ) -> Result<PillarChainStartupBootstrap>;
        pub fn pbft_service_pillar_consensus_threshold_with_final_chain(
            self: &BridgeConsensusApplication,
            period: u64,
        ) -> Result<PillarConsensusThresholdLookup>;
        pub fn pbft_service_pillar_prepare_single_vote_external_facts(
            self: &BridgeConsensusApplication,
            vote_rlp: Vec<u8>,
            context: PillarVoteSingleAdmissionContext,
            trusted_local_or_restore: bool,
        ) -> Result<PillarVoteSingleAdmissionPreparePlan>;
        pub fn pbft_service_pillar_validate_prepared_single_vote_external_facts(
            self: &BridgeConsensusApplication,
            prepared: PillarVoteSingleAdmissionPreparePlan,
            validator_vote_count: u64,
        ) -> Result<PillarVoteSingleAdmissionValidationPlan>;
        pub fn pbft_service_pillar_apply_prepared_single_vote_external_facts(
            self: &BridgeConsensusApplication,
            input: PillarVoteSingleAdmissionApplyInput,
        ) -> Result<PillarVoteSingleAdmissionApplyPlan>;
        pub fn pbft_service_pillar_plan_vote_relevance(
            self: &BridgeConsensusApplication,
            vote_rlp: Vec<u8>,
            context: PillarVoteSingleAdmissionContext,
        ) -> Result<PillarVoteRelevancePlan>;
        pub fn pbft_service_pillar_get_verified_vote_payloads(
            self: &BridgeConsensusApplication,
            period: u64,
            block_hash: &[u8; 32],
            above_threshold: bool,
        ) -> Result<PillarVotesPayloadLookup>;
        pub fn pbft_service_pillar_prepare_finalized_block_for_pbft(
            self: &BridgeConsensusApplication,
            request: PillarBlockFinalizationRequest,
        ) -> Result<PillarBlockFinalizationPrepareResult>;
        pub fn pbft_service_pillar_ack_finalize_block_for_pbft(
            self: &BridgeConsensusApplication,
            request: PillarBlockFinalizationAcknowledgeRequest,
        ) -> Result<PillarBlockFinalizationAcknowledgeResult>;

        // Storage

        type BridgeStorageQueries;
        type BridgeStorageBatch;

        pub fn create_pbft_storage_queries(
            runtime: &BridgeConsensusApplication,
        ) -> Box<BridgeStorageQueries>;
        pub fn create_dag_storage_queries(
            runtime: &BridgeConsensusApplication,
        ) -> Box<BridgeStorageQueries>;
        pub fn create_pbft_vote_storage_queries(
            runtime: &BridgeConsensusApplication,
        ) -> Box<BridgeStorageQueries>;
        pub fn create_transaction_storage_queries(
            runtime: &BridgeConsensusApplication,
        ) -> Box<BridgeStorageQueries>;
        pub fn create_final_chain_storage_queries(
            runtime: &BridgeConsensusApplication,
        ) -> Box<BridgeStorageQueries>;
        pub fn create_period_storage_queries(
            runtime: &BridgeConsensusApplication,
        ) -> Box<BridgeStorageQueries>;
        pub fn create_storage_shim_batch(
            runtime: &BridgeConsensusApplication,
        ) -> Box<BridgeStorageBatch>;
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
        pub fn storage_shim_clear_block_rewards_stats(
            runtime: &BridgeConsensusApplication,
        ) -> Result<()>;
        pub fn storage_shim_set_genesis_hash(
            runtime: &BridgeConsensusApplication,
            hash: &[u8; 32],
        ) -> Result<()>;
        #[allow(clippy::too_many_arguments)]
        pub fn storage_shim_seed_final_chain_conformance_lookup_rows(
            runtime: &BridgeConsensusApplication,
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

        pub fn dag_block_in_db(self: &BridgeStorageQueries, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_dag_block(self: &BridgeStorageQueries, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_dag_block_period_lookup(
            self: &BridgeStorageQueries,
            hash: &[u8; 32],
        ) -> Result<BlockPeriodLookup>;
        pub fn get_last_blocks_level(self: &BridgeStorageQueries) -> Result<u64>;
        pub fn get_blocks_by_level(self: &BridgeStorageQueries, level: u64) -> Result<Vec<u8>>;
        pub fn get_dag_blocks_at_level(
            self: &BridgeStorageQueries,
            level: u64,
            number_of_levels: u32,
        ) -> Result<Vec<CanonicalBytes>>;
        pub fn get_nonfinalized_dag_blocks(self: &BridgeStorageQueries)
            -> Result<Vec<LevelBlocks>>;
        pub fn get_proposal_period_for_dag_level(
            self: &BridgeStorageQueries,
            level: u64,
        ) -> Result<PeriodLookup>;

        /// Typed period reads (preferred for typed query surfaces).
        pub fn get_period_data_raw(self: &BridgeStorageQueries, period: u64) -> Result<Vec<u8>>;
        /// Typed period-by-PBFT-block hash lookup.
        pub fn get_period_from_pbft_hash(
            self: &BridgeStorageQueries,
            hash: &[u8; 32],
        ) -> Result<PeriodLookup>;
        /// Typed by-period receipts lookup.
        pub fn get_block_receipt(self: &BridgeStorageQueries, period: u64) -> Result<Vec<u8>>;
        pub fn pbft_block_in_db(self: &BridgeStorageQueries, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_pbft_mgr_field(self: &BridgeStorageQueries, field: u8) -> Result<u32>;
        pub fn get_pbft_mgr_status(self: &BridgeStorageQueries, field: u8) -> Result<bool>;
        pub fn get_pbft_head(self: &BridgeStorageQueries, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_cert_voted_block_in_round(self: &BridgeStorageQueries) -> Result<Vec<u8>>;
        pub fn save_proposed_pbft_block(
            self: &BridgeStorageQueries,
            expected_period: u64,
            expected_hash: &[u8; 32],
            expected_pivot_hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<bool>;
        pub fn get_proposed_pbft_blocks(self: &BridgeStorageQueries)
            -> Result<Vec<CanonicalBytes>>;
        pub fn get_own_verified_votes(self: &BridgeStorageQueries) -> Result<Vec<CanonicalBytes>>;
        pub fn get_all_two_t_plus_one_votes(
            self: &BridgeStorageQueries,
        ) -> Result<Vec<CanonicalBytes>>;
        pub fn get_reward_votes(self: &BridgeStorageQueries) -> Result<Vec<CanonicalBytes>>;
        pub fn transaction_in_db(self: &BridgeStorageQueries, hash: &[u8; 32]) -> Result<bool>;
        pub fn transaction_finalized(self: &BridgeStorageQueries, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_transaction_location(
            self: &BridgeStorageQueries,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_transaction(self: &BridgeStorageQueries, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_transaction_by_period_position(
            self: &BridgeStorageQueries,
            period: u64,
            position: u32,
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(self: &BridgeStorageQueries, period: u64) -> Result<u64>;
        pub fn get_system_transaction(
            self: &BridgeStorageQueries,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_all_nonfinalized_transactions(self: &BridgeStorageQueries)
            -> Result<Vec<TxRlp>>;
        pub fn get_all_transaction_period(self: &BridgeStorageQueries) -> Result<Vec<HashPeriod>>;
        pub fn get_period_system_transactions_hashes(
            self: &BridgeStorageQueries,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_meta_value(self: &BridgeStorageQueries, key: u32)
            -> Result<Vec<u8>>;
        pub fn get_final_chain_block_header(
            self: &BridgeStorageQueries,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_hash_by_number(
            self: &BridgeStorageQueries,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_number_by_hash(
            self: &BridgeStorageQueries,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_log_blooms_chunk(
            self: &BridgeStorageQueries,
            chunk_id: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_receipt_by_trx_hash(
            self: &BridgeStorageQueries,
            trx_hash: &[u8; 32],
        ) -> Result<Vec<u8>>;

        // Transaction envelope

        pub fn inspect_legacy_transaction_rlp(
            tx_rlp: Vec<u8>,
            source: u8,
        ) -> Result<LegacyTransactionInspection>;

        // FinalChain

        pub fn get_last_block_number(self: &BridgeConsensusApplication) -> Result<u64>;
        pub fn get_block_number(
            self: &BridgeConsensusApplication,
            hash: &[u8; 32],
        ) -> Result<FinalChainBlockNumberLookup>;
        pub fn get_block_hash(self: &BridgeConsensusApplication, num: u64) -> Result<Vec<u8>>;
        pub fn get_block_header(self: &BridgeConsensusApplication, num: u64) -> Result<Vec<u8>>;
        pub fn get_transaction_location(
            self: &BridgeConsensusApplication,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(self: &BridgeConsensusApplication, period: u64)
            -> Result<u64>;
        pub fn get_execution_status(
            self: &BridgeConsensusApplication,
        ) -> Result<FinalChainExecutionStatus>;
        pub fn get_blocks_with_bloom(
            self: &BridgeConsensusApplication,
            bloom: &[u8; 256],
            from: u64,
            to: u64,
        ) -> Result<Vec<u64>>;
        pub fn get_account_at_block(
            self: &BridgeConsensusApplication,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<AccountLookup>;
        pub fn get_dpos_eligible_vote_count(
            self: &BridgeConsensusApplication,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<u64>;
        pub fn get_dpos_eligible_total_vote_count(
            self: &BridgeConsensusApplication,
            block_number: u64,
        ) -> Result<u64>;
        pub fn get_dpos_validators_eligible_vote_counts(
            self: &BridgeConsensusApplication,
            block_number: u64,
        ) -> Result<Vec<HostValidatorVoteCount>>;
        pub fn get_dpos_validators_total_stakes(
            self: &BridgeConsensusApplication,
            block_number: u64,
        ) -> Result<Vec<DposValidatorStake>>;
        pub fn get_dpos_total_amount_delegated(
            self: &BridgeConsensusApplication,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_dpos_yield(self: &BridgeConsensusApplication, block_number: u64) -> Result<u64>;
        pub fn get_dpos_total_supply(
            self: &BridgeConsensusApplication,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn call(
            self: &BridgeConsensusApplication,
            request: FinalChainCall,
        ) -> Result<FinalChainCallOutcome>;
        type BridgeFinalChainExecutionSession;
        pub fn create_final_chain_execution_session(
            request: FinalChainExecutionRequest,
        ) -> Result<Box<BridgeFinalChainExecutionSession>>;
        pub fn recover_external_evm_pending_publication(
            self: &BridgeConsensusApplication,
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
            runtime: &BridgeConsensusApplication,
            session: Box<BridgeFinalChainExecutionSession>,
        ) -> Result<FinalChainExecutionCommitReport>;
        pub fn consensus_execution_next_execution_request(
            self: &BridgeConsensusExecutionApi,
            session: &mut BridgeFinalChainExecutionSession,
        ) -> Result<FinalChainExecutionStep>;
        pub fn consensus_execution_report_execution_result(
            self: &BridgeConsensusExecutionApi,
            runtime: &BridgeConsensusApplication,
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
            runtime: &BridgeConsensusApplication,
            session: &mut BridgeFinalChainExecutionSession,
            proposal_period_update: FinalChainProposalPeriodDagLevelUpdate,
        ) -> Result<FinalChainExternalEvmStateCommitIntent>;
        pub fn consensus_execution_report_state_commit_result(
            self: &BridgeConsensusExecutionApi,
            runtime: &BridgeConsensusApplication,
            session: &mut BridgeFinalChainExecutionSession,
            result: FinalChainExternalEvmStateCommitResult,
        ) -> Result<FinalChainExternalEvmCommitDecision>;
        pub fn consensus_execution_publish_state_commit(
            self: &BridgeConsensusExecutionApi,
            runtime: &BridgeConsensusApplication,
            session: &mut BridgeFinalChainExecutionSession,
        ) -> Result<FinalChainExternalEvmPublicationReport>;
        pub fn get_transaction_rlps(
            self: &BridgeConsensusApplication,
            period: u64,
        ) -> Result<Vec<TxRlp>>;
        pub fn get_transaction_receipt(
            self: &BridgeConsensusApplication,
            period: u64,
            position: u64,
        ) -> Result<Vec<u8>>;
    }
}
