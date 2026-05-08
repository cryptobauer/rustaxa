//! PBFT chain in-memory runtime state for the Rust rewrite shim.
//!
//! This module models the PBFT head fields and validation/update rules from the
//! legacy `PbftChain` class, but keeps storage and JSON parsing outside the
//! domain boundary. C++ shims own persisted `pbft_head` JSON decoding/encoding
//! and pass structured state into Rust.

use anyhow::{Result, anyhow};
use ethereum_types::H256;

/// PBFT head state mirrored from legacy `pbft_head` JSON payload.
///
/// Inputs/outputs:
/// - `head_hash`: key used for persisted PBFT head records.
/// - `size`: PBFT chain size including null-anchor blocks.
/// - `non_empty_size`: PBFT chain size excluding null-anchor blocks.
/// - `last_pbft_block_hash`: last appended PBFT block hash.
/// - `last_non_null_pbft_dag_anchor_hash`: latest non-null pivot DAG anchor.
///
/// Invariants:
/// - `non_empty_size <= size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PbftChainHead {
    pub head_hash: H256,
    pub size: u64,
    pub non_empty_size: u64,
    pub last_pbft_block_hash: H256,
    pub last_non_null_pbft_dag_anchor_hash: H256,
}

/// Candidate PBFT block validation result.
///
/// `Valid` means the candidate extends the current PBFT head. Mismatch variants
/// carry expected and actual values for explicit caller-side diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PbftBlockValidation {
    Valid,
    PeriodMismatch { expected: u64, actual: u64 },
    PreviousHashMismatch { expected: H256, actual: H256 },
}

/// In-memory PBFT chain state and transition rules.
///
/// This type owns only runtime state transitions:
/// - project and apply head updates for accepted PBFT blocks
/// - validate next-block period and previous-hash linkage
///
/// Storage/database lookup and JSON formatting are intentionally handled by the
/// bridge/shim layer to preserve existing persistence ownership boundaries.
#[derive(Debug, Clone)]
pub struct PbftChain {
    head: PbftChainHead,
}

impl PbftChain {
    /// Creates a PBFT runtime state object from an externally loaded head.
    ///
    /// Returns an error when invariants are invalid.
    pub fn new(head: PbftChainHead) -> Result<Self> {
        validate_head(head)?;
        Ok(Self { head })
    }

    /// Returns the current PBFT head snapshot.
    pub fn head(&self) -> PbftChainHead {
        self.head
    }

    /// Returns the projected PBFT head after appending a candidate block.
    ///
    /// Inputs:
    /// - `block_hash`: appended PBFT block hash.
    /// - `anchor_hash`: appended PBFT block pivot DAG anchor hash.
    ///
    /// Outputs:
    /// - Updated head snapshot without mutating internal state.
    ///
    /// Error behavior:
    /// - Returns an error if chain size arithmetic overflows.
    pub fn project_update(&self, block_hash: H256, anchor_hash: H256) -> Result<PbftChainHead> {
        let size = self
            .head
            .size
            .checked_add(1)
            .ok_or_else(|| anyhow!("pbft chain size overflow"))?;

        let (non_empty_size, last_non_null_pbft_dag_anchor_hash) = if anchor_hash == H256::zero() {
            (
                self.head.non_empty_size,
                self.head.last_non_null_pbft_dag_anchor_hash,
            )
        } else {
            (
                self.head
                    .non_empty_size
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("pbft non-empty chain size overflow"))?,
                anchor_hash,
            )
        };

        Ok(PbftChainHead {
            head_hash: self.head.head_hash,
            size,
            non_empty_size,
            last_pbft_block_hash: block_hash,
            last_non_null_pbft_dag_anchor_hash,
        })
    }

    /// Returns the projected legacy persisted-head JSON fields for a candidate block.
    ///
    /// Legacy callers compute persisted `pbft_head` JSON before the full pivot DAG
    /// anchor is passed into `update`. This projection therefore accepts only the
    /// null/non-null anchor classification and preserves the current hidden
    /// `last_non_null_pbft_dag_anchor_hash` field.
    pub fn project_legacy_json_head(
        &self,
        block_hash: H256,
        increments_non_empty_size: bool,
    ) -> Result<PbftChainHead> {
        let size = self
            .head
            .size
            .checked_add(1)
            .ok_or_else(|| anyhow!("pbft chain size overflow"))?;
        let non_empty_size = if increments_non_empty_size {
            self.head
                .non_empty_size
                .checked_add(1)
                .ok_or_else(|| anyhow!("pbft non-empty chain size overflow"))?
        } else {
            self.head.non_empty_size
        };

        Ok(PbftChainHead {
            head_hash: self.head.head_hash,
            size,
            non_empty_size,
            last_pbft_block_hash: block_hash,
            last_non_null_pbft_dag_anchor_hash: self.head.last_non_null_pbft_dag_anchor_hash,
        })
    }

    /// Applies a PBFT head update in place and returns the new head snapshot.
    pub fn update(&mut self, block_hash: H256, anchor_hash: H256) -> Result<PbftChainHead> {
        let next = self.project_update(block_hash, anchor_hash)?;
        self.head = next;
        Ok(next)
    }

    /// Validates whether a candidate PBFT block extends the current head.
    ///
    /// Rules:
    /// - candidate period must equal `head.size + 1`
    /// - candidate prev hash must equal `head.last_pbft_block_hash`
    pub fn validate_next_block(
        &self,
        candidate_period: u64,
        candidate_prev_hash: H256,
    ) -> PbftBlockValidation {
        let Some(expected_period) = self.head.size.checked_add(1) else {
            return PbftBlockValidation::PeriodMismatch {
                expected: u64::MAX,
                actual: candidate_period,
            };
        };

        if expected_period != candidate_period {
            return PbftBlockValidation::PeriodMismatch {
                expected: expected_period,
                actual: candidate_period,
            };
        }

        if self.head.last_pbft_block_hash != candidate_prev_hash {
            return PbftBlockValidation::PreviousHashMismatch {
                expected: self.head.last_pbft_block_hash,
                actual: candidate_prev_hash,
            };
        }

        PbftBlockValidation::Valid
    }
}

