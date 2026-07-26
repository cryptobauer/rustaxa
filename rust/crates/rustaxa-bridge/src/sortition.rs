//! CXX bridge helpers for Rust sortition parameter runtime wiring.
//!
//! This module converts between flat CXX-safe structs and the Rust consensus
//! sortition manager. Rust owns runtime state transitions and, for production
//! constructors, also owns storage reads used for startup replay and
//! period-specific threshold lookup.

use anyhow::{ensure, Context, Result};
use rustaxa_consensus::sortition::{
    calculate_dag_efficiency, SortitionConfig, SortitionParams, SortitionParamsChange, VdfParams,
    VrfParams,
};

use crate::dag_transaction_service::BridgeDagTransactionService;
use crate::ffi::rustaxa_ffi;

impl From<rustaxa_ffi::SortitionRuntimeConfig> for SortitionConfig {
    fn from(config: rustaxa_ffi::SortitionRuntimeConfig) -> Self {
        Self {
            params: SortitionParams {
                vrf: VrfParams {
                    threshold_upper: config.threshold_upper,
                },
                vdf: VdfParams {
                    difficulty_min: config.difficulty_min,
                    difficulty_max: config.difficulty_max,
                    difficulty_stale: config.difficulty_stale,
                    lambda_bound: config.lambda_bound,
                },
            },
            changes_count_for_average: config.changes_count_for_average,
            dag_efficiency_targets: (
                config.dag_efficiency_target_low,
                config.dag_efficiency_target_high,
            ),
            changing_interval: config.changing_interval,
            computation_interval: config.computation_interval,
        }
    }
}

impl From<rustaxa_ffi::SortitionParamsChangePayload> for SortitionParamsChange {
    fn from(change: rustaxa_ffi::SortitionParamsChangePayload) -> Self {
        Self {
            period: change.period,
            interval_efficiency: change.interval_efficiency,
            threshold_upper: change.threshold_upper,
        }
    }
}

impl From<SortitionParamsChange> for rustaxa_ffi::SortitionParamsChangePayload {
    fn from(change: SortitionParamsChange) -> Self {
        Self {
            period: change.period,
            interval_efficiency: change.interval_efficiency,
            threshold_upper: change.threshold_upper,
        }
    }
}

impl From<SortitionParamsChange> for rustaxa_ffi::SortitionParamsChangeResult {
    fn from(change: SortitionParamsChange) -> Self {
        Self {
            changed: true,
            period: change.period,
            interval_efficiency: change.interval_efficiency,
            threshold_upper: change.threshold_upper,
        }
    }
}

fn empty_change_result() -> rustaxa_ffi::SortitionParamsChangeResult {
    rustaxa_ffi::SortitionParamsChangeResult {
        changed: false,
        period: 0,
        interval_efficiency: 0,
        threshold_upper: 0,
    }
}

fn change_result(
    change: Option<SortitionParamsChange>,
) -> rustaxa_ffi::SortitionParamsChangeResult {
    change.map_or_else(
        empty_change_result,
        rustaxa_ffi::SortitionParamsChangeResult::from,
    )
}

impl BridgeDagTransactionService {
    /// Previews a finalized-period sortition transition without mutating runtime state.
    ///
    /// The returned change, when present, is suitable for inclusion in the PBFT
    /// primary finalization storage batch. Callers must later commit the same
    /// transition through `commit_finalized_period_with_live_snapshot` after
    /// storage succeeds.
    pub fn preview_finalized_period(
        &self,
        period: u64,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
        non_empty_pbft_chain_size: u64,
    ) -> Result<rustaxa_ffi::SortitionParamsChangeResult> {
        let dag_efficiency =
            efficiency_from_counts(has_pivot, unique_transactions, total_dag_transaction_refs)?;
        let change = self.sortition()?.preview_finalized_period(
            period,
            dag_efficiency,
            non_empty_pbft_chain_size,
        )?;
        Ok(change_result(change))
    }

    /// Commits a finalized period and returns live-report snapshot values captured
    /// from the same sortition mutex guard used for the mutation.
    pub fn commit_finalized_period_with_live_snapshot(
        &self,
        period: u64,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
        non_empty_pbft_chain_size: u64,
        expected_changed: bool,
        expected_change: rustaxa_ffi::SortitionParamsChangePayload,
    ) -> Result<(rustaxa_ffi::SortitionParamsChangeResult, u16, u64)> {
        let mut sortition = self.sortition()?;
        let expected = expected_changed.then(|| SortitionParamsChange::from(expected_change));
        let dag_efficiency =
            efficiency_from_counts(has_pivot, unique_transactions, total_dag_transaction_refs)?;
        let mut updated = sortition.clone();
        let actual =
            updated.record_finalized_period(period, dag_efficiency, non_empty_pbft_chain_size)?;
        ensure!(
            actual == expected,
            "PBFT_FINALIZE_SORTITION_CHANGE_MISMATCH"
        );

        let params = updated.current_params();
        let params_changes_count = updated.params_changes().len() as u64;
        *sortition = updated;
        Ok((
            change_result(actual),
            params.vrf.threshold_upper,
            params_changes_count,
        ))
    }
}

fn efficiency_from_counts(
    has_pivot: bool,
    unique_transactions: u64,
    total_dag_transaction_refs: u64,
) -> Result<Option<u16>> {
    if has_pivot {
        let unique_transactions = usize::try_from(unique_transactions)
            .context("unique transaction count does not fit usize")?;
        let total_dag_transaction_refs = usize::try_from(total_dag_transaction_refs)
            .context("total DAG transaction reference count does not fit usize")?;
        calculate_dag_efficiency(unique_transactions, total_dag_transaction_refs).map(Some)
    } else {
        Ok(None)
    }
}
