#include "consensus/external_evm_state_owner.hpp"

#ifdef RUSTAXA_ENABLE

#include <algorithm>
#include <array>
#include <cstring>
#include <iterator>
#include <stdexcept>
#include <utility>

#include "common/constants.hpp"
#include "common/encoding_rlp.hpp"
#include "common/encoding_solidity.hpp"
#include "consensus/consensus_application.hpp"
#include "rewards/block_stats.hpp"
#include "storage/storage.hpp"
#include "transaction/receipt.hpp"
#include "transaction/transaction.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kFinalChainEvmLifecycleStatusCommitted = 0;
constexpr uint8_t kFinalChainEvmLifecycleStatusRejected = 2;

std::array<uint8_t, 32> toArray(const h256& hash) {
  std::array<uint8_t, 32> bytes{};
  std::memcpy(bytes.data(), hash.data(), bytes.size());
  return bytes;
}

std::array<uint8_t, 20> toArray(const addr_t& address) {
  std::array<uint8_t, 20> bytes{};
  std::memcpy(bytes.data(), address.data(), bytes.size());
  return bytes;
}

addr_t toAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

rust::Vec<uint8_t> toRustBytes(const bytes& value) {
  rust::Vec<uint8_t> result;
  result.reserve(value.size());
  std::copy(value.begin(), value.end(), std::back_inserter(result));
  return result;
}

bytes fromRustBytes(const rust::Vec<uint8_t>& value) { return {value.begin(), value.end()}; }

rust::Vec<uint8_t> toBigEndianBytes(const u256& value) { return toRustBytes(dev::toBigEndian(value)); }

rust::Vec<uint8_t> toCanonicalNonce(const u256& value) {
  const auto encoded = dev::toBigEndian(value);
  const auto first_nonzero = std::find_if(encoded.begin(), encoded.end(), [](auto byte) { return byte != 0; });
  return toRustBytes(bytes(first_nonzero, encoded.end()));
}

u256 fromCanonicalNonce(const rust::Vec<uint8_t>& value) {
  if (value.size() > 32) throw std::runtime_error("FINAL_CHAIN_NONCE_EXCEEDS_CPP_U256");
  if (!value.empty() && value[0] == 0) throw std::runtime_error("FINAL_CHAIN_NONCE_NON_CANONICAL");
  return dev::fromBigEndian<u256>(fromRustBytes(value));
}