fn validate_head(head: PbftChainHead) -> Result<()> {
    if head.non_empty_size > head.size {
        return Err(anyhow!(
            "invalid pbft head: non_empty_size ({}) exceeds size ({})",
            head.non_empty_size,
            head.size
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    fn head(size: u64, non_empty_size: u64, last: u64, last_non_null: u64) -> PbftChainHead {
        PbftChainHead {
            head_hash: H256::zero(),
            size,
            non_empty_size,
            last_pbft_block_hash: hash(last),
            last_non_null_pbft_dag_anchor_hash: hash(last_non_null),
        }
    }

    #[test]
    fn rejects_invalid_head_invariant() {
        let err = PbftChain::new(head(2, 3, 0, 0)).unwrap_err().to_string();
        assert!(err.contains("non_empty_size"));
    }

    #[test]
    fn projects_and_updates_non_null_anchor_block() {
        let mut chain = PbftChain::new(head(1, 0, 11, 0)).unwrap();

        let projected = chain.project_update(hash(12), hash(99)).unwrap();
        assert_eq!(projected.size, 2);
        assert_eq!(projected.non_empty_size, 1);
        assert_eq!(projected.last_pbft_block_hash, hash(12));
        assert_eq!(projected.last_non_null_pbft_dag_anchor_hash, hash(99));

        let updated = chain.update(hash(12), hash(99)).unwrap();
        assert_eq!(updated, projected);
        assert_eq!(chain.head(), projected);
    }

    #[test]
    fn updates_null_anchor_without_changing_non_empty_or_last_non_null_anchor() {
        let mut chain = PbftChain::new(head(4, 2, 44, 777)).unwrap();

        let updated = chain.update(hash(45), H256::zero()).unwrap();
        assert_eq!(updated.size, 5);
        assert_eq!(updated.non_empty_size, 2);
        assert_eq!(updated.last_pbft_block_hash, hash(45));
        assert_eq!(updated.last_non_null_pbft_dag_anchor_hash, hash(777));
    }

    #[test]
    fn projects_legacy_json_head_from_anchor_classification() {
        let chain = PbftChain::new(head(4, 2, 44, 777)).unwrap();

        let non_empty = chain.project_legacy_json_head(hash(45), true).unwrap();
        assert_eq!(non_empty.size, 5);
        assert_eq!(non_empty.non_empty_size, 3);
        assert_eq!(non_empty.last_pbft_block_hash, hash(45));
        assert_eq!(non_empty.last_non_null_pbft_dag_anchor_hash, hash(777));

        let empty = chain.project_legacy_json_head(hash(46), false).unwrap();
        assert_eq!(empty.size, 5);
        assert_eq!(empty.non_empty_size, 2);
        assert_eq!(empty.last_pbft_block_hash, hash(46));
        assert_eq!(empty.last_non_null_pbft_dag_anchor_hash, hash(777));
    }

    #[test]
    fn validates_next_block_period_and_previous_hash() {
        let chain = PbftChain::new(head(3, 2, 333, 222)).unwrap();

        assert_eq!(
            chain.validate_next_block(4, hash(333)),
            PbftBlockValidation::Valid
        );
        assert_eq!(
            chain.validate_next_block(5, hash(333)),
            PbftBlockValidation::PeriodMismatch {
                expected: 4,
                actual: 5
            }
        );
        assert_eq!(
            chain.validate_next_block(4, hash(999)),
            PbftBlockValidation::PreviousHashMismatch {
                expected: hash(333),
                actual: hash(999)
            }
        );
    }
}
