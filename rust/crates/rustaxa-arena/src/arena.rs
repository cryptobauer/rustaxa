//! Fixed-slot packet arena for network ingress buffering.
//!
//! The arena stores packet metadata and common small payloads together in
//! preallocated slots so queue processing can usually read packet state and
//! bytes without following an additional heap pointer. Payloads larger than the
//! inline limit keep their [`bytes::Bytes`] allocation to avoid copying large
//! buffers into every arena slot.
//!
//! [`PacketId`](crate::arena::PacketId) combines a monotonic packet identifier
//! with the slot index and generation. Slot indexes are local to one arena and
//! may be reused after removal, while the generation distinguishes stale handles
//! from the packet currently occupying the same slot.

use anyhow::ensure;
use chrono::Utc;
use std::ops::Deref;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use thiserror::Error;

use rustaxa_types::ethereum::NodeId;
use rustaxa_types::time::Microseconds;

static ARENA_COUNTER: AtomicUsize = AtomicUsize::new(0);
static PACKET_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Arena handle assigned when a packet is inserted.
///
/// A packet id pairs a monotonic process-local identifier with the slot index
/// and generation used for fast arena access. The slot index can be reused
/// after removal, so lookups validate the generation before returning a packet.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PacketId {
    /// Monotonic process-local id used to identify packet insertion order.
    internal_id: usize,

    /// Slot index inside the arena.
    index: usize,

    /// Slot generation used to reject stale handles after reuse.
    generation: usize,
}

/// Target in-memory size for a packet stored in the arena.
///
/// Keeping packets at a fixed size gives predictable slot entry layout and
/// keeps common packet payload bytes close to the packet metadata.
#[allow(dead_code)]
const PACKET_SIZE: usize = 2048;

/// Maximum payload size stored directly inside a packet.
///
/// Larger payloads are stored as [`bytes::Bytes`] to avoid copying unusually
/// large buffers into every arena slot.
const INLINE_LIMIT: usize = 1936;

/// Packet payload storage optimized for common small packets.
///
/// Small payloads are stored inline to improve data locality for packets held
/// in arena slots. Large payloads keep their shared [`bytes::Bytes`] allocation.
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

impl Default for PacketPayload {
    fn default() -> Self {
        Self::Inline {
            len: 0,
            buf: [0; INLINE_LIMIT],
        }
    }
}

/// Network packet retained in the arena.
///
/// The packet keeps metadata needed for queueing and peer attribution together
/// with arena-optimized payload storage.
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

impl Default for Packet {
    fn default() -> Self {
        Self {
            id: PacketId::default(),
            received: Microseconds(0),
            from_node: NodeId(ethereum_types::H512::from([0u8; 64])),
            payload: PacketPayload::default(),
        }
    }
}

