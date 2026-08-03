//! External network/tarcap facade for Rust-owned consensus routing.
//!
//! This module defines the narrow API that network/tarcap code should call
//! instead of reaching into consensus managers or C++ shim classes. The facade
//! accepts operation-specific packet facts and exposes an executor-facing typed
//! effect queue. It deliberately does not own peer transport, packet wrapping,
//! gossip fanout, disconnect execution, or tarcap scheduling.
//!
//! Inputs are packet-family facts, peer/source context, canonical object bytes,
//! and executor result reports. Outputs are stable routing decisions, ordered
//! network effects, and acknowledgement summaries. Canonical bytes live only in
//! the packet-family carrier or its queued effects; this API has no generic
//! shadow-ingress arena.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::pbft_chain::PbftChainService;
use crate::pbft_vote_payload::build_optimized_pbft_vote_bundle;
use crate::pbft_vote_runtime::{PbftNextVotesBundleEgressPayloads, PbftVerifiedVotesService};
use crate::pillar_chain_service::PillarChainService;
use crate::pillar_vote_service::PillarVoteRecord;
use crate::proposed_blocks::ProposedBlocksService;
use crate::{
    PbftVoteIngressContext, PbftVoteIngressFact, PbftVoteIngressPlan, PbftVoteIngressStatus,
    PbftVotePayloadRecord, inspect_canonical_pbft_vote, inspect_pillar_vote_from_rlp,
    plan_pbft_vote_bundle_ingress, plan_pbft_vote_ingress,
};
use anyhow::{Context, Result, anyhow, ensure};
use ethereum_types::H256;
use rlp::{Rlp, RlpStream};
use rustaxa_storage::Storage;
use rustaxa_types::{PillarVote, encode_optimized_pillar_votes_bundle_rlp};

const MAX_PILLAR_VOTES_PER_BUNDLE_PACKET: usize = 250;
const MAX_EFFECTS_PER_DRAIN: usize = 1024;

const ERROR_PILLAR_VOTE_INGRESS_MALFORMED_RLP: &str = "PILLAR_VOTE_INGRESS_MALFORMED_RLP";
const ERROR_PILLAR_VOTE_INGRESS_INVALID_CONTEXT: &str = "PILLAR_VOTE_INGRESS_INVALID_CONTEXT";
const NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_RLP: u8 = 4;
const NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_CONTEXT: u8 = 5;

/// Network/tarcap packet facts were accepted for operation-specific routing.
pub const NETWORK_INGRESS_STATUS_ACCEPTED: u8 = 0;
/// A get-next-votes request carries a different PBFT period.
pub const NETWORK_INGRESS_STATUS_NEXT_VOTES_PERIOD_MISMATCH: u8 = 1;
/// Local PBFT round one has no previous-round next-vote family.
pub const NETWORK_INGRESS_STATUS_NEXT_VOTES_NO_PREVIOUS_ROUND: u8 = 2;
/// The requester claims a PBFT round ahead of the local snapshot.
pub const NETWORK_INGRESS_STATUS_NEXT_VOTES_PEER_ROUND_AHEAD: u8 = 3;
/// A get-pillar-votes request predates Ficus activation.
pub const NETWORK_INGRESS_STATUS_PILLAR_VOTES_INACTIVE: u8 = 6;
/// A get-pillar-votes request does not target an exact pillar PBFT period.
pub const NETWORK_INGRESS_STATUS_PILLAR_VOTES_INVALID_PERIOD: u8 = 7;
/// A valid pillar-vote request found no live or stored votes.
pub const NETWORK_INGRESS_STATUS_PILLAR_VOTES_NO_DATA: u8 = 8;
/// A native sibling lookup failed locally without implicating the peer.
pub const NETWORK_INGRESS_STATUS_LOCAL_LOOKUP_FAILED: u8 = 9;
/// A native sibling returned payloads that violated the network contract.
pub const NETWORK_INGRESS_STATUS_INVALID_NATIVE_RESULT: u8 = 10;

/// Network status/sync planner accepted the facts.
pub const NETWORK_STATUS_PLAN_STATUS_OK: u8 = 0;
/// Network status/sync planner found PBFT sync already active.
pub const NETWORK_STATUS_PLAN_STATUS_ALREADY_SYNCING: u8 = 1;
/// Network status/sync planner found no usable peer candidate.
pub const NETWORK_STATUS_PLAN_STATUS_NO_ELIGIBLE_PEER: u8 = 2;
/// Network status/sync planner found no sync work needed.
pub const NETWORK_STATUS_PLAN_STATUS_SYNC_NOT_NEEDED: u8 = 3;
/// Network status/sync planner found the peer already DAG-synced.
pub const NETWORK_STATUS_PLAN_STATUS_DAG_ALREADY_SYNCED: u8 = 4;
/// Network status/sync planner found the peer's PBFT period does not match local sync period.
pub const NETWORK_STATUS_PLAN_STATUS_DAG_PERIOD_MISMATCH: u8 = 5;
/// Network status/sync planner found a peer chain-id mismatch.
pub const NETWORK_STATUS_PLAN_STATUS_CHAIN_ID_MISMATCH: u8 = 6;
/// Network status/sync planner found a peer genesis hash mismatch.
pub const NETWORK_STATUS_PLAN_STATUS_GENESIS_MISMATCH: u8 = 7;
/// Network status/sync planner found a light node that cannot serve local history.
pub const NETWORK_STATUS_PLAN_STATUS_LIGHT_NODE_HISTORY_UNAVAILABLE: u8 = 8;

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
/// Network effect asks the executor to publish one consensus object through its narrow application boundary.
pub const NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT: u8 = 8;
/// Network effect asks tarcap to clear the target peer's syncing flag.
pub const NETWORK_EFFECT_KIND_CLEAR_PEER_SYNCING: u8 = 9;

/// Network sync effect requests PBFT chain synchronization.
pub const NETWORK_SYNC_KIND_PBFT_CHAIN: u8 = 0;
/// Network sync effect requests current-round PBFT next votes.
pub const NETWORK_SYNC_KIND_PBFT_NEXT_VOTES: u8 = 1;

/// Network peer report/disconnect reason for unsupported propose votes in a bundle.
pub const NETWORK_REASON_UNSUPPORTED_BUNDLE_PROPOSE_VOTE: u8 = 0;
/// Network peer report reason for mixed vote identity in a bundle.
pub const NETWORK_REASON_BUNDLE_VOTE_MISMATCH: u8 = 1;
/// Network peer report/disconnect reason for an invalid pillar-vote request schedule.
pub const NETWORK_REASON_INVALID_PILLAR_VOTES_REQUEST: u8 = 2;
/// Network peer report/disconnect reason for an invalid PBFT sync range.
pub const NETWORK_REASON_INVALID_PBFT_SYNC_REQUEST: u8 = 3;

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
/// Network object effect identifies a pillar vote validation request.
pub const NETWORK_OBJECT_KIND_PILLAR_VOTE_VALIDATION: u8 = 6;
/// Network object effect identifies a PBFT sync egress request.
pub const NETWORK_OBJECT_KIND_PBFT_SYNC_EGRESS_REQUEST: u8 = 8;
/// Network object effect identifies a DAG sync egress request.
pub const NETWORK_OBJECT_KIND_DAG_SYNC_EGRESS_REQUEST: u8 = 10;

/// Network packet effect identifies the latest PBFT vote packet.
pub const NETWORK_PACKET_KIND_PBFT_VOTE: u32 = 1;
/// Network packet effect identifies the latest optimized PBFT votes bundle.
pub const NETWORK_PACKET_KIND_PBFT_VOTES_BUNDLE: u32 = 3;
/// Network packet effect identifies the latest get-next-votes sync packet.
pub const NETWORK_PACKET_KIND_GET_NEXT_VOTES_SYNC: u32 = 2;
/// Network packet effect identifies the latest DAG block packet.
pub const NETWORK_PACKET_KIND_DAG_BLOCK: u32 = 5;
/// Network packet effect identifies the latest DAG sync packet.
pub const NETWORK_PACKET_KIND_DAG_SYNC: u32 = 6;
/// Network packet effect identifies the latest transaction packet.
pub const NETWORK_PACKET_KIND_TRANSACTION: u32 = 7;
/// Network packet effect identifies the latest get-PBFT-sync packet.
pub const NETWORK_PACKET_KIND_GET_PBFT_SYNC: u32 = 10;
/// Network packet effect identifies the latest PBFT sync packet.
pub const NETWORK_PACKET_KIND_PBFT_SYNC: u32 = 11;
/// Network packet effect identifies the latest get-DAG-sync packet.
pub const NETWORK_PACKET_KIND_GET_DAG_SYNC: u32 = 12;
/// Network packet effect identifies the latest pillar vote packet.
pub const NETWORK_PACKET_KIND_PILLAR_VOTE: u32 = 13;
/// Network packet effect identifies the latest get-pillar-votes-bundle packet.
pub const NETWORK_PACKET_KIND_GET_PILLAR_VOTES_BUNDLE: u32 = 14;
/// Network packet effect identifies the latest pillar votes bundle packet.
pub const NETWORK_PACKET_KIND_PILLAR_VOTES_BUNDLE: u32 = 15;
/// Network packet effect identifies the latest PBFT blocks bundle packet.
pub const NETWORK_PACKET_KIND_PBFT_BLOCKS_BUNDLE: u32 = 16;

const ERROR_NONE: &str = "";
const ERROR_UNKNOWN_EFFECT_ID: &str = "NETWORK_EFFECT_RESULT_UNKNOWN_EFFECT_ID";
const ERROR_DUPLICATE_EFFECT_RESULT: &str = "NETWORK_EFFECT_RESULT_DUPLICATE_EFFECT_ID";
const ERROR_INVALID_RESULT_STATUS: &str = "NETWORK_EFFECT_RESULT_INVALID_STATUS";
const ERROR_MISMATCHED_EFFECT_RESULT: &str = "NETWORK_EFFECT_RESULT_MISMATCHED_EFFECT";
const ERROR_NEXT_VOTES_PERIOD_MISMATCH: &str = "NETWORK_NEXT_VOTES_PERIOD_MISMATCH";
const ERROR_NEXT_VOTES_NO_PREVIOUS_ROUND: &str = "NETWORK_NEXT_VOTES_NO_PREVIOUS_ROUND";
const ERROR_NEXT_VOTES_PEER_ROUND_AHEAD: &str = "NETWORK_NEXT_VOTES_PEER_ROUND_AHEAD";
const ERROR_PILLAR_VOTES_INACTIVE: &str = "NETWORK_PILLAR_VOTES_REQUEST_BEFORE_FICUS";
const ERROR_PILLAR_VOTES_INVALID_PERIOD: &str = "NETWORK_PILLAR_VOTES_REQUEST_INVALID_PERIOD";
const ERROR_PILLAR_VOTES_NO_DATA: &str = "NETWORK_PILLAR_VOTES_NO_DATA";
const ERROR_NEXT_VOTES_LOOKUP_FAILED: &str = "NETWORK_NEXT_VOTES_LOCAL_LOOKUP_FAILED";
const ERROR_NEXT_VOTES_INVALID_NATIVE_RESULT: &str = "NETWORK_NEXT_VOTES_INVALID_NATIVE_RESULT";
const ERROR_PILLAR_VOTES_LOOKUP_FAILED: &str = "NETWORK_PILLAR_VOTES_LOCAL_LOOKUP_FAILED";
const ERROR_PILLAR_VOTES_INVALID_NATIVE_RESULT: &str = "NETWORK_PILLAR_VOTES_INVALID_NATIVE_RESULT";
const MAX_VOTES_PER_BUNDLE_PACKET: usize = 1000;
const MAX_PROPOSED_BLOCKS_PER_BUNDLE_PACKET: usize = 10;
const TARCAP_VERSION_5: u32 = 5;
const TARCAP_VERSION_6: u32 = 6;

/// A get-PBFT-sync request is not canonical one-field RLP.
pub const NETWORK_INGRESS_STATUS_PBFT_SYNC_MALFORMED_REQUEST: u8 = 11;
/// A get-PBFT-sync request arrived on an unsupported tarcap version.
pub const NETWORK_INGRESS_STATUS_PBFT_SYNC_UNSUPPORTED_VERSION: u8 = 12;
/// A peer requested a period ahead of the local finalized chain.
pub const NETWORK_INGRESS_STATUS_PBFT_SYNC_HEIGHT_AHEAD: u8 = 13;
/// A light node cannot serve the requested historical period.
pub const NETWORK_INGRESS_STATUS_PBFT_SYNC_HISTORY_UNAVAILABLE: u8 = 14;
/// One required finalized period was absent from native storage.
pub const NETWORK_INGRESS_STATUS_PBFT_SYNC_PERIOD_DATA_MISSING: u8 = 15;

const ERROR_PBFT_SYNC_MALFORMED_REQUEST: &str = "NETWORK_PBFT_SYNC_MALFORMED_REQUEST";
const ERROR_PBFT_SYNC_UNSUPPORTED_VERSION: &str = "NETWORK_PBFT_SYNC_UNSUPPORTED_VERSION";
const ERROR_PBFT_SYNC_HEIGHT_AHEAD: &str = "NETWORK_PBFT_SYNC_HEIGHT_AHEAD";
const ERROR_PBFT_SYNC_HISTORY_UNAVAILABLE: &str = "NETWORK_PBFT_SYNC_HISTORY_UNAVAILABLE";
const ERROR_PBFT_SYNC_PERIOD_DATA_MISSING: &str = "NETWORK_PBFT_SYNC_PERIOD_DATA_MISSING";

/// Executor-visible network effect planned by Rust consensus.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEffect {
    /// Stable effect id used to correlate executor result reports.
    pub effect_id: u64,
    /// Ingress payload id that caused this effect, when known.
    pub source_payload_id: u64,
    /// Tarcap version/lane that owns physical execution of this effect.
    pub transport_lane: u32,
    /// Stable effect kind.
    pub kind: u8,
    /// Target peer id when the effect applies to one peer.
    pub peer_id: [u8; 64],
    /// Packet kind for send/gossip effects.
    pub packet_kind: u32,
    /// Packet payload bytes for send/gossip effects.
    pub payload_bytes: Vec<u8>,
    /// Optional canonical companion payload required to execute the effect.
    pub related_payload_bytes: Vec<u8>,
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
    /// Whether verified-vote admission inserted the vote.
    pub admission_accepted: bool,
    /// Whether verified-vote admission found the vote already present.
    pub admission_already_present: bool,
    /// Whether verified-vote admission should mark this vote as known.
    pub admission_mark_vote_known: bool,
    /// Whether verified-vote admission allows vote gossip.
    pub admission_gossip_vote: bool,
    /// Whether verified-vote admission requested slashing reporting.
    pub admission_report_slashing: bool,
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftVoteIngressContext {
    /// Existing side-effect-free PBFT vote ingress context.
    pub ingress: PbftVoteIngressContext,
    /// Tarcap version/lane that owns any effects emitted for this vote.
    pub transport_lane: u32,
    /// Sending peer id. The network executor uses this for sync/report effects.
    pub peer_id: [u8; 64],
    /// Peer PBFT chain size known by tarcap at ingress time.
    pub peer_pbft_chain_size: u64,
    /// Optional network-owned source payload id for effect correlation.
    pub source_payload_id: u64,
    /// Whether vote admission should be routed through Rust-owned effects.
    pub enqueue_admission: bool,
    /// Whether accepted votes should be regossiped by Rust-owned follow-ups.
    pub allow_gossip: bool,
    /// Canonical vote hash for admission and gossip effects.
    pub vote_hash: [u8; 32],
    /// Canonical serialized vote bytes for admission.
    pub vote_rlp: Vec<u8>,
    /// Optional canonical PBFT block bytes attached to a propose vote.
    pub pbft_block_rlp: Vec<u8>,
    /// Optional PBFT block hash matching `pbft_block_rlp`.
    pub pbft_block_hash: [u8; 32],
    /// Optional PBFT block period matching `pbft_block_rlp`.
    pub pbft_block_period: u64,
}

/// Scalar context for authoritative pillar-vote ingress through the network API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPillarVoteIngressContext {
    /// Tarcap version/lane that owns any effects emitted for these votes.
    pub transport_lane: u32,
    /// Sending peer id.
    pub peer_id: [u8; 64],
    /// Optional network-owned source payload id for effect correlation.
    pub source_payload_id: u64,
    /// First period when pillar votes are active.
    pub ficus_activation_period: u64,
    /// Whether accepted votes should be regossiped by Rust-owned follow-ups.
    pub allow_gossip: bool,
}

/// Scalar request facts for PBFT next-vote bundle egress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftNextVotesBundleRequest {
    /// Tarcap version/lane that owns physical packet sends.
    pub transport_lane: u32,
    /// Peer that requested previous-round next votes.
    pub peer_id: [u8; 64],
    /// Peer period carried by the request packet.
    pub peer_period: u64,
    /// Peer round carried by the request packet.
    pub peer_round: u64,
    /// Local PBFT period snapshot.
    pub current_period: u64,
    /// Local PBFT round snapshot.
    pub current_round: u64,
    /// Optional network-owned packet identity for effect correlation.
    pub source_payload_id: u64,
}

/// Scalar request facts for one get-pillar-votes-bundle packet.
///
/// The request carries only transport identity and the exact native pillar
/// vote lookup coordinates. Schedule policy comes from the restored PBFT
/// service configuration; callers cannot override activation or interval
/// checks per packet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkGetPillarVotesBundleRequest {
    /// Tarcap version/lane that owns physical packet sends.
    pub transport_lane: u32,
    /// Peer that requested the pillar votes.
    pub peer_id: [u8; 64],
    /// Requested PBFT period containing the pillar block hash.
    pub period: u64,
    /// Pillar block hash whose votes should be served.
    pub pillar_block_hash: [u8; 32],
    /// Optional network-owned packet identity for effect correlation.
    pub source_payload_id: u64,
}

