//! Sortition threshold tuning helpers used by consensus.
//!
//! This module computes DAG efficiency and derives the next `threshold_upper`
//! value for sortition parameter updates.
//!
//! Scale and invariants:
//! - Efficiency values use fixed-point percent units where `100 == 1.00%`.
//! - `HUNDRED_PERCENT == 10000` represents `100.00%`.
//! - Inputs that violate basic invariants return `anyhow::Error`.
//! - Threshold clamping always enforces `threshold_upper_min..=u16::MAX`.

use anyhow::{Context, Result, ensure};
use rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream};
use std::collections::{BTreeMap, VecDeque};

/// Fixed-point unit representing `1.00%`.
pub const ONE_PERCENT: u16 = 100;
/// Fixed-point unit representing `100.00%`.
pub const HUNDRED_PERCENT: u16 = 100 * ONE_PERCENT;
/// Minimum allowed VRF threshold upper bound used by the C++ config.
pub const THRESHOLD_UPPER_MIN_VALUE: u16 = 0x50;

/// VRF selection parameters used by sortition.
///
/// The threshold is the upper bound accepted by VRF sortition. Runtime tuning
/// mutates only this field while preserving the VDF parameters configured at
/// genesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrfParams {
    /// Upper bound of VRF selection.
    pub threshold_upper: u16,
}

/// VDF difficulty parameters carried alongside the VRF threshold.
///
/// These values are supplied by genesis configuration and are returned to C++
/// unchanged by the Rust sortition parameter manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VdfParams {
    /// Minimum VDF difficulty.
    pub difficulty_min: u16,
    /// Maximum VDF difficulty.
    pub difficulty_max: u16,
    /// Difficulty treated as stale by DAG block proposal.
    pub difficulty_stale: u16,
    /// Lambda upper bound used by VDF dynamic lambda logic.
    pub lambda_bound: u16,
}

/// Complete sortition parameters consumed by VDF sortition.
///
/// `vrf.threshold_upper` may be updated at runtime. `vdf` is preserved from
/// configuration and is not adjusted by the DAG efficiency controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortitionParams {
    /// VRF threshold selection parameters.
    pub vrf: VrfParams,
    /// VDF difficulty parameters.
    pub vdf: VdfParams,
}

/// Runtime sortition configuration.
///
/// The manager uses the interval settings to decide which non-empty PBFT
/// blocks contribute to efficiency samples and when a new VRF threshold change
/// must be emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortitionConfig {
    /// Current sortition parameters.
    pub params: SortitionParams,
    /// Number of historical changes retained for averaging and interpolation.
    pub changes_count_for_average: u16,
    /// Inclusive target DAG efficiency band in fixed-point percent units.
    pub dag_efficiency_targets: (u16, u16),
    /// Number of non-empty PBFT blocks between threshold updates.
    pub changing_interval: u16,
    /// Number of non-empty PBFT blocks sampled at the end of each interval.
    pub computation_interval: u16,
}

impl SortitionConfig {
    /// Returns the midpoint of the configured efficiency target band.
    pub fn target_efficiency(self) -> u16 {
        (self.dag_efficiency_targets.0 + self.dag_efficiency_targets.1) / 2
    }

    /// Returns the threshold tuning subset of this runtime configuration.
    pub fn tuning_config(self) -> SortitionTuningConfig {
        SortitionTuningConfig {
            dag_efficiency_targets: self.dag_efficiency_targets,
            threshold_upper_min: THRESHOLD_UPPER_MIN_VALUE,
        }
    }
}

/// One historical sortition parameter update.
///
/// The triplet captures which threshold was in effect for a period and the
/// observed interval efficiency associated with that update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortitionParamsChange {
    /// Consensus period at which this change became active.
    pub period: u64,
    /// Observed interval DAG efficiency in fixed-point percent units.
    pub interval_efficiency: u16,
    /// Upper threshold used for the corresponding configuration.
    pub threshold_upper: u16,
}

