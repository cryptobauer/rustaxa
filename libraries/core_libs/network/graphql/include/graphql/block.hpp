#pragma once

#include <functional>
#include <memory>
#include <string>

#include "BlockObject.h"
#include "final_chain/final_chain.hpp"
#include "graphql/account.hpp"
#include "transaction/transaction_manager.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace graphql::taraxa {

// BlockTransactionReader is GraphQL's minimal block-transaction boundary. It
// supplies finalized transaction count and transaction vectors for one block
// number without exposing FinalChain or storage objects to field resolvers.
struct BlockTransactionReader {
  std::function<uint64_t(::taraxa::EthBlockNumber)> transaction_count;
  std::function<std::vector<std::shared_ptr<::taraxa::Transaction>>(::taraxa::EthBlockNumber)> transactions;
};

class Block {
 public:
  explicit Block(
      std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
      std::shared_ptr<::taraxa::TransactionManager> trx_manager,
      std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
      const ::taraxa::blk_hash_t& pbft_block_hash,
      std::shared_ptr<const ::taraxa::final_chain::BlockHeader> block_header
#ifdef RUSTAXA_ENABLE
      ,
      std::function<uint64_t(::taraxa::EthBlockNumber)> transaction_count_query = {},
      std::function<rustaxa::TransactionPublicView(::taraxa::EthBlockNumber, uint64_t)> transaction_query = {},
      std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> receipt_query = {}
#endif
      ) noexcept;
  explicit Block(AccountStateReader account_reader, std::shared_ptr<::taraxa::TransactionManager> trx_manager,
                 std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
                 const ::taraxa::blk_hash_t& pbft_block_hash,
                 std::shared_ptr<const ::taraxa::final_chain::BlockHeader> block_header) noexcept;
  explicit Block(
      AccountStateReader account_reader, BlockTransactionReader transaction_reader,
      std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
      const ::taraxa::blk_hash_t& pbft_block_hash,
      std::shared_ptr<const ::taraxa::final_chain::BlockHeader> block_header
#ifdef RUSTAXA_ENABLE
      ,
      std::function<uint64_t(::taraxa::EthBlockNumber)> transaction_count_query = {},
      std::function<rustaxa::TransactionPublicView(::taraxa::EthBlockNumber, uint64_t)> transaction_query = {},
      std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> receipt_query = {}
#endif
      ) noexcept;

  response::Value getNumber() const noexcept;
  response::Value getHash() const noexcept;
  response::Value getPbftHash() const noexcept;
  std::shared_ptr<object::Block> getParent() const noexcept;
  response::Value getNonce() const noexcept;
  response::Value getTransactionsRoot() const noexcept;
  std::optional<int> getTransactionCount() const noexcept;
  response::Value getStateRoot() const noexcept;
  response::Value getReceiptsRoot() const noexcept;
  std::shared_ptr<object::Account> getMiner(std::optional<response::Value>&& blockArg) const;
  response::Value getExtraData() const noexcept;
  response::Value getGasLimit() const noexcept;
  response::Value getGasUsed() const noexcept;
  response::Value getTimestamp() const noexcept;
  response::Value getLogsBloom() const noexcept;
  response::Value getMixHash() const noexcept;
  response::Value getDifficulty() const noexcept;
  response::Value getTotalDifficulty() const noexcept;
  std::optional<int> getOmmerCount() const noexcept;
  std::optional<std::vector<std::shared_ptr<object::Block>>> getOmmers() const noexcept;
  std::shared_ptr<object::Block> getOmmerAt(int&& indexArg) const noexcept;
  response::Value getOmmerHash() const noexcept;
  std::optional<std::vector<std::shared_ptr<object::Transaction>>> getTransactions() const noexcept;
  std::shared_ptr<object::Transaction> getTransactionAt(response::IntType&& indexArg) const noexcept;
  std::vector<std::shared_ptr<object::Log>> getLogs(BlockFilterCriteria&& filterArg) const noexcept;
  std::shared_ptr<object::Account> getAccount(response::Value&& addressArg) const;
  std::shared_ptr<object::CallResult> getCall(CallData&& dataArg) const noexcept;
  response::Value getEstimateGas(CallData&& dataArg) const noexcept;

 private:
  std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num_;
  AccountStateReader account_reader_;
  BlockTransactionReader transaction_reader_;
  const ::taraxa::blk_hash_t kPBftBlockHash;
  std::shared_ptr<const ::taraxa::final_chain::BlockHeader> block_header_;
  mutable std::vector<std::shared_ptr<::taraxa::Transaction>> transactions_;
#ifdef RUSTAXA_ENABLE
  std::function<uint64_t(::taraxa::EthBlockNumber)> transaction_count_query_;
  std::function<rustaxa::TransactionPublicView(::taraxa::EthBlockNumber, uint64_t)> transaction_query_;
  std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> receipt_query_;
#endif
};

}  // namespace graphql::taraxa
