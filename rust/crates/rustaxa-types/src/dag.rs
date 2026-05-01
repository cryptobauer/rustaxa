use ethereum_types::H256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagBlock {
    pub pivot: H256,
    pub level: u64,
    pub timestamp: u64,
    pub vdf: Vec<u8>,
    pub tips: Vec<H256>,
    pub transactions: Vec<H256>,
    pub signature: [u8; 65],
    pub gas_estimation: u64,
}