/// Canonical get-PBFT-sync ingress request owned by the native network service.
///
/// `request_rlp` is the complete one-field packet payload `[height_to_sync]`.
/// Versions five and six share finalized-period responses; only version six
/// receives proposed-block bundles. A zero `source_payload_id` remains a valid
/// unretained transport identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkGetPbftSyncRequest {
    /// Tarcap protocol version; exactly versions five and six are supported.
    pub tarcap_version: u32,
    /// Requesting peer.
    pub peer_id: [u8; 64],
    /// Canonical one-field get-PBFT-sync packet RLP.
    pub request_rlp: Vec<u8>,
    /// Optional network-owned packet identity for effect correlation.
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
    /// Unique application effect queued for synchronous admission correlation,
    /// or zero when no application leaf was queued.
    pub application_effect_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingVoteAdmissionContext {
    /// Tarcap version/lane that must execute follow-up effects.
    transport_lane: u32,
    /// Peer that supplied the accepted vote.
    peer_id: [u8; 64],
    /// Canonical vote hash and serialized payload.
    vote_hash: [u8; 32],
    vote_rlp: Vec<u8>,
    /// Optional PBFT block bytes attached to a propose vote.
    pbft_block_rlp: Vec<u8>,
    pbft_block_hash: [u8; 32],
    pbft_block_period: u64,
    /// Retained packet payload id for effect correlation.
    source_payload_id: u64,
    /// Whether accepted votes should be regossiped by Rust-owned follow-ups.
    allow_gossip: bool,
    /// Bundle aggregation identity and member data, when this vote came from
    /// one all-or-nothing preflighted bundle.
    bundle: Option<PendingVoteBundleMember>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingVoteBundleMember {
    bundle_id: u64,
    index: usize,
    vote: PbftVotePayloadRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingVoteBundle {
    source_payload_id: u64,
    transport_lane: u32,
    peer_id: [u8; 64],
    completed: Vec<bool>,
    accepted_votes: Vec<Option<PbftVotePayloadRecord>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingPillarVoteAdmissionContext {
    transport_lane: u32,
    peer_id: [u8; 64],
    vote_hash: [u8; 32],
    vote_rlp: Vec<u8>,
    period: u64,
    source_payload_id: u64,
    allow_gossip: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PillarVoteNetworkChunk {
    vote_hashes: Vec<[u8; 32]>,
    payload_bytes: Vec<u8>,
}

/// Compact local and peer facts needed to plan status-triggered sync work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkStatusSyncFacts {
    /// Whether the local node is already running PBFT sync.
    pub local_pbft_syncing: bool,
    /// Local finalized/synced PBFT period.
    pub local_pbft_synced_period: u64,
    /// Local current PBFT period.
    pub local_pbft_period: u64,
    /// Local current PBFT round.
    pub local_pbft_round: u64,
    /// Peer PBFT chain size learned from the status packet.
    pub peer_pbft_chain_size: u64,
    /// Peer PBFT period derived by tarcap from peer chain size.
    pub peer_pbft_period: u64,
    /// Peer PBFT round learned from the status packet.
    pub peer_pbft_round: u64,
    /// Whether tarcap already considers the peer's DAG synchronized.
    pub peer_dag_synced: bool,
    /// Previous status-reported PBFT chain size retained for the one-block-behind debounce.
    pub peer_last_status_pbft_chain_size: u64,
}

/// Side-effect-free status sync plan for the network/tarcap executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkStatusSyncPlan {
    /// Whether tarcap should start PBFT sync with the selected peer.
    pub request_pbft_sync: bool,
    /// Whether tarcap should request pending DAG blocks from the selected peer.
    pub request_pending_dag_blocks: bool,
    /// Whether tarcap should request next-vote bundles from the selected peer.
    pub request_next_votes: bool,
    /// Local PBFT period to put into the next-votes request.
    pub next_votes_period: u64,
    /// Local PBFT round to put into the next-votes request.
    pub next_votes_round: u64,
}

/// Compact facts needed to shape a local status packet for tarcap egress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkStatusEgressFacts {
    /// Whether tarcap is sending the initial status packet.
    pub initial: bool,
    /// Locally configured chain id for initial status packets.
    pub local_chain_id: u64,
    /// Locally configured genesis hash for initial status packets.
    pub genesis_hash: [u8; 32],
    /// Local node major version for initial status packets.
    pub node_major_version: u32,
    /// Local node minor version for initial status packets.
    pub node_minor_version: u32,
    /// Local node patch version for initial status packets.
    pub node_patch_version: u32,
    /// Whether this node is configured as a light node.
    pub is_light_node: bool,
    /// Number of recent periods served when this node is a light node.
    pub light_node_history: u64,
    /// Local PBFT chain size snapshot.
    pub local_pbft_chain_size: u64,
    /// Local PBFT round snapshot.
    pub local_pbft_round: u64,
    /// Local DAG max level snapshot.
    pub local_dag_level: u64,
    /// Whether local PBFT sync is active.
    pub pbft_syncing: bool,
    /// Whether local PBFT sync is deep sync.
    pub deep_pbft_syncing: bool,
}

/// Side-effect-free local status packet plan for tarcap egress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkStatusEgressPlan {
    /// Stable status for boundary logs and tests.
    pub status: u8,
    /// Stable textual status for boundary logs and tests.
    pub error_code: String,
    /// PBFT chain size to advertise.
    pub peer_pbft_chain_size: u64,
    /// PBFT round to advertise.
    pub peer_pbft_round: u64,
    /// DAG max level to advertise.
    pub peer_dag_level: u64,
    /// Syncing flag to advertise in the status packet.
    pub peer_syncing: bool,
    /// Whether initial status metadata should be included.
    pub include_initial_data: bool,
    /// Chain id for initial status metadata.
    pub chain_id: u64,
    /// Genesis hash for initial status metadata.
    pub genesis_hash: [u8; 32],
    /// Node major version for initial status metadata.
    pub node_major_version: u32,
    /// Node minor version for initial status metadata.
    pub node_minor_version: u32,
    /// Node patch version for initial status metadata.
    pub node_patch_version: u32,
    /// Light-node flag for initial status metadata.
    pub is_light_node: bool,
    /// Light-node history for initial status metadata.
    pub light_node_history: u64,
}

/// Compact facts needed to validate an initial status packet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkInitialStatusFacts {
    /// Locally configured chain id.
    pub local_chain_id: u64,
    /// Chain id advertised by the peer.
    pub peer_chain_id: u64,
    /// Locally configured genesis hash.
    pub expected_genesis_hash: [u8; 32],
    /// Genesis hash advertised by the peer.
    pub peer_genesis_hash: [u8; 32],
    /// Local PBFT sync period used for light-node serviceability checks.
    pub local_pbft_synced_period: u64,
    /// PBFT chain size advertised by the peer.
    pub peer_pbft_chain_size: u64,
    /// Whether the peer advertises itself as a light node.
    pub peer_is_light_node: bool,
    /// Number of recent periods the peer can serve when it is a light node.
    pub peer_light_node_history: u64,
}

/// Side-effect-free initial-status admission plan for tarcap execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkInitialStatusPlan {
    /// Stable status for boundary logs and tests.
    pub status: u8,
    /// Stable textual status for boundary logs and tests.
    pub error_code: String,
    /// Whether tarcap should accept and materialize the peer.
    pub accept_peer: bool,
    /// Whether tarcap should disconnect the peer.
    pub disconnect_peer: bool,
}

/// Compact peer candidate for PBFT sync-start planning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncPeerCandidate {
    /// Candidate peer id.
    pub peer_id: [u8; 64],
    /// PBFT chain size reported by the peer.
    pub pbft_chain_size: u64,
    /// DAG level reported by the peer. Used as tie-breaker when PBFT chain
    /// size is equal.
    pub dag_level: u64,
    /// Whether this peer is a light node.
    pub is_light_node: bool,
    /// Number of recent periods the light node can serve.
    pub light_node_history: u64,
    /// Whether tarcap already considers this peer's DAG synchronized.
    pub peer_dag_synced: bool,
    /// Whether tarcap already has a pending DAG sync request for this peer.
    pub peer_dag_syncing: bool,
    /// Whether tarcap allows requesting DAG sync from this peer now.
    pub dag_sync_allowed: bool,
}

/// Compact facts needed to plan PBFT sync start from known peers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncStartFacts {
    /// Whether local PBFT sync is already active.
    pub local_pbft_syncing: bool,
    /// Local finalized/synced PBFT period.
    pub local_pbft_synced_period: u64,
    /// Local PBFT chain size used only for diagnostics and temporary logs.
    pub local_pbft_chain_size: u64,
    /// Candidate peers known by tarcap.
    pub candidates: Vec<NetworkPbftSyncPeerCandidate>,
}

/// Side-effect-free PBFT sync-start plan for the network/tarcap executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncStartPlan {
    /// Stable status for boundary logs and tests.
    pub status: u8,
    /// Stable textual status for boundary logs and tests.
    pub error_code: String,
    /// Whether tarcap should start PBFT sync with `peer_id`.
    pub start_sync: bool,
    /// Whether the plan selected a usable peer.
    pub has_peer: bool,
    /// Selected peer id, or zeroes when no peer was selected.
    pub peer_id: [u8; 64],
    /// Selected peer PBFT chain size.
    pub peer_pbft_chain_size: u64,
    /// Period to request in the first `GetPbftSyncPacket`.
    pub request_period: u64,
    /// Whether tarcap should enable snapshot creation because PBFT sync is not needed.
    pub enable_snapshot_creation: bool,
}

/// Compact facts needed to select the best live network peer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPeerSelectionFacts {
    /// Local PBFT sync period used for light-node serviceability checks.
    pub local_pbft_syncing_period: u64,
    /// Candidate peers known by the network executor.
    pub candidates: Vec<NetworkPbftSyncPeerCandidate>,
}

/// Side-effect-free peer-selection plan for the network/tarcap executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPeerSelectionPlan {
    /// Stable status for boundary logs and tests.
    pub status: u8,
    /// Stable textual status for boundary logs and tests.
    pub error_code: String,
    /// Whether the plan selected a usable peer.
    pub has_peer: bool,
    /// Selected peer id, or zeroes when no peer was selected.
    pub peer_id: [u8; 64],
    /// Selected peer PBFT chain size.
    pub peer_pbft_chain_size: u64,
}

/// Compact facts needed to plan a pending-DAG-block request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPendingDagBlocksRequestFacts {
    /// Local PBFT sync period used to gate DAG requests.
    pub local_pbft_syncing_period: u64,
    /// Whether the caller supplied an explicit peer.
    pub has_explicit_peer: bool,
    /// Explicit peer candidate. Ignored when `has_explicit_peer` is false.
    pub explicit_peer: NetworkPbftSyncPeerCandidate,
    /// Candidate peers known by tarcap when no explicit peer is supplied.
    pub candidates: Vec<NetworkPbftSyncPeerCandidate>,
}

/// Side-effect-free pending-DAG request plan for the network/tarcap executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPendingDagBlocksRequestPlan {
    /// Stable status for boundary logs and tests.
    pub status: u8,
    /// Stable textual status for boundary logs and tests.
    pub error_code: String,
    /// Whether tarcap should reserve the peer and send a `GetDagSyncPacket`.
    pub request_pending_dag_blocks: bool,
    /// Whether the plan selected a peer.
    pub has_peer: bool,
    /// Selected peer id, or zeroes when no peer was selected.
    pub peer_id: [u8; 64],
    /// PBFT period to request in the `GetDagSyncPacket`.
    pub request_period: u64,
}

/// Cloneable native owner of consensus network routing and sibling queries.
///
/// Every clone shares one ordered network-effect queue while retaining handles
/// to the PBFT verified-vote and pillar siblings restored by the same
/// [`crate::PbftService`]. Operation methods never hold the network mutex while
/// querying a sibling: request policy is checked first, sibling payloads are
/// copied and fully validated next, and only then are packet-ready effects
/// appended under the network lock. Poisoned locks return errors; local
/// lookup and payload-invariant failures return typed zero-effect decisions.
#[derive(Clone)]
pub struct ConsensusNetworkService {
    api: Arc<Mutex<ConsensusNetworkApi>>,
    pillar: PillarChainService,
    verified_votes: PbftVerifiedVotesService,
    chain: PbftChainService,
    proposed_blocks: ProposedBlocksService,
    storage: Arc<Storage>,
    sync_level_size: u64,
    is_light_node: bool,
    light_node_history: u64,
}

impl ConsensusNetworkService {
    /// Constructs the single native network service for a restored PBFT root.
    ///
    /// `pillar_blocks_interval` must be greater than one while Ficus is
    /// enabled. A `u64::MAX` activation preserves the legacy disabled-Ficus
    /// configuration, where the interval is ignored. `sync_level_size` must be
    /// nonzero; light-node history is interpreted only when `is_light_node` is
    /// true. The sibling handles must come from the same restoration graph;
    /// clones then observe their shared native state.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pillar: PillarChainService,
        verified_votes: PbftVerifiedVotesService,
        chain: PbftChainService,
        proposed_blocks: ProposedBlocksService,
        storage: Arc<Storage>,
        ficus_activation_period: u64,
        pillar_blocks_interval: u64,
        sync_level_size: u64,
        is_light_node: bool,
        light_node_history: u64,
    ) -> Result<Self> {
        ensure!(
            ficus_activation_period == u64::MAX || pillar_blocks_interval > 1,
            "PBFT_SERVICE_PILLAR_BLOCKS_INTERVAL_MUST_EXCEED_ONE"
        );
        ensure!(sync_level_size > 0, "PBFT_SERVICE_SYNC_LEVEL_SIZE_ZERO");
        Ok(Self {
            api: Arc::new(Mutex::new(ConsensusNetworkApi::with_pillar_schedule(
                ficus_activation_period,
                pillar_blocks_interval,
            ))),
            pillar,
            verified_votes,
            chain,
            proposed_blocks,
            storage,
            sync_level_size,
            is_light_node,
            light_node_history,
        })
    }

    fn lock_api(&self) -> Result<MutexGuard<'_, ConsensusNetworkApi>> {
        self.api
            .lock()
            .map_err(|_| anyhow!("CONSENSUS_NETWORK_SERVICE_LOCK_POISONED"))
    }

    /// Drains at most the fixed native limit of dependency-ready lane effects.
    pub fn drain_work(&self, transport_lane: u32, budget: u32) -> Result<NetworkEffectBatch> {
        Ok(self.lock_api()?.drain_work(transport_lane, budget))
    }

    /// Validates and records scalar executor results for previously drained effects.
    pub fn report_effect_results(
        &self,
        results: Vec<NetworkEffectResult>,
    ) -> Result<NetworkEffectAck> {
        Ok(self.lock_api()?.report_effect_results(results))
    }

    /// Plans status-triggered sync work from caller-owned scalar snapshots.
    pub fn plan_status_sync(&self, facts: NetworkStatusSyncFacts) -> Result<NetworkStatusSyncPlan> {
        Ok(self.lock_api()?.plan_status_sync(facts))
    }

    /// Plans one local status packet from caller-owned scalar snapshots.
    pub fn plan_status_egress(
        &self,
        facts: NetworkStatusEgressFacts,
    ) -> Result<NetworkStatusEgressPlan> {
        Ok(self.lock_api()?.plan_status_egress(facts))
    }

    /// Validates one initial status packet without mutating peer transport state.
    pub fn plan_initial_status(
        &self,
        facts: NetworkInitialStatusFacts,
    ) -> Result<NetworkInitialStatusPlan> {
        Ok(self.lock_api()?.plan_initial_status(facts))
    }

    /// Selects and plans one PBFT sync start from compact peer facts.
    pub fn plan_pbft_sync_start(
        &self,
        facts: NetworkPbftSyncStartFacts,
    ) -> Result<NetworkPbftSyncStartPlan> {
        Ok(self.lock_api()?.plan_pbft_sync_start(facts))
    }

    /// Selects the best serviceable max-chain peer from compact peer facts.
    pub fn plan_max_chain_peer_selection(
        &self,
        facts: NetworkPeerSelectionFacts,
    ) -> Result<NetworkPeerSelectionPlan> {
        Ok(self.lock_api()?.plan_max_chain_peer_selection(facts))
    }

    /// Plans a pending-DAG request without executing peer or packet operations.
    pub fn plan_pending_dag_blocks_request(
        &self,
        facts: NetworkPendingDagBlocksRequestFacts,
    ) -> Result<NetworkPendingDagBlocksRequestPlan> {
        Ok(self.lock_api()?.plan_pending_dag_blocks_request(facts))
    }

    /// Routes one PBFT vote and appends its ordered network/application effects.
    pub fn ingest_pbft_vote(
        &self,
        fact: PbftVoteIngressFact,
        context: NetworkPbftVoteIngressContext,
    ) -> Result<NetworkIngressDecision> {
        Ok(self.lock_api()?.ingest_pbft_vote(fact, context))
    }

    /// Atomically preflights one PBFT vote bundle before queueing admissions.
    pub fn ingest_pbft_vote_bundle(
        &self,
        reference: PbftVoteIngressFact,
        votes: Vec<PbftVoteIngressFact>,
        contexts: Vec<NetworkPbftVoteIngressContext>,
    ) -> Result<Vec<NetworkIngressDecision>> {
        Ok(self
            .lock_api()?
            .ingest_pbft_vote_bundle(reference, votes, contexts))
    }

    /// Atomically preflights one pillar-vote bundle before queueing admissions.
    pub fn ingest_pillar_vote_bundle(
        &self,
        context: NetworkPillarVoteIngressContext,
        votes: Vec<Vec<u8>>,
    ) -> Result<Vec<NetworkIngressDecision>> {
        Ok(self.lock_api()?.ingest_pillar_vote_bundle(context, votes))
    }

    /// Serves one eligible previous-round next-vote request directly.
    ///
    /// Eligibility is resolved without a lock. The verified-vote sibling then
    /// returns both complete bundle families in one sibling lock epoch. Rust
    /// validates and chunks those canonical payloads before acquiring the
    /// network lock to append ordered send effects. Missing families and an
    /// empty pair succeeds with no effects; local lookup or payload-invariant
    /// failures return typed zero-effect decisions. Lock poisoning propagates.
    pub fn ingest_pbft_next_votes_bundle_request(
        &self,
        request: NetworkPbftNextVotesBundleRequest,
    ) -> Result<NetworkIngressDecision> {
        if let Some(decision) = next_votes_request_rejection(&request) {
            return Ok(decision);
        }
        let period = request.current_period;
        let round = request.current_round - 1;
        let payloads = match self
            .verified_votes
            .verified_votes_build_next_votes_bundle_egress(period, round)
        {
            Ok(payloads) => payloads,
            Err(error) if native_lock_poisoned(&error) => return Err(error),
            Err(_) => {
                return Ok(local_network_decision(
                    request.source_payload_id,
                    NETWORK_INGRESS_STATUS_LOCAL_LOOKUP_FAILED,
                    ERROR_NEXT_VOTES_LOOKUP_FAILED,
                ));
            }
        };
        let chunks = match validate_next_votes_payloads(payloads, period, round) {
            Ok(chunks) => chunks,
            Err(_) => {
                return Ok(local_network_decision(
                    request.source_payload_id,
                    NETWORK_INGRESS_STATUS_INVALID_NATIVE_RESULT,
                    ERROR_NEXT_VOTES_INVALID_NATIVE_RESULT,
                ));
            }
        };
        Ok(self
            .lock_api()?
            .enqueue_next_votes_bundle_send_effects(request, chunks))
    }

    /// Serves one schedule-valid pillar-vote bundle request directly.
    ///
    /// Before Ficus or off the exact pillar PBFT schedule, the peer is reported
    /// and then disconnected through a dependent effect. Valid requests query
    /// the pillar sibling for ordered live-first, stored-period-fallback full
    /// vote payloads. Every payload is revalidated for canonical encoding,
    /// period, block hash, signature, retained hash, uniqueness, and output
    /// order before the network queue is locked. Empty lookups and local
    /// failures return distinct typed zero-effect decisions; lock poisoning
    /// propagates as an error.
    pub fn ingest_get_pillar_votes_bundle_request(
        &self,
        request: NetworkGetPillarVotesBundleRequest,
    ) -> Result<NetworkIngressDecision> {
        {
            let mut api = self.lock_api()?;
            if let Some(decision) = api.reject_invalid_pillar_votes_request(&request) {
                return Ok(decision);
            }
        }

        let records = match self.pillar.pbft_service_pillar_get_verified_vote_payloads(
            request.period,
            &request.pillar_block_hash,
            false,
        ) {
            Ok(lookup) => lookup.votes,
            Err(error) if native_lock_poisoned(&error) => return Err(error),
            Err(_) => {
                return Ok(local_network_decision(
                    request.source_payload_id,
                    NETWORK_INGRESS_STATUS_LOCAL_LOOKUP_FAILED,
                    ERROR_PILLAR_VOTES_LOOKUP_FAILED,
                ));
            }
        };
        if records.is_empty() {
            return Ok(local_network_decision(
                request.source_payload_id,
                NETWORK_INGRESS_STATUS_PILLAR_VOTES_NO_DATA,
                ERROR_PILLAR_VOTES_NO_DATA,
            ));
        }
        let chunks = match validate_and_chunk_pillar_votes(
            records,
            request.period,
            request.pillar_block_hash,
        ) {
            Ok(chunks) => chunks,
            Err(_) => {
                return Ok(local_network_decision(
                    request.source_payload_id,
                    NETWORK_INGRESS_STATUS_INVALID_NATIVE_RESULT,
                    ERROR_PILLAR_VOTES_INVALID_NATIVE_RESULT,
                ));
            }
        };
        Ok(self
            .lock_api()?
            .enqueue_pillar_vote_bundle_send_effects(request, chunks))
    }

    /// Serves one canonical get-PBFT-sync request from native snapshots.
    ///
    /// Rust validates protocol version and canonical request bytes, snapshots
    /// chain, reward-vote, and shared proposal state, then completes every
    /// storage read before taking the network queue lock. Invalid range/history
    /// requests produce report and dependent-disconnect effects. Successful
    /// v5/v6 requests produce full PBFT sync packet payloads; v6 additionally
    /// produces proposal bundles of at most ten. Completed sync queues an
    /// independent clear-peer-syncing effect before proposals. Missing period
    /// data queues the already-built prefix and any v6 proposal snapshot.
    pub fn ingest_get_pbft_sync_request(
        &self,
        request: NetworkGetPbftSyncRequest,
    ) -> Result<NetworkIngressDecision> {
        if request.tarcap_version != TARCAP_VERSION_5 && request.tarcap_version != TARCAP_VERSION_6
        {
            return Ok(local_network_decision(
                request.source_payload_id,
                NETWORK_INGRESS_STATUS_PBFT_SYNC_UNSUPPORTED_VERSION,
                ERROR_PBFT_SYNC_UNSUPPORTED_VERSION,
            ));
        }
        let Some(height_to_sync) = decode_get_pbft_sync_request(&request.request_rlp) else {
            return Ok(self.lock_api()?.reject_invalid_pbft_sync_request(
                &request,
                0,
                NETWORK_INGRESS_STATUS_PBFT_SYNC_MALFORMED_REQUEST,
                ERROR_PBFT_SYNC_MALFORMED_REQUEST,
            ));
        };
        let chain_size = self.chain.try_head()?.size;
        if height_to_sync > chain_size {
            return Ok(self.lock_api()?.reject_invalid_pbft_sync_request(
                &request,
                height_to_sync,
                NETWORK_INGRESS_STATUS_PBFT_SYNC_HEIGHT_AHEAD,
                ERROR_PBFT_SYNC_HEIGHT_AHEAD,
            ));
        }
        if self.is_light_node
            && height_to_sync
                .checked_add(self.light_node_history)
                .is_none_or(|retained_end| retained_end <= chain_size)
        {
            return Ok(self.lock_api()?.reject_invalid_pbft_sync_request(
                &request,
                height_to_sync,
                NETWORK_INGRESS_STATUS_PBFT_SYNC_HISTORY_UNAVAILABLE,
                ERROR_PBFT_SYNC_HISTORY_UNAVAILABLE,
            ));
        }

        let total = chain_size - height_to_sync + 1;
        let blocks_to_transfer = total.min(self.sync_level_size);
        let chain_synced = total <= self.sync_level_size;
        let reward_snapshot = chain_synced
            .then(|| {
                self.verified_votes
                    .current_reward_vote_snapshot()
                    .context("NETWORK_PBFT_SYNC_REWARD_SNAPSHOT")
            })
            .transpose()?;
        let proposed_blocks = if chain_synced && request.tarcap_version == TARCAP_VERSION_6 {
            self.proposed_blocks.try_snapshot_entries()?
        } else {
            Vec::new()
        };
        let mut sync_payloads = Vec::with_capacity(blocks_to_transfer as usize);
        let mut missing_period_data = false;
        for offset in 0..blocks_to_transfer {
            let period = height_to_sync + offset;
            let period_data = self
                .storage
                .period()
                .data_raw(period)
                .context("PBFT_SYNC_EGRESS_PERIOD_DATA_LOAD")?;
            if period_data.is_empty() {
                missing_period_data = true;
                break;
            }
            let last_block = offset + 1 == blocks_to_transfer;
            let reward_bundle = if chain_synced
                && last_block
                && reward_snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.cursor.found
                        && snapshot.cursor.period == period
                        && !snapshot.records.is_empty()
                }) {
                let reward_snapshot = reward_snapshot
                    .as_ref()
                    .expect("matching reward snapshot checked above");
                Some(
                    build_optimized_pbft_vote_bundle(
                        &reward_snapshot.records,
                        reward_snapshot.cursor.block_hash,
                        reward_snapshot.cursor.period,
                        reward_snapshot.cursor.round,
                        reward_snapshot.cursor.step,
                    )?
                    .bundle_rlp,
                )
            } else {
                None
            };
            sync_payloads.push(encode_pbft_sync_packet(
                last_block,
                &period_data,
                reward_bundle.as_deref(),
            ));
        }
        let proposal_payloads = encode_proposed_block_bundles(proposed_blocks);
        Ok(self.lock_api()?.enqueue_pbft_sync_egress_effects(
            request,
            height_to_sync,
            sync_payloads,
            proposal_payloads,
            chain_synced && !missing_period_data,
            if missing_period_data {
                NETWORK_INGRESS_STATUS_PBFT_SYNC_PERIOD_DATA_MISSING
            } else {
                NETWORK_INGRESS_STATUS_ACCEPTED
            },
            if missing_period_data {
                ERROR_PBFT_SYNC_PERIOD_DATA_MISSING
            } else {
                ERROR_NONE
            },
        ))
    }
}

