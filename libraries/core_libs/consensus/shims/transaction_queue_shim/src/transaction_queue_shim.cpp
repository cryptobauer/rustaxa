#include "transaction/transaction_queue.hpp"

#include <cstring>
#include <stdexcept>
#include <utility>

#include "final_chain/final_chain.hpp"
#include "libdevcore/CommonData.h"
#include "transaction/transaction_manager.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kTransactionQueueStatusInserted = 0;
constexpr uint8_t kTransactionQueueStatusInsertedNonProposable = 1;
constexpr uint8_t kTransactionQueueStatusKnown = 2;
constexpr uint8_t kTransactionQueueStatusOverflow = 3;

TransactionStatus toTransactionStatus(uint8_t status) {
  switch (status) {
    case kTransactionQueueStatusInserted:
      return TransactionStatus::Inserted;
    case kTransactionQueueStatusInsertedNonProposable:
      return TransactionStatus::InsertedNonProposable;
    case kTransactionQueueStatusKnown:
      return TransactionStatus::Known;
    case kTransactionQueueStatusOverflow:
      return TransactionStatus::Overflow;
    default:
      throw std::runtime_error("Unknown Rust transaction queue status " + std::to_string(status));
  }
}

}  // namespace

TransactionQueue::TransactionQueue(std::shared_ptr<final_chain::FinalChain> final_chain, size_t max_size)
    : queue_(rustaxa::create_transaction_queue(rustaxa::TransactionQueueConfig{max_size})),
      known_txs_(max_size * 2, max_size / 5),
      final_chain_(std::move(final_chain)) {}

TransactionStatus TransactionQueue::insert(std::shared_ptr<Transaction>&& transaction, bool proposable,
                                           uint64_t last_block_number) {
  if (!transaction) {
    throw std::invalid_argument("TransactionQueue::insert requires a transaction");
  }

  const auto tx_hash = transaction->getHash();
  rustaxa::TransactionQueueInsertInput input;
  input.hash = toBridgeHash(tx_hash);
  input.sender = toBridgeAddress(transaction->getSender());
  input.nonce = toBridgeU256(transaction->getNonce());
  input.gas_price = toBridgeU256(transaction->getGasPrice());
  input.gas = transaction->getGas();
  input.data_size = transaction->getData().size();
  input.proposable = proposable;
  input.last_block_number = last_block_number;

  const auto outcome = queue_->transaction_queue_insert(std::move(input));
  const auto status = toTransactionStatus(outcome.status);
  if (status == TransactionStatus::Overflow) {
    transaction_overflow_time_ = std::chrono::system_clock::now();
  }

  forgetHashes(outcome.overflow_removed_hashes, true);
  if (outcome.inserted_hash_found && queue_->transaction_queue_contains(outcome.inserted_hash)) {
    storeTransaction(transaction);
  }

  if (status == TransactionStatus::Inserted || status == TransactionStatus::InsertedNonProposable) {
    known_txs_.insert(tx_hash);
  }
  return status;
}

bool TransactionQueue::erase(const SharedTransaction& transaction) {
  if (!transaction) {
    return false;
  }
  const auto hash = toBridgeHash(transaction->getHash());
  if (!queue_->transaction_queue_erase(hash)) {
    return false;
  }
  transactions_.erase(transaction->getHash());
  return true;
}

std::shared_ptr<Transaction> TransactionQueue::get(const trx_hash_t& hash) const {
  const auto it = transactions_.find(hash);
  if (it == transactions_.end()) {
    return nullptr;
  }
  return it->second;
}

std::vector<std::shared_ptr<Transaction>> TransactionQueue::getOrderedTransactions(uint64_t count) const {
  const auto hashes = queue_->transaction_queue_ordered_hashes(count);
  SharedTransactions transactions;
  transactions.reserve(hashes.size());
  for (const auto& hash : hashes) {
    if (auto transaction = get(fromBridgeHash(hash.hash)); transaction) {
      transactions.emplace_back(std::move(transaction));
    } else {
      throw std::runtime_error("Rust transaction queue returned a hash without a live C++ transaction");
    }
  }
  return transactions;
}

