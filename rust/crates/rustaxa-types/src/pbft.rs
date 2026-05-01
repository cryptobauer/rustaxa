use ethereum_types::H160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftBlockMetadata {
    pub author: H160,
    pub period: u64,
    pub timestamp: u64,
    pub extra_data: Vec<u8>,
}
