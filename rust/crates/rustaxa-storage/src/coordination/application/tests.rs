use super::super::tests::{configuration, contents};
use super::*;
use crate::db::DbWriter;
use crate::{Column, Storage};
use std::sync::{Arc, mpsc};
use std::time::Instant;

#[test]
fn owner_is_unique_and_foreign_keys_cannot_enter() {
    let first = StorageCoordinator::new();
    let second = StorageCoordinator::new();
    let owner = first.claim_application_owner().unwrap();
    assert!(
        second
            .try_reserve_application_read_for(&owner, Duration::ZERO)
            .is_err()
    );
    let read = first
        .try_reserve_application_read_for(&owner, Duration::ZERO)
        .unwrap();
    let effects = read.begin_owned_effects().unwrap();
    assert!(first.begin_mutation().is_err());
    drop(effects);
    drop(owner);
    assert!(matches!(
        first.claim_application_owner(),
        Err(StorageCoordinationError::ApplicationOwnerAlreadyBound)
    ));
    first.begin_mutation().unwrap();
}

#[test]
fn protected_modes_reject_all_generic_writes_and_preserve_complete_contents() {
    let (_root, config) = configuration();
    let storage = Storage::new(config.clone()).unwrap();
    DbWriter::put(&storage, Column::DefaultColumn, b"wal", b"visible").unwrap();
    let before = contents(&storage, Column::all());
    assert!(before.iter().any(|(_, rows)| rows.is_empty()));
    let owner = storage
        .mutation_coordinator()
        .claim_application_owner()
        .unwrap();
    let read = storage
        .mutation_coordinator()
        .try_reserve_application_read_for(&owner, Duration::ZERO)
        .unwrap();
    let generation = read.generation();
    let mut ordinary = storage.create_write_batch();
    storage
        .batch_put_raw(&mut ordinary, Column::DefaultColumn, b"ordinary", b"denied")
        .unwrap();
    assert!(
        storage
            .commit_write_batch_with_sync(ordinary, false)
            .is_err()
    );
    assert_eq!(contents(&storage, Column::all()), before);
    let mut effects = read.begin_owned_effects().unwrap();
    assert!(storage.metadata().write_status_field(0, 2).is_err());
    assert!(DbWriter::put(&storage, Column::DefaultColumn, b"bypass", b"denied").is_err());
    assert_eq!(contents(&storage, Column::all()), before);
    let mut batch = storage.create_write_batch();
    storage
        .batch_put_raw(&mut batch, Column::DefaultColumn, b"owned", b"published")
        .unwrap();
    effects.commit_batch(&storage, batch, true).unwrap();
    let (read, receipt) = effects.finish().unwrap();
    assert_eq!(read.generation(), generation + 1);
    assert!(read.accepts(&receipt));
    assert!(storage.metadata().write_status_field(0, 3).is_err());
    drop(read);
    DbWriter::put(&storage, Column::DefaultColumn, b"fresh", b"valid").unwrap();
    let committed = contents(&storage, Column::all());
    drop(storage);
    let reopened = Storage::new(config).unwrap();
    assert_eq!(contents(&reopened, Column::all()), committed);
}

