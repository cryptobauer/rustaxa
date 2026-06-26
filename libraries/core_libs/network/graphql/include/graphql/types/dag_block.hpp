#pragma once

#include <functional>
#include <mutex>
#include <optional>

#include "DagBlockObject.h"
#include "final_chain/final_chain.hpp"
#include "pbft/pbft_manager.hpp"
#include "transaction/transaction_manager.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace graphql::taraxa {

class DagBlock {
 public:
  explicit DagBlock(std::shared_ptr<::taraxa::DagBlock> dag_block,
                    std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                    std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                    std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                    std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num) noexcept;
#ifdef RUSTAXA_ENABLE
  explicit DagBlock(
      rustaxa::DagBlockPublicView dag_block, std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
      std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
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
  std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain_;
  std::shared_ptr<::taraxa::PbftManager> pbft_manager_;
  std::shared_ptr<::taraxa::TransactionManager> transaction_manager_;
  std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num_;
#ifdef RUSTAXA_ENABLE
  std::function<rustaxa::TransactionPublicView(const ::taraxa::trx_hash_t&)> transaction_query_;
  std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> receipt_query_;
#endif

  mutable std::mutex mu_;
  mutable std::optional<uint64_t> period_;
};

}  // namespace graphql::taraxa
