use rustaxa_storage::{Config, StatusField, Storage};
use std::path::PathBuf;

use anyhow::Result;

fn main() -> Result<()> {
    let path = PathBuf::from("/rocksdb/db");
    let config = Config::new(path);

    let storage = Storage::new(config)?;
    // let key = b"example_key";
    // println!("Reading key '{:?}'", String::from_utf8_lossy(key));

    match storage.get_status_field(StatusField::DagBlkCount) {
        Ok(val) => println!("Found value: {:?}", val),
        Err(e) => println!("Error reading key: {:?}", e),
    }

    Ok(())
}
