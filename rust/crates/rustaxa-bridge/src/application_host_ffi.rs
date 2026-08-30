//! Application-only CXX boundary for concrete consensus host leaves.
//!
//! Its dedicated translation unit keeps storage, VDF, and query leaf links from
//! retaining callbacks implemented only by the C++ application shell.

use crate::consensus_host_ports::{
    consensus_application_finalize, consensus_application_run,
    consensus_application_submit_transaction_with_execution,
    consensus_network_ingest_dag_block_packet, consensus_network_ingest_dag_sync_packet,
    consensus_network_ingest_transaction_packet,
};
use crate::dag_transaction_service::BridgeConsensusApplication;
use crate::ffi::BridgeConsensusNetworkApi;

// Bind the aggregate bridge's generated C++ opaque application type to its
// existing Rust owner instead of declaring a duplicate Rust type.
unsafe impl cxx::ExternType for BridgeConsensusApplication {
    type Id = cxx::type_id!("rustaxa::BridgeConsensusApplication");
    type Kind = cxx::kind::Opaque;
}

unsafe impl cxx::ExternType for BridgeConsensusNetworkApi {
    type Id = cxx::type_id!("rustaxa::BridgeConsensusNetworkApi");
    type Kind = cxx::kind::Opaque;
}

#[cxx::bridge(namespace = "rustaxa")]
pub mod application_host_ffi {
    /// Canonical encoded bytes used only by application host-port list payloads.
    struct CanonicalBytes {
        data: Vec<u8>,
    }

