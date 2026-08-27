#include <algorithm>
#include <array>
#include <chrono>
#include <cstring>
#include <stdexcept>
#include <string>
#include <thread>

#include "common/constants.hpp"
#include "common/encoding_rlp.hpp"
#include "common/encoding_solidity.hpp"
#include "consensus/consensus_host_ports.hpp"
#include "final_chain/final_chain.hpp"
#include "libdevcore/CommonData.h"
#include "rewards/block_stats.hpp"
#include "transaction/receipt.hpp"
#include "transaction/system_transaction.hpp"

namespace taraxa::final_chain {
namespace {

constexpr uint8_t kFinalChainEvmReportStatusSuccess = 0;
constexpr uint8_t kFinalChainEvmRewardsReportStatusSuccess = 0;
constexpr uint8_t kFinalChainEvmLifecycleStatusCommitted = 0;
constexpr uint8_t kFinalChainEvmLifecycleStatusRejected = 2;
constexpr uint8_t kFinalChainEvmPublicationStatusRejected = 1;

std::array<uint8_t, 32> into_bytes_array(const h256& hash) {
  std::array<uint8_t, 32> bytes{};
  std::memcpy(bytes.data(), hash.data(), bytes.size());
  return bytes;
}

std::array<uint8_t, 20> into_address_array(const addr_t& address) {
  std::array<uint8_t, 20> bytes{};
  std::memcpy(bytes.data(), address.data(), bytes.size());
  return bytes;
}

addr_t into_address(const std::array<uint8_t, 20>& address) {
  return addr_t(dev::bytes(address.begin(), address.end()));
}

rust::Vec<uint8_t> into_rust_vec(const dev::bytes& bytes) {
  rust::Vec<uint8_t> vec;
  vec.reserve(bytes.size());
  for (auto const byte : bytes) {
    vec.push_back(static_cast<uint8_t>(byte));
  }
  return vec;
}

rust::Vec<uint8_t> into_big_endian_vec(const u256& value) {
  rust::Vec<uint8_t> vec;
  auto bytes = dev::toBigEndian(value);
  vec.reserve(bytes.size());
  for (auto const byte : bytes) {
    vec.push_back(static_cast<uint8_t>(byte));
  }
  return vec;
}

// Nonces cross the Rust boundary as canonical minimal big-endian bytes. Keep
// the legacy C++ API's u256 surface intact, but never truncate a value that
// cannot be represented by that API.
rust::Vec<uint8_t> into_canonical_nonce_vec(const u256& value) {
  auto encoded = into_big_endian_vec(value);
  size_t first_nonzero = 0;
  while (first_nonzero < encoded.size() && encoded[first_nonzero] == 0) {
    ++first_nonzero;
  }
  rust::Vec<uint8_t> canonical;
  canonical.reserve(encoded.size() - first_nonzero);
  for (size_t index = first_nonzero; index < encoded.size(); ++index) {
    canonical.push_back(encoded[index]);
  }
  return canonical;
}

u256 nonce_from_canonical_vec(const rust::Vec<uint8_t>& nonce) {
  if (nonce.size() > 32) {
    throw std::runtime_error("FINAL_CHAIN_NONCE_EXCEEDS_CPP_U256");
  }
  if (!nonce.empty() && nonce[0] == 0) {
    throw std::runtime_error("FINAL_CHAIN_NONCE_NON_CANONICAL");
  }
  dev::bytes bytes;
  bytes.reserve(nonce.size());
  for (auto const byte : nonce) {
    bytes.push_back(byte);
  }
  return dev::fromBigEndian<u256>(bytes);
}

}  // namespace

rust::Vec<rustaxa::GenesisAccount> makeGenesisAccounts(const state_api::Config& config) {
  auto effective_balances = config.initial_balances;
  for (const auto& validator : config.dpos.initial_validators) {
    for (const auto& [delegator, amount] : validator.delegations) {
      effective_balances[delegator] -= amount;
    }
  }

  rust::Vec<rustaxa::GenesisAccount> accounts;
  accounts.reserve(effective_balances.size());
  for (const auto& [address, balance] : effective_balances) {
    rustaxa::GenesisAccount account;
    account.address = into_address_array(address);
    auto balance_bytes = dev::toBigEndian(balance);
    account.balance.reserve(balance_bytes.size());
    for (auto const byte : balance_bytes) {
      account.balance.push_back(static_cast<uint8_t>(byte));
    }
    accounts.push_back(std::move(account));
  }
  return accounts;
}

rust::Vec<rustaxa::GenesisValidator> makeGenesisValidators(const state_api::Config& config) {
  rust::Vec<rustaxa::GenesisValidator> validators;
  validators.reserve(config.dpos.initial_validators.size());
  for (const auto& validator : config.dpos.initial_validators) {
    rustaxa::GenesisValidator genesis_validator;
    genesis_validator.address = into_address_array(validator.address);
    genesis_validator.owner = into_address_array(validator.owner);
    genesis_validator.vrf_key = into_bytes_array(validator.vrf_key);
    genesis_validator.commission = validator.commission;
    genesis_validator.description = rust::String(validator.description);
    genesis_validator.endpoint = rust::String(validator.endpoint);
    u256 total_stake = 0;
    for (const auto& [_, amount] : validator.delegations) {
      total_stake += amount;
    }
    genesis_validator.total_stake = into_big_endian_vec(total_stake);
    genesis_validator.delegations.reserve(validator.delegations.size());
    for (const auto& [delegator, amount] : validator.delegations) {
      rustaxa::GenesisDelegation genesis_delegation;
      genesis_delegation.delegator = into_address_array(delegator);
      genesis_delegation.stake = into_big_endian_vec(amount);
      genesis_validator.delegations.push_back(std::move(genesis_delegation));
    }
    validators.push_back(std::move(genesis_validator));
  }
  return validators;
}

rustaxa::GenesisDposConfig makeGenesisDposConfig(const state_api::DPOSConfig& config,
                                                 uint64_t dag_vdf_sortition_total_vote_count_until_period) {
  rustaxa::GenesisDposConfig dpos_config;
  dpos_config.eligibility_balance_threshold = into_big_endian_vec(config.eligibility_balance_threshold);
  dpos_config.vote_eligibility_balance_step = into_big_endian_vec(config.vote_eligibility_balance_step);
  dpos_config.validator_maximum_stake = into_big_endian_vec(config.validator_maximum_stake);
  dpos_config.minimum_deposit = into_big_endian_vec(config.minimum_deposit);
  dpos_config.commission_change_delta = config.commission_change_delta;
  dpos_config.commission_change_frequency = config.commission_change_frequency;
  dpos_config.delegation_delay = config.delegation_delay;
  dpos_config.dag_vdf_sortition_total_vote_count_until_period = dag_vdf_sortition_total_vote_count_until_period;
  return dpos_config;
}

rustaxa::FinalChainRewardsConfig makeFinalChainRewardsConfig(const taraxa::FullNodeConfig& config) {
  rustaxa::FinalChainRewardsConfig rewards_config;
  rewards_config.committee_size = config.genesis.pbft.committee_size;
  rewards_config.magnolia_period = config.genesis.state.hardforks.magnolia_hf.block_num;
  rewards_config.aspen_part_one_period = config.genesis.state.hardforks.aspen_hf.block_num_part_one;
  rewards_config.fix_claim_all_block_num = config.genesis.state.hardforks.fix_claim_all_block_num;
  rewards_config.fix_redelegate_block_num = config.genesis.state.hardforks.fix_redelegate_block_num;
  rewards_config.aspen_part_two_period = config.genesis.state.hardforks.aspen_hf.block_num_part_two;
  rewards_config.max_block_author_reward_percent = config.genesis.state.dpos.max_block_author_reward;
  rewards_config.dag_proposers_reward_percent = config.genesis.state.dpos.dag_proposers_reward;
  rewards_config.yield_percentage = config.genesis.state.dpos.yield_percentage;
  rewards_config.dpos_blocks_per_year = config.genesis.state.dpos.blocks_per_year;
  rewards_config.dpos_delegation_locking_period = config.genesis.state.dpos.delegation_locking_period;
  rewards_config.cornus_period = config.genesis.state.hardforks.cornus_hf.block_num;
  rewards_config.cornus_delegation_locking_period = config.genesis.state.hardforks.cornus_hf.delegation_locking_period;
  rewards_config.phalaenopsis_period = config.genesis.state.hardforks.phalaenopsis_hf_block_num;
  u256 genesis_balance_sum = 0;
  for (const auto& [_, balance] : config.genesis.state.initial_balances) {
    genesis_balance_sum += balance;
  }
  rewards_config.genesis_balance_sum = into_big_endian_vec(genesis_balance_sum);
  rewards_config.aspen_max_supply = into_big_endian_vec(config.genesis.state.hardforks.aspen_hf.max_supply);
  rewards_config.aspen_generated_rewards =
      into_big_endian_vec(config.genesis.state.hardforks.aspen_hf.generated_rewards);
  rewards_config.cacti_period = config.genesis.state.hardforks.cacti_hf.block_num;
  rewards_config.cacti_delegation_locking_period = config.genesis.state.hardforks.cacti_hf.delegation_locking_period;
  rewards_config.magnolia_jail_time = config.genesis.state.hardforks.magnolia_hf.jail_time;
  rewards_config.cacti_jail_time = config.genesis.state.hardforks.cacti_hf.jail_time;
  rewards_config.redelegations.reserve(config.genesis.state.hardforks.redelegations.size());
  for (const auto& redelegation : config.genesis.state.hardforks.redelegations) {
    rustaxa::RedelegationCorrection correction;
    correction.validator = into_address_array(redelegation.validator);
    correction.delegator = into_address_array(redelegation.delegator);
    correction.amount = into_big_endian_vec(redelegation.amount);
    rewards_config.redelegations.push_back(std::move(correction));
  }
  rewards_config.frequency_rules.reserve(config.genesis.state.hardforks.rewards_distribution_frequency.size());
  for (const auto& [from_period, frequency] : config.genesis.state.hardforks.rewards_distribution_frequency) {
    rustaxa::RewardsFrequencyRule rule{};
    rule.from_period = from_period;
    rule.frequency = frequency;
    rewards_config.frequency_rules.push_back(rule);
  }
  return rewards_config;
}

namespace {

h256 into_h256(const rust::Vec<uint8_t>& bytes, const char* api_name) {
  if (bytes.size() != 32) {
    throw DbException("FinalChain::" + std::string(api_name) + " returned invalid hash size: expected 32, got " +
                      std::to_string(bytes.size()));
  }
  return h256(dev::bytes(bytes.begin(), bytes.end()));
}

h256 into_h256(const dev::bytes& bytes, const char* api_name) {
  if (bytes.size() != 32) {
    throw DbException("FinalChain::" + std::string(api_name) + " returned invalid hash size: expected 32, got " +
                      std::to_string(bytes.size()));
  }
  return h256(bytes);
}

std::string into_string(const rust::Vec<uint8_t>& bytes) {
  return std::string(reinterpret_cast<const char*>(bytes.data()), bytes.size());
}

dev::bytes into_bytes(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

[[noreturn]] void throw_unimplemented_final_chain_api(const char* api_name) {
  throw DbException("FinalChain::" + std::string(api_name) + " is not implemented in Rust shim mode");
}

}  // namespace

FinalChain::FinalChain(const fs::path& state_db_path, const taraxa::FullNodeConfig& config,
                       [[maybe_unused]] const addr_t& node_addr, SharedConsensusApplication consensus_application)
    : consensus_application_(std::move(consensus_application)),
      state_api_([this](auto n) { return blockHash(n).value_or(ZeroHash()); }, config.genesis.state,
                 config.opts_final_chain, {state_db_path.string()}),
      external_evm_state_api_(state_api_, state_api_mutex_),
      config_(config) {
  if (!consensus_application_) {
    throw std::invalid_argument("FinalChain requires the native consensus application root");
  }
  recoverExternalEvmPendingPublication();
  delegation_delay_ = config.genesis.state.dpos.delegation_delay;
}

FinalChain::ExternalEvmStateApiClient::ExternalEvmStateApiClient(StateAPI& state_api, std::mutex& state_api_mutex)
    : state_api_(state_api), state_api_mutex_(state_api_mutex) {}

rustaxa::HostFinalChainPreflightReport FinalChain::ExternalEvmStateApiClient::loadCommittedState(
    const rustaxa::HostFinalChainPreflightRequest& request) const {
  rustaxa::HostFinalChainPreflightReport report{};
  report.request_id = request.request_id;
  try {
    std::lock_guard lock(state_api_mutex_);
    const auto descriptor = state_api_.get_last_committed_state_descriptor();
    report.committed_period = descriptor.blk_num;
    report.committed_state_root = into_bytes_array(descriptor.state_root);
    report.succeeded = true;
  } catch (const std::exception& error) {
    report.error_code = rust::String(std::string("FINAL_CHAIN_STATE_DB_PREFLIGHT_FAILED: ") + error.what());
  }
  return report;
}

rustaxa::HostFinalChainSystemFactsReport FinalChain::ExternalEvmStateApiClient::loadSystemTransactionFacts(
    const rustaxa::HostFinalChainSystemFactsRequest& request) {
  rustaxa::HostFinalChainSystemFactsReport report{};
  report.request_id = request.request_id;
  report.period = request.period;
  const auto bridge_contract_address = into_address(request.bridge_contract_address);
  std::lock_guard lock(state_api_mutex_);
  const auto descriptor = state_api_.get_last_committed_state_descriptor();
  const auto bridge_contract = state_api_.get_account(descriptor.blk_num, bridge_contract_address);
  report.bridge_contract_found = bridge_contract.has_value();
  report.bridge_contract_has_code = bridge_contract && bridge_contract->code_size;
  if (request.is_pillar_block_period && report.bridge_contract_has_code) {
    const static auto should_finalize_method = util::EncodingSolidity::packFunctionCall("shouldFinalizeEpoch()");
    report.should_finalize_epoch =
        u256(state_api_
                 .dry_run_transaction(
                     descriptor.blk_num,
                     state_api::EVMBlock{dev::ZeroAddress, request.block_gas_limit, 0, BlockHeader::difficulty()},
                     state_api::EVMTransaction{dev::ZeroAddress, 1, bridge_contract_address,
                                               state_api::ZeroAccount.nonce, 0, 10000000, should_finalize_method})
                 .code_retval)
            .convert_to<bool>();
    if (report.should_finalize_epoch) {
      report.system_account_nonce = into_canonical_nonce_vec(
          state_api_.get_account(descriptor.blk_num, kTaraxaSystemAccount).value_or(state_api::ZeroAccount).nonce);
    }
  }
  report.succeeded = true;
  return report;
}

rustaxa::HostFinalChainExecutionReport FinalChain::ExternalEvmStateApiClient::executeTransactions(
    const rustaxa::HostFinalChainExecutionRequest& request) {
  std::vector<state_api::EVMTransaction> transactions;
  transactions.reserve(request.transactions.size());
  for (const auto& transaction : request.transactions) {
    transactions.emplace_back(state_api::EVMTransaction{
        into_address(transaction.sender), dev::fromBigEndian<u256>(into_bytes(transaction.gas_price)),
        transaction.receiver_found ? std::optional(into_address(transaction.receiver)) : std::nullopt,
        nonce_from_canonical_vec(transaction.nonce), dev::fromBigEndian<u256>(into_bytes(transaction.value)),
        transaction.gas_limit, into_bytes(transaction.data)});
  }
  std::vector<state_api::ExecutionResult> execution_results;
  {
    std::lock_guard lock(state_api_mutex_);
    execution_results = state_api_
                            .execute_transactions({into_address(request.block_author), request.block_gas_limit,
                                                   request.timestamp, BlockHeader::difficulty()},
                                                  transactions)
                            .execution_results;
  }
  if (execution_results.size() != request.transactions.size()) {
    throw DbException("FINAL_CHAIN_EXECUTION_RESULT_COUNT_MISMATCH");
  }
  rustaxa::HostFinalChainExecutionReport report{};
  report.request_id = request.request_id;
  report.status = kFinalChainEvmReportStatusSuccess;
  gas_t cumulative_gas_used = 0;
  report.results.reserve(execution_results.size());
  for (size_t index = 0; index < execution_results.size(); ++index) {
    const auto& execution = execution_results[index];
    rustaxa::HostFinalChainTransactionResult result{};
    result.position = request.transactions[index].position;
    result.hash = request.transactions[index].hash;
    result.status = static_cast<uint8_t>(execution.code_err.empty() && execution.consensus_err.empty());
    result.gas_used = execution.gas_used;
    result.cumulative_gas_used = cumulative_gas_used += execution.gas_used;
    LogEntries receipt_logs;
    receipt_logs.reserve(execution.logs.size());
    result.logs.reserve(execution.logs.size());
    for (const auto& log : execution.logs) {
      receipt_logs.emplace_back(LogEntry{log.address, log.topics, log.data});
      rustaxa::HostFinalChainLog host_log{};
      host_log.address = into_address_array(log.address);
      host_log.data = into_rust_vec(log.data);
      host_log.topics.reserve(log.topics.size());
      for (const auto& topic : log.topics) {
        host_log.topics.push_back(rustaxa::HostFinalChainLogTopic{into_bytes_array(topic)});
      }
      result.logs.push_back(std::move(host_log));
    }
    const auto new_contract = execution.new_contract_addr ? std::optional(execution.new_contract_addr) : std::nullopt;
    result.receipt_rlp = into_rust_vec(util::rlp_enc(TransactionReceipt{
        result.status, execution.gas_used, result.cumulative_gas_used, std::move(receipt_logs), new_contract}));
    result.new_contract_address_found = new_contract.has_value();
    result.new_contract_address = new_contract ? into_address_array(*new_contract) : std::array<uint8_t, 20>{};
    result.code_error = rust::String(execution.code_err);
    result.consensus_error = rust::String(execution.consensus_err);
    report.results.push_back(std::move(result));
  }
  report.cumulative_gas_used = cumulative_gas_used;
  return report;
}

rustaxa::HostFinalChainRewardsReport FinalChain::ExternalEvmStateApiClient::distributeRewards(
    const rustaxa::HostFinalChainRewardsRequest& request) {
  rustaxa::HostFinalChainRewardsReport report{};
  report.request_id = request.request_id;
  report.period = request.period;
  std::vector<rewards::BlockStats> rewards_stats;
  rewards_stats.reserve(request.distribution_stats.size());
  for (const auto& encoded_stats : request.distribution_stats) {
    const auto bytes = into_bytes(encoded_stats.data);
    rewards_stats.push_back(util::rlp_dec<rewards::BlockStats>(dev::RLP(bytes)));
  }
  std::lock_guard lock(state_api_mutex_);
  const auto& result = state_api_.distribute_rewards(rewards_stats);
  report.status = kFinalChainEvmRewardsReportStatusSuccess;
  report.state_root = into_bytes_array(result.state_root);
  report.total_reward = into_big_endian_vec(result.total_reward);
  return report;
}

rustaxa::HostFinalChainStateCommitReport FinalChain::ExternalEvmStateApiClient::commitState(
    const rustaxa::HostFinalChainStateCommitRequest& request) {
  rustaxa::HostFinalChainStateCommitReport report{};
  report.status = kFinalChainEvmLifecycleStatusCommitted;
  try {
    std::lock_guard lock(state_api_mutex_);
    state_api_.transition_state_commit();
    const auto descriptor = state_api_.get_last_committed_state_descriptor();
    report.committed_period = descriptor.blk_num;
    report.committed_state_root = into_bytes_array(descriptor.state_root);
    if (report.committed_period != request.period || report.committed_state_root != request.expected_state_root) {
      report.status = kFinalChainEvmLifecycleStatusRejected;
      report.error_code = rust::String("FINAL_CHAIN_STATE_DB_COMMITTED_DESCRIPTOR_MISMATCH");
    }
  } catch (const std::exception& error) {
    report.status = kFinalChainEvmLifecycleStatusRejected;
    report.error_code = rust::String(std::string("STATE_API_COMMIT_FAILED: ") + error.what());
  }
  return report;
}

state_api::StateDescriptor FinalChain::ExternalEvmStateApiClient::lastCommittedStateDescriptor() const {
  std::lock_guard lock(state_api_mutex_);
  return state_api_.get_last_committed_state_descriptor();
}

void FinalChain::ExternalEvmStateApiClient::updateStateConfig(const state_api::Config& new_config,
                                                              EthBlockNumber& delegation_delay) {
  std::lock_guard lock(state_api_mutex_);
  delegation_delay = new_config.dpos.delegation_delay;
  state_api_.update_state_config(new_config);
}

std::optional<state_api::Account> FinalChain::ExternalEvmStateApiClient::account(EthBlockNumber block_number,
                                                                                 const addr_t& address) const {
  try {
    std::lock_guard lock(state_api_mutex_);
    return state_api_.get_account(block_number, address);
  } catch (const state_api::ErrFutureBlock&) {
    return std::nullopt;
  }
}

h256 FinalChain::ExternalEvmStateApiClient::accountStorageOrZero(EthBlockNumber block_number, const addr_t& address,
                                                                 const u256& key) const {
  try {
    std::lock_guard lock(state_api_mutex_);
    return state_api_.get_account_storage(block_number, address, key);
  } catch (const state_api::ErrFutureBlock&) {
    return ZeroHash();
  }
}

bytes FinalChain::ExternalEvmStateApiClient::codeOrEmpty(EthBlockNumber block_number, const addr_t& address) const {
  try {
    std::lock_guard lock(state_api_mutex_);
    return state_api_.get_code_by_address(block_number, address);
  } catch (const state_api::ErrFutureBlock&) {
    return {};
  }
}

state_api::ExecutionResult FinalChain::ExternalEvmStateApiClient::dryRunTransaction(
    const BlockHeader& block_header, const state_api::EVMTransaction& transaction, bool lock_client) const {
  std::unique_lock lock(state_api_mutex_, std::defer_lock);
  if (lock_client) {
    lock.lock();
  }
  return state_api_.dry_run_transaction(
      block_header.number,
      {block_header.author, block_header.gas_limit, block_header.timestamp, BlockHeader::difficulty()}, transaction);
}

bytes FinalChain::ExternalEvmStateApiClient::traceTransactions(const BlockHeader& block_header,
                                                               const std::vector<state_api::EVMTransaction>& state_trxs,
                                                               const std::vector<state_api::EVMTransaction>& trxs,
                                                               std::optional<state_api::Tracing> params) const {
  std::lock_guard lock(state_api_mutex_);
  return state_api_.trace(
      block_header.number,
      {block_header.author, block_header.gas_limit, block_header.timestamp, BlockHeader::difficulty()}, state_trxs,
      trxs, params);
}

bool FinalChain::ExternalEvmStateApiClient::accountHasCode(EthBlockNumber block_number, const addr_t& address) const {
  std::lock_guard lock(state_api_mutex_);
  const auto account = state_api_.get_account(block_number, address);
  return account && account->code_size;
}

void FinalChain::recoverExternalEvmPendingPublication() {
  const auto state_descriptor = external_evm_state_api_.lastCommittedStateDescriptor();
  auto recovery_report = consensus_application_->service().recover_external_evm_pending_publication(
      state_descriptor.blk_num, into_bytes_array(state_descriptor.state_root));
  if (recovery_report.status == kFinalChainEvmPublicationStatusRejected) {
    throw DbException("FinalChain startup rejected Rust external EVM publication recovery: " +
                      std::string(recovery_report.error_code));
  }
}

void FinalChain::stop() {}

EthBlockNumber FinalChain::delegationDelay() const { return delegation_delay_; }

std::shared_ptr<const BlockHeader> FinalChain::blockHeader(std::optional<EthBlockNumber> n) const {
  auto const block_number = n.value_or(lastBlockNumber());
  auto rust_header = consensus_application_->service().get_block_header(static_cast<uint64_t>(block_number));
  if (rust_header.empty()) {
    return nullptr;
  }

  auto header_data = into_string(rust_header);
  return BlockHeader::fromRLP(dev::RLP(header_data));
}

EthBlockNumber FinalChain::lastBlockNumber() const { return consensus_application_->service().get_last_block_number(); }

std::optional<EthBlockNumber> FinalChain::blockNumber(h256 const& h) const {
  auto rust_lookup = consensus_application_->service().get_block_number(into_bytes_array(h));
  if (!rust_lookup.found) {
    return std::nullopt;
  }
  return rust_lookup.value;
}

std::optional<h256> FinalChain::blockHash(std::optional<EthBlockNumber> n) const {
  auto const block_number = n.value_or(lastBlockNumber());
  auto rust_hash = consensus_application_->service().get_block_hash(static_cast<uint64_t>(block_number));
  if (rust_hash.empty()) {
    return std::nullopt;
  }
  return into_h256(rust_hash, "blockHash");
}

std::optional<h256> FinalChain::finalChainHash(EthBlockNumber n) const {
  auto delay = delegationDelay();
  if (n <= delay) {
    return ZeroHash();
  }
  auto header = blockHeader(n - delay);
  if (!header) {
    return std::nullopt;
  }
  return header->hash;
}

void FinalChain::updateStateConfig(const state_api::Config& new_config) {
  external_evm_state_api_.updateStateConfig(new_config, delegation_delay_);
}

std::shared_ptr<const TransactionHashes> FinalChain::transactionHashes(std::optional<EthBlockNumber> n) const {
  auto ret = std::make_shared<TransactionHashes>();
  for (auto const& transaction : transactions(n)) {
    ret->push_back(transaction->getHash());
  }
  return ret;
}

const SharedTransactions FinalChain::transactions(std::optional<EthBlockNumber> n) const {
  SharedTransactions ret;
  auto const block_number = n.value_or(lastBlockNumber());
  auto rust_transactions = consensus_application_->service().get_transaction_rlps(block_number);
  ret.reserve(rust_transactions.size());
  for (auto const& transaction : rust_transactions) {
    if (transaction.is_system) {
      ret.push_back(std::make_shared<SystemTransaction>(into_bytes(transaction.data)));
    } else {
      ret.push_back(std::make_shared<Transaction>(into_bytes(transaction.data), false));
    }
  }
  return ret;
}

std::optional<TransactionLocation> FinalChain::transactionLocation(h256 const& trx_hash) const {
  auto rust_location = consensus_application_->service().get_transaction_location(into_bytes_array(trx_hash));
  if (rust_location.empty()) {
    return std::nullopt;
  }
  auto location_data = into_string(rust_location);
  return TransactionLocation::fromRlp(dev::RLP(location_data));
}

std::optional<TransactionReceipt> FinalChain::transactionReceipt(EthBlockNumber blk_n, uint64_t position,
                                                                 std::optional<trx_hash_t>) const {
  auto receipt = consensus_application_->service().get_transaction_receipt(blk_n, position);
  if (receipt.empty()) {
    return std::nullopt;
  }
  auto receipt_data = into_string(receipt);
  return util::rlp_dec<TransactionReceipt>(dev::RLP(receipt_data));
}

std::shared_ptr<Transaction> FinalChain::transaction(EthBlockNumber blk_n, uint32_t position) const {
  auto block_transactions = transactions(blk_n);
  if (position >= block_transactions.size()) {
    return nullptr;
  }
  return block_transactions[position];
}

uint64_t FinalChain::transactionCount(std::optional<EthBlockNumber> n) const {
  return consensus_application_->service().get_transaction_count(static_cast<uint64_t>(n.value_or(lastBlockNumber())));
}

std::vector<EthBlockNumber> FinalChain::withBlockBloom(LogBloom const& b, EthBlockNumber from,
                                                       EthBlockNumber to) const {
  std::array<uint8_t, 256> bloom{};
  std::memcpy(bloom.data(), b.data(), bloom.size());
  auto rust_blocks = consensus_application_->service().get_blocks_with_bloom(bloom, from, to);
  std::vector<EthBlockNumber> blocks;
  blocks.reserve(rust_blocks.size());
  for (auto const block : rust_blocks) {
    blocks.push_back(block);
  }
  return blocks;
}

std::optional<state_api::Account> FinalChain::getAccount(addr_t const& addr,
                                                         std::optional<EthBlockNumber> blk_n) const {
  const auto state_descriptor = external_evm_state_api_.lastCommittedStateDescriptor();
  const auto requested_block = blk_n.value_or(lastBlockNumber());
  if (requested_block <= state_descriptor.blk_num) {
    return external_evm_state_api_.account(requested_block, addr);
  }

  rustaxa::AccountLookup rust_account;
  try {
    rust_account = consensus_application_->service().get_account_at_block(requested_block, into_address_array(addr));
  } catch (const std::exception& error) {
    // External-EVM publications deliberately omit native account snapshots. When the requested native height is ahead
    // of the retained EVM state, use its latest committed snapshot for the concrete-EVM account boundary.
    if (std::string(error.what()).find("account snapshot unavailable") != std::string::npos) {
      return external_evm_state_api_.account(state_descriptor.blk_num, addr);
    }
    throw DbException("FinalChain::getAccount requested block " + std::to_string(requested_block) +
                      " beyond external EVM head " + std::to_string(state_descriptor.blk_num) + ": " + error.what());
  }
  if (!rust_account.found) {
    return std::nullopt;
  }

  state_api::Account account;
  account.nonce = nonce_from_canonical_vec(rust_account.nonce);
  account.balance = dev::fromBigEndian<u256>(dev::bytes(rust_account.balance.begin(), rust_account.balance.end()));
  account.storage_root_hash =
      h256(dev::bytes(rust_account.storage_root_hash.begin(), rust_account.storage_root_hash.end()));
  account.code_hash = h256(dev::bytes(rust_account.code_hash.begin(), rust_account.code_hash.end()));
  account.code_size = rust_account.code_size;
  return account;
}

h256 FinalChain::getAccountStorage(addr_t const& addr, u256 const& key, std::optional<EthBlockNumber> blk_n) const {
  const auto state_descriptor = external_evm_state_api_.lastCommittedStateDescriptor();
  const auto requested_block = blk_n.value_or(state_descriptor.blk_num);
  if (requested_block <= state_descriptor.blk_num) {
    return external_evm_state_api_.accountStorageOrZero(requested_block, addr, key);
  }
  return ZeroHash();
}

bytes FinalChain::getCode(addr_t const& addr, std::optional<EthBlockNumber> blk_n) const {
  const auto state_descriptor = external_evm_state_api_.lastCommittedStateDescriptor();
  const auto requested_block = blk_n.value_or(state_descriptor.blk_num);
  if (requested_block <= state_descriptor.blk_num) {
    return external_evm_state_api_.codeOrEmpty(requested_block, addr);
  }
  return {};
}

state_api::ExecutionResult FinalChain::call(state_api::EVMTransaction const& trx,
                                            std::optional<EthBlockNumber> blk_n) const {
  const auto state_descriptor = external_evm_state_api_.lastCommittedStateDescriptor();
  const auto requested_block = blk_n.value_or(lastBlockNumber());
  auto call_rust_final_chain = [&]() {
    rustaxa::FinalChainCall request;
    request.block_number = requested_block;
    request.sender = into_address_array(trx.from);
    if (trx.to) {
      request.receiver_found = true;
      request.receiver = into_address_array(*trx.to);
    } else {
      request.receiver_found = false;
      request.receiver = {};
    }
    request.value = into_big_endian_vec(trx.value);
    request.gas_price = into_big_endian_vec(trx.gas_price);
    request.gas_limit = trx.gas;
    request.input = into_rust_vec(trx.input);
    auto outcome = consensus_application_->service().call(std::move(request));

    state_api::ExecutionResult result;
    result.code_retval = into_bytes(outcome.code_retval);
    result.logs.reserve(outcome.logs.size());
    for (const auto& log : outcome.logs) {
      state_api::LogRecord converted;
      converted.address = into_address(log.address);
      converted.topics.reserve(log.topics.size());
      for (const auto& topic : log.topics) {
        converted.topics.emplace_back(dev::bytes(topic.topic.begin(), topic.topic.end()));
      }
      converted.data = into_bytes(log.data);
      result.logs.push_back(std::move(converted));
    }
    result.gas_used = outcome.gas_used;
    result.code_err = std::string(outcome.code_err);
    result.consensus_err = std::string(outcome.consensus_err);
    return result;
  };

  const auto dpos_contract_address = addr_t("0x00000000000000000000000000000000000000FE");
  if (trx.to && *trx.to == dpos_contract_address) {
    return call_rust_final_chain();
  }

  if (requested_block <= state_descriptor.blk_num || state_descriptor.blk_num < lastBlockNumber()) {
    const auto evm_block = std::min(requested_block, state_descriptor.blk_num);
    const auto blk_header = blockHeader(evm_block);
    if (!blk_header) {
      throw std::runtime_error("Future block");
    }
    try {
      return external_evm_state_api_.dryRunTransaction(*blk_header, trx, !blk_n.has_value());
    } catch (const state_api::ErrFutureBlock& e) {
      state_api::ExecutionResult result;
      result.consensus_err = e.what();
      return result;
    }
  }

  return call_rust_final_chain();
}

std::string FinalChain::trace(std::vector<state_api::EVMTransaction> state_trxs,
                              std::vector<state_api::EVMTransaction> trxs, EthBlockNumber blk_n,
                              std::optional<state_api::Tracing> params) const {
  const auto state_descriptor = external_evm_state_api_.lastCommittedStateDescriptor();
  if (blk_n <= state_descriptor.blk_num) {
    const auto blk_header = blockHeader(blk_n);
    if (!blk_header) {
      throw std::runtime_error("Future block");
    }
    return dev::asString(external_evm_state_api_.traceTransactions(*blk_header, state_trxs, trxs, params));
  }
  throw_unimplemented_final_chain_api("trace");
}

uint64_t FinalChain::dposEligibleTotalVoteCount(EthBlockNumber blk_num) const {
  return consensus_application_->service().get_dpos_eligible_total_vote_count(blk_num);
}

uint64_t FinalChain::dposEligibleVoteCount(EthBlockNumber blk_num, addr_t const& addr) const {
  return consensus_application_->service().get_dpos_eligible_vote_count(blk_num, into_address_array(addr));
}

std::vector<state_api::ValidatorVoteCount> FinalChain::dposValidatorsEligibleVoteCounts(EthBlockNumber blk_num) const {
  auto rust_vote_counts = consensus_application_->service().get_dpos_validators_eligible_vote_counts(blk_num);
  std::vector<state_api::ValidatorVoteCount> vote_counts;
  vote_counts.reserve(rust_vote_counts.size());
  for (const auto& rust_vote_count : rust_vote_counts) {
    vote_counts.push_back(
        state_api::ValidatorVoteCount{into_address(rust_vote_count.address), rust_vote_count.vote_count});
  }
  return vote_counts;
}

bool FinalChain::dposIsEligible(EthBlockNumber blk_num, addr_t const& addr) const {
  return dposEligibleVoteCount(blk_num, addr) > 0;
}

void FinalChain::waitForFinalized() { std::this_thread::sleep_for(std::chrono::milliseconds(10)); }

std::vector<state_api::ValidatorStake> FinalChain::dposValidatorsTotalStakes(EthBlockNumber blk_num) const {
  auto rust_stakes = consensus_application_->service().get_dpos_validators_total_stakes(blk_num);
  std::vector<state_api::ValidatorStake> stakes;
  stakes.reserve(rust_stakes.size());
  for (const auto& rust_stake : rust_stakes) {
    stakes.push_back(state_api::ValidatorStake{
        into_address(rust_stake.address),
        dev::fromBigEndian<u256>(dev::bytes(rust_stake.stake.begin(), rust_stake.stake.end())),
    });
  }
  return stakes;
}

uint256_t FinalChain::dposTotalAmountDelegated(EthBlockNumber blk_num) const {
  auto delegated = consensus_application_->service().get_dpos_total_amount_delegated(blk_num);
  return dev::fromBigEndian<u256>(dev::bytes(delegated.begin(), delegated.end()));
}

uint64_t FinalChain::dposYield(EthBlockNumber blk_num) const {
  return consensus_application_->service().get_dpos_yield(blk_num);
}

u256 FinalChain::dposTotalSupply(EthBlockNumber blk_num) const {
  auto supply = consensus_application_->service().get_dpos_total_supply(blk_num);
  return dev::fromBigEndian<u256>(dev::bytes(supply.begin(), supply.end()));
}

h256 FinalChain::getBridgeRoot(EthBlockNumber blk_num) const {
  const static auto get_bridge_root_method = util::EncodingSolidity::packFunctionCall("getBridgeRoot()");
  return readBridgeContractHash(blk_num, get_bridge_root_method, "getBridgeRoot");
}

h256 FinalChain::getBridgeEpoch(EthBlockNumber blk_num) const {
  const static auto get_bridge_epoch_method = util::EncodingSolidity::packFunctionCall("finalizedEpoch()");
  return readBridgeContractHash(blk_num, get_bridge_epoch_method, "getBridgeEpoch");
}

h256 FinalChain::readBridgeContractHash(EthBlockNumber block_number, const bytes& method, const char* api_name) const {
  const auto bridge_contract_address = config_.genesis.state.hardforks.ficus_hf.bridge_contract_address;
  const auto state_descriptor = external_evm_state_api_.lastCommittedStateDescriptor();
  // Native-only finalizations advance the canonical block height without changing EVM state. Query the latest committed
  // EVM snapshot at or before the requested native block, matching the retained concrete-EVM call boundary.
  const auto evm_block = std::min(block_number, state_descriptor.blk_num);
  if (!external_evm_state_api_.accountHasCode(evm_block, bridge_contract_address)) {
    return ZeroHash();
  }
  const auto block_header = blockHeader(evm_block);
  if (!block_header) {
    throw DbException("FinalChain::" + std::string(api_name) + " missing committed block header for block " +
                      std::to_string(evm_block));
  }
  const auto result = external_evm_state_api_.dryRunTransaction(
      *block_header,
      state_api::EVMTransaction{dev::ZeroAddress, 1, bridge_contract_address, state_api::ZeroAccount.nonce, 0, 10000000,
                                method},
      true);
  if (result.code_retval.empty()) {
    return ZeroHash();
  }
  return into_h256(result.code_retval, api_name);
}

std::pair<val_t, bool> FinalChain::getBalance(addr_t const& addr) const {
  if (auto account = getAccount(addr)) {
    return {account->balance, true};
  }
  return {0, false};
}

SharedTransactionReceipts FinalChain::blockReceipts(std::optional<EthBlockNumber> n) const {
  auto const block_number = n.value_or(lastBlockNumber());
  auto count = transactionCount(block_number);
  auto receipts = std::make_shared<std::vector<TransactionReceipt>>();
  receipts->reserve(count);
  for (uint64_t position = 0; position < count; ++position) {
    if (auto receipt = transactionReceipt(block_number, position)) {
      receipts->push_back(*receipt);
    }
  }
  return receipts;
}

}  // namespace taraxa::final_chain
