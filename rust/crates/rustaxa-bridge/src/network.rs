//! CXX conversion and lock-error mapping for the Rust-owned network/tarcap API; native consensus owns operation-specific
//! routing, lane-local queue ordering, and effect-result validation while callers execute typed effects.

use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeApp;
use crate::ffi::BridgeConsensusNetworkApi;
use crate::network_slashing::{
    empty_slashing_transaction_effect, slashing_submitter_identity_to_domain,
    slashing_transaction_effect_to_ffi,
};
use rustaxa_consensus::pbft_vote_progress::PbftVoteProgressIntent;

fn consensus_packet_request_to_domain(
    value: rustaxa_ffi::NetworkConsensusPacketRequest,
) -> rustaxa_consensus::NetworkConsensusPacketRequest {
    rustaxa_consensus::NetworkConsensusPacketRequest {
        transport_lane: value.transport_lane,
        peer_id: value.peer_id,
        peer_pbft_chain_size: value.peer_pbft_chain_size,
        source_payload_id: value.source_payload_id,
        packet_rlp: value.packet_rlp,
        current_period: value.current_period,
        current_round: value.current_round,
        current_step: value.current_step,
        max_future_period_delta: value.max_future_period_delta,
        max_future_round_delta: value.max_future_round_delta,
        max_future_step_delta: value.max_future_step_delta,
        validate_max_round_step: value.validate_max_round_step,
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

    /// Atomically selects a serviceable peer and starts one native PBFT-sync generation.
    pub fn consensus_network_begin_pbft_sync(
        &self,
        request: rustaxa_ffi::NetworkPbftSyncStartRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPbftSyncStartOutcome> {
        let outcome =
            self.network
                .begin_pbft_sync(rustaxa_consensus::NetworkPbftSyncStartRequest {
                    start: request.start,
                    now_ms: request.now_ms,
                    local_pbft_synced_period: request.local_pbft_synced_period,
                    local_pbft_chain_size: request.local_pbft_chain_size,
                    candidates: request
                        .candidates
                        .into_iter()
                        .map(to_domain_network_pbft_sync_peer_candidate)
                        .collect(),
                })?;
        Ok(rustaxa_ffi::NetworkPbftSyncStartOutcome {
            status: outcome.status,
            error_code: outcome.error_code,
            started: outcome.started,
            has_peer: outcome.has_peer,
            peer_id: outcome.peer_id,
            peer_pbft_chain_size: outcome.peer_pbft_chain_size,
            request_period: outcome.request_period,
            generation: outcome.generation,
            deep_syncing: outcome.deep_syncing,
            enable_snapshot_creation: outcome.enable_snapshot_creation,
        })
    }

    /// Records one accepted periodic status and returns application-owned follow-up work.
    pub fn consensus_network_process_status_followup(
        &self,
        request: rustaxa_ffi::NetworkStatusFollowupRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkStatusFollowupOutcome> {
        let outcome = self.network.process_status_followup(
            rustaxa_consensus::NetworkStatusFollowupRequest {
                peer_id: request.peer_id,
                local_pbft_synced_period: request.local_pbft_synced_period,
                local_pbft_period: request.local_pbft_period,
                local_pbft_round: request.local_pbft_round,
                peer_pbft_chain_size: request.peer_pbft_chain_size,
                peer_pbft_period: request.peer_pbft_period,
                peer_pbft_round: request.peer_pbft_round,
                peer_dag_synced: request.peer_dag_synced,
            },
        )?;
        Ok(rustaxa_ffi::NetworkStatusFollowupOutcome {
            request_pbft_sync: outcome.request_pbft_sync,
            request_pending_dag_blocks: outcome.request_pending_dag_blocks,
            request_next_votes: outcome.request_next_votes,
            next_votes_period: outcome.next_votes_period,
            next_votes_round: outcome.next_votes_round,
            sync_generation: outcome.sync_generation,
        })
    }

    /// Selects canonical status fields, projecting active state for initial
    /// packets and deep-sync state for periodic packets atomically.
    pub fn consensus_network_status_egress(
        &self,
        request: rustaxa_ffi::NetworkStatusEgressRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkStatusEgressOutcome> {
        let outcome =
            self.network
                .plan_status_egress(rustaxa_consensus::NetworkStatusEgressFacts {
                    initial: request.initial,
                    local_chain_id: request.local_chain_id,
                    genesis_hash: request.genesis_hash,
                    node_major_version: request.node_major_version,
                    node_minor_version: request.node_minor_version,
                    node_patch_version: request.node_patch_version,
                    is_light_node: request.is_light_node,
                    light_node_history: request.light_node_history,
                    local_pbft_chain_size: request.local_pbft_chain_size,
                    local_pbft_round: request.local_pbft_round,
                    local_dag_level: request.local_dag_level,
                    pbft_syncing: false,
                    deep_pbft_syncing: false,
                })?;
        Ok(rustaxa_ffi::NetworkStatusEgressOutcome {
            status: outcome.status,
            error_code: outcome.error_code,
            peer_pbft_chain_size: outcome.peer_pbft_chain_size,
            peer_pbft_round: outcome.peer_pbft_round,
            peer_dag_level: outcome.peer_dag_level,
            peer_syncing: outcome.peer_syncing,
            include_initial_data: outcome.include_initial_data,
            chain_id: outcome.chain_id,
            genesis_hash: outcome.genesis_hash,
            node_major_version: outcome.node_major_version,
            node_minor_version: outcome.node_minor_version,
            node_patch_version: outcome.node_patch_version,
            is_light_node: outcome.is_light_node,
            light_node_history: outcome.light_node_history,
        })
    }

    /// Applies native initial-status chain, genesis, and history admission policy.
    pub fn consensus_network_admit_initial_status(
        &self,
        request: rustaxa_ffi::NetworkInitialStatusRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkInitialStatusOutcome> {
        let outcome =
            self.network
                .admit_initial_status(rustaxa_consensus::NetworkInitialStatusFacts {
                    local_chain_id: request.local_chain_id,
                    peer_chain_id: request.peer_chain_id,
                    expected_genesis_hash: request.expected_genesis_hash,
                    peer_genesis_hash: request.peer_genesis_hash,
                    local_pbft_synced_period: request.local_pbft_synced_period,
                    peer_pbft_chain_size: request.peer_pbft_chain_size,
                    peer_is_light_node: request.peer_is_light_node,
                    peer_light_node_history: request.peer_light_node_history,
                })?;
        Ok(rustaxa_ffi::NetworkInitialStatusOutcome {
            status: outcome.status,
            error_code: outcome.error_code,
            accept_peer: outcome.accept_peer,
            disconnect_peer: outcome.disconnect_peer,
        })
    }

    /// Applies source, activity, stop, disconnect, or timer-expiry work to one
    /// generation; unknown kinds fail without mutation.
    pub fn consensus_network_apply_pbft_sync_command(
        &self,
        request: rustaxa_ffi::NetworkPbftSyncCommand,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPbftSyncCommandOutcome> {
        let outcome = self.network.apply_pbft_sync_command(
            rustaxa_consensus::NetworkPbftSyncCommandRequest {
                kind: request.kind,
                now_ms: request.now_ms,
                generation: request.generation,
                peer_id: request.peer_id,
                source: request.source,
                reason: request.reason,
                sync_queue_size: request.sync_queue_size,
                syncing_period: request.syncing_period,
                finalized_period: request.finalized_period,
                remote_period: request.remote_period,
                sync_level_size: request.sync_level_size,
                retry_count: request.retry_count,
                retry_delay_ms: request.retry_delay_ms,
            },
        )?;
        Ok(rustaxa_ffi::NetworkPbftSyncCommandOutcome {
            accepted: outcome.accepted,
            active: outcome.active,
            stopped: outcome.stopped,
            expired: outcome.expired,
            restart_sync: outcome.restart_sync,
            retry: outcome.retry,
            request_next: outcome.request_next,
            request_pending_dag_if_idle: outcome.request_pending_dag_if_idle,
            deep_syncing: outcome.deep_syncing,
            generation: outcome.generation,
            error_code: outcome.error_code,
        })
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

    /// Decodes and admits a complete canonical PBFT vote packet in native code.
    pub fn consensus_network_ingest_pbft_vote_packet(
        &self,
        request: rustaxa_ffi::NetworkConsensusPacketRequest,
        slashing_submitters: Vec<rustaxa_ffi::SlashingSubmitterIdentity>,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPbftVotePacketReport> {
        let report = self.network.ingest_pbft_vote_packet(
            self.pbft.as_ref(),
            self.final_chain.as_ref(),
            consensus_packet_request_to_domain(request),
            &slashing_submitters
                .into_iter()
                .map(slashing_submitter_identity_to_domain)
                .collect::<Vec<_>>(),
        );
        match report {
            Ok(report) => Ok(to_bridge_pbft_vote_packet_report(report)),
            Err(error) => match peer_packet_error_code(&error, &["NETWORK_PBFT_VOTE_PACKET_"]) {
                Some(error_code) => Ok(pbft_peer_packet_error_report(error_code)),
                None => Err(error),
            },
        }
    }

    /// Decodes and admits a complete optimized PBFT votes-bundle packet in native code.
    pub fn consensus_network_ingest_pbft_votes_bundle_packet(
        &self,
        request: rustaxa_ffi::NetworkConsensusPacketRequest,
        slashing_submitters: Vec<rustaxa_ffi::SlashingSubmitterIdentity>,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPbftVotePacketReport> {
        let report = self.network.ingest_pbft_votes_bundle_packet(
            self.pbft.as_ref(),
            self.final_chain.as_ref(),
            consensus_packet_request_to_domain(request),
            &slashing_submitters
                .into_iter()
                .map(slashing_submitter_identity_to_domain)
                .collect::<Vec<_>>(),
        );
        match report {
            Ok(report) => Ok(to_bridge_pbft_vote_packet_report(report)),
            Err(error) => match peer_packet_error_code(&error, &["NETWORK_PBFT_VOTES_BUNDLE_"]) {
                Some(error_code) => Ok(pbft_peer_packet_error_report(error_code)),
                None => Err(error),
            },
        }
    }

    /// Decodes and admits a complete canonical pillar-vote packet in native code.
    pub fn consensus_network_ingest_pillar_vote_packet(
        &self,
        request: rustaxa_ffi::NetworkConsensusPacketRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPillarVotePacketReport> {
        let report = self.network.ingest_pillar_vote_packet(
            self.pbft.as_ref(),
            self.final_chain.as_ref(),
            consensus_packet_request_to_domain(request),
        );
        match report {
            Ok(report) => Ok(to_bridge_pillar_vote_packet_report(report)),
            Err(error) => match peer_packet_error_code(&error, &["NETWORK_PILLAR_VOTE_PACKET_"]) {
                Some(error_code) => Ok(pillar_peer_packet_error_report(error_code)),
                None => Err(error),
            },
        }
    }

    /// Decodes and admits a complete optimized pillar-votes-bundle packet in native code.
    pub fn consensus_network_ingest_pillar_votes_bundle_packet(
        &self,
        request: rustaxa_ffi::NetworkConsensusPacketRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkPillarVotePacketReport> {
        let report = self.network.ingest_pillar_votes_bundle_packet(
            self.pbft.as_ref(),
            self.final_chain.as_ref(),
            consensus_packet_request_to_domain(request),
        );
        match report {
            Ok(report) => Ok(to_bridge_pillar_vote_packet_report(report)),
            Err(error) => {
                match peer_packet_error_code(&error, &["NETWORK_PILLAR_VOTES_BUNDLE_PACKET_"]) {
                    Some(error_code) => Ok(pillar_peer_packet_error_report(error_code)),
                    None => Err(error),
                }
            }
        }
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

    /// Prepares canonical packets and exact known-object probes for one
    /// application-owned egress operation. DAG and transaction payloads are
    /// materialized from native application state; no C++ consensus object or
    /// transaction sidecar crosses this boundary.
    pub fn consensus_network_prepare_egress(
        &self,
        application: &BridgeApp,
        request: rustaxa_ffi::NetworkEgressPrepareRequest,
    ) -> anyhow::Result<rustaxa_ffi::NetworkEgressPreparation> {
        let (transaction_accounts, dag_transactions) =
            application.0.prepare_network_egress_inputs(
                request.family,
                request.object_hash,
                &request.payload_bytes,
            )?;
        let payload_bytes = request.payload_bytes;
        let preparation = self.network.prepare_egress(
            rustaxa_consensus::NetworkEgressPrepareRequest {
                family: request.family,
                transport_lane: request.transport_lane,
                source_payload_id: request.source_payload_id,
                source_peer_id: request.source_peer_id,
                rebroadcast: request.rebroadcast,
                object_hash: request.object_hash,
                payload_bytes,
                related_payload_bytes: request.related_payload_bytes,
            },
            transaction_accounts,
            dag_transactions,
        )?;
        Ok(rustaxa_ffi::NetworkEgressPreparation {
            token: preparation.token,
            probes: preparation
                .probes
                .into_iter()
                .map(|probe| rustaxa_ffi::NetworkEgressProbe {
                    probe_id: probe.probe_id,
                    object_kind: probe.object_kind,
                    object_hash: probe.object_hash,
                })
                .collect(),
        })
    }

    /// Commits an immutable peer snapshot and queues exact-target transport effects.
    pub fn consensus_network_plan_egress(
        &self,
        token: u64,
        peers: Vec<rustaxa_ffi::NetworkEgressPeerSnapshot>,
    ) -> anyhow::Result<rustaxa_ffi::NetworkIngressDecision> {
        Ok(to_bridge_network_ingress_decision(
            self.network
                .plan_egress(rustaxa_consensus::NetworkEgressPlanRequest {
                    token,
                    peers: peers
                        .into_iter()
                        .map(|peer| rustaxa_consensus::NetworkEgressPeerSnapshot {
                            peer_id: peer.peer_id,
                            syncing: peer.syncing,
                            known_probe_ids: peer.known_probe_ids,
                        })
                        .collect(),
                })?,
        ))
    }

    /// Cancels one preparation during C++ lane-operation unwinding.
    pub fn consensus_network_cancel_egress(&self, token: u64) -> anyhow::Result<bool> {
        self.network.cancel_egress(token)
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

fn to_bridge_pbft_vote_packet_report(
    value: rustaxa_consensus::NetworkPbftVotePacketReport,
) -> rustaxa_ffi::NetworkPbftVotePacketReport {
    rustaxa_ffi::NetworkPbftVotePacketReport {
        status: 0,
        error_code: String::new(),
        malicious: false,
        outcomes: value
            .outcomes
            .into_iter()
            .map(to_bridge_pbft_vote_admission_outcome)
            .collect(),
        has_peer_pbft_chain_size: value.has_peer_pbft_chain_size,
        peer_pbft_chain_size: value.peer_pbft_chain_size,
        egress_payload_bytes: value.egress_payload_bytes,
    }
}

fn peer_packet_error_code(error: &anyhow::Error, prefixes: &[&str]) -> Option<String> {
    error.chain().find_map(|cause| {
        let message = cause.to_string();
        prefixes
            .iter()
            .any(|prefix| message.starts_with(prefix))
            .then_some(message)
    })
}

fn pbft_peer_packet_error_report(error_code: String) -> rustaxa_ffi::NetworkPbftVotePacketReport {
    rustaxa_ffi::NetworkPbftVotePacketReport {
        status: 1,
        error_code,
        malicious: true,
        outcomes: Vec::new(),
        has_peer_pbft_chain_size: false,
        peer_pbft_chain_size: 0,
        egress_payload_bytes: Vec::new(),
    }
}

fn to_bridge_pillar_vote_admission_outcome(
    value: rustaxa_consensus::NetworkPillarVoteAdmissionOutcome,
) -> rustaxa_ffi::NetworkPillarVoteAdmissionOutcome {
    let decision = to_bridge_network_ingress_decision(value.decision);
    let Some(admission) = value.admission else {
        return rustaxa_ffi::NetworkPillarVoteAdmissionOutcome {
            decision,
            has_admission: false,
            status: 0,
            accepted: false,
            duplicate: false,
            conflict_found: false,
            vote_hash: [0; 32],
            conflicting_vote_hash: [0; 32],
        };
    };
    rustaxa_ffi::NetworkPillarVoteAdmissionOutcome {
        decision,
        has_admission: true,
        status: admission.status,
        accepted: admission.accepted,
        duplicate: admission.duplicate,
        conflict_found: admission.conflict_found,
        vote_hash: admission.vote_hash,
        conflicting_vote_hash: admission.conflicting_vote_hash,
    }
}

fn to_bridge_pillar_vote_packet_report(
    value: rustaxa_consensus::NetworkPillarVotePacketReport,
) -> rustaxa_ffi::NetworkPillarVotePacketReport {
    rustaxa_ffi::NetworkPillarVotePacketReport {
        status: 0,
        error_code: String::new(),
        malicious: false,
        outcomes: value
            .outcomes
            .into_iter()
            .map(to_bridge_pillar_vote_admission_outcome)
            .collect(),
    }
}

fn pillar_peer_packet_error_report(
    error_code: String,
) -> rustaxa_ffi::NetworkPillarVotePacketReport {
    rustaxa_ffi::NetworkPillarVotePacketReport {
        status: 1,
        error_code,
        malicious: true,
        outcomes: Vec::new(),
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

    #[test]
    fn pillar_admission_outcome_conversion_preserves_terminal_facts() {
        let converted = to_bridge_pillar_vote_admission_outcome(
            rustaxa_consensus::NetworkPillarVoteAdmissionOutcome {
                decision: rustaxa_consensus::NetworkIngressDecision {
                    payload_id: 101,
                    payload_accepted: true,
                    routed: true,
                    status: 0,
                    error_code: String::new(),
                    queued_effect_count: 1,
                    application_effect_id: 0,
                },
                admission: Some(
                    rustaxa_consensus::pillar_vote_service::PillarVoteSingleAdmissionWithFinalChainPlan {
                        status: 5,
                        accepted: false,
                        duplicate: true,
                        conflict_found: true,
                        conflicting_vote_hash: [8; 32],
                        block_weight: 13,
                        validator_vote_count: 7,
                        period: 21,
                        vote_hash: [9; 32],
                        voter: [4; 20],
                    },
                ),
            },
        );

        assert_eq!(converted.decision.payload_id, 101);
        assert_eq!(converted.decision.application_effect_id, 0);
        assert!(converted.has_admission);
        assert_eq!(converted.status, 5);
        assert!(!converted.accepted);
        assert!(converted.duplicate);
        assert!(converted.conflict_found);
        assert_eq!(converted.vote_hash, [9; 32]);
        assert_eq!(converted.conflicting_vote_hash, [8; 32]);
    }
}
