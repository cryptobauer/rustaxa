use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("I/O error occurred: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(rocksdb::Error),

    #[error("Invalid configuration: {0}")]
    Config(String),

    #[error("Database read error: {0}")]
    ReadError(String),

    #[error("Unknown storage error")]
    Unknown,
}
