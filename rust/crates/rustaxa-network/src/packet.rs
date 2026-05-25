use chrono::Utc;
use rustaxa_types::ethereum::NodeId;
use rustaxa_types::time::Microseconds;

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
const INLINE_LIMIT: usize = 1960;

/// Packet payload storage optimized for common small packets.
///
/// Small payloads are stored inline to improve data locality. Large payloads
/// keep their shared [`bytes::Bytes`] allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum PacketPayload {
    /// Payload bytes stored directly in the packet value.
    ///
    /// The `len` field records the number of initialized bytes in `buf`.
    Inline { len: usize, buf: [u8; INLINE_LIMIT] },

    /// Payload bytes stored outside the fixed-size packet entry.
    Heap(bytes::Bytes),
}

impl Default for PacketPayload {
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
#[derive(Debug, Clone)]
pub struct Packet {
    /// Wall-clock receive timestamp in microseconds.
    pub received: Microseconds,

    /// Node that sent the packet.
    pub from_node: NodeId,

    /// Packet bytes, stored inline when they fit in the inline payload limit.
    payload: PacketPayload,
}

impl Default for Packet {
    fn default() -> Self {
        Self {
            received: Microseconds(0),
            from_node: NodeId(ethereum_types::H512::from([0u8; 64])),
            payload: PacketPayload::default(),
        }
    }
}

impl Packet {
    /// Creates a packet from the sender id and payload bytes.
    ///
    /// Payloads up to the inline payload limit are copied into the packet entry.
    /// Larger payloads retain the provided [`bytes::Bytes`] handle.
    pub fn new(from_node: NodeId, payload: bytes::Bytes) -> Self {
        Packet {
            received: Microseconds(Utc::now().timestamp_micros() as u64),
            from_node,
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
    fn test_packet_create() {
        let from_node = test_node_id();
        let payload = test_payload1();
        let checkpoint1 = Utc::now().timestamp_micros();
        let packet = Packet::new(from_node, payload.clone());
        let checkpoint2 = Utc::now().timestamp_micros();

        assert!(packet.received >= Microseconds(checkpoint1 as u64));
        assert!(packet.received <= Microseconds(checkpoint2 as u64));
        assert_eq!(packet.from_node, from_node);
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
    fn test_small_buffer_optimization_inline() {
        let from_node = test_node_id();
        let payload = test_payload_inline();
        let packet = Packet::new(from_node, payload.clone());

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
    fn test_small_buffer_optimization_heap() {
        let from_node = test_node_id();
        let payload = test_payload_heap();
        let packet = Packet::new(from_node, payload.clone());

        match &packet.payload {
            PacketPayload::Heap(bytes) => {
                assert_eq!(bytes.as_ref(), &payload[..]);
            }
            PacketPayload::Inline { .. } => panic!("expected heap payload storage"),
        }
        assert_eq!(packet.payload(), &payload[..]);
    }
}
