//! Deterministic transaction decision helpers for Rust-backed `TransactionManager` flows.
//!
//! The module currently owns three independent decision boundaries used by C++ shims:
//! - proposer transaction packing (`packTrxs`)
//! - DAG-block transaction persistence planning (`saveTransactionsFromDagBlock`)
//! - finalized transaction status updates (`updateFinalizedTransactionsStatus`)
//!
//! In each case Rust remains side-effect free and deterministic: it only computes the
//! plan and leaves live queue/cache mutation to C++ state.

use anyhow::{Context, Result, ensure};
use ethereum_types::{H256, U256};
use std::collections::HashSet;

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

/// One candidate transaction fact from a DAG block, supplied by the C++ caller.
///
/// The caller supplies sender/account nonce and live-cache facts because those
/// sources are not Rust-owned yet. Rust owns the nonce-gated finalized-storage
/// lookup by invoking the callback passed to [`plan_transactions_from_dag_block`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagTransactionSaveFact {
    /// Original input position in the C++ `SharedTransactions` slice.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// Raw non-finalized transaction payload to persist when accepted.
    pub trx_rlp: Vec<u8>,
    /// The transaction `nonce` declared by `Transaction::getNonce()`.
    pub transaction_nonce: U256,
    /// The sender account nonce fact, typically from `FinalChain::getAccount`.
    pub sender_account_nonce: U256,
    /// True when the transaction is already tracked in the non-finalized DAG cache.
    pub in_non_finalized_cache: bool,
    /// True when the transaction is already tracked in the recently-finalized cache.
    pub in_recently_finalized_cache: bool,
}

/// Persistent payload for one accepted DAG-block transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagTransactionSavePayload {
    /// Original input position of the accepted transaction.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// Raw transaction RLP payload to persist in the non-finalized transaction column.
    pub trx_rlp: Vec<u8>,
}

/// Deterministic plan for one DAG block persistence sweep.
///
/// `accepted_transactions` is in first-accepted order and already de-duplicated by hash.
/// `target_transaction_count` is the manager-owned status counter value that should be
/// written as `StatusDbField::TrxCount` once persistence succeeds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagTransactionSavePlan {
    pub accepted_transactions: Vec<DagTransactionSavePayload>,
    pub target_transaction_count: u64,
}

/// C++-originated finalized transaction fact supplied to Rust planning.
///
/// The caller supplies live cache membership because Rust does not yet own live
/// `TransactionManager` sidecars. The fact contains no transaction payload and
/// is stable across the CXX bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedTransactionStatusFact {
    /// Original input position in the C++ `PeriodData` transaction list.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// True when the transaction is currently still tracked in non-finalized DAG state.
    pub in_non_finalized_cache: bool,
}

/// Deterministic action for one finalized transaction after planning.
///
/// C++ uses `input_index` to resolve the live `SharedTransaction` pointer while
/// Rust controls which hashes participate in finalized-status side effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedTransactionStatusAction {
    /// Original input position to map back to C++ live structures.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
}

/// Deterministic finalized-transaction status plan.
///
/// `accepted_transactions` is emitted in input order with one entry per input,
/// matching legacy `TransactionManager::updateFinalizedTransactionsStatus`.
/// `target_transaction_count` increments the current counter only for
/// transactions that were not present in the non-finalized DAG cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedTransactionStatusPlan {
    pub accepted_transactions: Vec<FinalizedTransactionStatusAction>,
    pub target_transaction_count: u64,
    /// Some(stale_period) when `period > retention_window`, otherwise None.
    pub stale_period: Option<u64>,
    /// Legacy purge interval behavior: purge pending queue state every 100 periods.
    pub purge_transactions: bool,
}