h256 checkedHash(const bytes& value, const char* operation) {
  if (value.size() != 32) {
    throw DbException(std::string(operation) + " returned invalid hash size: expected 32, got " +
                      std::to_string(value.size()));
  }
  return h256(value);
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

ExternalEvmStateOwner::ExternalEvmStateOwner(const FullNodeConfig& config)
    : state_api_([this](EthBlockNumber block_number) { return blockHash(block_number); }, config.genesis.state,
                 config.opts_final_chain, {(config.db_path / "state_db").string()}),
      bridge_contract_address_(config.genesis.state.hardforks.ficus_hf.bridge_contract_address) {}

ExternalEvmStateOwner::~ExternalEvmStateOwner() = default;

void ExternalEvmStateOwner::bindApplication(const std::shared_ptr<ConsensusApplication>& application) {
  if (!application) throw std::invalid_argument("ExternalEvmStateOwner requires ConsensusApplication");
  const std::scoped_lock lock(application_mutex_);
  if (!application_.expired()) throw std::logic_error("ExternalEvmStateOwner is already bound");
  application_ = application;
}

std::shared_ptr<ConsensusApplication> ExternalEvmStateOwner::application() const {
  const std::scoped_lock lock(application_mutex_);
  auto result = application_.lock();
  if (!result) throw std::logic_error("EXTERNAL_EVM_APPLICATION_UNAVAILABLE");
  return result;
}

void ExternalEvmStateOwner::ensureReadableLocked() const {
  if (state_api_.get_pending_concrete_execution()) throw DbException("FINAL_CHAIN_CONCRETE_STATE_STAGED");
}

state_api::StateDescriptor ExternalEvmStateOwner::lastCommittedStateDescriptor() const {
  const std::scoped_lock lock(mutex_);
  return state_api_.get_last_committed_state_descriptor();
}

rustaxa::HostFinalChainPreflightReport ExternalEvmStateOwner::loadCommittedState(
    const rustaxa::HostFinalChainPreflightRequest& request) {
  rustaxa::HostFinalChainPreflightReport report{};
  report.request_id = request.request_id;
  try {
    const std::scoped_lock lock(mutex_);
    report.concrete_provenance_rlp = toRustBytes(state_api_.activate_concrete_root_policy(
        h256(request.concrete_chain_identity.data(), h256::ConstructFromPointer)));
    if (const auto pending = state_api_.get_pending_concrete_execution()) {
      report.pending_concrete_marker_rlp = toRustBytes(*pending);
    }
    const auto descriptor = state_api_.get_last_committed_state_descriptor();
    report.committed_period = descriptor.blk_num;
    report.committed_state_root = toArray(descriptor.state_root);
    report.succeeded = true;
  } catch (const std::exception& error) {
    report.error_code = rust::String(std::string("FINAL_CHAIN_STATE_DB_PREFLIGHT_FAILED: ") + error.what());
  }
  return report;
}

rustaxa::HostFinalChainSystemFactsReport ExternalEvmStateOwner::loadSystemTransactionFacts(
    const rustaxa::HostFinalChainSystemFactsRequest& request) {
  rustaxa::HostFinalChainSystemFactsReport report{};
  report.request_id = request.request_id;
  report.period = request.period;
  const auto bridge_contract_address = toAddress(request.bridge_contract_address);
  const std::scoped_lock lock(mutex_);
  ensureReadableLocked();
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
                     state_api::EVMBlock{dev::ZeroAddress, request.block_gas_limit, 0,
                                         final_chain::BlockHeader::difficulty()},
                     state_api::EVMTransaction{dev::ZeroAddress, 1, bridge_contract_address,
                                               state_api::ZeroAccount.nonce, 0, 10000000, should_finalize_method})
                 .code_retval)
            .convert_to<bool>();
    if (report.should_finalize_epoch) {
      report.system_account_nonce = toCanonicalNonce(
          state_api_.get_account(descriptor.blk_num, kTaraxaSystemAccount).value_or(state_api::ZeroAccount).nonce);
    }
  }
  report.succeeded = true;
  return report;
}

rustaxa::HostFinalChainExecutionReport ExternalEvmStateOwner::executeTransactions(
    const rustaxa::HostFinalChainExecutionRequest& request) {
  std::vector<state_api::EVMTransaction> transactions;
  transactions.reserve(request.transactions.size());
  for (const auto& transaction : request.transactions) {
    transactions.push_back(state_api::EVMTransaction{
        toAddress(transaction.sender), dev::fromBigEndian<u256>(fromRustBytes(transaction.gas_price)),
        transaction.receiver_found ? std::optional(toAddress(transaction.receiver)) : std::nullopt,
        fromCanonicalNonce(transaction.nonce), dev::fromBigEndian<u256>(fromRustBytes(transaction.value)),
        transaction.gas_limit, fromRustBytes(transaction.data)});
  }

  std::vector<state_api::ExecutionResult> execution_results;
  h256 post_transaction_state_root;
  {
    const std::scoped_lock lock(mutex_);
    ensureReadableLocked();
    state_api_.stage_concrete_execution(fromRustBytes(request.concrete_marker_rlp));
    const auto& execution = state_api_.execute_transactions({toAddress(request.block_author), request.block_gas_limit,
                                                             request.timestamp, final_chain::BlockHeader::difficulty()},
                                                            transactions);
    execution_results = execution.execution_results;
    post_transaction_state_root = state_api_.post_transaction_state_root();
  }
  if (execution_results.size() != request.transactions.size()) {
    throw DbException("FINAL_CHAIN_EXECUTION_RESULT_COUNT_MISMATCH");
  }

  rustaxa::HostFinalChainExecutionReport report{};
  report.post_transaction_state_root = toArray(post_transaction_state_root);
  gas_t cumulative_gas_used = 0;
  report.results.reserve(execution_results.size());
  for (const auto& execution : execution_results) {
    rustaxa::HostFinalChainTransactionResult result{};
    result.status = static_cast<uint8_t>(execution.code_err.empty() && execution.consensus_err.empty());
    result.gas_used = execution.gas_used;
    result.cumulative_gas_used = cumulative_gas_used += execution.gas_used;
    LogEntries receipt_logs;
    receipt_logs.reserve(execution.logs.size());
    result.logs.reserve(execution.logs.size());
    for (const auto& log : execution.logs) {
      receipt_logs.push_back(LogEntry{log.address, log.topics, log.data});
      rustaxa::HostFinalChainLog host_log{};
      host_log.address = toArray(log.address);
      host_log.data = toRustBytes(log.data);
      host_log.topics.reserve(log.topics.size());
      for (const auto& topic : log.topics) host_log.topics.push_back({toArray(topic)});
      result.logs.push_back(std::move(host_log));
    }
    const auto new_contract = execution.new_contract_addr ? std::optional(execution.new_contract_addr) : std::nullopt;
    result.receipt_rlp = toRustBytes(util::rlp_enc(TransactionReceipt{
        result.status, execution.gas_used, result.cumulative_gas_used, std::move(receipt_logs), new_contract}));
    result.new_contract_address_found = new_contract.has_value();
    if (new_contract) result.new_contract_address = toArray(*new_contract);
    result.output = toRustBytes(execution.code_retval);
    result.code_error = rust::String(execution.code_err);
    result.consensus_error = rust::String(execution.consensus_err);
    report.results.push_back(std::move(result));
  }
  report.cumulative_gas_used = cumulative_gas_used;
  return report;
}

