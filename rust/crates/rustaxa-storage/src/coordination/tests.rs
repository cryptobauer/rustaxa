use super::*;
use crate::db::DbWriter;
use crate::{Column, Config, LightHistoryPruneRequest, Storage, StorageError};
use ethereum_types::H256;
use std::sync::{Arc, mpsc};
use std::time::Instant;

pub(super) fn configuration() -> Config {
    let path = crate::test_paths::new_path("storage_coordination");
    let mut config = Config::new(path);
    config.db_write_buffer_size = 2 * 1024 * 1024;
    config.max_total_wal_size = 32 * 1024 * 1024;
    config.max_open_files = 32;
    config
}

pub(super) type Contents = Vec<(&'static str, Vec<(Vec<u8>, Vec<u8>)>)>;

pub(super) fn contents(storage: &Storage, columns: &[Column]) -> Contents {
    columns
        .iter()
        .map(|column| {
            let rows = storage
                .iter(*column)
                .map(|row| {
                    let (key, value) = row.unwrap();
                    (key.into_vec(), value.into_vec())
                })
                .collect();
            (column.name(), rows)
        })
        .collect()
}

fn refused(result: anyhow::Result<()>) {
    let error = result.unwrap_err();
    assert_eq!(
        error.downcast_ref::<StorageCoordinationError>(),
        Some(&StorageCoordinationError::MutationDuringRead)
    );
}

#[test]
fn retained_repository_aliases_refuse_writes_then_publish_and_reopen() {
    let config = configuration();
    let storage = Storage::new(config.clone()).unwrap();
    DbWriter::put(&storage, Column::DefaultColumn, b"wal", b"visible").unwrap();
    let dag = storage.dag();
    let metadata = storage.metadata();
    let pbft = storage.pbft();
    let period = storage.period();
    let pillar = storage.pillar();
    let transactions = storage.transaction();
    let final_chain = storage.final_chain();
    let writes: Vec<Box<dyn Fn() -> anyhow::Result<()>>> = vec![
        Box::new(|| dag.write_proposal_period_at_level(1, 2)),
        Box::new(|| metadata.write_status_field(0, 3)),
        Box::new(|| pbft.write_manager_field(0, 4)),
        Box::new(|| period.write_pbft_period(H256::repeat_byte(1), 5)),
        Box::new(|| pillar.write_own_vote(&[0xc0])),
        Box::new(|| transactions.write(H256::repeat_byte(2), &[0xc0])),
        Box::new(|| final_chain.delete_external_evm_pending_publication()),
    ];
    let before = contents(&storage, Column::all());
    assert_eq!(before.len(), Column::all().len());
    assert!(before.iter().any(|(_, rows)| rows.is_empty()));
    let reservation = storage
        .mutation_coordinator()
        .try_reserve_storage_read_for(Duration::ZERO)
        .unwrap();
    for write in &writes {
        refused(write());
        assert_eq!(contents(&storage, Column::all()), before);
        assert!(reservation.is_current());
    }
    drop(reservation);
    for write in &writes {
        write().unwrap();
    }
    drop(writes);
    let committed = contents(&storage, Column::all());
    assert_ne!(committed, before);
    drop(storage);
    let reopened = Storage::new(config).unwrap();
    assert_eq!(contents(&reopened, Column::all()), committed);
}

#[test]
fn prebuilt_batches_and_raw_deletes_cannot_cross_the_read_reservation() {
    let config = configuration();
    let storage = Storage::new(config).unwrap();
    DbWriter::put(&storage, Column::DefaultColumn, b"keep", b"wal").unwrap();
    let mut first = storage.create_write_batch();
    let mut second = storage.create_write_batch();
    for batch in [&mut first, &mut second] {
        storage
            .batch_put_raw(batch, Column::DefaultColumn, b"prefix", b"hidden")
            .unwrap();
        storage
            .batch_delete_raw(batch, Column::DefaultColumn, b"keep")
            .unwrap();
    }
    let before = contents(&storage, Column::all());
    let reservation = storage
        .mutation_coordinator()
        .try_reserve_storage_read_for(Duration::ZERO)
        .unwrap();
    refused(storage.commit_write_batch_with_sync(first, false));
    refused(storage.commit_write_batch_with_sync(second, true));
    refused(DbWriter::delete(&storage, Column::DefaultColumn, b"keep"));
    assert_eq!(contents(&storage, Column::all()), before);
    drop(reservation);
    let mut fresh = storage.create_write_batch();
    storage
        .batch_put_raw(&mut fresh, Column::DefaultColumn, b"fresh", b"valid")
        .unwrap();
    storage.commit_write_batch_with_sync(fresh, true).unwrap();
    assert_eq!(
        storage.get_raw(Column::DefaultColumn, b"keep").unwrap(),
        Some(b"wal".to_vec())
    );
    assert_eq!(
        storage.get_raw(Column::DefaultColumn, b"prefix").unwrap(),
        None
    );
    assert_eq!(
        storage.get_raw(Column::DefaultColumn, b"fresh").unwrap(),
        Some(b"valid".to_vec())
    );
}

#[test]
fn queued_actual_repository_writer_makes_progress_after_release() {
    let config = configuration();
    let storage = Arc::new(Storage::new(config.clone()).unwrap());
    let before = contents(&storage, Column::all());
    let reservation = storage
        .mutation_coordinator()
        .try_reserve_storage_read_for(Duration::ZERO)
        .unwrap();
    let attempts = storage.mutation_coordinator().mutation_attempts();
    let (done, completed) = mpsc::sync_channel(1);
    let worker_storage = Arc::clone(&storage);
    let worker = std::thread::spawn(move || {
        let result = worker_storage
            .transaction()
            .write(H256::repeat_byte(7), &[0xc0]);
        done.send(result).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while storage.mutation_coordinator().mutation_attempts() == attempts {
        assert!(
            Instant::now() < deadline,
            "writer did not reach the coordinator"
        );
        std::thread::yield_now();
    }
    assert!(matches!(
        completed.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert_eq!(contents(&storage, Column::all()), before);
    drop(reservation);
    completed
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    let committed = contents(&storage, Column::all());
    assert_ne!(committed, before);
    drop(storage);
    let reopened = Storage::new(config).unwrap();
    assert_eq!(contents(&reopened, Column::all()), committed);
}

#[test]
fn storage_serialization_guards_do_not_grant_reserved_writes() {
    let config = configuration();
    let storage = Storage::new(config).unwrap();
    let generation = storage.extra_reward_votes_reset_generation();
    let reservation = storage
        .mutation_coordinator()
        .try_reserve_storage_read_for(Duration::ZERO)
        .unwrap();
    {
        let _own_votes = storage.lock_own_verified_votes().unwrap();
        let reward_votes = storage.lock_extra_reward_votes().unwrap();
        let batch = storage.create_write_batch();
        let error = reward_votes.commit_reset_batch(batch, true).unwrap_err();
        assert_eq!(
            error.downcast_ref::<StorageCoordinationError>(),
            Some(&StorageCoordinationError::MutationDuringRead)
        );
        assert_eq!(storage.extra_reward_votes_reset_generation(), generation);
    }
    assert!(reservation.is_current());
    drop(reservation);
    let reward_votes = storage.lock_extra_reward_votes().unwrap();
    assert_eq!(
        reward_votes
            .commit_reset_batch(storage.create_write_batch(), true)
            .unwrap(),
        generation + 1
    );
}

#[test]
fn prune_refuses_a_reserved_read_before_its_private_lock() {
    let config = configuration();
    let storage = Storage::new(config).unwrap();
    let before = contents(&storage, Column::all());
    let request = LightHistoryPruneRequest {
        end_period_exclusive: 1,
        first_retained_dag_level: 0,
        live_cleanup: true,
        non_block_periods_to_keep: 1,
    };
    let reservation = storage
        .mutation_coordinator()
        .try_reserve_storage_read_for(Duration::ZERO)
        .unwrap();
    let error = storage.prune_light_history(request).unwrap_err();
    assert_eq!(
        error.downcast_ref::<StorageCoordinationError>(),
        Some(&StorageCoordinationError::MutationDuringRead)
    );
    assert_eq!(contents(&storage, Column::all()), before);
    drop(reservation);
    assert!(!storage.prune_light_history(request).unwrap().changed);
}

#[test]
fn failed_backend_write_invalidates_generation_without_publishing() {
    let mut config = configuration();
    config
        .column_families
        .retain(|column| *column != Column::Status);
    let storage = Storage::new(config.clone()).unwrap();
    let before = contents(&storage, &config.column_families);
    let generation = storage.mutation_coordinator().generation();
    let error = storage.metadata().write_status_field(0, 1).unwrap_err();
    assert!(error.downcast_ref::<StorageError>().is_some());
    assert!(storage.mutation_coordinator().generation() > generation);
    assert_eq!(contents(&storage, &config.column_families), before);
    DbWriter::put(&storage, Column::DefaultColumn, b"fresh", b"valid").unwrap();
}

#[test]
fn nested_mutations_keep_exclusion_when_outer_guard_drops_first() {
    let owner = Arc::new(StorageCoordinator::new());
    let outer = owner.begin_mutation().unwrap();
    let inner = owner.begin_mutation().unwrap();
    let generation = owner.generation();
    drop(outer);
    let other = Arc::clone(&owner);
    let busy = std::thread::spawn(move || {
        matches!(
            other.try_reserve_storage_read_for(Duration::ZERO),
            Err(StorageCoordinationError::ReadUnavailable)
        )
    })
    .join()
    .unwrap();
    assert!(busy);
    drop(inner);
    let reservation = owner.try_reserve_storage_read_for(Duration::ZERO).unwrap();
    assert_eq!(reservation.generation(), generation);
    assert!(reservation.is_current());
}

#[test]
fn equal_generations_cannot_substitute_another_owner() {
    let first = StorageCoordinator::new();
    let second = StorageCoordinator::new();
    let reservation = first.try_reserve_storage_read_for(Duration::ZERO).unwrap();
    assert_eq!(first.generation(), second.generation());
    assert!(reservation.is_from(&first));
    assert!(!reservation.is_from(&second));
}

#[test]
fn generation_exhaustion_refuses_before_the_backend() {
    let config = configuration();
    let storage = Storage::new(config).unwrap();
    let before = contents(&storage, Column::all());
    storage
        .mutation_coordinator()
        .state
        .lock()
        .borrow_mut()
        .generation = u64::MAX;
    let error = DbWriter::put(&storage, Column::DefaultColumn, b"overflow", b"hidden").unwrap_err();
    assert_eq!(
        error.downcast_ref::<StorageCoordinationError>(),
        Some(&StorageCoordinationError::GenerationExhausted)
    );
    assert_eq!(contents(&storage, Column::all()), before);
}

#[test]
fn existing_store_open_refuses_missing_path_and_preserves_reopened_contents() {
    let config = configuration();
    assert!(config.create_if_missing && config.create_missing_column_families);
    assert!(!config.db_path.exists());
    let parent = config.db_path.parent().unwrap();
    let parent_existed = parent.exists();
    assert!(Storage::open_existing(config.clone()).is_err());
    assert!(!config.db_path.exists());
    assert_eq!(parent.exists(), parent_existed);

    let storage = Storage::new(config.clone()).unwrap();
    DbWriter::put(&storage, Column::DefaultColumn, b"existing", b"committed").unwrap();
    let before = contents(&storage, Column::all());
    assert!(before.iter().any(|(_, rows)| rows.is_empty()));
    drop(storage);
    let reopened = Storage::open_existing(config.clone()).unwrap();
    assert_eq!(contents(&reopened, Column::all()), before);
    DbWriter::put(&reopened, Column::DefaultColumn, b"later", b"valid").unwrap();
    let after = contents(&reopened, Column::all());
    drop(reopened);
    let reopened = Storage::open_existing(config).unwrap();
    assert_eq!(contents(&reopened, Column::all()), after);
}

#[test]
fn existing_store_open_refuses_missing_columns_without_adding_them() {
    let full = configuration();
    let mut partial = full.clone();
    let omitted = partial.column_families.pop().unwrap();
    assert_ne!(omitted, Column::DefaultColumn);
    let storage = Storage::new(partial.clone()).unwrap();
    DbWriter::put(&storage, Column::DefaultColumn, b"existing", b"committed").unwrap();
    let before = contents(&storage, &partial.column_families);
    drop(storage);
    assert!(Storage::open_existing(full).is_err());
    let reopened = Storage::open_existing(partial.clone()).unwrap();
    assert_eq!(contents(&reopened, &partial.column_families), before);
    drop(reopened);
    let names = rocksdb::DB::list_cf(&rocksdb::Options::default(), &partial.db_path).unwrap();
    assert!(!names.iter().any(|name| name == omitted.name()));
    assert_eq!(names.len(), partial.column_families.len());
}

#[test]
fn existing_store_open_preserves_status_presence_and_malformed_bytes() {
    let config = configuration();
    let storage = Storage::new(config.clone()).unwrap();
    let field = crate::StatusField::DbMinorVersion as u8;
    let before = contents(&storage, Column::all());
    assert_eq!(
        storage.metadata().status_field_optional(field).unwrap(),
        None
    );
    assert_eq!(storage.metadata().status_field(field).unwrap(), 0);
    assert_eq!(contents(&storage, Column::all()), before);
    storage.metadata().write_status_field(field, 0).unwrap();
    let before = contents(&storage, Column::all());
    assert_eq!(
        storage.metadata().status_field_optional(field).unwrap(),
        Some(0)
    );
    assert_eq!(storage.metadata().status_field(field).unwrap(), 0);
    assert_eq!(contents(&storage, Column::all()), before);
    storage.metadata().write_status_field(field, 7).unwrap();
    assert_eq!(
        storage.metadata().status_field_optional(field).unwrap(),
        Some(7)
    );
    DbWriter::put(&storage, Column::Status, &[field], &[1]).unwrap();
    let malformed = contents(&storage, Column::all());
    assert!(storage.metadata().status_field_optional(field).is_err());
    assert!(storage.metadata().status_field(field).is_err());
    assert_eq!(contents(&storage, Column::all()), malformed);
    drop(storage);
    let reopened = Storage::open_existing(config).unwrap();
    assert_eq!(contents(&reopened, Column::all()), malformed);
}
