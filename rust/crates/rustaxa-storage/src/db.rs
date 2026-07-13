/// Storage module for Rustaxa, using RocksDB as the underlying database.
///
/// TODO:
///   - Schema & data migrations (also consider stand alone tool for this)
///   - Rebuild (also consider stand alone tool for this)
///   - Revert to period (also consider stand alone tool for this)
///   - Remove temporary files
///   - Make configurable via toml
///   - Snapshots
///
use anyhow::Result;
use rocksdb::{
    DBPinnableSlice, DBWithThreadMode, MultiThreaded, Options, WriteBatch, WriteOptions,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::Column;
use crate::Config;
use crate::DagRepository;
use crate::FinalChainRepository;
use crate::MetadataRepository;
use crate::PbftRepository;
use crate::PeriodRepository;
use crate::PillarRepository;
use crate::StorageError;
use crate::TransactionRepository;

/// Item returned by the database iterator.
/// Key and Value are boxed slices.
pub type IteratorItem = Result<(Box<[u8]>, Box<[u8]>)>;
/// Key/value tuple returned by seek-style helpers.
pub type KeyValueEntry = (Box<[u8]>, Box<[u8]>);
/// Iterator type for database queries.
pub type DbIterator<'a> = Box<dyn Iterator<Item = IteratorItem> + Send + Sync + 'a>;

/// Exclusive process-local capability for extra-reward-vote operations.
///
/// Holding this value serializes admission and reset storage operations. The
/// only public token-issuing operation commits the supplied reset batch before
/// advancing generation, so consumers cannot mint provenance independently of
/// a successful RocksDB commit.
pub struct ExtraRewardVotesGuard<'a> {
    _guard: MutexGuard<'a, ()>,
    storage: &'a Storage,
    reset_generation: &'a AtomicU64,
}

impl ExtraRewardVotesGuard<'_> {
    fn record_committed_reset(&self) -> u64 {
        let previous = self
            .reset_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                Some(generation.saturating_add(1))
            })
            .expect("extra-reward reset generation update cannot fail");
        previous.saturating_add(1)
    }

    fn commit_reset_batch_with(
        &self,
        batch: StorageWriteBatch,
        sync: bool,
        commit: impl FnOnce(StorageWriteBatch, bool) -> Result<()>,
    ) -> Result<u64> {
        commit(batch, sync)?;
        Ok(self.record_committed_reset())
    }

    /// Commits one locked reward-reset batch and returns its provenance token.
    ///
    /// RocksDB commit failure is returned without changing generation. After a
    /// successful commit the generation saturates at `u64::MAX`, preserving a
    /// non-zero valid token instead of wrapping into reserved generation zero.
    pub fn commit_reset_batch(&self, batch: StorageWriteBatch, sync: bool) -> Result<u64> {
        self.commit_reset_batch_with(batch, sync, |batch, sync| {
            self.storage.commit_write_batch_with_sync(batch, sync)
        })
    }
}

/// Trait abstracting database read operations.
pub trait DbReader: Send + Sync {
    /// The specific slice type returned by the backend.
    /// For RocksDB, this is `DBPinnableSlice` (zero-copy).
    /// For Mocks, this can be `Vec<u8>`.
    type Slice<'a>: AsRef<[u8]>
    where
        Self: 'a;

    fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>>;
    fn exist(&self, col: Column, key: &[u8]) -> Result<bool>;
    fn get_at_or_before(&self, col: Column, key: &[u8]) -> Result<Option<KeyValueEntry>>;
    fn get_at_or_after(&self, col: Column, key: &[u8]) -> Result<Option<KeyValueEntry>>;
    fn iter<'a>(&'a self, col: Column) -> DbIterator<'a>;
    fn iter_rev<'a>(&'a self, col: Column) -> DbIterator<'a>;
}

/// Trait abstracting database write operations.
pub trait DbWriter: Send + Sync {
    type Batch;