rustaxa::HostFinalChainRewardsReport ExternalEvmStateOwner::distributeRewards(
    const rustaxa::HostFinalChainRewardsRequest& request) {
  rustaxa::HostFinalChainRewardsReport report{};
  std::vector<rewards::BlockStats> rewards_stats;
  rewards_stats.reserve(request.distribution_stats.size());
  for (const auto& encoded_stats : request.distribution_stats) {
    const auto value = fromRustBytes(encoded_stats.data);
    rewards_stats.push_back(util::rlp_dec<rewards::BlockStats>(dev::RLP(value)));
  }
  const std::scoped_lock lock(mutex_);
  const auto pending_marker = state_api_.get_pending_concrete_execution();
  if (!pending_marker || *pending_marker != fromRustBytes(request.concrete_marker_rlp)) {
    throw DbException("FINAL_CHAIN_CONCRETE_REWARDS_MARKER_MISMATCH");
  }
  const auto& result = state_api_.distribute_rewards(rewards_stats);
  report.post_rewards_state_root = toArray(result.state_root);
  report.total_reward = toBigEndianBytes(result.total_reward);
  const auto projection = state_api_.get_concrete_state_projection();
  report.concrete_projection_rlp = toRustBytes(projection);
  report.concrete_projection_hash = toArray(dev::sha3(projection));
  return report;
}

rustaxa::HostFinalChainStateCommitReport ExternalEvmStateOwner::commitState(
    const rustaxa::HostFinalChainStateCommitRequest& request) {
  rustaxa::HostFinalChainStateCommitReport report{};
  report.status = kFinalChainEvmLifecycleStatusCommitted;
  try {
    const std::scoped_lock lock(mutex_);
    const auto pending_marker = state_api_.get_pending_concrete_execution();
    if (!pending_marker || *pending_marker != fromRustBytes(request.concrete_marker_rlp) ||
        state_api_.get_concrete_state_projection() != fromRustBytes(request.concrete_projection_rlp)) {
      throw DbException("FINAL_CHAIN_CONCRETE_COMMIT_IDENTITY_MISMATCH");
    }
    state_api_.concrete_commit(h256(request.concrete_projection_hash.data(), h256::ConstructFromPointer),
                               fromRustBytes(request.concrete_provenance_rlp));
    const auto descriptor = state_api_.get_last_committed_state_descriptor();
    report.committed_period = descriptor.blk_num;
    report.committed_state_root = toArray(descriptor.state_root);
    report.committed_state_found = true;
    report.concrete_provenance_rlp = toRustBytes(state_api_.get_concrete_state_provenance());
  } catch (const std::exception& error) {
    report.status = kFinalChainEvmLifecycleStatusRejected;
    report.error_code = rust::String(std::string("STATE_API_COMMIT_FAILED: ") + error.what());
  }
  return report;
}

