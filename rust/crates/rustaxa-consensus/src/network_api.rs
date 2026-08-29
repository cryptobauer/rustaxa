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

use crate::consensus_application::{DagBlockIngressReport, TransactionPacketIngressReport};
use crate::dag::{
    DAG_VERIFY_REJECT_ADD_BLOCK_METADATA, DAG_VERIFY_REJECT_AHEAD_BLOCK,
    DAG_VERIFY_REJECT_BLOCK_TOO_BIG, DAG_VERIFY_REJECT_EXPIRED_BLOCK,
    DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION, DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION,
    DAG_VERIFY_REJECT_FUTURE_BLOCK, DAG_VERIFY_REJECT_INCORRECT_TRANSACTIONS_ESTIMATION,
    DAG_VERIFY_REJECT_MISSING_TIP, DAG_VERIFY_REJECT_MISSING_TRANSACTION,
    DAG_VERIFY_REJECT_NOT_ELIGIBLE,
};
use crate::dag_service::DagRuntimeNonFinalizedSyncPayload;
use crate::dag_transaction_service::TransactionGossipAccount;
use crate::final_chain::FinalChain;
use crate::pbft_chain::PbftChainService;
use crate::pbft_vote_payload::build_optimized_pbft_vote_bundle;
use crate::pbft_vote_progress::PbftVoteProgressIntent;
use crate::pbft_vote_runtime::{PbftNextVotesBundleEgressPayloads, PbftVerifiedVotesService};
use crate::period_data_queue::{DecodedPbftSyncPacketPrecheck, decode_pbft_sync_packet_precheck};
use crate::pillar_chain_service::PillarChainService;
use crate::pillar_vote_service::{
    PillarVoteRecord, PillarVoteSingleAdmissionContext, PillarVoteSingleAdmissionWithFinalChainPlan,
};
use crate::proposed_blocks::ProposedBlocksService;
use crate::transaction_queue::TransactionQueueInsertStatus;
use crate::{
    PbftVoteIngressContext, PbftVoteIngressFact, PbftVoteIngressPlan, PbftVoteIngressStatus,
    PbftVotePayloadRecord, inspect_canonical_pbft_vote, inspect_pillar_vote_from_rlp,
    plan_pbft_vote_bundle_ingress, plan_pbft_vote_ingress,
};
use anyhow::{Context, Result, anyhow, ensure};
use ethereum_types::H256;
use rlp::{Rlp, RlpStream};
use rustaxa_storage::Storage;
use rustaxa_types::LegacyTransactionEnvelope;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::pbft::PbftBlockLink;
use rustaxa_types::{
    PbftBlockMetadata, PillarVote, decode_optimized_pillar_votes_bundle_rlp,
    encode_optimized_pillar_votes_bundle_rlp,
};

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
/// A peer supplied malformed or non-canonical packet bytes.
pub const NETWORK_INGRESS_STATUS_MALFORMED_PACKET: u8 = 11;

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
/// Network status ingress received a periodic update before initial admission completed.
pub const NETWORK_STATUS_PLAN_STATUS_PENDING_PEER_PERIODIC: u8 = 9;

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
/// Network peer report/disconnect reason for pillar votes sent before Ficus activation.
pub const NETWORK_REASON_PREACTIVATION_PILLAR_VOTE: u8 = 4;
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
/// A proposed-block bundle is not canonical one-field RLP.
pub const NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_MALFORMED: u8 = 16;
/// A proposed-block bundle exceeds the legacy ten-block limit.
pub const NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_TOO_LARGE: u8 = 17;
/// Two relevant proposed blocks have the same author and period.
pub const NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_DUPLICATE_AUTHOR: u8 = 18;
/// A relevant proposed block author is not eligible for its prior period.
pub const NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_INELIGIBLE_AUTHOR: u8 = 19;
/// A valid PBFT-sync block is already present in the native finalized chain.
pub const NETWORK_INGRESS_STATUS_PBFT_SYNC_DUPLICATE_BLOCK: u8 = 20;
/// A final-certificate packet proves the local sync cursor is complete.
pub const NETWORK_INGRESS_STATUS_PBFT_SYNC_COMPLETE: u8 = 21;
/// A valid PBFT-sync packet has an ordinary unexpected period and is dropped.
pub const NETWORK_INGRESS_STATUS_PBFT_SYNC_UNEXPECTED_PERIOD: u8 = 22;
/// A malformed or deterministically invalid PBFT-sync packet is malicious.
pub const NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS: u8 = 23;
/// A transaction packet does not have the canonical two-list shape.
pub const NETWORK_INGRESS_STATUS_TRANSACTION_PACKET_MALFORMED: u8 = 24;
/// A transaction packet exceeds a protocol member limit.
pub const NETWORK_INGRESS_STATUS_TRANSACTION_PACKET_TOO_LARGE: u8 = 25;
/// A transaction packet contains a deterministically rejected transaction.
pub const NETWORK_INGRESS_STATUS_TRANSACTION_REJECTED: u8 = 26;
/// A get-DAG-sync request is not canonical period/hash-list RLP.
pub const NETWORK_INGRESS_STATUS_DAG_SYNC_REQUEST_MALFORMED: u8 = 27;
/// A peer repeated get-DAG-sync before the transport rate window elapsed.
pub const NETWORK_INGRESS_STATUS_DAG_SYNC_REQUEST_THROTTLED: u8 = 28;
pub const NETWORK_INGRESS_STATUS_DAG_PACKET_MALFORMED: u8 = 29;
pub const NETWORK_INGRESS_STATUS_DAG_BLOCK_REJECTED: u8 = 30;
pub const NETWORK_INGRESS_STATUS_DAG_SYNC_PERIOD_AHEAD: u8 = 31;
pub const NETWORK_INGRESS_STATUS_DAG_SYNC_PERIOD_BEHIND: u8 = 32;
/// A rejected DAG block is intentionally ignored under peer-sync policy.
pub const NETWORK_INGRESS_STATUS_DAG_BLOCK_IGNORED: u8 = 33;
/// A rejected DAG block scheduled DAG recovery work.
pub const NETWORK_INGRESS_STATUS_DAG_BLOCK_SYNC_REQUESTED: u8 = 34;
/// A rejected DAG block requires disconnecting the peer without a malicious report.
pub const NETWORK_INGRESS_STATUS_DAG_BLOCK_DISCONNECT: u8 = 35;
/// A rejected DAG block is malicious under the legacy peer policy.
pub const NETWORK_INGRESS_STATUS_DAG_BLOCK_MALICIOUS: u8 = 36;

/// No DAG rejection action applies to an accepted or duplicate block.
pub const NETWORK_DAG_REJECTION_ACTION_NONE: u8 = 0;
/// Ignore the rejected block without mutating peer state.
pub const NETWORK_DAG_REJECTION_ACTION_IGNORE: u8 = 1;
/// Mark the peer DAG-unsynced and request DAG synchronization.
pub const NETWORK_DAG_REJECTION_ACTION_REQUEST_DAG_SYNC: u8 = 2;
/// Request pending DAG blocks once from an already-unsynced peer.
pub const NETWORK_DAG_REJECTION_ACTION_REQUEST_PENDING_DAG: u8 = 3;
/// Disconnect the peer without classifying it as malicious.
pub const NETWORK_DAG_REJECTION_ACTION_DISCONNECT: u8 = 4;
/// Report the peer as malicious and disconnect it.
pub const NETWORK_DAG_REJECTION_ACTION_MALICIOUS: u8 = 5;

const ERROR_PBFT_SYNC_MALFORMED_REQUEST: &str = "NETWORK_PBFT_SYNC_MALFORMED_REQUEST";
const ERROR_PBFT_SYNC_UNSUPPORTED_VERSION: &str = "NETWORK_PBFT_SYNC_UNSUPPORTED_VERSION";
const ERROR_PBFT_SYNC_HEIGHT_AHEAD: &str = "NETWORK_PBFT_SYNC_HEIGHT_AHEAD";
const ERROR_PBFT_SYNC_HISTORY_UNAVAILABLE: &str = "NETWORK_PBFT_SYNC_HISTORY_UNAVAILABLE";
const ERROR_PBFT_SYNC_PERIOD_DATA_MISSING: &str = "NETWORK_PBFT_SYNC_PERIOD_DATA_MISSING";
const ERROR_PBFT_BLOCKS_BUNDLE_MALFORMED: &str = "NETWORK_PBFT_BLOCKS_BUNDLE_MALFORMED";
const ERROR_PBFT_BLOCKS_BUNDLE_TOO_LARGE: &str = "NETWORK_PBFT_BLOCKS_BUNDLE_TOO_LARGE";
const ERROR_PBFT_BLOCKS_BUNDLE_DUPLICATE_AUTHOR: &str =
    "NETWORK_PBFT_BLOCKS_BUNDLE_DUPLICATE_AUTHOR";
const ERROR_PBFT_BLOCKS_BUNDLE_INELIGIBLE_AUTHOR: &str =
    "NETWORK_PBFT_BLOCKS_BUNDLE_INELIGIBLE_AUTHOR";
const ERROR_PBFT_SYNC_PACKET_MALFORMED: &str = "NETWORK_PBFT_SYNC_PACKET_MALFORMED";
const ERROR_PBFT_SYNC_PACKET_DUPLICATE_BLOCK: &str = "NETWORK_PBFT_SYNC_PACKET_DUPLICATE_BLOCK";
const ERROR_PBFT_SYNC_PACKET_COMPLETE: &str = "NETWORK_PBFT_SYNC_PACKET_COMPLETE";
const ERROR_PBFT_SYNC_PACKET_UNEXPECTED_PERIOD: &str = "NETWORK_PBFT_SYNC_PACKET_UNEXPECTED_PERIOD";
const ERROR_PBFT_SYNC_PACKET_CERT_SIGNATURE: &str = "NETWORK_PBFT_SYNC_PACKET_CERT_SIGNATURE";
const ERROR_PBFT_SYNC_PACKET_CURRENT_CERT_HASH: &str = "NETWORK_PBFT_SYNC_PACKET_CURRENT_CERT_HASH";
const ERROR_PBFT_SYNC_PACKET_PREVIOUS_CERT_HASH: &str =
    "NETWORK_PBFT_SYNC_PACKET_PREVIOUS_CERT_HASH";
const ERROR_PBFT_SYNC_PACKET_PILLAR_SCHEDULE: &str = "NETWORK_PBFT_SYNC_PACKET_PILLAR_SCHEDULE";
const ERROR_PBFT_SYNC_PACKET_ORDER_HASH: &str = "NETWORK_PBFT_SYNC_PACKET_ORDER_HASH";
const ERROR_TRANSACTION_PACKET_MALFORMED: &str = "NETWORK_TRANSACTION_PACKET_MALFORMED";
const ERROR_TRANSACTION_PACKET_TOO_LARGE: &str = "NETWORK_TRANSACTION_PACKET_TOO_LARGE";
const ERROR_TRANSACTION_PACKET_REJECTED: &str = "NETWORK_TRANSACTION_PACKET_REJECTED";
const ERROR_DAG_SYNC_REQUEST_MALFORMED: &str = "NETWORK_DAG_SYNC_REQUEST_MALFORMED";
const ERROR_DAG_SYNC_REQUEST_THROTTLED: &str = "NETWORK_DAG_SYNC_REQUEST_THROTTLED";
const ERROR_DAG_PACKET_MALFORMED: &str = "NETWORK_DAG_PACKET_MALFORMED";
const ERROR_DAG_BLOCK_REJECTED: &str = "NETWORK_DAG_BLOCK_REJECTED";
const ERROR_DAG_BLOCK_IGNORED: &str = "NETWORK_DAG_BLOCK_IGNORED";
const ERROR_DAG_BLOCK_SYNC_REQUESTED: &str = "NETWORK_DAG_BLOCK_SYNC_REQUESTED";
const ERROR_DAG_BLOCK_DISCONNECT: &str = "NETWORK_DAG_BLOCK_DISCONNECT";
const ERROR_DAG_BLOCK_MALICIOUS: &str = "NETWORK_DAG_BLOCK_MALICIOUS";
const ERROR_DAG_SYNC_PERIOD_AHEAD: &str = "NETWORK_DAG_SYNC_PERIOD_AHEAD";
const ERROR_DAG_SYNC_PERIOD_BEHIND: &str = "NETWORK_DAG_SYNC_PERIOD_BEHIND";
const MAX_PBFT_BLOCKS_PER_BUNDLE: usize = 10;
const MAX_PBFT_BLOCK_EXTRA_DATA_BYTES: usize = 1024;
const MAX_TRANSACTIONS_PER_PACKET: usize = 500;
const MAX_TRANSACTION_HASHES_PER_PACKET: usize = 5000;
const MAX_PENDING_EGRESS_OPERATIONS: usize = 256;
const MAX_EGRESS_OBJECT_PROBES: usize = 5_500;
const MAX_QUEUED_EGRESS_EFFECTS: usize = 65_536;

/// Native egress family for one bounded prepare/snapshot/plan operation.
pub const NETWORK_EGRESS_FAMILY_PBFT_VOTE: u8 = 0;
pub const NETWORK_EGRESS_FAMILY_PBFT_VOTES_BUNDLE: u8 = 1;
pub const NETWORK_EGRESS_FAMILY_PILLAR_VOTE: u8 = 2;
pub const NETWORK_EGRESS_FAMILY_DAG_BLOCK: u8 = 3;
pub const NETWORK_EGRESS_FAMILY_TRANSACTION_GOSSIP: u8 = 4;
pub const NETWORK_EGRESS_FAMILY_PILLAR_VOTES_REQUEST: u8 = 5;

/// Canonical application egress input retained only until one snapshot is committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEgressPrepareRequest {
    pub family: u8,
    pub transport_lane: u32,
    pub source_payload_id: u64,
    pub source_peer_id: [u8; 64],
    pub rebroadcast: bool,
    pub object_hash: [u8; 32],
    pub payload_bytes: Vec<u8>,
    pub related_payload_bytes: Vec<u8>,
}

/// Exact object identity whose known state must be sampled by transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEgressProbe {
    pub probe_id: u32,
    pub object_kind: u8,
    pub object_hash: [u8; 32],
}

/// One-shot native preparation result. The token is invalid after plan/cancel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEgressPreparation {
    pub token: u64,
    pub probes: Vec<NetworkEgressProbe>,
}

/// Immutable transport facts for one authenticated peer and exact native probes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEgressPeerSnapshot {
    /// Tarcap lane that owns the peer's physical socket.
    pub transport_lane: u32,
    /// Canonical 512-bit peer identity.
    pub peer_id: [u8; 64],
    /// Whether the peer is currently syncing and therefore ineligible for egress.
    pub syncing: bool,
    /// Prepared object probes already known by this peer.
    pub known_probe_ids: Vec<u32>,
    /// Peer-advertised PBFT chain size used by native target selection.
    pub pbft_chain_size: u64,
    /// Peer-advertised DAG level used as the target-selection tie breaker.
    pub dag_level: u64,
    /// Whether the peer serves only a bounded PBFT history.
    pub is_light_node: bool,
    /// Number of historical PBFT periods retained by a light node.
    pub light_node_history: u64,
}

impl Default for NetworkEgressPeerSnapshot {
    fn default() -> Self {
        Self {
            transport_lane: 0,
            peer_id: [0; 64],
            syncing: false,
            known_probe_ids: Vec::new(),
            pbft_chain_size: 0,
            dag_level: 0,
            is_light_node: false,
            light_node_history: 0,
        }
    }
}

/// Commit input for a previously prepared native egress operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEgressPlanRequest {
    pub token: u64,
    pub peers: Vec<NetworkEgressPeerSnapshot>,
}

/// Transport and peer facts for one canonical transaction packet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkTransactionPacketContext {
    pub transport_lane: u32,
    pub peer_id: [u8; 64],
    pub source_payload_id: u64,
}

/// Terminal native transaction-packet admission report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkTransactionPacketReport {
    pub decision: NetworkIngressDecision,
    pub transactions: Vec<TransactionPacketIngressReport>,
    pub extra_transaction_hashes: Vec<[u8; 32]>,
}

/// Transport facts for one canonical get-DAG-sync request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkGetDagSyncContext {
    pub transport_lane: u32,
    pub peer_id: [u8; 64],
    pub source_payload_id: u64,
    pub request_allowed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkDagBlockIngressContext {
    pub transport_lane: u32,
    pub peer_id: [u8; 64],
    pub source_payload_id: u64,
    pub rebroadcast: bool,
    /// Whether tarcap currently considers this peer's DAG synchronized.
    pub peer_dag_synced: bool,
    /// Whether a full DAG-sync request may be started for this peer now.
    pub dag_sync_allowed: bool,
    /// Whether the local transaction pool has dropped transactions.
    pub transactions_dropped: bool,
    /// Whether a pending-DAG request is already outstanding for this peer.
    pub pending_dag_request: bool,
    /// Whether local PBFT synchronization suppresses add-stage peer actions.
    pub local_pbft_syncing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PendingNetworkEgressPayload {
    PbftVote {
        vote_rlp: Vec<u8>,
        block_rlp: Vec<u8>,
        vote_hash: [u8; 32],
        block_hash: [u8; 32],
        period: u64,
        finalized_period: u64,
    },
    PbftVotesBundle {
        votes: Vec<PbftVotePayloadRecord>,
        period: u64,
        round: u64,
    },
    PillarVote {
        vote_rlp: Vec<u8>,
        vote_hash: [u8; 32],
        period: u64,
    },
    DagBlock {
        block_rlp: Vec<u8>,
        block_hash: [u8; 32],
        transactions: Vec<crate::TransactionGossipEntry>,
    },
    TransactionGossip {
        accounts: Vec<TransactionGossipAccount>,
    },
    PillarVotesRequest {
        period: u64,
        pillar_block_hash: [u8; 32],
        local_pbft_syncing_period: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingNetworkEgress {
    transport_lane: u32,
    source_payload_id: u64,
    source_peer_id: [u8; 64],
    rebroadcast: bool,
    probes: Vec<NetworkEgressProbe>,
    payload: PendingNetworkEgressPayload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkDagBlockIngressReport {
    pub decision: NetworkIngressDecision,
    pub admission: Option<DagBlockIngressReport>,
    /// Exact peer-policy action selected from native rejection facts.
    pub rejection_action: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkDagSyncIngressReport {
    pub decision: NetworkIngressDecision,
    pub request_period: u64,
    pub response_period: u64,
    /// Ordered transaction admissions committed before DAG blocks.
    pub transactions: Vec<TransactionPacketIngressReport>,
    pub blocks: Vec<DagBlockIngressReport>,
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
}

/// Canonical consensus-packet ingress plus the compact live policy snapshot.
///
/// `packet_rlp` is the complete tarcap packet payload, including its outer
/// family wrapper. Rust decodes every vote, optional proposed block, optimized
/// bundle member, and recovered voter from these bytes. Network-owned timer
/// reservations remain booleans because the physical scheduler owns their
/// clocks; all other fields are immutable consensus-window facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkConsensusPacketRequest {
    pub transport_lane: u32,
    pub peer_id: [u8; 64],
    pub peer_pbft_chain_size: u64,
    pub source_payload_id: u64,
    pub packet_rlp: Vec<u8>,
    pub current_period: u64,
    pub current_round: u64,
    pub current_step: u64,
    pub max_future_period_delta: u64,
    pub max_future_round_delta: u64,
    pub max_future_step_delta: u64,
    pub validate_max_round_step: bool,
    pub can_request_pbft_sync: bool,
    pub can_request_next_votes_sync: bool,
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
    /// Optional network-owned packet identity for effect correlation.
    pub source_payload_id: u64,
}

/// Canonical get-pillar-votes-bundle packet and transport identity.
///
/// Rust strictly decodes the complete two-field packet. Schedule policy comes
/// from application bootstrap; callers cannot inject decoded consensus facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkGetPillarVotesBundlePacketRequest {
    /// Tarcap version/lane that owns physical packet sends.
    pub transport_lane: u32,
    /// Peer that requested the pillar votes.
    pub peer_id: [u8; 64],
    /// Optional network-owned packet identity for effect correlation.
    pub source_payload_id: u64,
    /// Complete canonical request payload `[period, pillar_block_hash]`.
    pub packet_rlp: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GetPillarVotesBundleQuery {
    period: u64,
    pillar_block_hash: [u8; 32],
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

/// Composed native result for one PBFT vote arriving from tarcap.
///
/// Routing and authoritative vote admission complete before this value is
/// returned. `admission` is absent for a routing rejection or for a bundle
/// member cancelled after an earlier member produced a slashing conflict.
/// Network transport follow-ups are queued by the same root operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftVoteAdmissionOutcome {
    /// Packet-family routing decision.
    pub decision: NetworkIngressDecision,
    /// Native admission result when the vote reached the PBFT task.
    pub admission: Option<crate::PbftVoteAdmissionWithSlashingResult>,
}

/// Terminal result for one canonical PBFT vote-family packet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftVotePacketReport {
    /// One terminal outcome for a single packet, or one per admitted bundle member.
    pub outcomes: Vec<NetworkPbftVoteAdmissionOutcome>,
    /// Whether a single-vote packet carried the optional proposed-block payload.
    pub has_peer_pbft_chain_size: bool,
    /// Peer chain size carried beside the optional proposed block.
    pub peer_pbft_chain_size: u64,
    /// Canonical inner bundle containing only members selected for rebroadcast.
    pub egress_payload_bytes: Vec<u8>,
}

/// Composed native result for one pillar vote arriving from tarcap.
///
/// Routing and authoritative pillar admission complete before this value is
/// returned. `admission` is absent only when packet preflight rejected the
/// member. The terminal result retains exact acceptance, duplication,
/// conflict, status, and hash facts after the private application effect id is
/// consumed; transport follow-ups have already been queued by the same root
/// operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPillarVoteAdmissionOutcome {
    /// Packet-family routing decision with no outstanding application effect.
    pub decision: NetworkIngressDecision,
    /// Native pillar admission result when the member reached the PBFT task.
    pub admission: Option<PillarVoteSingleAdmissionWithFinalChainPlan>,
}

/// Terminal result for one canonical pillar-vote-family packet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPillarVotePacketReport {
    /// One terminal outcome for a single packet, or one per bundle member.
    pub outcomes: Vec<NetworkPillarVoteAdmissionOutcome>,
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
    /// Bundle aggregation identity and member data, when this vote came from
    /// one all-or-nothing preflighted bundle.
    bundle: Option<PendingVoteBundleMember>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingVoteBundleMember {
    bundle_id: u64,
    index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingVoteBundle {
    completed: Vec<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingPillarVoteAdmissionContext {
    transport_lane: u32,
    peer_id: [u8; 64],
    vote_hash: [u8; 32],
    vote_rlp: Vec<u8>,
    period: u64,
    source_payload_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PillarVoteNetworkChunk {
    vote_hashes: Vec<[u8; 32]>,
    payload_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NextVotesNetworkChunk {
    vote_hashes: Vec<[u8; 32]>,
    payload_bytes: Vec<u8>,
}

/// Compact local and peer facts needed to plan status-triggered sync work.
#[derive(Clone, Debug, Eq, PartialEq)]
struct NetworkStatusSyncFacts {
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
struct NetworkStatusSyncPlan {
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

/// Compact facts needed to validate an initial status packet.
#[derive(Clone, Debug, Eq, PartialEq)]
struct NetworkInitialStatusFacts {
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
struct NetworkInitialStatusPlan {
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

/// Stable stop reasons reported by the native PBFT-sync lifecycle.
pub const NETWORK_PBFT_SYNC_STOP_REASON_NONE: u8 = 0;
pub const NETWORK_PBFT_SYNC_STOP_REASON_COMPLETED: u8 = 1;
pub const NETWORK_PBFT_SYNC_STOP_REASON_INACTIVE: u8 = 2;
pub const NETWORK_PBFT_SYNC_STOP_REASON_DISCONNECTED: u8 = 3;
pub const NETWORK_PBFT_SYNC_STOP_REASON_TRANSPORT_FAILED: u8 = 4;
pub const NETWORK_PBFT_SYNC_STOP_REASON_REPLACED: u8 = 5;

const NETWORK_PBFT_SYNC_INACTIVITY_THRESHOLD_MS: u64 = 60_000;

/// Immutable protocol identity copied into the native network service once at
/// application bootstrap. Status ingress and egress can therefore neither
/// spoof nor accidentally drift chain, genesis, version, or history policy.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkNodeIdentity {
    pub chain_id: u64,
    pub genesis_hash: [u8; 32],
    pub node_major_version: u32,
    pub node_minor_version: u32,
    pub node_patch_version: u32,
    pub is_light_node: bool,
    pub light_node_history: u64,
}

/// Canonical status-packet ingress plus the local facts needed for native
/// follow-up policy. The peer payload crosses the bridge exactly once and is
/// decoded strictly in Rust; immutable node identity is service-owned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkStatusPacketRequest {
    pub peer_id: [u8; 64],
    pub packet_rlp: Vec<u8>,
    pub source_peer_ready: bool,
    pub local_pbft_synced_period: u64,
    pub local_pbft_period: u64,
    pub local_pbft_round: u64,
    pub peer_dag_synced: bool,
}

/// Typed peer-bookkeeping and follow-up report for one canonical status packet.
/// Malformed packets never mutate debounce state and are marked malicious.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkStatusPacketReport {
    pub status: u8,
    pub error_code: String,
    pub malicious: bool,
    pub initial: bool,
    pub accept_peer: bool,
    pub disconnect_peer: bool,
    pub peer_pbft_chain_size: u64,
    pub peer_pbft_period: u64,
    pub peer_pbft_round: u64,
    pub peer_dag_level: u64,
    pub peer_syncing: bool,
    pub peer_is_light_node: bool,
    pub peer_light_node_history: u64,
    pub node_major_version: u32,
    pub node_minor_version: u32,
    pub node_patch_version: u32,
    pub request_pbft_sync: bool,
    pub request_pending_dag_blocks: bool,
    pub request_next_votes: bool,
    pub next_votes_period: u64,
    pub next_votes_round: u64,
    /// Exact canonical payload for `GetNextVotesSyncPacket`, present only when requested.
    pub next_votes_request_rlp: Vec<u8>,
    pub sync_generation: u64,
}

/// Mutable local status facts used to build one exact canonical wire payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkStatusPacketBuildRequest {
    pub initial: bool,
    pub local_pbft_chain_size: u64,
    pub local_pbft_round: u64,
    pub local_dag_level: u64,
}

/// Terminal status-packet build result. Successful output is ready for direct
/// tarcap packet wrapping and must not be re-encoded by the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkStatusPacketBuildOutcome {
    pub status: u8,
    pub error_code: String,
    pub packet_rlp: Vec<u8>,
}

/// Canonical get-next-votes packet ingress with exact transport identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftNextVotesBundlePacketRequest {
    pub transport_lane: u32,
    pub peer_id: [u8; 64],
    pub source_payload_id: u64,
    pub packet_rlp: Vec<u8>,
}

/// Application-root request to atomically select a peer and begin PBFT sync.
///
/// The caller supplies a canonical snapshot of currently connected peers and
/// the monotonic time at which the snapshot was taken. Rust filters and orders
/// the candidates, rejects concurrent starts, creates a new generation, and
/// initializes activity/deep-sync state. Peer/socket ownership remains outside
/// the application root; a later transport failure must stop this exact
/// generation rather than an unrelated replacement session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncStartRequest {
    /// Whether to start a generation (`true`) or only select the best peer (`false`).
    pub start: bool,
    pub now_ms: u64,
    pub local_pbft_synced_period: u64,
    pub local_pbft_chain_size: u64,
    pub candidates: Vec<NetworkPbftSyncPeerCandidate>,
}

/// Result of an atomic native PBFT-sync start operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncStartOutcome {
    pub status: u8,
    pub error_code: String,
    pub started: bool,
    pub has_peer: bool,
    pub peer_id: [u8; 64],
    pub peer_pbft_chain_size: u64,
    pub request_period: u64,
    pub generation: u64,
    pub deep_syncing: bool,
    pub enable_snapshot_creation: bool,
}

/// Status packet facts needed for application-owned follow-up decisions.
///
/// The service retains the previous chain-size advertisement per peer, so the
/// one-block-behind debounce no longer relies on a handler-local sidecar. The
/// returned decision is computed and the new advertisement is recorded under
/// the same network-service lock.
#[derive(Clone, Debug, Eq, PartialEq)]
struct NetworkStatusFollowupRequest {
    pub peer_id: [u8; 64],
    pub local_pbft_synced_period: u64,
    pub local_pbft_period: u64,
    pub local_pbft_round: u64,
    pub peer_pbft_chain_size: u64,
    pub peer_pbft_period: u64,
    pub peer_pbft_round: u64,
    pub peer_dag_synced: bool,
}

/// Application-owned work selected after one accepted periodic status packet.
#[derive(Clone, Debug, Eq, PartialEq)]
struct NetworkStatusFollowupOutcome {
    pub request_pbft_sync: bool,
    pub request_pending_dag_blocks: bool,
    pub request_next_votes: bool,
    pub next_votes_period: u64,
    pub next_votes_round: u64,
    pub sync_generation: u64,
}

/// Correlation mode for PBFT synchronization responses.
pub const NETWORK_PBFT_SYNC_SOURCE_ACTIVE: u8 = 0;
pub const NETWORK_PBFT_SYNC_SOURCE_LAST: u8 = 1;

/// Request to correlate one response source with native PBFT-sync state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncSourceRequest {
    pub peer_id: [u8; 64],
    pub source: u8,
}

/// Result of PBFT synchronization response-source correlation.
///
/// Accepted active-source responses carry the exact generation that callers
/// must report with later activity, stop, and asynchronous executor results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncSourceOutcome {
    pub accepted: bool,
    pub generation: u64,
    pub active: bool,
    pub error_code: String,
}

/// Generation-scoped report that an accepted PBFT-sync response made progress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncActivityRequest {
    pub now_ms: u64,
    pub generation: u64,
    pub peer_id: [u8; 64],
}

/// Result of recording generation-scoped PBFT-sync activity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncActivityOutcome {
    pub accepted: bool,
    pub generation: u64,
    pub deep_syncing: bool,
    pub error_code: String,
}

/// Generation-scoped request to stop one PBFT-sync session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncStopRequest {
    pub generation: u64,
    pub peer_id: [u8; 64],
    pub reason: u8,
}

/// Result of a generation-scoped PBFT-sync stop request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncStopOutcome {
    pub stopped: bool,
    pub generation: u64,
    pub error_code: String,
}

/// Generation-scoped notification that the selected transport peer disconnected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncDisconnectRequest {
    pub generation: u64,
    pub peer_id: [u8; 64],
}

/// Recovery decision after one selected-peer disconnect notification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncDisconnectOutcome {
    pub stopped: bool,
    pub restart_sync: bool,
    pub generation: u64,
    pub error_code: String,
}

/// Generation-scoped inactivity check driven by the network timer lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncTickRequest {
    pub now_ms: u64,
    pub generation: u64,
}

/// Native inactivity and restart decision for one timer tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncTickOutcome {
    pub expired: bool,
    pub restart_sync: bool,
    pub generation: u64,
    pub error_code: String,
}

/// Shared application-root command envelope for generation-correlated PBFT sync.
///
/// Each command consumes only the fields required by its kind. Unknown kinds
/// are rejected without mutating lifecycle state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncCommandRequest {
    pub kind: u8,
    pub now_ms: u64,
    pub generation: u64,
    pub peer_id: [u8; 64],
    pub source: u8,
    pub reason: u8,
    pub sync_queue_size: u64,
    pub syncing_period: u64,
    pub finalized_period: u64,
    pub remote_period: u64,
    pub sync_level_size: u64,
    pub retry_count: u32,
    pub retry_delay_ms: u64,
}

