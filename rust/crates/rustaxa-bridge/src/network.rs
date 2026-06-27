//! CXX bridge for the external network/tarcap consensus API.
//!
//! This module exposes a direct Rust-owned `BridgeConsensusNetworkApi` facade
//! for network and tarcap callers. It is intentionally independent of consensus shim
//! classes: callers pass canonical packet bytes and receive typed executor
//! effects or acknowledgements. Packet-specific consensus planning can be added
//! behind this facade without giving network code access to consensus managers,
//! `DbStorage`, bridge batch ids, or legacy C++ sidecars.

use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeConsensusNetworkApi;
use crate::pbft_vote_ingress::{
    context_to_domain as vote_ingress_context_to_domain,
    fact_to_domain as vote_ingress_fact_to_domain, plan_to_ffi as vote_ingress_plan_to_ffi,
};
use ethereum_types::H256;
use std::sync::Mutex;

/// Creates an empty Rust-owned network/tarcap consensus API facade.
///
/// The returned handle owns ingress payload bytes and pending network effects.
/// It does not own peer transport, packet framing, gossip fanout, or network
/// scheduling; those remain external executor responsibilities.
pub fn create_consensus_network_api(
    config: rustaxa_ffi::NetworkApiConfig,
) -> Box<BridgeConsensusNetworkApi> {
    Box::new(BridgeConsensusNetworkApi {
        api: Mutex::new(rustaxa_consensus::ConsensusNetworkApi::with_config(
            to_domain_config(config),
        )),
    })
}

impl BridgeConsensusNetworkApi {
    /// Accepts canonical packet bytes into Rust-owned consensus ingress storage.
    ///
    /// A successful receipt means only that the bytes were retained for later
    /// packet-specific processing. It is not a protocol-validity or consensus
    /// admission result.
    pub fn consensus_network_ingest_packet(
        &self,
        packet: rustaxa_ffi::NetworkIngressPacket,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressReceipt> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        let receipt = api.ingest_packet(rustaxa_consensus::NetworkIngressPacket {
            packet_type: packet.packet_type,
            peer_id: packet.peer_id,
            payload_bytes: packet.payload_bytes,
            received_at_mono_ms: packet.received_at_mono_ms,
            source_packet_id: packet.source_packet_id,
        });
        Ok(rustaxa_ffi::NetworkIngressReceipt {
            accepted: receipt.accepted,
            payload_id: receipt.payload_id.0,
            status: receipt.status,
            error_code: receipt.error_code,
        })
    }

