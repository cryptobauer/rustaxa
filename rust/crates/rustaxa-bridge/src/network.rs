//! CXX conversion and lock-error mapping for the Rust-owned network/tarcap API; native consensus owns operation-specific
//! routing, lane-local queue ordering, and effect-result validation while callers execute typed effects.

use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeApp;
use crate::ffi::BridgeConsensusNetworkApi;
use crate::network_slashing::{
    empty_slashing_transaction_effect, slashing_submitter_identity_to_domain,
    slashing_transaction_effect_to_ffi,
};
use rustaxa_consensus::pbft_vote_ingress::{PbftVoteIngressContext, PbftVoteIngressFact};
use rustaxa_consensus::pbft_vote_progress::PbftVoteProgressIntent;
use rustaxa_consensus::verified_votes::PbftVoteType;

fn vote_ingress_fact_to_domain(
    value: rustaxa_ffi::PbftVoteIngressFact,
) -> anyhow::Result<PbftVoteIngressFact> {
    Ok(PbftVoteIngressFact {
        period: value.period,
        round: value.round,
        step: value.step,
        vote_type: PbftVoteType::try_from(value.vote_type)?,
    })
}

const fn vote_ingress_context_to_domain(
    value: rustaxa_ffi::PbftVoteIngressContext,
) -> PbftVoteIngressContext {
    PbftVoteIngressContext {
        current_period: value.current_period,
        current_round: value.current_round,
        current_step: value.current_step,
        max_future_period_delta: value.max_future_period_delta,
        max_future_round_delta: value.max_future_round_delta,
        max_future_step_delta: value.max_future_step_delta,
        validate_max_round_step: value.validate_max_round_step,
        source_peer_is_voter: value.source_peer_is_voter,
        can_request_pbft_sync: value.can_request_pbft_sync,
        can_request_next_votes_sync: value.can_request_next_votes_sync,
    }
}

/// Creates a thin network/tarcap adapter over the application PBFT service.
///
/// The returned handle clones the one native network service restored by the
/// root, so every C++ wrapper observes the same effect IDs, dependency state,
/// and protocol siblings. Peer transport and packet framing remain external.
pub fn create_consensus_network_api(
    service: &crate::ffi::BridgeApp,
) -> Box<BridgeConsensusNetworkApi> {
    Box::new(BridgeConsensusNetworkApi {
        network: service.0.network_service(),
        pbft: service.0.pbft_arc_for_bridge(),
        final_chain: service.0.final_chain_arc_for_bridge(),
    })
}

/// Returns the bounded ordered candidate identities used for peer-known tests.
pub fn consensus_network_transaction_gossip_candidate_hashes(
    application: &BridgeApp,
) -> anyhow::Result<Vec<rustaxa_ffi::DagHash>> {
    Ok(application
        .0
        .prepare_transaction_gossip(5500)?
        .into_iter()
        .flat_map(|account| account.transactions)
        .map(|transaction| rustaxa_ffi::DagHash {
            hash: transaction.hash.into(),
        })
        .collect())
}

impl BridgeConsensusNetworkApi {
    /// Drains lane work, optionally scoped to an exact source payload id.
    pub fn consensus_network_drain_work(
        &self,
        transport_lane: u32,
        source_payload_id: u64,
        source_scoped: bool,
        budget: u32,
    ) -> anyhow::Result<rustaxa_ffi::NetworkEffectBatch> {
        let batch = if source_scoped {
            self.network
                .drain_work_for_source(transport_lane, source_payload_id, budget)?
        } else {
            self.network.drain_work(transport_lane, budget)?
        };
        Ok(rustaxa_ffi::NetworkEffectBatch {
            status: batch.status,
            effects: batch
                .effects
                .into_iter()
                .map(to_bridge_network_effect)
                .collect(),
            more_available: batch.more_available,
            error_code: batch.error_code,
        })
    }

