use anyhow::Result;
use rustaxa_storage::Storage;
use std::sync::Arc;

const DB_META_LAST_NUMBER: u32 = 1;

pub struct FinalChain {
    storage: Arc<Storage>,
}

impl FinalChain {
    pub fn new(storage: Arc<Storage>) -> Result<Self> {
        Ok(FinalChain { storage })
    }

    pub fn last_block_number(&self) -> Result<u64, anyhow::Error> {
        let Some(raw) = self.storage.final_chain().meta_value(DB_META_LAST_NUMBER)? else {
            return Ok(0);
        };
        decode_u64_le(&raw, "final_chain_meta/LAST_NUMBER")
    }

    pub fn block_number(&self, hash: [u8; 32]) -> Result<Option<u64>, anyhow::Error> {
        let Some(raw) = self
            .storage
            .final_chain()
            .block_number_by_hash(ethereum_types::H256::from(hash))?
        else {
            return Ok(None);
        };
        Ok(Some(decode_u64_le(&raw, "final_chain_blk_number_by_hash")?))
    }

    pub fn block_hash(&self, num: u64) -> Result<Option<Vec<u8>>, anyhow::Error> {
        self.storage.final_chain().block_hash_by_number(num)
    }
}

fn decode_u64_le(raw: &[u8], field: &str) -> Result<u64, anyhow::Error> {
    if raw.len() != std::mem::size_of::<u64>() {
        anyhow::bail!(
            "invalid {field} value size: expected {}, got {}",
            std::mem::size_of::<u64>(),
            raw.len()
        );
    }

    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(raw);
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_storage::{Column, Config};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(test_name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rustaxa-consensus-final-chain-{test_name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn last_block_number_returns_zero_when_missing() {
        let path = temp_db_path("last-missing");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let final_chain = FinalChain::new(storage.clone()).unwrap();

        assert_eq!(final_chain.last_block_number().unwrap(), 0);

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn reads_batch_one_indexes() {
        let path = temp_db_path("batch-one-indexes");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let mut batch = storage.create_write_batch();
        let block_number = 42u64;
        let block_hash = [0xAB; 32];

        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainMeta,
                &DB_META_LAST_NUMBER.to_le_bytes(),
                &block_number.to_le_bytes(),
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainBlkHashByNumber,
                &block_number.to_le_bytes(),
                &block_hash,
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainBlkNumberByHash,
                &block_hash,
                &block_number.to_le_bytes(),
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        let final_chain = FinalChain::new(storage.clone()).unwrap();

        assert_eq!(final_chain.last_block_number().unwrap(), block_number);
        assert_eq!(
            final_chain.block_hash(block_number).unwrap(),
            Some(block_hash.to_vec())
        );
        assert_eq!(
            final_chain.block_number(block_hash).unwrap(),
            Some(block_number)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }
}