/// Rust-owned external network/tarcap API facade.
///
/// The facade owns an ordered network effect queue. It is intentionally small: packet-specific decoding and
/// consensus planning should be added behind this type without exposing
/// consensus managers, C++ sidecars, storage handles, or shim routes to the
/// network module.
#[derive(Debug)]
pub(crate) struct ConsensusNetworkApi {
    ficus_activation_period: u64,
    pillar_blocks_interval: u64,
    next_effect_id: u64,
    next_vote_bundle_id: u64,
    pending_effects: VecDeque<NetworkEffect>,
    pending_vote_admissions: HashMap<u64, PendingVoteAdmissionContext>,
    pending_vote_bundles: HashMap<u64, PendingVoteBundle>,
    pending_pillar_vote_admissions: HashMap<u64, PendingPillarVoteAdmissionContext>,
    outstanding_effects: HashMap<u64, NetworkEffect>,
    completed_dependency_status: HashMap<u64, bool>,
}

impl Default for ConsensusNetworkApi {
    fn default() -> Self {
        Self::with_pillar_schedule(0, 10)
    }
}

impl ConsensusNetworkApi {
    /// Creates an empty network/tarcap API facade.
    #[must_use]
    #[cfg(test)]
    fn new() -> Self {
        Self::default()
    }

    fn with_pillar_schedule(ficus_activation_period: u64, pillar_blocks_interval: u64) -> Self {
        Self {
            ficus_activation_period,
            pillar_blocks_interval,
            next_effect_id: 1,
            next_vote_bundle_id: 1,
            pending_effects: VecDeque::new(),
            pending_vote_admissions: HashMap::new(),
            pending_vote_bundles: HashMap::new(),
            pending_pillar_vote_admissions: HashMap::new(),
            outstanding_effects: HashMap::new(),
            completed_dependency_status: HashMap::new(),
        }
    }

    /// Drains up to `budget` dependency-ready effects for one transport lane.
    ///
    /// Ready effects retain queue order, but an unresolved dependency may be
    /// overtaken by later independent work in the same lane. A zero budget is
    /// valid. Effects owned by other lanes remain queued, and `more_available`
    /// reports only work remaining for the requested lane.
    #[must_use]
    pub fn drain_work(&mut self, transport_lane: u32, budget: u32) -> NetworkEffectBatch {
        let mut effects = Vec::new();
        let capped_budget = usize::try_from(budget)
            .unwrap_or(usize::MAX)
            .min(MAX_EFFECTS_PER_DRAIN);
        let mut retained = VecDeque::new();
        let queued_count = self.pending_effects.len();
        for _ in 0..queued_count {
            if effects.len() >= capped_budget {
                break;
            }
            let Some(effect) = self.pending_effects.pop_front() else {
                break;
            };
            if effect.transport_lane != transport_lane {
                retained.push_back(effect);
                continue;
            }
            if effect.dependency_id != 0 {
                match self.completed_dependency_status.get(&effect.dependency_id) {
                    Some(true) => {}
                    Some(false) => {
                        continue;
                    }
                    None => {
                        retained.push_back(effect);
                        continue;
                    }
                }
            }
            self.outstanding_effects
                .insert(effect.effect_id, effect.clone());
            effects.push(effect);
        }
        retained.append(&mut self.pending_effects);
        self.pending_effects = retained;
        self.completed_dependency_status.retain(|effect_id, _| {
            self.pending_effects
                .iter()
                .any(|effect| effect.dependency_id == *effect_id)
        });
        NetworkEffectBatch {
            status: NETWORK_EFFECT_BATCH_STATUS_OK,
            effects,
            more_available: self
                .pending_effects
                .iter()
                .any(|effect| effect.transport_lane == transport_lane),
            error_code: ERROR_NONE.to_owned(),
        }
    }

