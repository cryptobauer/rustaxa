//! Deterministic transaction packing decisions for `TransactionManager::packTrxs`.
//!
//! The planner owns the consensus-visible control flow around proposer transaction selection:
//! candidate scan sizing, declared-gas fit checks, invalid-estimate demotion decisions, accepted gas accumulation, and
//! the legacy stop condition. C++ remains responsible for live `Transaction` pointers, queue mutation, logging, and
//! FinalChain/EVM-backed gas estimation because those dependencies are not Rust-owned yet.

use anyhow::{Result, ensure};
use ethereum_types::H256;

/// Candidate metadata supplied before C++ runs a gas estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPackCandidate {
    /// Canonical transaction hash used by C++ to locate the live transaction.
    pub hash: H256,
    /// Declared transaction gas limit (`Transaction::getGas()`).
    pub declared_gas: u64,
}

/// Decision returned before C++ performs an expensive gas estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPackCandidateDecision {
    /// True when C++ should estimate this candidate and feed the result back to Rust.
    pub should_estimate: bool,
}

/// Gas-estimation fact supplied after C++ runs the live estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPackEstimate {
    /// Canonical transaction hash corresponding to the estimated candidate.
    pub hash: H256,
    /// Gas used returned by C++ FinalChain/EVM estimation.
    pub gas_used: u64,
}

/// Decision returned after Rust consumes a C++ gas estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPackEstimateOutcome {
    /// Candidate hash echoed for C++ pointer/cache updates.
    pub hash: H256,
    /// True when C++ should include the live transaction in the proposal output.
    pub selected: bool,
    /// True when C++ should demote the transaction to non-proposable queue state.
    pub demote_to_non_proposable: bool,
    /// True when the legacy "remaining space cannot fit even the smallest transaction" rule stops the scan.
    pub stop: bool,
    /// Gas value to store beside the selected transaction in the C++ return value.
    pub gas_used: u64,
}

/// Stateful planner for one `TransactionManager::packTrxs` invocation.
///
/// Invariants:
/// - `min_transaction_gas` must be non-zero.
/// - `total_weight` changes only after a valid gas estimate is accepted.
/// - fit and stop arithmetic uses wrapping operations to match legacy unsigned C++ behavior.
/// - the planner never stores live transaction data or mutates queue state.
#[derive(Clone, Debug)]
pub struct TransactionPackingPlanner {
    weight_limit: u64,
    min_transaction_gas: u64,
    total_weight: u64,
}

impl TransactionPackingPlanner {
    /// Creates a planner for one proposal-packing pass.
    pub fn new(weight_limit: u64, min_transaction_gas: u64) -> Result<Self> {
        ensure!(
            min_transaction_gas != 0,
            "minimum transaction gas must be non-zero"
        );
        Ok(Self {
            weight_limit,
            min_transaction_gas,
            total_weight: 0,
        })
    }

    /// Returns the maximum number of ordered queue candidates C++ should fetch for this packing pass.
    pub fn max_candidate_count(&self) -> u64 {
        self.weight_limit / self.min_transaction_gas
    }

    /// Decides whether a candidate can proceed to live gas estimation.
    pub fn consider_candidate(
        &self,
        candidate: TransactionPackCandidate,
    ) -> Result<TransactionPackCandidateDecision> {
        ensure!(
            !candidate.hash.is_zero(),
            "transaction candidate hash cannot be zero"
        );
        Ok(TransactionPackCandidateDecision {
            should_estimate: self.total_weight.wrapping_add(candidate.declared_gas)
                <= self.weight_limit,
        })
    }

    /// Consumes a C++ gas-estimation result and returns the required live-state action.
    pub fn record_estimate(
        &mut self,
        estimate: TransactionPackEstimate,
    ) -> Result<TransactionPackEstimateOutcome> {
        ensure!(
            !estimate.hash.is_zero(),
            "transaction estimate hash cannot be zero"
        );
        if estimate.gas_used < self.min_transaction_gas {
            return Ok(TransactionPackEstimateOutcome {
                hash: estimate.hash,
                selected: false,
                demote_to_non_proposable: true,
                stop: false,
                gas_used: estimate.gas_used,
            });
        }

        self.total_weight = self.total_weight.wrapping_add(estimate.gas_used);
        Ok(TransactionPackEstimateOutcome {
            hash: estimate.hash,
            selected: true,
            demote_to_non_proposable: false,
            stop: self.weight_limit.wrapping_sub(self.total_weight) <= self.min_transaction_gas,
            gas_used: estimate.gas_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(hash: u8, gas: u64) -> TransactionPackCandidate {
        TransactionPackCandidate {
            hash: H256::from([hash; 32]),
            declared_gas: gas,
        }
    }

    #[test]
    fn max_candidate_count_matches_weight_limit_floor() {
        let planner = TransactionPackingPlanner::new(63_000, 21_000).unwrap();
        assert_eq!(planner.max_candidate_count(), 3);
    }

    #[test]
    fn candidate_fit_uses_accepted_estimate_weight() {
        let mut planner = TransactionPackingPlanner::new(63_000, 21_000).unwrap();

        assert!(
            planner
                .consider_candidate(tx(1, 40_000))
                .unwrap()
                .should_estimate
        );
        let first = planner
            .record_estimate(TransactionPackEstimate {
                hash: H256::from([1; 32]),
                gas_used: 40_000,
            })
            .unwrap();
        assert!(first.selected);
        assert!(!first.stop);

        assert!(
            !planner
                .consider_candidate(tx(2, 24_000))
                .unwrap()
                .should_estimate
        );
        assert!(
            planner
                .consider_candidate(tx(3, 23_000))
                .unwrap()
                .should_estimate
        );
    }

    #[test]
    fn invalid_estimate_requests_non_proposable_demote_without_weight_change() {
        let mut planner = TransactionPackingPlanner::new(63_000, 21_000).unwrap();
        let invalid = planner
            .record_estimate(TransactionPackEstimate {
                hash: H256::from([1; 32]),
                gas_used: 20_999,
            })
            .unwrap();

        assert!(!invalid.selected);
        assert!(invalid.demote_to_non_proposable);
        assert!(!invalid.stop);
        assert!(
            planner
                .consider_candidate(tx(2, 63_000))
                .unwrap()
                .should_estimate
        );
    }

    #[test]
    fn stop_matches_legacy_remaining_minimum_rule() {
        let mut planner = TransactionPackingPlanner::new(63_000, 21_000).unwrap();
        let outcome = planner
            .record_estimate(TransactionPackEstimate {
                hash: H256::from([1; 32]),
                gas_used: 42_000,
            })
            .unwrap();

        assert!(outcome.selected);
        assert!(outcome.stop);
    }
}