rustaxa::HostFinalChainPreflightReport ExternalEvmStateOwner::discardState(
    const rustaxa::CanonicalBytes& concrete_marker) {
  rustaxa::HostFinalChainPreflightReport report{};
  try {
    const std::scoped_lock lock(mutex_);
    state_api_.discard_concrete_execution(fromRustBytes(concrete_marker.data));
    const auto descriptor = state_api_.get_last_committed_state_descriptor();
    report.committed_period = descriptor.blk_num;
    report.committed_state_root = toArray(descriptor.state_root);
    report.concrete_provenance_rlp = toRustBytes(state_api_.get_concrete_state_provenance());
    if (const auto pending = state_api_.get_pending_concrete_execution()) {
      report.pending_concrete_marker_rlp = toRustBytes(*pending);
    }
    report.succeeded = report.pending_concrete_marker_rlp.empty();
    if (!report.succeeded) report.error_code = rust::String("FINAL_CHAIN_CONCRETE_DISCARD_REOPEN_MISMATCH");
  } catch (const std::exception& error) {
    report.error_code = rust::String(std::string("FINAL_CHAIN_CONCRETE_DISCARD_FAILED: ") + error.what());
  }
  return report;
}

std::shared_ptr<const final_chain::BlockHeader> ExternalEvmStateOwner::blockHeader(EthBlockNumber block_number) const {
  const auto view = (*application()->queryClient())->consensus_query_final_chain_block_by_number(block_number);
  if (!view.found) return nullptr;
  const auto encoded = bytes(view.header_rlp.begin(), view.header_rlp.end());
  return final_chain::BlockHeader::fromRLP(dev::RLP(encoded));
}

EthBlockNumber ExternalEvmStateOwner::lastBlockNumber() const {
  return (*application()->queryClient())->consensus_query_final_chain_last_block_number();
}

h256 ExternalEvmStateOwner::blockHash(EthBlockNumber block_number) const {
  std::shared_ptr<ConsensusApplication> bound_application;
  {
    const std::scoped_lock lock(application_mutex_);
    bound_application = application_.lock();
  }
  // StateAPI may ask for a block hash while opening the database, before the
  // native root can be restored from its descriptor. Genesis/open-time lookups
  // retain the legacy zero-hash callback; every post-bind operation queries the
  // canonical native header index.
  if (!bound_application) return ZeroHash();
  const auto view = (*bound_application->queryClient())->consensus_query_final_chain_block_by_number(block_number);
  return view.found ? h256(view.hash.data(), h256::ConstructFromPointer) : ZeroHash();
}

std::optional<state_api::Account> ExternalEvmStateOwner::account(const addr_t& address,
                                                                 std::optional<EthBlockNumber> block_number) const {
  state_api::StateDescriptor descriptor;
  {
    const std::scoped_lock lock(mutex_);
    ensureReadableLocked();
    descriptor = state_api_.get_last_committed_state_descriptor();
  }
  const auto requested_block = block_number.value_or(lastBlockNumber());
  if (requested_block <= descriptor.blk_num) {
    try {
      const std::scoped_lock lock(mutex_);
      ensureReadableLocked();
      return state_api_.get_account(requested_block, address);
    } catch (const state_api::ErrFutureBlock&) {
      return std::nullopt;
    }
  }

  // Native-only blocks do not mutate concrete EVM state. Read the latest
  // committed StateAPI snapshot instead of exposing a second native account
  // service through CXX.
  const std::scoped_lock lock(mutex_);
  ensureReadableLocked();
  return state_api_.get_account(descriptor.blk_num, address);
}

h256 ExternalEvmStateOwner::accountStorage(const addr_t& address, const u256& key,
                                           std::optional<EthBlockNumber> block_number) const {
  const auto descriptor = lastCommittedStateDescriptor();
  const auto requested_block = block_number.value_or(descriptor.blk_num);
  if (requested_block > descriptor.blk_num) return ZeroHash();
  try {
    const std::scoped_lock lock(mutex_);
    ensureReadableLocked();
    return state_api_.get_account_storage(requested_block, address, key);
  } catch (const state_api::ErrFutureBlock&) {
    return ZeroHash();
  }
}

