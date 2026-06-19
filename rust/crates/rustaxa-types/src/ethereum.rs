use ethereum_types::H512;

/// Ethereum node identifier used to attribute peer-originated network data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(
    /// Raw 512-bit node id.
    pub H512,
);

impl NodeId {
    pub fn new(bytes: [u8; 64]) -> Self {
        Self(H512::from(bytes))
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self(H512::from([0u8; 64]))
    }
}
