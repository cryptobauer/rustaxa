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

use anyhow::{Result, ensure};
use std::collections::BTreeMap;

/// Fixed-point unit representing `1.00%`.
pub const ONE_PERCENT: u16 = 100;
/// Fixed-point unit representing `100.00%`.
pub const HUNDRED_PERCENT: u16 = 100 * ONE_PERCENT;

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
}
