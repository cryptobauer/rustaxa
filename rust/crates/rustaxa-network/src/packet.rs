//! Packet metadata and payload storage for network ingress.
//!
//! Packets carry the sender session, receive timestamp, packet type, and raw
//! payload bytes used by later ingress stages. Most Taraxa network packets are
//! small, so payloads up to [`INLINE_LIMIT`] are stored directly in the packet
//! value while larger payloads keep their shared [`bytes::Bytes`] allocation.

use chrono::Utc;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use rustaxa_types::ethereum::NodeId;
use rustaxa_types::time::Microseconds;

use crate::{
    filter::{Flag, PacketFilter},
    peers::{PeerRef, PeerRegistry, SessionId},
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
/// Wire-compatible packet type identifiers.
pub enum PacketType {
    /// Marker for the start of consensus packets with high processing priority.
    HighPriorityPackets,
    /// Vote payload; may also contain an optional PBFT block.
    VotePacket,
    /// Request for the next votes needed during vote synchronization.
    GetNextVotesSyncPacket,
    /// Bundle of votes sent during synchronization.
    VotesBundlePacket,

    /// Marker for the start of standard packets with medium processing priority.
    MidPriorityPackets,
    /// DAG block propagation payload.
    DagBlockPacket,
    /// DAG sync payload, including ad-hoc sync when blocks miss tips or pivot.
    DagSyncPacket,
    /// Transaction propagation payload.
    TransactionPacket,

    /// Marker for the start of non-critical packets with low processing priority.
    LowPriorityPackets,
    /// Peer status exchange payload.
    StatusPacket,
    /// Request for PBFT synchronization data.
    GetPbftSyncPacket,
    /// PBFT synchronization response payload.
    PbftSyncPacket,
    /// Request for DAG synchronization data.
    GetDagSyncPacket,
    /// Pillar vote propagation payload.
    PillarVotePacket,
    /// Request for a bundle of pillar votes.
    GetPillarVotesBundlePacket,
    /// Bundle of pillar votes sent during synchronization.
    PillarVotesBundlePacket,
    /// Bundle of PBFT blocks sent during synchronization.
    PbftBlocksBundlePacket,

    /// Number of known packet type values below [`PacketType::Unknown`].
    PacketCount,

    /// Unknown or unsupported packet type marker.
    Unknown = 254,
}

/// Target in-memory size for a packet used by the network pipeline.
///
/// Keeping packets at a fixed size gives predictable storage layout and keeps
/// common packet payload bytes close to the packet metadata.
#[allow(dead_code)]
const PACKET_SIZE: usize = 2048;

/// Maximum payload size stored directly inside a packet.
///
/// Larger payloads are stored as [`bytes::Bytes`] to avoid copying unusually
/// large buffers into every packet value.
const INLINE_LIMIT: usize = 1946;

/// Packet payload storage optimized for common small packets.
///
/// Small payloads are stored inline to improve data locality. Large payloads
/// keep their shared [`bytes::Bytes`] allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum PacketPayload {
    /// Payload bytes stored directly in the packet value.
    ///
    /// The `len` field records the number of initialized bytes in `buf`.
    Inline { len: usize, buf: [u8; INLINE_LIMIT] },

    /// Payload bytes stored outside the fixed-size packet entry.
    Heap(bytes::Bytes),
}

impl Default for PacketPayload {
    /// Creates an empty inline payload.
    fn default() -> Self {
        Self::Inline {
            len: 0,
            buf: [0; INLINE_LIMIT],
        }
    }
}

/// Network packet passed through the ingress pipeline.
///
/// The packet keeps metadata needed for queueing and peer attribution together
/// with payload storage optimized for the common small-packet path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Type of the packet.
    pub packet_type: PacketType,

    /// Node that sent the packet.
    pub peer: PeerRef,

    /// Wall-clock receive timestamp in microseconds.
    pub received: Microseconds,

    /// Packet bytes, stored inline when they fit in the inline payload limit.
    payload: PacketPayload,
}

impl Default for Packet {
    /// Creates an empty packet with an unknown packet type and zeroed peer ref.
    fn default() -> Self {
        Self {
            packet_type: PacketType::Unknown,
            peer: PeerRef::new(NodeId(ethereum_types::H512::from([0u8; 64])), SessionId(0)),
            received: Microseconds(0),
            payload: PacketPayload::default(),
        }
    }
}

impl Packet {
    /// Creates a packet from the sender id and payload bytes.
    ///
    /// Payloads up to the inline payload limit are copied into the packet entry.
    /// Larger payloads retain the provided [`bytes::Bytes`] handle.
    pub fn new(packet_type: PacketType, peer: PeerRef, payload: bytes::Bytes) -> Self {
        Packet {
            packet_type,
            peer,
            received: Microseconds(Utc::now().timestamp_micros() as u64),
            payload: if payload.len() > INLINE_LIMIT {
                PacketPayload::Heap(payload)
            } else {
                let mut buf = [0u8; INLINE_LIMIT];
                buf[..payload.len()].copy_from_slice(&payload);
                PacketPayload::Inline {
                    len: payload.len(),
                    buf,
                }
            },
        }
    }

    /// Returns the packet payload as a byte slice.
    pub fn payload(&self) -> &[u8] {
        match &self.payload {
            PacketPayload::Heap(bytes) => bytes.as_ref(),
            PacketPayload::Inline { len, buf } => &buf[..*len],
        }
    }
}

