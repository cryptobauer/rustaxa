#pragma once

#include <functional>
#include <mutex>
#include <optional>

#include "DagBlockObject.h"
#include "final_chain/final_chain.hpp"
#include "graphql/account.hpp"
#ifndef RUSTAXA_ENABLE
#include "pbft/pbft_manager.hpp"
#endif
#include "transaction/transaction_manager.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace graphql::taraxa {

// DagBlockTransactionReader is GraphQL's minimal DAG transaction boundary. It
// resolves DAG transaction hashes to transaction payloads without exposing
// TransactionManager to the DAG block field resolver.
struct DagBlockTransactionReader {
  std::function<std::shared_ptr<::taraxa::Transaction>(const ::taraxa::trx_hash_t&)> transaction_by_hash;
};

// DagBlockPeriodReader is GraphQL's minimal DAG finalization boundary. It
// resolves a DAG block hash to its finalized PBFT period without exposing
// PbftManager to DAG block field resolvers.
struct DagBlockPeriodReader {
  std::function<std::optional<uint64_t>(const ::taraxa::blk_hash_t&)> period_by_hash;
};

class DagBlock {
 public:
#ifndef RUSTAXA_ENABLE
  explicit DagBlock(std::shared_ptr<::taraxa::DagBlock> dag_block,
                    std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                    std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                    std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                    std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept;
  explicit DagBlock(AccountStateReader account_reader, std::shared_ptr<::taraxa::DagBlock> dag_block,
                    std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                    std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                    std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept;
  explicit DagBlock(AccountStateReader account_reader, DagBlockTransactionReader transaction_reader,
                    std::shared_ptr<::taraxa::DagBlock> dag_block, std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                    std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                    std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept;
#endif
  explicit DagBlock(AccountStateReader account_reader, DagBlockTransactionReader transaction_reader,
                    DagBlockPeriodReader period_reader, std::shared_ptr<::taraxa::DagBlock> dag_block,
                    std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept;
#ifdef RUSTAXA_ENABLE
  explicit DagBlock(
      rustaxa::DagBlockPublicView dag_block, AccountStateReader account_reader,
      std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num,
      std::function<rustaxa::TransactionPublicView(const ::taraxa::trx_hash_t&)> transaction_query,
      std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> receipt_query) noexcept;
#endif

  response::Value getHash() const noexcept;
  response::Value getPivot() const noexcept;
  std::vector<response::Value> getTips() const noexcept;
  response::Value getLevel() const noexcept;
  std::optional<response::Value> getPbftPeriod() const noexcept;
  std::shared_ptr<object::Account> getAuthor() const noexcept;
  response::Value getTimestamp() const noexcept;
  response::Value getSignature() const noexcept;
  int getVdf() const noexcept;
  int getTransactionCount() const noexcept;
  std::optional<std::vector<std::shared_ptr<object::Transaction>>> getTransactions() const noexcept;

 private:
  std::shared_ptr<::taraxa::DagBlock> dag_block_;
#ifdef RUSTAXA_ENABLE
  std::optional<rustaxa::DagBlockPublicView> rust_dag_block_;
#endif
  AccountStateReader account_reader_;
  DagBlockTransactionReader transaction_reader_;
  DagBlockPeriodReader period_reader_;
  std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num_;
#ifdef RUSTAXA_ENABLE
  std::function<rustaxa::TransactionPublicView(const ::taraxa::trx_hash_t&)> transaction_query_;
  std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> receipt_query_;
#endif

  mutable std::mutex mu_;
  mutable std::optional<uint64_t> period_;
};

}  // namespace graphql::taraxa
