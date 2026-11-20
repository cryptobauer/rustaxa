//! Storage capabilities for a closed application construction path.
//! These scopes grant no vote, finalization, or production qualification authority.
mod concrete;
pub use concrete::ApplicationConcreteState;
mod publication;
pub use publication::{CertifiedPrimaryPlan, CertifiedPublicationIntent, PreparedCertifiedState};

use crate::coordination::{ApplicationOwnerKey, StorageMutationGuard};
use crate::{Storage, StorageCoordinationError};
use std::sync::Arc;
use std::time::Duration;

/// Opaque pending certified work in Column::Status. Presence is not approval.
pub const PBFT_CERTIFIED_INTENT_KEY: &[u8] = b"rustaxa/pbft-certified-intent/v1";

/// Protected scopes expose no generic promotion method.
///
/// ```compile_fail
/// use rustaxa_storage::application_control::ApplicationReadScope;
/// fn promote(scope: ApplicationReadScope<'_>) {
///     let _ = scope.begin_owned_effects();
/// }
/// ```
pub use crate::coordination::{
    ApplicationReadScope, ApplicationWaitCallbackScope, OwnedEffectScope,
};

/// One permanent owner claim on the exact storage coordinator.
/// Trusted composition can share this value through Arc.
///
/// ```compile_fail
/// use rustaxa_storage::application_control::ApplicationOwner;
/// fn duplicate(owner: &ApplicationOwner) -> ApplicationOwner {
///     owner.clone()
/// }
/// ```
pub struct ApplicationOwner {
    storage: Arc<Storage>,
    key: ApplicationOwnerKey,
    publisher_attached: std::sync::atomic::AtomicBool,
}

/// An ordinary application mutation, entered before native or service locks.
/// This scope does not grant corrected execution authority.
///
/// ```compile_fail
/// use rustaxa_storage::application_control::ApplicationMutationScope;
/// fn require_send<T: Send>(_: T) {}
/// fn transfer(scope: ApplicationMutationScope<'_>) {
///     require_send(scope);
/// }
/// ```
#[derive(Debug)]
pub struct ApplicationMutationScope<'owner> {
    _guard: StorageMutationGuard<'owner>,
    owner_key: &'owner ApplicationOwnerKey,
}

impl ApplicationMutationScope<'_> {
    /// Returns the generation held by this ordinary mutation scope.
    /// An outer mutation advances generation when it enters.
    pub fn generation(&self) -> u64 {
        self._guard.generation()
    }
}

impl Storage {
    /// Claims the only application owner for this coordinator.
    /// Dropping the returned owner does not permit a later claim.
    pub fn claim_application_owner(
        self: &Arc<Self>,
    ) -> Result<ApplicationOwner, StorageCoordinationError> {
        let key = self.mutation_coordinator().claim_application_owner()?;
        Ok(ApplicationOwner {
            storage: Arc::clone(self),
            key,
            publisher_attached: std::sync::atomic::AtomicBool::new(false),
        })
    }
}