    /// Records network executor result reports.
    ///
    /// Accepted reports update dependency and admission state, then are
    /// discarded. Keeping a production result journal would let peers grow
    /// memory indefinitely by repeatedly requesting valid response bundles.
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
                let effect = self.outstanding_effects.get(&result.effect_id).cloned();
                self.outstanding_effects.remove(&result.effect_id);
                if let Some(effect) = effect.as_ref()
                    && effect.kind == NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
                    && effect.object_kind == NETWORK_OBJECT_KIND_PBFT_VOTE
                    && let Some(context) = self.pending_vote_admissions.remove(&result.effect_id)
                {
                    if result.status == NETWORK_EFFECT_RESULT_STATUS_OK {
                        self.enqueue_vote_admission_follow_ups(context, result);
                    } else if let Some(bundle) = context.bundle {
                        self.cancel_vote_bundle(bundle.bundle_id);
                    }
                }
                if let Some(effect) = effect.as_ref()
                    && effect.kind == NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
                    && effect.object_kind == NETWORK_OBJECT_KIND_PILLAR_VOTE
                    && let Some(context) = self
                        .pending_pillar_vote_admissions
                        .remove(&result.effect_id)
                    && result.status == NETWORK_EFFECT_RESULT_STATUS_OK
                {
                    self.enqueue_pillar_vote_admission_follow_ups(context, result);
                }
                if self
                    .pending_effects
                    .iter()
                    .any(|effect| effect.dependency_id == result.effect_id)
                {
                    self.completed_dependency_status.insert(
                        result.effect_id,
                        result.status == NETWORK_EFFECT_RESULT_STATUS_OK,
                    );
                }
            }
        }

        NetworkEffectAck {
            status,
            accepted_results,
            failed_results,
            error_code: error_code.to_owned(),
        }
    }

    /// Plans deterministic sync follow-up for an accepted status packet.
    ///
    /// Tarcap supplies compact local and peer status facts, while Rust owns the
    /// decision to request PBFT sync, pending DAG blocks, or next-vote bundles.
    /// The returned plan is side-effect free: peer state updates, packet
    /// encoding, and sends remain network/tarcap executor work.
    #[must_use]
    pub fn plan_status_sync(&self, facts: NetworkStatusSyncFacts) -> NetworkStatusSyncPlan {
        plan_status_sync(facts)
    }

    /// Plans local status packet egress.
    ///
    /// Rust owns status packet shaping from compact local snapshot facts.
    /// Tarcap still owns gathering live snapshot facts, RLP encoding, packet
    /// framing, and transport send execution.
    #[must_use]
    pub fn plan_status_egress(&self, facts: NetworkStatusEgressFacts) -> NetworkStatusEgressPlan {
        plan_status_egress(facts)
    }

    /// Plans initial status packet admission.
    ///
    /// Rust owns deterministic chain-id, genesis, and light-node history
    /// admission decisions. Tarcap still owns pending-peer lookup, peer-state
    /// materialization, logging, and disconnect execution.
    #[must_use]
    pub fn plan_initial_status(
        &self,
        facts: NetworkInitialStatusFacts,
    ) -> NetworkInitialStatusPlan {
        plan_initial_status(facts)
    }

    /// Plans whether PBFT sync should start and which peer should serve it.
    ///
    /// Rust owns max-chain peer selection, light-node serviceability checks,
    /// and the start/not-needed decision. Tarcap still enumerates live peers,
    /// mutates `PbftSyncingState`, sends `GetPbftSyncPacket`, and applies the
    /// snapshot lifecycle side effects returned by this plan.
    #[must_use]
    pub fn plan_pbft_sync_start(
        &self,
        facts: NetworkPbftSyncStartFacts,
    ) -> NetworkPbftSyncStartPlan {
        plan_pbft_sync_start(facts)
    }

    /// Selects the best max-chain peer from compact network-owned peer facts.
    ///
    /// Rust owns the deterministic PBFT chain size and DAG level ordering plus
    /// light-node history eligibility. Network still owns peer maps, live peer
    /// lookup, packet handler dispatch, and transport execution.
    #[must_use]
    pub fn plan_max_chain_peer_selection(
        &self,
        facts: NetworkPeerSelectionFacts,
    ) -> NetworkPeerSelectionPlan {
        plan_max_chain_peer_selection(facts)
    }

    /// Plans whether tarcap should request pending DAG blocks from a peer.
    ///
    /// Rust owns explicit-peer gating, max-chain peer selection, light-node
    /// serviceability checks, and the PBFT-period match required before a
    /// `GetDagSyncPacket` is sent. Tarcap still owns the atomic
    /// `peer_dag_syncing_` reservation, non-finalized DAG snapshot
    /// materialization, packet encoding, and network send execution.
    #[must_use]
    pub fn plan_pending_dag_blocks_request(
        &self,
        facts: NetworkPendingDagBlocksRequestFacts,
    ) -> NetworkPendingDagBlocksRequestPlan {
        plan_pending_dag_blocks_request(facts)
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
        let plan = plan_pbft_vote_ingress(fact, context.ingress);
        self.decision_from_vote_plan(plan, fact, context)
    }

    /// Preflights and routes one complete PBFT vote bundle.
    ///
    /// Every member is shape-checked before any application effect is queued.
    /// On success, the returned decisions contain one exact admission effect id
    /// per input member. Rust retains a unique aggregation session until all
    /// reports arrive, then emits an accepted-only optimized bundle gossip
    /// effect. Executor failure or slashing cancels the session and remaining
    /// queued admissions without rolling back already published votes.
    pub fn ingest_pbft_vote_bundle(
        &mut self,
        reference: PbftVoteIngressFact,
        votes: Vec<PbftVoteIngressFact>,
        mut contexts: Vec<NetworkPbftVoteIngressContext>,
    ) -> Vec<NetworkIngressDecision> {
        if votes.is_empty() || votes.len() != contexts.len() {
            return Vec::new();
        }
        let first_identity = (
            contexts[0].transport_lane,
            contexts[0].peer_id,
            contexts[0].source_payload_id,
        );
        if contexts.iter().any(|context| {
            (
                context.transport_lane,
                context.peer_id,
                context.source_payload_id,
            ) != first_identity
        }) {
            return Vec::new();
        }
        for context in &mut contexts {
            context.enqueue_admission = true;
            context.allow_gossip = false;
        }

        let plans = votes
            .iter()
            .zip(&contexts)
            .map(|(vote, context)| plan_pbft_vote_bundle_ingress(reference, *vote, context.ingress))
            .collect::<Vec<_>>();
        if let Some(index) = plans
            .iter()
            .position(|plan| plan.status != PbftVoteIngressStatus::Accepted)
        {
            return vec![self.decision_from_vote_plan(
                plans[index],
                votes[index],
                contexts[index].clone(),
            )];
        }

        let bundle_id = self.next_vote_bundle_id;
        self.next_vote_bundle_id = self.next_vote_bundle_id.saturating_add(1);
        let first = &contexts[0];
        self.pending_vote_bundles.insert(
            bundle_id,
            PendingVoteBundle {
                source_payload_id: first.source_payload_id,
                transport_lane: first.transport_lane,
                peer_id: first.peer_id,
                completed: vec![false; votes.len()],
                accepted_votes: vec![None; votes.len()],
            },
        );

        plans
            .into_iter()
            .zip(votes)
            .zip(contexts)
            .enumerate()
            .map(|(index, ((plan, vote), context))| {
                let member = PendingVoteBundleMember {
                    bundle_id,
                    index,
                    vote: PbftVotePayloadRecord {
                        hash: H256::from(context.vote_hash),
                        vote_rlp: context.vote_rlp.clone(),
                    },
                };
                self.decision_from_vote_plan_with_bundle(plan, vote, context, member)
            })
            .collect()
    }

    /// Routes one pillar-vote ingress decision and queues one admission effect.
    ///
    /// The canonical RLP input is decoded and checked for signature validity and
    /// activation-period eligibility. A valid vote returns an exact application
    /// effect id; malformed or ineligible votes return a typed rejection and
    /// queue nothing. State-dependent relevance and duplication stay inside the
    /// later atomic PBFT-service admission leaf.
    pub fn ingest_pillar_vote(
        &mut self,
        context: NetworkPillarVoteIngressContext,
        vote_rlp: Vec<u8>,
    ) -> NetworkIngressDecision {
        let vote_inspection = match inspect_pillar_vote_from_rlp(&vote_rlp) {
            Ok(inspection) => inspection,
            Err(_) => {
                return NetworkIngressDecision {
                    payload_id: context.source_payload_id,
                    payload_accepted: context.source_payload_id != 0,
                    routed: true,
                    status: NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_RLP,
                    error_code: ERROR_PILLAR_VOTE_INGRESS_MALFORMED_RLP.to_owned(),
                    queued_effect_count: 0,
                    application_effect_id: 0,
                };
            }
        };

        if !vote_inspection.signature_valid
            || vote_inspection.period < context.ficus_activation_period
        {
            return NetworkIngressDecision {
                payload_id: context.source_payload_id,
                payload_accepted: context.source_payload_id != 0,
                routed: true,
                status: NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_CONTEXT,
                error_code: ERROR_PILLAR_VOTE_INGRESS_INVALID_CONTEXT.to_owned(),
                queued_effect_count: 0,
                application_effect_id: 0,
            };
        }

        let source_payload_id = context.source_payload_id;
        let before_effects = self.pending_effects.len();
        let application_effect_id = self.enqueue_pillar_vote_admission_effect(
            context,
            vote_inspection.vote_hash.to_fixed_bytes(),
            vote_inspection.period,
            vote_rlp,
        );
        NetworkIngressDecision {
            payload_id: source_payload_id,
            payload_accepted: source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
            application_effect_id,
        }
    }

    /// Routes one complete pillar-vote packet as an atomic preflight unit.
    ///
    /// Inputs share one transport/source context and retain packet order. Empty
    /// input returns no decisions. Oversized, malformed, invalid-signature,
    /// pre-activation, or duplicate-hash input returns one rejection per member
    /// without queueing any effect. Otherwise one exact-id admission decision is
    /// returned per member in input order; state-dependent admission remains an
    /// executor result and cannot release follow-ups before acknowledgement.
    pub fn ingest_pillar_vote_bundle(
        &mut self,
        context: NetworkPillarVoteIngressContext,
        votes: Vec<Vec<u8>>,
    ) -> Vec<NetworkIngressDecision> {
        if votes.is_empty() {
            return Vec::new();
        }
        if votes.len() > MAX_PILLAR_VOTES_PER_BUNDLE_PACKET {
            let payload_id = context.source_payload_id;
            let payload_accepted = payload_id != 0;
            return votes
                .into_iter()
                .map(|_| NetworkIngressDecision {
                    payload_id,
                    payload_accepted,
                    routed: true,
                    status: NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_CONTEXT,
                    error_code: ERROR_PILLAR_VOTE_INGRESS_INVALID_CONTEXT.to_owned(),
                    queued_effect_count: 0,
                    application_effect_id: 0,
                })
                .collect();
        }

        let inspections = votes
            .iter()
            .map(|vote_rlp| inspect_pillar_vote_from_rlp(vote_rlp))
            .collect::<anyhow::Result<Vec<_>>>();
        let inspections = match inspections {
            Ok(inspections) => inspections,
            Err(_) => {
                return pillar_vote_bundle_rejection(
                    &context,
                    votes.len(),
                    NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_RLP,
                    ERROR_PILLAR_VOTE_INGRESS_MALFORMED_RLP,
                );
            }
        };
        let mut seen = HashSet::with_capacity(inspections.len());
        if inspections.iter().any(|inspection| {
            !inspection.signature_valid
                || inspection.period < context.ficus_activation_period
                || !seen.insert(inspection.vote_hash.to_fixed_bytes())
        }) {
            return pillar_vote_bundle_rejection(
                &context,
                votes.len(),
                NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_CONTEXT,
                ERROR_PILLAR_VOTE_INGRESS_INVALID_CONTEXT,
            );
        }

        votes
            .into_iter()
            .map(|vote_rlp| self.ingest_pillar_vote(context.clone(), vote_rlp))
            .collect()
    }

    fn reject_invalid_pillar_votes_request(
        &mut self,
        request: &NetworkGetPillarVotesBundleRequest,
    ) -> Option<NetworkIngressDecision> {
        let inactive = self.ficus_activation_period == u64::MAX
            || request.period < self.ficus_activation_period;
        let first_pillar_period = if self.ficus_activation_period == 0 {
            self.pillar_blocks_interval
        } else {
            self.ficus_activation_period
        };
        let wrong_period = !inactive
            && (request.period < first_pillar_period
                || request.period % self.pillar_blocks_interval != 1);
        let (status, error_code) = if inactive {
            (
                NETWORK_INGRESS_STATUS_PILLAR_VOTES_INACTIVE,
                ERROR_PILLAR_VOTES_INACTIVE,
            )
        } else if wrong_period {
            (
                NETWORK_INGRESS_STATUS_PILLAR_VOTES_INVALID_PERIOD,
                ERROR_PILLAR_VOTES_INVALID_PERIOD,
            )
        } else {
            return None;
        };

        let report_id = self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: request.source_payload_id,
            transport_lane: request.transport_lane,
            kind: NETWORK_EFFECT_KIND_REPORT_PEER,
            peer_id: request.peer_id,
            packet_kind: 0,
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
            object_hash: request.pillar_block_hash,
            sync_kind: 0,
            sync_start: 0,
            reason_code: NETWORK_REASON_INVALID_PILLAR_VOTES_REQUEST,
            dependency_id: 0,
            period: request.period,
            round: 0,
        });
        self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: request.source_payload_id,
            transport_lane: request.transport_lane,
            kind: NETWORK_EFFECT_KIND_DISCONNECT_PEER,
            peer_id: request.peer_id,
            packet_kind: 0,
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
            object_hash: request.pillar_block_hash,
            sync_kind: 0,
            sync_start: 0,
            reason_code: NETWORK_REASON_INVALID_PILLAR_VOTES_REQUEST,
            dependency_id: report_id,
            period: request.period,
            round: 0,
        });
        Some(NetworkIngressDecision {
            payload_id: request.source_payload_id,
            payload_accepted: request.source_payload_id != 0,
            routed: true,
            status,
            error_code: error_code.to_owned(),
            queued_effect_count: 2,
            application_effect_id: 0,
        })
    }

    fn reject_invalid_pbft_sync_request(
        &mut self,
        request: &NetworkGetPbftSyncRequest,
        height_to_sync: u64,
        status: u8,
        error_code: &str,
    ) -> NetworkIngressDecision {
        let report_id = self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: request.source_payload_id,
            transport_lane: request.tarcap_version,
            kind: NETWORK_EFFECT_KIND_REPORT_PEER,
            peer_id: request.peer_id,
            packet_kind: NETWORK_PACKET_KIND_GET_PBFT_SYNC,
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: NETWORK_OBJECT_KIND_PBFT_SYNC_EGRESS_REQUEST,
            object_hash: [0; 32],
            sync_kind: NETWORK_SYNC_KIND_PBFT_CHAIN,
            sync_start: height_to_sync,
            reason_code: NETWORK_REASON_INVALID_PBFT_SYNC_REQUEST,
            dependency_id: 0,
            period: height_to_sync,
            round: 0,
        });
        self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: request.source_payload_id,
            transport_lane: request.tarcap_version,
            kind: NETWORK_EFFECT_KIND_DISCONNECT_PEER,
            peer_id: request.peer_id,
            packet_kind: NETWORK_PACKET_KIND_GET_PBFT_SYNC,
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: NETWORK_OBJECT_KIND_PBFT_SYNC_EGRESS_REQUEST,
            object_hash: [0; 32],
            sync_kind: NETWORK_SYNC_KIND_PBFT_CHAIN,
            sync_start: height_to_sync,
            reason_code: NETWORK_REASON_INVALID_PBFT_SYNC_REQUEST,
            dependency_id: report_id,
            period: height_to_sync,
            round: 0,
        });
        NetworkIngressDecision {
            payload_id: request.source_payload_id,
            payload_accepted: request.source_payload_id != 0,
            routed: true,
            status,
            error_code: error_code.to_owned(),
            queued_effect_count: 2,
            application_effect_id: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn enqueue_pbft_sync_egress_effects(
        &mut self,
        request: NetworkGetPbftSyncRequest,
        height_to_sync: u64,
        sync_payloads: Vec<Vec<u8>>,
        proposal_payloads: Vec<Vec<u8>>,
        clear_peer_syncing: bool,
        status: u8,
        error_code: &str,
    ) -> NetworkIngressDecision {
        let before_effects = self.pending_effects.len();
        let mut final_sync_effect_id = 0;
        for (offset, payload_bytes) in sync_payloads.into_iter().enumerate() {
            final_sync_effect_id = self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: request.source_payload_id,
                transport_lane: request.tarcap_version,
                kind: NETWORK_EFFECT_KIND_SEND_PACKET,
                peer_id: request.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PBFT_SYNC,
                payload_bytes,
                related_payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_PERIOD_DATA,
                object_hash: [0; 32],
                sync_kind: NETWORK_SYNC_KIND_PBFT_CHAIN,
                sync_start: height_to_sync,
                reason_code: 0,
                dependency_id: 0,
                period: height_to_sync.saturating_add(offset as u64),
                round: 0,
            });
        }
        if clear_peer_syncing && final_sync_effect_id != 0 {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: request.source_payload_id,
                transport_lane: request.tarcap_version,
                kind: NETWORK_EFFECT_KIND_CLEAR_PEER_SYNCING,
                peer_id: request.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PBFT_SYNC,
                payload_bytes: Vec::new(),
                related_payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_SYNC_EGRESS_REQUEST,
                object_hash: [0; 32],
                sync_kind: NETWORK_SYNC_KIND_PBFT_CHAIN,
                sync_start: height_to_sync,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }
        for payload_bytes in proposal_payloads {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: request.source_payload_id,
                transport_lane: request.tarcap_version,
                kind: NETWORK_EFFECT_KIND_SEND_PACKET,
                peer_id: request.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PBFT_BLOCKS_BUNDLE,
                payload_bytes,
                related_payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_BLOCK,
                object_hash: [0; 32],
                sync_kind: NETWORK_SYNC_KIND_PBFT_CHAIN,
                sync_start: height_to_sync,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }
        NetworkIngressDecision {
            payload_id: request.source_payload_id,
            payload_accepted: request.source_payload_id != 0,
            routed: true,
            status,
            error_code: error_code.to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
            application_effect_id: 0,
        }
    }

    fn decision_from_vote_plan(
        &mut self,
        plan: PbftVoteIngressPlan,
        fact: PbftVoteIngressFact,
        context: NetworkPbftVoteIngressContext,
    ) -> NetworkIngressDecision {
        let source_payload_id = context.source_payload_id;
        let before_effects = self.pending_effects.len();
        self.enqueue_vote_plan_effects(plan, fact, &context);
        let should_admit =
            plan.status == PbftVoteIngressStatus::Accepted && context.enqueue_admission;
        let application_effect_id = if should_admit {
            self.enqueue_vote_admission_effect(context, None)
        } else {
            0
        };
        let queued_effect_count = self.pending_effects.len().saturating_sub(before_effects) as u32;

        NetworkIngressDecision {
            payload_id: source_payload_id,
            payload_accepted: source_payload_id != 0,
            routed: true,
            status: plan.status.as_u8(),
            error_code: pbft_vote_ingress_error_code(plan.status).to_owned(),
            queued_effect_count,
            application_effect_id,
        }
    }

    fn decision_from_vote_plan_with_bundle(
        &mut self,
        plan: PbftVoteIngressPlan,
        fact: PbftVoteIngressFact,
        context: NetworkPbftVoteIngressContext,
        bundle: PendingVoteBundleMember,
    ) -> NetworkIngressDecision {
        let source_payload_id = context.source_payload_id;
        let before_effects = self.pending_effects.len();
        self.enqueue_vote_plan_effects(plan, fact, &context);
        let application_effect_id = self.enqueue_vote_admission_effect(context, Some(bundle));
        NetworkIngressDecision {
            payload_id: source_payload_id,
            payload_accepted: source_payload_id != 0,
            routed: true,
            status: plan.status.as_u8(),
            error_code: pbft_vote_ingress_error_code(plan.status).to_owned(),
            queued_effect_count: self.pending_effects.len().saturating_sub(before_effects) as u32,
            application_effect_id,
        }
    }

    fn enqueue_vote_admission_effect(
        &mut self,
        context: NetworkPbftVoteIngressContext,
        bundle: Option<PendingVoteBundleMember>,
    ) -> u64 {
        let vote_hash = context.vote_hash;
        let vote_rlp = context.vote_rlp;
        let pbft_block_rlp = context.pbft_block_rlp;
        let pbft_block_hash = context.pbft_block_hash;
        let vote_effect_id = self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: context.source_payload_id,
            transport_lane: context.transport_lane,
            kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
            peer_id: context.peer_id,
            packet_kind: 0,
            payload_bytes: vote_rlp.clone(),
            related_payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
            object_hash: vote_hash,
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id: 0,
            period: 0,
            round: 0,
        });

        self.pending_vote_admissions.insert(
            vote_effect_id,
            PendingVoteAdmissionContext {
                transport_lane: context.transport_lane,
                peer_id: context.peer_id,
                vote_hash,
                vote_rlp,
                pbft_block_rlp,
                pbft_block_hash,
                pbft_block_period: context.pbft_block_period,
                source_payload_id: context.source_payload_id,
                allow_gossip: context.allow_gossip,
                bundle,
            },
        );
        vote_effect_id
    }

    fn enqueue_vote_admission_follow_ups(
        &mut self,
        context: PendingVoteAdmissionContext,
        result: &NetworkEffectResult,
    ) {
        let bundle = context.bundle.clone();
        if result.admission_report_slashing {
            if let Some(member) = bundle {
                self.cancel_vote_bundle(member.bundle_id);
            }
            return;
        }
        let vote_payload = context.vote_rlp;
        let block_payload = context.pbft_block_rlp;
        let block_effect_id = if block_payload.is_empty()
            || !result.admission_accepted && !result.admission_already_present
        {
            0
        } else {
            let block_effect_id = self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: block_payload.clone(),
                related_payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_BLOCK,
                object_hash: context.pbft_block_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: context.pbft_block_period,
                round: 0,
            });
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                related_payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_BLOCK,
                object_hash: context.pbft_block_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: block_effect_id,
                period: context.pbft_block_period,
                round: 0,
            });
            block_effect_id
        };

        if result.admission_mark_vote_known || result.admission_already_present {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                related_payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                object_hash: context.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }

        if result.admission_accepted && result.admission_gossip_vote && context.allow_gossip {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_GOSSIP_PACKET,
                peer_id: context.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PBFT_VOTE,
                payload_bytes: vote_payload,
                related_payload_bytes: block_payload,
                exclude_peers: vec![context.peer_id],
                object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                object_hash: context.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: block_effect_id,
                period: 0,
                round: 0,
            });
        }

        if let Some(member) = bundle {
            self.record_vote_bundle_admission(member, result.admission_accepted);
        }
    }

    fn enqueue_pillar_vote_admission_effect(
        &mut self,
        context: NetworkPillarVoteIngressContext,
        vote_hash: [u8; 32],
        vote_period: u64,
        vote_rlp: Vec<u8>,
    ) -> u64 {
        let source_payload_id = context.source_payload_id;
        let effect_id = self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id,
            transport_lane: context.transport_lane,
            kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
            peer_id: context.peer_id,
            packet_kind: 0,
            payload_bytes: vote_rlp.clone(),
            related_payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
            object_hash: vote_hash,
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id: 0,
            period: vote_period,
            round: 0,
        });

        self.pending_pillar_vote_admissions.insert(
            effect_id,
            PendingPillarVoteAdmissionContext {
                transport_lane: context.transport_lane,
                peer_id: context.peer_id,
                vote_hash,
                vote_rlp,
                period: vote_period,
                source_payload_id,
                allow_gossip: context.allow_gossip,
            },
        );
        effect_id
    }

    fn enqueue_pillar_vote_admission_follow_ups(
        &mut self,
        context: PendingPillarVoteAdmissionContext,
        result: &NetworkEffectResult,
    ) {
        if result.admission_accepted {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                related_payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                object_hash: context.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: context.period,
                round: 0,
            });
        }

        if result.admission_accepted && context.allow_gossip {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_GOSSIP_PACKET,
                peer_id: context.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PILLAR_VOTE,
                payload_bytes: context.vote_rlp,
                related_payload_bytes: Vec::new(),
                exclude_peers: vec![context.peer_id],
                object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                object_hash: context.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: context.period,
                round: 0,
            });
        }
    }

    /// Records one exact-ID-correlated bundle admission result and emits one
    /// accepted-only optimized bundle gossip effect after every member has
    /// completed. Empty accepted sets intentionally produce no effect.
    fn record_vote_bundle_admission(&mut self, member: PendingVoteBundleMember, accepted: bool) {
        let Some(bundle) = self.pending_vote_bundles.get_mut(&member.bundle_id) else {
            return;
        };
        if member.index >= bundle.completed.len() || bundle.completed[member.index] {
            return;
        }
        bundle.completed[member.index] = true;
        if accepted {
            bundle.accepted_votes[member.index] = Some(member.vote);
        }
        if bundle.completed.iter().any(|completed| !completed) {
            return;
        }

        let bundle = self
            .pending_vote_bundles
            .remove(&member.bundle_id)
            .expect("bundle exists");
        let accepted_votes = bundle
            .accepted_votes
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let Some(first) = accepted_votes.first() else {
            return;
        };
        let Ok(inspection) = inspect_canonical_pbft_vote(&first.vote_rlp) else {
            return;
        };
        let Ok(payload) = build_optimized_pbft_vote_bundle(
            &accepted_votes,
            inspection.block_hash,
            inspection.period,
            inspection.round,
            inspection.step,
        ) else {
            return;
        };
        self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: bundle.source_payload_id,
            transport_lane: bundle.transport_lane,
            kind: NETWORK_EFFECT_KIND_GOSSIP_PACKET,
            peer_id: bundle.peer_id,
            packet_kind: NETWORK_PACKET_KIND_PBFT_VOTES_BUNDLE,
            payload_bytes: payload.bundle_rlp,
            related_payload_bytes: Vec::new(),
            exclude_peers: vec![bundle.peer_id],
            object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
            object_hash: [0; 32],
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id: 0,
            period: inspection.period,
            round: inspection.round,
        });
    }

    /// Cancels one bundle aggregation session and removes admission effects
    /// that have not yet crossed the application boundary.
    fn cancel_vote_bundle(&mut self, bundle_id: u64) {
        self.pending_vote_bundles.remove(&bundle_id);
        let cancelled_effect_ids = self
            .pending_vote_admissions
            .iter()
            .filter_map(|(effect_id, context)| {
                context
                    .bundle
                    .as_ref()
                    .filter(|bundle| bundle.bundle_id == bundle_id)
                    .map(|_| *effect_id)
            })
            .collect::<HashSet<_>>();
        self.pending_vote_admissions
            .retain(|effect_id, _| !cancelled_effect_ids.contains(effect_id));
        self.pending_effects
            .retain(|effect| !cancelled_effect_ids.contains(&effect.effect_id));
    }

    fn enqueue_next_votes_bundle_send_effects(
        &mut self,
        request: NetworkPbftNextVotesBundleRequest,
        chunks: Vec<Vec<u8>>,
    ) -> NetworkIngressDecision {
        let queued_effect_count = u32::try_from(chunks.len()).unwrap_or(u32::MAX);
        let round = request.current_round - 1;
        for payload_bytes in chunks {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: request.source_payload_id,
                transport_lane: request.transport_lane,
                kind: NETWORK_EFFECT_KIND_SEND_PACKET,
                peer_id: request.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PBFT_VOTES_BUNDLE,
                payload_bytes,
                related_payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                object_hash: [0; 32],
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: request.current_period,
                round,
            });
        }
        NetworkIngressDecision {
            payload_id: request.source_payload_id,
            payload_accepted: request.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count,
            application_effect_id: 0,
        }
    }

    fn enqueue_pillar_vote_bundle_send_effects(
        &mut self,
        request: NetworkGetPillarVotesBundleRequest,
        chunks: Vec<PillarVoteNetworkChunk>,
    ) -> NetworkIngressDecision {
        let queued_effect_count = chunks.iter().fold(0_u32, |count, chunk| {
            count.saturating_add(
                1_u32.saturating_add(u32::try_from(chunk.vote_hashes.len()).unwrap_or(u32::MAX)),
            )
        });
        for chunk in chunks {
            let send_id = self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: request.source_payload_id,
                transport_lane: request.transport_lane,
                kind: NETWORK_EFFECT_KIND_SEND_PACKET,
                peer_id: request.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PILLAR_VOTES_BUNDLE,
                payload_bytes: chunk.payload_bytes,
                related_payload_bytes: Vec::new(),
                exclude_peers: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                object_hash: request.pillar_block_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: request.period,
                round: 0,
            });
            for vote_hash in chunk.vote_hashes {
                self.enqueue_effect(NetworkEffect {
                    effect_id: 0,
                    source_payload_id: request.source_payload_id,
                    transport_lane: request.transport_lane,
                    kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                    peer_id: request.peer_id,
                    packet_kind: 0,
                    payload_bytes: Vec::new(),
                    related_payload_bytes: Vec::new(),
                    exclude_peers: Vec::new(),
                    object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                    object_hash: vote_hash,
                    sync_kind: 0,
                    sync_start: 0,
                    reason_code: 0,
                    dependency_id: send_id,
                    period: request.period,
                    round: 0,
                });
            }
        }
        NetworkIngressDecision {
            payload_id: request.source_payload_id,
            payload_accepted: request.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count,
            application_effect_id: 0,
        }
    }

    fn enqueue_vote_plan_effects(
        &mut self,
        plan: PbftVoteIngressPlan,
        fact: PbftVoteIngressFact,
        context: &NetworkPbftVoteIngressContext,
    ) {
        if plan.request_pbft_sync {
            let sync_start = fact
                .period
                .saturating_sub(1)
                .max(context.peer_pbft_chain_size);
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_REQUEST_SYNC,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                related_payload_bytes: Vec::new(),
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
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_REQUEST_SYNC,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                related_payload_bytes: Vec::new(),
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
                    transport_lane: context.transport_lane,
                    kind: NETWORK_EFFECT_KIND_REPORT_PEER,
                    peer_id: context.peer_id,
                    packet_kind: 0,
                    payload_bytes: Vec::new(),
                    related_payload_bytes: Vec::new(),
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
                    transport_lane: context.transport_lane,
                    kind: NETWORK_EFFECT_KIND_DISCONNECT_PEER,
                    peer_id: context.peer_id,
                    packet_kind: 0,
                    payload_bytes: Vec::new(),
                    related_payload_bytes: Vec::new(),
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
                    transport_lane: context.transport_lane,
                    kind: NETWORK_EFFECT_KIND_REPORT_PEER,
                    peer_id: context.peer_id,
                    packet_kind: 0,
                    payload_bytes: Vec::new(),
                    related_payload_bytes: Vec::new(),
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

    /// Enqueues an executor effect for tests and future packet-specific
    /// planners.
    pub fn enqueue_effect(&mut self, mut effect: NetworkEffect) -> u64 {
        let effect_id = self.next_effect_id;
        self.next_effect_id = self.next_effect_id.saturating_add(1);
        effect.effect_id = effect_id;
        self.pending_effects.push_back(effect);
        effect_id
    }
}

fn next_votes_request_rejection(
    request: &NetworkPbftNextVotesBundleRequest,
) -> Option<NetworkIngressDecision> {
    let (status, error_code) = if request.current_period != request.peer_period {
        (
            NETWORK_INGRESS_STATUS_NEXT_VOTES_PERIOD_MISMATCH,
            ERROR_NEXT_VOTES_PERIOD_MISMATCH,
        )
    } else if request.current_round <= 1 {
        (
            NETWORK_INGRESS_STATUS_NEXT_VOTES_NO_PREVIOUS_ROUND,
            ERROR_NEXT_VOTES_NO_PREVIOUS_ROUND,
        )
    } else if request.current_round < request.peer_round {
        (
            NETWORK_INGRESS_STATUS_NEXT_VOTES_PEER_ROUND_AHEAD,
            ERROR_NEXT_VOTES_PEER_ROUND_AHEAD,
        )
    } else {
        return None;
    };
    Some(NetworkIngressDecision {
        payload_id: request.source_payload_id,
        payload_accepted: request.source_payload_id != 0,
        routed: true,
        status,
        error_code: error_code.to_owned(),
        queued_effect_count: 0,
        application_effect_id: 0,
    })
}

fn local_network_decision(
    source_payload_id: u64,
    status: u8,
    error_code: &str,
) -> NetworkIngressDecision {
    NetworkIngressDecision {
        payload_id: source_payload_id,
        payload_accepted: source_payload_id != 0,
        routed: true,
        status,
        error_code: error_code.to_owned(),
        queued_effect_count: 0,
        application_effect_id: 0,
    }
}

fn decode_get_pbft_sync_request(request_rlp: &[u8]) -> Option<u64> {
    let request = Rlp::new(request_rlp);
    if !request.is_list() || request.item_count().ok()? != 1 {
        return None;
    }
    let height: u64 = request.val_at(0).ok()?;
    let mut canonical = RlpStream::new_list(1);
    canonical.append(&height);
    (canonical.out().as_ref() == request_rlp).then_some(height)
}

fn encode_pbft_sync_packet(
    last_block: bool,
    period_data_rlp: &[u8],
    reward_votes_bundle_rlp: Option<&[u8]>,
) -> Vec<u8> {
    let mut packet = RlpStream::new_list(3);
    packet.append(&last_block);
    packet.append_raw(period_data_rlp, 1);
    if let Some(reward_votes_bundle_rlp) = reward_votes_bundle_rlp {
        packet.append_raw(reward_votes_bundle_rlp, 1);
    } else {
        packet.append(&0u8);
    }
    packet.out().to_vec()
}

fn encode_proposed_block_bundles(
    proposed_blocks: Vec<crate::proposed_blocks::ProposedBlockEntry>,
) -> Vec<Vec<u8>> {
    proposed_blocks
        .chunks(MAX_PROPOSED_BLOCKS_PER_BUNDLE_PACKET)
        .map(|blocks| {
            let mut packet = RlpStream::new_list(1);
            packet.begin_list(blocks.len());
            for block in blocks {
                packet.append_raw(&block.block_rlp, 1);
            }
            packet.out().to_vec()
        })
        .collect()
}

fn native_lock_poisoned(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string();
        message.contains("LOCK_POISONED") || message.contains("lock poisoned")
    })
}

