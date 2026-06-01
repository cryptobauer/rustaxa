//! Fixed-slot arena for bounded handoff pipelines.
//!
//! The arena stores generic values in preallocated slots and returns stable
//! handles that can be sent through an external queue. Producers reserve a slot,
//! write a value, and publish the returned [`SlotId`](crate::arena::SlotId).
//! Consumers borrow the value by handle and remove it when processing is
//! complete.
//!
//! [`SlotId`](crate::arena::SlotId) combines the arena id, slot index, and slot
//! generation. Slot indexes are local to one arena and may be reused after
//! removal, while the generation distinguishes stale handles from the value
//! currently occupying the same slot.

use anyhow::ensure;
use crossbeam_utils::CachePadded;
use std::ops::Deref;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use thiserror::Error;

/// Process-local counter used to assign each arena a distinct id.
static ARENA_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Maximum number of slots a reservation attempt probes before reporting full.
const RESERVER_ATTEMPTS: u8 = 16;

/// Fixed-slot arena.
///
/// The arena owns values and returns [`SlotId`] handles for later lookup or
/// removal. Each handle contains the arena id, an arena-local slot index, and a
/// generation for stale-handle detection.
pub struct Arena<T> {
    /// Process-local id used to reject handles from another arena.
    id: usize,

    /// Fixed number of slots owned by this arena.
    size: usize,

    /// Hot reservation cursor isolated from unrelated arena fields.
    cursor: CachePadded<ArenaCursor>,

    /// Preallocated fixed-size slot storage.
    slots: CachePadded<Vec<SlotMeta>>,

    /// Stored value protected while a reader or remover holds the slot.
    data: CachePadded<Vec<Mutex<T>>>,
}

/// Producer reservation state used together on the hot reservation path.
struct ArenaCursor {
    /// Next candidate slot index for producer reservation.
    next: AtomicUsize,

    /// Mask used to wrap monotonically increasing slot positions.
    bitmask: usize,
}

