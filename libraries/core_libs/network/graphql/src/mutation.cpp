#include "graphql/mutation.hpp"

#include <stdexcept>

#include "common/util.hpp"
#include "libdevcore/CommonJS.h"
#include "transaction/transaction_manager.hpp"

using namespace std::literals;

namespace graphql::taraxa {

namespace {
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

void fillMissingMutationTransactionApiCallbacks(MutationTransactionApi& api,
                                                std::weak_ptr<::taraxa::TransactionManager> trx_manager) {
  auto defaults = makeMutationTransactionApi(std::move(trx_manager));
  if (!api.insert_transaction) {
    api.insert_transaction = std::move(defaults.insert_transaction);
  }
}
}  // namespace

Mutation::Mutation(std::shared_ptr<::taraxa::TransactionManager> trx_manager,
                   MutationTransactionApi transaction_api) noexcept
    : trx_manager_(std::move(trx_manager)), transaction_api_(std::move(transaction_api)) {
  fillMissingMutationTransactionApiCallbacks(transaction_api_, trx_manager_);
}

Mutation::Mutation(MutationTransactionApi transaction_api) noexcept : transaction_api_(std::move(transaction_api)) {
  fillMissingMutationTransactionApiCallbacks(transaction_api_, trx_manager_);
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
