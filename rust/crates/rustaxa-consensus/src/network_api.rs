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

use ethereum_types::H256;
use rlp::{Rlp, RlpStream};

use crate::pbft_vote_payload::build_optimized_pbft_vote_bundle;
use crate::{
    PbftVoteIngressContext, PbftVoteIngressFact, PbftVoteIngressPlan, PbftVoteIngressStatus,
    PbftVotePayloadRecord, PillarVoteRelevanceFact, PillarVoteRelevancePlan,
    inspect_canonical_pbft_vote, plan_pbft_vote_bundle_ingress, plan_pbft_vote_ingress,
    plan_pillar_vote_relevance,
};

/// Network/tarcap packet facts were accepted for operation-specific routing.
pub const NETWORK_INGRESS_STATUS_ACCEPTED: u8 = 0;
/// A get-next-votes request carries a different PBFT period.
pub const NETWORK_INGRESS_STATUS_NEXT_VOTES_PERIOD_MISMATCH: u8 = 1;
/// Local PBFT round one has no previous-round next-vote family.
pub const NETWORK_INGRESS_STATUS_NEXT_VOTES_NO_PREVIOUS_ROUND: u8 = 2;
/// The requester claims a PBFT round ahead of the local snapshot.
pub const NETWORK_INGRESS_STATUS_NEXT_VOTES_PEER_ROUND_AHEAD: u8 = 3;

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
/// Network object effect identifies a pillar vote validation request.
pub const NETWORK_OBJECT_KIND_PILLAR_VOTE_VALIDATION: u8 = 6;
/// Network object effect identifies a PBFT next-votes bundle egress request.
pub const NETWORK_OBJECT_KIND_PBFT_NEXT_VOTES_BUNDLE_EGRESS_REQUEST: u8 = 7;
/// Network object effect identifies a PBFT sync egress request.
pub const NETWORK_OBJECT_KIND_PBFT_SYNC_EGRESS_REQUEST: u8 = 8;
/// Network object effect identifies a pillar votes bundle egress request.
pub const NETWORK_OBJECT_KIND_PILLAR_VOTES_BUNDLE_EGRESS_REQUEST: u8 = 9;
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
const MAX_VOTES_PER_BUNDLE_PACKET: usize = 1000;

/// Effect-drain limit for the external network/tarcap facade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkApiConfig {
    /// Maximum number of effects returned by one drain call.
    pub max_effects_per_drain: usize,
}

impl Default for NetworkApiConfig {
    fn default() -> Self {
        Self {
            max_effects_per_drain: 1024,
        }
    }
}

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
    /// Primary payload returned by an operation-shaped application effect.
    pub payload_bytes: Vec<u8>,
    /// Secondary payload returned by an operation-shaped application effect.
    pub related_payload_bytes: Vec<u8>,
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
struct PendingNextVotesBundleEgress {
    transport_lane: u32,
    peer_id: [u8; 64],
    source_payload_id: u64,
    period: u64,
    round: u64,
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

/// Rust-owned external network/tarcap API facade.
///
/// The facade owns an ordered network effect queue. It is intentionally small: packet-specific decoding and
/// consensus planning should be added behind this type without exposing
/// consensus managers, C++ sidecars, storage handles, or shim routes to the
/// network module.
#[derive(Debug, Default)]
pub struct ConsensusNetworkApi {
    config: NetworkApiConfig,
    next_effect_id: u64,
    next_vote_bundle_id: u64,
    pending_effects: VecDeque<NetworkEffect>,
    pending_vote_admissions: HashMap<u64, PendingVoteAdmissionContext>,
    pending_vote_bundles: HashMap<u64, PendingVoteBundle>,
    pending_next_votes_bundle_egress: HashMap<u64, PendingNextVotesBundleEgress>,
    outstanding_effects: HashMap<u64, NetworkEffect>,
    completed_dependency_status: HashMap<u64, bool>,
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
            next_effect_id: 1,
            next_vote_bundle_id: 1,
            pending_effects: VecDeque::new(),
            pending_vote_admissions: HashMap::new(),
            pending_vote_bundles: HashMap::new(),
            pending_next_votes_bundle_egress: HashMap::new(),
            outstanding_effects: HashMap::new(),
            completed_dependency_status: HashMap::new(),
            effect_results: Vec::new(),
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
            .min(self.config.max_effects_per_drain);
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
    /// Accepted reports retain only scalar identity and status metadata. Any
    /// response payloads are consumed while applying the result and discarded
    /// before archival so peer-triggered application responses cannot grow the
    /// diagnostic journal by their encoded bundle size.
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
            if effect.kind == NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
                && effect.object_kind == NETWORK_OBJECT_KIND_PBFT_NEXT_VOTES_BUNDLE_EGRESS_REQUEST
                && !next_votes_bundle_result_is_valid(result, effect.period, effect.round)
            {
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
                    && effect.object_kind
                        == NETWORK_OBJECT_KIND_PBFT_NEXT_VOTES_BUNDLE_EGRESS_REQUEST
                    && let Some(context) = self
                        .pending_next_votes_bundle_egress
                        .remove(&result.effect_id)
                    && result.status == NETWORK_EFFECT_RESULT_STATUS_OK
                {
                    self.enqueue_next_votes_bundle_send_effects(context, result);
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
            self.effect_results
                .extend(results.into_iter().map(|mut result| {
                    result.payload_bytes = Vec::new();
                    result.related_payload_bytes = Vec::new();
                    result
                }));
        }

        NetworkEffectAck {
            status,
            accepted_results,
            failed_results,
            error_code: error_code.to_owned(),
        }
    }

