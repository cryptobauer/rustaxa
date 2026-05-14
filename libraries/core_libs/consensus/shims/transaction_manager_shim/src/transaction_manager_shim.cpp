#include <cstring>
#include <limits>
#include <shared_mutex>
#include <stdexcept>
#include <unordered_set>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction_manager.hpp"

namespace taraxa {
namespace {

std::array<uint8_t, 32> toBridgeHash(const trx_hash_t& hash) {
  std::array<uint8_t, 32> bytes{};
  std::memcpy(bytes.data(), hash.data(), bytes.size());
  return bytes;
}

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
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
   * Persists DAG-accepted transactions through Rust storage, then updates the C++
   * live transaction indexes after the storage commit succeeds.
   *
   * Inputs are live C++ transaction objects from a DAG block. The method keeps
   * TransactionManager as the owner of duplicate/finalized filtering and the
   * `trx_count_` value because those decisions depend on C++ live caches and
   * FinalChain account reads during this migration slice. Rust owns the atomic
   * storage write of accepted transaction RLP payloads and the target status
   * counter. If the bridge write fails, no C++ pool/cache/count mutation occurs.
   */
  static void saveTransactionsFromDagBlock(TransactionManagerOld& manager, SharedTransactions const& trxs) {
    std::unique_lock transactions_lock(manager.transactions_mutex_);

    SharedTransactions accepted_transactions;
    std::unordered_set<trx_hash_t> accepted_hashes;
    accepted_transactions.reserve(trxs.size());
    accepted_hashes.reserve(trxs.size());

    for (const auto& t : trxs) {
      const auto trx_hash = t->getHash();

      bool transaction_in_dag_or_finalized = manager.nonfinalized_transactions_in_dag_.contains(trx_hash) ||
                                             manager.recently_finalized_transactions_.contains(trx_hash) ||
                                             accepted_hashes.contains(trx_hash);
      if (transaction_in_dag_or_finalized) {
        continue;
      }

      // Checking nonce is cheaper than checking DB, verify with nonce if possible.
      const auto account = manager.final_chain_->getAccount(t->getSender()).value_or(taraxa::state_api::ZeroAccount);
      if (account.nonce >= t->getNonce()) {
        // This is a very rare scenario but it can happen:
        // The check against database is needed because there is a possibility that transaction was executed within last
        // 100 period (dag proposal period) but it might not be part of recently_finalized_transactions_
        transaction_in_dag_or_finalized = manager.db_->transactionFinalized(trx_hash);
      }

      if (!transaction_in_dag_or_finalized) {
        accepted_hashes.emplace(trx_hash);
        accepted_transactions.push_back(t);
      }
    }

    if (accepted_transactions.empty()) {
      return;
    }

    if (manager.trx_count_ > std::numeric_limits<uint64_t>::max() - accepted_transactions.size()) {
      throw std::overflow_error("RUST_STORAGE_DAG_TX_PERSIST_FAILED: transaction count overflow");
    }

    const auto new_transaction_count = manager.trx_count_ + accepted_transactions.size();
    rust::Vec<rustaxa::NonFinalizedTransactionPayload> payloads;
    payloads.reserve(accepted_transactions.size());
    for (const auto& transaction : accepted_transactions) {
      rustaxa::NonFinalizedTransactionPayload payload;
      payload.hash = toBridgeHash(transaction->getHash());
      payload.trx_rlp = toBridgeBytes(transaction->rlp());
      payloads.push_back(std::move(payload));
    }

    try {
      manager.db_->rustStorage().save_non_finalized_transactions(std::move(payloads), new_transaction_count);
    } catch (const std::exception& e) {
      throw DbException(std::string("RUST_STORAGE_DAG_TX_PERSIST_FAILED: ") + e.what());
    }

    for (const auto& transaction : accepted_transactions) {
      const auto trx_hash = transaction->getHash();
      manager.nonfinalized_transactions_in_dag_.emplace(trx_hash, transaction);
      if (manager.transactions_pool_.erase(transaction)) {
        LOG(manager.log_dg_) << "Transaction " << trx_hash << " removed from trx pool ";
      }
    }
    manager.trx_count_ = new_transaction_count;
  }
};

std::pair<SharedTransactions, std::vector<uint64_t>> TransactionManager::packTrxs(PbftPeriod proposal_period,
                                                                                  uint64_t weight_limit) {
  return TransactionManagerRustShimAccess::packTrxs(*this, proposal_period, weight_limit);
}

void TransactionManager::saveTransactionsFromDagBlock(const SharedTransactions& trxs) {
  TransactionManagerRustShimAccess::saveTransactionsFromDagBlock(*this, trxs);
}

}  // namespace taraxa
