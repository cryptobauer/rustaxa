#pragma once

#include <memory>
#include <optional>
#include <string>

#include "LogObject.h"
#include "final_chain/final_chain.hpp"
#include "graphql/account.hpp"
#include "graphql/transaction.hpp"
#include "transaction/transaction_manager.hpp"

namespace graphql::taraxa {

class Log {
 public:
  explicit Log(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
               std::shared_ptr<::taraxa::TransactionManager> trx_manager,
               std::shared_ptr<const Transaction> transaction, ::taraxa::LogEntry log, int index) noexcept;
  explicit Log(AccountStateReader account_reader, std::shared_ptr<const Transaction> transaction,
               ::taraxa::LogEntry log, int index) noexcept;

  int getIndex() const noexcept;
  std::shared_ptr<object::Account> getAccount(std::optional<response::Value>&& blockArg) const noexcept;
  std::vector<response::Value> getTopics() const noexcept;
  response::Value getData() const noexcept;
  std::shared_ptr<object::Transaction> getTransaction() const noexcept;

 private:
  AccountStateReader account_reader_;
  std::shared_ptr<const Transaction> kTransaction;
  const ::taraxa::LogEntry kLog;
  const int kIndex;
};

}  // namespace graphql::taraxa
