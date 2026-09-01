//! Application-owned network operations over private consensus siblings.
//!
//! [`ConsensusNetworkApi`] is the only native capability projected to the
//! external tarcap adapter. It retains the exact PBFT, FinalChain,
//! DAG/transaction, query, and network-service owners created by one
//! [`crate::ConsensusApplication`], but exposes no sibling getter or mutable
//! object graph. Each method is operation-shaped: canonical inputs enter once,
//! native siblings are consulted without holding the network queue lock, and
//! only typed decisions, effects, or executor continuations are returned.

use std::sync::Arc;

use anyhow::Result;
use ethereum_types::H256;

use crate::consensus_application::{
    ConsensusIngressService, DagBlockIngressRequest, DagSyncIngressRequest,
    TransactionPacketIngressRequest,
};
use crate::consensus_application_runtime::ConsensusExecutionPort;
use crate::consensus_query_api::ConsensusQueryApi;
use crate::dag_transaction_service::DagTransactionService;
use crate::final_chain::FinalChain;
use crate::network_api::{
    ConsensusNetworkService, NETWORK_EGRESS_FAMILY_DAG_BLOCK,
    NETWORK_EGRESS_FAMILY_TRANSACTION_GOSSIP, NetworkConsensusPacketRequest,
    NetworkDagBlockIngressContext, NetworkDagBlockIngressReport, NetworkDagSyncIngressReport,
    NetworkEffectAck, NetworkEffectBatch, NetworkEffectResult, NetworkEgressPlanRequest,
    NetworkEgressPreparation, NetworkEgressPrepareRequest, NetworkGetDagSyncContext,
    NetworkGetPbftSyncRequest, NetworkGetPillarVotesBundlePacketRequest, NetworkIngressDecision,
    NetworkPbftNextVotesBundlePacketRequest, NetworkPbftSyncCommandOutcome,
    NetworkPbftSyncCommandRequest, NetworkPbftSyncStartOutcome, NetworkPbftSyncStartRequest,
    NetworkPbftVotePacketReport, NetworkPendingDagBlocksRequestFacts,
    NetworkPillarVotePacketReport, NetworkStatusPacketBuildOutcome,
    NetworkStatusPacketBuildRequest, NetworkStatusPacketReport, NetworkStatusPacketRequest,
    NetworkTransactionPacketContext, NetworkTransactionPacketReport,
};
use crate::pbft_service::{PbftService, PbftSyncIngressStep};
use crate::{
    PublicTransactionSubmissionReport, PublicTransactionSubmissionRequest,
    SlashingSubmitterIdentity, TransactionGossipAccount, TransactionGossipEntry,
};

/// Clone-free native client bound to one complete application composition.
///
/// Construction is crate-private and performed only by
/// [`crate::ConsensusApplication`]. The held `Arc`s extend existing sibling
/// lifetimes; they cannot construct an independent topology. No accessor
/// returns them, so bridge code can invoke only the exact operations below.
pub struct ConsensusNetworkApi {
    network: ConsensusNetworkService,
    pbft: Arc<PbftService>,
    final_chain: Arc<FinalChain>,
    dag_transaction: Arc<DagTransactionService>,
    ingress: Arc<ConsensusIngressService>,
    query: ConsensusQueryApi,
}

impl ConsensusNetworkApi {
    pub(crate) fn new(
        network: ConsensusNetworkService,
        pbft: Arc<PbftService>,
        final_chain: Arc<FinalChain>,
        dag_transaction: Arc<DagTransactionService>,
        ingress: Arc<ConsensusIngressService>,
        query: ConsensusQueryApi,
    ) -> Self {
        Self {
            network,
            pbft,
            final_chain,
            dag_transaction,
            ingress,
            query,
        }
    }

    /// Drains dependency-ready physical effects from one lane, optionally
    /// restricted to a source payload. Unrelated same-lane work stays queued.
    pub fn drain_work(
        &self,
        transport_lane: u32,
        source_payload_id: u64,
        source_scoped: bool,
        budget: u32,
    ) -> Result<NetworkEffectBatch> {
        if source_scoped {
            self.network
                .drain_work_for_source(transport_lane, source_payload_id, budget)
        } else {
            self.network.drain_work(transport_lane, budget)
        }
    }

