use anyhow::Result;
use ethereum_types::H256;
use rustaxa_storage::{Config, Storage};
use std::path::PathBuf;

fn main() -> Result<()> {
    let path = PathBuf::from("/rocksdb/db");
    let config = Config::new(path);

    let storage = Storage::new(config)?;

    let block_level = storage.dag().last_blocks_level()?;
    println!("Last block level: {}", block_level);

    let blocks = storage.dag().blocks_by_level(block_level)?;
    println!("Blocks at level {}: {:?}", block_level, blocks);

    let block = blocks.first().cloned().unwrap_or_else(H256::zero);
    match storage.dag().dag_block(block) {
        Ok(val) => println!("Found value: {:?}", val),
        Err(e) => println!("Error reading key: {:?}", e),
    }

    Ok(())
}
