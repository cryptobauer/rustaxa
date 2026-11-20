/// Storage module for Rustaxa, using RocksDB as the underlying database.
///
/// TODO:
///   - Schema & data migrations (also consider stand alone tool for this)
///   - Rebuild (also consider stand alone tool for this)
///   - Revert to period (also consider stand alone tool for this)
///   - Remove temporary files
///   - Make configurable via toml
///
use anyhow::Result;
use rocksdb::{
    DBPinnableSlice, DBWithThreadMode, MultiThreaded, Options, SnapshotWithThreadMode, WriteBatch,
    WriteOptions,
};
use rustaxa_types::codec::rlp::dag::FinalizedDagBlockBundleRlp;
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
use tiny_keccak::{Hasher, Keccak};

const PBFT_BLOCK_POS_IN_PERIOD_DATA: usize = 0;
const DAG_BLOCKS_POS_IN_PERIOD_DATA: usize = 2;
const TRANSACTIONS_POS_IN_PERIOD_DATA: usize = 3;

#[derive(Debug)]
struct FinalizedPeriodIndexKeys {
    pbft_block: [u8; 32],
    dag_blocks: Vec<[u8; 32]>,
    transactions: Vec<[u8; 32]>,
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    let mut hash = [0_u8; 32];
    hasher.update(bytes);
    hasher.finalize(&mut hash);
    hash
}

/// Extracts the exact reverse-index keys owned by one canonical `PeriodData` row.
///
/// The decoder hashes the canonical PBFT block and transaction RLP children and
/// reconstructs canonical DAG blocks from their compact finalized bundle before
/// hashing. Malformed input fails before the caller commits its pruning batch.
fn finalized_period_index_keys(period_data: &[u8]) -> Result<FinalizedPeriodIndexKeys> {
    let period_data = rlp::Rlp::new(period_data);
    let field_count = period_data.item_count()?;
    if field_count != 4 && field_count != 5 {
        return Err(StorageError::Read(
            "LIGHT_HISTORY_PRUNE_INVALID_PERIOD_DATA_FIELD_COUNT".into(),
        )
        .into());
    }
    let pbft_block = period_data.at(PBFT_BLOCK_POS_IN_PERIOD_DATA)?;
    let dag_bundle = period_data.at(DAG_BLOCKS_POS_IN_PERIOD_DATA)?;
    let transactions = period_data.at(TRANSACTIONS_POS_IN_PERIOD_DATA)?;

    let dag_blocks = if dag_bundle.is_empty() {
        Vec::new()
    } else {
        let compact_blocks = dag_bundle.at(2)?;
        let bundle = FinalizedDagBlockBundleRlp::new(dag_bundle.as_raw());
        (0..compact_blocks.item_count()?)
            .map(|position| {
                bundle
                    .canonical_block_rlp(position)
                    .map(|rlp| keccak256(&rlp))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let transactions = (0..transactions.item_count()?)
        .map(|position| transactions.at(position).map(|rlp| keccak256(rlp.as_raw())))
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(FinalizedPeriodIndexKeys {
        pbft_block: keccak256(pbft_block.as_raw()),
        dag_blocks,
        transactions,
    })
}

/// Exact native storage task for pruning finalized light-node history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightHistoryPruneRequest {
    /// Finalized periods strictly below this value are removed from bulk history columns.
    pub end_period_exclusive: u64,
    /// DAG level keys strictly below this value are removed.
    pub first_retained_dag_level: u64,
    /// Selects incremental cleanup semantics used during live finalization.
    pub live_cleanup: bool,
    /// Number of recent periods whose transaction/DAG/PBFT reverse indexes remain available.
    pub non_block_periods_to_keep: u64,
}

/// Result of one atomic native light-history pruning task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightHistoryPruneReport {
    /// True when at least one durable key was selected and deleted.
    pub changed: bool,
    /// Exclusive finalized-period cutoff applied by the task.
    pub end_period_exclusive: u64,
    /// First retained DAG level applied by the task.
    pub first_retained_dag_level: u64,
    /// True when the non-live rebuild policy retained only the configured recent reverse indexes.
    pub rebuilt_secondary_indexes: bool,
}

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
    _owner: crate::coordination::StorageObservationGuard<'a>,
    storage: &'a Storage,
    reset_generation: &'a AtomicU64,
}

impl ExtraRewardVotesGuard<'_> {
    pub(crate) fn record_committed_reset(&self) -> u64 {
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

/// Narrow point-read port with backend-owned borrowed bytes and typed errors.
/// Missing, empty, and backend failure remain distinct. The returned slice
/// cannot outlive the backend borrow; this port exposes no iterator or writes.
pub trait DbPointReader: Send + Sync {
    type Slice<'a>: AsRef<[u8]>
    where
        Self: 'a;
    type Error;
    fn point_get<'a>(
        &'a self,
        column: Column,
        key: &[u8],
    ) -> std::result::Result<Option<Self::Slice<'a>>, Self::Error>;
}

