#include "Test.h"

#include <jsonrpccpp/common/errors.h>
#include <jsonrpccpp/common/exception.h>
#include <libdevcore/CommonJS.h>

#include <stdexcept>

#include "common/types.hpp"
#include "dag/dag_manager.hpp"
#include "network/network.hpp"
#include "pbft/pbft_manager.hpp"
#include "transaction/transaction_manager.hpp"
#include "vote_manager/vote_manager.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

using namespace std;
using namespace dev;
using namespace ::taraxa::final_chain;
using namespace jsonrpc;
using namespace taraxa;

namespace taraxa::net {

namespace {
LiveStatusSnapshot collectLiveStatusSnapshot(const std::shared_ptr<taraxa::AppBase> &node) {
  LiveStatusSnapshot snapshot;
  const auto chain_size = node->getPbftChain()->getPbftChainSize();
  const auto dpos_total_votes = node->getPbftManager()->getCurrentDposTotalVotesCount();
  const auto dpos_node_votes = node->getPbftManager()->getCurrentNodeVotesCount();
  const auto dpos_quorum = node->getVoteManager()->getPbftTwoTPlusOne(chain_size, PbftVoteTypes::cert_vote);

  snapshot.pbft_syncing = node->getNetwork()->pbft_syncing();
  snapshot.syncing_seconds = node->getNetwork()->syncTimeSeconds();
  snapshot.peer_count = node->getNetwork()->getPeerCount();
  snapshot.node_count = node->getNetwork()->getNodeCount();
  snapshot.pbft_chain_size = chain_size;
  snapshot.pbft_sync_period = node->getPbftManager()->pbftSyncingPeriod();
  snapshot.pbft_round = node->getPbftManager()->getPbftRound();
  snapshot.dpos_total_votes = dpos_total_votes.value_or(0);
  snapshot.dpos_node_votes = dpos_node_votes.value_or(0);
  snapshot.dpos_quorum = dpos_quorum.value_or(0);
  snapshot.pbft_sync_queue_size = node->getPbftManager()->periodDataQueueSize();
  snapshot.transaction_pool_size = node->getTransactionManager()->getTransactionPoolSize();
  snapshot.nonfinalized_transaction_size = node->getTransactionManager()->getNonfinalizedTrxSize();
  if (const auto peer = node->getNetwork()->getMaxChainPeer()) {
    snapshot.max_peer_pbft_chain_size = peer->pbft_chain_size_.load();
  }
  snapshot.compatibility_network_status = node->getNetwork()->getStatus();
  return snapshot;
}

TestTransactionApi makeTestTransactionApi(std::weak_ptr<taraxa::AppBase> app) {
  TestTransactionApi api;
  api.next_account_nonce = [app](const addr_t &address) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TEST_TRANSACTION_API_APP_EXPIRED");
    }
    return node->getFinalChain()->getAccount(address).value().nonce.convert_to<uint64_t>() + 1;
  };
  api.insert_transaction = [app](const SharedTransaction &trx) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TEST_TRANSACTION_API_APP_EXPIRED");
    }
    return node->getTransactionManager()->insertTransaction(trx);
  };
  return api;
}

TestNetworkReader makeTestNetworkReader(std::weak_ptr<taraxa::AppBase> app) {
  TestNetworkReader reader;
  reader.peer_count = [app] {
    auto node = app.lock();
    if (!node) {
      return uint64_t(0);
    }
    return static_cast<uint64_t>(node->getNetwork()->getPeerCount());
  };
  reader.all_nodes = [app] {
    std::vector<TestNetworkNodeView> result;
    auto node = app.lock();
    if (!node) {
      return result;
    }
    const auto nodes = node->getNetwork()->getAllNodes();
    result.reserve(nodes.size());
    for (const auto &network_node : nodes) {
      result.push_back(TestNetworkNodeView{network_node.id().toString(), network_node.endpoint().address().to_string(),
                                           network_node.endpoint().tcpPort()});
    }
    return result;
  };
  return reader;
}

void fillMissingTestTransactionApiCallbacks(TestTransactionApi &api, std::weak_ptr<taraxa::AppBase> app) {
  auto defaults = makeTestTransactionApi(std::move(app));
  if (!api.next_account_nonce) {
    api.next_account_nonce = std::move(defaults.next_account_nonce);
  }
  if (!api.insert_transaction) {
    api.insert_transaction = std::move(defaults.insert_transaction);
  }
}

