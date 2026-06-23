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

use std::collections::{HashMap, HashSet, VecDeque};

use crate::{
    PbftVoteIngressContext, PbftVoteIngressFact, PbftVoteIngressPlan, PbftVoteIngressStatus,
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
/// Network effect result did not match the effect it claims to acknowledge.
pub const NETWORK_EFFECT_ACK_STATUS_MISMATCHED_EFFECT_RESULT: u8 = 4;

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
/// Network effect asks the executor to record a consensus object.
pub const NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT: u8 = 8;

/// Network sync effect requests PBFT chain synchronization.
pub const NETWORK_SYNC_KIND_PBFT_CHAIN: u8 = 0;
/// Network sync effect requests current-round PBFT next votes.
pub const NETWORK_SYNC_KIND_PBFT_NEXT_VOTES: u8 = 1;

/// Network peer report/disconnect reason for unsupported propose votes in a bundle.
pub const NETWORK_REASON_UNSUPPORTED_BUNDLE_PROPOSE_VOTE: u8 = 0;
/// Network peer report reason for mixed vote identity in a bundle.
pub const NETWORK_REASON_BUNDLE_VOTE_MISMATCH: u8 = 1;

/// Network known-object effect identifies a PBFT vote hash.
pub const NETWORK_OBJECT_KIND_PBFT_VOTE: u8 = 0;
/// Network known-object effect identifies a PBFT block hash.
pub const NETWORK_OBJECT_KIND_PBFT_BLOCK: u8 = 1;
/// Network object effect identifies a transaction hash.
pub const NETWORK_OBJECT_KIND_TRANSACTION: u8 = 2;
/// Network object effect identifies a DAG block hash.
pub const NETWORK_OBJECT_KIND_DAG_BLOCK: u8 = 3;
/// Network object effect identifies PBFT period data keyed by PBFT block hash.
pub const NETWORK_OBJECT_KIND_PBFT_PERIOD_DATA: u8 = 4;
/// Network object effect identifies a pillar vote hash.
pub const NETWORK_OBJECT_KIND_PILLAR_VOTE: u8 = 5;

/// Network packet effect identifies the latest PBFT vote packet.
pub const NETWORK_PACKET_KIND_PBFT_VOTE: u32 = 1;
/// Network packet effect identifies the latest DAG block packet.
pub const NETWORK_PACKET_KIND_DAG_BLOCK: u32 = 5;
/// Network packet effect identifies the latest DAG sync packet.
pub const NETWORK_PACKET_KIND_DAG_SYNC: u32 = 6;
/// Network packet effect identifies the latest transaction packet.
pub const NETWORK_PACKET_KIND_TRANSACTION: u32 = 7;
/// Network packet effect identifies the latest PBFT sync packet.
pub const NETWORK_PACKET_KIND_PBFT_SYNC: u32 = 11;
/// Network packet effect identifies the latest pillar vote packet.
pub const NETWORK_PACKET_KIND_PILLAR_VOTE: u32 = 13;
/// Network packet effect identifies the latest pillar votes bundle packet.
pub const NETWORK_PACKET_KIND_PILLAR_VOTES_BUNDLE: u32 = 15;
/// Network packet effect identifies the latest PBFT blocks bundle packet.
pub const NETWORK_PACKET_KIND_PBFT_BLOCKS_BUNDLE: u32 = 16;

const ERROR_NONE: &str = "";
const ERROR_REJECTED_EMPTY_PAYLOAD: &str = "NETWORK_INGRESS_REJECTED_EMPTY_PAYLOAD";
const ERROR_UNSUPPORTED_PACKET_TYPE: &str = "NETWORK_INGRESS_UNSUPPORTED_PACKET_TYPE";
const ERROR_PAYLOAD_TOO_LARGE: &str = "NETWORK_INGRESS_PAYLOAD_TOO_LARGE";
const ERROR_QUEUE_FULL: &str = "NETWORK_INGRESS_QUEUE_FULL";
const ERROR_PAYLOAD_ID_EXHAUSTED: &str = "NETWORK_INGRESS_PAYLOAD_ID_EXHAUSTED";
const ERROR_UNKNOWN_EFFECT_ID: &str = "NETWORK_EFFECT_RESULT_UNKNOWN_EFFECT_ID";
const ERROR_DUPLICATE_EFFECT_RESULT: &str = "NETWORK_EFFECT_RESULT_DUPLICATE_EFFECT_ID";
const ERROR_INVALID_RESULT_STATUS: &str = "NETWORK_EFFECT_RESULT_INVALID_STATUS";
const ERROR_MISMATCHED_EFFECT_RESULT: &str = "NETWORK_EFFECT_RESULT_MISMATCHED_EFFECT";

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
    /// Effect kind that the executor attempted.
    pub kind: u8,
    /// Target peer id used by the executor.
    pub peer_id: [u8; 64],
    /// Packet kind used by send/gossip effects.
    pub packet_kind: u32,
    /// Object kind used by known-object or record-object effects.
    pub object_kind: u8,
    /// Object hash used by known-object or record-object effects.
    pub object_hash: [u8; 32],
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

/// Scalar context for authoritative PBFT vote ingress through the network API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkPbftVoteIngressContext {
    /// Existing side-effect-free PBFT vote ingress context.
    pub ingress: PbftVoteIngressContext,
    /// Sending peer id. The network executor uses this for sync/report effects.
    pub peer_id: [u8; 64],
    /// Peer PBFT chain size known by tarcap at ingress time.
    pub peer_pbft_chain_size: u64,
    /// Optional retained packet payload id when this decision follows
    /// [`ConsensusNetworkApi::ingest_packet`].
    pub source_payload_id: u64,
}