    /// Validates exact effect identities and records physical executor results.
    pub fn report_effect_results(
        &self,
        results: Vec<NetworkEffectResult>,
    ) -> Result<NetworkEffectAck> {
        self.network.report_effect_results(results)
    }

    /// Selects a peer and starts one generation-correlated PBFT sync session.
    pub fn begin_pbft_sync(
        &self,
        request: NetworkPbftSyncStartRequest,
    ) -> Result<NetworkPbftSyncStartOutcome> {
        self.network.begin_pbft_sync(request)
    }

    /// Decodes and validates one canonical status packet and returns only
    /// peer-bookkeeping facts plus exact native follow-up decisions.
    pub fn ingest_status_packet(
        &self,
        request: NetworkStatusPacketRequest,
    ) -> Result<NetworkStatusPacketReport> {
        self.network.ingest_status_packet(request)
    }

    /// Builds one canonical initial or periodic status packet from native
    /// bootstrap identity and coherent sync state.
    pub fn build_status_packet(
        &self,
        request: NetworkStatusPacketBuildRequest,
    ) -> Result<NetworkStatusPacketBuildOutcome> {
        self.network.build_status_packet(request)
    }

    /// Applies one generation-correlated PBFT-sync lifecycle command.
    pub fn apply_pbft_sync_command(
        &self,
        request: NetworkPbftSyncCommandRequest,
    ) -> Result<NetworkPbftSyncCommandOutcome> {
        self.network.apply_pbft_sync_command(request)
    }

    /// Selects a pending-DAG peer and queues canonical non-finalized hashes
    /// from the application query snapshot.
    pub fn request_pending_dag_blocks(
        &self,
        transport_lane: u32,
        source_payload_id: u64,
        facts: NetworkPendingDagBlocksRequestFacts,
    ) -> Result<NetworkIngressDecision> {
        let hashes = self
            .query
            .dag_live_non_finalized_index()?
            .levels
            .into_iter()
            .flat_map(|level| level.hashes)
            .collect();
        self.network
            .request_pending_dag_blocks(transport_lane, source_payload_id, facts, hashes)
    }

    /// Decodes and authoritatively admits one canonical PBFT vote packet.
    pub fn ingest_pbft_vote_packet(
        &self,
        request: NetworkConsensusPacketRequest,
        slashing_submitters: &[SlashingSubmitterIdentity],
    ) -> Result<NetworkPbftVotePacketReport> {
        self.network.ingest_pbft_vote_packet(
            self.pbft.as_ref(),
            self.final_chain.as_ref(),
            request,
            slashing_submitters,
        )
    }

    /// Decodes, preflights, and sequentially admits one optimized PBFT vote bundle.
    pub fn ingest_pbft_votes_bundle_packet(
        &self,
        request: NetworkConsensusPacketRequest,
        slashing_submitters: &[SlashingSubmitterIdentity],
    ) -> Result<NetworkPbftVotePacketReport> {
        self.network.ingest_pbft_votes_bundle_packet(
            self.pbft.as_ref(),
            self.final_chain.as_ref(),
            request,
            slashing_submitters,
        )
    }

    /// Decodes and admits one canonical pillar vote through the root siblings.
    pub fn ingest_pillar_vote_packet(
        &self,
        request: NetworkConsensusPacketRequest,
    ) -> Result<NetworkPillarVotePacketReport> {
        self.network.ingest_pillar_vote_packet(
            self.pbft.as_ref(),
            self.final_chain.as_ref(),
            request,
        )
    }

    /// Decodes and admits one optimized pillar-vote bundle through root siblings.
    pub fn ingest_pillar_votes_bundle_packet(
        &self,
        request: NetworkConsensusPacketRequest,
    ) -> Result<NetworkPillarVotePacketReport> {
        self.network.ingest_pillar_votes_bundle_packet(
            self.pbft.as_ref(),
            self.final_chain.as_ref(),
            request,
        )
    }

