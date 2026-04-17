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

use crate::Column;
use crate::Config;
use crate::DagRepository;
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
    #[allow(dead_code)]
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    // Individual repositories splitting query/apply into domains.
    dag: DagRepository<DBWithThreadMode<MultiThreaded>>,
    pbft: PbftRepository<DBWithThreadMode<MultiThreaded>>,
    pillar: PillarRepository<DBWithThreadMode<MultiThreaded>>,
    period: PeriodRepository<DBWithThreadMode<MultiThreaded>>,
    transaction: TransactionRepository<DBWithThreadMode<MultiThreaded>>,
    metadata: MetadataRepository<DBWithThreadMode<MultiThreaded>>,
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

        Ok(Storage {
            db,
            dag,
            metadata,
            period,
            pillar,
            pbft,
            transaction,
        })
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
