//! The existing fixed native bootstrap conversion, shared with the historical bridge.
#[cfg(feature = "production_integration_test")]
pub use crate::consensus_application::production_application::integration_fixture::{
    prepare_disposable_native_fixture, signed_fixture_bridge_creation, signed_fixture_transaction,
};
use crate::gas_pricer::GasPricerConfig as DomainGasPricerConfig;
use crate::pbft_service::PbftServiceConfig as DomainPbftServiceConfig;
use crate::sortition::SortitionConfig as DomainSortitionConfig;
use crate::transaction_service::TransactionServiceConfig;
use crate::{
    ConsensusApplicationBootstrap, ConsensusApplicationConfig, DagServiceConfig,
    DagTransactionServiceConfig,
};
use anyhow::Result;
use ethereum_types::{H256, U256};
use rustaxa_native_config::*;

pub fn application_bootstrap_from_selected(
    source: &SelectedProductionConfig,
) -> Result<ConsensusApplicationBootstrap> {
    application_bootstrap_from_bridge(source.bootstrap().clone())
}

fn domain_gas_pricer_config(config: GasPricerConfig) -> DomainGasPricerConfig {
    DomainGasPricerConfig {
        percentile: config.percentile,
        minimum_price: U256::from_big_endian(&config.minimum_price),
        history_blocks: config.history_blocks,
        is_light_node: config.is_light_node,
        blocks_gas_pricer: config.blocks_gas_pricer,
    }
}

fn domain_sortition_config(config: SortitionRuntimeConfig) -> DomainSortitionConfig {
    DomainSortitionConfig::from_runtime_values(
        config.threshold_upper,
        config.difficulty_min,
        config.difficulty_max,
        config.difficulty_stale,
        config.lambda_bound,
        config.changes_count_for_average,
        config.dag_efficiency_target_low,
        config.dag_efficiency_target_high,
        config.changing_interval,
        config.computation_interval,
    )
}

fn domain_pbft_config(config: &PbftServiceConfig) -> Result<DomainPbftServiceConfig> {
    DomainPbftServiceConfig::from_runtime_values(
        config.genesis_lambda_ms,
        config.cacti_lambda_max_ms,
        config.cacti_lambda_default_ms,
        config.cacti_block,
        config.max_exponential_lambda_ms,
        config.max_steps,
        config.deadline_ms,
        config.polling_interval_ms,
        config.report_malicious_behaviour,
        config.magnolia_activation_period,
        config.ficus_activation_period,
        config.pillar_blocks_interval,
        config.sync_level_size,
        config.deep_syncing_threshold,
        config.is_light_node,
        config.light_node_history,
        config.committee_size,
        config.number_of_proposers,
        config.dag_blocks_size,
        config.ghost_path_move_back,
        (
            config.node_version_major,
            config.node_version_minor,
            config.node_version_patch,
            config.node_version_network,
        ),
        config.node_version_suffix.clone(),
        config.default_pbft_gas_limit,
        config.cornus_activation_period,
        config.cornus_pbft_gas_limit,
        config.lambda_min_ms,
        config.lambda_change_interval,
        config.lambda_change_ms,
        config.consensus_delay_ms,
        config.dpos_blocks_per_year,
        config.recently_finalized_factor,
        config.chain_id,
    )
}

pub fn application_bootstrap_from_bridge(
    config: ConsensusApplicationBootstrapConfig,
) -> Result<ConsensusApplicationBootstrap> {
    let polling_interval_ms = config.pbft_config.polling_interval_ms;
    let signing_identities = config
        .signing_identities
        .into_iter()
        .map(|identity| crate::SigningIdentity {
            wallet_index: identity.wallet_index,
            address: identity.address,
            node_public_key: identity.node_public_key,
            vrf_public_key: identity.vrf_public_key,
        })
        .collect();
    let mut native_pbft_config = domain_pbft_config(&config.pbft_config)?;
    native_pbft_config.network_identity.genesis_hash = config.storage_genesis;
    Ok(ConsensusApplicationBootstrap {
        storage_path: config.storage_path.into(),
        schema_major: config.schema_major,
        schema_minor: config.schema_minor,
        storage_genesis_hash: H256::from(config.storage_genesis),
        final_chain: consensus_final_chain_config_from_ffi(FinalChainBootstrapInput {
            block_gas_limit: config.final_chain_block_gas_limit,
            genesis_timestamp: config.final_chain_genesis_timestamp,
            state_api_committed_period: config.final_chain_state_api_committed_period,
            state_api_committed_root: config.final_chain_state_api_committed_root,
            bridge_contract_address: config.final_chain_bridge_contract_address,
            genesis_accounts: config.final_chain_genesis_accounts,
            genesis_validators: config.final_chain_genesis_validators,
            genesis_dpos_config: config.final_chain_genesis_dpos_config,
            rewards_config: config.final_chain_rewards_config,
        })?,
        consensus: ConsensusApplicationConfig {
            dag_transaction: DagTransactionServiceConfig {
                transaction: TransactionServiceConfig {
                    queue_max_size: config.transaction_queue_capacity,
                    gas_pricer_config: domain_gas_pricer_config(config.gas_pricer_config),
                    proposal_dag_gas_limit: config.proposal_dag_gas_limit,
                },
                dag: DagServiceConfig {
                    genesis_hash: H256::from(config.dag_genesis),
                    dag_expiry_limit: config.dag_expiry_limit,
                    max_levels_per_period: config.max_levels_per_period,
                },
                sortition: domain_sortition_config(config.sortition_config),
            },
            dag_proposer: crate::DagProposerConfig {
                total_transaction_shards: config.dag_proposer.total_transaction_shards,
                proposal_dag_gas_limit: config.dag_proposer.proposal_dag_gas_limit,
                default_dag_gas_limit: config.dag_proposer.default_dag_gas_limit,
                default_pbft_gas_limit: config.pbft_config.default_pbft_gas_limit,
                cornus_activation_period: config.pbft_config.cornus_activation_period,
                cornus_dag_gas_limit: config.dag_proposer.cornus_dag_gas_limit,
                cornus_pbft_gas_limit: config.pbft_config.cornus_pbft_gas_limit,
            },
            pbft: native_pbft_config,
            signing_identities,
            polling_interval_ms,
        },
    })
}

