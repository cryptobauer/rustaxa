//! External network/tarcap facade for Rust-owned consensus ingress.
//!
//! This module defines the narrow API that network/tarcap code should call
//! instead of reaching into consensus managers or C++ shim classes. The facade
//! accepts canonical packet bytes into a Rust-owned ingress arena and exposes an
//! executor-facing effect queue. It deliberately does not own peer transport,
//! packet wrapping, gossip fanout, disconnect execution, or tarcap scheduling.
//!
//! Inputs are packet type ids, peer ids, canonical payload bytes, and executor
//! result reports. Outputs are stable ingress receipts, ordered network effects,
//! and acknowledgement summaries. The current first slice records ingress bytes
//! and leaves effect production empty until packet-specific consensus pipelines
//! are routed behind this API.

use std::collections::{HashSet, VecDeque};

use crate::{
    PbftVoteIngressContext, PbftVoteIngressFact, PbftVoteIngressPlan,
    plan_pbft_vote_bundle_ingress, plan_pbft_vote_ingress,
};

/// Network/tarcap packet bytes were accepted into the ingress arena.
pub const NETWORK_INGRESS_STATUS_ACCEPTED: u8 = 0;
/// Network/tarcap supplied an empty packet payload.
pub const NETWORK_INGRESS_STATUS_REJECTED_EMPTY_PAYLOAD: u8 = 1;
/// Network/tarcap supplied a packet type not routed by this first facade slice.
pub const NETWORK_INGRESS_STATUS_UNSUPPORTED_PACKET_TYPE: u8 = 2;
/// Network/tarcap supplied a payload larger than the configured boundary.
pub const NETWORK_INGRESS_STATUS_PAYLOAD_TOO_LARGE: u8 = 3;
/// Network/tarcap supplied bytes while the retained ingress arena was full.
pub const NETWORK_INGRESS_STATUS_QUEUE_FULL: u8 = 4;
/// Network/tarcap supplied bytes after payload ids were exhausted.
pub const NETWORK_INGRESS_STATUS_PAYLOAD_ID_EXHAUSTED: u8 = 5;

/// Network work drain completed successfully.
pub const NETWORK_EFFECT_BATCH_STATUS_OK: u8 = 0;

/// Network effect result reports were accepted.
pub const NETWORK_EFFECT_ACK_STATUS_ACCEPTED: u8 = 0;
/// Network effect result referenced an unknown effect id.
pub const NETWORK_EFFECT_ACK_STATUS_UNKNOWN_EFFECT_ID: u8 = 1;
/// Network effect result repeated an effect id in the same report batch.
pub const NETWORK_EFFECT_ACK_STATUS_DUPLICATE_EFFECT_RESULT: u8 = 2;
/// Network effect result used an unsupported executor status code.
pub const NETWORK_EFFECT_ACK_STATUS_INVALID_RESULT_STATUS: u8 = 3;

/// Network effect executor reported success.
pub const NETWORK_EFFECT_RESULT_STATUS_OK: u8 = 0;
/// Network effect executor reported failure.
pub const NETWORK_EFFECT_RESULT_STATUS_FAILED: u8 = 1;

/// Network effect asks the executor to send a packet to one peer.
pub const NETWORK_EFFECT_KIND_SEND_PACKET: u8 = 0;
/// Network effect asks the executor to gossip a packet.
pub const NETWORK_EFFECT_KIND_GOSSIP_PACKET: u8 = 1;
/// Network effect asks the executor to mark an object known for a peer.
pub const NETWORK_EFFECT_KIND_MARK_PEER_KNOWN: u8 = 2;
/// Network effect asks the executor to request synchronization.
pub const NETWORK_EFFECT_KIND_REQUEST_SYNC: u8 = 3;
/// Network effect asks the executor to report peer behavior.
pub const NETWORK_EFFECT_KIND_REPORT_PEER: u8 = 4;
/// Network effect asks the executor to disconnect a peer.
pub const NETWORK_EFFECT_KIND_DISCONNECT_PEER: u8 = 5;
/// Network effect asks the executor to block peer-order dependent work.
pub const NETWORK_EFFECT_KIND_BLOCK_PEER_ORDER: u8 = 6;
/// Network effect asks the executor to drive PBFT progress.
pub const NETWORK_EFFECT_KIND_DRIVE_CONSENSUS_PROGRESS: u8 = 7;

