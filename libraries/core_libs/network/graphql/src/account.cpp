#include "graphql/account.hpp"

#include "libdevcore/CommonJS.h"

using namespace std::literals;

namespace graphql::taraxa {

AccountStateReader makeAccountStateReader(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain) {
  AccountStateReader reader;
  reader.account_at = [final_chain](const dev::Address& address, std::optional<::taraxa::EthBlockNumber> block_number) {
    return final_chain->getAccount(address, block_number);
  };
  reader.storage_at = [final_chain](const dev::Address& address, const dev::u256& key,
                                    std::optional<::taraxa::EthBlockNumber> block_number) {
    return final_chain->getAccountStorage(address, key, block_number);
  };
  reader.code_at = [final_chain](const dev::Address& address, std::optional<::taraxa::EthBlockNumber> block_number) {
    return final_chain->getCode(address, block_number);
  };
  reader.latest_finalized_block_number = [final_chain] { return final_chain->lastBlockNumber(); };
  return reader;
}

Account::Account(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain, dev::Address address,
                 ::taraxa::EthBlockNumber blk_n)
    : Account(makeAccountStateReader(std::move(final_chain)), std::move(address), blk_n) {}

Account::Account(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain, dev::Address address)
    : Account(makeAccountStateReader(std::move(final_chain)), std::move(address)) {}

Account::Account(AccountStateReader reader, dev::Address address, ::taraxa::EthBlockNumber blk_n)
    : kAddress(std::move(address)), reader_(std::move(reader)), block_number_(blk_n) {
  account_ = reader_.account_at(kAddress, block_number_);
}

Account::Account(AccountStateReader reader, dev::Address address)
    : kAddress(std::move(address)), reader_(std::move(reader)) {
  account_ = reader_.account_at(kAddress, std::nullopt);
}

response::Value Account::getAddress() const noexcept { return response::Value(kAddress.toString()); }

response::Value Account::getBalance() const noexcept {
  if (account_) {
    return response::Value(dev::toJS(account_->balance));
  }
  return response::Value(dev::toJS(0));
}

response::Value Account::getTransactionCount() const noexcept {
  if (account_) {
    return response::Value(static_cast<int>(account_->nonce));
  }
  return response::Value(0);
}

response::Value Account::getCode() const noexcept {
  const auto block_number = block_number_.value_or(reader_.latest_finalized_block_number());
  return response::Value(dev::toJS(reader_.code_at(kAddress, block_number)));
}

response::Value Account::getStorage(response::Value&& slotArg) const {
  const auto block_number = block_number_.value_or(reader_.latest_finalized_block_number());
  return response::Value(dev::toJS(reader_.storage_at(kAddress, dev::u256(slotArg.get<std::string>()), block_number)));
}

}  // namespace graphql::taraxa
