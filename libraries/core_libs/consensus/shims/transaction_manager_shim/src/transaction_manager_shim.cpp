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

dev::bytes fromBridgeBytes(const rust::Vec<uint8_t>& bytes) {
  return dev::bytes(bytes.begin(), bytes.end());
}

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

}  // namespace

class TransactionManagerRustShimAccess {
 public:
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
        auto transaction = candidate;
        manager.transactions_pool_.erase(transaction);
        manager.transactions_pool_.insert(std::move(transaction), false, manager.final_chain_->lastBlockNumber());
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

}  // namespace taraxa
