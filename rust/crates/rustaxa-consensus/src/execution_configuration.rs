//! Immutable constructor configuration. This record does not grant execution authority.
use crate::{
    consensus_application::{ConsensusApplicationConfig, DagProposerConfig},
    dag_service::DagServiceConfig,
    dag_transaction_service::DagTransactionServiceConfig,
    final_chain::FinalChain,
    network_api::NetworkNodeIdentity,
    pbft_service::{PbftProcessSyncedPolicy, PbftServiceConfig},
};
use ethereum_types::H256;
use rlp::RlpStream;
use rustaxa_storage::Storage;
use rustaxa_types::{
    Account, DposTokenAmount, FinalChainBlockNumber, FinalChainGas, FinalChainRewardsConfig,
    GenesisDposConfig, GenesisValidator, GenesisValidatorMetadata, RedelegationCorrection,
    evm_correction::CorrectionStage,
};
use std::collections::HashMap;
use tiny_keccak::{Hasher, Keccak};

/// A local configuration capture failure, never a candidate rejection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigurationUnavailable {
    Conflict(&'static str),
    MissingGenesis,
    StorageRead(String),
}
impl std::fmt::Display for ConfigurationUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ConfigurationUnavailable {}

/// Read-only configuration captured from the actual application constructor.
/// This identity does not prove network approval, enrollment or native state.
///
/// ```compile_fail
/// use rustaxa_consensus::ExecutionConfiguration;
/// let configuration = ExecutionConfiguration { canonical: Vec::new(), digest: [0; 32] };
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionConfiguration {
    canonical: Vec<u8>,
    digest: [u8; 32],
    full_genesis: H256,
}
impl ExecutionConfiguration {
    /// Borrows the complete canonical RLP record captured at construction.
    /// The caller cannot change the record through this view.
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    /// Returns Keccak-256 of the complete canonical record. The value identifies
    /// constructor inputs; it does not prove network or state approval.
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub(crate) fn full_genesis(&self) -> H256 {
        self.full_genesis
    }
    pub(crate) fn capture(
        storage: &Storage,
        expected_genesis: H256,
        chain: &FinalChain,
        config: &ConsensusApplicationConfig,
        bridge: [u8; 20],
    ) -> Result<Self, ConfigurationUnavailable> {
        let stored = storage
            .metadata()
            .genesis_hash()
            .map_err(|e| ConfigurationUnavailable::StorageRead(e.to_string()))?
            .ok_or(ConfigurationUnavailable::MissingGenesis)?;
        if stored.as_slice() != expected_genesis.as_bytes() {
            return Err(ConfigurationUnavailable::Conflict("full_genesis"));
        }
        Self::prepare(
            expected_genesis,
            &chain.native_execution_configuration(),
            config,
            bridge,
        )
    }

    /// Prepares constructor identity without storage access or execution authority.
    pub(crate) fn prepare(
        genesis: H256,
        native: &NativeExecutionConfiguration,
        config: &ConsensusApplicationConfig,
        bridge: [u8; 20],
    ) -> Result<Self, ConfigurationUnavailable> {
        Self::compose(genesis, native, config, bridge)
    }