/// Builds a deterministic save plan from C++-supplied DAG transaction facts.
///
/// Filtering preserves legacy behavior from `TransactionManager::saveTransactionsFromDagBlock`:
/// - skip entries already known in non-finalized/recently-finalized in-memory sets
/// - skip duplicates within the same DAG block by hash
/// - when sender account nonce >= transaction nonce, consult storage through the provided callback
/// - accept all others
///
/// The returned `target_transaction_count` is computed by incrementing the supplied
/// `current_transaction_count` for each accepted transaction and errors on overflow.
pub fn plan_transactions_from_dag_block<F>(
    facts: Vec<DagTransactionSaveFact>,
    current_transaction_count: u64,
    mut is_finalized: F,
) -> Result<DagTransactionSavePlan>
where
    F: FnMut(H256) -> Result<bool>,
{
    let mut accepted_transactions = Vec::new();
    let mut accepted_hashes = HashSet::with_capacity(facts.len());
    let mut target_transaction_count = current_transaction_count;

    for fact in facts {
        ensure!(!fact.hash.is_zero(), "DAG transaction hash cannot be zero");

        if fact.in_non_finalized_cache
            || fact.in_recently_finalized_cache
            || !accepted_hashes.insert(fact.hash)
        {
            continue;
        }

        if fact.sender_account_nonce >= fact.transaction_nonce && is_finalized(fact.hash)? {
            continue;
        }

        target_transaction_count = target_transaction_count.checked_add(1).context(
            "transaction count overflow while planning DAG block transaction persistence",
        )?;

        accepted_transactions.push(DagTransactionSavePayload {
            input_index: fact.input_index,
            hash: fact.hash,
            trx_rlp: fact.trx_rlp,
        });
    }

    Ok(DagTransactionSavePlan {
        accepted_transactions,
        target_transaction_count,
    })
}