impl Packet {
    /// Creates a packet from the sender id and payload bytes.
    ///
    /// Payloads up to [`INLINE_LIMIT`] bytes are copied into the packet entry.
    /// Larger payloads retain the provided [`bytes::Bytes`] handle.
    fn new(from_node: NodeId, payload: bytes::Bytes, index: usize, generation: usize) -> Self {
        Packet {
            id: PacketId {
                internal_id: PACKET_COUNTER.fetch_add(1, Ordering::Relaxed),
                index,
                generation,
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

/// Error returned when a packet handle cannot be borrowed or removed.
#[derive(Error, Debug)]
pub enum BorrowError {
    /// The packet slot is currently locked by another reader or remover.
    #[error("Slot is already borrowed")]
    Busy,

    /// The supplied packet handle does not match the packet currently stored in the slot.
    #[error("Packet handle mismatch with current handle: {0:?}")]
    StaleHandle(PacketId),

    /// The supplied packet handle references a slot outside this arena.
    #[error("Packet handle is outside this arena: {0:?}")]
    InvalidHandle(PacketId),

    /// The packet mutex was poisoned by a panic while locked.
    #[error("Packet mutex is poisoned")]
    Poisoned,
}

/// Error returned when a reserved packet slot cannot be filled.
#[derive(Error, Debug)]
pub enum InsertError {
    /// The reservation does not belong to this arena or no longer matches the slot state.
    #[error("Reservation does not match a writable slot")]
    InvalidReservation,

    /// The packet mutex was poisoned by a panic while locked.
    #[error("Packet mutex is poisoned")]
    Poisoned,
}

/// Read guard for a packet borrowed from the arena.
///
/// The guard holds the slot mutex while it is alive and restores the slot state
/// to occupied when dropped.
pub struct PacketReadGuard<'a> {
    slot: &'a Slot,
    packet: MutexGuard<'a, Packet>,
}

impl Deref for PacketReadGuard<'_> {
    type Target = Packet;

    fn deref(&self) -> &Self::Target {
        &self.packet
    }
}

impl Drop for PacketReadGuard<'_> {
    fn drop(&mut self) {
        self.slot
            .state
            .store(SlotState::Occupied.as_u8(), Ordering::Release);
    }
}

struct Slot {
    generation: AtomicUsize,
    state: AtomicU8,
    packet: Mutex<Packet>,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            generation: AtomicUsize::new(0),
            state: AtomicU8::new(SlotState::Free.as_u8()),
            packet: Mutex::new(Packet::default()),
        }
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum SlotState {
    Free = 0,
    Writing = 1,
    Occupied = 2,
    Reading = 3,
}

impl SlotState {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Producer-owned reservation for an arena slot.
///
/// A reservation is created by [`Arena::try_reserve`] after the slot has moved
/// from free to writing. Passing it to [`Arena::insert`] publishes the packet
/// and returns a shareable [`PacketId`].
pub struct Reservation {
    arena_id: usize,
    index: usize,
    generation: usize,
}

/// Fixed-slot packet arena.
///
/// The arena owns packets and returns [`PacketId`] handles for later lookup or
/// removal. Each handle contains an arena-local slot index plus a generation
/// for stale-handle detection.
pub struct Arena {
    id: usize,
    size: usize,
    bitmask: usize,
    slots: Vec<Slot>,
    next_slot: AtomicUsize,
}

impl Arena {
    const RESERVE_ATTEMPTS: u8 = 16;

    /// Creates an empty arena with exactly `size` packet slots.
    ///
    /// The size must be a power of two so slot indexes can be selected from a
    /// wrapping forward scan.
    pub fn new(size: usize) -> Result<Self, anyhow::Error> {
        ensure!(size.is_power_of_two(), "arena size must be power of 2");
        let slots = (0..size).map(|_| Slot::default()).collect();
        Ok(Arena {
            id: ARENA_COUNTER.fetch_add(1, Ordering::Relaxed),
            bitmask: size - 1,
            size,
            slots,
            next_slot: AtomicUsize::new(0),
        })
    }

    /// Reserves a free slot for packet insertion.
    ///
    /// A reservation is producer-owned and is not visible to consumers until it
    /// is passed to [`Arena::insert`].
    pub fn try_reserve(&self) -> Option<Reservation> {
        for _ in 0..Arena::RESERVE_ATTEMPTS {
            let next_free = self.next_slot.fetch_add(1, Ordering::AcqRel) & self.bitmask;
            let slot = &self.slots[next_free];

            if slot
                .state
                .compare_exchange(
                    SlotState::Free.as_u8(),
                    SlotState::Writing.as_u8(),
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Some(Reservation {
                    arena_id: self.id,
                    index: next_free,
                    generation: slot.generation.load(Ordering::Relaxed),
                });
            }
        }

        None
    }

