#include "graphql/types/dag_block.hpp"

#include <libdevcore/CommonJS.h>

#include "graphql/account.hpp"
#include "graphql/transaction.hpp"

namespace graphql::taraxa {

#ifdef RUSTAXA_ENABLE
namespace {
dev::h256 hashFromBridge(const std::array<uint8_t, 32>& hash) {
  return dev::h256(hash.data(), dev::h256::ConstructFromPointer);
}

dev::Address addressFromBridge(const std::array<uint8_t, 20>& address) {
  return dev::Address(address.data(), dev::Address::ConstructFromPointer);
}

dev::bytes bytesFromBridge(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }
}  // namespace
#endif

DagBlock::DagBlock(std::shared_ptr<::taraxa::DagBlock> dag_block,
                   std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                   std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                   std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                   std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept
    : dag_block_(std::move(dag_block)),
      final_chain_(std::move(final_chain)),
      pbft_manager_(std::move(pbft_manager)),
      transaction_manager_(std::move(transaction_manager)),
      get_block_by_num_(get_block_by_num) {}

#ifdef RUSTAXA_ENABLE
DagBlock::DagBlock(rustaxa::DagBlockPublicView dag_block,
                   std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                   std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                   std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                   std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept
    : rust_dag_block_(std::move(dag_block)),
      final_chain_(std::move(final_chain)),
      pbft_manager_(std::move(pbft_manager)),
      transaction_manager_(std::move(transaction_manager)),
      get_block_by_num_(get_block_by_num) {}
#endif

response::Value DagBlock::getHash() const noexcept {
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    return response::Value(hashFromBridge(rust_dag_block_->hash).toString());
  }
#endif
  return response::Value(dag_block_->getHash().toString());
}

response::Value DagBlock::getPivot() const noexcept {
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    return response::Value(hashFromBridge(rust_dag_block_->pivot).toString());
  }
#endif
  return response::Value(dag_block_->getPivot().toString());
}

std::vector<response::Value> DagBlock::getTips() const noexcept {
  std::vector<response::Value> tips_result;
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    std::transform(rust_dag_block_->tips.begin(), rust_dag_block_->tips.end(), std::back_inserter(tips_result),
                   [](const auto& tip) -> response::Value { return response::Value(hashFromBridge(tip.hash).toString()); });
    return tips_result;
  }
#endif
  const auto tips = dag_block_->getTips();

  std::transform(tips.begin(), tips.end(), std::back_inserter(tips_result),
                 [](const auto& tip) -> response::Value { return response::Value(tip.toString()); });

  return tips_result;
}

response::Value DagBlock::getLevel() const noexcept {
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    return response::Value(static_cast<int>(rust_dag_block_->level));
  }
#endif
  return response::Value(static_cast<int>(dag_block_->getLevel()));
}

std::optional<response::Value> DagBlock::getPbftPeriod() const noexcept {
  std::lock_guard<std::mutex> lock{mu_};
  if (period_) {
    return response::Value(static_cast<int>(*period_));
  }
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    const auto [has_period, period] = pbft_manager_->getDagBlockPeriod(hashFromBridge(rust_dag_block_->hash));
    if (has_period) {
      period_ = period;
      return {response::Value(static_cast<int>(*period_))};
    }
    return std::nullopt;
  }
#endif
  const auto [has_period, period] = pbft_manager_->getDagBlockPeriod(::taraxa::blk_hash_t(dag_block_->getHash()));
  if (has_period) {
    period_ = period;
    return {response::Value(static_cast<int>(*period_))};
  }
  return std::nullopt;
}

std::shared_ptr<object::Account> DagBlock::getAuthor() const noexcept {
  std::lock_guard<std::mutex> lock{mu_};
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    const auto sender = addressFromBridge(rust_dag_block_->sender);
    if (!period_) {
      const auto [has_period, period] = pbft_manager_->getDagBlockPeriod(hashFromBridge(rust_dag_block_->hash));
      if (has_period) {
        period_ = period;
        return std::make_shared<object::Account>(std::make_shared<Account>(final_chain_, sender, *period_));
      }
    }
    return std::make_shared<object::Account>(std::make_shared<Account>(final_chain_, sender));
  }
#endif
  if (!period_) {
    const auto [has_period, period] = pbft_manager_->getDagBlockPeriod(::taraxa::blk_hash_t(dag_block_->getHash()));
    if (has_period) {
      period_ = period;
      return std::make_shared<object::Account>(
          std::make_shared<Account>(final_chain_, dag_block_->getSender(), *period_));
    }
  }
  return std::make_shared<object::Account>(std::make_shared<Account>(final_chain_, dag_block_->getSender()));
}

response::Value DagBlock::getTimestamp() const noexcept {
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    return response::Value(static_cast<int>(rust_dag_block_->timestamp));
  }
#endif
  return response::Value(static_cast<int>(dag_block_->getTimestamp()));
}

response::Value DagBlock::getSignature() const noexcept {
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    return response::Value(dev::toJS(bytesFromBridge(rust_dag_block_->signature)));
  }
#endif
  return response::Value(dev::toJS(dag_block_->getSig()));
}

int DagBlock::getVdf() const noexcept {
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    return rust_dag_block_->vdf_difficulty;
  }
#endif
  return dag_block_->getDifficulty();
}

int DagBlock::getTransactionCount() const noexcept {
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    return static_cast<int>(rust_dag_block_->transactions.size());
  }
#endif
  return static_cast<int>(dag_block_->getTrxs().size());
}

std::optional<std::vector<std::shared_ptr<object::Transaction>>> DagBlock::getTransactions() const noexcept {
  std::vector<std::shared_ptr<object::Transaction>> transactions_result;
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    for (const auto& trx_hash : rust_dag_block_->transactions) {
      transactions_result.push_back(std::make_shared<object::Transaction>(
          std::make_shared<Transaction>(final_chain_, transaction_manager_, get_block_by_num_,
                                        transaction_manager_->getTransaction(hashFromBridge(trx_hash.hash)))));
    }
    return transactions_result;
  }
#endif
  for (const auto& trx_hash : dag_block_->getTrxs()) {
    transactions_result.push_back(std::make_shared<object::Transaction>(std::make_shared<Transaction>(
        final_chain_, transaction_manager_, get_block_by_num_, transaction_manager_->getTransaction(trx_hash))));
  }

  return transactions_result;
}

}  // namespace graphql::taraxa
