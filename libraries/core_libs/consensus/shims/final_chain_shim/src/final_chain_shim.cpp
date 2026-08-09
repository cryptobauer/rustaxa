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
#include "final_chain/final_chain.hpp"
#include "libdevcore/CommonData.h"
#include "rewards/block_stats.hpp"
#include "transaction/receipt.hpp"
#include "transaction/system_transaction.hpp"

namespace taraxa::final_chain {
namespace {

constexpr uint8_t kFinalChainExecutionModeExternalEvmAllowed = 1;
constexpr uint8_t kFinalChainExecutionStatusComplete = 1;
constexpr uint8_t kFinalChainExecutionActionComplete = 0;
constexpr uint8_t kFinalChainExecutionActionCommitNative = 1;
constexpr uint8_t kFinalChainExecutionActionExecuteExternalEvm = 2;
constexpr uint8_t kFinalChainExecutionActionReject = 3;
constexpr uint8_t kFinalChainExecutionActionDistributeExternalEvmRewards = 4;
constexpr uint8_t kFinalChainExecutionActionProvideSystemTransactions = 5;
constexpr uint8_t kFinalChainExecutionActionPublishExternalEvmStorage = 9;
constexpr uint8_t kFinalChainEvmReportStatusSuccess = 0;
constexpr uint8_t kFinalChainEvmRewardsReportStatusSuccess = 0;
constexpr uint8_t kFinalChainEvmLifecycleStatusCommitted = 0;
constexpr uint8_t kFinalChainEvmLifecycleStatusRejected = 2;
constexpr uint8_t kFinalChainEvmCommitDecisionReadyToPublish = 0;
constexpr uint8_t kFinalChainEvmCommitDecisionRejected = 1;
constexpr uint8_t kFinalChainEvmStateCommitIntentReadyToCommit = 0;
constexpr uint8_t kFinalChainEvmPublicationStatusApplied = 0;
constexpr uint8_t kFinalChainEvmPublicationStatusRejected = 1;
constexpr uint8_t kFinalChainEvmPublicationStatusAlreadyApplied = 2;

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

rust::Vec<rustaxa::FinalizationTransaction> make_finalization_transactions(const SharedTransactions& transactions) {
  rust::Vec<rustaxa::FinalizationTransaction> rust_transactions;
  rust_transactions.reserve(transactions.size());
  for (const auto& transaction : transactions) {
    rustaxa::FinalizationTransaction rust_transaction;
    rust_transaction.hash = into_bytes_array(transaction->getHash());
    rust_transaction.sender = into_address_array(transaction->getSender());
    if (auto const& receiver = transaction->getReceiver(); receiver) {
      rust_transaction.receiver_found = true;
      rust_transaction.receiver = into_address_array(*receiver);
    } else {
      rust_transaction.receiver_found = false;
      rust_transaction.receiver = {};
    }
    rust_transaction.nonce = into_canonical_nonce_vec(transaction->getNonce());
    rust_transaction.value = into_big_endian_vec(transaction->getValue());
    rust_transaction.gas_price = into_big_endian_vec(transaction->getGasPrice());
    rust_transaction.gas_limit = transaction->getGas();
    rust_transaction.data = into_rust_vec(transaction->getData());
    rust_transaction.rlp = into_rust_vec(transaction->rlp());
    rust_transactions.push_back(std::move(rust_transaction));
  }
  return rust_transactions;
}

rust::Vec<rustaxa::FinalizationDagBlock> make_finalization_dag_blocks(
    const std::vector<std::shared_ptr<DagBlock>>& dag_blocks) {
  rust::Vec<rustaxa::FinalizationDagBlock> rust_dag_blocks;
  rust_dag_blocks.reserve(dag_blocks.size());
  for (const auto& dag_block : dag_blocks) {
    rustaxa::FinalizationDagBlock rust_dag_block;
    rust_dag_block.author = into_address_array(dag_block->getSender());
    rust_dag_block.difficulty = dag_block->getDifficulty();
    rust_dag_block.transaction_hashes.reserve(dag_block->getTrxs().size());
    for (const auto& transaction_hash : dag_block->getTrxs()) {
      rustaxa::DagHash rust_transaction_hash;
      rust_transaction_hash.hash = into_bytes_array(transaction_hash);
      rust_dag_block.transaction_hashes.push_back(std::move(rust_transaction_hash));
    }
    rust_dag_blocks.push_back(std::move(rust_dag_block));
  }
  return rust_dag_blocks;
}

rust::Vec<rustaxa::RewardsCertVoteFact> make_rewards_cert_votes(
    const std::vector<std::shared_ptr<PbftVote>>& cert_votes) {
  rust::Vec<rustaxa::RewardsCertVoteFact> rust_votes;
  rust_votes.reserve(cert_votes.size());
  for (const auto& vote : cert_votes) {
    auto weight = vote->getWeight();
    if (!weight) {
      throw std::runtime_error("FinalChain::finalize cert vote is missing validator weight");
    }
    rustaxa::RewardsCertVoteFact fact{};
    fact.voter = into_address_array(vote->getVoterAddr());
    fact.weight = *weight;
    fact.period = vote->getPeriod();
    rust_votes.push_back(fact);
  }
  return rust_votes;
}

rustaxa::FinalChainExecutionRequest make_final_chain_execution_request(const PeriodData& period_data,
                                                                       uint32_t blocks_per_year,
                                                                       uint64_t block_gas_limit) {
  rustaxa::FinalChainExecutionRequest request;
  request.pbft_block_rlp = into_rust_vec(period_data.pbft_blk->rlp(true));
  request.transactions = make_finalization_transactions(period_data.transactions);
  request.finalized_dag_blocks = make_finalization_dag_blocks(period_data.dag_blocks);
  request.blocks_per_year = blocks_per_year;
  request.cert_votes = make_rewards_cert_votes(period_data.previous_block_cert_votes);
  request.block_gas_limit = block_gas_limit;
  request.mode = kFinalChainExecutionModeExternalEvmAllowed;
  return request;
}

rust::Vec<rustaxa::GenesisAccount> make_genesis_accounts(const state_api::Config& config) {
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

rust::Vec<rustaxa::GenesisValidator> make_genesis_validators(const state_api::Config& config) {
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

rustaxa::GenesisDposConfig make_genesis_dpos_config(const state_api::DPOSConfig& config,
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

rustaxa::FinalChainRewardsConfig make_final_chain_rewards_config(const taraxa::FullNodeConfig& config) {
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

std::future<std::shared_ptr<const FinalizationResult>> ready_finalization_result(
    std::shared_ptr<const FinalizationResult> result) {
  std::promise<std::shared_ptr<const FinalizationResult>> promise;
  promise.set_value(std::move(result));
  return promise.get_future();
}

state_api::EVMTransaction to_evm_transaction(const SharedTransaction& trx) {
  return state_api::EVMTransaction{
      trx->getSender(), trx->getGasPrice(), trx->getReceiver(), trx->getNonce(),
      trx->getValue(),  trx->getGas(),      trx->getData(),
  };
}

std::vector<state_api::EVMTransaction> to_evm_transactions(const SharedTransactions& trxs) {
  std::vector<state_api::EVMTransaction> evm_trxs;
  evm_trxs.reserve(trxs.size());
  std::transform(trxs.cbegin(), trxs.cend(), std::back_inserter(evm_trxs), to_evm_transaction);
  return evm_trxs;
}

std::array<uint8_t, 32> zero_hash_array() { return {}; }

rust::Vec<rustaxa::FinalChainEvmLogTopic> make_evm_log_topics(const h256s& topics) {
  rust::Vec<rustaxa::FinalChainEvmLogTopic> rust_topics;
  rust_topics.reserve(topics.size());
  for (const auto& topic : topics) {
    rustaxa::FinalChainEvmLogTopic rust_topic;
    rust_topic.topic = into_bytes_array(topic);
    rust_topics.push_back(std::move(rust_topic));
  }
  return rust_topics;
}

rust::Vec<rustaxa::FinalChainEvmLog> make_evm_logs(const std::vector<state_api::LogRecord>& logs) {
  rust::Vec<rustaxa::FinalChainEvmLog> rust_logs;
  rust_logs.reserve(logs.size());
  for (const auto& log : logs) {
    rustaxa::FinalChainEvmLog rust_log;
    rust_log.address = into_address_array(log.address);
    rust_log.topics = make_evm_log_topics(log.topics);
    rust_log.data = into_rust_vec(log.data);
    rust_logs.push_back(std::move(rust_log));
  }
  return rust_logs;
}

rustaxa::FinalChainEvmExecutionReport make_evm_execution_report(
    const rustaxa::FinalChainEvmExecutionRequest& request,
    const std::vector<state_api::ExecutionResult>& execution_results, const TransactionReceipts& receipts) {
  if (request.transactions.size() != execution_results.size() || request.transactions.size() != receipts.size()) {
    throw DbException("FinalChain::finalize external EVM result count mismatch");
  }

  rustaxa::FinalChainEvmExecutionReport report;
  report.request_id = request.request_id;
  report.status = kFinalChainEvmReportStatusSuccess;
  report.state_root = zero_hash_array();
  report.cumulative_gas_used = receipts.empty() ? 0 : receipts.back().cumulative_gas_used;
  report.results.reserve(execution_results.size());

  for (size_t i = 0; i < execution_results.size(); ++i) {
    const auto& requested = request.transactions[i];
    const auto& execution = execution_results[i];
    const auto& receipt = receipts[i];
    rustaxa::FinalChainEvmTransactionResult result;
    result.position = requested.position;
    result.hash = requested.hash;
    result.status = receipt.status_code;
    result.gas_used = execution.gas_used;
    result.cumulative_gas_used = receipt.cumulative_gas_used;
    result.receipt_rlp = into_rust_vec(util::rlp_enc(receipt));
    result.logs = make_evm_logs(execution.logs);
    if (execution.new_contract_addr) {
      result.new_contract_address_found = true;
      result.new_contract_address = into_address_array(execution.new_contract_addr);
    } else {
      result.new_contract_address_found = false;
      result.new_contract_address = {};
    }
    result.code_error = rust::String(execution.code_err);
    result.consensus_error = rust::String(execution.consensus_err);
    report.results.push_back(std::move(result));
  }

  return report;
}

}  // namespace

FinalChain::FinalChain(const std::shared_ptr<DbStorage>& db, const taraxa::FullNodeConfig& config,
                       [[maybe_unused]] const addr_t& node_addr)
    : db_(db),
      rust_final_chain_(rustaxa::create_final_chain_with_rewards_config(
          db->rustStorage(), config.genesis.pbft.gas_limit, config.genesis.dag_genesis_block.getTimestamp(),
          make_genesis_accounts(config.genesis.state), make_genesis_validators(config.genesis.state),
          make_genesis_dpos_config(config.genesis.state.dpos, config.genesis.state.hardforks.magnolia_hf.block_num),
          make_final_chain_rewards_config(config))),
      rust_execution_api_(rustaxa::create_consensus_execution_api()),
      state_api_([this](auto n) { return blockHash(n).value_or(ZeroHash()); }, config.genesis.state,
                 config.opts_final_chain, {db->stateDbStoragePath().string()}),
      external_evm_state_api_(state_api_, state_api_mutex_),
      config_(config) {
  recoverExternalEvmPendingPublication();
  delegation_delay_ = config.genesis.state.dpos.delegation_delay;
  block_gas_limit_ = config.genesis.pbft.gas_limit;
  max_levels_per_period_ = config.max_levels_per_period;
  auto execution_status = rust_final_chain_.value()->get_execution_status();
  num_executed_dag_blk_ = execution_status.executed_dag_block_count;
  num_executed_trx_ = execution_status.executed_transaction_count;
}

FinalChain::ExternalEvmStateApiClient::ExternalEvmStateApiClient(StateAPI& state_api, std::mutex& state_api_mutex)
    : state_api_(state_api), state_api_mutex_(state_api_mutex) {}

rustaxa::FinalChainSystemTransactionPlanFact FinalChain::ExternalEvmStateApiClient::collectSystemTransactionFacts(
    const rustaxa::FinalChainSystemTransactionRequest& request, bool is_pillar_block_period, uint64_t block_gas_limit,
    const addr_t& bridge_contract_address) {
  state_api::StateDescriptor state_descriptor;
  std::optional<state_api::Account> bridge_contract;
  bool should_finalize = false;
  u256 system_account_nonce = 0;
  {
    std::lock_guard lock(state_api_mutex_);
    state_descriptor = state_api_.get_last_committed_state_descriptor();
    const auto last_committed_evm_block = state_descriptor.blk_num;
    bridge_contract = state_api_.get_account(last_committed_evm_block, bridge_contract_address);
    const auto bridge_contract_has_code = bridge_contract && bridge_contract->code_size;
    if (is_pillar_block_period && bridge_contract_has_code) {
      const static auto should_finalize_method = util::EncodingSolidity::packFunctionCall("shouldFinalizeEpoch()");
      should_finalize = u256(state_api_
                                 .dry_run_transaction(last_committed_evm_block,
                                                      state_api::EVMBlock{dev::ZeroAddress, block_gas_limit, 0,
                                                                          BlockHeader::difficulty()},
                                                      state_api::EVMTransaction{
                                                          dev::ZeroAddress,
                                                          1,
                                                          bridge_contract_address,
                                                          state_api::ZeroAccount.nonce,
                                                          0,
                                                          10000000,
                                                          should_finalize_method,
                                                      })
                                 .code_retval)
                            .convert_to<bool>();
      if (should_finalize) {
        system_account_nonce = state_api_.get_account(last_committed_evm_block, kTaraxaSystemAccount)
                                   .value_or(state_api::ZeroAccount)
                                   .nonce;
      }
    }
  }
  const auto bridge_contract_has_code = bridge_contract && bridge_contract->code_size;

  rustaxa::FinalChainSystemTransactionPlanFact fact;
  fact.request_id = request.request_id;
  fact.period = request.period;
  fact.is_pillar_block_period = is_pillar_block_period;
  fact.bridge_contract_address = into_address_array(bridge_contract_address);
  fact.bridge_contract_found = bridge_contract.has_value();
  fact.bridge_contract_has_code = bridge_contract_has_code;
  fact.should_finalize_epoch = should_finalize;
  fact.system_account_nonce = into_canonical_nonce_vec(system_account_nonce);
  fact.block_gas_limit = block_gas_limit;
  return fact;
}

FinalChain::ExternalEvmStateApiClient::ExecutionOutcome FinalChain::ExternalEvmStateApiClient::executeTransactions(
    const rustaxa::FinalChainEvmExecutionRequest& request, const std::vector<SharedTransaction>& transactions,
    const addr_t& beneficiary, uint64_t block_gas_limit, uint64_t timestamp) {
  auto evm_trxs = to_evm_transactions(transactions);
  std::vector<state_api::ExecutionResult> exec_results;
  {
    std::lock_guard lock(state_api_mutex_);
    exec_results =
        state_api_.execute_transactions({beneficiary, block_gas_limit, timestamp, BlockHeader::difficulty()}, evm_trxs)
            .execution_results;
  }

  ExecutionOutcome outcome;
  outcome.receipts.reserve(exec_results.size());
  gas_t cumulative_gas_used = 0;
  for (const auto& r : exec_results) {
    LogEntries logs;
    logs.reserve(r.logs.size());
    std::transform(r.logs.cbegin(), r.logs.cend(), std::back_inserter(logs),
                   [](const auto& l) { return LogEntry{l.address, l.topics, l.data}; });
    outcome.receipts.emplace_back(TransactionReceipt{
        static_cast<uint8_t>(r.code_err.empty() && r.consensus_err.empty()),
        r.gas_used,
        cumulative_gas_used += r.gas_used,
        std::move(logs),
        r.new_contract_addr ? std::optional(r.new_contract_addr) : std::nullopt,
    });
  }
  outcome.report = make_evm_execution_report(request, exec_results, outcome.receipts);
  return outcome;
}

FinalChain::ExternalEvmStateApiClient::RewardsOutcome FinalChain::ExternalEvmStateApiClient::distributeRewards(
    const rustaxa::FinalChainEvmRewardsRequest& request) {
  std::vector<rewards::BlockStats> rewards_stats;
  rewards_stats.reserve(request.distribution_stats.size());
  for (const auto& encoded_stats : request.distribution_stats) {
    const auto encoded_bytes = into_bytes(encoded_stats.data);
    rewards_stats.push_back(util::rlp_dec<rewards::BlockStats>(dev::RLP(encoded_bytes)));
  }

  h256 state_root;
  u256 total_reward;
  {
    std::lock_guard lock(state_api_mutex_);
    const auto& rewards_result = state_api_.distribute_rewards(rewards_stats);
    state_root = rewards_result.state_root;
    total_reward = rewards_result.total_reward;
  }

  RewardsOutcome outcome;
  outcome.report.request_id = request.request_id;
  outcome.report.period = request.period;
  outcome.report.status = kFinalChainEvmRewardsReportStatusSuccess;
  outcome.report.state_root = into_bytes_array(state_root);
  outcome.report.total_reward = into_big_endian_vec(total_reward);
  return outcome;
}

rustaxa::FinalChainExternalEvmStateCommitResult FinalChain::ExternalEvmStateApiClient::commitState() {
  rustaxa::FinalChainExternalEvmStateCommitResult result;
  result.status = kFinalChainEvmLifecycleStatusCommitted;
  result.error_code = rust::String();
  try {
    std::lock_guard lock(state_api_mutex_);
    state_api_.transition_state_commit();
  } catch (const std::exception& e) {
    result.status = kFinalChainEvmLifecycleStatusRejected;
    result.error_code = rust::String(std::string("STATE_API_COMMIT_FAILED: ") + e.what());
  } catch (...) {
    result.status = kFinalChainEvmLifecycleStatusRejected;
    result.error_code = rust::String("STATE_API_COMMIT_FAILED");
  }
  return result;
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
  auto recovery_report = rust_final_chain_.value()->recover_external_evm_pending_publication(
      state_descriptor.blk_num, into_bytes_array(state_descriptor.state_root));
  if (recovery_report.status == kFinalChainEvmPublicationStatusRejected) {
    throw DbException("FinalChain startup rejected Rust external EVM publication recovery: " +
                      std::string(recovery_report.error_code));
  }
}

void FinalChain::stop() {}

EthBlockNumber FinalChain::delegationDelay() const { return delegation_delay_; }

std::future<std::shared_ptr<const FinalizationResult>> FinalChain::finalize(
    PeriodData&& period_data, std::vector<h256>&& finalized_dag_blk_hashes, uint32_t blocks_per_year,
    std::shared_ptr<DagBlock>&& anchor) {
  auto session = rustaxa::create_final_chain_execution_session(
      make_final_chain_execution_request(period_data, blocks_per_year, block_gas_limit_));
  auto step = rust_execution_api_.value()->consensus_execution_next_execution_request(*session);
  if (step.action == kFinalChainExecutionActionProvideSystemTransactions ||
      step.action == kFinalChainExecutionActionExecuteExternalEvm) {
    auto result = finalizeExternalEvm(std::move(session), std::move(step), std::move(period_data),
                                      std::move(finalized_dag_blk_hashes), std::move(anchor));
    return ready_finalization_result(std::move(result));
  }
  if (step.action == kFinalChainExecutionActionReject) {
    throw DbException("FinalChain::finalize Rust execution runtime rejected request: " + std::string(step.error_code));
  }
  if (step.action != kFinalChainExecutionActionCommitNative) {
    throw DbException("FinalChain::finalize Rust execution runtime returned unexpected action " +
                      std::to_string(step.action));
  }
  auto commit_report =
      rust_execution_api_.value()->consensus_execution_commit_session(*rust_final_chain_.value(), std::move(session));
  if (commit_report.status != kFinalChainExecutionStatusComplete || !commit_report.error_code.empty()) {
    throw DbException("FinalChain::finalize Rust execution runtime failed commit: " +
                      std::string(commit_report.error_code));
  }
  auto header_data = into_string(commit_report.block_header_rlp);
  auto header = BlockHeader::fromRLP(dev::RLP(header_data));
  TransactionReceipts receipts;
  receipts.reserve(commit_report.receipts.size());
  for (auto const& receipt : commit_report.receipts) {
    auto receipt_data = into_string(receipt.data);
    receipts.push_back(util::rlp_dec<TransactionReceipt>(dev::RLP(receipt_data)));
  }
  auto result = std::make_shared<FinalizationResult>(FinalizationResult{
      {
          period_data.pbft_blk->getBeneficiary(),
          period_data.pbft_blk->getTimestamp(),
          std::move(finalized_dag_blk_hashes),
          period_data.pbft_blk->getBlockHash(),
      },
      std::move(header),
      std::move(period_data.transactions),
      std::move(receipts),
  });
  block_finalized_emitter_.emit(result);
  return ready_finalization_result(std::move(result));
}

std::vector<SharedTransaction> FinalChain::makeSystemTransactions(
    const rustaxa::FinalChainSystemTransactionRequest& request) {
  const auto bridge_contract_address = config_.genesis.state.hardforks.ficus_hf.bridge_contract_address;
  const auto is_pillar_block_period =
      config_.genesis.state.hardforks.ficus_hf.isPillarBlockPeriod(request.period + delegationDelay());
  auto plan = rust_execution_api_.value()->consensus_execution_plan_system_transactions(
      external_evm_state_api_.collectSystemTransactionFacts(request, is_pillar_block_period, block_gas_limit_,
                                                            bridge_contract_address));
  if (plan.request_id != request.request_id || plan.period != request.period) {
    throw DbException("FinalChain::makeSystemTransactions Rust plan identity mismatch");
  }

  std::vector<SharedTransaction> system_transactions;
  system_transactions.reserve(plan.transactions.size());
  for (const auto& tx_rlp : plan.transactions) {
    system_transactions.push_back(std::make_shared<SystemTransaction>(into_bytes(tx_rlp.data)));
  }
  return system_transactions;
}

std::shared_ptr<const FinalizationResult> FinalChain::finalizeExternalEvm(
    rust::Box<rustaxa::BridgeFinalChainExecutionSession> session, rustaxa::FinalChainExecutionStep step,
    PeriodData&& period_data, std::vector<h256>&& finalized_dag_blk_hashes, std::shared_ptr<DagBlock>&& anchor) {
  block_applying_emitter_.emit(lastBlockNumber() + 1);

  auto all_transactions = period_data.transactions;
  if (step.action == kFinalChainExecutionActionProvideSystemTransactions) {
    auto system_transactions = makeSystemTransactions(step.system_transaction_request);
    rustaxa::FinalChainSystemTransactionReport system_report;
    system_report.request_id = step.system_transaction_request.request_id;
    system_report.period = step.system_transaction_request.period;
    system_report.transactions.reserve(system_transactions.size());
    for (const auto& trx : system_transactions) {
      rustaxa::TxRlp tx_rlp;
      tx_rlp.data = into_rust_vec(trx->rlp());
      system_report.transactions.push_back(std::move(tx_rlp));
      all_transactions.push_back(trx);
    }
    step =
        rust_execution_api_.value()->consensus_execution_report_system_transactions(*session, std::move(system_report));
  }

  if (step.action != kFinalChainExecutionActionExecuteExternalEvm) {
    throw DbException("FinalChain::finalize expected external EVM execution action, got " +
                      std::to_string(step.action) + ": " + std::string(step.error_code));
  }
  if (step.evm_request.transactions.size() != all_transactions.size()) {
    throw DbException("FinalChain::finalize external EVM request transaction count mismatch");
  }

  auto execution = external_evm_state_api_.executeTransactions(step.evm_request, all_transactions,
                                                               period_data.pbft_blk->getBeneficiary(), block_gas_limit_,
                                                               period_data.pbft_blk->getTimestamp());
  step = rust_execution_api_.value()->consensus_execution_report_execution_result(*rust_final_chain_.value(), *session,
                                                                                  std::move(execution.report));
  if (step.action != kFinalChainExecutionActionDistributeExternalEvmRewards) {
    throw DbException("FinalChain::finalize expected external EVM rewards action, got " + std::to_string(step.action) +
                      ": " + std::string(step.error_code));
  }

  auto rewards_execution = external_evm_state_api_.distributeRewards(step.evm_rewards_request);
  auto commit_plan = rust_execution_api_.value()->consensus_execution_report_rewards_result(
      *session, std::move(rewards_execution.report));
  if (!commit_plan.error_code.empty()) {
    throw DbException("FinalChain::finalize Rust external EVM commit plan failed: " +
                      std::string(commit_plan.error_code));
  }
  auto state_commit_intent = rust_execution_api_.value()->consensus_execution_prepare_external_evm_state_commit(
      *rust_final_chain_.value(), *session,
      rustaxa::FinalChainProposalPeriodDagLevelUpdate{
          .has_update = !!anchor, .level = anchor ? anchor->getLevel() + max_levels_per_period_ : 0});
  if (state_commit_intent.status != kFinalChainEvmStateCommitIntentReadyToCommit ||
      !state_commit_intent.error_code.empty()) {
    throw DbException("FinalChain::finalize Rust external EVM state commit intent failed: " +
                      std::string(state_commit_intent.error_code));
  }

  auto state_commit_result = external_evm_state_api_.commitState();
  if (state_commit_result.status != kFinalChainEvmLifecycleStatusCommitted) {
    const auto error_code = std::string(state_commit_result.error_code);
    auto rejected_decision = rust_execution_api_.value()->consensus_execution_report_state_commit_result(
        *rust_final_chain_.value(), *session, std::move(state_commit_result));
    if (rejected_decision.status != kFinalChainEvmCommitDecisionRejected) {
      throw DbException("FinalChain::finalize Rust external EVM lifecycle unexpectedly accepted failed state commit");
    }
    throw DbException("FinalChain::finalize StateAPI external EVM state commit failed: " + error_code);
  }
  auto decision = rust_execution_api_.value()->consensus_execution_report_state_commit_result(
      *rust_final_chain_.value(), *session, std::move(state_commit_result));
  if (decision.status != kFinalChainEvmCommitDecisionReadyToPublish || !decision.error_code.empty()) {
    throw DbException("FinalChain::finalize Rust external EVM lifecycle rejected: " + std::string(decision.error_code));
  }

  auto publication_step = rust_execution_api_.value()->consensus_execution_next_execution_request(*session);
  if (publication_step.action != kFinalChainExecutionActionPublishExternalEvmStorage) {
    throw DbException("FinalChain::finalize expected external EVM storage publication action, got " +
                      std::to_string(publication_step.action) + ": " + std::string(publication_step.error_code));
  }
  auto publication_report =
      rust_execution_api_.value()->consensus_execution_publish_state_commit(*rust_final_chain_.value(), *session);
  if (publication_report.status != kFinalChainEvmPublicationStatusApplied &&
      publication_report.status != kFinalChainEvmPublicationStatusAlreadyApplied) {
    throw DbException("FinalChain::finalize Rust external EVM publication rejected: " +
                      std::string(publication_report.error_code));
  }
  auto publication_complete_step = rust_execution_api_.value()->consensus_execution_next_execution_request(*session);
  if (publication_complete_step.action != kFinalChainExecutionActionComplete) {
    throw DbException("FinalChain::finalize expected completed external EVM publication session, got " +
                      std::to_string(publication_complete_step.action) + ": " +
                      std::string(publication_complete_step.error_code));
  }
  num_executed_dag_blk_ = publication_report.executed_dag_block_count;
  num_executed_trx_ = publication_report.executed_transaction_count;

  auto block_header_data = into_string(rust_final_chain_.value()->get_block_header(publication_report.period));
  auto blk_header = BlockHeader::fromRLP(dev::RLP(block_header_data));
  auto result = std::make_shared<FinalizationResult>(FinalizationResult{
      {
          period_data.pbft_blk->getBeneficiary(),
          period_data.pbft_blk->getTimestamp(),
          std::move(finalized_dag_blk_hashes),
          period_data.pbft_blk->getBlockHash(),
      },
      std::move(blk_header),
      std::move(all_transactions),
      std::move(execution.receipts),
  });
  block_finalized_emitter_.emit(result);
  return result;
}

std::shared_ptr<const BlockHeader> FinalChain::blockHeader(std::optional<EthBlockNumber> n) const {
  auto const block_number = n.value_or(lastBlockNumber());
  auto rust_header = rust_final_chain_.value()->get_block_header(static_cast<uint64_t>(block_number));
  if (rust_header.empty()) {
    return nullptr;
  }

  auto header_data = into_string(rust_header);
  return BlockHeader::fromRLP(dev::RLP(header_data));
}

EthBlockNumber FinalChain::lastBlockNumber() const { return rust_final_chain_.value()->get_last_block_number(); }

std::optional<EthBlockNumber> FinalChain::blockNumber(h256 const& h) const {
  auto rust_lookup = rust_final_chain_.value()->get_block_number(into_bytes_array(h));
  if (!rust_lookup.found) {
    return std::nullopt;
  }
  return rust_lookup.value;
}

std::optional<h256> FinalChain::blockHash(std::optional<EthBlockNumber> n) const {
  auto const block_number = n.value_or(lastBlockNumber());
  auto rust_hash = rust_final_chain_.value()->get_block_hash(static_cast<uint64_t>(block_number));
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
  auto rust_transactions = rust_final_chain_.value()->get_transaction_rlps(block_number);
  ret.reserve(rust_transactions.size());
  for (auto const& transaction : rust_transactions) {
    ret.push_back(std::make_shared<Transaction>(into_bytes(transaction.data), false));
  }
  return ret;
}

std::optional<TransactionLocation> FinalChain::transactionLocation(h256 const& trx_hash) const {
  auto rust_location = rust_final_chain_.value()->get_transaction_location(into_bytes_array(trx_hash));
  if (rust_location.empty()) {
    return std::nullopt;
  }
  auto location_data = into_string(rust_location);
  return TransactionLocation::fromRlp(dev::RLP(location_data));
}

std::optional<TransactionReceipt> FinalChain::transactionReceipt(EthBlockNumber blk_n, uint64_t position,
                                                                 std::optional<trx_hash_t>) const {
  auto receipt = rust_final_chain_.value()->get_transaction_receipt(blk_n, position);
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
  return rust_final_chain_.value()->get_transaction_count(static_cast<uint64_t>(n.value_or(lastBlockNumber())));
}

std::vector<EthBlockNumber> FinalChain::withBlockBloom(LogBloom const& b, EthBlockNumber from,
                                                       EthBlockNumber to) const {
  std::array<uint8_t, 256> bloom{};
  std::memcpy(bloom.data(), b.data(), bloom.size());
  auto rust_blocks = rust_final_chain_.value()->get_blocks_with_bloom(bloom, from, to);
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

  auto rust_account = blk_n.has_value()
                          ? rust_final_chain_.value()->get_account_at_block(*blk_n, into_address_array(addr))
                          : rust_final_chain_.value()->get_account(into_address_array(addr));
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
    auto outcome = rust_final_chain_.value()->call(std::move(request));

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
  return rust_final_chain_.value()->get_dpos_eligible_total_vote_count(blk_num);
}

uint64_t FinalChain::dposEligibleVoteCount(EthBlockNumber blk_num, addr_t const& addr) const {
  return rust_final_chain_.value()->get_dpos_eligible_vote_count(blk_num, into_address_array(addr));
}

bool FinalChain::dposIsEligible(EthBlockNumber blk_num, addr_t const& addr) const {
  return rust_final_chain_.value()->get_dpos_is_eligible(blk_num, into_address_array(addr));
}

void FinalChain::prune(EthBlockNumber) { throw_unimplemented_final_chain_api("prune"); }

void FinalChain::waitForFinalized() { std::this_thread::sleep_for(std::chrono::milliseconds(10)); }

std::vector<state_api::ValidatorStake> FinalChain::dposValidatorsTotalStakes(EthBlockNumber blk_num) const {
  auto rust_stakes = rust_final_chain_.value()->get_dpos_validators_total_stakes(blk_num);
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
  auto delegated = rust_final_chain_.value()->get_dpos_total_amount_delegated(blk_num);
  return dev::fromBigEndian<u256>(dev::bytes(delegated.begin(), delegated.end()));
}

uint64_t FinalChain::dposYield(EthBlockNumber blk_num) const {
  return rust_final_chain_.value()->get_dpos_yield(blk_num);
}

u256 FinalChain::dposTotalSupply(EthBlockNumber blk_num) const {
  auto supply = rust_final_chain_.value()->get_dpos_total_supply(blk_num);
  return dev::fromBigEndian<u256>(dev::bytes(supply.begin(), supply.end()));
}

h256 FinalChain::getBridgeRoot(EthBlockNumber blk_num) const {
  const static auto get_bridge_root_method = util::EncodingSolidity::packFunctionCall("getBridgeRoot()");
  const auto requested_block = blk_num;
  const auto bridge_contract_address = config_.genesis.state.hardforks.ficus_hf.bridge_contract_address;
  const auto state_descriptor = external_evm_state_api_.lastCommittedStateDescriptor();
  if (requested_block > state_descriptor.blk_num) {
    if (!external_evm_state_api_.accountHasCode(state_descriptor.blk_num, bridge_contract_address)) {
      return ZeroHash();
    }
    throw DbException("FinalChain::getBridgeRoot requires committed external-EVM state for block " +
                      std::to_string(requested_block));
  }
  if (!external_evm_state_api_.accountHasCode(requested_block, bridge_contract_address)) {
    return ZeroHash();
  }
  const auto block_header = blockHeader(requested_block);
  if (!block_header) {
    throw DbException("FinalChain::getBridgeRoot missing committed block header for block " +
                      std::to_string(requested_block));
  }

  const auto result = external_evm_state_api_.dryRunTransaction(
      *block_header,
      state_api::EVMTransaction{dev::ZeroAddress, 1, bridge_contract_address, state_api::ZeroAccount.nonce, 0, 10000000,
                                get_bridge_root_method},
      true);
  if (!result.code_err.empty() || !result.consensus_err.empty()) {
    throw DbException("FinalChain::getBridgeRoot bridge-contract read failed: " + result.code_err +
                      result.consensus_err);
  }
  return into_h256(result.code_retval, "getBridgeRoot");
}

h256 FinalChain::getBridgeEpoch(EthBlockNumber blk_num) const {
  const static auto get_bridge_epoch_method = util::EncodingSolidity::packFunctionCall("finalizedEpoch()");
  const auto requested_block = blk_num;
  const auto bridge_contract_address = config_.genesis.state.hardforks.ficus_hf.bridge_contract_address;
  const auto state_descriptor = external_evm_state_api_.lastCommittedStateDescriptor();
  if (requested_block > state_descriptor.blk_num) {
    if (!external_evm_state_api_.accountHasCode(state_descriptor.blk_num, bridge_contract_address)) {
      return ZeroHash();
    }
    throw DbException("FinalChain::getBridgeEpoch requires committed external-EVM state for block " +
                      std::to_string(requested_block));
  }
  if (!external_evm_state_api_.accountHasCode(requested_block, bridge_contract_address)) {
    return ZeroHash();
  }
  const auto block_header = blockHeader(requested_block);
  if (!block_header) {
    throw DbException("FinalChain::getBridgeEpoch missing committed block header for block " +
                      std::to_string(requested_block));
  }

  const auto result = external_evm_state_api_.dryRunTransaction(
      *block_header,
      state_api::EVMTransaction{dev::ZeroAddress, 1, bridge_contract_address, state_api::ZeroAccount.nonce, 0, 10000000,
                                get_bridge_epoch_method},
      true);
  if (!result.code_err.empty() || !result.consensus_err.empty()) {
    throw DbException("FinalChain::getBridgeEpoch bridge-contract read failed: " + result.code_err +
                      result.consensus_err);
  }
  return into_h256(result.code_retval, "getBridgeEpoch");
}

std::pair<val_t, bool> FinalChain::getBalance(addr_t const& addr) const {
  if (auto account = getAccount(addr)) {
    return {account->balance, true};
  }
  return {0, false};
}

std::shared_ptr<const FinalizationResult> FinalChain::finalize_(PeriodData&&, std::vector<h256>&&, uint32_t,
                                                                std::shared_ptr<DagBlock>&&) {
  throw_unimplemented_final_chain_api("finalize_");
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