    /// Inserts a packet into a previously reserved slot and returns its handle.
    ///
    /// The returned [`PacketId`] includes the slot index, slot generation, and
    /// packet's monotonic process-local id. Reservations from another arena or
    /// from a slot that is no longer writable are rejected.
    pub fn insert(
        &self,
        reservation: Reservation,
        from_node: NodeId,
        payload: bytes::Bytes,
    ) -> Result<PacketId, InsertError> {
        if reservation.arena_id != self.id {
            return Err(InsertError::InvalidReservation);
        }

        let Some(slot) = self.slots.get(reservation.index) else {
            return Err(InsertError::InvalidReservation);
        };

        if slot.state.load(Ordering::Acquire) != SlotState::Writing.as_u8()
            || slot.generation.load(Ordering::Acquire) != reservation.generation
        {
            return Err(InsertError::InvalidReservation);
        }

        let packet = Packet::new(
            from_node,
            payload,
            reservation.index,
            reservation.generation,
        );
        let packet_id = packet.id;
        *slot.packet.lock().map_err(|_| InsertError::Poisoned)? = packet;
        slot.state
            .store(SlotState::Occupied.as_u8(), Ordering::Relaxed);

        Ok(packet_id)
    }

    /// Removes a packet from the arena and frees the slot for reuse.
    ///
    /// Returns [`BorrowError::StaleHandle`] when the supplied [`PacketId`]
    /// refers to an older generation of the same slot.
    pub fn remove(&self, id: PacketId) -> Result<bool, BorrowError> {
        let Some(slot) = self.slots.get(id.index) else {
            return Err(BorrowError::InvalidHandle(id));
        };

        let packet = match slot.packet.try_lock() {
            Ok(packet) => packet,
            Err(TryLockError::WouldBlock) => return Err(BorrowError::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(BorrowError::Poisoned),
        };

        if slot.state.load(Ordering::Acquire) != SlotState::Occupied.as_u8() || packet.id != id {
            return Err(BorrowError::StaleHandle(packet.id));
        }

        slot.generation.fetch_add(1, Ordering::Relaxed);
        slot.state.store(SlotState::Free.as_u8(), Ordering::Relaxed);

        Ok(true)
    }

    /// Returns the packet identified by `packet_id`, if it is still present.
    ///
    /// The slot generation must match, so stale packet ids do not resolve to
    /// newer packets that reused the same slot.
    pub fn borrow(&self, id: PacketId) -> Result<PacketReadGuard<'_>, BorrowError> {
        let Some(slot) = self.slots.get(id.index) else {
            return Err(BorrowError::InvalidHandle(id));
        };

        let packet = match slot.packet.try_lock() {
            Ok(packet) => packet,
            Err(TryLockError::WouldBlock) => return Err(BorrowError::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(BorrowError::Poisoned),
        };

        if slot.state.load(Ordering::Acquire) != SlotState::Occupied.as_u8() || packet.id != id {
            return Err(BorrowError::StaleHandle(packet.id));
        }

        slot.state
            .store(SlotState::Reading.as_u8(), Ordering::Release);

        Ok(PacketReadGuard { slot, packet })
    }

    /// Returns the fixed slot capacity of the arena.
    pub fn capacity(&self) -> usize {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethereum_types::H512;
    use std::mem;
    use std::sync::{Arc, Barrier};
    use std::thread;

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
        let packet1 = Packet::new(from_node, payload.clone(), 0, 0);
        let checkpoint2 = chrono::Utc::now().timestamp_micros();

        assert!(packet1.received >= Microseconds(checkpoint1 as u64));
        assert!(packet1.received <= Microseconds(checkpoint2 as u64));
        assert_eq!(packet1.from_node, from_node);
        assert_eq!(packet1.payload(), [0xc3, 0x01, 0x02, 0x03]);
        assert_eq!(packet1.id.index, 0);
        assert_eq!(packet1.id.generation, 0);
    }

    #[test]
    fn test_packet_id_increments() {
        let from_node = test_nodeid();
        let payload = test_payload1();
        let packet1 = Packet::new(from_node, payload.clone(), 0, 0);
        let packet2 = Packet::new(from_node, payload.clone(), 1, 0);
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
        let packet = Packet::new(from_node, payload.clone(), 0, 0);
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
        let packet = Packet::new(from_node, payload.clone(), 0, 0);
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
        let arena = Arena::new(1024).expect("power-of-two arena size should be valid");
        assert_eq!(arena.capacity(), 1024);
        assert_eq!(arena.next_slot.load(Ordering::Relaxed), 0);
        assert!(
            arena
                .slots
                .iter()
                .all(|slot| slot.state.load(Ordering::Relaxed) == SlotState::Free.as_u8())
        );
    }

    #[test]
    fn test_arena_rejects_non_power_of_two_size() {
        assert!(Arena::new(0).is_err());
        assert!(Arena::new(6).is_err());
    }

    #[test]
    fn test_reserve_scans_forward_and_reports_full() {
        let arena = Arena::new(2).expect("arena should be created");

        let reservation1 = arena.try_reserve().expect("first slot should be reserved");
        let reservation2 = arena.try_reserve().expect("second slot should be reserved");

        assert_eq!(reservation1.index, 0);
        assert_eq!(reservation2.index, 1);
        assert!(arena.try_reserve().is_none());
    }

    #[test]
    fn test_reserve_scans_past_first_thirty_two_occupied_slots() {
        let arena = Arena::new(64).expect("arena should be created");
        let from_node = test_nodeid();
        let mut keys = Vec::with_capacity(64);

        for _ in 0..64 {
            let reservation = arena.try_reserve().expect("slot should be reserved");
            keys.push(
                arena
                    .insert(reservation, from_node, test_payload1())
                    .expect("insert should succeed"),
            );
        }

        arena
            .remove(keys[8])
            .expect("packet beyond first 32 slots should be removed");

        let reservation = arena
            .try_reserve()
            .expect("forward scan should find the free slot beyond 32 probes");
        assert_eq!(reservation.index, 8);
    }

    #[test]
    fn test_arena_insert_and_borrow() {
        let arena = Arena::new(4).expect("arena should be created");
        let from_node = test_nodeid();
        let payload = test_payload1();
        let expected_payload = payload.clone();
        let expected_node = from_node;
        let reservation = arena.try_reserve().expect("slot should be reserved");

        let packet_id = arena
            .insert(reservation, from_node, payload)
            .expect("insert should succeed");

        assert_eq!(arena.capacity(), 4);
        let retrieved = arena.borrow(packet_id).expect("packet should be present");
        assert_eq!(retrieved.id.index, packet_id.index);
        assert_eq!(retrieved.id.generation, packet_id.generation);
        assert_eq!(retrieved.from_node, expected_node);
        assert_eq!(retrieved.payload(), expected_payload.as_ref());
    }

    #[test]
    fn test_insert_rejects_cross_arena_reservation() {
        let arena_a = Arena::new(1).expect("first arena should be created");
        let arena_b = Arena::new(1).expect("second arena should be created");
        let from_node = test_nodeid();
        let reservation = arena_a.try_reserve().expect("slot should be reserved");

        assert!(matches!(
            arena_b.insert(reservation, from_node, test_payload1()),
            Err(InsertError::InvalidReservation)
        ));
    }

    #[test]
    fn test_insert_rejects_stale_reservation_after_slot_reuse() {
        let arena = Arena::new(1).expect("arena should be created");
        let from_node = test_nodeid();
        let stale_reservation = Reservation {
            arena_id: arena.id,
            index: 0,
            generation: 0,
        };

        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, from_node, test_payload1())
            .expect("insert should succeed");
        arena.remove(key).expect("packet should be removed");

        let current_reservation = arena.try_reserve().expect("slot should be reusable");
        assert_eq!(current_reservation.generation, 1);

        assert!(matches!(
            arena.insert(stale_reservation, from_node, test_payload_inline()),
            Err(InsertError::InvalidReservation)
        ));
    }