    fn create_batch(&self) -> Self::Batch;
    fn batch_put(
        &self,
        batch: &mut Self::Batch,
        col: Column,
        key: &[u8],
        value: &[u8],
    ) -> Result<()>;
    fn batch_delete(&self, batch: &mut Self::Batch, col: Column, key: &[u8]) -> Result<()>;
    fn commit_batch(&self, batch: Self::Batch) -> Result<()>;
    fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()>;
    fn delete(&self, col: Column, key: &[u8]) -> Result<()>;
}

/// Public batch type used by bridge-owned batch registries.
pub type StorageWriteBatch = WriteBatch;

impl DbReader for DBWithThreadMode<MultiThreaded> {
    type Slice<'a> = DBPinnableSlice<'a>;

    fn exist(&self, col: Column, key: &[u8]) -> Result<bool> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;

        if !self.key_may_exist_cf(&handle, key) {
            return Ok(false);
        }

        self.get_pinned_cf(&handle, key)
            .map(|value| value.is_some())
            .map_err(|e| StorageError::Database(e).into())
    }

    fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        self.get_pinned_cf(&handle, key)
            .map_err(|e| StorageError::Database(e).into())
    }

    fn get_at_or_before(&self, col: Column, key: &[u8]) -> Result<Option<KeyValueEntry>> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        let mut iter = self.iterator_cf(
            &handle,
            rocksdb::IteratorMode::From(key, rocksdb::Direction::Reverse),
        );
        match iter.next() {
            Some(res) => res
                .map(|(k, v)| Some((k, v)))
                .map_err(|e| StorageError::Database(e).into()),
            None => Ok(None),
        }
    }

    fn get_at_or_after(&self, col: Column, key: &[u8]) -> Result<Option<KeyValueEntry>> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        let mut iter = self.iterator_cf(
            &handle,
            rocksdb::IteratorMode::From(key, rocksdb::Direction::Forward),
        );
        match iter.next() {
            Some(res) => res
                .map(|(k, v)| Some((k, v)))
                .map_err(|e| StorageError::Database(e).into()),
            None => Ok(None),
        }
    }

    fn iter<'a>(&'a self, col: Column) -> DbIterator<'a> {
        match self.cf_handle(col.name()) {
            Some(handle) => {
                let iter = self
                    .iterator_cf(&handle, rocksdb::IteratorMode::Start)
                    .map(|res| res.map_err(|e| StorageError::Database(e).into()));
                Box::new(iter)
            }
            None => Box::new(std::iter::once(Err(StorageError::Config(format!(
                "Missing column family: {}",
                col.name()
            ))
            .into()))),
        }
    }

    fn iter_rev<'a>(&'a self, col: Column) -> DbIterator<'a> {
        match self.cf_handle(col.name()) {
            Some(handle) => {
                let iter = self
                    .iterator_cf(&handle, rocksdb::IteratorMode::End)
                    .map(|res| res.map_err(|e| StorageError::Database(e).into()));
                Box::new(iter)
            }
            None => Box::new(std::iter::once(Err(StorageError::Config(format!(
                "Missing column family: {}",
                col.name()
            ))
            .into()))),
        }
    }
}

impl DbWriter for DBWithThreadMode<MultiThreaded> {
    type Batch = WriteBatch;

    fn create_batch(&self) -> Self::Batch {
        WriteBatch::default()
    }

    fn batch_put(
        &self,
        batch: &mut Self::Batch,
        col: Column,
        key: &[u8],
        value: &[u8],
    ) -> Result<()> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        batch.put_cf(&handle, key, value);
        Ok(())
    }

    fn batch_delete(&self, batch: &mut Self::Batch, col: Column, key: &[u8]) -> Result<()> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        batch.delete_cf(&handle, key);
        Ok(())
    }

    fn commit_batch(&self, batch: Self::Batch) -> Result<()> {
        self.write_opt(batch, &WriteOptions::default())
            .map_err(|e| StorageError::Database(e).into())
    }

    fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        self.put_cf(&handle, key, value)
            .map_err(|e| StorageError::Database(e).into())
    }

    fn delete(&self, col: Column, key: &[u8]) -> Result<()> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        self.delete_cf(&handle, key)
            .map_err(|e| StorageError::Database(e).into())
    }
}

