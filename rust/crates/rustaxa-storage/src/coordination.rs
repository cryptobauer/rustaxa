//! Shared exclusion for storage writes. This does not grant execution authority.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

mod application;
pub(crate) use application::{ApplicationEffectReceipt, ApplicationOwnerKey};
pub use application::{ApplicationReadScope, ApplicationWaitCallbackScope, OwnedEffectScope};

use parking_lot::{ReentrantMutex, ReentrantMutexGuard};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StorageCoordinationError {
    #[error("storage mutation attempted during a reserved read")]
    MutationDuringRead,
    #[error("application mutation or reentry attempted during process wait")]
    WaitCallbackActive,
    #[error("storage mutation generation exhausted")]
    GenerationExhausted,
    #[error("storage mutation depth exhausted")]
    DepthExhausted,
    #[error("storage read reservation unavailable")]
    ReadUnavailable,
    #[error("application owner is already bound")]
    ApplicationOwnerAlreadyBound,
    #[error("application owner is foreign")]
    ForeignApplicationOwner,
    #[error("application state needs recovery")]
    ApplicationPoisoned,
}

#[derive(Debug, Default)]
struct State {
    generation: u64,
    mutation_depth: usize,
    reserved_read: bool,
    application_owner: Option<Weak<()>>,
    application_phase: Option<application::ApplicationPhase>,
    application_poisoned: bool,
}

/// Coordinates mutations that share one production storage backend.
///
/// This is one storage boundary. A production execution owner must also enroll
/// complete native and concrete operations before it can reserve that state.
#[derive(Debug, Default)]
pub struct StorageCoordinator {
    state: ReentrantMutex<RefCell<State>>,
    mutation_attempts: AtomicU64,
    identity: Arc<()>,
}

impl StorageCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enters a mutation before its first effect. Nested scopes retain the same
    /// generation. A failed write still invalidates the previous generation.
    pub fn begin_mutation(&self) -> Result<StorageMutationGuard<'_>, StorageCoordinationError> {
        self.ensure_not_waiting()?;
        self.mutation_attempts
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            })
            .expect("storage mutation attempt update cannot fail");
        let guard = self.state.lock();
        {
            let mut state = guard.borrow_mut();
            if state.application_poisoned {
                return Err(StorageCoordinationError::ApplicationPoisoned);
            }
            if state.reserved_read {
                return Err(StorageCoordinationError::MutationDuringRead);
            }
            let depth = state
                .mutation_depth
                .checked_add(1)
                .ok_or(StorageCoordinationError::DepthExhausted)?;
            if state.mutation_depth == 0 {
                state.generation = state
                    .generation
                    .checked_add(1)
                    .ok_or(StorageCoordinationError::GenerationExhausted)?;
            }
            state.mutation_depth = depth;
        }
        Ok(StorageMutationGuard { guard })
    }

    /// Acquires the coordinator before an existing storage serialization lock.
    /// It permits reads under a reservation and cannot permit a reserved write.
    pub(crate) fn observe(&self) -> StorageObservationGuard<'_> {
        StorageObservationGuard {
            _guard: self.state.lock(),
        }
    }

    /// Storage-only reservation. Native services and host callbacks must not
    /// run under this guard; their whole operations are not yet enrolled here.
    pub(crate) fn try_reserve_storage_read_for(
        &self,
        wait: Duration,
    ) -> Result<StorageReadReservation<'_>, StorageCoordinationError> {
        let guard = self
            .state
            .try_lock_for(wait)
            .ok_or(StorageCoordinationError::ReadUnavailable)?;
        let generation;
        {
            let mut state = guard.borrow_mut();
            if state.reserved_read || state.mutation_depth != 0 {
                return Err(StorageCoordinationError::ReadUnavailable);
            }
            state.reserved_read = true;
            generation = state.generation;
        }
        Ok(StorageReadReservation {
            owner: self,
            guard,
            generation,
        })
    }

    pub fn generation(&self) -> u64 {
        self.state.lock().borrow().generation
    }

    pub fn mutation_attempts(&self) -> u64 {
        self.mutation_attempts.load(Ordering::Relaxed)
    }
}

/// Retains one mutation scope. It cannot move to another thread.
///
/// ```compile_fail
/// use rustaxa_storage::StorageCoordinator;
/// let owner = StorageCoordinator::new();
/// let guard = owner.begin_mutation().unwrap();
/// std::thread::scope(|scope| { scope.spawn(move || drop(guard)); });
/// ```
#[derive(Debug)]
pub struct StorageMutationGuard<'a> {
    guard: ReentrantMutexGuard<'a, RefCell<State>>,
}

impl StorageMutationGuard<'_> {
    pub(crate) fn generation(&self) -> u64 {
        self.guard.borrow().generation
    }
}

impl Drop for StorageMutationGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.guard.borrow_mut();
        state.mutation_depth = state
            .mutation_depth
            .checked_sub(1)
            .expect("storage mutation guard owns a depth entry");
    }
}

#[derive(Debug)]
pub(crate) struct StorageObservationGuard<'a> {
    _guard: ReentrantMutexGuard<'a, RefCell<State>>,
}

#[derive(Debug)]
pub(crate) struct StorageReadReservation<'a> {
    owner: &'a StorageCoordinator,
    guard: ReentrantMutexGuard<'a, RefCell<State>>,
    generation: u64,
}

impl StorageReadReservation<'_> {
    pub(crate) fn is_from(&self, owner: &StorageCoordinator) -> bool {
        std::ptr::eq(self.owner, owner)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn is_current(&self) -> bool {
        let state = self.guard.borrow();
        state.reserved_read && state.mutation_depth == 0 && state.generation == self.generation
    }
}

impl Drop for StorageReadReservation<'_> {
    fn drop(&mut self) {
        let mut state = self.guard.borrow_mut();
        assert!(
            state.reserved_read,
            "storage read guard owns its reservation"
        );
        state.reserved_read = false;
    }
}

#[cfg(test)]
mod tests;