impl SortitionParamsChange {
    /// Builds the genesis/default change persisted when storage has no history.
    ///
    /// The change is active from period zero and uses the target efficiency so
    /// the first runtime update starts from configured genesis parameters.
    pub fn genesis_default(config: SortitionConfig) -> Self {
        Self {
            period: 0,
            interval_efficiency: config.target_efficiency(),
            threshold_upper: config.params.vrf.threshold_upper,
        }
    }

    /// Decodes one C++-compatible sortition parameter change RLP payload.
    ///
    /// The storage format is `[threshold_upper, period, interval_efficiency]`.
    pub fn from_rlp_bytes(bytes: &[u8]) -> Result<Self> {
        let rlp = Rlp::new(bytes);
        Self::decode(&rlp).context("decode sortition params change RLP")
    }

    /// Encodes this change using the storage format consumed by C++.
    pub fn to_rlp_bytes(self) -> Vec<u8> {
        rlp::encode(&self).to_vec()
    }
}

impl Encodable for SortitionParamsChange {
    fn rlp_append(&self, stream: &mut RlpStream) {
        stream.begin_list(3);
        stream.append(&self.threshold_upper);
        stream.append(&self.period);
        stream.append(&self.interval_efficiency);
    }
}

impl Decodable for SortitionParamsChange {
    fn decode(rlp: &Rlp<'_>) -> std::result::Result<Self, DecoderError> {
        Ok(Self {
            threshold_upper: rlp.val_at(0)?,
            period: rlp.val_at(1)?,
            interval_efficiency: rlp.val_at(2)?,
        })
    }
}

/// Configuration for threshold retuning.
///
/// `dag_efficiency_targets` is an inclusive `(low, high)` efficiency band.
/// Values inside the band preserve the current threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortitionTuningConfig {
    /// Inclusive target band for efficiency `(low, high)` in fixed-point units.
    pub dag_efficiency_targets: (u16, u16),
    /// Minimum allowed threshold upper bound after clamping.
    pub threshold_upper_min: u16,
}

impl SortitionTuningConfig {
    /// Returns the midpoint of the configured efficiency target band.
    pub fn target_efficiency(self) -> u16 {
        (self.dag_efficiency_targets.0 + self.dag_efficiency_targets.1) / 2
    }
}

/// Computes DAG efficiency as `unique / total` in fixed-point percent units.
///
/// Returns `100.00%` when `total_dag_transaction_refs == 0`.
///
/// Errors when `unique_transactions > total_dag_transaction_refs`.
pub fn calculate_dag_efficiency(
    unique_transactions: usize,
    total_dag_transaction_refs: usize,
) -> Result<u16> {
    ensure!(
        unique_transactions <= total_dag_transaction_refs,
        "unique transactions ({unique_transactions}) exceed total DAG transaction references ({total_dag_transaction_refs})"
    );

    if total_dag_transaction_refs == 0 {
        return Ok(HUNDRED_PERCENT);
    }

    Ok(((unique_transactions * HUNDRED_PERCENT as usize) / total_dag_transaction_refs) as u16)
}

/// Stateful runtime manager for sortition parameter updates.
///
/// The manager mirrors the C++ `SortitionParamsManager` runtime state that is
/// needed by C++ callers: cached parameter changes, collected interval
/// efficiencies, ignored sample counter, and current VRF threshold. Storage I/O
/// remains outside this type so bridge code can load and persist RLP payloads
/// without delegating behavior to legacy C++.
#[derive(Debug, Clone)]
pub struct SortitionParamsManager {
    config: SortitionConfig,
    dag_efficiencies: VecDeque<u16>,
    ignored_efficiency_counter: u32,
    params_changes: VecDeque<SortitionParamsChange>,
}

