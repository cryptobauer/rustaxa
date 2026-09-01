#ifdef RUSTAXA_ENABLE

#include "consensus/consensus_application.hpp"

#include <algorithm>
#include <array>
#include <cstddef>
#include <iterator>
#include <stdexcept>
#include <utility>

#include "common/constants.hpp"
#include "config/config.hpp"
#include "config/version.hpp"
#include "consensus/external_evm_state_owner.hpp"

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

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& value) {
  rust::Vec<uint8_t> out;
  out.reserve(value.size());
  std::copy(value.begin(), value.end(), std::back_inserter(out));
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
  std::error_code directory_error;
  fs::create_directories(config.db_path, directory_error);
  if (directory_error) {
    throw std::runtime_error("FINAL_CHAIN_STATE_DB_PARENT_CREATE_FAILED: " + directory_error.message());
  }
  auto external_evm_state = std::make_shared<ExternalEvmStateOwner>(config);
  const auto state_api_descriptor = external_evm_state->lastCommittedStateDescriptor();
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

  auto application = std::make_shared<ConsensusApplication>(
      rustaxa::create_consensus_application(
          config.db_path.string(), TARAXA_DB_MAJOR_VERSION, TARAXA_DB_MINOR_VERSION,
          config.genesis.genesisHash().asArray(), config.genesis.dag_genesis_block.getHash().asArray(),
          config.dag_expiry_limit, config.max_levels_per_period, sortitionRuntimeConfigFromNodeConfig(config),
          rustaxa::TransactionQueueConfig{config.transactions_pool_size}, gasPricerConfigFromNodeConfig(config),
          config.propose_dag_gas_limit, std::move(pbft_config), std::move(signing_identities),
          std::move(dag_proposer_config), config.genesis.pbft.gas_limit,
          config.genesis.dag_genesis_block.getTimestamp(), state_api_descriptor.blk_num,
          state_api_descriptor.state_root.asArray(),
          config.genesis.state.hardforks.ficus_hf.bridge_contract_address.asArray(),
          makeGenesisAccounts(config.genesis.state), makeGenesisValidators(config.genesis.state),
          makeGenesisDposConfig(config.genesis.state.dpos, config.genesis.state.hardforks.magnolia_hf.block_num),
          makeFinalChainRewardsConfig(config)),
      external_evm_state);
  external_evm_state->bindApplication(application);
  application->recoverExternalEvmPendingPublication(application);
  return application;
}

ConsensusApplication::ConsensusApplication(rust::Box<rustaxa::BridgeConsensusApplication> service,
                                           std::shared_ptr<ExternalEvmStateOwner> external_evm_state)
    : service_(std::move(service)),
      query_client_(std::make_shared<rust::Box<rustaxa::BridgeConsensusQueryApi>>(
          rustaxa::create_consensus_query_api(*service_))),
      external_evm_state_(std::move(external_evm_state)) {
  if (!external_evm_state_) throw std::invalid_argument("ConsensusApplication requires ExternalEvmStateOwner");
}

ConsensusApplication::~ConsensusApplication() = default;

PublicTransactionSubmissionResult ConsensusApplication::submitTransaction(const SharedTransaction& transaction,
                                                                          const FullNodeConfig& config) const {
  if (!transaction) {
    throw std::invalid_argument("PUBLIC_TRANSACTION_MISSING");
  }

  const auto query = queryClient();
  const auto last_block_number = (*query)->consensus_query_final_chain_last_block_number();
  rustaxa::PublicTransactionSubmissionRequest request;
  request.transaction_rlp = toBridgeBytes(transaction->rlp());
  request.expected_chain_id = config.genesis.chain_id;
  request.maximum_gas_limit = config.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
  request.minimum_gas_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  request.last_block_number = last_block_number;
  request.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(last_block_number);

  auto report = rustaxa::consensus_application_submit_transaction(service(), std::move(request));
  auto result =
      PublicTransactionSubmissionResult{trx_hash_t(report.transaction_hash.data(), trx_hash_t::ConstructFromPointer),
                                        report.accepted, std::string(report.message), report.transaction_observed};
  if (result.transaction_observed) {
    transaction_observed_.emit(result.transaction_hash);
  }
  return result;
}

std::optional<state_api::Account> ConsensusApplication::getAccount(const addr_t& address,
                                                                   std::optional<EthBlockNumber> block_number) const {
  return external_evm_state_->account(address, block_number);
}

h256 ConsensusApplication::getAccountStorage(const addr_t& address, const u256& key,
                                             std::optional<EthBlockNumber> block_number) const {
  return external_evm_state_->accountStorage(address, key, block_number);
}

bytes ConsensusApplication::getCode(const addr_t& address, std::optional<EthBlockNumber> block_number) const {
  return external_evm_state_->code(address, block_number);
}

state_api::ExecutionResult ConsensusApplication::call(const state_api::EVMTransaction& transaction,
                                                      std::optional<EthBlockNumber> block_number) const {
  return external_evm_state_->call(transaction, block_number);
}

std::string ConsensusApplication::trace(std::vector<state_api::EVMTransaction> state_transactions,
                                        std::vector<state_api::EVMTransaction> transactions,
                                        EthBlockNumber block_number, std::optional<state_api::Tracing> params) const {
  return external_evm_state_->trace(std::move(state_transactions), std::move(transactions), block_number, params);
}

void ConsensusApplication::pruneFinalChain(EthBlockNumber block_number) const {
  external_evm_state_->prune(block_number);
}

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

void ConsensusApplication::pruneLightHistory(PbftPeriod end_period_exclusive, uint64_t dag_level_to_keep,
                                             bool live_cleanup, uint64_t non_block_periods_to_keep) const {
  rustaxa::LightHistoryPruneRequest request;
  request.end_period_exclusive = end_period_exclusive;
  // Legacy DeleteRange used `dag_level_to_keep - 1` as its exclusive end. Preserve that retained-level boundary.
  request.first_retained_dag_level = dag_level_to_keep == 0 ? 0 : dag_level_to_keep - 1;
  request.live_cleanup = live_cleanup;
  request.non_block_periods_to_keep = non_block_periods_to_keep;
  rustaxa::consensus_application_prune_light_history(service(), request);
}

}  // namespace taraxa

#endif  // RUSTAXA_ENABLE
