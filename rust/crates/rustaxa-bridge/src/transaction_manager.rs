//! CXX bridge wrappers for Rust transaction packing decisions.
//!
//! The bridge exposes a short-lived planner object used by the C++ `TransactionManager` shim while one DAG proposal is
//! being packed. C++ supplies live transaction metadata and gas-estimation results; Rust returns deterministic
//! selection, demotion, and stop decisions without owning transaction pointers or FinalChain state.

use crate::ffi::rustaxa_ffi::{
    TransactionPackCandidateDecision, TransactionPackCandidateInput, TransactionPackEstimateInput,
    TransactionPackEstimateOutcome,
};
use crate::ffi::BridgeTransactionPackPlanner;
use anyhow::Result;
use ethereum_types::H256;
use rustaxa_consensus::transaction_manager::{
    TransactionPackCandidate, TransactionPackEstimate, TransactionPackingPlanner,
};

/// Creates a planner for one `TransactionManager::packTrxs` invocation.
pub fn create_transaction_pack_planner(
    weight_limit: u64,
    min_transaction_gas: u64,
) -> Result<Box<BridgeTransactionPackPlanner>> {
    Ok(Box::new(BridgeTransactionPackPlanner(
        TransactionPackingPlanner::new(weight_limit, min_transaction_gas)?,
    )))
}

impl BridgeTransactionPackPlanner {
    /// Returns the maximum number of ordered queue candidates C++ should snapshot.
    pub fn transaction_pack_max_candidate_count(&self) -> u64 {
        self.0.max_candidate_count()
    }

    /// Decides whether C++ should run a live gas estimate for this candidate.
    pub fn transaction_pack_consider_candidate(
        &self,
        input: TransactionPackCandidateInput,
    ) -> Result<TransactionPackCandidateDecision> {
        let decision = self.0.consider_candidate(TransactionPackCandidate {
            hash: H256::from(input.hash),
            declared_gas: input.declared_gas,
        })?;
        Ok(TransactionPackCandidateDecision {
            should_estimate: decision.should_estimate,
        })
    }

    /// Records a C++ gas estimate and returns the live-state action C++ must apply.
    pub fn transaction_pack_record_estimate(
        &mut self,
        input: TransactionPackEstimateInput,
    ) -> Result<TransactionPackEstimateOutcome> {
        let outcome = self.0.record_estimate(TransactionPackEstimate {
            hash: H256::from(input.hash),
            gas_used: input.gas_used,
        })?;
        Ok(TransactionPackEstimateOutcome {
            hash: outcome.hash.0,
            selected: outcome.selected,
            demote_to_non_proposable: outcome.demote_to_non_proposable,
            stop: outcome.stop,
            gas_used: outcome.gas_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_planner_round_trips_estimation_decisions() {
        let mut planner = create_transaction_pack_planner(63_000, 21_000).unwrap();
        assert_eq!(planner.transaction_pack_max_candidate_count(), 3);

        let decision = planner
            .transaction_pack_consider_candidate(TransactionPackCandidateInput {
                hash: [1; 32],
                declared_gas: 42_000,
            })
            .unwrap();
        assert!(decision.should_estimate);

        let outcome = planner
            .transaction_pack_record_estimate(TransactionPackEstimateInput {
                hash: [1; 32],
                gas_used: 42_000,
            })
            .unwrap();
        assert!(outcome.selected);
        assert_eq!(outcome.hash, [1; 32]);
        assert!(outcome.stop);
    }
}
