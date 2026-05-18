//! Fixed-size packet arena for network ingress buffering.
//!
//! The arena stores packet metadata and common small payloads together in slab
//! entries so queue processing can usually read packet state and bytes without
//! following an additional heap pointer. Payloads larger than the inline limit
//! keep their [`bytes::Bytes`] allocation to avoid copying large buffers into
//! every arena slot.
//!
//! [`PacketId`] combines a monotonic packet identifier with the packet's slab
//! storage key. The storage key is local to one arena and may be reused after
//! removal, while the monotonic identifier distinguishes stale handles from the
//! packet currently occupying the same slab slot.

use chrono::Utc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rustaxa_types::ethereum::NodeId;
use rustaxa_types::time::Microseconds;

static PACKET_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Arena handle assigned when a packet is inserted.
///
/// A packet id pairs a monotonic process-local identifier with the slab storage
/// key used for fast arena access. The storage key can be reused after removal,
/// so lookups should validate both fields before returning a packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PacketId {
    /// Monotonic process-local id used to distinguish stale slab handles.
    internal_id: usize,

    /// Slab key where the packet was inserted.
    storage_key: usize,
}

/// Target in-memory size for a packet stored in the arena.
///
/// Keeping packets at a fixed size gives predictable slab entry layout and
/// keeps common packet payload bytes close to the packet metadata.
#[allow(dead_code)]
const PACKET_SIZE: usize = 2048;

/// Maximum payload size stored directly inside a packet.
///
/// Larger payloads are stored as [`bytes::Bytes`] to avoid copying unusually
/// large buffers into every arena slot.
const INLINE_LIMIT: usize = 1944;

/// Packet payload storage optimized for common small packets.
///
/// Small payloads are stored inline to improve data locality for packets held
/// in the slab. Large payloads keep their shared [`bytes::Bytes`] allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum PacketPayload {
    /// Payload bytes stored directly in the packet arena entry.
    ///
    /// The `len` field records the number of initialized bytes in `buf`.
    Inline { len: usize, buf: [u8; INLINE_LIMIT] },

    /// Payload bytes stored outside the fixed-size packet entry.
    Heap(bytes::Bytes),
}

/// Network packet retained in the arena.
///
/// The packet keeps metadata needed for queueing and peer attribution together
/// with payload storage chosen by [`PacketPayload`].
#[derive(Debug, Clone)]
pub struct Packet {
    /// Monotonic process-local packet identifier.
    pub id: PacketId,

    /// Wall-clock receive timestamp in microseconds.
    pub received: Microseconds,

    /// Node that sent the packet.
    pub from_node: NodeId,

    /// Packet bytes, stored inline when they fit in [`INLINE_LIMIT`].
    payload: PacketPayload,
}

