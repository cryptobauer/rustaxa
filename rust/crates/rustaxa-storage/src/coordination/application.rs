//! Storage primitives for the production owner's protected operation.
//! Native/service enrollment and vote authority remain separate requirements.

use super::*;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Weak};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ApplicationPhase {
    Read,
    Effects,
}

/// A unique construction key for this coordinator. It grants no vote authority.
#[derive(Debug)]
pub(crate) struct ApplicationOwnerKey {
    coordinator: Arc<()>,
    pub(crate) issuer: Arc<()>,
}

type Guard<'a> = ReentrantMutexGuard<'a, RefCell<State>>;

/// An exclusive read scope. All production writers must first be enrolled.
/// This scope cannot move to another thread or publish an execution result.
#[derive(Debug)]
pub struct ApplicationReadScope<'a> {
    coordinator: &'a StorageCoordinator,
    issuer: Arc<()>,
    guard: Option<Guard<'a>>,
    generation: u64,
}

/// A private owner's bounded write phase. Generic writers remain blocked.
#[derive(Debug)]
pub struct OwnedEffectScope<'a> {
    coordinator: &'a StorageCoordinator,
    issuer: Arc<()>,
    guard: Option<Guard<'a>>,
    generation: u64,
    write_attempted: bool,
}

/// Storage evidence only. The production owner must also bind exact effects,
/// task, phase and canonical bytes before transport or other external effects.
#[derive(Debug)]
pub(crate) struct ApplicationEffectReceipt {
    coordinator: Arc<()>,
    issuer: Arc<()>,
    generation: u64,
    _same_thread: PhantomData<Rc<()>>,
}

impl StorageCoordinator {
    /// Claims one owner for this handle. Dropping its key does not permit rebind.
    pub(crate) fn claim_application_owner(
        &self,
    ) -> Result<ApplicationOwnerKey, StorageCoordinationError> {
        self.ensure_not_waiting()?;
        let guard = self.state.lock();
        let mut state = guard.borrow_mut();
        if state.application_owner.is_some() {
            return Err(StorageCoordinationError::ApplicationOwnerAlreadyBound);
        }
        if state.reserved_read || state.mutation_depth != 0 {
            return Err(StorageCoordinationError::ReadUnavailable);
        }
        let issuer = Arc::new(());
        state.application_owner = Some(Arc::downgrade(&issuer));
        Ok(ApplicationOwnerKey {
            coordinator: Arc::clone(&self.identity),
            issuer,
        })
    }

    /// Holds the shared gate before any native/service lock. The key is not an
    /// execution approval, and the scope must not cross a process/network wait.
    pub(crate) fn try_reserve_application_read_for(
        &self,
        key: &ApplicationOwnerKey,
        wait: Duration,
    ) -> Result<ApplicationReadScope<'_>, StorageCoordinationError> {
        if !Arc::ptr_eq(&self.identity, &key.coordinator) {
            return Err(StorageCoordinationError::ForeignApplicationOwner);
        }
        let guard = self
            .state
            .try_lock_for(wait)
            .ok_or(StorageCoordinationError::ReadUnavailable)?;
        let generation;
        {
            let mut state = guard.borrow_mut();
            if state.application_poisoned {
                return Err(StorageCoordinationError::ApplicationPoisoned);
            }
            if !issuer_matches(&state, &key.issuer) {
                return Err(StorageCoordinationError::ForeignApplicationOwner);
            }
            if state.reserved_read || state.mutation_depth != 0 || state.application_phase.is_some()
            {
                return Err(StorageCoordinationError::ReadUnavailable);
            }
            generation = state.generation;
            state.reserved_read = true;
            state.application_phase = Some(ApplicationPhase::Read);
        }
        Ok(ApplicationReadScope {
            coordinator: self,
            issuer: Arc::clone(&key.issuer),
            guard: Some(guard),
            generation,
        })
    }
}

fn issuer_matches(state: &State, issuer: &Arc<()>) -> bool {
    state
        .application_owner
        .as_ref()
        .is_some_and(|registered| Weak::ptr_eq(registered, &Arc::downgrade(issuer)))
}

fn validate(
    state: &State,
    issuer: &Arc<()>,
    generation: u64,
    phase: ApplicationPhase,
) -> Result<(), StorageCoordinationError> {
    if state.application_poisoned {
        return Err(StorageCoordinationError::ApplicationPoisoned);
    }
    if !issuer_matches(state, issuer) {
        return Err(StorageCoordinationError::ForeignApplicationOwner);
    }
    if !state.reserved_read
        || state.generation != generation
        || state.application_phase != Some(phase)
        || state.mutation_depth != usize::from(phase == ApplicationPhase::Effects)
    {
        return Err(StorageCoordinationError::ReadUnavailable);
    }
    Ok(())
}