    /// Plans deterministic pillar-vote relevance through the Network/Tarcap API.
    ///
    /// Tarcap supplies decoded vote facts plus compact local pillar-chain
    /// context. Rust owns the relevance decision so network handlers do not
    /// call pillar manager relevance helpers directly. Signature,
    /// eligibility, and insertion remain separate executor/validation
    /// boundaries until the pillar vote runtime is fully injected here.
    pub fn plan_pillar_vote_relevance(
        &self,
        fact: PillarVoteRelevanceFact,
    ) -> anyhow::Result<PillarVoteRelevancePlan> {
        plan_pillar_vote_relevance(fact)
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

    /// Routes one get-next-votes request through an application leaf.
    ///
    /// Rust owns request eligibility, the exact previous-round query, and all
    /// follow-up send/chunk ordering. The executor returns packet-ready inner
    /// optimized bundle payloads from the native verified-vote service.
    pub fn ingest_pbft_next_votes_bundle_request(
        &mut self,
        request: NetworkPbftNextVotesBundleRequest,
    ) -> NetworkIngressDecision {
        let rejection = if request.current_period != request.peer_period {
            Some((
                NETWORK_INGRESS_STATUS_NEXT_VOTES_PERIOD_MISMATCH,
                ERROR_NEXT_VOTES_PERIOD_MISMATCH,
            ))
        } else if request.current_round <= 1 {
            Some((
                NETWORK_INGRESS_STATUS_NEXT_VOTES_NO_PREVIOUS_ROUND,
                ERROR_NEXT_VOTES_NO_PREVIOUS_ROUND,
            ))
        } else if request.current_round < request.peer_round {
            Some((
                NETWORK_INGRESS_STATUS_NEXT_VOTES_PEER_ROUND_AHEAD,
                ERROR_NEXT_VOTES_PEER_ROUND_AHEAD,
            ))
        } else {
            None
        };
        if let Some((status, error_code)) = rejection {
            return NetworkIngressDecision {
                payload_id: request.source_payload_id,
                payload_accepted: request.source_payload_id != 0,
                routed: true,
                status,
                error_code: error_code.to_owned(),
                queued_effect_count: 0,
                application_effect_id: 0,
            };
        }

        let period = request.current_period;
        let round = request.current_round - 1;
        let context = PendingNextVotesBundleEgress {
            transport_lane: request.transport_lane,
            peer_id: request.peer_id,
            source_payload_id: request.source_payload_id,
            period,
            round,
        };
        let effect_id = self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: request.source_payload_id,
            transport_lane: request.transport_lane,
            kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
            peer_id: request.peer_id,
            packet_kind: 0,
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
            exclude_peers: Vec::new(),
            object_kind: NETWORK_OBJECT_KIND_PBFT_NEXT_VOTES_BUNDLE_EGRESS_REQUEST,
            object_hash: [0; 32],
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id: 0,
            period,
            round,
        });
        self.pending_next_votes_bundle_egress
            .insert(effect_id, context);
        NetworkIngressDecision {
            payload_id: request.source_payload_id,
            payload_accepted: request.source_payload_id != 0,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: ERROR_NONE.to_owned(),
            queued_effect_count: 1,
            application_effect_id: effect_id,
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
        context: PendingNextVotesBundleEgress,
        result: &NetworkEffectResult,
    ) {
        let next_chunks = validate_and_chunk_next_votes_bundle(
            &result.payload_bytes,
            context.period,
            context.round,
            false,
        )
        .expect("next-votes result was validated before acknowledgement");
        let next_null_chunks = validate_and_chunk_next_votes_bundle(
            &result.related_payload_bytes,
            context.period,
            context.round,
            true,
        )
        .expect("next-null result was validated before acknowledgement");
        for payload_bytes in next_chunks.into_iter().chain(next_null_chunks) {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_SEND_PACKET,
                peer_id: context.peer_id,
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
                period: context.period,
                round: context.round,
            });
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