    fn compose(
        genesis: H256,
        native: &NativeExecutionConfiguration,
        config: &ConsensusApplicationConfig,
        bridge: [u8; 20],
    ) -> Result<Self, ConfigurationUnavailable> {
        let ConsensusApplicationConfig {
            dag_transaction,
            pbft,
            signing_identities: _,
            polling_interval_ms: _,
            dag_proposer,
        } = config;
        let DagTransactionServiceConfig {
            transaction: _,
            dag,
            sortition: _,
        } = dag_transaction;
        let DagServiceConfig {
            genesis_hash: dag_genesis,
            dag_expiry_limit: _,
            max_levels_per_period,
        } = dag;
        let PbftServiceConfig {
            genesis_lambda_ms,
            cacti_lambda_max_ms,
            cacti_lambda_default_ms,
            cacti_block,
            max_exponential_lambda_ms: _,
            max_steps: _,
            deadline_ms: _,
            polling_interval_ms: _,
            report_malicious_behaviour: _,
            magnolia_activation_period,
            ficus_activation_period,
            pillar_blocks_interval,
            sync_level_size: _,
            deep_syncing_threshold: _,
            is_light_node: _,
            light_node_history: _,
            network_identity,
            committee_size,
            number_of_proposers: _,
            dag_blocks_size: _,
            ghost_path_move_back: _,
            node_version: _,
            node_version_suffix: _,
            default_pbft_gas_limit,
            cornus_activation_period,
            cornus_pbft_gas_limit,
            process_synced_policy,
        } = pbft;
        let NetworkNodeIdentity {
            chain_id,
            genesis_hash,
            node_major_version: _,
            node_minor_version: _,
            node_patch_version: _,
            is_light_node: _,
            light_node_history: _,
        } = network_identity;
        let PbftProcessSyncedPolicy {
            chain_id: synced_chain_id,
            lambda_min_ms,
            lambda_change_interval,
            lambda_change_ms,
            consensus_delay_ms,
            dpos_blocks_per_year,
            recently_finalized_factor: _,
        } = process_synced_policy;
        let DagProposerConfig {
            total_transaction_shards: _,
            proposal_dag_gas_limit: _,
            default_dag_gas_limit: _,
            default_pbft_gas_limit: proposer_default,
            cornus_activation_period: proposer_cornus,
            cornus_dag_gas_limit: _,
            cornus_pbft_gas_limit: proposer_cornus_gas,
        } = dag_proposer;
        let rewards = native.rewards();
        let checks = [
            (chain_id == synced_chain_id, "chain_id"),
            (
                genesis_hash.as_slice() == genesis.as_bytes(),
                "full_genesis",
            ),
            (
                *magnolia_activation_period == rewards.magnolia_period.as_u64(),
                "magnolia_period",
            ),
            (
                *cacti_block == rewards.cacti_period.as_u64(),
                "cacti_period",
            ),
            (
                *cornus_activation_period == rewards.cornus_period.as_u64(),
                "cornus_period",
            ),
            (
                u32::try_from(*committee_size).ok() == Some(rewards.committee_size),
                "committee_size",
            ),
            (
                *dpos_blocks_per_year == rewards.dpos_blocks_per_year,
                "dpos_blocks_per_year",
            ),
            (
                proposer_default == default_pbft_gas_limit,
                "proposer_default_pbft_gas_limit",
            ),
            (
                proposer_cornus == cornus_activation_period,
                "proposer_cornus_period",
            ),
            (
                proposer_cornus_gas == cornus_pbft_gas_limit,
                "proposer_cornus_pbft_gas_limit",
            ),
            (
                *ficus_activation_period == native.ficus_period().as_u64(),
                "ficus_period",
            ),
        ];
        for (valid, field) in checks {
            if !valid {
                return Err(ConfigurationUnavailable::Conflict(field));
            }
        }
        let mut root = RlpStream::new_list(10);
        raw(&mut root, b"rustaxa-execution-configuration");
        raw(&mut root, &1u16.to_be_bytes());
        raw(&mut root, &chain_id.to_be_bytes());
        raw(&mut root, genesis.as_bytes());
        raw(&mut root, dag_genesis.as_bytes());
        raw(&mut root, &max_levels_per_period.to_be_bytes());
        root.append_raw(native.canonical_bytes(), 1);
        root.begin_list(3);
        raw(&mut root, &bridge);
        raw(&mut root, &pillar_blocks_interval.to_be_bytes());
        raw(&mut root, &10_000_000u64.to_be_bytes());
        root.begin_list(9);
        raw(&mut root, &genesis_lambda_ms.to_be_bytes());
        raw(&mut root, &cacti_block.to_be_bytes());
        for value in [
            lambda_min_ms,
            cacti_lambda_max_ms,
            cacti_lambda_default_ms,
            lambda_change_interval,
            lambda_change_ms,
            consensus_delay_ms,
            dpos_blocks_per_year,
        ] {
            raw(&mut root, &value.to_be_bytes());
        }
        root.begin_list(3);
        for value in [
            default_pbft_gas_limit,
            cornus_activation_period,
            cornus_pbft_gas_limit,
        ] {
            raw(&mut root, &value.to_be_bytes());
        }
        let canonical = root.out().to_vec();
        let mut digest = [0; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&canonical);
        hasher.finalize(&mut digest);
        Ok(Self {
            canonical,
            digest,
            full_genesis: genesis,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeConfigurationInput {
    block_gas_limit: FinalChainGas,
    genesis_timestamp: u64,
    genesis_root: H256,
    enforce: bool,
    ficus: FinalChainBlockNumber,
    accounts: Vec<([u8; 20], Vec<u8>)>,
    validators: Vec<GenesisValidator>,
    dpos: GenesisDposConfig,
    original_sum: Option<DposTokenAmount>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeConfigurationSeed {
    input: NativeConfigurationInput,
    rewards: FinalChainRewardsConfig,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeExecutionConfiguration {
    canonical: Vec<u8>,
    seed: NativeConfigurationSeed,
}
impl NativeConfigurationInput {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture(
        block_gas_limit: FinalChainGas,
        genesis_timestamp: u64,
        genesis_root: H256,
        enforce: bool,
        ficus: FinalChainBlockNumber,
        accounts: &HashMap<[u8; 20], Account>,
        validators: &[GenesisValidator],
        dpos: &GenesisDposConfig,
        original_sum: Option<DposTokenAmount>,
    ) -> Self {
        let mut accounts: Vec<_> = accounts
            .iter()
            .map(|(address, account)| {
                let Account {
                    balance,
                    nonce: _,
                    storage_root_hash: _,
                    code_hash: _,
                    code_size: _,
                } = account;
                (*address, balance.to_snapshot_bytes())
            })
            .collect();
        accounts.sort_by_key(|entry| entry.0);
        Self {
            block_gas_limit,
            genesis_timestamp,
            genesis_root,
            enforce,
            ficus,
            accounts,
            validators: validators.to_vec(),
            dpos: dpos.clone(),
            original_sum,
        }
    }
    pub(crate) fn finish(self, rewards: &FinalChainRewardsConfig) -> NativeConfigurationSeed {
        NativeConfigurationSeed {
            input: self,
            rewards: rewards.clone(),
        }
    }
}
impl NativeExecutionConfiguration {
    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn rewards(&self) -> &FinalChainRewardsConfig {
        &self.seed.rewards
    }
    pub(crate) fn dpos(&self) -> &GenesisDposConfig {
        &self.seed.input.dpos
    }
    pub(crate) fn ficus_period(&self) -> FinalChainBlockNumber {
        self.seed.input.ficus
    }
    pub(crate) fn original_genesis_balance_sum(&self) -> Option<DposTokenAmount> {
        self.seed.input.original_sum
    }
}
impl NativeConfigurationSeed {
    pub(crate) fn fixed(&self, stage: CorrectionStage) -> NativeExecutionConfiguration {
        let tag = match stage {
            CorrectionStage::Historical => 0,
            CorrectionStage::TransientRollback => 1,
            CorrectionStage::SingleSelfDestructRefund => 2,
            CorrectionStage::CheckedReturnDataCopy => 3,
            CorrectionStage::BoundedFalconAbi => 4,
            CorrectionStage::SlashingHardError => 5,
        };
        self.encode(0, |stream| raw(stream, &[tag]))
    }
    pub(crate) fn activation(
        &self,
        activation: Option<FinalChainBlockNumber>,
    ) -> NativeExecutionConfiguration {
        self.encode(1, |stream| {
            stream.begin_list(usize::from(activation.is_some()));
            if let Some(period) = activation {
                raw(stream, &period.as_u64().to_be_bytes());
            }
        })
    }
    fn encode(&self, tag: u8, policy: impl FnOnce(&mut RlpStream)) -> NativeExecutionConfiguration {
        let input = &self.input;
        let mut stream = RlpStream::new_list(13);
        raw(
            &mut stream,
            b"rustaxa-final-chain-constructor-configuration",
        );
        raw(&mut stream, &1u16.to_be_bytes());
        raw(&mut stream, &input.block_gas_limit.as_u64().to_be_bytes());
        raw(&mut stream, &input.genesis_timestamp.to_be_bytes());
        raw(&mut stream, input.genesis_root.as_bytes());
        raw(&mut stream, &[u8::from(input.enforce)]);
        raw(&mut stream, &input.ficus.as_u64().to_be_bytes());
        stream.begin_list(input.accounts.len());
        for (address, balance) in &input.accounts {
            stream.begin_list(2);
            raw(&mut stream, address);
            raw(&mut stream, balance);
        }
        stream.begin_list(input.validators.len());
        for validator in &input.validators {
            let GenesisValidator {
                address,
                vrf_key,
                total_stake,
                delegations,
                metadata,
            } = validator;
            let GenesisValidatorMetadata {
                owner,
                commission,
                description,
                endpoint,
            } = metadata;
            stream.begin_list(8);
            raw(&mut stream, address);
            raw(&mut stream, vrf_key);
            raw(&mut stream, total_stake);
            stream.begin_list(delegations.len());
            for (address, stake) in delegations {
                stream.begin_list(2);
                raw(&mut stream, address);
                raw(&mut stream, stake);
            }
            raw(&mut stream, owner);
            raw(&mut stream, &commission.to_be_bytes());
            raw(&mut stream, description.as_bytes());
            raw(&mut stream, endpoint.as_bytes());
        }
        encode_dpos(&mut stream, &input.dpos);
        encode_rewards(&mut stream, &self.rewards);
        option_amount(&mut stream, input.original_sum);
        stream.begin_list(2);
        raw(&mut stream, &[tag]);
        policy(&mut stream);
        NativeExecutionConfiguration {
            canonical: stream.out().to_vec(),
            seed: self.clone(),
        }
    }
}
fn raw(stream: &mut RlpStream, bytes: &[u8]) {
    stream.append(&bytes);
}
fn option_amount(stream: &mut RlpStream, amount: Option<DposTokenAmount>) {
    stream.begin_list(usize::from(amount.is_some()));
    if let Some(value) = amount {
        raw(stream, &value.to_fixed_be_bytes());
    }
}
fn encode_dpos(stream: &mut RlpStream, value: &GenesisDposConfig) {
    let GenesisDposConfig {
        eligibility_balance_threshold,
        vote_eligibility_balance_step,
        validator_maximum_stake,
        minimum_deposit,
        commission_change_delta,
        commission_change_frequency,
        delegation_delay,
        dag_vdf_sortition_total_vote_count_until_period,
    } = value;
    stream.begin_list(8);
    raw(stream, &eligibility_balance_threshold.to_fixed_be_bytes());
    raw(stream, &vote_eligibility_balance_step.to_fixed_be_bytes());
    raw(stream, &validator_maximum_stake.to_fixed_be_bytes());
    raw(stream, &minimum_deposit.to_fixed_be_bytes());
    raw(stream, &commission_change_delta.to_be_bytes());
    raw(stream, &commission_change_frequency.to_be_bytes());
    raw(stream, &delegation_delay.to_be_bytes());
    raw(
        stream,
        &dag_vdf_sortition_total_vote_count_until_period
            .as_u64()
            .to_be_bytes(),
    );
}
fn encode_rewards(stream: &mut RlpStream, value: &FinalChainRewardsConfig) {
    let FinalChainRewardsConfig {
        committee_size,
        magnolia_period,
        phalaenopsis_period,
        aspen_part_one_period,
        fix_claim_all_block_num,
        fix_redelegate_block_num,
        aspen_part_two_period,
        redelegations,
        max_block_author_reward_percent,
        dag_proposers_reward_percent,
        yield_percentage,
        dpos_blocks_per_year,
        dpos_delegation_locking_period,
        cornus_period,
        cornus_delegation_locking_period,
        genesis_balance_sum,
        aspen_max_supply,
        aspen_generated_rewards,
        cacti_period,
        cacti_delegation_locking_period,
        magnolia_jail_time,
        cacti_jail_time,
        rewards_distribution_frequency,
    } = value;
    stream.begin_list(23);
    raw(stream, &committee_size.to_be_bytes());
    raw(stream, &magnolia_period.as_u64().to_be_bytes());
    raw(stream, &phalaenopsis_period.as_u64().to_be_bytes());
    raw(stream, &aspen_part_one_period.as_u64().to_be_bytes());
    raw(stream, &fix_claim_all_block_num.as_u64().to_be_bytes());
    raw(stream, &fix_redelegate_block_num.as_u64().to_be_bytes());
    raw(stream, &aspen_part_two_period.as_u64().to_be_bytes());
    stream.begin_list(redelegations.len());
    for RedelegationCorrection {
        validator,
        delegator,
        amount,
    } in redelegations
    {
        stream.begin_list(3);
        raw(stream, validator);
        raw(stream, delegator);
        raw(stream, &amount.to_fixed_be_bytes());
    }
    raw(stream, &max_block_author_reward_percent.to_be_bytes());
    raw(stream, &dag_proposers_reward_percent.to_be_bytes());
    raw(stream, &yield_percentage.to_be_bytes());
    raw(stream, &dpos_blocks_per_year.to_be_bytes());
    raw(stream, &dpos_delegation_locking_period.to_be_bytes());
    raw(stream, &cornus_period.as_u64().to_be_bytes());
    raw(stream, &cornus_delegation_locking_period.to_be_bytes());
    option_amount(stream, *genesis_balance_sum);
    raw(stream, &aspen_max_supply.to_fixed_be_bytes());
    raw(stream, &aspen_generated_rewards.to_fixed_be_bytes());
    raw(stream, &cacti_period.as_u64().to_be_bytes());
    raw(stream, &cacti_delegation_locking_period.to_be_bytes());
    raw(stream, &magnolia_jail_time.to_be_bytes());
    raw(stream, &cacti_jail_time.to_be_bytes());
    stream.begin_list(rewards_distribution_frequency.len());
    for (period, frequency) in rewards_distribution_frequency {
        stream.begin_list(2);
        raw(stream, &period.as_u64().to_be_bytes());
        raw(stream, &frequency.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus_application::consensus_application_test_bootstrap;
    use rustaxa_types::{FinalChainAccountBalance, FinalChainNonce};
    use std::path::PathBuf;

    fn amount(value: u64) -> DposTokenAmount {
        ethereum_types::U256::from(value).into()
    }
    fn account(value: &[u8]) -> Account {
        Account {
            nonce: FinalChainNonce::zero(),
            balance: FinalChainAccountBalance::from_snapshot_bytes(value).unwrap(),
            storage_root_hash: [0; 32],
            code_hash: [0; 32],
            code_size: 0,
        }
    }
    fn fixture() -> (NativeConfigurationSeed, ConsensusApplicationConfig) {
        let bootstrap = consensus_application_test_bootstrap(PathBuf::new(), vec![], 0);
        let mut config = bootstrap.consensus;
        let dpos = GenesisDposConfig {
            eligibility_balance_threshold: amount(1),
            vote_eligibility_balance_step: amount(2),
            validator_maximum_stake: amount(3),
            minimum_deposit: amount(4),
            commission_change_delta: 5,
            commission_change_frequency: 6,
            delegation_delay: 7,
            dag_vdf_sortition_total_vote_count_until_period: 8.into(),
        };
        let rewards = FinalChainRewardsConfig {
            committee_size: 1,
            magnolia_period: 2.into(),
            phalaenopsis_period: 3.into(),
            aspen_part_one_period: 4.into(),
            fix_claim_all_block_num: 5.into(),
            fix_redelegate_block_num: 6.into(),
            aspen_part_two_period: 7.into(),
            redelegations: vec![
                RedelegationCorrection {
                    validator: [11; 20],
                    delegator: [12; 20],
                    amount: amount(13),
                },
                RedelegationCorrection {
                    validator: [14; 20],
                    delegator: [15; 20],
                    amount: amount(16),
                },
            ],
            max_block_author_reward_percent: 9,
            dag_proposers_reward_percent: 10,
            yield_percentage: 11,
            dpos_blocks_per_year: 12,
            dpos_delegation_locking_period: 13,
            cornus_period: 14.into(),
            cornus_delegation_locking_period: 15,
            genesis_balance_sum: Some(amount(16)),
            aspen_max_supply: amount(17),
            aspen_generated_rewards: amount(18),
            cacti_period: 19.into(),
            cacti_delegation_locking_period: 20,
            magnolia_jail_time: 21,
            cacti_jail_time: 22,
            rewards_distribution_frequency: vec![(24.into(), 25), (26.into(), 27)],
        };
        config.pbft.network_identity.chain_id = 9;
        config.pbft.network_identity.genesis_hash = [10; 32];
        config.pbft.process_synced_policy.chain_id = 9;
        config.pbft.magnolia_activation_period = rewards.magnolia_period.as_u64();
        config.pbft.cacti_block = rewards.cacti_period.as_u64();
        config.pbft.cornus_activation_period = rewards.cornus_period.as_u64();
        config.pbft.committee_size = u64::from(rewards.committee_size);
        config.pbft.process_synced_policy.dpos_blocks_per_year = rewards.dpos_blocks_per_year;
        config.pbft.ficus_activation_period = 5;
        config.dag_proposer.cornus_activation_period = config.pbft.cornus_activation_period;
        let accounts = HashMap::from([([1; 20], account(&[2])), ([3; 20], account(&[]))]);
        let validators = vec![
            GenesisValidator {
                address: [4; 20],
                vrf_key: [5; 32],
                total_stake: vec![6],
                delegations: vec![([7; 20], vec![8]), ([9; 20], vec![10])],
                metadata: GenesisValidatorMetadata {
                    owner: [11; 20],
                    commission: 12,
                    description: "description".into(),
                    endpoint: "endpoint".into(),
                },
            },
            GenesisValidator {
                address: [13; 20],
                vrf_key: [14; 32],
                total_stake: vec![],
                delegations: vec![],
                metadata: GenesisValidatorMetadata::default(),
            },
        ];
        let seed = NativeConfigurationInput::capture(
            1.into(),
            2,
            H256([3; 32]),
            true,
            5.into(),
            &accounts,
            &validators,
            &dpos,
            Some(amount(6)),
        )
        .finish(&rewards);
        (seed, config)
    }
    fn root(
        seed: &NativeConfigurationSeed,
        config: &ConsensusApplicationConfig,
    ) -> Result<ExecutionConfiguration, ConfigurationUnavailable> {
        ExecutionConfiguration::compose(
            H256([10; 32]),
            &seed.fixed(CorrectionStage::Historical),
            config,
            [17; 20],
        )
    }

    // This encoder uses the RLP byte rules directly, without the production RLP builder.
    enum Item {
        Bytes(Vec<u8>),
        List(Vec<Item>),
    }
    fn b(bytes: &[u8]) -> Item {
        Item::Bytes(bytes.to_vec())
    }
    fn n(value: u64, width: usize) -> Item {
        let mut bytes = vec![0; width];
        let source = value.to_be_bytes();
        let count = width.min(8);
        bytes[width - count..].copy_from_slice(&source[8 - count..]);
        b(&bytes)
    }
    fn l(items: Vec<Item>) -> Item {
        Item::List(items)
    }
    fn encode(item: &Item) -> Vec<u8> {
        let (payload, offset) = match item {
            Item::Bytes(value) if value.len() == 1 && value[0] < 128 => return value.clone(),
            Item::Bytes(value) => (value.clone(), 128u8),
            Item::List(values) => (values.iter().flat_map(encode).collect(), 192u8),
        };
        let mut result = if payload.len() < 56 {
            vec![offset + payload.len() as u8]
        } else {
            let bytes = (payload.len() as u64).to_be_bytes();
            let start = bytes.iter().position(|byte| *byte != 0).unwrap();
            let mut prefix = vec![offset + 55 + (8 - start) as u8];
            prefix.extend(&bytes[start..]);
            prefix
        };
        result.extend(payload);
        result
    }
    fn expected_native() -> Item {
        l(vec![
            b(b"rustaxa-final-chain-constructor-configuration"),
            n(1, 2),
            n(1, 8),
            n(2, 8),
            b(&[3; 32]),
            n(1, 1),
            n(5, 8),
            l(vec![
                l(vec![b(&[1; 20]), b(&[2])]),
                l(vec![b(&[3; 20]), b(&[])]),
            ]),
            l(vec![
                l(vec![
                    b(&[4; 20]),
                    b(&[5; 32]),
                    b(&[6]),
                    l(vec![
                        l(vec![b(&[7; 20]), b(&[8])]),
                        l(vec![b(&[9; 20]), b(&[10])]),
                    ]),
                    b(&[11; 20]),
                    n(12, 2),
                    b(b"description"),
                    b(b"endpoint"),
                ]),
                l(vec![
                    b(&[13; 20]),
                    b(&[14; 32]),
                    b(&[]),
                    l(vec![]),
                    b(&[0; 20]),
                    n(0, 2),
                    b(&[]),
                    b(&[]),
                ]),
            ]),
            l(vec![
                n(1, 32),
                n(2, 32),
                n(3, 32),
                n(4, 32),
                n(5, 2),
                n(6, 4),
                n(7, 8),
                n(8, 8),
            ]),
            l(vec![
                n(1, 4),
                n(2, 8),
                n(3, 8),
                n(4, 8),
                n(5, 8),
                n(6, 8),
                n(7, 8),
                l(vec![
                    l(vec![b(&[11; 20]), b(&[12; 20]), n(13, 32)]),
                    l(vec![b(&[14; 20]), b(&[15; 20]), n(16, 32)]),
                ]),
                n(9, 2),
                n(10, 2),
                n(11, 2),
                n(12, 4),
                n(13, 8),
                n(14, 8),
                n(15, 8),
                l(vec![n(16, 32)]),
                n(17, 32),
                n(18, 32),
                n(19, 8),
                n(20, 8),
                n(21, 8),
                n(22, 8),
                l(vec![
                    l(vec![n(24, 8), n(25, 4)]),
                    l(vec![n(26, 8), n(27, 4)]),
                ]),
            ]),
            l(vec![n(6, 32)]),
            l(vec![n(0, 1), n(0, 1)]),
        ])
    }
    fn expected_root() -> Vec<u8> {
        encode(&l(vec![
            b(b"rustaxa-execution-configuration"),
            n(1, 2),
            n(9, 8),
            b(&[10; 32]),
            b(&[1; 32]),
            n(100, 8),
            expected_native(),
            l(vec![b(&[17; 20]), n(10, 8), n(10_000_000, 8)]),
            l(vec![
                n(100, 4),
                n(19, 8),
                n(100, 4),
                n(1500, 4),
                n(500, 4),
                n(10, 4),
                n(10, 4),
                n(400, 4),
                n(12, 4),
            ]),
            l(vec![n(1_000_000, 8), n(14, 8), n(1_000_000, 8)]),
        ]))
    }

    #[test]
    fn execution_configuration_canonical_schema() {
        let (seed, config) = fixture();
        let value = root(&seed, &config).unwrap();
        let expected = expected_root();
        assert_eq!(
            seed.fixed(CorrectionStage::Historical).canonical_bytes(),
            encode(&expected_native())
        );
        assert_eq!(value.canonical_bytes(), expected);
        let mut digest = [0; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&expected);
        hasher.finalize(&mut digest);
        assert_eq!(value.digest(), digest);
        let record = rlp::Rlp::new(value.canonical_bytes());
        assert_eq!(record.item_count().unwrap(), 10);
        assert_eq!(record.at(6).unwrap().item_count().unwrap(), 13);
        assert_eq!(
            record.at(6).unwrap().at(9).unwrap().item_count().unwrap(),
            8
        );
        assert_eq!(
            record.at(6).unwrap().at(10).unwrap().item_count().unwrap(),
            23
        );
        let mut covered = Vec::new();
        macro_rules! native_change {
            ($name:literal,$change:expr) => {{
                let mut changed = seed.clone();
                ($change)(&mut changed);
                let actual = root(&changed, &config);
                match actual {
                    Ok(actual) => {
                        assert_ne!(
                            actual.canonical_bytes(),
                            value.canonical_bytes(),
                            "{}",
                            $name
                        );
                        assert_ne!(actual.digest(), value.digest(), "{}", $name);
                    }
                    Err(ConfigurationUnavailable::Conflict(_)) => {}
                    Err(other) => panic!("{other:?}"),
                }
                covered.push($name);
            }};
        }
        native_change!(
            "dpos.eligibility_balance_threshold",
            |v: &mut NativeConfigurationSeed| v.input.dpos.eligibility_balance_threshold =
                amount(99)
        );
        native_change!(
            "dpos.vote_eligibility_balance_step",
            |v: &mut NativeConfigurationSeed| v.input.dpos.vote_eligibility_balance_step =
                amount(99)
        );
        native_change!(
            "dpos.validator_maximum_stake",
            |v: &mut NativeConfigurationSeed| v.input.dpos.validator_maximum_stake = amount(99)
        );
        native_change!("dpos.minimum_deposit", |v: &mut NativeConfigurationSeed| {
            v.input.dpos.minimum_deposit = amount(99)
        });
        native_change!(
            "dpos.commission_change_delta",
            |v: &mut NativeConfigurationSeed| v.input.dpos.commission_change_delta = 99
        );
        native_change!(
            "dpos.commission_change_frequency",
            |v: &mut NativeConfigurationSeed| v.input.dpos.commission_change_frequency = 99
        );
        native_change!(
            "dpos.delegation_delay",
            |v: &mut NativeConfigurationSeed| v.input.dpos.delegation_delay = 99
        );
        native_change!(
            "dpos.dag_vdf_sortition_total_vote_count_until_period",
            |v: &mut NativeConfigurationSeed| v
                .input
                .dpos
                .dag_vdf_sortition_total_vote_count_until_period =
                99.into()
        );
        native_change!(
            "rewards.committee_size",
            |v: &mut NativeConfigurationSeed| v.rewards.committee_size = 99
        );
        native_change!(
            "rewards.magnolia_period",
            |v: &mut NativeConfigurationSeed| v.rewards.magnolia_period = 99.into()
        );
        native_change!(
            "rewards.phalaenopsis_period",
            |v: &mut NativeConfigurationSeed| v.rewards.phalaenopsis_period = 99.into()
        );
        native_change!(
            "rewards.aspen_part_one_period",
            |v: &mut NativeConfigurationSeed| v.rewards.aspen_part_one_period = 99.into()
        );
        native_change!(
            "rewards.fix_claim_all_block_num",
            |v: &mut NativeConfigurationSeed| v.rewards.fix_claim_all_block_num = 99.into()
        );
        native_change!(
            "rewards.fix_redelegate_block_num",
            |v: &mut NativeConfigurationSeed| v.rewards.fix_redelegate_block_num = 99.into()
        );
        native_change!(
            "rewards.aspen_part_two_period",
            |v: &mut NativeConfigurationSeed| v.rewards.aspen_part_two_period = 99.into()
        );
        native_change!(
            "rewards.redelegations",
            |v: &mut NativeConfigurationSeed| v.rewards.redelegations = vec![]
        );
        native_change!(
            "rewards.max_block_author_reward_percent",
            |v: &mut NativeConfigurationSeed| v.rewards.max_block_author_reward_percent = 99
        );
        native_change!(
            "rewards.dag_proposers_reward_percent",
            |v: &mut NativeConfigurationSeed| v.rewards.dag_proposers_reward_percent = 99
        );
        native_change!(
            "rewards.yield_percentage",
            |v: &mut NativeConfigurationSeed| v.rewards.yield_percentage = 99
        );
        native_change!(
            "rewards.dpos_blocks_per_year",
            |v: &mut NativeConfigurationSeed| v.rewards.dpos_blocks_per_year = 99
        );
        native_change!(
            "rewards.dpos_delegation_locking_period",
            |v: &mut NativeConfigurationSeed| v.rewards.dpos_delegation_locking_period = 99
        );
        native_change!(
            "rewards.cornus_period",
            |v: &mut NativeConfigurationSeed| v.rewards.cornus_period = 99.into()
        );
        native_change!(
            "rewards.cornus_delegation_locking_period",
            |v: &mut NativeConfigurationSeed| v.rewards.cornus_delegation_locking_period = 99
        );
        native_change!(
            "rewards.genesis_balance_sum",
            |v: &mut NativeConfigurationSeed| v.rewards.genesis_balance_sum = None
        );
        native_change!(
            "rewards.aspen_max_supply",
            |v: &mut NativeConfigurationSeed| v.rewards.aspen_max_supply = amount(99)
        );
        native_change!(
            "rewards.aspen_generated_rewards",
            |v: &mut NativeConfigurationSeed| v.rewards.aspen_generated_rewards = amount(99)
        );
        native_change!("rewards.cacti_period", |v: &mut NativeConfigurationSeed| {
            v.rewards.cacti_period = 99.into()
        });
        native_change!(
            "rewards.cacti_delegation_locking_period",
            |v: &mut NativeConfigurationSeed| v.rewards.cacti_delegation_locking_period = 99
        );
        native_change!(
            "rewards.magnolia_jail_time",
            |v: &mut NativeConfigurationSeed| v.rewards.magnolia_jail_time = 99
        );
        native_change!(
            "rewards.cacti_jail_time",
            |v: &mut NativeConfigurationSeed| v.rewards.cacti_jail_time = 99
        );
        native_change!(
            "rewards.rewards_distribution_frequency",
            |v: &mut NativeConfigurationSeed| v.rewards.rewards_distribution_frequency = vec![]
        );
        native_change!("block_gas_limit", |v: &mut NativeConfigurationSeed| {
            v.input.block_gas_limit = 99.into();
        });
        native_change!("genesis_timestamp", |v: &mut NativeConfigurationSeed| {
            v.input.genesis_timestamp = 99;
        });
        native_change!("genesis_root", |v: &mut NativeConfigurationSeed| {
            v.input.genesis_root = H256([99; 32]);
        });
        native_change!("enforce", |v: &mut NativeConfigurationSeed| {
            v.input.enforce = false;
        });
        native_change!("ficus", |v: &mut NativeConfigurationSeed| {
            v.input.ficus = 99.into();
        });
        native_change!("original_sum", |v: &mut NativeConfigurationSeed| {
            v.input.original_sum = None;
        });
        native_change!("original_sum_value", |v: &mut NativeConfigurationSeed| {
            v.input.original_sum = Some(amount(99));
        });
        native_change!("accounts", |v: &mut NativeConfigurationSeed| {
            v.input.accounts.clear();
        });
        native_change!("account_address", |v: &mut NativeConfigurationSeed| {
            v.input.accounts[0].0 = [99; 20];
        });
        native_change!("account_balance", |v: &mut NativeConfigurationSeed| {
            v.input.accounts[0].1 = vec![99];
        });
        native_change!("validators", |v: &mut NativeConfigurationSeed| {
            v.input.validators.clear();
        });
        native_change!("validator_address", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].address = [99; 20];
        });
        native_change!("validator_vrf", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].vrf_key = [99; 32];
        });
        native_change!("validator_stake", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].total_stake = vec![99];
        });
        native_change!("delegations", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].delegations.clear();
        });
        native_change!("delegator", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].delegations[0].0 = [99; 20];
        });
        native_change!("delegated_stake", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].delegations[0].1 = vec![99];
        });
        native_change!("owner", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].metadata.owner = [99; 20];
        });
        native_change!("commission", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].metadata.commission = 99;
        });
        native_change!("description", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].metadata.description = "changed".into();
        });
        native_change!("endpoint", |v: &mut NativeConfigurationSeed| {
            v.input.validators[0].metadata.endpoint = "changed".into();
        });
        native_change!(
            "redelegation_validator",
            |v: &mut NativeConfigurationSeed| {
                v.rewards.redelegations[0].validator = [99; 20];
            }
        );
        native_change!(
            "redelegation_delegator",
            |v: &mut NativeConfigurationSeed| {
                v.rewards.redelegations[0].delegator = [99; 20];
            }
        );
        native_change!("redelegation_amount", |v: &mut NativeConfigurationSeed| {
            v.rewards.redelegations[0].amount = amount(99);
        });
        native_change!("frequency_period", |v: &mut NativeConfigurationSeed| {
            v.rewards.rewards_distribution_frequency[0].0 = 99.into();
        });
        native_change!("frequency_value", |v: &mut NativeConfigurationSeed| {
            v.rewards.rewards_distribution_frequency[0].1 = 99;
        });
        native_change!("effective_sum_value", |v: &mut NativeConfigurationSeed| {
            v.rewards.genesis_balance_sum = Some(amount(99));
        });
        assert_eq!(covered.len(), 58);
        macro_rules! root_change {
            ($name:literal,$change:expr,$conflict:expr) => {{
                let mut changed = config.clone();
                ($change)(&mut changed);
                let actual = root(&seed, &changed);
                if let Some(field) = $conflict {
                    assert_eq!(actual, Err(ConfigurationUnavailable::Conflict(field)));
                } else {
                    let actual = actual.unwrap();
                    assert_ne!(
                        actual.canonical_bytes(),
                        value.canonical_bytes(),
                        "{}",
                        $name
                    );
                    assert_ne!(actual.digest(), value.digest(), "{}", $name);
                }
            }};
        }
        root_change!(
            "chain_id",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.network_identity.chain_id = 99;
            },
            Some("chain_id")
        );
        root_change!(
            "full_genesis",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.network_identity.genesis_hash = [99; 32];
            },
            Some("full_genesis")
        );
        root_change!(
            "dag_genesis",
            |v: &mut ConsensusApplicationConfig| {
                v.dag_transaction.dag.genesis_hash = H256([99; 32]);
            },
            None::<&str>
        );
        root_change!(
            "max_levels",
            |v: &mut ConsensusApplicationConfig| {
                v.dag_transaction.dag.max_levels_per_period = 99;
            },
            None::<&str>
        );
        root_change!(
            "genesis_lambda",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.genesis_lambda_ms = 99;
            },
            None::<&str>
        );
        root_change!(
            "cacti",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.cacti_block = 99;
            },
            Some("cacti_period")
        );
        root_change!(
            "lambda_min",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.process_synced_policy.lambda_min_ms = 99;
            },
            None::<&str>
        );
        root_change!(
            "lambda_max",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.cacti_lambda_max_ms = 99;
            },
            None::<&str>
        );
        root_change!(
            "lambda_default",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.cacti_lambda_default_ms = 99;
            },
            None::<&str>
        );
        root_change!(
            "lambda_interval",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.process_synced_policy.lambda_change_interval = 99;
            },
            None::<&str>
        );
        root_change!(
            "lambda_change",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.process_synced_policy.lambda_change_ms = 99;
            },
            None::<&str>
        );
        root_change!(
            "consensus_delay",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.process_synced_policy.consensus_delay_ms = 99;
            },
            None::<&str>
        );
        root_change!(
            "blocks_per_year",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.process_synced_policy.dpos_blocks_per_year = 99;
            },
            Some("dpos_blocks_per_year")
        );
        root_change!(
            "pillar_interval",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.pillar_blocks_interval = 99;
            },
            None::<&str>
        );
        root_change!(
            "default_gas",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.default_pbft_gas_limit = 99;
            },
            Some("proposer_default_pbft_gas_limit")
        );
        root_change!(
            "cornus_period",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.cornus_activation_period = 99;
            },
            Some("cornus_period")
        );
        root_change!(
            "cornus_gas",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.cornus_pbft_gas_limit = 99;
            },
            Some("proposer_cornus_pbft_gas_limit")
        );
        root_change!(
            "magnolia",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.magnolia_activation_period = 99;
            },
            Some("magnolia_period")
        );
        root_change!(
            "committee",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.committee_size = 99;
            },
            Some("committee_size")
        );
        root_change!(
            "ficus",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.ficus_activation_period = 99;
            },
            Some("ficus_period")
        );
        root_change!(
            "synced_chain",
            |v: &mut ConsensusApplicationConfig| {
                v.pbft.process_synced_policy.chain_id = 99;
            },
            Some("chain_id")
        );
        root_change!(
            "proposer_default",
            |v: &mut ConsensusApplicationConfig| {
                v.dag_proposer.default_pbft_gas_limit = 99;
            },
            Some("proposer_default_pbft_gas_limit")
        );
        root_change!(
            "proposer_cornus",
            |v: &mut ConsensusApplicationConfig| {
                v.dag_proposer.cornus_activation_period = 99;
            },
            Some("proposer_cornus_period")
        );
        root_change!(
            "proposer_cornus_gas",
            |v: &mut ConsensusApplicationConfig| {
                v.dag_proposer.cornus_pbft_gas_limit = 99;
            },
            Some("proposer_cornus_pbft_gas_limit")
        );
        let native = seed.fixed(CorrectionStage::Historical);
        let changed =
            ExecutionConfiguration::compose(H256([10; 32]), &native, &config, [99; 20]).unwrap();
        assert_ne!(changed.digest(), value.digest());
        let mut changed = config.clone();
        changed.pbft.network_identity.genesis_hash = [99; 32];
        let full =
            ExecutionConfiguration::compose(H256([99; 32]), &native, &changed, [17; 20]).unwrap();
        assert_ne!(full.digest(), value.digest());
        changed = config.clone();
        changed.pbft.network_identity.chain_id = 99;
        changed.pbft.process_synced_policy.chain_id = 99;
        assert_ne!(root(&seed, &changed).unwrap().digest(), value.digest());
        let mut policies = Vec::new();
        for (tag, stage) in [
            CorrectionStage::Historical,
            CorrectionStage::TransientRollback,
            CorrectionStage::SingleSelfDestructRefund,
            CorrectionStage::CheckedReturnDataCopy,
            CorrectionStage::BoundedFalconAbi,
            CorrectionStage::SlashingHardError,
        ]
        .into_iter()
        .enumerate()
        {
            let native = seed.fixed(stage);
            let policy = rlp::Rlp::new(native.canonical_bytes()).at(12).unwrap();
            assert_eq!(policy.item_count().unwrap(), 2);
            assert_eq!(policy.at(0).unwrap().data().unwrap(), &[0]);
            assert_eq!(policy.at(1).unwrap().data().unwrap(), &[tag as u8]);
            policies.push(
                ExecutionConfiguration::compose(H256([10; 32]), &native, &config, [17; 20])
                    .unwrap(),
            );
        }
        for activation in [None, Some(0.into()), Some(1.into()), Some(u64::MAX.into())] {
            let native = seed.activation(activation);
            let policy = rlp::Rlp::new(native.canonical_bytes()).at(12).unwrap();
            assert_eq!(policy.at(0).unwrap().data().unwrap(), &[1]);
            let payload = policy.at(1).unwrap();
            assert_eq!(
                payload.item_count().unwrap(),
                usize::from(activation.is_some())
            );
            if let Some(period) = activation {
                assert_eq!(
                    payload.at(0).unwrap().data().unwrap(),
                    period.as_u64().to_be_bytes()
                );
            }
            policies.push(
                ExecutionConfiguration::compose(H256([10; 32]), &native, &config, [17; 20])
                    .unwrap(),
            );
        }
        for (index, policy) in policies.iter().enumerate() {
            for other in &policies[index + 1..] {
                assert_ne!(policy.canonical_bytes(), other.canonical_bytes());
                assert_ne!(policy.digest(), other.digest());
            }
        }
    }

    #[test]
    fn execution_configuration_preserves_input_order_and_width() {
        let (seed, config) = fixture();
        let capture = |entries: Vec<([u8; 20], Account)>| {
            NativeConfigurationInput::capture(
                seed.input.block_gas_limit,
                seed.input.genesis_timestamp,
                seed.input.genesis_root,
                seed.input.enforce,
                seed.input.ficus,
                &entries.into_iter().collect(),
                &seed.input.validators,
                &seed.input.dpos,
                seed.input.original_sum,
            )
            .finish(&seed.rewards)
        };
        let distinct = vec![([1; 20], account(&[2])), ([3; 20], account(&[]))];
        let mut reversed = distinct.clone();
        reversed.reverse();
        assert_eq!(
            root(&capture(distinct), &config),
            root(&capture(reversed), &config)
        );
        for value in [0u8, 2] {
            let minimal = if value == 0 { vec![] } else { vec![value] };
            let mut fixed = vec![0; 32];
            fixed[31] = value;
            let minimal_account = account(&minimal);
            let fixed_account = account(&fixed);
            let left = capture(vec![([1; 20], minimal_account.clone())]);
            let right = capture(vec![([1; 20], fixed_account.clone())]);
            assert_eq!(left.input.accounts.len(), 1);
            assert_eq!(right.input.accounts.len(), 1);
            assert_ne!(
                root(&left, &config).unwrap(),
                root(&right, &config).unwrap()
            );
            let duplicate = capture(vec![
                ([1; 20], fixed_account.clone()),
                ([1; 20], minimal_account.clone()),
            ]);
            let reverse = capture(vec![([1; 20], minimal_account), ([1; 20], fixed_account)]);
            assert_eq!(root(&duplicate, &config), root(&left, &config));
            assert_eq!(root(&reverse, &config), root(&right, &config));
        }
        let values = capture(vec![([1; 20], account(&[1])), ([1; 20], account(&[2]))]);
        let reversed = capture(vec![([1; 20], account(&[2])), ([1; 20], account(&[1]))]);
        assert_ne!(root(&values, &config), root(&reversed, &config));
        let original = root(&seed, &config).unwrap();
        let mut variants = Vec::new();
        let mut changed = seed.clone();
        changed.input.validators.reverse();
        variants.push(changed);
        let mut changed = seed.clone();
        changed.input.validators[0].delegations.reverse();
        variants.push(changed);
        let mut changed = seed.clone();
        changed.rewards.redelegations.reverse();
        variants.push(changed);
        let mut changed = seed.clone();
        changed.rewards.rewards_distribution_frequency.reverse();
        variants.push(changed);
        let mut changed = seed.clone();
        changed
            .input
            .validators
            .push(changed.input.validators[0].clone());
        variants.push(changed);
        let mut changed = seed.clone();
        changed.input.validators[0]
            .delegations
            .push(([7; 20], vec![8]));
        variants.push(changed);
        for value in [0u8, 6] {
            let minimal = if value == 0 { vec![] } else { vec![value] };
            let mut fixed = vec![0; 32];
            fixed[31] = value;
            for delegation in [false, true] {
                let mut left = seed.clone();
                let mut right = seed.clone();
                if delegation {
                    left.input.validators[0].delegations[0].1 = minimal.clone();
                    right.input.validators[0].delegations[0].1 = fixed.clone();
                } else {
                    left.input.validators[0].total_stake = minimal.clone();
                    right.input.validators[0].total_stake = fixed.clone();
                }
                assert_ne!(root(&left, &config), root(&right, &config));
            }
        }
        for changed in variants {
            let actual = root(&changed, &config).unwrap();
            assert_ne!(actual.canonical_bytes(), original.canonical_bytes());
            assert_ne!(actual.digest(), original.digest());
        }
    }
    include!("execution_configuration_preparation_tests.rs");
}