impl SortitionParamsManager {
    /// Creates a manager from config and typed persisted parameter changes.
    ///
    /// Callers should pass the latest changes in chronological order. If the
    /// list is empty, Rust inserts the same period-zero default used by legacy
    /// C++ startup and exposes it through `params_changes` so the C++ shim can
    /// persist it with normal storage/batch ownership.
    pub fn from_changes(
        mut config: SortitionConfig,
        mut params_changes: VecDeque<SortitionParamsChange>,
    ) -> Result<Self> {
        validate_sortition_config(config)?;
        if params_changes.is_empty() {
            params_changes.push_back(SortitionParamsChange::genesis_default(config));
        } else {
            let last = params_changes
                .back()
                .copied()
                .context("sortition params changes unexpectedly empty")?;
            config.params.vrf.threshold_upper = last.threshold_upper;
        }

        Ok(Self {
            config,
            dag_efficiencies: VecDeque::new(),
            ignored_efficiency_counter: 0,
            params_changes,
        })
    }

    /// Creates a manager from config and persisted parameter-change RLP values.
    ///
    /// If storage has no history, a period-zero default change is inserted into
    /// memory and returned to the caller so it can be persisted by the bridge.
    /// If history is present, the current VRF threshold is restored from the
    /// latest persisted change. Malformed RLP returns an error and does not
    /// synthesize fallback state.
    pub fn from_persisted_rlp(
        config: SortitionConfig,
        changes_rlp: Vec<Vec<u8>>,
    ) -> Result<(Self, Option<SortitionParamsChange>)> {
        let mut params_changes = VecDeque::new();
        for change_rlp in changes_rlp {
            params_changes.push_back(SortitionParamsChange::from_rlp_bytes(&change_rlp)?);
        }
        let default_change = params_changes
            .is_empty()
            .then(|| SortitionParamsChange::genesis_default(config));
        Ok((Self::from_changes(config, params_changes)?, default_change))
    }

    /// Returns current config-backed sortition parameters.
    ///
    /// When `changing_interval == 0`, runtime tuning is disabled and genesis
    /// parameters are returned unchanged.
    pub fn current_params(&self) -> SortitionParams {
        self.config.params
    }

    /// Returns sortition parameters for a period using an optional persisted change.
    ///
    /// The caller supplies the latest change at-or-before the requested period
    /// when one exists in storage. Absence of a change leaves the current config
    /// threshold unchanged.
    pub fn params_for_period(
        &self,
        period_change: Option<SortitionParamsChange>,
    ) -> SortitionParams {
        if self.config.changing_interval == 0 {
            return self.config.params;
        }

        let mut params = self.config.params;
        if let Some(change) = period_change {
            params.vrf.threshold_upper = change.threshold_upper;
        }
        params
    }

    /// Records one finalized PBFT block efficiency sample.
    ///
    /// Blocks without a pivot DAG block pass `None` and do not affect runtime
    /// counters. For sampled non-empty blocks the method may return a new
    /// parameter change when `non_empty_pbft_chain_size` reaches the configured
    /// changing interval. The returned change has already been applied to the
    /// manager and should be persisted by the bridge.
    pub fn record_finalized_period(
        &mut self,
        period: u64,
        dag_efficiency: Option<u16>,
        non_empty_pbft_chain_size: u64,
    ) -> Result<Option<SortitionParamsChange>> {
        if self.config.changing_interval == 0 {
            return Ok(None);
        }

        let Some(dag_efficiency) = dag_efficiency else {
            return Ok(None);
        };

        if self.should_collect_efficiency() {
            self.dag_efficiencies.push_back(dag_efficiency);
            if non_empty_pbft_chain_size.is_multiple_of(u64::from(self.config.changing_interval)) {
                let params_change = self.calculate_change(period)?;
                self.params_changes.push_back(params_change);
                self.cleanup();
                self.ignored_efficiency_counter = 0;
                return Ok(Some(params_change));
            }
        } else {
            self.ignored_efficiency_counter += 1;
        }

        Ok(None)
    }

    /// Previews one finalized PBFT block efficiency sample without mutating runtime state.
    ///
    /// Inputs and outputs match `record_finalized_period`, but all counter,
    /// threshold, and parameter-change mutations are applied only to a cloned
    /// runtime. PBFT finalization uses this to build the durable storage stage
    /// before committing live sortition state after the primary batch succeeds.
    pub fn preview_finalized_period(
        &self,
        period: u64,
        dag_efficiency: Option<u16>,
        non_empty_pbft_chain_size: u64,
    ) -> Result<Option<SortitionParamsChange>> {
        let mut preview = self.clone();
        preview.record_finalized_period(period, dag_efficiency, non_empty_pbft_chain_size)
    }