/// Uniform lifecycle result returned to the physical network adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncCommandOutcome {
    pub accepted: bool,
    pub active: bool,
    pub stopped: bool,
    pub expired: bool,
    pub restart_sync: bool,
    pub retry: bool,
    pub request_next: bool,
    pub request_pending_dag_if_idle: bool,
    pub deep_syncing: bool,
    pub generation: u64,
    pub error_code: String,
}

/// Read-only public/query view of the native PBFT-sync lifecycle.
///
/// Reading this snapshot never expires or otherwise mutates a session. The
/// supplied monotonic time is used only to derive elapsed/activity durations;
/// timer-lane callers must invoke [`ConsensusNetworkService::tick_pbft_sync`]
/// to apply inactivity policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPbftSyncSnapshot {
    pub active: bool,
    pub deep_syncing: bool,
    pub generation: u64,
    pub has_peer: bool,
    pub peer_id: [u8; 64],
    pub has_last_peer: bool,
    pub last_peer_id: [u8; 64],
    pub target_chain_size: u64,
    pub current_period: u64,
    pub request_period: u64,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
    pub elapsed_ms: u64,
    pub inactive_for_ms: u64,
    pub start_count: u64,
    pub stop_count: u64,
    pub inactivity_count: u64,
    pub disconnect_count: u64,
    pub last_stop_reason: u8,
}

#[derive(Clone, Debug)]
struct NetworkPbftSyncLifecycle {
    active: bool,
    deep_syncing: bool,
    generation: u64,
    peer_id: [u8; 64],
    last_peer_id: [u8; 64],
    has_last_peer: bool,
    target_chain_size: u64,
    current_period: u64,
    request_period: u64,
    started_at_ms: u64,
    last_activity_ms: u64,
    deep_syncing_threshold: u64,
    inactivity_threshold_ms: u64,
    start_count: u64,
    stop_count: u64,
    inactivity_count: u64,
    disconnect_count: u64,
    last_stop_reason: u8,
    peer_last_status_chain_size: HashMap<[u8; 64], u64>,
}

impl NetworkPbftSyncLifecycle {
    fn new(deep_syncing_threshold: u64) -> Self {
        Self {
            active: false,
            deep_syncing: false,
            generation: 0,
            peer_id: [0; 64],
            last_peer_id: [0; 64],
            has_last_peer: false,
            target_chain_size: 0,
            current_period: 0,
            request_period: 0,
            started_at_ms: 0,
            last_activity_ms: 0,
            deep_syncing_threshold,
            inactivity_threshold_ms: NETWORK_PBFT_SYNC_INACTIVITY_THRESHOLD_MS,
            start_count: 0,
            stop_count: 0,
            inactivity_count: 0,
            disconnect_count: 0,
            last_stop_reason: NETWORK_PBFT_SYNC_STOP_REASON_NONE,
            peer_last_status_chain_size: HashMap::new(),
        }
    }

    fn refresh_deep_syncing(&mut self) {
        self.deep_syncing = self.active
            && self.target_chain_size.saturating_sub(self.current_period)
                >= self.deep_syncing_threshold;
    }

    fn stop(&mut self, reason: u8) {
        self.active = false;
        self.deep_syncing = false;
        self.peer_id = [0; 64];
        self.stop_count = self.stop_count.saturating_add(1);
        self.last_stop_reason = reason;
        if reason == NETWORK_PBFT_SYNC_STOP_REASON_INACTIVE {
            self.inactivity_count = self.inactivity_count.saturating_add(1);
        }
        if reason == NETWORK_PBFT_SYNC_STOP_REASON_DISCONNECTED {
            self.disconnect_count = self.disconnect_count.saturating_add(1);
        }
    }

    fn snapshot(&self, now_ms: u64) -> NetworkPbftSyncSnapshot {
        NetworkPbftSyncSnapshot {
            active: self.active,
            deep_syncing: self.deep_syncing,
            generation: self.generation,
            has_peer: self.active,
            peer_id: self.peer_id,
            has_last_peer: self.has_last_peer,
            last_peer_id: self.last_peer_id,
            target_chain_size: self.target_chain_size,
            current_period: self.current_period,
            request_period: self.request_period,
            started_at_ms: self.started_at_ms,
            last_activity_ms: self.last_activity_ms,
            elapsed_ms: if self.active {
                now_ms.saturating_sub(self.started_at_ms)
            } else {
                0
            },
            inactive_for_ms: if self.active {
                now_ms.saturating_sub(self.last_activity_ms)
            } else {
                0
            },
            start_count: self.start_count,
            stop_count: self.stop_count,
            inactivity_count: self.inactivity_count,
            disconnect_count: self.disconnect_count,
            last_stop_reason: self.last_stop_reason,
        }
    }
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
    manager: crate::pbft_manager::PbftManagerService,
    storage: Arc<Storage>,
    sync_level_size: u64,
    is_light_node: bool,
    light_node_history: u64,
    node_identity: NetworkNodeIdentity,
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
        manager: crate::pbft_manager::PbftManagerService,
        storage: Arc<Storage>,
        ficus_activation_period: u64,
        pillar_blocks_interval: u64,
        deep_syncing_threshold: u64,
        sync_level_size: u64,
        is_light_node: bool,
        light_node_history: u64,
        node_identity: NetworkNodeIdentity,
    ) -> Result<Self> {
        ensure!(
            ficus_activation_period == u64::MAX || pillar_blocks_interval > 1,
            "PBFT_SERVICE_PILLAR_BLOCKS_INTERVAL_MUST_EXCEED_ONE"
        );
        ensure!(sync_level_size > 0, "PBFT_SERVICE_SYNC_LEVEL_SIZE_ZERO");
        Ok(Self {
            api: Arc::new(Mutex::new(
                ConsensusNetworkApi::with_pillar_schedule_and_sync(
                    ficus_activation_period,
                    pillar_blocks_interval,
                    deep_syncing_threshold,
                ),
            )),
            pillar,
            verified_votes,
            chain,
            proposed_blocks,
            manager,
            storage,
            sync_level_size,
            is_light_node,
            light_node_history,
            node_identity,
        })
    }

    fn lock_api(&self) -> Result<MutexGuard<'_, ConsensusNetworkApi>> {
        Self::lock_shared_api(&self.api)
    }

    fn lock_shared_api(
        api: &Arc<Mutex<ConsensusNetworkApi>>,
    ) -> Result<MutexGuard<'_, ConsensusNetworkApi>> {
        api.lock()
            .map_err(|_| anyhow!("CONSENSUS_NETWORK_SERVICE_LOCK_POISONED"))
    }

    /// Drains at most the fixed native limit of dependency-ready lane effects.
    pub fn drain_work(&self, transport_lane: u32, budget: u32) -> Result<NetworkEffectBatch> {
        Ok(self.lock_api()?.drain_work(transport_lane, budget))
    }

    /// Drains only dependency-ready effects correlated to one source payload.
    ///
    /// Unrelated work in the same transport lane remains queued. This is the
    /// operation-scoped boundary used by specialized synchronous packet
    /// adapters so concurrent protocol families cannot consume each other's
    /// effects.
    pub fn drain_work_for_source(
        &self,
        transport_lane: u32,
        source_payload_id: u64,
        budget: u32,
    ) -> Result<NetworkEffectBatch> {
        Ok(self
            .lock_api()?
            .drain_work_matching(transport_lane, Some(source_payload_id), budget))
    }

    /// Validates and records scalar executor results for previously drained effects.
    pub fn report_effect_results(
        &self,
        results: Vec<NetworkEffectResult>,
    ) -> Result<NetworkEffectAck> {
        Ok(self.lock_api()?.report_effect_results(results))
    }

    /// Atomically selects a serviceable peer and starts one native sync generation.
    pub fn begin_pbft_sync(
        &self,
        request: NetworkPbftSyncStartRequest,
    ) -> Result<NetworkPbftSyncStartOutcome> {
        Ok(self.lock_api()?.begin_pbft_sync(request))
    }

    /// Strictly decodes one legacy status payload, validates immutable initial
    /// identity, and atomically records periodic debounce state with its
    /// follow-up decision. No network lock is held while decoding.
    pub fn ingest_status_packet(
        &self,
        request: NetworkStatusPacketRequest,
    ) -> Result<NetworkStatusPacketReport> {
        let packet = match decode_status_packet(&request.packet_rlp) {
            Ok(packet) => packet,
            Err(_) => return Ok(malformed_status_packet_report()),
        };
        let Some(peer_pbft_period) = packet.peer_pbft_chain_size.checked_add(1) else {
            return Ok(malformed_status_packet_report());
        };
        let mut report = NetworkStatusPacketReport {
            status: NETWORK_STATUS_PLAN_STATUS_OK,
            error_code: ERROR_NONE.to_owned(),
            malicious: false,
            initial: packet.initial_data.is_some(),
            accept_peer: true,
            disconnect_peer: false,
            peer_pbft_chain_size: packet.peer_pbft_chain_size,
            peer_pbft_period,
            peer_pbft_round: packet.peer_pbft_round,
            peer_dag_level: packet.peer_dag_level,
            peer_syncing: packet.peer_syncing,
            peer_is_light_node: false,
            peer_light_node_history: 0,
            node_major_version: 0,
            node_minor_version: 0,
            node_patch_version: 0,
            request_pbft_sync: false,
            request_pending_dag_blocks: false,
            request_next_votes: false,
            next_votes_period: 0,
            next_votes_round: 0,
            next_votes_request_rlp: Vec::new(),
            sync_generation: 0,
        };
        if let Some(initial) = packet.initial_data {
            report.peer_is_light_node = initial.is_light_node;
            report.peer_light_node_history = initial.light_node_history;
            report.node_major_version = initial.node_major_version;
            report.node_minor_version = initial.node_minor_version;
            report.node_patch_version = initial.node_patch_version;
            let plan = plan_initial_status(NetworkInitialStatusFacts {
                local_chain_id: self.node_identity.chain_id,
                peer_chain_id: initial.chain_id,
                expected_genesis_hash: self.node_identity.genesis_hash,
                peer_genesis_hash: initial.genesis_hash,
                local_pbft_synced_period: request.local_pbft_synced_period,
                peer_pbft_chain_size: packet.peer_pbft_chain_size,
                peer_is_light_node: initial.is_light_node,
                peer_light_node_history: initial.light_node_history,
            });
            report.status = plan.status;
            report.error_code = plan.error_code;
            report.accept_peer = plan.accept_peer;
            report.disconnect_peer = plan.disconnect_peer;
            return Ok(report);
        }

        if !request.source_peer_ready {
            report.status = NETWORK_STATUS_PLAN_STATUS_PENDING_PEER_PERIODIC;
            report.error_code = "NETWORK_STATUS_PERIODIC_FROM_PENDING_PEER".to_owned();
            report.accept_peer = false;
            report.disconnect_peer = true;
            return Ok(report);
        }

        let followup = self
            .lock_api()?
            .process_status_followup(NetworkStatusFollowupRequest {
                peer_id: request.peer_id,
                local_pbft_synced_period: request.local_pbft_synced_period,
                local_pbft_period: request.local_pbft_period,
                local_pbft_round: request.local_pbft_round,
                peer_pbft_chain_size: packet.peer_pbft_chain_size,
                peer_pbft_period,
                peer_pbft_round: packet.peer_pbft_round,
                peer_dag_synced: request.peer_dag_synced,
            });
        report.request_pbft_sync = followup.request_pbft_sync;
        report.request_pending_dag_blocks = followup.request_pending_dag_blocks;
        report.request_next_votes = followup.request_next_votes;
        report.next_votes_period = followup.next_votes_period;
        report.next_votes_round = followup.next_votes_round;
        report.next_votes_request_rlp = if followup.request_next_votes {
            encode_get_next_votes_packet(followup.next_votes_period, followup.next_votes_round)
        } else {
            Vec::new()
        };
        report.sync_generation = followup.sync_generation;
        Ok(report)
    }

    /// Builds one canonical legacy status payload using bootstrap-owned node
    /// identity and the lock-coherent native sync mode.
    pub fn build_status_packet(
        &self,
        request: NetworkStatusPacketBuildRequest,
    ) -> Result<NetworkStatusPacketBuildOutcome> {
        let api = self.lock_api()?;
        let syncing = if request.initial {
            api.pbft_sync.active
        } else {
            api.pbft_sync.deep_syncing
        };
        Ok(NetworkStatusPacketBuildOutcome {
            status: NETWORK_STATUS_PLAN_STATUS_OK,
            error_code: ERROR_NONE.to_owned(),
            packet_rlp: encode_status_packet(
                request,
                syncing,
                request.initial.then_some(&self.node_identity),
            ),
        })
    }

    /// Correlates one response peer with the active or most-recent sync generation.
    pub fn admit_pbft_sync_source(
        &self,
        request: NetworkPbftSyncSourceRequest,
    ) -> Result<NetworkPbftSyncSourceOutcome> {
        Ok(self.lock_api()?.admit_pbft_sync_source(request))
    }

    /// Records progress for an exact native PBFT-sync generation.
    pub fn record_pbft_sync_activity(
        &self,
        request: NetworkPbftSyncActivityRequest,
    ) -> Result<NetworkPbftSyncActivityOutcome> {
        Ok(self.lock_api()?.record_pbft_sync_activity(request))
    }

    /// Stops an exact native PBFT-sync generation.
    pub fn stop_pbft_sync(
        &self,
        request: NetworkPbftSyncStopRequest,
    ) -> Result<NetworkPbftSyncStopOutcome> {
        Ok(self.lock_api()?.stop_pbft_sync(request))
    }

    /// Applies selected-peer disconnect recovery to an exact sync generation.
    pub fn handle_pbft_sync_disconnect(
        &self,
        request: NetworkPbftSyncDisconnectRequest,
    ) -> Result<NetworkPbftSyncDisconnectOutcome> {
        Ok(self.lock_api()?.handle_pbft_sync_disconnect(request))
    }

    /// Applies the inactivity policy for an exact timer-observed generation.
    pub fn tick_pbft_sync(
        &self,
        request: NetworkPbftSyncTickRequest,
    ) -> Result<NetworkPbftSyncTickOutcome> {
        Ok(self.lock_api()?.tick_pbft_sync(request))
    }

    /// Applies one generation-correlated lifecycle command under the service lock.
    pub fn apply_pbft_sync_command(
        &self,
        request: NetworkPbftSyncCommandRequest,
    ) -> Result<NetworkPbftSyncCommandOutcome> {
        self.lock_api()?.apply_pbft_sync_command(request)
    }
}

impl ConsensusNetworkApi {
    fn apply_pbft_sync_command(
        &mut self,
        request: NetworkPbftSyncCommandRequest,
    ) -> Result<NetworkPbftSyncCommandOutcome> {
        let api = self;
        let mut outcome = match request.kind {
            0 => {
                let outcome = api.admit_pbft_sync_source(NetworkPbftSyncSourceRequest {
                    peer_id: request.peer_id,
                    source: request.source,
                });
                Ok(NetworkPbftSyncCommandOutcome {
                    accepted: outcome.accepted,
                    active: outcome.active,
                    stopped: false,
                    expired: false,
                    restart_sync: false,
                    retry: false,
                    request_next: false,
                    request_pending_dag_if_idle: false,
                    deep_syncing: false,
                    generation: outcome.generation,
                    error_code: outcome.error_code,
                })
            }
            1 => {
                let outcome = api.record_pbft_sync_activity(NetworkPbftSyncActivityRequest {
                    now_ms: request.now_ms,
                    generation: request.generation,
                    peer_id: request.peer_id,
                });
                Ok(NetworkPbftSyncCommandOutcome {
                    accepted: outcome.accepted,
                    active: outcome.accepted,
                    stopped: false,
                    expired: false,
                    restart_sync: false,
                    retry: false,
                    request_next: false,
                    request_pending_dag_if_idle: false,
                    deep_syncing: outcome.deep_syncing,
                    generation: outcome.generation,
                    error_code: outcome.error_code,
                })
            }
            2 => {
                let outcome = api.stop_pbft_sync(NetworkPbftSyncStopRequest {
                    generation: request.generation,
                    peer_id: request.peer_id,
                    reason: request.reason,
                });
                Ok(NetworkPbftSyncCommandOutcome {
                    accepted: outcome.stopped,
                    active: false,
                    stopped: outcome.stopped,
                    expired: false,
                    restart_sync: false,
                    retry: false,
                    request_next: false,
                    request_pending_dag_if_idle: false,
                    deep_syncing: false,
                    generation: outcome.generation,
                    error_code: outcome.error_code,
                })
            }
            3 => {
                let outcome = api.handle_pbft_sync_disconnect(NetworkPbftSyncDisconnectRequest {
                    generation: request.generation,
                    peer_id: request.peer_id,
                });
                Ok(NetworkPbftSyncCommandOutcome {
                    accepted: outcome.stopped,
                    active: false,
                    stopped: outcome.stopped,
                    expired: false,
                    restart_sync: outcome.restart_sync,
                    retry: false,
                    request_next: false,
                    request_pending_dag_if_idle: false,
                    deep_syncing: false,
                    generation: outcome.generation,
                    error_code: outcome.error_code,
                })
            }
            4 => {
                let outcome = api.tick_pbft_sync(NetworkPbftSyncTickRequest {
                    now_ms: request.now_ms,
                    generation: request.generation,
                });
                Ok(NetworkPbftSyncCommandOutcome {
                    accepted: outcome.expired,
                    active: !outcome.expired,
                    stopped: outcome.expired,
                    expired: outcome.expired,
                    restart_sync: outcome.restart_sync,
                    retry: false,
                    request_next: false,
                    request_pending_dag_if_idle: false,
                    deep_syncing: false,
                    generation: outcome.generation,
                    error_code: outcome.error_code,
                })
            }
            5 => {
                if !api.pbft_sync.active
                    || api.pbft_sync.generation != request.generation
                    || api.pbft_sync.peer_id != request.peer_id
                {
                    Ok(NetworkPbftSyncCommandOutcome {
                        accepted: false,
                        active: api.pbft_sync.active,
                        stopped: false,
                        expired: false,
                        restart_sync: false,
                        retry: false,
                        request_next: false,
                        request_pending_dag_if_idle: false,
                        deep_syncing: api.pbft_sync.deep_syncing,
                        generation: api.pbft_sync.generation,
                        error_code: "NETWORK_PBFT_SYNC_STALE_COMPLETION".to_owned(),
                    })
                } else if request.sync_queue_size != 0 {
                    Ok(NetworkPbftSyncCommandOutcome {
                        accepted: true,
                        active: true,
                        stopped: false,
                        expired: false,
                        restart_sync: false,
                        retry: true,
                        request_next: false,
                        request_pending_dag_if_idle: false,
                        deep_syncing: api.pbft_sync.deep_syncing,
                        generation: api.pbft_sync.generation,
                        error_code: ERROR_NONE.to_owned(),
                    })
                } else {
                    let stopped = api.stop_pbft_sync(NetworkPbftSyncStopRequest {
                        generation: request.generation,
                        peer_id: request.peer_id,
                        reason: NETWORK_PBFT_SYNC_STOP_REASON_COMPLETED,
                    });
                    Ok(NetworkPbftSyncCommandOutcome {
                        accepted: stopped.stopped,
                        active: false,
                        stopped: stopped.stopped,
                        expired: false,
                        restart_sync: stopped.stopped,
                        retry: false,
                        request_next: false,
                        request_pending_dag_if_idle: stopped.stopped,
                        deep_syncing: false,
                        generation: stopped.generation,
                        error_code: stopped.error_code,
                    })
                }
            }
            6 | 7 => {
                if !api.pbft_sync.active
                    || api.pbft_sync.generation != request.generation
                    || api.pbft_sync.peer_id != request.peer_id
                {
                    Ok(NetworkPbftSyncCommandOutcome {
                        accepted: false,
                        active: api.pbft_sync.active,
                        stopped: false,
                        expired: false,
                        restart_sync: false,
                        retry: false,
                        request_next: false,
                        request_pending_dag_if_idle: false,
                        deep_syncing: api.pbft_sync.deep_syncing,
                        generation: api.pbft_sync.generation,
                        error_code: "NETWORK_PBFT_SYNC_STALE_CONTINUATION".to_owned(),
                    })
                } else if request.kind == 7
                    && (request.retry_delay_ms == 0
                        || u64::from(request.retry_count)
                            > NETWORK_PBFT_SYNC_INACTIVITY_THRESHOLD_MS / request.retry_delay_ms)
                {
                    let stopped = api.stop_pbft_sync(NetworkPbftSyncStopRequest {
                        generation: request.generation,
                        peer_id: request.peer_id,
                        reason: NETWORK_PBFT_SYNC_STOP_REASON_TRANSPORT_FAILED,
                    });
                    Ok(NetworkPbftSyncCommandOutcome {
                        accepted: stopped.stopped,
                        active: false,
                        stopped: stopped.stopped,
                        expired: false,
                        restart_sync: false,
                        retry: false,
                        request_next: false,
                        request_pending_dag_if_idle: false,
                        deep_syncing: false,
                        generation: stopped.generation,
                        error_code: stopped.error_code,
                    })
                } else if request.kind == 6 && request.syncing_period > request.remote_period {
                    let stopped = api.stop_pbft_sync(NetworkPbftSyncStopRequest {
                        generation: request.generation,
                        peer_id: request.peer_id,
                        reason: NETWORK_PBFT_SYNC_STOP_REASON_COMPLETED,
                    });
                    Ok(NetworkPbftSyncCommandOutcome {
                        accepted: stopped.stopped,
                        active: false,
                        stopped: stopped.stopped,
                        expired: false,
                        restart_sync: false,
                        retry: false,
                        request_next: false,
                        request_pending_dag_if_idle: false,
                        deep_syncing: false,
                        generation: stopped.generation,
                        error_code: stopped.error_code,
                    })
                } else {
                    let wait_threshold = request
                        .finalized_period
                        .saturating_add(10_u64.saturating_mul(request.sync_level_size));
                    let retry = request.syncing_period > wait_threshold;
                    Ok(NetworkPbftSyncCommandOutcome {
                        accepted: true,
                        active: true,
                        stopped: false,
                        expired: false,
                        restart_sync: false,
                        retry,
                        request_next: !retry,
                        request_pending_dag_if_idle: false,
                        deep_syncing: api.pbft_sync.deep_syncing,
                        generation: api.pbft_sync.generation,
                        error_code: ERROR_NONE.to_owned(),
                    })
                }
            }
            kind => Err(anyhow!("unknown PBFT-sync lifecycle command kind {kind}")),
        }?;
        let snapshot = api.pbft_sync_status(request.now_ms);
        outcome.active = snapshot.active;
        outcome.deep_syncing = snapshot.deep_syncing;
        outcome.generation = snapshot.generation;
        Ok(outcome)
    }
}

impl ConsensusNetworkService {
    /// Updates native PBFT sync progress and recomputes deep-sync state.
    pub fn update_pbft_sync_period(&self, current_period: u64) -> Result<NetworkPbftSyncSnapshot> {
        Ok(self.lock_api()?.update_pbft_sync_period(current_period))
    }