const ERROR_NONE: &str = "";
const ERROR_REJECTED_EMPTY_PAYLOAD: &str = "NETWORK_INGRESS_REJECTED_EMPTY_PAYLOAD";
const ERROR_UNSUPPORTED_PACKET_TYPE: &str = "NETWORK_INGRESS_UNSUPPORTED_PACKET_TYPE";
const ERROR_PAYLOAD_TOO_LARGE: &str = "NETWORK_INGRESS_PAYLOAD_TOO_LARGE";
const ERROR_QUEUE_FULL: &str = "NETWORK_INGRESS_QUEUE_FULL";
const ERROR_PAYLOAD_ID_EXHAUSTED: &str = "NETWORK_INGRESS_PAYLOAD_ID_EXHAUSTED";
const ERROR_UNKNOWN_EFFECT_ID: &str = "NETWORK_EFFECT_RESULT_UNKNOWN_EFFECT_ID";
const ERROR_DUPLICATE_EFFECT_RESULT: &str = "NETWORK_EFFECT_RESULT_DUPLICATE_EFFECT_ID";
const ERROR_INVALID_RESULT_STATUS: &str = "NETWORK_EFFECT_RESULT_INVALID_STATUS";

/// Capacity limits for the external network/tarcap facade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkApiConfig {
    /// Maximum accepted canonical packet payload size in bytes.
    pub max_payload_bytes: usize,
    /// Maximum number of retained ingress payloads.
    pub max_retained_payloads: usize,
    /// Maximum number of effects returned by one drain call.
    pub max_effects_per_drain: usize,
}

impl Default for NetworkApiConfig {
    fn default() -> Self {
        Self {
            max_payload_bytes: 2 * 1024 * 1024,
            max_retained_payloads: 8192,
            max_effects_per_drain: 1024,
        }
    }
}

/// Canonical packet bytes submitted by network/tarcap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkIngressPacket {
    /// Latest-tarcap packet type id.
    pub packet_type: u32,
    /// Sending node id as fixed 64-byte public key bytes.
    pub peer_id: [u8; 64],
    /// Canonical packet payload bytes.
    pub payload_bytes: Vec<u8>,
    /// Boundary-supplied monotonic receive timestamp in milliseconds.
    pub received_at_mono_ms: u64,
    /// Optional network-owned packet id for diagnostics. Rust stores and echoes
    /// it only as ingress metadata.
    pub source_packet_id: u64,
}

/// Opaque id for canonical payload bytes accepted from network/tarcap.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct NetworkPayloadId(pub u64);

/// Canonical packet bytes accepted into Rust-owned ingress storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkIngressPayload {
    /// Opaque payload id assigned by the facade.
    pub payload_id: NetworkPayloadId,
    /// Latest-tarcap packet type id.
    pub packet_type: u32,
    /// Sending node id as fixed 64-byte public key bytes.
    pub peer_id: [u8; 64],
    /// Canonical packet payload bytes.
    pub payload_bytes: Vec<u8>,
    /// Boundary-supplied monotonic receive timestamp in milliseconds.
    pub received_at_mono_ms: u64,
    /// Optional network-owned packet id for diagnostics.
    pub source_packet_id: u64,
}

/// Result of accepting or rejecting packet bytes at the consensus boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkIngressReceipt {
    /// Whether payload bytes were accepted into the Rust-owned ingress arena.
    pub accepted: bool,
    /// Opaque payload id. Zero is reserved for rejected ingress.
    pub payload_id: NetworkPayloadId,
    /// Stable ingress status code.
    pub status: u8,
    /// Stable textual status for boundary logs and tests.
    pub error_code: String,
}

