#include "graphql/log.hpp"

#include <optional>

#include "libdevcore/CommonJS.h"

using namespace std::literals;

namespace graphql::taraxa {

#ifndef RUSTAXA_ENABLE
Log::Log(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain, std::shared_ptr<const Transaction> transaction,
         ::taraxa::LogEntry log, int index) noexcept
    : Log(makeAccountStateReader(std::move(final_chain)), std::move(transaction), std::move(log), index) {}
#endif

Log::Log(AccountStateReader account_reader, std::shared_ptr<const Transaction> transaction, ::taraxa::LogEntry log,
         int index) noexcept
    : account_reader_(std::move(account_reader)),
      kTransaction(std::move(transaction)),
      kLog(std::move(log)),
      kIndex(index) {}

int Log::getIndex() const noexcept { return kIndex; }

std::shared_ptr<object::Account> Log::getAccount(std::optional<response::Value>&&) const noexcept {
  return std::make_shared<object::Account>(std::make_shared<Account>(account_reader_, kLog.address));
}

std::vector<response::Value> Log::getTopics() const noexcept {
  std::vector<response::Value> ret;
  ret.reserve(kLog.topics.size());
  for (auto t : kLog.topics) ret.push_back(response::Value(dev::toJS(t)));
  return ret;
}

response::Value Log::getData() const noexcept { return response::Value(dev::toJS(kLog.data)); }

std::shared_ptr<object::Transaction> Log::getTransaction() const noexcept {
  return std::make_shared<object::Transaction>(kTransaction);
}

}  // namespace graphql::taraxa