    /// Restores one already-finalized non-empty PBFT block into the current efficiency window.
    ///
    /// Recovery never emits parameter changes because persisted changes are
    /// loaded before replay. It only rebuilds the in-memory efficiency window
    /// and ignored sample counter for periods after the latest stored change.
    pub fn restore_finalized_period(&mut self, dag_efficiency: Option<u16>) -> Result<()> {
        if self.config.changing_interval == 0 {
            return Ok(());
        }

        let Some(dag_efficiency) = dag_efficiency else {
            return Ok(());
        };

        if self.should_collect_efficiency() {
            self.dag_efficiencies.push_back(dag_efficiency);
        } else {
            self.ignored_efficiency_counter += 1;
        }
        Ok(())
    }

    /// Returns the arithmetic mean of currently collected DAG efficiencies.
    ///
    /// Errors when no efficiencies are available, matching the C++ invariant
    /// that callers only query this after at least one sampled non-empty block.
    pub fn average_dag_efficiency(&self) -> Result<u16> {
        ensure!(
            !self.dag_efficiencies.is_empty(),
            "dag_efficiencies must not be empty"
        );
        let total: u32 = self
            .dag_efficiencies
            .iter()
            .map(|value| u32::from(*value))
            .sum();
        Ok((total / self.dag_efficiencies.len() as u32) as u16)
    }

    /// Returns cached parameter changes in chronological order.
    pub fn params_changes(&self) -> &VecDeque<SortitionParamsChange> {
        &self.params_changes
    }

    fn calculate_change(&mut self, period: u64) -> Result<SortitionParamsChange> {
        let average_dag_efficiency = self.average_dag_efficiency()?;
        let last_threshold_upper = self
            .params_changes
            .back()
            .context("params_changes must not be empty")?
            .threshold_upper;
        let new_upper_range = get_new_upper_range(
            average_dag_efficiency,
            last_threshold_upper,
            self.params_changes.make_contiguous(),
            self.config.tuning_config(),
        )?;
        let threshold_upper = clamp_threshold_upper(new_upper_range, self.config.tuning_config());

        self.config.params.vrf.threshold_upper = threshold_upper;

        Ok(SortitionParamsChange {
            period,
            interval_efficiency: average_dag_efficiency,
            threshold_upper,
        })
    }

    fn should_collect_efficiency(&self) -> bool {
        let ignored_limit =
            i32::from(self.config.changing_interval) - i32::from(self.config.computation_interval);
        self.ignored_efficiency_counter as i32 >= ignored_limit
    }

    fn cleanup(&mut self) {
        self.dag_efficiencies.clear();
        while self.params_changes.len() > usize::from(self.config.changes_count_for_average) {
            self.params_changes.pop_front();
        }
    }
}

/// Validates sortition runtime configuration before stateful use.
fn validate_sortition_config(config: SortitionConfig) -> Result<()> {
    ensure!(
        config.changes_count_for_average > 0,
        "changes_count_for_average must be greater than zero"
    );
    ensure!(
        config.changing_interval == 0 || config.computation_interval <= config.changing_interval,
        "computation_interval must not exceed changing_interval"
    );
    ensure!(
        config.dag_efficiency_targets.0 <= config.dag_efficiency_targets.1,
        "DAG efficiency target lower bound must not exceed upper bound"
    );
    Ok(())
}

