#include <algorithm>
#include <cstring>
#include <mutex>
#include <shared_mutex>
#include <stdexcept>
#include <unordered_set>
#include <utility>
#include <vector>

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

dev::bytes fromBridgeBytes(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

constexpr uint8_t kStoredTransactionPending = 1;
constexpr uint8_t kStoredTransactionFinalizedRegular = 2;
constexpr uint8_t kStoredTransactionFinalizedSystem = 3;
constexpr uint8_t kTMVerifyTransactionAccepted = 0;
constexpr uint8_t kTMVerifyTransactionChainIdMismatch = 1;
constexpr uint8_t kTMVerifyTransactionInvalidGas = 2;
constexpr uint8_t kTMVerifyTransactionIntrinsicGas = 3;
constexpr uint8_t kTMVerifyTransactionInvalidSignature = 4;
constexpr uint8_t kTMVerifyTransactionGasPrice = 5;

constexpr uint8_t kTMInsertTransactionAccepted = 0;
constexpr uint8_t kTMInsertTransactionKnown = 1;
constexpr uint8_t kTMInsertTransactionFinalized = 2;
constexpr uint8_t kTMInsertTransactionCouldNotInsert = 3;
constexpr uint8_t kTMQueueStatusInserted = 0;
constexpr uint8_t kTMQueueStatusInsertedNonProposable = 1;
constexpr uint8_t kTMQueueStatusKnown = 2;
constexpr uint8_t kTMQueueStatusOverflow = 3;

bool isFinalizedStoredTransactionSource(uint8_t source) {
  return source == kStoredTransactionFinalizedRegular || source == kStoredTransactionFinalizedSystem;
}

std::shared_ptr<Transaction> materializeStoredTransaction(
    const rustaxa::TransactionManagerStoredTransactionLookup& lookup, const char* error_prefix) {
  if (!lookup.found) {
    return nullptr;
  }

  std::shared_ptr<Transaction> transaction;
  if (lookup.source == kStoredTransactionPending || lookup.source == kStoredTransactionFinalizedRegular) {
    transaction = std::make_shared<Transaction>(fromBridgeBytes(lookup.tx_rlp));
  } else if (lookup.source == kStoredTransactionFinalizedSystem) {
    transaction = std::make_shared<SystemTransaction>(fromBridgeBytes(lookup.tx_rlp));
  } else {
    throw DbException(std::string(error_prefix) + ": Rust returned an unknown transaction source");
  }

  if (transaction->getHash() != fromBridgeHash(lookup.hash)) {
    throw DbException(std::string(error_prefix) + ": Rust returned transaction RLP that does not match the key hash");
  }
  return transaction;
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

std::pair<bool, std::string> insertTransactionResultFromRustStatus(uint8_t status, uint64_t finalized_period) {
  switch (status) {
    case kTMInsertTransactionAccepted:
      return {true, ""};
    case kTMInsertTransactionKnown:
      return {false, "Transaction already in transactions pool"};
    case kTMInsertTransactionFinalized:
      return {false, "Transaction already finalized in period" + std::to_string(finalized_period)};
    case kTMInsertTransactionCouldNotInsert:
      return {false, "Transaction could not be inserted"};
    default:
      throw std::runtime_error("TransactionManager shim received unknown insert status from Rust bridge");
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

class TransactionManagerRustShimAccess {
 public:
  static std::pair<bool, std::string> verifyTransaction(const TransactionManagerOld& manager,
                                                        const std::shared_ptr<Transaction>& trx) {
    if (!manager.final_chain_) {
      return {true, ""};
    }

    bool signature_valid = true;
    try {
      trx->getSender();
    } catch (const Transaction::InvalidSignature&) {
      signature_valid = false;
    }

    const auto block_num = manager.final_chain_->lastBlockNumber();
    rustaxa::TransactionManagerVerifyTransactionFact fact;
    fact.tx_hash = toBridgeHash(trx->getHash());
    fact.chain_id = trx->getChainID();
    fact.expected_chain_id = manager.kConf.genesis.chain_id;
    fact.gas_limit = trx->getGas();
    fact.max_gas_limit = manager.kConf.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
    fact.last_block_number = block_num;
    fact.cornus_active = manager.kConf.genesis.state.hardforks.isOnCornusHardfork(block_num);
    fact.intrinsic_gas_covered = trx->intrinsicGasCovered();
    fact.signature_valid = signature_valid;
    fact.gas_price = toBridgeU256(trx->getGasPrice());
    fact.minimum_gas_price = toBridgeU256(val_t(manager.kConf.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));

    const auto outcome = [&]() {
      try {
        return rustaxa::transaction_manager_verify_transaction(fact);
      } catch (const std::exception& e) {
        throw std::runtime_error(std::string("RUST_TX_MANAGER_VERIFY_TRANSACTION_FAILED: ") + e.what());
      }
    }();

    return verifyTransactionResultFromRustStatus(outcome.status, fact.chain_id, fact.expected_chain_id);
  }

  static std::pair<bool, std::string> insertTransaction(TransactionManager& manager,
                                                        const std::shared_ptr<Transaction>& trx) {
    if (isTransactionKnown(manager, trx->getHash())) {
      return {false, "Transaction already in transactions pool"};
    }

    const auto verified = verifyTransaction(manager, trx);
    if (!verified.first) {
      return verified;
    }

    auto trx_copy = trx;
    const auto queue_status = insertValidatedTransaction(manager, std::move(trx_copy), false);

    bool has_finalized_period = false;
    uint64_t finalized_period = 0;
    if (queue_status == TransactionStatus::Known) {
      if (const auto location = manager.db_->getTransactionLocation(trx->getHash())) {
        has_finalized_period = true;
        finalized_period = location->period;
      }
    }

    rustaxa::TransactionManagerInsertTransactionFact fact;
    fact.tx_hash = toBridgeHash(trx->getHash());
    fact.hash_known = false;
    fact.queue_status = static_cast<uint8_t>(queue_status);
    fact.has_finalized_period = has_finalized_period;
    fact.finalized_period = finalized_period;

    const auto outcome = [&]() {
      try {
        return rustaxa::transaction_manager_insert_transaction(fact);
      } catch (const std::exception& e) {
        throw std::runtime_error(std::string("RUST_TX_MANAGER_INSERT_TRANSACTION_FAILED: ") + e.what());
      }
    }();

    return insertTransactionResultFromRustStatus(
        outcome.status, outcome.finalized_period_known ? outcome.finalized_period : finalized_period);
  }

  static TransactionStatus insertValidatedTransaction(TransactionManager& manager, std::shared_ptr<Transaction>&& tx,
                                                      bool insert_non_proposable) {
    const auto trx_hash = tx->getHash();

    std::unique_lock transactions_lock(manager.transactions_mutex_);
    const auto sender = tx->getSender();
    const auto account = manager.final_chain_->getAccount(sender);

    rustaxa::TransactionManagerValidatedInsertFact fact;
    fact.tx_hash = toBridgeHash(trx_hash);
    fact.transaction_nonce = toBridgeU256(tx->getNonce());
    fact.transaction_cost = toBridgeU256(tx->getCost());
    fact.gas_limit = tx->getGas();
    fact.propose_dag_gas_limit = manager.kConf.propose_dag_gas_limit;
    fact.insert_non_proposable = insert_non_proposable;
    fact.in_non_finalized_cache = manager.nonfinalized_transactions_in_dag_.contains(trx_hash);
    fact.in_recently_finalized_cache = manager.recently_finalized_transactions_.contains(trx_hash);
    fact.account_found = account.has_value();
    fact.account_nonce = toBridgeU256(account ? account->nonce : 0);
    fact.account_balance = toBridgeU256(account ? account->balance : 0);

    const auto plan = [&]() {
      try {
        return rustaxa::transaction_manager_plan_validated_insert(fact);
      } catch (const std::exception& e) {
        throw std::runtime_error(std::string("RUST_TX_MANAGER_VALIDATED_INSERT_FAILED: ") + e.what());
      }
    }();

    if (!plan.should_insert_queue) {
      return transactionStatusFromBridge(plan.status);
    }

    LOG(manager.log_dg_) << "Transaction " << trx_hash << " inserted in trx pool";
    if (plan.emit_transaction_added) {
      manager.emitTransactionAddedForRust(trx_hash);
    }
    const auto queue_status = manager.transactions_pool_.insert(std::move(tx), plan.queue_proposable,
                                                                manager.final_chain_->lastBlockNumber());
    return queue_status;
  }

  /**
   * Runs Rust-backed deterministic transaction packing against the legacy manager's live C++ state.
   *
   * The friend accessor exists only because the migration facade must reuse existing private pool, lock, cache, and
   * FinalChain members without copying the whole transaction lifecycle into shim-owned C++ code. Rust owns the planner
   * decisions; C++ owns live transaction pointers, estimation, queue mutation, and logging.
   */
  static std::pair<SharedTransactions, std::vector<uint64_t>> packTrxs(TransactionManagerOld& manager,
                                                                       PbftPeriod proposal_period,
                                                                       uint64_t weight_limit) {
    auto planner = rustaxa::create_transaction_pack_planner(weight_limit, kMinTxGas);

    SharedTransactions candidates;
    {
      std::shared_lock transactions_lock(manager.transactions_mutex_);
      candidates = manager.transactions_pool_.getOrderedTransactions(planner->transaction_pack_max_candidate_count());
    }

    std::vector<uint64_t> estimations;
    SharedTransactions selected_transactions;
    for (const auto& candidate : candidates) {
      rustaxa::TransactionPackCandidateInput input;
      input.hash = toBridgeHash(candidate->getHash());
      input.declared_gas = candidate->getGas();
      if (!planner->transaction_pack_consider_candidate(input).should_estimate) {
        continue;
      }

      auto estimate = manager.estimateTransactionGas(candidate, proposal_period);
      rustaxa::TransactionPackEstimateInput estimate_input;
      estimate_input.hash = input.hash;
      estimate_input.gas_used = estimate.gas_used;
      const auto outcome = planner->transaction_pack_record_estimate(estimate_input);

      if (outcome.demote_to_non_proposable) {
        LOG(manager.log_er_) << "Transaction " << candidate->getHash()
                             << " has invalid estimation: " << estimate.gas_used;
        std::unique_lock transactions_lock(manager.transactions_mutex_);
        if (!manager.transactions_pool_.demoteToNonProposable(candidate->getHash(),
                                                              manager.final_chain_->lastBlockNumber())) {
          throw std::runtime_error("Rust transaction queue failed to demote invalid-estimate transaction");
        }
        continue;
      }

      if (outcome.selected) {
        selected_transactions.push_back(candidate);
        estimations.push_back(outcome.gas_used);
      }

      if (outcome.stop) {
        break;
      }
    }
    return {selected_transactions, estimations};
  }

  /**
   * Persists transactions accepted by a DAG block.
   *
   * C++ owns only the live transaction pointers, cache fact snapshot, and
   * sidecar mutation. Rust owns duplicate filtering, nonce-gated finalized
   * storage checks, accepted ordering, count planning, and the atomic storage
   * write. If the bridge write fails, no C++ transaction state is mutated.
   */
  static void saveTransactionsFromDagBlock(TransactionManagerOld& manager, SharedTransactions const& trxs) {
    std::unique_lock transactions_lock(manager.transactions_mutex_);

    rust::Vec<rustaxa::DagTransactionSaveFact> facts;
    facts.reserve(trxs.size());
    std::vector<std::shared_ptr<Transaction>> transaction_by_input_index;
    transaction_by_input_index.reserve(trxs.size());
    manager.nonfinalized_transactions_in_dag_.reserve(manager.nonfinalized_transactions_in_dag_.size() + trxs.size());

    uint64_t input_index = 0;
    for (const auto& transaction : trxs) {
      const auto trx_hash = transaction->getHash();
      const auto account =
          manager.final_chain_->getAccount(transaction->getSender()).value_or(taraxa::state_api::ZeroAccount);

      rustaxa::DagTransactionSaveFact fact;
      fact.input_index = input_index++;
      fact.hash = toBridgeHash(trx_hash);
      fact.trx_rlp = toBridgeBytes(transaction->rlp());
      fact.transaction_nonce = toBridgeU256(transaction->getNonce());
      fact.sender_account_nonce = toBridgeU256(account.nonce);
      fact.in_non_finalized_cache = manager.nonfinalized_transactions_in_dag_.contains(trx_hash);
      fact.in_recently_finalized_cache = manager.recently_finalized_transactions_.contains(trx_hash);
      facts.push_back(std::move(fact));
      transaction_by_input_index.push_back(transaction);
    }

    const auto outcome = [&]() {
      try {
        return rustaxa::save_transactions_from_dag_block(manager.db_->rustStorage(), manager.trx_count_,
                                                         std::move(facts));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_DAG_TX_PERSIST_FAILED: ") + e.what());
      }
    }();

    for (const auto& accepted : outcome.accepted) {
      if (accepted.input_index >= transaction_by_input_index.size()) {
        throw DbException("RUST_STORAGE_DAG_TX_PERSIST_FAILED: Rust returned an out-of-range transaction input index");
      }
      const auto& transaction = transaction_by_input_index[static_cast<size_t>(accepted.input_index)];
      const auto trx_hash = transaction->getHash();
      if (fromBridgeHash(accepted.hash) != trx_hash) {
        throw DbException("RUST_STORAGE_DAG_TX_PERSIST_FAILED: Rust returned a transaction hash/index mismatch");
      }

      manager.nonfinalized_transactions_in_dag_.emplace(trx_hash, transaction);
      if (manager.transactions_pool_.erase(transaction)) {
        LOG(manager.log_dg_) << "Transaction " << trx_hash << " removed from trx pool ";
      }
    }
    manager.trx_count_ = outcome.target_transaction_count;
  }

  /**
   * Clears only live non-finalized transaction sidecars after Rust finalization
   * storage cleanup has committed.
   *
   * This intentionally performs no storage deletes and does not update
   * transaction counters, matching the legacy expired-DAG cleanup semantics after
   * moving persistent deletion to Rust.
   */
  static void forgetExpiredNonFinalizedTransactionSidecars(TransactionManagerOld& manager,
                                                           std::unordered_set<trx_hash_t>&& transactions) {
    for (const auto& trx_hash : transactions) {
      manager.nonfinalized_transactions_in_dag_.erase(trx_hash);
    }
  }

  /**
   * Retrieves a transaction from C++ live caches first and falls back to Rust-backed
   * storage for persistence-backed lookup.
   *
   * This keeps transaction object materialization and identity ownership in-memory,
   * while storage reads remain authoritative for non-live transactions.
   */
  static std::shared_ptr<Transaction> getTransaction(const TransactionManagerOld& manager, const trx_hash_t& hash) {
    {
      std::shared_lock transactions_lock(manager.transactions_mutex_);
      if (const auto trx = manager.transactions_pool_.get(hash)) {
        return trx;
      }
      if (const auto it = manager.nonfinalized_transactions_in_dag_.find(hash);
          it != manager.nonfinalized_transactions_in_dag_.end()) {
        return it->second;
      }
      if (const auto it = manager.recently_finalized_transactions_.find(hash);
          it != manager.recently_finalized_transactions_.end()) {
        return it->second;
      }
    }

    rust::Vec<rustaxa::TransactionManagerStoredTransactionRequest> requests;
    requests.reserve(1);
    rustaxa::TransactionManagerStoredTransactionRequest request;
    request.input_index = 0;
    request.hash = toBridgeHash(hash);
    requests.push_back(request);

    const auto lookups = [&]() {
      try {
        return rustaxa::transaction_manager_load_stored_transactions(manager.db_->rustStorage(), std::move(requests));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_TX_RETRIEVAL_FAILED: ") + e.what());
      }
    }();

    if (lookups.size() != 1 || lookups[0].input_index != 0 || fromBridgeHash(lookups[0].hash) != hash) {
      throw DbException("RUST_STORAGE_TX_RETRIEVAL_FAILED: Rust returned an invalid transaction lookup response");
    }

    return materializeStoredTransaction(lookups[0], "RUST_STORAGE_TX_RETRIEVAL_FAILED");
  }

  static std::vector<std::shared_ptr<Transaction>> getNonfinalizedTrx(const TransactionManagerOld& manager,
                                                                      const std::vector<trx_hash_t>& hashes) {
    std::vector<std::shared_ptr<Transaction>> ret;
    ret.reserve(hashes.size());
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    for (const auto& hash : hashes) {
      if (const auto it = manager.nonfinalized_transactions_in_dag_.find(hash);
          it != manager.nonfinalized_transactions_in_dag_.end()) {
        ret.push_back(it->second);
      }
    }
    return ret;
  }

  static std::shared_ptr<Transaction> getNonFinalizedTransaction(const TransactionManagerOld& manager,
                                                                 const trx_hash_t& hash) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    if (const auto it = manager.nonfinalized_transactions_in_dag_.find(hash);
        it != manager.nonfinalized_transactions_in_dag_.end()) {
      return it->second;
    }
    return {};
  }

  static std::unordered_set<trx_hash_t> excludeFinalizedTransactions(const TransactionManagerOld& manager,
                                                                     const std::vector<trx_hash_t>& hashes) {
    rust::Vec<rustaxa::TransactionManagerFinalizedFilterFact> facts;
    facts.reserve(hashes.size());
    {
      std::shared_lock transactions_lock(manager.transactions_mutex_);
      uint64_t input_index = 0;
      for (const auto& hash : hashes) {
        rustaxa::TransactionManagerFinalizedFilterFact fact;
        fact.input_index = input_index++;
        fact.hash = toBridgeHash(hash);
        fact.in_recently_finalized_cache = manager.recently_finalized_transactions_.contains(hash);
        facts.push_back(fact);
      }
    }

    const auto plan = [&]() {
      try {
        return rustaxa::transaction_manager_filter_non_finalized(manager.db_->rustStorage(), std::move(facts));
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

  static bool verifyTransactionsNotFinalized(const TransactionManagerOld& manager, const SharedTransactions& trxs) {
    rust::Vec<rustaxa::TransactionManagerVerifyNotFinalizedFact> facts;
    facts.reserve(trxs.size());
    uint64_t input_index = 0;
    for (const auto& transaction : trxs) {
      const auto trx_hash = transaction->getHash();
      const auto account = manager.final_chain_->getAccount(transaction->getSender()).value_or(state_api::ZeroAccount);

      rustaxa::TransactionManagerVerifyNotFinalizedFact fact;
      fact.input_index = input_index++;
      fact.hash = toBridgeHash(trx_hash);
      fact.transaction_nonce = toBridgeU256(transaction->getNonce());
      fact.sender_account_nonce = toBridgeU256(account.nonce);
      {
        std::shared_lock transactions_lock(manager.transactions_mutex_);
        fact.in_recently_finalized_cache = manager.recently_finalized_transactions_.contains(trx_hash);
      }
      facts.push_back(fact);
    }

    const auto outcome = [&]() {
      try {
        return rustaxa::transaction_manager_verify_not_finalized(manager.db_->rustStorage(), std::move(facts));
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

    std::shared_lock transactions_lock(manager.transactions_mutex_);
    if (manager.recently_finalized_transactions_.contains(trx_hash)) {
      LOG(manager.log_er_) << "Transaction " << trx_hash << " already finalized";
    } else {
      LOG(manager.log_er_) << "Transaction " << trx_hash << " already finalized in db";
    }
    return false;
  }

  static std::vector<SharedTransactions> getAllPoolTrxs(const TransactionManagerOld& manager) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    return manager.transactions_pool_.getAllTransactions();
  }

  static std::pair<std::vector<std::shared_ptr<Transaction>>, std::vector<trx_hash_t>> getPoolTransactions(
      const TransactionManagerOld& manager, const std::vector<trx_hash_t>& trx_to_query) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    std::pair<std::vector<std::shared_ptr<Transaction>>, std::vector<trx_hash_t>> result;
    for (const auto& hash : trx_to_query) {
      auto trx = manager.transactions_pool_.get(hash);
      if (trx) {
        result.first.emplace_back(trx);
      } else {
        result.second.emplace_back(hash);
      }
    }
    return result;
  }

  static unsigned long getTransactionCount(const TransactionManagerOld& manager) {
    std::shared_lock shared_transactions_lock(manager.transactions_mutex_);
    return manager.trx_count_;
  }

  static void blockFinalized(TransactionManagerOld& manager, EthBlockNumber block_number) {
    std::unique_lock transactions_lock(manager.transactions_mutex_);
    manager.transactions_pool_.blockFinalized(block_number);
  }

  static bool isTransactionKnown(TransactionManagerOld& manager, const trx_hash_t& trx_hash) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    return manager.transactions_pool_.isTransactionKnown(trx_hash);
  }

  static size_t getTransactionPoolSize(const TransactionManagerOld& manager) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    return manager.transactions_pool_.size();
  }

  static bool isTransactionPoolFull(const TransactionManagerOld& manager, size_t percentage) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    return manager.transactions_pool_.size() >= (manager.kConf.transactions_pool_size * percentage / 100);
  }

  static bool nonProposableTransactionsOverTheLimit(const TransactionManagerOld& manager) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    return manager.transactions_pool_.nonProposableTransactionsOverTheLimit();
  }

  static size_t getNonfinalizedTrxSize(const TransactionManagerOld& manager) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    return manager.nonfinalized_transactions_in_dag_.size();
  }

  static bool transactionsDropped(const TransactionManagerOld& manager) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    return manager.transactions_pool_.transactionsDropped();
  }

  static val_t getMinGasPriceForBlockInclusion(const TransactionManagerOld& manager) {
    std::shared_lock transactions_lock(manager.transactions_mutex_);
    return manager.transactions_pool_.getMinGasPriceForBlockInclusion(manager.kConf.propose_dag_gas_limit);
  }

  /**
   * Materializes ordered transaction hashes from C++ live views and Rust-backed
   * storage.
   *
   * C++ keeps live `SharedTransaction` ownership and proposal-period nonce
   * filtering. Rust owns storage lookup, transaction-location decoding, period
   * data extraction, and regular/system source classification for cache misses.
   */
  static SharedTransactions getTransactions(const TransactionManagerOld& manager, const vec_trx_t& trxs_hashes,
                                            PbftPeriod proposal_period) {
    std::vector<std::shared_ptr<Transaction>> ordered_transactions(trxs_hashes.size());
    rust::Vec<rustaxa::TransactionManagerStoredTransactionRequest> requests;

    {
      std::shared_lock transactions_lock(manager.transactions_mutex_);
      for (size_t i = 0; i < trxs_hashes.size(); ++i) {
        const auto& tx_hash = trxs_hashes[i];
        if (auto trx = manager.transactions_pool_.get(tx_hash)) {
          ordered_transactions[i] = std::move(trx);
        } else {
          auto trx_it = manager.nonfinalized_transactions_in_dag_.find(tx_hash);
          if (trx_it != manager.nonfinalized_transactions_in_dag_.end()) {
            ordered_transactions[i] = trx_it->second;
          } else {
            trx_it = manager.recently_finalized_transactions_.find(tx_hash);
            if (trx_it != manager.recently_finalized_transactions_.end()) {
              ordered_transactions[i] = trx_it->second;
            } else {
              rustaxa::TransactionManagerStoredTransactionRequest request;
              request.input_index = static_cast<uint64_t>(i);
              request.hash = toBridgeHash(tx_hash);
              requests.push_back(request);
            }
          }
        }
      }
    }

    if (!requests.empty()) {
      const auto lookups = [&]() {
        try {
          return rustaxa::transaction_manager_load_stored_transactions(manager.db_->rustStorage(), std::move(requests));
        } catch (const std::exception& e) {
          throw DbException(std::string("RUST_STORAGE_TX_RETRIEVAL_FAILED: ") + e.what());
        }
      }();

      for (const auto& lookup : lookups) {
        if (lookup.input_index >= trxs_hashes.size()) {
          throw DbException("RUST_STORAGE_TX_RETRIEVAL_FAILED: Rust returned an out-of-range transaction index");
        }
        const auto input_index = static_cast<size_t>(lookup.input_index);
        const auto& expected_hash = trxs_hashes[input_index];
        if (fromBridgeHash(lookup.hash) != expected_hash) {
          throw DbException("RUST_STORAGE_TX_RETRIEVAL_FAILED: Rust returned a transaction hash/index mismatch");
        }

        auto transaction = materializeStoredTransaction(lookup, "RUST_STORAGE_TX_RETRIEVAL_FAILED");
        if (!transaction) {
          continue;
        }

        if (isFinalizedStoredTransactionSource(lookup.source)) {
          auto acc = manager.final_chain_->getAccount(transaction->getSender(), proposal_period);
          if (acc.has_value() && acc->nonce > transaction->getNonce()) {
            LOG(manager.log_er_) << "Old transaction: " << transaction->getHash();
            continue;
          }
        }
        ordered_transactions[input_index] = std::move(transaction);
      }
    }

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
   * Rebuilds in-memory non-finalized transaction sidecars from Rust-backed storage.
   *
   * Rust loads the recovery payloads and removes stale finalized rows from
   * non-finalized storage before C++ materializes survivor transactions into
   * the live sidecar map. Each survivor has its sender cached before insertion.
   */
  static void recoverNonfinalizedTransactions(TransactionManagerOld& manager) {
    const auto entries = [&]() {
      try {
        return rustaxa::transaction_manager_load_nonfinalized_recovery(manager.db_->rustStorage());
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_TX_RECOVERY_FAILED: ") + e.what());
      }
    }();

    std::vector<std::pair<trx_hash_t, std::shared_ptr<Transaction>>> recovered_transactions;
    recovered_transactions.reserve(entries.size());
    for (const auto& entry : entries) {
      if (entry.finalized) {
        continue;
      }

      auto transaction = std::make_shared<Transaction>(fromBridgeBytes(entry.tx_rlp));
      const auto trx_hash = fromBridgeHash(entry.hash);
      if (transaction->getHash() != trx_hash) {
        throw DbException(
            "RUST_STORAGE_TX_RECOVERY_FAILED: Rust returned transaction RLP that does not match the key hash");
      }
      transaction->getSender();
      recovered_transactions.emplace_back(trx_hash, std::move(transaction));
    }

    std::unique_lock transactions_lock(manager.transactions_mutex_);
    for (auto& [trx_hash, transaction] : recovered_transactions) {
      manager.nonfinalized_transactions_in_dag_.emplace(trx_hash, std::move(transaction));
    }
  }

  static void initializeRecentlyFinalizedTransactions(TransactionManagerOld& manager, const PeriodData& period_data) {
    std::unique_lock transactions_lock(manager.transactions_mutex_);
    for (auto const& trx : period_data.transactions) {
      const auto hash = trx->getHash();
      manager.recently_finalized_transactions_[hash] = trx;
      manager.recently_finalized_transactions_per_period_[period_data.pbft_blk->getPeriod()].push_back(hash);
    }
  }

  static void updateFinalizedTransactionsStatus(TransactionManagerOld& manager, const PeriodData& period_data) {
    const auto recently_finalized_transactions_periods =
        kRecentlyFinalizedTransactionsFactor * manager.final_chain_->delegationDelay();

    rust::Vec<rustaxa::FinalizedTransactionStatusFact> facts;
    facts.reserve(period_data.transactions.size());
    uint64_t input_index = 0;
    for (const auto& transaction : period_data.transactions) {
      const auto trx_hash = transaction->getHash();
      rustaxa::FinalizedTransactionStatusFact fact;
      fact.input_index = input_index++;
      fact.hash = toBridgeHash(trx_hash);
      fact.in_non_finalized_cache = manager.nonfinalized_transactions_in_dag_.contains(trx_hash);
      facts.push_back(std::move(fact));
    }

    const auto plan = [&]() {
      try {
        return rustaxa::update_finalized_transactions_status(
            manager.db_->rustStorage(), period_data.pbft_blk->getPeriod(), recently_finalized_transactions_periods,
            manager.trx_count_, std::move(facts));
      } catch (const std::exception& e) {
        throw DbException(std::string("RUST_STORAGE_FINALIZED_TX_STATUS_FAILED: ") + e.what());
      }
    }();

    if (plan.has_stale_period) {
      const auto stale_period = static_cast<PbftPeriod>(plan.stale_period);
      if (const auto stale_period_it = manager.recently_finalized_transactions_per_period_.find(stale_period);
          stale_period_it != manager.recently_finalized_transactions_per_period_.end()) {
        for (const auto& hash : stale_period_it->second) {
          manager.recently_finalized_transactions_.erase(hash);
        }
        manager.recently_finalized_transactions_per_period_.erase(stale_period_it);
      }
    }

    for (const auto& action : plan.accepted) {
      if (action.input_index >= period_data.transactions.size()) {
        throw DbException("RUST_STORAGE_FINALIZED_TX_STATUS_FAILED: Rust returned an out-of-range transaction index");
      }
      const auto& transaction = period_data.transactions[static_cast<size_t>(action.input_index)];
      const auto trx_hash = transaction->getHash();
      if (fromBridgeHash(action.hash) != trx_hash) {
        throw DbException("RUST_STORAGE_FINALIZED_TX_STATUS_FAILED: Rust returned a transaction hash/index mismatch");
      }

      manager.recently_finalized_transactions_[trx_hash] = transaction;
      manager.recently_finalized_transactions_per_period_[period_data.pbft_blk->getPeriod()].push_back(trx_hash);
      manager.transactions_pool_.markTransactionKnown(trx_hash);
      if (manager.nonfinalized_transactions_in_dag_.erase(trx_hash)) {
        LOG(manager.log_dg_) << "Transaction " << trx_hash << " removed from nonfinalized transactions";
      }
      if (manager.transactions_pool_.erase(transaction)) {
        LOG(manager.log_dg_) << "Transaction " << trx_hash << " removed from transactions_pool_";
      }
    }

    manager.trx_count_ = plan.target_transaction_count;

    if (plan.purge_transaction_queue) {
      manager.transactions_pool_.purge();
    }
  }
};

std::pair<SharedTransactions, std::vector<uint64_t>> TransactionManager::packTrxs(PbftPeriod proposal_period,
                                                                                  uint64_t weight_limit) {
  return TransactionManagerRustShimAccess::packTrxs(*this, proposal_period, weight_limit);
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

std::shared_ptr<Transaction> TransactionManager::getNonFinalizedTransaction(const trx_hash_t& hash) const {
  return TransactionManagerRustShimAccess::getNonFinalizedTransaction(*this, hash);
}

unsigned long TransactionManager::getTransactionCount() const {
  return TransactionManagerRustShimAccess::getTransactionCount(*this);
}

void TransactionManager::saveTransactionsFromDagBlock(const SharedTransactions& trxs) {
  TransactionManagerRustShimAccess::saveTransactionsFromDagBlock(*this, trxs);
}

void TransactionManager::forgetExpiredNonFinalizedTransactionSidecars(std::unordered_set<trx_hash_t>&& transactions) {
  TransactionManagerRustShimAccess::forgetExpiredNonFinalizedTransactionSidecars(*this, std::move(transactions));
}

void TransactionManager::updateFinalizedTransactionsStatus(const PeriodData& period_data) {
  TransactionManagerRustShimAccess::updateFinalizedTransactionsStatus(*this, period_data);
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
