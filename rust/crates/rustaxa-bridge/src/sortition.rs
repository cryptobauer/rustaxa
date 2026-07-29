//! CXX configuration conversion for native sortition runtime wiring.
//!
//! Rust owns restoration and finalized-period transitions. This module retains
//! only the flat configuration conversion required by the CXX application
//! constructor.

use rustaxa_consensus::sortition::{SortitionConfig, SortitionParams, VdfParams, VrfParams};

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
