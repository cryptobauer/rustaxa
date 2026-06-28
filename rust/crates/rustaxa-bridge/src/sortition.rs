//! CXX bridge helpers for Rust sortition parameter runtime wiring.
//!
//! This module converts between flat CXX-safe structs and the Rust consensus
//! sortition manager. Rust owns runtime state transitions and, for production
//! constructors, also owns storage reads used for startup replay and
//! period-specific threshold lookup.

use anyhow::{ensure, Context, Result};
use rustaxa_consensus::sortition::{
    calculate_dag_efficiency, SortitionConfig, SortitionParams, SortitionParamsChange,
    SortitionParamsManager, VdfParams, VrfParams,
};

use crate::ffi::{rustaxa_ffi, BridgeSortitionParamsManager, BridgeStorage};

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

impl BridgeSortitionParamsManager {
    /// Creates a Rust sortition manager from Rust storage.
    ///
    /// Startup behavior mirrors the legacy C++ manager while keeping storage
    /// access in Rust:
    /// - latest persisted changes are loaded in chronological order
    /// - missing history creates and persists the period-zero default change
    /// - finalized `PeriodData` after the latest change is replayed from
    ///   canonical RLP to restore the efficiency window
    pub fn create_from_storage(
        config: rustaxa_ffi::SortitionRuntimeConfig,
        storage: &BridgeStorage,
    ) -> Result<Box<Self>> {
        let manager = SortitionParamsManager::from_storage(config.into(), storage.0.clone())?;
        Ok(Box::new(Self { manager }))
    }

    /// Returns the manager's current runtime sortition parameters.
    pub fn current_params(&self) -> rustaxa_ffi::SortitionRuntimeParams {
        self.manager.current_params().into()
    }