impl<T> Arena<T>
where
    T: Default,
{
    /// Creates an empty arena with exactly `size` slots.
    ///
    /// The size must be a power of two so slot indexes can be selected from a
    /// wrapping forward scan.
    pub fn new(size: usize) -> Result<Self, anyhow::Error> {
        ensure!(size.is_power_of_two(), "arena size must be power of 2");
        let slots = (0..size).map(|_| SlotMeta::default()).collect();
        let data = (0..size).map(|_| Mutex::new(T::default())).collect();
        Ok(Arena {
            id: ARENA_COUNTER.fetch_add(1, Ordering::Relaxed),
            size,
            cursor: CachePadded::new(ArenaCursor {
                next: AtomicUsize::new(0),
                bitmask: size - 1,
            }),
            slots: CachePadded::new(slots),
            data: CachePadded::new(data),
        })
    }

    /// Reserves a free slot for value insertion.
    ///
    /// A reservation is producer-owned and is not visible to consumers until it
    /// is passed to [`Arena::insert`]. Returns [`TryReserveError::AttemptsExceeded`]
    /// when the bounded forward scan does not find a free slot.
    pub fn try_reserve(&self) -> Result<SlotReservationGuard<'_>, TryReserveError> {
        for _ in 0..RESERVER_ATTEMPTS {
            let next_free = self.cursor.next.fetch_add(1, Ordering::AcqRel) & self.cursor.bitmask;
            let slot = &self.slots[next_free];

            if slot
                .state
                .compare_exchange(
                    SlotState::Free.as_u8(),
                    SlotState::Reserved.as_u8(),
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Ok(SlotReservationGuard {
                    slot,
                    arena_id: self.id,
                    index: next_free,
                    generation: slot.generation.load(Ordering::Relaxed),
                });
            }
        }

        Err(TryReserveError::AttemptsExceeded {
            attempts: RESERVER_ATTEMPTS,
        })
    }

    /// Inserts a value into a previously reserved slot and returns its handle.
    ///
    /// The returned [`SlotId`] includes this arena's id, the slot index, and
    /// the slot generation. Reservations from another arena or from a slot that
    /// is no longer writable are rejected.
    pub fn insert(
        &self,
        reservation: SlotReservationGuard<'_>,
        data: T,
    ) -> Result<SlotId, InsertError> {
        if reservation.arena_id != self.id {
            return Err(InsertError::InvalidReservation);
        }

        let Some(slot) = self.slots.get(reservation.index) else {
            return Err(InsertError::InvalidReservation);
        };

        if slot.state.load(Ordering::Acquire) != SlotState::Reserved.as_u8()
            || slot.generation.load(Ordering::Acquire) != reservation.generation
        {
            return Err(InsertError::InvalidReservation);
        }

        *self.data[reservation.index]
            .lock()
            .map_err(|_| InsertError::Poisoned)? = data;
        slot.state
            .store(SlotState::Occupied.as_u8(), Ordering::Release);

        Ok(SlotId {
            arena_id: self.id,
            index: reservation.index,
            generation: reservation.generation,
        })
    }

    /// Removes a value from the arena and frees the slot for reuse.
    ///
    /// Returns [`BorrowError::StaleHandle`] when the supplied [`SlotId`]
    /// refers to an older generation of the same slot or a value from another
    /// arena at the same slot index.
    pub fn remove(&self, id: SlotId) -> Result<bool, BorrowError> {
        let Some(slot) = self.slots.get(id.index) else {
            return Err(BorrowError::InvalidHandle(id));
        };

        // Fast first check before mutex.
        self.ensure_occupied_handle(slot, id)?;

        match self.data[id.index].try_lock() {
            Err(TryLockError::WouldBlock) => return Err(BorrowError::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(BorrowError::Poisoned),
            Ok(_) => (),
        };

        // Check again under mutex.
        self.ensure_occupied_handle(slot, id)?;

        slot.generation.fetch_add(1, Ordering::Relaxed);
        slot.state.store(SlotState::Free.as_u8(), Ordering::Release);

        Ok(true)
    }

    /// Returns the value identified by `id`, if it is still present.
    ///
    /// The arena id and slot generation must match, so stale slot ids do not
    /// resolve to newer values that reused the same slot.
    pub fn borrow(&self, id: SlotId) -> Result<SlotReadGuard<'_, T>, BorrowError> {
        let Some(slot) = self.slots.get(id.index) else {
            return Err(BorrowError::InvalidHandle(id));
        };

        // Fast first check before mutex.
        self.ensure_occupied_handle(slot, id)?;

        let data = match self.data[id.index].try_lock() {
            Ok(data) => data,
            Err(TryLockError::WouldBlock) => return Err(BorrowError::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(BorrowError::Poisoned),
        };

        // Check again under mutex.
        self.ensure_occupied_handle(slot, id)?;

        slot.state
            .store(SlotState::Reading.as_u8(), Ordering::Release);

        Ok(SlotReadGuard { slot, data })
    }

    /// Returns the fixed slot capacity of the arena.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Validates that `id` identifies the current occupied value in `slot`.
    fn ensure_occupied_handle(&self, slot: &SlotMeta, id: SlotId) -> Result<(), BorrowError> {
        let state = slot.state.load(Ordering::Acquire);
        let generation = slot.generation.load(Ordering::Acquire);

        if id.arena_id != self.id || id.generation != generation {
            return Err(BorrowError::StaleHandle(id));
        }

        match state {
            state if state == SlotState::Occupied.as_u8() => Ok(()),
            state if state == SlotState::Reading.as_u8() => Err(BorrowError::Busy),
            _ => Err(BorrowError::StaleHandle(id)),
        }
    }
}

/// Single arena slot with generation, lifecycle state, and stored data.
struct SlotMeta {
    /// Generation incremented every time the slot is removed and freed.
    generation: AtomicUsize,

    /// Current [`SlotState`] encoded as `u8` for atomic transitions.
    state: AtomicU8,
}

impl Default for SlotMeta {
    fn default() -> Self {
        Self {
            generation: AtomicUsize::new(0),
            state: AtomicU8::new(SlotState::Free.as_u8()),
        }
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum SlotState {
    /// Slot is available for a producer reservation.
    Free = 0,

    /// Slot has been reserved by a producer and is not visible to consumers.
    Reserved = 1,

    /// Slot contains a published value.
    Occupied = 2,

    /// Slot is currently borrowed by a consumer.
    Reading = 3,
}

impl SlotState {
    /// Returns the byte representation stored in [`SlotMeta::state`].
    fn as_u8(self) -> u8 {
        self as u8
    }

    /// Returns a human-readable name for debug assertions.
    fn name_from_u8(state: u8) -> &'static str {
        match state {
            state if state == Self::Free.as_u8() => "free",
            state if state == Self::Reserved.as_u8() => "reserved",
            state if state == Self::Occupied.as_u8() => "occupied",
            state if state == Self::Reading.as_u8() => "reading",
            _ => "unknown",
        }
    }
}

/// Arena handle assigned when a value is inserted.
///
/// A slot id identifies the owning arena plus the slot generation used for
/// stale-handle detection. The slot index can be reused after removal, so
/// lookups validate the whole handle before returning a value.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SlotId {
    /// Process-local id of the arena that owns the slot.
    arena_id: usize,

