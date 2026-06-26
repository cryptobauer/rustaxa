#pragma once

#include <utility>

#include "TestFace.h"
#include "common/app_base.hpp"
#include "network/live_status.hpp"
#include "transaction/transaction.hpp"

namespace dev::eth {
class Client;
}

namespace taraxa::net {

// TestTransactionApi is the Test RPC boundary for test/admin transaction
// submission. The RPC layer owns JSON parsing and transaction construction; the
// adapter supplies external account nonce facts and transaction insertion
// without exposing FinalChain or TransactionManager to public methods.
struct TestTransactionApi {
  std::function<uint64_t(const addr_t&)> next_account_nonce;
  std::function<std::pair<bool, std::string>(const SharedTransaction&)> insert_transaction;
};

class Test : public TestFace {
 public:
  explicit Test(const std::shared_ptr<taraxa::AppBase>& app, LiveStatusReader live_status = {},
                TestTransactionApi transaction_api = {}, uint64_t chain_id = 0);
  virtual RPCModules implementedModules() const override { return RPCModules{RPCModule{"test", "1.0"}}; }

  virtual Json::Value get_sortition_change(const Json::Value& param1) override;
  virtual Json::Value send_coin_transaction(const Json::Value& param1) override;
  virtual Json::Value send_coin_transactions(const Json::Value& param1) override;
  virtual Json::Value get_account_address() override;
  virtual Json::Value get_peer_count() override;
  virtual Json::Value get_node_status() override;
  virtual Json::Value get_all_nodes() override;

 private:
  std::weak_ptr<taraxa::AppBase> app_;
  const uint64_t kChainId;
  LiveStatusReader live_status_;
  TestTransactionApi transaction_api_;
};

}  // namespace taraxa::net
