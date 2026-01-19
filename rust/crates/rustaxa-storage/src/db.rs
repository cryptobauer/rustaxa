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
use rocksdb::{DBWithThreadMode, MultiThreaded, Options};
use std::sync::Arc;

use crate::Config;
use crate::DagRepository;
use crate::StorageError;

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