/// Preserve the existing read result and error for every ordinary backend.
impl<D: DbReader> DbPointReader for D {
    type Slice<'a>
        = <D as DbReader>::Slice<'a>
    where
        Self: 'a;
    type Error = anyhow::Error;
    fn point_get<'a>(
        &'a self,
        column: Column,
        key: &[u8],
    ) -> std::result::Result<Option<Self::Slice<'a>>, Self::Error> {
        self.get(column, key)
    }
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
    /// Commits a batch with an optional durability barrier.
    ///
    /// In-memory/test writers may use the default behavior; persistent RocksDB
    /// writers override this to honor `sync` through `WriteOptions`.
    fn commit_batch_with_sync(&self, batch: Self::Batch, _sync: bool) -> Result<()> {
        self.commit_batch(batch)
    }
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

    fn commit_batch_with_sync(&self, batch: Self::Batch, sync: bool) -> Result<()> {
        let mut options = WriteOptions::default();
        options.set_sync(sync);
        self.write_opt(batch, &options)
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

/// Persistent backend shared by all repositories returned by `Storage`.
/// The raw writable handle cannot leave this module.
pub struct CoordinatedDatabase {
    raw: DBWithThreadMode<MultiThreaded>,
    coordinator: crate::StorageCoordinator,
}

impl CoordinatedDatabase {
    fn new(raw: DBWithThreadMode<MultiThreaded>) -> Self {
        Self {
            raw,
            coordinator: crate::StorageCoordinator::new(),
        }
    }
}

impl DbReader for CoordinatedDatabase {
    type Slice<'a> = DBPinnableSlice<'a>;

    fn exist(&self, col: Column, key: &[u8]) -> Result<bool> {
        DbReader::exist(&self.raw, col, key)
    }

    fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
        DbReader::get(&self.raw, col, key)
    }

    fn get_at_or_before(&self, col: Column, key: &[u8]) -> Result<Option<KeyValueEntry>> {
        DbReader::get_at_or_before(&self.raw, col, key)
    }

    fn get_at_or_after(&self, col: Column, key: &[u8]) -> Result<Option<KeyValueEntry>> {
        DbReader::get_at_or_after(&self.raw, col, key)
    }

    fn iter<'a>(&'a self, col: Column) -> DbIterator<'a> {
        DbReader::iter(&self.raw, col)
    }

    fn iter_rev<'a>(&'a self, col: Column) -> DbIterator<'a> {
        DbReader::iter_rev(&self.raw, col)
    }
}

impl DbWriter for CoordinatedDatabase {
    type Batch = WriteBatch;

    fn create_batch(&self) -> Self::Batch {
        DbWriter::create_batch(&self.raw)
    }

    fn batch_put(
        &self,
        batch: &mut Self::Batch,
        col: Column,
        key: &[u8],
        value: &[u8],
    ) -> Result<()> {
        DbWriter::batch_put(&self.raw, batch, col, key, value)
    }

    fn batch_delete(&self, batch: &mut Self::Batch, col: Column, key: &[u8]) -> Result<()> {
        DbWriter::batch_delete(&self.raw, batch, col, key)
    }

    fn commit_batch(&self, batch: Self::Batch) -> Result<()> {
        let _mutation = self.coordinator.begin_mutation()?;
        DbWriter::commit_batch(&self.raw, batch)
    }

    fn commit_batch_with_sync(&self, batch: Self::Batch, sync: bool) -> Result<()> {
        let _mutation = self.coordinator.begin_mutation()?;
        DbWriter::commit_batch_with_sync(&self.raw, batch, sync)
    }

    fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()> {
        let _mutation = self.coordinator.begin_mutation()?;
        DbWriter::put(&self.raw, col, key, value)
    }

    fn delete(&self, col: Column, key: &[u8]) -> Result<()> {
        let _mutation = self.coordinator.begin_mutation()?;
        DbWriter::delete(&self.raw, col, key)
    }
}

/// Retains the common owner before the own-vote serialization mutex.
pub struct OwnVerifiedVotesGuard<'a> {
    _guard: MutexGuard<'a, ()>,
    _owner: crate::coordination::StorageObservationGuard<'a>,
}

/// Read-only point reads from one database sequence.
///
/// All columns use the same RocksDB snapshot. Later writes through the same
/// storage handle do not change its values. Reads copy their result, preserve
/// missing and empty values, and return column or database errors.
///
/// The snapshot cannot outlive its storage owner.
///
/// ```compile_fail
/// use rustaxa_storage::{Storage, StorageReadSnapshot};
/// fn escape(storage: Storage) -> StorageReadSnapshot<'static> {
///     storage.read_snapshot()
/// }
/// ```
///
/// The snapshot does not expose write operations.
///
/// ```compile_fail
/// use rustaxa_storage::{Column, StorageReadSnapshot};
/// fn write(snapshot: &StorageReadSnapshot<'_>) {
///     snapshot.put(Column::Status, b"key", b"value");
/// }
/// ```
///
/// Its database handle is private.
///
/// ```compile_fail
/// use rustaxa_storage::StorageReadSnapshot;
/// fn database(snapshot: &StorageReadSnapshot<'_>) {
///     let _ = snapshot.db;
/// }
/// ```
pub struct StorageReadSnapshot<'a> {
    snapshot: SnapshotWithThreadMode<'a, DBWithThreadMode<MultiThreaded>>,
    db: &'a DBWithThreadMode<MultiThreaded>,
}

