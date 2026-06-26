#pragma once

#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <vector>

#include "TransactionObject.h"
#include "final_chain/final_chain.hpp"
#include "graphql/account.hpp"
#include "transaction/receipt.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace graphql::taraxa {

/**
 * Read-only transaction finalization facts used by GraphQL transaction field resolvers.
 *
 * The reader supplies finalized transaction location and receipt data for one public
 * transaction object. Implementations may adapt legacy `FinalChain` temporarily or a
 * Rust query facade directly. Missing callbacks or missing rows are treated as absent
 * data so field resolvers can return GraphQL nulls instead of driving consensus work.
 */
struct TransactionReceiptReader {
  std::function<std::optional<::taraxa::TransactionLocation>(const ::taraxa::trx_hash_t&)> location;
  std::function<std::optional<::taraxa::TransactionReceipt>(::taraxa::EthBlockNumber, uint32_t,
                                                            const ::taraxa::trx_hash_t&)>
      receipt;
};

class Transaction final : public std::enable_shared_from_this<Transaction> {
 public:
  explicit Transaction(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                       std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)>,
                       std::shared_ptr<::taraxa::Transaction> transaction) noexcept;
  explicit Transaction(TransactionReceiptReader receipt_reader,
                       std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)>,
                       std::shared_ptr<::taraxa::Transaction> transaction) noexcept;
  explicit Transaction(TransactionReceiptReader receipt_reader, AccountStateReader account_reader,
                       std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)>,
                       std::shared_ptr<::taraxa::Transaction> transaction) noexcept;
#ifdef RUSTAXA_ENABLE
  explicit Transaction(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                       std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)>,
                       std::shared_ptr<::taraxa::Transaction> transaction,
                       const rustaxa::TransactionPublicView& transaction_view,
                       const rustaxa::TransactionReceiptPublicView& receipt_view) noexcept;
  explicit Transaction(TransactionReceiptReader receipt_reader, AccountStateReader account_reader,
                       std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)>,
                       std::shared_ptr<::taraxa::Transaction> transaction,
                       const rustaxa::TransactionPublicView& transaction_view,
                       const rustaxa::TransactionReceiptPublicView& receipt_view) noexcept;
#endif

  response::Value getHash() const noexcept;
  response::Value getNonce() const noexcept;
  std::optional<int> getIndex() const noexcept;
  std::shared_ptr<object::Account> getFrom(std::optional<response::Value>&& blockArg) const;
  std::shared_ptr<object::Account> getTo(std::optional<response::Value>&& blockArg) const;
  response::Value getValue() const noexcept;
  response::Value getGasPrice() const noexcept;
  response::Value getGas() const noexcept;
  response::Value getInputData() const noexcept;
  std::shared_ptr<object::Block> getBlock() const;
  std::optional<response::Value> getStatus() const noexcept;
  std::optional<response::Value> getGasUsed() const noexcept;
  std::optional<response::Value> getCumulativeGasUsed() const noexcept;
  std::shared_ptr<object::Account> getCreatedContract(std::optional<response::Value>&& blockArg) const noexcept;
  std::optional<std::vector<std::shared_ptr<object::Log>>> getLogs() const noexcept;
  response::Value getR() const noexcept;
  response::Value getS() const noexcept;
  response::Value getV() const noexcept;

 private:
  std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num_;
  std::shared_ptr<::taraxa::Transaction> transaction_;
  AccountStateReader account_reader_;
  TransactionReceiptReader receipt_reader_;
  // Caching for performance
  mutable std::optional<::taraxa::TransactionReceipt> receipt_;
  ::taraxa::TransactionLocation location_;
  mutable bool receipt_lookup_complete_ = false;

  bool ensureReceipt() const noexcept;
};

}  // namespace graphql::taraxa
