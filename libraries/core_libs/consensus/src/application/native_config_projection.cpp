#ifdef RUSTAXA_ENABLE
#include "consensus/native_config_projection.hpp"
#include "common/constants.hpp"
#include "config/version.hpp"
#include <algorithm>
#include <iterator>
#include <utility>

namespace taraxa {
namespace {
std::array<uint8_t, 32> toArray(const h256& value) { return value.asArray(); }
std::array<uint8_t, 20> toArray(const addr_t& value) { return value.asArray(); }
rust::Vec<uint8_t> toBigEndianBytes(const u256& value) {
  const auto bytes = dev::toBigEndian(value);
  rust::Vec<uint8_t> result;
  result.reserve(bytes.size());
  std::copy(bytes.begin(), bytes.end(), std::back_inserter(result));
  return result;
}
using production_config_detail::toBridgeU256;
rustaxa::SortitionRuntimeConfig sortitionRuntimeConfigFromNodeConfig(const FullNodeConfig& config) {
  const auto& sortition = config.genesis.sortition;
  rustaxa::SortitionRuntimeConfig bridge_config;
  bridge_config.threshold_upper = sortition.vrf.threshold_upper;
  bridge_config.difficulty_min = sortition.vdf.difficulty_min;
  bridge_config.difficulty_max = sortition.vdf.difficulty_max;
  bridge_config.difficulty_stale = sortition.vdf.difficulty_stale;
  bridge_config.lambda_bound = sortition.vdf.lambda_bound;
  bridge_config.changes_count_for_average = sortition.changes_count_for_average;
  bridge_config.dag_efficiency_target_low = sortition.dag_efficiency_targets.first;
  bridge_config.dag_efficiency_target_high = sortition.dag_efficiency_targets.second;
  bridge_config.changing_interval = sortition.changing_interval;
  bridge_config.computation_interval = sortition.computation_interval;
  return bridge_config;
}

rustaxa::GasPricerConfig gasPricerConfigFromNodeConfig(const FullNodeConfig& config) {
  rustaxa::GasPricerConfig bridge_config;
  bridge_config.percentile = config.genesis.gas_price.percentile;
  bridge_config.minimum_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  bridge_config.history_blocks = config.genesis.gas_price.blocks;
  bridge_config.is_light_node = config.is_light_node;
  bridge_config.blocks_gas_pricer = config.blocks_gas_pricer;
  return bridge_config;
}
}  // namespace
rust::Vec<rustaxa::GenesisAccount> makeGenesisAccounts(const state_api::Config& config) {
  auto effective_balances = config.initial_balances;
  for (const auto& validator : config.dpos.initial_validators) {
    for (const auto& [delegator, amount] : validator.delegations) effective_balances[delegator] -= amount;
  }
  rust::Vec<rustaxa::GenesisAccount> accounts;
  accounts.reserve(effective_balances.size());
  for (const auto& [address, balance] : effective_balances) {
    rustaxa::GenesisAccount account;
    account.address = toArray(address);
    account.balance = toBigEndianBytes(balance);
    accounts.push_back(std::move(account));
  }
  return accounts;
}

rust::Vec<rustaxa::GenesisValidator> makeGenesisValidators(const state_api::Config& config) {
  rust::Vec<rustaxa::GenesisValidator> validators;
  validators.reserve(config.dpos.initial_validators.size());
  for (const auto& validator : config.dpos.initial_validators) {
    rustaxa::GenesisValidator converted;
    converted.address = toArray(validator.address);
    converted.owner = toArray(validator.owner);
    converted.vrf_key = toArray(validator.vrf_key);
    converted.commission = validator.commission;
    converted.description = rust::String(validator.description);
    converted.endpoint = rust::String(validator.endpoint);
    u256 total_stake = 0;
    for (const auto& [_, amount] : validator.delegations) total_stake += amount;
    converted.total_stake = toBigEndianBytes(total_stake);
    converted.delegations.reserve(validator.delegations.size());
    for (const auto& [delegator, amount] : validator.delegations) {
      rustaxa::GenesisDelegation delegation;
      delegation.delegator = toArray(delegator);
      delegation.stake = toBigEndianBytes(amount);
      converted.delegations.push_back(std::move(delegation));
    }
    validators.push_back(std::move(converted));
  }
  return validators;
}

rustaxa::GenesisDposConfig makeGenesisDposConfig(const state_api::DPOSConfig& config,
                                                 uint64_t dag_vdf_sortition_total_vote_count_until_period) {
  rustaxa::GenesisDposConfig result;
  result.eligibility_balance_threshold = toBigEndianBytes(config.eligibility_balance_threshold);
  result.vote_eligibility_balance_step = toBigEndianBytes(config.vote_eligibility_balance_step);
  result.validator_maximum_stake = toBigEndianBytes(config.validator_maximum_stake);
  result.minimum_deposit = toBigEndianBytes(config.minimum_deposit);
  result.commission_change_delta = config.commission_change_delta;
  result.commission_change_frequency = config.commission_change_frequency;
  result.delegation_delay = config.delegation_delay;
  result.dag_vdf_sortition_total_vote_count_until_period = dag_vdf_sortition_total_vote_count_until_period;
  return result;
}

rustaxa::FinalChainRewardsConfig makeFinalChainRewardsConfig(const FullNodeConfig& config) {
  rustaxa::FinalChainRewardsConfig result;
  result.committee_size = config.genesis.pbft.committee_size;
  result.magnolia_period = config.genesis.state.hardforks.magnolia_hf.block_num;
  result.aspen_part_one_period = config.genesis.state.hardforks.aspen_hf.block_num_part_one;
  result.fix_claim_all_block_num = config.genesis.state.hardforks.fix_claim_all_block_num;
  result.fix_redelegate_block_num = config.genesis.state.hardforks.fix_redelegate_block_num;
  result.aspen_part_two_period = config.genesis.state.hardforks.aspen_hf.block_num_part_two;
  result.max_block_author_reward_percent = config.genesis.state.dpos.max_block_author_reward;
  result.dag_proposers_reward_percent = config.genesis.state.dpos.dag_proposers_reward;
  result.yield_percentage = config.genesis.state.dpos.yield_percentage;
  result.dpos_blocks_per_year = config.genesis.state.dpos.blocks_per_year;
  result.dpos_delegation_locking_period = config.genesis.state.dpos.delegation_locking_period;
  result.cornus_period = config.genesis.state.hardforks.cornus_hf.block_num;
  result.cornus_delegation_locking_period = config.genesis.state.hardforks.cornus_hf.delegation_locking_period;
  result.phalaenopsis_period = config.genesis.state.hardforks.phalaenopsis_hf_block_num;
  u256 genesis_balance_sum = 0;
  for (const auto& [_, balance] : config.genesis.state.initial_balances) genesis_balance_sum += balance;
  result.genesis_balance_sum = toBigEndianBytes(genesis_balance_sum);
  result.aspen_max_supply = toBigEndianBytes(config.genesis.state.hardforks.aspen_hf.max_supply);
  result.aspen_generated_rewards = toBigEndianBytes(config.genesis.state.hardforks.aspen_hf.generated_rewards);
  result.cacti_period = config.genesis.state.hardforks.cacti_hf.block_num;
  result.cacti_delegation_locking_period = config.genesis.state.hardforks.cacti_hf.delegation_locking_period;
  result.magnolia_jail_time = config.genesis.state.hardforks.magnolia_hf.jail_time;
  result.cacti_jail_time = config.genesis.state.hardforks.cacti_hf.jail_time;
  result.redelegations.reserve(config.genesis.state.hardforks.redelegations.size());
  for (const auto& redelegation : config.genesis.state.hardforks.redelegations) {
    rustaxa::RedelegationCorrection correction;
    correction.validator = toArray(redelegation.validator);
    correction.delegator = toArray(redelegation.delegator);
    correction.amount = toBigEndianBytes(redelegation.amount);
    result.redelegations.push_back(std::move(correction));
  }
  result.frequency_rules.reserve(config.genesis.state.hardforks.rewards_distribution_frequency.size());
  for (const auto& [from_period, frequency] : config.genesis.state.hardforks.rewards_distribution_frequency) {
    result.frequency_rules.push_back(rustaxa::RewardsFrequencyRule{from_period, frequency});
  }
  return result;
}

rustaxa::ConsensusApplicationBootstrapConfig makeConsensusBootstrapConfig(
    const FullNodeConfig& config, std::uint64_t committed_period,
    const std::array<std::uint8_t, 32>& committed_root) {
  rustaxa::PbftServiceConfig pbft_config{};
  pbft_config.genesis_lambda_ms = config.genesis.pbft.lambda_ms;
  pbft_config.cacti_lambda_max_ms = config.genesis.state.hardforks.cacti_hf.lambda_max;
  pbft_config.cacti_lambda_default_ms = config.genesis.state.hardforks.cacti_hf.lambda_default;
  pbft_config.cacti_block = config.genesis.state.hardforks.cacti_hf.block_num;
  pbft_config.max_exponential_lambda_ms = 60000;
  pbft_config.max_steps = 13;
  pbft_config.deadline_ms = 4 * static_cast<uint64_t>(config.genesis.pbft.lambda_ms);
  pbft_config.polling_interval_ms = 100;
  pbft_config.report_malicious_behaviour = config.report_malicious_behaviour;
  pbft_config.magnolia_activation_period = config.genesis.state.hardforks.magnolia_hf.block_num;
  pbft_config.ficus_activation_period = config.genesis.state.hardforks.ficus_hf.block_num;
  pbft_config.pillar_blocks_interval = config.genesis.state.hardforks.ficus_hf.pillar_blocks_interval;
  pbft_config.sync_level_size = config.network.sync_level_size;
  pbft_config.deep_syncing_threshold = config.network.deep_syncing_threshold;
  pbft_config.is_light_node = config.is_light_node;
  pbft_config.light_node_history = config.light_node_history;
  pbft_config.committee_size = config.genesis.pbft.committee_size;
  pbft_config.number_of_proposers = config.genesis.pbft.number_of_proposers;
  pbft_config.dag_blocks_size = config.genesis.pbft.dag_blocks_size;
  pbft_config.ghost_path_move_back = config.genesis.pbft.ghost_path_move_back;
  pbft_config.node_version_major = TARAXA_MAJOR_VERSION;
  pbft_config.node_version_minor = TARAXA_MINOR_VERSION;
  pbft_config.node_version_patch = TARAXA_PATCH_VERSION;
  pbft_config.node_version_network = TARAXA_NET_VERSION;
  pbft_config.node_version_suffix.push_back('T');
  pbft_config.lambda_min_ms = config.genesis.state.hardforks.cacti_hf.lambda_min;
  pbft_config.lambda_change_interval = config.genesis.state.hardforks.cacti_hf.lambda_change_interval;
  pbft_config.lambda_change_ms = config.genesis.state.hardforks.cacti_hf.lambda_change;
  pbft_config.consensus_delay_ms = config.genesis.state.hardforks.cacti_hf.consensus_delay;
  pbft_config.dpos_blocks_per_year = config.genesis.state.dpos.blocks_per_year;
  pbft_config.recently_finalized_factor = kRecentlyFinalizedTransactionsFactor;
  pbft_config.chain_id = config.genesis.chain_id;
  pbft_config.default_pbft_gas_limit = config.genesis.pbft.gas_limit;
  pbft_config.cornus_activation_period = config.genesis.state.hardforks.cornus_hf.block_num;
  pbft_config.cornus_pbft_gas_limit = config.genesis.state.hardforks.cornus_hf.pbft_gas_limit;

  rust::Vec<rustaxa::SigningIdentity> signing_identities;
  signing_identities.reserve(config.wallets.size());
  for (size_t wallet_index = 0; wallet_index < config.wallets.size(); ++wallet_index) {
    const auto& wallet = config.wallets[wallet_index];
    rustaxa::SigningIdentity identity{};
    identity.wallet_index = wallet_index;
    identity.address = wallet.node_addr.asArray();
    identity.node_public_key = wallet.node_pk.asArray();
    identity.vrf_public_key = wallet.vrf_pk.asArray();
    signing_identities.push_back(std::move(identity));
  }

  rustaxa::DagProposerConfig dag_proposer_config;
  dag_proposer_config.total_transaction_shards = std::max(config.genesis.dag.block_proposer.shard, uint16_t(1));
  dag_proposer_config.proposal_dag_gas_limit = config.propose_dag_gas_limit;
  dag_proposer_config.default_dag_gas_limit = config.genesis.dag.gas_limit;
  dag_proposer_config.cornus_dag_gas_limit = config.genesis.state.hardforks.cornus_hf.dag_gas_limit;

  return rustaxa::ConsensusApplicationBootstrapConfig{
          rust::String(config.db_path.string()),
          TARAXA_DB_MAJOR_VERSION,
          TARAXA_DB_MINOR_VERSION,
          config.genesis.genesisHash().asArray(),
          config.genesis.dag_genesis_block.getHash().asArray(),
          config.dag_expiry_limit,
          config.max_levels_per_period,
          sortitionRuntimeConfigFromNodeConfig(config),
          config.transactions_pool_size,
          gasPricerConfigFromNodeConfig(config),
          config.propose_dag_gas_limit,
          std::move(pbft_config),
          std::move(signing_identities),
          std::move(dag_proposer_config),
          config.genesis.pbft.gas_limit,
          config.genesis.dag_genesis_block.getTimestamp(),
          committed_period,
          committed_root,
          config.genesis.state.hardforks.ficus_hf.bridge_contract_address.asArray(),
          makeGenesisAccounts(config.genesis.state),
          makeGenesisValidators(config.genesis.state),
          makeGenesisDposConfig(config.genesis.state.dpos, config.genesis.state.hardforks.magnolia_hf.block_num),
          makeFinalChainRewardsConfig(config)};
}
}  // namespace taraxa
#endif
