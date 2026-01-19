use rustaxa_storage::Config;
use rustaxa_storage::Storage as InnerStorage;
use std::path::PathBuf;

pub struct Storage(InnerStorage);

#[cxx::bridge(namespace = "rustaxa::storage")]
mod ffi {
    extern "Rust" {
        type Storage;
        fn create_storage(path: &str) -> Box<Storage>;
    }
}

pub fn create_storage(path: &str) -> Box<Storage> {
    let path_buf = PathBuf::from(path);
    let config = Config::new(path_buf);
    // TODO: better error handling?
    let storage = InnerStorage::new(config).expect("Failed to create storage");
    Box::new(Storage(storage))
}
