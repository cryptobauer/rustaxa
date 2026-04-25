use crate::ffi::BridgeFinalChain;
use crate::ffi::Storage;
use rustaxa_consensus::FinalChain;

pub fn create_final_chain(storage: &Storage) -> Result<Box<BridgeFinalChain>, anyhow::Error> {
    let final_chain = FinalChain::new(storage.0.clone())?;
    Ok(Box::new(BridgeFinalChain(final_chain)))
}

impl BridgeFinalChain {
    pub fn get_block_hash(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .block_hash(num)
            .map_err(|e| anyhow::anyhow!(e))?
            .unwrap_or_default())
    }
}