    #[test]
    fn test_borrow_and_remove_reject_out_of_range_handle() {
        let arena = Arena::new(1).expect("arena should be created");
        let invalid = PacketId {
            internal_id: 0,
            index: 99,
            generation: 0,
        };

        assert!(matches!(
            arena.borrow(invalid),
            Err(BorrowError::InvalidHandle(handle)) if handle == invalid
        ));
        assert!(matches!(
            arena.remove(invalid),
            Err(BorrowError::InvalidHandle(handle)) if handle == invalid
        ));
    }

    #[test]
    fn test_borrow_and_remove_reject_free_and_writing_slots() {
        let arena = Arena::new(2).expect("arena should be created");
        let free_handle = PacketId {
            internal_id: 0,
            index: 0,
            generation: 0,
        };

        assert!(matches!(
            arena.borrow(free_handle),
            Err(BorrowError::StaleHandle(_))
        ));
        assert!(matches!(
            arena.remove(free_handle),
            Err(BorrowError::StaleHandle(_))
        ));

        let reservation = arena.try_reserve().expect("slot should be reserved");
        let writing_handle = PacketId {
            internal_id: 0,
            index: reservation.index,
            generation: reservation.generation,
        };

        assert!(matches!(
            arena.borrow(writing_handle),
            Err(BorrowError::StaleHandle(_))
        ));
        assert!(matches!(
            arena.remove(writing_handle),
            Err(BorrowError::StaleHandle(_))
        ));
    }

