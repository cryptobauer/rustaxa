use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeFinalChain;
use crate::ffi::BridgeStorage;
use rustaxa_consensus::FinalChain;

pub fn create_final_chain(storage: &BridgeStorage) -> Result<Box<BridgeFinalChain>, anyhow::Error> {
    let final_chain = FinalChain::new(storage.0.clone())?;
    Ok(Box::new(BridgeFinalChain(final_chain)))
}

impl BridgeFinalChain {
    pub fn get_last_block_number(self: &BridgeFinalChain) -> Result<u64, anyhow::Error> {
        self.0.last_block_number()
    }

    pub fn get_block_number(
        self: &BridgeFinalChain,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::FinalChainBlockNumberLookup, anyhow::Error> {
        Ok(match self.0.block_number(*hash)? {
            Some(value) => rustaxa_ffi::FinalChainBlockNumberLookup { found: true, value },
            None => rustaxa_ffi::FinalChainBlockNumberLookup {
                found: false,
                value: 0,
            },
        })
    }

    pub fn get_block_hash(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .block_hash(num)
            .map_err(|e| anyhow::anyhow!(e))?
            .unwrap_or_default())
    }
}
