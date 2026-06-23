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

    /// Queues network effects derived from accepted PBFT vote admission.
    pub fn consensus_network_queue_pbft_vote_admission_effects(
        &self,
        effects: rustaxa_ffi::NetworkPbftVoteAdmissionEffects,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(
            api.queue_pbft_vote_admission_effects(to_domain_pbft_vote_admission_effects(effects)),
        ))
    }

    /// Queues a verified-vote admission request for an accepted PBFT vote.
    pub fn consensus_network_queue_pbft_vote_admission_request_effects(
        &self,
        effects: rustaxa_ffi::NetworkPbftVoteAdmissionRequestEffects,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(
            api.queue_pbft_vote_admission_request_effects(
                to_domain_pbft_vote_admission_request_effects(effects),
            ),
        ))
    }

    /// Queues network effects derived from accepted PBFT block sidecars.
    pub fn consensus_network_queue_pbft_block_admission_effects(
        &self,
        effects: rustaxa_ffi::NetworkPbftBlockAdmissionEffects,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(
            api.queue_pbft_block_admission_effects(to_domain_pbft_block_admission_effects(effects)),
        ))
    }

    /// Queues network effects derived from accepted PBFT vote gossip.
    pub fn consensus_network_queue_pbft_vote_gossip_effects(
        &self,
        effects: rustaxa_ffi::NetworkPbftVoteGossipEffects,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(
            api.queue_pbft_vote_gossip_effects(to_domain_pbft_vote_gossip_effects(effects)),
        ))
    }

    /// Queues network effects derived from proposed PBFT block sidecars.
    pub fn consensus_network_queue_pbft_proposed_block_sidecar_effects(
        &self,
        effects: rustaxa_ffi::NetworkPbftProposedBlockSidecarEffects,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(
            api.queue_pbft_proposed_block_sidecar_effects(
                to_domain_pbft_proposed_block_sidecar_effects(effects),
            ),
        ))
    }

    /// Queues network effects derived from proposed PBFT blocks bundle intake.
    pub fn consensus_network_queue_pbft_proposed_block_bundle_effects(
        &self,
        effects: rustaxa_ffi::NetworkPbftProposedBlockSidecarEffects,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(
            api.queue_pbft_proposed_block_bundle_effects(
                to_domain_pbft_proposed_block_sidecar_effects(effects),
            ),
        ))
    }

    /// Queues transaction-pool admission for a transaction packet member.
    pub fn consensus_network_queue_transaction_admission_request_effects(
        &self,
        effects: rustaxa_ffi::NetworkTransactionAdmissionRequestEffects,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(
            api.queue_transaction_admission_request_effects(
                to_domain_transaction_admission_request_effects(effects),
            ),
        ))
    }

    /// Queues DAG block admission for a DAG block packet.
    pub fn consensus_network_queue_dag_block_admission_request_effects(
        &self,
        effects: rustaxa_ffi::NetworkDagBlockAdmissionRequestEffects,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| anyhow::anyhow!("consensus network api lock poisoned"))?;
        Ok(to_bridge_network_ingress_decision(
            api.queue_dag_block_admission_request_effects(
                to_domain_dag_block_admission_request_effects(effects),
            ),
        ))
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

fn to_domain_pbft_vote_admission_effects(
    value: rustaxa_ffi::NetworkPbftVoteAdmissionEffects,
) -> rustaxa_consensus::NetworkPbftVoteAdmissionEffects {
    rustaxa_consensus::NetworkPbftVoteAdmissionEffects {
        peer_id: value.peer_id,
        vote_hash: value.vote_hash,
        source_payload_id: value.source_payload_id,
        mark_vote_known: value.mark_vote_known,
    }
}

fn to_domain_pbft_vote_admission_request_effects(
    value: rustaxa_ffi::NetworkPbftVoteAdmissionRequestEffects,
) -> rustaxa_consensus::NetworkPbftVoteAdmissionRequestEffects {
    rustaxa_consensus::NetworkPbftVoteAdmissionRequestEffects {
        peer_id: value.peer_id,
        vote_hash: value.vote_hash,
        source_payload_id: value.source_payload_id,
        admit_vote: value.admit_vote,
    }
}

fn to_domain_pbft_block_admission_effects(
    value: rustaxa_ffi::NetworkPbftBlockAdmissionEffects,
) -> rustaxa_consensus::NetworkPbftBlockAdmissionEffects {
    rustaxa_consensus::NetworkPbftBlockAdmissionEffects {
        peer_id: value.peer_id,
        block_hash: value.block_hash,
        source_payload_id: value.source_payload_id,
        mark_block_known: value.mark_block_known,
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

fn to_domain_pbft_proposed_block_sidecar_effects(
    value: rustaxa_ffi::NetworkPbftProposedBlockSidecarEffects,
) -> rustaxa_consensus::NetworkPbftProposedBlockSidecarEffects {
    rustaxa_consensus::NetworkPbftProposedBlockSidecarEffects {
        peer_id: value.peer_id,
        period: value.period,
        block_hash: value.block_hash,
        pivot_hash: value.pivot_hash,
        block_rlp: value.block_rlp,
        source_payload_id: value.source_payload_id,
        record_block: value.record_block,
    }
}

fn to_domain_transaction_admission_request_effects(
    value: rustaxa_ffi::NetworkTransactionAdmissionRequestEffects,
) -> rustaxa_consensus::NetworkTransactionAdmissionRequestEffects {
    rustaxa_consensus::NetworkTransactionAdmissionRequestEffects {
        peer_id: value.peer_id,
        transaction_hash: value.transaction_hash,
        transaction_rlp: value.transaction_rlp,
        source_payload_id: value.source_payload_id,
        admit_transaction: value.admit_transaction,
    }
}

fn to_domain_dag_block_admission_request_effects(
    value: rustaxa_ffi::NetworkDagBlockAdmissionRequestEffects,
) -> rustaxa_consensus::NetworkDagBlockAdmissionRequestEffects {
    rustaxa_consensus::NetworkDagBlockAdmissionRequestEffects {
        peer_id: value.peer_id,
        block_hash: value.block_hash,
        block_rlp: value.block_rlp,
        transaction_count: value.transaction_count,
        source_payload_id: value.source_payload_id,
        admit_block: value.admit_block,
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
