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
use rocksdb::{DBPinnableSlice, DBWithThreadMode, MultiThreaded, Options};
use std::sync::Arc;

use crate::Column;
use crate::Config;
use crate::DagRepository;
use crate::StorageError;

/// Trait abstracting database read operations.
pub trait DbReader: Send + Sync {
    /// The specific slice type returned by the backend.
    /// For RocksDB, this is `DBPinnableSlice` (zero-copy).
    /// For Mocks, this can be `Vec<u8>`.
    type Slice<'a>: AsRef<[u8]>
    where
        Self: 'a;

    fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>>;
    fn get_last_key(&self, col: Column) -> Result<Option<Vec<u8>>>;
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

    fn get_last_key(&self, col: Column) -> Result<Option<Vec<u8>>> {
        let handle = self.cf_handle(col.name()).ok_or_else(|| {
            StorageError::Config(format!("Missing column family: {}", col.name()))
        })?;
        let mut iter = self.raw_iterator_cf(&handle);
        iter.seek_to_last();
        if let Some(key) = iter.key() {
            Ok(Some(key.to_vec()))
        } else {
            Ok(None)
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
}