fn validate_next_votes_payloads(
    payloads: PbftNextVotesBundleEgressPayloads,
    period: u64,
    round: u64,
) -> Result<Vec<Vec<u8>>> {
    let next =
        validate_and_chunk_next_votes_bundle(&payloads.next_votes_bundle_rlp, period, round, false)
            .ok_or_else(|| anyhow!("NETWORK_NEXT_VOTES_NATIVE_PAYLOAD_INVALID"))?;
    let next_null = validate_and_chunk_next_votes_bundle(
        &payloads.next_null_votes_bundle_rlp,
        period,
        round,
        true,
    )
    .ok_or_else(|| anyhow!("NETWORK_NEXT_NULL_VOTES_NATIVE_PAYLOAD_INVALID"))?;
    Ok(next.into_iter().chain(next_null).collect())
}

fn validate_and_chunk_next_votes_bundle(
    payload: &[u8],
    expected_period: u64,
    expected_round: u64,
    expect_null_block: bool,
) -> Option<Vec<Vec<u8>>> {
    if payload.is_empty() {
        return Some(Vec::new());
    }
    let bundle = Rlp::new(payload);
    if !bundle.is_list() || bundle.item_count().ok()? != 5 {
        return None;
    }
    let block_hash: H256 = bundle.val_at(0).ok()?;
    let period: u64 = bundle.val_at(1).ok()?;
    let round: u64 = bundle.val_at(2).ok()?;
    let step: u64 = bundle.val_at(3).ok()?;
    if period != expected_period
        || round != expected_round
        || block_hash.is_zero() != expect_null_block
    {
        return None;
    }
    let votes = bundle.at(4).ok()?;
    if !votes.is_list() {
        return None;
    }
    let vote_count = votes.item_count().ok()?;
    if vote_count == 0 {
        return None;
    }
    let mut raw_votes = Vec::with_capacity(vote_count);
    for index in 0..vote_count {
        let vote = votes.at(index).ok()?;
        if !vote.is_list() || vote.item_count().ok()? != 2 {
            return None;
        }
        let proof = vote.at(0).ok()?.data().ok()?;
        let signature = vote.at(1).ok()?.data().ok()?;
        if proof.len() != 80 || signature.len() != 65 {
            return None;
        }
        let mut sortition = RlpStream::new_list(4);
        sortition.append(&period);
        sortition.append(&round);
        sortition.append(&step);
        sortition.append(&proof);
        let sortition = sortition.out().to_vec();
        let mut canonical_vote = RlpStream::new_list(3);
        canonical_vote.append(&block_hash);
        canonical_vote.append(&sortition);
        canonical_vote.append(&signature);
        let inspection = inspect_canonical_pbft_vote(&canonical_vote.out()).ok()?;
        if !inspection.signature_valid
            || inspection.block_hash != block_hash
            || inspection.period != period
            || inspection.round != round
            || inspection.step != step
            || inspection.vote_type != crate::verified_votes::PbftVoteType::Next
        {
            return None;
        }
        raw_votes.push(vote.as_raw());
    }

    Some(
        raw_votes
            .chunks(MAX_VOTES_PER_BUNDLE_PACKET)
            .map(|chunk| {
                let mut stream = RlpStream::new_list(5);
                stream.append(&block_hash);
                stream.append(&period);
                stream.append(&round);
                stream.append(&step);
                stream.begin_list(chunk.len());
                for vote in chunk {
                    stream.append_raw(vote, 1);
                }
                stream.out().to_vec()
            })
            .collect(),
    )
}

fn validate_and_chunk_pillar_votes(
    records: Vec<PillarVoteRecord>,
    expected_period: u64,
    expected_block_hash: [u8; 32],
) -> Result<Vec<PillarVoteNetworkChunk>> {
    let expected_block_hash = H256::from(expected_block_hash);
    let mut seen = HashSet::with_capacity(records.len());
    let mut validated = Vec::with_capacity(records.len());
    for record in records {
        let vote = PillarVote::decode_rlp(&record.vote_rlp)
            .context("NETWORK_PILLAR_VOTE_NATIVE_PAYLOAD_DECODE_FAILED")?;
        ensure!(
            vote.encode_rlp() == record.vote_rlp,
            "NETWORK_PILLAR_VOTE_NATIVE_PAYLOAD_NONCANONICAL"
        );
        ensure!(
            vote.period == expected_period && vote.block_hash == expected_block_hash,
            "NETWORK_PILLAR_VOTE_NATIVE_PAYLOAD_IDENTITY_MISMATCH"
        );
        let inspection = inspect_pillar_vote_from_rlp(&record.vote_rlp)
            .context("NETWORK_PILLAR_VOTE_NATIVE_PAYLOAD_INSPECTION_FAILED")?;
        ensure!(
            inspection.signature_valid,
            "NETWORK_PILLAR_VOTE_NATIVE_PAYLOAD_INVALID_SIGNATURE"
        );
        let vote_hash = inspection.vote_hash.to_fixed_bytes();
        ensure!(
            vote_hash == record.vote_hash,
            "NETWORK_PILLAR_VOTE_NATIVE_PAYLOAD_HASH_MISMATCH"
        );
        ensure!(
            seen.insert(vote_hash),
            "NETWORK_PILLAR_VOTE_NATIVE_PAYLOAD_DUPLICATE_HASH"
        );
        validated.push((vote, vote_hash));
    }

    let mut chunks =
        Vec::with_capacity(validated.len().div_ceil(MAX_PILLAR_VOTES_PER_BUNDLE_PACKET));
    for entries in validated.chunks(MAX_PILLAR_VOTES_PER_BUNDLE_PACKET) {
        let votes = entries
            .iter()
            .map(|(vote, _)| vote.clone())
            .collect::<Vec<_>>();
        let payload_bytes = encode_optimized_pillar_votes_bundle_rlp(&votes)
            .context("NETWORK_PILLAR_VOTE_NATIVE_BUNDLE_ENCODING_FAILED")?;
        let decoded = rustaxa_types::decode_optimized_pillar_votes_bundle_rlp(&payload_bytes)
            .context("NETWORK_PILLAR_VOTE_NATIVE_BUNDLE_REVALIDATION_FAILED")?;
        ensure!(
            decoded == votes,
            "NETWORK_PILLAR_VOTE_NATIVE_BUNDLE_ORDER_MISMATCH"
        );
        chunks.push(PillarVoteNetworkChunk {
            vote_hashes: entries.iter().map(|(_, hash)| *hash).collect(),
            payload_bytes,
        });
    }
    Ok(chunks)
}

fn plan_status_sync(facts: NetworkStatusSyncFacts) -> NetworkStatusSyncPlan {
    if facts.local_pbft_syncing {
        return NetworkStatusSyncPlan {
            request_pbft_sync: false,
            request_pending_dag_blocks: false,
            request_next_votes: false,
            next_votes_period: 0,
            next_votes_round: 0,
        };
    }

    let request_pbft_sync = facts.local_pbft_synced_period < facts.peer_pbft_chain_size
        && (facts.local_pbft_synced_period + 1 < facts.peer_pbft_chain_size
            || facts.peer_last_status_pbft_chain_size == facts.peer_pbft_chain_size);
    let request_pending_dag_blocks = !request_pbft_sync
        && facts.local_pbft_synced_period == facts.peer_pbft_chain_size
        && !facts.peer_dag_synced;
    let request_next_votes = facts.local_pbft_period == facts.peer_pbft_period
        && facts.local_pbft_round < facts.peer_pbft_round;

    NetworkStatusSyncPlan {
        request_pbft_sync,
        request_pending_dag_blocks,
        request_next_votes,
        next_votes_period: if request_next_votes {
            facts.local_pbft_period
        } else {
            0
        },
        next_votes_round: if request_next_votes {
            facts.local_pbft_round
        } else {
            0
        },
    }
}

fn plan_status_egress(facts: NetworkStatusEgressFacts) -> NetworkStatusEgressPlan {
    NetworkStatusEgressPlan {
        status: NETWORK_STATUS_PLAN_STATUS_OK,
        error_code: ERROR_NONE.to_owned(),
        peer_pbft_chain_size: facts.local_pbft_chain_size,
        peer_pbft_round: facts.local_pbft_round,
        peer_dag_level: facts.local_dag_level,
        peer_syncing: if facts.initial {
            facts.pbft_syncing
        } else {
            facts.deep_pbft_syncing
        },
        include_initial_data: facts.initial,
        chain_id: if facts.initial {
            facts.local_chain_id
        } else {
            0
        },
        genesis_hash: if facts.initial {
            facts.genesis_hash
        } else {
            [0; 32]
        },
        node_major_version: if facts.initial {
            facts.node_major_version
        } else {
            0
        },
        node_minor_version: if facts.initial {
            facts.node_minor_version
        } else {
            0
        },
        node_patch_version: if facts.initial {
            facts.node_patch_version
        } else {
            0
        },
        is_light_node: facts.initial && facts.is_light_node,
        light_node_history: if facts.initial {
            facts.light_node_history
        } else {
            0
        },
    }
}

fn plan_initial_status(facts: NetworkInitialStatusFacts) -> NetworkInitialStatusPlan {
    if facts.peer_chain_id != facts.local_chain_id {
        return NetworkInitialStatusPlan {
            status: NETWORK_STATUS_PLAN_STATUS_CHAIN_ID_MISMATCH,
            error_code: "NETWORK_STATUS_CHAIN_ID_MISMATCH".to_owned(),
            accept_peer: false,
            disconnect_peer: true,
        };
    }

    if facts.peer_genesis_hash != facts.expected_genesis_hash {
        return NetworkInitialStatusPlan {
            status: NETWORK_STATUS_PLAN_STATUS_GENESIS_MISMATCH,
            error_code: "NETWORK_STATUS_GENESIS_MISMATCH".to_owned(),
            accept_peer: false,
            disconnect_peer: true,
        };
    }

    if facts.peer_is_light_node
        && facts
            .local_pbft_synced_period
            .saturating_add(facts.peer_light_node_history)
            < facts.peer_pbft_chain_size
    {
        return NetworkInitialStatusPlan {
            status: NETWORK_STATUS_PLAN_STATUS_LIGHT_NODE_HISTORY_UNAVAILABLE,
            error_code: "NETWORK_STATUS_LIGHT_NODE_HISTORY_UNAVAILABLE".to_owned(),
            accept_peer: false,
            disconnect_peer: true,
        };
    }

    NetworkInitialStatusPlan {
        status: NETWORK_STATUS_PLAN_STATUS_OK,
        error_code: ERROR_NONE.to_owned(),
        accept_peer: true,
        disconnect_peer: false,
    }
}

fn plan_pbft_sync_start(facts: NetworkPbftSyncStartFacts) -> NetworkPbftSyncStartPlan {
    if facts.local_pbft_syncing {
        return NetworkPbftSyncStartPlan {
            status: NETWORK_STATUS_PLAN_STATUS_ALREADY_SYNCING,
            error_code: "NETWORK_STATUS_ALREADY_SYNCING".to_owned(),
            start_sync: false,
            has_peer: false,
            peer_id: [0; 64],
            peer_pbft_chain_size: 0,
            request_period: 0,
            enable_snapshot_creation: false,
        };
    }

    let selected =
        select_serviceable_max_chain_peer(facts.candidates, facts.local_pbft_synced_period, |_| {
            true
        });

    let Some(peer) = selected else {
        return NetworkPbftSyncStartPlan {
            status: NETWORK_STATUS_PLAN_STATUS_NO_ELIGIBLE_PEER,
            error_code: "NETWORK_STATUS_NO_ELIGIBLE_SYNC_PEER".to_owned(),
            start_sync: false,
            has_peer: false,
            peer_id: [0; 64],
            peer_pbft_chain_size: 0,
            request_period: 0,
            enable_snapshot_creation: false,
        };
    };

    if peer.pbft_chain_size <= facts.local_pbft_synced_period {
        return NetworkPbftSyncStartPlan {
            status: NETWORK_STATUS_PLAN_STATUS_SYNC_NOT_NEEDED,
            error_code: "NETWORK_STATUS_SYNC_NOT_NEEDED".to_owned(),
            start_sync: false,
            has_peer: true,
            peer_id: peer.peer_id,
            peer_pbft_chain_size: peer.pbft_chain_size,
            request_period: 0,
            enable_snapshot_creation: true,
        };
    }

    NetworkPbftSyncStartPlan {
        status: NETWORK_STATUS_PLAN_STATUS_OK,
        error_code: ERROR_NONE.to_owned(),
        start_sync: true,
        has_peer: true,
        peer_id: peer.peer_id,
        peer_pbft_chain_size: peer.pbft_chain_size,
        request_period: facts.local_pbft_synced_period + 1,
        enable_snapshot_creation: false,
    }
}

fn plan_max_chain_peer_selection(facts: NetworkPeerSelectionFacts) -> NetworkPeerSelectionPlan {
    let selected = select_serviceable_max_chain_peer(
        facts.candidates,
        facts.local_pbft_syncing_period,
        |_| true,
    );

    let Some(peer) = selected else {
        return NetworkPeerSelectionPlan {
            status: NETWORK_STATUS_PLAN_STATUS_NO_ELIGIBLE_PEER,
            error_code: "NETWORK_STATUS_NO_ELIGIBLE_PEER".to_owned(),
            has_peer: false,
            peer_id: [0; 64],
            peer_pbft_chain_size: 0,
        };
    };

    NetworkPeerSelectionPlan {
        status: NETWORK_STATUS_PLAN_STATUS_OK,
        error_code: ERROR_NONE.to_owned(),
        has_peer: true,
        peer_id: peer.peer_id,
        peer_pbft_chain_size: peer.pbft_chain_size,
    }
}

fn plan_pending_dag_blocks_request(
    facts: NetworkPendingDagBlocksRequestFacts,
) -> NetworkPendingDagBlocksRequestPlan {
    let selected = if facts.has_explicit_peer {
        Some(facts.explicit_peer)
    } else {
        select_serviceable_max_chain_peer(
            facts.candidates,
            facts.local_pbft_syncing_period,
            |candidate| !candidate.peer_dag_synced && candidate.dag_sync_allowed,
        )
    };

    let Some(peer) = selected else {
        return NetworkPendingDagBlocksRequestPlan {
            status: NETWORK_STATUS_PLAN_STATUS_NO_ELIGIBLE_PEER,
            error_code: "NETWORK_STATUS_NO_ELIGIBLE_DAG_PEER".to_owned(),
            request_pending_dag_blocks: false,
            has_peer: false,
            peer_id: [0; 64],
            request_period: 0,
        };
    };

    if peer.peer_dag_synced {
        return NetworkPendingDagBlocksRequestPlan {
            status: NETWORK_STATUS_PLAN_STATUS_DAG_ALREADY_SYNCED,
            error_code: "NETWORK_STATUS_DAG_ALREADY_SYNCED".to_owned(),
            request_pending_dag_blocks: false,
            has_peer: true,
            peer_id: peer.peer_id,
            request_period: 0,
        };
    }

    if facts.local_pbft_syncing_period != peer.pbft_chain_size {
        return NetworkPendingDagBlocksRequestPlan {
            status: NETWORK_STATUS_PLAN_STATUS_DAG_PERIOD_MISMATCH,
            error_code: "NETWORK_STATUS_DAG_PERIOD_MISMATCH".to_owned(),
            request_pending_dag_blocks: false,
            has_peer: true,
            peer_id: peer.peer_id,
            request_period: 0,
        };
    }

    NetworkPendingDagBlocksRequestPlan {
        status: NETWORK_STATUS_PLAN_STATUS_OK,
        error_code: ERROR_NONE.to_owned(),
        request_pending_dag_blocks: true,
        has_peer: true,
        peer_id: peer.peer_id,
        request_period: facts.local_pbft_syncing_period,
    }
}

fn select_serviceable_max_chain_peer(
    candidates: Vec<NetworkPbftSyncPeerCandidate>,
    local_pbft_syncing_period: u64,
    filter: impl Fn(&NetworkPbftSyncPeerCandidate) -> bool,
) -> Option<NetworkPbftSyncPeerCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| {
            filter(candidate)
                && (!candidate.is_light_node
                    || local_pbft_syncing_period.saturating_add(candidate.light_node_history)
                        >= candidate.pbft_chain_size)
        })
        .max_by(|left, right| {
            left.pbft_chain_size
                .cmp(&right.pbft_chain_size)
                .then_with(|| left.dag_level.cmp(&right.dag_level))
        })
}

fn pillar_vote_bundle_rejection(
    context: &NetworkPillarVoteIngressContext,
    vote_count: usize,
    status: u8,
    error_code: &str,
) -> Vec<NetworkIngressDecision> {
    (0..vote_count)
        .map(|_| NetworkIngressDecision {
            payload_id: context.source_payload_id,
            payload_accepted: context.source_payload_id != 0,
            routed: true,
            status,
            error_code: error_code.to_owned(),
            queued_effect_count: 0,
            application_effect_id: 0,
        })
        .collect()
}

