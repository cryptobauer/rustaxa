#include "graphql/types/dag_block.hpp"

#include <libdevcore/CommonJS.h>

#include "graphql/transaction.hpp"

#ifdef RUSTAXA_ENABLE
#include "transaction/system_transaction.hpp"
#endif

namespace graphql::taraxa {

namespace {
#ifndef RUSTAXA_ENABLE
DagBlockTransactionReader makeDagBlockTransactionReader(
    const std::shared_ptr<::taraxa::TransactionManager>& transaction_manager) {
  DagBlockTransactionReader reader;
  reader.transaction_by_hash = [transaction_manager](const ::taraxa::trx_hash_t& hash) {
    return transaction_manager ? transaction_manager->getTransaction(hash) : nullptr;
  };
  return reader;
}

DagBlockPeriodReader makeDagBlockPeriodReader(const std::shared_ptr<::taraxa::PbftManager>& pbft_manager) {
  DagBlockPeriodReader reader;
  reader.period_by_hash = [pbft_manager](const ::taraxa::blk_hash_t& hash) -> std::optional<uint64_t> {
    if (!pbft_manager) {
      return std::nullopt;
    }
    const auto [has_period, period] = pbft_manager->getDagBlockPeriod(hash);
    if (!has_period) {
      return std::nullopt;
    }
    return period;
  };
  return reader;
}
#endif
}  // namespace

#ifdef RUSTAXA_ENABLE
namespace {
constexpr uint8_t kConsensusQueryTransactionSourceMissing = 0;
constexpr uint8_t kConsensusQueryTransactionSourcePending = 1;
constexpr uint8_t kConsensusQueryTransactionSourceFinalizedRegular = 2;
constexpr uint8_t kConsensusQueryTransactionSourceFinalizedSystem = 3;

dev::h256 hashFromBridge(const std::array<uint8_t, 32>& hash) {
  return dev::h256(hash.data(), dev::h256::ConstructFromPointer);
}

dev::Address addressFromBridge(const std::array<uint8_t, 20>& address) {
  return dev::Address(address.data(), dev::Address::ConstructFromPointer);
}

dev::bytes bytesFromBridge(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

std::shared_ptr<::taraxa::Transaction> materializeTransactionView(const rustaxa::TransactionPublicView& view) {
  if (!view.found) {
    return nullptr;
  }

  std::shared_ptr<::taraxa::Transaction> transaction;
  if (view.source == kConsensusQueryTransactionSourceFinalizedSystem) {
    transaction = std::make_shared<::taraxa::SystemTransaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.source == kConsensusQueryTransactionSourcePending ||
             view.source == kConsensusQueryTransactionSourceFinalizedRegular) {
    transaction = std::make_shared<::taraxa::Transaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.source != kConsensusQueryTransactionSourceMissing) {
    return nullptr;
  }

  if (transaction && transaction->getHash() != hashFromBridge(view.hash)) {
    return nullptr;
  }
  return transaction;
}
}  // namespace
#endif

#ifndef RUSTAXA_ENABLE
DagBlock::DagBlock(std::shared_ptr<::taraxa::DagBlock> dag_block,
                   std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                   std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                   std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                   std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept
    : dag_block_(std::move(dag_block)),
      account_reader_(makeAccountStateReader(final_chain)),
      transaction_reader_(makeDagBlockTransactionReader(transaction_manager)),
      period_reader_(makeDagBlockPeriodReader(pbft_manager)),
      get_block_by_num_(get_block_by_num) {
  (void)pbft_manager;
}

DagBlock::DagBlock(AccountStateReader account_reader, std::shared_ptr<::taraxa::DagBlock> dag_block,
                   std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                   std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                   std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept
    : DagBlock(std::move(account_reader), makeDagBlockTransactionReader(transaction_manager),
               makeDagBlockPeriodReader(pbft_manager), std::move(dag_block), std::move(get_block_by_num)) {}

DagBlock::DagBlock(AccountStateReader account_reader, DagBlockTransactionReader transaction_reader,
                   std::shared_ptr<::taraxa::DagBlock> dag_block, std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                   std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                   std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept
    : DagBlock(std::move(account_reader), std::move(transaction_reader), makeDagBlockPeriodReader(pbft_manager),
               std::move(dag_block), std::move(get_block_by_num)) {
  (void)transaction_manager;
}
#endif

DagBlock::DagBlock(AccountStateReader account_reader, DagBlockTransactionReader transaction_reader,
                   DagBlockPeriodReader period_reader, std::shared_ptr<::taraxa::DagBlock> dag_block,
                   std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept
    : dag_block_(std::move(dag_block)),
      account_reader_(std::move(account_reader)),
      transaction_reader_(std::move(transaction_reader)),
      period_reader_(std::move(period_reader)),
      get_block_by_num_(std::move(get_block_by_num)) {}

#ifdef RUSTAXA_ENABLE
DagBlock::DagBlock(
    rustaxa::DagBlockPublicView dag_block, AccountStateReader account_reader,
    std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
    std::function<rustaxa::TransactionPublicView(const ::taraxa::trx_hash_t&)> transaction_query,
    std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> receipt_query) noexcept
    : rust_dag_block_(std::move(dag_block)),
      account_reader_(std::move(account_reader)),
      get_block_by_num_(std::move(get_block_by_num)),
      transaction_query_(std::move(transaction_query)),
      receipt_query_(std::move(receipt_query)) {}
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
    std::transform(
        rust_dag_block_->tips.begin(), rust_dag_block_->tips.end(), std::back_inserter(tips_result),
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
    if (rust_dag_block_->finalized_period_found) {
      period_ = rust_dag_block_->finalized_period;
      return {response::Value(static_cast<int>(*period_))};
    }
    return std::nullopt;
  }
#endif
  if (period_reader_.period_by_hash) {
    period_ = period_reader_.period_by_hash(::taraxa::blk_hash_t(dag_block_->getHash()));
  }
  if (period_) {
    return {response::Value(static_cast<int>(*period_))};
  }
  return std::nullopt;
}

std::shared_ptr<object::Account> DagBlock::getAuthor() const noexcept {
  std::lock_guard<std::mutex> lock{mu_};
#ifdef RUSTAXA_ENABLE
  if (rust_dag_block_) {
    const auto sender = addressFromBridge(rust_dag_block_->sender);
    if (rust_dag_block_->finalized_period_found) {
      period_ = rust_dag_block_->finalized_period;
      return std::make_shared<object::Account>(std::make_shared<Account>(account_reader_, sender, *period_));
    }
    return std::make_shared<object::Account>(std::make_shared<Account>(account_reader_, sender));
  }
#endif
  if (!period_ && period_reader_.period_by_hash) {
    period_ = period_reader_.period_by_hash(::taraxa::blk_hash_t(dag_block_->getHash()));
    if (period_) {
      return std::make_shared<object::Account>(
          std::make_shared<Account>(account_reader_, dag_block_->getSender(), *period_));
    }
  }
  return std::make_shared<object::Account>(std::make_shared<Account>(account_reader_, dag_block_->getSender()));
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
    if (!transaction_query_ || !receipt_query_) {
      return std::nullopt;
    }
    for (const auto& trx_hash : rust_dag_block_->transactions) {
      auto transaction_view = transaction_query_(hashFromBridge(trx_hash.hash));
      auto transaction = materializeTransactionView(transaction_view);
      if (!transaction) {
        return std::nullopt;
      }
      auto receipt_view = receipt_query_(transaction->getHash());
      transactions_result.push_back(std::make_shared<object::Transaction>(
          std::make_shared<Transaction>(TransactionReceiptReader{}, account_reader_, get_block_by_num_,
                                        std::move(transaction), transaction_view, receipt_view)));
    }
    return transactions_result;
  }
#endif
  for (const auto& trx_hash : dag_block_->getTrxs()) {
    if (!transaction_reader_.transaction_by_hash) {
      return std::nullopt;
    }
    auto transaction = transaction_reader_.transaction_by_hash(trx_hash);
    if (!transaction) {
      return std::nullopt;
    }
    transactions_result.push_back(std::make_shared<object::Transaction>(std::make_shared<Transaction>(
        TransactionReceiptReader{}, account_reader_, get_block_by_num_, std::move(transaction))));
  }

  return transactions_result;
}

}  // namespace graphql::taraxa