    /// Drains up to `budget` pending network effects for external execution.
    ///
    /// Effects are returned in the order produced by Rust consensus. The first
    /// API slice may return an empty batch because packet-specific pipelines
    /// are not routed behind the facade yet.
    pub fn consensus_network_drain_work(
        &self,
        budget: u32,
    ) -> anyhow::Result<rustaxa_ffi::NetworkEffectBatch> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        let batch = api.drain_work(budget);
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
    /// The report path is part of the stable external API even before concrete
    /// effect kinds are emitted. Later slices should validate reported effects
    /// against active Rust-owned sessions before advancing consensus cursors.
    pub fn consensus_network_report_effect_results(
        &self,
        results: Vec<rustaxa_ffi::NetworkEffectResult>,
    ) -> anyhow::Result<rustaxa_ffi::NetworkEffectAck> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        let ack =
            api.report_effect_results(results.into_iter().map(from_bridge_effect_result).collect());
        Ok(rustaxa_ffi::NetworkEffectAck {
            status: ack.status,
            accepted_results: ack.accepted_results,
            failed_results: ack.failed_results,
            error_code: ack.error_code,
        })
    }

    /// Plans single-vote PBFT ingress through the external network/tarcap API.
    ///
    /// This keeps packet-adjacent vote relevance and sync-hint decisions behind
    /// the same facade as canonical packet ingress. C++ still supplies compact
    /// decoded facts and executes returned hints while vote materialization and
    /// admission are being migrated.
    pub fn consensus_network_plan_pbft_vote_ingress(
        &self,
        fact: rustaxa_ffi::PbftVoteIngressFact,
        context: rustaxa_ffi::PbftVoteIngressContext,
    ) -> anyhow::Result<rustaxa_ffi::PbftVoteIngressPlan> {
        let api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(vote_ingress_plan_to_ffi(api.plan_pbft_vote_ingress(
            vote_ingress_fact_to_domain(fact)?,
            vote_ingress_context_to_domain(context),
        )))
    }

    /// Plans PBFT vote-bundle ingress through the external network/tarcap API.
    ///
    /// The method is side-effect free and intentionally limited to the scalar
    /// facts needed by bundle-shape and network-window checks.
    pub fn consensus_network_plan_pbft_vote_bundle_ingress(
        &self,
        reference: rustaxa_ffi::PbftVoteIngressFact,
        vote: rustaxa_ffi::PbftVoteIngressFact,
        context: rustaxa_ffi::PbftVoteIngressContext,
    ) -> anyhow::Result<rustaxa_ffi::PbftVoteIngressPlan> {
        let api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(vote_ingress_plan_to_ffi(api.plan_pbft_vote_bundle_ingress(
            vote_ingress_fact_to_domain(reference)?,
            vote_ingress_fact_to_domain(vote)?,
            vote_ingress_context_to_domain(context),
        )))
    }

    /// Plans deterministic sync follow-up for an accepted status packet.
    pub fn consensus_network_plan_status_sync(
        &self,
        facts: rustaxa_ffi::NetworkStatusSyncFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkStatusSyncPlan> {
        let api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_status_sync_plan(api.plan_status_sync(
            to_domain_network_status_sync_facts(facts),
        )))
    }

    /// Plans local status packet egress through the network/tarcap API.
    pub fn consensus_network_plan_status_egress(
        &self,
        facts: rustaxa_ffi::NetworkStatusEgressFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkStatusEgressPlan> {
        let api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_status_egress_plan(
            api.plan_status_egress(to_domain_network_status_egress_facts(facts)),
        ))
    }

    /// Plans whether an initial status packet should be accepted.
    pub fn consensus_network_plan_initial_status(
        &self,
        facts: rustaxa_ffi::NetworkInitialStatusFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkInitialStatusPlan> {
        let api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_initial_status_plan(
            api.plan_initial_status(to_domain_network_initial_status_facts(facts)),
        ))
    }

    /// Plans whether PBFT sync should start and which peer should serve it.
    pub fn consensus_network_plan_pbft_sync_start(
        &self,
        facts: rustaxa_ffi::NetworkPbftSyncStartFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPbftSyncStartPlan> {
        let api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_pbft_sync_start_plan(
            api.plan_pbft_sync_start(to_domain_network_pbft_sync_start_facts(facts)),
        ))
    }

    /// Selects the best max-chain peer from compact network-owned peer facts.
    pub fn consensus_network_plan_max_chain_peer_selection(
        &self,
        facts: rustaxa_ffi::NetworkPeerSelectionFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPeerSelectionPlan> {
        let api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_peer_selection_plan(
            api.plan_max_chain_peer_selection(to_domain_network_peer_selection_facts(facts)),
        ))
    }

    /// Plans whether pending DAG blocks should be requested and from which peer.
    pub fn consensus_network_plan_pending_dag_blocks_request(
        &self,
        facts: rustaxa_ffi::NetworkPendingDagBlocksRequestFacts,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPendingDagBlocksRequestPlan> {
        let api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_pending_dag_blocks_request_plan(
            api.plan_pending_dag_blocks_request(
                to_domain_network_pending_dag_blocks_request_facts(facts),
            ),
        ))
    }

    /// Routes single-vote PBFT ingress and queues network effects.
    ///
    /// Unlike the side-effect-free planner method, this production-facing route
    /// updates the network facade's effect queue so C++ can execute sync,
    /// report, and disconnect requests through `drain_work` /
    /// `report_effect_results`.
    pub fn consensus_network_ingest_pbft_vote(
        &self,
        fact: rustaxa_ffi::PbftVoteIngressFact,
        context: rustaxa_ffi::NetworkPbftVoteIngressContext,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(api.ingest_pbft_vote(
            vote_ingress_fact_to_domain(fact)?,
            to_domain_pbft_vote_ingress_context(context),
        )))
    }

    /// Routes one vote-bundle member through PBFT ingress and queues network effects.
    pub fn consensus_network_ingest_pbft_vote_bundle_member(
        &self,
        reference: rustaxa_ffi::PbftVoteIngressFact,
        vote: rustaxa_ffi::PbftVoteIngressFact,
        context: rustaxa_ffi::NetworkPbftVoteIngressContext,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(
            api.ingest_pbft_vote_bundle_member(
                vote_ingress_fact_to_domain(reference)?,
                vote_ingress_fact_to_domain(vote)?,
                to_domain_pbft_vote_ingress_context(context),
            ),
        ))
    }

    /// Plans pillar-vote relevance through the external network/tarcap API.
    pub fn consensus_network_plan_pillar_vote_relevance(
        &self,
        fact: rustaxa_ffi::PillarVoteRelevanceFact,
    ) -> anyhow::Result<rustaxa_ffi::PillarVoteRelevancePlan> {
        let api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        let plan = api.plan_pillar_vote_relevance(to_domain_pillar_vote_relevance_fact(fact)?)?;
        Ok(rustaxa_ffi::PillarVoteRelevancePlan {
            status: plan.status_code(),
            is_relevant: plan.is_relevant,
        })
    }

    /// Requests PBFT vote gossip through the external network/tarcap API.
    ///
    /// Rust owns the decision that an accepted vote should be gossiped. The
    /// network executor still owns peer filtering, packet wrapping, and
    /// transport, so this API exposes the action directly instead of the old
    /// temporary queue-helper naming.
    pub fn consensus_network_gossip_pbft_vote(
        &self,
        effects: rustaxa_ffi::NetworkPbftVoteGossipEffects,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(api.gossip_pbft_vote(
            to_domain_pbft_vote_gossip_effects(effects),
        )))
    }
}

