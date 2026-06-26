#pragma once

#include <functional>
#include <string>
#include <utility>
#include <vector>

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

// TestNetworkReader is the Test RPC boundary for live network facts. The RPC
// layer owns JSON formatting; the adapter supplies peer counts and node
// endpoint views without exposing Network to public methods.
struct TestNetworkNodeView {
  std::string node_id;
  std::string address;
  uint64_t listen_port = 0;
};

struct TestNetworkReader {
  std::function<uint64_t()> peer_count;
  std::function<std::vector<TestNetworkNodeView>()> all_nodes;
};

// TestNodeStatusView contains the storage-backed counters and DAG head fact
// that Test RPC exposes in get_node_status. Live sync, peer, and pool facts are
// intentionally carried by LiveStatusSnapshot instead.
struct TestNodeStatusView {
  uint64_t blocks_executed = 0;
  uint64_t dag_blocks_count = 0;
  uint64_t transactions_executed = 0;
  uint64_t transactions_count = 0;
  uint64_t dag_level = 0;
};

// TestNodeStatusReader is the Test RPC boundary for persisted node-status
// counters. The RPC layer owns JSON formatting; the adapter supplies the
// counters without exposing DbStorage, DagManager, or TransactionManager to the
// public method.
struct TestNodeStatusReader {
  std::function<TestNodeStatusView()> status;
};

class Test : public TestFace {
 public:
  explicit Test(const std::shared_ptr<taraxa::AppBase>& app, LiveStatusReader live_status = {},
                TestTransactionApi transaction_api = {}, uint64_t chain_id = 0, TestNetworkReader network_reader = {},
                TestNodeStatusReader node_status_reader = {});
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
  TestNetworkReader network_reader_;
  TestNodeStatusReader node_status_reader_;
};

}  // namespace taraxa::net