bytes ExternalEvmStateOwner::code(const addr_t& address, std::optional<EthBlockNumber> block_number) const {
  const auto descriptor = lastCommittedStateDescriptor();
  const auto requested_block = block_number.value_or(descriptor.blk_num);
  if (requested_block > descriptor.blk_num) return {};
  try {
    const std::scoped_lock lock(mutex_);
    ensureReadableLocked();
    return state_api_.get_code_by_address(requested_block, address);
  } catch (const state_api::ErrFutureBlock&) {
    return {};
  }
}

state_api::ExecutionResult ExternalEvmStateOwner::call(const state_api::EVMTransaction& transaction,
                                                       std::optional<EthBlockNumber> block_number) const {
  const auto descriptor = lastCommittedStateDescriptor();
  const auto requested_block = block_number.value_or(lastBlockNumber());
  const auto dpos_contract_address = addr_t("0x00000000000000000000000000000000000000FE");
  if (transaction.to && *transaction.to == dpos_contract_address) {
    rustaxa::FinalChainNativeCall request;
    request.block_number = requested_block;
    request.sender = toArray(transaction.from);
    request.receiver_found = true;
    request.receiver = toArray(*transaction.to);
    request.value = toBigEndianBytes(transaction.value);
    request.gas_price = toBigEndianBytes(transaction.gas_price);
    request.gas_limit = transaction.gas;
    request.input = toRustBytes(transaction.input);
    const auto outcome = (*application()->queryClient())->consensus_query_final_chain_native_call(std::move(request));

    state_api::ExecutionResult result;
    result.code_retval = fromRustBytes(outcome.code_retval);
    result.logs.reserve(outcome.logs.size());
    for (const auto& log : outcome.logs) {
      state_api::LogRecord converted;
      converted.address = toAddress(log.address);
      converted.topics.reserve(log.topics.size());
      for (const auto& topic : log.topics) {
        converted.topics.emplace_back(bytes(topic.topic.begin(), topic.topic.end()));
      }
      converted.data = fromRustBytes(log.data);
      result.logs.push_back(std::move(converted));
    }
    result.gas_used = outcome.gas_used;
    result.code_err = std::string(outcome.code_err);
    result.consensus_err = std::string(outcome.consensus_err);
    return result;
  }
  const auto evm_block = std::min(requested_block, descriptor.blk_num);
  const auto header = blockHeader(evm_block);
  if (!header) throw std::runtime_error("Future block");
  try {
    const std::scoped_lock lock(mutex_);
    ensureReadableLocked();
    return state_api_.dry_run_transaction(
        header->number, {header->author, header->gas_limit, header->timestamp, final_chain::BlockHeader::difficulty()},
        transaction);
  } catch (const state_api::ErrFutureBlock& error) {
    state_api::ExecutionResult result;
    result.consensus_err = error.what();
    return result;
  }
}

std::string ExternalEvmStateOwner::trace(std::vector<state_api::EVMTransaction> state_transactions,
                                         std::vector<state_api::EVMTransaction> transactions,
                                         EthBlockNumber block_number, std::optional<state_api::Tracing> params) const {
  const auto descriptor = lastCommittedStateDescriptor();
  if (block_number > descriptor.blk_num) throw DbException("FinalChain::trace is not implemented above EVM head");
  const auto header = blockHeader(block_number);
  if (!header) throw std::runtime_error("Future block");
  const std::scoped_lock lock(mutex_);
  ensureReadableLocked();
  return dev::asString(state_api_.trace(
      header->number, {header->author, header->gas_limit, header->timestamp, final_chain::BlockHeader::difficulty()},
      state_transactions, transactions, params));
}

h256 ExternalEvmStateOwner::readBridgeContractHash(EthBlockNumber block_number, const bytes& method,
                                                   const char* operation) const {
  const auto descriptor = lastCommittedStateDescriptor();
  const auto evm_block = std::min(block_number, descriptor.blk_num);
  {
    const std::scoped_lock lock(mutex_);
    ensureReadableLocked();
    const auto account = state_api_.get_account(evm_block, bridge_contract_address_);
    if (!account || !account->code_size) return ZeroHash();
  }
  const auto header = blockHeader(evm_block);
  if (!header) {
    throw DbException(std::string(operation) + " missing committed block header for block " +
                      std::to_string(evm_block));
  }
  const std::scoped_lock lock(mutex_);
  ensureReadableLocked();
  const auto result = state_api_.dry_run_transaction(
      header->number, {header->author, header->gas_limit, header->timestamp, final_chain::BlockHeader::difficulty()},
      state_api::EVMTransaction{dev::ZeroAddress, 1, bridge_contract_address_, state_api::ZeroAccount.nonce, 0,
                                10000000, method});
  return result.code_retval.empty() ? ZeroHash() : checkedHash(result.code_retval, operation);
}

