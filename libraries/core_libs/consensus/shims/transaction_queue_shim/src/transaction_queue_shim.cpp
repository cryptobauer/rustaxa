#include <cstring>
#include <stdexcept>
#include <utility>

#include "final_chain/final_chain.hpp"
#include "libdevcore/CommonData.h"
#include "transaction/transaction_manager.hpp"
#include "transaction/transaction_queue.hpp"

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
  input.tx_rlp = toBridgeBytes(transaction->rlp());
  input.proposable = proposable;
  input.last_block_number = last_block_number;

  const auto outcome = queue_->transaction_queue_insert(std::move(input));
  return toTransactionStatus(outcome.status);
}

bool TransactionQueue::erase(const SharedTransaction& transaction) {
  if (!transaction) {
    return false;
  }
  const auto hash = toBridgeHash(transaction->getHash());
  if (!queue_->transaction_queue_erase(hash)) {
    return false;
  }
  return true;
}

std::shared_ptr<Transaction> TransactionQueue::get(const trx_hash_t& hash) const {
  return materializeTransaction(queue_->transaction_queue_get_transaction(toBridgeHash(hash)));
}

std::vector<std::shared_ptr<Transaction>> TransactionQueue::getOrderedTransactions(uint64_t count) const {
  const auto queued_transactions = queue_->transaction_queue_ordered_transactions(count);
  SharedTransactions transactions;
  transactions.reserve(queued_transactions.size());
  for (const auto& queued_transaction : queued_transactions) {
    if (auto transaction = materializeTransaction(queued_transaction); transaction) {
      transactions.emplace_back(std::move(transaction));
    } else {
      throw std::runtime_error("Rust transaction queue returned a missing ordered transaction payload");
    }
  }
  return transactions;
}

std::vector<SharedTransactions> TransactionQueue::getAllTransactions() const {
  const auto groups = queue_->transaction_queue_all_transaction_groups();
  std::vector<SharedTransactions> transactions;
  transactions.reserve(groups.size());
  for (const auto& group : groups) {
    SharedTransactions group_transactions;
    group_transactions.reserve(group.transactions.size());
    for (const auto& queued_transaction : group.transactions) {
      if (auto transaction = materializeTransaction(queued_transaction); transaction) {
        group_transactions.emplace_back(std::move(transaction));
      } else {
        throw std::runtime_error("Rust transaction queue returned a missing grouped transaction payload");
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
  queue_->transaction_queue_block_finalized(block_number);
}

void TransactionQueue::purge() {
  if (!final_chain_) {
    if (size() == 0) {
      return;
    }
    throw std::runtime_error("TransactionQueue::purge requires FinalChain for non-empty Rust queue");
  }
  if (size() == 0) {
    return;
  }

  try {
    queue_->transaction_queue_purge_with_final_chain(final_chain_->rustFinalChainForRust());
  } catch (const std::exception& e) {
    throw std::runtime_error(std::string("TransactionQueue::purge failed in Rust FinalChain-backed route: ") +
                             e.what());
  }
}

void TransactionQueue::markTransactionKnown(const trx_hash_t& trx_hash) {
  queue_->transaction_queue_mark_transaction_known(toBridgeHash(trx_hash));
}

bool TransactionQueue::isTransactionKnown(const trx_hash_t& trx_hash) const {
  return queue_->transaction_queue_is_transaction_known(toBridgeHash(trx_hash));
}

bool TransactionQueue::transactionsDropped() const { return queue_->transaction_queue_transactions_dropped(); }

bool TransactionQueue::nonProposableTransactionsOverTheLimit() const {
  return queue_->transaction_queue_non_proposable_over_limit();
}

val_t TransactionQueue::getMinGasPriceForBlockInclusion(uint64_t limit) const {
  return fromBridgeU256(queue_->transaction_queue_min_gas_price_for_block_inclusion(limit));
}

std::shared_ptr<Transaction> TransactionQueue::materializeTransaction(
    const rustaxa::TransactionQueueStoredTransaction& stored) {
  if (!stored.found) {
    return nullptr;
  }
  auto transaction = std::make_shared<Transaction>(dev::bytes(stored.tx_rlp.begin(), stored.tx_rlp.end()));
  if (transaction->getHash() != fromBridgeHash(stored.hash)) {
    throw std::runtime_error("Rust transaction queue returned transaction RLP that does not match the queue hash");
  }
  return transaction;
}

rust::Vec<uint8_t> TransactionQueue::toBridgeBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
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
