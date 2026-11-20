//! Immutable selected native configuration. This source is not network approval.
use anyhow::{Result, ensure};

pub use ffi::{
    ConsensusApplicationBootstrapConfig, DagProposerConfig, FinalChainRewardsConfig,
    GasPricerConfig, GenesisAccount, GenesisDelegation, GenesisDposConfig, GenesisValidator,
    NativeSelectedProductionConfig, PbftServiceConfig, RedelegationCorrection,
    RewardsFrequencyRule, SigningIdentity, SortitionRuntimeConfig,
};

/// A snapshot from the native selected configuration.
///
/// ```compile_fail
/// use rustaxa_native_config::{ConsensusApplicationBootstrapConfig, SelectedProductionConfig};
/// fn forge(bootstrap: ConsensusApplicationBootstrapConfig) -> SelectedProductionConfig {
///     SelectedProductionConfig { bootstrap, transaction_max_gas: 1, transaction_min_gas_price: 1 }
/// }
/// ```
pub struct SelectedProductionConfig {
    bootstrap: ConsensusApplicationBootstrapConfig,
    transaction_max_gas: u64,
    transaction_min_gas_price: u64,
}

impl SelectedProductionConfig {
    pub fn bootstrap(&self) -> &ConsensusApplicationBootstrapConfig {
        &self.bootstrap
    }
    pub fn transaction_max_gas(&self) -> u64 {
        self.transaction_max_gas
    }
    pub fn transaction_min_gas_price(&self) -> u64 {
        self.transaction_min_gas_price
    }
}

pub fn bind_selected_production_config(
    source: cxx::UniquePtr<ffi::NativeSelectedProductionConfig>,
) -> Result<Box<SelectedProductionConfig>> {
    let source = source
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("PRODUCTION_SELECTED_CONFIG_SOURCE_UNAVAILABLE"))?;
    let bootstrap = source.native_bootstrap().clone();
    ensure!(
        bootstrap.schema_major != 0,
        "PRODUCTION_SELECTED_SCHEMA_MAJOR_ZERO"
    );
    Ok(Box::new(SelectedProductionConfig {
        bootstrap,
        transaction_max_gas: source.native_transaction_max_gas(),
        transaction_min_gas_price: source.native_transaction_min_gas_price(),
    }))
}