    /// Returns sortition parameters for `period` by reading the latest
    /// at-or-before sortition change from Rust storage.
    pub fn params_for_period_from_storage(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::SortitionRuntimeParams> {
        Ok(self.manager.params_for_period_from_storage(period)?.into())
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

        let Some(change) = self.manager.record_finalized_period(
            period,
            dag_efficiency,
            non_empty_pbft_chain_size,
        )?
        else {
            return Ok(empty_change_result());
        };

        Ok(change.into())
    }

    /// Records a finalized period and persists any emitted threshold change
    /// through the Rust storage handle owned by the manager.
    ///
    /// This replaces the legacy Rust-mode route where C++ provided a `Batch&`
    /// solely so the storage shim could translate it into a bridge batch id.
    /// The Rust manager writes the emitted sortition change before publishing
    /// the live threshold transition.
    pub fn record_finalized_period_and_persist(
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

        let Some(change) = self.manager.record_finalized_period_and_persist(
            period,
            dag_efficiency,
            non_empty_pbft_chain_size,
        )?
        else {
            return Ok(empty_change_result());
        };

        Ok(change.into())
    }

    /// Previews a finalized-period sortition transition without mutating runtime state.
    ///
    /// The returned change, when present, is suitable for inclusion in the PBFT
    /// primary finalization storage batch. Callers must later commit the same
    /// transition through `commit_finalized_period` after storage succeeds.
    pub fn preview_finalized_period(
        &self,
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
        let change = self.manager.preview_finalized_period(
            period,
            dag_efficiency,
            non_empty_pbft_chain_size,
        )?;
        Ok(change_result(change))
    }

    /// Commits a finalized-period sortition transition and verifies it matches the preview.
    ///
    /// Inputs are the same period facts used by the preview phase plus the
    /// expected optional change. Any mismatch returns an error before C++ can
    /// report success to the PBFT finalization runtime cursor.
    pub fn commit_finalized_period(
        &mut self,
        period: u64,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
        non_empty_pbft_chain_size: u64,
        expected_changed: bool,
        expected_change: rustaxa_ffi::SortitionParamsChangePayload,
    ) -> Result<rustaxa_ffi::SortitionParamsChangeResult> {
        let expected = expected_changed.then(|| SortitionParamsChange::from(expected_change));
        let dag_efficiency = self.efficiency_from_counts(
            has_pivot,
            unique_transactions,
            total_dag_transaction_refs,
        )?;
        let actual = self.manager.record_finalized_period(
            period,
            dag_efficiency,
            non_empty_pbft_chain_size,
        )?;
        ensure!(
            actual == expected,
            "PBFT_FINALIZE_SORTITION_CHANGE_MISMATCH"
        );
        Ok(change_result(actual))
    }

    /// Returns the average of currently collected DAG efficiency samples.
    pub fn average_dag_efficiency(&self) -> Result<u16> {
        self.manager.average_dag_efficiency()
    }

    /// Returns cached parameter changes in chronological order.
    pub fn params_changes(&self) -> Vec<rustaxa_ffi::SortitionParamsChangePayload> {
        self.manager
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

    /// CXX-exported method returning period-specific sortition parameters using
    /// Rust-owned storage lookup.
    pub fn sortition_params_for_period_from_storage(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::SortitionRuntimeParams> {
        self.params_for_period_from_storage(period)
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

    /// CXX-exported method recording and persisting a finalized-period sortition update.
    pub fn sortition_record_finalized_period_and_persist(
        &mut self,
        period: u64,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
        non_empty_pbft_chain_size: u64,
    ) -> Result<rustaxa_ffi::SortitionParamsChangeResult> {
        self.record_finalized_period_and_persist(
            period,
            has_pivot,
            unique_transactions,
            total_dag_transaction_refs,
            non_empty_pbft_chain_size,
        )
    }

    /// CXX-exported method previewing a finalized period without mutating sortition state.
    pub fn sortition_preview_finalized_period(
        &self,
        period: u64,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
        non_empty_pbft_chain_size: u64,
    ) -> Result<rustaxa_ffi::SortitionParamsChangeResult> {
        self.preview_finalized_period(
            period,
            has_pivot,
            unique_transactions,
            total_dag_transaction_refs,
            non_empty_pbft_chain_size,
        )
    }

    /// CXX-exported method committing a previously previewed finalized period.
    pub fn sortition_commit_finalized_period(
        &mut self,
        period: u64,
        has_pivot: bool,
        unique_transactions: u64,
        total_dag_transaction_refs: u64,
        non_empty_pbft_chain_size: u64,
        expected_changed: bool,
        expected_change: rustaxa_ffi::SortitionParamsChangePayload,
    ) -> Result<rustaxa_ffi::SortitionParamsChangeResult> {
        self.commit_finalized_period(
            period,
            has_pivot,
            unique_transactions,
            total_dag_transaction_refs,
            non_empty_pbft_chain_size,
            expected_changed,
            expected_change,
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

/// Constructs a Rust sortition manager by loading and replaying Rust storage.
pub fn create_sortition_params_manager_from_storage(
    config: rustaxa_ffi::SortitionRuntimeConfig,
    storage: &BridgeStorage,
) -> Result<Box<BridgeSortitionParamsManager>> {
    BridgeSortitionParamsManager::create_from_storage(config, storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::create_storage;
    use ethereum_types::H256;
    use rlp::RlpStream;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after UNIX_EPOCH")
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("{prefix}_{now_ns}_{id}"))
    }

    fn runtime_config() -> rustaxa_ffi::SortitionRuntimeConfig {
        rustaxa_ffi::SortitionRuntimeConfig {
            threshold_upper: 1_000,
            difficulty_min: 1,
            difficulty_max: 10,
            difficulty_stale: 3,
            lambda_bound: 100,
            changes_count_for_average: 4,
            dag_efficiency_target_low: 4_800,
            dag_efficiency_target_high: 5_200,
            changing_interval: 1,
            computation_interval: 1,
        }
    }

    fn pbft_block_rlp(period: u64, pivot: H256) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from_low_u64_be(1));
        block.append(&pivot);
        block.append(&H256::from_low_u64_be(2));
        block.append(&H256::from_low_u64_be(3));
        block.append(&period);
        block.append(&123_u64);
        block.begin_list(0);
        block.append(&vec![0_u8; 65]);
        block.out().to_vec()
    }

    fn finalized_dag_bundle_rlp(block_ref_counts: &[usize]) -> Vec<u8> {
        let mut bundle = RlpStream::new_list(3);
        bundle.begin_list(4);
        for idx in 0..4_u64 {
            bundle.append(&H256::from_low_u64_be(idx + 10));
        }
        bundle.begin_list(block_ref_counts.len());
        for refs in block_ref_counts {
            bundle.begin_list(*refs);
            for idx in 0..*refs {
                bundle.append(&idx);
            }
        }
        bundle.begin_list(block_ref_counts.len());
        for _ in block_ref_counts {
            bundle.begin_list(7);
            bundle.append(&H256::zero());
            bundle.append(&1_u64);
            bundle.append(&1_u64);
            bundle.append(&Vec::<u8>::new());
            bundle.begin_list(0);
            bundle.append(&vec![0_u8; 65]);
            bundle.append(&0_u64);
        }
        bundle.out().to_vec()
    }

    fn period_data_rlp(
        period: u64,
        pivot: H256,
        unique_transactions: usize,
        block_ref_counts: &[usize],
    ) -> Vec<u8> {
        let pbft_block = pbft_block_rlp(period, pivot);
        let dag_bundle = finalized_dag_bundle_rlp(block_ref_counts);
        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(&pbft_block, 1);
        period_data.append_empty_data();
        period_data.append_raw(&dag_bundle, 1);
        period_data.begin_list(unique_transactions);
        for idx in 0..unique_transactions {
            period_data.begin_list(9);
            period_data.append(&0_u64);
            period_data.append(&0_u64);
            period_data.append(&0_u64);
            period_data.append(&H256::from_low_u64_be(idx as u64 + 100));
            period_data.append(&Vec::<u8>::new());
            period_data.append(&0_u64);
            period_data.append(&0_u64);
            period_data.append(&0_u64);
            period_data.append(&0_u64);
        }
        period_data.out().to_vec()
    }

    #[test]
    fn create_sortition_manager_from_storage_persists_default_change() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_sortition_default");
        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let manager = create_sortition_params_manager_from_storage(runtime_config(), &storage)
                .expect("manager should initialize");

            assert_eq!(manager.sortition_params_changes().len(), 1);
            let stored = storage
                .0
                .metadata()
                .last_sortition_params_changes_rlp(10)
                .expect("storage lookup should succeed");
            assert_eq!(stored.len(), 1);
            let change = SortitionParamsChange::from_rlp_bytes(&stored[0])
                .expect("default change should decode");
            assert_eq!(change.period, 0);
            assert_eq!(change.threshold_upper, 1_000);
        }
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn sortition_params_for_period_reads_change_from_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_sortition_period_lookup");
        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let change = SortitionParamsChange {
                period: 10,
                interval_efficiency: 5_000,
                threshold_upper: 1_234,
            };
            storage
                .0
                .metadata()
                .write_sortition_params_change(10, &change.to_rlp_bytes())
                .expect("change should persist");
            let manager = create_sortition_params_manager_from_storage(runtime_config(), &storage)
                .expect("manager should initialize");

            let params = manager
                .sortition_params_for_period_from_storage(11)
                .expect("storage lookup should succeed");
            assert_eq!(params.threshold_upper, 1_234);
        }
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn sortition_startup_replays_period_data_from_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_sortition_replay");
        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            storage
                .0
                .period()
                .write(
                    1,
                    &period_data_rlp(1, H256::from_low_u64_be(42), 2, &[2, 2]),
                )
                .expect("period data should persist");

            let manager = create_sortition_params_manager_from_storage(runtime_config(), &storage)
                .expect("manager should initialize");
            assert_eq!(
                manager
                    .sortition_average_dag_efficiency()
                    .expect("replayed average should exist"),
                5_000
            );
        }
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn sortition_record_finalized_period_and_persist_writes_change() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_sortition_persist");
        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut manager =
                create_sortition_params_manager_from_storage(runtime_config(), &storage)
                    .expect("manager should initialize");
            let result = manager
                .sortition_record_finalized_period_and_persist(9, true, 1, 2, 1)
                .expect("change should persist");
            assert!(result.changed);

            let stored = storage
                .0
                .metadata()
                .params_change_for_period_rlp(9)
                .expect("lookup should succeed")
                .expect("change should exist");
            let decoded =
                SortitionParamsChange::from_rlp_bytes(&stored).expect("change should decode");
            assert_eq!(decoded.threshold_upper, result.threshold_upper);
        }
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