/// Computes the next sortition `threshold_upper` candidate before clamping.
///
/// Behavior:
/// - If `efficiency` is inside the target band, returns `last_threshold_upper`.
/// - Otherwise applies a deviation-driven threshold delta.
/// - If enough historical points exist across the target, may blend with the
///   closest historical threshold to reduce oscillation.
///
/// Errors when `params_changes` is empty.
pub fn get_new_upper_range(
    efficiency: u16,
    last_threshold_upper: u16,
    params_changes: &[SortitionParamsChange],
    config: SortitionTuningConfig,
) -> Result<i32> {
    ensure!(
        !params_changes.is_empty(),
        "params_changes must not be empty"
    );

    if efficiency >= config.dag_efficiency_targets.0
        && efficiency <= config.dag_efficiency_targets.1
    {
        return Ok(last_threshold_upper as i32);
    }

    let target_efficiency = config.target_efficiency();
    let mut threshold_change = get_threshold_change(
        efficiency,
        target_efficiency,
        i32::from(last_threshold_upper),
    );
    let is_over_target_efficiency = efficiency >= target_efficiency;

    if !is_over_target_efficiency {
        threshold_change *= -1;
    }

    let efficiencies_to_upper_range = get_efficiencies_to_upper_range(
        efficiency,
        i32::from(last_threshold_upper),
        params_changes,
    );

    if efficiencies_to_upper_range.is_empty()
        || efficiencies_to_upper_range
            .last_key_value()
            .is_some_and(|(key, _)| *key < target_efficiency)
        || efficiencies_to_upper_range
            .first_key_value()
            .is_some_and(|(key, _)| *key >= target_efficiency)
    {
        return Ok(i32::from(last_threshold_upper) + threshold_change);
    }

    let closest_threshold = get_closest_threshold(
        &efficiencies_to_upper_range,
        target_efficiency,
        is_over_target_efficiency,
    )?;
    let is_over_last_threshold = closest_threshold >= i32::from(last_threshold_upper);

    if is_over_target_efficiency == is_over_last_threshold {
        Ok((closest_threshold + i32::from(last_threshold_upper)) / 2)
    } else {
        Ok(i32::from(last_threshold_upper) + threshold_change)
    }
}

/// Clamps a threshold candidate into `[threshold_upper_min, u16::MAX]`.
pub fn clamp_threshold_upper(new_upper_range: i32, config: SortitionTuningConfig) -> u16 {
    if new_upper_range < i32::from(config.threshold_upper_min) {
        config.threshold_upper_min
    } else if new_upper_range > i32::from(u16::MAX) {
        u16::MAX
    } else {
        new_upper_range as u16
    }
}

/// Computes threshold delta magnitude from efficiency deviation.
///
/// Larger deviations return larger steps. The sign is applied by caller.
fn get_threshold_change(efficiency: u16, target_efficiency: u16, current_threshold: i32) -> i32 {
    let deviation = if efficiency > target_efficiency {
        (u32::from(efficiency - target_efficiency) * 100
            / u32::from(HUNDRED_PERCENT - target_efficiency)) as u16
    } else if efficiency < target_efficiency {
        (u32::from(target_efficiency - efficiency) * 100 / u32::from(target_efficiency)) as u16
    } else {
        return 0;
    };

    if deviation < 20 {
        return current_threshold / 100;
    }
    if deviation < 40 {
        return i32::from(u16::MAX / 50);
    }
    i32::from(u16::MAX / 20)
}

/// Builds a map from observed efficiencies to prior threshold values.
///
/// Each point maps `params_changes[i].interval_efficiency` to
/// `params_changes[i - 1].threshold_upper`, and includes the current
/// `(efficiency, last_threshold_upper)` when history has at least two entries.
fn get_efficiencies_to_upper_range(
    efficiency: u16,
    last_threshold_upper: i32,
    params_changes: &[SortitionParamsChange],
) -> BTreeMap<u16, i32> {
    let mut efficiencies_to_upper_range = BTreeMap::new();

    for i in 1..params_changes.len() {
        efficiencies_to_upper_range.insert(
            params_changes[i].interval_efficiency,
            i32::from(params_changes[i - 1].threshold_upper),
        );
    }

    if params_changes.len() > 1 {
        efficiencies_to_upper_range.insert(efficiency, last_threshold_upper);
    }

    efficiencies_to_upper_range
}

