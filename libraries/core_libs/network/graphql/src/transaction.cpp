#include "graphql/transaction.hpp"

#include <optional>

#include "common/encoding_rlp.hpp"
#include "graphql/log.hpp"
#include "libdevcore/CommonJS.h"

using namespace std::literals;

namespace graphql::taraxa {
namespace {
TransactionReceiptReader makeTransactionReceiptReader(
    const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain) {
  TransactionReceiptReader reader;
  reader.location = [final_chain](const ::taraxa::trx_hash_t& hash) {
    if (!final_chain) {
      return std::optional<::taraxa::TransactionLocation>{};
    }
    return final_chain->transactionLocation(hash);
  };
  reader.receipt = [final_chain](::taraxa::EthBlockNumber period, uint32_t position, const ::taraxa::trx_hash_t& hash) {
    if (!final_chain) {
      return std::optional<::taraxa::TransactionReceipt>{};
    }
    return final_chain->transactionReceipt(period, position, hash);
  };
  return reader;
}
}  // namespace

#ifdef RUSTAXA_ENABLE
namespace {
dev::bytes bytesFromBridge(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }
}  // namespace
#endif

Transaction::Transaction(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                         std::shared_ptr<::taraxa::TransactionManager> trx_manager,
                         std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
                         std::shared_ptr<::taraxa::Transaction> transaction) noexcept
    : get_block_by_num_(std::move(get_block_by_num)),
      transaction_(std::move(transaction)),
      account_reader_(makeAccountStateReader(final_chain)),
      receipt_reader_(makeTransactionReceiptReader(final_chain)) {
  (void)trx_manager;
  if (receipt_reader_.location) {
    if (auto location = receipt_reader_.location(transaction_->getHash())) {
      location_ = *location;
    }
  }
}

Transaction::Transaction(TransactionReceiptReader receipt_reader,
                         std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
                         std::shared_ptr<::taraxa::Transaction> transaction) noexcept
    : Transaction(std::move(receipt_reader), AccountStateReader{}, std::move(get_block_by_num),
                  std::move(transaction)) {}

Transaction::Transaction(TransactionReceiptReader receipt_reader, AccountStateReader account_reader,
                         std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
                         std::shared_ptr<::taraxa::Transaction> transaction) noexcept
    : get_block_by_num_(std::move(get_block_by_num)),
      transaction_(std::move(transaction)),
      account_reader_(std::move(account_reader)),
      receipt_reader_(std::move(receipt_reader)) {
  if (receipt_reader_.location) {
    if (auto location = receipt_reader_.location(transaction_->getHash())) {
      location_ = *location;
    }
  }
}

#ifdef RUSTAXA_ENABLE
Transaction::Transaction(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                         std::shared_ptr<::taraxa::TransactionManager> trx_manager,
                         std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
                         std::shared_ptr<::taraxa::Transaction> transaction,
                         const rustaxa::TransactionPublicView& transaction_view,
                         const rustaxa::TransactionReceiptPublicView& receipt_view) noexcept
    : get_block_by_num_(std::move(get_block_by_num)),
      transaction_(std::move(transaction)),
      account_reader_(makeAccountStateReader(final_chain)),
      receipt_reader_(makeTransactionReceiptReader(final_chain)),
      location_{transaction_view.block_number, transaction_view.transaction_index, transaction_view.is_system},
      receipt_lookup_complete_(true) {
  (void)trx_manager;
  if (receipt_view.found) {
    auto receipt_bytes = bytesFromBridge(receipt_view.receipt_rlp);
    receipt_ = ::taraxa::util::rlp_dec<::taraxa::TransactionReceipt>(dev::RLP(receipt_bytes));
  }
}

Transaction::Transaction(TransactionReceiptReader receipt_reader, AccountStateReader account_reader,
                         std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
                         std::shared_ptr<::taraxa::Transaction> transaction,
                         const rustaxa::TransactionPublicView& transaction_view,
                         const rustaxa::TransactionReceiptPublicView& receipt_view) noexcept
    : get_block_by_num_(std::move(get_block_by_num)),
      transaction_(std::move(transaction)),
      account_reader_(std::move(account_reader)),
      receipt_reader_(std::move(receipt_reader)),
      location_{transaction_view.block_number, transaction_view.transaction_index, transaction_view.is_system},
      receipt_lookup_complete_(true) {
  if (receipt_view.found) {
    auto receipt_bytes = bytesFromBridge(receipt_view.receipt_rlp);
    receipt_ = ::taraxa::util::rlp_dec<::taraxa::TransactionReceipt>(dev::RLP(receipt_bytes));
  }
}
#endif

bool Transaction::ensureReceipt() const noexcept {
  if (!receipt_ && !receipt_lookup_complete_) {
    if (receipt_reader_.receipt) {
      receipt_ = receipt_reader_.receipt(location_.period, location_.position, transaction_->getHash());
    }
    receipt_lookup_complete_ = true;
  }
  return receipt_.has_value();
}

response::Value Transaction::getHash() const noexcept { return response::Value(transaction_->getHash().toString()); }

response::Value Transaction::getNonce() const noexcept { return response::Value(transaction_->getNonce().str()); }

std::optional<int> Transaction::getIndex() const noexcept { return {location_.position}; }

std::shared_ptr<object::Account> Transaction::getFrom(std::optional<response::Value>&&) const {
  return std::make_shared<object::Account>(
      std::make_shared<Account>(account_reader_, transaction_->getSender(), location_.period));
}

std::shared_ptr<object::Account> Transaction::getTo(std::optional<response::Value>&&) const {
  if (!transaction_->getReceiver()) return nullptr;
  return std::make_shared<object::Account>(
      std::make_shared<Account>(account_reader_, *transaction_->getReceiver(), location_.period));
}

response::Value Transaction::getValue() const noexcept { return response::Value(transaction_->getValue().str()); }

response::Value Transaction::getGasPrice() const noexcept { return response::Value(transaction_->getGasPrice().str()); }

response::Value Transaction::getGas() const noexcept {
  return response::Value(static_cast<int>(transaction_->getGas()));
}

response::Value Transaction::getInputData() const noexcept {
  return response::Value(dev::toJS(transaction_->getData()));
}

std::shared_ptr<object::Block> Transaction::getBlock() const { return get_block_by_num_(location_.period); }

std::optional<response::Value> Transaction::getStatus() const noexcept {
  if (!ensureReceipt()) return std::nullopt;
  return response::Value(static_cast<int>(receipt_->status_code));
}

std::optional<response::Value> Transaction::getGasUsed() const noexcept {
  if (!ensureReceipt()) return std::nullopt;
  return response::Value(static_cast<int>(receipt_->gas_used));
}

std::optional<response::Value> Transaction::getCumulativeGasUsed() const noexcept {
  if (!ensureReceipt()) return std::nullopt;
  return response::Value(static_cast<int>(receipt_->cumulative_gas_used));
}

std::shared_ptr<object::Account> Transaction::getCreatedContract(std::optional<response::Value>&&) const noexcept {
  if (!ensureReceipt()) return nullptr;
  if (!receipt_->new_contract_address) return nullptr;
  return std::make_shared<object::Account>(std::make_shared<Account>(account_reader_, *receipt_->new_contract_address));
}

std::optional<std::vector<std::shared_ptr<object::Log>>> Transaction::getLogs() const noexcept {
  std::vector<std::shared_ptr<object::Log>> logs;
  if (!ensureReceipt()) return std::nullopt;

  for (int i = 0; i < static_cast<int>(receipt_->logs.size()); ++i) {
    logs.push_back(std::make_shared<object::Log>(
        std::make_shared<Log>(account_reader_, shared_from_this(), receipt_->logs[i], i)));
  }

  return logs;
}

response::Value Transaction::getR() const noexcept {
  return response::Value(dev::toJS(dev::u256(transaction_->getVRS().r)));
}

response::Value Transaction::getS() const noexcept {
  return response::Value(dev::toJS(dev::u256(transaction_->getVRS().s)));
}

response::Value Transaction::getV() const noexcept { return response::Value(dev::toJS(transaction_->getVRS().v)); }

}  // namespace graphql::taraxa