#[test]
fn queued_writer_cannot_enter_during_either_handoff() {
    let (_root, config) = configuration();
    let storage = Arc::new(Storage::new(config).unwrap());
    let owner = storage
        .mutation_coordinator()
        .claim_application_owner()
        .unwrap();
    let read = storage
        .mutation_coordinator()
        .try_reserve_application_read_for(&owner, Duration::ZERO)
        .unwrap();
    let attempts = storage.mutation_coordinator().mutation_attempts();
    let (done_tx, done_rx) = mpsc::channel();
    let other = Arc::clone(&storage);
    let worker = std::thread::spawn(move || {
        let result = DbWriter::put(other.as_ref(), Column::DefaultColumn, b"queued", b"valid");
        done_tx.send(result).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while storage.mutation_coordinator().mutation_attempts() == attempts {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(done_rx.try_recv().is_err());
    let effects = read.begin_owned_effects().unwrap();
    assert!(done_rx.try_recv().is_err());
    let (read, receipt) = effects.finish().unwrap();
    assert!(read.accepts(&receipt));
    assert!(done_rx.try_recv().is_err());
    drop(read);
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    let later = storage
        .mutation_coordinator()
        .try_reserve_application_read_for(&owner, Duration::ZERO)
        .unwrap();
    assert!(!later.accepts(&receipt));
}

#[test]
fn foreign_store_is_refused_before_write_and_does_not_poison_owner() {
    let (_a, a_config) = configuration();
    let (_b, b_config) = configuration();
    let a = Storage::new(a_config).unwrap();
    let b = Storage::new(b_config).unwrap();
    let before_a = contents(&a, Column::all());
    let before_b = contents(&b, Column::all());
    let owner = a.mutation_coordinator().claim_application_owner().unwrap();
    let read = a
        .mutation_coordinator()
        .try_reserve_application_read_for(&owner, Duration::ZERO)
        .unwrap();
    let mut effects = read.begin_owned_effects().unwrap();
    let mut batch = b.create_write_batch();
    b.batch_put_raw(&mut batch, Column::DefaultColumn, b"foreign", b"denied")
        .unwrap();
    assert!(effects.commit_batch(&b, batch, true).is_err());
    assert_eq!(contents(&a, Column::all()), before_a);
    assert_eq!(contents(&b, Column::all()), before_b);
    let (read, _) = effects.finish().unwrap();
    assert!(read.is_current());
}

#[test]
fn generation_exhaustion_releases_read_without_effects() {
    let coordinator = StorageCoordinator::new();
    let owner = coordinator.claim_application_owner().unwrap();
    coordinator.state.lock().borrow_mut().generation = u64::MAX;
    let read = coordinator
        .try_reserve_application_read_for(&owner, Duration::ZERO)
        .unwrap();
    assert!(matches!(
        read.begin_owned_effects(),
        Err(StorageCoordinationError::GenerationExhausted)
    ));
    assert!(!coordinator.state.lock().borrow().reserved_read);
    assert_eq!(coordinator.generation(), u64::MAX);
}

#[test]
fn abandoned_effects_after_a_write_require_recovery() {
    let (_root, config) = configuration();
    let storage = Storage::new(config).unwrap();
    let owner = storage
        .mutation_coordinator()
        .claim_application_owner()
        .unwrap();
    let read = storage
        .mutation_coordinator()
        .try_reserve_application_read_for(&owner, Duration::ZERO)
        .unwrap();
    let mut effects = read.begin_owned_effects().unwrap();
    let mut batch = storage.create_write_batch();
    storage
        .batch_put_raw(&mut batch, Column::DefaultColumn, b"durable", b"owned")
        .unwrap();
    effects.commit_batch(&storage, batch, true).unwrap();
    drop(effects);
    assert_eq!(
        storage.get_raw(Column::DefaultColumn, b"durable").unwrap(),
        Some(b"owned".to_vec())
    );
    assert!(
        storage
            .mutation_coordinator()
            .try_reserve_application_read_for(&owner, Duration::ZERO)
            .is_err()
    );
    assert!(DbWriter::put(&storage, Column::DefaultColumn, b"later", b"denied").is_err());
}

#[test]
fn ambiguous_backend_error_poison_blocks_all_following_writers() {
    let (_root, config) = configuration();
    let storage = Storage::new(config).unwrap();
    let owner = storage
        .mutation_coordinator()
        .claim_application_owner()
        .unwrap();
    let mut batch = storage.create_write_batch();
    storage
        .batch_put_raw(&mut batch, Column::PeriodData, b"failed", b"not-published")
        .unwrap();
    storage
        .drop_column_for_application_scope_test(Column::PeriodData)
        .unwrap();
    let read = storage
        .mutation_coordinator()
        .try_reserve_application_read_for(&owner, Duration::ZERO)
        .unwrap();
    let mut effects = read.begin_owned_effects().unwrap();
    assert!(effects.commit_batch(&storage, batch, true).is_err());
    assert!(effects.finish().is_err());
    assert!(DbWriter::put(&storage, Column::DefaultColumn, b"after-error", b"denied").is_err());
    assert!(
        storage
            .mutation_coordinator()
            .try_reserve_application_read_for(&owner, Duration::ZERO)
            .is_err()
    );
}