    /// Slot index inside the arena.
    index: usize,

    /// Slot generation used to reject stale handles after reuse.
    generation: usize,
}

/// Read guard for a value borrowed from the arena.
///
/// The guard holds the slot mutex while it is alive and restores the slot state
/// to occupied when dropped.
pub struct SlotReadGuard<'a, T> {
    /// Slot whose state is restored when the guard is dropped.
    slot: &'a SlotMeta,

    /// Locked value borrowed from the slot.
    data: MutexGuard<'a, T>,
}

impl<T> Deref for SlotReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> Drop for SlotReadGuard<'_, T> {
    fn drop(&mut self) {
        self.slot
            .state
            .store(SlotState::Occupied.as_u8(), Ordering::Release);
    }
}

/// Producer-owned reservation for an arena slot.
///
/// A reservation is created by [`Arena::try_reserve`] after the slot has moved
/// from free to reserved. Passing it to [`Arena::insert`] publishes the value
/// and returns a shareable [`SlotId`].
pub struct SlotReservationGuard<'a> {
    /// Slot whose state is restored when the guard is dropped.
    slot: &'a SlotMeta,

    /// Process-local id of the arena that owns the reserved slot.
    arena_id: usize,

    /// Slot index reserved for one pending insertion.
    index: usize,

    /// Slot generation observed when the reservation was acquired.
    generation: usize,
}

impl Drop for SlotReservationGuard<'_> {
    fn drop(&mut self) {
        if self.slot.generation.load(Ordering::Acquire) != self.generation {
            let state = self.slot.state.load(Ordering::Acquire);
            debug_assert_ne!(
                state,
                SlotState::Reserved.as_u8(),
                "reservation generation changed while slot is still reserved"
            );
            return;
        }

        match self.slot.state.compare_exchange(
            SlotState::Reserved.as_u8(),
            SlotState::Free.as_u8(),
            Ordering::Release,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // Reservation was abandoned before insert; slot is available again.
            }
            Err(state) if state == SlotState::Occupied.as_u8() => {
                // Reservation was consumed by insert.
            }
            Err(state) => {
                debug_assert!(
                    state == SlotState::Occupied.as_u8(),
                    "reservation dropped while slot is in unexpected state: {} ({state})",
                    SlotState::name_from_u8(state)
                );
            }
        }
    }
}

/// Error returned when a slot handle cannot be borrowed or removed.
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowError {
    /// The slot is currently locked by another reader or remover.
    #[error("Slot is already borrowed")]
    Busy,

    /// The supplied handle does not match the value currently stored in the slot.
    ///
    /// This covers stale generations and handles from another arena that happen
    /// to reference an in-range slot index.
    #[error("slot handle mismatch with current handle: {0:?}")]
    StaleHandle(SlotId),

    /// The supplied handle references a slot index outside this arena.
    #[error("slot handle is outside this arena: {0:?}")]
    InvalidHandle(SlotId),

    /// The slot mutex was poisoned by a panic while locked.
    #[error("slot mutex is poisoned")]
    Poisoned,
}

/// Error returned when a reserved slot cannot be filled.
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertError {
    /// The reservation does not belong to this arena or no longer matches the slot state.
    #[error("Reservation does not match a writable slot")]
    InvalidReservation,

    /// The slot mutex was poisoned by a panic while locked.
    #[error("slot mutex is poisoned")]
    Poisoned,
}

