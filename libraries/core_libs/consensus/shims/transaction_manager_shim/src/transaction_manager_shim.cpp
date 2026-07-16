#include <algorithm>
#include <atomic>
#include <cstring>
#include <future>
#include <mutex>
#include <optional>
#include <shared_mutex>
#include <stdexcept>
#include <thread>
#include <unordered_set>
#include <utility>
#include <vector>

#include "common/encoding_rlp.hpp"
#include "config/config.hpp"
#include "dag/dag_block.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/system_transaction.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa {
namespace {

std::array<uint8_t, 32> toBridgeHash(const trx_hash_t& hash) {
  std::array<uint8_t, 32> bytes{};
  std::memcpy(bytes.data(), hash.data(), bytes.size());
  return bytes;
}

trx_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return trx_hash_t(hash.data(), trx_hash_t::ConstructFromPointer);
}

addr_t fromBridgeAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

std::array<uint8_t, 20> toBridgeAddress(const addr_t& address) {
  std::array<uint8_t, 20> bytes{};
  std::memcpy(bytes.data(), address.data(), bytes.size());
  return bytes;
}

dev::bytes fromBridgeBytes(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

rust::Vec<uint8_t> cloneBridgeBytes(const rust::Vec<uint8_t>& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(byte);
  }
  return out;
}

