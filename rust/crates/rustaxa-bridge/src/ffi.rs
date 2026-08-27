use crate::consensus_host_ports::*;
pub(crate) use crate::dag_transaction_service::BridgeApp;
pub use crate::dag_transaction_service::BridgeConsensusApplication;
use crate::dag_transaction_service::*;
use crate::final_chain::*;
use crate::network::*;
use crate::network_slashing::*;
use crate::query::*;
use crate::storage_admin::*;
use crate::vdf::*;
use rustaxa_consensus::ConsensusExecutionApi;
use rustaxa_consensus::ConsensusQueryApi;
use std::sync::Arc;

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

    /// Exact exclusive cutoffs for the application-root light-history admin task.
    struct LightHistoryPruneRequest {
        end_period_exclusive: u64,
        first_retained_dag_level: u64,
        live_cleanup: bool,
        non_block_periods_to_keep: u64,
    }

    /// Typed outcome of one atomic application-root light-history admin task.
    struct LightHistoryPruneReport {
        changed: bool,
        end_period_exclusive: u64,
        first_retained_dag_level: u64,
        rebuilt_secondary_indexes: bool,
    }

    /// One ordered key/value result from the closed v1 storage conformance scenario.
    struct StorageConformanceObservation {
        key: String,
        value: String,
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

    /// Lock-owned live DAG status for public and operational clients.
    ///
    /// The DTO contains only scalar/hash projections and never exposes the
    /// native graph, manager, service handle, or mutable lock domain.
    struct LiveDagStatusView {
        vertex_count: u64,
        edge_count: u64,
        max_level: u64,
        period: u64,
        old_anchor: [u8; 32],
        current_anchor: [u8; 32],
        expiry_level: u64,
        non_finalized_levels: u64,
        non_finalized_blocks: u64,
    }

    /// Lock-owned live transaction-pool status for public clients and metrics.
    struct LiveTransactionStatusView {
        transaction_count: u64,
        queue_size: u64,
        non_finalized_size: u64,
        gas_price_bid: [u8; 32],
        transactions_dropped: bool,
        non_proposable_over_limit: bool,
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

    /// TransactionQueue construction limits.
    struct TransactionQueueConfig {
        max_size: usize,
    }

    /// Canonical public transaction plus immutable chain-policy facts.
    ///
    /// Rust decodes every transaction field from `transaction_rlp`; the scalar
    /// policy fields describe the configured chain rules at `last_block_number`.
    struct PublicTransactionSubmissionRequest {
        transaction_rlp: Vec<u8>,
        expected_chain_id: u64,
        maximum_gas_limit: u64,
        minimum_gas_price: [u8; 32],
        last_block_number: u64,
        cornus_active: bool,
    }

    /// Exact FinalChain facts for the sender of one public transaction.
    ///
    /// `finalized_period_found == false` means the transaction is not present
    /// in finalized storage and `finalized_period` is ignored.
    struct PublicTransactionFinalChainFacts {
        sender: [u8; 20],
        account_found: bool,
        account_nonce: [u8; 32],
        account_balance: [u8; 32],
        finalized_period_found: bool,
        finalized_period: u64,
    }

    /// Terminal native result for one operation-shaped public submission.
    ///
    /// Deterministic rejection is represented by `accepted == false`; bridge
    /// errors are reserved for malformed input or infrastructure failure.
    struct PublicTransactionSubmissionReport {
        transaction_hash: [u8; 32],
        accepted: bool,
        message: String,
        verification_status: u8,
        queue_status_found: bool,
        queue_status: u8,
        transaction_observed: bool,
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
        allow_gossip: bool,
    }

    /// Operation-specific canonical get-PBFT-sync ingress request.
    struct NetworkGetPbftSyncRequest {
        tarcap_version: u32,
        peer_id: [u8; 64],
        request_rlp: Vec<u8>,
        source_payload_id: u64,
    }

    /// Canonical transaction-packet ingress plus immutable chain policy.
    struct NetworkTransactionPacketRequest {
        transport_lane: u32,
        peer_id: [u8; 64],
        source_payload_id: u64,
        packet_rlp: Vec<u8>,
        expected_chain_id: u64,
        maximum_gas_limit: u64,
        minimum_gas_price: [u8; 32],
        last_block_number: u64,
        cornus_active: bool,
        rebroadcast: bool,
    }

    /// One transaction member admitted from a canonical network packet.
    struct NetworkTransactionPacketMemberReport {
        submission: PublicTransactionSubmissionReport,
        observe_transaction: bool,
        gossip_transaction: bool,
        transaction_rlp: Vec<u8>,
    }

    /// Terminal report for one canonical transaction packet.
    struct NetworkTransactionPacketReport {
        decision: NetworkIngressDecision,
        transactions: Vec<NetworkTransactionPacketMemberReport>,
        extra_transaction_hashes: Vec<DagHash>,
    }

    /// Canonical get-DAG-sync ingress plus transport rate-limit fact.
    struct NetworkGetDagSyncRequest {
        transport_lane: u32,
        peer_id: [u8; 64],
        source_payload_id: u64,
        request_allowed: bool,
        request_rlp: Vec<u8>,
    }

    struct NetworkTransactionGossipPeer {
        peer_id: [u8; 64],
        known_hashes: Vec<DagHash>,
    }
    struct NetworkTransactionGossipRequest {
        transport_lane: u32,
        source_payload_id: u64,
        peers: Vec<NetworkTransactionGossipPeer>,
    }

    struct NetworkDagGossipPeer {
        peer_id: [u8; 64],
        syncing: bool,
        known_block: bool,
    }
    struct NetworkDagGossipRequest {
        transport_lane: u32,
        source_payload_id: u64,
        source_peer_id: [u8; 64],
        block_hash: [u8; 32],
        packet_rlp: Vec<u8>,
        peers: Vec<NetworkDagGossipPeer>,
    }

    struct NetworkDagPacketRequest {
        transport_lane: u32,
        peer_id: [u8; 64],
        source_payload_id: u64,
        packet_rlp: Vec<u8>,
        expected_chain_id: u64,
        maximum_gas_limit: u64,
        minimum_gas_price: [u8; 32],
        last_block_number: u64,
        cornus_active: bool,
        rebroadcast: bool,
        peer_dag_synced: bool,
        dag_sync_allowed: bool,
        transactions_dropped: bool,
        pending_dag_request: bool,
        local_pbft_syncing: bool,
    }

    #[derive(Default)]
    struct DagBlockIngressReport {
        block_hash: [u8; 32],
        block_level: u64,
        accepted: bool,
        duplicate: bool,
        reject_code: u32,
        observe_block: bool,
        gossip_block: bool,
        block_rlp: Vec<u8>,
    }

    struct NetworkDagBlockIngressReport {
        decision: NetworkIngressDecision,
        admission_found: bool,
        admission: DagBlockIngressReport,
        rejection_action: u8,
    }
    struct NetworkDagSyncIngressReport {
        decision: NetworkIngressDecision,
        request_period: u64,
        response_period: u64,
        transactions: Vec<NetworkTransactionPacketMemberReport>,
        blocks: Vec<DagBlockIngressReport>,
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

    /// One pillar vote packet after native routing and root-owned admission.
    struct NetworkPillarVoteAdmissionOutcome {
        decision: NetworkIngressDecision,
        has_admission: bool,
        status: u8,
        accepted: bool,
        duplicate: bool,
        conflict_found: bool,
        vote_hash: [u8; 32],
        conflicting_vote_hash: [u8; 32],
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

    /// GasPricer construction limits and mode flags supplied by C++ genesis config.
    struct GasPricerConfig {
        percentile: u64,
        minimum_price: [u8; 32],
        history_blocks: usize,
        is_light_node: bool,
        blocks_gas_pricer: bool,
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

    struct FinalizationDagBlock {
        author: [u8; 20],
        difficulty: u16,
        transaction_hashes: Vec<DagHash>,
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

    /// Key-custody-free native DAG proposer policy for one signing identity.
    struct DagProposerConfig {
        total_transaction_shards: u16,
        proposal_dag_gas_limit: u64,
        default_dag_gas_limit: u64,
        cornus_dag_gas_limit: u64,
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
        pub fn consensus_query_live_dag_status(
            self: &BridgeConsensusQueryApi,
        ) -> Result<LiveDagStatusView>;
        pub fn consensus_query_live_transaction_status(
            self: &BridgeConsensusQueryApi,
        ) -> Result<LiveTransactionStatusView>;
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
        pub fn consensus_application_prune_light_history(
            runtime: &BridgeConsensusApplication,
            request: LightHistoryPruneRequest,
        ) -> Result<LightHistoryPruneReport>;
        pub fn consensus_application_run_storage_conformance_v1(
            runtime: &BridgeConsensusApplication,
        ) -> Result<Vec<StorageConformanceObservation>>;
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
        ) -> Result<Vec<NetworkPillarVoteAdmissionOutcome>>;
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
        pub fn consensus_network_ingest_get_dag_sync_request(
            self: &BridgeConsensusNetworkApi,
            application: &BridgeConsensusApplication,
            request: NetworkGetDagSyncRequest,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_plan_transaction_gossip(
            self: &BridgeConsensusNetworkApi,
            application: &BridgeConsensusApplication,
            request: NetworkTransactionGossipRequest,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_plan_dag_block_gossip(
            self: &BridgeConsensusNetworkApi,
            request: NetworkDagGossipRequest,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_transaction_gossip_candidate_hashes(
            application: &BridgeConsensusApplication,
        ) -> Result<Vec<DagHash>>;
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
        pub fn consensus_network_request_pending_dag_blocks(
            self: &BridgeConsensusNetworkApi,
            application: &BridgeConsensusApplication,
            transport_lane: u32,
            source_payload_id: u64,
            facts: NetworkPendingDagBlocksRequestFacts,
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

        pub fn prove_legacy_vrf_sortition(
            secret_key: &[u8; 64],
            message: &[u8],
            vote_count: u16,
        ) -> VrfProofResult;

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
            dag_proposer: DagProposerConfig,
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
        /// Submits one canonical signed transaction without exposing the
        /// application-owned transaction service or queue.
        pub fn consensus_application_submit_transaction(
            application: &BridgeConsensusApplication,
            request: PublicTransactionSubmissionRequest,
            final_chain: PublicTransactionFinalChainFacts,
        ) -> Result<PublicTransactionSubmissionReport>;
        /// Returns the adaptive native transaction bid needed before the host
        /// signs an operation-specific system or slashing transaction.
        pub fn consensus_application_transaction_gas_price_bid(
            application: &BridgeConsensusApplication,
        ) -> Result<[u8; 32]>;
        /// Prunes native FinalChain lookup indexes below the retained block.
        pub fn prune_final_chain_before(
            self: &BridgeConsensusApplication,
            first_to_keep: u64,
        ) -> Result<u64>;
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
        // Network-owned verified-vote slashing acknowledgement leaf.
        pub fn pbft_service_verified_votes_report_slashing_transaction_submission(
            self: &BridgeConsensusApplication,
            proof_hash: &[u8; 32],
            transaction_inserted: bool,
        ) -> Result<bool>;
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
