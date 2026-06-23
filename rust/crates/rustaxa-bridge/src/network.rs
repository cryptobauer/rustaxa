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

fn from_bridge_effect_result(
    result: rustaxa_ffi::NetworkEffectResult,
) -> rustaxa_consensus::NetworkEffectResult {
    rustaxa_consensus::NetworkEffectResult {
        effect_id: result.effect_id,
        status: result.status,
        diagnostic: result.diagnostic,
    }
}