    /// Stable identity for one native-consensus request executed by the host.
    struct HostEffectId {
        generation: u64,
        sequence: u64,
    }
    /// Blocking wait requested by the native consensus runner.
    struct HostWaitRequest {
        effect_id: HostEffectId,
        delay_ms: u64,
    }
    /// Result of one host wait. `outcome` is 0 for elapsed, 1 for stopped.
    struct HostWaitReport {
        effect_id: HostEffectId,
        outcome: u8,
    }
    /// Digest-signing request for a configured wallet index.
    struct HostSignRequest {
        effect_id: HostEffectId,
        wallet_index: u64,
        digest: [u8; 32],
    }
    /// Recoverable-signature result for one digest-signing request.
    struct HostSignReport {
        effect_id: HostEffectId,
        succeeded: bool,
        signature: Vec<u8>,
        error_code: String,
    }
    /// VRF proof request whose secret key remains in the C++ host.
    struct HostVrfRequest {
        effect_id: HostEffectId,
        wallet_index: u64,
        message: Vec<u8>,
    }
    /// Public VRF proof/output produced by the selected host wallet.
    struct HostVrfReport {
        effect_id: HostEffectId,
        succeeded: bool,
        proof: Vec<u8>,
        output: Vec<u8>,
        error_code: String,
    }
    /// Gossips one canonical vote and its optional proposed PBFT block.
    struct HostGossipVoteRequest {
        effect_id: HostEffectId,
        vote_rlp: Vec<u8>,
        proposed_block_rlp: Vec<u8>,
        rebroadcast: bool,
    }
    /// Gossips one canonical PBFT vote bundle.
    struct HostGossipVoteBundleRequest {
        effect_id: HostEffectId,
        votes_bundle_rlp: Vec<u8>,
        rebroadcast: bool,
    }
    /// Gossips one canonical signed pillar-chain vote.
    struct HostGossipPillarVoteRequest {
        effect_id: HostEffectId,
        pillar_vote_rlp: Vec<u8>,
        rebroadcast: bool,
    }
    /// Publishes one locally committed canonical DAG block by hash.
    struct HostGossipDagBlockRequest {
        effect_id: HostEffectId,
        block_hash: [u8; 32],
        block_rlp: Vec<u8>,
    }
    /// Reports a malicious peer and canonical evidence to the network leaf.
    struct HostMaliciousPeerRequest {
        effect_id: HostEffectId,
        peer_id: [u8; 64],
        evidence_rlp: Vec<u8>,
    }
    /// Common acknowledgement for one operation-specific transport request.
    struct HostTransportReport {
        effect_id: HostEffectId,
        succeeded: bool,
        error_code: String,
    }
    /// Current physical transport availability and packet-queue pressure.
    struct HostTransportStatus {
        available: bool,
        packet_queue_over_limit: bool,
    }
    /// One canonical EVM address carried by an ordered host request.
    struct HostAddress20 {
        bytes: [u8; 20],
    }
    /// One FinalChain account observed at the report's exact block number.
    struct HostFinalChainAccountFact {
        address: [u8; 20],
        found: bool,
        nonce: [u8; 32],
        balance: [u8; 32],
    }
    /// Ordered account addresses requested from one FinalChain snapshot.
    struct HostFinalChainAccountFactsRequest {
        effect_id: HostEffectId,
        addresses: Vec<HostAddress20>,
    }
    /// Exact account facts observed from one FinalChain block.
    struct HostFinalChainAccountFactsReport {
        effect_id: HostEffectId,
        succeeded: bool,
        observed_block: u64,
        accounts: Vec<HostFinalChainAccountFact>,
        error_code: String,
    }
    /// Exact FinalChain lookup needed to restore a persisted pillar anchor.
    struct HostPillarAnchorStateRequest {
        effect_id: HostEffectId,
        period: u64,
    }
    /// Exact bridge-contract facts returned by the concrete EVM leaf.
    struct HostPillarAnchorStateReport {
        effect_id: HostEffectId,
        succeeded: bool,
        bridge_root: [u8; 32],
        bridge_epoch: [u8; 32],
        error_code: String,
    }
    /// Exact StateAPI facts needed by native system-transaction planning.
    struct HostFinalChainSystemFactsRequest {
        request_id: [u8; 32],
        period: u64,
        is_pillar_block_period: bool,
        bridge_contract_address: [u8; 20],
        block_gas_limit: u64,
    }
    /// Read-only state-db descriptor preflight before pending execution begins.
    struct HostFinalChainPreflightRequest {
        request_id: [u8; 32],
        next_period: u64,
        expected_prior_period: u64,
        expected_prior_state_root: [u8; 32],
    }
    struct HostFinalChainPreflightReport {
        request_id: [u8; 32],
        committed_period: u64,
        committed_state_root: [u8; 32],
        succeeded: bool,
        error_code: String,
    }
    struct HostFinalChainSystemFactsReport {
        request_id: [u8; 32],
        period: u64,
        bridge_contract_found: bool,
        bridge_contract_has_code: bool,
        should_finalize_epoch: bool,
        system_account_nonce: Vec<u8>,
        succeeded: bool,
        error_code: String,
    }
    struct HostFinalChainTransactionInput {
        position: u32,
        hash: [u8; 32],
        sender: [u8; 20],
        receiver_found: bool,
        receiver: [u8; 20],
        nonce: Vec<u8>,
        value: Vec<u8>,
        gas_price: Vec<u8>,
        gas_limit: u64,
        data: Vec<u8>,
        rlp: Vec<u8>,
        kind: u8,
        is_system: bool,
    }
    struct HostFinalChainExecutionRequest {
        request_id: [u8; 32],
        period: u64,
        block_author: [u8; 20],
        timestamp: u64,
        block_gas_limit: u64,
        transactions: Vec<HostFinalChainTransactionInput>,
    }
    struct HostFinalChainLogTopic {
        topic: [u8; 32],
    }
    struct HostFinalChainLog {
        address: [u8; 20],
        topics: Vec<HostFinalChainLogTopic>,
        data: Vec<u8>,
    }
    struct HostFinalChainTransactionResult {
        position: u32,
        hash: [u8; 32],
        status: u8,
        gas_used: u64,
        cumulative_gas_used: u64,
        receipt_rlp: Vec<u8>,
        logs: Vec<HostFinalChainLog>,
        new_contract_address_found: bool,
        new_contract_address: [u8; 20],
        code_error: String,
        consensus_error: String,
    }
    struct HostFinalChainExecutionReport {
        request_id: [u8; 32],
        status: u8,
        cumulative_gas_used: u64,
        results: Vec<HostFinalChainTransactionResult>,
        error_code: String,
    }
    struct HostFinalChainRewardsRequest {
        request_id: [u8; 32],
        period: u64,
        block_author: [u8; 20],
        block_gas_used: u64,
        transaction_gas_used: Vec<u64>,
        transaction_fees: Vec<CanonicalBytes>,
        finalized_dag_block_count: u64,
        distribution_stats: Vec<HostRewardsStatsPeriod>,
    }
    struct HostRewardsStatsPeriod {
        period: u64,
        data: Vec<u8>,
    }
    struct HostFinalChainRewardsReport {
        request_id: [u8; 32],
        period: u64,
        status: u8,
        state_root: [u8; 32],
        total_reward: Vec<u8>,
        error_code: String,
    }
    struct HostFinalChainStateCommitRequest {
        request_id: [u8; 32],
        plan_id: [u8; 32],
        period: u64,
        publication_block_hash: [u8; 32],
        expected_state_root: [u8; 32],
    }
    struct HostFinalChainStateCommitReport {
        status: u8,
        committed_period: u64,
        committed_state_root: [u8; 32],
        error_code: String,
    }
    struct HostFinalChainFinalizeTask {
        pbft_block_rlp: Vec<u8>,
        previous_cert_vote_bundle_rlp: Vec<u8>,
        dag_block_bundle_rlp: Vec<u8>,
        transaction_rlps: Vec<CanonicalBytes>,
        previous_cert_votes: Vec<HostRewardCertVote>,
        finalized_dag_hashes: Vec<DagHash>,
        blocks_per_year: u32,
        anchor_block_rlp: Vec<u8>,
    }
    /// Signed canonical PBFT vote plus the verified legacy weight sidecar.
    struct HostRewardCertVote {
        rlp: Vec<u8>,
        weight: u64,
    }
    struct HostFinalChainFinalizeReport {
        period: u64,
        block_hash: [u8; 32],
        executed_dag_blocks: u64,
        executed_transactions: u64,
        status: u8,
        error_code: String,
    }
    /// Bidirectional gas batch with echoed identity/order and all-or-none results.
    struct HostDagGasBatch {
        effect_id: HostEffectId,
        proposal_period: u64,
        transaction_hashes: Vec<DagHash>,
        transaction_rlps: Vec<CanonicalBytes>,
        succeeded: bool,
        observed_block: u64,
        gas_used: Vec<u64>,
        result_rlps: Vec<CanonicalBytes>,
        error_code: String,
    }
    /// Starts one asynchronous native DAG-VDF proof job.
    struct HostDagVdfRequest {
        effect_id: HostEffectId,
        wallet_index: u64,
        vrf_input: Vec<u8>,
        vdf_message: Vec<u8>,
        vote_count: u64,
        max_vote_count: u64,
        difficulty: u16,
        lambda_bound: u16,
    }
    struct HostDagVdfStartReport {
        effect_id: HostEffectId,
        started: bool,
        job_id: u64,
        error_code: String,
    }
    /// Identifies one exact asynchronous DAG-VDF job for polling or cancellation.
    struct HostDagVdfJobRequest {
        effect_id: HostEffectId,
        job_id: u64,
    }
    struct HostDagVdfPollReport {
        effect_id: HostEffectId,
        job_id: u64,
        complete: bool,
        succeeded: bool,
        cancelled: bool,
        vdf_rlp: Vec<u8>,
        error_code: String,
    }
    struct HostDagVdfCancelReport {
        effect_id: HostEffectId,
        job_id: u64,
        cancelled: bool,
        error_code: String,
    }
    /// Post-commit public observation emitted by native consensus.
    struct HostConsensusObservationRequest {
        effect_id: HostEffectId,
        kind: u8,
        period: u64,
        hash: [u8; 32],
        canonical_rlp: Vec<u8>,
    }
    struct HostConsensusObservationReport {
        effect_id: HostEffectId,
        succeeded: bool,
        error_code: String,
    }
    /// Terminal reason returned by the blocking native consensus runner.
    struct ConsensusRunExit {
        generation: u64,
        reason: u8,
        error_code: String,
    }