impl<'a> ApplicationReadScope<'a> {
    pub(crate) fn matches_owner(&self, key: &ApplicationOwnerKey) -> bool {
        Arc::ptr_eq(&self.coordinator.identity, &key.coordinator)
            && Arc::ptr_eq(&self.issuer, &key.issuer)
            && self.is_current()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_current(&self) -> bool {
        self.guard.as_ref().is_some_and(|guard| {
            validate(
                &guard.borrow(),
                &self.issuer,
                self.generation,
                ApplicationPhase::Read,
            )
            .is_ok()
        })
    }

    /// Consumes the read phase without releasing the gate. The owner must first
    /// consume its private accepted action. No host callback may run in effects.
    pub(crate) fn begin_owned_effects(
        mut self,
    ) -> Result<OwnedEffectScope<'a>, StorageCoordinationError> {
        self.coordinator.ensure_not_waiting()?;
        {
            let guard = self.guard.as_ref().expect("read scope owns the gate");
            let mut state = guard.borrow_mut();
            validate(
                &state,
                &self.issuer,
                self.generation,
                ApplicationPhase::Read,
            )?;
            let generation = state
                .generation
                .checked_add(1)
                .ok_or(StorageCoordinationError::GenerationExhausted)?;
            state.generation = generation;
            state.mutation_depth = 1;
            state.application_phase = Some(ApplicationPhase::Effects);
            self.generation = generation;
        }
        Ok(OwnedEffectScope {
            coordinator: self.coordinator,
            issuer: Arc::clone(&self.issuer),
            guard: self.guard.take(),
            generation: self.generation,
            write_attempted: false,
        })
    }

    pub(crate) fn accepts(&self, receipt: &ApplicationEffectReceipt) -> bool {
        self.is_current()
            && self.generation == receipt.generation
            && Arc::ptr_eq(&self.coordinator.identity, &receipt.coordinator)
            && Arc::ptr_eq(&self.issuer, &receipt.issuer)
    }
}

impl<'a> OwnedEffectScope<'a> {
    pub(crate) fn validate_for(
        &self,
        coordinator: &StorageCoordinator,
    ) -> Result<(), StorageCoordinationError> {
        self.coordinator.ensure_not_waiting()?;
        if !Arc::ptr_eq(&self.coordinator.identity, &coordinator.identity) {
            return Err(StorageCoordinationError::ForeignApplicationOwner);
        }
        let guard = self.guard.as_ref().expect("effect scope owns the gate");
        validate(
            &guard.borrow(),
            &self.issuer,
            self.generation,
            ApplicationPhase::Effects,
        )
    }

    pub(crate) fn poison(&mut self) {
        self.guard
            .as_ref()
            .expect("effect scope owns the gate")
            .borrow_mut()
            .application_poisoned = true;
    }

    pub(crate) fn note_write_attempt(&mut self) {
        self.write_attempted = true;
    }

    /// Commits a primary batch only for this owner. The production operation must
    /// bind the batch to its exact accepted effect plan before it calls this leaf.
    pub(crate) fn commit_batch(
        &mut self,
        storage: &crate::Storage,
        batch: crate::StorageWriteBatch,
        sync: bool,
    ) -> anyhow::Result<()> {
        storage.commit_owned_effect_batch(self, batch, sync)
    }

    /// Returns to protected read mode without unlocking. Only a successful exact
    /// private effect operation may use the receipt for its later host handoff.
    pub(crate) fn finish(
        mut self,
    ) -> Result<(ApplicationReadScope<'a>, ApplicationEffectReceipt), StorageCoordinationError>
    {
        {
            let guard = self.guard.as_ref().expect("effect scope owns the gate");
            let mut state = guard.borrow_mut();
            validate(
                &state,
                &self.issuer,
                self.generation,
                ApplicationPhase::Effects,
            )?;
            state.mutation_depth = 0;
            state.application_phase = Some(ApplicationPhase::Read);
        }
        let receipt = ApplicationEffectReceipt {
            coordinator: Arc::clone(&self.coordinator.identity),
            issuer: Arc::clone(&self.issuer),
            generation: self.generation,
            _same_thread: PhantomData,
        };
        let read = ApplicationReadScope {
            coordinator: self.coordinator,
            issuer: Arc::clone(&self.issuer),
            guard: self.guard.take(),
            generation: self.generation,
        };
        Ok((read, receipt))
    }
}

impl Drop for ApplicationReadScope<'_> {
    fn drop(&mut self) {
        if let Some(guard) = self.guard.take() {
            let mut state = guard.borrow_mut();
            assert_eq!(state.application_phase, Some(ApplicationPhase::Read));
            assert_eq!(state.mutation_depth, 0);
            state.application_phase = None;
            state.reserved_read = false;
        }
    }
}

