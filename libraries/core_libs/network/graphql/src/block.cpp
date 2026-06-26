#include "graphql/block.hpp"

#include <cstddef>
#include <optional>

#include "graphql/transaction.hpp"
#include "libdevcore/CommonJS.h"
#include "transaction/system_transaction.hpp"

using namespace std::literals;

namespace graphql::taraxa {

namespace {
BlockTransactionReader makeBlockTransactionReader(
    const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain) {
  BlockTransactionReader reader;
  reader.transaction_count = [final_chain](::taraxa::EthBlockNumber block_number) {
    return final_chain ? final_chain->transactionCount(block_number) : 0;
  };
  reader.transactions = [final_chain](::taraxa::EthBlockNumber block_number) {
    if (!final_chain) {
      return std::vector<std::shared_ptr<::taraxa::Transaction>>{};
    }
    return final_chain->transactions(block_number);
  };
  return reader;
}
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

Block::Block(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
             std::shared_ptr<::taraxa::TransactionManager> trx_manager,
             std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
             const ::taraxa::blk_hash_t& pbft_block_hash,
             std::shared_ptr<const ::taraxa::final_chain::BlockHeader> block_header
#ifdef RUSTAXA_ENABLE
             ,
             std::function<uint64_t(::taraxa::EthBlockNumber)> transaction_count_query,
             std::function<rustaxa::TransactionPublicView(::taraxa::EthBlockNumber, uint64_t)> transaction_query,
             std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> receipt_query
#endif
             ) noexcept
    : get_block_by_num_(std::move(get_block_by_num)),
      account_reader_(makeAccountStateReader(final_chain)),
      transaction_reader_(makeBlockTransactionReader(final_chain)),
      kPBftBlockHash(pbft_block_hash),
      block_header_(std::move(block_header))
#ifdef RUSTAXA_ENABLE
      ,
      transaction_count_query_(std::move(transaction_count_query)),
      transaction_query_(std::move(transaction_query)),
      receipt_query_(std::move(receipt_query))
#endif
{
  (void)trx_manager;
}

Block::Block(AccountStateReader account_reader, std::shared_ptr<::taraxa::TransactionManager> trx_manager,
             std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
             const ::taraxa::blk_hash_t& pbft_block_hash,
             std::shared_ptr<const ::taraxa::final_chain::BlockHeader> block_header) noexcept
    : Block(std::move(account_reader), BlockTransactionReader{}, std::move(get_block_by_num), pbft_block_hash,
            std::move(block_header)) {
  (void)trx_manager;
}

Block::Block(AccountStateReader account_reader, BlockTransactionReader transaction_reader,
             std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
             const ::taraxa::blk_hash_t& pbft_block_hash,
             std::shared_ptr<const ::taraxa::final_chain::BlockHeader> block_header
#ifdef RUSTAXA_ENABLE
             ,
             std::function<uint64_t(::taraxa::EthBlockNumber)> transaction_count_query,
             std::function<rustaxa::TransactionPublicView(::taraxa::EthBlockNumber, uint64_t)> transaction_query,
             std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> receipt_query
#endif
             ) noexcept
    : get_block_by_num_(std::move(get_block_by_num)),
      account_reader_(std::move(account_reader)),
      transaction_reader_(std::move(transaction_reader)),
      kPBftBlockHash(pbft_block_hash),
      block_header_(std::move(block_header))
#ifdef RUSTAXA_ENABLE
      ,
      transaction_count_query_(std::move(transaction_count_query)),
      transaction_query_(std::move(transaction_query)),
      receipt_query_(std::move(receipt_query))
#endif
{
}

response::Value Block::getNumber() const noexcept { return response::Value(static_cast<int>(block_header_->number)); }

response::Value Block::getHash() const noexcept { return response::Value(block_header_->hash.toString()); }

response::Value Block::getPbftHash() const noexcept { return response::Value(kPBftBlockHash.toString()); }

std::shared_ptr<object::Block> Block::getParent() const noexcept {
  return get_block_by_num_(block_header_->number - 1);
}

response::Value Block::getNonce() const noexcept { return response::Value(block_header_->nonce().toString()); }

response::Value Block::getTransactionsRoot() const noexcept {
  return response::Value(block_header_->transactions_root.toString());
}

std::optional<int> Block::getTransactionCount() const noexcept {
#ifdef RUSTAXA_ENABLE
  if (transaction_count_query_) {
    return std::optional<int>(static_cast<int>(transaction_count_query_(block_header_->number)));
  }
#endif
  if (transaction_reader_.transaction_count) {
    return std::optional<int>(static_cast<int>(transaction_reader_.transaction_count(block_header_->number)));
  }
  if (!transactions_.size()) {
    return 0;
  } else {
    return std::optional<int>(transactions_.size());
  }
}

response::Value Block::getStateRoot() const noexcept { return response::Value(block_header_->state_root.toString()); }

response::Value Block::getReceiptsRoot() const noexcept {
  return response::Value(block_header_->receipts_root.toString());
}

std::shared_ptr<object::Account> Block::getMiner(std::optional<response::Value>&& blockArg) const {
  if (blockArg) {
    return std::make_shared<object::Account>(
        std::make_shared<Account>(account_reader_, block_header_->author, blockArg->get<int>()));
  } else {
    return std::make_shared<object::Account>(std::make_shared<Account>(account_reader_, block_header_->author));
  }
}

response::Value Block::getExtraData() const noexcept { return response::Value(dev::toHex(block_header_->extra_data)); }

response::Value Block::getGasLimit() const noexcept {
  return response::Value(static_cast<int>(block_header_->gas_limit));
}

response::Value Block::getGasUsed() const noexcept {
  return response::Value(static_cast<int>(block_header_->gas_used));
}

response::Value Block::getTimestamp() const noexcept {
  return response::Value(static_cast<int>(block_header_->timestamp));
}

response::Value Block::getLogsBloom() const noexcept { return response::Value(block_header_->log_bloom.toString()); }

response::Value Block::getMixHash() const noexcept { return response::Value(block_header_->mixHash().toString()); }

response::Value Block::getDifficulty() const noexcept { return response::Value(block_header_->difficulty().str()); }

response::Value Block::getTotalDifficulty() const noexcept {
  return response::Value(block_header_->difficulty().str());
}

std::optional<int> Block::getOmmerCount() const noexcept { return {}; }

std::optional<std::vector<std::shared_ptr<object::Block>>> Block::getOmmers() const noexcept { return std::nullopt; }

std::shared_ptr<object::Block> Block::getOmmerAt(int&&) const noexcept { return nullptr; }

response::Value Block::getOmmerHash() const noexcept { return response::Value(block_header_->unclesHash().toString()); }

std::optional<std::vector<std::shared_ptr<object::Transaction>>> Block::getTransactions() const noexcept {
#ifdef RUSTAXA_ENABLE
  if (transaction_count_query_ && transaction_query_ && receipt_query_) {
    const auto transaction_count = transaction_count_query_(block_header_->number);
    if (transaction_count == 0) {
      return std::nullopt;
    }

    std::vector<std::shared_ptr<object::Transaction>> ret;
    ret.reserve(static_cast<size_t>(transaction_count));
    for (uint64_t index = 0; index < transaction_count; ++index) {
      auto transaction_view = transaction_query_(block_header_->number, index);
      auto transaction = materializeTransactionView(transaction_view);
      if (!transaction) {
        return std::nullopt;
      }
      auto receipt_view = receipt_query_(transaction->getHash());
      ret.emplace_back(std::make_shared<object::Transaction>(
          std::make_shared<Transaction>(TransactionReceiptReader{}, account_reader_, get_block_by_num_,
                                        std::move(transaction), transaction_view, receipt_view)));
    }
    return ret;
  }
#endif

  std::vector<std::shared_ptr<object::Transaction>> ret;
  if (!transactions_.size()) {
    if (transaction_reader_.transactions) {
      transactions_ = transaction_reader_.transactions(block_header_->number);
    }
    if (!transactions_.size()) return std::nullopt;
  }
  ret.reserve(transactions_.size());
  for (auto& t : transactions_) {
    ret.emplace_back(std::make_shared<object::Transaction>(
        std::make_shared<Transaction>(TransactionReceiptReader{}, account_reader_, get_block_by_num_, t)));
  }
  return ret;
}

std::shared_ptr<object::Transaction> Block::getTransactionAt(response::IntType&& index) const noexcept {
#ifdef RUSTAXA_ENABLE
  if (transaction_query_ && receipt_query_) {
    if (index < 0) {
      return nullptr;
    }

    auto transaction_view = transaction_query_(block_header_->number, static_cast<uint64_t>(index));
    auto transaction = materializeTransactionView(transaction_view);
    if (!transaction) {
      return nullptr;
    }
    auto receipt_view = receipt_query_(transaction->getHash());
    return std::make_shared<object::Transaction>(
        std::make_shared<Transaction>(TransactionReceiptReader{}, account_reader_, get_block_by_num_,
                                      std::move(transaction), transaction_view, receipt_view));
  }
#endif

  if (!transactions_.size()) {
    if (transaction_reader_.transactions) {
      transactions_ = transaction_reader_.transactions(block_header_->number);
    }
    if (!transactions_.size()) return nullptr;
  }
  if (transactions_.size() <= static_cast<size_t>(index)) {
    return nullptr;
  }
  return std::make_shared<object::Transaction>(std::make_shared<Transaction>(
      TransactionReceiptReader{}, account_reader_, get_block_by_num_, transactions_[index]));
}

std::vector<std::shared_ptr<object::Log>> Block::getLogs(BlockFilterCriteria&&) const noexcept {
  std::vector<std::shared_ptr<object::Log>> ret;
  return ret;
}

std::shared_ptr<object::Account> Block::getAccount(response::Value&& addressArg) const {
  return std::make_shared<object::Account>(std::make_shared<Account>(
      account_reader_, ::taraxa::addr_t(addressArg.get<std::string>()), block_header_->number));
}

std::shared_ptr<object::CallResult> Block::getCall(CallData&&) const noexcept { return nullptr; }

response::Value Block::getEstimateGas(CallData&&) const noexcept { return response::Value(0); }

}  // namespace graphql::taraxa
