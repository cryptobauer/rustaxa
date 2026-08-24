//! Immutable consensus bootstrap conversion for the CXX construction boundary.
//!
//! Runtime scheduling and protocol behavior stay in the native application;
//! this module only validates and converts construction-time configuration.

use crate::ffi::rustaxa_ffi::PbftServiceConfig as FfiPbftServiceConfig;
use anyhow::anyhow;
use rustaxa_consensus::pbft_service::PbftProcessSyncedPolicy;
use rustaxa_consensus::PbftServiceConfig;

fn to_startup_u32(value: u64, field: &str) -> anyhow::Result<u32> {
    u32::try_from(value).map_err(|_| anyhow!("CONSENSUS_STARTUP_{field}_OVERFLOW"))
}

/// Converts the complete immutable consensus bootstrap configuration.
///
/// Millisecond values stored natively as `u32` are checked before root
/// construction; an overflow prevents publication of a partial application.
pub(crate) fn consensus_config_from_ffi(
    config: FfiPbftServiceConfig,
) -> anyhow::Result<PbftServiceConfig> {
    Ok(PbftServiceConfig {
        genesis_lambda_ms: to_startup_u32(config.genesis_lambda_ms, "GENESIS_LAMBDA")?,
        cacti_lambda_max_ms: to_startup_u32(config.cacti_lambda_max_ms, "CACTI_LAMBDA_MAX")?,
        cacti_lambda_default_ms: to_startup_u32(
            config.cacti_lambda_default_ms,
            "CACTI_LAMBDA_DEFAULT",
        )?,
        cacti_block: config.cacti_block,
        max_exponential_lambda_ms: config.max_exponential_lambda_ms,
        max_steps: config.max_steps,
        deadline_ms: config.deadline_ms,
        polling_interval_ms: config.polling_interval_ms,
        report_malicious_behaviour: config.report_malicious_behaviour,
        magnolia_activation_period: config.magnolia_activation_period,
        ficus_activation_period: config.ficus_activation_period,
        pillar_blocks_interval: config.pillar_blocks_interval,
        sync_level_size: config.sync_level_size,
        is_light_node: config.is_light_node,
        light_node_history: config.light_node_history,
        committee_size: config.committee_size,
        number_of_proposers: config.number_of_proposers,
        dag_blocks_size: config.dag_blocks_size,
        ghost_path_move_back: config.ghost_path_move_back,
        node_version: (
            config.node_version_major,
            config.node_version_minor,
            config.node_version_patch,
            config.node_version_network,
        ),
        node_version_suffix: config.node_version_suffix,
        default_pbft_gas_limit: config.default_pbft_gas_limit,
        cornus_activation_period: config.cornus_activation_period,
        cornus_pbft_gas_limit: config.cornus_pbft_gas_limit,
        process_synced_policy: PbftProcessSyncedPolicy {
            lambda_min_ms: to_startup_u32(config.lambda_min_ms, "LAMBDA_MIN")?,
            lambda_change_interval: to_startup_u32(
                config.lambda_change_interval,
                "LAMBDA_CHANGE_INTERVAL",
            )?,
            lambda_change_ms: to_startup_u32(config.lambda_change_ms, "LAMBDA_CHANGE_MS")?,
            consensus_delay_ms: to_startup_u32(config.consensus_delay_ms, "CONSENSUS_DELAY")?,
            dpos_blocks_per_year: to_startup_u32(
                config.dpos_blocks_per_year,
                "DPOS_BLOCKS_PER_YEAR",
            )?,
            recently_finalized_factor: config.recently_finalized_factor,
            chain_id: config.chain_id,
        },
    })
}
