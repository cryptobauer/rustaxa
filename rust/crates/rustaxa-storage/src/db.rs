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
use ethereum_types::H256;
use rocksdb::{DBPinnableSlice, DBWithThreadMode, MultiThreaded, Options};
use std::sync::Arc;

use crate::Column;
use crate::Config;
use crate::DagRepository;
use crate::StorageError;

/// Item returned by the database iterator.
/// Key and Value are boxed slices.
pub type IteratorItem = Result<(Box<[u8]>, Box<[u8]>)>;
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
    fn iter<'a>(&'a self, col: Column) -> DbIterator<'a>;
    fn iter_rev<'a>(&'a self, col: Column) -> DbIterator<'a>;
}

impl DbReader for DBWithThreadMode<MultiThreaded> {
    type Slice<'a> = DBPinnableSlice<'a>;

    fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        self.get_pinned_cf(&handle, key)
            .map_err(|e| StorageError::Database(e).into())
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

pub struct Storage {
    #[allow(dead_code)]
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    dag: DagRepository<DBWithThreadMode<MultiThreaded>>,
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

        let descriptors = config
            .column_families
            .iter()
            .map(|col| col.descriptor(&opts))
            .collect::<Vec<_>>();

        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &opts,
            &config.db_path,
            descriptors,
        )
        .map_err(StorageError::Database)?;

        let db = Arc::new(db);
        let dag = DagRepository::new(db.clone());

        Ok(Storage { db, dag })
    }

    pub fn dag(&self) -> &DagRepository<DBWithThreadMode<MultiThreaded>> {
        &self.dag
    }

    pub fn genesis_hash(&self) -> Result<Option<H256>> {
        Ok(self
            .get(Column::Genesis, &0i32.to_le_bytes())?
            .map(|val| H256::from_slice(val.as_ref())))
    }
}

impl DbReader for Storage {
    type Slice<'a> = DBPinnableSlice<'a>;

    fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
        DbReader::get(&*self.db, col, key)
    }

    fn iter<'a>(&'a self, col: Column) -> DbIterator<'a> {
        DbReader::iter(&*self.db, col)
    }

    fn iter_rev<'a>(&'a self, col: Column) -> DbIterator<'a> {
        DbReader::iter_rev(&*self.db, col)
    }
}
