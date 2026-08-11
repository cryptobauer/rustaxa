//! PBFT-chain bridge adapters backed by the Rust consensus and storage implementations.
//!
//! `PbftChain`-facing queries and storage lookups are implemented on
//! [`BridgePbftService`]. Chain state is protected by its own read/write lock,
//! so public reads do not contend on the separately synchronized manager.

use crate::ffi::rustaxa_ffi::{BlockRlpLookup as FfiBlockRlpLookup, PbftChainHeadPayload};
use crate::ffi::BridgePbftService;
use ethereum_types::H256;
use rustaxa_consensus::pbft_chain::{PbftBlockStorageLookup, PbftChainHead};
impl BridgePbftService {
    /// Returns whether storage recovery initialized the default PBFT chain head.
    pub fn pbft_chain_initialized_default(&self) -> bool {
        self.0.pbft_chain_initialized_default()
    }

    /// Returns the current PBFT chain head payload for C++ JSON formatting and public accessors.
    pub fn pbft_chain_head(&self) -> PbftChainHeadPayload {
        self.0.pbft_chain_head().into()
    }

    /// Returns a non-mutating preview for the legacy persisted-head JSON path.
    pub fn pbft_chain_project_legacy_json_head(
        &self,
        block_hash: &[u8; 32],
        increments_non_empty_size: bool,
    ) -> Result<PbftChainHeadPayload, anyhow::Error> {
        Ok(self
            .0
            .pbft_chain_project_legacy_json_head(
                H256::from(*block_hash),
                increments_non_empty_size,
            )?
            .into())
    }

    /// Applies an in-memory PBFT chain update without writing storage.
    pub fn pbft_chain_update(
        &self,
        block_hash: &[u8; 32],
        anchor_hash: &[u8; 32],
    ) -> Result<PbftChainHeadPayload, anyhow::Error> {
        Ok(self
            .0
            .pbft_chain_update(H256::from(*block_hash), H256::from(*anchor_hash))?
            .into())
    }

    /// Returns whether this storage-backed PBFT chain runtime has a finalized
    /// PBFT block hash in Rust storage.
    pub fn pbft_chain_block_exists(&self, block_hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.pbft_chain_block_exists(H256::from(*block_hash))
    }

    /// Loads canonical signed PBFT block RLP from this runtime's owned Rust
    /// storage handle.
    pub fn pbft_chain_block_rlp(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<FfiBlockRlpLookup, anyhow::Error> {
        Ok(self.0.pbft_chain_block_rlp(H256::from(*block_hash))?.into())
    }
}

impl From<PbftChainHeadPayload> for PbftChainHead {
    fn from(value: PbftChainHeadPayload) -> Self {
        Self {
            head_hash: H256::from(value.head_hash),
            size: value.size,
            non_empty_size: value.non_empty_size,
            last_pbft_block_hash: H256::from(value.last_pbft_block_hash),
            last_non_null_pbft_dag_anchor_hash: H256::from(value.last_non_null_anchor_hash),
        }
    }
}

impl From<PbftChainHead> for PbftChainHeadPayload {
    fn from(value: PbftChainHead) -> Self {
        Self {
            head_hash: value.head_hash.into(),
            size: value.size,
            non_empty_size: value.non_empty_size,
            last_pbft_block_hash: value.last_pbft_block_hash.into(),
            last_non_null_anchor_hash: value.last_non_null_pbft_dag_anchor_hash.into(),
        }
    }
}

impl From<PbftBlockStorageLookup> for FfiBlockRlpLookup {
    fn from(value: PbftBlockStorageLookup) -> Self {
        Self {
            found: value.found,
            block_rlp: value.block_rlp,
        }
    }
}