    /// Returns a side-effect-free snapshot for query, statistics, and egress readers.
    pub fn pbft_sync_status(&self, now_ms: u64) -> Result<NetworkPbftSyncSnapshot> {
        Ok(self.lock_api()?.pbft_sync_status(now_ms))
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

    /// Routes and authoritatively admits one PBFT vote through root siblings.
    ///
    /// The network lock is released before PBFT validation, FinalChain reads,
    /// or persistence. On return no vote-admission application effect remains:
    /// Rust has converted the admission result directly into ordered transport
    /// follow-ups. A typed slashing transaction remains in the admission value
    /// for the external signing/insertion leaf.
    pub fn ingest_and_admit_pbft_vote(
        &self,
        pbft: &crate::PbftService,
        final_chain: &FinalChain,
        fact: PbftVoteIngressFact,
        context: NetworkPbftVoteIngressContext,
        slashing_submitters: &[crate::SlashingSubmitterIdentity],
    ) -> Result<NetworkPbftVoteAdmissionOutcome> {
        let mut decision = self.lock_api()?.ingest_pbft_vote(fact, context);
        let effect_id = decision.application_effect_id;
        if effect_id == 0 {
            return Ok(NetworkPbftVoteAdmissionOutcome {
                decision,
                admission: None,
            });
        }
        let Some(pending) = self.lock_api()?.take_native_vote_admission(effect_id) else {
            return Err(anyhow!("NETWORK_NATIVE_VOTE_ADMISSION_EFFECT_MISSING"));
        };
        let admission =
            pbft.admit_network_verified_vote(final_chain, &pending.vote_rlp, slashing_submitters)?;
        let add = admission.transaction.outcome.add_outcome.as_ref();
        if !pending.pbft_block_rlp.is_empty()
            && admission.transaction.transition_published
            && add.is_some_and(|outcome| outcome.inserted || outcome.duplicate_vote_hash)
        {
            pbft.publish_proposed_block_effect(pending.pbft_block_rlp.clone())?;
        }
        let _ = self
            .lock_api()?
            .complete_native_vote_admission(pending, &admission);
        decision.application_effect_id = 0;
        Ok(NetworkPbftVoteAdmissionOutcome {
            decision,
            admission: Some(admission),
        })
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

    /// Atomically preflights and sequentially admits one PBFT vote bundle.
    ///
    /// Bundle shape remains all-or-nothing. After preflight, each member is
    /// admitted without holding the network lock. A slashing conflict cancels
    /// remaining queued members; already published members are not rolled back.
    pub fn ingest_and_admit_pbft_vote_bundle(
        &self,
        pbft: &crate::PbftService,
        final_chain: &FinalChain,
        reference: PbftVoteIngressFact,
        votes: Vec<PbftVoteIngressFact>,
        contexts: Vec<NetworkPbftVoteIngressContext>,
        slashing_submitters: &[crate::SlashingSubmitterIdentity],
    ) -> Result<Vec<NetworkPbftVoteAdmissionOutcome>> {
        Self::ingest_and_admit_pbft_vote_bundle_with(
            &self.api,
            reference,
            votes,
            contexts,
            |vote_rlp| pbft.admit_network_verified_vote(final_chain, vote_rlp, slashing_submitters),
        )
    }

    /// Decodes and authoritatively admits one complete canonical PBFT vote packet.
    ///
    /// The operation owns the outer packet shape, canonical vote inspection,
    /// optional proposed-block correlation, ingress planning, admission, and
    /// follow-up queueing. Only the optional peer chain-size observation is
    /// returned for network-owned peer bookkeeping.
    pub fn ingest_pbft_vote_packet(
        &self,
        pbft: &crate::PbftService,
        final_chain: &FinalChain,
        request: NetworkConsensusPacketRequest,
        slashing_submitters: &[crate::SlashingSubmitterIdentity],
    ) -> Result<NetworkPbftVotePacketReport> {
        let decoded = decode_pbft_vote_packet(&request.packet_rlp)
            .context("NETWORK_PBFT_VOTE_PACKET_MALFORMED")?;
        let inspection = inspect_canonical_pbft_vote(&decoded.vote_rlp)
            .context("NETWORK_PBFT_VOTE_PACKET_MALFORMED_VOTE")?;
        ensure!(
            inspection.signature_valid,
            "NETWORK_PBFT_VOTE_PACKET_INVALID_SIGNATURE"
        );
        let fact = pbft_vote_ingress_fact(&inspection);
        let mut context = pbft_vote_packet_context(&request, &inspection, decoded.vote_rlp);
        context.peer_pbft_chain_size = context
            .peer_pbft_chain_size
            .max(decoded.peer_pbft_chain_size.unwrap_or_default());
        if let Some(block_rlp) = decoded.pbft_block_rlp {
            let (link, metadata) = decode_pbft_blocks_bundle_member(&block_rlp)
                .context("NETWORK_PBFT_VOTE_PACKET_INVALID_PROPOSED_BLOCK")?;
            ensure!(
                link.block_hash == inspection.block_hash
                    && link.period == inspection.period
                    && metadata.period == inspection.period,
                "NETWORK_PBFT_VOTE_PACKET_PROPOSED_BLOCK_MISMATCH"
            );
            context.pbft_block_hash = link.block_hash.to_fixed_bytes();
            context.pbft_block_period = link.period;
            context.pbft_block_rlp = block_rlp;
        }
        let outcome =
            self.ingest_and_admit_pbft_vote(pbft, final_chain, fact, context, slashing_submitters)?;
        Ok(NetworkPbftVotePacketReport {
            outcomes: vec![outcome],
            has_peer_pbft_chain_size: decoded.peer_pbft_chain_size.is_some(),
            peer_pbft_chain_size: decoded.peer_pbft_chain_size.unwrap_or_default(),
            egress_payload_bytes: Vec::new(),
        })
    }

    /// Decodes, atomically preflights, and sequentially admits one complete
    /// canonical optimized PBFT votes-bundle packet.
    pub fn ingest_pbft_votes_bundle_packet(
        &self,
        pbft: &crate::PbftService,
        final_chain: &FinalChain,
        request: NetworkConsensusPacketRequest,
        slashing_submitters: &[crate::SlashingSubmitterIdentity],
    ) -> Result<NetworkPbftVotePacketReport> {
        let vote_rlps = decode_pbft_votes_bundle_packet(&request.packet_rlp)
            .context("NETWORK_PBFT_VOTES_BUNDLE_PACKET_MALFORMED")?;
        let inspected = vote_rlps
            .iter()
            .map(|vote_rlp| inspect_canonical_pbft_vote(vote_rlp))
            .collect::<Result<Vec<_>>>()
            .context("NETWORK_PBFT_VOTES_BUNDLE_MEMBER_MALFORMED")?;
        ensure!(
            inspected.iter().all(|vote| vote.signature_valid),
            "NETWORK_PBFT_VOTES_BUNDLE_INVALID_SIGNATURE"
        );
        let facts = inspected
            .iter()
            .map(pbft_vote_ingress_fact)
            .collect::<Vec<_>>();
        let reference = *facts
            .first()
            .ok_or_else(|| anyhow!("NETWORK_PBFT_VOTES_BUNDLE_EMPTY"))?;
        let validate_max_round_step = !matches!(
            reference.vote_type,
            crate::verified_votes::PbftVoteType::Cert | crate::verified_votes::PbftVoteType::Next
        );
        let contexts = inspected
            .iter()
            .zip(vote_rlps.iter().cloned())
            .map(|(inspection, vote_rlp)| {
                let mut context = pbft_vote_packet_context(&request, inspection, vote_rlp);
                context.ingress.validate_max_round_step = validate_max_round_step;
                context
            })
            .collect();
        let outcomes = self.ingest_and_admit_pbft_vote_bundle(
            pbft,
            final_chain,
            reference,
            facts,
            contexts,
            slashing_submitters,
        )?;
        let accepted_votes = outcomes
            .iter()
            .zip(inspected.iter().zip(vote_rlps))
            .filter_map(|(outcome, (inspection, vote_rlp))| {
                outcome
                    .admission
                    .as_ref()
                    .filter(|admission| {
                        admission.transaction.transition_published
                            && admission
                                .transaction
                                .outcome
                                .execution
                                .as_ref()
                                .is_some_and(|execution| {
                                    execution.pipeline_step.progress_plan.intents.iter().any(
                                        |intent| {
                                            matches!(
                                                intent,
                                                PbftVoteProgressIntent::GossipVote { .. }
                                            )
                                        },
                                    )
                                })
                    })
                    .map(|_| PbftVotePayloadRecord {
                        hash: inspection.vote_hash,
                        vote_rlp,
                    })
            })
            .collect::<Vec<_>>();
        let egress_payload_bytes = if accepted_votes.is_empty() {
            Vec::new()
        } else {
            build_optimized_pbft_vote_bundle(
                &accepted_votes,
                inspected[0].block_hash,
                inspected[0].period,
                inspected[0].round,
                inspected[0].step,
            )?
            .bundle_rlp
        };
        Ok(NetworkPbftVotePacketReport {
            outcomes,
            has_peer_pbft_chain_size: false,
            peer_pbft_chain_size: 0,
            egress_payload_bytes,
        })
    }

    /// Runs composed bundle routing with an injected authoritative admission task.
    ///
    /// The injection is internal testability for infrastructure failures. Any
    /// admission error cancels every not-yet-admitted member before it escapes,
    /// so no obsolete application effect can later be acknowledged as success.
    fn ingest_and_admit_pbft_vote_bundle_with(
        api: &Arc<Mutex<ConsensusNetworkApi>>,
        reference: PbftVoteIngressFact,
        votes: Vec<PbftVoteIngressFact>,
        contexts: Vec<NetworkPbftVoteIngressContext>,
        mut admit: impl FnMut(&[u8]) -> Result<crate::PbftVoteAdmissionWithSlashingResult>,
    ) -> Result<Vec<NetworkPbftVoteAdmissionOutcome>> {
        let decisions =
            Self::lock_shared_api(api)?.ingest_pbft_vote_bundle(reference, votes, contexts);
        let mut outcomes = Vec::with_capacity(decisions.len());
        let mut follow_up_effect_ids = Vec::new();
        for mut decision in decisions {
            let effect_id = decision.application_effect_id;
            if effect_id == 0 {
                outcomes.push(NetworkPbftVoteAdmissionOutcome {
                    decision,
                    admission: None,
                });
                continue;
            }
            let pending = Self::lock_shared_api(api)?.take_native_vote_admission(effect_id);
            let Some(pending) = pending else {
                decision.application_effect_id = 0;
                outcomes.push(NetworkPbftVoteAdmissionOutcome {
                    decision,
                    admission: None,
                });
                continue;
            };
            let bundle_id = pending.bundle.as_ref().map(|member| member.bundle_id);
            let admission = match admit(&pending.vote_rlp) {
                Ok(admission) => admission,
                Err(error) => {
                    if let Some(bundle_id) = bundle_id {
                        Self::lock_shared_api(api)?.cancel_vote_bundle(bundle_id);
                    }
                    Self::lock_shared_api(api)?.cancel_pending_effects(&follow_up_effect_ids);
                    return Err(error);
                }
            };
            follow_up_effect_ids.extend(
                Self::lock_shared_api(api)?.complete_native_vote_admission(pending, &admission),
            );
            decision.application_effect_id = 0;
            outcomes.push(NetworkPbftVoteAdmissionOutcome {
                decision,
                admission: Some(admission),
            });
        }
        Ok(outcomes)
    }

    /// Atomically preflights one pillar-vote bundle before queueing admissions.
    pub fn ingest_pillar_vote_bundle(
        &self,
        context: NetworkPillarVoteIngressContext,
        votes: Vec<Vec<u8>>,
    ) -> Result<Vec<NetworkIngressDecision>> {
        Ok(self.lock_api()?.ingest_pillar_vote_bundle(context, votes))
    }

    /// Atomically preflights and sequentially admits one pillar-vote packet.
    ///
    /// The network owner removes each private admission effect before calling
    /// the root-owned pillar/FinalChain task, so canonical vote bytes never
    /// cross CXX merely to re-enter Rust. Native admission runs without the
    /// network lock. Accepted votes then queue only physical peer-known and
    /// gossip leaves; rejected or duplicate votes queue no transport work.
    pub fn ingest_and_admit_pillar_vote_bundle(
        &self,
        pbft: &crate::PbftService,
        final_chain: &FinalChain,
        context: NetworkPillarVoteIngressContext,
        votes: Vec<Vec<u8>>,
    ) -> Result<Vec<NetworkPillarVoteAdmissionOutcome>> {
        let (first_pillar_block_period, pillar_blocks_interval) = {
            let api = self.lock_api()?;
            let first = if api.ficus_activation_period == 0 {
                api.pillar_blocks_interval
            } else {
                api.ficus_activation_period
            };
            (first, api.pillar_blocks_interval)
        };
        let admission_context = PillarVoteSingleAdmissionContext {
            first_pillar_block_period,
            pillar_blocks_interval,
        };
        Self::ingest_and_admit_pillar_vote_bundle_with(&self.api, context, votes, |vote_rlp| {
            pbft.apply_single_pillar_vote_with_final_chain(
                final_chain,
                vote_rlp.to_vec(),
                admission_context,
                false,
            )
        })
    }

    /// Decodes and admits one complete canonical pillar-vote packet.
    pub fn ingest_pillar_vote_packet(
        &self,
        pbft: &crate::PbftService,
        final_chain: &FinalChain,
        request: NetworkConsensusPacketRequest,
    ) -> Result<NetworkPillarVotePacketReport> {
        let vote_rlp = decode_single_wrapped_packet(&request.packet_rlp)
            .context("NETWORK_PILLAR_VOTE_PACKET_MALFORMED")?;
        let outcomes = self.ingest_and_admit_pillar_vote_bundle(
            pbft,
            final_chain,
            pillar_vote_packet_context(&request),
            vec![vote_rlp],
        )?;
        Ok(NetworkPillarVotePacketReport { outcomes })
    }

    /// Decodes and admits one complete canonical optimized pillar-votes bundle packet.
    pub fn ingest_pillar_votes_bundle_packet(
        &self,
        pbft: &crate::PbftService,
        final_chain: &FinalChain,
        request: NetworkConsensusPacketRequest,
    ) -> Result<NetworkPillarVotePacketReport> {
        let bundle_rlp = decode_single_wrapped_packet(&request.packet_rlp)
            .context("NETWORK_PILLAR_VOTES_BUNDLE_PACKET_MALFORMED")?;
        let votes = decode_optimized_pillar_votes_bundle_rlp(&bundle_rlp)
            .context("NETWORK_PILLAR_VOTES_BUNDLE_PACKET_MALFORMED")?
            .into_iter()
            .map(|vote| vote.encode_rlp())
            .collect::<Vec<_>>();
        let outcomes = self.ingest_and_admit_pillar_vote_bundle(
            pbft,
            final_chain,
            pillar_vote_packet_context(&request),
            votes,
        )?;
        Ok(NetworkPillarVotePacketReport { outcomes })
    }

    /// Runs pillar bundle routing with an injected authoritative admission task.
    ///
    /// The injection exists for infrastructure-failure coverage. On any
    /// failure, every not-yet-admitted member is removed from both the effect
    /// queue and its private context registry before the error escapes.
    fn ingest_and_admit_pillar_vote_bundle_with(
        api: &Arc<Mutex<ConsensusNetworkApi>>,
        context: NetworkPillarVoteIngressContext,
        votes: Vec<Vec<u8>>,
        mut admit: impl FnMut(&[u8]) -> Result<PillarVoteSingleAdmissionWithFinalChainPlan>,
    ) -> Result<Vec<NetworkPillarVoteAdmissionOutcome>> {
        let decisions = Self::lock_shared_api(api)?.ingest_pillar_vote_bundle(context, votes);
        let effect_ids = decisions
            .iter()
            .map(|decision| decision.application_effect_id)
            .collect::<Vec<_>>();
        let mut admitted = Vec::with_capacity(decisions.len());
        let mut follow_up_effect_ids = Vec::new();
        for (index, mut decision) in decisions.into_iter().enumerate() {
            let effect_id = decision.application_effect_id;
            if effect_id == 0 {
                admitted.push(NetworkPillarVoteAdmissionOutcome {
                    decision,
                    admission: None,
                });
                continue;
            }
            let pending = Self::lock_shared_api(api)?.take_native_pillar_vote_admission(effect_id);
            let Some(pending) = pending else {
                Self::lock_shared_api(api)?.cancel_pillar_vote_admissions(&effect_ids[index + 1..]);
                Self::lock_shared_api(api)?.cancel_effects(&follow_up_effect_ids);
                return Err(anyhow!(
                    "NETWORK_NATIVE_PILLAR_VOTE_ADMISSION_EFFECT_MISSING"
                ));
            };
            let result = match admit(&pending.vote_rlp) {
                Ok(result) => result,
                Err(error) => {
                    let mut api = Self::lock_shared_api(api)?;
                    api.cancel_pillar_vote_admissions(&effect_ids[index + 1..]);
                    api.cancel_effects(&follow_up_effect_ids);
                    return Err(error);
                }
            };
            follow_up_effect_ids.extend(
                Self::lock_shared_api(api)?.complete_native_pillar_vote_admission(pending, &result),
            );
            decision.application_effect_id = 0;
            admitted.push(NetworkPillarVoteAdmissionOutcome {
                decision,
                admission: Some(result),
            });
        }
        Ok(admitted)
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
        let manager_snapshot = self.manager.lock().state.snapshot();
        let current_period = manager_snapshot.period;
        let current_round = manager_snapshot.round;
        if let Some(decision) =
            next_votes_request_rejection(&request, current_period, current_round)
        {
            return Ok(decision);
        }
        let period = current_period;
        let round = current_round - 1;
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
        Ok(self.lock_api()?.enqueue_next_votes_bundle_send_effects(
            request,
            current_period,
            current_round,
            chunks,
        ))
    }

    /// Strictly decodes and serves one canonical get-next-votes request.
    /// Malformed peer bytes produce a typed zero-effect decision.
    pub fn ingest_pbft_next_votes_bundle_packet_request(
        &self,
        request: NetworkPbftNextVotesBundlePacketRequest,
    ) -> Result<NetworkIngressDecision> {
        let (peer_period, peer_round) = match decode_get_next_votes_packet(&request.packet_rlp) {
            Ok(decoded) => decoded,
            Err(_) => {
                return Ok(local_network_decision(
                    request.source_payload_id,
                    NETWORK_INGRESS_STATUS_MALFORMED_PACKET,
                    "NETWORK_GET_NEXT_VOTES_MALFORMED_RLP",
                ));
            }
        };
        self.ingest_pbft_next_votes_bundle_request(NetworkPbftNextVotesBundleRequest {
            transport_lane: request.transport_lane,
            peer_id: request.peer_id,
            peer_period,
            peer_round,
            source_payload_id: request.source_payload_id,
        })
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
        request: NetworkGetPillarVotesBundlePacketRequest,
    ) -> Result<NetworkIngressDecision> {
        let query = match decode_get_pillar_votes_bundle_packet(&request.packet_rlp) {
            Ok(query) => query,
            Err(_) => {
                return Ok(local_network_decision(
                    request.source_payload_id,
                    NETWORK_INGRESS_STATUS_MALFORMED_PACKET,
                    "NETWORK_GET_PILLAR_VOTES_BUNDLE_MALFORMED_RLP",
                ));
            }
        };
        {
            let mut api = self.lock_api()?;
            if let Some(decision) = api.reject_invalid_pillar_votes_request(&request, &query) {
                return Ok(decision);
            }
        }

        let records = match self.pillar.pbft_service_pillar_get_verified_vote_payloads(
            query.period,
            &query.pillar_block_hash,
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
        let chunks =
            match validate_and_chunk_pillar_votes(records, query.period, query.pillar_block_hash) {
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
            .enqueue_pillar_vote_bundle_send_effects(request, query, chunks))
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

    /// Admits one latest-tarcap proposed-PBFT-block bundle directly into native state.
    ///
    /// The input is the canonical outer packet RLP. Rust decodes signed blocks,
    /// samples the native manager period once, ignores blocks outside the legacy
    /// `[period, period + 5]` window, enforces one recovered author per relevant
    /// period, queries the native FinalChain DPoS view when that view covers the
    /// prior period, and publishes accepted blocks through the native proposal
    /// owner. Publication is intentionally sequential: if a later member is
    /// malicious, earlier accepted members remain published as in the reference
    /// handler. Malformed, oversized, duplicate-author, and ineligible bundles
    /// return typed peer-fault decisions; storage and lock failures are errors.
    pub fn ingest_pbft_blocks_bundle(
        &self,
        final_chain: &FinalChain,
        packet_rlp: &[u8],
        source_payload_id: u64,
    ) -> Result<NetworkIngressDecision> {
        let current_period = self.manager.snapshot().period;
        let final_chain_head = final_chain.last_block_number()?;
        admit_pbft_blocks_bundle(
            packet_rlp,
            source_payload_id,
            current_period,
            final_chain_head,
            |period, author| match final_chain.pbft_dpos_eligible_vote_count(period, author) {
                Ok(Some(count)) => Ok(count > 0),
                Ok(None) => Ok(false),
                Err(error) => Err(error),
            },
            |link, block_rlp| {
                self.proposed_blocks.push_with_storage(
                    link.period,
                    link.block_hash,
                    link.pivot_dag_block_hash,
                    block_rlp,
                )
            },
        )
    }

    /// Decodes and admits one canonical transaction packet sequentially.
    ///
    /// The network owner enforces the legacy packet limits and preserves member
    /// order. `admit` is the application-root operation that owns transaction
    /// verification, external account facts, and queue mutation. Accepted new
    /// transactions queue known/gossip effects; deterministic rejection stops
    /// the packet and returns typed peer-fault data without a bridge exception.
    pub fn ingest_transaction_packet<F>(
        &self,
        context: NetworkTransactionPacketContext,
        packet_rlp: &[u8],
        mut admit: F,
    ) -> Result<NetworkTransactionPacketReport>
    where
        F: FnMut(Vec<u8>) -> Result<TransactionPacketIngressReport>,
    {
        let (transactions, extra_transaction_hashes) = match decode_transaction_packet(packet_rlp) {
            Ok(value) => value,
            Err(TransactionPacketDecodeError::Malformed) => {
                return Ok(NetworkTransactionPacketReport {
                    decision: local_network_decision(
                        context.source_payload_id,
                        NETWORK_INGRESS_STATUS_TRANSACTION_PACKET_MALFORMED,
                        ERROR_TRANSACTION_PACKET_MALFORMED,
                    ),
                    transactions: Vec::new(),
                    extra_transaction_hashes: Vec::new(),
                });
            }
            Err(TransactionPacketDecodeError::TooLarge) => {
                return Ok(NetworkTransactionPacketReport {
                    decision: local_network_decision(
                        context.source_payload_id,
                        NETWORK_INGRESS_STATUS_TRANSACTION_PACKET_TOO_LARGE,
                        ERROR_TRANSACTION_PACKET_TOO_LARGE,
                    ),
                    transactions: Vec::new(),
                    extra_transaction_hashes: Vec::new(),
                });
            }
        };
        let mut reports = Vec::with_capacity(transactions.len());
        for transaction_rlp in transactions {
            let report = admit(transaction_rlp)?;
            let accepted = transaction_packet_member_is_benign(&report);
            reports.push(report);
            if !accepted {
                return Ok(NetworkTransactionPacketReport {
                    decision: local_network_decision(
                        context.source_payload_id,
                        NETWORK_INGRESS_STATUS_TRANSACTION_REJECTED,
                        ERROR_TRANSACTION_PACKET_REJECTED,
                    ),
                    transactions: reports,
                    extra_transaction_hashes,
                });
            }
        }
        let decision = self.lock_api()?.enqueue_transaction_packet_effects(
            &context,
            &reports,
            &extra_transaction_hashes,
        );
        Ok(NetworkTransactionPacketReport {
            decision,
            transactions: reports,
            extra_transaction_hashes,
        })
    }

    /// Serves one canonical get-DAG-sync request from a native DAG snapshot.
    ///
    /// Rust decodes and deduplicates requested hashes, asks the application
    /// owner for canonical non-finalized block/transaction bytes, encodes one
    /// exact DAG-sync response, and queues only the physical send leaf.
    pub fn ingest_get_dag_sync_request<F>(
        &self,
        context: NetworkGetDagSyncContext,
        request_rlp: &[u8],
        prepare: F,
    ) -> Result<NetworkIngressDecision>
    where
        F: FnOnce(Vec<H256>) -> Result<DagRuntimeNonFinalizedSyncPayload>,
    {
        if !context.request_allowed {
            return Ok(local_network_decision(
                context.source_payload_id,
                NETWORK_INGRESS_STATUS_DAG_SYNC_REQUEST_THROTTLED,
                ERROR_DAG_SYNC_REQUEST_THROTTLED,
            ));
        }
        let Some((request_period, hashes)) = decode_get_dag_sync_request(request_rlp) else {
            return Ok(local_network_decision(
                context.source_payload_id,
                NETWORK_INGRESS_STATUS_DAG_SYNC_REQUEST_MALFORMED,
                ERROR_DAG_SYNC_REQUEST_MALFORMED,
            ));
        };
        let payload = prepare(hashes)?;
        let response = encode_dag_sync_packet(request_period, &payload);
        Ok(self.lock_api()?.enqueue_dag_sync_response(
            &context,
            request_period,
            payload.period,
            response,
        ))
    }

    /// Prepares one bounded application-owned egress operation and returns the
    /// exact object identities whose peer-known state transport must snapshot.
    /// Canonical decoding and packet-family validation complete before the
    /// one-shot token is published; the retained state is capped and is never
    /// an externally readable payload handle.
    pub fn prepare_egress(
        &self,
        request: NetworkEgressPrepareRequest,
        transaction_accounts: Vec<TransactionGossipAccount>,
        dag_transactions: Vec<crate::TransactionGossipEntry>,
    ) -> Result<NetworkEgressPreparation> {
        let finalized_period = self.chain.try_head()?.size;
        self.lock_api()?.prepare_egress(
            request,
            transaction_accounts,
            dag_transactions,
            finalized_period,
        )
    }

    /// Consumes one prepared operation using an immutable authenticated peer
    /// snapshot. Native policy selects every exact target, constructs complete
    /// packet bytes, and queues only exact sends plus send-dependent marks.
    pub fn plan_egress(&self, request: NetworkEgressPlanRequest) -> Result<NetworkIngressDecision> {
        self.lock_api()?.plan_egress(request)
    }

    /// Cancels one undrained preparation. Unknown/stale tokens are harmless.
    pub fn cancel_egress(&self, token: u64) -> Result<bool> {
        Ok(self.lock_api()?.pending_egress.remove(&token).is_some())
    }

    /// Selects a peer and queues one canonical pending-DAG request.
    pub fn request_pending_dag_blocks(
        &self,
        transport_lane: u32,
        source_payload_id: u64,
        facts: NetworkPendingDagBlocksRequestFacts,
        hashes: Vec<H256>,
    ) -> Result<NetworkIngressDecision> {
        let plan = self.plan_pending_dag_blocks_request(facts)?;
        if !plan.request_pending_dag_blocks || !plan.has_peer {
            return Ok(local_network_decision(
                source_payload_id,
                plan.status,
                &plan.error_code,
            ));
        }
        let mut packet = RlpStream::new_list(2);
        packet.append(&plan.request_period);
        packet.begin_list(hashes.len());
        for hash in hashes {
            packet.append(&hash);
        }
        Ok(self.lock_api()?.enqueue_pending_dag_request(
            transport_lane,
            source_payload_id,
            plan.peer_id,
            plan.request_period,
            packet.out().to_vec(),
        ))
    }

    /// Decodes one DAG-block packet and delegates authoritative admission once.
    pub fn ingest_dag_block_packet<F>(
        &self,
        context: NetworkDagBlockIngressContext,
        packet_rlp: &[u8],
        admit: F,
    ) -> Result<NetworkDagBlockIngressReport>
    where
        F: FnOnce(Vec<u8>, Vec<Vec<u8>>) -> Result<DagBlockIngressReport>,
    {
        let Some((transactions, block_rlp)) = decode_dag_block_packet(packet_rlp) else {
            return Ok(NetworkDagBlockIngressReport {
                decision: local_network_decision(
                    context.source_payload_id,
                    NETWORK_INGRESS_STATUS_DAG_PACKET_MALFORMED,
                    ERROR_DAG_PACKET_MALFORMED,
                ),
                admission: None,
                rejection_action: NETWORK_DAG_REJECTION_ACTION_MALICIOUS,
            });
        };
        let admission = admit(block_rlp, transactions.clone())?;
        let (decision, rejection_action) = if admission.accepted || admission.duplicate {
            (
                self.lock_api()?
                    .enqueue_dag_block_effects(&context, &admission, &transactions)?,
                NETWORK_DAG_REJECTION_ACTION_NONE,
            )
        } else {
            self.lock_api()?
                .plan_dag_block_rejection_decision(&context, admission.reject_code)?
        };
        Ok(NetworkDagBlockIngressReport {
            decision,
            admission: Some(admission),
            rejection_action,
        })
    }

    /// Sequentially admits a canonical DAG-sync packet with partial commits.
    pub fn ingest_dag_sync_packet<F>(
        &self,
        context: NetworkDagBlockIngressContext,
        packet_rlp: &[u8],
        admit: F,
    ) -> Result<NetworkDagSyncIngressReport>
    where
        F: FnOnce(Vec<Vec<u8>>, Vec<Vec<u8>>) -> Result<crate::DagSyncIngressReport>,
    {
        let Some((request_period, response_period, transactions, blocks)) =
            decode_dag_sync_packet(packet_rlp)
        else {
            return Ok(NetworkDagSyncIngressReport {
                decision: local_network_decision(
                    context.source_payload_id,
                    NETWORK_INGRESS_STATUS_DAG_PACKET_MALFORMED,
                    ERROR_DAG_PACKET_MALFORMED,
                ),
                request_period: 0,
                response_period: 0,
                transactions: Vec::new(),
                blocks: Vec::new(),
            });
        };
        if response_period > request_period {
            return Ok(NetworkDagSyncIngressReport {
                decision: local_network_decision(
                    context.source_payload_id,
                    NETWORK_INGRESS_STATUS_DAG_SYNC_PERIOD_AHEAD,
                    ERROR_DAG_SYNC_PERIOD_AHEAD,
                ),
                request_period,
                response_period,
                transactions: Vec::new(),
                blocks: Vec::new(),
            });
        }
        if response_period < request_period {
            return Ok(NetworkDagSyncIngressReport {
                decision: local_network_decision(
                    context.source_payload_id,
                    NETWORK_INGRESS_STATUS_DAG_SYNC_PERIOD_BEHIND,
                    ERROR_DAG_SYNC_PERIOD_BEHIND,
                ),
                request_period,
                response_period,
                transactions: Vec::new(),
                blocks: Vec::new(),
            });
        }
        let native = admit(transactions.clone(), blocks)?;
        let transaction_reports = native.transactions;
        let reports = native.blocks;
        let mut decision =
            self.lock_api()?
                .enqueue_dag_sync_ingress_effects(&context, &reports, &transactions)?;
        if !native.accepted {
            decision.status = NETWORK_INGRESS_STATUS_DAG_BLOCK_REJECTED;
            decision.error_code = ERROR_DAG_BLOCK_REJECTED.to_owned();
        }
        Ok(NetworkDagSyncIngressReport {
            decision,
            request_period,
            response_period,
            transactions: transaction_reports,
            blocks: reports,
        })
    }

    /// Prechecks one raw latest-tarcap PBFT-sync packet without mutation.
    ///
    /// Rust owns exact outer decoding, optimized certificate reconstruction,
    /// strict `PeriodData` decoding, native chain/queue sequencing checks,
    /// certificate target hashes, Ficus pillar/extra-data scheduling, and DAG
    /// order hashing. The existing ingress-decision carrier distinguishes
    /// continue, duplicate, sync-complete, ordinary-drop, and malicious
    /// outcomes. Queue insertion and verified-vote mutation remain outside this
    /// precheck. The application root consumes accepted decisions immediately
    /// in its resumable weighted-admission and queue-push session.
    pub(crate) fn precheck_pbft_sync_packet(
        &self,
        packet_rlp: &[u8],
        source_payload_id: u64,
    ) -> Result<NetworkIngressDecision> {
        let packet = match decode_pbft_sync_packet_precheck(packet_rlp) {
            Ok(packet) => packet,
            Err(_) => {
                return Ok(local_network_decision(
                    source_payload_id,
                    NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS,
                    ERROR_PBFT_SYNC_PACKET_MALFORMED,
                ));
            }
        };
        let block_in_chain = self
            .chain
            .block_exists(packet.period_data.entry.block_hash)?;

        let manager = self.manager.lock();
        let chain_head = manager.chain.head();
        let syncing_period = manager.period_data_queue.syncing_period(chain_head.size);
        let last_block_hash = manager.period_data_queue.last_block_hash_or_chain(
            chain_head.size.saturating_add(1),
            chain_head.last_pbft_block_hash,
        );
        drop(manager);

        let (ficus_activation_period, pillar_blocks_interval) = {
            let api = self.lock_api()?;
            (api.ficus_activation_period, api.pillar_blocks_interval)
        };
        Ok(classify_pbft_sync_packet_precheck(
            packet,
            source_payload_id,
            block_in_chain,
            syncing_period,
            last_block_hash,
            ficus_activation_period,
            pillar_blocks_interval,
        ))
    }
}

/// Returns whether one native transaction packet member may continue ingress.
///
/// A transaction already known to the local queue is the legacy fast-path: the
/// sending peer is marked as knowing it, but is neither blamed nor regossiped.
fn transaction_packet_member_is_benign(report: &TransactionPacketIngressReport) -> bool {
    report.submission.accepted
        || report.submission.queue_status == Some(TransactionQueueInsertStatus::Known)
}

fn classify_pbft_sync_packet_precheck(
    packet: DecodedPbftSyncPacketPrecheck,
    source_payload_id: u64,
    block_in_chain: bool,
    syncing_period: u64,
    last_block_hash: H256,
    ficus_activation_period: u64,
    pillar_blocks_interval: u64,
) -> NetworkIngressDecision {
    let block = &packet.period_data.entry;
    if block_in_chain {
        return local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_DUPLICATE_BLOCK,
            ERROR_PBFT_SYNC_PACKET_DUPLICATE_BLOCK,
        );
    }
    if block.period != syncing_period.saturating_add(1) {
        if packet.current_cert_votes_present && block.period == syncing_period {
            return local_network_decision(
                source_payload_id,
                NETWORK_INGRESS_STATUS_PBFT_SYNC_COMPLETE,
                ERROR_PBFT_SYNC_PACKET_COMPLETE,
            );
        }
        return local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_UNEXPECTED_PERIOD,
            ERROR_PBFT_SYNC_PACKET_UNEXPECTED_PERIOD,
        );
    }
    if packet
        .period_data
        .current_block_cert_vote_rlps
        .iter()
        .chain(block.previous_cert_vote_rlps.iter())
        .any(|vote| {
            inspect_canonical_pbft_vote(vote).map_or(true, |inspection| !inspection.signature_valid)
        })
    {
        return local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS,
            ERROR_PBFT_SYNC_PACKET_CERT_SIGNATURE,
        );
    }
    if packet
        .period_data
        .current_block_cert_vote_rlps
        .iter()
        .any(|vote| {
            inspect_canonical_pbft_vote(vote)
                .map_or(true, |vote| vote.block_hash != block.block_hash)
        })
    {
        return local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS,
            ERROR_PBFT_SYNC_PACKET_CURRENT_CERT_HASH,
        );
    }
    if block.previous_cert_vote_rlps.iter().any(|vote| {
        inspect_canonical_pbft_vote(vote).map_or(true, |vote| vote.block_hash != last_block_hash)
    }) {
        return local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS,
            ERROR_PBFT_SYNC_PACKET_PREVIOUS_CERT_HASH,
        );
    }
    let ficus_active =
        ficus_activation_period != u64::MAX && block.period >= ficus_activation_period;
    let first_pillar_period = if ficus_activation_period == 0 {
        pillar_blocks_interval
    } else {
        ficus_activation_period
    };
    let pillar_period = ficus_activation_period != u64::MAX
        && block.period >= first_pillar_period
        && block.period % pillar_blocks_interval == 1;
    if block.extra_data_present != ficus_active
        || block.extra_data_pillar_block_hash_present != pillar_period
        || block.pillar_votes_present != pillar_period
    {
        return local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS,
            ERROR_PBFT_SYNC_PACKET_PILLAR_SCHEDULE,
        );
    }
    if packet.declared_order_hash != packet.calculated_order_hash {
        return local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS,
            ERROR_PBFT_SYNC_PACKET_ORDER_HASH,
        );
    }
    local_network_decision(source_payload_id, NETWORK_INGRESS_STATUS_ACCEPTED, "")
}

fn admit_pbft_blocks_bundle<E, P>(
    packet_rlp: &[u8],
    source_payload_id: u64,
    current_period: u64,
    final_chain_head: u64,
    mut eligible: E,
    mut publish: P,
) -> Result<NetworkIngressDecision>
where
    E: FnMut(u64, [u8; 20]) -> Result<bool>,
    P: FnMut(PbftBlockLink, Vec<u8>) -> Result<bool>,
{
    let packet = Rlp::new(packet_rlp);
    let blocks = match packet.item_count().ok().filter(|count| *count == 1) {
        Some(_) => packet.at(0).ok().filter(Rlp::is_list),
        None => None,
    };
    let Some(blocks) = blocks else {
        return Ok(local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_MALFORMED,
            ERROR_PBFT_BLOCKS_BUNDLE_MALFORMED,
        ));
    };
    let Ok(block_count) = blocks.item_count() else {
        return Ok(local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_MALFORMED,
            ERROR_PBFT_BLOCKS_BUNDLE_MALFORMED,
        ));
    };
    if block_count > MAX_PBFT_BLOCKS_PER_BUNDLE {
        return Ok(local_network_decision(
            source_payload_id,
            NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_TOO_LARGE,
            ERROR_PBFT_BLOCKS_BUNDLE_TOO_LARGE,
        ));
    }

    let mut decoded_blocks = Vec::with_capacity(block_count);
    for index in 0..block_count {
        let Ok(block) = blocks.at(index) else {
            return Ok(local_network_decision(
                source_payload_id,
                NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_MALFORMED,
                ERROR_PBFT_BLOCKS_BUNDLE_MALFORMED,
            ));
        };
        let block_rlp = block.as_raw().to_vec();
        let Ok((link, metadata)) = decode_pbft_blocks_bundle_member(&block_rlp) else {
            return Ok(local_network_decision(
                source_payload_id,
                NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_MALFORMED,
                ERROR_PBFT_BLOCKS_BUNDLE_MALFORMED,
            ));
        };
        decoded_blocks.push((block_rlp, link, metadata));
    }

    let last_relevant_period = current_period.saturating_add(5);
    let mut unique_authors = HashSet::new();
    for (block_rlp, link, metadata) in decoded_blocks {
        if link.period < current_period || link.period > last_relevant_period {
            continue;
        }
        if !unique_authors.insert((link.period, metadata.author)) {
            return Ok(local_network_decision(
                source_payload_id,
                NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_DUPLICATE_AUTHOR,
                ERROR_PBFT_BLOCKS_BUNDLE_DUPLICATE_AUTHOR,
            ));
        }
        let eligibility_period = link.period.saturating_sub(1);
        if final_chain_head >= eligibility_period
            && !eligible(eligibility_period, metadata.author.into())?
        {
            return Ok(local_network_decision(
                source_payload_id,
                NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_INELIGIBLE_AUTHOR,
                ERROR_PBFT_BLOCKS_BUNDLE_INELIGIBLE_AUTHOR,
            ));
        }
        publish(link, block_rlp)?;
    }
    Ok(local_network_decision(
        source_payload_id,
        NETWORK_INGRESS_STATUS_ACCEPTED,
        ERROR_NONE,
    ))
}

fn decode_pbft_blocks_bundle_member(
    block_rlp: &[u8],
) -> Result<(PbftBlockLink, PbftBlockMetadata)> {
    let block = Rlp::new(block_rlp);
    ensure!(matches!(block.item_count()?, 8 | 9));
    let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(block_rlp))?;
    let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(block_rlp))?;
    ensure!(metadata.extra_data.len() <= MAX_PBFT_BLOCK_EXTRA_DATA_BYTES);
    let _: H256 = block.val_at(2)?;
    let _: H256 = block.val_at(3)?;

    let signature: Vec<u8> = block.val_at(block.item_count()? - 1)?;
    ensure!(signature.len() == 65 && signature[64] <= 3);

    let reward_votes = block.at(6)?;
    ensure!(reward_votes.is_list());
    let mut unique_reward_votes = HashSet::new();
    for index in 0..reward_votes.item_count()? {
        ensure!(unique_reward_votes.insert(reward_votes.val_at::<H256>(index)?));
    }

    Ok((link, metadata))
}

