use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeApp;
use rustaxa_consensus::Account;
use rustaxa_consensus::ConsensusFinalChainConfig;

pub(crate) fn account_to_lookup(account: Option<Account>) -> rustaxa_ffi::AccountLookup {
    match account {
        Some(account) => rustaxa_ffi::AccountLookup {
            found: true,
            nonce: account.nonce.to_bytes(),
            balance: account.balance.to_snapshot_bytes(),
            storage_root_hash: account.storage_root_hash,
            code_hash: account.code_hash,
            code_size: account.code_size,
        },
        None => rustaxa_ffi::AccountLookup {
            found: false,
            nonce: vec![],
            balance: vec![],
            storage_root_hash: [0; 32],
            code_hash: [0; 32],
            code_size: 0,
        },
    }
}

fn genesis_account_from_ffi(
    account: rustaxa_ffi::GenesisAccount,
) -> Result<rustaxa_consensus::GenesisAccount, anyhow::Error> {
    Ok(rustaxa_consensus::GenesisAccount {
        address: account.address,
        balance: rustaxa_types::FinalChainAccountBalance::from_cpp_genesis_bytes(&account.balance)?,
    })
}

pub(crate) fn final_chain_call_request_from_ffi(
    request: rustaxa_ffi::FinalChainCall,
) -> Result<rustaxa_consensus::FinalChainCallRequest, anyhow::Error> {
    Ok(rustaxa_consensus::FinalChainCallRequest {
        block_number: request.block_number.into(),
        sender: request.sender,
        receiver: request.receiver_found.then_some(request.receiver),
        value: rustaxa_types::FinalChainTransactionValue::try_from(request.value.as_slice())
            .map_err(|_| anyhow::anyhow!("FINAL_CHAIN_TRANSACTION_VALUE_EXCEEDS_U256"))?,
        gas_price: rustaxa_types::FinalChainGasPrice::try_from(request.gas_price.as_slice())
            .map_err(|_| anyhow::anyhow!("FINAL_CHAIN_GAS_PRICE_EXCEEDS_U256"))?,
        gas_limit: request.gas_limit.into(),
        input: request.input,
    })
}

pub(crate) fn final_chain_call_outcome_to_ffi(
    outcome: rustaxa_types::FinalChainCallOutcome,
) -> rustaxa_ffi::FinalChainCallOutcome {
    rustaxa_ffi::FinalChainCallOutcome {
        code_retval: outcome.code_retval,
        logs: outcome
            .logs
            .into_iter()
            .map(|log| rustaxa_ffi::FinalChainEvmLog {
                address: log.address,
                topics: log
                    .topics
                    .into_iter()
                    .map(|topic| rustaxa_ffi::FinalChainEvmLogTopic { topic })
                    .collect(),
                data: log.data,
            })
            .collect(),
        gas_used: outcome.gas_used.as_u64(),
        code_err: outcome.code_err,
        consensus_err: outcome.consensus_err,
    }
}

pub(crate) fn genesis_dpos_config_from_ffi(
    config: rustaxa_ffi::GenesisDposConfig,
) -> Result<rustaxa_consensus::GenesisDposConfig, anyhow::Error> {
    let amount = |bytes: &[u8]| {
        rustaxa_types::DposTokenAmount::try_from_be_slice(bytes)
            .map_err(|_| anyhow::anyhow!("FINAL_CHAIN_DPOS_TOKEN_AMOUNT_EXCEEDS_U256"))
    };
    Ok(rustaxa_consensus::GenesisDposConfig {
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
pub(crate) fn rewards_token_amount_from_ffi(
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
pub(crate) fn redelegation_corrections_from_ffi(
    corrections: Vec<rustaxa_ffi::RedelegationCorrection>,
) -> Result<Vec<rustaxa_consensus::RedelegationCorrection>, anyhow::Error> {
    corrections
        .into_iter()
        .enumerate()
        .map(|(index, correction)| {
            Ok(rustaxa_consensus::RedelegationCorrection {
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

pub(crate) fn consensus_final_chain_config_from_ffi(
    block_gas_limit: u64,
    genesis_timestamp: u64,
    state_api_committed_period: u64,
    state_api_committed_root: [u8; 32],
    bridge_contract_address: [u8; 20],
    genesis_accounts: Vec<rustaxa_ffi::GenesisAccount>,
    genesis_validators: Vec<rustaxa_ffi::GenesisValidator>,
    genesis_dpos_config: rustaxa_ffi::GenesisDposConfig,
    rewards_config: rustaxa_ffi::FinalChainRewardsConfig,
) -> Result<ConsensusFinalChainConfig, anyhow::Error> {
    let genesis_accounts = genesis_accounts
        .into_iter()
        .map(genesis_account_from_ffi)
        .collect::<Result<Vec<_>, _>>()?;
    let genesis_validators = genesis_validators_from_ffi(genesis_validators);
    Ok(ConsensusFinalChainConfig {
        block_gas_limit: block_gas_limit.into(),
        genesis_timestamp,
        state_api_committed_period,
        state_api_committed_root: state_api_committed_root.into(),
        bridge_contract_address,
        genesis_accounts,
        genesis_validators,
        genesis_dpos: genesis_dpos_config_from_ffi(genesis_dpos_config)?,
        rewards: rustaxa_consensus::FinalChainRewardsConfig {
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

pub(crate) fn genesis_validators_from_ffi(
    genesis_validators: Vec<rustaxa_ffi::GenesisValidator>,
) -> Vec<rustaxa_consensus::GenesisValidator> {
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
            rustaxa_consensus::GenesisValidator {
                address,
                vrf_key,
                total_stake,
                delegations: delegations
                    .into_iter()
                    .map(|delegation| (delegation.delegator, delegation.stake))
                    .collect(),
                metadata: rustaxa_consensus::GenesisValidatorMetadata {
                    owner,
                    commission,
                    description,
                    endpoint,
                },
            }
        })
        .collect()
}

impl BridgeApp {
    /// Prunes native FinalChain lookup indexes without exposing a batch or repository handle.
    pub fn prune_final_chain_before(
        self: &BridgeApp,
        first_to_keep: u64,
    ) -> Result<u64, anyhow::Error> {
        self.0
            .final_chain_for_bridge()
            .prune_block_indexes_before(first_to_keep)
    }

    /// Reads an exact native account snapshot for the retained public-state adapter.
    ///
    /// This operation remains application-root bound because the C++ adapter
    /// composes it with the concrete `state_db` head. It is not part of the
    /// general public consensus query facade and exposes no FinalChain handle.
    pub fn get_account_at_block(
        self: &BridgeApp,
        block_number: u64,
        address: &[u8; 20],
    ) -> Result<rustaxa_ffi::AccountLookup, anyhow::Error> {
        Ok(account_to_lookup(
            self.0
                .final_chain_for_bridge()
                .account_at_block(block_number.into(), *address)?,
        ))
    }

    /// Executes the native read-only call subset for the retained hybrid EVM adapter.
    ///
    /// Arbitrary EVM calls remain in C++ `StateAPI`; this leaf handles only native FinalChain calls.
    pub fn call(
        self: &BridgeApp,
        request: rustaxa_ffi::FinalChainCall,
    ) -> Result<rustaxa_ffi::FinalChainCallOutcome, anyhow::Error> {
        let outcome = self
            .0
            .final_chain_for_bridge()
            .call(final_chain_call_request_from_ffi(request)?)?;
        Ok(final_chain_call_outcome_to_ffi(outcome))
    }
}
