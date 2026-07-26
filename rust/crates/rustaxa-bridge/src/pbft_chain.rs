//! PBFT-chain bridge adapters backed by the Rust consensus and storage implementations.
//!
//! `PbftChain`-facing queries and storage lookups are implemented on
//! [`BridgePbftService`]. Chain state is protected by its own read/write lock,
//! so public reads do not contend on the separately synchronized manager.

use crate::ffi::rustaxa_ffi::{
    PbftBlockStorageLookup as FfiPbftBlockStorageLookup, PbftBlockValidationResult,
    PbftChainHeadPayload,
};
use crate::ffi::BridgePbftService;
use ethereum_types::H256;
use rustaxa_consensus::pbft_chain::{PbftBlockStorageLookup, PbftBlockValidation, PbftChainHead};

const PBFT_VALIDATION_VALID: u8 = 0;
const PBFT_VALIDATION_PERIOD_MISMATCH: u8 = 1;
const PBFT_VALIDATION_PREVIOUS_HASH_MISMATCH: u8 = 2;
impl BridgePbftService {
    /// Returns whether storage recovery initialized the default PBFT chain head.
    pub fn pbft_chain_initialized_default(&self) -> bool {
        self.chain.initialized_default()
    }

    /// Returns the current PBFT chain head payload for C++ JSON formatting and public accessors.
    pub fn pbft_chain_head(&self) -> PbftChainHeadPayload {
        self.chain.head().into()
    }

    /// Returns a non-mutating preview for the legacy persisted-head JSON path.
    pub fn pbft_chain_project_legacy_json_head(
        &self,
        block_hash: &[u8; 32],
        increments_non_empty_size: bool,
    ) -> Result<PbftChainHeadPayload, anyhow::Error> {
        Ok(self
            .chain
            .project_legacy_json_head(H256::from(*block_hash), increments_non_empty_size)?
            .into())
    }

    /// Applies an in-memory PBFT chain update without writing storage.
    pub fn pbft_chain_update(
        &self,
        block_hash: &[u8; 32],
        anchor_hash: &[u8; 32],
    ) -> Result<PbftChainHeadPayload, anyhow::Error> {
        Ok(self
            .chain
            .update(H256::from(*block_hash), H256::from(*anchor_hash))?
            .into())
    }

    /// Returns whether this storage-backed PBFT chain runtime has a finalized
    /// PBFT block hash in Rust storage.
    pub fn pbft_chain_block_exists(&self, block_hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.chain.block_exists(H256::from(*block_hash))
    }

    /// Loads canonical signed PBFT block RLP from this runtime's owned Rust
    /// storage handle.
    pub fn pbft_chain_block_rlp(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<FfiPbftBlockStorageLookup, anyhow::Error> {
        Ok(self.chain.block_rlp(H256::from(*block_hash))?.into())
    }

    /// Checks whether the supplied candidate block extends the current PBFT head.
    pub fn pbft_chain_validate_block(
        &self,
        period: u64,
        prev_hash: &[u8; 32],
    ) -> PbftBlockValidationResult {
        match self.chain.validate_block(period, H256::from(*prev_hash)) {
            PbftBlockValidation::Valid => PbftBlockValidationResult {
                ok: true,
                code: PBFT_VALIDATION_VALID,
                expected_period: 0,
                actual_period: period,
                expected_prev_hash: [0; 32],
                actual_prev_hash: *prev_hash,
            },
            PbftBlockValidation::PeriodMismatch { expected, actual } => PbftBlockValidationResult {
                ok: false,
                code: PBFT_VALIDATION_PERIOD_MISMATCH,
                expected_period: expected,
                actual_period: actual,
                expected_prev_hash: [0; 32],
                actual_prev_hash: *prev_hash,
            },
            PbftBlockValidation::PreviousHashMismatch { expected, actual } => {
                PbftBlockValidationResult {
                    ok: false,
                    code: PBFT_VALIDATION_PREVIOUS_HASH_MISMATCH,
                    expected_period: 0,
                    actual_period: period,
                    expected_prev_hash: expected.into(),
                    actual_prev_hash: actual.into(),
                }
            }
        }
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

impl From<PbftBlockStorageLookup> for FfiPbftBlockStorageLookup {
    fn from(value: PbftBlockStorageLookup) -> Self {
        Self {
            found: value.found,
            block_rlp: value.block_rlp,
        }
    }
}
