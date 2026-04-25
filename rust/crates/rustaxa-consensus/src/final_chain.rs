use anyhow::Result;
use rustaxa_storage::Storage;
use std::sync::Arc;

pub struct FinalChain {
    storage: Arc<Storage>,
}

impl FinalChain {
    pub fn new(storage: Arc<Storage>) -> Result<Self> {
        Ok(FinalChain { storage })
    }

    pub fn block_hash(&self, num: u64) -> Result<Option<Vec<u8>>, anyhow::Error> {
        self.storage.final_chain().block_hash_by_number(num)
    }
}
