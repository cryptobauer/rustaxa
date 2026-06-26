#pragma once

#include <functional>
#include <memory>
#include <optional>
#include <string>

#include "AccountObject.h"
#include "final_chain/final_chain.hpp"
#include "final_chain/state_api.hpp"

namespace graphql::taraxa {

// AccountStateReader is GraphQL's minimal account-state boundary. Callers provide
// account, code, storage, and finalized-block facts for one requested block
// context; missing accounts are represented by std::nullopt, while field-level
// read failures keep the existing exception behavior of the backing adapter.
struct AccountStateReader {
  std::function<std::optional<::taraxa::state_api::Account>(const dev::Address&,
                                                            std::optional<::taraxa::EthBlockNumber>)>
      account_at;
  std::function<dev::h256(const dev::Address&, const dev::u256&, std::optional<::taraxa::EthBlockNumber>)> storage_at;
  std::function<dev::bytes(const dev::Address&, std::optional<::taraxa::EthBlockNumber>)> code_at;
  std::function<::taraxa::EthBlockNumber()> latest_finalized_block_number;
};

// Builds the temporary compatibility adapter for GraphQL account-state reads.
// The returned reader keeps the GraphQL object API narrow while the backing
// implementation remains on the external FinalChain/StateAPI boundary.
AccountStateReader makeAccountStateReader(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain);

class Account {
 public:
  explicit Account(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain, dev::Address address,
                   ::taraxa::EthBlockNumber blk_n);
  explicit Account(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain, dev::Address address);
  explicit Account(AccountStateReader reader, dev::Address address, ::taraxa::EthBlockNumber blk_n);
  explicit Account(AccountStateReader reader, dev::Address address);

  response::Value getAddress() const noexcept;
  response::Value getBalance() const noexcept;
  response::Value getTransactionCount() const noexcept;
  response::Value getCode() const noexcept;
  response::Value getStorage(response::Value&& slotArg) const;

 private:
  const dev::Address kAddress;
  std::optional<::taraxa::state_api::Account> account_;
  AccountStateReader reader_;
  std::optional<::taraxa::EthBlockNumber> block_number_;
};
}  // namespace graphql::taraxa
