#include <cstring>
#include <shared_mutex>
#include <stdexcept>
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
};

std::pair<SharedTransactions, std::vector<uint64_t>> TransactionManager::packTrxs(PbftPeriod proposal_period,
                                                                                  uint64_t weight_limit) {
  return TransactionManagerRustShimAccess::packTrxs(*this, proposal_period, weight_limit);
}

}  // namespace taraxa