/// Executor-visible network effect planned by Rust consensus.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEffect {
    /// Stable effect id used to correlate executor result reports.
    pub effect_id: u64,
    /// Ingress payload id that caused this effect, when known.
    pub source_payload_id: u64,
    /// Stable effect kind.
    pub kind: u8,
    /// Target peer id when the effect applies to one peer.
    pub peer_id: [u8; 64],
    /// Packet kind for send/gossip effects.
    pub packet_kind: u32,
    /// Packet payload bytes for send/gossip effects.
    pub payload_bytes: Vec<u8>,
    /// Peers excluded from gossip effects.
    pub exclude_peers: Vec<[u8; 64]>,
    /// Optional object kind for known-peer effects.
    pub object_kind: u8,
    /// Optional object hash for known-peer effects.
    pub object_hash: [u8; 32],
    /// Optional sync kind for sync-request effects.
    pub sync_kind: u8,
    /// Optional sync cursor, period, level, or block number.
    pub sync_start: u64,
    /// Optional report or disconnect reason code.
    pub reason_code: u8,
    /// Optional dependency id for peer-order blocking.
    pub dependency_id: u64,
    /// Optional PBFT period for progress-driving effects.
    pub period: u64,
    /// Optional PBFT round for progress-driving effects.
    pub round: u64,
}

/// Ordered network effects returned to the network/tarcap executor.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkEffectBatch {
    /// Stable batch status code.
    pub status: u8,
    /// Effects to execute in order.
    pub effects: Vec<NetworkEffect>,
    /// Whether additional effects remain queued after this drain.
    pub more_available: bool,
    /// Stable textual status for boundary logs and tests.
    pub error_code: String,
}

/// Result reported by the network/tarcap executor for one effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEffectResult {
    /// Effect id from [`NetworkEffect`].
    pub effect_id: u64,
    /// Stable executor result status.
    pub status: u8,
    /// Optional diagnostic text for logging at the boundary.
    pub diagnostic: String,
}

/// Acknowledgement returned after applying effect result reports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEffectAck {
    /// Stable acknowledgement status code.
    pub status: u8,
    /// Number of result reports consumed by the facade.
    pub accepted_results: u64,
    /// Number of failed effect result reports.
    pub failed_results: u64,
    /// Stable textual status for boundary logs and tests.
    pub error_code: String,
}

/// Rust-owned external network/tarcap API facade.
///
/// The facade owns canonical ingress payload bytes and an ordered network
/// effect queue. It is intentionally small: packet-specific decoding and
/// consensus planning should be added behind this type without exposing
/// consensus managers, C++ sidecars, storage handles, or shim routes to the
/// network module.
#[derive(Debug, Default)]
pub struct ConsensusNetworkApi {
    config: NetworkApiConfig,
    next_payload_id: u64,
    next_effect_id: u64,
    ingress_payloads: Vec<NetworkIngressPayload>,
    pending_effects: VecDeque<NetworkEffect>,
    outstanding_effects: HashSet<u64>,
    effect_results: Vec<NetworkEffectResult>,
}