pub struct Storage {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    // Individual repositories splitting query/apply into domains.
    dag: DagRepository<DBWithThreadMode<MultiThreaded>>,
    pbft: PbftRepository<DBWithThreadMode<MultiThreaded>>,
    pillar: PillarRepository<DBWithThreadMode<MultiThreaded>>,
    period: PeriodRepository<DBWithThreadMode<MultiThreaded>>,
    transaction: TransactionRepository<DBWithThreadMode<MultiThreaded>>,
    metadata: MetadataRepository<DBWithThreadMode<MultiThreaded>>,
    final_chain: FinalChainRepository<DBWithThreadMode<MultiThreaded>>,
    own_verified_votes_lock: Mutex<()>,
    extra_reward_votes_lock: Mutex<()>,
    extra_reward_votes_reset_generation: AtomicU64,
}

impl Storage {
    pub fn new(config: Config) -> Result<Self> {
        std::fs::create_dir_all(&config.db_path).map_err(StorageError::Io)?;

        let mut opts = Options::default();
        opts.create_if_missing(config.create_if_missing);
        opts.create_missing_column_families(config.create_missing_column_families);
        opts.set_compression_type(config.compression);
        opts.set_max_total_wal_size(config.max_total_wal_size);
        opts.set_write_buffer_size(config.db_write_buffer_size);
        opts.set_max_open_files(config.max_open_files);

        let descriptors = || {
            config
                .column_families
                .iter()
                .map(|col| col.descriptor(&opts))
                .collect::<Vec<_>>()
        };

        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &opts,
            &config.db_path,
            descriptors(),
        )
        .map_err(StorageError::Database)?;

        let db = Arc::new(db);
        let dag = DagRepository::new(db.clone());
        let metadata = MetadataRepository::new(db.clone());
        let period = PeriodRepository::new(db.clone());
        let pillar = PillarRepository::new(db.clone());
        let pbft = PbftRepository::new(db.clone());
        let transaction = TransactionRepository::new(db.clone());
        let final_chain = FinalChainRepository::new(db.clone());

