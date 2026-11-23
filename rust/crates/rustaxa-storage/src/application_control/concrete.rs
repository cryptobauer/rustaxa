use super::{ApplicationMutationScope, ApplicationOwner, ApplicationReadScope};
use crate::{
    ConcreteCheckpointReaders, ConcreteLifecycleObservation, ConcreteStateLifecycle,
    ConcreteTransitionBuilder,
};
use anyhow::{Result, ensure};
use rustaxa_types::FinalChainBlockNumber;
use rustaxa_types::concrete_state::{ConcreteAccountRecord, ConcreteRead, ConcreteStateIdentity};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Actual committed state retained by one application owner.
/// The lifecycle cannot be replaced or obtained through this read capability.
///
/// ```compile_fail
/// use rustaxa_storage::application_control::ApplicationConcreteState;
/// fn fabricate() -> ApplicationConcreteState {
///     ApplicationConcreteState { owner: todo!(), lifecycle: todo!() }
/// }
/// ```
pub struct ApplicationConcreteState {
    pub(super) owner: Arc<ApplicationOwner>,
    pub(super) lifecycle: Mutex<Option<ConcreteStateLifecycle>>,
}

impl ApplicationConcreteState {
    #[cfg(feature = "production_integration_test")]
    pub fn integration_database_image(
        &self,
        scope: &ApplicationReadScope<'_>,
    ) -> Result<crate::integration_inspection::DatabaseImage> {
        ensure!(
            scope.matches_owner(&self.owner.key) && scope.is_current(),
            "APPLICATION_CONCRETE_FOREIGN_OR_STALE_READ_SCOPE"
        );
        self.lifecycle
            .lock()
            .map_err(|_| anyhow::anyhow!("APPLICATION_CONCRETE_LOCK_POISONED"))?
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("APPLICATION_CONCRETE_REOPEN_REQUIRED"))?
            .integration_database_image()
    }
    /// Consumes the actual lifecycle and binds it permanently to this owner.
    pub fn new(owner: Arc<ApplicationOwner>, lifecycle: ConcreteStateLifecycle) -> Result<Self> {
        {
            let _scope = owner.try_read(Duration::from_secs(1))?;
            ensure!(
                lifecycle.observation()?.pending_marker_rlp.is_empty(),
                "APPLICATION_CONCRETE_PENDING_STATE"
            );
        }
        Ok(Self {
            owner,
            lifecycle: Mutex::new(Some(lifecycle)),
        })
    }

    pub fn is_for(&self, owner: &ApplicationOwner) -> bool {
        std::ptr::eq(self.owner.as_ref(), owner)
    }

    pub fn observation(
        &self,
        scope: &ApplicationReadScope<'_>,
    ) -> Result<ConcreteLifecycleObservation> {
        ensure!(
            scope.matches_owner(&self.owner.key),
            "APPLICATION_CONCRETE_FOREIGN_READ_SCOPE"
        );
        Ok(self
            .lifecycle
            .lock()
            .map_err(|_| anyhow::anyhow!("APPLICATION_CONCRETE_LOCK_POISONED"))?
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("APPLICATION_CONCRETE_REOPEN_REQUIRED"))?
            .observation()?)
    }

    /// Borrows isolated preparation and real retained readers under this owner.
    /// The callback receives no lifecycle, writer or publication authority.
    pub fn with_preparation<R>(
        &self,
        scope: &ApplicationReadScope<'_>,
        expected: &ConcreteLifecycleObservation,
        next_period: FinalChainBlockNumber,
        delayed: ConcreteStateIdentity,
        operation: impl for<'state> FnOnce(
            ConcreteTransitionBuilder<'state>,
            Arc<ConcreteCheckpointReaders>,
        ) -> Result<R>,
    ) -> Result<R> {
        ensure!(
            scope.matches_owner(&self.owner.key) && scope.is_current(),
            "APPLICATION_CONCRETE_FOREIGN_OR_STALE_READ_SCOPE"
        );
        let guard = self
            .lifecycle
            .lock()
            .map_err(|_| anyhow::anyhow!("APPLICATION_CONCRETE_LOCK_POISONED"))?;
        let lifecycle = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("APPLICATION_CONCRETE_REOPEN_REQUIRED"))?;
        let before = lifecycle.observation()?;
        ensure!(
            &before == expected && before.pending_marker_rlp.is_empty(),
            "APPLICATION_CONCRETE_PREPARATION_SOURCE_CHANGED"
        );
        let readers = Arc::new(lifecycle.open_retained_checkpoint(delayed)?);
        let result = operation(lifecycle.begin_transition(next_period)?, readers);
        ensure!(
            scope.is_current() && lifecycle.observation()? == before,
            "APPLICATION_CONCRETE_CHANGED_DURING_PREPARATION"
        );
        result
    }

    /// Borrows a concrete read snapshot for an identity from finalized history.
    /// The application binds the requested historical root to a stored header.
    pub fn with_checkpoint<R>(
        &self,
        scope: &ApplicationReadScope<'_>,
        committed: ConcreteStateIdentity,
        requested: ConcreteStateIdentity,
        read: impl FnOnce(&ConcreteCheckpointReaders) -> Result<R>,
    ) -> Result<R> {
        ensure!(
            scope.matches_owner(&self.owner.key) && scope.is_current(),
            "APPLICATION_CONCRETE_FOREIGN_OR_STALE_READ_SCOPE"
        );
        self.with_retained_checkpoint(committed, requested, read)
    }

    /// Reads one exact checkpoint while the application owns a mutation.
    pub fn with_mutation_checkpoint<R>(
        &self,
        scope: &ApplicationMutationScope<'_>,
        committed: ConcreteStateIdentity,
        requested: ConcreteStateIdentity,
        read: impl FnOnce(&ConcreteCheckpointReaders) -> Result<R>,
    ) -> Result<R> {
        ensure!(
            std::ptr::eq(scope.owner_key, &self.owner.key),
            "APPLICATION_CONCRETE_FOREIGN_MUTATION_SCOPE"
        );
        self.with_retained_checkpoint(committed, requested, read)
    }

    fn with_retained_checkpoint<R>(
        &self,
        committed: ConcreteStateIdentity,
        requested: ConcreteStateIdentity,
        read: impl FnOnce(&ConcreteCheckpointReaders) -> Result<R>,
    ) -> Result<R> {
        let lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| anyhow::anyhow!("APPLICATION_CONCRETE_LOCK_POISONED"))?;
        let lifecycle = lifecycle
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("APPLICATION_CONCRETE_REOPEN_REQUIRED"))?;
        let observation = lifecycle.observation()?;
        ensure!(
            observation.committed == committed && observation.pending_marker_rlp.is_empty(),
            "APPLICATION_CONCRETE_COMMITTED_IDENTITY_CHANGED"
        );
        read(&lifecycle.open_retained_checkpoint(requested)?)
    }

    pub fn accounts(
        &self,
        scope: &ApplicationMutationScope<'_>,
        expected: ConcreteStateIdentity,
        addresses: &[[u8; 20]],
    ) -> Result<Vec<Option<ConcreteAccountRecord>>> {
        ensure!(
            std::ptr::eq(scope.owner_key, &self.owner.key),
            "APPLICATION_CONCRETE_FOREIGN_MUTATION_SCOPE"
        );
        let guard = self
            .lifecycle
            .lock()
            .map_err(|_| anyhow::anyhow!("APPLICATION_CONCRETE_LOCK_POISONED"))?;
        let lifecycle = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("APPLICATION_CONCRETE_REOPEN_REQUIRED"))?;
        let observation = lifecycle.observation()?;
        ensure!(
            observation.pending_marker_rlp.is_empty() && observation.committed == expected,
            "APPLICATION_CONCRETE_COMMITTED_IDENTITY_CHANGED"
        );
        let reader = lifecycle.prior_reader();
        ensure!(
            reader.identity() == expected,
            "APPLICATION_CONCRETE_READER_IDENTITY_CHANGED"
        );
        let records = addresses
            .iter()
            .map(|address| match reader.account(*address)? {
                ConcreteRead::Present(record) => Ok(Some(record)),
                ConcreteRead::Absent | ConcreteRead::Tombstone => Ok(None),
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            lifecycle.observation()? == observation && reader.identity() == expected,
            "APPLICATION_CONCRETE_CHANGED_DURING_READ"
        );
        Ok(records)
    }
}