rustaxa::HostPillarAnchorStateReport ExternalEvmStateOwner::loadPillarAnchorState(
    const rustaxa::HostPillarAnchorStateRequest& request) const {
  rustaxa::HostPillarAnchorStateReport report{};
  report.effect_id = request.effect_id;
  try {
    const static auto root_method = util::EncodingSolidity::packFunctionCall("getBridgeRoot()");
    const static auto epoch_method = util::EncodingSolidity::packFunctionCall("finalizedEpoch()");
    report.bridge_root = toArray(readBridgeContractHash(request.period, root_method, "getBridgeRoot"));
    report.bridge_epoch = toArray(readBridgeContractHash(request.period, epoch_method, "getBridgeEpoch"));
    report.succeeded = true;
  } catch (const std::exception& error) {
    report.error_code = rust::String(std::string("PILLAR_ANCHOR_STATE_READ_FAILED: ") + error.what());
  }
  return report;
}

rustaxa::HostDagGasBatch ExternalEvmStateOwner::estimateDagTransactionGas(
    const rustaxa::HostDagGasBatch& request) const {
  rustaxa::HostDagGasBatch report{};
  report.effect_id = request.effect_id;
  report.proposal_period = request.proposal_period;
  for (const auto& hash : request.transaction_hashes) report.transaction_hashes.push_back(hash);
  try {
    if (request.transaction_hashes.size() != request.transaction_rlps.size()) {
      report.error_code = rust::String("DAG_GAS_TRANSACTION_COUNT_MISMATCH");
      return report;
    }
    report.observed_block = lastBlockNumber();
    report.gas_used.reserve(request.transaction_hashes.size());
    report.result_rlps.reserve(request.transaction_hashes.size());
    for (size_t index = 0; index < request.transaction_hashes.size(); ++index) {
      const auto transaction = std::make_shared<Transaction>(fromRustBytes(request.transaction_rlps[index].data));
      if (transaction->getHash().asArray() != request.transaction_hashes[index].hash) {
        report.gas_used.clear();
        report.result_rlps.clear();
        report.error_code = rust::String("DAG_GAS_TRANSACTION_HASH_MISMATCH");
        return report;
      }
      const auto result =
          call(state_api::EVMTransaction{transaction->getSender(), transaction->getGasPrice(),
                                         transaction->getReceiver(), transaction->getNonce(), transaction->getValue(),
                                         transaction->getGas(), transaction->getData()},
               request.proposal_period);
      report.gas_used.push_back(result.gas_used);
      rustaxa::CanonicalBytes encoded{};
      encoded.data = toRustBytes(util::rlp_enc(result));
      report.result_rlps.push_back(std::move(encoded));
    }
    report.succeeded = true;
  } catch (const std::exception& error) {
    report.gas_used.clear();
    report.result_rlps.clear();
    report.error_code = rust::String(std::string("DAG_GAS_ESTIMATION_FAILED: ") + error.what());
  }
  return report;
}

void ExternalEvmStateOwner::prune(EthBlockNumber block_number) {
  const auto first_header_to_keep = blockHeader(block_number);
  if (!first_header_to_keep) return;
  {
    const std::scoped_lock lock(mutex_);
    if (state_api_.get_pending_concrete_execution()) throw DbException("FINAL_CHAIN_CONCRETE_STATE_STAGED");
    const auto evm_head = state_api_.get_last_committed_state_descriptor().blk_num;
    if (evm_head >= first_header_to_keep->number) {
      std::vector<h256> roots_to_keep;
      for (auto header = first_header_to_keep; header && header->number <= evm_head;
           header = blockHeader(header->number + 1)) {
        roots_to_keep.push_back(header->state_root);
      }
      state_api_.prune(roots_to_keep, first_header_to_keep->number);
    }
  }
}

}  // namespace taraxa

#endif  // RUSTAXA_ENABLE