impl Packet {
    /// Creates a packet from the sender id and payload bytes.
    ///
    /// Payloads up to [`INLINE_LIMIT`] bytes are copied into the packet entry.
    /// Larger payloads retain the provided [`bytes::Bytes`] handle.
    fn new(from_node: NodeId, payload: bytes::Bytes, storage_key: usize) -> Self {
        Packet {
            id: PacketId {
                internal_id: PACKET_COUNTER.fetch_add(1, Ordering::Relaxed),
                storage_key,
            },
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

/// Slab-backed packet arena.
///
/// The arena owns packets and returns [`PacketId`] handles for later lookup or
/// removal. Each handle contains an arena-local slab key plus a monotonic id for
/// stale-handle detection.
pub struct Arena {
    store: slab::Slab<Packet>,
}

impl Arena {
    /// Creates an empty arena with storage reserved for at least `capacity` packets.
    pub fn new(capacity: usize) -> Self {
        Arena {
            store: slab::Slab::with_capacity(capacity),
        }
    }

    /// Inserts a packet and returns its arena handle.
    ///
    /// The returned [`PacketId`] includes both the slab storage key and the
    /// packet's monotonic process-local id.
    pub fn insert(&mut self, from_node: NodeId, payload: bytes::Bytes) -> PacketId {
        let key = self.store.vacant_key();
        let packet = Packet::new(from_node, payload, key);
        let packet_id = packet.id;
        self.store.insert(packet);
        packet_id
    }

    /// Returns the packet identified by `packet_id`, if it is still present.
    ///
    /// Both the slab storage key and monotonic id must match, so stale packet
    /// ids do not resolve to newer packets that reused the same slab slot.
    pub fn get(&self, packet_id: PacketId) -> Option<&Packet> {
        self.store
            .get(packet_id.storage_key)
            .filter(|packet| packet.id == packet_id)
    }

    /// Removes and returns the packet at `packet_id`'s storage key, if occupied.
    ///
    /// This uses the storage key embedded in [`PacketId`]. Callers that may hold
    /// stale ids should check [`Arena::get`] first if they need the monotonic id
    /// to match before removal.
    pub fn try_remove(&mut self, packet_id: PacketId) -> Option<Packet> {
        self.store.try_remove(packet_id.storage_key)
    }

    /// Returns the number of packets currently stored in the arena.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Returns `true` when the arena contains no packets.
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethereum_types::H512;
    use std::mem;

    fn test_nodeid() -> NodeId {
        // All zeros except last byte for uniqueness
        let mut arr = [0u8; 64];
        arr[63] = 1;
        NodeId(H512::from(arr))
    }

    fn test_payload1() -> Bytes {
        // Simple RLP-encoded list [1, 2, 3]
        Bytes::from(vec![0xc3, 0x01, 0x02, 0x03])
    }

    fn test_payload_inline() -> Bytes {
        // Exactly INLINE_LIMIT bytes
        Bytes::from(vec![0xAB; INLINE_LIMIT])
    }

    fn test_payload_heap() -> Bytes {
        // INLINE_LIMIT + 1 bytes
        Bytes::from(vec![0xCD; INLINE_LIMIT + 1])
    }

    #[test]
    fn test_packet_create() {
        let from_node = test_nodeid();
        let payload = test_payload1();
        let checkpoint1 = chrono::Utc::now().timestamp_micros();
        let packet1 = Packet::new(from_node, payload.clone(), 0);
        let checkpoint2 = chrono::Utc::now().timestamp_micros();

        assert!(packet1.received >= Microseconds(checkpoint1 as u64));
        assert!(packet1.received <= Microseconds(checkpoint2 as u64));
        assert_eq!(packet1.from_node, from_node);
        assert_eq!(packet1.payload(), [0xc3, 0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_packet_id_increments() {
        let from_node = test_nodeid();
        let payload = test_payload1();
        let packet1 = Packet::new(from_node, payload.clone(), 0);
        let packet2 = Packet::new(from_node, payload.clone(), 1);
        assert!(
            packet2.id.internal_id > packet1.id.internal_id,
            "PacketId should increment"
        );
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
        let from_node = test_nodeid();
        let payload = test_payload_inline();
        let packet = Packet::new(from_node, payload.clone(), 0);
        // Should use Inline variant
        match &packet.payload {
            PacketPayload::Inline { len, buf } => {
                assert_eq!(*len, INLINE_LIMIT);
                assert_eq!(&buf[..*len], &payload[..]);
            }
            _ => panic!("Expected Inline variant for small buffer optimization"),
        }
        assert_eq!(packet.payload(), &payload[..]);
    }

    #[test]
    fn test_small_buffer_optimization_heap() {
        let from_node = test_nodeid();
        let payload = test_payload_heap();
        let packet = Packet::new(from_node, payload.clone(), 0);
        // Should use Heap variant
        match &packet.payload {
            PacketPayload::Heap(bytes) => {
                assert_eq!(bytes.as_ref(), &payload[..]);
            }
            _ => panic!("Expected Heap variant for large buffer"),
        }
        assert_eq!(packet.payload(), &payload[..]);
    }

    #[test]
    fn test_arena_creation() {
        let arena = Arena::new(1024);
        assert_eq!(arena.store.len(), 0);
        assert!(arena.store.capacity() >= 1024);
    }

    #[test]
    fn test_arena_insert_and_get() {
        let mut arena = Arena::new(4);
        let from_node = test_nodeid();
        let payload = test_payload1();
        let expected_payload = payload.clone();
        let expected_node = from_node;

        let packet_id = arena.insert(from_node, payload);

        assert_eq!(arena.len(), 1);
        let retrieved = arena.get(packet_id).expect("packet should be present");
        assert_eq!(retrieved.id.storage_key, packet_id.storage_key);
        assert_eq!(retrieved.from_node, expected_node);
        assert_eq!(retrieved.payload(), expected_payload.as_ref());
    }

    #[test]
    fn test_arena_insert_multiple_and_get() {
        let mut arena = Arena::new(4);
        let from_node = test_nodeid();

        let key1 = arena.insert(from_node, test_payload1());
        let key2 = arena.insert(from_node, test_payload_inline());
        let key3 = arena.insert(from_node, test_payload_heap());

        assert_eq!(arena.len(), 3);
        assert_eq!(arena.get(key1).unwrap().payload(), [0xc3, 0x01, 0x02, 0x03]);
        assert_eq!(
            arena.get(key2).unwrap().payload(),
            vec![0xAB; INLINE_LIMIT].as_slice()
        );
        assert_eq!(
            arena.get(key3).unwrap().payload(),
            vec![0xCD; INLINE_LIMIT + 1].as_slice()
        );
    }

    #[test]
    fn test_arena_remove() {
        let mut arena = Arena::new(4);
        let from_node = test_nodeid();
        let key = arena.insert(from_node, test_payload1());

        assert_eq!(arena.len(), 1);
        let removed = arena.try_remove(key).unwrap();
        assert_eq!(removed.payload(), [0xc3, 0x01, 0x02, 0x03]);
        assert_eq!(arena.len(), 0);
        assert!(arena.get(key).is_none());
    }
}
