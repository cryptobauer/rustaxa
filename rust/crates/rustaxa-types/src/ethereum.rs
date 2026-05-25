use ethereum_types::H512;

/// Ethereum node identifier used to attribute peer-originated network data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(
    /// Raw 512-bit node id.
    pub H512,
);

impl Default for NodeId {
    fn default() -> Self {
        Self(H512::from([0u8; 64]))
    }
}