/// Error returned when a slot cannot be reserved.
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryReserveError {
    /// The bounded reservation scan did not find a free slot.
    #[error("reservation scan exceeded {attempts} attempts")]
    AttemptsExceeded {
        /// Number of slots probed before giving up.
        attempts: u8,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[derive(Debug, Default, Clone, PartialEq, Eq)]
    struct TestValue {
        sequence: usize,
        payload: Vec<u8>,
    }

    impl TestValue {
        fn new(sequence: usize, len: usize) -> Self {
            Self {
                sequence,
                payload: vec![(sequence % 251) as u8; len],
            }
        }
    }

    #[test]
    fn test_arena_creation() {
        let arena = Arena::<TestValue>::new(1024).expect("power-of-two arena size should be valid");
        assert_eq!(arena.size(), 1024);
        assert_eq!(arena.cursor.next.load(Ordering::Relaxed), 0);
        assert!(
            arena
                .slots
                .iter()
                .all(|slot| slot.state.load(Ordering::Relaxed) == SlotState::Free.as_u8())
        );
    }

    #[test]
    fn test_arena_rejects_non_power_of_two_size() {
        assert!(Arena::<TestValue>::new(0).is_err());
        assert!(Arena::<TestValue>::new(6).is_err());
    }

    #[test]
    fn test_reserve_scans_forward_and_reports_full() {
        let arena = Arena::<TestValue>::new(2).expect("arena should be created");

        let reservation1 = arena.try_reserve().expect("first slot should be reserved");
        let reservation2 = arena.try_reserve().expect("second slot should be reserved");

        assert_eq!(reservation1.index, 0);
        assert_eq!(reservation2.index, 1);
        assert!(matches!(
            arena.try_reserve(),
            Err(TryReserveError::AttemptsExceeded {
                attempts: RESERVER_ATTEMPTS
            })
        ));
    }

    #[test]
    fn test_reserve_scans_within_probe_limit() {
        let arena = Arena::<TestValue>::new(32).expect("arena should be created");
        let mut keys = Vec::with_capacity(32);

        for i in 0..32 {
            let reservation = arena.try_reserve().expect("slot should be reserved");
            keys.push(
                arena
                    .insert(reservation, TestValue::new(i, 4))
                    .expect("insert should succeed"),
            );
        }

        arena
            .remove(keys[8])
            .expect("value within probe limit should be removed");

        let reservation = arena
            .try_reserve()
            .expect("forward scan should find the free slot within the probe limit");
        assert_eq!(reservation.index, 8);
    }

    #[test]
    fn test_arena_insert_and_borrow() {
        let arena = Arena::<TestValue>::new(4).expect("arena should be created");
        let value = TestValue::new(7, 4);
        let reservation = arena.try_reserve().expect("slot should be reserved");

        let slot_id = arena
            .insert(reservation, value.clone())
            .expect("insert should succeed");

        assert_eq!(arena.size(), 4);
        let retrieved = arena.borrow(slot_id).expect("value should be present");
        assert_eq!(*retrieved, value);
    }

    #[test]
    fn test_insert_rejects_cross_arena_reservation() {
        let arena_a = Arena::<TestValue>::new(1).expect("first arena should be created");
        let arena_b = Arena::<TestValue>::new(1).expect("second arena should be created");
        let reservation = arena_a.try_reserve().expect("slot should be reserved");

        assert!(matches!(
            arena_b.insert(reservation, TestValue::new(0, 4)),
            Err(InsertError::InvalidReservation)
        ));
    }

    #[test]
    fn test_insert_rejects_stale_reservation_after_slot_reuse() {
        let arena = Arena::<TestValue>::new(1).expect("arena should be created");
        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, TestValue::new(1, 4))
            .expect("insert should succeed");
        arena.remove(key).expect("value should be removed");

        let current_reservation = arena.try_reserve().expect("slot should be reusable");
        assert_eq!(current_reservation.generation, 1);
        let current_key = arena
            .insert(current_reservation, TestValue::new(2, 4))
            .expect("insert should succeed");

        let stale_reservation = SlotReservationGuard {
            slot: &arena.slots[0],
            arena_id: arena.id,
            index: 0,
            generation: 0,
        };

        assert!(matches!(
            arena.insert(stale_reservation, TestValue::new(3, 4)),
            Err(InsertError::InvalidReservation)
        ));
        assert_eq!(arena.borrow(current_key).unwrap().sequence, 2);
    }

    #[test]
    fn test_borrow_and_remove_reject_out_of_range_handle() {
        let arena = Arena::<TestValue>::new(1).expect("arena should be created");
        let invalid = SlotId {
            arena_id: 0,
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
    fn test_borrow_and_remove_reject_free_and_reserved_slots() {
        let arena = Arena::<TestValue>::new(2).expect("arena should be created");
        let free_handle = SlotId {
            arena_id: arena.id,
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
        let writing_handle = SlotId {
            arena_id: arena.id,
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
    fn test_dropped_reservation_frees_slot() {
        let arena = Arena::<TestValue>::new(1).expect("arena should be created");

        {
            let reservation = arena.try_reserve().expect("slot should be reserved");
            assert_eq!(reservation.index, 0);
            assert!(matches!(
                arena.try_reserve(),
                Err(TryReserveError::AttemptsExceeded {
                    attempts: RESERVER_ATTEMPTS
                })
            ));
        }

        let reservation = arena
            .try_reserve()
            .expect("dropped reservation should free the slot");
        assert_eq!(reservation.index, 0);
    }

    #[test]
    fn test_arena_insert_multiple_and_borrow() {
        let arena = Arena::<TestValue>::new(4).expect("arena should be created");

        let reservation1 = arena.try_reserve().expect("first slot should be reserved");
        let key1 = arena
            .insert(reservation1, TestValue::new(1, 4))
            .expect("insert should succeed");
        let reservation2 = arena.try_reserve().expect("second slot should be reserved");
        let key2 = arena
            .insert(reservation2, TestValue::new(2, 8))
            .expect("insert should succeed");
        let reservation3 = arena.try_reserve().expect("third slot should be reserved");
        let key3 = arena
            .insert(reservation3, TestValue::new(3, 16))
            .expect("insert should succeed");

        assert_eq!(arena.size(), 4);
        assert_eq!(arena.borrow(key1).unwrap().payload.len(), 4);
        assert_eq!(arena.borrow(key2).unwrap().payload.len(), 8);
        assert_eq!(arena.borrow(key3).unwrap().payload.len(), 16);
    }

    #[test]
    fn test_borrow_and_remove_reject_cross_arena_handle() {
        let arena_a = Arena::<TestValue>::new(1).expect("first arena should be created");
        let arena_b = Arena::<TestValue>::new(1).expect("second arena should be created");

        let reservation_a = arena_a
            .try_reserve()
            .expect("first slot should be reserved");
        let key_a = arena_a
            .insert(reservation_a, TestValue::new(1, 4))
            .expect("insert should succeed");
        let reservation_b = arena_b
            .try_reserve()
            .expect("second slot should be reserved");
        let key_b = arena_b
            .insert(reservation_b, TestValue::new(2, 4))
            .expect("insert should succeed");

        let cross_arena = SlotId {
            arena_id: key_b.arena_id,
            index: key_a.index,
            generation: key_a.generation,
        };

        assert!(matches!(
            arena_a.borrow(cross_arena),
            Err(BorrowError::StaleHandle(handle)) if handle == cross_arena
        ));
        assert!(matches!(
            arena_a.remove(cross_arena),
            Err(BorrowError::StaleHandle(handle)) if handle == cross_arena
        ));
    }

    #[test]
    fn test_arena_remove() {
        let arena = Arena::<TestValue>::new(4).expect("arena should be created");
        let value = TestValue::new(1, 4);
        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, value.clone())
            .expect("insert should succeed");

        assert_eq!(arena.size(), 4);
        assert_eq!(*arena.borrow(key).unwrap(), value);
        assert!(arena.remove(key).unwrap());
        assert_eq!(arena.size(), 4);
        assert!(matches!(
            arena.borrow(key),
            Err(BorrowError::StaleHandle(_))
        ));
    }

    #[test]
    fn test_borrow_guard_blocks_other_borrowers_until_drop() {
        let arena = Arena::<TestValue>::new(4).expect("arena should be created");
        let value = TestValue::new(1, 4);
        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, value.clone())
            .expect("insert should succeed");

        let guard = arena.borrow(key).expect("value should be borrowed");

        assert!(matches!(arena.borrow(key), Err(BorrowError::Busy)));
        assert!(matches!(arena.remove(key), Err(BorrowError::Busy)));

        drop(guard);

        assert_eq!(*arena.borrow(key).unwrap(), value);
    }

    #[test]
    fn test_remove_frees_slot_for_new_generation() {
        let arena = Arena::<TestValue>::new(2).expect("arena should be created");

        let first_reservation = arena.try_reserve().expect("first slot should be reserved");
        assert_eq!(first_reservation.index, 0);
        let first_key = arena
            .insert(first_reservation, TestValue::new(1, 4))
            .expect("insert should succeed");

        let second_reservation = arena.try_reserve().expect("second slot should be reserved");
        assert_eq!(second_reservation.index, 1);
        let second_key = arena
            .insert(second_reservation, TestValue::new(2, 8))
            .expect("insert should succeed");

        arena
            .remove(first_key)
            .expect("first value should be removed");

        let reused_reservation = arena.try_reserve().expect("freed slot should be reserved");
        assert_eq!(reused_reservation.index, first_key.index);
        assert_eq!(reused_reservation.generation, first_key.generation + 1);

        let reused_key = arena
            .insert(reused_reservation, TestValue::new(3, 16))
            .expect("insert should succeed");
        assert_eq!(reused_key.index, first_key.index);
        assert_eq!(reused_key.generation, first_key.generation + 1);
        assert_ne!(reused_key, first_key);

        assert!(matches!(
            arena.borrow(first_key),
            Err(BorrowError::StaleHandle(_))
        ));
        assert_eq!(arena.borrow(reused_key).unwrap().sequence, 3);
        assert_eq!(arena.borrow(second_key).unwrap().sequence, 2);
    }

    #[test]
    fn test_remove_rejects_stale_handle_after_reuse() {
        let arena = Arena::<TestValue>::new(1).expect("arena should be created");

        let first_reservation = arena.try_reserve().expect("slot should be reserved");
        let first_key = arena
            .insert(first_reservation, TestValue::new(1, 4))
            .expect("insert should succeed");
        arena.remove(first_key).expect("value should be removed");

        let reused_reservation = arena.try_reserve().expect("slot should be reusable");
        let reused_key = arena
            .insert(reused_reservation, TestValue::new(2, 4))
            .expect("insert should succeed");

        assert!(matches!(
            arena.remove(first_key),
            Err(BorrowError::StaleHandle(stale)) if stale == first_key
        ));
        assert_eq!(arena.borrow(reused_key).unwrap().sequence, 2);
    }

    #[test]
    fn test_borrow_reports_poisoned_mutex() {
        let arena = Arc::new(Arena::<TestValue>::new(1).expect("arena should be created"));
        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, TestValue::new(1, 4))
            .expect("insert should succeed");

        let poisoner = {
            let arena = Arc::clone(&arena);
            thread::spawn(move || {
                let _guard = arena.data[key.index].lock().unwrap();
                panic!("poison slot mutex");
            })
        };
        assert!(poisoner.join().is_err());

        assert!(matches!(arena.borrow(key), Err(BorrowError::Poisoned)));
        assert!(matches!(arena.remove(key), Err(BorrowError::Poisoned)));
    }

    #[test]
    fn test_insert_reports_poisoned_mutex() {
        let arena = Arc::new(Arena::<TestValue>::new(1).expect("arena should be created"));

        let poisoner = {
            let arena = Arc::clone(&arena);
            thread::spawn(move || {
                let _guard = arena.data[0].lock().unwrap();
                panic!("poison slot mutex");
            })
        };
        assert!(poisoner.join().is_err());

        let reservation = arena.try_reserve().expect("slot should be reserved");
        assert!(matches!(
            arena.insert(reservation, TestValue::new(1, 4)),
            Err(InsertError::Poisoned)
        ));
    }

    #[test]
    fn test_concurrent_reservations_are_unique() {
        const THREADS: usize = 8;

        let arena = Arc::new(Arena::<TestValue>::new(THREADS).expect("arena should be created"));
        let barrier = Arc::new(Barrier::new(THREADS));
        let release = Arc::new(Barrier::new(THREADS));
        let mut handles = Vec::with_capacity(THREADS);

        for _ in 0..THREADS {
            let arena = Arc::clone(&arena);
            let barrier = Arc::clone(&barrier);
            let release = Arc::clone(&release);
            handles.push(thread::spawn(move || {
                barrier.wait();
                let reservation = arena
                    .try_reserve()
                    .expect("each thread should reserve one slot");
                let result = (reservation.index, reservation.generation);
                release.wait();
                result
            }));
        }

        let mut reservations = handles
            .into_iter()
            .map(|handle| handle.join().expect("reservation thread should not panic"))
            .collect::<Vec<_>>();

        reservations.sort_by_key(|reservation| reservation.0);
        assert_eq!(
            reservations
                .iter()
                .map(|reservation| reservation.0)
                .collect::<Vec<_>>(),
            (0..THREADS).collect::<Vec<_>>()
        );
        assert!(reservations.iter().all(|reservation| reservation.1 == 0));
        assert!(arena.try_reserve().is_ok());
    }

    #[test]
    fn test_concurrent_producers_can_reserve_and_insert() {
        const THREADS: usize = 8;

        let arena = Arc::new(Arena::<TestValue>::new(THREADS).expect("arena should be created"));
        let barrier = Arc::new(Barrier::new(THREADS));
        let mut handles = Vec::with_capacity(THREADS);

        for thread_id in 0..THREADS {
            let arena = Arc::clone(&arena);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                let reservation = arena.try_reserve().expect("slot should be reserved");
                arena
                    .insert(reservation, TestValue::new(thread_id, 4))
                    .expect("insert should succeed")
            }));
        }

        let keys = handles
            .into_iter()
            .map(|handle| handle.join().expect("producer thread should not panic"))
            .collect::<Vec<_>>();

        assert_eq!(keys.len(), THREADS);
        for key in keys {
            let value = arena
                .borrow(key)
                .expect("inserted value should be readable");
            assert_eq!(
                value.payload.as_slice(),
                vec![(value.sequence % 251) as u8; 4]
            );
        }
        assert!(matches!(
            arena.try_reserve(),
            Err(TryReserveError::AttemptsExceeded {
                attempts: RESERVER_ATTEMPTS
            })
        ));
    }

    #[test]
    fn test_producer_consumer_wraparound_chase() {
        const CAPACITY: usize = 4;
        const PACKETS: usize = 64;

        let arena = Arc::new(Arena::<TestValue>::new(CAPACITY).expect("arena should be created"));
        let (tx, rx) = mpsc::sync_channel::<SlotId>(CAPACITY);

        let producer = {
            let arena = Arc::clone(&arena);
            thread::spawn(move || {
                for i in 0..PACKETS {
                    let reservation = loop {
                        if let Ok(reservation) = arena.try_reserve() {
                            break reservation;
                        }
                        thread::yield_now();
                    };
                    let slot_id = arena
                        .insert(reservation, TestValue::new(i, 8))
                        .expect("insert should succeed");
                    tx.send(slot_id).expect("consumer should receive slot id");
                }
            })
        };

        let consumer = {
            let arena = Arc::clone(&arena);
            thread::spawn(move || {
                let mut seen_generations = Vec::with_capacity(PACKETS);

                for i in 0..PACKETS {
                    let slot_id = rx.recv().expect("producer should send slot id");
                    let value = arena.borrow(slot_id).expect("value should be readable");
                    assert_eq!(value.sequence, i);
                    assert_eq!(value.payload.as_slice(), vec![(i % 251) as u8; 8]);
                    seen_generations.push(slot_id.generation);
                    drop(value);
                    arena.remove(slot_id).expect("value should be removable");
                }

                seen_generations
            })
        };

        producer.join().expect("producer should not panic");
        let seen_generations = consumer.join().expect("consumer should not panic");

        assert!(
            seen_generations.iter().copied().max().unwrap_or_default() > 0,
            "test should force slot reuse after wrapping over capacity"
        );
        assert!(arena.try_reserve().is_ok());
    }

    #[test]
    fn test_cross_thread_borrow_blocks_remove_until_guard_drops() {
        let arena = Arena::<TestValue>::new(1).expect("arena should be created");
        let reservation = arena.try_reserve().expect("slot should be reserved");
        let key = arena
            .insert(reservation, TestValue::new(1, 4))
            .expect("insert should succeed");
        let arena = Arc::new(arena);
        let borrowed = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));

        let reader = {
            let arena = Arc::clone(&arena);
            let borrowed = Arc::clone(&borrowed);
            let release = Arc::clone(&release);
            thread::spawn(move || {
                let guard = arena.borrow(key).expect("reader should borrow value");
                borrowed.wait();
                release.wait();
                assert_eq!(guard.sequence, 1);
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
        let arena = Arena::<TestValue>::new(1).expect("arena should be created");
        let first_reservation = arena.try_reserve().expect("slot should be reserved");
        let first_key = arena
            .insert(first_reservation, TestValue::new(1, 4))
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