/// Result of routing PBFT vote ingress through the network/tarcap API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkIngressDecision {
    /// Retained payload id associated with the decision, or zero when the
    /// caller has not associated one yet.
    pub payload_id: u64,
    /// Whether canonical bytes were already accepted at the byte-ingress stage.
    pub payload_accepted: bool,
    /// Whether this API recognized and routed the packet-specific decision.
    pub routed: bool,
    /// Stable packet-specific status code.
    pub status: u8,
    /// Stable textual status for boundary logs and tests.
    pub error_code: String,
    /// Number of network effects queued by this decision.
    pub queued_effect_count: u32,
}

/// Accepted-vote network effects derived after verified-vote admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftVoteAdmissionEffects {
    /// Peer that supplied the accepted vote.
    pub peer_id: [u8; 64],
    /// Accepted vote hash.
    pub vote_hash: [u8; 32],
    /// Optional retained packet payload id.
    pub source_payload_id: u64,
    /// Whether the network executor should mark the vote as known for the peer.
    pub mark_vote_known: bool,
}

/// Accepted-ingress vote admission request for the external executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftVoteAdmissionRequestEffects {
    /// Peer that supplied the vote.
    pub peer_id: [u8; 64],
    /// Vote hash to admit through the temporary verified-vote executor.
    pub vote_hash: [u8; 32],
    /// Optional retained packet payload id.
    pub source_payload_id: u64,
    /// Whether the executor should run verified-vote admission for this vote.
    pub admit_vote: bool,
}

/// Accepted-PBFT-block network effects derived after verified-vote admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftBlockAdmissionEffects {
    /// Peer that supplied the accepted vote with the block sidecar.
    pub peer_id: [u8; 64],
    /// Accepted PBFT block hash attached to the vote packet.
    pub block_hash: [u8; 32],
    /// Optional retained packet payload id.
    pub source_payload_id: u64,
    /// Whether the network executor should mark the block as known for the peer.
    pub mark_block_known: bool,
}

/// Accepted-vote gossip effects derived after verified-vote admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftVoteGossipEffects {
    /// Peer that supplied the accepted vote. The network executor may use this
    /// identity for exclusion or peer-cache checks.
    pub peer_id: [u8; 64],
    /// Accepted vote hash used to correlate the gossip intent with the live
    /// vote object at the network boundary.
    pub vote_hash: [u8; 32],
    /// Optional retained packet payload id.
    pub source_payload_id: u64,
    /// Whether the network executor should gossip the accepted vote.
    pub gossip_vote: bool,
}

/// Proposed-block sidecar effects derived from accepted PBFT vote packets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftProposedBlockSidecarEffects {
    /// Peer that supplied the vote packet carrying the proposed block.
    pub peer_id: [u8; 64],
    /// PBFT period decoded from the proposed block.
    pub period: u64,
    /// Proposed PBFT block hash.
    pub block_hash: [u8; 32],
    /// Pivot DAG block hash decoded from the proposed block.
    pub pivot_hash: [u8; 32],
    /// Canonical signed PBFT block RLP.
    pub block_rlp: Vec<u8>,
    /// Optional retained packet payload id.
    pub source_payload_id: u64,
    /// Whether the executor should record the sidecar in proposed-block state.
    pub record_block: bool,
}

/// PBFT sync period-data admission request supplied by network/tarcap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncPeriodDataAdmissionRequestEffects {
    /// Peer that supplied the PBFT sync packet.
    pub peer_id: [u8; 64],
    /// PBFT block hash decoded from the period data.
    pub block_hash: [u8; 32],
    /// PBFT period decoded from the period data.
    pub period: u64,
    /// Canonical period-data RLP.
    pub period_data_rlp: Vec<u8>,
    /// Number of current-block certificate votes supplied beside the period
    /// data. The executor validates this against the live vote vector before
    /// enqueueing period data.
    pub current_block_cert_vote_count: u64,
    /// Optional retained packet payload id.
    pub source_payload_id: u64,
    /// Whether the executor should enqueue period data for PBFT processing.
    pub admit_period_data: bool,
}

/// Pillar vote admission request supplied by network/tarcap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPillarVoteAdmissionRequestEffects {
    /// Peer that supplied the pillar vote.
    pub peer_id: [u8; 64],
    /// Pillar vote hash decoded by the network boundary.
    pub vote_hash: [u8; 32],
    /// Pillar period decoded from the vote.
    pub period: u64,
    /// Canonical pillar vote RLP.
    pub vote_rlp: Vec<u8>,
    /// Optional retained packet payload id.
    pub source_payload_id: u64,
    /// Whether the executor should run verified pillar-vote admission.
    pub admit_vote: bool,
}

/// Transaction admission request supplied by network/tarcap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkTransactionAdmissionRequestEffects {
    /// Peer that supplied the transaction packet.
    pub peer_id: [u8; 64],
    /// Transaction hash decoded by the network boundary.
    pub transaction_hash: [u8; 32],
    /// Canonical transaction RLP.
    pub transaction_rlp: Vec<u8>,
    /// Optional retained packet payload id.
    pub source_payload_id: u64,
    /// Whether the executor should run transaction-pool admission.
    pub admit_transaction: bool,
}