std::vector<SharedTransactions> TransactionQueue::getAllTransactions() const {
  const auto groups = queue_->transaction_queue_all_hash_groups();
  std::vector<SharedTransactions> transactions;
  transactions.reserve(groups.size());
  for (const auto& group : groups) {
    SharedTransactions group_transactions;
    group_transactions.reserve(group.hashes.size());
    for (const auto& hash : group.hashes) {
      if (auto transaction = get(fromBridgeHash(hash.hash)); transaction) {
        group_transactions.emplace_back(std::move(transaction));
      } else {
        throw std::runtime_error("Rust transaction queue returned a group hash without a live C++ transaction");
      }
    }
    transactions.emplace_back(std::move(group_transactions));
  }
  return transactions;
}

bool TransactionQueue::contains(const trx_hash_t& hash) const {
  const auto bridge_hash = toBridgeHash(hash);
  return queue_->transaction_queue_contains(bridge_hash);
}

size_t TransactionQueue::size() const { return queue_->transaction_queue_size(); }

void TransactionQueue::blockFinalized(uint64_t block_number) {
  const auto expired = queue_->transaction_queue_block_finalized(block_number);
  forgetHashes(expired, true);
}

void TransactionQueue::purge() {
  if (!final_chain_) {
    return;
  }

  const auto accounts = queue_->transaction_queue_proposable_accounts();
  for (const auto& account : accounts) {
    const auto address = addr_t(account.address.data(), addr_t::ConstructFromPointer);
    const auto account_state = final_chain_->getAccount(address);
    if (!account_state.has_value()) {
      continue;
    }
    const auto sender = toBridgeAddress(address);
    const auto nonce = toBridgeU256(account_state->nonce);
    const auto removed = queue_->transaction_queue_purge_account(sender, nonce);
    forgetHashes(removed, false);
  }
}

void TransactionQueue::markTransactionKnown(const trx_hash_t& trx_hash) { known_txs_.insert(trx_hash); }

bool TransactionQueue::isTransactionKnown(const trx_hash_t& trx_hash) const { return known_txs_.contains(trx_hash); }

bool TransactionQueue::nonProposableTransactionsOverTheLimit() const {
  return queue_->transaction_queue_non_proposable_over_limit();
}

val_t TransactionQueue::getMinGasPriceForBlockInclusion(uint64_t limit) const {
  return fromBridgeU256(queue_->transaction_queue_min_gas_price_for_block_inclusion(limit));
}

void TransactionQueue::forgetHashes(const rust::Vec<rustaxa::TransactionQueueHash>& hashes, bool erase_known) {
  for (const auto& hash : hashes) {
    const auto local_hash = fromBridgeHash(hash.hash);
    transactions_.erase(local_hash);
    if (erase_known) {
      known_txs_.erase(local_hash);
    }
  }
}

void TransactionQueue::storeTransaction(const SharedTransaction& transaction) {
  transactions_[transaction->getHash()] = transaction;
}

trx_hash_t TransactionQueue::fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return trx_hash_t(hash.data(), trx_hash_t::ConstructFromPointer);
}

std::array<uint8_t, 32> TransactionQueue::toBridgeHash(const trx_hash_t& hash) {
  std::array<uint8_t, 32> bytes{};
  std::memcpy(bytes.data(), hash.data(), bytes.size());
  return bytes;
}

std::array<uint8_t, 20> TransactionQueue::toBridgeAddress(const addr_t& address) {
  std::array<uint8_t, 20> bytes{};
  std::memcpy(bytes.data(), address.data(), bytes.size());
  return bytes;
}

std::array<uint8_t, 32> TransactionQueue::toBridgeU256(const val_t& value) {
  std::array<uint8_t, 32> out{};
  const auto bytes = dev::toBigEndian(value);
  if (bytes.size() > out.size()) {
    throw std::runtime_error("u256 value exceeds 32 bytes");
  }
  std::copy(bytes.begin(), bytes.end(), out.begin() + (out.size() - bytes.size()));
  return out;
}

val_t TransactionQueue::fromBridgeU256(const std::array<uint8_t, 32>& value) {
  return dev::fromBigEndian<val_t>(dev::bytes(value.begin(), value.end()));
}

}  // namespace taraxa
