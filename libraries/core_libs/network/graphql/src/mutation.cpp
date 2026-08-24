#include "graphql/mutation.hpp"

#include <stdexcept>

#include "common/util.hpp"
#include "libdevcore/CommonJS.h"
#ifndef RUSTAXA_ENABLE
#include "transaction/transaction_manager.hpp"
#endif

using namespace std::literals;

namespace graphql::taraxa {

namespace {
#ifndef RUSTAXA_ENABLE
MutationTransactionApi makeMutationTransactionApi(std::weak_ptr<::taraxa::TransactionManager> trx_manager) {
  MutationTransactionApi api;
  api.insert_transaction = [trx_manager](const ::taraxa::SharedTransaction& trx) {
    auto manager = trx_manager.lock();
    if (!manager) {
      throw std::runtime_error("GRAPHQL_MUTATION_TRANSACTION_API_MANAGER_EXPIRED");
    }
    return manager->insertTransaction(trx);
  };
  return api;
}
#endif

void fillMissingMutationTransactionApiCallbacks(MutationTransactionApi& api
#ifndef RUSTAXA_ENABLE
                                                ,
                                                std::weak_ptr<::taraxa::TransactionManager> trx_manager
#endif
) {
#ifndef RUSTAXA_ENABLE
  auto defaults = makeMutationTransactionApi(std::move(trx_manager));
  if (!api.insert_transaction) {
    api.insert_transaction = std::move(defaults.insert_transaction);
  }
#else
  if (!api.insert_transaction) {
    api.insert_transaction = [](const ::taraxa::SharedTransaction&) -> std::pair<bool, std::string> {
      throw std::runtime_error("GRAPHQL_MUTATION_TRANSACTION_API_UNAVAILABLE");
    };
  }
#endif
}
}  // namespace

#ifndef RUSTAXA_ENABLE
Mutation::Mutation(std::shared_ptr<::taraxa::TransactionManager> trx_manager,
                   MutationTransactionApi transaction_api) noexcept
    : transaction_api_(std::move(transaction_api)) {
  fillMissingMutationTransactionApiCallbacks(transaction_api_, std::move(trx_manager));
}
#endif

Mutation::Mutation(MutationTransactionApi transaction_api) noexcept : transaction_api_(std::move(transaction_api)) {
  fillMissingMutationTransactionApiCallbacks(transaction_api_
#ifndef RUSTAXA_ENABLE
                                             ,
                                             {}
#endif
  );
}

response::Value Mutation::applySendRawTransaction(response::Value&& dataArg) const {
  const auto trx =
      std::make_shared<::taraxa::Transaction>(jsToBytes(dataArg.get<std::string>(), dev::OnFailed::Throw), true);
  if (auto [ok, err_msg] = transaction_api_.insert_transaction(trx); !ok) {
    throw(
        std::runtime_error(::taraxa::fmt("Transaction is rejected.\n"
                                         "RLP: %s\n"
                                         "Reason: %s",
                                         dev::toJS(trx->rlp()), err_msg)));
  }
  return response::Value(dev::toJS(trx->getHash()));
}

}  // namespace graphql::taraxa