    #[test]
    fn test_arena_insert_multiple_and_borrow() {
        let arena = Arena::new(4).expect("arena should be created");
        let from_node = test_nodeid();

        let reservation1 = arena.try_reserve().expect("first slot should be reserved");
        let key1 = arena
            .insert(reservation1, from_node, test_payload1())
            .expect("insert should succeed");
        let reservation2 = arena.try_reserve().expect("second slot should be reserved");
        let key2 = arena
            .insert(reservation2, from_node, test_payload_inline())
            .expect("insert should succeed");
        let reservation3 = arena.try_reserve().expect("third slot should be reserved");
        let key3 = arena
            .insert(reservation3, from_node, test_payload_heap())
            .expect("insert should succeed");

        assert_eq!(arena.capacity(), 4);
        assert_eq!(
            arena.borrow(key1).unwrap().payload(),
            [0xc3, 0x01, 0x02, 0x03]
        );
        assert_eq!(
            arena.borrow(key2).unwrap().payload(),
            vec![0xAB; INLINE_LIMIT].as_slice()
        );
        assert_eq!(
            arena.borrow(key3).unwrap().payload(),
            vec![0xCD; INLINE_LIMIT + 1].as_slice()
        );
    }

    #[test]
    fn test_arena_remove() {
        let arena = Arena::new(4).expect("arena should be created");
        let from_node = test_nodeid();
        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, from_node, test_payload1())
            .expect("insert should succeed");

        assert_eq!(arena.capacity(), 4);
        assert_eq!(
            arena.borrow(key).unwrap().payload(),
            [0xc3, 0x01, 0x02, 0x03]
        );
        assert!(arena.remove(key).unwrap());
        assert_eq!(arena.capacity(), 4);
        assert!(matches!(
            arena.borrow(key),
            Err(BorrowError::StaleHandle(_))
        ));
    }

    #[test]
    fn test_borrow_guard_blocks_other_borrowers_until_drop() {
        let arena = Arena::new(4).expect("arena should be created");
        let from_node = test_nodeid();
        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, from_node, test_payload1())
            .expect("insert should succeed");

        let guard = arena.borrow(key).expect("packet should be borrowed");

        assert!(matches!(arena.borrow(key), Err(BorrowError::Busy)));
        assert!(matches!(arena.remove(key), Err(BorrowError::Busy)));

        drop(guard);

        assert_eq!(
            arena.borrow(key).unwrap().payload(),
            [0xc3, 0x01, 0x02, 0x03]
        );
    }