#[derive(Debug)]
enum TransactionPacketDecodeError {
    Malformed,
    TooLarge,
}

fn decode_transaction_packet(
    packet_rlp: &[u8],
) -> std::result::Result<(Vec<Vec<u8>>, Vec<[u8; 32]>), TransactionPacketDecodeError> {
    let packet = Rlp::new(packet_rlp);
    if packet.item_count().ok() != Some(2) {
        return Err(TransactionPacketDecodeError::Malformed);
    }
    let transactions = packet
        .at(0)
        .ok()
        .filter(Rlp::is_list)
        .ok_or(TransactionPacketDecodeError::Malformed)?;
    let hashes = packet
        .at(1)
        .ok()
        .filter(Rlp::is_list)
        .ok_or(TransactionPacketDecodeError::Malformed)?;
    let transaction_count = transactions
        .item_count()
        .map_err(|_| TransactionPacketDecodeError::Malformed)?;
    let hash_count = hashes
        .item_count()
        .map_err(|_| TransactionPacketDecodeError::Malformed)?;
    if transaction_count > MAX_TRANSACTIONS_PER_PACKET
        || hash_count > MAX_TRANSACTION_HASHES_PER_PACKET
    {
        return Err(TransactionPacketDecodeError::TooLarge);
    }
    let transactions = (0..transaction_count)
        .map(|index| {
            transactions
                .at(index)
                .map(|value| value.as_raw().to_vec())
                .map_err(|_| TransactionPacketDecodeError::Malformed)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let hashes = (0..hash_count)
        .map(|index| {
            hashes
                .val_at::<H256>(index)
                .map(Into::into)
                .map_err(|_| TransactionPacketDecodeError::Malformed)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((transactions, hashes))
}

fn decode_get_dag_sync_request(packet_rlp: &[u8]) -> Option<(u64, Vec<H256>)> {
    let packet = Rlp::new(packet_rlp);
    if packet.item_count().ok()? != 2 {
        return None;
    }
    let request_period = packet.val_at(0).ok()?;
    let hashes = packet.at(1).ok()?;
    if !hashes.is_list() {
        return None;
    }
    let mut unique = HashSet::new();
    let mut ordered = Vec::new();
    for index in 0..hashes.item_count().ok()? {
        let hash = hashes.val_at::<H256>(index).ok()?;
        if unique.insert(hash) {
            ordered.push(hash);
        }
    }
    Some((request_period, ordered))
}

fn encode_dag_sync_packet(
    request_period: u64,
    payload: &DagRuntimeNonFinalizedSyncPayload,
) -> Vec<u8> {
    let mut packet = RlpStream::new_list(4);
    packet.append(&request_period);
    packet.append(&payload.period);
    packet.begin_list(payload.storage.transactions.len());
    for transaction in &payload.storage.transactions {
        packet.append_raw(&transaction.tx_rlp, 1);
    }
    packet.begin_list(payload.storage.blocks.len());
    for block in &payload.storage.blocks {
        packet.append_raw(&block.block_rlp, 1);
    }
    packet.out().to_vec()
}

fn decode_rlp_list_raw(value: &Rlp<'_>) -> Option<Vec<Vec<u8>>> {
    if !value.is_list() {
        return None;
    }
    (0..value.item_count().ok()?)
        .map(|index| value.at(index).ok().map(|item| item.as_raw().to_vec()))
        .collect()
}

fn decode_dag_block_packet(packet_rlp: &[u8]) -> Option<(Vec<Vec<u8>>, Vec<u8>)> {
    let packet = Rlp::new(packet_rlp);
    if packet.item_count().ok()? != 2 {
        return None;
    }
    let transactions = decode_rlp_list_raw(&packet.at(0).ok()?)?;
    let block = packet.at(1).ok()?.as_raw().to_vec();
    Some((transactions, block))
}

fn decode_dag_sync_packet(packet_rlp: &[u8]) -> Option<(u64, u64, Vec<Vec<u8>>, Vec<Vec<u8>>)> {
    let packet = Rlp::new(packet_rlp);
    if packet.item_count().ok()? != 4 {
        return None;
    }
    Some((
        packet.val_at(0).ok()?,
        packet.val_at(1).ok()?,
        decode_rlp_list_raw(&packet.at(2).ok()?)?,
        decode_rlp_list_raw(&packet.at(3).ok()?)?,
    ))
}

struct DecodedPbftVotePacket {
    vote_rlp: Vec<u8>,
    pbft_block_rlp: Option<Vec<u8>>,
    peer_pbft_chain_size: Option<u64>,
}

fn decode_single_wrapped_packet(packet_rlp: &[u8]) -> Result<Vec<u8>> {
    let packet = Rlp::new(packet_rlp);
    ensure!(packet.item_count()? == 1);
    Ok(packet.at(0)?.as_raw().to_vec())
}

fn decode_pbft_vote_packet(packet_rlp: &[u8]) -> Result<DecodedPbftVotePacket> {
    let packet = Rlp::new(packet_rlp);
    ensure!(
        packet.item_count()? == 2,
        "NETWORK_PBFT_VOTE_PACKET_MALFORMED"
    );
    let vote_rlp = packet.at(0)?.as_raw().to_vec();
    let optional = packet.at(1)?;
    if optional.is_empty() {
        return Ok(DecodedPbftVotePacket {
            vote_rlp,
            pbft_block_rlp: None,
            peer_pbft_chain_size: None,
        });
    }
    ensure!(
        optional.item_count()? == 2,
        "NETWORK_PBFT_VOTE_PACKET_OPTIONAL_DATA_MALFORMED"
    );
    Ok(DecodedPbftVotePacket {
        vote_rlp,
        pbft_block_rlp: Some(optional.at(0)?.as_raw().to_vec()),
        peer_pbft_chain_size: Some(optional.val_at(1)?),
    })
}

fn decode_pbft_votes_bundle_packet(packet_rlp: &[u8]) -> Result<Vec<Vec<u8>>> {
    decode_pbft_votes_bundle_packet_with_limit(packet_rlp, MAX_VOTES_PER_BUNDLE_PACKET)
}

fn decode_pbft_votes_bundle_packet_with_limit(
    packet_rlp: &[u8],
    maximum_votes: usize,
) -> Result<Vec<Vec<u8>>> {
    let bundle_rlp = decode_single_wrapped_packet(packet_rlp)
        .context("NETWORK_PBFT_VOTES_BUNDLE_PACKET_MALFORMED")?;
    let bundle = Rlp::new(&bundle_rlp);
    ensure!(
        bundle.item_count()? == 5,
        "NETWORK_PBFT_VOTES_BUNDLE_PACKET_MALFORMED"
    );
    let block_hash: H256 = bundle.val_at(0)?;
    let period: u64 = bundle.val_at(1)?;
    let round: u64 = bundle.val_at(2)?;
    let step: u64 = bundle.val_at(3)?;
    let votes = bundle.at(4)?;
    ensure!(
        votes.is_list(),
        "NETWORK_PBFT_VOTES_BUNDLE_PACKET_MALFORMED"
    );
    let vote_count = votes.item_count()?;
    ensure!(
        vote_count > 0 && vote_count <= maximum_votes,
        "NETWORK_PBFT_VOTES_BUNDLE_PACKET_SIZE_INVALID"
    );

    (0..vote_count)
        .map(|index| {
            let optimized = votes.at(index)?;
            ensure!(
                optimized.item_count()? == 2,
                "NETWORK_PBFT_VOTES_BUNDLE_MEMBER_MALFORMED"
            );
            let proof = optimized.at(0)?.data()?;
            let signature = optimized.at(1)?.data()?;
            let mut sortition = RlpStream::new_list(4);
            sortition.append(&period);
            sortition.append(&round);
            sortition.append(&step);
            sortition.append(&proof);
            let sortition_rlp = sortition.out();

            let mut vote = RlpStream::new_list(3);
            vote.append(&block_hash);
            vote.append(&sortition_rlp.as_ref());
            vote.append(&signature);
            Ok(vote.out().to_vec())
        })
        .collect()
}

fn encode_pbft_vote_egress_packet(
    vote_rlp: &[u8],
    block_rlp: Option<&[u8]>,
    finalized_period: u64,
) -> Vec<u8> {
    let mut packet = RlpStream::new_list(2);
    packet.append_raw(vote_rlp, 1);
    if let Some(block_rlp) = block_rlp {
        packet.begin_list(2);
        packet.append_raw(block_rlp, 1);
        packet.append(&finalized_period);
    } else {
        packet.append(&0_u64);
    }
    packet.out().to_vec()
}

fn pbft_vote_ingress_fact(inspection: &crate::PbftCanonicalVoteInspection) -> PbftVoteIngressFact {
    PbftVoteIngressFact {
        period: inspection.period,
        round: inspection.round,
        step: inspection.step,
        vote_type: inspection.vote_type,
    }
}

fn pbft_vote_packet_context(
    request: &NetworkConsensusPacketRequest,
    inspection: &crate::PbftCanonicalVoteInspection,
    vote_rlp: Vec<u8>,
) -> NetworkPbftVoteIngressContext {
    NetworkPbftVoteIngressContext {
        ingress: PbftVoteIngressContext {
            current_period: request.current_period,
            current_round: request.current_round,
            current_step: request.current_step,
            max_future_period_delta: request.max_future_period_delta,
            max_future_round_delta: request.max_future_round_delta,
            max_future_step_delta: request.max_future_step_delta,
            validate_max_round_step: request.validate_max_round_step,
            source_peer_is_voter: request.peer_id == inspection.recovered_public_key,
            can_request_pbft_sync: request.can_request_pbft_sync,
            can_request_next_votes_sync: request.can_request_next_votes_sync,
        },
        transport_lane: request.transport_lane,
        peer_id: request.peer_id,
        peer_pbft_chain_size: request.peer_pbft_chain_size,
        source_payload_id: request.source_payload_id,
        enqueue_admission: true,
        vote_hash: inspection.vote_hash.to_fixed_bytes(),
        vote_rlp,
        pbft_block_rlp: Vec::new(),
        pbft_block_hash: [0; 32],
        pbft_block_period: 0,
    }
}

fn pillar_vote_packet_context(
    request: &NetworkConsensusPacketRequest,
) -> NetworkPillarVoteIngressContext {
    NetworkPillarVoteIngressContext {
        transport_lane: request.transport_lane,
        peer_id: request.peer_id,
        source_payload_id: request.source_payload_id,
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
    next_egress_token: u64,
    next_transaction_gossip_account: usize,
    pending_effects: VecDeque<NetworkEffect>,
    pending_vote_admissions: HashMap<u64, PendingVoteAdmissionContext>,
    pending_vote_bundles: HashMap<u64, PendingVoteBundle>,
    pending_pillar_vote_admissions: HashMap<u64, PendingPillarVoteAdmissionContext>,
    pending_egress: HashMap<u64, PendingNetworkEgress>,
    outstanding_effects: HashMap<u64, NetworkEffect>,
    completed_dependency_status: HashMap<u64, bool>,
    pbft_sync: NetworkPbftSyncLifecycle,
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
        Self::with_pillar_schedule_and_sync(ficus_activation_period, pillar_blocks_interval, 10)
    }

    fn with_pillar_schedule_and_sync(
        ficus_activation_period: u64,
        pillar_blocks_interval: u64,
        deep_syncing_threshold: u64,
    ) -> Self {
        Self {
            ficus_activation_period,
            pillar_blocks_interval,
            next_effect_id: 1,
            next_vote_bundle_id: 1,
            next_egress_token: 1,
            next_transaction_gossip_account: 0,
            pending_effects: VecDeque::new(),
            pending_vote_admissions: HashMap::new(),
            pending_vote_bundles: HashMap::new(),
            pending_pillar_vote_admissions: HashMap::new(),
            pending_egress: HashMap::new(),
            outstanding_effects: HashMap::new(),
            completed_dependency_status: HashMap::new(),
            pbft_sync: NetworkPbftSyncLifecycle::new(deep_syncing_threshold),
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
        self.drain_work_matching(transport_lane, None, budget)
    }

    fn drain_work_matching(
        &mut self,
        transport_lane: u32,
        source_payload_id: Option<u64>,
        budget: u32,
    ) -> NetworkEffectBatch {
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
            if effect.transport_lane != transport_lane
                || source_payload_id.is_some_and(|source| effect.source_payload_id != source)
            {
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
            more_available: self.pending_effects.iter().any(|effect| {
                effect.transport_lane == transport_lane
                    && source_payload_id.is_none_or(|source| effect.source_payload_id == source)
            }),
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
                        let _ = self.enqueue_vote_admission_follow_ups(context, result);
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

    /// Atomically selects a serviceable peer and opens a new sync generation.
    #[must_use]
    pub fn begin_pbft_sync(
        &mut self,
        request: NetworkPbftSyncStartRequest,
    ) -> NetworkPbftSyncStartOutcome {
        if !request.start {
            let plan = plan_max_chain_peer_selection(NetworkPeerSelectionFacts {
                local_pbft_syncing_period: request.local_pbft_synced_period,
                candidates: request.candidates,
            });
            return NetworkPbftSyncStartOutcome {
                status: plan.status,
                error_code: plan.error_code,
                started: false,
                has_peer: plan.has_peer,
                peer_id: plan.peer_id,
                peer_pbft_chain_size: plan.peer_pbft_chain_size,
                request_period: request.local_pbft_synced_period.saturating_add(1),
                generation: self.pbft_sync.generation,
                deep_syncing: self.pbft_sync.deep_syncing,
                enable_snapshot_creation: false,
            };
        }
        let plan = plan_pbft_sync_start(NetworkPbftSyncStartFacts {
            local_pbft_syncing: self.pbft_sync.active,
            local_pbft_synced_period: request.local_pbft_synced_period,
            local_pbft_chain_size: request.local_pbft_chain_size,
            candidates: request.candidates,
        });
        if !plan.start_sync {
            return NetworkPbftSyncStartOutcome {
                status: plan.status,
                error_code: plan.error_code,
                started: false,
                has_peer: plan.has_peer,
                peer_id: plan.peer_id,
                peer_pbft_chain_size: plan.peer_pbft_chain_size,
                request_period: plan.request_period,
                generation: self.pbft_sync.generation,
                deep_syncing: self.pbft_sync.deep_syncing,
                enable_snapshot_creation: plan.enable_snapshot_creation,
            };
        }

        let Some(generation) = self.pbft_sync.generation.checked_add(1) else {
            return NetworkPbftSyncStartOutcome {
                status: NETWORK_STATUS_PLAN_STATUS_ALREADY_SYNCING,
                error_code: "NETWORK_PBFT_SYNC_GENERATION_EXHAUSTED".to_owned(),
                started: false,
                has_peer: plan.has_peer,
                peer_id: plan.peer_id,
                peer_pbft_chain_size: plan.peer_pbft_chain_size,
                request_period: plan.request_period,
                generation: self.pbft_sync.generation,
                deep_syncing: false,
                enable_snapshot_creation: false,
            };
        };
        self.pbft_sync.active = true;
        self.pbft_sync.generation = generation;
        self.pbft_sync.peer_id = plan.peer_id;
        self.pbft_sync.last_peer_id = plan.peer_id;
        self.pbft_sync.has_last_peer = true;
        self.pbft_sync.target_chain_size = plan.peer_pbft_chain_size;
        self.pbft_sync.current_period = request.local_pbft_synced_period;
        self.pbft_sync.request_period = plan.request_period;
        self.pbft_sync.started_at_ms = request.now_ms;
        self.pbft_sync.last_activity_ms = request.now_ms;
        self.pbft_sync.start_count = self.pbft_sync.start_count.saturating_add(1);
        self.pbft_sync.last_stop_reason = NETWORK_PBFT_SYNC_STOP_REASON_NONE;
        self.pbft_sync.refresh_deep_syncing();

        NetworkPbftSyncStartOutcome {
            status: plan.status,
            error_code: plan.error_code,
            started: true,
            has_peer: true,
            peer_id: plan.peer_id,
            peer_pbft_chain_size: plan.peer_pbft_chain_size,
            request_period: plan.request_period,
            generation,
            deep_syncing: self.pbft_sync.deep_syncing,
            enable_snapshot_creation: false,
        }
    }

    /// Applies periodic status follow-up policy and advances native debounce state.
    #[must_use]
    fn process_status_followup(
        &mut self,
        request: NetworkStatusFollowupRequest,
    ) -> NetworkStatusFollowupOutcome {
        let previous_chain_size = self
            .pbft_sync
            .peer_last_status_chain_size
            .insert(request.peer_id, request.peer_pbft_chain_size)
            .unwrap_or_default();
        let plan = plan_status_sync(NetworkStatusSyncFacts {
            local_pbft_syncing: self.pbft_sync.active,
            local_pbft_synced_period: request.local_pbft_synced_period,
            local_pbft_period: request.local_pbft_period,
            local_pbft_round: request.local_pbft_round,
            peer_pbft_chain_size: request.peer_pbft_chain_size,
            peer_pbft_period: request.peer_pbft_period,
            peer_pbft_round: request.peer_pbft_round,
            peer_dag_synced: request.peer_dag_synced,
            peer_last_status_pbft_chain_size: previous_chain_size,
        });
        NetworkStatusFollowupOutcome {
            request_pbft_sync: plan.request_pbft_sync,
            request_pending_dag_blocks: plan.request_pending_dag_blocks,
            request_next_votes: plan.request_next_votes,
            next_votes_period: plan.next_votes_period,
            next_votes_round: plan.next_votes_round,
            sync_generation: self.pbft_sync.generation,
        }
    }

    /// Correlates one response source against native current/last peer identity.
    #[must_use]
    pub fn admit_pbft_sync_source(
        &self,
        request: NetworkPbftSyncSourceRequest,
    ) -> NetworkPbftSyncSourceOutcome {
        let accepted = match request.source {
            NETWORK_PBFT_SYNC_SOURCE_ACTIVE => {
                self.pbft_sync.active && request.peer_id == self.pbft_sync.peer_id
            }
            NETWORK_PBFT_SYNC_SOURCE_LAST => {
                self.pbft_sync.has_last_peer && request.peer_id == self.pbft_sync.last_peer_id
            }
            _ => false,
        };
        let error_code = if accepted {
            ERROR_NONE
        } else if request.source != NETWORK_PBFT_SYNC_SOURCE_ACTIVE
            && request.source != NETWORK_PBFT_SYNC_SOURCE_LAST
        {
            "NETWORK_PBFT_SYNC_SOURCE_KIND_INVALID"
        } else if request.source == NETWORK_PBFT_SYNC_SOURCE_ACTIVE && !self.pbft_sync.active {
            "NETWORK_PBFT_SYNC_NOT_ACTIVE"
        } else {
            "NETWORK_PBFT_SYNC_SOURCE_MISMATCH"
        };
        NetworkPbftSyncSourceOutcome {
            accepted,
            generation: self.pbft_sync.generation,
            active: self.pbft_sync.active,
            error_code: error_code.to_owned(),
        }
    }

    /// Records activity only when generation and response source still match.
    #[must_use]
    pub fn record_pbft_sync_activity(
        &mut self,
        request: NetworkPbftSyncActivityRequest,
    ) -> NetworkPbftSyncActivityOutcome {
        let accepted = self.pbft_sync.active
            && request.generation == self.pbft_sync.generation
            && request.peer_id == self.pbft_sync.peer_id;
        if accepted {
            self.pbft_sync.last_activity_ms = self.pbft_sync.last_activity_ms.max(request.now_ms);
        }
        NetworkPbftSyncActivityOutcome {
            accepted,
            generation: self.pbft_sync.generation,
            deep_syncing: self.pbft_sync.deep_syncing,
            error_code: if accepted {
                ERROR_NONE.to_owned()
            } else {
                "NETWORK_PBFT_SYNC_STALE_ACTIVITY".to_owned()
            },
        }
    }

    /// Stops a session only when its generation and selected peer still match.
    #[must_use]
    pub fn stop_pbft_sync(
        &mut self,
        request: NetworkPbftSyncStopRequest,
    ) -> NetworkPbftSyncStopOutcome {
        let valid_reason = matches!(
            request.reason,
            NETWORK_PBFT_SYNC_STOP_REASON_COMPLETED
                | NETWORK_PBFT_SYNC_STOP_REASON_INACTIVE
                | NETWORK_PBFT_SYNC_STOP_REASON_DISCONNECTED
                | NETWORK_PBFT_SYNC_STOP_REASON_TRANSPORT_FAILED
                | NETWORK_PBFT_SYNC_STOP_REASON_REPLACED
        );
        let stopped = valid_reason
            && self.pbft_sync.active
            && request.generation == self.pbft_sync.generation
            && request.peer_id == self.pbft_sync.peer_id;
        if stopped {
            self.pbft_sync.stop(request.reason);
        }
        NetworkPbftSyncStopOutcome {
            stopped,
            generation: self.pbft_sync.generation,
            error_code: if stopped {
                ERROR_NONE.to_owned()
            } else if !valid_reason {
                "NETWORK_PBFT_SYNC_STOP_REASON_INVALID".to_owned()
            } else {
                "NETWORK_PBFT_SYNC_STALE_STOP".to_owned()
            },
        }
    }

    /// Stops the selected session and requests recovery after a matching disconnect.
    #[must_use]
    pub fn handle_pbft_sync_disconnect(
        &mut self,
        request: NetworkPbftSyncDisconnectRequest,
    ) -> NetworkPbftSyncDisconnectOutcome {
        let current_generation = request.generation == self.pbft_sync.generation;
        if current_generation {
            self.pbft_sync
                .peer_last_status_chain_size
                .remove(&request.peer_id);
        }
        let stopped = self.pbft_sync.active
            && current_generation
            && request.peer_id == self.pbft_sync.peer_id;
        if stopped {
            self.pbft_sync
                .stop(NETWORK_PBFT_SYNC_STOP_REASON_DISCONNECTED);
        }
        NetworkPbftSyncDisconnectOutcome {
            stopped,
            restart_sync: stopped,
            generation: self.pbft_sync.generation,
            error_code: if stopped {
                ERROR_NONE.to_owned()
            } else {
                "NETWORK_PBFT_SYNC_STALE_DISCONNECT".to_owned()
            },
        }
    }

    /// Applies the fixed inactivity policy to the generation observed by a timer.
    #[must_use]
    pub fn tick_pbft_sync(
        &mut self,
        request: NetworkPbftSyncTickRequest,
    ) -> NetworkPbftSyncTickOutcome {
        let current_generation = request.generation == self.pbft_sync.generation;
        let expired = self.pbft_sync.active
            && current_generation
            && request
                .now_ms
                .saturating_sub(self.pbft_sync.last_activity_ms)
                > self.pbft_sync.inactivity_threshold_ms;
        if expired {
            self.pbft_sync.stop(NETWORK_PBFT_SYNC_STOP_REASON_INACTIVE);
        }
        NetworkPbftSyncTickOutcome {
            expired,
            restart_sync: expired,
            generation: self.pbft_sync.generation,
            error_code: if expired || (current_generation && self.pbft_sync.active) {
                ERROR_NONE.to_owned()
            } else if !current_generation {
                "NETWORK_PBFT_SYNC_STALE_TICK".to_owned()
            } else {
                "NETWORK_PBFT_SYNC_NOT_ACTIVE".to_owned()
            },
        }
    }

    /// Updates the current period and recomputes deep sync with saturating subtraction.
    #[must_use]
    pub fn update_pbft_sync_period(&mut self, current_period: u64) -> NetworkPbftSyncSnapshot {
        self.pbft_sync.current_period = current_period;
        self.pbft_sync.refresh_deep_syncing();
        self.pbft_sync.snapshot(self.pbft_sync.last_activity_ms)
    }

    /// Returns a read-only sync snapshot without applying inactivity policy.
    #[must_use]
    pub fn pbft_sync_status(&self, now_ms: u64) -> NetworkPbftSyncSnapshot {
        self.pbft_sync.snapshot(now_ms)
    }

    /// Plans whether PBFT sync should start and which peer should serve it.
    ///
    /// Rust owns max-chain peer selection, light-node serviceability checks,
    /// the start/not-needed decision, and application-owned lifecycle state.
    /// Tarcap supplies canonical live-peer snapshots and retains only peer
    /// bookkeeping plus `GetPbftSyncPacket` wrapping and physical transport.
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
        self.pending_vote_bundles.insert(
            bundle_id,
            PendingVoteBundle {
                completed: vec![false; votes.len()],
            },
        );

        plans
            .into_iter()
            .zip(votes)
            .zip(contexts)
            .enumerate()
            .map(|(index, ((plan, vote), context))| {
                let member = PendingVoteBundleMember { bundle_id, index };
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

        if !vote_inspection.signature_valid || vote_inspection.period < self.ficus_activation_period
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
                    0,
                );
            }
        };
        let mut seen = HashSet::with_capacity(inspections.len());
        if let Some(preactivation) = inspections
            .iter()
            .find(|inspection| inspection.period < self.ficus_activation_period)
        {
            let object_hash = preactivation.vote_hash.to_fixed_bytes();
            let report_id = self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_REPORT_PEER,
                peer_id: context.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PILLAR_VOTE,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                object_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: NETWORK_REASON_PREACTIVATION_PILLAR_VOTE,
                dependency_id: 0,
                period: preactivation.period,
                round: 0,
            });
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_DISCONNECT_PEER,
                peer_id: context.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PILLAR_VOTE,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                object_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: NETWORK_REASON_PREACTIVATION_PILLAR_VOTE,
                dependency_id: report_id,
                period: preactivation.period,
                round: 0,
            });
            return pillar_vote_bundle_rejection(
                &context,
                votes.len(),
                NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_CONTEXT,
                ERROR_PILLAR_VOTE_INGRESS_INVALID_CONTEXT,
                2,
            );
        }
        if inspections.iter().any(|inspection| {
            !inspection.signature_valid || !seen.insert(inspection.vote_hash.to_fixed_bytes())
        }) {
            return pillar_vote_bundle_rejection(
                &context,
                votes.len(),
                NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_CONTEXT,
                ERROR_PILLAR_VOTE_INGRESS_INVALID_CONTEXT,
                0,
            );
        }

        votes
            .into_iter()
            .map(|vote_rlp| self.ingest_pillar_vote(context.clone(), vote_rlp))
            .collect()
    }

    fn reject_invalid_pillar_votes_request(
        &mut self,
        request: &NetworkGetPillarVotesBundlePacketRequest,
        query: &GetPillarVotesBundleQuery,
    ) -> Option<NetworkIngressDecision> {
        let inactive =
            self.ficus_activation_period == u64::MAX || query.period < self.ficus_activation_period;
        let first_pillar_period = if self.ficus_activation_period == 0 {
            self.pillar_blocks_interval
        } else {
            self.ficus_activation_period
        };
        let wrong_period = !inactive
            && (query.period < first_pillar_period
                || query.period % self.pillar_blocks_interval != 1);
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
            object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
            object_hash: query.pillar_block_hash,
            sync_kind: 0,
            sync_start: 0,
            reason_code: NETWORK_REASON_INVALID_PILLAR_VOTES_REQUEST,
            dependency_id: 0,
            period: query.period,
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
            object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
            object_hash: query.pillar_block_hash,
            sync_kind: 0,
            sync_start: 0,
            reason_code: NETWORK_REASON_INVALID_PILLAR_VOTES_REQUEST,
            dependency_id: report_id,
            period: query.period,
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
                bundle,
            },
        );
        vote_effect_id
    }

    /// Removes one queued PBFT admission before native sibling execution.
    ///
    /// Only an undrained record-vote effect can be taken. The associated
    /// context is returned to the root-bound service, so canonical bytes never
    /// cross CXX merely to re-enter Rust admission.
    fn take_native_vote_admission(
        &mut self,
        effect_id: u64,
    ) -> Option<PendingVoteAdmissionContext> {
        let position = self.pending_effects.iter().position(|effect| {
            effect.effect_id == effect_id
                && effect.kind == NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
                && effect.object_kind == NETWORK_OBJECT_KIND_PBFT_VOTE
        })?;
        self.pending_effects.remove(position)?;
        self.pending_vote_admissions.remove(&effect_id)
    }

    /// Converts one root-owned PBFT admission into transport follow-ups.
    ///
    /// Persistence rejection is terminal and queues no dependent work. A
    /// published duplicate is marked known; a published insertion may gossip.
    /// Slashing conflicts cancel the remaining bundle members and expose their
    /// transaction effect only through the operation return value.
    fn complete_native_vote_admission(
        &mut self,
        context: PendingVoteAdmissionContext,
        admission: &crate::PbftVoteAdmissionWithSlashingResult,
    ) -> Vec<u64> {
        let add = admission.transaction.outcome.add_outcome.as_ref();
        let intents = admission
            .transaction
            .outcome
            .execution
            .as_ref()
            .map(|execution| execution.pipeline_step.progress_plan.intents.as_slice())
            .unwrap_or_default();
        let result = NetworkEffectResult {
            effect_id: 0,
            kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
            peer_id: context.peer_id,
            packet_kind: 0,
            object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
            object_hash: context.vote_hash,
            status: NETWORK_EFFECT_RESULT_STATUS_OK,
            diagnostic: String::new(),
            admission_accepted: admission.transaction.transition_published
                && admission.validation.accepted
                && add.is_some_and(|outcome| outcome.inserted),
            admission_already_present: admission.transaction.transition_published
                && add.is_some_and(|outcome| outcome.duplicate_vote_hash),
            admission_mark_vote_known: admission.transaction.transition_published
                && intents
                    .iter()
                    .any(|intent| matches!(intent, PbftVoteProgressIntent::MarkKnown { .. })),
            admission_gossip_vote: admission.transaction.transition_published
                && intents
                    .iter()
                    .any(|intent| matches!(intent, PbftVoteProgressIntent::GossipVote { .. })),
            admission_report_slashing: admission.slashing_transaction_effect.is_some(),
        };
        self.enqueue_vote_admission_follow_ups(context, &result)
    }

    fn enqueue_vote_admission_follow_ups(
        &mut self,
        context: PendingVoteAdmissionContext,
        result: &NetworkEffectResult,
    ) -> Vec<u64> {
        let mut effect_ids = Vec::new();
        let bundle = context.bundle.clone();
        if result.admission_report_slashing {
            if let Some(member) = bundle {
                self.cancel_vote_bundle(member.bundle_id);
            }
            return effect_ids;
        }
        let block_effect_id = if context.pbft_block_rlp.is_empty()
            || !result.admission_accepted && !result.admission_already_present
        {
            0
        } else {
            let effect_id = self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_BLOCK,
                object_hash: context.pbft_block_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: context.pbft_block_period,
                round: 0,
            });
            effect_ids.push(effect_id);
            0
        };

        if result.admission_mark_vote_known || result.admission_already_present {
            effect_ids.push(self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                object_hash: context.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            }));
        }

        let _ = block_effect_id;

        if let Some(member) = bundle {
            self.record_vote_bundle_admission(member);
        }
        effect_ids
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
            },
        );
        effect_id
    }

    /// Removes one undrained pillar admission before native sibling execution.
    fn take_native_pillar_vote_admission(
        &mut self,
        effect_id: u64,
    ) -> Option<PendingPillarVoteAdmissionContext> {
        let position = self.pending_effects.iter().position(|effect| {
            effect.effect_id == effect_id
                && effect.kind == NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
                && effect.object_kind == NETWORK_OBJECT_KIND_PILLAR_VOTE
        })?;
        self.pending_effects.remove(position)?;
        self.pending_pillar_vote_admissions.remove(&effect_id)
    }

    /// Cancels exact not-yet-admitted pillar members after a sibling failure.
    ///
    /// Zero and already-consumed ids are harmless. Both queue and context
    /// storage are cleaned in one network lock epoch, preventing a later drain
    /// or stale executor acknowledgement from resurrecting a partial bundle.
    fn cancel_pillar_vote_admissions(&mut self, effect_ids: &[u64]) {
        let ids = effect_ids
            .iter()
            .copied()
            .filter(|effect_id| *effect_id != 0)
            .collect::<HashSet<_>>();
        if ids.is_empty() {
            return;
        }
        self.pending_effects.retain(|effect| {
            !(ids.contains(&effect.effect_id)
                && effect.kind == NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
                && effect.object_kind == NETWORK_OBJECT_KIND_PILLAR_VOTE)
        });
        self.pending_pillar_vote_admissions
            .retain(|effect_id, _| !ids.contains(effect_id));
    }

    /// Removes exact undrained follow-up effects created by a failed composed operation.
    fn cancel_effects(&mut self, effect_ids: &[u64]) {
        let ids = effect_ids.iter().copied().collect::<HashSet<_>>();
        self.pending_effects
            .retain(|effect| !ids.contains(&effect.effect_id));
    }

    /// Converts native pillar admission into transport-only follow-up leaves.
    fn complete_native_pillar_vote_admission(
        &mut self,
        context: PendingPillarVoteAdmissionContext,
        admission: &PillarVoteSingleAdmissionWithFinalChainPlan,
    ) -> Vec<u64> {
        let result = NetworkEffectResult {
            effect_id: 0,
            kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
            peer_id: context.peer_id,
            packet_kind: 0,
            object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
            object_hash: context.vote_hash,
            status: NETWORK_EFFECT_RESULT_STATUS_OK,
            diagnostic: String::new(),
            admission_accepted: admission.accepted,
            admission_already_present: admission.duplicate,
            admission_mark_vote_known: false,
            admission_gossip_vote: false,
            admission_report_slashing: false,
        };
        self.enqueue_pillar_vote_admission_follow_ups(context, &result)
    }

    fn enqueue_pillar_vote_admission_follow_ups(
        &mut self,
        context: PendingPillarVoteAdmissionContext,
        result: &NetworkEffectResult,
    ) -> Vec<u64> {
        let mut effect_ids = Vec::with_capacity(2);
        if result.admission_accepted {
            effect_ids.push(self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                object_hash: context.vote_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: context.period,
                round: 0,
            }));
        }

        effect_ids
    }

    /// Records one exact-ID-correlated bundle admission result and releases the
    /// private aggregation state after every member completes. Egress is a
    /// separate prepare/snapshot/plan operation owned by the caller's lane.
    fn record_vote_bundle_admission(&mut self, member: PendingVoteBundleMember) {
        let Some(bundle) = self.pending_vote_bundles.get_mut(&member.bundle_id) else {
            return;
        };
        if member.index >= bundle.completed.len() || bundle.completed[member.index] {
            return;
        }
        bundle.completed[member.index] = true;
        if bundle.completed.iter().any(|completed| !completed) {
            return;
        }

        self.pending_vote_bundles
            .remove(&member.bundle_id)
            .expect("bundle exists");
    }

    /// Removes only the operation-owned transport effects identified by an
    /// application-root composition that failed before source-scoped drain.
    fn cancel_pending_effects(&mut self, effect_ids: &[u64]) {
        let effect_ids = effect_ids.iter().copied().collect::<HashSet<_>>();
        self.pending_effects
            .retain(|effect| !effect_ids.contains(&effect.effect_id));
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
        current_period: u64,
        current_round: u64,
        chunks: Vec<NextVotesNetworkChunk>,
    ) -> NetworkIngressDecision {
        let queued_effect_count = chunks.iter().fold(0_u32, |count, chunk| {
            count.saturating_add(
                1_u32.saturating_add(u32::try_from(chunk.vote_hashes.len()).unwrap_or(u32::MAX)),
            )
        });
        let round = current_round - 1;
        for chunk in chunks {
            let mut packet = RlpStream::new_list(1);
            packet.append_raw(&chunk.payload_bytes, 1);
            let send_id = self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: request.source_payload_id,
                transport_lane: request.transport_lane,
                kind: NETWORK_EFFECT_KIND_SEND_PACKET,
                peer_id: request.peer_id,
                packet_kind: NETWORK_PACKET_KIND_PBFT_VOTES_BUNDLE,
                payload_bytes: packet.out().to_vec(),
                object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                object_hash: [0; 32],
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: current_period,
                round,
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
                    object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                    object_hash: vote_hash,
                    sync_kind: 0,
                    sync_start: 0,
                    reason_code: 0,
                    dependency_id: send_id,
                    period: current_period,
                    round,
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

    fn enqueue_pillar_vote_bundle_send_effects(
        &mut self,
        request: NetworkGetPillarVotesBundlePacketRequest,
        query: GetPillarVotesBundleQuery,
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
                object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                object_hash: query.pillar_block_hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: query.period,
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
                    object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                    object_hash: vote_hash,
                    sync_kind: 0,
                    sync_start: 0,
                    reason_code: 0,
                    dependency_id: send_id,
                    period: query.period,
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
        _fact: PbftVoteIngressFact,
        context: &NetworkPbftVoteIngressContext,
    ) {
        if plan.request_pbft_sync {
            // Join the application-owned PBFT-sync lifecycle at the next
            // locally admissible period. The remote chain size is an
            // eligibility fact, not a safe response cursor.
            let sync_start = context.ingress.current_period;
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_REQUEST_SYNC,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: encode_get_pbft_sync_packet(sync_start),
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
                payload_bytes: encode_get_next_votes_packet(
                    context.ingress.current_period,
                    context.ingress.current_round,
                ),
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

    fn enqueue_transaction_packet_effects(
        &mut self,
        context: &NetworkTransactionPacketContext,
        reports: &[TransactionPacketIngressReport],
        extra_hashes: &[[u8; 32]],
    ) -> NetworkIngressDecision {
        let before = self.pending_effects.len();
        for report in reports {
            let hash: [u8; 32] = report.submission.transaction_hash.into();
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_TRANSACTION,
                object_hash: hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }
        for hash in extra_hashes {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_TRANSACTION,
                object_hash: *hash,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }
        NetworkIngressDecision {
            payload_id: context.source_payload_id,
            payload_accepted: true,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: String::new(),
            queued_effect_count: u32::try_from(self.pending_effects.len() - before)
                .unwrap_or(u32::MAX),
            application_effect_id: 0,
        }
    }

    fn enqueue_dag_sync_response(
        &mut self,
        context: &NetworkGetDagSyncContext,
        request_period: u64,
        response_period: u64,
        payload_bytes: Vec<u8>,
    ) -> NetworkIngressDecision {
        self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: context.source_payload_id,
            transport_lane: context.transport_lane,
            kind: NETWORK_EFFECT_KIND_SEND_PACKET,
            peer_id: context.peer_id,
            packet_kind: NETWORK_PACKET_KIND_DAG_SYNC,
            payload_bytes,
            object_kind: NETWORK_OBJECT_KIND_DAG_SYNC_EGRESS_REQUEST,
            object_hash: [0; 32],
            sync_kind: 0,
            sync_start: request_period,
            reason_code: 0,
            dependency_id: 0,
            period: response_period,
            round: 0,
        });
        NetworkIngressDecision {
            payload_id: context.source_payload_id,
            payload_accepted: true,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: String::new(),
            queued_effect_count: 1,
            application_effect_id: 0,
        }
    }

    fn enqueue_transaction_gossip(
        &mut self,
        transport_lane: u32,
        source_payload_id: u64,
        peers: Vec<([u8; 64], Vec<[u8; 32]>)>,
        accounts: Vec<TransactionGossipAccount>,
    ) -> NetworkIngressDecision {
        if accounts.is_empty() || peers.is_empty() {
            return local_network_decision(
                source_payload_id,
                NETWORK_INGRESS_STATUS_ACCEPTED,
                ERROR_NONE,
            );
        }
        let before = self.pending_effects.len();
        let mut account_start = self.next_transaction_gossip_account % accounts.len();
        for (peer_id, known_hashes) in peers {
            let known = known_hashes.into_iter().collect::<HashSet<_>>();
            let mut full = Vec::new();
            let mut hashes = Vec::new();
            let mut next_start = (account_start + 1) % accounts.len();
            for offset in 0..accounts.len() {
                let index = (account_start + offset) % accounts.len();
                for transaction in &accounts[index].transactions {
                    let hash: [u8; 32] = transaction.hash.into();
                    if known.contains(&hash) {
                        continue;
                    }
                    if full.len() < MAX_TRANSACTIONS_PER_PACKET {
                        full.push((hash, transaction.transaction_rlp.clone()));
                        if full.len() == MAX_TRANSACTIONS_PER_PACKET {
                            next_start = (index + 1) % accounts.len();
                        }
                    } else if hashes.len() < MAX_TRANSACTION_HASHES_PER_PACKET {
                        hashes.push(hash);
                    }
                }
                if hashes.len() == MAX_TRANSACTION_HASHES_PER_PACKET {
                    break;
                }
            }
            account_start = next_start;
            if full.is_empty() && hashes.is_empty() {
                continue;
            }
            let mut packet = RlpStream::new_list(2);
            packet.begin_list(full.len());
            for (_, transaction_rlp) in &full {
                packet.append_raw(transaction_rlp, 1);
            }
            packet.begin_list(hashes.len());
            for hash in &hashes {
                packet.append(&H256::from(*hash));
            }
            let send_id = self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id,
                transport_lane,
                kind: NETWORK_EFFECT_KIND_SEND_PACKET,
                peer_id,
                packet_kind: NETWORK_PACKET_KIND_TRANSACTION,
                payload_bytes: packet.out().to_vec(),
                object_kind: NETWORK_OBJECT_KIND_TRANSACTION,
                object_hash: [0; 32],
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
            for (hash, _) in full {
                self.enqueue_effect(NetworkEffect {
                    effect_id: 0,
                    source_payload_id,
                    transport_lane,
                    kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                    peer_id,
                    packet_kind: 0,
                    payload_bytes: Vec::new(),
                    object_kind: NETWORK_OBJECT_KIND_TRANSACTION,
                    object_hash: hash,
                    sync_kind: 0,
                    sync_start: 0,
                    reason_code: 0,
                    dependency_id: send_id,
                    period: 0,
                    round: 0,
                });
            }
        }
        self.next_transaction_gossip_account = account_start;
        NetworkIngressDecision {
            payload_id: source_payload_id,
            payload_accepted: true,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: String::new(),
            queued_effect_count: u32::try_from(self.pending_effects.len() - before)
                .unwrap_or(u32::MAX),
            application_effect_id: 0,
        }
    }

    fn prepare_egress(
        &mut self,
        request: NetworkEgressPrepareRequest,
        transaction_accounts: Vec<TransactionGossipAccount>,
        dag_transactions: Vec<crate::TransactionGossipEntry>,
        finalized_period: u64,
    ) -> Result<NetworkEgressPreparation> {
        ensure!(
            self.pending_egress.len() < MAX_PENDING_EGRESS_OPERATIONS,
            "NETWORK_EGRESS_PREPARATION_LIMIT"
        );
        let (payload, probes) = match request.family {
            NETWORK_EGRESS_FAMILY_PBFT_VOTE => {
                let (vote_rlp, related_payload_bytes) = if request.source_peer_id == [0; 64] {
                    (
                        request.payload_bytes.clone(),
                        request.related_payload_bytes.clone(),
                    )
                } else {
                    let decoded = decode_pbft_vote_packet(&request.payload_bytes)
                        .context("NETWORK_EGRESS_PBFT_VOTE_PACKET_MALFORMED")?;
                    (decoded.vote_rlp, decoded.pbft_block_rlp.unwrap_or_default())
                };
                let vote = inspect_canonical_pbft_vote(&vote_rlp)
                    .context("NETWORK_EGRESS_PBFT_VOTE_MALFORMED")?;
                ensure!(
                    vote.signature_valid,
                    "NETWORK_EGRESS_PBFT_VOTE_INVALID_SIGNATURE"
                );
                let mut probes = vec![NetworkEgressProbe {
                    probe_id: 0,
                    object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                    object_hash: vote.vote_hash.to_fixed_bytes(),
                }];
                let (block_hash, block_period) = if related_payload_bytes.is_empty() {
                    ([0; 32], 0)
                } else {
                    let link =
                        PbftBlockLink::try_from(SignedPbftBlockRlp::new(&related_payload_bytes))
                            .context("NETWORK_EGRESS_PBFT_BLOCK_MALFORMED")?;
                    ensure!(
                        link.block_hash == vote.block_hash,
                        "NETWORK_EGRESS_PBFT_BLOCK_IDENTITY_MISMATCH"
                    );
                    probes.push(NetworkEgressProbe {
                        probe_id: 1,
                        object_kind: NETWORK_OBJECT_KIND_PBFT_BLOCK,
                        object_hash: link.block_hash.to_fixed_bytes(),
                    });
                    (link.block_hash.to_fixed_bytes(), link.period)
                };
                (
                    PendingNetworkEgressPayload::PbftVote {
                        vote_rlp,
                        block_rlp: related_payload_bytes,
                        vote_hash: vote.vote_hash.to_fixed_bytes(),
                        block_hash,
                        period: block_period,
                        finalized_period,
                    },
                    probes,
                )
            }
            NETWORK_EGRESS_FAMILY_PBFT_VOTES_BUNDLE => {
                let packet_rlp = if request.source_peer_id == [0; 64] {
                    let mut outer = RlpStream::new_list(1);
                    outer.append_raw(&request.payload_bytes, 1);
                    outer.out().to_vec()
                } else {
                    request.payload_bytes.clone()
                };
                let vote_rlps = decode_pbft_votes_bundle_packet_with_limit(
                    &packet_rlp,
                    MAX_EGRESS_OBJECT_PROBES,
                )
                .context("NETWORK_EGRESS_PBFT_BUNDLE_MALFORMED")?;
                let mut probes = Vec::with_capacity(vote_rlps.len());
                let mut votes = Vec::with_capacity(vote_rlps.len());
                let mut period = 0;
                let mut round = 0;
                for (index, vote_rlp) in vote_rlps.into_iter().enumerate() {
                    let inspected = inspect_canonical_pbft_vote(&vote_rlp)
                        .context("NETWORK_EGRESS_PBFT_BUNDLE_MEMBER_MALFORMED")?;
                    ensure!(
                        inspected.signature_valid,
                        "NETWORK_EGRESS_PBFT_BUNDLE_MEMBER_INVALID_SIGNATURE"
                    );
                    period = inspected.period;
                    round = inspected.round;
                    probes.push(NetworkEgressProbe {
                        probe_id: index as u32,
                        object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                        object_hash: inspected.vote_hash.to_fixed_bytes(),
                    });
                    votes.push(PbftVotePayloadRecord {
                        hash: inspected.vote_hash,
                        vote_rlp,
                    });
                }
                (
                    PendingNetworkEgressPayload::PbftVotesBundle {
                        votes,
                        period,
                        round,
                    },
                    probes,
                )
            }
            NETWORK_EGRESS_FAMILY_PILLAR_VOTE => {
                let vote_rlp = if request.source_peer_id == [0; 64] {
                    request.payload_bytes.clone()
                } else {
                    decode_single_wrapped_packet(&request.payload_bytes)
                        .context("NETWORK_EGRESS_PILLAR_VOTE_PACKET_MALFORMED")?
                };
                let inspected = inspect_pillar_vote_from_rlp(&vote_rlp)
                    .context("NETWORK_EGRESS_PILLAR_VOTE_MALFORMED")?;
                ensure!(
                    inspected.signature_valid,
                    "NETWORK_EGRESS_PILLAR_VOTE_INVALID_SIGNATURE"
                );
                let vote_hash = inspected.vote_hash.to_fixed_bytes();
                (
                    PendingNetworkEgressPayload::PillarVote {
                        vote_rlp,
                        vote_hash,
                        period: inspected.period,
                    },
                    vec![NetworkEgressProbe {
                        probe_id: 0,
                        object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                        object_hash: vote_hash,
                    }],
                )
            }
            NETWORK_EGRESS_FAMILY_DAG_BLOCK => {
                ensure!(
                    !request.payload_bytes.is_empty(),
                    "NETWORK_EGRESS_DAG_BLOCK_MALFORMED"
                );
                ensure!(
                    request.object_hash != [0; 32],
                    "NETWORK_EGRESS_DAG_HASH_MISSING"
                );
                let mut probes = vec![NetworkEgressProbe {
                    probe_id: 0,
                    object_kind: NETWORK_OBJECT_KIND_DAG_BLOCK,
                    object_hash: request.object_hash,
                }];
                probes.extend(
                    dag_transactions
                        .iter()
                        .enumerate()
                        .map(|(index, transaction)| NetworkEgressProbe {
                            probe_id: (index + 1) as u32,
                            object_kind: NETWORK_OBJECT_KIND_TRANSACTION,
                            object_hash: transaction.hash.to_fixed_bytes(),
                        }),
                );
                (
                    PendingNetworkEgressPayload::DagBlock {
                        block_rlp: request.payload_bytes.clone(),
                        block_hash: request.object_hash,
                        transactions: dag_transactions,
                    },
                    probes,
                )
            }
            NETWORK_EGRESS_FAMILY_TRANSACTION_GOSSIP => {
                let accounts = if request.payload_bytes.is_empty() {
                    transaction_accounts
                } else {
                    let (transactions, _) = decode_transaction_packet(&request.payload_bytes)
                        .map_err(|_| anyhow!("NETWORK_EGRESS_TRANSACTION_PACKET_MALFORMED"))?;
                    let mut grouped = Vec::<TransactionGossipAccount>::new();
                    for transaction_rlp in transactions {
                        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)
                            .context("NETWORK_EGRESS_TRANSACTION_MALFORMED")?;
                        let sender = envelope
                            .sender
                            .ok_or_else(|| anyhow!("NETWORK_EGRESS_TRANSACTION_SENDER_MISSING"))?;
                        if let Some(account) =
                            grouped.iter_mut().find(|account| account.sender == sender)
                        {
                            account.transactions.push(crate::TransactionGossipEntry {
                                hash: envelope.hash,
                                transaction_rlp,
                            });
                        } else {
                            grouped.push(TransactionGossipAccount {
                                sender,
                                transactions: vec![crate::TransactionGossipEntry {
                                    hash: envelope.hash,
                                    transaction_rlp,
                                }],
                            });
                        }
                    }
                    grouped
                };
                let probes = accounts
                    .iter()
                    .flat_map(|account| account.transactions.iter())
                    .enumerate()
                    .map(|(index, transaction)| NetworkEgressProbe {
                        probe_id: index as u32,
                        object_kind: NETWORK_OBJECT_KIND_TRANSACTION,
                        object_hash: transaction.hash.to_fixed_bytes(),
                    })
                    .collect();
                (
                    PendingNetworkEgressPayload::TransactionGossip { accounts },
                    probes,
                )
            }
            NETWORK_EGRESS_FAMILY_PILLAR_VOTES_REQUEST => {
                ensure!(
                    request.payload_bytes.len() == 8,
                    "NETWORK_PILLAR_VOTES_REQUEST_PERIOD_MALFORMED"
                );
                ensure!(
                    request.related_payload_bytes.len() == 8,
                    "NETWORK_PILLAR_VOTES_REQUEST_CURSOR_MALFORMED"
                );
                let period = u64::from_be_bytes(request.payload_bytes.as_slice().try_into()?);
                let local_pbft_syncing_period =
                    u64::from_be_bytes(request.related_payload_bytes.as_slice().try_into()?);
                let inactive = self.ficus_activation_period == u64::MAX
                    || period < self.ficus_activation_period;
                let first_pillar_period = if self.ficus_activation_period == 0 {
                    self.pillar_blocks_interval
                } else {
                    self.ficus_activation_period
                };
                ensure!(
                    !inactive
                        && period >= first_pillar_period
                        && period % self.pillar_blocks_interval == 1,
                    "NETWORK_PILLAR_VOTES_OUTBOUND_INVALID_PERIOD"
                );
                (
                    PendingNetworkEgressPayload::PillarVotesRequest {
                        period,
                        pillar_block_hash: request.object_hash,
                        local_pbft_syncing_period,
                    },
                    Vec::new(),
                )
            }
            _ => return Err(anyhow!("NETWORK_EGRESS_FAMILY_UNSUPPORTED")),
        };
        let token = self.next_egress_token;
        self.next_egress_token = self
            .next_egress_token
            .checked_add(1)
            .ok_or_else(|| anyhow!("NETWORK_EGRESS_TOKEN_EXHAUSTED"))?;
        self.pending_egress.insert(
            token,
            PendingNetworkEgress {
                transport_lane: request.transport_lane,
                source_payload_id: request.source_payload_id,
                source_peer_id: request.source_peer_id,
                rebroadcast: request.rebroadcast,
                probes: probes.clone(),
                payload,
            },
        );
        Ok(NetworkEgressPreparation { token, probes })
    }

    fn plan_egress(&mut self, request: NetworkEgressPlanRequest) -> Result<NetworkIngressDecision> {
        let pending = self
            .pending_egress
            .remove(&request.token)
            .ok_or_else(|| anyhow!("NETWORK_EGRESS_STALE_TOKEN"))?;
        let mut unique_peers = HashSet::new();
        let probe_count = pending.probes.len() as u32;
        for peer in &request.peers {
            ensure!(
                unique_peers.insert(peer.peer_id),
                "NETWORK_EGRESS_DUPLICATE_PEER"
            );
            let mut unique_probes = HashSet::new();
            for probe_id in &peer.known_probe_ids {
                ensure!(*probe_id < probe_count, "NETWORK_EGRESS_UNKNOWN_PROBE");
                ensure!(
                    unique_probes.insert(*probe_id),
                    "NETWORK_EGRESS_DUPLICATE_PROBE"
                );
            }
        }
        let before = self.pending_effects.len();
        let maximum_sends_per_peer = match &pending.payload {
            PendingNetworkEgressPayload::PbftVotesBundle { votes, .. } => {
                votes.len().div_ceil(MAX_VOTES_PER_BUNDLE_PACKET)
            }
            _ => 1,
        };
        let maximum_new_effects = request
            .peers
            .len()
            .saturating_mul(pending.probes.len().saturating_add(maximum_sends_per_peer));
        ensure!(
            self.pending_effects
                .len()
                .saturating_add(maximum_new_effects)
                <= MAX_QUEUED_EGRESS_EFFECTS,
            "NETWORK_EGRESS_EFFECT_LIMIT"
        );
        let effect_owner = pending.clone();
        match pending.payload {
            PendingNetworkEgressPayload::PbftVote {
                vote_rlp,
                block_rlp,
                vote_hash,
                block_hash,
                period,
                finalized_period,
            } => {
                for peer in request.peers {
                    let known = peer.known_probe_ids.into_iter().collect::<HashSet<_>>();
                    if peer.syncing
                        || peer.peer_id == pending.source_peer_id
                        || (!pending.rebroadcast && known.contains(&0))
                    {
                        continue;
                    }
                    let include_block =
                        !block_rlp.is_empty() && (pending.rebroadcast || !known.contains(&1));
                    let packet = encode_pbft_vote_egress_packet(
                        &vote_rlp,
                        include_block.then_some(block_rlp.as_slice()),
                        finalized_period,
                    );
                    let send_id = self.enqueue_exact_egress_send(
                        &effect_owner,
                        peer.peer_id,
                        NETWORK_PACKET_KIND_PBFT_VOTE,
                        packet,
                        NETWORK_OBJECT_KIND_PBFT_VOTE,
                        vote_hash,
                        period,
                    );
                    self.enqueue_egress_known_mark(
                        &effect_owner,
                        peer.peer_id,
                        send_id,
                        NETWORK_OBJECT_KIND_PBFT_VOTE,
                        vote_hash,
                    );
                    if include_block {
                        self.enqueue_egress_known_mark(
                            &effect_owner,
                            peer.peer_id,
                            send_id,
                            NETWORK_OBJECT_KIND_PBFT_BLOCK,
                            block_hash,
                        );
                    }
                }
            }
            PendingNetworkEgressPayload::PbftVotesBundle {
                votes,
                period,
                round,
            } => {
                for peer in request.peers {
                    if peer.syncing || peer.peer_id == pending.source_peer_id {
                        continue;
                    }
                    let known = peer.known_probe_ids.into_iter().collect::<HashSet<_>>();
                    let selected = votes
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| {
                            pending.rebroadcast || !known.contains(&(*index as u32))
                        })
                        .map(|(_, vote)| vote.clone())
                        .collect::<Vec<_>>();
                    for chunk in selected.chunks(MAX_VOTES_PER_BUNDLE_PACKET) {
                        let Some(first) = chunk.first() else {
                            continue;
                        };
                        let inspected = inspect_canonical_pbft_vote(&first.vote_rlp)?;
                        let bundle = build_optimized_pbft_vote_bundle(
                            chunk,
                            inspected.block_hash,
                            inspected.period,
                            inspected.round,
                            inspected.step,
                        )?;
                        let mut packet = RlpStream::new_list(1);
                        packet.append_raw(&bundle.bundle_rlp, 1);
                        let send_id = self.enqueue_exact_egress_send(
                            &effect_owner,
                            peer.peer_id,
                            NETWORK_PACKET_KIND_PBFT_VOTES_BUNDLE,
                            packet.out().to_vec(),
                            NETWORK_OBJECT_KIND_PBFT_VOTE,
                            [0; 32],
                            period,
                        );
                        for vote in chunk {
                            self.enqueue_egress_known_mark(
                                &effect_owner,
                                peer.peer_id,
                                send_id,
                                NETWORK_OBJECT_KIND_PBFT_VOTE,
                                vote.hash.to_fixed_bytes(),
                            );
                        }
                    }
                }
                let _ = round;
            }
            PendingNetworkEgressPayload::PillarVote {
                vote_rlp,
                vote_hash,
                period,
            } => {
                let mut packet = RlpStream::new_list(1);
                packet.append_raw(&vote_rlp, 1);
                let packet = packet.out().to_vec();
                for peer in request.peers {
                    if peer.syncing
                        || peer.peer_id == pending.source_peer_id
                        || (!pending.rebroadcast && peer.known_probe_ids.contains(&0))
                    {
                        continue;
                    }
                    let send_id = self.enqueue_exact_egress_send(
                        &effect_owner,
                        peer.peer_id,
                        NETWORK_PACKET_KIND_PILLAR_VOTE,
                        packet.clone(),
                        NETWORK_OBJECT_KIND_PILLAR_VOTE,
                        vote_hash,
                        period,
                    );
                    self.enqueue_egress_known_mark(
                        &effect_owner,
                        peer.peer_id,
                        send_id,
                        NETWORK_OBJECT_KIND_PILLAR_VOTE,
                        vote_hash,
                    );
                }
            }
            PendingNetworkEgressPayload::DagBlock {
                block_rlp,
                block_hash,
                transactions,
            } => {
                for peer in request.peers {
                    if peer.syncing
                        || peer.peer_id == pending.source_peer_id
                        || peer.known_probe_ids.contains(&0)
                    {
                        continue;
                    }
                    let known = peer.known_probe_ids.into_iter().collect::<HashSet<_>>();
                    let selected_transactions = transactions
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| !known.contains(&((*index + 1) as u32)))
                        .collect::<Vec<_>>();
                    let mut packet = RlpStream::new_list(2);
                    packet.begin_list(selected_transactions.len());
                    for (_, transaction) in &selected_transactions {
                        packet.append_raw(&transaction.transaction_rlp, 1);
                    }
                    packet.append_raw(&block_rlp, 1);
                    let send_id = self.enqueue_exact_egress_send(
                        &effect_owner,
                        peer.peer_id,
                        NETWORK_PACKET_KIND_DAG_BLOCK,
                        packet.out().to_vec(),
                        NETWORK_OBJECT_KIND_DAG_BLOCK,
                        block_hash,
                        0,
                    );
                    self.enqueue_egress_known_mark(
                        &effect_owner,
                        peer.peer_id,
                        send_id,
                        NETWORK_OBJECT_KIND_DAG_BLOCK,
                        block_hash,
                    );
                    for (_, transaction) in selected_transactions {
                        self.enqueue_egress_known_mark(
                            &effect_owner,
                            peer.peer_id,
                            send_id,
                            NETWORK_OBJECT_KIND_TRANSACTION,
                            transaction.hash.to_fixed_bytes(),
                        );
                    }
                }
            }
            PendingNetworkEgressPayload::TransactionGossip { accounts } => {
                let probes = pending
                    .probes
                    .iter()
                    .map(|probe| (probe.object_hash, probe.probe_id))
                    .collect::<HashMap<_, _>>();
                let peers = request
                    .peers
                    .into_iter()
                    .filter(|peer| !peer.syncing && peer.peer_id != pending.source_peer_id)
                    .map(|peer| {
                        let known_ids = peer.known_probe_ids.into_iter().collect::<HashSet<_>>();
                        (
                            peer.peer_id,
                            probes
                                .iter()
                                .filter_map(|(hash, id)| known_ids.contains(id).then_some(*hash))
                                .collect(),
                        )
                    })
                    .collect();
                self.enqueue_transaction_gossip(
                    pending.transport_lane,
                    pending.source_payload_id,
                    peers,
                    accounts,
                );
            }
            PendingNetworkEgressPayload::PillarVotesRequest {
                period,
                pillar_block_hash,
                local_pbft_syncing_period,
            } => {
                if let Some(peer) = request
                    .peers
                    .into_iter()
                    .filter(|peer| {
                        !peer.syncing
                            && (!peer.is_light_node
                                || local_pbft_syncing_period
                                    .saturating_add(peer.light_node_history)
                                    >= peer.pbft_chain_size)
                    })
                    .max_by(|left, right| {
                        left.pbft_chain_size
                            .cmp(&right.pbft_chain_size)
                            .then_with(|| left.dag_level.cmp(&right.dag_level))
                            .then_with(|| right.peer_id.cmp(&left.peer_id))
                            .then_with(|| right.transport_lane.cmp(&left.transport_lane))
                    })
                {
                    self.enqueue_effect(NetworkEffect {
                        effect_id: 0,
                        source_payload_id: pending.source_payload_id,
                        transport_lane: peer.transport_lane,
                        kind: NETWORK_EFFECT_KIND_SEND_PACKET,
                        peer_id: peer.peer_id,
                        packet_kind: NETWORK_PACKET_KIND_GET_PILLAR_VOTES_BUNDLE,
                        payload_bytes: encode_get_pillar_votes_bundle_packet(
                            period,
                            pillar_block_hash,
                        ),
                        object_kind: NETWORK_OBJECT_KIND_PILLAR_VOTE,
                        object_hash: pillar_block_hash,
                        sync_kind: 0,
                        sync_start: 0,
                        reason_code: 0,
                        dependency_id: 0,
                        period,
                        round: 0,
                    });
                }
            }
        }
        Ok(NetworkIngressDecision {
            payload_id: effect_owner.source_payload_id,
            payload_accepted: true,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: String::new(),
            queued_effect_count: (self.pending_effects.len() - before) as u32,
            application_effect_id: 0,
        })
    }

    fn enqueue_exact_egress_send(
        &mut self,
        pending: &PendingNetworkEgress,
        peer_id: [u8; 64],
        packet_kind: u32,
        payload_bytes: Vec<u8>,
        object_kind: u8,
        object_hash: [u8; 32],
        period: u64,
    ) -> u64 {
        self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: pending.source_payload_id,
            transport_lane: pending.transport_lane,
            kind: NETWORK_EFFECT_KIND_SEND_PACKET,
            peer_id,
            packet_kind,
            payload_bytes,
            object_kind,
            object_hash,
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id: 0,
            period,
            round: 0,
        })
    }

    fn enqueue_egress_known_mark(
        &mut self,
        pending: &PendingNetworkEgress,
        peer_id: [u8; 64],
        dependency_id: u64,
        object_kind: u8,
        object_hash: [u8; 32],
    ) {
        self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: pending.source_payload_id,
            transport_lane: pending.transport_lane,
            kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
            peer_id,
            packet_kind: 0,
            payload_bytes: Vec::new(),
            object_kind,
            object_hash,
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id,
            period: 0,
            round: 0,
        });
    }

    fn enqueue_pending_dag_request(
        &mut self,
        transport_lane: u32,
        source_payload_id: u64,
        peer_id: [u8; 64],
        period: u64,
        payload_bytes: Vec<u8>,
    ) -> NetworkIngressDecision {
        self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id,
            transport_lane,
            kind: NETWORK_EFFECT_KIND_SEND_PACKET,
            peer_id,
            packet_kind: NETWORK_PACKET_KIND_GET_DAG_SYNC,
            payload_bytes,
            object_kind: NETWORK_OBJECT_KIND_DAG_SYNC_EGRESS_REQUEST,
            object_hash: [0; 32],
            sync_kind: 0,
            sync_start: period,
            reason_code: 0,
            dependency_id: 0,
            period,
            round: 0,
        });
        NetworkIngressDecision {
            payload_id: source_payload_id,
            payload_accepted: true,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: String::new(),
            queued_effect_count: 1,
            application_effect_id: 0,
        }
    }

    fn enqueue_dag_block_effects(
        &mut self,
        context: &NetworkDagBlockIngressContext,
        report: &DagBlockIngressReport,
        transactions: &[Vec<u8>],
    ) -> Result<NetworkIngressDecision> {
        let before = self.pending_effects.len();
        for transaction in transactions {
            let envelope = LegacyTransactionEnvelope::decode(transaction)
                .context("NETWORK_DAG_TRANSACTION_DECODE")?;
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_TRANSACTION,
                object_hash: envelope.hash.0,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }
        self.enqueue_effect(NetworkEffect {
            effect_id: 0,
            source_payload_id: context.source_payload_id,
            transport_lane: context.transport_lane,
            kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
            peer_id: context.peer_id,
            packet_kind: 0,
            payload_bytes: Vec::new(),
            object_kind: NETWORK_OBJECT_KIND_DAG_BLOCK,
            object_hash: report.block_hash.0,
            sync_kind: 0,
            sync_start: 0,
            reason_code: 0,
            dependency_id: 0,
            period: 0,
            round: 0,
        });
        Ok(NetworkIngressDecision {
            payload_id: context.source_payload_id,
            payload_accepted: true,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: String::new(),
            queued_effect_count: u32::try_from(self.pending_effects.len() - before)
                .unwrap_or(u32::MAX),
            application_effect_id: 0,
        })
    }

    /// Plans the legacy DAG rejection policy using only compact peer facts.
    ///
    /// The terminal action is authoritative and deliberately is not placed on
    /// the shared effect queue: DAG-sync execution can re-enter this API and
    /// must happen only after the caller releases its transport-lane lock.
    fn plan_dag_block_rejection_decision(
        &self,
        context: &NetworkDagBlockIngressContext,
        reject_code: u32,
    ) -> Result<(NetworkIngressDecision, u8)> {
        let action = plan_dag_block_rejection(context, reject_code)?;
        let (status, error_code) = match action {
            NETWORK_DAG_REJECTION_ACTION_IGNORE => (
                NETWORK_INGRESS_STATUS_DAG_BLOCK_IGNORED,
                ERROR_DAG_BLOCK_IGNORED,
            ),
            NETWORK_DAG_REJECTION_ACTION_REQUEST_DAG_SYNC
            | NETWORK_DAG_REJECTION_ACTION_REQUEST_PENDING_DAG => (
                NETWORK_INGRESS_STATUS_DAG_BLOCK_SYNC_REQUESTED,
                ERROR_DAG_BLOCK_SYNC_REQUESTED,
            ),
            NETWORK_DAG_REJECTION_ACTION_DISCONNECT => (
                NETWORK_INGRESS_STATUS_DAG_BLOCK_DISCONNECT,
                ERROR_DAG_BLOCK_DISCONNECT,
            ),
            NETWORK_DAG_REJECTION_ACTION_MALICIOUS => (
                NETWORK_INGRESS_STATUS_DAG_BLOCK_MALICIOUS,
                ERROR_DAG_BLOCK_MALICIOUS,
            ),
            _ => (
                NETWORK_INGRESS_STATUS_DAG_BLOCK_REJECTED,
                ERROR_DAG_BLOCK_REJECTED,
            ),
        };
        Ok((
            NetworkIngressDecision {
                payload_id: context.source_payload_id,
                payload_accepted: context.source_payload_id != 0,
                routed: true,
                status,
                error_code: error_code.to_owned(),
                queued_effect_count: 0,
                application_effect_id: 0,
            },
            action,
        ))
    }

    fn enqueue_dag_sync_ingress_effects(
        &mut self,
        context: &NetworkDagBlockIngressContext,
        reports: &[DagBlockIngressReport],
        transactions: &[Vec<u8>],
    ) -> Result<NetworkIngressDecision> {
        let before = self.pending_effects.len();
        for transaction in transactions {
            let envelope = LegacyTransactionEnvelope::decode(transaction)
                .context("NETWORK_DAG_SYNC_TRANSACTION_DECODE")?;
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_TRANSACTION,
                object_hash: envelope.hash.0,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }
        for report in reports {
            self.enqueue_effect(NetworkEffect {
                effect_id: 0,
                source_payload_id: context.source_payload_id,
                transport_lane: context.transport_lane,
                kind: NETWORK_EFFECT_KIND_MARK_PEER_KNOWN,
                peer_id: context.peer_id,
                packet_kind: 0,
                payload_bytes: Vec::new(),
                object_kind: NETWORK_OBJECT_KIND_DAG_BLOCK,
                object_hash: report.block_hash.0,
                sync_kind: 0,
                sync_start: 0,
                reason_code: 0,
                dependency_id: 0,
                period: 0,
                round: 0,
            });
        }
        Ok(NetworkIngressDecision {
            payload_id: context.source_payload_id,
            payload_accepted: true,
            routed: true,
            status: NETWORK_INGRESS_STATUS_ACCEPTED,
            error_code: String::new(),
            queued_effect_count: u32::try_from(self.pending_effects.len() - before)
                .unwrap_or(u32::MAX),
            application_effect_id: 0,
        })
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
    current_period: u64,
    current_round: u64,
) -> Option<NetworkIngressDecision> {
    let (status, error_code) = if current_period != request.peer_period {
        (
            NETWORK_INGRESS_STATUS_NEXT_VOTES_PERIOD_MISMATCH,
            ERROR_NEXT_VOTES_PERIOD_MISMATCH,
        )
    } else if current_round <= 1 {
        (
            NETWORK_INGRESS_STATUS_NEXT_VOTES_NO_PREVIOUS_ROUND,
            ERROR_NEXT_VOTES_NO_PREVIOUS_ROUND,
        )
    } else if current_round < request.peer_round {
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

fn plan_dag_block_rejection(
    context: &NetworkDagBlockIngressContext,
    reject_code: u32,
) -> Result<u8> {
    let action = match reject_code {
        DAG_VERIFY_REJECT_MISSING_TRANSACTION => {
            if context.dag_sync_allowed {
                NETWORK_DAG_REJECTION_ACTION_REQUEST_DAG_SYNC
            } else if context.transactions_dropped {
                NETWORK_DAG_REJECTION_ACTION_DISCONNECT
            } else {
                NETWORK_DAG_REJECTION_ACTION_MALICIOUS
            }
        }
        DAG_VERIFY_REJECT_MISSING_TIP => {
            if context.peer_dag_synced && context.dag_sync_allowed {
                NETWORK_DAG_REJECTION_ACTION_REQUEST_DAG_SYNC
            } else if context.peer_dag_synced {
                NETWORK_DAG_REJECTION_ACTION_MALICIOUS
            } else if context.pending_dag_request {
                NETWORK_DAG_REJECTION_ACTION_IGNORE
            } else {
                NETWORK_DAG_REJECTION_ACTION_REQUEST_PENDING_DAG
            }
        }
        DAG_VERIFY_REJECT_AHEAD_BLOCK | DAG_VERIFY_REJECT_FUTURE_BLOCK => {
            if context.peer_dag_synced {
                NETWORK_DAG_REJECTION_ACTION_DISCONNECT
            } else {
                NETWORK_DAG_REJECTION_ACTION_IGNORE
            }
        }
        DAG_VERIFY_REJECT_EXPIRED_BLOCK => NETWORK_DAG_REJECTION_ACTION_IGNORE,
        DAG_VERIFY_REJECT_ADD_BLOCK_METADATA => {
            if context.local_pbft_syncing || context.pending_dag_request {
                NETWORK_DAG_REJECTION_ACTION_IGNORE
            } else if context.peer_dag_synced {
                NETWORK_DAG_REJECTION_ACTION_MALICIOUS
            } else {
                NETWORK_DAG_REJECTION_ACTION_REQUEST_PENDING_DAG
            }
        }
        DAG_VERIFY_REJECT_INCORRECT_TRANSACTIONS_ESTIMATION
        | DAG_VERIFY_REJECT_BLOCK_TOO_BIG
        | DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        | DAG_VERIFY_REJECT_NOT_ELIGIBLE
        | DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION => NETWORK_DAG_REJECTION_ACTION_MALICIOUS,
        _ => return Err(anyhow!("NETWORK_DAG_UNKNOWN_REJECT_CODE:{reject_code}")),
    };
    Ok(action)
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
) -> Result<Vec<NextVotesNetworkChunk>> {
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
) -> Option<Vec<NextVotesNetworkChunk>> {
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
        raw_votes.push((vote.as_raw(), inspection.vote_hash.to_fixed_bytes()));
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
                for (vote, _) in chunk {
                    stream.append_raw(vote, 1);
                }
                NextVotesNetworkChunk {
                    vote_hashes: chunk.iter().map(|(_, hash)| *hash).collect(),
                    payload_bytes: stream.out().to_vec(),
                }
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
        let bundle_bytes = encode_optimized_pillar_votes_bundle_rlp(&votes)
            .context("NETWORK_PILLAR_VOTE_NATIVE_BUNDLE_ENCODING_FAILED")?;
        let decoded = rustaxa_types::decode_optimized_pillar_votes_bundle_rlp(&bundle_bytes)
            .context("NETWORK_PILLAR_VOTE_NATIVE_BUNDLE_REVALIDATION_FAILED")?;
        ensure!(
            decoded == votes,
            "NETWORK_PILLAR_VOTE_NATIVE_BUNDLE_ORDER_MISMATCH"
        );
        let mut packet = RlpStream::new_list(1);
        packet.append_raw(&bundle_bytes, 1);
        chunks.push(PillarVoteNetworkChunk {
            vote_hashes: entries.iter().map(|(_, hash)| *hash).collect(),
            payload_bytes: packet.out().to_vec(),
        });
    }
    Ok(chunks)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DecodedStatusPacket {
    peer_pbft_chain_size: u64,
    peer_pbft_round: u64,
    peer_dag_level: u64,
    peer_syncing: bool,
    initial_data: Option<DecodedStatusInitialData>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DecodedStatusInitialData {
    chain_id: u64,
    genesis_hash: [u8; 32],
    node_major_version: u32,
    node_minor_version: u32,
    node_patch_version: u32,
    is_light_node: bool,
    light_node_history: u64,
}

fn decode_status_packet(bytes: &[u8]) -> Result<DecodedStatusPacket> {
    let rlp = Rlp::new(bytes);
    ensure!(
        rlp.item_count()? == 5,
        "NETWORK_STATUS_PACKET_INVALID_FIELD_COUNT"
    );
    let initial = rlp.at(4)?;
    let initial_data = if initial.is_empty() {
        ensure!(
            !initial.is_list(),
            "NETWORK_STATUS_PACKET_INVALID_INITIAL_DATA"
        );
        None
    } else {
        ensure!(
            initial.item_count()? == 7,
            "NETWORK_STATUS_PACKET_INVALID_INITIAL_FIELD_COUNT"
        );
        Some(DecodedStatusInitialData {
            chain_id: initial.val_at(0)?,
            genesis_hash: initial.val_at::<H256>(1)?.into(),
            node_major_version: initial.val_at(2)?,
            node_minor_version: initial.val_at(3)?,
            node_patch_version: initial.val_at(4)?,
            is_light_node: initial.val_at(5)?,
            light_node_history: initial.val_at(6)?,
        })
    };
    let decoded = DecodedStatusPacket {
        peer_pbft_chain_size: rlp.val_at(0)?,
        peer_pbft_round: rlp.val_at(1)?,
        peer_dag_level: rlp.val_at(2)?,
        peer_syncing: rlp.val_at(3)?,
        initial_data,
    };
    let request = NetworkStatusPacketBuildRequest {
        initial: decoded.initial_data.is_some(),
        local_pbft_chain_size: decoded.peer_pbft_chain_size,
        local_pbft_round: decoded.peer_pbft_round,
        local_dag_level: decoded.peer_dag_level,
    };
    let identity = decoded
        .initial_data
        .as_ref()
        .map(|initial| NetworkNodeIdentity {
            chain_id: initial.chain_id,
            genesis_hash: initial.genesis_hash,
            node_major_version: initial.node_major_version,
            node_minor_version: initial.node_minor_version,
            node_patch_version: initial.node_patch_version,
            is_light_node: initial.is_light_node,
            light_node_history: initial.light_node_history,
        });
    ensure!(
        encode_status_packet(request, decoded.peer_syncing, identity.as_ref()) == bytes,
        "NETWORK_STATUS_PACKET_NON_CANONICAL_RLP"
    );
    Ok(decoded)
}

fn encode_status_packet(
    request: NetworkStatusPacketBuildRequest,
    syncing: bool,
    identity: Option<&NetworkNodeIdentity>,
) -> Vec<u8> {
    let mut stream = RlpStream::new_list(5);
    stream.append(&request.local_pbft_chain_size);
    stream.append(&request.local_pbft_round);
    stream.append(&request.local_dag_level);
    stream.append(&syncing);
    if let Some(identity) = identity {
        stream.begin_list(7);
        stream.append(&identity.chain_id);
        stream.append(&identity.genesis_hash.as_slice());
        stream.append(&identity.node_major_version);
        stream.append(&identity.node_minor_version);
        stream.append(&identity.node_patch_version);
        stream.append(&identity.is_light_node);
        stream.append(&identity.light_node_history);
    } else {
        stream.append(&0_u8);
    }
    stream.out().to_vec()
}

fn encode_get_next_votes_packet(period: u64, round: u64) -> Vec<u8> {
    let mut stream = RlpStream::new_list(2);
    stream.append(&period);
    stream.append(&round);
    stream.out().to_vec()
}

fn encode_get_pbft_sync_packet(height_to_sync: u64) -> Vec<u8> {
    let mut stream = RlpStream::new_list(1);
    stream.append(&height_to_sync);
    stream.out().to_vec()
}

fn decode_get_next_votes_packet(bytes: &[u8]) -> Result<(u64, u64)> {
    let rlp = Rlp::new(bytes);
    ensure!(
        rlp.item_count()? == 2,
        "NETWORK_GET_NEXT_VOTES_INVALID_FIELD_COUNT"
    );
    let period = rlp.val_at(0)?;
    let round = rlp.val_at(1)?;
    ensure!(
        encode_get_next_votes_packet(period, round) == bytes,
        "NETWORK_GET_NEXT_VOTES_NON_CANONICAL_RLP"
    );
    Ok((period, round))
}

fn encode_get_pillar_votes_bundle_packet(period: u64, pillar_block_hash: [u8; 32]) -> Vec<u8> {
    let mut stream = RlpStream::new_list(2);
    stream.append(&period);
    stream.append(&pillar_block_hash.as_slice());
    stream.out().to_vec()
}

fn decode_get_pillar_votes_bundle_packet(bytes: &[u8]) -> Result<GetPillarVotesBundleQuery> {
    let rlp = Rlp::new(bytes);
    ensure!(
        rlp.item_count()? == 2,
        "NETWORK_GET_PILLAR_VOTES_BUNDLE_INVALID_FIELD_COUNT"
    );
    let period = rlp.val_at(0)?;
    let hash_bytes: Vec<u8> = rlp.val_at(1)?;
    ensure!(
        hash_bytes.len() == 32,
        "NETWORK_GET_PILLAR_VOTES_BUNDLE_INVALID_HASH_LENGTH"
    );
    let mut pillar_block_hash = [0_u8; 32];
    pillar_block_hash.copy_from_slice(&hash_bytes);
    ensure!(
        encode_get_pillar_votes_bundle_packet(period, pillar_block_hash) == bytes,
        "NETWORK_GET_PILLAR_VOTES_BUNDLE_NON_CANONICAL_RLP"
    );
    Ok(GetPillarVotesBundleQuery {
        period,
        pillar_block_hash,
    })
}

fn malformed_status_packet_report() -> NetworkStatusPacketReport {
    NetworkStatusPacketReport {
        status: NETWORK_INGRESS_STATUS_MALFORMED_PACKET,
        error_code: "NETWORK_STATUS_PACKET_MALFORMED_RLP".to_owned(),
        malicious: true,
        initial: false,
        accept_peer: false,
        disconnect_peer: true,
        peer_pbft_chain_size: 0,
        peer_pbft_period: 0,
        peer_pbft_round: 0,
        peer_dag_level: 0,
        peer_syncing: false,
        peer_is_light_node: false,
        peer_light_node_history: 0,
        node_major_version: 0,
        node_minor_version: 0,
        node_patch_version: 0,
        request_pbft_sync: false,
        request_pending_dag_blocks: false,
        request_next_votes: false,
        next_votes_period: 0,
        next_votes_round: 0,
        next_votes_request_rlp: Vec::new(),
        sync_generation: 0,
    }
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
    queued_effect_count: u32,
) -> Vec<NetworkIngressDecision> {
    (0..vote_count)
        .map(|index| NetworkIngressDecision {
            payload_id: context.source_payload_id,
            payload_accepted: context.source_payload_id != 0,
            routed: true,
            status,
            error_code: error_code.to_owned(),
            queued_effect_count: if index == 0 { queued_effect_count } else { 0 },
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
            vote_hash: [0; 32],
            vote_rlp: Vec::new(),
            pbft_block_rlp: Vec::new(),
            pbft_block_hash: [0; 32],
            pbft_block_period: 0,
        }
    }

    fn pillar_vote_context() -> NetworkPillarVoteIngressContext {
        NetworkPillarVoteIngressContext {
            transport_lane: 6,
            peer_id: peer(8),
            source_payload_id: 101,
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

    fn pillar_admission(
        vote_rlp: &[u8],
        accepted: bool,
        duplicate: bool,
    ) -> PillarVoteSingleAdmissionWithFinalChainPlan {
        let inspection = inspect_pillar_vote_from_rlp(vote_rlp).unwrap();
        PillarVoteSingleAdmissionWithFinalChainPlan {
            status: 0,
            accepted,
            duplicate,
            conflict_found: false,
            conflicting_vote_hash: [0; 32],
            block_weight: u64::from(accepted),
            validator_vote_count: u64::from(accepted),
            period: inspection.period,
            vote_hash: inspection.vote_hash.to_fixed_bytes(),
            voter: inspection.voter.to_fixed_bytes(),
        }
    }

    fn pillar_request(period: u64, block_hash: H256) -> NetworkGetPillarVotesBundlePacketRequest {
        NetworkGetPillarVotesBundlePacketRequest {
            transport_lane: 6,
            peer_id: peer(8),
            source_payload_id: 102,
            packet_rlp: encode_get_pillar_votes_bundle_packet(period, block_hash.into()),
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

    fn structurally_valid_unrecoverable_bundle(block_hash: H256, period: u64) -> Vec<u8> {
        let mut optimized_vote = RlpStream::new_list(2);
        optimized_vote.append(&[0x41_u8; 80].as_slice());
        optimized_vote.append(&[0_u8; 65].as_slice());
        let mut bundle = RlpStream::new_list(5);
        bundle.append(&block_hash);
        bundle.append(&period);
        bundle.append(&0_u64);
        bundle.append(&3_u64);
        bundle.begin_list(1);
        bundle.append_raw(&optimized_vote.out(), 1);
        bundle.out().to_vec()
    }

    fn signed_pbft_sync_block(period: u64, order_hash: H256) -> Vec<u8> {
        fn append_fields(stream: &mut RlpStream, period: u64, order_hash: H256) {
            stream.append(&H256::zero());
            stream.append(&H256::zero());
            stream.append(&order_hash);
            stream.append(&H256::zero());
            stream.append(&period);
            stream.append(&7_u64);
            stream.begin_list(0);
        }

        let signing_key = SigningKey::from_slice(&[0x6a; 32]).unwrap();
        let mut unsigned = RlpStream::new_list(7);
        append_fields(&mut unsigned, period, order_hash);
        let mut digest = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&unsigned.out());
        hasher.finalize(&mut digest);
        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
        let mut signature = signature.to_bytes().to_vec();
        signature.push(recovery_id.to_byte());

        let mut block = RlpStream::new_list(8);
        append_fields(&mut block, period, order_hash);
        block.append(&signature);
        block.out().to_vec()
    }

    fn decoded_pbft_sync_packet(
        period: u64,
        order_hash: H256,
        previous_cert_bundle: Option<&[u8]>,
        current_cert_bundle: Option<&[u8]>,
    ) -> DecodedPbftSyncPacketPrecheck {
        let block = signed_pbft_sync_block(period, order_hash);
        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(&block, 1);
        if let Some(bundle) = previous_cert_bundle {
            period_data.append_raw(bundle, 1);
        } else {
            period_data.append_empty_data();
        }
        period_data.append_empty_data();
        period_data.begin_list(0);

        let mut packet = RlpStream::new_list(3);
        packet.append(&true);
        packet.append_raw(&period_data.out(), 1);
        if let Some(bundle) = current_cert_bundle {
            packet.append_raw(bundle, 1);
        } else {
            packet.append(&0_u8);
        }
        decode_pbft_sync_packet_precheck(&packet.out()).unwrap()
    }

    fn classify_test_pbft_sync_packet(
        packet: DecodedPbftSyncPacketPrecheck,
        block_in_chain: bool,
        syncing_period: u64,
        last_block_hash: H256,
        ficus_activation_period: u64,
    ) -> NetworkIngressDecision {
        classify_pbft_sync_packet_precheck(
            packet,
            77,
            block_in_chain,
            syncing_period,
            last_block_hash,
            ficus_activation_period,
            10,
        )
    }

    #[test]
    fn pbft_sync_precheck_classifies_accepted_and_duplicate_packets() {
        let accepted = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::zero(), None, None),
            false,
            0,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(accepted.status, NETWORK_INGRESS_STATUS_ACCEPTED);

        let duplicate = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::zero(), None, None),
            true,
            0,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(
            duplicate.status,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_DUPLICATE_BLOCK
        );
    }

    #[test]
    fn pbft_sync_precheck_classifies_sync_complete_and_unexpected_period() {
        let block = signed_pbft_sync_block(1, H256::zero());
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block)).unwrap();
        let current_bundle = optimized_bundle(link.block_hash.into(), 1, 0, 1, 0x71);
        let complete = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::zero(), None, Some(&current_bundle)),
            false,
            1,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(complete.status, NETWORK_INGRESS_STATUS_PBFT_SYNC_COMPLETE);

        let unexpected = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::zero(), None, None),
            false,
            4,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(
            unexpected.status,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_UNEXPECTED_PERIOD
        );
    }

    #[test]
    fn pbft_sync_precheck_preserves_duplicate_and_drop_before_signature_recovery() {
        let block = signed_pbft_sync_block(1, H256::zero());
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block)).unwrap();
        let invalid_bundle = structurally_valid_unrecoverable_bundle(link.block_hash, 1);

        let duplicate = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::zero(), None, Some(&invalid_bundle)),
            true,
            0,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(
            duplicate.status,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_DUPLICATE_BLOCK
        );

        let unexpected = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::zero(), None, Some(&invalid_bundle)),
            false,
            4,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(
            unexpected.status,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_UNEXPECTED_PERIOD
        );

        let accepted_height = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::zero(), None, Some(&invalid_bundle)),
            false,
            0,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(
            accepted_height.status,
            NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS
        );
        assert_eq!(
            accepted_height.error_code,
            ERROR_PBFT_SYNC_PACKET_CERT_SIGNATURE
        );
    }

    #[test]
    fn pbft_sync_precheck_classifies_certificate_hash_mismatches_as_malicious() {
        let wrong_current = optimized_bundle(hash(0x91), 1, 0, 1, 0x72);
        let current = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::zero(), None, Some(&wrong_current)),
            false,
            0,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(current.status, NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS);
        assert_eq!(current.error_code, ERROR_PBFT_SYNC_PACKET_CURRENT_CERT_HASH);

        let previous_bundle = optimized_bundle(hash(0x92), 1, 0, 1, 0x73);
        let previous = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(2, H256::zero(), Some(&previous_bundle), None),
            false,
            1,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(previous.status, NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS);
        assert_eq!(
            previous.error_code,
            ERROR_PBFT_SYNC_PACKET_PREVIOUS_CERT_HASH
        );
    }

    #[test]
    fn pbft_sync_precheck_classifies_pillar_schedule_and_order_hash_as_malicious() {
        let schedule = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::zero(), None, None),
            false,
            0,
            H256::zero(),
            1,
        );
        assert_eq!(schedule.status, NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS);
        assert_eq!(schedule.error_code, ERROR_PBFT_SYNC_PACKET_PILLAR_SCHEDULE);

        let order = classify_test_pbft_sync_packet(
            decoded_pbft_sync_packet(1, H256::repeat_byte(0x44), None, None),
            false,
            0,
            H256::zero(),
            u64::MAX,
        );
        assert_eq!(order.status, NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS);
        assert_eq!(order.error_code, ERROR_PBFT_SYNC_PACKET_ORDER_HASH);
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
    fn canonical_vote_packet_decoder_preserves_vote_and_optional_chain_facts() {
        let vote = canonical_bundle_vote(0x31);
        let mut packet = RlpStream::new_list(2);
        packet.append_raw(&vote.vote_rlp, 1);
        packet.append(&0_u8);
        let decoded = decode_pbft_vote_packet(&packet.out()).unwrap();
        assert_eq!(decoded.vote_rlp, vote.vote_rlp);
        assert!(decoded.pbft_block_rlp.is_none());
        assert_eq!(decoded.peer_pbft_chain_size, None);

        let block = signed_pbft_block(0x32, 10, 100);
        let mut optional = RlpStream::new_list(2);
        optional.append_raw(&block, 1);
        optional.append(&44_u64);
        let mut packet = RlpStream::new_list(2);
        packet.append_raw(&vote.vote_rlp, 1);
        packet.append_raw(&optional.out(), 1);
        let decoded = decode_pbft_vote_packet(&packet.out()).unwrap();
        assert_eq!(decoded.pbft_block_rlp, Some(block));
        assert_eq!(decoded.peer_pbft_chain_size, Some(44));
    }

    #[test]
    fn canonical_optimized_vote_packet_decoder_reconstructs_signed_votes() {
        let vote = canonical_bundle_vote(0x33);
        let inspected = inspect_canonical_pbft_vote(&vote.vote_rlp).unwrap();
        let optimized = build_optimized_pbft_vote_bundle(
            std::slice::from_ref(&vote),
            inspected.block_hash,
            inspected.period,
            inspected.round,
            inspected.step,
        )
        .unwrap();
        let mut packet = RlpStream::new_list(1);
        packet.append_raw(&optimized.bundle_rlp, 1);

        let decoded = decode_pbft_votes_bundle_packet(&packet.out()).unwrap();
        assert_eq!(decoded, vec![vote.vote_rlp]);
        assert!(decode_pbft_votes_bundle_packet(&[0xc0]).is_err());
    }

    #[test]
    fn canonical_optimized_pillar_packet_wrapper_decodes_without_cpp_objects() {
        let votes = vec![
            signed_pillar_vote(0x34, 20, 8),
            signed_pillar_vote(0x35, 20, 8),
        ];
        let optimized = encode_optimized_pillar_votes_bundle_rlp(&votes).unwrap();
        let mut packet = RlpStream::new_list(1);
        packet.append_raw(&optimized, 1);
        let inner = decode_single_wrapped_packet(&packet.out()).unwrap();
        assert_eq!(
            decode_optimized_pillar_votes_bundle_rlp(&inner).unwrap(),
            votes
        );
    }

    fn signed_pbft_block_with_options(
        seed: u8,
        period: u64,
        timestamp: u64,
        reward_votes: &[H256],
        extra_data: Option<&[u8]>,
        recovery_id_override: Option<u8>,
    ) -> Vec<u8> {
        fn append_fields(
            stream: &mut RlpStream,
            period: u64,
            timestamp: u64,
            reward_votes: &[H256],
            extra_data: Option<&[u8]>,
        ) {
            stream.append(&H256::from_low_u64_be(10));
            stream.append(&H256::from_low_u64_be(11));
            stream.append(&H256::from_low_u64_be(12));
            stream.append(&H256::from_low_u64_be(13));
            stream.append(&period);
            stream.append(&timestamp);
            stream.begin_list(reward_votes.len());
            for vote in reward_votes {
                stream.append(vote);
            }
            if let Some(extra_data) = extra_data {
                stream.append(&extra_data);
            }
        }

        let signing_key = SigningKey::from_slice(&[seed; 32]).unwrap();
        let unsigned_fields = 7 + usize::from(extra_data.is_some());
        let mut unsigned = RlpStream::new_list(unsigned_fields);
        append_fields(&mut unsigned, period, timestamp, reward_votes, extra_data);
        let mut digest = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&unsigned.out());
        hasher.finalize(&mut digest);
        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
        let mut signature = signature.to_bytes().to_vec();
        signature.push(recovery_id_override.unwrap_or_else(|| recovery_id.to_byte()));

        let mut signed = RlpStream::new_list(unsigned_fields + 1);
        append_fields(&mut signed, period, timestamp, reward_votes, extra_data);
        signed.append(&signature);
        signed.out().to_vec()
    }

    fn signed_pbft_block(seed: u8, period: u64, timestamp: u64) -> Vec<u8> {
        signed_pbft_block_with_options(seed, period, timestamp, &[], None, None)
    }

    fn pbft_blocks_bundle(blocks: &[Vec<u8>]) -> Vec<u8> {
        let mut encoded_blocks = RlpStream::new_list(blocks.len());
        for block in blocks {
            encoded_blocks.append_raw(block, 1);
        }
        let mut packet = RlpStream::new_list(1);
        packet.append_raw(&encoded_blocks.out(), 1);
        packet.out().to_vec()
    }

    #[test]
    fn proposed_block_bundle_rejects_malformed_and_oversized_packets_before_callbacks() {
        let mut eligibility_calls = 0;
        let mut publication_calls = 0;
        let malformed = admit_pbft_blocks_bundle(
            &[0xc0],
            41,
            10,
            10,
            |_, _| {
                eligibility_calls += 1;
                Ok(true)
            },
            |_, _| {
                publication_calls += 1;
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(
            malformed.status,
            NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_MALFORMED
        );

        let block = signed_pbft_block(9, 10, 1);
        let oversized = admit_pbft_blocks_bundle(
            &pbft_blocks_bundle(&vec![block; 11]),
            42,
            10,
            10,
            |_, _| {
                eligibility_calls += 1;
                Ok(true)
            },
            |_, _| {
                publication_calls += 1;
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(
            oversized.status,
            NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_TOO_LARGE
        );

        let malformed_member = admit_pbft_blocks_bundle(
            &pbft_blocks_bundle(&[signed_pbft_block(9, 10, 1), vec![0xc0]]),
            47,
            10,
            10,
            |_, _| {
                eligibility_calls += 1;
                Ok(true)
            },
            |_, _| {
                publication_calls += 1;
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(
            malformed_member.status,
            NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_MALFORMED
        );
        assert_eq!((eligibility_calls, publication_calls), (0, 0));
    }

    #[test]
    fn proposed_block_bundle_rejects_legacy_constructor_invariant_violations() {
        let duplicate_reward = H256::from_low_u64_be(99);
        let duplicate_rewards = signed_pbft_block_with_options(
            9,
            10,
            1,
            &[duplicate_reward, duplicate_reward],
            None,
            None,
        );
        let invalid_recovery_id = signed_pbft_block_with_options(9, 10, 1, &[], None, Some(4));
        let oversized_extra = vec![0_u8; MAX_PBFT_BLOCK_EXTRA_DATA_BYTES + 1];
        let oversized_extra =
            signed_pbft_block_with_options(9, 10, 1, &[], Some(&oversized_extra), None);

        for block in [duplicate_rewards, invalid_recovery_id, oversized_extra] {
            let decision = admit_pbft_blocks_bundle(
                &pbft_blocks_bundle(&[block]),
                48,
                10,
                10,
                |_, _| panic!("malformed block must not query DPoS"),
                |_, _| panic!("malformed block must not publish"),
            )
            .unwrap();
            assert_eq!(
                decision.status,
                NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_MALFORMED
            );
        }
    }

    #[test]
    fn proposed_block_bundle_ignores_irrelevant_blocks_before_eligibility() {
        let blocks = vec![signed_pbft_block(7, 9, 1), signed_pbft_block(7, 16, 2)];
        let decision = admit_pbft_blocks_bundle(
            &pbft_blocks_bundle(&blocks),
            43,
            10,
            100,
            |_, _| panic!("irrelevant blocks must not query DPoS"),
            |_, _| panic!("irrelevant blocks must not publish"),
        )
        .unwrap();
        assert_eq!(decision.status, NETWORK_INGRESS_STATUS_ACCEPTED);
    }

    #[test]
    fn proposed_block_bundle_preserves_sequential_publication_before_late_duplicate() {
        let blocks = vec![signed_pbft_block(8, 10, 1), signed_pbft_block(8, 10, 2)];
        let mut published = Vec::new();
        let decision = admit_pbft_blocks_bundle(
            &pbft_blocks_bundle(&blocks),
            44,
            10,
            10,
            |_, _| Ok(true),
            |link, _| {
                published.push(link.block_hash);
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(
            decision.status,
            NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_DUPLICATE_AUTHOR
        );
        assert_eq!(published.len(), 1);
    }

    #[test]
    fn proposed_block_bundle_uses_head_gate_and_rejects_ineligible_author() {
        let block = signed_pbft_block(6, 12, 1);
        let packet = pbft_blocks_bundle(&[block]);
        let mut queried = Vec::new();
        let mut published = 0;
        let accepted = admit_pbft_blocks_bundle(
            &packet,
            45,
            12,
            10,
            |period, _| {
                queried.push(period);
                Ok(false)
            },
            |_, _| {
                published += 1;
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(accepted.status, NETWORK_INGRESS_STATUS_ACCEPTED);
        assert!(queried.is_empty());
        assert_eq!(published, 1);

        let rejected = admit_pbft_blocks_bundle(
            &packet,
            46,
            12,
            11,
            |period, _| {
                queried.push(period);
                Ok(false)
            },
            |_, _| panic!("ineligible block must not publish"),
        )
        .unwrap();
        assert_eq!(
            rejected.status,
            NETWORK_INGRESS_STATUS_PBFT_BLOCKS_BUNDLE_INELIGIBLE_AUTHOR
        );
        assert_eq!(queried, [11]);
    }

    #[test]
    fn proposed_block_bundle_propagates_eligibility_lookup_failure() {
        let packet = pbft_blocks_bundle(&[signed_pbft_block(6, 12, 1)]);
        let error = admit_pbft_blocks_bundle(
            &packet,
            47,
            12,
            11,
            |_, _| Err(anyhow!("FINAL_CHAIN_DPOS_LOOKUP_FAILED")),
            |_, _| panic!("lookup failure must not publish or become a peer-fault decision"),
        )
        .expect_err("operational FinalChain failure propagates");

        assert!(error.to_string().contains("FINAL_CHAIN_DPOS_LOOKUP_FAILED"));
    }

    #[test]
    fn pillar_vote_ingress_releases_source_known_only_after_acceptance() {
        let mut api = ConsensusNetworkApi::new();
        let vote = signed_pillar_vote(0x41, 11, 90);
        let vote_rlp = vote.encode_rlp();
        let decision = api.ingest_pillar_vote_bundle(pillar_vote_context(), vec![vote_rlp.clone()]);

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
        assert_eq!(follow_ups.effects.len(), 1);
        assert_eq!(
            follow_ups.effects[0].kind,
            NETWORK_EFFECT_KIND_MARK_PEER_KNOWN
        );
    }

    #[test]
    fn pillar_vote_bundle_preflight_is_atomic_for_malformed_and_duplicate_members() {
        let mut api = ConsensusNetworkApi::new();
        let vote = signed_pillar_vote(0x42, 11, 91).encode_rlp();

        let malformed =
            api.ingest_pillar_vote_bundle(pillar_vote_context(), vec![vote.clone(), vec![0x7f]]);
        assert_eq!(malformed.len(), 2);
        assert!(malformed.iter().all(|decision| {
            decision.status == NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_RLP
                && decision.application_effect_id == 0
        }));
        assert!(api.drain_work(6, 10).effects.is_empty());

        let duplicate =
            api.ingest_pillar_vote_bundle(pillar_vote_context(), vec![vote.clone(), vote]);
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
            pillar_vote_context(),
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
    fn composed_pillar_bundle_returns_terminal_member_admission_facts() {
        let api = Arc::new(Mutex::new(ConsensusNetworkApi::with_pillar_schedule(
            10, 10,
        )));
        let votes = vec![
            signed_pillar_vote(0x53, 11, 102).encode_rlp(),
            signed_pillar_vote(0x54, 12, 103).encode_rlp(),
        ];
        let mut member = 0usize;
        let outcomes = ConsensusNetworkService::ingest_and_admit_pillar_vote_bundle_with(
            &api,
            pillar_vote_context(),
            votes.clone(),
            |vote_rlp| {
                let result = pillar_admission(vote_rlp, member == 0, member == 1);
                member += 1;
                Ok(result)
            },
        )
        .unwrap();

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|outcome| {
            outcome.decision.application_effect_id == 0 && outcome.admission.is_some()
        }));
        let first = outcomes[0].admission.as_ref().unwrap();
        assert!(first.accepted);
        assert!(!first.duplicate);
        assert_eq!(
            first.vote_hash,
            inspect_pillar_vote_from_rlp(&votes[0]).unwrap().vote_hash.0
        );
        let second = outcomes[1].admission.as_ref().unwrap();
        assert!(!second.accepted);
        assert!(second.duplicate);
        assert_eq!(second.status, 0);
    }

    #[test]
    fn composed_pillar_bundle_member_failure_cancels_all_operation_follow_ups() {
        let api = Arc::new(Mutex::new(ConsensusNetworkApi::with_pillar_schedule(
            10, 10,
        )));
        let votes = vec![
            signed_pillar_vote(0x55, 11, 104).encode_rlp(),
            signed_pillar_vote(0x56, 12, 105).encode_rlp(),
            signed_pillar_vote(0x57, 13, 106).encode_rlp(),
        ];
        let mut member = 0usize;
        let error = ConsensusNetworkService::ingest_and_admit_pillar_vote_bundle_with(
            &api,
            pillar_vote_context(),
            votes,
            |vote_rlp| {
                let index = member;
                member += 1;
                if index == 1 {
                    return Err(anyhow!("INJECTED_PILLAR_ADMISSION_FAILURE"));
                }
                Ok(pillar_admission(vote_rlp, true, false))
            },
        )
        .expect_err("second native admission fails");
        assert_eq!(error.to_string(), "INJECTED_PILLAR_ADMISSION_FAILURE");

        {
            let mut locked_api = api.lock().unwrap();
            assert!(locked_api.pending_pillar_vote_admissions.is_empty());
            let remaining = locked_api.drain_work(6, 10).effects;
            assert!(remaining.iter().all(|effect| {
                effect.kind != NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT
                    || effect.object_kind != NETWORK_OBJECT_KIND_PILLAR_VOTE
            }));
            assert!(remaining.is_empty());
        }

        let next = signed_pillar_vote(0x58, 14, 107).encode_rlp();
        let outcomes = ConsensusNetworkService::ingest_and_admit_pillar_vote_bundle_with(
            &api,
            pillar_vote_context(),
            vec![next.clone()],
            |vote_rlp| Ok(pillar_admission(vote_rlp, true, false)),
        )
        .expect("the next packet on the lane remains serviceable");
        assert_eq!(outcomes.len(), 1);
        let follow_ups = api.lock().unwrap().drain_work(6, 10).effects;
        assert_eq!(follow_ups.len(), 1);
        assert!(follow_ups.iter().all(|effect| {
            effect.object_hash
                == inspect_pillar_vote_from_rlp(&next)
                    .unwrap()
                    .vote_hash
                    .to_fixed_bytes()
        }));
    }

    #[test]
    fn preactivation_pillar_bundle_queues_report_then_disconnect() {
        let mut api = ConsensusNetworkApi::with_pillar_schedule(20, 10);
        let decisions = api.ingest_pillar_vote_bundle(
            pillar_vote_context(),
            vec![
                signed_pillar_vote(0x59, 21, 108).encode_rlp(),
                signed_pillar_vote(0x5a, 19, 109).encode_rlp(),
            ],
        );
        assert_eq!(decisions.len(), 2);
        assert_eq!(
            decisions[0].status,
            NETWORK_INGRESS_STATUS_PILLAR_VOTE_INVALID_CONTEXT
        );
        assert_eq!(decisions[0].application_effect_id, 0);
        assert_eq!(decisions[0].queued_effect_count, 2);
        assert_eq!(decisions[1].queued_effect_count, 0);

        let report = api.drain_work(6, 10);
        assert_eq!(report.effects.len(), 1);
        assert_eq!(report.effects[0].kind, NETWORK_EFFECT_KIND_REPORT_PEER);
        assert_eq!(report.effects[0].period, 19);
        assert_eq!(
            report.effects[0].object_hash,
            inspect_pillar_vote_from_rlp(&signed_pillar_vote(0x5a, 19, 109).encode_rlp())
                .unwrap()
                .vote_hash
                .to_fixed_bytes()
        );
        assert_eq!(
            report.effects[0].reason_code,
            NETWORK_REASON_PREACTIVATION_PILLAR_VOTE
        );
        let accepted = effect_result(&report.effects[0], NETWORK_EFFECT_RESULT_STATUS_OK);
        assert_eq!(
            api.report_effect_results(vec![accepted]).status,
            NETWORK_EFFECT_ACK_STATUS_ACCEPTED
        );
        let disconnect = api.drain_work(6, 10);
        assert_eq!(disconnect.effects.len(), 1);
        assert_eq!(
            disconnect.effects[0].kind,
            NETWORK_EFFECT_KIND_DISCONNECT_PEER
        );
    }

    #[test]
    fn failed_pillar_vote_admission_clears_context_without_follow_ups() {
        let mut api = ConsensusNetworkApi::new();
        api.ingest_pillar_vote_bundle(
            pillar_vote_context(),
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
        let query = decode_get_pillar_votes_bundle_packet(&request.packet_rlp).unwrap();
        let decision = api
            .reject_invalid_pillar_votes_request(&request, &query)
            .unwrap();
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
        let valid = pillar_request(11, H256::from_low_u64_be(90));
        let valid_query = decode_get_pillar_votes_bundle_packet(&valid.packet_rlp).unwrap();
        assert!(
            api.reject_invalid_pillar_votes_request(&valid, &valid_query)
                .is_none()
        );
        let invalid = pillar_request(12, H256::from_low_u64_be(90));
        let invalid_query = decode_get_pillar_votes_bundle_packet(&invalid.packet_rlp).unwrap();
        let decision = api
            .reject_invalid_pillar_votes_request(&invalid, &invalid_query)
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
        let request = pillar_request(11, block_hash);
        let query = decode_get_pillar_votes_bundle_packet(&request.packet_rlp).unwrap();
        let decision = api.enqueue_pillar_vote_bundle_send_effects(request, query, chunks);
        assert_eq!(decision.queued_effect_count, 3);
        let send = api.drain_work(6, 10);
        assert_eq!(send.effects.len(), 1);
        assert_eq!(send.effects[0].kind, NETWORK_EFFECT_KIND_SEND_PACKET);
        assert_eq!(
            send.effects[0].packet_kind,
            NETWORK_PACKET_KIND_PILLAR_VOTES_BUNDLE
        );
        assert_eq!(
            rustaxa_types::decode_optimized_pillar_votes_bundle_rlp(
                Rlp::new(&send.effects[0].payload_bytes)
                    .at(0)
                    .unwrap()
                    .as_raw()
            )
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
    fn failed_pillar_bundle_send_suppresses_dependent_known_marks() {
        let block_hash = H256::from_low_u64_be(91);
        let vote = signed_pillar_vote(0x48, 11, 91);
        let chunks =
            validate_and_chunk_pillar_votes(vec![pillar_record(&vote)], 11, block_hash.into())
                .unwrap();
        let mut api = ConsensusNetworkApi::with_pillar_schedule(10, 10);
        let request = pillar_request(11, block_hash);
        let query = decode_get_pillar_votes_bundle_packet(&request.packet_rlp).unwrap();
        api.enqueue_pillar_vote_bundle_send_effects(request, query, chunks);
        let send = api.drain_work(6, 10).effects;
        assert_eq!(send.len(), 1);
        api.report_effect_results(vec![effect_result(
            &send[0],
            NETWORK_EFFECT_RESULT_STATUS_FAILED,
        )]);
        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn pillar_bundle_chunking_wraps_complete_packets_at_two_hundred_fifty_votes() {
        let block_hash = H256::from_low_u64_be(91);
        let records = (1_u16..=251)
            .map(|index| pillar_record(&signed_pillar_vote(index as u8, 11, 91)))
            .collect();
        let chunks = validate_and_chunk_pillar_votes(records, 11, block_hash.into()).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].vote_hashes.len(), 250);
        assert_eq!(chunks[1].vote_hashes.len(), 1);
        for (chunk, expected) in chunks.iter().zip([250, 1]) {
            let packet = Rlp::new(&chunk.payload_bytes);
            assert_eq!(packet.item_count().unwrap(), 1);
            let votes =
                decode_optimized_pillar_votes_bundle_rlp(packet.at(0).unwrap().as_raw()).unwrap();
            assert_eq!(votes.len(), expected);
        }
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
        for (request, current_period, current_round, expected_status) in [
            (
                NetworkPbftNextVotesBundleRequest {
                    peer_period: 9,
                    ..next_votes_request()
                },
                10,
                3,
                NETWORK_INGRESS_STATUS_NEXT_VOTES_PERIOD_MISMATCH,
            ),
            (
                next_votes_request(),
                10,
                1,
                NETWORK_INGRESS_STATUS_NEXT_VOTES_NO_PREVIOUS_ROUND,
            ),
            (
                NetworkPbftNextVotesBundleRequest {
                    peer_round: 4,
                    ..next_votes_request()
                },
                10,
                3,
                NETWORK_INGRESS_STATUS_NEXT_VOTES_PEER_ROUND_AHEAD,
            ),
        ] {
            let decision =
                next_votes_request_rejection(&request, current_period, current_round).unwrap();
            assert!(decision.routed);
            assert_eq!(decision.status, expected_status);
            assert_eq!(decision.queued_effect_count, 0);
            assert_eq!(decision.application_effect_id, 0);
        }
        assert!(next_votes_request_rejection(&next_votes_request(), 10, 3).is_none());
    }

    #[test]
    fn status_codec_matches_legacy_optional_none_and_rejects_non_canonical_bytes() {
        let request = NetworkStatusPacketBuildRequest {
            initial: false,
            local_pbft_chain_size: 10,
            local_pbft_round: 2,
            local_dag_level: 44,
        };
        let encoded = encode_status_packet(request, false, None);
        assert_eq!(encoded, vec![0xc5, 0x0a, 0x02, 0x2c, 0x80, 0x80]);
        assert_eq!(
            decode_status_packet(&encoded).unwrap(),
            DecodedStatusPacket {
                peer_pbft_chain_size: 10,
                peer_pbft_round: 2,
                peer_dag_level: 44,
                peer_syncing: false,
                initial_data: None,
            }
        );
        assert!(decode_status_packet(&[0xc6, 0x81, 0x0a, 0x02, 0x2c, 0x80, 0x80]).is_err());
    }

    #[test]
    fn status_codec_preserves_exact_bootstrap_identity() {
        let identity = NetworkNodeIdentity {
            chain_id: u64::MAX,
            genesis_hash: [0x5a; 32],
            node_major_version: 2,
            node_minor_version: 4,
            node_patch_version: 6,
            is_light_node: true,
            light_node_history: 99,
        };
        let encoded = encode_status_packet(
            NetworkStatusPacketBuildRequest {
                initial: true,
                local_pbft_chain_size: 17,
                local_pbft_round: 8,
                local_dag_level: 21,
            },
            true,
            Some(&identity),
        );
        let decoded = decode_status_packet(&encoded).unwrap();
        assert_eq!(
            decoded.initial_data.unwrap().genesis_hash,
            identity.genesis_hash
        );
        assert_eq!(decoded.peer_pbft_chain_size, 17);
        assert!(decoded.peer_syncing);
    }

    #[test]
    fn get_next_votes_codec_is_canonical_and_exact() {
        let encoded = encode_get_next_votes_packet(10, 2);
        assert_eq!(encoded, vec![0xc2, 0x0a, 0x02]);
        assert_eq!(decode_get_next_votes_packet(&encoded).unwrap(), (10, 2));
        assert!(decode_get_next_votes_packet(&[0xc3, 0x81, 0x0a, 0x02]).is_err());
        assert!(decode_get_next_votes_packet(&[0xc1, 0x0a]).is_err());
    }

    #[test]
    fn get_pillar_votes_codec_is_canonical_and_exact() {
        let hash = H256::from_low_u64_be(91);
        let encoded = encode_get_pillar_votes_bundle_packet(11, hash.into());
        let decoded = decode_get_pillar_votes_bundle_packet(&encoded).unwrap();
        assert_eq!(decoded.period, 11);
        assert_eq!(decoded.pillar_block_hash, hash.to_fixed_bytes());
        assert!(decode_get_pillar_votes_bundle_packet(&[0xc1, 0x0b]).is_err());
        let mut noncanonical = RlpStream::new_list(2);
        noncanonical.append_raw(&[0x81, 0x0b], 1);
        noncanonical.append(&hash);
        assert!(decode_get_pillar_votes_bundle_packet(&noncanonical.out()).is_err());
        let mut short_hash = RlpStream::new_list(2);
        short_hash.append(&11_u64);
        short_hash.append(&[7_u8; 31].as_slice());
        assert!(decode_get_pillar_votes_bundle_packet(&short_hash.out()).is_err());
    }

    #[test]
    fn outbound_pillar_request_selects_exact_lane_and_builds_complete_packet() {
        let mut api = ConsensusNetworkApi::with_pillar_schedule(10, 10);
        let hash = H256::from_low_u64_be(91);
        let preparation = api
            .prepare_egress(
                NetworkEgressPrepareRequest {
                    family: NETWORK_EGRESS_FAMILY_PILLAR_VOTES_REQUEST,
                    transport_lane: 0,
                    source_payload_id: 104,
                    source_peer_id: [0; 64],
                    rebroadcast: false,
                    object_hash: hash.into(),
                    payload_bytes: 11_u64.to_be_bytes().to_vec(),
                    related_payload_bytes: 10_u64.to_be_bytes().to_vec(),
                },
                Vec::new(),
                Vec::new(),
                0,
            )
            .unwrap();
        assert!(preparation.probes.is_empty());
        let decision = api
            .plan_egress(NetworkEgressPlanRequest {
                token: preparation.token,
                peers: vec![
                    NetworkEgressPeerSnapshot {
                        transport_lane: 5,
                        peer_id: peer(1),
                        pbft_chain_size: 12,
                        dag_level: 2,
                        ..Default::default()
                    },
                    NetworkEgressPeerSnapshot {
                        transport_lane: 6,
                        peer_id: peer(2),
                        pbft_chain_size: 13,
                        dag_level: 1,
                        ..Default::default()
                    },
                ],
            })
            .unwrap();
        assert_eq!(decision.queued_effect_count, 1);
        let effect = api.drain_work(6, 10).effects.pop().unwrap();
        assert_eq!(effect.peer_id, peer(2));
        assert_eq!(
            effect.packet_kind,
            NETWORK_PACKET_KIND_GET_PILLAR_VOTES_BUNDLE
        );
        assert_eq!(
            decode_get_pillar_votes_bundle_packet(&effect.payload_bytes).unwrap(),
            GetPillarVotesBundleQuery {
                period: 11,
                pillar_block_hash: hash.into(),
            }
        );
    }

    #[test]
    fn failed_next_vote_send_suppresses_its_known_mark() {
        let mut api = ConsensusNetworkApi::new();
        let chunks = validate_next_votes_payloads(
            PbftNextVotesBundleEgressPayloads {
                next_votes_bundle_rlp: optimized_bundle([0x44; 32], 10, 2, 1, 1),
                next_null_votes_bundle_rlp: Vec::new(),
            },
            10,
            2,
        )
        .unwrap();
        api.enqueue_next_votes_bundle_send_effects(next_votes_request(), 10, 3, chunks);
        let send = api.drain_work(6, 10).effects;
        assert_eq!(send.len(), 1);
        api.report_effect_results(vec![effect_result(
            &send[0],
            NETWORK_EFFECT_RESULT_STATUS_FAILED,
        )]);
        assert!(api.drain_work(6, 10).effects.is_empty());
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
        let decision =
            api.enqueue_next_votes_bundle_send_effects(next_votes_request(), 10, 3, chunks);
        assert_eq!(decision.application_effect_id, 0);
        assert_eq!(decision.queued_effect_count, 1005);

        let sends = api.drain_work(6, 8).effects;
        assert_eq!(sends.len(), 3);
        assert!(sends.iter().all(|effect| {
            effect.kind == NETWORK_EFFECT_KIND_SEND_PACKET
                && effect.packet_kind == NETWORK_PACKET_KIND_PBFT_VOTES_BUNDLE
                && effect.peer_id == peer(7)
                && effect.source_payload_id == 99
        }));
        let first_packet = Rlp::new(&sends[0].payload_bytes);
        let second_packet = Rlp::new(&sends[1].payload_bytes);
        let third_packet = Rlp::new(&sends[2].payload_bytes);
        assert_eq!(first_packet.item_count().unwrap(), 1);
        let first_bundle = first_packet.at(0).unwrap();
        let second_bundle = second_packet.at(0).unwrap();
        let third_bundle = third_packet.at(0).unwrap();
        let first: H256 = first_bundle.val_at(0).unwrap();
        let second: H256 = second_bundle.val_at(0).unwrap();
        let third: H256 = third_bundle.val_at(0).unwrap();
        assert_eq!(first, H256::from([0x44; 32]));
        assert_eq!(second, first);
        assert!(third.is_zero());
        assert_eq!(first_bundle.at(4).unwrap().item_count().unwrap(), 1000);
        assert_eq!(second_bundle.at(4).unwrap().item_count().unwrap(), 1);

        let send_ids = sends
            .iter()
            .map(|send| (send.effect_id, send.peer_id))
            .collect::<HashSet<_>>();
        api.report_effect_results(
            sends
                .iter()
                .map(|send| effect_result(send, NETWORK_EFFECT_RESULT_STATUS_OK))
                .collect(),
        );
        let marks = api.drain_work(6, 2_000).effects;
        assert_eq!(marks.len(), 1002);
        assert!(marks.iter().all(|mark| {
            mark.kind == NETWORK_EFFECT_KIND_MARK_PEER_KNOWN
                && mark.object_kind == NETWORK_OBJECT_KIND_PBFT_VOTE
                && send_ids.contains(&(mark.dependency_id, mark.peer_id))
        }));
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
        let mut facts = status_sync_facts();
        facts.peer_pbft_chain_size = 13;

        let plan = plan_status_sync(facts);

        assert!(plan.request_pbft_sync);
        assert!(!plan.request_pending_dag_blocks);
        assert!(!plan.request_next_votes);
    }

    #[test]
    fn plan_status_sync_debounces_one_block_pbft_sync() {
        let mut facts = status_sync_facts();
        facts.peer_pbft_chain_size = 11;
        facts.peer_last_status_pbft_chain_size = 10;

        assert!(!plan_status_sync(facts.clone()).request_pbft_sync);

        facts.peer_last_status_pbft_chain_size = 11;
        assert!(plan_status_sync(facts).request_pbft_sync);
    }

    #[test]
    fn plan_status_sync_requests_pending_dag_blocks_when_periods_match() {
        let mut facts = status_sync_facts();
        facts.peer_dag_synced = false;

        let plan = plan_status_sync(facts);

        assert!(!plan.request_pbft_sync);
        assert!(plan.request_pending_dag_blocks);
        assert!(!plan.request_next_votes);
    }

    #[test]
    fn plan_status_sync_requests_next_votes_when_peer_round_is_ahead() {
        let mut facts = status_sync_facts();
        facts.peer_pbft_round = 4;

        let plan = plan_status_sync(facts);

        assert!(!plan.request_pbft_sync);
        assert!(!plan.request_pending_dag_blocks);
        assert!(plan.request_next_votes);
        assert_eq!(plan.next_votes_period, 11);
        assert_eq!(plan.next_votes_round, 2);
    }

    #[test]
    fn plan_status_sync_returns_no_actions_while_local_pbft_syncing() {
        let mut facts = status_sync_facts();
        facts.local_pbft_syncing = true;
        facts.peer_pbft_chain_size = 13;
        facts.peer_pbft_round = 4;
        facts.peer_dag_synced = false;

        let plan = plan_status_sync(facts);

        assert!(!plan.request_pbft_sync);
        assert!(!plan.request_pending_dag_blocks);
        assert!(!plan.request_next_votes);
    }

    #[test]
    fn plan_initial_status_accepts_matching_status() {
        let plan = plan_initial_status(initial_status_facts());

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_OK);
        assert!(plan.accept_peer);
        assert!(!plan.disconnect_peer);
    }

    #[test]
    fn plan_initial_status_rejects_chain_id_mismatch() {
        let mut facts = initial_status_facts();
        facts.peer_chain_id = 8;

        let plan = plan_initial_status(facts);

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_CHAIN_ID_MISMATCH);
        assert!(!plan.accept_peer);
        assert!(plan.disconnect_peer);
    }

    #[test]
    fn plan_initial_status_rejects_genesis_mismatch() {
        let mut facts = initial_status_facts();
        facts.peer_genesis_hash = hash(2);

        let plan = plan_initial_status(facts);

        assert_eq!(plan.status, NETWORK_STATUS_PLAN_STATUS_GENESIS_MISMATCH);
        assert!(!plan.accept_peer);
        assert!(plan.disconnect_peer);
    }

    #[test]
    fn plan_initial_status_rejects_light_node_without_history() {
        let mut facts = initial_status_facts();
        facts.peer_is_light_node = true;
        facts.peer_light_node_history = 1;
        facts.peer_pbft_chain_size = 20;

        let plan = plan_initial_status(facts);

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
        assert_eq!(effect.sync_start, 10);
        assert_eq!(effect.payload_bytes, vec![0xc1, 0x0a]);
        assert_eq!(effect.source_payload_id, 99);

        let started = api.begin_pbft_sync(NetworkPbftSyncStartRequest {
            start: true,
            now_ms: 1_000,
            local_pbft_synced_period: effect.sync_start - 1,
            local_pbft_chain_size: effect.sync_start - 1,
            candidates: vec![sync_candidate(7, 20, 20)],
        });
        assert!(started.started);
        assert_eq!(started.peer_id, effect.peer_id);
        assert_eq!(started.request_period, effect.sync_start);
        assert_eq!(
            encode_get_pbft_sync_packet(started.request_period),
            effect.payload_bytes
        );
        let stopped = api.stop_pbft_sync(NetworkPbftSyncStopRequest {
            generation: started.generation,
            peer_id: started.peer_id,
            reason: NETWORK_PBFT_SYNC_STOP_REASON_TRANSPORT_FAILED,
        });
        assert!(stopped.stopped);
        assert!(!api.pbft_sync_status(1_001).active);
        assert_eq!(
            api.pbft_sync_status(1_001).last_stop_reason,
            NETWORK_PBFT_SYNC_STOP_REASON_TRANSPORT_FAILED
        );
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
    fn bundle_admission_releases_private_aggregation_without_generic_gossip() {
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

        assert!(api.drain_work(6, 10).effects.is_empty());
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

        assert_ne!(decisions[2].application_effect_id, 0);
        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn composed_bundle_infrastructure_error_cancels_every_unadmitted_effect() {
        let api = Arc::new(Mutex::new(ConsensusNetworkApi::new()));
        let reference = vote_fact(10, 3, 2, PbftVoteType::Soft);
        let votes = [
            canonical_bundle_vote(0x45),
            canonical_bundle_vote(0x46),
            canonical_bundle_vote(0x47),
        ];
        let error = ConsensusNetworkService::ingest_and_admit_pbft_vote_bundle_with(
            &api,
            reference,
            vec![reference; votes.len()],
            votes.iter().map(bundle_context).collect(),
            |_| Err(anyhow!("INJECTED_ADMISSION_FAILURE")),
        )
        .expect_err("injected application-root admission error");
        assert_eq!(error.to_string(), "INJECTED_ADMISSION_FAILURE");

        let mut api = api.lock().unwrap();
        assert!(api.pending_vote_admissions.is_empty());
        assert!(api.pending_vote_bundles.is_empty());
        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn bundle_failure_cleanup_cancels_accepted_prefix_followups() {
        let mut api = ConsensusNetworkApi::new();
        let reference = vote_fact(10, 3, 2, PbftVoteType::Soft);
        let votes = [canonical_bundle_vote(0x48), canonical_bundle_vote(0x49)];
        let decisions = api.ingest_pbft_vote_bundle(
            reference,
            vec![reference; votes.len()],
            votes.iter().map(bundle_context).collect(),
        );
        let first = api
            .take_native_vote_admission(decisions[0].application_effect_id)
            .expect("first bundle admission");
        let bundle_id = first.bundle.as_ref().expect("bundle member").bundle_id;
        let result = NetworkEffectResult {
            effect_id: 0,
            kind: NETWORK_EFFECT_KIND_RECORD_CONSENSUS_OBJECT,
            peer_id: first.peer_id,
            packet_kind: 0,
            object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
            object_hash: first.vote_hash,
            status: NETWORK_EFFECT_RESULT_STATUS_OK,
            diagnostic: String::new(),
            admission_accepted: true,
            admission_already_present: false,
            admission_mark_vote_known: true,
            admission_gossip_vote: true,
            admission_report_slashing: false,
        };
        let follow_up_effect_ids = api.enqueue_vote_admission_follow_ups(first, &result);
        assert!(!follow_up_effect_ids.is_empty());

        api.cancel_vote_bundle(bundle_id);
        api.cancel_pending_effects(&follow_up_effect_ids);
        assert!(api.pending_vote_admissions.is_empty());
        assert!(api.pending_vote_bundles.is_empty());
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
        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn post_admission_route_emits_source_known_follow_up_after_native_block_publication() {
        let mut api = ConsensusNetworkApi::new();
        let mut context = vote_context();
        context.transport_lane = 6;
        context.peer_id = peer(10);
        context.source_payload_id = 79;
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

        let dependents = api.drain_work(6, 10);
        assert_eq!(dependents.effects.len(), 1);
        assert_eq!(
            dependents.effects[0].kind,
            NETWORK_EFFECT_KIND_MARK_PEER_KNOWN
        );
        assert_eq!(
            dependents.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PBFT_BLOCK
        );
        assert_eq!(dependents.effects[0].dependency_id, 0);
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
            NETWORK_EFFECT_KIND_MARK_PEER_KNOWN
        );
        assert_eq!(
            dependent.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PBFT_BLOCK
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
    fn failed_block_known_ack_leaves_no_generic_gossip_after_native_publication() {
        let mut api = ConsensusNetworkApi::new();
        let mut context = vote_context();
        context.enqueue_admission = true;
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

        let block_known = api.drain_work(6, 1);
        assert_eq!(
            block_known.effects[0].object_kind,
            NETWORK_OBJECT_KIND_PBFT_BLOCK
        );
        api.report_effect_results(vec![effect_result(
            &block_known.effects[0],
            NETWORK_EFFECT_RESULT_STATUS_FAILED,
        )]);

        assert!(api.drain_work(6, 10).effects.is_empty());
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
    fn source_scoped_drain_retains_unrelated_same_lane_work() {
        let mut api = ConsensusNetworkApi::new();
        for (source_payload_id, byte) in [(41, 1), (42, 2), (41, 3)] {
            let mut context = vote_context();
            context.transport_lane = 6;
            context.enqueue_admission = true;
            context.peer_id = peer(byte);
            context.vote_hash = hash(byte);
            context.vote_rlp = vec![0xc1, byte];
            context.source_payload_id = source_payload_id;
            api.ingest_pbft_vote(vote_fact(10, 3, 2, PbftVoteType::Soft), context);
        }

        let scoped = api.drain_work_matching(6, Some(41), 10);
        assert_eq!(scoped.effects.len(), 2);
        assert_eq!(scoped.effects[0].source_payload_id, 41);
        assert_eq!(scoped.effects[1].source_payload_id, 41);
        assert!(!scoped.more_available);

        let retained = api.drain_work(6, 10);
        assert_eq!(retained.effects.len(), 1);
        assert_eq!(retained.effects[0].source_payload_id, 42);
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

    #[test]
    fn transaction_packet_decoder_preserves_members_and_rejects_bad_shape() {
        let mut packet = RlpStream::new_list(2);
        packet.begin_list(1);
        packet.append_raw(&[0x01], 1);
        packet.begin_list(1);
        packet.append(&H256::from([7; 32]));
        let (transactions, hashes) = decode_transaction_packet(&packet.out()).unwrap();
        assert_eq!(transactions, vec![vec![0x01]]);
        assert_eq!(hashes, vec![[7; 32]]);
        assert!(matches!(
            decode_transaction_packet(&[0xc0]),
            Err(TransactionPacketDecodeError::Malformed)
        ));
    }

    #[test]
    fn known_transaction_packet_member_is_benign_without_regossip() {
        let report = TransactionPacketIngressReport {
            submission: crate::PublicTransactionSubmissionReport {
                transaction_hash: H256::from([9; 32]),
                accepted: false,
                message: "Transaction already in transactions pool".to_owned(),
                verification_status:
                    crate::transaction_manager::TransactionManagerVerifyTransactionStatus::Accepted,
                queue_status: Some(TransactionQueueInsertStatus::Known),
                transaction_observed: false,
            },
            peer_id: peer(3),
            observe_transaction: false,
            transaction_rlp: vec![0x01],
        };

        assert!(transaction_packet_member_is_benign(&report));
        assert!(!report.observe_transaction);
    }

    #[test]
    fn get_dag_sync_decoder_deduplicates_hashes_in_request_order() {
        let mut packet = RlpStream::new_list(2);
        packet.append(&9u64);
        packet.begin_list(3);
        packet.append(&H256::from([1; 32]));
        packet.append(&H256::from([2; 32]));
        packet.append(&H256::from([1; 32]));
        let (period, hashes) = decode_get_dag_sync_request(&packet.out()).unwrap();
        assert_eq!(period, 9);
        assert_eq!(hashes, vec![H256::from([1; 32]), H256::from([2; 32])]);
    }

    #[test]
    fn transaction_gossip_known_marks_depend_on_successful_send() {
        let mut api = ConsensusNetworkApi::new();
        let decision = api.enqueue_transaction_gossip(
            6,
            77,
            vec![(peer(4), Vec::new())],
            vec![TransactionGossipAccount {
                sender: [3; 20].into(),
                transactions: vec![crate::TransactionGossipEntry {
                    hash: H256::from([5; 32]),
                    transaction_rlp: vec![0x01],
                }],
            }],
        );
        assert_eq!(decision.queued_effect_count, 2);
        let send = api.drain_work(6, 10).effects;
        assert_eq!(send.len(), 1);
        assert_eq!(send[0].kind, NETWORK_EFFECT_KIND_SEND_PACKET);
        let failed = effect_result(&send[0], NETWORK_EFFECT_RESULT_STATUS_FAILED);
        assert_eq!(api.report_effect_results(vec![failed]).status, 0);
        assert!(api.drain_work(6, 10).effects.is_empty());
    }

    #[test]
    fn exact_pbft_vote_egress_filters_snapshot_and_builds_complete_packet() {
        let mut api = ConsensusNetworkApi::new();
        let vote = canonical_bundle_vote(0x42);
        let preparation = api
            .prepare_egress(
                NetworkEgressPrepareRequest {
                    family: NETWORK_EGRESS_FAMILY_PBFT_VOTE,
                    transport_lane: 6,
                    source_payload_id: 91,
                    source_peer_id: peer(1),
                    rebroadcast: false,
                    object_hash: [0; 32],
                    payload_bytes: encode_pbft_vote_egress_packet(&vote.vote_rlp, None, 77),
                    related_payload_bytes: Vec::new(),
                },
                Vec::new(),
                Vec::new(),
                77,
            )
            .unwrap();
        assert_eq!(preparation.probes.len(), 1);
        assert_eq!(
            preparation.probes[0].object_hash,
            vote.hash.to_fixed_bytes()
        );

        let decision = api
            .plan_egress(NetworkEgressPlanRequest {
                token: preparation.token,
                peers: vec![
                    NetworkEgressPeerSnapshot {
                        peer_id: peer(1),
                        syncing: false,
                        known_probe_ids: Vec::new(),
                        ..Default::default()
                    },
                    NetworkEgressPeerSnapshot {
                        peer_id: peer(2),
                        syncing: true,
                        known_probe_ids: Vec::new(),
                        ..Default::default()
                    },
                    NetworkEgressPeerSnapshot {
                        peer_id: peer(3),
                        syncing: false,
                        known_probe_ids: vec![0],
                        ..Default::default()
                    },
                    NetworkEgressPeerSnapshot {
                        peer_id: peer(4),
                        syncing: false,
                        known_probe_ids: Vec::new(),
                        ..Default::default()
                    },
                ],
            })
            .unwrap();
        assert_eq!(decision.queued_effect_count, 2);
        let sends = api.drain_work(6, 10).effects;
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].kind, NETWORK_EFFECT_KIND_SEND_PACKET);
        assert_eq!(sends[0].peer_id, peer(4));
        let decoded = decode_pbft_vote_packet(&sends[0].payload_bytes).unwrap();
        assert_eq!(decoded.vote_rlp, vote.vote_rlp);
        assert!(decoded.pbft_block_rlp.is_none());
        api.report_effect_results(vec![effect_result(
            &sends[0],
            NETWORK_EFFECT_RESULT_STATUS_OK,
        )]);
        let marks = api.drain_work(6, 10).effects;
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, NETWORK_EFFECT_KIND_MARK_PEER_KNOWN);
        assert_eq!(marks[0].dependency_id, sends[0].effect_id);
        assert!(
            api.plan_egress(NetworkEgressPlanRequest {
                token: preparation.token,
                peers: Vec::new()
            })
            .is_err()
        );
    }

    #[test]
    fn exact_bundle_egress_filters_members_before_native_reencoding() {
        let mut api = ConsensusNetworkApi::new();
        let votes = vec![canonical_bundle_vote(0x42), canonical_bundle_vote(0x43)];
        let first = inspect_canonical_pbft_vote(&votes[0].vote_rlp).unwrap();
        let bundle = build_optimized_pbft_vote_bundle(
            &votes,
            first.block_hash,
            first.period,
            first.round,
            first.step,
        )
        .unwrap();
        let preparation = api
            .prepare_egress(
                NetworkEgressPrepareRequest {
                    family: NETWORK_EGRESS_FAMILY_PBFT_VOTES_BUNDLE,
                    transport_lane: 6,
                    source_payload_id: 92,
                    source_peer_id: [0; 64],
                    rebroadcast: false,
                    object_hash: [0; 32],
                    payload_bytes: bundle.bundle_rlp,
                    related_payload_bytes: Vec::new(),
                },
                Vec::new(),
                Vec::new(),
                77,
            )
            .unwrap();
        assert_eq!(preparation.probes.len(), 2);
        api.plan_egress(NetworkEgressPlanRequest {
            token: preparation.token,
            peers: vec![NetworkEgressPeerSnapshot {
                peer_id: peer(5),
                syncing: false,
                known_probe_ids: vec![0],
                ..Default::default()
            }],
        })
        .unwrap();
        let send = api.drain_work(6, 10).effects;
        assert_eq!(send.len(), 1);
        let decoded = decode_pbft_votes_bundle_packet(&send[0].payload_bytes).unwrap();
        assert_eq!(decoded, vec![votes[1].vote_rlp.clone()]);
    }

    #[test]
    fn application_bundle_egress_chunks_more_than_one_wire_packet() {
        let mut api = ConsensusNetworkApi::new();
        let vote = canonical_bundle_vote(0x42);
        let inspected = inspect_canonical_pbft_vote(&vote.vote_rlp).unwrap();
        let votes = vec![vote; MAX_VOTES_PER_BUNDLE_PACKET + 1];
        let bundle = build_optimized_pbft_vote_bundle(
            &votes,
            inspected.block_hash,
            inspected.period,
            inspected.round,
            inspected.step,
        )
        .unwrap();
        let preparation = api
            .prepare_egress(
                NetworkEgressPrepareRequest {
                    family: NETWORK_EGRESS_FAMILY_PBFT_VOTES_BUNDLE,
                    transport_lane: 6,
                    source_payload_id: 93,
                    source_peer_id: [0; 64],
                    rebroadcast: false,
                    object_hash: [0; 32],
                    payload_bytes: bundle.bundle_rlp,
                    related_payload_bytes: Vec::new(),
                },
                Vec::new(),
                Vec::new(),
                77,
            )
            .unwrap();
        assert_eq!(preparation.probes.len(), MAX_VOTES_PER_BUNDLE_PACKET + 1);
        api.plan_egress(NetworkEgressPlanRequest {
            token: preparation.token,
            peers: vec![NetworkEgressPeerSnapshot {
                peer_id: peer(5),
                syncing: false,
                known_probe_ids: Vec::new(),
                ..Default::default()
            }],
        })
        .unwrap();
        let sends = api.drain_work(6, 10).effects;
        assert_eq!(sends.len(), 2);
        assert_eq!(
            decode_pbft_votes_bundle_packet(&sends[0].payload_bytes)
                .unwrap()
                .len(),
            MAX_VOTES_PER_BUNDLE_PACKET
        );
        assert_eq!(
            decode_pbft_votes_bundle_packet(&sends[1].payload_bytes)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn bundle_effect_limit_counts_every_chunk_send() {
        let mut api = ConsensusNetworkApi::new();
        let vote = canonical_bundle_vote(0x42);
        let vote_count = 2_047;
        let probes = (0..vote_count)
            .map(|probe_id| NetworkEgressProbe {
                probe_id: probe_id as u32,
                object_kind: NETWORK_OBJECT_KIND_PBFT_VOTE,
                object_hash: vote.hash.to_fixed_bytes(),
            })
            .collect::<Vec<_>>();
        api.pending_egress.insert(
            77,
            PendingNetworkEgress {
                transport_lane: 6,
                source_payload_id: 95,
                source_peer_id: [0; 64],
                rebroadcast: false,
                probes,
                payload: PendingNetworkEgressPayload::PbftVotesBundle {
                    votes: vec![vote; vote_count],
                    period: 1,
                    round: 1,
                },
            },
        );
        let peers = (1..=32)
            .map(|id| NetworkEgressPeerSnapshot {
                peer_id: peer(id),
                syncing: false,
                known_probe_ids: Vec::new(),
                ..Default::default()
            })
            .collect();
        let error = api
            .plan_egress(NetworkEgressPlanRequest { token: 77, peers })
            .unwrap_err();
        assert!(error.to_string().contains("NETWORK_EGRESS_EFFECT_LIMIT"));
        assert!(api.pending_effects.is_empty());
    }

    #[test]
    fn dag_egress_builds_peer_specific_packets_and_transaction_marks() {
        let mut api = ConsensusNetworkApi::new();
        let transaction = crate::TransactionGossipEntry {
            hash: H256::from([8; 32]),
            transaction_rlp: vec![0x01],
        };
        let preparation = api
            .prepare_egress(
                NetworkEgressPrepareRequest {
                    family: NETWORK_EGRESS_FAMILY_DAG_BLOCK,
                    transport_lane: 6,
                    source_payload_id: 94,
                    source_peer_id: [0; 64],
                    rebroadcast: false,
                    object_hash: [7; 32],
                    payload_bytes: vec![0xc0],
                    related_payload_bytes: Vec::new(),
                },
                Vec::new(),
                vec![transaction.clone()],
                77,
            )
            .unwrap();
        assert_eq!(preparation.probes.len(), 2);
        api.plan_egress(NetworkEgressPlanRequest {
            token: preparation.token,
            peers: vec![
                NetworkEgressPeerSnapshot {
                    peer_id: peer(5),
                    syncing: false,
                    known_probe_ids: vec![1],
                    ..Default::default()
                },
                NetworkEgressPeerSnapshot {
                    peer_id: peer(6),
                    syncing: false,
                    known_probe_ids: Vec::new(),
                    ..Default::default()
                },
            ],
        })
        .unwrap();
        let sends = api.drain_work(6, 10).effects;
        assert_eq!(sends.len(), 2);
        let known_packet = sends.iter().find(|send| send.peer_id == peer(5)).unwrap();
        let unknown_packet = sends.iter().find(|send| send.peer_id == peer(6)).unwrap();
        assert_eq!(
            decode_dag_block_packet(&known_packet.payload_bytes)
                .unwrap()
                .0
                .len(),
            0
        );
        assert_eq!(
            decode_dag_block_packet(&unknown_packet.payload_bytes)
                .unwrap()
                .0,
            vec![transaction.transaction_rlp]
        );
        api.report_effect_results(
            sends
                .iter()
                .map(|send| effect_result(send, NETWORK_EFFECT_RESULT_STATUS_OK))
                .collect(),
        );
        let marks = api.drain_work(6, 10).effects;
        assert_eq!(marks.len(), 3);
        assert_eq!(
            marks
                .iter()
                .filter(|mark| mark.object_kind == NETWORK_OBJECT_KIND_TRANSACTION)
                .count(),
            1
        );
    }

    fn dag_rejection_context() -> NetworkDagBlockIngressContext {
        NetworkDagBlockIngressContext {
            transport_lane: 6,
            peer_id: peer(9),
            source_payload_id: 77,
            rebroadcast: false,
            peer_dag_synced: true,
            dag_sync_allowed: false,
            transactions_dropped: false,
            pending_dag_request: false,
            local_pbft_syncing: false,
        }
    }

    #[test]
    fn dag_rejection_planner_preserves_missing_transaction_policy() {
        let mut context = dag_rejection_context();
        context.dag_sync_allowed = true;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_MISSING_TRANSACTION).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_REQUEST_DAG_SYNC
        );
        context.dag_sync_allowed = false;
        context.transactions_dropped = true;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_MISSING_TRANSACTION).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_DISCONNECT
        );
        context.transactions_dropped = false;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_MISSING_TRANSACTION).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_MALICIOUS
        );
    }

    #[test]
    fn dag_rejection_planner_preserves_missing_tip_policy() {
        let mut context = dag_rejection_context();
        context.dag_sync_allowed = true;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_MISSING_TIP).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_REQUEST_DAG_SYNC
        );
        context.dag_sync_allowed = false;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_MISSING_TIP).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_MALICIOUS
        );
        context.peer_dag_synced = false;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_MISSING_TIP).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_REQUEST_PENDING_DAG
        );
        context.pending_dag_request = true;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_MISSING_TIP).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_IGNORE
        );
    }

    #[test]
    fn dag_rejection_planner_disconnects_ahead_only_when_peer_was_synced() {
        let mut context = dag_rejection_context();
        for reject_code in [
            DAG_VERIFY_REJECT_AHEAD_BLOCK,
            DAG_VERIFY_REJECT_FUTURE_BLOCK,
        ] {
            assert_eq!(
                plan_dag_block_rejection(&context, reject_code).unwrap(),
                NETWORK_DAG_REJECTION_ACTION_DISCONNECT
            );
            context.peer_dag_synced = false;
            assert_eq!(
                plan_dag_block_rejection(&context, reject_code).unwrap(),
                NETWORK_DAG_REJECTION_ACTION_IGNORE
            );
            context.peer_dag_synced = true;
        }
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_EXPIRED_BLOCK).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_IGNORE
        );
    }

    #[test]
    fn dag_rejection_planner_marks_only_invalid_proofs_and_shapes_malicious() {
        let context = dag_rejection_context();
        for reject_code in [
            DAG_VERIFY_REJECT_INCORRECT_TRANSACTIONS_ESTIMATION,
            DAG_VERIFY_REJECT_BLOCK_TOO_BIG,
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION,
            DAG_VERIFY_REJECT_NOT_ELIGIBLE,
            DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION,
        ] {
            assert_eq!(
                plan_dag_block_rejection(&context, reject_code).unwrap(),
                NETWORK_DAG_REJECTION_ACTION_MALICIOUS
            );
        }
        assert!(plan_dag_block_rejection(&context, u32::MAX).is_err());
    }

    #[test]
    fn add_stage_metadata_rejection_preserves_sync_state_policy() {
        let mut context = dag_rejection_context();
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_ADD_BLOCK_METADATA).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_MALICIOUS
        );

        context.pending_dag_request = true;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_ADD_BLOCK_METADATA).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_IGNORE
        );

        context.pending_dag_request = false;
        context.local_pbft_syncing = true;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_ADD_BLOCK_METADATA).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_IGNORE
        );

        context.local_pbft_syncing = false;
        context.peer_dag_synced = false;
        assert_eq!(
            plan_dag_block_rejection(&context, DAG_VERIFY_REJECT_ADD_BLOCK_METADATA).unwrap(),
            NETWORK_DAG_REJECTION_ACTION_REQUEST_PENDING_DAG
        );
    }

    #[test]
    fn terminal_dag_sync_actions_never_enter_shared_effect_queue() {
        let mut api = ConsensusNetworkApi::new();
        let mut context = dag_rejection_context();
        context.dag_sync_allowed = true;
        let (decision, action) = api
            .plan_dag_block_rejection_decision(&context, DAG_VERIFY_REJECT_MISSING_TRANSACTION)
            .unwrap();
        assert_eq!(action, NETWORK_DAG_REJECTION_ACTION_REQUEST_DAG_SYNC);
        assert_eq!(decision.queued_effect_count, 0);
        assert!(
            api.drain_work(context.transport_lane, 10)
                .effects
                .is_empty()
        );
    }

    fn sync_start_request(
        now_ms: u64,
        peer_byte: u8,
        peer_chain_size: u64,
    ) -> NetworkPbftSyncStartRequest {
        NetworkPbftSyncStartRequest {
            start: true,
            now_ms,
            local_pbft_synced_period: 10,
            local_pbft_chain_size: 10,
            candidates: vec![sync_candidate(peer_byte, peer_chain_size, 20)],
        }
    }

    fn sync_command(kind: u8, generation: u64, peer_byte: u8) -> NetworkPbftSyncCommandRequest {
        NetworkPbftSyncCommandRequest {
            kind,
            now_ms: 1_100,
            generation,
            peer_id: peer(peer_byte),
            source: NETWORK_PBFT_SYNC_SOURCE_ACTIVE,
            reason: NETWORK_PBFT_SYNC_STOP_REASON_NONE,
            sync_queue_size: 0,
            syncing_period: 10,
            finalized_period: 10,
            remote_period: 10,
            sync_level_size: 10,
            retry_count: 0,
            retry_delay_ms: 100,
        }
    }

    #[test]
    fn native_pbft_sync_start_owns_generation_peer_and_deep_state() {
        let mut api = ConsensusNetworkApi::new();

        let started = api.begin_pbft_sync(sync_start_request(1_000, 7, 20));

        assert!(started.started);
        assert_eq!(started.generation, 1);
        assert_eq!(started.peer_id, peer(7));
        assert_eq!(started.request_period, 11);
        assert!(started.deep_syncing);
        let snapshot = api.pbft_sync_status(1_250);
        assert!(snapshot.active);
        assert!(snapshot.deep_syncing);
        assert_eq!(snapshot.peer_id, peer(7));
        assert_eq!(snapshot.last_peer_id, peer(7));
        assert_eq!(snapshot.elapsed_ms, 250);
        assert_eq!(snapshot.inactive_for_ms, 250);
        assert_eq!(snapshot.start_count, 1);

        let duplicate = api.begin_pbft_sync(sync_start_request(1_300, 8, 30));
        assert!(!duplicate.started);
        assert_eq!(duplicate.status, NETWORK_STATUS_PLAN_STATUS_ALREADY_SYNCING);
        assert_eq!(duplicate.generation, 1);
        assert_eq!(api.pbft_sync_status(1_300).peer_id, peer(7));
    }

    #[test]
    fn native_pbft_sync_period_update_uses_saturating_deep_calculation() {
        let mut api = ConsensusNetworkApi::new();
        assert!(
            api.begin_pbft_sync(sync_start_request(1_000, 7, 20))
                .deep_syncing
        );

        let near_tip = api.update_pbft_sync_period(19);
        assert!(near_tip.active);
        assert!(!near_tip.deep_syncing);

        let beyond_peer_tip = api.update_pbft_sync_period(u64::MAX);
        assert!(beyond_peer_tip.active);
        assert!(!beyond_peer_tip.deep_syncing);
    }

    #[test]
    fn native_completion_waits_for_queue_then_stops_and_requests_followup() {
        let mut api = ConsensusNetworkApi::new();
        let start = api.begin_pbft_sync(sync_start_request(1_000, 7, 20));
        let mut completion = sync_command(5, start.generation, 7);
        completion.sync_queue_size = 1;
        let waiting = api.apply_pbft_sync_command(completion.clone()).unwrap();
        assert!(waiting.accepted);
        assert!(waiting.active);
        assert!(waiting.deep_syncing);
        assert!(waiting.retry);
        assert!(!waiting.restart_sync);

        completion.sync_queue_size = 0;
        let completed = api.apply_pbft_sync_command(completion).unwrap();
        assert!(completed.stopped);
        assert!(!completed.active);
        assert!(!completed.deep_syncing);
        assert!(completed.restart_sync);
        assert!(completed.request_pending_dag_if_idle);
        assert_eq!(
            api.pbft_sync_status(1_100).last_stop_reason,
            NETWORK_PBFT_SYNC_STOP_REASON_COMPLETED
        );
    }

    #[test]
    fn native_last_block_and_delayed_continuation_own_retry_and_stop_policy() {
        let mut api = ConsensusNetworkApi::new();
        let first = api.begin_pbft_sync(sync_start_request(1_000, 7, 200));
        let mut last = sync_command(6, first.generation, 7);
        last.syncing_period = 20;
        last.finalized_period = 1;
        last.remote_period = 20;
        last.sync_level_size = 1;
        assert!(api.apply_pbft_sync_command(last.clone()).unwrap().retry);

        last.finalized_period = 20;
        let next = api.apply_pbft_sync_command(last).unwrap();
        assert!(next.request_next);
        assert!(!next.retry);

        let mut remote_behind = sync_command(6, first.generation, 7);
        remote_behind.syncing_period = 21;
        remote_behind.remote_period = 20;
        assert!(api.apply_pbft_sync_command(remote_behind).unwrap().stopped);

        let second = api.begin_pbft_sync(sync_start_request(2_000, 8, 200));
        let mut delayed = sync_command(7, second.generation, 8);
        delayed.retry_count = 601;
        let exhausted = api.apply_pbft_sync_command(delayed).unwrap();
        assert!(exhausted.stopped);
        assert!(!exhausted.active);
        assert_eq!(
            api.pbft_sync_status(2_100).last_stop_reason,
            NETWORK_PBFT_SYNC_STOP_REASON_TRANSPORT_FAILED
        );
    }

    #[test]
    fn stale_completion_and_command_reports_project_replacement_state() {
        let mut api = ConsensusNetworkApi::new();
        let first = api.begin_pbft_sync(sync_start_request(1_000, 7, 20));
        let mut stop = sync_command(2, first.generation, 7);
        stop.reason = NETWORK_PBFT_SYNC_STOP_REASON_REPLACED;
        assert!(api.apply_pbft_sync_command(stop).unwrap().stopped);
        let second = api.begin_pbft_sync(sync_start_request(2_000, 8, 30));

        let stale = api
            .apply_pbft_sync_command(sync_command(5, first.generation, 7))
            .unwrap();
        assert!(!stale.accepted);
        assert!(stale.active);
        assert!(stale.deep_syncing);
        assert_eq!(stale.generation, second.generation);
        assert_eq!(api.pbft_sync_status(2_100).peer_id, peer(8));

        let inactive_tick = {
            let mut stop = sync_command(2, second.generation, 8);
            stop.reason = NETWORK_PBFT_SYNC_STOP_REASON_COMPLETED;
            assert!(api.apply_pbft_sync_command(stop).unwrap().stopped);
            api.apply_pbft_sync_command(sync_command(4, second.generation, 8))
                .unwrap()
        };
        assert!(!inactive_tick.active);
        assert!(!inactive_tick.deep_syncing);
    }

    #[test]
    fn max_peer_selection_does_not_open_a_sync_generation() {
        let mut api = ConsensusNetworkApi::new();
        let mut request = sync_start_request(1_000, 7, 20);
        request.start = false;
        let selected = api.begin_pbft_sync(request);
        assert!(!selected.started);
        assert!(selected.has_peer);
        assert_eq!(selected.peer_id, peer(7));
        assert!(!api.pbft_sync_status(1_000).active);
    }

    #[test]
    fn native_status_followup_owns_one_block_debounce() {
        let mut api = ConsensusNetworkApi::new();
        let request = NetworkStatusFollowupRequest {
            peer_id: peer(4),
            local_pbft_synced_period: 10,
            local_pbft_period: 11,
            local_pbft_round: 2,
            peer_pbft_chain_size: 11,
            peer_pbft_period: 12,
            peer_pbft_round: 2,
            peer_dag_synced: true,
        };

        assert!(
            !api.process_status_followup(request.clone())
                .request_pbft_sync
        );
        assert!(api.process_status_followup(request).request_pbft_sync);

        let ahead_round = NetworkStatusFollowupRequest {
            peer_id: peer(5),
            local_pbft_synced_period: 10,
            local_pbft_period: 11,
            local_pbft_round: 2,
            peer_pbft_chain_size: 10,
            peer_pbft_period: 11,
            peer_pbft_round: 3,
            peer_dag_synced: true,
        };
        let outcome = api.process_status_followup(ahead_round);
        assert!(outcome.request_next_votes);
        assert_eq!(outcome.next_votes_period, 11);
        assert_eq!(outcome.next_votes_round, 2);
    }

    #[test]
    fn native_source_correlation_retains_last_peer_after_stop() {
        let mut api = ConsensusNetworkApi::new();
        let start = api.begin_pbft_sync(sync_start_request(1_000, 7, 20));
        let active = api.admit_pbft_sync_source(NetworkPbftSyncSourceRequest {
            peer_id: peer(7),
            source: NETWORK_PBFT_SYNC_SOURCE_ACTIVE,
        });
        assert!(active.accepted);
        assert_eq!(active.generation, start.generation);
        assert!(
            !api.admit_pbft_sync_source(NetworkPbftSyncSourceRequest {
                peer_id: peer(8),
                source: NETWORK_PBFT_SYNC_SOURCE_ACTIVE,
            })
            .accepted
        );

        assert!(
            api.stop_pbft_sync(NetworkPbftSyncStopRequest {
                generation: start.generation,
                peer_id: peer(7),
                reason: NETWORK_PBFT_SYNC_STOP_REASON_COMPLETED,
            })
            .stopped
        );
        assert!(
            !api.admit_pbft_sync_source(NetworkPbftSyncSourceRequest {
                peer_id: peer(7),
                source: NETWORK_PBFT_SYNC_SOURCE_ACTIVE,
            })
            .accepted
        );
        assert!(
            api.admit_pbft_sync_source(NetworkPbftSyncSourceRequest {
                peer_id: peer(7),
                source: NETWORK_PBFT_SYNC_SOURCE_LAST,
            })
            .accepted
        );
    }

    #[test]
    fn stale_generation_reports_cannot_stop_or_touch_replacement_sync() {
        let mut api = ConsensusNetworkApi::new();
        let first = api.begin_pbft_sync(sync_start_request(1_000, 7, 20));
        assert!(
            api.stop_pbft_sync(NetworkPbftSyncStopRequest {
                generation: first.generation,
                peer_id: peer(7),
                reason: NETWORK_PBFT_SYNC_STOP_REASON_REPLACED,
            })
            .stopped
        );
        let second = api.begin_pbft_sync(sync_start_request(2_000, 8, 30));
        assert_eq!(second.generation, first.generation + 1);

        assert!(
            !api.record_pbft_sync_activity(NetworkPbftSyncActivityRequest {
                now_ms: 9_000,
                generation: first.generation,
                peer_id: peer(7),
            })
            .accepted
        );
        assert!(
            !api.stop_pbft_sync(NetworkPbftSyncStopRequest {
                generation: first.generation,
                peer_id: peer(7),
                reason: NETWORK_PBFT_SYNC_STOP_REASON_TRANSPORT_FAILED,
            })
            .stopped
        );
        assert!(
            !api.handle_pbft_sync_disconnect(NetworkPbftSyncDisconnectRequest {
                generation: first.generation,
                peer_id: peer(7),
            })
            .stopped
        );
        let current = api.pbft_sync_status(9_000);
        assert!(current.active);
        assert_eq!(current.generation, second.generation);
        assert_eq!(current.peer_id, peer(8));
        assert_eq!(current.last_activity_ms, 2_000);
    }

    #[test]
    fn query_is_side_effect_free_and_timer_expires_only_after_threshold() {
        let mut api = ConsensusNetworkApi::new();
        let start = api.begin_pbft_sync(sync_start_request(1_000, 7, 20));

        let overdue_query = api.pbft_sync_status(61_001);
        assert!(overdue_query.active);
        assert_eq!(overdue_query.inactive_for_ms, 60_001);
        assert!(
            !api.tick_pbft_sync(NetworkPbftSyncTickRequest {
                now_ms: 61_000,
                generation: start.generation,
            })
            .expired
        );
        assert!(api.pbft_sync_status(61_000).active);

        let expired = api.tick_pbft_sync(NetworkPbftSyncTickRequest {
            now_ms: 61_001,
            generation: start.generation,
        });
        assert!(expired.expired);
        assert!(expired.restart_sync);
        let stopped = api.pbft_sync_status(61_001);
        assert!(!stopped.active);
        assert_eq!(stopped.inactivity_count, 1);
        assert_eq!(
            stopped.last_stop_reason,
            NETWORK_PBFT_SYNC_STOP_REASON_INACTIVE
        );
    }

    #[test]
    fn selected_peer_disconnect_stops_generation_and_requests_restart() {
        let mut api = ConsensusNetworkApi::new();
        let start = api.begin_pbft_sync(sync_start_request(1_000, 7, 20));
        let disconnected = api
            .apply_pbft_sync_command(sync_command(3, start.generation, 7))
            .unwrap();
        assert!(disconnected.stopped);
        assert!(disconnected.restart_sync);
        assert!(!disconnected.active);
        let snapshot = api.pbft_sync_status(1_100);
        assert_eq!(snapshot.disconnect_count, 1);
        assert_eq!(
            snapshot.last_stop_reason,
            NETWORK_PBFT_SYNC_STOP_REASON_DISCONNECTED
        );
    }
}
