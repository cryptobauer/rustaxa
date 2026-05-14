#include <algorithm>
#include <cstring>
#include <shared_mutex>
#include <stdexcept>
#include <unordered_set>
#include <utility>
#include <vector>

#include "rustaxa-bridge/ffi.rs.h"
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

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
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

}  // namespace taraxa
