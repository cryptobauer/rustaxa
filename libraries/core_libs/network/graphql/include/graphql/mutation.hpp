#pragma once

#include <functional>
#include <memory>
#include <string>
#include <utility>

#include "MutationObject.h"
#include "transaction/transaction.hpp"

#ifndef RUSTAXA_ENABLE
namespace taraxa {
class TransactionManager;
}
#endif

namespace graphql::taraxa {

// MutationTransactionApi is the GraphQL mutation boundary for external
// transaction submission. GraphQL owns raw input decoding and response
// formatting; the adapter owns mempool insertion without exposing
// TransactionManager to mutation methods.
struct MutationTransactionApi {
  std::function<std::pair<bool, std::string>(const ::taraxa::SharedTransaction&)> insert_transaction;
};

class Mutation {
 public:
  Mutation() = default;
#ifndef RUSTAXA_ENABLE
  explicit Mutation(std::shared_ptr<::taraxa::TransactionManager> trx_manager,
                    MutationTransactionApi transaction_api = {}) noexcept;
#endif
  explicit Mutation(MutationTransactionApi transaction_api) noexcept;

  response::Value applySendRawTransaction(response::Value&& dataArg) const;

 private:
  MutationTransactionApi transaction_api_;
};

}  // namespace graphql::taraxa
