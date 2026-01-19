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
    Read(String),

    #[error("Types error: {0}")]
    Types(#[from] rustaxa_types::TypesError),

    #[error("DAG error: {0}")]
    Dag(String),

    #[error("Unknown storage error")]
    Unknown,
}