/// DAG block admission request supplied by network/tarcap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkDagBlockAdmissionRequestEffects {
    /// Peer that supplied the DAG block packet.
    pub peer_id: [u8; 64],
    /// DAG block hash decoded by the network boundary.
    pub block_hash: [u8; 32],
    /// Canonical signed DAG block RLP.
    pub block_rlp: Vec<u8>,
    /// Number of transactions supplied with the packet.
    pub transaction_count: u64,
    /// Optional retained packet payload id.
    pub source_payload_id: u64,
    /// Whether the executor should run DAG block admission.
    pub admit_block: bool,
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
    outstanding_effects: HashMap<u64, NetworkEffect>,
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
            outstanding_effects: HashMap::new(),
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
            self.outstanding_effects
                .insert(effect.effect_id, effect.clone());
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
            let Some(effect) = self.outstanding_effects.get(&result.effect_id) else {
                status = NETWORK_EFFECT_ACK_STATUS_UNKNOWN_EFFECT_ID;
                error_code = ERROR_UNKNOWN_EFFECT_ID;
                break;
            };
            if !effect_result_matches_effect(result, effect) {
                status = NETWORK_EFFECT_ACK_STATUS_MISMATCHED_EFFECT_RESULT;
                error_code = ERROR_MISMATCHED_EFFECT_RESULT;
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

    /// Routes one PBFT vote ingress decision and queues network effects.
    ///
    /// This is the first authoritative production path behind the external
    /// network/tarcap facade. It still accepts decoded scalar facts while Rust
    /// packet decoding is pending, but sync hints are converted into typed
    /// effects that the network executor drains and reports through this API.
    pub fn ingest_pbft_vote(
        &mut self,
        fact: PbftVoteIngressFact,
        context: NetworkPbftVoteIngressContext,
    ) -> NetworkIngressDecision {
        let plan = self.plan_pbft_vote_ingress(fact, context.ingress);
        self.decision_from_vote_plan(plan, fact, context)
    }

    /// Routes one PBFT vote-bundle member decision and queues network effects.
    ///
    /// Bundle-shape rejections become report/disconnect effects instead of
    /// direct tarcap side effects. Accepted votes still proceed to the existing
    /// admission boundary until a later slice moves verified-vote mutation
    /// behind this facade.
    pub fn ingest_pbft_vote_bundle_member(
        &mut self,
        reference: PbftVoteIngressFact,
        vote: PbftVoteIngressFact,
        context: NetworkPbftVoteIngressContext,
    ) -> NetworkIngressDecision {
        let plan = self.plan_pbft_vote_bundle_ingress(reference, vote, context.ingress);
        self.decision_from_vote_plan(plan, vote, context)
    }

    /// Queues network effects derived from accepted PBFT vote admission.
    ///
    /// The admission mutation itself remains outside this facade for now. This
    /// method lets tarcap stop executing the resulting network cache mutation
    /// directly and instead use the same drain/report executor path as rejected
    /// ingress decisions.
    pub fn queue_pbft_vote_admission_effects(
        &mut self,
        effects: NetworkPbftVoteAdmissionEffects,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        if effects.mark_vote_known {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: effects.source_payload_id,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: effects.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                object_hash: effects.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }

        NetworkIngressDecision {
            payload_id: effects.source_payload_id,
            payload_accepted: effects.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
        }
    }

    /// Queues the verified-vote admission request for an accepted PBFT vote.
    ///
    /// The network facade owns the admission request identity and result-report
    /// contract. The current executor still calls the temporary VoteManager
    /// boundary until verified-vote mutation is fully owned behind this facade.
    pub fn queue_pbft_vote_admission_request_effects(
        &mut self,
        effects: NetworkPbftVoteAdmissionRequestEffects,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        if effects.admit_vote {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: effects.source_payload_id,
                kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
                peer_id: effects.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PBFT_VOTE,
                payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                object_hash: effects.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }

        NetworkIngressDecision {
            payload_id: effects.source_payload_id,
            payload_accepted: effects.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
        }
    }

    /// Queues network effects derived from accepted PBFT block sidecars.
    ///
    /// Vote packets may include the PBFT block voted for. Tarcap still owns
    /// the peer object cache during this migration stage, but Rust owns the
    /// decision to request that external peer-cache mutation through the same
    /// effect queue used by vote admission and rejected ingress paths.
    pub fn queue_pbft_block_admission_effects(
        &mut self,
        effects: NetworkPbftBlockAdmissionEffects,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        if effects.mark_block_known {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: effects.source_payload_id,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: effects.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_BLOCK,
                object_hash: effects.block_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }

        NetworkIngressDecision {
            payload_id: effects.source_payload_id,
            payload_accepted: effects.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
        }
    }

    /// Queues network effects derived from accepted PBFT vote gossip.
    ///
    /// Rust owns the decision that an accepted vote should be gossiped. The
    /// network executor still owns peer filtering, packet wrapping, and
    /// transport, so this effect carries only the stable packet kind and vote
    /// identity needed to execute the live boundary action.
    pub fn queue_pbft_vote_gossip_effects(
        &mut self,
        effects: NetworkPbftVoteGossipEffects,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        if effects.gossip_vote {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: effects.source_payload_id,
                kind: NETWORK_EFFECT_KIND_GOSSIP_PACKET,
                peer_id: effects.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PBFT_VOTE,
                payload_bytes: Vec::new(),
                exclude_peers: vec![effects.peer_id],
                object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                object_hash: effects.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }

        NetworkIngressDecision {
            payload_id: effects.source_payload_id,
            payload_accepted: effects.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
        }
    }

    /// Queues effects for a proposed PBFT block carried beside an accepted vote.
    ///
    /// The network/tarcap boundary supplies canonical block bytes and compact
    /// decoded facts. Rust owns the sidecar routing decision and returns a
    /// typed record-object effect; the temporary C++ executor still performs
    /// live PBFT manager insertion until the network API is wired to the shared
    /// Rust proposed-block runtime.
    pub fn queue_pbft_proposed_block_sidecar_effects(
        &mut self,
        effects: NetworkPbftProposedBlockSidecarEffects,
    ) -> NetworkIngressDecision {
        self.queue_pbft_proposed_block_record_effects(effects, NETWORK_PACKET_KIND_PBFT_VOTE)
    }

    /// Queues effects for proposed PBFT blocks received in a PBFT blocks bundle.
    ///
    /// The network/tarcap boundary supplies canonical block bytes and compact
    /// decoded facts after bundle-level peer, period, and author checks. Rust
    /// owns the record-object effect identity for this external route; the
    /// temporary C++ executor still performs live PBFT manager insertion until
    /// the facade is wired to the shared Rust proposed-block runtime.
    pub fn queue_pbft_proposed_block_bundle_effects(
        &mut self,
        effects: NetworkPbftProposedBlockSidecarEffects,
    ) -> NetworkIngressDecision {
        self.queue_pbft_proposed_block_record_effects(
            effects,
            NETWORK_PACKET_KIND_PBFT_BLOCKS_BUNDLE,
        )
    }

    fn queue_pbft_proposed_block_record_effects(
        &mut self,
        effects: NetworkPbftProposedBlockSidecarEffects,
        packet_kind: u32,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        if effects.record_block {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: effects.source_payload_id,
                kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
                peer_id: effects.peer_id,
                packet_kind,
                payload_bytes: effects.block_rlp,
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_BLOCK,
                object_hash: effects.block_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: effects.period,
                round: 0,
            });
        }

        NetworkIngressDecision {
            payload_id: effects.source_payload_id,
            payload_accepted: effects.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
        }
    }

    /// Queues PBFT sync period-data admission for external execution.
    ///
    /// Rust owns the period-data admission request identity and effect result
    /// contract for PBFT sync packets. The live period-data queue mutation
    /// remains a temporary C++ executor boundary until PBFT sync intake is
    /// backed by the Rust PBFT manager runtime.
    pub fn queue_pbft_sync_period_data_admission_request_effects(
        &mut self,
        effects: NetworkPbftSyncPeriodDataAdmissionRequestEffects,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        if effects.admit_period_data {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: effects.source_payload_id,
                kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
                peer_id: effects.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PBFT_SYNC,
                payload_bytes: effects.period_data_rlp,
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_PERIOD_DATA,
                object_hash: effects.block_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: effects.current_block_cert_vote_count,
                period: effects.period,
                round: 0,
            });
        }

        NetworkIngressDecision {
            payload_id: effects.source_payload_id,
            payload_accepted: effects.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
        }
    }

    /// Queues verified pillar-vote admission for a single pillar vote packet.
    ///
    /// Rust owns the admission request identity and effect result contract.
    /// The live pillar-chain insertion remains a temporary C++ executor
    /// boundary until pillar vote intake is backed by the Rust pillar runtime.
    pub fn queue_pillar_vote_admission_request_effects(
        &mut self,
        effects: NetworkPillarVoteAdmissionRequestEffects,
    ) -> NetworkIngressDecision {
        self.queue_pillar_vote_record_effects(effects, NETWORK_PACKET_KIND_PILLAR_VOTE)
    }

    /// Queues verified pillar-vote admission for a pillar votes bundle member.
    ///
    /// The packet kind is distinct from single-vote admission so external
    /// executor reports can identify which tarcap route supplied the vote.
    pub fn queue_pillar_vote_bundle_member_admission_request_effects(
        &mut self,
        effects: NetworkPillarVoteAdmissionRequestEffects,
    ) -> NetworkIngressDecision {
        self.queue_pillar_vote_record_effects(effects, NETWORK_PACKET_KIND_PILLAR_VOTES_BUNDLE)
    }

    fn queue_pillar_vote_record_effects(
        &mut self,
        effects: NetworkPillarVoteAdmissionRequestEffects,
        packet_kind: u32,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        if effects.admit_vote {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: effects.source_payload_id,
                kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
                peer_id: effects.peer_id,
                packet_kind,
                payload_bytes: effects.vote_rlp,
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                object_hash: effects.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: effects.period,
                round: 0,
            });
        }

        NetworkIngressDecision {
            payload_id: effects.source_payload_id,
            payload_accepted: effects.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
        }
    }

    /// Queues transaction-pool admission for a transaction received from tarcap.
    ///
    /// Rust owns the transaction admission request identity and effect result
    /// contract. The live transaction-pool verification and insertion remain a
    /// temporary C++ executor boundary until transaction gossip admission is
    /// backed by a Rust transaction runtime/storage handle.
    pub fn queue_transaction_admission_request_effects(
        &mut self,
        effects: NetworkTransactionAdmissionRequestEffects,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        if effects.admit_transaction {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: effects.source_payload_id,
                kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
                peer_id: effects.peer_id,
                packet_kind: NETWORK_PACKET_KIND_TRANSACTION,
                payload_bytes: effects.transaction_rlp,
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_TRANSACTION,
                object_hash: effects.transaction_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }

        NetworkIngressDecision {
            payload_id: effects.source_payload_id,
            payload_accepted: effects.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
        }
    }

    /// Queues DAG block admission for a DAG block received from tarcap.
    ///
    /// Rust owns the DAG block admission request identity and effect result
    /// contract. The live DAG block verification, graph insertion, missing-data
    /// sync reaction, and peer penalty behavior remain a temporary C++ executor
    /// boundary until DAG intake is backed by the Rust DAG runtime/storage path.
    pub fn queue_dag_block_admission_request_effects(
        &mut self,
        effects: NetworkDagBlockAdmissionRequestEffects,
    ) -> NetworkIngressDecision {
        self.queue_dag_block_record_effects(effects, NETWORK_PACKET_KIND_DAG_BLOCK)
    }

    /// Queues DAG block admission for a DAG block received from DAG sync.
    ///
    /// Rust owns the DAG-sync block admission request identity and effect
    /// result contract. The live DAG sync verification and insertion behavior
    /// remain a temporary C++ executor boundary until DAG sync intake is backed
    /// by the Rust DAG runtime/storage path.
    pub fn queue_dag_sync_block_admission_request_effects(
        &mut self,
        effects: NetworkDagBlockAdmissionRequestEffects,
    ) -> NetworkIngressDecision {
        self.queue_dag_block_record_effects(effects, NETWORK_PACKET_KIND_DAG_SYNC)
    }

    fn queue_dag_block_record_effects(
        &mut self,
        effects: NetworkDagBlockAdmissionRequestEffects,
        packet_kind: u32,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        if effects.admit_block {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: effects.source_payload_id,
                kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
                peer_id: effects.peer_id,
                packet_kind,
                payload_bytes: effects.block_rlp,
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_DAG_BLOCK,
                object_hash: effects.block_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: effects.transaction_count,
                period: 0,
                round: 0,
            });
        }

        NetworkIngressDecision {
            payload_id: effects.source_payload_id,
            payload_accepted: effects.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
        }
    }

    fn decision_from_vote_plan(
        &mut self,
        plan: PbftVoteIngressPlan,
        fact: PbftVoteIngressFact,
        context: NetworkPbftVoteIngressContext,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        self.enqueue_vote_plan_effects(plan, fact, context);
        let queued_effect_count = self.pending_effects.len().saturating_sub(before_effects) as u32;

        NetworkIngressDecision {
            payload_id: context.source_payload_id,
            payload_accepted: context.source_payload_id != 0,
            routed: true,
            status: plan.status.as_u8(),
            error_code: pbft_vote_ingress_error_code(plan.status).to_owned(),
            queued_effect_count,
        }
    }

    fn enqueue_vote_plan_effects(
        &mut self,
        plan: PbftVoteIngressPlan,
        fact: PbftVoteIngressFact,
        context: NetworkPbftVoteIngressContext,
    ) {
        if plan.request_pbft_sync {
            let sync_start = fact
                .period
                .saturating_sub(1)
                .max(context.peer_pbft_chain_size);
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                kind: NETWORK_EFFECT_KIND_REQUEST_SYNC,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: 0,
                object_hash: [0; 32],
                sync_kind: NETWORK_SYNC_KIND_PBFT_CHAIN,
                sync_start,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }
        if plan.request_next_votes_sync {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                kind: NETWORK_EFFECT_KIND_REQUEST_SYNC,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: 0,
                object_hash: [0; 32],
                sync_kind: NETWORK_SYNC_KIND_PBFT_NEXT_VOTES,
                sync_start: context.ingress.current_period,
                reason_code: 0,
                dependency_id: 0,
                period: context.ingress.current_period,
                round: context.ingress.current_round,
            });
        }
        match plan.status {
            PbftVoteIngressStatus::UnsupportedBundleProposeVote => {
                self.enqueue_effect(NetworkEffect {
                    effect_id: 0,
                    source_payload_id: context.source_payload_id,
                    kind: NETWORK_EFFECT_KIND_REPORT_PEER,
                    peer_id: context.peer_id,
                    packet_kind: 0,
                    payload_bytes: Vec::new(),
                    exclude_peers: Vec::new(),
                    object_kind: 0,
                    object_hash: [0; 32],
                    sync_kind: 0,
                    sync_start: 0,
                    reason_code: NETWORK_REASON_UNSUPPORTED_BUNDLE_PROPOSE_VOTE,
                    dependency_id: 0,
                    period: 0,
                    round: 0,
                });
                self.enqueue_effect(NetworkEffect {
                    effect_id: 0,
                    source_payload_id: context.source_payload_id,
                    kind: NETWORK_EFFECT_KIND_DISCONNECT_PEER,
                    peer_id: context.peer_id,
                    packet_kind: 0,
                    payload_bytes: Vec::new(),
                    exclude_peers: Vec::new(),
                    object_kind: 0,
                    object_hash: [0; 32],
                    sync_kind: 0,
                    sync_start: 0,
                    reason_code: NETWORK_REASON_UNSUPPORTED_BUNDLE_PROPOSE_VOTE,
                    dependency_id: 0,
                    period: 0,
                    round: 0,
                });
            }
            PbftVoteIngressStatus::BundleVoteMismatch => {
                self.enqueue_effect(NetworkEffect {
                    effect_id: 0,
                    source_payload_id: context.source_payload_id,
                    kind: NETWORK_EFFECT_KIND_REPORT_PEER,
                    peer_id: context.peer_id,
                    packet_kind: 0,
                    payload_bytes: Vec::new(),
                    exclude_peers: Vec::new(),
                    object_kind: 0,
                    object_hash: [0; 32],
                    sync_kind: 0,
                    sync_start: 0,
                    reason_code: NETWORK_REASON_BUNDLE_VOTE_MISMATCH,
                    dependency_id: 0,
                    period: 0,
                    round: 0,
                });
            }
            _ => {}
        }
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
    // `kVotePacket = 1`, `kVotesBundlePacket = 3`, `kDagBlockPacket = 5`,
    // `kDagSyncPacket = 6`, `kTransactionPacket = 7`,
    // `kPbftSyncPacket = 11`, `kPillarVotePacket = 13`,
    // `kPillarVotesBundlePacket = 15`, and `kPbftBlocksBundlePacket = 16`.
    matches!(
        packet_type,
        1 | 3
            | NETWORK_PACKET_KIND_DAG_BLOCK
            | NETWORK_PACKET_KIND_DAG_SYNC
            | NETWORK_PACKET_KIND_TRANSACTION
            | NETWORK_PACKET_KIND_PBFT_SYNC
            | NETWORK_PACKET_KIND_PILLAR_VOTE
            | NETWORK_PACKET_KIND_PILLAR_VOTES_BUNDLE
            | NETWORK_PACKET_KIND_PBFT_BLOCKS_BUNDLE
    )
}

