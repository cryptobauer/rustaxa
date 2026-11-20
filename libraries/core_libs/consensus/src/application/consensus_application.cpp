#ifdef RUSTAXA_ENABLE

#include "consensus/consensus_application.hpp"

#include <algorithm>
#include <array>
#include <cstddef>
#include <iterator>
#include <stdexcept>
#include <utility>

#include "common/constants.hpp"
#include "common/encoding_rlp.hpp"
#include "config/config.hpp"
#include "config/version.hpp"
#include "consensus/external_evm_state_owner.hpp"
#include "consensus/native_config_projection.hpp"

namespace taraxa {
using production_config_detail::toBridgeU256;
namespace {

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& value) {
  rust::Vec<uint8_t> out;
  out.reserve(value.size());
  std::copy(value.begin(), value.end(), std::back_inserter(out));
  return out;
}

}  // namespace

[[maybe_unused]] SharedConsensusApplication createConsensusApplication(const FullNodeConfig& config) {
  std::error_code directory_error;
  fs::create_directories(config.db_path, directory_error);
  if (directory_error) {
    throw std::runtime_error("FINAL_CHAIN_STATE_DB_PARENT_CREATE_FAILED: " + directory_error.message());
  }
  auto external_evm_state = std::make_shared<ExternalEvmStateOwner>(config);
  const auto state_api_descriptor = external_evm_state->lastCommittedStateDescriptor();
  auto application = std::make_shared<ConsensusApplication>(
      rustaxa::create_consensus_application(makeConsensusBootstrapConfig(config, state_api_descriptor.blk_num, state_api_descriptor.state_root.asArray())),
      external_evm_state);
  external_evm_state->bindApplication(application);
  application->recoverExternalEvmPendingPublication(application);
  return application;
}

ConsensusApplication::ConsensusApplication(rust::Box<rustaxa::BridgeConsensusApplication> service)
    : service_(std::move(service)),
      query_client_(std::make_shared<rust::Box<rustaxa::BridgeConsensusQueryApi>>(
          rustaxa::create_consensus_query_api(*service_))) {}

ConsensusApplication::ConsensusApplication(rust::Box<rustaxa::BridgeConsensusApplication> service,
                                           std::shared_ptr<ExternalEvmStateOwner> external_evm_state)
    : ConsensusApplication(std::move(service)) {
  external_evm_state_ = std::move(external_evm_state);
  if (!external_evm_state_) throw std::invalid_argument("ConsensusApplication requires ExternalEvmStateOwner");
}

ConsensusApplication::~ConsensusApplication() = default;

ExternalEvmStateOwner& ConsensusApplication::legacyExecution() const {
  if (!external_evm_state_) throw std::runtime_error("RUST_EXECUTION_OPERATION_UNAVAILABLE");
  return *external_evm_state_;
}

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
  if (!usesRustExecution()) return legacyExecution().account(address, block_number);
  const auto account = (*query_client_)->consensus_query_finalized_account(
      rustaxa::FinalizedAccountField::Account, !block_number.has_value(), block_number.value_or(0), address.asArray(), {});
  if (!account.found) return std::nullopt;
  return state_api::Account{dev::fromBigEndian<u256>(bytes(account.nonce.begin(), account.nonce.end())),
                            dev::fromBigEndian<u256>(bytes(account.balance.begin(), account.balance.end())),
                            h256(account.storage_root.data(), h256::ConstructFromPointer),
                            h256(account.code_hash.data(), h256::ConstructFromPointer), account.code_size};
}

h256 ConsensusApplication::getAccountStorage(const addr_t& address, const u256& key,
                                             std::optional<EthBlockNumber> block_number) const {
  if (!usesRustExecution()) return legacyExecution().accountStorage(address, key, block_number);
  const auto word = (*query_client_)->consensus_query_finalized_account(
      rustaxa::FinalizedAccountField::Storage, !block_number.has_value(), block_number.value_or(0),
      address.asArray(), toBridgeU256(key)).storage_word;
  return h256(word.data(), h256::ConstructFromPointer);
}

bytes ConsensusApplication::getCode(const addr_t& address, std::optional<EthBlockNumber> block_number) const {
  if (!usesRustExecution()) return legacyExecution().code(address, block_number);
  const auto code = (*query_client_)->consensus_query_finalized_account(
      rustaxa::FinalizedAccountField::Code, !block_number.has_value(), block_number.value_or(0),
      address.asArray(), {}).code;
  return {code.begin(), code.end()};
}

state_api::ExecutionResult ConsensusApplication::call(const state_api::EVMTransaction& transaction,
                                                      std::optional<EthBlockNumber> block_number) const {
  if (!usesRustExecution()) return legacyExecution().call(transaction, block_number);
  const auto period = block_number ? *block_number : (*query_client_)->consensus_query_final_chain_last_block_number();
  return queryFinalizedCall(transaction, period);
}

state_api::ExecutionResult ConsensusApplication::queryFinalizedCall(const state_api::EVMTransaction& transaction,
                                                                    EthBlockNumber block_number) const {
  rustaxa::FinalChainNativeCall request{};
  request.block_number = block_number;
  request.sender = transaction.from.asArray();
  request.receiver_found = transaction.to.has_value();
  if (transaction.to) request.receiver = transaction.to->asArray();
  request.value = toBridgeBytes(dev::toBigEndian(u256(transaction.value)));
  request.gas_price = toBridgeBytes(dev::toBigEndian(transaction.gas_price));
  request.gas_limit = transaction.gas;
  request.input = toBridgeBytes(transaction.input);
  const auto outcome = (*query_client_)->consensus_query_final_chain_native_call(std::move(request), {});
  state_api::ExecutionResult result;
  result.code_retval.assign(outcome.code_retval.begin(), outcome.code_retval.end());
  result.logs.reserve(outcome.logs.size());
  for (const auto& log : outcome.logs) {
    auto& converted = result.logs.emplace_back();
    converted.address = addr_t(log.address.data(), addr_t::ConstructFromPointer);
    converted.topics.reserve(log.topics.size());
    for (const auto& topic : log.topics) {
      converted.topics.emplace_back(topic.topic.data(), h256::ConstructFromPointer);
    }
    converted.data.assign(log.data.begin(), log.data.end());
  }
  result.gas_used = outcome.gas_used;
  result.code_err = std::string(outcome.code_err);
  result.consensus_err = std::string(outcome.consensus_err);
  return result;
}

std::string ConsensusApplication::trace(std::vector<state_api::EVMTransaction> state_transactions,
                                        std::vector<state_api::EVMTransaction> transactions,
                                        EthBlockNumber block_number, std::optional<state_api::Tracing> params) const {
  if (!usesRustExecution()) {
    return legacyExecution().trace(std::move(state_transactions), std::move(transactions), block_number, params);
  }
  if (params) throw std::runtime_error("PRODUCTION_OPENETHEREUM_TRACE_UNAVAILABLE");
  dev::RLPStream stream;
  util::rlp_tuple(stream, state_transactions, transactions);
  const auto& encoded = stream.out();
  rustaxa::FinalChainNativeCall request{};
  request.block_number = block_number;
  const auto outcome = (*query_client_)->consensus_query_final_chain_native_call(
      std::move(request), rust::Slice<const uint8_t>(encoded.data(), encoded.size()));
  return {outcome.code_retval.begin(), outcome.code_retval.end()};
}

void ConsensusApplication::pruneFinalChain(EthBlockNumber block_number) const {
  legacyExecution().prune(block_number);
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
