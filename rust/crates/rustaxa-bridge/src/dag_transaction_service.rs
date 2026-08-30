//! Application-root construction and operation-shaped DAG/transaction bridge leaves.

use crate::ffi::rustaxa_ffi::*;
use anyhow::Result;
use ethereum_types::{H256, U256};
use rustaxa_consensus::gas_pricer::GasPricerConfig as DomainGasPricerConfig;
use rustaxa_consensus::pbft_service::PbftServiceConfig as DomainPbftServiceConfig;
use rustaxa_consensus::sortition::SortitionConfig as DomainSortitionConfig;
use rustaxa_consensus::transaction_service::TransactionServiceConfig;
use rustaxa_consensus::{
    ConsensusApplication, ConsensusApplicationBootstrap, ConsensusApplicationConfig,
    DagServiceConfig, DagTransactionServiceConfig,
};

/// Opaque CXX lifetime receiver for one fully restored native application.
/// No native manager or service handle is constructible or retrievable from CXX.
pub struct BridgeConsensusApplication(pub(crate) ConsensusApplication);

pub(crate) type BridgeApp = BridgeConsensusApplication;

pub(crate) fn public_transaction_request_to_native(
    request: PublicTransactionSubmissionRequest,
) -> rustaxa_consensus::PublicTransactionSubmissionRequest {
    rustaxa_consensus::PublicTransactionSubmissionRequest {
        transaction_rlp: request.transaction_rlp,
        expected_chain_id: request.expected_chain_id,
        maximum_gas_limit: request.maximum_gas_limit,
        minimum_gas_price: U256::from_big_endian(&request.minimum_gas_price),
        last_block_number: request.last_block_number,
        cornus_active: request.cornus_active,
    }
}

pub(crate) fn public_transaction_report_to_ffi(
    report: rustaxa_consensus::PublicTransactionSubmissionReport,
) -> PublicTransactionSubmissionReport {
    let (queue_status_found, queue_status) = report.queue_status.map_or((false, 0), |status| {
        use rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus;
        let code = match status {
            TransactionQueueInsertStatus::Inserted => 0,
            TransactionQueueInsertStatus::InsertedNonProposable => 1,
            TransactionQueueInsertStatus::Known => 2,
            TransactionQueueInsertStatus::Overflow => 3,
        };
        (true, code)
    });
    PublicTransactionSubmissionReport {
        transaction_hash: report.transaction_hash.into(),
        accepted: report.accepted,
        message: report.message,
        verification_status: report.verification_status.as_u8(),
        queue_status_found,
        queue_status,
        transaction_observed: report.transaction_observed,
    }
}

/// Submits one canonical signed transaction through the native application root.
pub fn consensus_application_submit_transaction(
    application: &BridgeConsensusApplication,
    request: PublicTransactionSubmissionRequest,
    final_chain: PublicTransactionFinalChainFacts,
) -> Result<PublicTransactionSubmissionReport> {
    let report = application.0.submit_public_transaction(
        public_transaction_request_to_native(request),
        rustaxa_consensus::PublicTransactionFinalChainFacts {
            sender: final_chain.sender,
            account_found: final_chain.account_found,
            account_nonce: U256::from_big_endian(&final_chain.account_nonce),
            account_balance: U256::from_big_endian(&final_chain.account_balance),
            finalized_period: final_chain
                .finalized_period_found
                .then_some(final_chain.finalized_period),
        },
    )?;
    Ok(public_transaction_report_to_ffi(report))
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

/// Restores one production application root from complete CXX bootstrap facts.
#[allow(clippy::too_many_arguments)]
pub fn create_consensus_application(
    storage_path: &str,
    schema_major: u32,
    schema_minor: u32,
    storage_genesis: &[u8; 32],
    dag_genesis: &[u8; 32],
    dag_expiry_limit: u32,
    max_levels_per_period: u64,
    sortition_config: SortitionRuntimeConfig,
    transaction_queue_config: TransactionQueueConfig,
    gas_pricer_config: GasPricerConfig,
    proposal_dag_gas_limit: u64,
    pbft_config: PbftServiceConfig,
    signing_identities: Vec<SigningIdentity>,
    dag_proposer: DagProposerConfig,
    final_chain_block_gas_limit: u64,
    final_chain_genesis_timestamp: u64,
    final_chain_bridge_contract_address: [u8; 20],
    final_chain_genesis_accounts: Vec<GenesisAccount>,
    final_chain_genesis_validators: Vec<GenesisValidator>,
    final_chain_genesis_dpos_config: GenesisDposConfig,
    final_chain_rewards_config: FinalChainRewardsConfig,
) -> Result<Box<BridgeApp>> {
    let polling_interval_ms = pbft_config.polling_interval_ms;
    let signing_identities = signing_identities
        .into_iter()
        .map(|identity| rustaxa_consensus::SigningIdentity {
            wallet_index: identity.wallet_index,
            address: identity.address,
            node_public_key: identity.node_public_key,
            vrf_public_key: identity.vrf_public_key,
        })
        .collect();
    let mut native_pbft_config = domain_pbft_config(&pbft_config)?;
    native_pbft_config.network_identity.genesis_hash = *storage_genesis;
    let root = ConsensusApplicationBootstrap {
        storage_path: storage_path.into(),
        schema_major,
        schema_minor,
        storage_genesis_hash: H256::from(*storage_genesis),
        final_chain: crate::final_chain::consensus_final_chain_config_from_ffi(
            final_chain_block_gas_limit,
            final_chain_genesis_timestamp,
            final_chain_bridge_contract_address,
            final_chain_genesis_accounts,
            final_chain_genesis_validators,
            final_chain_genesis_dpos_config,
            final_chain_rewards_config,
        )?,
        consensus: ConsensusApplicationConfig {
            dag_transaction: DagTransactionServiceConfig {
                transaction: TransactionServiceConfig {
                    queue_max_size: transaction_queue_config.max_size,
                    gas_pricer_config: domain_gas_pricer_config(gas_pricer_config),
                    proposal_dag_gas_limit,
                },
                dag: DagServiceConfig {
                    genesis_hash: H256::from(*dag_genesis),
                    dag_expiry_limit,
                    max_levels_per_period,
                },
                sortition: domain_sortition_config(sortition_config),
            },
            dag_proposer: rustaxa_consensus::DagProposerConfig {
                total_transaction_shards: dag_proposer.total_transaction_shards,
                proposal_dag_gas_limit: dag_proposer.proposal_dag_gas_limit,
                default_dag_gas_limit: dag_proposer.default_dag_gas_limit,
                default_pbft_gas_limit: pbft_config.default_pbft_gas_limit,
                cornus_activation_period: pbft_config.cornus_activation_period,
                cornus_dag_gas_limit: dag_proposer.cornus_dag_gas_limit,
                cornus_pbft_gas_limit: pbft_config.cornus_pbft_gas_limit,
            },
            pbft: native_pbft_config,
            signing_identities,
            polling_interval_ms,
        },
    }
    .bootstrap()?;
    Ok(Box::new(BridgeConsensusApplication(root)))
}