dev::bytes toDevBytes(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

state_api::ExecutionResult executionResultFromBridgeBytes(const rust::Vec<uint8_t>& bytes) {
  const auto encoded = toDevBytes(bytes);
  return util::rlp_dec<state_api::ExecutionResult>(dev::RLP(encoded));
}

rust::Vec<uint8_t> executionResultToBridgeBytes(const state_api::ExecutionResult& result) {
  return toBridgeBytes(util::rlp_enc(result));
}

constexpr uint8_t kLegacyTransactionSourceRegular = 0;
constexpr uint8_t kVerifyNotFinalizedRecentSidecar = 1;
constexpr uint8_t kTMVerifyTransactionAccepted = 0;
constexpr uint8_t kTMVerifyTransactionChainIdMismatch = 1;
constexpr uint8_t kTMVerifyTransactionInvalidGas = 2;
constexpr uint8_t kTMVerifyTransactionIntrinsicGas = 3;
constexpr uint8_t kTMVerifyTransactionInvalidSignature = 4;
constexpr uint8_t kTMVerifyTransactionGasPrice = 5;

constexpr uint8_t kTMQueueStatusInserted = 0;
constexpr uint8_t kTMQueueStatusInsertedNonProposable = 1;
constexpr uint8_t kTMQueueStatusKnown = 2;
constexpr uint8_t kTMQueueStatusOverflow = 3;
constexpr uint8_t kTMAdmissionShellIntentLogInserted = 1;
constexpr uint8_t kTMAdmissionShellIntentEmitTransactionAdded = 2;

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

constexpr uint8_t kTMTransactionViewSourceMissing = 0;
constexpr uint8_t kTMTransactionViewSourceQueue = 1;
constexpr uint8_t kTMTransactionViewSourceNonFinalizedSidecar = 2;
constexpr uint8_t kTMTransactionViewSourceRecentlyFinalizedSidecar = 3;
constexpr uint8_t kTMTransactionViewSourceStoragePending = 4;
constexpr uint8_t kTMTransactionViewSourceStorageFinalizedRegular = 5;
constexpr uint8_t kTMTransactionViewSourceStorageFinalizedSystem = 6;

std::shared_ptr<Transaction> materializeTransactionView(const rustaxa::TransactionManagerTransactionView& view,
                                                        const char* error_prefix) {
  if (!view.found) {
    return nullptr;
  }

  std::shared_ptr<Transaction> transaction;
  if (view.source == kTMTransactionViewSourceStorageFinalizedSystem) {
    transaction = std::make_shared<SystemTransaction>(fromBridgeBytes(view.tx_rlp));
  } else if (view.source == kTMTransactionViewSourceQueue ||
             view.source == kTMTransactionViewSourceNonFinalizedSidecar ||
             view.source == kTMTransactionViewSourceRecentlyFinalizedSidecar ||
             view.source == kTMTransactionViewSourceStoragePending ||
             view.source == kTMTransactionViewSourceStorageFinalizedRegular) {
    transaction = std::make_shared<Transaction>(fromBridgeBytes(view.tx_rlp));
  } else if (view.source != kTMTransactionViewSourceMissing) {
    throw DbException(std::string(error_prefix) + ": Rust returned an unknown transaction view source");
  } else {
    return nullptr;
  }

  if (transaction->getHash() != fromBridgeHash(view.hash)) {
    throw DbException(std::string(error_prefix) + ": Rust returned transaction RLP that does not match the view hash");
  }
  return transaction;
}

std::shared_ptr<Transaction> materializeQueuedTransaction(const rustaxa::TransactionQueueStoredTransaction& stored,
                                                          const char* error_prefix) {
  if (!stored.found) {
    return nullptr;
  }
  auto transaction = std::make_shared<Transaction>(fromBridgeBytes(stored.tx_rlp));
  if (transaction->getHash() != fromBridgeHash(stored.hash)) {
    throw DbException(std::string(error_prefix) +
                      ": Rust returned queued transaction RLP that does not match the hash");
  }
  return transaction;
}

rust::Vec<rustaxa::TransactionManagerTransactionViewRequest> buildTransactionViewRequests(
    const std::vector<trx_hash_t>& hashes) {
  rust::Vec<rustaxa::TransactionManagerTransactionViewRequest> requests;
  requests.reserve(hashes.size());
  for (size_t i = 0; i < hashes.size(); ++i) {
    rustaxa::TransactionManagerTransactionViewRequest request;
    request.input_index = static_cast<uint64_t>(i);
    request.hash = toBridgeHash(hashes[i]);
    requests.push_back(std::move(request));
  }
  return requests;
}

rust::Vec<rustaxa::TransactionManagerSidecarLookupRequest> buildSidecarLookupRequests(
    const std::vector<trx_hash_t>& hashes) {
  rust::Vec<rustaxa::TransactionManagerSidecarLookupRequest> requests;
  requests.reserve(hashes.size());
  for (size_t i = 0; i < hashes.size(); ++i) {
    rustaxa::TransactionManagerSidecarLookupRequest request;
    request.input_index = static_cast<uint64_t>(i);
    request.hash = toBridgeHash(hashes[i]);
    requests.push_back(std::move(request));
  }
  return requests;
}

rustaxa::TransactionManagerSidecarInsertInput toSidecarPayload(const rustaxa::LegacyTransactionInspection& envelope) {
  rustaxa::TransactionManagerSidecarInsertInput input;
  input.hash = envelope.hash;
  input.trx_rlp = cloneBridgeBytes(envelope.tx_rlp);
  return input;
}

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

val_t fromBridgeU256(const std::array<uint8_t, 32>& value) {
  return dev::fromBigEndian<val_t>(dev::bytes(value.begin(), value.end()));
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

rust::Vec<rustaxa::GasPricerGasPrice> gasPricesFromTransactions(const SharedTransactions& trxs) {
  rust::Vec<rustaxa::GasPricerGasPrice> gas_prices;
  gas_prices.reserve(trxs.size());
  for (const auto& trx : trxs) {
    rustaxa::GasPricerGasPrice gas_price;
    gas_price.price = toBridgeU256(trx->getGasPrice());
    gas_prices.push_back(std::move(gas_price));
  }
  return gas_prices;
}

rustaxa::LegacyTransactionInspection inspectRegularTransaction(const std::shared_ptr<Transaction>& transaction,
                                                               const char* error_prefix) {
  if (!transaction) {
    throw std::invalid_argument(std::string(error_prefix) + ": transaction is null");
  }
  auto envelope = [&]() {
    try {
      return rustaxa::inspect_legacy_transaction_rlp(toBridgeBytes(transaction->rlp()),
                                                     kLegacyTransactionSourceRegular);
    } catch (const std::exception& e) {
      throw std::runtime_error(std::string(error_prefix) + ": " + e.what());
    }
  }();
  if (fromBridgeHash(envelope.hash) != transaction->getHash()) {
    throw std::runtime_error(std::string(error_prefix) + ": Rust transaction envelope hash mismatch");
  }
  return envelope;
}

rustaxa::LegacyTransactionInspection inspectRegularTransactionPayload(const trx_hash_t& expected_hash,
                                                                      const dev::bytes& transaction_rlp,
                                                                      const char* error_prefix) {
  auto envelope = [&]() {
    try {
      return rustaxa::inspect_legacy_transaction_rlp(toBridgeBytes(transaction_rlp), kLegacyTransactionSourceRegular);
    } catch (const std::exception& e) {
      throw std::runtime_error(std::string(error_prefix) + ": " + e.what());
    }
  }();
  if (fromBridgeHash(envelope.hash) != expected_hash) {
    throw std::runtime_error(std::string(error_prefix) + ": transaction payload hash mismatch");
  }
  return envelope;
}

std::array<uint8_t, 20> requireEnvelopeSender(const rustaxa::LegacyTransactionInspection& envelope,
                                              const char* error_prefix) {
  if (!envelope.sender_found) {
    throw std::runtime_error(std::string(error_prefix) + ": Rust transaction envelope has no recovered sender");
  }
  return envelope.sender;
}

rustaxa::TransactionQueueInsertInput toRuntimeQueueInsertInput(const rustaxa::LegacyTransactionInspection& envelope,
                                                               bool proposable, uint64_t last_block_number) {
  rustaxa::TransactionQueueInsertInput input;
  input.hash = envelope.hash;
  input.sender = requireEnvelopeSender(envelope, "RUST_TX_MANAGER_QUEUE_ENVELOPE_FAILED");
  input.nonce = envelope.nonce;
  input.gas_price = envelope.gas_price;
  input.gas = envelope.gas_limit;
  input.data_size = envelope.data_size;
  input.tx_rlp = cloneBridgeBytes(envelope.tx_rlp);
  input.proposable = proposable;
  input.last_block_number = last_block_number;
  return input;
}

std::pair<bool, std::string> verifyTransactionResultFromRustStatus(uint8_t status, uint64_t chain_id,
                                                                   uint64_t expected_chain_id) {
  switch (status) {
    case kTMVerifyTransactionAccepted:
      return {true, ""};
    case kTMVerifyTransactionChainIdMismatch:
      return {false, "chain_id mismatch " + std::to_string(chain_id) + " " + std::to_string(expected_chain_id)};
    case kTMVerifyTransactionInvalidGas:
      return {false, "invalid gas"};
    case kTMVerifyTransactionIntrinsicGas:
      return {false, "intrinsic gas too low"};
    case kTMVerifyTransactionInvalidSignature:
      return {false, "invalid signature"};
    case kTMVerifyTransactionGasPrice:
      return {false, "gas_price too low"};
    default:
      throw std::runtime_error("TransactionManager shim received unknown verify status from Rust bridge");
  }
}

TransactionStatus transactionStatusFromBridge(uint8_t status) {
  switch (status) {
    case kTMQueueStatusInserted:
      return TransactionStatus::Inserted;
    case kTMQueueStatusInsertedNonProposable:
      return TransactionStatus::InsertedNonProposable;
    case kTMQueueStatusKnown:
      return TransactionStatus::Known;
    case kTMQueueStatusOverflow:
      return TransactionStatus::Overflow;
    default:
      throw std::runtime_error("TransactionManager shim received unknown queue status from Rust bridge");
  }
}

}  // namespace

SharedDagTransactionService createDagTransactionService(const FullNodeConfig& config, DbStorage& db) {
  return std::make_shared<DagTransactionService>(rustaxa::create_dag_transaction_service_from_storage(
      db.rustStorage(), config.genesis.dag_genesis_block.getHash().asArray(), config.dag_expiry_limit,
      config.max_levels_per_period, sortitionRuntimeConfigFromNodeConfig(config),
      rustaxa::TransactionQueueConfig{config.transactions_pool_size}, gasPricerConfigFromNodeConfig(config),
      config.propose_dag_gas_limit));
}

SharedDagTransactionService createTransactionManagerCompatibilityService(const FullNodeConfig& config, DbStorage& db) {
  return std::make_shared<DagTransactionService>(rustaxa::create_dag_transaction_service_for_transaction_manager(
      db.rustStorage(), rustaxa::TransactionQueueConfig{config.transactions_pool_size},
      gasPricerConfigFromNodeConfig(config), config.propose_dag_gas_limit));
}

TransactionManager::TransactionManager(const FullNodeConfig& conf, std::shared_ptr<DbStorage> db,
                                       std::shared_ptr<final_chain::FinalChain> final_chain, addr_t node_addr)
    : TransactionManager(conf, db, std::move(final_chain), node_addr,
                         createTransactionManagerCompatibilityService(conf, *db)) {}

TransactionManager::TransactionManager(const FullNodeConfig& conf, std::shared_ptr<DbStorage> db,
                                       std::shared_ptr<final_chain::FinalChain> final_chain, addr_t node_addr,
                                       SharedDagTransactionService dag_transaction_service)
    : kConf(conf),
      final_chain_(std::move(final_chain)),
      dag_transaction_service_(std::move(dag_transaction_service)),
      estimation_thread_pool_(std::thread::hardware_concurrency() / 2) {
  static_cast<void>(db);
  if (!dag_transaction_service_) {
    throw std::invalid_argument("TransactionManager requires a DAG/transaction service");
  }
  LOG_OBJECTS_CREATE("TRXMGR");
}

class TransactionManagerRustShimAccess {
 public:
  static std::shared_mutex& transactionsMutex(TransactionManager& manager) { return manager.transactions_mutex_; }
  static std::shared_mutex& transactionsMutex(const TransactionManager& manager) {
    return const_cast<TransactionManager&>(manager).transactions_mutex_;
  }

  static uint64_t rustFinalChainLastBlockNumber(const TransactionManager& manager) {
    if (!manager.final_chain_) {
      throw std::runtime_error("TransactionManager requires FinalChain for Rust FinalChain height facts");
    }
    return manager.final_chain_->lastBlockNumber();
  }

  static std::pair<bool, std::string> verifyTransaction(const TransactionManager& manager,
                                                        const rustaxa::LegacyTransactionInspection& envelope) {
    if (!manager.final_chain_) {
      return {true, ""};
    }

    const auto fact = buildVerifyTransactionFact(manager, envelope);

    const auto outcome = [&]() {
      try {
        return rustaxa::transaction_manager_verify_transaction(fact);
      } catch (const std::exception& e) {
        throw std::runtime_error(std::string("RUST_TX_MANAGER_VERIFY_TRANSACTION_FAILED: ") + e.what());
      }
    }();

    return verifyTransactionResultFromRustStatus(outcome.status, fact.chain_id, fact.expected_chain_id);
  }

  static std::pair<bool, std::string> verifyTransaction(const TransactionManager& manager,
                                                        const std::shared_ptr<Transaction>& trx) {
    const auto envelope = inspectRegularTransaction(trx, "RUST_TX_MANAGER_VERIFY_ENVELOPE_FAILED");
    return verifyTransaction(manager, envelope);
  }

  static rustaxa::TransactionManagerVerifyTransactionFact buildVerifyTransactionFact(
      const TransactionManager& manager, const rustaxa::LegacyTransactionInspection& envelope) {
    const auto block_num = rustFinalChainLastBlockNumber(manager);
    rustaxa::TransactionManagerVerifyTransactionFact fact;
    fact.tx_hash = envelope.hash;
    fact.chain_id = envelope.chain_id;
    fact.expected_chain_id = manager.kConf.genesis.chain_id;
    fact.gas_limit = envelope.gas_limit;
    fact.max_gas_limit = manager.kConf.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
    fact.last_block_number = block_num;
    fact.cornus_active = manager.kConf.genesis.state.hardforks.isOnCornusHardfork(block_num);
    fact.intrinsic_gas_covered = envelope.intrinsic_gas_covered;
    fact.signature_valid = envelope.signature_valid && envelope.sender_found;
    fact.gas_price = envelope.gas_price;
    fact.minimum_gas_price = toBridgeU256(val_t(manager.kConf.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
    return fact;
  }

  static rustaxa::TransactionManagerValidatedInsertRuntimeFact buildValidatedInsertRuntimeFact(
      const TransactionManager& manager, const rustaxa::LegacyTransactionInspection& envelope,
      bool insert_non_proposable) {
    rustaxa::TransactionManagerValidatedInsertRuntimeFact fact;
    fact.tx_hash = envelope.hash;
    fact.sender = requireEnvelopeSender(envelope, "RUST_TX_MANAGER_ADMISSION_ENVELOPE_FAILED");
    fact.transaction_nonce = envelope.nonce;
    fact.transaction_cost = envelope.cost;
    fact.gas_limit = envelope.gas_limit;
    fact.propose_dag_gas_limit = manager.kConf.propose_dag_gas_limit;
    fact.insert_non_proposable = insert_non_proposable;
    return fact;
  }

  static std::optional<state_api::Account> latestAccountFact(const TransactionManager& manager, const addr_t& sender) {
    try {
      return manager.final_chain_->getAccount(sender);
    } catch (const std::exception&) {
      return std::nullopt;
    }
  }

  static rust::Vec<rustaxa::TransactionQueueAccountNonceFact> buildAccountNonceFacts(
      const TransactionManager& manager) {
    rust::Vec<rustaxa::TransactionQueueAccountNonceFact> account_nonce_facts;
    const auto proposable_senders =
        manager.dag_transaction_service_->service().transaction_manager_runtime_queue_proposable_accounts();
    account_nonce_facts.reserve(proposable_senders.size());

    for (const auto& sender : proposable_senders) {
      const auto account = latestAccountFact(manager, fromBridgeAddress(sender.sender));
      rustaxa::TransactionQueueAccountNonceFact fact;
      fact.sender = sender.sender;
      fact.account_found = account.has_value();
      fact.account_nonce = account.has_value() ? toBridgeU256(account->nonce) : std::array<uint8_t, 32>{};
      account_nonce_facts.push_back(std::move(fact));
    }

    return account_nonce_facts;
  }

  static rustaxa::TransactionManagerFinalChainAdmissionFact buildFinalChainAdmissionFact(
      const TransactionManager& manager, const rustaxa::LegacyTransactionInspection& envelope) {
    rustaxa::TransactionManagerFinalChainAdmissionFact fact;
    const auto sender = requireEnvelopeSender(envelope, "RUST_TX_MANAGER_ADMISSION_ENVELOPE_FAILED");
    const auto account = latestAccountFact(manager, fromBridgeAddress(sender));
    fact.account_found = account.has_value();
    fact.account_nonce = toBridgeU256(account.value_or(state_api::ZeroAccount).nonce);
    fact.account_balance = toBridgeU256(account.value_or(state_api::ZeroAccount).balance);

    const auto location = manager.final_chain_->transactionLocation(fromBridgeHash(envelope.hash));
    fact.finalized_period_known = location.has_value();
    fact.finalized_period = location ? location->period : 0;
    return fact;
  }

  static rustaxa::TransactionManagerAdmissionCommandReport executeValidatedAdmissionReport(
      TransactionManager& manager, const rustaxa::LegacyTransactionInspection& envelope,
      const std::shared_ptr<Transaction>& tx, bool insert_non_proposable) {
    if (envelope.hash != toBridgeHash(tx->getHash())) {
      throw std::runtime_error("RUST_TX_MANAGER_ADMISSION_ENVELOPE_FAILED: Rust transaction envelope hash mismatch");
    }
    if (!manager.final_chain_) {
      throw std::runtime_error(
          "RUST_TX_MANAGER_ADMISSION_EXECUTION_FAILED: FinalChain is required for admission facts");
    }

    std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    const auto fact = buildValidatedInsertRuntimeFact(manager, envelope, insert_non_proposable);
    const auto final_chain_fact = buildFinalChainAdmissionFact(manager, envelope);
    return [&]() {
      try {
        return manager.dag_transaction_service_->service()
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts_command_report(
                fact, final_chain_fact,
                toRuntimeQueueInsertInput(envelope, false, rustFinalChainLastBlockNumber(manager)));
      } catch (const std::exception& e) {
        throw std::runtime_error(std::string("RUST_TX_MANAGER_ADMISSION_EXECUTION_FAILED: ") + e.what());
      }
    }();
  }

  static rustaxa::TransactionManagerAdmissionCommandReport executeValidatedAdmissionReport(
      TransactionManager& manager, const std::shared_ptr<Transaction>& tx, bool insert_non_proposable) {
    const auto envelope = inspectRegularTransaction(tx, "RUST_TX_MANAGER_ADMISSION_ENVELOPE_FAILED");
    return executeValidatedAdmissionReport(manager, envelope, tx, insert_non_proposable);
  }

  static rustaxa::TransactionManagerPublicAdmissionCommandReport executePublicAdmissionReport(
      TransactionManager& manager, const rustaxa::LegacyTransactionInspection& envelope,
      const std::shared_ptr<Transaction>& tx) {
    if (envelope.hash != toBridgeHash(tx->getHash())) {
      throw std::runtime_error("RUST_TX_MANAGER_ADMISSION_ENVELOPE_FAILED: Rust transaction envelope hash mismatch");
    }

    const auto verify_fact = buildVerifyTransactionFact(manager, envelope);
    const auto admission_fact = buildValidatedInsertRuntimeFact(manager, envelope, false);
    const auto final_chain_fact = buildFinalChainAdmissionFact(manager, envelope);
    std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    return [&]() {
      try {
        return manager.dag_transaction_service_->service()
            .transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_facts_command_report(
                verify_fact, admission_fact, final_chain_fact,
                toRuntimeQueueInsertInput(envelope, false, rustFinalChainLastBlockNumber(manager)));
      } catch (const std::exception& e) {
        throw std::runtime_error(std::string("RUST_TX_MANAGER_ADMISSION_EXECUTION_FAILED: ") + e.what());
      }
    }();
  }

  static void applyAdmissionCommandReport(TransactionManager& manager,
                                          const rustaxa::TransactionManagerAdmissionCommandReport& report) {
    if (!report.admission.present) {
      return;
    }
    for (const auto& intent : report.shell_intents) {
      switch (intent.kind) {
        case kTMAdmissionShellIntentLogInserted:
          LOG(manager.log_dg_) << "Transaction " << fromBridgeHash(intent.hash) << " inserted in trx pool";
          break;
        case kTMAdmissionShellIntentEmitTransactionAdded:
          manager.emitTransactionAddedForRust(fromBridgeHash(intent.hash));
          break;
        default:
          throw std::runtime_error("RUST_TX_MANAGER_ADMISSION_EXECUTION_FAILED: unknown admission shell intent");
      }
    }
  }

  static std::pair<bool, std::string> insertTransaction(TransactionManager& manager,
                                                        const std::shared_ptr<Transaction>& trx) {
    const auto envelope = inspectRegularTransaction(trx, "RUST_TX_MANAGER_INSERT_ENVELOPE_FAILED");
    const auto report = executePublicAdmissionReport(manager, envelope, trx);
    applyAdmissionCommandReport(manager, report.admission);
    return {report.public_result.accepted, std::string(report.public_result.message)};
  }

  static TransactionStatus insertValidatedTransaction(TransactionManager& manager, std::shared_ptr<Transaction>&& tx,
                                                      bool insert_non_proposable) {
    const auto report = executeValidatedAdmissionReport(manager, tx, insert_non_proposable);
    applyAdmissionCommandReport(manager, report);
    return transactionStatusFromBridge(report.admission.transaction_status);
  }

  /**
   * Estimates one transaction using Rust-owned cache policy.
   *
   * Rust owns the declared-gas fast path, cache hit/miss decision, and bounded
   * cache insertion. C++ materializes the EVM transaction and calls FinalChain
   * only when Rust reports a miss.
   */
  static state_api::ExecutionResult estimateTransactionGas(TransactionManager& manager,
                                                           std::shared_ptr<Transaction> trx,
                                                           PbftPeriod proposal_period) {
    rustaxa::TransactionManagerGasEstimationFact fact;
    fact.hash = toBridgeHash(trx->getHash());
    fact.declared_gas = trx->getGas();
    fact.proposal_period = proposal_period;
    fact.estimate_gas_limit = manager.kEstimateGasLimit;

    rustaxa::TransactionManagerGasEstimationPlan plan;
    {
      std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
      try {
        plan = manager.dag_transaction_service_->service().transaction_manager_runtime_plan_gas_estimation(fact);
      } catch (const std::exception& e) {
        throw std::runtime_error(std::string("RUST_TX_MANAGER_GAS_ESTIMATION_PLAN_FAILED: ") + e.what());
      }
    }

    if (plan.use_declared_gas) {
      state_api::ExecutionResult result;
      result.gas_used = plan.gas_used;
      return result;
    }
    if (plan.cache_hit) {
      return executionResultFromBridgeBytes(plan.result_rlp);
    }
    if (!plan.requires_evm_call) {
      throw std::runtime_error("Rust transaction manager runtime returned an invalid gas-estimation plan");
    }

    auto evm_trx = state_api::EVMTransaction{
        trx->getSender(), trx->getGasPrice(), trx->getReceiver(), trx->getNonce(),
        trx->getValue(),  trx->getGas(),      trx->getData(),
    };
    auto result = manager.final_chain_->call(evm_trx, proposal_period);

    rustaxa::TransactionManagerGasEstimationResult cache_result;
    cache_result.hash = fact.hash;
    cache_result.proposal_period = proposal_period;
    cache_result.gas_used = result.gas_used;
    cache_result.result_rlp = executionResultToBridgeBytes(result);
    {
      std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
      try {
        manager.dag_transaction_service_->service().transaction_manager_runtime_store_gas_estimation(
            std::move(cache_result));
      } catch (const std::exception& e) {
        throw std::runtime_error(std::string("RUST_TX_MANAGER_GAS_ESTIMATION_STORE_FAILED: ") + e.what());
      }
    }

    return result;
  }

  /**
   * Estimates aggregate transaction weight while Rust owns per-transaction cache
   * decisions and C++ keeps the thread-pool EVM execution shell.
   */
  static uint64_t estimateTransactions(TransactionManager& manager, const SharedTransactions& trxs,
                                       PbftPeriod proposal_period) {
    std::atomic<uint64_t> total_gas = 0;
    std::vector<std::future<void>> futures;
    futures.reserve(trxs.size());
    for (const auto& trx : trxs) {
      futures.emplace_back(manager.estimation_thread_pool_.post([&manager, trx, proposal_period, &total_gas]() {
        total_gas += estimateTransactionGas(manager, trx, proposal_period).gas_used;
      }));
    }
    for (auto& future : futures) {
      future.get();
    }
    return total_gas.load();
  }

  /**
   * Estimates one Rust runtime pack candidate from Rust-inspected envelope facts.
   *
   * This keeps the deterministic candidate envelope in Rust and avoids
   * materializing a legacy `Transaction` object unless the candidate is selected
   * for the public `packTrxs` return value.
   */
  static state_api::ExecutionResult executePackCandidateGasEstimation(
      TransactionManager& manager, const rustaxa::TransactionPackSessionCandidate& candidate,
      PbftPeriod proposal_period) {
    std::optional<addr_t> receiver;
    if (candidate.receiver_found) {
      receiver = fromBridgeAddress(candidate.receiver);
    }
    auto evm_trx = state_api::EVMTransaction{
        fromBridgeAddress(candidate.sender), fromBridgeU256(candidate.gas_price), receiver,
        fromBridgeU256(candidate.nonce),     fromBridgeU256(candidate.value),     candidate.gas,
        toDevBytes(candidate.data),
    };
    return manager.final_chain_->call(evm_trx, proposal_period);
  }

  /**
   * Runs Rust-backed deterministic transaction packing against the standalone facade's executor state.
   *
   * The friend accessor centralizes access to the facade's private Rust runtime,
   * lock, and FinalChain executor. Rust owns candidate scan, accepted ordering,
   * invalid-estimate demotion, and stop decisions; C++ owns transaction
   * materialization, EVM estimation execution, and logging.
   */
  static rustaxa::TransactionPackSessionStep packTransactionPayloads(TransactionManager& manager,
                                                                     PbftPeriod proposal_period, uint64_t weight_limit,
                                                                     uint16_t total_shards = 1,
                                                                     uint16_t node_trx_shard = 0,
                                                                     uint64_t shard_period_interval = 1) {
    std::lock_guard pack_lock(manager.pack_mutex_);
    bool session_active = false;
    try {
      rustaxa::TransactionPackPreparedPlan prepare_plan = [&]() {
        std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
        try {
          return manager.dag_transaction_service_->service().transaction_manager_runtime_pack_prepare_sharded(
              weight_limit, kMinTxGas, proposal_period, manager.kEstimateGasLimit,
              rustFinalChainLastBlockNumber(manager), total_shards, node_trx_shard, shard_period_interval);
        } catch (const std::exception& e) {
          throw std::runtime_error(std::string("RUST_TX_MANAGER_PACK_PREPARE_FAILED: ") + e.what());
        }
      }();
      session_active = true;

      rust::Vec<rustaxa::TransactionPackSessionEstimateInput> estimate_inputs;
      estimate_inputs.reserve(prepare_plan.request_estimates.size());

      for (const auto& candidate : prepare_plan.request_estimates) {
        if (!candidate.found) {
          throw std::runtime_error(
              "Rust transaction manager runtime requested estimation for a missing pack candidate");
        }

        const auto estimate = executePackCandidateGasEstimation(manager, candidate, proposal_period);
        if (estimate.gas_used < kMinTxGas) {
          LOG(manager.log_er_) << "Transaction " << fromBridgeHash(candidate.hash)
                               << " has invalid estimation: " << estimate.gas_used;
        }

        rustaxa::TransactionPackSessionEstimateInput estimate_input;
        estimate_input.hash = candidate.hash;
        estimate_input.gas_used = estimate.gas_used;
        estimate_input.last_block_number = rustFinalChainLastBlockNumber(manager);
        estimate_input.result_rlp = executionResultToBridgeBytes(estimate);
        estimate_inputs.push_back(std::move(estimate_input));
      }

      rustaxa::TransactionPackSessionStep step = [&]() {
        if (prepare_plan.request_estimates.empty()) {
          return rustaxa::TransactionPackSessionStep{
              .request_estimate = false,
              .candidate = rustaxa::TransactionPackSessionCandidate{},
              .selected_transactions = prepare_plan.selected_transactions,
              .demoted_hashes = prepare_plan.demoted_hashes,
              .stopped = prepare_plan.stopped,
          };
        }

        return [&]() {
          std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
          try {
            return manager.dag_transaction_service_->service().transaction_manager_runtime_pack_finalize_with_estimates(
                estimate_inputs);
          } catch (const std::exception& e) {
            throw std::runtime_error(std::string("RUST_TX_MANAGER_PACK_FINALIZE_FAILED: ") + e.what());
          }
        }();
      }();
      session_active = false;

      return step;
    } catch (...) {
      if (session_active) {
        try {
          std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
          manager.dag_transaction_service_->service().transaction_manager_runtime_pack_abort();
        } catch (...) {
        }
      }
      throw;
    }
  }

  static std::pair<SharedTransactions, std::vector<uint64_t>> materializePackedTransactions(
      const rust::Vec<rustaxa::TransactionPackSelectedTransaction>& payloads) {
    SharedTransactions selected_transactions;
    std::vector<uint64_t> gas_estimations;
    selected_transactions.reserve(payloads.size());
    gas_estimations.reserve(payloads.size());
    for (const auto& selected : payloads) {
      auto transaction = std::make_shared<Transaction>(fromBridgeBytes(selected.tx_rlp));
      if (transaction->getHash() != fromBridgeHash(selected.hash)) {
        throw std::runtime_error("Rust transaction manager runtime returned selected pack RLP with mismatched hash");
      }
      selected_transactions.push_back(std::move(transaction));
      gas_estimations.push_back(selected.gas_used);
    }
    return {std::move(selected_transactions), std::move(gas_estimations)};
  }

  static std::pair<SharedTransactions, std::vector<uint64_t>> packTrxs(TransactionManager& manager,
                                                                       PbftPeriod proposal_period,
                                                                       uint64_t weight_limit, uint16_t total_shards = 1,
                                                                       uint16_t node_trx_shard = 0,
                                                                       uint64_t shard_period_interval = 1) {
    auto pack_step = packTransactionPayloads(manager, proposal_period, weight_limit, total_shards, node_trx_shard,
                                             shard_period_interval);
    return materializePackedTransactions(pack_step.selected_transactions);
  }

  /**
   * Runs the external EVM portion of one composed DAG proposer pack session.
   *
   * The C++ lock prevents public compatibility packing from replacing the
   * Rust transaction pack while EVM execution occurs outside Rust locks.
   */
  static rustaxa::DagProposerSessionStep executeDagProposerTransactionPack(TransactionManager& manager,
                                                                           uint64_t session_id,
                                                                           bool network_throttled) {
    std::lock_guard pack_lock(manager.pack_mutex_);
    try {
      const auto last_block_number = network_throttled ? 0 : rustFinalChainLastBlockNumber(manager);
      auto step = manager.dag_transaction_service_->service().dag_transaction_service_proposer_pack_prepare(
          session_id, network_throttled, kMinTxGas, manager.kEstimateGasLimit, last_block_number);
      if (step.transaction_estimate_requests.empty()) {
        return step;
      }

      rust::Vec<rustaxa::TransactionPackSessionEstimateInput> estimate_inputs;
      estimate_inputs.reserve(step.transaction_estimate_requests.size());
      for (const auto& candidate : step.transaction_estimate_requests) {
        if (!candidate.found) {
          throw std::runtime_error("Rust DAG proposer pack requested estimation for a missing transaction candidate");
        }
        const auto estimate = executePackCandidateGasEstimation(manager, candidate, step.proposal_period);
        if (estimate.gas_used < kMinTxGas) {
          LOG(manager.log_er_) << "Transaction " << fromBridgeHash(candidate.hash)
                               << " has invalid estimation: " << estimate.gas_used;
        }

        rustaxa::TransactionPackSessionEstimateInput estimate_input;
        estimate_input.hash = candidate.hash;
        estimate_input.gas_used = estimate.gas_used;
        estimate_input.last_block_number = rustFinalChainLastBlockNumber(manager);
        estimate_input.result_rlp = executionResultToBridgeBytes(estimate);
        estimate_inputs.push_back(std::move(estimate_input));
      }
      return manager.dag_transaction_service_->service().dag_transaction_service_proposer_pack_finalize(
          session_id, std::move(estimate_inputs));
    } catch (...) {
      try {
        manager.dag_transaction_service_->service().dag_transaction_service_proposer_pack_abort(session_id);
      } catch (...) {
      }
      throw;
    }
  }

  /**
   * Persists transactions accepted by a DAG block.
   *
   * This compatibility overload is an edge adapter: it extracts only hashes and
   * canonical RLP payload bytes from public `Transaction` objects, then delegates
   * to the payload-oriented Rust command path. Rust re-inspects those bytes and
   * remains the authority for duplicate filtering, nonce-gated finalized storage
   * checks, accepted ordering, count planning, and queue/sidecar mutation.
   */
  static void saveTransactionsFromDagBlock(TransactionManager& manager, SharedTransactions const& trxs) {
    vec_trx_t transaction_hashes;
    std::vector<dev::bytes> transaction_rlps;
    transaction_hashes.reserve(trxs.size());
    transaction_rlps.reserve(trxs.size());
    for (const auto& transaction : trxs) {
      transaction_hashes.emplace_back(transaction->getHash());
      transaction_rlps.emplace_back(transaction->rlp());
    }
    persistDagBlockTransactionPayloadsForCompatibility(manager, transaction_hashes, transaction_rlps);
  }

  /**
   * Persists DAG-accepted transaction payloads without first materializing
   * compatibility `Transaction` objects.
   *
   * The proposed-DAG path already carries canonical RLP bytes from the Rust
   * proposer packing session. Rust re-inspects those bytes, verifies the
   * supplied hashes, sources FinalChain nonce facts, and owns the accepted
   * storage/sidecar mutation. C++ keeps only the transaction mutex and logging
   * mechanics around the Rust-owned runtime/storage operation.
   */
  static void persistDagBlockTransactionPayloadsForCompatibility(TransactionManager& manager,
                                                                 const vec_trx_t& transaction_hashes,
                                                                 const std::vector<dev::bytes>& transaction_rlps) {
    if (transaction_hashes.size() != transaction_rlps.size()) {
      throw DbException("RUST_STORAGE_DAG_TX_PERSIST_FAILED: DAG transaction payload lengths do not match");
    }
    if (transaction_hashes.empty()) {
      return;
    }
    if (!manager.final_chain_) {
      throw DbException(
          "RUST_STORAGE_DAG_TX_PERSIST_FAILED: FinalChain is required for non-empty DAG transaction save");
    }

    std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));

    rust::Vec<rustaxa::DagTransactionSaveSidecarFact> facts;
    facts.reserve(transaction_hashes.size());

    for (size_t idx = 0; idx < transaction_hashes.size(); ++idx) {
      const auto envelope = inspectRegularTransactionPayload(transaction_hashes[idx], transaction_rlps[idx],
                                                             "RUST_STORAGE_DAG_TX_ENVELOPE_FAILED");
      const auto sender = requireEnvelopeSender(envelope, "RUST_STORAGE_DAG_TX_ENVELOPE_FAILED");
      const auto account = latestAccountFact(manager, fromBridgeAddress(sender)).value_or(state_api::ZeroAccount);

      rustaxa::DagTransactionSaveSidecarFact fact;
      fact.input_index = static_cast<uint64_t>(idx);
      fact.hash = envelope.hash;
      fact.trx_rlp = cloneBridgeBytes(envelope.tx_rlp);
      fact.transaction_nonce = envelope.nonce;
      fact.sender_account_nonce = toBridgeU256(account.nonce);
      facts.push_back(std::move(fact));
    }

    const auto report = [&]() {
      try {
        return rustaxa::save_transactions_from_dag_block_command_report_with_runtime(
            manager.dag_transaction_service_->service(), std::move(facts));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_DAG_TX_PERSIST_FAILED: ") + e.what());
      }
    }();

    for (const auto& erased : report.queue_erased) {
      LOG(manager.log_dg_) << "Transaction " << fromBridgeHash(erased.hash) << " removed from trx pool ";
    }
  }

  /**
   * Clears live non-finalized transaction sidecars and their pending storage rows.
   *
   * Rust owns the storage delete batch and commits it before mutating the live
   * sidecar. Transaction counters are unchanged, matching the legacy
   * expired-DAG cleanup semantics.
   */
  static void removeNonFinalizedTransactions(TransactionManager& manager,
                                             std::unordered_set<trx_hash_t>&& transactions) {
    std::vector<trx_hash_t> hashes;
    hashes.reserve(transactions.size());
    for (const auto& hash : transactions) {
      hashes.push_back(hash);
    }
    try {
      manager.dag_transaction_service_->service().transaction_manager_runtime_remove_non_finalized(
          buildSidecarLookupRequests(hashes));
    } catch (const std::exception& e) {
      throw DbException(std::string("RUST_TX_MANAGER_SIDECAR_EXPIRED_REMOVE_FAILED: ") + e.what());
    }
  }

  /**
   * Retrieves a transaction through the Rust-owned transaction view.
   *
   * Rust owns queue, sidecar, and storage source precedence. C++ only
   * materializes the retained RLP bytes into the public transaction object.
   */
  static std::shared_ptr<Transaction> getTransaction(const TransactionManager& manager, const trx_hash_t& hash) {
    const std::vector<trx_hash_t> hashes{hash};
    const auto ordered = getTransactionsWithBoundedView(manager, hashes);
    return ordered.size() == 1 ? ordered.front() : nullptr;
  }

  static std::vector<std::shared_ptr<Transaction>> getNonfinalizedTrx(const TransactionManager& manager,
                                                                      const std::vector<trx_hash_t>& hashes) {
    std::vector<std::shared_ptr<Transaction>> ret;
    if (hashes.empty()) {
      return ret;
    }

    auto requests = buildTransactionViewRequests(hashes);
    const auto views = [&]() {
      std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
      try {
        return manager.dag_transaction_service_->service()
            .transaction_manager_runtime_lookup_non_finalized_transaction_views(std::move(requests));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_TX_MANAGER_NONFINALIZED_SIDECAR_LOOKUP_FAILED: ") + e.what());
      }
    }();

    if (views.size() != hashes.size()) {
      throw DbException(
          "RUST_TX_MANAGER_NONFINALIZED_SIDECAR_LOOKUP_FAILED: Rust returned a malformed transaction view");
    }
    ret.reserve(views.size());
    for (const auto& view : views) {
      if (view.input_index >= hashes.size()) {
        throw DbException(
            "RUST_TX_MANAGER_NONFINALIZED_SIDECAR_LOOKUP_FAILED: Rust returned an out-of-range transaction index");
      }
      const auto& expected_hash = hashes[static_cast<size_t>(view.input_index)];
      if (fromBridgeHash(view.hash) != expected_hash) {
        throw DbException(
            "RUST_TX_MANAGER_NONFINALIZED_SIDECAR_LOOKUP_FAILED: Rust returned a transaction hash/index mismatch");
      }

      if (!view.found) {
        continue;
      }
      auto transaction = materializeTransactionView(view, "RUST_TX_MANAGER_NONFINALIZED_SIDECAR_LOOKUP_FAILED");
      if (transaction) {
        ret.push_back(std::move(transaction));
      }
    }
    return ret;
  }

  static std::shared_ptr<Transaction> getNonFinalizedTransaction(const TransactionManager& manager,
                                                                 const trx_hash_t& hash) {
    std::vector<trx_hash_t> hashes{hash};
    const auto transactions = getNonfinalizedTrx(manager, hashes);
    if (!transactions.empty()) {
      return transactions.front();
    }
    return {};
  }

  static std::vector<std::shared_ptr<Transaction>> getTransactionsWithBoundedView(
      const TransactionManager& manager, const std::vector<trx_hash_t>& hashes,
      const std::optional<PbftPeriod>& proposal_period = std::nullopt) {
    std::vector<std::shared_ptr<Transaction>> ordered_transactions(hashes.size());
    if (hashes.empty()) {
      return ordered_transactions;
    }

    auto requests = buildTransactionViewRequests(hashes);
    rustaxa::TransactionManagerTransactionViewPlan view_plan;
    {
      std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
      view_plan = [&]() {
        try {
          if (proposal_period.has_value() && manager.final_chain_) {
            return manager.dag_transaction_service_->service()
                .transaction_manager_runtime_lookup_proposal_transaction_views_with_account_nonce_facts(
                    proposal_period.value(), std::move(requests), buildAccountNonceFacts(manager), 0);
          }
          return manager.dag_transaction_service_->service().transaction_manager_runtime_lookup_transaction_views(
              std::move(requests), 0);
        } catch (const std::exception& e) {
          if (proposal_period.has_value()) {
            throw DbException(std::string("RUST_TX_MANAGER_PROPOSAL_VIEW_LOOKUP_FAILED: ") + e.what());
          }
          throw DbException(std::string("RUST_TX_MANAGER_VIEW_LOOKUP_FAILED: ") + e.what());
        }
      }();
    }

    if (view_plan.requested_count != hashes.size()) {
      throw DbException(
          "RUST_TX_MANAGER_VIEW_LOOKUP_FAILED: Rust returned an unexpected transaction view request count");
    }
    if (!view_plan.complete) {
      throw DbException("RUST_TX_MANAGER_VIEW_LOOKUP_FAILED: Rust returned a truncated transaction view");
    }
    if (view_plan.views.size() != view_plan.requested_count) {
      throw DbException("RUST_TX_MANAGER_VIEW_LOOKUP_FAILED: Rust returned a malformed transaction view plan");
    }

    for (const auto& view : view_plan.views) {
      if (view.input_index >= view_plan.requested_count) {
        throw DbException("RUST_TX_MANAGER_VIEW_LOOKUP_FAILED: Rust returned an out-of-range transaction index");
      }
      const auto view_index = static_cast<size_t>(view.input_index);
      if (fromBridgeHash(view.hash) != hashes[view_index]) {
        throw DbException("RUST_TX_MANAGER_VIEW_LOOKUP_FAILED: Rust returned a transaction hash/index mismatch");
      }
      if (view.old_finalized) {
        LOG(manager.log_er_) << "Old transaction: " << hashes[view_index];
        continue;
      }
      auto transaction = materializeTransactionView(view, "RUST_TX_MANAGER_VIEW_LOOKUP_FAILED");
      if (transaction) {
        ordered_transactions[view_index] = std::move(transaction);
      }
    }
    return ordered_transactions;
  }

  static rust::Vec<rustaxa::FinalizedTransactionStatusSidecarFact> buildFinalizedStatusPayloadFacts(
      const vec_trx_t& transaction_hashes, const std::vector<dev::bytes>& transaction_rlps) {
    if (transaction_hashes.size() != transaction_rlps.size()) {
      throw DbException("RUST_STORAGE_FINALIZED_TX_STATUS_FAILED: finalized transaction payload lengths do not match");
    }

    rust::Vec<rustaxa::FinalizedTransactionStatusSidecarFact> facts;
    facts.reserve(transaction_hashes.size());
    for (size_t idx = 0; idx < transaction_hashes.size(); ++idx) {
      const auto envelope = inspectRegularTransactionPayload(transaction_hashes[idx], transaction_rlps[idx],
                                                             "RUST_STORAGE_FINALIZED_TX_STATUS_ENVELOPE_FAILED");
      rustaxa::FinalizedTransactionStatusSidecarFact fact;
      fact.input_index = static_cast<uint64_t>(idx);
      fact.hash = envelope.hash;
      fact.trx_rlp = cloneBridgeBytes(envelope.tx_rlp);
      facts.push_back(std::move(fact));
    }
    return facts;
  }

  static std::pair<vec_trx_t, std::vector<dev::bytes>> periodTransactionPayloads(const PeriodData& period_data) {
    vec_trx_t transaction_hashes;
    std::vector<dev::bytes> transaction_rlps;
    transaction_hashes.reserve(period_data.transactions.size());
    transaction_rlps.reserve(period_data.transactions.size());
    for (const auto& transaction : period_data.transactions) {
      transaction_hashes.emplace_back(transaction->getHash());
      transaction_rlps.emplace_back(transaction->rlp());
    }
    return {std::move(transaction_hashes), std::move(transaction_rlps)};
  }

  static std::unordered_set<trx_hash_t> excludeFinalizedTransactions(const TransactionManager& manager,
                                                                     const std::vector<trx_hash_t>& hashes) {
    const auto plan = [&]() {
      try {
        return rustaxa::transaction_manager_filter_non_finalized_with_runtime(
            manager.dag_transaction_service_->service(), buildSidecarLookupRequests(hashes));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_TX_FILTER_FAILED: ") + e.what());
      }
    }();

    std::unordered_set<trx_hash_t> ret;
    ret.reserve(plan.not_finalized.size());
    for (const auto& action : plan.not_finalized) {
      if (action.input_index >= hashes.size()) {
        throw DbException("RUST_STORAGE_TX_FILTER_FAILED: Rust returned an out-of-range transaction index");
      }
      const auto& expected_hash = hashes[static_cast<size_t>(action.input_index)];
      if (fromBridgeHash(action.hash) != expected_hash) {
        throw DbException("RUST_STORAGE_TX_FILTER_FAILED: Rust returned a transaction hash/index mismatch");
      }
      ret.insert(expected_hash);
    }
    return ret;
  }

  static bool verifyTransactionsNotFinalized(const TransactionManager& manager, const SharedTransactions& trxs) {
    if (!manager.final_chain_) {
      throw DbException("RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_FAILED: FinalChain is required for transaction facts");
    }

    rust::Vec<rustaxa::TransactionManagerVerifyNotFinalizedSidecarFact> facts;
    facts.reserve(trxs.size());
    uint64_t input_index = 0;
    for (const auto& transaction : trxs) {
      const auto envelope =
          inspectRegularTransaction(transaction, "RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_ENVELOPE_FAILED");
      const auto sender = requireEnvelopeSender(envelope, "RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_ENVELOPE_FAILED");
      const auto account = latestAccountFact(manager, fromBridgeAddress(sender)).value_or(state_api::ZeroAccount);

      rustaxa::TransactionManagerVerifyNotFinalizedSidecarFact fact;
      fact.input_index = input_index++;
      fact.hash = envelope.hash;
      fact.transaction_nonce = envelope.nonce;
      fact.sender_account_nonce = toBridgeU256(account.nonce);
      facts.push_back(fact);
    }

    const auto outcome = [&]() {
      try {
        return rustaxa::transaction_manager_verify_not_finalized_with_runtime(
            manager.dag_transaction_service_->service(), std::move(facts));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_FAILED: ") + e.what());
      }
    }();

    if (!outcome.is_finalized) {
      return true;
    }

    if (outcome.input_index >= trxs.size()) {
      throw DbException("RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_FAILED: Rust returned an out-of-range transaction index");
    }
    const auto& transaction = trxs[static_cast<size_t>(outcome.input_index)];
    const auto trx_hash = transaction->getHash();
    if (fromBridgeHash(outcome.hash) != trx_hash) {
      throw DbException("RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_FAILED: Rust returned a transaction hash/index mismatch");
    }

    if (outcome.source == kVerifyNotFinalizedRecentSidecar) {
      LOG(manager.log_er_) << "Transaction " << trx_hash << " already finalized";
    } else {
      LOG(manager.log_er_) << "Transaction " << trx_hash << " already finalized in db";
    }
    return false;
  }

  static rustaxa::TransactionManagerVerifyNotFinalizedOutcome verifyTransactionsNotFinalizedDetailed(
      const TransactionManager& manager, std::vector<TransactionManagerVerifyNotFinalizedInput>&& facts) {
    if (!manager.final_chain_) {
      throw DbException("RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_FAILED: FinalChain is required for transaction facts");
    }

    std::vector<trx_hash_t> expected_hashes;
    expected_hashes.reserve(facts.size());
    for (const auto& fact : facts) {
      expected_hashes.emplace_back(fact.hash.data(), trx_hash_t::ConstructFromPointer);
    }

    rust::Vec<rustaxa::TransactionManagerVerifyNotFinalizedSidecarFact> sidecar_facts;
    sidecar_facts.reserve(facts.size());
    for (const auto& fact : facts) {
      const auto account = latestAccountFact(manager, addr_t(fact.sender.data(), addr_t::ConstructFromPointer))
                               .value_or(state_api::ZeroAccount);

      rustaxa::TransactionManagerVerifyNotFinalizedSidecarFact sidecar_fact;
      sidecar_fact.input_index = fact.input_index;
      sidecar_fact.hash = fact.hash;
      sidecar_fact.transaction_nonce = fact.transaction_nonce;
      sidecar_fact.sender_account_nonce = toBridgeU256(account.nonce);
      sidecar_facts.push_back(sidecar_fact);
    }

    const auto outcome = [&]() {
      try {
        return rustaxa::transaction_manager_verify_not_finalized_with_runtime(
            manager.dag_transaction_service_->service(), std::move(sidecar_facts));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_FAILED: ") + e.what());
      }
    }();

    if (outcome.is_finalized) {
      if (outcome.input_index >= expected_hashes.size()) {
        throw DbException(
            "RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_FAILED: Rust returned an out-of-range transaction index");
      }

      const auto trx_hash = fromBridgeHash(outcome.hash);
      if (trx_hash != expected_hashes[static_cast<size_t>(outcome.input_index)]) {
        throw DbException(
            "RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_FAILED: Rust returned a transaction hash/index mismatch");
      }

      if (outcome.source == kVerifyNotFinalizedRecentSidecar) {
        LOG(manager.log_er_) << "Transaction " << trx_hash << " already finalized";
      } else {
        LOG(manager.log_er_) << "Transaction " << trx_hash << " already finalized in db";
      }
    }
    return outcome;
  }

  static std::vector<SharedTransactions> getAllPoolTrxs(const TransactionManager& manager) {
    std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    const auto groups =
        manager.dag_transaction_service_->service().transaction_manager_runtime_queue_all_transaction_groups();
    std::vector<SharedTransactions> transactions;
    transactions.reserve(groups.size());
    for (const auto& group : groups) {
      SharedTransactions group_transactions;
      group_transactions.reserve(group.transactions.size());
      for (const auto& queued_transaction : group.transactions) {
        if (auto transaction =
                materializeQueuedTransaction(queued_transaction, "RUST_TX_MANAGER_QUEUE_GROUP_LOOKUP_FAILED")) {
          group_transactions.emplace_back(std::move(transaction));
        } else {
          throw std::runtime_error("Rust transaction manager runtime returned a missing grouped queue payload");
        }
      }
      transactions.emplace_back(std::move(group_transactions));
    }
    return transactions;
  }

  static std::pair<std::vector<std::shared_ptr<Transaction>>, std::vector<trx_hash_t>> getPoolTransactions(
      const TransactionManager& manager, const std::vector<trx_hash_t>& trx_to_query) {
    std::pair<std::vector<std::shared_ptr<Transaction>>, std::vector<trx_hash_t>> result;
    if (trx_to_query.empty()) {
      return result;
    }

    auto requests = buildTransactionViewRequests(trx_to_query);
    const auto views = [&]() {
      std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
      try {
        return manager.dag_transaction_service_->service().transaction_manager_runtime_queue_lookup_transaction_views(
            std::move(requests));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_TX_MANAGER_QUEUE_POOL_LOOKUP_FAILED: ") + e.what());
      }
    }();

    if (views.size() != trx_to_query.size()) {
      throw DbException("RUST_TX_MANAGER_QUEUE_POOL_LOOKUP_FAILED: Rust returned a malformed transaction view");
    }
    result.first.reserve(views.size());
    for (const auto& view : views) {
      if (view.input_index >= trx_to_query.size()) {
        throw DbException("RUST_TX_MANAGER_QUEUE_POOL_LOOKUP_FAILED: Rust returned an out-of-range transaction index");
      }
      const auto& expected_hash = trx_to_query[static_cast<size_t>(view.input_index)];
      if (fromBridgeHash(view.hash) != expected_hash) {
        throw DbException("RUST_TX_MANAGER_QUEUE_POOL_LOOKUP_FAILED: Rust returned a transaction hash/index mismatch");
      }

      auto trx = materializeTransactionView(view, "RUST_TX_MANAGER_QUEUE_POOL_LOOKUP_FAILED");
      if (trx) {
        result.first.emplace_back(std::move(trx));
      } else {
        result.second.emplace_back(expected_hash);
      }
    }
    return result;
  }

  static unsigned long getTransactionCount(const TransactionManager& manager) {
    std::shared_lock shared_transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    return manager.dag_transaction_service_->service().transaction_manager_runtime_transaction_count();
  }

  static void blockFinalized(TransactionManager& manager, EthBlockNumber block_number) {
    std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    [&]() {
      try {
        manager.dag_transaction_service_->service().transaction_manager_runtime_queue_block_finalized(block_number);
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_TX_MANAGER_BLOCK_FINALIZED_FAILED: ") + e.what());
      }
    }();
  }

  static bool isTransactionKnown(TransactionManager& manager, const trx_hash_t& trx_hash) {
    std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    try {
      return manager.dag_transaction_service_->service().transaction_manager_runtime_is_transaction_known_hash(
          toBridgeHash(trx_hash));
    } catch (const std::exception& e) {
      throw DbException(std::string("RUST_TX_MANAGER_KNOWN_LOOKUP_FAILED: ") + e.what());
    }
  }

  static size_t getTransactionPoolSize(const TransactionManager& manager) {
    std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    return manager.dag_transaction_service_->service().transaction_manager_runtime_queue_size();
  }

  static bool isTransactionPoolFull(const TransactionManager& manager, size_t percentage) {
    std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    return manager.dag_transaction_service_->service().transaction_manager_runtime_queue_size() >=
           (manager.kConf.transactions_pool_size * percentage / 100);
  }

  static bool nonProposableTransactionsOverTheLimit(const TransactionManager& manager) {
    std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    return manager.dag_transaction_service_->service().transaction_manager_runtime_queue_non_proposable_over_limit();
  }

  static size_t getNonfinalizedTrxSize(const TransactionManager& manager) {
    return manager.dag_transaction_service_->service().transaction_manager_runtime_non_finalized_size();
  }

  static bool transactionsDropped(const TransactionManager& manager) {
    std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    return manager.dag_transaction_service_->service().transaction_manager_runtime_queue_transactions_dropped();
  }

  static val_t getMinGasPriceForBlockInclusion(const TransactionManager& manager) {
    std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    return fromBridgeU256(
        manager.dag_transaction_service_->service().transaction_manager_runtime_queue_min_gas_price_for_block_inclusion(
            manager.kConf.propose_dag_gas_limit));
  }

  static val_t gasPriceBid(const TransactionManager& manager) {
    std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    return fromBridgeU256(manager.dag_transaction_service_->service().transaction_manager_runtime_gas_price_bid());
  }

  static void updateGasPrice(TransactionManager& manager, const SharedTransactions& trxs) {
    std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    manager.dag_transaction_service_->service().transaction_manager_runtime_gas_price_update(
        gasPricesFromTransactions(trxs));
  }

  /**
   * Materializes ordered transaction hashes from the Rust-owned transaction view.
   *
   * Rust owns queue/sidecar/storage source precedence, transaction-location
   * decoding, period data extraction, source classification, and
   * proposal-period nonce filtering. C++ keeps transaction object
   * materialization and public API ordering.
   */
  static SharedTransactions getTransactions(const TransactionManager& manager, const vec_trx_t& trxs_hashes,
                                            PbftPeriod proposal_period) {
    auto ordered_transactions =
        getTransactionsWithBoundedView(manager, trxs_hashes, std::make_optional(proposal_period));
    SharedTransactions transactions;
    transactions.reserve(trxs_hashes.size());
    for (auto& transaction : ordered_transactions) {
      if (transaction) {
        transactions.emplace_back(std::move(transaction));
      }
    }
    return transactions;
  }

  /**
   * Resolves the active DAG verification transaction query through the composed service.
   *
   * Rust prepares ordered payload views without advancing the verification cursor. C++ validates every materialized
   * payload, then reads each resolved sender's account nonce at the exact proposal period. Only after those operations
   * succeed does Rust revalidate the cursor and lookup, apply nonce filtering, and advance. The retained C++ EVM
   * boundary receives the materialized transactions only after completion succeeds.
   */
  static std::pair<rustaxa::DagVerifyBlockSessionStep, SharedTransactions> executeDagVerifyTransactionAvailability(
      const TransactionManager& manager) {
    std::shared_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    auto preparation = rustaxa::dag_manager_runtime_verify_block_session_prepare_transactions(
        manager.dag_transaction_service_->service());

    SharedTransactions transactions;
    transactions.reserve(preparation.transactions.size());
    std::vector<addr_t> senders;
    senders.reserve(preparation.transactions.size());
    std::unordered_set<addr_t> seen_senders;
    seen_senders.reserve(preparation.transactions.size());
    for (const auto& view : preparation.transactions) {
      auto transaction = materializeTransactionView(view, "RUST_DAG_VERIFY_TRANSACTION_RESOLUTION_FAILED");
      if (transaction) {
        const auto sender = transaction->getSender();
        if (seen_senders.emplace(sender).second) {
          senders.emplace_back(sender);
        }
        transactions.emplace_back(std::move(transaction));
      }
    }

    if (!senders.empty() && !manager.final_chain_) {
      throw std::runtime_error("RUST_DAG_VERIFY_TRANSACTION_ACCOUNT_LOOKUP_FAILED: FinalChain is unavailable");
    }

    rust::Vec<rustaxa::TransactionQueueAccountNonceFact> account_nonce_facts;
    account_nonce_facts.reserve(senders.size());
    for (const auto& sender : senders) {
      const auto account = manager.final_chain_->getAccount(sender, preparation.proposal_period);
      rustaxa::TransactionQueueAccountNonceFact fact;
      fact.sender = toBridgeAddress(sender);
      fact.account_found = account.has_value();
      fact.account_nonce = account.has_value() ? toBridgeU256(account->nonce) : std::array<uint8_t, 32>{};
      account_nonce_facts.push_back(std::move(fact));
    }

    rustaxa::DagVerifyBlockTransactionCompletionReport report;
    report.cursor_id = preparation.cursor_id;
    report.proposal_period = preparation.proposal_period;
    report.account_nonce_facts = std::move(account_nonce_facts);
    auto step = rustaxa::dag_manager_runtime_verify_block_session_complete_transactions(
        manager.dag_transaction_service_->service(), std::move(report));
    return {std::move(step), std::move(transactions)};
  }

  /**
   * Rebuilds in-memory non-finalized transaction sidecars from Rust-backed storage.
   *
   * Rust loads the recovery payloads and removes stale finalized rows from
   * non-finalized storage before C++ materializes survivor transactions into
   * the live sidecar map. Each survivor has its sender cached before insertion.
   */
  static void recoverNonfinalizedTransactions(TransactionManager& manager) {
    std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    [&]() {
      try {
        rustaxa::transaction_manager_recover_nonfinalized_with_runtime(manager.dag_transaction_service_->service());
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_TX_RECOVERY_FAILED: ") + e.what());
      }
    }();
  }

  static void initializeRecentlyFinalizedTransactionPayloads(TransactionManager& manager, PbftPeriod period,
                                                             const vec_trx_t& transaction_hashes,
                                                             const std::vector<dev::bytes>& transaction_rlps) {
    if (transaction_hashes.size() != transaction_rlps.size()) {
      throw DbException(
          "RUST_TX_MANAGER_RECENT_SIDECAR_INIT_FAILED: finalized transaction payload lengths do not match");
    }
    std::unique_lock transactions_lock(TransactionManagerRustShimAccess::transactionsMutex(manager));
    rust::Vec<rustaxa::TransactionManagerSidecarInsertInput> payloads;
    payloads.reserve(transaction_hashes.size());
    for (size_t idx = 0; idx < transaction_hashes.size(); ++idx) {
      const auto envelope = inspectRegularTransactionPayload(transaction_hashes[idx], transaction_rlps[idx],
                                                             "RUST_TX_MANAGER_RECENT_SIDECAR_INIT_ENVELOPE_FAILED");
      payloads.push_back(toSidecarPayload(envelope));
    }
    try {
      manager.dag_transaction_service_->service().transaction_manager_runtime_initialize_recently_finalized_payloads(
          period, std::move(payloads));
    } catch (const std::exception& e) {
      throw DbException(std::string("RUST_TX_MANAGER_RECENT_SIDECAR_INIT_FAILED: ") + e.what());
    }
  }

  static void initializeRecentlyFinalizedTransactions(TransactionManager& manager, const PeriodData& period_data) {
    const auto payloads = periodTransactionPayloads(period_data);
    initializeRecentlyFinalizedTransactionPayloads(manager, period_data.pbft_blk->getPeriod(), payloads.first,
                                                   payloads.second);
  }

  static rustaxa::TransactionManagerFinalizedStatusCommandReport updateFinalizedTransactionsStatusReport(
      TransactionManager& manager, PbftPeriod period,
      rust::Vec<rustaxa::FinalizedTransactionStatusSidecarFact>&& facts) {
    const auto recently_finalized_transactions_periods =
        kRecentlyFinalizedTransactionsFactor * manager.final_chain_->delegationDelay();

    const auto report = [&]() {
      try {
        return rustaxa::update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts(
            manager.dag_transaction_service_->service(), period, recently_finalized_transactions_periods,
            buildAccountNonceFacts(manager), std::move(facts));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_FINALIZED_TX_STATUS_FAILED: ") + e.what());
      }
    }();

    for (const auto& removed : report.removed_non_finalized) {
      LOG(manager.log_dg_) << "Transaction " << fromBridgeHash(removed.hash)
                           << " removed from nonfinalized transactions";
    }
    for (const auto& erased : report.queue_erased) {
      LOG(manager.log_dg_) << "Transaction " << fromBridgeHash(erased.hash) << " removed from transactions_pool_";
    }
    for (const auto& purged : report.finalized_account_purged) {
      LOG(manager.log_dg_) << "Transaction " << fromBridgeHash(purged.hash)
                           << " removed from transactions_pool_ by finalized account nonce";
    }
    return report;
  }

  static rustaxa::TransactionManagerFinalizedStatusCommandReport updateFinalizedTransactionsStatusReport(
      TransactionManager& manager, const PeriodData& period_data) {
    const auto payloads = periodTransactionPayloads(period_data);
    return updateFinalizedTransactionsStatusReport(manager, period_data.pbft_blk->getPeriod(),
                                                   buildFinalizedStatusPayloadFacts(payloads.first, payloads.second));
  }

  static void updateFinalizedTransactionsStatus(TransactionManager& manager, const PeriodData& period_data) {
    updateFinalizedTransactionsStatusReport(manager, period_data);
  }
};

std::pair<SharedTransactions, std::vector<uint64_t>> TransactionManager::packTrxs(PbftPeriod proposal_period,
                                                                                  uint64_t weight_limit) {
  return TransactionManagerRustShimAccess::packTrxs(*this, proposal_period, weight_limit);
}

std::pair<SharedTransactions, std::vector<uint64_t>> TransactionManager::packShardedTrxs(
    PbftPeriod proposal_period, uint64_t weight_limit, uint16_t total_shards, uint16_t node_trx_shard,
    uint64_t shard_period_interval) {
  return TransactionManagerRustShimAccess::packTrxs(*this, proposal_period, weight_limit, total_shards, node_trx_shard,
                                                    shard_period_interval);
}

rustaxa::DagProposerSessionStep TransactionManager::executeDagProposerTransactionPack(uint64_t session_id,
                                                                                      bool network_throttled) {
  return TransactionManagerRustShimAccess::executeDagProposerTransactionPack(*this, session_id, network_throttled);
}

uint64_t TransactionManager::estimateTransactions(const SharedTransactions& trxs, PbftPeriod proposal_period) {
  return TransactionManagerRustShimAccess::estimateTransactions(*this, trxs, proposal_period);
}

state_api::ExecutionResult TransactionManager::estimateTransactionGas(std::shared_ptr<Transaction> trx,
                                                                      PbftPeriod proposal_period) {
  return TransactionManagerRustShimAccess::estimateTransactionGas(*this, std::move(trx), proposal_period);
}

void TransactionManager::blockFinalized(EthBlockNumber block_number) {
  TransactionManagerRustShimAccess::blockFinalized(*this, block_number);
}

bool TransactionManager::isTransactionKnown(const trx_hash_t& trx_hash) {
  return TransactionManagerRustShimAccess::isTransactionKnown(*this, trx_hash);
}

size_t TransactionManager::getTransactionPoolSize() const {
  return TransactionManagerRustShimAccess::getTransactionPoolSize(*this);
}

bool TransactionManager::isTransactionPoolFull(size_t percentage) const {
  return TransactionManagerRustShimAccess::isTransactionPoolFull(*this, percentage);
}

bool TransactionManager::nonProposableTransactionsOverTheLimit() const {
  return TransactionManagerRustShimAccess::nonProposableTransactionsOverTheLimit(*this);
}

size_t TransactionManager::getNonfinalizedTrxSize() const {
  return TransactionManagerRustShimAccess::getNonfinalizedTrxSize(*this);
}

std::vector<std::shared_ptr<Transaction>> TransactionManager::getNonfinalizedTrx(
    const std::vector<trx_hash_t>& hashes) {
  return TransactionManagerRustShimAccess::getNonfinalizedTrx(*this, hashes);
}

std::unordered_set<trx_hash_t> TransactionManager::excludeFinalizedTransactions(const std::vector<trx_hash_t>& hashes) {
  return TransactionManagerRustShimAccess::excludeFinalizedTransactions(*this, hashes);
}

bool TransactionManager::verifyTransactionsNotFinalized(const SharedTransactions& trxs) {
  return TransactionManagerRustShimAccess::verifyTransactionsNotFinalized(*this, trxs);
}

rustaxa::TransactionManagerVerifyNotFinalizedOutcome TransactionManager::verifyTransactionsNotFinalizedDetailed(
    std::vector<TransactionManagerVerifyNotFinalizedInput>&& facts) {
  return TransactionManagerRustShimAccess::verifyTransactionsNotFinalizedDetailed(*this, std::move(facts));
}

std::vector<SharedTransactions> TransactionManager::getAllPoolTrxs() {
  return TransactionManagerRustShimAccess::getAllPoolTrxs(*this);
}

std::pair<std::vector<std::shared_ptr<Transaction>>, std::vector<trx_hash_t>> TransactionManager::getPoolTransactions(
    const std::vector<trx_hash_t>& trx_to_query) const {
  return TransactionManagerRustShimAccess::getPoolTransactions(*this, trx_to_query);
}

bool TransactionManager::transactionsDropped() const {
  return TransactionManagerRustShimAccess::transactionsDropped(*this);
}

val_t TransactionManager::getMinGasPriceForBlockInclusion() const {
  return TransactionManagerRustShimAccess::getMinGasPriceForBlockInclusion(*this);
}

val_t TransactionManager::gasPriceBid() const { return TransactionManagerRustShimAccess::gasPriceBid(*this); }

void TransactionManager::updateGasPrice(const SharedTransactions& trxs) {
  TransactionManagerRustShimAccess::updateGasPrice(*this, trxs);
}

std::shared_ptr<Transaction> TransactionManager::getNonFinalizedTransaction(const trx_hash_t& hash) const {
  return TransactionManagerRustShimAccess::getNonFinalizedTransaction(*this, hash);
}

unsigned long TransactionManager::getTransactionCount() const {
  return TransactionManagerRustShimAccess::getTransactionCount(*this);
}

void TransactionManager::saveTransactionsFromDagBlock(const SharedTransactions& trxs) {
  TransactionManagerRustShimAccess::saveTransactionsFromDagBlock(*this, trxs);
}

void TransactionManager::removeNonFinalizedTransactions(std::unordered_set<trx_hash_t>&& transactions) {
  TransactionManagerRustShimAccess::removeNonFinalizedTransactions(*this, std::move(transactions));
}

std::shared_mutex& TransactionManager::getTransactionsMutex() {
  // Keep lock ownership inside the shim boundary while Rust owns queue state.
  return TransactionManagerRustShimAccess::transactionsMutex(*this);
}

void TransactionManager::updateFinalizedTransactionsStatus(const PeriodData& period_data) {
  TransactionManagerRustShimAccess::updateFinalizedTransactionsStatus(*this, period_data);
}

rustaxa::TransactionManagerFinalizedStatusCommandReport
TransactionManager::updateFinalizedTransactionsStatusForPbftFinalization(const PeriodData& period_data) {
  return TransactionManagerRustShimAccess::updateFinalizedTransactionsStatusReport(*this, period_data);
}

void TransactionManager::initializeRecentlyFinalizedTransactions(const PeriodData& period_data) {
  TransactionManagerRustShimAccess::initializeRecentlyFinalizedTransactions(*this, period_data);
}

SharedTransactions TransactionManager::getBlockTransactions(const DagBlock& blk, PbftPeriod proposal_period) {
  return TransactionManagerRustShimAccess::getTransactions(*this, blk.getTrxs(), proposal_period);
}

SharedTransactions TransactionManager::getTransactions(const vec_trx_t& trxs_hashes, PbftPeriod proposal_period) {
  return TransactionManagerRustShimAccess::getTransactions(*this, trxs_hashes, proposal_period);
}

std::pair<rustaxa::DagVerifyBlockSessionStep, SharedTransactions>
TransactionManager::executeDagVerifyTransactionAvailability() const {
  return TransactionManagerRustShimAccess::executeDagVerifyTransactionAvailability(*this);
}

std::shared_ptr<Transaction> TransactionManager::getTransaction(const trx_hash_t& hash) const {
  return TransactionManagerRustShimAccess::getTransaction(*this, hash);
}

void TransactionManager::recoverNonfinalizedTransactions() {
  TransactionManagerRustShimAccess::recoverNonfinalizedTransactions(*this);
}

std::pair<bool, std::string> TransactionManager::verifyTransaction(const std::shared_ptr<Transaction>& trx) const {
  return TransactionManagerRustShimAccess::verifyTransaction(*this, trx);
}

std::pair<bool, std::string> TransactionManager::insertTransaction(const std::shared_ptr<Transaction>& trx) {
  return TransactionManagerRustShimAccess::insertTransaction(*this, trx);
}

TransactionStatus TransactionManager::insertValidatedTransaction(std::shared_ptr<Transaction>&& tx,
                                                                 bool insert_non_proposable) {
  return TransactionManagerRustShimAccess::insertValidatedTransaction(*this, std::move(tx), insert_non_proposable);
}

}  // namespace taraxa
