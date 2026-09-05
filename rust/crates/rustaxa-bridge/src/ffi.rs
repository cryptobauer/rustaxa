use crate::arena::create_packet_arena;
use crate::consensus_host_ports::*;
pub(crate) use crate::dag_transaction_service::BridgeApp;
pub use crate::dag_transaction_service::BridgeConsensusApplication;
use crate::dag_transaction_service::*;
use crate::network::*;
use crate::network_ingress::*;
use crate::query::*;
use crate::storage_admin::*;
use rustaxa_arena::arena::Arena;
use rustaxa_consensus::ConsensusQueryApi;
use rustaxa_network::{network::Network, packet::Packet};
use std::sync::Arc;

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
pub struct BridgeConsensusNetworkApi(pub(crate) Arc<rustaxa_consensus::ConsensusNetworkApi>);

/// Experimental packet ingress pipeline; independent of the native consensus network API.
pub struct BridgeNetwork(pub Network);
/// Shared packet storage for the experimental ingress pipeline.
pub struct BridgePacketArena(pub Arc<Arena<Packet>>);

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

    /// Read-only projection of the application-owned PBFT-sync lifecycle.
    ///
    /// The DTO contains stable scalar and peer-identity facts only. It exposes
    /// neither a network-service handle nor mutable lifecycle state; elapsed
    /// fields are derived from the caller's monotonic timestamp without
    /// advancing inactivity policy.
    struct PbftSyncStatusView {
        active: bool,
        deep_syncing: bool,
        generation: u64,
        has_peer: bool,
        peer_id: [u8; 64],
        has_last_peer: bool,
        last_peer_id: [u8; 64],
        target_chain_size: u64,
        current_period: u64,
        request_period: u64,
        started_at_ms: u64,
        last_activity_ms: u64,
        elapsed_ms: u64,
        inactive_for_ms: u64,
        start_count: u64,
        stop_count: u64,
        inactivity_count: u64,
        disconnect_count: u64,
        last_stop_reason: u8,
    }

    /// Public/query sortition params-change view for Test RPC compatibility.
    struct SortitionParamsChangeView {
        found: bool,
        period: u64,
        interval_efficiency: u16,
        threshold_upper: u16,
        threshold_upper_min: u16,
    }

    /// Rewards distribution frequency rule active from `from_period` onward.
    struct RewardsFrequencyRule {
        from_period: u64,
        frequency: u32,
    }

    /// Previous-block cert-vote fact for rewards-stat planning.
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
    /// Executor-visible network effect planned by Rust consensus.
    struct NetworkEffect {
        effect_id: u64,
        source_payload_id: u64,
        transport_lane: u32,
        kind: u8,
        peer_id: [u8; 64],
        packet_kind: u32,
        payload_bytes: Vec<u8>,
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

    /// Complete canonical vote-family packet plus compact live ingress policy.
    struct NetworkConsensusPacketRequest {
        transport_lane: u32,
        peer_id: [u8; 64],
        peer_pbft_chain_size: u64,
        source_payload_id: u64,
        packet_rlp: Vec<u8>,
        current_period: u64,
        current_round: u64,
        current_step: u64,
        max_future_period_delta: u64,
        max_future_round_delta: u64,
        max_future_step_delta: u64,
        validate_max_round_step: bool,
        can_request_pbft_sync: bool,
        can_request_next_votes_sync: bool,
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
    }

    /// One transaction member admitted from a canonical network packet.
    struct NetworkTransactionPacketMemberReport {
        submission: PublicTransactionSubmissionReport,
        observe_transaction: bool,
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

    /// Canonical input for one bounded native egress preparation.
    struct NetworkEgressPrepareRequest {
        family: u8,
        transport_lane: u32,
        source_payload_id: u64,
        source_peer_id: [u8; 64],
        rebroadcast: bool,
        object_hash: [u8; 32],
        payload_bytes: Vec<u8>,
        related_payload_bytes: Vec<u8>,
    }

    struct NetworkEgressProbe {
        probe_id: u32,
        object_kind: u8,
        object_hash: [u8; 32],
    }

    struct NetworkEgressPreparation {
        token: u64,
        probes: Vec<NetworkEgressProbe>,
    }

    struct NetworkEgressPeerSnapshot {
        transport_lane: u32,
        peer_id: [u8; 64],
        syncing: bool,
        known_probe_ids: Vec<u32>,
        pbft_chain_size: u64,
        dag_level: u64,
        is_light_node: bool,
        light_node_history: u64,
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

    /// Terminal native result for one PBFT vote-family packet.
    struct NetworkPbftVotePacketReport {
        status: u8,
        error_code: String,
        malicious: bool,
        outcomes: Vec<NetworkPbftVoteAdmissionOutcome>,
        has_peer_pbft_chain_size: bool,
        peer_pbft_chain_size: u64,
        egress_payload_bytes: Vec<u8>,
    }

    /// Terminal native result for one pillar-vote-family packet.
    struct NetworkPillarVotePacketReport {
        status: u8,
        error_code: String,
        malicious: bool,
        outcomes: Vec<NetworkPillarVoteAdmissionOutcome>,
    }

    /// Complete canonical status ingress plus mutable local follow-up facts.
    struct NetworkStatusPacketRequest {
        peer_id: [u8; 64],
        packet_rlp: Vec<u8>,
        source_peer_ready: bool,
        local_pbft_synced_period: u64,
        local_pbft_period: u64,
        local_pbft_round: u64,
        peer_dag_synced: bool,
    }

    /// Typed peer bookkeeping and exact follow-up transport for one status packet.
    struct NetworkStatusPacketReport {
        status: u8,
        error_code: String,
        malicious: bool,
        initial: bool,
        accept_peer: bool,
        disconnect_peer: bool,
        peer_pbft_chain_size: u64,
        peer_pbft_period: u64,
        peer_pbft_round: u64,
        peer_dag_level: u64,
        peer_syncing: bool,
        peer_is_light_node: bool,
        peer_light_node_history: u64,
        node_major_version: u32,
        node_minor_version: u32,
        node_patch_version: u32,
        request_pbft_sync: bool,
        request_pending_dag_blocks: bool,
        request_next_votes: bool,
        next_votes_period: u64,
        next_votes_round: u64,
        next_votes_request_rlp: Vec<u8>,
        sync_generation: u64,
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

    /// Compact facts needed to plan a pending-DAG-block request.
    struct NetworkPendingDagBlocksRequestFacts {
        local_pbft_syncing_period: u64,
        has_explicit_peer: bool,
        explicit_peer: NetworkPbftSyncPeerCandidate,
        candidates: Vec<NetworkPbftSyncPeerCandidate>,
    }

    /// Canonical peer snapshot used to start one application-owned PBFT-sync generation.
    struct NetworkPbftSyncStartRequest {
        start: bool,
        now_ms: u64,
        local_pbft_synced_period: u64,
        local_pbft_chain_size: u64,
        candidates: Vec<NetworkPbftSyncPeerCandidate>,
    }

    /// Atomic result of selecting a peer and starting native PBFT sync.
    struct NetworkPbftSyncStartOutcome {
        status: u8,
        error_code: String,
        started: bool,
        has_peer: bool,
        peer_id: [u8; 64],
        peer_pbft_chain_size: u64,
        request_period: u64,
        generation: u64,
        deep_syncing: bool,
        enable_snapshot_creation: bool,
    }

    /// Mutable status fields; immutable identity is application-owned.
    struct NetworkStatusPacketBuildRequest {
        initial: bool,
        local_pbft_chain_size: u64,
        local_pbft_round: u64,
        local_dag_level: u64,
    }

    /// Exact canonical legacy status payload ready for packet wrapping.
    struct NetworkStatusPacketBuildOutcome {
        status: u8,
        error_code: String,
        packet_rlp: Vec<u8>,
    }

    /// Complete canonical request packet with transport correlation.
    struct NetworkCanonicalRequestPacket {
        transport_lane: u32,
        peer_id: [u8; 64],
        source_payload_id: u64,
        packet_rlp: Vec<u8>,
    }

    /// Exact native lifecycle command. Kinds 0-4 consume source/time/generation/
    /// peer/reason fields; completion kind 5 consumes queue size, and continuation
    /// kinds 6-7 consume period/level plus retry count/delay facts.
    struct NetworkPbftSyncCommand {
        kind: u8,
        now_ms: u64,
        generation: u64,
        peer_id: [u8; 64],
        source: u8,
        reason: u8,
        sync_queue_size: u64,
        syncing_period: u64,
        finalized_period: u64,
        remote_period: u64,
        sync_level_size: u64,
        retry_count: u32,
        retry_delay_ms: u64,
    }

    /// Shared typed result projection for exact native sync lifecycle operations.
    struct NetworkPbftSyncCommandOutcome {
        accepted: bool,
        active: bool,
        stopped: bool,
        expired: bool,
        restart_sync: bool,
        retry: bool,
        request_next: bool,
        request_pending_dag_if_idle: bool,
        deep_syncing: bool,
        generation: u64,
        error_code: String,
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
        deep_syncing_threshold: u64,
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

    /// Public/query PBFT `2t+1` threshold lookup result.
    struct PbftTwoTPlusOneThresholdPlan {
        status: u8,
        error_code: String,
        has_threshold: bool,
        threshold: u64,
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
        header_rlp: Vec<u8>,
        stored_header_rlp: Vec<u8>,
        has_pbft_hash: bool,
        pbft_block_hash: [u8; 32],
    }

    struct FinalChainBlockNumberLookup {
        found: bool,
        value: u64,
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

    struct DposValidatorStake {
        address: [u8; 20],
        stake: Vec<u8>,
    }

    struct FinalChainNativeCall {
        block_number: u64,
        sender: [u8; 20],
        receiver_found: bool,
        receiver: [u8; 20],
        value: Vec<u8>,
        gas_price: Vec<u8>,
        gas_limit: u64,
        input: Vec<u8>,
    }

    struct FinalChainNativeCallLogTopic {
        topic: [u8; 32],
    }

    struct FinalChainNativeCallLog {
        address: [u8; 20],
        topics: Vec<FinalChainNativeCallLogTopic>,
        data: Vec<u8>,
    }

    struct FinalChainNativeCallOutcome {
        code_retval: Vec<u8>,
        logs: Vec<FinalChainNativeCallLog>,
        gas_used: u64,
        code_err: String,
        consensus_err: String,
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
        pub fn consensus_query_final_chain_dpos_eligible_vote_count(
            self: &BridgeConsensusQueryApi,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<u64>;
        pub fn consensus_query_final_chain_dpos_eligible_total_vote_count(
            self: &BridgeConsensusQueryApi,
            block_number: u64,
        ) -> Result<u64>;
        pub fn consensus_query_final_chain_dpos_validators_total_stakes(
            self: &BridgeConsensusQueryApi,
            block_number: u64,
        ) -> Result<Vec<DposValidatorStake>>;
        pub fn consensus_query_final_chain_dpos_total_amount_delegated(
            self: &BridgeConsensusQueryApi,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn consensus_query_final_chain_dpos_yield(
            self: &BridgeConsensusQueryApi,
            block_number: u64,
        ) -> Result<u64>;
        pub fn consensus_query_final_chain_dpos_total_supply(
            self: &BridgeConsensusQueryApi,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn consensus_query_final_chain_native_call(
            self: &BridgeConsensusQueryApi,
            request: FinalChainNativeCall,
        ) -> Result<FinalChainNativeCallOutcome>;
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
        pub fn consensus_query_pbft_sync_status(
            self: &BridgeConsensusQueryApi,
            now_ms: u64,
        ) -> Result<PbftSyncStatusView>;
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
        pub fn consensus_network_ingest_pbft_vote_packet(
            self: &BridgeConsensusNetworkApi,
            request: NetworkConsensusPacketRequest,
            slashing_submitters: Vec<SlashingSubmitterIdentity>,
        ) -> Result<NetworkPbftVotePacketReport>;
        pub fn consensus_network_ingest_pbft_votes_bundle_packet(
            self: &BridgeConsensusNetworkApi,
            request: NetworkConsensusPacketRequest,
            slashing_submitters: Vec<SlashingSubmitterIdentity>,
        ) -> Result<NetworkPbftVotePacketReport>;
        pub fn consensus_network_ingest_pillar_vote_packet(
            self: &BridgeConsensusNetworkApi,
            request: NetworkConsensusPacketRequest,
        ) -> Result<NetworkPillarVotePacketReport>;
        pub fn consensus_network_ingest_pillar_votes_bundle_packet(
            self: &BridgeConsensusNetworkApi,
            request: NetworkConsensusPacketRequest,
        ) -> Result<NetworkPillarVotePacketReport>;
        pub fn consensus_network_ingest_pbft_next_votes_bundle_request(
            self: &BridgeConsensusNetworkApi,
            request: NetworkCanonicalRequestPacket,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_ingest_pillar_votes_bundle_request(
            self: &BridgeConsensusNetworkApi,
            request: NetworkCanonicalRequestPacket,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_ingest_get_pbft_sync_request(
            self: &BridgeConsensusNetworkApi,
            request: NetworkGetPbftSyncRequest,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_ingest_pbft_blocks_bundle(
            self: &BridgeConsensusNetworkApi,
            packet_rlp: Vec<u8>,
            source_payload_id: u64,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_ingest_get_dag_sync_request(
            self: &BridgeConsensusNetworkApi,
            request: NetworkGetDagSyncRequest,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_prepare_egress(
            self: &BridgeConsensusNetworkApi,
            request: NetworkEgressPrepareRequest,
        ) -> Result<NetworkEgressPreparation>;
        pub fn consensus_network_plan_egress(
            self: &BridgeConsensusNetworkApi,
            token: u64,
            peers: Vec<NetworkEgressPeerSnapshot>,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_cancel_egress(
            self: &BridgeConsensusNetworkApi,
            token: u64,
        ) -> Result<bool>;
        pub fn consensus_network_begin_pbft_sync(
            self: &BridgeConsensusNetworkApi,
            request: NetworkPbftSyncStartRequest,
        ) -> Result<NetworkPbftSyncStartOutcome>;
        pub fn consensus_network_ingest_status_packet(
            self: &BridgeConsensusNetworkApi,
            request: NetworkStatusPacketRequest,
        ) -> Result<NetworkStatusPacketReport>;
        pub fn consensus_network_build_status_packet(
            self: &BridgeConsensusNetworkApi,
            request: NetworkStatusPacketBuildRequest,
        ) -> Result<NetworkStatusPacketBuildOutcome>;
        pub fn consensus_network_apply_pbft_sync_command(
            self: &BridgeConsensusNetworkApi,
            request: NetworkPbftSyncCommand,
        ) -> Result<NetworkPbftSyncCommandOutcome>;
        pub fn consensus_network_request_pending_dag_blocks(
            self: &BridgeConsensusNetworkApi,
            transport_lane: u32,
            source_payload_id: u64,
            facts: NetworkPendingDagBlocksRequestFacts,
        ) -> Result<NetworkIngressDecision>;
        pub fn consensus_network_begin_pbft_sync_ingress(
            self: &BridgeConsensusNetworkApi,
            packet_rlp: &[u8],
            source_payload_id: u64,
            source_peer_id: [u8; 64],
            slashing_submitters: Vec<SlashingSubmitterIdentity>,
        ) -> Result<PbftSyncIngressStep>;
        pub fn consensus_network_report_pbft_sync_ingress_slashing(
            self: &BridgeConsensusNetworkApi,
            proof_hash: [u8; 32],
            transaction_inserted: bool,
        ) -> Result<PbftSyncIngressStep>;
        pub fn consensus_network_report_verified_vote_slashing_submission(
            self: &BridgeConsensusNetworkApi,
            proof_hash: &[u8; 32],
            transaction_inserted: bool,
        ) -> Result<bool>;

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
            final_chain_state_api_committed_period: u64,
            final_chain_state_api_committed_root: [u8; 32],
            final_chain_bridge_contract_address: [u8; 20],
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
        ) -> Result<PublicTransactionSubmissionReport>;
        // Network

        type BridgeNetwork;

        pub fn create_network(
            arena: &BridgePacketArena,
            queue_size: usize,
        ) -> Result<Box<BridgeNetwork>>;
        pub fn start_network(self: &mut BridgeNetwork) -> Result<()>;
        pub fn connect_peer(self: &mut BridgeNetwork, node: [u8; 64]) -> Result<bool>;
        pub fn disconnect_peer(self: &mut BridgeNetwork, node: [u8; 64]) -> Result<()>;
        pub fn queue_is_full(self: &BridgeNetwork) -> bool;
        pub fn ingest_network_packet(
            self: &mut BridgeNetwork,
            packet_type: u8,
            from_node: [u8; 64],
            data: Vec<u8>,
        ) -> Result<()>;

        // Arena

        type BridgePacketArena;

        pub fn create_packet_arena(size: usize) -> Result<Box<BridgePacketArena>>;
    }
}