/// Selects a historical threshold nearest to `target`.
///
/// For low-efficiency corrections (`is_over_target == false`), chooses from
/// `target..` first. For high-efficiency corrections, chooses from `..target`
/// first. Falls back to edge entries when the preferred side is missing.
///
/// Errors when `efficiencies` is empty.
fn get_closest_threshold(
    efficiencies: &BTreeMap<u16, i32>,
    target: u16,
    is_over_target: bool,
) -> Result<i32> {
    ensure!(!efficiencies.is_empty(), "efficiencies must not be empty");

    let closest = efficiencies
        .range(target..)
        .next()
        .or_else(|| efficiencies.last_key_value())
        .map(|(_, threshold)| *threshold);

    let mut closest = match closest {
        Some(value) => value,
        None => anyhow::bail!("efficiencies map has no threshold values"),
    };
    if is_over_target {
        closest = match efficiencies
            .range(..target)
            .next_back()
            .or_else(|| efficiencies.first_key_value())
            .map(|(_, threshold)| *threshold)
        {
            Some(value) => value,
            None => anyhow::bail!("efficiencies map has no threshold values"),
        };
    }

    Ok(closest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_cfg() -> SortitionTuningConfig {
        SortitionTuningConfig {
            dag_efficiency_targets: (48 * ONE_PERCENT, 52 * ONE_PERCENT),
            threshold_upper_min: 100,
        }
    }

    fn runtime_cfg() -> SortitionConfig {
        SortitionConfig {
            params: SortitionParams {
                vrf: VrfParams {
                    threshold_upper: 10_000,
                },
                vdf: VdfParams {
                    difficulty_min: 16,
                    difficulty_max: 21,
                    difficulty_stale: 23,
                    lambda_bound: 1500,
                },
            },
            changes_count_for_average: 10,
            dag_efficiency_targets: (48 * ONE_PERCENT, 52 * ONE_PERCENT),
            changing_interval: 1,
            computation_interval: 1,
        }
    }

    #[test]
    fn dag_efficiency_is_hundred_percent_when_no_transaction_refs() {
        assert_eq!(calculate_dag_efficiency(0, 0).unwrap(), HUNDRED_PERCENT);
    }

    #[test]
    fn dag_efficiency_matches_unique_over_total_references() {
        assert_eq!(calculate_dag_efficiency(70, 100).unwrap(), 70 * ONE_PERCENT);
    }

    #[test]
    fn dag_efficiency_rejects_impossible_unique_count() {
        assert!(calculate_dag_efficiency(101, 100).is_err());
    }

    #[test]
    fn upper_range_is_unchanged_inside_target_band() {
        let params = [SortitionParamsChange {
            period: 0,
            interval_efficiency: 50 * ONE_PERCENT,
            threshold_upper: 1500,
        }];

        let new_upper =
            get_new_upper_range(50 * ONE_PERCENT, 1500, &params, default_cfg()).unwrap();
        assert_eq!(new_upper, 1500);
    }

    #[test]
    fn lower_efficiency_without_history_reduces_threshold() {
        let params = [SortitionParamsChange {
            period: 0,
            interval_efficiency: 50 * ONE_PERCENT,
            threshold_upper: 2000,
        }];

        let new_upper =
            get_new_upper_range(25 * ONE_PERCENT, 2000, &params, default_cfg()).unwrap();
        assert_eq!(new_upper, 2000 - i32::from(u16::MAX / 20));
    }

    #[test]
    fn higher_efficiency_without_history_increases_threshold() {
        let params = [SortitionParamsChange {
            period: 0,
            interval_efficiency: 50 * ONE_PERCENT,
            threshold_upper: 2000,
        }];

        let new_upper =
            get_new_upper_range(75 * ONE_PERCENT, 2000, &params, default_cfg()).unwrap();
        assert_eq!(new_upper, 2000 + i32::from(u16::MAX / 20));
    }

    #[test]
    fn history_can_choose_midpoint_between_last_and_closest() {
        let cfg = default_cfg();
        let params = [
            SortitionParamsChange {
                period: 10,
                interval_efficiency: 75 * ONE_PERCENT,
                threshold_upper: 4776,
            },
            SortitionParamsChange {
                period: 20,
                interval_efficiency: 65 * ONE_PERCENT,
                threshold_upper: 6086,
            },
            SortitionParamsChange {
                period: 40,
                interval_efficiency: 55 * ONE_PERCENT,
                threshold_upper: 7396,
            },
        ];
        let last_threshold_upper = params.last().unwrap().threshold_upper;

        let new_upper =
            get_new_upper_range(47 * ONE_PERCENT, last_threshold_upper, &params, cfg).unwrap();
        assert_eq!(new_upper, (6086 + 7396) / 2);
    }

    #[test]
    fn threshold_is_clamped_to_bounds() {
        let cfg = default_cfg();
        assert_eq!(clamp_threshold_upper(-10, cfg), cfg.threshold_upper_min);
        assert_eq!(
            clamp_threshold_upper(i32::from(u16::MAX) + 500, cfg),
            u16::MAX
        );
        assert_eq!(clamp_threshold_upper(1234, cfg), 1234);
    }

    #[test]
    fn params_change_rlp_matches_cpp_field_order() {
        let change = SortitionParamsChange {
            period: 2,
            interval_efficiency: 27 * ONE_PERCENT,
            threshold_upper: 1300,
        };

        let decoded = SortitionParamsChange::from_rlp_bytes(&change.to_rlp_bytes()).unwrap();

        assert_eq!(decoded, change);
    }

    #[test]
    fn runtime_manager_initializes_default_change_when_storage_is_empty() {
        let (manager, default_change) =
            SortitionParamsManager::from_persisted_rlp(runtime_cfg(), Vec::new()).unwrap();

        let default_change = default_change.unwrap();
        assert_eq!(default_change.period, 0);
        assert_eq!(default_change.interval_efficiency, 50 * ONE_PERCENT);
        assert_eq!(default_change.threshold_upper, 10_000);
        assert_eq!(manager.params_changes().len(), 1);
    }

    #[test]
    fn runtime_manager_restores_threshold_from_latest_change() {
        let latest = SortitionParamsChange {
            period: 10,
            interval_efficiency: 75 * ONE_PERCENT,
            threshold_upper: 12_000,
        };

        let (manager, default_change) =
            SortitionParamsManager::from_persisted_rlp(runtime_cfg(), vec![latest.to_rlp_bytes()])
                .unwrap();

        assert!(default_change.is_none());
        assert_eq!(manager.current_params().vrf.threshold_upper, 12_000);
    }

    #[test]
    fn runtime_manager_records_and_emits_threshold_change() {
        let (mut manager, _) =
            SortitionParamsManager::from_persisted_rlp(runtime_cfg(), Vec::new()).unwrap();

        let change = manager
            .record_finalized_period(10, Some(25 * ONE_PERCENT), 1)
            .unwrap()
            .unwrap();

        assert_eq!(change.period, 10);
        assert_eq!(change.interval_efficiency, 25 * ONE_PERCENT);
        assert_eq!(change.threshold_upper, 10_000 - u16::MAX / 20);
        assert_eq!(
            manager.current_params().vrf.threshold_upper,
            change.threshold_upper
        );
    }

    #[test]
    fn runtime_manager_preview_does_not_mutate_until_recorded() {
        let (mut manager, _) =
            SortitionParamsManager::from_persisted_rlp(runtime_cfg(), Vec::new()).unwrap();

        let preview = manager
            .preview_finalized_period(10, Some(25 * ONE_PERCENT), 1)
            .unwrap()
            .unwrap();

        assert_eq!(preview.period, 10);
        assert_eq!(manager.current_params().vrf.threshold_upper, 10_000);
        assert_eq!(manager.params_changes().len(), 1);

        let committed = manager
            .record_finalized_period(10, Some(25 * ONE_PERCENT), 1)
            .unwrap()
            .unwrap();

        assert_eq!(committed, preview);
        assert_eq!(
            manager.current_params().vrf.threshold_upper,
            committed.threshold_upper
        );
        assert_eq!(manager.params_changes().len(), 2);
    }
}