use crate::ConsensusFinalChainConfig;
use rustaxa_native_config as rustaxa_ffi;

fn genesis_account_from_ffi(
    account: rustaxa_ffi::GenesisAccount,
) -> Result<crate::GenesisAccount, anyhow::Error> {
    Ok(crate::GenesisAccount {
        address: account.address,
        balance: rustaxa_types::FinalChainAccountBalance::from_cpp_genesis_bytes(&account.balance)?,
    })
}

pub fn genesis_dpos_config_from_ffi(
    config: rustaxa_ffi::GenesisDposConfig,
) -> Result<crate::GenesisDposConfig, anyhow::Error> {
    let amount = |bytes: &[u8]| {
        rustaxa_types::DposTokenAmount::try_from_be_slice(bytes)
            .map_err(|_| anyhow::anyhow!("FINAL_CHAIN_DPOS_TOKEN_AMOUNT_EXCEEDS_U256"))
    };
    Ok(crate::GenesisDposConfig {
        eligibility_balance_threshold: amount(&config.eligibility_balance_threshold)?,
        vote_eligibility_balance_step: amount(&config.vote_eligibility_balance_step)?,
        validator_maximum_stake: amount(&config.validator_maximum_stake)?,
        minimum_deposit: amount(&config.minimum_deposit)?,
        commission_change_delta: config.commission_change_delta,
        commission_change_frequency: config.commission_change_frequency,
        delegation_delay: config.delegation_delay,
        dag_vdf_sortition_total_vote_count_until_period: config
            .dag_vdf_sortition_total_vote_count_until_period
            .into(),
    })
}

/// Converts one rewards-configuration monetary input at the CXX ingress.
///
/// CXX retains byte-vector carriers for compatibility. Rust accepts every
/// unsigned big-endian value through 32 bytes and rejects wider inputs before
/// constructing or publishing a FinalChain instance.
pub fn rewards_token_amount_from_ffi(
    bytes: &[u8],
    field: &'static str,
) -> Result<rustaxa_types::DposTokenAmount, anyhow::Error> {
    rustaxa_types::DposTokenAmount::try_from_be_slice(bytes)
        .map_err(|_| anyhow::anyhow!("FINAL_CHAIN_DPOS_TOKEN_AMOUNT_EXCEEDS_U256: field={field}"))
}

/// Converts ordered FFI hardfork corrections into typed Rust configuration.
///
/// Vector order and duplicate entries are consensus-significant and are
/// retained exactly. Amounts wider than `U256` fail construction before a
/// FinalChain instance is published.
pub fn redelegation_corrections_from_ffi(
    corrections: Vec<rustaxa_ffi::RedelegationCorrection>,
) -> Result<Vec<crate::RedelegationCorrection>, anyhow::Error> {
    corrections
        .into_iter()
        .enumerate()
        .map(|(index, correction)| {
            Ok(crate::RedelegationCorrection {
                validator: correction.validator,
                delegator: correction.delegator,
                amount: rustaxa_types::DposTokenAmount::try_from_be_slice(&correction.amount)
                    .map_err(|_| {
                        anyhow::anyhow!(
                            "FINAL_CHAIN_DPOS_TOKEN_AMOUNT_EXCEEDS_U256: field=redelegations[{index}].amount"
                        )
                    })?,
            })
        })
        .collect()
}