    #[test]
    fn test_remove_frees_slot_for_new_generation() {
        let arena = Arena::new(2).expect("arena should be created");
        let from_node = test_nodeid();

        let first_reservation = arena.try_reserve().expect("first slot should be reserved");
        assert_eq!(first_reservation.index, 0);
        let first_key = arena
            .insert(first_reservation, from_node, test_payload1())
            .expect("insert should succeed");

        let second_reservation = arena.try_reserve().expect("second slot should be reserved");
        assert_eq!(second_reservation.index, 1);
        let second_key = arena
            .insert(second_reservation, from_node, test_payload_inline())
            .expect("insert should succeed");

        arena
            .remove(first_key)
            .expect("first packet should be removed");

        let reused_reservation = arena.try_reserve().expect("freed slot should be reserved");
        assert_eq!(reused_reservation.index, first_key.index);
        assert_eq!(reused_reservation.generation, first_key.generation + 1);

        let reused_key = arena
            .insert(reused_reservation, from_node, test_payload_heap())
            .expect("insert should succeed");
        assert_eq!(reused_key.index, first_key.index);
        assert_eq!(reused_key.generation, first_key.generation + 1);
        assert_ne!(reused_key, first_key);

        assert!(matches!(
            arena.borrow(first_key),
            Err(BorrowError::StaleHandle(_))
        ));
        assert_eq!(
            arena.borrow(reused_key).unwrap().payload(),
            vec![0xCD; INLINE_LIMIT + 1].as_slice()
        );
        assert_eq!(
            arena.borrow(second_key).unwrap().payload(),
            vec![0xAB; INLINE_LIMIT].as_slice()
        );
    }

    #[test]
    fn test_remove_rejects_stale_handle_after_reuse() {
        let arena = Arena::new(1).expect("arena should be created");
        let from_node = test_nodeid();

        let first_reservation = arena.try_reserve().expect("slot should be reserved");
        let first_key = arena
            .insert(first_reservation, from_node, test_payload1())
            .expect("insert should succeed");
        arena.remove(first_key).expect("packet should be removed");

        let reused_reservation = arena.try_reserve().expect("slot should be reusable");
        let reused_key = arena
            .insert(reused_reservation, from_node, test_payload_inline())
            .expect("insert should succeed");

        assert!(matches!(
            arena.remove(first_key),
            Err(BorrowError::StaleHandle(current)) if current == reused_key
        ));
    }

    #[test]
    fn test_borrow_reports_poisoned_mutex() {
        let arena = Arc::new(Arena::new(1).expect("arena should be created"));
        let from_node = test_nodeid();
        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, from_node, test_payload1())
            .expect("insert should succeed");

        let poisoner = {
            let arena = Arc::clone(&arena);
            thread::spawn(move || {
                let _guard = arena.slots[key.index].packet.lock().unwrap();
                panic!("poison packet mutex");
            })
        };
        assert!(poisoner.join().is_err());