impl ConsensusNetworkApi {
    /// Creates an empty network/tarcap API facade.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(NetworkApiConfig::default())
    }

    /// Creates an empty network/tarcap API facade with explicit capacity
    /// limits.
    #[must_use]
    pub fn with_config(config: NetworkApiConfig) -> Self {
        Self {
            config,
            next_payload_id: 1,
            next_effect_id: 1,
            ingress_payloads: Vec::new(),
            pending_effects: VecDeque::new(),
            outstanding_effects: HashSet::new(),
            effect_results: Vec::new(),
        }
    }

    /// Accepts canonical packet bytes into the Rust-owned ingress arena.
    ///
    /// Empty payloads are rejected at the boundary because there are no
    /// canonical bytes for later packet-specific planners to inspect. A
    /// successful receipt means only that bytes were stored; it does not mean
    /// the packet is protocol-valid or consensus-accepted.
    pub fn ingest_packet(&mut self, packet: NetworkIngressPacket) -> NetworkIngressReceipt {
        if !is_supported_ingress_packet(packet.packet_type) {
            return NetworkIngressReceipt {
                accepted: false,
                payload_id: NetworkPayloadId(0),
                status: NETWORK_INGRESS_STATUS_UNSUPPORTED_PACKET_TYPE,
                error_code: ERROR_UNSUPPORTED_PACKET_TYPE.to_owned(),
            };
        }

        if packet.payload_bytes.is_empty() {
            return NetworkIngressReceipt {
                accepted: false,
                payload_id: NetworkPayloadId(0),
                status: NETWORK_INGRESS_STATUS_REJECTED_EMPTY_PAYLOAD,
                error_code: ERROR_REJECTED_EMPTY_PAYLOAD.to_owned(),
            };
        }

        if packet.payload_bytes.len() > self.config.max_payload_bytes {
            return NetworkIngressReceipt {
                accepted: false,
                payload_id: NetworkPayloadId(0),
                status: NETWORK_INGRESS_STATUS_PAYLOAD_TOO_LARGE,
                error_code: ERROR_PAYLOAD_TOO_LARGE.to_owned(),
            };
        }

        if self.ingress_payloads.len() >= self.config.max_retained_payloads {
            return NetworkIngressReceipt {
                accepted: false,
                payload_id: NetworkPayloadId(0),
                status: NETWORK_INGRESS_STATUS_QUEUE_FULL,
                error_code: ERROR_QUEUE_FULL.to_owned(),
            };
        }

        let payload_id = NetworkPayloadId(self.next_payload_id);
        let Some(next_payload_id) = self.next_payload_id.checked_add(1) else {
            return NetworkIngressReceipt {
                accepted: false,
                payload_id: NetworkPayloadId(0),
                status: NETWORK_INGRESS_STATUS_PAYLOAD_ID_EXHAUSTED,
                error_code: ERROR_PAYLOAD_ID_EXHAUSTED.to_owned(),
            };
        };
        self.next_payload_id = next_payload_id;
        self.ingress_payloads.push(NetworkIngressPayload {
            payload_id,
            packet_type: packet.packet_type,
            peer_id: packet.peer_id,
            payload_bytes: packet.payload_bytes,
            received_at_mono_ms: packet.received_at_mono_ms,
            source_packet_id: packet.source_packet_id,
        });

        NetworkIngressReceipt {
            accepted: true,
            payload_id,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
        }
    }

    /// Drains up to `budget` pending network effects in FIFO order.
    ///
    /// A zero budget is valid and returns an empty batch. The first slice does
    /// not yet produce effects from ingress, so this method is primarily the
    /// stable executor contract for later packet-specific routing.
    #[must_use]
    pub fn drain_work(&mut self, budget: u32) -> NetworkEffectBatch {
        let mut effects = Vec::new();
        let capped_budget = usize::try_from(budget)
            .unwrap_or(usize::MAX)
            .min(self.config.max_effects_per_drain);
        for _ in 0..capped_budget {
            let Some(effect) = self.pending_effects.pop_front() else {
                break;
            };
            self.outstanding_effects.insert(effect.effect_id);
            effects.push(effect);
        }
        NetworkEffectBatch {
            status: NETWORK_EFFECT_BATCH_STATUS_OK,
            effects,
            more_available: !self.pending_effects.is_empty(),
            error_code: ERROR_NONE.to_owned(),
        }
    }

    /// Records network executor result reports.
    ///
    /// Consensus-specific result validation will become stricter when concrete
    /// effects are emitted. For now, every report is retained and summarized so
    /// C++ callers can prove the direct result-reporting path without shim
    /// involvement.
    pub fn report_effect_results(&mut self, results: Vec<NetworkEffectResult>) -> NetworkEffectAck {
        let mut accepted_results = 0u64;
        let mut seen = HashSet::new();
        let mut status = NETWORK_EFFECT_ACK_STATUS_ACCEPTED;
        let mut error_code = ERROR_NONE;
        let failed_results = results
            .iter()
            .filter(|result| result.status == NETWORK_EFFECT_RESULT_STATUS_FAILED)
            .count() as u64;

        for result in &results {
            if result.status != NETWORK_EFFECT_RESULT_STATUS_OK
                && result.status != NETWORK_EFFECT_RESULT_STATUS_FAILED
            {
                status = NETWORK_EFFECT_ACK_STATUS_INVALID_RESULT_STATUS;
                error_code = ERROR_INVALID_RESULT_STATUS;
                break;
            }
            if !seen.insert(result.effect_id) {
                status = NETWORK_EFFECT_ACK_STATUS_DUPLICATE_EFFECT_RESULT;
                error_code = ERROR_DUPLICATE_EFFECT_RESULT;
                break;
            }
            if !self.outstanding_effects.contains(&result.effect_id) {
                status = NETWORK_EFFECT_ACK_STATUS_UNKNOWN_EFFECT_ID;
                error_code = ERROR_UNKNOWN_EFFECT_ID;
                break;
            }
            accepted_results += 1;
        }

        if status == NETWORK_EFFECT_ACK_STATUS_ACCEPTED {
            for result in &results {
                self.outstanding_effects.remove(&result.effect_id);
            }
            self.effect_results.extend(results);
        }

        NetworkEffectAck {
            status,
            accepted_results,
            failed_results,
            error_code: error_code.to_owned(),
        }
    }

    /// Plans deterministic PBFT vote ingress for a single tarcap vote packet.
    ///
    /// The network facade owns this packet-adjacent decision so external
    /// tarcap handlers do not call standalone consensus bridge helpers. The
    /// caller still supplies compact vote facts and scalar local PBFT context;
    /// live vote validation, verified-vote mutation, and peer transport
    /// execution remain outside this API until later routing slices move them
    /// behind explicit executor reports.
    #[must_use]
    pub fn plan_pbft_vote_ingress(
        &self,
        fact: PbftVoteIngressFact,
        context: PbftVoteIngressContext,
    ) -> PbftVoteIngressPlan {
        plan_pbft_vote_ingress(fact, context)
    }

    /// Plans deterministic PBFT vote ingress for one vote inside a tarcap
    /// vote-bundle packet.
    ///
    /// The reference vote describes the bundle identity that every bundled
    /// vote must match. The returned plan is side-effect free; callers execute
    /// sync hints, peer reports, and admission work through the boundary
    /// appropriate to the current migration stage.
    #[must_use]
    pub fn plan_pbft_vote_bundle_ingress(
        &self,
        reference: PbftVoteIngressFact,
        vote: PbftVoteIngressFact,
        context: PbftVoteIngressContext,
    ) -> PbftVoteIngressPlan {
        plan_pbft_vote_bundle_ingress(reference, vote, context)
    }

    /// Returns the number of accepted ingress payloads retained by the facade.
    #[must_use]
    pub fn ingress_len(&self) -> usize {
        self.ingress_payloads.len()
    }

    /// Returns the latest accepted ingress payload.
    #[must_use]
    pub fn latest_ingress(&self) -> Option<&NetworkIngressPayload> {
        self.ingress_payloads.last()
    }

    /// Enqueues an executor effect for tests and future packet-specific
    /// planners.
    pub fn enqueue_effect(&mut self, mut effect: NetworkEffect) -> u64 {
        let effect_id = self.next_effect_id;
        self.next_effect_id = self.next_effect_id.checked_add(1).unwrap_or(u64::MAX);
        effect.effect_id = effect_id;
        self.pending_effects.push_back(effect);
        effect_id
    }

    /// Returns the number of retained executor result reports.
    #[must_use]
    pub fn effect_result_len(&self) -> usize {
        self.effect_results.len()
    }
}