    /// Serves one canonical get-next-votes request and queues exact-target effects.
    pub fn ingest_pbft_next_votes_bundle_request(
        &self,
        request: NetworkPbftNextVotesBundlePacketRequest,
    ) -> Result<NetworkIngressDecision> {
        self.network
            .ingest_pbft_next_votes_bundle_packet_request(request)
    }

    /// Serves one canonical pillar-vote bundle request from native state.
    pub fn ingest_pillar_votes_bundle_request(
        &self,
        request: NetworkGetPillarVotesBundlePacketRequest,
    ) -> Result<NetworkIngressDecision> {
        self.network.ingest_get_pillar_votes_bundle_request(request)
    }

    /// Serves one canonical get-PBFT-sync request from native snapshots.
    pub fn ingest_get_pbft_sync_request(
        &self,
        request: NetworkGetPbftSyncRequest,
    ) -> Result<NetworkIngressDecision> {
        self.network.ingest_get_pbft_sync_request(request)
    }

    /// Admits one proposed-block bundle using the exact application FinalChain.
    pub fn ingest_pbft_blocks_bundle(
        &self,
        packet_rlp: &[u8],
        source_payload_id: u64,
    ) -> Result<NetworkIngressDecision> {
        self.network.ingest_pbft_blocks_bundle(
            self.final_chain.as_ref(),
            packet_rlp,
            source_payload_id,
        )
    }

    /// Serves one canonical DAG-sync request from the application DAG snapshot.
    pub fn ingest_get_dag_sync_request(
        &self,
        context: NetworkGetDagSyncContext,
        request_rlp: &[u8],
    ) -> Result<NetworkIngressDecision> {
        self.network
            .ingest_get_dag_sync_request(context, request_rlp, |hashes| {
                self.dag_transaction.dag_non_finalized_sync(hashes)
            })
    }

    /// Decodes a transaction packet, admits each canonical member through the
    /// application ingress owner, and queues exact native network effects.
    pub fn ingest_transaction_packet(
        &self,
        context: NetworkTransactionPacketContext,
        packet_rlp: &[u8],
        policy: PublicTransactionSubmissionRequest,
    ) -> Result<NetworkTransactionPacketReport> {
        let peer_id = context.peer_id;
        self.network
            .ingest_transaction_packet(context, packet_rlp, |transaction_rlp| {
                let mut submission = policy.clone();
                submission.transaction_rlp = transaction_rlp;
                self.ingress
                    .ingest_transaction_packet(TransactionPacketIngressRequest {
                        submission,
                        peer_id,
                    })
            })
    }

    /// Decodes and authoritatively admits one canonical DAG-block packet.
    pub fn ingest_dag_block_packet<E: ConsensusExecutionPort>(
        &self,
        context: NetworkDagBlockIngressContext,
        packet_rlp: &[u8],
        execution: &E,
    ) -> Result<NetworkDagBlockIngressReport> {
        self.network
            .ingest_dag_block_packet(context, packet_rlp, |block_rlp, transaction_rlps| {
                self.ingress.ingest_dag_block_packet(
                    DagBlockIngressRequest {
                        block_rlp,
                        transaction_rlps,
                        proposed: false,
                    },
                    execution,
                )
            })
    }

    /// Decodes and admits one DAG-sync bundle with sequential partial commits.
    pub fn ingest_dag_sync_packet<E: ConsensusExecutionPort>(
        &self,
        context: NetworkDagBlockIngressContext,
        packet_rlp: &[u8],
        policy: PublicTransactionSubmissionRequest,
        execution: &E,
    ) -> Result<NetworkDagSyncIngressReport> {
        let peer_id = context.peer_id;
        self.network
            .ingest_dag_sync_packet(context, packet_rlp, |transactions, blocks| {
                self.ingress.ingest_dag_sync_packet(
                    DagSyncIngressRequest {
                        transactions: transactions
                            .into_iter()
                            .map(|transaction_rlp| TransactionPacketIngressRequest {
                                submission: PublicTransactionSubmissionRequest {
                                    transaction_rlp,
                                    ..policy.clone()
                                },
                                peer_id,
                            })
                            .collect(),
                        blocks: blocks
                            .into_iter()
                            .map(|block_rlp| DagBlockIngressRequest {
                                block_rlp,
                                transaction_rlps: Vec::new(),
                                proposed: false,
                            })
                            .collect(),
                    },
                    execution,
                )
            })
    }