/// Ordered CXX bootstrap values consumed by the native FinalChain converter.
/// Token-width checks remain in that converter; native owners validate configuration.
pub struct FinalChainBootstrapInput {
    pub block_gas_limit: u64,
    pub genesis_timestamp: u64,
    pub state_api_committed_period: u64,
    pub state_api_committed_root: [u8; 32],
    pub bridge_contract_address: [u8; 20],
    pub genesis_accounts: Vec<rustaxa_ffi::GenesisAccount>,
    pub genesis_validators: Vec<rustaxa_ffi::GenesisValidator>,
    pub genesis_dpos_config: rustaxa_ffi::GenesisDposConfig,
    pub rewards_config: rustaxa_ffi::FinalChainRewardsConfig,
}

/// Converts bootstrap values in the original validation order. Token amounts
/// wider than U256 return an error before application restoration.
pub fn consensus_final_chain_config_from_ffi(
    input: FinalChainBootstrapInput,
) -> Result<ConsensusFinalChainConfig, anyhow::Error> {
    let rewards_config = input.rewards_config;
    let genesis_accounts = input
        .genesis_accounts
        .into_iter()
        .map(genesis_account_from_ffi)
        .collect::<Result<Vec<_>, _>>()?;
    let genesis_validators = genesis_validators_from_ffi(input.genesis_validators);
    Ok(ConsensusFinalChainConfig {
        block_gas_limit: input.block_gas_limit.into(),
        genesis_timestamp: input.genesis_timestamp,
        state_api_committed_period: input.state_api_committed_period,
        state_api_committed_root: input.state_api_committed_root.into(),
        bridge_contract_address: input.bridge_contract_address,
        genesis_accounts,
        genesis_validators,
        genesis_dpos: genesis_dpos_config_from_ffi(input.genesis_dpos_config)?,
        rewards: crate::FinalChainRewardsConfig {
            committee_size: rewards_config.committee_size,
            magnolia_period: rewards_config.magnolia_period.into(),
            phalaenopsis_period: rewards_config.phalaenopsis_period.into(),
            aspen_part_one_period: rewards_config.aspen_part_one_period.into(),
            fix_claim_all_block_num: rewards_config.fix_claim_all_block_num.into(),
            fix_redelegate_block_num: rewards_config.fix_redelegate_block_num.into(),
            aspen_part_two_period: rewards_config.aspen_part_two_period.into(),
            max_block_author_reward_percent: rewards_config.max_block_author_reward_percent,
            dag_proposers_reward_percent: rewards_config.dag_proposers_reward_percent,
            yield_percentage: rewards_config.yield_percentage,
            dpos_blocks_per_year: rewards_config.dpos_blocks_per_year,
            dpos_delegation_locking_period: rewards_config.dpos_delegation_locking_period,
            cornus_period: rewards_config.cornus_period.into(),
            cornus_delegation_locking_period: rewards_config.cornus_delegation_locking_period,
            genesis_balance_sum: if rewards_config.genesis_balance_sum.is_empty() {
                None
            } else {
                Some(rewards_token_amount_from_ffi(
                    &rewards_config.genesis_balance_sum,
                    "genesis_balance_sum",
                )?)
            },
            aspen_max_supply: rewards_token_amount_from_ffi(
                &rewards_config.aspen_max_supply,
                "aspen_max_supply",
            )?,
            aspen_generated_rewards: rewards_token_amount_from_ffi(
                &rewards_config.aspen_generated_rewards,
                "aspen_generated_rewards",
            )?,
            cacti_period: rewards_config.cacti_period.into(),
            cacti_delegation_locking_period: rewards_config.cacti_delegation_locking_period,
            magnolia_jail_time: rewards_config.magnolia_jail_time,
            cacti_jail_time: rewards_config.cacti_jail_time,
            rewards_distribution_frequency: rewards_config
                .frequency_rules
                .into_iter()
                .map(|rule| (rule.from_period.into(), rule.frequency))
                .collect(),
            redelegations: redelegation_corrections_from_ffi(rewards_config.redelegations)?,
        },
    })
}

pub fn genesis_validators_from_ffi(
    genesis_validators: Vec<rustaxa_ffi::GenesisValidator>,
) -> Vec<crate::GenesisValidator> {
    genesis_validators
        .into_iter()
        .map(|validator| {
            let rustaxa_ffi::GenesisValidator {
                address,
                owner,
                vrf_key,
                commission,
                description,
                endpoint,
                total_stake,
                delegations,
            } = validator;
            crate::GenesisValidator {
                address,
                vrf_key,
                total_stake,
                delegations: delegations
                    .into_iter()
                    .map(|delegation| (delegation.delegator, delegation.stake))
                    .collect(),
                metadata: crate::GenesisValidatorMetadata {
                    owner,
                    commission,
                    description,
                    endpoint,
                },
            }
        })
        .collect()
}