#[cxx::bridge(namespace = "rustaxa")]
mod ffi {
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct GenesisAccount {
        address: [u8; 20],
        balance: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct GenesisDelegation {
        delegator: [u8; 20],
        stake: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct GenesisValidator {
        address: [u8; 20],
        owner: [u8; 20],
        vrf_key: [u8; 32],
        commission: u16,
        description: String,
        endpoint: String,
        total_stake: Vec<u8>,
        delegations: Vec<GenesisDelegation>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct GenesisDposConfig {
        eligibility_balance_threshold: Vec<u8>,
        vote_eligibility_balance_step: Vec<u8>,
        validator_maximum_stake: Vec<u8>,
        minimum_deposit: Vec<u8>,
        commission_change_delta: u16,
        commission_change_frequency: u32,
        delegation_delay: u64,
        // Exclusive period boundary below which legacy DAG VDF sortition uses
        // the snapshot total eligible vote count as denominator.
        dag_vdf_sortition_total_vote_count_until_period: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RewardsFrequencyRule {
        from_period: u64,
        frequency: u32,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RedelegationCorrection {
        validator: [u8; 20],
        delegator: [u8; 20],
        amount: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct FinalChainRewardsConfig {
        committee_size: u32,
        magnolia_period: u64,
        phalaenopsis_period: u64,
        aspen_part_one_period: u64,
        fix_claim_all_block_num: u64,
        fix_redelegate_block_num: u64,
        aspen_part_two_period: u64,
        max_block_author_reward_percent: u16,
        dag_proposers_reward_percent: u16,
        yield_percentage: u16,
        dpos_blocks_per_year: u32,
        dpos_delegation_locking_period: u64,
        cornus_period: u64,
        cornus_delegation_locking_period: u64,
        genesis_balance_sum: Vec<u8>,
        aspen_max_supply: Vec<u8>,
        aspen_generated_rewards: Vec<u8>,
        cacti_period: u64,
        cacti_delegation_locking_period: u64,
        magnolia_jail_time: u64,
        cacti_jail_time: u64,
        frequency_rules: Vec<RewardsFrequencyRule>,
        redelegations: Vec<RedelegationCorrection>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct GasPricerConfig {
        percentile: u64,
        minimum_price: [u8; 32],
        history_blocks: usize,
        is_light_node: bool,
        blocks_gas_pricer: bool,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct SortitionRuntimeConfig {
        threshold_upper: u16,
        difficulty_min: u16,
        difficulty_max: u16,
        difficulty_stale: u16,
        lambda_bound: u16,
        changes_count_for_average: u16,
        dag_efficiency_target_low: u16,
        dag_efficiency_target_high: u16,
        changing_interval: u16,
        computation_interval: u16,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct PbftServiceConfig {
        genesis_lambda_ms: u64,
        cacti_lambda_max_ms: u64,
        cacti_lambda_default_ms: u64,
        cacti_block: u64,
        max_exponential_lambda_ms: u64,
        max_steps: u64,
        deadline_ms: u64,
        polling_interval_ms: u64,
        report_malicious_behaviour: bool,
        magnolia_activation_period: u64,
        ficus_activation_period: u64,
        pillar_blocks_interval: u64,
        sync_level_size: u64,
        deep_syncing_threshold: u64,
        is_light_node: bool,
        light_node_history: u64,
        committee_size: u64,
        number_of_proposers: u64,
        dag_blocks_size: u64,
        ghost_path_move_back: u64,
        node_version_major: u16,
        node_version_minor: u16,
        node_version_patch: u16,
        node_version_network: u16,
        node_version_suffix: Vec<u8>,
        default_pbft_gas_limit: u64,
        cornus_activation_period: u64,
        cornus_pbft_gas_limit: u64,
        lambda_min_ms: u64,
        lambda_change_interval: u64,
        lambda_change_ms: u64,
        consensus_delay_ms: u64,
        dpos_blocks_per_year: u64,
        recently_finalized_factor: u64,
        chain_id: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct SigningIdentity {
        wallet_index: u64,
        address: [u8; 20],
        node_public_key: [u8; 64],
        vrf_public_key: [u8; 32],
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct DagProposerConfig {
        total_transaction_shards: u16,
        proposal_dag_gas_limit: u64,
        default_dag_gas_limit: u64,
        cornus_dag_gas_limit: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct ConsensusApplicationBootstrapConfig {
        storage_path: String,
        schema_major: u32,
        schema_minor: u32,
        storage_genesis: [u8; 32],
        dag_genesis: [u8; 32],
        dag_expiry_limit: u32,
        max_levels_per_period: u64,
        sortition_config: SortitionRuntimeConfig,
        transaction_queue_capacity: usize,
        gas_pricer_config: GasPricerConfig,
        proposal_dag_gas_limit: u64,
        pbft_config: PbftServiceConfig,
        signing_identities: Vec<SigningIdentity>,
        dag_proposer: DagProposerConfig,
        final_chain_block_gas_limit: u64,
        final_chain_genesis_timestamp: u64,
        final_chain_state_api_committed_period: u64,
        final_chain_state_api_committed_root: [u8; 32],
        final_chain_bridge_contract_address: [u8; 20],
        final_chain_genesis_accounts: Vec<GenesisAccount>,
        final_chain_genesis_validators: Vec<GenesisValidator>,
        final_chain_genesis_dpos_config: GenesisDposConfig,
        final_chain_rewards_config: FinalChainRewardsConfig,
    }

    unsafe extern "C++" {
        include!("rustaxa/selected_production_config.hpp");
        type NativeSelectedProductionConfig;
        fn native_bootstrap(
            self: &NativeSelectedProductionConfig,
        ) -> &ConsensusApplicationBootstrapConfig;
        fn native_transaction_max_gas(self: &NativeSelectedProductionConfig) -> u64;
        fn native_transaction_min_gas_price(self: &NativeSelectedProductionConfig) -> u64;
    }
    extern "Rust" {
        type SelectedProductionConfig;
        fn bind_selected_production_config(
            source: UniquePtr<NativeSelectedProductionConfig>,
        ) -> Result<Box<SelectedProductionConfig>>;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn absent_native_source_is_refused() {
        let result = super::bind_selected_production_config(cxx::UniquePtr::null());
        let error = result.err().expect("null source must fail");
        assert!(
            error
                .to_string()
                .contains("PRODUCTION_SELECTED_CONFIG_SOURCE_UNAVAILABLE")
        );
    }
}
