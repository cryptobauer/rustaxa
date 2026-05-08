use ethereum_types::{H160, H256};

/// Minimal deterministic PBFT block linkage data used by consensus state machines.
///
/// This type intentionally contains only the fields needed to validate PBFT-chain continuity and recover anchors. Full
/// PBFT block materialization remains at the C++ boundary until the block model itself is ported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PbftBlockLink {
    /// Canonical signed PBFT block hash.
    pub block_hash: H256,
    /// Previous PBFT block hash encoded in the signed block.
    pub prev_block_hash: H256,
    /// Pivot DAG block hash encoded in the signed block.
    pub pivot_dag_block_hash: H256,
    /// PBFT period encoded in the signed block.
    pub period: u64,
}

/// Minimal proposer metadata decoded from a signed PBFT block.
///
/// This preserves the subset of PBFT block fields currently needed by FinalChain and consensus bridge code without
/// coupling those consumers to the full C++ `PbftBlock` implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftBlockMetadata {
    pub author: H160,
    pub period: u64,
    pub timestamp: u64,
    pub extra_data: Vec<u8>,
}