impl StorageReadSnapshot<'_> {
    /// Returns the owned value at this snapshot's sequence, or a read error.
    pub fn get(&self, column: Column, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let handle = self.db.cf_handle(column.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", column.name()))
        })?;
        self.snapshot
            .get_cf(&handle, key)
            .map_err(|error| StorageError::Database(error).into())
    }

    /// Reads the exact rule schedule at this sequence without enrollment.
    /// Missing and empty values stay distinct. Read errors propagate.
    pub fn pbft_execution_schedule_raw(&self) -> Result<Option<Vec<u8>>> {
        self.get(Column::Status, crate::metadata::PBFT_EXECUTION_SCHEDULE_KEY)
    }

    /// Reads the opaque pending-publication marker at this snapshot's sequence.
    /// Missing and empty values remain distinct. Read errors propagate.
    pub fn external_evm_pending_publication_raw(&self) -> Result<Option<Vec<u8>>> {
        self.get(
            Column::FinalChainMeta,
            crate::final_chain::EXTERNAL_EVM_PENDING_PUBLICATION_KEY,
        )
    }

    /// Reads the exact native snapshot at this sequence. Missing and empty
    /// values remain distinct, and backend errors propagate.
    pub fn dpos_snapshot_raw(&self, number: u64) -> Result<Option<Vec<u8>>> {
        self.get(
            Column::FinalChainMeta,
            &FinalChainRepository::<DBWithThreadMode<MultiThreaded>>::dpos_snapshot_key(number),
        )
    }

    /// Reads the recorded application pairing at this sequence without
    /// initializing it. Missing and empty values remain distinct.
    pub fn concrete_state_pairing_raw(&self) -> Result<Option<Vec<u8>>> {
        self.get(
            Column::FinalChainMeta,
            FinalChainRepository::<DBWithThreadMode<MultiThreaded>>::CONCRETE_STATE_PAIRING_KEY,
        )
    }
}

pub struct Storage {
    db: Arc<CoordinatedDatabase>,
    // Individual repositories splitting query/apply into domains.
    dag: DagRepository<CoordinatedDatabase>,
    pbft: PbftRepository<CoordinatedDatabase>,
    pillar: PillarRepository<CoordinatedDatabase>,
    period: PeriodRepository<CoordinatedDatabase>,
    transaction: TransactionRepository<CoordinatedDatabase>,
    metadata: MetadataRepository<CoordinatedDatabase>,
    final_chain: FinalChainRepository<CoordinatedDatabase>,
    own_verified_votes_lock: Mutex<()>,
    extra_reward_votes_lock: Mutex<()>,
    light_history_prune_lock: Mutex<()>,
    extra_reward_votes_reset_generation: AtomicU64,
}

impl Storage {
    #[cfg(feature = "production_integration_test")]
    pub fn integration_database_image(
        &self,
    ) -> Result<crate::integration_inspection::DatabaseImage> {
        let db = &self.db.raw;
        crate::integration_inspection::capture(db.path(), |name, remaining, maximum_rows| {
            let cf = db
                .cf_handle(name)
                .ok_or_else(|| anyhow::anyhow!("integration column missing"))?;
            crate::integration_inspection::collect_rows(
                db.iterator_cf(&cf, rocksdb::IteratorMode::Start),
                remaining,
                maximum_rows,
            )
        })
    }
    /// Shared mutation boundary for this storage owner. It grants no vote authority.
    pub fn mutation_coordinator(&self) -> &crate::StorageCoordinator {
        &self.db.coordinator
    }