        assert!(matches!(arena.borrow(key), Err(BorrowError::Poisoned)));
        assert!(matches!(arena.remove(key), Err(BorrowError::Poisoned)));
    }

    #[test]
    fn test_insert_reports_poisoned_mutex() {
        let arena = Arc::new(Arena::new(1).expect("arena should be created"));

        let poisoner = {
            let arena = Arc::clone(&arena);
            thread::spawn(move || {
                let _guard = arena.slots[0].packet.lock().unwrap();
                panic!("poison packet mutex");
            })
        };
        assert!(poisoner.join().is_err());

        let reservation = arena.try_reserve().expect("slot should be reserved");
        assert!(matches!(
            arena.insert(reservation, test_nodeid(), test_payload1()),
            Err(InsertError::Poisoned)
        ));
    }

    #[test]
    fn test_concurrent_reservations_are_unique() {
        const THREADS: usize = 8;

        let arena = Arc::new(Arena::new(THREADS).expect("arena should be created"));
        let barrier = Arc::new(Barrier::new(THREADS));
        let mut handles = Vec::with_capacity(THREADS);

        for _ in 0..THREADS {
            let arena = Arc::clone(&arena);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                arena
                    .try_reserve()
                    .expect("each thread should reserve one slot")
            }));
        }

        let mut reservations = handles
            .into_iter()
            .map(|handle| handle.join().expect("reservation thread should not panic"))
            .collect::<Vec<_>>();

        reservations.sort_by_key(|reservation| reservation.index);
        assert_eq!(
            reservations
                .iter()
                .map(|reservation| reservation.index)
                .collect::<Vec<_>>(),
            (0..THREADS).collect::<Vec<_>>()
        );
        assert!(
            reservations
                .iter()
                .all(|reservation| reservation.generation == 0)
        );
        assert!(arena.try_reserve().is_none());
    }

    #[test]
    fn test_concurrent_producers_can_reserve_and_insert() {
        const THREADS: usize = 8;

        let arena = Arc::new(Arena::new(THREADS).expect("arena should be created"));
        let barrier = Arc::new(Barrier::new(THREADS));
        let from_node = test_nodeid();
        let mut handles = Vec::with_capacity(THREADS);

        for thread_id in 0..THREADS {
            let arena = Arc::clone(&arena);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                let reservation = arena.try_reserve().expect("slot should be reserved");
                let payload = Bytes::from(vec![thread_id as u8; 4]);
                arena
                    .insert(reservation, from_node, payload)
                    .expect("insert should succeed")
            }));
        }

        let keys = handles
            .into_iter()
            .map(|handle| handle.join().expect("producer thread should not panic"))
            .collect::<Vec<_>>();

        assert_eq!(keys.len(), THREADS);
        for key in keys {
            let packet = arena
                .borrow(key)
                .expect("inserted packet should be readable");
            assert_eq!(packet.payload(), vec![packet.payload()[0]; 4].as_slice());
        }
        assert!(arena.try_reserve().is_none());
    }

    #[test]
    fn test_cross_thread_borrow_blocks_remove_until_guard_drops() {
        let arena = Arena::new(1).expect("arena should be created");
        let from_node = test_nodeid();
        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, from_node, test_payload1())
            .expect("insert should succeed");
        let arena = Arc::new(arena);
        let borrowed = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));

        let reader = {
            let arena = Arc::clone(&arena);
            let borrowed = Arc::clone(&borrowed);
            let release = Arc::clone(&release);
            thread::spawn(move || {
                let guard = arena.borrow(key).expect("reader should borrow packet");
                borrowed.wait();
                release.wait();
                assert_eq!(guard.payload(), [0xc3, 0x01, 0x02, 0x03]);
            })
        };

        borrowed.wait();
        assert!(matches!(arena.remove(key), Err(BorrowError::Busy)));
        release.wait();
        reader.join().expect("reader thread should not panic");

        assert!(
            arena
                .remove(key)
                .expect("remove should succeed after guard drop")
        );
    }

    #[test]
    fn test_cross_thread_remove_allows_reuse_on_main_thread() {
        let arena = Arena::new(1).expect("arena should be created");
        let from_node = test_nodeid();
        let first_reservation = arena.try_reserve().expect("slot should be reserved");
        let first_key = arena
            .insert(first_reservation, from_node, test_payload1())
            .expect("insert should succeed");
        let arena = Arc::new(arena);

        let remover = {
            let arena = Arc::clone(&arena);
            thread::spawn(move || arena.remove(first_key).expect("remove should succeed"))
        };

        assert!(remover.join().expect("remove thread should not panic"));

        let reused_reservation = arena.try_reserve().expect("slot should be reusable");
        assert_eq!(reused_reservation.index, first_key.index);
        assert_eq!(reused_reservation.generation, first_key.generation + 1);
    }
}