    unsafe extern "C++" {
        include!("consensus/consensus_host_ports.hpp");
        include!("rustaxa-bridge/src/ffi.rs.h");

        type BridgeConsensusApplication =
            crate::dag_transaction_service::BridgeConsensusApplication;
        type DagHash = crate::ffi::rustaxa_ffi::DagHash;
        type HostValidatorVoteCount = crate::ffi::rustaxa_ffi::HostValidatorVoteCount;
        type PublicTransactionSubmissionRequest =
            crate::ffi::rustaxa_ffi::PublicTransactionSubmissionRequest;
        type PublicTransactionSubmissionReport =
            crate::ffi::rustaxa_ffi::PublicTransactionSubmissionReport;
        type NetworkTransactionPacketRequest =
            crate::ffi::rustaxa_ffi::NetworkTransactionPacketRequest;
        type NetworkTransactionPacketReport =
            crate::ffi::rustaxa_ffi::NetworkTransactionPacketReport;
        type NetworkDagPacketRequest = crate::ffi::rustaxa_ffi::NetworkDagPacketRequest;
        type NetworkDagBlockIngressReport = crate::ffi::rustaxa_ffi::NetworkDagBlockIngressReport;
        type NetworkDagSyncIngressReport = crate::ffi::rustaxa_ffi::NetworkDagSyncIngressReport;
        type BridgeConsensusNetworkApi = crate::ffi::BridgeConsensusNetworkApi;

        #[namespace = "taraxa"]
        type ConsensusProcessPort;
        #[namespace = "taraxa"]
        type ConsensusSignerPort;
        #[namespace = "taraxa"]
        type ConsensusTransportPort;
        #[namespace = "taraxa"]
        type ExternalEvmPort;

        #[cxx_name = "consensusNowMillis"]
        fn consensus_now_millis(self: &ConsensusProcessPort) -> u64;
        #[cxx_name = "consensusUnixTimeSeconds"]
        fn consensus_unix_time_seconds(self: &ConsensusProcessPort) -> u64;
        #[cxx_name = "consensusWait"]
        fn consensus_wait(
            self: &ConsensusProcessPort,
            request: &HostWaitRequest,
        ) -> Result<HostWaitReport>;
        #[cxx_name = "consensusStopRequested"]
        fn consensus_stop_requested(self: &ConsensusProcessPort, generation: u64) -> bool;

        #[cxx_name = "consensusSignDigest"]
        fn consensus_sign_digest(
            self: &ConsensusSignerPort,
            request: &HostSignRequest,
        ) -> Result<HostSignReport>;
        #[cxx_name = "consensusProveVrf"]
        fn consensus_prove_vrf(
            self: &ConsensusSignerPort,
            request: &HostVrfRequest,
        ) -> Result<HostVrfReport>;

        #[cxx_name = "consensusGossipVote"]
        fn consensus_gossip_vote(
            self: &ConsensusTransportPort,
            request: &HostGossipVoteRequest,
        ) -> Result<HostTransportReport>;
        #[cxx_name = "consensusGossipVoteBundle"]
        fn consensus_gossip_vote_bundle(
            self: &ConsensusTransportPort,
            request: &HostGossipVoteBundleRequest,
        ) -> Result<HostTransportReport>;
        #[cxx_name = "consensusGossipPillarVote"]
        fn consensus_gossip_pillar_vote(
            self: &ConsensusTransportPort,
            request: &HostGossipPillarVoteRequest,
        ) -> Result<HostTransportReport>;
        #[cxx_name = "consensusGossipDagBlock"]
        fn consensus_gossip_dag_block(
            self: &ConsensusTransportPort,
            request: &HostGossipDagBlockRequest,
        ) -> Result<HostTransportReport>;
        #[cxx_name = "consensusTransportStatus"]
        fn consensus_transport_status(self: &ConsensusTransportPort) -> HostTransportStatus;
        #[cxx_name = "consensusReportMaliciousPeer"]
        fn consensus_report_malicious_peer(
            self: &ConsensusTransportPort,
            request: &HostMaliciousPeerRequest,
        ) -> Result<HostTransportReport>;
        #[cxx_name = "consensusStartDagVdf"]
        fn consensus_start_dag_vdf(
            self: &ConsensusProcessPort,
            request: &HostDagVdfRequest,
        ) -> Result<HostDagVdfStartReport>;
        #[cxx_name = "consensusPollDagVdf"]
        fn consensus_poll_dag_vdf(
            self: &ConsensusProcessPort,
            request: &HostDagVdfJobRequest,
        ) -> Result<HostDagVdfPollReport>;
        #[cxx_name = "consensusCancelDagVdf"]
        fn consensus_cancel_dag_vdf(
            self: &ConsensusProcessPort,
            request: &HostDagVdfJobRequest,
        ) -> Result<HostDagVdfCancelReport>;
        #[cxx_name = "consensusObserve"]
        fn consensus_observe(
            self: &ConsensusProcessPort,
            request: &HostConsensusObservationRequest,
        ) -> Result<HostConsensusObservationReport>;

        #[cxx_name = "consensusLoadFinalChainSystemFacts"]
        fn consensus_load_final_chain_system_facts(
            self: &ExternalEvmPort,
            request: &HostFinalChainSystemFactsRequest,
        ) -> Result<HostFinalChainSystemFactsReport>;
        #[cxx_name = "consensusLoadFinalChainCommittedState"]
        fn consensus_load_final_chain_committed_state(
            self: &ExternalEvmPort,
            request: &HostFinalChainPreflightRequest,
        ) -> Result<HostFinalChainPreflightReport>;
        #[cxx_name = "consensusExecuteFinalChainTransactions"]
        fn consensus_execute_final_chain_transactions(
            self: &ExternalEvmPort,
            request: &HostFinalChainExecutionRequest,
        ) -> Result<HostFinalChainExecutionReport>;
        #[cxx_name = "consensusDistributeFinalChainRewards"]
        fn consensus_distribute_final_chain_rewards(
            self: &ExternalEvmPort,
            request: &HostFinalChainRewardsRequest,
        ) -> Result<HostFinalChainRewardsReport>;
        #[cxx_name = "consensusCommitFinalChainState"]
        fn consensus_commit_final_chain_state(
            self: &ExternalEvmPort,
            request: &HostFinalChainStateCommitRequest,
        ) -> Result<HostFinalChainStateCommitReport>;
        #[cxx_name = "consensusLoadPillarAnchorState"]
        fn consensus_load_pillar_anchor_state(
            self: &ExternalEvmPort,
            request: &HostPillarAnchorStateRequest,
        ) -> Result<HostPillarAnchorStateReport>;
        #[cxx_name = "consensusLoadFinalChainAccountFacts"]
        fn consensus_load_final_chain_account_facts(
            self: &ExternalEvmPort,
            request: &HostFinalChainAccountFactsRequest,
        ) -> Result<HostFinalChainAccountFactsReport>;
        #[cxx_name = "consensusEstimateDagTransactionGas"]
        fn consensus_estimate_dag_transaction_gas(
            self: &ExternalEvmPort,
            request: &HostDagGasBatch,
        ) -> Result<HostDagGasBatch>;
    }