    /// Captures one read sequence from this storage handle without blocking writes.
    /// The returned snapshot borrows this owner and exposes only point reads.
    pub fn read_snapshot(&self) -> StorageReadSnapshot<'_> {
        StorageReadSnapshot {
            snapshot: self.db.raw.snapshot(),
            db: &self.db.raw,
        }
    }

    pub fn new(config: Config) -> Result<Self> {
        std::fs::create_dir_all(&config.db_path).map_err(StorageError::Io)?;
        Self::open_database(config)
    }

    /// Opens an existing database with database and column creation disabled.
    pub fn open_existing(mut config: Config) -> Result<Self> {
        let metadata = std::fs::metadata(&config.db_path).map_err(StorageError::Io)?;
        if !metadata.is_dir() {
            return Err(StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "existing database path is not a directory",
            ))
            .into());
        }
        config.create_if_missing = false;
        config.create_missing_column_families = false;
        Self::open_database(config)
    }

    fn open_database(config: Config) -> Result<Self> {
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

        let db = Arc::new(CoordinatedDatabase::new(db));
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
            light_history_prune_lock: Mutex::new(()),
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
    pub fn lock_own_verified_votes(&self) -> Result<OwnVerifiedVotesGuard<'_>> {
        let owner = self.mutation_coordinator().observe();
        let guard = self.own_verified_votes_lock.lock().map_err(|_| {
            StorageError::Read("own verified votes serialization lock poisoned".into())
        })?;
        Ok(OwnVerifiedVotesGuard {
            _guard: guard,
            _owner: owner,
        })
    }

    /// Acquires the process-local serialization guard for extra reward votes.
    ///
    /// Production admission writes, queries, and finalization/reset executors
    /// hold this guard across their complete operation. Handles sharing an
    /// `Arc<Storage>` therefore cannot insert a stale reward vote between reset
    /// enumeration and commit. Lock poisoning is returned as a storage error.
    pub fn lock_extra_reward_votes(&self) -> Result<ExtraRewardVotesGuard<'_>> {
        let owner = self.mutation_coordinator().observe();
        let guard = self.extra_reward_votes_lock.lock().map_err(|_| {
            StorageError::Read("extra reward votes serialization lock poisoned".into())
        })?;
        Ok(ExtraRewardVotesGuard {
            _guard: guard,
            _owner: owner,
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

    pub fn dag(&self) -> &DagRepository<CoordinatedDatabase> {
        &self.dag
    }

    pub fn period(&self) -> &PeriodRepository<CoordinatedDatabase> {
        &self.period
    }

    pub fn metadata(&self) -> &MetadataRepository<CoordinatedDatabase> {
        &self.metadata
    }

    pub fn pbft(&self) -> &PbftRepository<CoordinatedDatabase> {
        &self.pbft
    }

    pub fn pillar(&self) -> &PillarRepository<CoordinatedDatabase> {
        &self.pillar
    }

    pub fn transaction(&self) -> &TransactionRepository<CoordinatedDatabase> {
        &self.transaction
    }

    pub fn final_chain(&self) -> &FinalChainRepository<CoordinatedDatabase> {
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

    /// Appends one RocksDB range tombstone to a caller-owned atomic batch.
    pub fn batch_delete_range_raw(
        &self,
        batch: &mut StorageWriteBatch,
        col: Column,
        from: &[u8],
        to: &[u8],
    ) -> Result<()> {
        let handle = self.db.raw.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        batch.delete_range_cf(&handle, from, to);
        Ok(())
    }

    pub fn commit_write_batch_with_sync(&self, batch: StorageWriteBatch, sync: bool) -> Result<()> {
        DbWriter::commit_batch_with_sync(self.db.as_ref(), batch, sync)
    }

    pub(crate) fn commit_owned_effect_batch(
        &self,
        scope: &mut crate::OwnedEffectScope<'_>,
        batch: StorageWriteBatch,
        sync: bool,
    ) -> Result<()> {
        scope.validate_for(self.mutation_coordinator())?;
        scope.note_write_attempt();
        let result = DbWriter::commit_batch_with_sync(&self.db.raw, batch, sync);
        if result.is_err() {
            scope.poison();
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn drop_column_for_application_scope_test(&self, column: Column) -> Result<()> {
        self.db
            .raw
            .drop_cf(column.name())
            .map_err(|error| anyhow::Error::from(error))
    }

    /// Atomically removes finalized light-node history below explicit period and DAG-level cutoffs.
    ///
    /// The operation owns the complete RocksDB batch, including bulk period rows and their transaction,
    /// DAG, PBFT, and receipt reverse indexes. Level zero is a valid no-DAG-deletion cutoff. A retry with
    /// the same cutoffs is idempotent and reports `changed = false` after the first successful commit.
    pub fn prune_light_history(
        &self,
        request: LightHistoryPruneRequest,
    ) -> Result<LightHistoryPruneReport> {
        let _mutation = self.mutation_coordinator().begin_mutation()?;
        let _guard = self
            .light_history_prune_lock
            .lock()
            .map_err(|_| StorageError::Read("LIGHT_HISTORY_PRUNE_LOCK_POISONED".into()))?;
        let Some(first_period_entry) = self.iter(Column::PeriodData).next() else {
            return Ok(LightHistoryPruneReport {
                changed: false,
                end_period_exclusive: request.end_period_exclusive,
                first_retained_dag_level: request.first_retained_dag_level,
                rebuilt_secondary_indexes: false,
            });
        };
        let (first_period_key, _) = first_period_entry?;
        let first_period =
            u64::from_le_bytes(first_period_key.as_ref().try_into().map_err(|_| {
                StorageError::Read("LIGHT_HISTORY_PRUNE_INVALID_PERIOD_DATA_KEY".into())
            })?);
        if first_period >= request.end_period_exclusive {
            return Ok(LightHistoryPruneReport {
                changed: false,
                end_period_exclusive: request.end_period_exclusive,
                first_retained_dag_level: request.first_retained_dag_level,
                rebuilt_secondary_indexes: false,
            });
        }
        let rebuild_secondary_indexes = !request.live_cleanup
            && request.end_period_exclusive.saturating_sub(first_period)
                > request.non_block_periods_to_keep.saturating_mul(2);
        let secondary_cutoff = if rebuild_secondary_indexes {
            request
                .end_period_exclusive
                .saturating_sub(request.non_block_periods_to_keep)
        } else {
            request.end_period_exclusive
        };

        let mut batch = self.create_write_batch();
        let changed = true;
        let decode_u64 = |bytes: &[u8], label: &str| -> Result<u64> {
            let bytes: [u8; 8] = bytes.try_into().map_err(|_| {
                StorageError::Read(format!("LIGHT_HISTORY_PRUNE_INVALID_{label}_KEY"))
            })?;
            Ok(u64::from_le_bytes(bytes))
        };

        if !rebuild_secondary_indexes {
            // Live cleanup and short offline cuts read only the deleted PeriodData
            // prefix and derive exact reverse-index keys from its canonical bytes.
            for entry in self.iter(Column::PeriodData) {
                let (period_key, period_data) = entry?;
                let period = decode_u64(&period_key, Column::PeriodData.name())?;
                if period >= request.end_period_exclusive {
                    break;
                }
                let keys = finalized_period_index_keys(&period_data)?;
                self.batch_delete_raw(&mut batch, Column::PbftBlockPeriod, &keys.pbft_block)?;
                for hash in keys.dag_blocks {
                    self.batch_delete_raw(&mut batch, Column::DagBlockPeriod, &hash)?;
                }
                for hash in keys.transactions {
                    self.batch_delete_raw(&mut batch, Column::TrxPeriod, &hash)?;
                    self.batch_delete_raw(&mut batch, Column::FinalChainReceiptByTrxHash, &hash)?;
                }
            }
        }

        for column in [
            Column::PeriodData,
            Column::PillarBlock,
            Column::FinalChainReceiptByPeriod,
            Column::PeriodLambda,
        ] {
            self.batch_delete_range_raw(
                &mut batch,
                column,
                &first_period.to_le_bytes(),
                &request.end_period_exclusive.to_le_bytes(),
            )?;
        }
        if request.first_retained_dag_level > 0 {
            self.batch_delete_range_raw(
                &mut batch,
                Column::DagBlocksLevel,
                &0_u64.to_le_bytes(),
                &request.first_retained_dag_level.to_le_bytes(),
            )?;
        }

        let mut retained_transaction_hashes = std::collections::HashSet::new();
        if rebuild_secondary_indexes {
            for entry in self.iter(Column::PbftBlockPeriod) {
                let (key, value) = entry?;
                if decode_u64(&value, Column::PbftBlockPeriod.name())? < secondary_cutoff {
                    self.batch_delete_raw(&mut batch, Column::PbftBlockPeriod, &key)?;
                }
            }
            for column in [Column::TrxPeriod, Column::DagBlockPeriod] {
                for entry in self.iter(column) {
                    let (key, value) = entry?;
                    let period: u64 = rlp::Rlp::new(&value).val_at(0).map_err(|error| {
                        StorageError::Read(format!(
                            "LIGHT_HISTORY_PRUNE_INVALID_{}_VALUE: {error}",
                            column.name()
                        ))
                    })?;
                    if period < secondary_cutoff {
                        self.batch_delete_raw(&mut batch, column, &key)?;
                    } else if column == Column::TrxPeriod {
                        retained_transaction_hashes.insert(key.to_vec());
                    }
                }
            }
            for entry in self.iter(Column::FinalChainReceiptByTrxHash) {
                let (key, _) = entry?;
                if !retained_transaction_hashes.contains(key.as_ref()) {
                    self.batch_delete_raw(&mut batch, Column::FinalChainReceiptByTrxHash, &key)?;
                }
            }
        }

        self.commit_write_batch_with_sync(batch, false)?;
        Ok(LightHistoryPruneReport {
            changed,
            end_period_exclusive: request.end_period_exclusive,
            first_retained_dag_level: request.first_retained_dag_level,
            rebuilt_secondary_indexes: rebuild_secondary_indexes,
        })
    }
}

#[cfg(test)]
mod native_predecessor_snapshot_tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct RetainedRoot(PathBuf);

    impl Drop for RetainedRoot {
        fn drop(&mut self) {
            crate::closeout_test_scratch::preserve(&self.0);
        }
    }

    fn root() -> (RetainedRoot, Config) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = crate::closeout_test_scratch::new_path(
            std::env::temp_dir().join(format!("native_snapshot_{}_{nonce}", std::process::id())),
            true,
        );
        let mut config = Config::new(path.clone());
        config.db_write_buffer_size = 2 * 1024 * 1024;
        config.max_total_wal_size = 32 * 1024 * 1024;
        config.max_open_files = 32;
        (RetainedRoot(path), config)
    }

    #[test]
    fn pbft_execution_schedule_storage_preserves_namespace_snapshots_and_reopen() {
        fn contents(
            storage: &Storage,
            columns: &[Column],
        ) -> std::collections::BTreeMap<String, Vec<(Vec<u8>, Vec<u8>)>> {
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
                    (column.name().to_string(), rows)
                })
                .collect()
        }
        let (_root, config) = root();
        let columns = config.column_families.clone();
        let storage = Storage::new(config.clone()).unwrap();
        let mut batch = storage.create_write_batch();
        for field in 0..=u8::MAX {
            storage
                .metadata()
                .write_status_field_in_batch(&mut batch, field, u64::from(field))
                .unwrap();
        }
        storage.commit_write_batch_with_sync(batch, true).unwrap();
        let original = contents(&storage, &columns);
        assert_eq!(
            storage.metadata().pbft_execution_schedule_raw().unwrap(),
            None
        );
        let mut dropped = storage.create_write_batch();
        storage
            .metadata()
            .write_pbft_execution_schedule_in_batch(&mut dropped, &[0xc0])
            .unwrap();
        drop(dropped);
        assert_eq!(contents(&storage, &columns), original);
        let missing = storage.read_snapshot();
        let mut batch = storage.create_write_batch();
        storage
            .metadata()
            .write_pbft_execution_schedule_in_batch(&mut batch, &[])
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
        let empty = storage.read_snapshot();
        let record = vec![0xc1, 1];
        let mut batch = storage.create_write_batch();
        storage
            .metadata()
            .write_pbft_execution_schedule_in_batch(&mut batch, &record)
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
        assert_eq!(missing.pbft_execution_schedule_raw().unwrap(), None);
        assert_eq!(empty.pbft_execution_schedule_raw().unwrap(), Some(vec![]));
        assert_eq!(
            storage
                .read_snapshot()
                .pbft_execution_schedule_raw()
                .unwrap(),
            Some(record.clone())
        );
        let mut expected = original;
        let status = expected.get_mut(Column::Status.name()).unwrap();
        status.push((
            crate::metadata::PBFT_EXECUTION_SCHEDULE_KEY.to_vec(),
            record.clone(),
        ));
        status.sort();
        assert_eq!(contents(&storage, &columns), expected);
        drop(empty);
        drop(missing);
        drop(storage);
        let reopened = Storage::open_existing(config).unwrap();
        assert_eq!(
            reopened.metadata().pbft_execution_schedule_raw().unwrap(),
            Some(record)
        );
        assert_eq!(contents(&reopened, &columns), expected);
    }

    #[test]
    fn pbft_execution_schedule_snapshot_propagates_missing_column() {
        let (_root, mut config) = root();
        config
            .column_families
            .retain(|column| *column != Column::Status);
        let storage = Storage::new(config).unwrap();
        let error = storage
            .read_snapshot()
            .pbft_execution_schedule_raw()
            .unwrap_err();
        assert!(error.downcast_ref::<StorageError>().is_some());
        assert!(error.to_string().contains("Missing column family"));
    }

    #[test]
    fn native_predecessor_snapshot_raw_methods_preserve_values() {
        let (_root, config) = root();
        let storage = Storage::new(config).unwrap();
        let number = 0x0102_0304_0506_0708_u64;
        let native_key = [b"rustaxa:dpos_snapshot:".as_slice(), &number.to_le_bytes()].concat();
        let pairing_key = b"rustaxa:concrete_state_pairing:v1";
        let missing = storage.read_snapshot();
        assert_eq!(missing.dpos_snapshot_raw(number).unwrap(), None);
        assert_eq!(missing.concrete_state_pairing_raw().unwrap(), None);
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, Column::FinalChainMeta, &native_key, &[])
            .unwrap();
        storage
            .batch_put_raw(&mut batch, Column::FinalChainMeta, pairing_key, &[])
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
        let empty = storage.read_snapshot();
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, Column::FinalChainMeta, &native_key, &[1, 2, 3])
            .unwrap();
        storage
            .batch_put_raw(&mut batch, Column::FinalChainMeta, pairing_key, &[4, 5, 6])
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
        assert_eq!(missing.dpos_snapshot_raw(number).unwrap(), None);
        assert_eq!(missing.concrete_state_pairing_raw().unwrap(), None);
        assert_eq!(empty.dpos_snapshot_raw(number).unwrap(), Some(vec![]));
        assert_eq!(empty.concrete_state_pairing_raw().unwrap(), Some(vec![]));
        let fresh = storage.read_snapshot();
        assert_eq!(
            fresh.dpos_snapshot_raw(number).unwrap(),
            Some(vec![1, 2, 3])
        );
        assert_eq!(
            fresh.concrete_state_pairing_raw().unwrap(),
            Some(vec![4, 5, 6])
        );
        assert_eq!(
            storage.final_chain().dpos_snapshot_raw(number).unwrap(),
            fresh.dpos_snapshot_raw(number).unwrap()
        );
        assert_eq!(
            storage.final_chain().concrete_state_pairing_raw().unwrap(),
            fresh.concrete_state_pairing_raw().unwrap()
        );
    }

    #[test]
    fn native_predecessor_snapshot_raw_methods_propagate_backend_errors() {
        let (_root, mut config) = root();
        config
            .column_families
            .retain(|column| *column != Column::FinalChainMeta);
        let storage = Storage::new(config).unwrap();
        let snapshot = storage.read_snapshot();
        for result in [
            snapshot.dpos_snapshot_raw(1),
            snapshot.concrete_state_pairing_raw(),
        ] {
            let error = result.unwrap_err();
            assert!(error.downcast_ref::<StorageError>().is_some());
            assert!(error.to_string().contains("Missing column family"));
        }
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

    fn commit_batch_with_sync(&self, batch: Self::Batch, sync: bool) -> Result<()> {
        DbWriter::commit_batch_with_sync(&*self.db, batch, sync)
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
        if std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST").is_some() {
            crate::closeout_test_scratch::register(&path);
        } else {
            let _ = fs::remove_dir_all(&path);
        }
        let storage = Storage::new(Config::new(path.clone())).expect("storage should initialize");
        crate::closeout_test_scratch::created(&path);
        (path, storage)
    }

    fn u64_le(value: u64) -> [u8; 8] {
        value.to_le_bytes()
    }

    fn period_lookup(period: u64) -> Vec<u8> {
        let mut stream = rlp::RlpStream::new_list(2);
        stream.append(&period);
        stream.append(&0_u64);
        stream.out().to_vec()
    }

    /// Encodes the canonical pre-Ficus four-field layout. The optional fifth
    /// pillar-vote field is deliberately absent to cover historical pruning.
    fn period_data_with_transactions(transactions: &[&[u8]]) -> Vec<u8> {
        let mut transaction_list = rlp::RlpStream::new_list(transactions.len());
        for transaction in transactions {
            transaction_list.append_raw(transaction, 1);
        }
        let mut empty_dag_bundle = rlp::RlpStream::new_list(3);
        empty_dag_bundle.begin_list(0);
        empty_dag_bundle.begin_list(0);
        empty_dag_bundle.begin_list(0);
        let mut period_data = rlp::RlpStream::new_list(4);
        period_data.append_raw(&[0xc0], 1);
        period_data.append_raw(&[0xc0], 1);
        period_data.append_raw(&empty_dag_bundle.out(), 1);
        period_data.append_raw(&transaction_list.out(), 1);
        period_data.out().to_vec()
    }

    #[test]
    fn storage_read_snapshot_keeps_cross_column_batch() {
        let (path, storage) = storage_at("snapshot_cross_column_batch");
        let columns = [Column::Status, Column::FinalChainMeta];
        let key = b"snapshot_sequence";
        let mut batch = storage.create_write_batch();
        for column in columns {
            storage
                .batch_put_raw(&mut batch, column, key, b"old")
                .unwrap();
        }
        storage.commit_write_batch_with_sync(batch, false).unwrap();
        let old = storage.read_snapshot();
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let mut batch = storage.create_write_batch();
                    for column in columns {
                        storage
                            .batch_put_raw(&mut batch, column, key, b"new")
                            .unwrap();
                    }
                    storage.commit_write_batch_with_sync(batch, false).unwrap();
                })
                .join()
                .unwrap();
        });
        let fresh = storage.read_snapshot();
        for column in columns {
            assert_eq!(old.get(column, key).unwrap(), Some(b"old".to_vec()));
            assert_eq!(fresh.get(column, key).unwrap(), Some(b"new".to_vec()));
        }
        drop(fresh);
        drop(old);
        drop(storage);
        if !crate::closeout_test_scratch::preserve(&path) {
            let _ = fs::remove_dir_all(path);
        }
    }

    #[test]
    fn storage_read_snapshot_distinguishes_missing_empty_and_error() {
        let (path, storage) = storage_at("snapshot_read_results");
        let key = crate::final_chain::EXTERNAL_EVM_PENDING_PUBLICATION_KEY;
        let missing = storage.read_snapshot();
        assert_eq!(missing.get(Column::FinalChainMeta, key).unwrap(), None);
        assert_eq!(
            missing.external_evm_pending_publication_raw().unwrap(),
            None
        );

        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, Column::FinalChainMeta, key, b"")
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
        let empty = storage.read_snapshot();
        assert_eq!(
            empty.get(Column::FinalChainMeta, key).unwrap(),
            Some(Vec::new())
        );
        assert_eq!(
            empty.external_evm_pending_publication_raw().unwrap(),
            Some(Vec::new())
        );

        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, Column::FinalChainMeta, key, b"pending")
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
        let present = storage.read_snapshot();
        assert_eq!(
            missing.external_evm_pending_publication_raw().unwrap(),
            None
        );
        assert_eq!(
            empty.external_evm_pending_publication_raw().unwrap(),
            Some(Vec::new())
        );
        assert_eq!(
            present.get(Column::FinalChainMeta, key).unwrap(),
            Some(b"pending".to_vec())
        );
        assert_eq!(
            present.external_evm_pending_publication_raw().unwrap(),
            Some(b"pending".to_vec())
        );

        storage
            .db
            .raw
            .drop_cf(Column::FinalChainMeta.name())
            .unwrap();
        assert!(present.get(Column::FinalChainMeta, key).is_err());
        assert!(present.external_evm_pending_publication_raw().is_err());
        drop(present);
        drop(empty);
        drop(missing);
        drop(storage);
        if !crate::closeout_test_scratch::preserve(&path) {
            let _ = fs::remove_dir_all(path);
        }
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
        if !crate::closeout_test_scratch::preserve(&path) {
            let _ = fs::remove_dir_all(path);
        }
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
        if !crate::closeout_test_scratch::preserve(&path) {
            let _ = fs::remove_dir_all(path);
        }
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
        if !crate::closeout_test_scratch::preserve(&path) {
            let _ = fs::remove_dir_all(path);
        }
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
        if !crate::closeout_test_scratch::preserve(&path) {
            let _ = fs::remove_dir_all(path);
        }
    }

    #[test]
    fn light_history_prune_is_atomic_idempotent_and_survives_reopen() {
        let (path, storage) = storage_at("light_history_prune");
        let old_pbft_hash = keccak256(&[0xc0]);
        let old_transaction_rlp = [0x01];
        let old_transaction_hash = keccak256(&old_transaction_rlp);
        let retained_hash = [2_u8; 32];
        let malformed_retained_hash = [3_u8; 32];
        let mut batch = storage.create_write_batch();
        for column in [
            Column::PeriodData,
            Column::PillarBlock,
            Column::FinalChainReceiptByPeriod,
            Column::PeriodLambda,
        ] {
            let old_value = if column == Column::PeriodData {
                period_data_with_transactions(&[&old_transaction_rlp])
            } else {
                b"old".to_vec()
            };
            storage
                .batch_put_raw(&mut batch, column, &u64_le(4), &old_value)
                .unwrap();
            storage
                .batch_put_raw(&mut batch, column, &u64_le(5), b"retained")
                .unwrap();
        }
        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainReceiptByTrxHash,
                &old_transaction_hash,
                b"old receipt",
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainReceiptByTrxHash,
                &retained_hash,
                b"retained receipt",
            )
            .unwrap();
        storage
            .batch_put_raw(&mut batch, Column::DagBlocksLevel, &u64_le(6), b"old")
            .unwrap();
        storage
            .batch_put_raw(&mut batch, Column::DagBlocksLevel, &u64_le(7), b"retained")
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::PbftBlockPeriod,
                &old_pbft_hash,
                &u64_le(4),
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::PbftBlockPeriod,
                &retained_hash,
                &u64_le(5),
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::TrxPeriod,
                &old_transaction_hash,
                &period_lookup(4),
            )
            .unwrap();
        for column in [Column::TrxPeriod, Column::DagBlockPeriod] {
            storage
                .batch_put_raw(&mut batch, column, &retained_hash, &period_lookup(5))
                .unwrap();
        }
        // A malformed retained reverse index proves live cleanup does not scan
        // unrelated retained rows while deriving the expired exact keys.
        storage
            .batch_put_raw(
                &mut batch,
                Column::TrxPeriod,
                &malformed_retained_hash,
                b"not rlp",
            )
            .unwrap();
        storage
            .commit_write_batch_with_sync(batch, true)
            .expect("fixture batch should commit");

        let request = LightHistoryPruneRequest {
            end_period_exclusive: 5,
            first_retained_dag_level: 7,
            non_block_periods_to_keep: 2,
            live_cleanup: true,
        };
        let first = storage
            .prune_light_history(request)
            .expect("prune should succeed");
        assert!(first.changed);
        assert!(!first.rebuilt_secondary_indexes);
        let second = storage
            .prune_light_history(request)
            .expect("retry should succeed");
        assert!(!second.changed);

        drop(storage);
        let reopened = Storage::new(Config::new(path.clone())).expect("storage should reopen");
        for column in [
            Column::PeriodData,
            Column::PillarBlock,
            Column::FinalChainReceiptByPeriod,
            Column::PeriodLambda,
        ] {
            assert_eq!(reopened.get_raw(column, &u64_le(4)).unwrap(), None);
            assert_eq!(
                reopened.get_raw(column, &u64_le(5)).unwrap(),
                Some(b"retained".to_vec())
            );
        }
        assert_eq!(
            reopened
                .get_raw(Column::DagBlocksLevel, &u64_le(6))
                .unwrap(),
            None
        );
        assert_eq!(
            reopened
                .get_raw(Column::DagBlocksLevel, &u64_le(7))
                .unwrap(),
            Some(b"retained".to_vec())
        );
        assert_eq!(
            reopened
                .get_raw(Column::PbftBlockPeriod, &old_pbft_hash)
                .unwrap(),
            None
        );
        assert_eq!(
            reopened
                .get_raw(Column::TrxPeriod, &old_transaction_hash)
                .unwrap(),
            None
        );
        assert_eq!(
            reopened
                .get_raw(Column::FinalChainReceiptByTrxHash, &old_transaction_hash)
                .unwrap(),
            None
        );
        for column in [Column::TrxPeriod, Column::DagBlockPeriod] {
            assert!(reopened.get_raw(column, &retained_hash).unwrap().is_some());
        }
        assert_eq!(
            reopened
                .get_raw(Column::TrxPeriod, &malformed_retained_hash)
                .unwrap(),
            Some(b"not rlp".to_vec())
        );
        drop(reopened);
        if !crate::closeout_test_scratch::preserve(&path) {
            let _ = fs::remove_dir_all(path);
        }
    }

    #[test]
    fn malformed_light_history_index_does_not_commit_partial_prune() {
        let (path, storage) = storage_at("invalid_light_history_prune");
        storage
            .put(Column::PeriodData, &u64_le(1), b"malformed period data")
            .unwrap();

        let result = storage.prune_light_history(LightHistoryPruneRequest {
            end_period_exclusive: 2,
            first_retained_dag_level: 1,
            non_block_periods_to_keep: 0,
            live_cleanup: true,
        });

        assert!(result.is_err());
        assert_eq!(
            storage.get_raw(Column::PeriodData, &u64_le(1)).unwrap(),
            Some(b"malformed period data".to_vec())
        );
        drop(storage);
        if !crate::closeout_test_scratch::preserve(&path) {
            let _ = fs::remove_dir_all(path);
        }
    }

    #[test]
    fn offline_light_history_prune_rebuilds_receipt_index_from_retained_transactions() {
        let (path, storage) = storage_at("offline_light_history_prune");
        let old_hash = [1_u8; 32];
        let retained_hash = [2_u8; 32];
        let orphan_receipt_hash = [3_u8; 32];
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, Column::PeriodData, &u64_le(1), b"old")
            .unwrap();
        storage
            .batch_put_raw(&mut batch, Column::PeriodData, &u64_le(10), b"retained")
            .unwrap();
        storage
            .batch_put_raw(&mut batch, Column::TrxPeriod, &old_hash, &period_lookup(1))
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::TrxPeriod,
                &retained_hash,
                &period_lookup(9),
            )
            .unwrap();
        for hash in [old_hash, retained_hash, orphan_receipt_hash] {
            storage
                .batch_put_raw(
                    &mut batch,
                    Column::FinalChainReceiptByTrxHash,
                    &hash,
                    b"receipt",
                )
                .unwrap();
        }
        storage
            .commit_write_batch_with_sync(batch, true)
            .expect("fixture batch should commit");

        let report = storage
            .prune_light_history(LightHistoryPruneRequest {
                end_period_exclusive: 10,
                first_retained_dag_level: 0,
                non_block_periods_to_keep: 2,
                live_cleanup: false,
            })
            .expect("offline prune should succeed");

        assert!(report.changed);
        assert!(report.rebuilt_secondary_indexes);
        assert_eq!(
            storage
                .get_raw(Column::FinalChainReceiptByTrxHash, &old_hash)
                .unwrap(),
            None
        );
        assert_eq!(
            storage
                .get_raw(Column::FinalChainReceiptByTrxHash, &orphan_receipt_hash)
                .unwrap(),
            None
        );
        assert!(
            storage
                .get_raw(Column::FinalChainReceiptByTrxHash, &retained_hash)
                .unwrap()
                .is_some()
        );
        assert!(
            !storage
                .prune_light_history(LightHistoryPruneRequest {
                    end_period_exclusive: 10,
                    first_retained_dag_level: 0,
                    non_block_periods_to_keep: 2,
                    live_cleanup: false,
                })
                .unwrap()
                .changed
        );
        drop(storage);
        if !crate::closeout_test_scratch::preserve(&path) {
            let _ = fs::remove_dir_all(path);
        }
    }
}