impl ApplicationOwner {
    /// Binds one trusted execution assembly. This grants no candidate approval.
    pub fn claim_execution_publisher(&self) -> anyhow::Result<()> {
        let _operation = self.begin_mutation()?;
        anyhow::ensure!(
            self.publisher_attached
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire
                )
                .is_ok(),
            "APPLICATION_EXECUTION_PUBLISHER_ALREADY_BOUND"
        );
        Ok(())
    }
    /// Refuses runtime or action reentry on this thread during process wait.
    pub fn ensure_not_waiting(&self) -> Result<(), StorageCoordinationError> {
        self.storage.mutation_coordinator().ensure_not_waiting()
    }

    /// Fences this thread for process.wait without retaining a state lock.
    /// Waits for other threads to release the coordinator. The caller must
    /// release all read, mutation, native, service and lifecycle guards first.
    /// Compare this generation with the prepared action before calling the host.
    pub fn try_wait_callback(
        &self,
    ) -> Result<ApplicationWaitCallbackScope<'_>, StorageCoordinationError> {
        self.storage
            .mutation_coordinator()
            .try_wait_callback_for(&self.key)
    }

    /// Checks the exact storage coordinator. This check grants no authority.
    pub fn is_for(&self, storage: &Storage) -> bool {
        std::ptr::eq(
            self.storage.mutation_coordinator(),
            storage.mutation_coordinator(),
        )
    }

    /// Enters an ordinary operation before its first native or service lock.
    /// A protected read or owned-effect scope on this thread refuses mutation.
    pub fn begin_mutation(&self) -> Result<ApplicationMutationScope<'_>, StorageCoordinationError> {
        Ok(ApplicationMutationScope {
            _guard: self.storage.mutation_coordinator().begin_mutation()?,
            owner_key: &self.key,
        })
    }

    /// Reserves the existing gate for a bounded protected read.
    /// The scope cannot cross a process or network wait.
    pub fn try_read(
        &self,
        wait: Duration,
    ) -> Result<ApplicationReadScope<'_>, StorageCoordinationError> {
        self.storage
            .mutation_coordinator()
            .try_reserve_application_read_for(&self.key, wait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, Config};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestStorage {
        storage: Option<Arc<Storage>>,
        path: PathBuf,
    }

    impl TestStorage {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = crate::closeout_test_scratch::new_path(
                std::env::temp_dir().join(format!(
                    "rustaxa-application-control-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed),
                )),
                true,
            );
            let mut config = Config::new(path.clone());
            config.db_write_buffer_size = 2 * 1024 * 1024;
            config.max_total_wal_size = 32 * 1024 * 1024;
            config.max_open_files = 32;
            let storage = Arc::new(Storage::new(config).unwrap());
            Self {
                storage: Some(storage),
                path,
            }
        }

        fn storage(&self) -> &Arc<Storage> {
            self.storage.as_ref().unwrap()
        }
    }

    impl Drop for TestStorage {
        fn drop(&mut self) {
            drop(self.storage.take());
            crate::closeout_test_scratch::preserve(&self.path);
        }
    }

    #[test]
    fn concrete_reader_rejects_foreign_owner_scopes() {
        let primary = TestStorage::new();
        let foreign = TestStorage::new();
        let owner = Arc::new(primary.storage().claim_application_owner().unwrap());
        let other = foreign.storage().claim_application_owner().unwrap();
        let lifecycle = crate::ConcreteStateLifecycle::create_fresh_exclusive(
            primary.path.join("state_db"),
            [4; 32],
            crate::ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        let expected = lifecycle.observation().unwrap();
        let state = ApplicationConcreteState::new(Arc::clone(&owner), lifecycle).unwrap();
        assert!(state.is_for(&owner));
        assert!(!state.is_for(&other));
        let read = other.try_read(Duration::ZERO).unwrap();
        assert!(state.observation(&read).is_err());
        drop(read);
        let mutation = other.begin_mutation().unwrap();
        assert!(
            state
                .accounts(&mutation, expected.committed, &[[1; 20]])
                .is_err()
        );
        drop(mutation);
        let read = owner.try_read(Duration::ZERO).unwrap();
        assert_eq!(state.observation(&read).unwrap(), expected);
        drop(read);
        let mutation = owner.begin_mutation().unwrap();
        assert_eq!(
            state
                .accounts(&mutation, expected.committed, &[[1; 20]])
                .unwrap(),
            vec![None]
        );
    }

    #[test]
    fn execution_publisher_attachment_is_single_use() {
        let primary = TestStorage::new();
        let owner = primary.storage().claim_application_owner().unwrap();
        owner.claim_execution_publisher().unwrap();
        assert!(
            owner
                .claim_execution_publisher()
                .unwrap_err()
                .to_string()
                .contains("APPLICATION_EXECUTION_PUBLISHER_ALREADY_BOUND")
        );
    }

    #[test]
    fn facade_owner_uses_exact_coordinator_and_cannot_rebind() {
        let primary = TestStorage::new();
        let foreign = TestStorage::new();
        let owner = primary.storage().claim_application_owner().unwrap();
        assert!(owner.is_for(primary.storage()));
        assert!(!owner.is_for(foreign.storage()));
        assert!(matches!(
            primary.storage().claim_application_owner(),
            Err(StorageCoordinationError::ApplicationOwnerAlreadyBound)
        ));
        drop(owner);
        assert!(matches!(
            primary.storage().claim_application_owner(),
            Err(StorageCoordinationError::ApplicationOwnerAlreadyBound)
        ));
    }

    #[test]
    fn facade_scopes_block_alias_writes_and_keep_generation() {
        let primary = TestStorage::new();
        let alias = Arc::clone(primary.storage());
        let owner = primary.storage().claim_application_owner().unwrap();
        let key = b"rustaxa/application-control/test";
        let before = primary.storage().mutation_coordinator().generation();
        {
            let _mutation = owner.begin_mutation().unwrap();
            assert!(matches!(
                owner.try_read(Duration::ZERO),
                Err(StorageCoordinationError::ReadUnavailable)
            ));
            let mut batch = alias.create_write_batch();
            alias
                .batch_put_raw(&mut batch, Column::Status, key, b"before")
                .unwrap();
            alias.commit_write_batch_with_sync(batch, true).unwrap();
        }
        let read = owner.try_read(Duration::ZERO).unwrap();
        let generation = read.generation();
        assert_eq!(generation, before + 1);
        assert!(read.is_current());
        assert!(matches!(
            owner.begin_mutation(),
            Err(StorageCoordinationError::MutationDuringRead)
        ));
        for storage in [primary.storage(), &alias] {
            let mut batch = storage.create_write_batch();
            storage
                .batch_put_raw(&mut batch, Column::Status, key, b"blocked")
                .unwrap();
            let error = storage
                .commit_write_batch_with_sync(batch, true)
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<StorageCoordinationError>(),
                Some(&StorageCoordinationError::MutationDuringRead)
            );
        }
        assert_eq!(
            alias.get_raw(Column::Status, key).unwrap(),
            Some(b"before".to_vec())
        );
        assert_eq!(read.generation(), generation);
        assert!(read.is_current());
        drop(read);
        {
            let _mutation = owner.begin_mutation().unwrap();
            let mut batch = alias.create_write_batch();
            alias
                .batch_put_raw(&mut batch, Column::Status, key, b"after")
                .unwrap();
            alias.commit_write_batch_with_sync(batch, true).unwrap();
        }
        let read = owner.try_read(Duration::ZERO).unwrap();
        assert_eq!(read.generation(), generation + 1);
        assert!(read.is_current());
        assert_eq!(
            alias.get_raw(Column::Status, key).unwrap(),
            Some(b"after".to_vec())
        );
    }

    #[test]
    fn wait_callback_blocks_same_thread_aliases_and_owned_reentry() {
        let primary = TestStorage::new();
        let alias = Arc::clone(primary.storage());
        let owner = primary.storage().claim_application_owner().unwrap();
        let key = b"rustaxa/application-control/wait";
        let mut prebuilt = alias.create_write_batch();
        alias
            .batch_put_raw(&mut prebuilt, Column::Status, key, b"blocked")
            .unwrap();
        let before = alias.mutation_coordinator().generation();
        let attempts = alias.mutation_coordinator().mutation_attempts();
        let fence = owner.try_wait_callback().unwrap();
        assert_eq!(fence.generation(), before);
        assert_eq!(
            owner.ensure_not_waiting(),
            Err(StorageCoordinationError::WaitCallbackActive)
        );
        assert!(matches!(
            owner.try_wait_callback(),
            Err(StorageCoordinationError::WaitCallbackActive)
        ));
        assert!(matches!(
            owner.begin_mutation(),
            Err(StorageCoordinationError::WaitCallbackActive)
        ));
        let error = alias
            .commit_write_batch_with_sync(prebuilt, true)
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<StorageCoordinationError>(),
            Some(&StorageCoordinationError::WaitCallbackActive)
        );
        let read = owner.try_read(Duration::ZERO).unwrap();
        assert_eq!(read.generation(), before);
        assert!(read.is_current());
        assert!(matches!(
            read.begin_owned_effects(),
            Err(StorageCoordinationError::WaitCallbackActive)
        ));
        assert_eq!(
            owner.ensure_not_waiting(),
            Err(StorageCoordinationError::WaitCallbackActive)
        );
        assert_eq!(alias.get_raw(Column::Status, key).unwrap(), None);
        assert_eq!(alias.mutation_coordinator().generation(), before);
        assert_eq!(alias.mutation_coordinator().mutation_attempts(), attempts);
        drop(fence);
        owner.ensure_not_waiting().unwrap();
        drop(owner.begin_mutation().unwrap());
    }

    #[test]
    fn wait_callback_allows_a_concurrent_mutation_to_finish() {
        let primary = TestStorage::new();
        let owner = primary.storage().claim_application_owner().unwrap();
        let mutation = owner.begin_mutation().unwrap();
        let generation = mutation.generation();
        let (starting, started) = std::sync::mpsc::channel();
        let (complete, completed) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                starting.send(()).unwrap();
                let result = owner.try_wait_callback().map(|guard| guard.generation());
                complete.send(result).unwrap();
            });
            started.recv().unwrap();
            let while_mutating = completed.recv_timeout(Duration::from_millis(100));
            drop(mutation);
            assert_eq!(
                while_mutating,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            );
            assert_eq!(
                completed.recv_timeout(Duration::from_secs(1)).unwrap(),
                Ok(generation)
            );
        });
    }

    #[test]
    fn wait_callback_allows_other_thread_progress_and_exposes_stale_generation() {
        let primary = TestStorage::new();
        let alias = Arc::clone(primary.storage());
        let owner = Arc::new(primary.storage().claim_application_owner().unwrap());
        let fence = owner.try_wait_callback().unwrap();
        let captured = fence.generation();
        let writer_owner = Arc::clone(&owner);
        let (done, completion) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let _mutation = writer_owner.begin_mutation()?;
                let mut batch = alias.create_write_batch();
                alias.batch_put_raw(
                    &mut batch,
                    Column::Status,
                    b"rustaxa/application-control/other-thread",
                    b"committed",
                )?;
                alias.commit_write_batch_with_sync(batch, true)?;
                Ok(())
            })();
            let _ = done.send(result.is_ok());
            result
        });
        let completed_while_fenced = completion.recv_timeout(Duration::from_secs(2));
        drop(fence);
        let result = writer.join().expect("writer thread did not panic");
        assert_eq!(completed_while_fenced.unwrap(), true);
        result.unwrap();
        let read = owner.try_read(Duration::ZERO).unwrap();
        assert_ne!(read.generation(), captured);
        assert!(read.is_current());
        drop(read);
        let resumed = owner.begin_mutation().unwrap();
        assert_ne!(
            resumed.generation(),
            captured.checked_add(1).unwrap(),
            "an intervening writer must invalidate the suspended source"
        );
    }

    #[test]
    fn wait_callback_cleanup_preserves_other_owner_and_unwind() {
        let primary = TestStorage::new();
        let foreign = TestStorage::new();
        let owner = primary.storage().claim_application_owner().unwrap();
        let other = foreign.storage().claim_application_owner().unwrap();
        let outer = owner.try_wait_callback().unwrap();
        let inner = other.try_wait_callback().unwrap();
        drop(inner);
        other.ensure_not_waiting().unwrap();
        assert_eq!(
            owner.ensure_not_waiting(),
            Err(StorageCoordinationError::WaitCallbackActive)
        );
        drop(outer);
        owner.ensure_not_waiting().unwrap();
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _fence = owner.try_wait_callback().unwrap();
            panic!("test wait unwind");
        }));
        assert!(failure.is_err());
        owner.ensure_not_waiting().unwrap();
        drop(owner.begin_mutation().unwrap());
    }
}