/// Builds a deterministic finalized-transaction status plan from C++ facts.
///
/// Inputs:
/// - `facts`: finalized transaction hashes in legacy period-data order plus
///   live non-finalized-cache membership.
/// - `current_transaction_count`: manager-owned `TrxCount` before the period.
/// - `period`: finalized PBFT period.
/// - `retention_window`: recently-finalized cache retention in PBFT periods.
///
/// Behavior:
/// - rejects zero hashes as malformed bridge input.
/// - preserves one action per input without de-duplicating.
/// - increments `target_transaction_count` only when a finalized transaction is
///   not found in the non-finalized DAG cache.
/// - reports stale cache eviction when `period > retention_window`.
/// - reports periodic queue purge when `period` is divisible by 100.
pub fn plan_finalized_transactions_status(
    facts: Vec<FinalizedTransactionStatusFact>,
    current_transaction_count: u64,
    period: u64,
    retention_window: u64,
) -> Result<FinalizedTransactionStatusPlan> {
    let mut target_transaction_count = current_transaction_count;
    let mut accepted_transactions = Vec::with_capacity(facts.len());

    for fact in facts {
        ensure!(
            !fact.hash.is_zero(),
            "finalized transaction hash cannot be zero"
        );

        if !fact.in_non_finalized_cache {
            target_transaction_count = target_transaction_count
                .checked_add(1)
                .context("transaction count overflow while planning finalized status updates")?;
        }

        accepted_transactions.push(FinalizedTransactionStatusAction {
            input_index: fact.input_index,
            hash: fact.hash,
        });
    }

    Ok(FinalizedTransactionStatusPlan {
        accepted_transactions,
        target_transaction_count,
        stale_period: if period > retention_window {
            Some(period - retention_window)
        } else {
            None
        },
        purge_transactions: period.is_multiple_of(100),
    })
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

    fn save_fact(
        hash: u8,
        trx_nonce: u64,
        sender_nonce: u64,
        in_non_finalized_cache: bool,
        in_recently_finalized_cache: bool,
    ) -> DagTransactionSaveFact {
        DagTransactionSaveFact {
            input_index: hash as u64,
            hash: H256::from([hash; 32]),
            trx_rlp: vec![hash],
            transaction_nonce: U256::from(trx_nonce),
            sender_account_nonce: U256::from(sender_nonce),
            in_non_finalized_cache,
            in_recently_finalized_cache,
        }
    }

    fn finalized_status_fact(
        input_index: u64,
        hash: u8,
        in_non_finalized_cache: bool,
    ) -> FinalizedTransactionStatusFact {
        FinalizedTransactionStatusFact {
            input_index,
            hash: H256::from([hash; 32]),
            in_non_finalized_cache,
        }
    }

    #[test]
    fn dag_block_save_plan_filters_known_flags_duplicates_and_nonce_gates_finalization() {
        let plan = plan_transactions_from_dag_block(
            vec![
                save_fact(1, 5, 4, false, false),
                save_fact(1, 5, 4, false, false),
                save_fact(2, 9, 11, true, false),
                save_fact(3, 9, 11, false, true),
                save_fact(4, 1, 5, false, false),
                save_fact(5, 5, 11, false, false),
                save_fact(6, 2, 1, false, false),
            ],
            12,
            |hash| Ok(hash == H256::from([4; 32])),
        )
        .unwrap();

        assert_eq!(plan.target_transaction_count, 15);
        assert_eq!(
            plan.accepted_transactions
                .iter()
                .map(|payload| (payload.input_index, payload.hash))
                .collect::<Vec<_>>(),
            vec![
                (1, H256::from([1; 32])),
                (5, H256::from([5; 32])),
                (6, H256::from([6; 32])),
            ]
        );
        assert_eq!(plan.accepted_transactions[0].trx_rlp, vec![1]);
        assert_eq!(plan.accepted_transactions[1].trx_rlp, vec![5]);
        assert_eq!(plan.accepted_transactions[2].trx_rlp, vec![6]);
    }

    #[test]
    fn dag_block_save_plan_overflow_is_reported_before_persistence() {
        let result = plan_transactions_from_dag_block(
            vec![save_fact(1, 1, 0, false, false)],
            u64::MAX,
            |_| Ok(false),
        );

        assert!(result.is_err());
    }

    #[test]
    fn dag_block_save_plan_only_checks_storage_when_nonce_requires_it() {
        let mut looked_up = Vec::new();
        let plan = plan_transactions_from_dag_block(
            vec![
                save_fact(1, 5, 4, false, false),
                save_fact(2, 5, 5, false, false),
                save_fact(3, 5, 8, false, false),
            ],
            0,
            |hash| {
                looked_up.push(hash);
                Ok(hash == H256::from([2; 32]))
            },
        )
        .unwrap();

        assert_eq!(looked_up, vec![H256::from([2; 32]), H256::from([3; 32])]);
        assert_eq!(
            plan.accepted_transactions
                .iter()
                .map(|payload| (payload.input_index, payload.hash))
                .collect::<Vec<_>>(),
            vec![(1, H256::from([1; 32])), (3, H256::from([3; 32]))]
        );
    }

    #[test]
    fn finalized_status_plan_counts_only_when_not_in_non_finalized_cache() {
        let plan = plan_finalized_transactions_status(
            vec![
                finalized_status_fact(0, 1, false),
                finalized_status_fact(1, 2, true),
                finalized_status_fact(2, 3, false),
            ],
            7,
            220,
            20,
        )
        .unwrap();

        assert_eq!(plan.target_transaction_count, 9);
        assert_eq!(
            plan.accepted_transactions
                .iter()
                .map(|action| (action.input_index, action.hash))
                .collect::<Vec<_>>(),
            vec![
                (0, H256::from([1; 32])),
                (1, H256::from([2; 32])),
                (2, H256::from([3; 32]))
            ]
        );
    }

    #[test]
    fn finalized_status_plan_includes_stale_period_and_purge_flag() {
        let plan = plan_finalized_transactions_status(
            vec![
                finalized_status_fact(0, 1, false),
                finalized_status_fact(1, 2, false),
            ],
            0,
            200,
            10,
        )
        .unwrap();

        assert_eq!(plan.stale_period, Some(190));
        assert!(plan.purge_transactions);
    }

    #[test]
    fn finalized_status_plan_omits_stale_period_when_window_not_exceeded() {
        let plan =
            plan_finalized_transactions_status(vec![finalized_status_fact(0, 1, false)], 0, 5, 10)
                .unwrap();

        assert_eq!(plan.stale_period, None);
        assert!(!plan.purge_transactions);
    }

    #[test]
    fn finalized_status_plan_overflow_is_reported_before_persistence() {
        let result = plan_finalized_transactions_status(
            vec![finalized_status_fact(0, 1, false)],
            u64::MAX,
            200,
            10,
        );

        assert!(result.is_err());
    }
}