    /// Returns the native adaptive gas bid used by a host-signed transaction.
    pub fn transaction_gas_price_bid(&self) -> Result<[u8; 32]> {
        self.dag_transaction.transaction_gas_price_bid()
    }

    /// Submits a host-signed canonical transaction through the same native
    /// ingress owner used by network packets.
    pub fn submit_transaction_from_native_state(
        &self,
        request: PublicTransactionSubmissionRequest,
    ) -> Result<PublicTransactionSubmissionReport> {
        self.ingress
            .submit_public_transaction_from_native_state(request)
    }

    /// Resolves native DAG/transaction inputs and publishes one bounded egress
    /// preparation. Invalid canonical inputs fail before a token is retained.
    pub fn prepare_egress(
        &self,
        request: NetworkEgressPrepareRequest,
    ) -> Result<NetworkEgressPreparation> {
        let transaction_accounts: Vec<TransactionGossipAccount> = match request.family {
            NETWORK_EGRESS_FAMILY_TRANSACTION_GOSSIP if request.payload_bytes.is_empty() => {
                self.dag_transaction.transaction_gossip_snapshot(5500)?
            }
            _ => Vec::new(),
        };
        let dag_transactions: Vec<TransactionGossipEntry> = match request.family {
            NETWORK_EGRESS_FAMILY_DAG_BLOCK => self.dag_transaction.dag_block_egress_transactions(
                H256::from(request.object_hash),
                &request.payload_bytes,
            )?,
            _ => Vec::new(),
        };
        self.network
            .prepare_egress(request, transaction_accounts, dag_transactions)
    }

    /// Consumes one prepared token with an immutable authenticated peer snapshot.
    pub fn plan_egress(&self, request: NetworkEgressPlanRequest) -> Result<NetworkIngressDecision> {
        self.network.plan_egress(request)
    }

    /// Cancels an undrained egress preparation; stale tokens are harmless.
    pub fn cancel_egress(&self, token: u64) -> Result<bool> {
        self.network.cancel_egress(token)
    }

    /// Starts one PBFT-sync ingress session over canonical packet bytes.
    pub fn begin_pbft_sync_ingress(
        &self,
        packet_rlp: &[u8],
        source_payload_id: u64,
        source_peer_id: [u8; 64],
        slashing_submitters: Vec<SlashingSubmitterIdentity>,
    ) -> Result<PbftSyncIngressStep> {
        self.pbft.begin_pbft_sync_ingress(
            self.final_chain.as_ref(),
            packet_rlp,
            source_payload_id,
            source_peer_id,
            slashing_submitters,
        )
    }

    /// Reports one sync-ingress slashing insertion and resumes the same session.
    pub fn report_pbft_sync_ingress_slashing(
        &self,
        proof_hash: H256,
        transaction_inserted: bool,
    ) -> Result<PbftSyncIngressStep> {
        self.pbft.report_pbft_sync_ingress_slashing(
            self.final_chain.as_ref(),
            proof_hash,
            transaction_inserted,
        )
    }

    /// Reports one network vote slashing insertion against its exact proof hash.
    pub fn report_verified_vote_slashing_transaction_submission(
        &self,
        proof_hash: H256,
        transaction_inserted: bool,
    ) -> Result<bool> {
        Ok(self
            .pbft
            .report_verified_vote_slashing_transaction_submission(proof_hash, transaction_inserted)?
            .submitted)
    }
}
