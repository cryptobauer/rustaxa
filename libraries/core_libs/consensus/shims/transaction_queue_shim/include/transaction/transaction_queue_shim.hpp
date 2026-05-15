#pragma once

#include <memory>

#include "common/constants.hpp"
#include "common/util.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"

namespace taraxa {

/** @addtogroup Transaction
 * @{
 */

enum class TransactionStatus;
namespace final_chain {
class FinalChain;
}

/**
 * Rust-mode transaction queue facade.
 *
 * The class preserves the public `TransactionQueue` API while moving deterministic queue metadata and queued
 * transaction payload bytes into Rust. C++ materializes `Transaction` objects from Rust-retained canonical RLP for API
 * callers and keeps FinalChain account lookups used by purge. Rust owns proposer/non-proposer indexes, per-account
 * nonce ordering, replacement, expiry planning, pool limits, gas-price aggregates, queued payload retention, the local
 * known-transaction expiration cache, and overflow/drop observation state.
 *
 * Edge behavior:
 * - insert status values mirror `TransactionStatus`
 * - overflow/drop observations update `transactionsDropped()` through Rust-owned bridge state
 * - Rust errors at the metadata boundary are surfaced as local exceptions instead of falling back to legacy C++
 */
class TransactionQueue {
 public:
  TransactionQueue(std::shared_ptr<final_chain::FinalChain> final_chain, size_t max_size = kMinTransactionPoolSize);
  TransactionQueue(const TransactionQueue&) = delete;
  TransactionQueue(TransactionQueue&&) = delete;
  TransactionQueue& operator=(const TransactionQueue&) = delete;
  TransactionQueue& operator=(TransactionQueue&&) = delete;

  /**
   * Inserts a verified transaction into proposer or non-proposer queue state.
   */
  TransactionStatus insert(std::shared_ptr<Transaction>&& transaction, bool proposable, uint64_t last_block_number = 0);

  /**
   * Removes a transaction from proposer or non-proposer queue state.
   */
  bool erase(const SharedTransaction& transaction);

  /**
   * Moves a queued transaction from proposer ordering to non-proposer state.
   */
  bool demoteToNonProposable(const trx_hash_t& hash, uint64_t last_block_number);

  /**
   * Materializes and returns the queued transaction for `hash`, or null when absent.
   */
  std::shared_ptr<Transaction> get(const trx_hash_t& hash) const;

  /**
   * Returns up to `count` live transactions in Rust-determined proposal order.
   */
  std::vector<std::shared_ptr<Transaction>> getOrderedTransactions(uint64_t count) const;

  /**
   * Returns live proposable transactions grouped per account and ordered by nonce within each account.
   */
  std::vector<SharedTransactions> getAllTransactions() const;

  /**
   * Returns true when the hash is known to proposer or non-proposer queue state.
   */
  bool contains(const trx_hash_t& hash) const;

  /**
   * Returns the number of proposable transactions.
   */
  size_t size() const;

  /**
   * Expires non-proposable transactions after finalization advances.
   */
  void blockFinalized(uint64_t block_number);

  /**
   * Removes proposer transactions whose nonce is below finalized account nonce.
   */
  void purge();

  /**
   * Marks a transaction hash as known to the Rust-owned local expiration cache.
   */
  void markTransactionKnown(const trx_hash_t& trx_hash);

  /**
   * Returns true when the transaction hash is known to the Rust-owned local expiration cache.
   */
  bool isTransactionKnown(const trx_hash_t& trx_hash) const;

  /**
   * Returns true for a short time after Rust observes queue overflow drops or rejects transactions.
   */
  bool transactionsDropped() const;

  /**
   * Returns true when non-proposable queue state reached its configured limit.
   */
  bool nonProposableTransactionsOverTheLimit() const;

  /**
   * Returns the minimum gas price required for next-block inclusion under `limit`.
   */
  val_t getMinGasPriceForBlockInclusion(uint64_t limit) const;

 private:
  /**
   * Materializes a legacy transaction object from Rust-owned queued bytes.
   */
  static std::shared_ptr<Transaction> materializeTransaction(const rustaxa::TransactionQueueStoredTransaction& stored);

  /**
   * Converts a byte vector into the bridge representation.
   */
  static rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& bytes);

  /**
   * Converts a Rust hash handle into the local hash type.
   */
  static trx_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash);

  /**
   * Converts a local hash into the bridge fixed-byte representation.
   */
  static std::array<uint8_t, 32> toBridgeHash(const trx_hash_t& hash);

  /**
   * Converts an address into the bridge fixed-byte representation.
   */
  static std::array<uint8_t, 20> toBridgeAddress(const addr_t& address);

  /**
   * Converts a `u256` value into fixed-width big-endian bytes.
   */
  static std::array<uint8_t, 32> toBridgeU256(const val_t& value);

  /**
   * Converts fixed-width big-endian bytes into a `u256` value.
   */
  static val_t fromBridgeU256(const std::array<uint8_t, 32>& value);

  /**
   * Collects finalized nonce facts for currently proposable accounts.
   */
  rust::Vec<rustaxa::TransactionQueueAccountNonceFact> collectPurgeAccountFacts() const;

 private:
  ::rust::Box<rustaxa::BridgeTransactionQueue> queue_;
  std::shared_ptr<final_chain::FinalChain> final_chain_;
};

/** @}*/

}  // namespace taraxa