fn is_supported_ingress_packet(packet_type: u32) -> bool {
    // Keep this first direct network facade slice intentionally narrow. The
    // current latest-tarcap packet ids come from `SubprotocolPacketType`:
    // `kVotePacket = 1` and `kVotesBundlePacket = 3`.
    matches!(packet_type, 1 | 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(byte: u8) -> [u8; 64] {
        [byte; 64]
    }

    fn packet(packet_type: u32, payload_bytes: Vec<u8>) -> NetworkIngressPacket {
        NetworkIngressPacket {
            packet_type,
            peer_id: peer(3),
            payload_bytes,
            received_at_mono_ms: 44,
            source_packet_id: 99,
        }
    }

    #[test]
    fn ingest_packet_accepts_canonical_bytes_without_protocol_acceptance() {
        let mut api = ConsensusNetworkApi::new();

        let receipt = api.ingest_packet(packet(1, vec![1, 2, 3]));

        assert!(receipt.accepted);
        assert_eq!(receipt.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(receipt.error_code, "");
        assert_eq!(receipt.payload_id, NetworkPayloadId(1));
        assert_eq!(api.ingress_len(), 1);

        let latest = api.latest_ingress().expect("accepted ingress payload");
        assert_eq!(latest.packet_type, 1);
        assert_eq!(latest.peer_id, peer(3));
        assert_eq!(latest.payload_bytes, vec![1, 2, 3]);
        assert_eq!(latest.received_at_mono_ms, 44);
        assert_eq!(latest.source_packet_id, 99);
    }

    #[test]
    fn ingest_packet_rejects_empty_payload_without_allocating_id() {
        let mut api = ConsensusNetworkApi::new();

        let receipt = api.ingest_packet(packet(1, Vec::new()));

        assert!(!receipt.accepted);
        assert_eq!(
            receipt.status,
            NETWORK_INGRESS_STATUS_REJECTED_EMPTY_PAYLOAD
        );
        assert_eq!(receipt.error_code, ERROR_REJECTED_EMPTY_PAYLOAD);
        assert_eq!(receipt.payload_id, NetworkPayloadId(0));
        assert_eq!(api.ingress_len(), 0);
    }

    #[test]
    fn ingest_packet_rejects_unsupported_packet_type() {
        let mut api = ConsensusNetworkApi::new();

        let receipt = api.ingest_packet(packet(9, vec![1]));

        assert!(!receipt.accepted);
        assert_eq!(
            receipt.status,
            NETWORK_INGRESS_STATUS_UNSUPPORTED_PACKET_TYPE
        );
        assert_eq!(receipt.error_code, ERROR_UNSUPPORTED_PACKET_TYPE);
        assert_eq!(receipt.payload_id, NetworkPayloadId(0));
        assert_eq!(api.ingress_len(), 0);
    }

    #[test]
    fn ingest_packet_rejects_over_capacity_payloads() {
        let mut api = ConsensusNetworkApi::with_config(NetworkApiConfig {
            max_payload_bytes: 1,
            max_retained_payloads: 10,
            max_effects_per_drain: 10,
        });

        let receipt = api.ingest_packet(packet(1, vec![1, 2]));

        assert!(!receipt.accepted);
        assert_eq!(receipt.status, NETWORK_INGRESS_STATUS_PAYLOAD_TOO_LARGE);
        assert_eq!(receipt.error_code, ERROR_PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn ingest_packet_rejects_full_arena() {
        let mut api = ConsensusNetworkApi::with_config(NetworkApiConfig {
            max_payload_bytes: 10,
            max_retained_payloads: 1,
            max_effects_per_drain: 10,
        });

        assert!(api.ingest_packet(packet(1, vec![1])).accepted);
        let receipt = api.ingest_packet(packet(3, vec![2]));

        assert!(!receipt.accepted);
        assert_eq!(receipt.status, NETWORK_INGRESS_STATUS_QUEUE_FULL);
        assert_eq!(receipt.error_code, ERROR_QUEUE_FULL);
    }

    #[test]
    fn drain_work_preserves_effect_order_and_budget() {
        let mut api = ConsensusNetworkApi::new();
        let first = NetworkEffect {
            effect_id: 0,
            source_payload_id: 0,
            kind: NETWORK_EFFECT_KIND_REQUEST_SYNC,
            peer_id: peer(1),
            packet_kind: 0,
            payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: 0,
            object_hash: [0; 32],
            sync_kind: 2,
            sync_start: 10,
            reason_code: 0,
            dependency_id: 0,
            period: 0,
            round: 0,
        };
        let second = NetworkEffect {
            effect_id: 0,
            source_payload_id: 0,
            kind: NETWORK_EFFECT_KIND_DRIVE_CONSENSUS_PROGRESS,
            peer_id: [0; 64],
            packet_kind: 0,
            payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: 0,
            object_hash: [0; 32],
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id: 0,
            period: 11,
            round: 3,
        };
        api.enqueue_effect(first);
        api.enqueue_effect(second);

        let first_batch = api.drain_work(1);
        assert_eq!(first_batch.status, NETWORK_EFFECT_BATCH_STATUS_OK);
        assert_eq!(first_batch.effects.len(), 1);
        assert!(first_batch.more_available);
        assert_eq!(first_batch.effects[0].effect_id, 1);
        assert_eq!(
            first_batch.effects[0].kind,
            NETWORK_EFFECT_KIND_REQUEST_SYNC
        );

        let second_batch = api.drain_work(10);
        assert_eq!(second_batch.effects.len(), 1);
        assert!(!second_batch.more_available);
        assert_eq!(second_batch.effects[0].effect_id, 2);
        assert_eq!(
            second_batch.effects[0].kind,
            NETWORK_EFFECT_KIND_DRIVE_CONSENSUS_PROGRESS
        );
    }

    #[test]
    fn report_effect_results_counts_failures() {
        let mut api = ConsensusNetworkApi::new();

        let ack = api.report_effect_results(vec![
            NetworkEffectResult {
                effect_id: 1,
                status: NETWORK_EFFECT_RESULT_STATUS_OK,
                diagnostic: String::new(),
            },
            NetworkEffectResult {
                effect_id: 2,
                status: NETWORK_EFFECT_RESULT_STATUS_FAILED,
                diagnostic: "send failed".to_owned(),
            },
        ]);

        assert_eq!(ack.status, NETWORK_EFFECT_ACK_STATUS_UNKNOWN_EFFECT_ID);
        assert_eq!(ack.accepted_results, 0);
        assert_eq!(ack.failed_results, 1);
        assert_eq!(ack.error_code, ERROR_UNKNOWN_EFFECT_ID);
        assert_eq!(api.effect_result_len(), 0);
    }

    #[test]
    fn report_effect_results_accepts_known_drained_effects() {
        let mut api = ConsensusNetworkApi::new();
        api.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: 1,
            kind: NETWORK_EFFECT_KIND_SEND_PACKET,
            peer_id: peer(1),
            packet_kind: 1,
            payload_bytes: vec![1],
            exclude_peers: Vec::new(),
            object_kind: 0,
            object_hash: [0; 32],
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id: 0,
            period: 0,
            round: 0,
        });
        let batch = api.drain_work(1);
        assert_eq!(batch.effects[0].effect_id, 1);

        let ack = api.report_effect_results(vec![NetworkEffectResult {
            effect_id: 1,
            status: NETWORK_EFFECT_RESULT_STATUS_OK,
            diagnostic: String::new(),
        }]);

        assert_eq!(ack.status, NETWORK_EFFECT_ACK_STATUS_ACCEPTED);
        assert_eq!(ack.accepted_results, 1);
        assert_eq!(ack.failed_results, 0);
        assert_eq!(ack.error_code, "");
        assert_eq!(api.effect_result_len(), 1);
    }
}
