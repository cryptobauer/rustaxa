#ifdef RUSTAXA_ENABLE

#include "consensus/consensus_application.hpp"

#include <algorithm>
#include <array>
#include <cstddef>
#include <stdexcept>
#include <utility>

#include "common/constants.hpp"
#include "config/config.hpp"
#include "config/version.hpp"
#include "final_chain/final_chain.hpp"

namespace taraxa {
namespace {

template <typename Value>
std::array<uint8_t, 32> toBridgeU256(const Value& value) {
  std::array<uint8_t, 32> out{};
  const auto bytes = dev::toBigEndian(value);
  if (bytes.size() > out.size()) {
    throw std::runtime_error("u256 value exceeds 32 bytes");
  }
  std::copy(bytes.begin(), bytes.end(), out.begin() + static_cast<std::ptrdiff_t>(out.size() - bytes.size()));
  return out;
}

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

SharedConsensusApplication createConsensusApplication(const FullNodeConfig& config) {
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

  return std::make_shared<ConsensusApplication>(rustaxa::create_consensus_application(
      config.db_path.string(), TARAXA_DB_MAJOR_VERSION, TARAXA_DB_MINOR_VERSION, config.genesis.genesisHash().asArray(),
      config.genesis.dag_genesis_block.getHash().asArray(), config.dag_expiry_limit, config.max_levels_per_period,
      sortitionRuntimeConfigFromNodeConfig(config), rustaxa::TransactionQueueConfig{config.transactions_pool_size},
      gasPricerConfigFromNodeConfig(config), config.propose_dag_gas_limit, std::move(pbft_config),
      std::move(signing_identities), config.genesis.pbft.gas_limit, config.genesis.dag_genesis_block.getTimestamp(),
      final_chain::makeGenesisAccounts(config.genesis.state), final_chain::makeGenesisValidators(config.genesis.state),
      final_chain::makeGenesisDposConfig(config.genesis.state.dpos,
                                         config.genesis.state.hardforks.magnolia_hf.block_num),
      final_chain::makeFinalChainRewardsConfig(config)));
}

ConsensusApplication::ConsensusApplication(rust::Box<rustaxa::BridgeConsensusApplication> service)
    : service_(std::move(service)) {}

ConsensusApplication::~ConsensusApplication() = default;

ConsensusRuntimeStatus ConsensusApplication::runtimeStatus() const {
  const auto status = rustaxa::consensus_application_live_status(service());
  return ConsensusRuntimeStatus{status.period,
                                static_cast<PbftRound>(status.round),
                                static_cast<PbftStep>(status.step),
                                status.finalized_chain_size,
                                status.syncing_period,
                                static_cast<size_t>(status.sync_queue_size)};
}

std::optional<uint64_t> ConsensusApplication::currentNodeVotesCount() const {
  const auto status = rustaxa::consensus_application_live_status(service());
  if (!status.has_current_node_votes) {
    return std::nullopt;
  }
  return status.current_node_votes;
}

std::optional<uint64_t> ConsensusApplication::currentDposTotalVotesCount() const {
  const auto status = rustaxa::consensus_application_live_status(service());
  if (!status.has_total_eligible_votes) {
    return std::nullopt;
  }
  return status.total_eligible_votes;
}

}  // namespace taraxa

#endif  // RUSTAXA_ENABLE