fn effect_result_matches_effect(result: &NetworkEffectResult, effect: &NetworkEffect) -> bool {
    let identity_matches = result.kind == effect.kind
        && result.peer_id == effect.peer_id
        && result.packet_kind == effect.packet_kind
        && result.object_kind == effect.object_kind
        && result.object_hash == effect.object_hash;
    if !identity_matches {
        return false;
    }

    let has_admission_outcome = result.admission_accepted
        || result.admission_already_present
        || result.admission_mark_vote_known
        || result.admission_gossip_vote
        || result.admission_report_slashing;
    let is_vote_admission = effect.kind == NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        && matches!(
            effect.object_kind,
            NETWORK_OBJECT_KIND_PBFT_VOTE | NETWORK_OBJECT_KIND_PILLAR_VOTE
        );
    if !is_vote_admission || result.status == NETWORK_EFFECT_RESULT_STATUS_FAILED {
        return !has_admission_outcome;
    }

    !(result.admission_accepted && result.admission_already_present)
        && (!result.admission_gossip_vote || result.admission_accepted)
        && (!result.admission_mark_vote_known
            || result.admission_accepted
            || result.admission_already_present)
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
    use crate::pbft_vote_generation::{PbftVoteGenerationInput, generate_pbft_vote};
    use crate::verified_votes::PbftVoteType;
    use k256::ecdsa::SigningKey;
    use rlp::Rlp;
    use rustaxa_types::PillarVote;
    use rustaxa_vdf::vrf;
    use tiny_keccak::{Hasher, Keccak};

    const TEST_VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn peer(byte: u8) -> [u8; 64] {
        [byte; 64]
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

    const fn status_sync_facts() -> NetworkStatusSyncFacts {
        NetworkStatusSyncFacts {
            local_pbft_syncing: false,
            local_pbft_synced_period: 10,
            local_pbft_period: 11,
            local_pbft_round: 2,
            peer_pbft_chain_size: 10,
            peer_pbft_period: 11,
            peer_pbft_round: 2,
            peer_dag_synced: true,
            peer_last_status_pbft_chain_size: 9,
        }
    }

    fn status_egress_facts(initial: bool) -> NetworkStatusEgressFacts {
        NetworkStatusEgressFacts {
            initial,
            local_chain_id: 7,
            genesis_hash: hash(1),
            node_major_version: 2,
            node_minor_version: 3,
            node_patch_version: 4,
            is_light_node: true,
            light_node_history: 8,
            local_pbft_chain_size: 10,
            local_pbft_round: 5,
            local_dag_level: 44,
            pbft_syncing: true,
            deep_pbft_syncing: false,
        }
    }

    fn initial_status_facts() -> NetworkInitialStatusFacts {
        NetworkInitialStatusFacts {
            local_chain_id: 7,
            peer_chain_id: 7,
            expected_genesis_hash: hash(1),
            peer_genesis_hash: hash(1),
            local_pbft_synced_period: 10,
            peer_pbft_chain_size: 12,
            peer_is_light_node: false,
            peer_light_node_history: 0,
        }
    }

    fn sync_candidate(
        byte: u8,
        pbft_chain_size: u64,
        dag_level: u64,
    ) -> NetworkPbftSyncPeerCandidate {
        NetworkPbftSyncPeerCandidate {
            peer_id: peer(byte),
            pbft_chain_size,
            dag_level,
            is_light_node: false,
            light_node_history: 0,
            peer_dag_synced: false,
            peer_dag_syncing: false,
            dag_sync_allowed: true,
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
            transport_lane: 6,
            peer_id: peer(7),
            peer_pbft_chain_size: 11,
            source_payload_id: 99,
            enqueue_admission: false,
            allow_gossip: false,
            vote_hash: [0; 32],
            vote_rlp: Vec::new(),
            pbft_block_rlp: Vec::new(),
            pbft_block_hash: [0; 32],
            pbft_block_period: 0,
        }
    }

    fn pillar_vote_context(allow_gossip: bool) -> NetworkPillarVoteIngressContext {
        NetworkPillarVoteIngressContext {
            transport_lane: 6,
            peer_id: peer(8),
            source_payload_id: 101,
            ficus_activation_period: 10,
            allow_gossip,
        }
    }

    fn signed_pillar_vote(seed: u8, period: u64, block_hash: u64) -> PillarVote {
        let signing_key = SigningKey::from_slice(&[seed; 32]).unwrap();
        let mut vote = PillarVote {
            period,
            block_hash: H256::from_low_u64_be(block_hash),
            signature: [0; 65],
        };
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(vote.hash(false).as_bytes())
            .unwrap();
        vote.signature[..64].copy_from_slice(&signature.to_bytes());
        vote.signature[64] = recovery_id.to_byte();
        vote
    }

    fn pillar_request(period: u64, block_hash: H256) -> NetworkGetPillarVotesBundleRequest {
        NetworkGetPillarVotesBundleRequest {
            transport_lane: 6,
            peer_id: peer(8),
            period,
            pillar_block_hash: block_hash.into(),
            source_payload_id: 102,
        }
    }

    fn pillar_record(vote: &PillarVote) -> PillarVoteRecord {
        PillarVoteRecord {
            vote_hash: vote.hash(true).into(),
            weight: 0,
            vote_rlp: vote.encode_rlp(),
        }
    }

    fn hash(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn next_votes_request() -> NetworkPbftNextVotesBundleRequest {
        NetworkPbftNextVotesBundleRequest {
            transport_lane: 6,
            peer_id: peer(7),
            peer_period: 10,
            peer_round: 2,
            current_period: 10,
            current_round: 3,
            source_payload_id: 99,
        }
    }

    fn optimized_bundle(
        block_hash: [u8; 32],
        period: u64,
        round: u64,
        vote_count: usize,
        marker: u8,
    ) -> Vec<u8> {
        let node_secret = [marker; 32];
        let signing_key = SigningKey::from_slice(&node_secret).unwrap();
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let mut digest = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut digest);
        let generated = generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: H256::from(block_hash),
            vote_type: PbftVoteType::Next,
            period,
            round,
            step: 4,
            node_secret,
            vrf_secret: TEST_VRF_SECRET,
            expected_voter: ethereum_types::H160::from_slice(&digest[12..]),
            expected_vrf_public_key: vrf::public_key_from_secret(&TEST_VRF_SECRET).unwrap(),
        })
        .unwrap();
        let vote = Rlp::new(&generated.vote_rlp);
        let sortition_rlp: Vec<u8> = vote.val_at(1).unwrap();
        let proof: Vec<u8> = Rlp::new(&sortition_rlp).val_at(3).unwrap();
        let signature: Vec<u8> = vote.val_at(2).unwrap();
        let mut optimized_vote = RlpStream::new_list(2);
        optimized_vote.append(&proof);
        optimized_vote.append(&signature);
        let optimized_vote = optimized_vote.out().to_vec();

        let mut stream = RlpStream::new_list(5);
        stream.append(&H256::from(block_hash));
        stream.append(&period);
        stream.append(&round);
        stream.append(&4_u64);
        stream.begin_list(vote_count);
        for _ in 0..vote_count {
            stream.append_raw(&optimized_vote, 1);
        }
        stream.out().to_vec()
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
            admission_accepted: false,
            admission_already_present: false,
            admission_mark_vote_known: false,
            admission_gossip_vote: false,
            admission_report_slashing: false,
        }
    }

    fn canonical_bundle_vote(secret_byte: u8) -> PbftVotePayloadRecord {
        let node_secret = [secret_byte; 32];
        let vrf_secret = TEST_VRF_SECRET;
        let signing_key = SigningKey::from_slice(&node_secret).unwrap();
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let mut digest = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut digest);
        let generated = generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: H256::from(hash(0xA4)),
            vote_type: PbftVoteType::Soft,
            period: 10,
            round: 3,
            step: 2,
            node_secret,
            vrf_secret,
            expected_voter: ethereum_types::H160::from_slice(&digest[12..]),
            expected_vrf_public_key: vrf::public_key_from_secret(&vrf_secret).unwrap(),
        })
        .unwrap();
        let inspection = inspect_canonical_pbft_vote(&generated.vote_rlp).unwrap();
        PbftVotePayloadRecord {
            hash: inspection.vote_hash,
            vote_rlp: generated.vote_rlp,
        }
    }

    #[test]
    fn pillar_vote_ingress_releases_known_and_gossip_only_after_acceptance() {
        let mut api = ConsensusNetworkApi::new();
        let vote = signed_pillar_vote(0x41, 11, 90);
        let vote_rlp = vote.encode_rlp();
        let decision =
            api.ingest_pillar_vote_bundle(pillar_vote_context(true), vec![vote_rlp.clone()]);

        assert_eq!(decision.len(), 1);
        assert_eq!(decision[0].status, NETWORK_INGRESS_STATUS_ACCEPTED);
        let application = api.drain_work(6, 10);
        assert_eq!(application.effects.len(), 1);
        assert_eq!(
            application.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        assert_eq!(
            application.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PILLAR_VOTE
        );
        assert_eq!(application.effects[0].payload_bytes, vote_rlp);
        assert_eq!(
            application.effects[0].effect_id,
            decision[0].application_effect_id
        );

        let mut accepted = effect_result(&application.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        accepted.admission_accepted = true;
        assert_eq!(
            api.report_effect_results(vec![accepted]).status,
            NETWORK_EFFECT_ACK_STATUS_ACCEPTED
        );
        let follow_ups = api.drain_work(6, 10);
        assert_eq!(follow_ups.effects.len(), 2);
        assert_eq!(
            follow_ups.effects[0].kind,
            NETWORK_EFFECT_KIND_MARK_PEER_KNOWN
        );
        assert_eq!(
            follow_ups.effects[1].kind,
            NETWORK_EFFECT_KIND_GOSSIP_PACKET
        );
        assert_eq!(
            follow_ups.effects[1].packet_kind,
            NETWORK_PACKET_KIND_PILLAR_VOTE
        );
    }

    #[test]
    fn pillar_vote_bundle_preflight_is_atomic_for_malformed_and_duplicate_members() {
        let mut api = ConsensusNetworkApi::new();
        let vote = signed_pillar_vote(0x42, 11, 91).encode_rlp();

        let malformed = api
            .ingest_pillar_vote_bundle(pillar_vote_context(false), vec![vote.clone(), vec![0x7f]]);
        assert_eq!(malformed.len(), 2);
        assert!(malformed.iter().all(|decision| {
            decision.status == NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_RLP
                && decision.application_effect_id == 0
        }));
        assert!(api.drain_work(6, 10).effects.is_empty());

        let duplicate =
            api.ingest_pillar_vote_bundle(pillar_vote_context(false), vec![vote.clone(), vote]);
        assert_eq!(duplicate.len(), 2);
        assert!(duplicate.iter().all(|decision| {
            decision.status == NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_CONTEXT
                && decision.application_effect_id == 0
        }));
        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn pillar_vote_bundle_marks_only_newly_accepted_members_and_never_gossips() {
        let mut api = ConsensusNetworkApi::new();
        let decisions = api.ingest_pillar_vote_bundle(
            pillar_vote_context(false),
            vec![
                signed_pillar_vote(0x43, 11, 92).encode_rlp(),
                signed_pillar_vote(0x44, 12, 93).encode_rlp(),
            ],
        );
        assert_eq!(decisions.len(), 2);
        let applications = api.drain_work(6, 10);
        assert_eq!(applications.effects.len(), 2);
        let mut accepted = effect_result(&applications.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        accepted.admission_accepted = true;
        let mut duplicate =
            effect_result(&applications.effects[1], NETWORK_EFFECT_RESULT_STATUS_OK);
        duplicate.admission_already_present = true;
        assert_eq!(
            api.report_effect_results(vec![accepted, duplicate]).status,
            NETWORK_EFFECT_ACK_STATUS_ACCEPTED
        );

        let follow_ups = api.drain_work(6, 10);
        assert_eq!(follow_ups.effects.len(), 1);
        assert_eq!(
            follow_ups.effects[0].kind,
            NETWORK_EFFECT_KIND_MARK_PEER_KNOWN
        );
    }

    #[test]
    fn failed_pillar_vote_admission_clears_context_without_follow_ups() {
        let mut api = ConsensusNetworkApi::new();
        api.ingest_pillar_vote_bundle(
            pillar_vote_context(true),
            vec![signed_pillar_vote(0x45, 11, 94).encode_rlp()],
        );
        let application = api.drain_work(6, 1);
        assert_eq!(application.effects.len(), 1);
        assert_eq!(
            api.report_effect_results(vec![effect_result(
                &application.effects[0],
                NETWORK_EFFECT_RESULT_STATUS_FAILED,
            )])
            .status,
            NETWORK_EFFECT_ACK_STATUS_ACCEPTED
        );
        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn invalid_pillar_request_reports_before_dependent_disconnect() {
        let mut api = ConsensusNetworkApi::with_pillar_schedule(10, 10);
        let request = pillar_request(9, H256::from_low_u64_be(90));
        let decision = api.reject_invalid_pillar_votes_request(&request).unwrap();
        assert_eq!(
            decision.status,
            NETWORK_INGRESS_STATUS_PILLAR_VOTES_INACTIVE
        );
        assert_eq!(decision.queued_effect_count, 2);

        let report = api.drain_work(6, 10);
        assert_eq!(report.effects.len(), 1);
        assert_eq!(report.effects[0].kind, NETWORK_EFFECT_KIND_REPORT_PEER);
        assert!(report.more_available);
        assert_eq!(
            api.report_effect_results(vec![effect_result(
                &report.effects[0],
                NETWORK_EFFECT_RESULT_STATUS_OK,
            )])
            .status,
            NETWORK_EFFECT_ACK_STATUS_ACCEPTED
        );

        let disconnect = api.drain_work(6, 10);
        assert_eq!(disconnect.effects.len(), 1);
        assert_eq!(
            disconnect.effects[0].kind,
            NETWORK_EFFECT_KIND_DISCONNECT_PEER
        );
        assert_eq!(
            disconnect.effects[0].dependency_id,
            report.effects[0].effect_id
        );
    }

    #[test]
    fn pillar_request_requires_exact_pbft_with_pillar_period() {
        let mut api = ConsensusNetworkApi::with_pillar_schedule(10, 10);
        assert!(
            api.reject_invalid_pillar_votes_request(&pillar_request(11, H256::from_low_u64_be(90)))
                .is_none()
        );
        let decision = api
            .reject_invalid_pillar_votes_request(&pillar_request(12, H256::from_low_u64_be(90)))
            .unwrap();
        assert_eq!(
            decision.status,
            NETWORK_INGRESS_STATUS_PILLAR_VOTES_INVALID_PERIOD
        );
    }

    #[test]
    fn pillar_native_payloads_send_then_mark_each_vote_known_in_order() {
        let block_hash = H256::from_low_u64_be(91);
        let first = signed_pillar_vote(0x46, 11, 91);
        let second = signed_pillar_vote(0x47, 11, 91);
        let chunks = validate_and_chunk_pillar_votes(
            vec![pillar_record(&first), pillar_record(&second)],
            11,
            block_hash.into(),
        )
        .unwrap();
        assert_eq!(chunks.len(), 1);

        let mut api = ConsensusNetworkApi::with_pillar_schedule(10, 10);
        let decision =
            api.enqueue_pillar_vote_bundle_send_effects(pillar_request(11, block_hash), chunks);
        assert_eq!(decision.queued_effect_count, 3);
        let send = api.drain_work(6, 10);
        assert_eq!(send.effects.len(), 1);
        assert_eq!(send.effects[0].kind, NETWORK_EFFECT_KIND_SEND_PACKET);
        assert_eq!(
            send.effects[0].packet_kind,
            NETWORK_PACKET_KIND_PILLAR_VOTES_BUNDLE
        );
        assert_eq!(
            rustaxa_types::decode_optimized_pillar_votes_bundle_rlp(&send.effects[0].payload_bytes)
                .unwrap(),
            vec![first.clone(), second.clone()]
        );
        api.report_effect_results(vec![effect_result(
            &send.effects[0],
            NETWORK_EFFECT_RESULT_STATUS_OK,
        )]);
        let marks = api.drain_work(6, 10).effects;
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].object_hash, <[u8; 32]>::from(first.hash(true)));
        assert_eq!(marks[1].object_hash, <[u8; 32]>::from(second.hash(true)));
        assert!(marks.iter().all(|effect| {
            effect.kind == NETWORK_EFFECT_KIND_MARK_PEER_KNOWN
                && effect.dependency_id == send.effects[0].effect_id
        }));
    }

    #[test]
    fn pillar_native_payload_invariant_failure_queues_nothing() {
        let vote = signed_pillar_vote(0x48, 11, 92);
        let mut duplicate = pillar_record(&vote);
        duplicate.weight = 99;
        assert!(
            validate_and_chunk_pillar_votes(
                vec![pillar_record(&vote), duplicate],
                11,
                H256::from_low_u64_be(92).into(),
            )
            .is_err()
        );
        let mut api = ConsensusNetworkApi::with_pillar_schedule(10, 10);
        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn local_failure_decisions_are_typed_and_never_claim_effects() {
        for (status, error_code) in [
            (
                NETWORK_INGRESS_STATUS_LOCAL_LOOKUP_FAILED,
                ERROR_NEXT_VOTES_LOOKUP_FAILED,
            ),
            (
                NETWORK_INGRESS_STATUS_INVALID_NATIVE_RESULT,
                ERROR_PILLAR_VOTES_INVALID_NATIVE_RESULT,
            ),
            (
                NETWORK_INGRESS_STATUS_PILLAR_VOTES_NO_DATA,
                ERROR_PILLAR_VOTES_NO_DATA,
            ),
        ] {
            let decision = local_network_decision(77, status, error_code);
            assert_eq!(decision.status, status);
            assert_eq!(decision.error_code, error_code);
            assert_eq!(decision.queued_effect_count, 0);
            assert_eq!(decision.application_effect_id, 0);
        }
    }

    #[test]
    fn next_votes_request_gate_accepts_only_eligible_previous_round_query() {
        for (request, expected_status) in [
            (
                NetworkPbftNextVotesBundleRequest {
                    peer_period: 9,
                    ..next_votes_request()
                },
                NETWORK_INGRESS_STATUS_NEXT_VOTES_PERIOD_MISMATCH,
            ),
            (
                NetworkPbftNextVotesBundleRequest {
                    current_round: 1,
                    ..next_votes_request()
                },
                NETWORK_INGRESS_STATUS_NEXT_VOTES_NO_PREVIOUS_ROUND,
            ),
            (
                NetworkPbftNextVotesBundleRequest {
                    peer_round: 4,
                    ..next_votes_request()
                },
                NETWORK_INGRESS_STATUS_NEXT_VOTES_PEER_ROUND_AHEAD,
            ),
        ] {
            let decision = next_votes_request_rejection(&request).unwrap();
            assert!(decision.routed);
            assert_eq!(decision.status, expected_status);
            assert_eq!(decision.queued_effect_count, 0);
            assert_eq!(decision.application_effect_id, 0);
        }
        assert!(next_votes_request_rejection(&next_votes_request()).is_none());
    }

    #[test]
    fn next_votes_native_payloads_chunk_and_order_next_before_next_null_sends() {
        let mut api = ConsensusNetworkApi::new();
        let chunks = validate_next_votes_payloads(
            PbftNextVotesBundleEgressPayloads {
                next_votes_bundle_rlp: optimized_bundle([0x44; 32], 10, 2, 1001, 1),
                next_null_votes_bundle_rlp: optimized_bundle([0; 32], 10, 2, 1, 9),
            },
            10,
            2,
        )
        .unwrap();
        let decision = api.enqueue_next_votes_bundle_send_effects(next_votes_request(), chunks);
        assert_eq!(decision.application_effect_id, 0);
        assert_eq!(decision.queued_effect_count, 3);

        let sends = api.drain_work(6, 8).effects;
        assert_eq!(sends.len(), 3);
        assert!(sends.iter().all(|effect| {
            effect.kind == NETWORK_EFFECT_KIND_SEND_PACKET
                && effect.packet_kind == NETWORK_PACKET_KIND_PBFT_VOTES_BUNDLE
                && effect.peer_id == peer(7)
                && effect.source_payload_id == 99
        }));
        let first: H256 = Rlp::new(&sends[0].payload_bytes).val_at(0).unwrap();
        let second: H256 = Rlp::new(&sends[1].payload_bytes).val_at(0).unwrap();
        let third: H256 = Rlp::new(&sends[2].payload_bytes).val_at(0).unwrap();
        assert_eq!(first, H256::from([0x44; 32]));
        assert_eq!(second, first);
        assert!(third.is_zero());
        assert_eq!(
            Rlp::new(&sends[0].payload_bytes)
                .at(4)
                .unwrap()
                .item_count()
                .unwrap(),
            1000
        );
        assert_eq!(
            Rlp::new(&sends[1].payload_bytes)
                .at(4)
                .unwrap()
                .item_count()
                .unwrap(),
            1
        );
    }

    #[test]
    fn next_votes_native_payload_validation_rejects_swapped_family_atomically() {
        let mut api = ConsensusNetworkApi::new();
        assert!(
            validate_next_votes_payloads(
                PbftNextVotesBundleEgressPayloads {
                    next_votes_bundle_rlp: optimized_bundle([0; 32], 10, 2, 1, 1),
                    next_null_votes_bundle_rlp: optimized_bundle([0x55; 32], 10, 2, 1, 2),
                },
                10,
                2,
            )
            .is_err()
        );
        assert!(api.drain_work(6, 8).effects.is_empty());
    }

    fn bundle_context(vote: &PbftVotePayloadRecord) -> NetworkPbftVoteIngressContext {
        let mut context = vote_context();
        context.enqueue_admission = true;
        context.source_payload_id = 41;
        context.peer_id = peer(9);
        context.vote_hash = vote.hash.into();
        context.vote_rlp = vote.vote_rlp.clone();
        context
    }

    #[test]
    fn drain_work_preserves_effect_order_and_budget() {
        let mut api = ConsensusNetworkApi::new();
        let first = NetworkEffect {
            effect_id: 0,
            source_payload_id: 0,
            transport_lane: 6,
            kind: NETWORK_EFFECT_KIND_REQUEST_SYNC,
            peer_id: peer(1),
            packet_kind: 0,
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
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
            transport_lane: 6,
            kind: NETWORK_EFFECT_KIND_DRIVE_CONSENSUS_PROGRESS,
            peer_id: [0; 64],
            packet_kind: 0,
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
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

        let first_batch = api.drain_work(6, 1);
        assert_eq!(first_batch.status, NETWORK_EFFECT_BATCH_STATUS_OK);
        assert_eq!(first_batch.effects.len(), 1);
        assert!(first_batch.more_available);
        assert_eq!(first_batch.effects[0].effect_id, 1);
        assert_eq!(
            first_batch.effects[0].kind,
            NETWORK_EFFECT_KIND_REQUEST_SYNC
        );

        let second_batch = api.drain_work(6, 10);
        assert_eq!(second_batch.effects.len(), 1);
        assert!(!second_batch.more_available);
        assert_eq!(second_batch.effects[0].effect_id, 2);
        assert_eq!(
            second_batch.effects[0].kind,
            NETWORK_EFFECT_KIND_DRIVE_CONSENSUS_PROGRESS
        );
    }

    #[test]
    fn drain_work_applies_fixed_native_maximum() {
        let mut api = ConsensusNetworkApi::new();
        let effect = NetworkEffect {
            effect_id: 0,
            source_payload_id: 0,
            transport_lane: 6,
            kind: NETWORK_EFFECT_KIND_REQUEST_SYNC,
            peer_id: peer(1),
            packet_kind: 0,
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: 0,
            object_hash: [0; 32],
            sync_kind: NETWORK_SYNC_KIND_PBFT_CHAIN,
            sync_start: 1,
            reason_code: 0,
            dependency_id: 0,
            period: 0,
            round: 0,
        };
        for _ in 0..=MAX_EFFECTS_PER_DRAIN {
            api.enqueue_effect(effect.clone());
        }

        let batch = api.drain_work(6, u32::MAX);
        assert_eq!(batch.effects.len(), MAX_EFFECTS_PER_DRAIN);
        assert!(batch.more_available);
        assert_eq!(api.drain_work(6, u32::MAX).effects.len(), 1);
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
                admission_accepted: false,
                admission_already_present: false,
                admission_mark_vote_known: false,
                admission_gossip_vote: false,
                admission_report_slashing: false,
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
                admission_accepted: false,
                admission_already_present: false,
                admission_mark_vote_known: false,
                admission_gossip_vote: false,
                admission_report_slashing: false,
            },
        ]);

        assert_eq!(ack.status, NETWORK_EFFECT_ACK_STATUS_UNKNOWN_EFFECT_ID);
        assert_eq!(ack.accepted_results, 0);
        assert_eq!(ack.failed_results, 1);
        assert_eq!(ack.error_code, ERROR_UNKNOWN_EFFECT_ID);
    }

    #[test]
    fn report_effect_results_accepts_known_drained_effects() {
        let mut api = ConsensusNetworkApi::new();
        api.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: 1,
            transport_lane: 6,
            kind: NETWORK_EFFECT_KIND_SEND_PACKET,
            peer_id: peer(1),
            packet_kind: 1,
            payload_bytes: vec![1],
            related_payload_bytes: Vec::new(),
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
        let batch = api.drain_work(6, 1);
        assert_eq!(batch.effects[0].effect_id, 1);

        let ack = api.report_effect_results(vec![effect_result(
            &batch.effects[0],
            NETWORK_EFFECT_RESULT_STATUS_OK,
        )]);

        assert_eq!(ack.status, NETWORK_EFFECT_ACK_STATUS_ACCEPTED);
        assert_eq!(ack.accepted_results, 1);
        assert_eq!(ack.failed_results, 0);
        assert_eq!(ack.error_code, "");
        assert!(api.outstanding_effects.is_empty());
    }

    #[test]
    fn report_effect_results_rejects_mismatched_effect_identity() {
        let mut api = ConsensusNetworkApi::new();
        api.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: 1,
            transport_lane: 6,
            kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
            peer_id: peer(1),
            packet_kind: 0,
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
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
        let batch = api.drain_work(6, 1);
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
        assert_eq!(api.outstanding_effects.len(), 1);
    }

    #[test]
    fn plan_status_sync_requests_pbft_sync_when_peer_is_far_ahead() {
        let api = ConsensusNetworkApi::new();
        let mut facts = status_sync_facts();
        facts.peer_pbft_chain_size = 13;

        let plan = api.plan_status_sync(facts);

        assert!(plan.request_pbft_sync);
        assert!(!plan.request_pending_dag_blocks);
        assert!(!plan.request_next_votes);
    }

    #[test]
    fn plan_status_sync_debounces_one_block_pbft_sync() {
        let api = ConsensusNetworkApi::new();
        let mut facts = status_sync_facts();
        facts.peer_pbft_chain_size = 11;
        facts.peer_last_status_pbft_chain_size = 10;

        assert!(!api.plan_status_sync(facts.clone()).request_pbft_sync);

        facts.peer_last_status_pbft_chain_size = 11;
        assert!(api.plan_status_sync(facts).request_pbft_sync);
    }

    #[test]
    fn plan_status_sync_requests_pending_dag_blocks_when_periods_match() {
        let api = ConsensusNetworkApi::new();
        let mut facts = status_sync_facts();
        facts.peer_dag_synced = false;

        let plan = api.plan_status_sync(facts);

        assert!(!plan.request_pbft_sync);
        assert!(plan.request_pending_dag_blocks);
        assert!(!plan.request_next_votes);
    }

    #[test]
    fn plan_status_sync_requests_next_votes_when_peer_round_is_ahead() {
        let api = ConsensusNetworkApi::new();
        let mut facts = status_sync_facts();
        facts.peer_pbft_round = 4;

        let plan = api.plan_status_sync(facts);

        assert!(!plan.request_pbft_sync);
        assert!(!plan.request_pending_dag_blocks);
        assert!(plan.request_next_votes);
        assert_eq!(plan.next_votes_period, 11);
        assert_eq!(plan.next_votes_round, 2);
    }

    #[test]
    fn plan_status_sync_returns_no_actions_while_local_pbft_syncing() {
        let api = ConsensusNetworkApi::new();
        let mut facts = status_sync_facts();
        facts.local_pbft_syncing = true;
        facts.peer_pbft_chain_size = 13;
        facts.peer_pbft_round = 4;
        facts.peer_dag_synced = false;

        let plan = api.plan_status_sync(facts);

        assert!(!plan.request_pbft_sync);
        assert!(!plan.request_pending_dag_blocks);
        assert!(!plan.request_next_votes);
    }

    #[test]
    fn plan_status_egress_includes_initial_metadata() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_status_egress(status_egress_facts(true));

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_OK);
        assert_eq!(plan.peer_pbft_chain_size, 10);
        assert_eq!(plan.peer_pbft_round, 5);
        assert_eq!(plan.peer_dag_level, 44);
        assert!(plan.peer_syncing);
        assert!(plan.include_initial_data);
        assert_eq!(plan.chain_id, 7);
        assert_eq!(plan.genesis_hash, hash(1));
        assert_eq!(plan.node_major_version, 2);
        assert_eq!(plan.node_minor_version, 3);
        assert_eq!(plan.node_patch_version, 4);
        assert!(plan.is_light_node);
        assert_eq!(plan.light_node_history, 8);
    }

    #[test]
    fn plan_status_egress_uses_deep_sync_flag_for_standard_status() {
        let api = ConsensusNetworkApi::new();
        let mut facts = status_egress_facts(false);
        facts.pbft_syncing = true;
        facts.deep_pbft_syncing = false;

        let plan = api.plan_status_egress(facts);

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_OK);
        assert!(!plan.peer_syncing);
        assert!(!plan.include_initial_data);
        assert_eq!(plan.chain_id, 0);
        assert_eq!(plan.genesis_hash, [0; 32]);
        assert_eq!(plan.light_node_history, 0);
    }

    #[test]
    fn plan_initial_status_accepts_matching_status() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_initial_status(initial_status_facts());

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_OK);
        assert!(plan.accept_peer);
        assert!(!plan.disconnect_peer);
    }

    #[test]
    fn plan_initial_status_rejects_chain_id_mismatch() {
        let api = ConsensusNetworkApi::new();
        let mut facts = initial_status_facts();
        facts.peer_chain_id = 8;

        let plan = api.plan_initial_status(facts);

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_CHAIN_ID_MISMATCH);
        assert!(!plan.accept_peer);
        assert!(plan.disconnect_peer);
    }

    #[test]
    fn plan_initial_status_rejects_genesis_mismatch() {
        let api = ConsensusNetworkApi::new();
        let mut facts = initial_status_facts();
        facts.peer_genesis_hash = hash(2);

        let plan = api.plan_initial_status(facts);

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_GENESIS_MISMATCH);
        assert!(!plan.accept_peer);
        assert!(plan.disconnect_peer);
    }

    #[test]
    fn plan_initial_status_rejects_light_node_without_history() {
        let api = ConsensusNetworkApi::new();
        let mut facts = initial_status_facts();
        facts.peer_is_light_node = true;
        facts.peer_light_node_history = 1;
        facts.peer_pbft_chain_size = 20;

        let plan = api.plan_initial_status(facts);

        assert_eq!(
            plan.status,
            NETWORK_STATUS_PLAN_STATUS_LIGHT_NODE_HISTORY_UNAVAILABLE
        );
        assert!(!plan.accept_peer);
        assert!(plan.disconnect_peer);
    }

    #[test]
    fn plan_pbft_sync_start_selects_max_chain_peer() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_pbft_sync_start(NetworkPbftSyncStartFacts {
            local_pbft_syncing: false,
            local_pbft_synced_period: 10,
            local_pbft_chain_size: 10,
            candidates: vec![sync_candidate(1, 12, 20), sync_candidate(2, 13, 1)],
        });

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_OK);
        assert!(plan.start_sync);
        assert_eq!(plan.peer_id, peer(2));
        assert_eq!(plan.peer_pbft_chain_size, 13);
        assert_eq!(plan.request_period, 11);
        assert!(!plan.enable_snapshot_creation);
    }

    #[test]
    fn plan_pbft_sync_start_breaks_chain_size_ties_by_dag_level() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_pbft_sync_start(NetworkPbftSyncStartFacts {
            local_pbft_syncing: false,
            local_pbft_synced_period: 10,
            local_pbft_chain_size: 10,
            candidates: vec![sync_candidate(1, 12, 20), sync_candidate(2, 12, 21)],
        });

        assert!(plan.start_sync);
        assert_eq!(plan.peer_id, peer(2));
    }

    #[test]
    fn plan_pbft_sync_start_skips_light_peer_that_cannot_serve_history() {
        let api = ConsensusNetworkApi::new();
        let mut light = sync_candidate(1, 20, 50);
        light.is_light_node = true;
        light.light_node_history = 4;

        let plan = api.plan_pbft_sync_start(NetworkPbftSyncStartFacts {
            local_pbft_syncing: false,
            local_pbft_synced_period: 10,
            local_pbft_chain_size: 10,
            candidates: vec![light, sync_candidate(2, 13, 1)],
        });

        assert!(plan.start_sync);
        assert_eq!(plan.peer_id, peer(2));
    }

    #[test]
    fn plan_pbft_sync_start_enables_snapshots_when_sync_not_needed() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_pbft_sync_start(NetworkPbftSyncStartFacts {
            local_pbft_syncing: false,
            local_pbft_synced_period: 13,
            local_pbft_chain_size: 13,
            candidates: vec![sync_candidate(1, 12, 20)],
        });

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_SYNC_NOT_NEEDED);
        assert!(!plan.start_sync);
        assert!(plan.has_peer);
        assert!(plan.enable_snapshot_creation);
    }

    #[test]
    fn plan_pbft_sync_start_returns_no_action_while_already_syncing() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_pbft_sync_start(NetworkPbftSyncStartFacts {
            local_pbft_syncing: true,
            local_pbft_synced_period: 10,
            local_pbft_chain_size: 10,
            candidates: vec![sync_candidate(1, 12, 20)],
        });

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_ALREADY_SYNCING);
        assert!(!plan.start_sync);
        assert!(!plan.has_peer);
    }

    #[test]
    fn plan_max_chain_peer_selection_selects_highest_chain_peer() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_max_chain_peer_selection(NetworkPeerSelectionFacts {
            local_pbft_syncing_period: 10,
            candidates: vec![sync_candidate(1, 12, 20), sync_candidate(2, 13, 1)],
        });

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_OK);
        assert!(plan.has_peer);
        assert_eq!(plan.peer_id, peer(2));
        assert_eq!(plan.peer_pbft_chain_size, 13);
    }

    #[test]
    fn plan_max_chain_peer_selection_breaks_ties_by_dag_level() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_max_chain_peer_selection(NetworkPeerSelectionFacts {
            local_pbft_syncing_period: 10,
            candidates: vec![sync_candidate(1, 12, 20), sync_candidate(2, 12, 21)],
        });

        assert!(plan.has_peer);
        assert_eq!(plan.peer_id, peer(2));
    }

    #[test]
    fn plan_max_chain_peer_selection_skips_light_peer_that_cannot_serve_history() {
        let api = ConsensusNetworkApi::new();
        let mut light = sync_candidate(1, 20, 50);
        light.is_light_node = true;
        light.light_node_history = 4;

        let plan = api.plan_max_chain_peer_selection(NetworkPeerSelectionFacts {
            local_pbft_syncing_period: 10,
            candidates: vec![light, sync_candidate(2, 13, 1)],
        });

        assert!(plan.has_peer);
        assert_eq!(plan.peer_id, peer(2));
    }

    #[test]
    fn plan_max_chain_peer_selection_returns_no_peer_without_candidates() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_max_chain_peer_selection(NetworkPeerSelectionFacts {
            local_pbft_syncing_period: 10,
            candidates: vec![],
        });

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_NO_ELIGIBLE_PEER);
        assert!(!plan.has_peer);
    }

    #[test]
    fn plan_pending_dag_blocks_request_accepts_explicit_peer_on_matching_period() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_pending_dag_blocks_request(NetworkPendingDagBlocksRequestFacts {
            local_pbft_syncing_period: 12,
            has_explicit_peer: true,
            explicit_peer: sync_candidate(1, 12, 20),
            candidates: vec![],
        });

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_OK);
        assert!(plan.request_pending_dag_blocks);
        assert!(plan.has_peer);
        assert_eq!(plan.peer_id, peer(1));
        assert_eq!(plan.request_period, 12);
    }

    #[test]
    fn plan_pending_dag_blocks_request_rejects_explicit_peer_already_synced() {
        let api = ConsensusNetworkApi::new();
        let mut candidate = sync_candidate(1, 12, 20);
        candidate.peer_dag_synced = true;

        let plan = api.plan_pending_dag_blocks_request(NetworkPendingDagBlocksRequestFacts {
            local_pbft_syncing_period: 12,
            has_explicit_peer: true,
            explicit_peer: candidate,
            candidates: vec![],
        });

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_DAG_ALREADY_SYNCED);
        assert!(!plan.request_pending_dag_blocks);
        assert!(plan.has_peer);
        assert_eq!(plan.peer_id, peer(1));
    }

    #[test]
    fn plan_pending_dag_blocks_request_selects_max_eligible_peer() {
        let api = ConsensusNetworkApi::new();
        let mut already_synced = sync_candidate(1, 14, 50);
        already_synced.peer_dag_synced = true;
        let mut disallowed = sync_candidate(2, 13, 40);
        disallowed.dag_sync_allowed = false;

        let plan = api.plan_pending_dag_blocks_request(NetworkPendingDagBlocksRequestFacts {
            local_pbft_syncing_period: 12,
            has_explicit_peer: false,
            explicit_peer: sync_candidate(0, 0, 0),
            candidates: vec![
                already_synced,
                disallowed,
                sync_candidate(3, 11, 100),
                sync_candidate(4, 12, 10),
            ],
        });

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_OK);
        assert!(plan.request_pending_dag_blocks);
        assert_eq!(plan.peer_id, peer(4));
        assert_eq!(plan.request_period, 12);
    }

    #[test]
    fn plan_pending_dag_blocks_request_breaks_chain_size_ties_by_dag_level() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_pending_dag_blocks_request(NetworkPendingDagBlocksRequestFacts {
            local_pbft_syncing_period: 12,
            has_explicit_peer: false,
            explicit_peer: sync_candidate(0, 0, 0),
            candidates: vec![sync_candidate(1, 12, 10), sync_candidate(2, 12, 11)],
        });

        assert!(plan.request_pending_dag_blocks);
        assert_eq!(plan.peer_id, peer(2));
    }

    #[test]
    fn plan_pending_dag_blocks_request_rejects_period_mismatch() {
        let api = ConsensusNetworkApi::new();

        let plan = api.plan_pending_dag_blocks_request(NetworkPendingDagBlocksRequestFacts {
            local_pbft_syncing_period: 11,
            has_explicit_peer: true,
            explicit_peer: sync_candidate(1, 12, 20),
            candidates: vec![],
        });

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_DAG_PERIOD_MISMATCH);
        assert!(!plan.request_pending_dag_blocks);
        assert!(plan.has_peer);
        assert_eq!(plan.peer_id, peer(1));
    }

    #[test]
    fn plan_pending_dag_blocks_request_skips_light_peer_that_cannot_serve_history() {
        let api = ConsensusNetworkApi::new();
        let mut light = sync_candidate(1, 20, 50);
        light.is_light_node = true;
        light.light_node_history = 4;

        let plan = api.plan_pending_dag_blocks_request(NetworkPendingDagBlocksRequestFacts {
            local_pbft_syncing_period: 12,
            has_explicit_peer: false,
            explicit_peer: sync_candidate(0, 0, 0),
            candidates: vec![light, sync_candidate(2, 12, 1)],
        });

        assert!(plan.request_pending_dag_blocks);
        assert_eq!(plan.peer_id, peer(2));
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

        let batch = api.drain_work(6, 10);
        assert_eq!(batch.effects.len(), 1);
        let effect = &batch.effects[0];
        assert_eq!(effect.kind, NETWORK_EFFECT_KIND_REQUEST_SYNC);
        assert_eq!(effect.peer_id, peer(7));
        assert_eq!(effect.sync_kind, NETWORK_SYNC_KIND_PBFT_CHAIN);
        assert_eq!(effect.sync_start, 13);
        assert_eq!(effect.source_payload_id, 99);
    }

    #[test]
    fn ingest_pbft_vote_bundle_queues_report_and_disconnect_for_propose_votes() {
        let mut api = ConsensusNetworkApi::new();

        let decisions = api.ingest_pbft_vote_bundle(
            vote_fact(10, 3, 2, PbftVoteType::Propose),
            vec![vote_fact(10, 3, 2, PbftVoteType::Propose)],
            vec![vote_context()],
        );
        let decision = &decisions[0];

        assert_eq!(
            decision.status,
            PbftVoteIngressStatus::UnsupportedBundleProposeVote.as_u8()
        );
        assert_eq!(decision.queued_effect_count, 2);

        let batch = api.drain_work(6, 10);
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
    fn bundle_admission_aggregates_only_accepted_members_before_gossip() {
        let mut api = ConsensusNetworkApi::new();
        let reference = vote_fact(10, 3, 2, PbftVoteType::Soft);
        let votes = [
            canonical_bundle_vote(0x42),
            canonical_bundle_vote(0x43),
            canonical_bundle_vote(0x44),
        ];
        let decisions = api.ingest_pbft_vote_bundle(
            reference,
            vec![reference; votes.len()],
            votes.iter().map(bundle_context).collect(),
        );

        for (index, decision) in decisions.iter().enumerate() {
            assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
            let admission = api.drain_work(6, 1);
            let effect = admission
                .effects
                .iter()
                .find(|effect| effect.effect_id == decision.application_effect_id)
                .unwrap();
            let mut result = effect_result(effect, NETWORK_EFFECT_RESULT_STATUS_OK);
            result.admission_accepted = index != 1;
            api.report_effect_results(vec![result]);
        }

        let gossip = api.drain_work(6, 10);
        assert_eq!(gossip.effects.len(), 1);
        let effect = &gossip.effects[0];
        assert_eq!(effect.kind, NETWORK_EFFECT_KIND_GOSSIP_PACKET);
        assert_eq!(effect.packet_kind, NETWORK_PACKET_KIND_PBFT_VOTES_BUNDLE);
        assert_eq!(effect.exclude_peers, vec![peer(9)]);
        assert_eq!(effect.period, 10);
        assert_eq!(effect.round, 3);
        let payload = Rlp::new(&effect.payload_bytes);
        assert_eq!(payload.item_count().unwrap(), 5);
        assert_eq!(payload.at(0).unwrap().data().unwrap(), hash(0xA4));
        assert_eq!(payload.val_at::<u64>(1).unwrap(), 10);
        assert_eq!(payload.val_at::<u64>(2).unwrap(), 3);
        assert_eq!(payload.val_at::<u64>(3).unwrap(), 2);
        assert_eq!(payload.at(4).unwrap().item_count().unwrap(), 2);
        let expected = build_optimized_pbft_vote_bundle(
            &[votes[0].clone(), votes[2].clone()],
            H256::from(hash(0xA4)),
            10,
            3,
            2,
        )
        .unwrap();
        assert_eq!(effect.payload_bytes, expected.bundle_rlp);
    }

    #[test]
    fn bundle_admission_does_not_gossip_when_every_member_is_rejected() {
        let mut api = ConsensusNetworkApi::new();
        let reference = vote_fact(10, 3, 2, PbftVoteType::Soft);
        let vote = canonical_bundle_vote(0x42);

        api.ingest_pbft_vote_bundle(reference, vec![reference], vec![bundle_context(&vote)]);
        let admission = api.drain_work(6, 10);
        api.report_effect_results(vec![effect_result(
            &admission.effects[0],
            NETWORK_EFFECT_RESULT_STATUS_OK,
        )]);

        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn bundle_preflight_rejects_late_mismatch_before_any_admission() {
        let mut api = ConsensusNetworkApi::new();
        let reference = vote_fact(10, 3, 2, PbftVoteType::Soft);
        let votes = [canonical_bundle_vote(0x42), canonical_bundle_vote(0x43)];
        let decisions = api.ingest_pbft_vote_bundle(
            reference,
            vec![reference, vote_fact(10, 4, 2, PbftVoteType::Soft)],
            votes.iter().map(bundle_context).collect(),
        );

        assert_eq!(decisions.len(), 1);
        assert_eq!(
            decisions[0].status,
            PbftVoteIngressStatus::BundleVoteMismatch.as_u8()
        );
        let effects = api.drain_work(6, 10).effects;
        assert!(effects.iter().all(|effect| {
            effect.kind != NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
                || effect.object_kind != NETWORK_OBJECT_KIND_PBFT_VOTE
        }));
    }

    #[test]
    fn bundle_slashing_result_cancels_remaining_admissions_and_gossip() {
        let mut api = ConsensusNetworkApi::new();
        let reference = vote_fact(10, 3, 2, PbftVoteType::Soft);
        let votes = [
            canonical_bundle_vote(0x42),
            canonical_bundle_vote(0x43),
            canonical_bundle_vote(0x44),
        ];
        let decisions = api.ingest_pbft_vote_bundle(
            reference,
            vec![reference; votes.len()],
            votes.iter().map(bundle_context).collect(),
        );

        let first = api.drain_work(6, 1);
        let mut first_result = effect_result(&first.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        first_result.admission_accepted = true;
        api.report_effect_results(vec![first_result]);

        let second = api.drain_work(6, 1);
        assert_eq!(
            second.effects[0].effect_id,
            decisions[1].application_effect_id
        );
        let mut slashing = effect_result(&second.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        slashing.admission_report_slashing = true;
        api.report_effect_results(vec![slashing]);

        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn final_bundle_slashing_result_never_releases_accepted_prefix() {
        let mut api = ConsensusNetworkApi::new();
        let reference = vote_fact(10, 3, 2, PbftVoteType::Soft);
        let votes = [canonical_bundle_vote(0x42), canonical_bundle_vote(0x43)];
        api.ingest_pbft_vote_bundle(
            reference,
            vec![reference; votes.len()],
            votes.iter().map(bundle_context).collect(),
        );

        let first = api.drain_work(6, 1);
        let mut first_result = effect_result(&first.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        first_result.admission_accepted = true;
        api.report_effect_results(vec![first_result]);

        let second = api.drain_work(6, 1);
        let mut slashing = effect_result(&second.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        slashing.admission_accepted = true;
        slashing.admission_report_slashing = true;
        api.report_effect_results(vec![slashing]);

        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn bundle_executor_failure_cleans_session_and_allows_next_bundle() {
        let mut api = ConsensusNetworkApi::new();
        let reference = vote_fact(10, 3, 2, PbftVoteType::Soft);
        let first_vote = canonical_bundle_vote(0x42);
        api.ingest_pbft_vote_bundle(
            reference,
            vec![reference],
            vec![bundle_context(&first_vote)],
        );
        let failed = api.drain_work(6, 1);
        api.report_effect_results(vec![effect_result(
            &failed.effects[0],
            NETWORK_EFFECT_RESULT_STATUS_FAILED,
        )]);
        assert!(api.drain_work(6, 10).effects.is_empty());

        let next_vote = canonical_bundle_vote(0x43);
        api.ingest_pbft_vote_bundle(reference, vec![reference], vec![bundle_context(&next_vote)]);
        let next = api.drain_work(6, 1);
        let mut accepted = effect_result(&next.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        accepted.admission_accepted = true;
        api.report_effect_results(vec![accepted]);
        let gossip = api.drain_work(6, 10);
        assert_eq!(gossip.effects.len(), 1);
        assert_eq!(
            gossip.effects[0].packet_kind,
            NETWORK_PACKET_KIND_PBFT_VOTES_BUNDLE
        );
    }

    #[test]
    fn post_admission_route_orders_block_publication_before_gossip() {
        let mut api = ConsensusNetworkApi::new();
        let mut context = vote_context();
        context.transport_lane = 6;
        context.peer_id = peer(10);
        context.source_payload_id = 79;
        context.allow_gossip = true;
        context.enqueue_admission = true;
        context.vote_hash = hash(0xEF);
        context.vote_rlp = vec![0xc3, 1, 2, 3];
        context.pbft_block_rlp = vec![0xc2, 4, 5];
        context.pbft_block_hash = hash(0xAB);
        context.pbft_block_period = 42;

        let decision = api.ingest_pbft_vote(vote_fact(10, 3, 2, PbftVoteType::Soft), context);

        assert!(decision.routed);
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);

        let publication = api.drain_work(6, 10);
        assert_eq!(publication.effects.len(), 1);
        assert_eq!(
            publication.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        assert_eq!(
            publication.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PBFT_VOTE
        );
        assert_eq!(publication.effects[0].object_hash, hash(0xEF));
        assert_eq!(publication.effects[0].payload_bytes, vec![0xc3, 1, 2, 3]);
        assert_eq!(
            decision.application_effect_id,
            publication.effects[0].effect_id
        );
        let mut result = effect_result(&publication.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        result.admission_accepted = true;
        result.admission_gossip_vote = true;
        api.report_effect_results(vec![result]);

        let block_publication = api.drain_work(6, 10);
        assert_eq!(block_publication.effects.len(), 1);
        assert_eq!(
            block_publication.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        assert_eq!(
            block_publication.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PBFT_BLOCK
        );
        assert_eq!(block_publication.effects[0].dependency_id, 0);
        api.report_effect_results(vec![effect_result(
            &block_publication.effects[0],
            NETWORK_EFFECT_RESULT_STATUS_OK,
        )]);

        let dependents = api.drain_work(6, 10);
        assert_eq!(dependents.effects.len(), 2);
        assert_eq!(
            dependents.effects[0].kind,
            NETWORK_EFFECT_KIND_MARK_PEER_KNOWN
        );
        assert_eq!(
            dependents.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PBFT_BLOCK
        );
        assert_eq!(
            dependents.effects[0].dependency_id,
            block_publication.effects[0].effect_id
        );
        assert_eq!(
            dependents.effects[1].kind,
            NETWORK_EFFECT_KIND_GOSSIP_PACKET
        );
        assert_eq!(dependents.effects[1].peer_id, peer(10));
        assert_eq!(
            dependents.effects[1].packet_kind,
            NETWORK_PACKET_KIND_PBFT_VOTE
        );
        assert_eq!(dependents.effects[1].exclude_peers, vec![peer(10)]);
        assert_eq!(
            dependents.effects[1].object_kind,
            NETWORK_OBJECT_KIND_PBFT_VOTE
        );
        assert_eq!(dependents.effects[1].object_hash, hash(0xEF));
        assert_eq!(dependents.effects[1].payload_bytes, vec![0xc3, 1, 2, 3]);
        assert_eq!(
            dependents.effects[1].related_payload_bytes,
            vec![0xc2, 4, 5]
        );
        assert_eq!(dependents.effects[1].source_payload_id, 79);
        assert_eq!(
            dependents.effects[1].dependency_id,
            block_publication.effects[0].effect_id
        );
    }

    #[test]
    fn exact_duplicate_still_routes_attached_proposed_block_without_gossip() {
        let mut api = ConsensusNetworkApi::new();
        let mut context = vote_context();
        context.transport_lane = 6;
        context.peer_id = peer(9);
        context.vote_hash = hash(0xAA);
        context.vote_rlp = vec![0xc1, 1];
        context.pbft_block_rlp = vec![0xc1, 2];
        context.pbft_block_hash = hash(0xBB);
        context.pbft_block_period = 11;
        context.enqueue_admission = true;
        let decision = api.ingest_pbft_vote(vote_fact(10, 3, 2, PbftVoteType::Soft), context);

        assert_eq!(decision.queued_effect_count, 1);
        let first = api.drain_work(6, 10);
        assert_eq!(
            first.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        let mut result = effect_result(&first.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        result.admission_already_present = true;
        api.report_effect_results(vec![result]);
        let dependent = api.drain_work(6, 10);
        assert_eq!(dependent.effects.len(), 2);
        assert_eq!(
            dependent.effects[0].kind,
            NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
        );
        assert_eq!(
            dependent.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PBFT_BLOCK
        );
        assert!(
            dependent
                .effects
                .iter()
                .all(|effect| effect.kind != NETWORK_EFFECT_KIND_GOSSIP_PACKET)
        );
    }

    #[test]
    fn failed_admission_cancels_dependent_known_and_gossip_effects() {
        let mut api = ConsensusNetworkApi::new();
        let mut context = vote_context();
        context.transport_lane = 6;
        context.peer_id = peer(9);
        context.vote_hash = hash(0xAA);
        context.vote_rlp = vec![0xc1, 1];
        context.pbft_block_rlp = vec![0xc1, 2];
        context.pbft_block_hash = hash(0xBB);
        context.pbft_block_period = 11;
        context.enqueue_admission = true;
        api.ingest_pbft_vote(vote_fact(10, 3, 2, PbftVoteType::Soft), context);

        let publication = api.drain_work(6, 10);
        assert_eq!(publication.effects.len(), 1);
        let result = effect_result(&publication.effects[0], NETWORK_EFFECT_RESULT_STATUS_FAILED);
        let ack = api.report_effect_results(vec![result]);
        assert_eq!(ack.status, NETWORK_EFFECT_ACK_STATUS_ACCEPTED);

        let cancelled = api.drain_work(6, 10);
        assert!(cancelled.effects.is_empty());
        assert!(!cancelled.more_available);
    }

    #[test]
    fn failed_block_publication_cancels_block_known_and_gossip_effects() {
        let mut api = ConsensusNetworkApi::new();
        let mut context = vote_context();
        context.enqueue_admission = true;
        context.allow_gossip = true;
        context.vote_hash = hash(0xA2);
        context.vote_rlp = vec![0xc1, 1];
        context.pbft_block_rlp = vec![0xc1, 2];
        context.pbft_block_hash = hash(0xB2);
        context.pbft_block_period = 11;
        api.ingest_pbft_vote(vote_fact(10, 3, 2, PbftVoteType::Soft), context);

        let admission = api.drain_work(6, 1);
        let mut admission_result =
            effect_result(&admission.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        admission_result.admission_accepted = true;
        admission_result.admission_gossip_vote = true;
        api.report_effect_results(vec![admission_result]);

        let publication = api.drain_work(6, 1);
        assert_eq!(
            publication.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PBFT_BLOCK
        );
        api.report_effect_results(vec![effect_result(
            &publication.effects[0],
            NETWORK_EFFECT_RESULT_STATUS_FAILED,
        )]);

        let cancelled = api.drain_work(6, 10);
        assert!(cancelled.effects.is_empty());
        assert!(!cancelled.more_available);
    }

    #[test]
    fn admission_result_validation_rejects_impossible_or_unscoped_outcomes() {
        let mut api = ConsensusNetworkApi::new();
        let mut context = vote_context();
        context.enqueue_admission = true;
        context.vote_hash = hash(0xA1);
        context.vote_rlp = vec![0xc1, 1];
        let decision = api.ingest_pbft_vote(vote_fact(10, 3, 2, PbftVoteType::Soft), context);
        let batch = api.drain_work(6, 1);
        assert_eq!(decision.application_effect_id, batch.effects[0].effect_id);

        let mut impossible = effect_result(&batch.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        impossible.admission_accepted = true;
        impossible.admission_already_present = true;
        let ack = api.report_effect_results(vec![impossible]);
        assert_eq!(
            ack.status,
            NETWORK_EFFECT_ACK_STATUS_MISMATCHED_EFFECT_RESULT
        );

        let mut retry = effect_result(&batch.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        retry.admission_accepted = true;
        retry.admission_mark_vote_known = true;
        assert_eq!(
            api.report_effect_results(vec![retry]).status,
            NETWORK_EFFECT_ACK_STATUS_ACCEPTED
        );

        let follow_up = api.drain_work(6, 1);
        assert_eq!(
            follow_up.effects[0].kind,
            NETWORK_EFFECT_KIND_MARK_PEER_KNOWN
        );
        let mut unscoped = effect_result(&follow_up.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        unscoped.admission_accepted = true;
        assert_eq!(
            api.report_effect_results(vec![unscoped]).status,
            NETWORK_EFFECT_ACK_STATUS_MISMATCHED_EFFECT_RESULT
        );
    }

    #[test]
    fn gossip_payloads_survive_producer_scope_and_drain_fifo() {
        let mut api = ConsensusNetworkApi::new();
        for byte in [1, 2] {
            let mut context = vote_context();
            context.transport_lane = 6;
            context.enqueue_admission = true;
            context.allow_gossip = true;
            context.peer_id = peer(byte);
            context.vote_hash = hash(byte);
            context.vote_rlp = vec![0xc1, byte];
            context.pbft_block_rlp = Vec::new();
            context.pbft_block_hash = [0; 32];
            context.pbft_block_period = 0;
            context.source_payload_id = u64::from(byte);
            api.ingest_pbft_vote(vote_fact(10, 3, 2, PbftVoteType::Soft), context);
        }

        let batch = api.drain_work(6, 2);
        assert_eq!(batch.effects.len(), 2);
        assert_eq!(batch.effects[0].payload_bytes, vec![0xc1, 1]);
        assert_eq!(batch.effects[1].payload_bytes, vec![0xc1, 2]);
        let mut result_one = effect_result(&batch.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        result_one.admission_accepted = true;
        let mut result_two = effect_result(&batch.effects[1], NETWORK_EFFECT_RESULT_STATUS_OK);
        result_two.admission_accepted = true;
        api.report_effect_results(vec![result_one, result_two]);
        assert!(
            batch
                .effects
                .iter()
                .all(|effect| effect.related_payload_bytes.is_empty())
        );
    }

    #[test]
    fn drain_work_isolates_interleaved_transport_lanes() {
        let mut api = ConsensusNetworkApi::new();
        for (transport_lane, byte) in [(6, 1), (5, 2), (6, 3), (5, 4)] {
            let mut context = vote_context();
            context.transport_lane = transport_lane;
            context.enqueue_admission = true;
            context.peer_id = peer(byte);
            context.vote_hash = hash(byte);
            context.vote_rlp = vec![0xc1, byte];
            context.pbft_block_rlp = Vec::new();
            context.pbft_block_hash = [0; 32];
            context.pbft_block_period = 0;
            context.source_payload_id = u64::from(byte);
            api.ingest_pbft_vote(vote_fact(10, 3, 2, PbftVoteType::Soft), context);
        }

        let first_v5 = api.drain_work(5, 1);
        assert_eq!(first_v5.effects.len(), 1);
        assert_eq!(first_v5.effects[0].effect_id, 2);
        assert_eq!(first_v5.effects[0].payload_bytes, vec![0xc1, 2]);
        assert!(first_v5.more_available);

        let latest = api.drain_work(6, 10);
        assert_eq!(latest.effects.len(), 2);
        assert_eq!(latest.effects[0].effect_id, 1);
        assert_eq!(latest.effects[0].payload_bytes, vec![0xc1, 1]);
        assert_eq!(latest.effects[1].effect_id, 3);
        assert_eq!(latest.effects[1].payload_bytes, vec![0xc1, 3]);
        assert!(!latest.more_available);

        let second_v5 = api.drain_work(5, 10);
        assert_eq!(second_v5.effects.len(), 1);
        assert_eq!(second_v5.effects[0].effect_id, 4);
        assert_eq!(second_v5.effects[0].payload_bytes, vec![0xc1, 4]);
        assert!(!second_v5.more_available);
    }

    #[test]
    fn get_pbft_sync_request_requires_exact_canonical_one_field_rlp() {
        let mut canonical = RlpStream::new_list(1);
        canonical.append(&12u64);
        assert_eq!(decode_get_pbft_sync_request(&canonical.out()), Some(12));

        assert_eq!(decode_get_pbft_sync_request(&[0xc0]), None);
        assert_eq!(decode_get_pbft_sync_request(&[0xc2, 0x81, 0x01]), None);
        let mut extra = RlpStream::new_list(2);
        extra.append(&12u64);
        extra.append(&13u64);
        assert_eq!(decode_get_pbft_sync_request(&extra.out()), None);
    }

    #[test]
    fn pbft_sync_and_proposal_packet_encoders_match_full_wire_shapes() {
        let period_data = [0xc2, 0x01, 0x02];
        let packet = encode_pbft_sync_packet(true, &period_data, None);
        let decoded = Rlp::new(&packet);
        assert_eq!(decoded.item_count().unwrap(), 3);
        assert!(decoded.val_at::<bool>(0).unwrap());
        assert_eq!(decoded.at(1).unwrap().as_raw(), period_data);
        assert_eq!(decoded.val_at::<u8>(2).unwrap(), 0);

        let proposals = (0..21)
            .map(|period| crate::proposed_blocks::ProposedBlockEntry {
                period,
                block_hash: H256::from_low_u64_be(period),
                pivot_hash: H256::zero(),
                block_rlp: vec![0xc1, period as u8],
                is_valid: false,
            })
            .collect();
        let bundles = encode_proposed_block_bundles(proposals);
        assert_eq!(bundles.len(), 3);
        assert_eq!(
            Rlp::new(&bundles[0]).at(0).unwrap().item_count().unwrap(),
            10
        );
        assert_eq!(
            Rlp::new(&bundles[1]).at(0).unwrap().item_count().unwrap(),
            10
        );
        assert_eq!(
            Rlp::new(&bundles[2]).at(0).unwrap().item_count().unwrap(),
            1
        );
    }

    #[test]
    fn pbft_sync_egress_queues_sync_clear_then_proposals_independently() {
        let mut api = ConsensusNetworkApi::new();
        let request = NetworkGetPbftSyncRequest {
            tarcap_version: 6,
            peer_id: peer(7),
            request_rlp: vec![0xc1, 0x01],
            source_payload_id: 99,
        };
        let decision = api.enqueue_pbft_sync_egress_effects(
            request,
            1,
            vec![vec![0xc1, 1], vec![0xc1, 2]],
            vec![vec![0xc1, 3]],
            true,
            NETWORK_INGRESS_STATUS_ACCEPTED,
            ERROR_NONE,
        );
        assert_eq!(decision.queued_effect_count, 4);
        let effects = api.drain_work(6, 10).effects;
        assert_eq!(effects.len(), 4);
        assert_eq!(effects[0].packet_kind, NETWORK_PACKET_KIND_PBFT_SYNC);
        assert_eq!(effects[1].packet_kind, NETWORK_PACKET_KIND_PBFT_SYNC);
        assert_eq!(effects[2].kind, NETWORK_EFFECT_KIND_CLEAR_PEER_SYNCING);
        assert_eq!(effects[2].dependency_id, 0);
        assert_eq!(
            effects[3].packet_kind,
            NETWORK_PACKET_KIND_PBFT_BLOCKS_BUNDLE
        );
        assert_eq!(effects[3].dependency_id, 0);

        let mut missing_api = ConsensusNetworkApi::new();
        let missing_request = NetworkGetPbftSyncRequest {
            tarcap_version: 6,
            peer_id: peer(9),
            request_rlp: vec![0xc1, 0x01],
            source_payload_id: 100,
        };
        let missing = missing_api.enqueue_pbft_sync_egress_effects(
            missing_request,
            1,
            vec![vec![0xc1, 1]],
            vec![vec![0xc1, 3]],
            false,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_PERIOD_DATA_MISSING,
            ERROR_PBFT_SYNC_PERIOD_DATA_MISSING,
        );
        assert_eq!(
            missing.status,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_PERIOD_DATA_MISSING
        );
        assert_eq!(missing.queued_effect_count, 2);
        let missing_effects = missing_api.drain_work(6, 10).effects;
        assert_eq!(missing_effects.len(), 2);
        assert_eq!(
            missing_effects[0].packet_kind,
            NETWORK_PACKET_KIND_PBFT_SYNC
        );
        assert_eq!(
            missing_effects[1].packet_kind,
            NETWORK_PACKET_KIND_PBFT_BLOCKS_BUNDLE
        );
    }

    #[test]
    fn malformed_pbft_sync_request_reports_then_dependently_disconnects() {
        let mut api = ConsensusNetworkApi::new();
        let request = NetworkGetPbftSyncRequest {
            tarcap_version: 6,
            peer_id: peer(8),
            request_rlp: vec![0xc0],
            source_payload_id: 101,
        };
        let decision = api.reject_invalid_pbft_sync_request(
            &request,
            0,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_MALFORMED_REQUEST,
            ERROR_PBFT_SYNC_MALFORMED_REQUEST,
        );
        assert_eq!(decision.queued_effect_count, 2);
        let report = api.drain_work(6, 10).effects;
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, NETWORK_EFFECT_KIND_REPORT_PEER);
        assert_eq!(
            report[0].reason_code,
            NETWORK_REASON_INVALID_PBFT_SYNC_REQUEST
        );
        let result = effect_result(&report[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        assert_eq!(api.report_effect_results(vec![result]).status, 0);
        let disconnect = api.drain_work(6, 10).effects;
        assert_eq!(disconnect.len(), 1);
        assert_eq!(disconnect[0].kind, NETWORK_EFFECT_KIND_DISCONNECT_PEER);
        assert_eq!(disconnect[0].dependency_id, report[0].effect_id);
    }
}