        Ok(Storage {
            db,
            dag,
            metadata,
            period,
            pillar,
            pbft,
            transaction,
            final_chain,
            own_verified_votes_lock: Mutex::new(()),
            extra_reward_votes_lock: Mutex::new(()),
            extra_reward_votes_reset_generation: AtomicU64::new(0),
        })
    }

    /// Acquires the process-local serialization guard for own PBFT vote rows.
    ///
    /// Production queries, saves, and lifecycle/direct clears must hold this
    /// guard for their complete storage operation. Handles sharing an
    /// `Arc<Storage>` therefore cannot interleave enumeration with a save or
    /// clear commit. Poisoning is reported as a storage error instead of
    /// silently permitting an unserialized operation.
    pub fn lock_own_verified_votes(&self) -> Result<MutexGuard<'_, ()>> {
        self.own_verified_votes_lock.lock().map_err(|_| {
            StorageError::Read("own verified votes serialization lock poisoned".into()).into()
        })
    }

    /// Acquires the process-local serialization guard for extra reward votes.
    ///
    /// Production admission writes, queries, and finalization/reset executors
    /// hold this guard across their complete operation. Handles sharing an
    /// `Arc<Storage>` therefore cannot insert a stale reward vote between reset
    /// enumeration and commit. Lock poisoning is returned as a storage error.
    pub fn lock_extra_reward_votes(&self) -> Result<ExtraRewardVotesGuard<'_>> {
        let guard = self.extra_reward_votes_lock.lock().map_err(|_| {
            StorageError::Read("extra reward votes serialization lock poisoned".into())
        })?;
        Ok(ExtraRewardVotesGuard {
            _guard: guard,
            storage: self,
            reset_generation: &self.extra_reward_votes_reset_generation,
        })
    }

    /// Returns the process-local provenance generation of the latest committed
    /// extra-reward-vote reset.
    ///
    /// Generation zero means this storage handle has not committed a reset.
    /// The counter is intentionally process-local and is not restored after a
    /// restart; restarted manager executors must obtain a new token from a new
    /// committed reset before reporting a reward-reset action.
    /// Admission writes deliberately do not change the generation, so a vote
    /// admitted for the next cycle cannot invalidate proof of the preceding
    /// reset. Callers use this only while validating a token returned by the
    /// Rust reset executor sharing this `Storage` instance.
    pub fn extra_reward_votes_reset_generation(&self) -> u64 {
        self.extra_reward_votes_reset_generation
            .load(Ordering::Acquire)
    }

    pub fn dag(&self) -> &DagRepository<DBWithThreadMode<MultiThreaded>> {
        &self.dag
    }

    pub fn period(&self) -> &PeriodRepository<DBWithThreadMode<MultiThreaded>> {
        &self.period
    }

    pub fn metadata(&self) -> &MetadataRepository<DBWithThreadMode<MultiThreaded>> {
        &self.metadata
    }

    pub fn pbft(&self) -> &PbftRepository<DBWithThreadMode<MultiThreaded>> {
        &self.pbft
    }

    pub fn pillar(&self) -> &PillarRepository<DBWithThreadMode<MultiThreaded>> {
        &self.pillar
    }

    pub fn transaction(&self) -> &TransactionRepository<DBWithThreadMode<MultiThreaded>> {
        &self.transaction
    }

    pub fn final_chain(&self) -> &FinalChainRepository<DBWithThreadMode<MultiThreaded>> {
        &self.final_chain
    }

    pub fn get_raw(&self, col: Column, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(DbReader::get(self, col, key)?.map(|v| v.as_ref().to_vec()))
    }

    pub fn seek_forward(&self, col: Column, key: &[u8]) -> Result<Option<KeyValueEntry>> {
        DbReader::get_at_or_after(self, col, key)
    }

    pub fn iter(&self, col: Column) -> DbIterator<'_> {
        DbReader::iter(self, col)
    }

    pub fn create_write_batch(&self) -> StorageWriteBatch {
        DbWriter::create_batch(self)
    }

    pub fn batch_put_raw(
        &self,
        batch: &mut StorageWriteBatch,
        col: Column,
        key: &[u8],
        value: &[u8],
    ) -> Result<()> {
        DbWriter::batch_put(self, batch, col, key, value)
    }

    pub fn batch_delete_raw(
        &self,
        batch: &mut StorageWriteBatch,
        col: Column,
        key: &[u8],
    ) -> Result<()> {
        DbWriter::batch_delete(self, batch, col, key)
    }

    pub fn commit_write_batch_with_sync(&self, batch: StorageWriteBatch, sync: bool) -> Result<()> {
        let mut opts = WriteOptions::default();
        opts.set_sync(sync);
        self.db
            .write_opt(batch, &opts)
            .map_err(|e| StorageError::Database(e).into())
    }
}

impl DbReader for Storage {
    type Slice<'a> = DBPinnableSlice<'a>;

    fn exist(&self, col: Column, key: &[u8]) -> Result<bool> {
        DbReader::exist(&*self.db, col, key)
    }

    fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
        DbReader::get(&*self.db, col, key)
    }

    fn get_at_or_before(&self, col: Column, key: &[u8]) -> Result<Option<KeyValueEntry>> {
        DbReader::get_at_or_before(&*self.db, col, key)
    }

    fn get_at_or_after(&self, col: Column, key: &[u8]) -> Result<Option<KeyValueEntry>> {
        DbReader::get_at_or_after(&*self.db, col, key)
    }

    fn iter<'a>(&'a self, col: Column) -> DbIterator<'a> {
        DbReader::iter(&*self.db, col)
    }

    fn iter_rev<'a>(&'a self, col: Column) -> DbIterator<'a> {
        DbReader::iter_rev(&*self.db, col)
    }
}

impl DbWriter for Storage {
    type Batch = WriteBatch;

    fn create_batch(&self) -> Self::Batch {
        DbWriter::create_batch(&*self.db)
    }

    fn batch_put(
        &self,
        batch: &mut Self::Batch,
        col: Column,
        key: &[u8],
        value: &[u8],
    ) -> Result<()> {
        DbWriter::batch_put(&*self.db, batch, col, key, value)
    }

    fn batch_delete(&self, batch: &mut Self::Batch, col: Column, key: &[u8]) -> Result<()> {
        DbWriter::batch_delete(&*self.db, batch, col, key)
    }

    fn commit_batch(&self, batch: Self::Batch) -> Result<()> {
        DbWriter::commit_batch(&*self.db, batch)
    }

    fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()> {
        DbWriter::put(&*self.db, col, key, value)
    }

    fn delete(&self, col: Column, key: &[u8]) -> Result<()> {
        DbWriter::delete(&*self.db, col, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rustaxa_storage_db_{name}_{}_{}",
            std::process::id(),
            unique
        ))
    }

    fn storage_at(name: &str) -> (std::path::PathBuf, Storage) {
        let path = unique_temp_dir(name);
        let _ = fs::remove_dir_all(&path);
        let storage = Storage::new(Config::new(path.clone())).expect("storage should initialize");
        (path, storage)
    }

    fn u64_le(value: u64) -> [u8; 8] {
        value.to_le_bytes()
    }

    #[test]
    fn raw_batch_commit_persists_status_value() {
        let (path, storage) = storage_at("raw_batch_commit");
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, Column::Status, &[0], &u64_le(123))
            .expect("batch put should append");
        storage
            .commit_write_batch_with_sync(batch, false)
            .expect("batch should commit");

        assert_eq!(
            storage
                .get_raw(Column::Status, &[0])
                .expect("status lookup should succeed"),
            Some(u64_le(123).to_vec())
        );
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn failed_reward_reset_commit_does_not_advance_generation() {
        let (path, storage) = storage_at("failed_reward_reset_generation");
        let guard = storage.lock_extra_reward_votes().unwrap();
        let batch = storage.create_write_batch();

        let result = guard.commit_reset_batch_with(batch, false, |_, _| {
            Err(anyhow::anyhow!("injected reset commit failure"))
        });

        assert!(result.is_err());
        assert_eq!(storage.extra_reward_votes_reset_generation(), 0);
        drop(guard);
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn dropped_raw_batch_does_not_persist_status_value() {
        let (path, storage) = storage_at("raw_batch_drop");
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, Column::Status, &[1], &u64_le(77))
            .expect("batch put should append");
        drop(batch);

        assert_eq!(
            storage
                .get_raw(Column::Status, &[1])
                .expect("status lookup should succeed"),
            None
        );
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn raw_batch_delete_removes_status_value() {
        let (path, storage) = storage_at("raw_batch_delete");
        storage
            .metadata()
            .write_status_field(2, 55)
            .expect("status seed should persist");

        let mut batch = storage.create_write_batch();
        storage
            .batch_delete_raw(&mut batch, Column::Status, &[2])
            .expect("batch delete should append");
        storage
            .commit_write_batch_with_sync(batch, false)
            .expect("delete batch should commit");

        assert_eq!(
            storage
                .get_raw(Column::Status, &[2])
                .expect("status lookup should succeed"),
            None
        );
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }
}