fn to_domain_config(config: rustaxa_ffi::NetworkApiConfig) -> rustaxa_consensus::NetworkApiConfig {
    rustaxa_consensus::NetworkApiConfig {
        max_payload_bytes: usize::try_from(config.max_payload_bytes).unwrap_or(usize::MAX),
        max_retained_payloads: usize::try_from(config.max_retained_payloads).unwrap_or(usize::MAX),
        max_effects_per_drain: usize::try_from(config.max_effects_per_drain).unwrap_or(usize::MAX),
    }
}

fn to_bridge_network_effect(
    effect: rustaxa_consensus::NetworkEffect,
) -> rustaxa_ffi::NetworkEffect {
    rustaxa_ffi::NetworkEffect {
        effect_id: effect.effect_id,
        source_payload_id: effect.source_payload_id,
        kind: effect.kind,
        peer_id: effect.peer_id,
        packet_kind: effect.packet_kind,
        payload_bytes: effect.payload_bytes,
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
        peer_id: value.peer_id,
        peer_pbft_chain_size: value.peer_pbft_chain_size,
        source_payload_id: value.source_payload_id,
    }
}

fn to_domain_pillar_vote_relevance_fact(
    value: rustaxa_ffi::PillarVoteRelevanceFact,
) -> anyhow::Result<rustaxa_consensus::PillarVoteRelevanceFact> {
    let current_pillar_block_period = if value.has_current_pillar_block {
        Some(value.current_pillar_block_period)
    } else {
        None
    };
    let current_pillar_block_hash = if value.has_current_pillar_block {
        Some(H256::from(value.current_pillar_block_hash))
    } else {
        None
    };

    Ok(rustaxa_consensus::PillarVoteRelevanceFact {
        vote_period: value.vote_period,
        vote_block_hash: H256::from(value.vote_block_hash),
        current_pillar_block_period,
        current_pillar_block_hash,
        first_pillar_block_period: value.first_pillar_block_period,
        pillar_blocks_interval: value.pillar_blocks_interval,
        vote_already_known: value.vote_already_known,
    })
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

fn to_bridge_network_ingress_decision(
    decision: rustaxa_consensus::NetworkIngressDecision,
) -> rustaxa_ffi::NetworkIngressDecision {
    rustaxa_ffi::NetworkIngressDecision {
        payload_id: decision.payload_id,
        payload_accepted: decision.payload_accepted,
        routed: decision.routed,
        status: decision.status,
        error_code: decision.error_code,
        queued_effect_count: decision.queued_effect_count,
    }
}

fn to_domain_pbft_vote_gossip_effects(
    value: rustaxa_ffi::NetworkPbftVoteGossipEffects,
) -> rustaxa_consensus::NetworkPbftVoteGossipEffects {
    rustaxa_consensus::NetworkPbftVoteGossipEffects {
        peer_id: value.peer_id,
        vote_hash: value.vote_hash,
        source_payload_id: value.source_payload_id,
        gossip_vote: value.gossip_vote,
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
    }
}