    /// Records network executor result reports.
    ///
    /// Results are validated against active Rust-owned effects before cursors advance.
    pub fn consensus_network_report_effect_results(
        &self,
        results: Vec<rustaxa_ffi::NetworkEffectResult>,
    ) -> anyhow::Result<rustaxa_ffi::NetworkEffectAck> {
        let ack = self
            .network
            .report_effect_results(results.into_iter().map(from_bridge_effect_result).collect())?;
        Ok(rustaxa_ffi::NetworkEffectAck {
            status: ack.status,
            accepted_results: ack.accepted_results,
            failed_results: ack.failed_results,
            error_code: ack.error_code,
        })
    }

    /// Plans deterministic sync follow-up for an accepted status packet.
    pub fn consensus_network_plan_status_sync(
        &self,
        facts: rustaxa_ffi::NetworkStatusSyncFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkStatusSyncPlan> {
        Ok(to_bridge_network_status_sync_plan(
            self.network
                .plan_status_sync(to_domain_network_status_sync_facts(facts))?,
        ))
    }

    /// Plans local status packet egress through the network/tarcap API.
    pub fn consensus_network_plan_status_egress(
        &self,
        facts: rustaxa_ffi::NetworkStatusEgressFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkStatusEgressPlan> {
        Ok(to_bridge_network_status_egress_plan(
            self.network
                .plan_status_egress(to_domain_network_status_egress_facts(facts))?,
        ))
    }

    /// Plans whether an initial status packet should be accepted.
    pub fn consensus_network_plan_initial_status(
        &self,
        facts: rustaxa_ffi::NetworkInitialStatusFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkInitialStatusPlan> {
        Ok(to_bridge_network_initial_status_plan(
            self.network
                .plan_initial_status(to_domain_network_initial_status_facts(facts))?,
        ))
    }

    /// Plans whether PBFT sync should start and which peer should serve it.
    pub fn consensus_network_plan_pbft_sync_start(
        &self,
        facts: rustaxa_ffi::NetworkPbftSyncStartFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPbftSyncStartPlan> {
        Ok(to_bridge_network_pbft_sync_start_plan(
            self.network
                .plan_pbft_sync_start(to_domain_network_pbft_sync_start_facts(facts))?,
        ))
    }

    /// Selects the best max-chain peer from compact network-owned peer facts.
    pub fn consensus_network_plan_max_chain_peer_selection(
        &self,
        facts: rustaxa_ffi::NetworkPeerSelectionFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPeerSelectionPlan> {
        Ok(to_bridge_network_peer_selection_plan(
            self.network
                .plan_max_chain_peer_selection(to_domain_network_peer_selection_facts(facts))?,
        ))
    }

    /// Plans whether pending DAG blocks should be requested and from which peer.
    pub fn consensus_network_plan_pending_dag_blocks_request(
        &self,
        facts: rustaxa_ffi::NetworkPendingDagBlocksRequestFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPendingDagBlocksRequestPlan> {
        Ok(to_bridge_network_pending_dag_blocks_request_plan(
            self.network.plan_pending_dag_blocks_request(
                to_domain_network_pending_dag_blocks_request_facts(facts),
            )?,
        ))
    }

    /// Selects the pending-DAG peer and queues canonical non-finalized hashes.
    pub fn consensus_network_request_pending_dag_blocks(
        &self,
        application: &BridgeApp,
        transport_lane: u32,
        source_payload_id: u64,
        facts: rustaxa_ffi::NetworkPendingDagBlocksRequestFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let hashes = application
            .0
            .consensus_query_api_for_bridge()
            .dag_live_non_finalized_index()?
            .levels
            .into_iter()
            .flat_map(|level| level.hashes)
            .collect();
        Ok(to_bridge_network_ingress_decision(
            self.network.request_pending_dag_blocks(
                transport_lane,
                source_payload_id,
                to_domain_network_pending_dag_blocks_request_facts(facts),
                hashes,
            )?,
        ))
    }

    /// Routes single-vote PBFT ingress and queues network effects.
    ///
    /// Unlike the side-effect-free planner method, this production-facing route
    /// updates the network facade's effect queue so C++ can execute sync,
    /// report, and disconnect requests through `drain_work` /
    /// `report_effect_results`.
    pub fn consensus_network_admit_pbft_vote(
        &self,
        fact: rustaxa_ffi::PbftVoteIngressFact,
        context: rustaxa_ffi::NetworkPbftVoteIngressContext,
        slashing_submitters: Vec<rustaxa_ffi::SlashingSubmitterIdentity>,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPbftVoteAdmissionOutcome> {
        let outcome = self.network.ingest_and_admit_pbft_vote(
            self.pbft.as_ref(),
            self.final_chain.as_ref(),
            vote_ingress_fact_to_domain(fact)?,
            to_domain_pbft_vote_ingress_context(context),
            &slashing_submitters
                .into_iter()
                .map(slashing_submitter_identity_to_domain)
                .collect::<Vec<_>>(),
        )?;
        Ok(to_bridge_pbft_vote_admission_outcome(outcome))
    }

    /// Preflights one complete vote bundle and queues its grouped admission effects.
    pub fn consensus_network_admit_pbft_vote_bundle(
        &self,
        reference: rustaxa_ffi::PbftVoteIngressFact,
        votes: Vec<rustaxa_ffi::PbftVoteIngressFact>,
        contexts: Vec<rustaxa_ffi::NetworkPbftVoteIngressContext>,
        slashing_submitters: Vec<rustaxa_ffi::SlashingSubmitterIdentity>,
    ) -> anyhow::Result<Vec<rustaxa_ffi::NetworkPbftVoteAdmissionOutcome>> {
        let votes = votes
            .into_iter()
            .map(vote_ingress_fact_to_domain)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(self
            .network
            .ingest_and_admit_pbft_vote_bundle(
                self.pbft.as_ref(),
                self.final_chain.as_ref(),
                vote_ingress_fact_to_domain(reference)?,
                votes,
                contexts
                    .into_iter()
                    .map(to_domain_pbft_vote_ingress_context)
                    .collect(),
                &slashing_submitters
                    .into_iter()
                    .map(slashing_submitter_identity_to_domain)
                    .collect::<Vec<_>>(),
            )?
            .into_iter()
            .map(to_bridge_pbft_vote_admission_outcome)
            .collect())
    }

    /// Converts one ordered canonical pillar-vote packet for native atomic preflight and exact-id effect queueing.
    /// A poisoned shared network root is returned as a bridge error without queueing partial work.
    pub fn consensus_network_ingest_pillar_vote_bundle(
        &self,
        context: rustaxa_ffi::NetworkPillarVoteIngressContext,
        votes: Vec<rustaxa_ffi::PillarVoteRlpPayload>,
    ) -> anyhow::Result<Vec<rustaxa_ffi::NetworkIngressDecision>> {
        Ok(self
            .network
            .ingest_pillar_vote_bundle(
                to_domain_pillar_vote_ingress_context(context),
                votes.into_iter().map(|value| value.vote_rlp).collect(),
            )?
            .into_iter()
            .map(to_bridge_network_ingress_decision)
            .collect())
    }

    /// Routes one get-next-votes request and queues its native egress leaf.
    ///
    /// Peer request facts are passed directly. The native network service reads
    /// its sibling manager snapshot before verified-vote lookup, then owns
    /// eligibility, previous-round selection, validation, chunking, and sends.
    pub fn consensus_network_ingest_pbft_next_votes_bundle_request(
        &self,
        transport_lane: u32,
        peer_id: [u8; 64],
        peer_period: u64,
        peer_round: u64,
        source_payload_id: u64,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        Ok(to_bridge_network_ingress_decision(
            self.network.ingest_pbft_next_votes_bundle_request(
                rustaxa_consensus::NetworkPbftNextVotesBundleRequest {
                    transport_lane,
                    peer_id,
                    peer_period,
                    peer_round,
                    source_payload_id,
                },
            )?,
        ))
    }

    /// Routes one pillar-vote bundle request through the native PBFT application root.
    pub fn consensus_network_ingest_pillar_votes_bundle_request(
        &self,
        transport_lane: u32,
        peer_id: [u8; 64],
        period: u64,
        pillar_block_hash: [u8; 32],
        source_payload_id: u64,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        Ok(to_bridge_network_ingress_decision(
            self.network.ingest_get_pillar_votes_bundle_request(
                rustaxa_consensus::NetworkGetPillarVotesBundleRequest {
                    transport_lane,
                    peer_id,
                    period,
                    pillar_block_hash,
                    source_payload_id,
                },
            )?,
        ))
    }

    /// Routes canonical get-PBFT-sync bytes through native range validation,
    /// snapshotting, storage reads, packet encoding, and ordered effect queueing.
    pub fn consensus_network_ingest_get_pbft_sync_request(
        &self,
        request: rustaxa_ffi::NetworkGetPbftSyncRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        Ok(to_bridge_network_ingress_decision(
            self.network.ingest_get_pbft_sync_request(
                rustaxa_consensus::NetworkGetPbftSyncRequest {
                    tarcap_version: request.tarcap_version,
                    peer_id: request.peer_id,
                    request_rlp: request.request_rlp,
                    source_payload_id: request.source_payload_id,
                },
            )?,
        ))
    }

    /// Admits one latest-tarcap proposed-block bundle through native consensus.
    ///
    /// C++ supplies only canonical packet bytes and the retained FinalChain
    /// leaf handle. Native consensus owns decoding, relevance, author
    /// uniqueness, DPoS queries, and storage-first proposal publication.
    pub fn consensus_network_ingest_pbft_blocks_bundle(
        &self,
        runtime: &BridgeApp,
        packet_rlp: Vec<u8>,
        source_payload_id: u64,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        Ok(to_bridge_network_ingress_decision(
            self.network.ingest_pbft_blocks_bundle(
                runtime.0.final_chain_for_bridge(),
                &packet_rlp,
                source_payload_id,
            )?,
        ))
    }

    /// Serves one canonical get-DAG-sync request from application-owned bytes.
    pub fn consensus_network_ingest_get_dag_sync_request(
        &self,
        application: &BridgeApp,
        request: rustaxa_ffi::NetworkGetDagSyncRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        Ok(to_bridge_network_ingress_decision(
            self.network.ingest_get_dag_sync_request(
                rustaxa_consensus::NetworkGetDagSyncContext {
                    transport_lane: request.transport_lane,
                    peer_id: request.peer_id,
                    source_payload_id: request.source_payload_id,
                    request_allowed: request.request_allowed,
                },
                &request.request_rlp,
                |hashes| application.0.prepare_dag_sync_egress(hashes),
            )?,
        ))
    }

    /// Plans exact per-peer transaction packets from a bounded native snapshot.
    pub fn consensus_network_plan_transaction_gossip(
        &self,
        application: &BridgeApp,
        request: rustaxa_ffi::NetworkTransactionGossipRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let snapshot = application.0.prepare_transaction_gossip(5500)?;
        Ok(to_bridge_network_ingress_decision(
            self.network.plan_transaction_gossip(
                rustaxa_consensus::NetworkTransactionGossipRequest {
                    transport_lane: request.transport_lane,
                    source_payload_id: request.source_payload_id,
                    peers: request
                        .peers
                        .into_iter()
                        .map(|peer| rustaxa_consensus::NetworkTransactionGossipPeer {
                            peer_id: peer.peer_id,
                            known_hashes: peer
                                .known_hashes
                                .into_iter()
                                .map(|hash| hash.hash)
                                .collect(),
                        })
                        .collect(),
                },
                snapshot,
            )?,
        ))
    }

    /// Plans exact DAG fanout after application admission has committed.
    pub fn consensus_network_plan_dag_block_gossip(
        &self,
        request: rustaxa_ffi::NetworkDagGossipRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        Ok(to_bridge_network_ingress_decision(
            self.network
                .plan_dag_block_gossip(rustaxa_consensus::NetworkDagGossipRequest {
                    transport_lane: request.transport_lane,
                    source_payload_id: request.source_payload_id,
                    source_peer_id: request.source_peer_id,
                    block_hash: request.block_hash,
                    packet_rlp: request.packet_rlp,
                    peers: request
                        .peers
                        .into_iter()
                        .map(|peer| rustaxa_consensus::NetworkDagGossipPeer {
                            peer_id: peer.peer_id,
                            syncing: peer.syncing,
                            known_block: peer.known_block,
                        })
                        .collect(),
                })?,
        ))
    }
}

fn to_bridge_network_effect(
    effect: rustaxa_consensus::NetworkEffect,
) -> rustaxa_ffi::NetworkEffect {
    rustaxa_ffi::NetworkEffect {
        effect_id: effect.effect_id,
        source_payload_id: effect.source_payload_id,
        transport_lane: effect.transport_lane,
        kind: effect.kind,
        peer_id: effect.peer_id,
        packet_kind: effect.packet_kind,
        payload_bytes: effect.payload_bytes,
        related_payload_bytes: effect.related_payload_bytes,
        exclude_peers: effect
            .exclude_peers
            .into_iter()
            .map(|id| rustaxa_ffi::NetworkPeerId { id })
            .collect(),
        object_kind: effect.object_kind,
        object_hash: effect.object_hash,
        sync_kind: effect.sync_kind,
        sync_start: effect.sync_start,
        reason_code: effect.reason_code,
        dependency_id: effect.dependency_id,
        period: effect.period,
        round: effect.round,
    }
}

fn to_domain_pbft_vote_ingress_context(
    value: rustaxa_ffi::NetworkPbftVoteIngressContext,
) -> rustaxa_consensus::NetworkPbftVoteIngressContext {
    rustaxa_consensus::NetworkPbftVoteIngressContext {
        ingress: vote_ingress_context_to_domain(value.ingress),
        transport_lane: value.transport_lane,
        peer_id: value.peer_id,
        peer_pbft_chain_size: value.peer_pbft_chain_size,
        source_payload_id: value.source_payload_id,
        enqueue_admission: value.enqueue_admission,
        allow_gossip: value.allow_gossip,
        vote_hash: value.vote_hash,
        vote_rlp: value.vote_rlp,
        pbft_block_rlp: value.pbft_block_rlp,
        pbft_block_hash: value.pbft_block_hash,
        pbft_block_period: value.pbft_block_period,
    }
}

fn to_domain_pillar_vote_ingress_context(
    value: rustaxa_ffi::NetworkPillarVoteIngressContext,
) -> rustaxa_consensus::NetworkPillarVoteIngressContext {
    rustaxa_consensus::NetworkPillarVoteIngressContext {
        transport_lane: value.transport_lane,
        peer_id: value.peer_id,
        source_payload_id: value.source_payload_id,
        ficus_activation_period: value.ficus_activation_period,
        allow_gossip: value.allow_gossip,
    }
}

fn to_domain_network_status_sync_facts(
    value: rustaxa_ffi::NetworkStatusSyncFacts,
) -> rustaxa_consensus::NetworkStatusSyncFacts {
    rustaxa_consensus::NetworkStatusSyncFacts {
        local_pbft_syncing: value.local_pbft_syncing,
        local_pbft_synced_period: value.local_pbft_synced_period,
        local_pbft_period: value.local_pbft_period,
        local_pbft_round: value.local_pbft_round,
        peer_pbft_chain_size: value.peer_pbft_chain_size,
        peer_pbft_period: value.peer_pbft_period,
        peer_pbft_round: value.peer_pbft_round,
        peer_dag_synced: value.peer_dag_synced,
        peer_last_status_pbft_chain_size: value.peer_last_status_pbft_chain_size,
    }
}

fn to_bridge_network_status_sync_plan(
    plan: rustaxa_consensus::NetworkStatusSyncPlan,
) -> rustaxa_ffi::NetworkStatusSyncPlan {
    rustaxa_ffi::NetworkStatusSyncPlan {
        request_pbft_sync: plan.request_pbft_sync,
        request_pending_dag_blocks: plan.request_pending_dag_blocks,
        request_next_votes: plan.request_next_votes,
        next_votes_period: plan.next_votes_period,
        next_votes_round: plan.next_votes_round,
    }
}

fn to_domain_network_status_egress_facts(
    value: rustaxa_ffi::NetworkStatusEgressFacts,
) -> rustaxa_consensus::NetworkStatusEgressFacts {
    rustaxa_consensus::NetworkStatusEgressFacts {
        initial: value.initial,
        local_chain_id: value.local_chain_id,
        genesis_hash: value.genesis_hash,
        node_major_version: value.node_major_version,
        node_minor_version: value.node_minor_version,
        node_patch_version: value.node_patch_version,
        is_light_node: value.is_light_node,
        light_node_history: value.light_node_history,
        local_pbft_chain_size: value.local_pbft_chain_size,
        local_pbft_round: value.local_pbft_round,
        local_dag_level: value.local_dag_level,
        pbft_syncing: value.pbft_syncing,
        deep_pbft_syncing: value.deep_pbft_syncing,
    }
}

fn to_bridge_network_status_egress_plan(
    plan: rustaxa_consensus::NetworkStatusEgressPlan,
) -> rustaxa_ffi::NetworkStatusEgressPlan {
    rustaxa_ffi::NetworkStatusEgressPlan {
        status: plan.status,
        error_code: plan.error_code,
        peer_pbft_chain_size: plan.peer_pbft_chain_size,
        peer_pbft_round: plan.peer_pbft_round,
        peer_dag_level: plan.peer_dag_level,
        peer_syncing: plan.peer_syncing,
        include_initial_data: plan.include_initial_data,
        chain_id: plan.chain_id,
        genesis_hash: plan.genesis_hash,
        node_major_version: plan.node_major_version,
        node_minor_version: plan.node_minor_version,
        node_patch_version: plan.node_patch_version,
        is_light_node: plan.is_light_node,
        light_node_history: plan.light_node_history,
    }
}

fn to_domain_network_initial_status_facts(
    value: rustaxa_ffi::NetworkInitialStatusFacts,
) -> rustaxa_consensus::NetworkInitialStatusFacts {
    rustaxa_consensus::NetworkInitialStatusFacts {
        local_chain_id: value.local_chain_id,
        peer_chain_id: value.peer_chain_id,
        expected_genesis_hash: value.expected_genesis_hash,
        peer_genesis_hash: value.peer_genesis_hash,
        local_pbft_synced_period: value.local_pbft_synced_period,
        peer_pbft_chain_size: value.peer_pbft_chain_size,
        peer_is_light_node: value.peer_is_light_node,
        peer_light_node_history: value.peer_light_node_history,
    }
}

fn to_bridge_network_initial_status_plan(
    plan: rustaxa_consensus::NetworkInitialStatusPlan,
) -> rustaxa_ffi::NetworkInitialStatusPlan {
    rustaxa_ffi::NetworkInitialStatusPlan {
        status: plan.status,
        error_code: plan.error_code,
        accept_peer: plan.accept_peer,
        disconnect_peer: plan.disconnect_peer,
    }
}

fn to_domain_network_pbft_sync_start_facts(
    value: rustaxa_ffi::NetworkPbftSyncStartFacts,
) -> rustaxa_consensus::NetworkPbftSyncStartFacts {
    rustaxa_consensus::NetworkPbftSyncStartFacts {
        local_pbft_syncing: value.local_pbft_syncing,
        local_pbft_synced_period: value.local_pbft_synced_period,
        local_pbft_chain_size: value.local_pbft_chain_size,
        candidates: value
            .candidates
            .into_iter()
            .map(to_domain_network_pbft_sync_peer_candidate)
            .collect(),
    }
}

fn to_domain_network_pbft_sync_peer_candidate(
    candidate: rustaxa_ffi::NetworkPbftSyncPeerCandidate,
) -> rustaxa_consensus::NetworkPbftSyncPeerCandidate {
    rustaxa_consensus::NetworkPbftSyncPeerCandidate {
        peer_id: candidate.peer_id,
        pbft_chain_size: candidate.pbft_chain_size,
        dag_level: candidate.dag_level,
        is_light_node: candidate.is_light_node,
        light_node_history: candidate.light_node_history,
        peer_dag_synced: candidate.peer_dag_synced,
        peer_dag_syncing: candidate.peer_dag_syncing,
        dag_sync_allowed: candidate.dag_sync_allowed,
    }
}

fn to_bridge_network_pbft_sync_start_plan(
    plan: rustaxa_consensus::NetworkPbftSyncStartPlan,
) -> rustaxa_ffi::NetworkPbftSyncStartPlan {
    rustaxa_ffi::NetworkPbftSyncStartPlan {
        status: plan.status,
        error_code: plan.error_code,
        start_sync: plan.start_sync,
        has_peer: plan.has_peer,
        peer_id: plan.peer_id,
        peer_pbft_chain_size: plan.peer_pbft_chain_size,
        request_period: plan.request_period,
        enable_snapshot_creation: plan.enable_snapshot_creation,
    }
}

fn to_domain_network_peer_selection_facts(
    value: rustaxa_ffi::NetworkPeerSelectionFacts,
) -> rustaxa_consensus::NetworkPeerSelectionFacts {
    rustaxa_consensus::NetworkPeerSelectionFacts {
        local_pbft_syncing_period: value.local_pbft_syncing_period,
        candidates: value
            .candidates
            .into_iter()
            .map(to_domain_network_pbft_sync_peer_candidate)
            .collect(),
    }
}

fn to_bridge_network_peer_selection_plan(
    plan: rustaxa_consensus::NetworkPeerSelectionPlan,
) -> rustaxa_ffi::NetworkPeerSelectionPlan {
    rustaxa_ffi::NetworkPeerSelectionPlan {
        status: plan.status,
        error_code: plan.error_code,
        has_peer: plan.has_peer,
        peer_id: plan.peer_id,
        peer_pbft_chain_size: plan.peer_pbft_chain_size,
    }
}

fn to_domain_network_pending_dag_blocks_request_facts(
    value: rustaxa_ffi::NetworkPendingDagBlocksRequestFacts,
) -> rustaxa_consensus::NetworkPendingDagBlocksRequestFacts {
    rustaxa_consensus::NetworkPendingDagBlocksRequestFacts {
        local_pbft_syncing_period: value.local_pbft_syncing_period,
        has_explicit_peer: value.has_explicit_peer,
        explicit_peer: to_domain_network_pbft_sync_peer_candidate(value.explicit_peer),
        candidates: value
            .candidates
            .into_iter()
            .map(to_domain_network_pbft_sync_peer_candidate)
            .collect(),
    }
}

fn to_bridge_network_pending_dag_blocks_request_plan(
    plan: rustaxa_consensus::NetworkPendingDagBlocksRequestPlan,
) -> rustaxa_ffi::NetworkPendingDagBlocksRequestPlan {
    rustaxa_ffi::NetworkPendingDagBlocksRequestPlan {
        status: plan.status,
        error_code: plan.error_code,
        request_pending_dag_blocks: plan.request_pending_dag_blocks,
        has_peer: plan.has_peer,
        peer_id: plan.peer_id,
        request_period: plan.request_period,
    }
}

pub(crate) fn to_bridge_network_ingress_decision(
    decision: rustaxa_consensus::NetworkIngressDecision,
) -> rustaxa_ffi::NetworkIngressDecision {
    rustaxa_ffi::NetworkIngressDecision {
        payload_id: decision.payload_id,
        payload_accepted: decision.payload_accepted,
        routed: decision.routed,
        status: decision.status,
        error_code: decision.error_code,
        queued_effect_count: decision.queued_effect_count,
        application_effect_id: decision.application_effect_id,
    }
}

fn to_bridge_pbft_vote_admission_outcome(
    value: rustaxa_consensus::NetworkPbftVoteAdmissionOutcome,
) -> rustaxa_ffi::NetworkPbftVoteAdmissionOutcome {
    let decision = to_bridge_network_ingress_decision(value.decision);
    let Some(admission) = value.admission else {
        return rustaxa_ffi::NetworkPbftVoteAdmissionOutcome {
            decision,
            has_admission: false,
            accepted: false,
            already_present: false,
            mark_vote_known: false,
            gossip_vote: false,
            report_slashing: false,
            has_slashing_transaction_effect: false,
            slashing_transaction_effect: empty_slashing_transaction_effect(),
        };
    };
    let transition_published = admission.transaction.transition_published;
    let add = admission.transaction.outcome.add_outcome.as_ref();
    let intents = admission
        .transaction
        .outcome
        .execution
        .as_ref()
        .map(|execution| execution.pipeline_step.progress_plan.intents.as_slice())
        .unwrap_or_default();
    let slashing_transaction_effect = admission.slashing_transaction_effect;
    rustaxa_ffi::NetworkPbftVoteAdmissionOutcome {
        decision,
        has_admission: true,
        accepted: transition_published
            && admission.validation.accepted
            && add.is_some_and(|outcome| outcome.inserted),
        already_present: transition_published
            && add.is_some_and(|outcome| outcome.duplicate_vote_hash),
        mark_vote_known: transition_published
            && intents
                .iter()
                .any(|intent| matches!(intent, PbftVoteProgressIntent::MarkKnown { .. })),
        gossip_vote: transition_published
            && intents
                .iter()
                .any(|intent| matches!(intent, PbftVoteProgressIntent::GossipVote { .. })),
        report_slashing: slashing_transaction_effect.is_some(),
        has_slashing_transaction_effect: slashing_transaction_effect.is_some(),
        slashing_transaction_effect: slashing_transaction_effect
            .map(slashing_transaction_effect_to_ffi)
            .unwrap_or_else(empty_slashing_transaction_effect),
    }
}

fn from_bridge_effect_result(
    result: rustaxa_ffi::NetworkEffectResult,
) -> rustaxa_consensus::NetworkEffectResult {
    rustaxa_consensus::NetworkEffectResult {
        effect_id: result.effect_id,
        kind: result.kind,
        peer_id: result.peer_id,
        packet_kind: result.packet_kind,
        object_kind: result.object_kind,
        object_hash: result.object_hash,
        status: result.status,
        diagnostic: result.diagnostic,
        admission_accepted: result.admission_accepted,
        admission_already_present: result.admission_already_present,
        admission_mark_vote_known: result.admission_mark_vote_known,
        admission_gossip_vote: result.admission_gossip_vote,
        admission_report_slashing: result.admission_report_slashing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_result_conversion_preserves_executor_facts() {
        let result = from_bridge_effect_result(rustaxa_ffi::NetworkEffectResult {
            effect_id: 7,
            kind: 8,
            peer_id: [3; 64],
            packet_kind: 0,
            object_kind: 7,
            object_hash: [4; 32],
            status: 0,
            diagnostic: String::new(),
            admission_accepted: true,
            admission_already_present: false,
            admission_mark_vote_known: false,
            admission_gossip_vote: false,
            admission_report_slashing: false,
        });
        assert!(result.admission_accepted);
        assert_eq!(result.effect_id, 7);
    }
}