    extern "Rust" {
        /// Runs the native consensus scheduler on the calling thread.
        ///
        /// The borrowed ports must outlive this blocking call; no CXX object is
        /// retained after the native runner returns.
        pub fn consensus_application_run(
            application: &BridgeConsensusApplication,
            process: &ConsensusProcessPort,
            signer: &ConsensusSignerPort,
            transport: &ConsensusTransportPort,
            external_evm: &ExternalEvmPort,
        ) -> Result<ConsensusRunExit>;
        /// Executes one FinalChain task through the native application root.
        pub fn consensus_application_finalize(
            application: &BridgeConsensusApplication,
            external_evm: &ExternalEvmPort,
            task: HostFinalChainFinalizeTask,
        ) -> Result<HostFinalChainFinalizeReport>;
        /// Submits one canonical transaction through a borrowed external-EVM
        /// account-fact leaf without retaining either CXX object.
        pub fn consensus_application_submit_transaction_with_execution(
            application: &BridgeConsensusApplication,
            request: PublicTransactionSubmissionRequest,
            external_evm: &ExternalEvmPort,
        ) -> Result<PublicTransactionSubmissionReport>;
        /// Routes one canonical transaction packet through native network and
        /// application owners while borrowing only the external-EVM leaf.
        pub fn consensus_network_ingest_transaction_packet(
            network: &BridgeConsensusNetworkApi,
            application: &BridgeConsensusApplication,
            request: NetworkTransactionPacketRequest,
            external_evm: &ExternalEvmPort,
        ) -> Result<NetworkTransactionPacketReport>;
        pub fn consensus_network_ingest_dag_block_packet(
            network: &BridgeConsensusNetworkApi,
            application: &BridgeConsensusApplication,
            request: NetworkDagPacketRequest,
            external_evm: &ExternalEvmPort,
        ) -> Result<NetworkDagBlockIngressReport>;
        pub fn consensus_network_ingest_dag_sync_packet(
            network: &BridgeConsensusNetworkApi,
            application: &BridgeConsensusApplication,
            request: NetworkDagPacketRequest,
            external_evm: &ExternalEvmPort,
        ) -> Result<NetworkDagSyncIngressReport>;
    }
}