    /// Returns the number of retained executor result reports.
    #[must_use]
    pub fn effect_result_len(&self) -> usize {
        self.effect_results.len()
    }
}

fn next_votes_bundle_result_is_valid(
    result: &NetworkEffectResult,
    period: u64,
    round: u64,
) -> bool {
    if result.status == NETWORK_EFFECT_RESULT_STATUS_FAILED {
        return result.payload_bytes.is_empty() && result.related_payload_bytes.is_empty();
    }
    validate_and_chunk_next_votes_bundle(&result.payload_bytes, period, round, false).is_some()
        && validate_and_chunk_next_votes_bundle(&result.related_payload_bytes, period, round, true)
            .is_some()
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
        && effect.object_kind == NETWORK_OBJECT_KIND_PBFT_VOTE;
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
            payload_bytes: Vec::new(),
            related_payload_bytes: Vec::new(),
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
    fn next_votes_request_gate_queues_only_eligible_previous_round_leaf() {
        let mut api = ConsensusNetworkApi::new();
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
            let decision = api.ingest_pbft_next_votes_bundle_request(request);
            assert!(decision.routed);
            assert_eq!(decision.status, expected_status);
            assert_eq!(decision.queued_effect_count, 0);
            assert_eq!(decision.application_effect_id, 0);
        }

        let decision = api.ingest_pbft_next_votes_bundle_request(next_votes_request());
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert_eq!(decision.queued_effect_count, 1);
        let batch = api.drain_work(6, 4);
        assert_eq!(batch.effects.len(), 1);
        let effect = &batch.effects[0];
        assert_eq!(effect.effect_id, decision.application_effect_id);
        assert_eq!(effect.source_payload_id, 99);
        assert_eq!(effect.peer_id, peer(7));
        assert_eq!(effect.period, 10);
        assert_eq!(effect.round, 2);
        assert_eq!(
            effect.object_kind,
            NETWORK_OBJECT_KIND_PBFT_NEXT_VOTES_BUNDLE_EGRESS_REQUEST
        );
    }

    #[test]
    fn next_votes_result_chunks_and_orders_next_before_next_null_sends() {
        let mut api = ConsensusNetworkApi::new();
        api.ingest_pbft_next_votes_bundle_request(next_votes_request());
        let application = api.drain_work(6, 1).effects.remove(0);
        let mut result = effect_result(&application, NETWORK_EFFECT_RESULT_STATUS_OK);
        result.payload_bytes = optimized_bundle([0x44; 32], 10, 2, 1001, 1);
        result.related_payload_bytes = optimized_bundle([0; 32], 10, 2, 1, 9);
        let ack = api.report_effect_results(vec![result]);
        assert_eq!(ack.status, NETWORK_EFFECT_ACK_STATUS_ACCEPTED);
        assert!(api.effect_results[0].payload_bytes.is_empty());
        assert!(api.effect_results[0].related_payload_bytes.is_empty());

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
    fn next_votes_result_rejects_swapped_family_atomically_then_accepts_retry() {
        let mut api = ConsensusNetworkApi::new();
        api.ingest_pbft_next_votes_bundle_request(next_votes_request());
        let application = api.drain_work(6, 1).effects.remove(0);
        let mut invalid = effect_result(&application, NETWORK_EFFECT_RESULT_STATUS_OK);
        invalid.payload_bytes = optimized_bundle([0; 32], 10, 2, 1, 1);
        invalid.related_payload_bytes = optimized_bundle([0x55; 32], 10, 2, 1, 2);
        let ack = api.report_effect_results(vec![invalid]);
        assert_eq!(
            ack.status,
            NETWORK_EFFECT_ACK_STATUS_MISMATCHED_EFFECT_RESULT
        );
        assert!(api.drain_work(6, 8).effects.is_empty());

        let mut retry = effect_result(&application, NETWORK_EFFECT_RESULT_STATUS_OK);
        retry.payload_bytes = optimized_bundle([0x55; 32], 10, 2, 1, 1);
        retry.related_payload_bytes = optimized_bundle([0; 32], 10, 2, 1, 2);
        let ack = api.report_effect_results(vec![retry]);
        assert_eq!(ack.status, NETWORK_EFFECT_ACK_STATUS_ACCEPTED);
        assert_eq!(api.drain_work(6, 8).effects.len(), 2);
    }

    #[test]
    fn next_votes_failed_leaf_requires_empty_payloads_and_emits_no_sends() {
        let mut api = ConsensusNetworkApi::new();
        api.ingest_pbft_next_votes_bundle_request(next_votes_request());
        let application = api.drain_work(6, 1).effects.remove(0);
        let mut invalid = effect_result(&application, NETWORK_EFFECT_RESULT_STATUS_FAILED);
        invalid.payload_bytes = optimized_bundle([0x55; 32], 10, 2, 1, 1);
        assert_eq!(
            api.report_effect_results(vec![invalid]).status,
            NETWORK_EFFECT_ACK_STATUS_MISMATCHED_EFFECT_RESULT
        );

        let failed = effect_result(&application, NETWORK_EFFECT_RESULT_STATUS_FAILED);
        assert_eq!(
            api.report_effect_results(vec![failed]).status,
            NETWORK_EFFECT_ACK_STATUS_ACCEPTED
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
                payload_bytes: Vec::new(),
                related_payload_bytes: Vec::new(),
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
                payload_bytes: Vec::new(),
                related_payload_bytes: Vec::new(),
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
        assert_eq!(api.effect_result_len(), 0);
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
        assert_eq!(api.effect_result_len(), 1);
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
        assert_eq!(api.effect_result_len(), 0);
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
}
