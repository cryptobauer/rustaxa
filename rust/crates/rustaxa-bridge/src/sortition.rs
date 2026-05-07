//! CXX bridge helpers for Rust sortition parameter runtime wiring.
//!
//! This module converts between flat CXX-safe structs and the Rust consensus
//! sortition manager. C++ owns storage and batch persistence; Rust owns runtime
//! state transitions and returns typed change payloads for C++ to persist.

use anyhow::{Context, Result};
use rustaxa_consensus::sortition::{
    calculate_dag_efficiency, SortitionConfig, SortitionParams, SortitionParamsChange,
    SortitionParamsManager, VdfParams, VrfParams,
};
use std::collections::VecDeque;

use crate::ffi::{rustaxa_ffi, BridgeSortitionParamsManager};

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

impl From<SortitionParams> for rustaxa_ffi::SortitionRuntimeParams {
    fn from(params: SortitionParams) -> Self {
        Self {
            threshold_upper: params.vrf.threshold_upper,
            difficulty_min: params.vdf.difficulty_min,
            difficulty_max: params.vdf.difficulty_max,
            difficulty_stale: params.vdf.difficulty_stale,
            lambda_bound: params.vdf.lambda_bound,
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

impl BridgeSortitionParamsManager {
    /// Creates a Rust sortition manager from persisted state supplied by C++.
    ///
    /// The changes must be ordered oldest to newest. If empty, Rust inserts the
    /// period-zero default change in memory so C++ can read it through
    /// `sortition_params_changes` and persist it with normal batch ownership.
    pub fn create(
        config: rustaxa_ffi::SortitionRuntimeConfig,
        params_changes: Vec<rustaxa_ffi::SortitionParamsChangePayload>,
    ) -> Result<Box<Self>> {
        let params_changes = params_changes
            .into_iter()
            .map(SortitionParamsChange::from)
            .collect::<VecDeque<_>>();
        let manager = SortitionParamsManager::from_changes(config.into(), params_changes)
            .context("create sortition params manager")?;

        Ok(Box::new(Self(manager)))
    }

    /// Returns the manager's current runtime sortition parameters.
    pub fn current_params(&self) -> rustaxa_ffi::SortitionRuntimeParams {
        self.0.current_params().into()
    }

    /// Returns sortition parameters using an optional persisted change.
    ///
    /// C++ supplies the latest storage change at-or-before the requested period
    /// when one exists. Absence leaves the current config threshold unchanged.
    pub fn params_for_period(
        &self,
        found: bool,
        change: rustaxa_ffi::SortitionParamsChangePayload,
    ) -> rustaxa_ffi::SortitionRuntimeParams {
        let period_change = found.then(|| SortitionParamsChange::from(change));
        self.0.params_for_period(period_change).into()
    }

    /// Restores a finalized period while rebuilding runtime state during startup.
    pub fn restore_finalized_period(
        &mut self,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
    ) -> Result<()> {
        let dag_efficiency = self.efficiency_from_counts(
            has_pivot,
            unique_transactions,
            total_dag_transaction_refs,
        )?;
        self.0.restore_finalized_period(dag_efficiency)?;
        Ok(())
    }

    /// Records a finalized period and returns any emitted threshold change.
    ///
    /// `unique_transactions` is the count of finalized unique transactions and
    /// `total_dag_transaction_refs` is the total transaction references across
    /// finalized DAG blocks for the PBFT period. When `has_pivot` is false, the
    /// period is ignored just like the C++ null-pivot path.
    pub fn record_finalized_period(
        &mut self,
        period: u64,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
        non_empty_pbft_chain_size: u64,
    ) -> Result<rustaxa_ffi::SortitionParamsChangeResult> {
        let dag_efficiency = self.efficiency_from_counts(
            has_pivot,
            unique_transactions,
            total_dag_transaction_refs,
        )?;

        let Some(change) =
            self.0
                .record_finalized_period(period, dag_efficiency, non_empty_pbft_chain_size)?
        else {
            return Ok(rustaxa_ffi::SortitionParamsChangeResult {
                changed: false,
                period: 0,
                interval_efficiency: 0,
                threshold_upper: 0,
            });
        };

        Ok(change.into())
    }

    /// Returns the average of currently collected DAG efficiency samples.
    pub fn average_dag_efficiency(&self) -> Result<u16> {
        self.0.average_dag_efficiency()
    }

    /// Returns cached parameter changes in chronological order.
    pub fn params_changes(&self) -> Vec<rustaxa_ffi::SortitionParamsChangePayload> {
        self.0
            .params_changes()
            .iter()
            .copied()
            .map(rustaxa_ffi::SortitionParamsChangePayload::from)
            .collect()
    }

    /// Calculates DAG efficiency from transaction counts.
    pub fn calculate_dag_efficiency(
        &self,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
    ) -> Result<u16> {
        let unique_transactions = usize::try_from(unique_transactions)
            .context("unique transaction count does not fit usize")?;
        let total_dag_transaction_refs = usize::try_from(total_dag_transaction_refs)
            .context("total DAG transaction reference count does not fit usize")?;
        calculate_dag_efficiency(unique_transactions, total_dag_transaction_refs)
    }

    fn efficiency_from_counts(
        &self,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
    ) -> Result<Option<u16>> {
        if has_pivot {
            self.calculate_dag_efficiency(unique_transactions, total_dag_transaction_refs)
                .map(Some)
        } else {
            Ok(None)
        }
    }

    /// CXX-exported method returning current sortition parameters.
    pub fn sortition_current_params(&self) -> rustaxa_ffi::SortitionRuntimeParams {
        self.current_params()
    }

    /// CXX-exported method returning period-specific sortition parameters.
    pub fn sortition_params_for_period(
        &self,
        found: bool,
        change: rustaxa_ffi::SortitionParamsChangePayload,
    ) -> rustaxa_ffi::SortitionRuntimeParams {
        self.params_for_period(found, change)
    }

    /// CXX-exported method restoring a finalized period sample.
    pub fn sortition_restore_finalized_period(
        &mut self,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
    ) -> Result<()> {
        self.restore_finalized_period(has_pivot, unique_transactions, total_dag_transaction_refs)
    }

    /// CXX-exported method recording a finalized period sample.
    pub fn sortition_record_finalized_period(
        &mut self,
        period: u64,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
        non_empty_pbft_chain_size: u64,
    ) -> Result<rustaxa_ffi::SortitionParamsChangeResult> {
        self.record_finalized_period(
            period,
            has_pivot,
            unique_transactions,
            total_dag_transaction_refs,
            non_empty_pbft_chain_size,
        )
    }

    /// CXX-exported method returning current interval average efficiency.
    pub fn sortition_average_dag_efficiency(&self) -> Result<u16> {
        self.average_dag_efficiency()
    }

    /// CXX-exported method returning cached parameter changes.
    pub fn sortition_params_changes(&self) -> Vec<rustaxa_ffi::SortitionParamsChangePayload> {
        self.params_changes()
    }

    /// CXX-exported method calculating DAG efficiency from transaction counts.
    pub fn sortition_calculate_dag_efficiency(
        &self,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
    ) -> rustaxa_ffi::SortitionEfficiencyResult {
        match self.calculate_dag_efficiency(unique_transactions, total_dag_transaction_refs) {
            Ok(efficiency) => rustaxa_ffi::SortitionEfficiencyResult {
                ok: true,
                value: efficiency,
                error: String::new(),
            },
            Err(err) => rustaxa_ffi::SortitionEfficiencyResult {
                ok: false,
                value: 0,
                error: err.to_string(),
            },
        }
    }
}

/// Constructs a Rust sortition manager for C++ runtime wiring.
pub fn create_sortition_params_manager(
    config: rustaxa_ffi::SortitionRuntimeConfig,
    params_changes: Vec<rustaxa_ffi::SortitionParamsChangePayload>,
) -> Result<Box<BridgeSortitionParamsManager>> {
    BridgeSortitionParamsManager::create(config, params_changes)
}