impl Drop for OwnedEffectScope<'_> {
    fn drop(&mut self) {
        if let Some(guard) = self.guard.take() {
            let mut state = guard.borrow_mut();
            assert_eq!(state.application_phase, Some(ApplicationPhase::Effects));
            assert_eq!(state.mutation_depth, 1);
            if self.write_attempted {
                state.application_poisoned = true;
            }
            state.application_phase = None;
            state.mutation_depth = 0;
            state.reserved_read = false;
        }
    }
}

struct WaitCallbackEntry {
    coordinator: Arc<()>,
    issuer: Arc<()>,
}

std::thread_local! {
    static WAIT_CALLBACKS: RefCell<Vec<WaitCallbackEntry>> = const { RefCell::new(Vec::new()) };
}

/// A calling-thread process-wait fence. It holds no coordinator lock.
/// Other threads can change the source generation while this fence lives.
///
/// ```compile_fail
/// use rustaxa_storage::application_control::ApplicationWaitCallbackScope;
/// fn require_send<T: Send>(_: T) {}
/// fn transfer(scope: ApplicationWaitCallbackScope<'_>) {
///     require_send(scope);
/// }
/// ```
#[derive(Debug)]
pub struct ApplicationWaitCallbackScope<'owner> {
    coordinator: &'owner StorageCoordinator,
    issuer: Arc<()>,
    generation: u64,
    _same_thread: PhantomData<Rc<()>>,
}

impl ApplicationWaitCallbackScope<'_> {
    /// Returns the generation observed before the no-lock wait.
    /// This value does not prove that the source remained unchanged.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl StorageCoordinator {
    pub(crate) fn ensure_not_waiting(&self) -> Result<(), StorageCoordinationError> {
        WAIT_CALLBACKS.with(|entries| {
            if entries
                .borrow()
                .iter()
                .any(|entry| Arc::ptr_eq(&entry.coordinator, &self.identity))
            {
                Err(StorageCoordinationError::WaitCallbackActive)
            } else {
                Ok(())
            }
        })
    }

    pub(crate) fn try_wait_callback_for(
        &self,
        key: &ApplicationOwnerKey,
    ) -> Result<ApplicationWaitCallbackScope<'_>, StorageCoordinationError> {
        if !Arc::ptr_eq(&self.identity, &key.coordinator) {
            return Err(StorageCoordinationError::ForeignApplicationOwner);
        }
        self.ensure_not_waiting()?;
        let guard = self.state.lock();
        let generation;
        {
            let state = guard.borrow();
            if state.application_poisoned {
                return Err(StorageCoordinationError::ApplicationPoisoned);
            }
            if !issuer_matches(&state, &key.issuer) {
                return Err(StorageCoordinationError::ForeignApplicationOwner);
            }
            if state.reserved_read || state.mutation_depth != 0 || state.application_phase.is_some()
            {
                return Err(StorageCoordinationError::ReadUnavailable);
            }
            generation = state.generation;
        }
        drop(guard);
        WAIT_CALLBACKS.with(|entries| {
            entries.borrow_mut().push(WaitCallbackEntry {
                coordinator: Arc::clone(&self.identity),
                issuer: Arc::clone(&key.issuer),
            });
        });
        Ok(ApplicationWaitCallbackScope {
            coordinator: self,
            issuer: Arc::clone(&key.issuer),
            generation,
            _same_thread: PhantomData,
        })
    }
}

impl Drop for ApplicationWaitCallbackScope<'_> {
    fn drop(&mut self) {
        WAIT_CALLBACKS.with(|entries| {
            let mut entries = entries.borrow_mut();
            let position = entries
                .iter()
                .position(|entry| {
                    Arc::ptr_eq(&entry.coordinator, &self.coordinator.identity)
                        && Arc::ptr_eq(&entry.issuer, &self.issuer)
                })
                .expect("wait callback owns its thread fence");
            entries.remove(position);
        });
    }
}

#[cfg(test)]
mod tests;