impl PacketFilter for Packet {
    fn peer_connected(&self, registry: &PeerRegistry) -> Result<bool, anyhow::Error> {
        Ok(registry.connected(self.peer.node)?.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethereum_types::H512;
    use std::mem;

    fn test_node_id() -> NodeId {
        let mut arr = [0u8; 64];
        arr[63] = 1;
        NodeId(H512::from(arr))
    }

    fn test_session_id() -> SessionId {
        SessionId(1234)
    }

    fn test_payload1() -> Bytes {
        Bytes::from(vec![0xc3, 0x01, 0x02, 0x03])
    }

    fn test_payload_inline() -> Bytes {
        Bytes::from(vec![0xAB; INLINE_LIMIT])
    }

    fn test_payload_heap() -> Bytes {
        Bytes::from(vec![0xCD; INLINE_LIMIT + 1])
    }

    #[test]
    fn test_packet_type_numeric_mapping() {
        let expected = [
            (0, PacketType::HighPriorityPackets),
            (1, PacketType::VotePacket),
            (2, PacketType::GetNextVotesSyncPacket),
            (3, PacketType::VotesBundlePacket),
            (4, PacketType::MidPriorityPackets),
            (5, PacketType::DagBlockPacket),
            (6, PacketType::DagSyncPacket),
            (7, PacketType::TransactionPacket),
            (8, PacketType::LowPriorityPackets),
            (9, PacketType::StatusPacket),
            (10, PacketType::GetPbftSyncPacket),
            (11, PacketType::PbftSyncPacket),
            (12, PacketType::GetDagSyncPacket),
            (13, PacketType::PillarVotePacket),
            (14, PacketType::GetPillarVotesBundlePacket),
            (15, PacketType::PillarVotesBundlePacket),
            (16, PacketType::PbftBlocksBundlePacket),
            (17, PacketType::PacketCount),
            (254, PacketType::Unknown),
        ];

        for (raw, packet_type) in expected {
            assert_eq!(PacketType::try_from_primitive(raw).unwrap(), packet_type);
            assert_eq!(u8::from(packet_type), raw);
        }

        assert!(PacketType::try_from_primitive(18).is_err());
        assert!(PacketType::try_from_primitive(253).is_err());
        assert!(PacketType::try_from_primitive(255).is_err());
    }

    #[test]
    fn test_packet_default_is_empty_unknown_packet() {
        let packet = Packet::default();

        assert_eq!(packet.packet_type, PacketType::Unknown);
        assert_eq!(
            packet.peer,
            PeerRef::new(NodeId(H512::zero()), SessionId(0))
        );
        assert_eq!(packet.received, Microseconds(0));
        assert_eq!(packet.payload(), &[]);
    }

    #[test]
    fn test_packet_create_sets_metadata_and_payload() {
        let from_node = test_node_id();
        let session = test_session_id();
        let peer = PeerRef::new(from_node, session);
        let payload = test_payload1();
        let checkpoint1 = Utc::now().timestamp_micros();
        let packet = Packet::new(PacketType::DagBlockPacket, peer.clone(), payload.clone());
        let checkpoint2 = Utc::now().timestamp_micros();

        assert!(packet.received >= Microseconds(checkpoint1 as u64));
        assert!(packet.received <= Microseconds(checkpoint2 as u64));
        assert_eq!(packet.peer, peer);
        assert_eq!(packet.payload(), payload.as_ref());
    }

    #[test]
    fn test_packet_size_is_2048() {
        assert_eq!(
            mem::size_of::<Packet>(),
            PACKET_SIZE,
            "Packet size is not 2048 bytes"
        );
    }

    #[test]
    fn test_empty_payload_uses_inline_storage() {
        let peer = PeerRef::new(test_node_id(), test_session_id());
        let payload = Bytes::new();
        let packet = Packet::new(PacketType::StatusPacket, peer, payload);

        match &packet.payload {
            PacketPayload::Inline { len, buf } => {
                assert_eq!(*len, 0);
                assert_eq!(&buf[..*len], &[]);
            }
            PacketPayload::Heap(_) => panic!("expected inline payload storage"),
        }
        assert_eq!(packet.payload(), &[]);
    }

    #[test]
    fn test_inline_limit_payload_uses_inline_storage() {
        let from_node = test_node_id();
        let session = test_session_id();
        let peer = PeerRef::new(from_node, session);
        let payload = test_payload_inline();
        let packet = Packet::new(PacketType::GetPbftSyncPacket, peer, payload.clone());

        match &packet.payload {
            PacketPayload::Inline { len, buf } => {
                assert_eq!(*len, INLINE_LIMIT);
                assert_eq!(&buf[..*len], &payload[..]);
            }
            PacketPayload::Heap(_) => panic!("expected inline payload storage"),
        }
        assert_eq!(packet.payload(), &payload[..]);
    }

    #[test]
    fn test_payload_larger_than_inline_limit_uses_heap_storage() {
        let from_node = test_node_id();
        let session = test_session_id();
        let peer = PeerRef::new(from_node, session);
        let payload = test_payload_heap();
        let packet = Packet::new(PacketType::LowPriorityPackets, peer, payload.clone());

        match &packet.payload {
            PacketPayload::Heap(bytes) => {
                assert_eq!(bytes.as_ref(), &payload[..]);
            }
            PacketPayload::Inline { .. } => panic!("expected heap payload storage"),
        }
        assert_eq!(packet.payload(), &payload[..]);
    }
}