void fillMissingTestNetworkReaderCallbacks(TestNetworkReader &reader, std::weak_ptr<taraxa::AppBase> app) {
  auto defaults = makeTestNetworkReader(std::move(app));
  if (!reader.peer_count) {
    reader.peer_count = std::move(defaults.peer_count);
  }
  if (!reader.all_nodes) {
    reader.all_nodes = std::move(defaults.all_nodes);
  }
}
}  // namespace

Test::Test(const std::shared_ptr<taraxa::AppBase> &app, LiveStatusReader live_status,
           TestTransactionApi transaction_api, uint64_t chain_id, TestNetworkReader network_reader)
    : app_(app),
      kChainId(app ? app->getConfig().genesis.chain_id : chain_id),
      live_status_(std::move(live_status)),
      transaction_api_(std::move(transaction_api)),
      network_reader_(std::move(network_reader)) {
  fillMissingTestTransactionApiCallbacks(transaction_api_, app_);
  fillMissingTestNetworkReaderCallbacks(network_reader_, app_);
}

Json::Value Test::get_sortition_change(const Json::Value &param1) {
  try {
    Json::Value res;
    if (auto node = app_.lock()) {
      uint64_t period = param1["period"].asUInt64();
#ifdef RUSTAXA_ENABLE
      {
        const auto query_api = rustaxa::create_consensus_query_api(node->getDB()->rustStorage());
        const auto params_change = query_api->consensus_query_sortition_params_change_by_period(period);
        if (!params_change.found) {
          return res;
        }
        res["interval_efficiency"] = params_change.interval_efficiency;
        res["period"] = Json::UInt64(params_change.period);
        res["threshold_upper"] = params_change.threshold_upper;
        res["kThresholdUpperMinValue"] = params_change.threshold_upper_min;
        return res;
      }
#endif
      auto params_change = node->getDB()->getParamsChangeForPeriod(period);  // RUSTAXA_QUERY_COMPAT_READ
      res["interval_efficiency"] = params_change->interval_efficiency;
      res["period"] = params_change->period;
      res["threshold_upper"] = params_change->vrf_params.threshold_upper;
      res["kThresholdUpperMinValue"] = params_change->vrf_params.kThresholdUpperMinValue;
    }
    return res;
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Test::send_coin_transaction(const Json::Value &param1) {
  Json::Value res;
  secret_t sk = secret_t(param1["secret"].asString());
  uint64_t nonce = 0;
  if (!param1["nonce"]) {
    nonce = transaction_api_.next_account_nonce(toAddress(sk));
  } else {
    nonce = dev::jsToInt(param1["nonce"].asString());
  }
  val_t value = val_t(param1["value"].asString());
  val_t gas_price = val_t(param1["gasPrice"].asString());
  auto gas = dev::jsToInt(param1["gas"].asString());
  addr_t receiver = addr_t(param1["receiver"].asString());
  auto trx = std::make_shared<Transaction>(nonce, value, gas_price, gas, bytes(), sk, receiver, kChainId);
  if (auto [ok, err_msg] = transaction_api_.insert_transaction(trx); !ok) {
    res["status"] = err_msg;
  } else {
    res = toHex(trx->rlp());
  }
  return res;
}

Json::Value Test::send_coin_transactions(const Json::Value &param1) {
  Json::Value res;
  uint32_t inserted = 0;
  secret_t sk = secret_t(param1["secret"].asString());
  auto nonce = param1["nonce"].asUInt64();
  val_t value = val_t(param1["value"].asString());
  val_t gas_price = val_t(param1["gasPrice"].asString());
  auto gas = dev::jsToInt(param1["gas"].asString());
  auto transactions_count = param1["transaction_count"].asUInt64();
  std::vector<addr_t> receivers;
  std::transform(param1["receiver"].begin(), param1["receiver"].end(), std::back_inserter(receivers),
                 [](const auto rec) { return addr_t(rec.asString()); });
  for (uint32_t i = 0; i < transactions_count; i++) {
    auto trx = std::make_shared<Transaction>(nonce, value, gas_price, gas, bytes(), sk, receivers[i % receivers.size()],
                                             kChainId);
    nonce++;
    if (auto [ok, err_msg] = transaction_api_.insert_transaction(trx); !ok) {
      res["err"] = err_msg;
      break;
    } else {
      inserted++;
    }
  }
  res["status"] = Json::UInt64(inserted);

  return res;
}

Json::Value Test::get_account_address() {
  Json::Value res;
  if (auto node = app_.lock()) {
    addr_t addr = node->getAddress();
    res["value"] = addr.toString();
  }
  return res;
}

Json::Value Test::get_peer_count() {
  Json::Value res;
  if (network_reader_.peer_count) {
    res["value"] = Json::UInt64(network_reader_.peer_count());
  }
  return res;
}

Json::Value Test::get_node_status() {
  Json::Value res;
  if (auto node = app_.lock()) {
    const auto live_status = live_status_ ? live_status_() : collectLiveStatusSnapshot(node);
#ifdef RUSTAXA_ENABLE
    const auto query_api = rustaxa::create_consensus_query_api(node->getDB()->rustStorage());
    const auto chain_stats = query_api->consensus_query_chain_stats();
    const auto consensus_status = query_api->consensus_query_status();
#endif

    res["synced"] = !live_status.pbft_syncing;
    res["syncing_seconds"] = Json::UInt64(live_status.syncing_seconds);
    res["peer_count"] = Json::UInt64(live_status.peer_count);
    res["node_count"] = Json::UInt64(live_status.node_count);
#ifdef RUSTAXA_ENABLE
    res["blk_executed"] = Json::UInt64(chain_stats.dag_blocks_executed);
    res["blk_count"] = Json::UInt64(chain_stats.dag_blocks_count);
    res["trx_executed"] = Json::UInt64(chain_stats.transactions_executed);
    res["trx_count"] = Json::UInt64(chain_stats.transactions_count);
#else
    res["blk_executed"] = Json::UInt64(node->getDB()->getNumBlockExecuted());        // RUSTAXA_QUERY_COMPAT_READ
    res["blk_count"] = Json::UInt64(node->getDB()->getDagBlocksCount());             // RUSTAXA_QUERY_COMPAT_READ
    res["trx_executed"] = Json::UInt64(node->getDB()->getNumTransactionExecuted());  // RUSTAXA_QUERY_COMPAT_READ
    res["trx_count"] = Json::UInt64(node->getTransactionManager()->getTransactionCount());
#endif
#ifdef RUSTAXA_ENABLE
    res["dag_level"] = Json::UInt64(consensus_status.latest_dag_level);
#else
    res["dag_level"] = Json::UInt64(node->getDagManager()->getMaxLevel());
#endif
    res["pbft_size"] = Json::UInt64(live_status.pbft_chain_size);
    res["pbft_sync_period"] = Json::UInt64(live_status.pbft_sync_period);
    res["pbft_round"] = Json::UInt64(live_status.pbft_round);
    res["dpos_total_votes"] = Json::UInt64(live_status.dpos_total_votes);
    res["dpos_node_votes"] = Json::UInt64(live_status.dpos_node_votes);
    res["dpos_quorum"] = Json::UInt64(live_status.dpos_quorum);
    res["pbft_sync_queue_size"] = Json::UInt64(live_status.pbft_sync_queue_size);
    res["trx_pool_size"] = Json::UInt64(live_status.transaction_pool_size);
    res["trx_nonfinalized_size"] = Json::UInt64(live_status.nonfinalized_transaction_size);
    res["network"] = live_status.compatibility_network_status;
  }
  return res;
}

Json::Value Test::get_all_nodes() {
  Json::Value res;

  if (network_reader_.all_nodes) {
    const auto nodes = network_reader_.all_nodes();
    res["nodes_count"] = Json::UInt64(nodes.size());
    res["nodes"] = Json::Value(Json::arrayValue);
    for (const auto &n : nodes) {
      Json::Value node;
      node["node_id"] = n.node_id;
      node["address"] = n.address;
      node["listen_port"] = Json::UInt64(n.listen_port);
      res["nodes"].append(node);
    }
  }
  return res;
}

}  // namespace taraxa::net
