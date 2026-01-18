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
use rocksdb::{BoundColumnFamily, DBWithThreadMode, MultiThreaded, Options};
use std::sync::Arc;

use crate::StorageError;
use crate::{Column, Config, StatusField};

pub struct Storage {
    db: DBWithThreadMode<MultiThreaded>,
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

        Ok(Storage { db })
    }

    fn handle(&self, column: Column) -> Arc<BoundColumnFamily<'_>> {
        self.db
            .cf_handle(column.name())
            .expect("Missing column family") // We don't expect this to happen.
    }

    pub fn get_status_field(&self, field: StatusField) -> Result<u64> {
        let key = (field as u8).to_le_bytes();
        let handle = self.handle(Column::Status);
        match self.db.get_pinned_cf(&handle, key) {
            Ok(Some(value)) => {
                if value.len() >= 8 {
                    Ok(u64::from_le_bytes(value[0..8].try_into().unwrap()))
                } else {
                    Err(StorageError::ReadError(format!(
                        "StatusField value for {:?} is too short: {:?}",
                        field,
                        value.len()
                    ))
                    .into())
                }
            }
            Ok(None) => {
                Err(StorageError::ReadError(format!("Status field {:?} not found", field)).into())
            }
            Err(e) => Err(StorageError::Database(e).into()),
        }
    }
}