fn effect_result_matches_effect(result: &NetworkEffectResult, effect: &NetworkEffect) -> bool {
    result.kind == effect.kind
        && result.peer_id == effect.peer_id
        && result.packet_kind == effect.packet_kind
        && result.object_kind == effect.object_kind
        && result.object_hash == effect.object_hash
}

const fn pbft_vote_ingress_error_code(status: PbftVoteIngressStatus) -> &'static str {
    match status {
        PbftVoteIngressStatus::Accepted => "",
        PbftVoteIngressStatus::Irrelevant => "PBFT_VOTE_INGRESS_IRRELEVANT",
        PbftVoteIngressStatus::InvalidPeriodTooSmall => {
            "PBFT_VOTE_INGRESS_INVALID_PERIOD_TOO_SMALL"
        }
        PbftVoteIngressStatus::InvalidPeriodTooBig => "PBFT_VOTE_INGRESS_INVALID_PERIOD_TOO_BIG",
        PbftVoteIngressStatus::InvalidRoundTooSmall => "PBFT_VOTE_INGRESS_INVALID_ROUND_TOO_SMALL",
        PbftVoteIngressStatus::InvalidRoundTooBig => "PBFT_VOTE_INGRESS_INVALID_ROUND_TOO_BIG",
        PbftVoteIngressStatus::InvalidStepTooBig => "PBFT_VOTE_INGRESS_INVALID_STEP_TOO_BIG",
        PbftVoteIngressStatus::UnsupportedBundleProposeVote => {
            "PBFT_VOTE_INGRESS_UNSUPPORTED_BUNDLE_PROPOSE_VOTE"
        }
        PbftVoteIngressStatus::BundleVoteMismatch => "PBFT_VOTE_INGRESS_BUNDLE_VOTE_MISMATCH",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verified_votes::PbftVoteType;

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

    const fn vote_fact(
        period: u64,
        round: u64,
        step: u64,
        vote_type: PbftVoteType,
    ) -> PbftVoteIngressFact {
        PbftVoteIngressFact {
            period,
            round,
            step,
            vote_type,
        }
    }

    fn vote_context() -> NetworkPbftVoteIngressContext {
        NetworkPbftVoteIngressContext {
            ingress: PbftVoteIngressContext {
                current_period: 10,
                current_round: 3,
                current_step: 2,
                max_future_period_delta: 2,
                max_future_round_delta: 2,
                max_future_step_delta: 2,
                validate_max_round_step: true,
                source_peer_is_voter: true,
                can_request_pbft_sync: true,
                can_request_next_votes_sync: true,
            },
            peer_id: peer(7),
            peer_pbft_chain_size: 11,
            source_payload_id: 99,
        }
    }

    fn hash(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn effect_result(effect: &NetworkEffect, status: u8) -> NetworkEffectResult {
        NetworkEffectResult {
            effect_id: effect.effect_id,
            kind: effect.kind,
            peer_id: effect.peer_id,
            packet_kind: effect.packet_kind,
            object_kind: effect.object_kind,
            object_hash: effect.object_hash,
            status,
            diagnostic: String::new(),
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
                kind: 0,
                peer_id: [0; 64],
                packet_kind: 0,
                object_kind: 0,
                object_hash: [0; 32],
                status: NETWORK_EFFECT_RESULT_STATUS_OK,
                diagnostic: String::new(),
            },
            NetworkEffectResult {
                effect_id: 2,
                kind: 0,
                peer_id: [0; 64],
                packet_kind: 0,
                object_kind: 0,
                object_hash: [0; 32],
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

        let ack = api.report_effect_results(vec![effect_result(
            &batch.effects[0],
            NETWORK_EFFECT_RESULT_STATUS_OK,
        )]);

        assert_eq!(ack.status, NETWORK_EFFECT_ACK_STATUS_ACCEPTED);
        assert_eq!(ack.accepted_results, 1);
        assert_eq!(ack.failed_results, 0);
        assert_eq!(ack.error_code, "");
        assert_eq!(api.effect_result_len(), 1);
    }

    #[test]
    fn report_effect_results_rejects_mismatched_effect_identity() {
        let mut api = ConsensusNetworkApi::new();
        api.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: 1,
            kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
            peer_id: peer(1),
            packet_kind: 0,
            payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
            object_hash: hash(0xAA),
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id: 0,
            period: 0,
            round: 0,
        });
        let batch = api.drain_work(1);
        let mut result = effect_result(&batch.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        result.object_hash = hash(0xBB);

        let ack = api.report_effect_results(vec![result]);

        assert_eq!(
            ack.status,
            NETWORK_EFFECT_ACK_STATUS_MISMATCHED_EFFECT_RESULT
        );
        assert_eq!(ack.accepted_results, 0);
        assert_eq!(ack.failed_results, 0);
        assert_eq!(ack.error_code, ERROR_MISMATCHED_EFFECT_RESULT);
        assert_eq!(api.effect_result_len(), 0);
    }

    #[test]
    fn ingest_pbft_vote_queues_pbft_chain_sync_effect() {
        let mut api = ConsensusNetworkApi::new();

        let decision =
            api.ingest_pbft_vote(vote_fact(14, 3, 1, PbftVoteType::Soft), vote_context());

        assert!(decision.routed);
        assert!(decision.payload_accepted);
        assert_eq!(decision.payload_id, 99);
        assert_eq!(
            decision.status,
            PbftVoteIngressStatus::InvalidPeriodTooBig.as_u8()
        );
        assert_eq!(
            decision.error_code,
            "PBFT_VOTE_INGRESS_INVALID_PERIOD_TOO_BIG"
        );
        assert_eq!(decision.queued_effect_count, 1);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 1);
        let effect = &batch.effects[0];
        assert_eq!(effect.kind, NETWORK_EFFECT_KIND_REQUEST_SYNC);
        assert_eq!(effect.peer_id, peer(7));
        assert_eq!(effect.sync_kind, NETWORK_SYNC_KIND_PBFT_CHAIN);
        assert_eq!(effect.sync_start, 13);
        assert_eq!(effect.source_payload_id, 99);
    }

    #[test]
    fn ingest_pbft_vote_bundle_member_queues_report_and_disconnect_for_propose_votes() {
        let mut api = ConsensusNetworkApi::new();

        let decision = api.ingest_pbft_vote_bundle_member(
            vote_fact(10, 3, 2, PbftVoteType::Propose),
            vote_fact(10, 3, 2, PbftVoteType::Propose),
            vote_context(),
        );

        assert_eq!(
            decision.status,
            PbftVoteIngressStatus::UnsupportedBundleProposeVote.as_u8()
        );
        assert_eq!(decision.queued_effect_count, 2);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 2);
        assert_eq!(batch.effects[0].kind, NETWORK_EFFECT_KIND_REPORT_PEER);
        assert_eq!(
            batch.effects[0].reason_code,
            NETWORK_REASON_UNSUPPORTED_BUNDLE_PROPOSE_VOTE
        );
        assert_eq!(batch.effects[1].kind, NETWORK_EFFECT_KIND_DISCONNECT_PEER);
        assert_eq!(
            batch.effects[1].reason_code,
            NETWORK_REASON_UNSUPPORTED_BUNDLE_PROPOSE_VOTE
        );
    }

    #[test]
    fn queue_pbft_vote_admission_effects_marks_vote_known() {
        let mut api = ConsensusNetworkApi::new();

        let decision = api.queue_pbft_vote_admission_effects(NetworkPbftVoteAdmissionEffects {
            peer_id: peer(8),
            vote_hash: hash(0xAB),
            source_payload_id: 77,
            mark_vote_known: true,
        });

        assert!(decision.routed);
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 1);
        assert_eq!(batch.effects[0].kind, NETWORK_EFFECT_KIND_MARK_PEER_KNOWN);
        assert_eq!(batch.effects[0].peer_id, peer(8));
        assert_eq!(batch.effects[0].object_kind, NETWORK_OBJECT_KIND_PBFT_VOTE);
        assert_eq!(batch.effects[0].object_hash, hash(0xAB));
        assert_eq!(batch.effects[0].source_payload_id, 77);
    }

    #[test]
    fn queue_pbft_vote_admission_request_effects_records_vote() {
        let mut api = ConsensusNetworkApi::new();

        let decision =
            api.queue_pbft_vote_admission_request_effects(NetworkPbftVoteAdmissionRequestEffects {
                peer_id: peer(12),
                vote_hash: hash(0xAC),
                source_payload_id: 81,
                admit_vote: true,
            });

        assert!(decision.routed);
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 1);
        assert_eq!(
            batch.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        assert_eq!(batch.effects[0].peer_id, peer(12));
        assert_eq!(batch.effects[0].packet_kind, NETWORK_PACKET_KIND_PBFT_VOTE);
        assert_eq!(batch.effects[0].object_kind, NETWORK_OBJECT_KIND_PBFT_VOTE);
        assert_eq!(batch.effects[0].object_hash, hash(0xAC));
        assert_eq!(batch.effects[0].source_payload_id, 81);
    }

    #[test]
    fn queue_pbft_block_admission_effects_marks_block_known() {
        let mut api = ConsensusNetworkApi::new();

        let decision = api.queue_pbft_block_admission_effects(NetworkPbftBlockAdmissionEffects {
            peer_id: peer(9),
            block_hash: hash(0xCD),
            source_payload_id: 78,
            mark_block_known: true,
        });

        assert!(decision.routed);
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 1);
        assert_eq!(batch.effects[0].kind, NETWORK_EFFECT_KIND_MARK_PEER_KNOWN);
        assert_eq!(batch.effects[0].peer_id, peer(9));
        assert_eq!(batch.effects[0].object_kind, NETWORK_OBJECT_KIND_PBFT_BLOCK);
        assert_eq!(batch.effects[0].object_hash, hash(0xCD));
        assert_eq!(batch.effects[0].source_payload_id, 78);
    }

    #[test]
    fn queue_pbft_sync_period_data_admission_request_effects_records_period_data() {
        let mut api = ConsensusNetworkApi::new();

        let decision = api.queue_pbft_sync_period_data_admission_request_effects(
            NetworkPbftSyncPeriodDataAdmissionRequestEffects {
                peer_id: peer(13),
                block_hash: hash(0xAE),
                period: 44,
                period_data_rlp: vec![0xC0, 0x06],
                current_block_cert_vote_count: 5,
                source_payload_id: 82,
                admit_period_data: true,
            },
        );

        assert!(decision.routed);
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 1);
        assert_eq!(
            batch.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        assert_eq!(batch.effects[0].peer_id, peer(13));
        assert_eq!(batch.effects[0].packet_kind, NETWORK_PACKET_KIND_PBFT_SYNC);
        assert_eq!(batch.effects[0].payload_bytes, vec![0xC0, 0x06]);
        assert_eq!(
            batch.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PBFT_PERIOD_DATA
        );
        assert_eq!(batch.effects[0].object_hash, hash(0xAE));
        assert_eq!(batch.effects[0].period, 44);
        assert_eq!(batch.effects[0].dependency_id, 5);
        assert_eq!(batch.effects[0].source_payload_id, 82);
    }

    #[test]
    fn queue_pillar_vote_admission_request_effects_records_vote() {
        let mut api = ConsensusNetworkApi::new();

        let decision = api.queue_pillar_vote_admission_request_effects(
            NetworkPillarVoteAdmissionRequestEffects {
                peer_id: peer(14),
                vote_hash: hash(0xAF),
                period: 45,
                vote_rlp: vec![0xC0, 0x07],
                source_payload_id: 83,
                admit_vote: true,
            },
        );

        assert!(decision.routed);
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 1);
        assert_eq!(
            batch.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        assert_eq!(batch.effects[0].peer_id, peer(14));
        assert_eq!(
            batch.effects[0].packet_kind,
            NETWORK_PACKET_KIND_PILLAR_VOTE
        );
        assert_eq!(batch.effects[0].payload_bytes, vec![0xC0, 0x07]);
        assert_eq!(
            batch.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PILLAR_VOTE
        );
        assert_eq!(batch.effects[0].object_hash, hash(0xAF));
        assert_eq!(batch.effects[0].period, 45);
        assert_eq!(batch.effects[0].source_payload_id, 83);
    }

    #[test]
    fn queue_pillar_vote_bundle_member_admission_request_effects_records_vote() {
        let mut api = ConsensusNetworkApi::new();

        let decision = api.queue_pillar_vote_bundle_member_admission_request_effects(
            NetworkPillarVoteAdmissionRequestEffects {
                peer_id: peer(15),
                vote_hash: hash(0xB0),
                period: 46,
                vote_rlp: vec![0xC0, 0x08],
                source_payload_id: 84,
                admit_vote: true,
            },
        );

        assert!(decision.routed);
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 1);
        assert_eq!(
            batch.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        assert_eq!(batch.effects[0].peer_id, peer(15));
        assert_eq!(
            batch.effects[0].packet_kind,
            NETWORK_PACKET_KIND_PILLAR_VOTES_BUNDLE
        );
        assert_eq!(batch.effects[0].payload_bytes, vec![0xC0, 0x08]);
        assert_eq!(
            batch.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PILLAR_VOTE
        );
        assert_eq!(batch.effects[0].object_hash, hash(0xB0));
        assert_eq!(batch.effects[0].period, 46);
        assert_eq!(batch.effects[0].source_payload_id, 84);
    }

    #[test]
    fn queue_pbft_vote_gossip_effects_gossips_vote_packet() {
        let mut api = ConsensusNetworkApi::new();

        let decision = api.queue_pbft_vote_gossip_effects(NetworkPbftVoteGossipEffects {
            peer_id: peer(10),
            vote_hash: hash(0xEF),
            source_payload_id: 79,
            gossip_vote: true,
        });

        assert!(decision.routed);
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 1);
        assert_eq!(batch.effects[0].kind, NETWORK_EFFECT_KIND_GOSSIP_PACKET);
        assert_eq!(batch.effects[0].peer_id, peer(10));
        assert_eq!(batch.effects[0].packet_kind, NETWORK_PACKET_KIND_PBFT_VOTE);
        assert_eq!(batch.effects[0].exclude_peers, vec![peer(10)]);
        assert_eq!(batch.effects[0].object_kind, NETWORK_OBJECT_KIND_PBFT_VOTE);
        assert_eq!(batch.effects[0].object_hash, hash(0xEF));
        assert_eq!(batch.effects[0].source_payload_id, 79);
    }

    #[test]
    fn queue_pbft_proposed_block_sidecar_effects_records_block() {
        let mut api = ConsensusNetworkApi::new();

        let decision =
            api.queue_pbft_proposed_block_sidecar_effects(NetworkPbftProposedBlockSidecarEffects {
                peer_id: peer(11),
                period: 42,
                block_hash: hash(0xA1),
                pivot_hash: hash(0xB2),
                block_rlp: vec![0xC0, 0x01],
                source_payload_id: 80,
                record_block: true,
            });

        assert!(decision.routed);
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);

        let batch = api.drain_work(10);
        assert_eq!(batch.effects.len(), 1);
        assert_eq!(
            batch.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        assert_eq!(batch.effects[0].peer_id, peer(11));
        assert_eq!(batch.effects[0].packet_kind, NETWORK_PACKET_KIND_PBFT_VOTE);
        assert_eq!(batch.effects[0].payload_bytes, vec![0xC0, 0x01]);
        assert_eq!(batch.effects[0].object_kind, NETWORK_OBJECT_KIND_PBFT_BLOCK);
        assert_eq!(batch.effects[0].object_hash, hash(0xA1));
        assert_eq!(batch.effects[0].period, 42);
        assert_eq!(batch.effects[0].source_payload_id, 80);
    }
}
