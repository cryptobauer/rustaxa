#include <gtest/gtest.h>
#include <jsonrpccpp/common/exception.h>
#include <libdevcore/Common.h>
#include <libdevcore/CommonJS.h>

#include <filesystem>
#include <sstream>

#include "common/encoding_rlp.hpp"
#include "graphql/account.hpp"
#include "graphql/block.hpp"
#include "graphql/log.hpp"
#include "graphql/mutation.hpp"
#include "graphql/query.hpp"
#include "graphql/sync_state.hpp"
#include "graphql/transaction.hpp"
#include "graphql/types/current_state.hpp"
#include "graphql/types/dag_block.hpp"
#include "network/rpc/Debug.h"
#include "network/rpc/Taraxa.h"
#include "network/rpc/Test.h"
#include "network/rpc/eth/Eth.h"
#include "network/rpc/eth/LiveLogSubscription.hpp"
#include "network/subscriptions.hpp"
#include "test_util/samples.hpp"

namespace taraxa::core_tests {

struct RPCTest : NodesTest {
  std::vector<FullNodeConfig> make_isolated_node_cfgs(size_t total_count, size_t validators_count = 1,
                                                      uint tests_speed = 1, bool enable_rpc_http = false,
                                                      bool enable_rpc_ws = false) {
    auto cfgs = make_node_cfgs(total_count, validators_count, tests_speed, enable_rpc_http, enable_rpc_ws);
    const auto* test_info = ::testing::UnitTest::GetInstance()->current_test_info();
    const auto test_data_dir =
        std::filesystem::temp_directory_path() / "taraxa_node_tests" / test_info->test_suite_name() / test_info->name();
    std::filesystem::remove_all(test_data_dir);
    std::filesystem::create_directories(test_data_dir);
    static uint16_t next_port_base = 20000;
    const auto port_base = next_port_base;
    next_port_base += 10;
    for (size_t idx = 0; idx < cfgs.size(); ++idx) {
      auto& cfg = cfgs[idx];
      cfg.data_path = test_data_dir / ("node" + std::to_string(idx));
      cfg.db_path = cfg.data_path / "db";
      cfg.log_path = cfg.data_path / "log";
      cfg.network.listen_port = port_base + idx;
      if (cfg.network.rpc->http_port) {
        cfg.network.rpc->http_port = port_base + 100 + idx;
      }
      if (cfg.network.rpc->ws_port) {
        cfg.network.rpc->ws_port = port_base + 200 + idx;
      }
      if (cfgs.size() == 1) {
        cfg.network.boot_nodes.clear();
      }
      cfg.log_configs.clear();
    }
    return cfgs;
  }
};

TEST_F(RPCTest, eth_syncing_uses_live_status_reader) {
  auto eth_json_rpc = net::rpc::eth::NewEth(net::rpc::eth::EthParams{});
  EXPECT_FALSE(eth_json_rpc->eth_syncing().asBool());

  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.live_status = [] {
    net::LiveStatusSnapshot snapshot;
    snapshot.pbft_syncing = true;
    snapshot.pbft_chain_size = 4;
    snapshot.pbft_sync_period = 9;
    return snapshot;
  };
  eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  const auto syncing = eth_json_rpc->eth_syncing();
  EXPECT_EQ(dev::toJS(4), syncing["startingBlock"].asString());
  EXPECT_EQ(dev::toJS(4), syncing["currentBlock"].asString());
  EXPECT_EQ(dev::toJS(9), syncing["highestBlock"].asString());
}

TEST_F(RPCTest, graphql_syncing_uses_live_status_reader) {
  graphql::taraxa::SyncState sync_state(
      nullptr, {}, [] { return 6; },
      [] {
        net::LiveStatusSnapshot snapshot;
        snapshot.max_peer_pbft_chain_size = 12;
        return snapshot;
      });

  EXPECT_EQ(0, sync_state.getStartingBlock().get<int>());
  EXPECT_EQ(6, sync_state.getCurrentBlock().get<int>());
  EXPECT_EQ(12, sync_state.getHighestBlock().get<int>());
}

TEST_F(RPCTest, graphql_status_objects_use_reader_apis) {
  auto final_block_called = std::make_shared<bool>(false);
  auto dag_level_called = std::make_shared<bool>(false);
  auto dag_period_called = std::make_shared<bool>(false);
  graphql::taraxa::CurrentStateReader current_reader;
  current_reader.final_block = [final_block_called] {
    *final_block_called = true;
    return uint64_t(10);
  };
  current_reader.dag_block_level = [dag_level_called] {
    *dag_level_called = true;
    return uint64_t(11);
  };
  current_reader.dag_block_period = [dag_period_called] {
    *dag_period_called = true;
    return uint64_t(12);
  };

  graphql::taraxa::CurrentState current_state(std::move(current_reader));
  EXPECT_EQ(10, current_state.getFinalBlock().get<int>());
  EXPECT_EQ(11, current_state.getDagBlockLevel().get<int>());
  EXPECT_EQ(12, current_state.getDagBlockPeriod().get<int>());
  ASSERT_TRUE(*final_block_called);
  ASSERT_TRUE(*dag_level_called);
  ASSERT_TRUE(*dag_period_called);

  auto current_block_called = std::make_shared<bool>(false);
  auto highest_block_called = std::make_shared<bool>(false);
  graphql::taraxa::SyncStateReader sync_reader;
  sync_reader.current_block = [current_block_called] {
    *current_block_called = true;
    return uint64_t(13);
  };
  sync_reader.highest_block = [highest_block_called] {
    *highest_block_called = true;
    return std::optional<uint64_t>(14);
  };

  graphql::taraxa::SyncState sync_state(std::move(sync_reader));
  EXPECT_EQ(13, sync_state.getCurrentBlock().get<int>());
  EXPECT_EQ(14, sync_state.getHighestBlock().get<int>());
  ASSERT_TRUE(*current_block_called);
  ASSERT_TRUE(*highest_block_called);
}

TEST_F(RPCTest, live_log_subscription_uses_subscription_api) {
  std::vector<std::string> sent_messages;
  auto called = std::make_shared<bool>(false);
  const auto log_address = addr_t::random();
  const auto topic = h256::random();
  const auto block_hash = blk_hash_t::random();
  const auto trx_hash = trx_hash_t::random();

  net::rpc::eth::LiveLogSubscriptionApi live_logs;
  live_logs.matching_logs = [called, log_address, topic, block_hash, trx_hash](
                                const net::rpc::eth::LogFilter&, const net::rpc::eth::LiveLogBlock& block) {
    *called = true;
    EXPECT_EQ(7, block.block_number);
    EXPECT_EQ(block_hash, block.block_hash);

    net::rpc::eth::LocalisedLogEntry entry;
    entry.le = LogEntry{log_address, {topic}, bytes{0xaa, 0xbb}};
    entry.trx_loc.period = block.block_number;
    entry.trx_loc.blk_h = block.block_hash;
    entry.trx_loc.trx_hash = trx_hash;
    entry.position_in_receipt = 2;
    return std::vector<net::rpc::eth::LocalisedLogEntry>{entry};
  };

  net::Subscriptions subscriptions([&](std::string&& message) { sent_messages.push_back(std::move(message)); },
                                   std::move(live_logs));
  subscriptions.addSubscription(std::make_shared<net::LogsSubscription>(
      3, net::rpc::eth::LogFilter(0, std::nullopt, {}, net::rpc::eth::LogFilter::Topics{})));

  net::rpc::eth::LiveLogBlock block;
  block.block_number = 7;
  block.block_hash = block_hash;
  subscriptions.processLogs(block);

  ASSERT_TRUE(*called);
  ASSERT_EQ(1, sent_messages.size());

  Json::CharReaderBuilder builder;
  Json::Value message;
  std::string errors;
  std::istringstream stream(sent_messages.front());
  ASSERT_TRUE(Json::parseFromStream(builder, stream, &message, &errors)) << errors;
  EXPECT_EQ("eth_subscription", message["method"].asString());
  EXPECT_EQ(dev::toJS(3), message["params"]["subscription"].asString());
  EXPECT_EQ(dev::toJS(log_address), message["params"]["result"]["address"].asString());
  EXPECT_EQ(dev::toJS(topic), message["params"]["result"]["topics"][0].asString());
  EXPECT_EQ(dev::toJS(block_hash), message["params"]["result"]["blockHash"].asString());
  EXPECT_EQ(dev::toJS(trx_hash), message["params"]["result"]["transactionHash"].asString());
}

TEST_F(RPCTest, eth_filter_changes_uses_live_log_subscription_api) {
  auto called = std::make_shared<bool>(false);
  const auto log_address = addr_t::random();
  const auto topic = h256::random();
  const auto block_hash = blk_hash_t::random();
  const auto trx_hash = trx_hash_t::random();

  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.live_log_subscription.matching_logs = [called, log_address, topic, block_hash, trx_hash](
                                                           const net::rpc::eth::LogFilter&,
                                                           const net::rpc::eth::LiveLogBlock& block) {
    *called = true;
    EXPECT_EQ(8, block.block_number);
    EXPECT_EQ(block_hash, block.block_hash);

    net::rpc::eth::LocalisedLogEntry entry;
    entry.le = LogEntry{log_address, {topic}, bytes{0xcc}};
    entry.trx_loc.period = block.block_number;
    entry.trx_loc.position = 0;
    entry.trx_loc.blk_h = block.block_hash;
    entry.trx_loc.trx_hash = trx_hash;
    return std::vector<net::rpc::eth::LocalisedLogEntry>{entry};
  };
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));
  Json::Value filter(Json::objectValue);
  filter["fromBlock"] = dev::toJS(0);
  const auto filter_id = eth_json_rpc->eth_newFilter(filter);

  final_chain::BlockHeader header;
  header.number = 8;
  header.hash = block_hash;
  eth_json_rpc->note_block_executed(header, {}, {});

  const auto changes = eth_json_rpc->eth_getFilterChanges(filter_id);
  ASSERT_TRUE(*called);
  ASSERT_EQ(1, changes.size());
  EXPECT_EQ(dev::toJS(log_address), changes[0]["address"].asString());
  EXPECT_EQ(dev::toJS(topic), changes[0]["topics"][0].asString());
  EXPECT_EQ(dev::toJS(block_hash), changes[0]["blockHash"].asString());
  EXPECT_EQ(dev::toJS(trx_hash), changes[0]["transactionHash"].asString());
}

TEST_F(RPCTest, debug_dpos_reads_use_debug_dpos_reader) {
  const auto validator = addr_t::random();
  auto stakes_called = std::make_shared<bool>(false);
  auto delegated_called = std::make_shared<bool>(false);

  net::DebugDposReader reader;
  reader.validators_total_stakes = [stakes_called, validator](EthBlockNumber block_number) {
    *stakes_called = true;
    EXPECT_EQ(9, block_number);
    return std::vector<state_api::ValidatorStake>{{validator, u256(1234)}};
  };
  reader.total_amount_delegated = [delegated_called](EthBlockNumber block_number) {
    *delegated_called = true;
    EXPECT_EQ(9, block_number);
    return uint256_t(5678);
  };
  reader.eligible_total_vote_count = [](EthBlockNumber) { return 0; };

  net::Debug debug_rpc(nullptr, 0, std::move(reader));

  const auto stakes = debug_rpc.debug_dposValidatorTotalStakes(dev::toJS(9));
  ASSERT_TRUE(*stakes_called);
  ASSERT_EQ(1, stakes.size());
  EXPECT_EQ("0x" + validator.toString(), stakes[0]["address"].asString());
  EXPECT_EQ("1234", stakes[0]["total_stake"].asString());

  EXPECT_EQ(dev::toJS(uint256_t(5678)), debug_rpc.debug_dposTotalAmountDelegated(dev::toJS(9)).asString());
  ASSERT_TRUE(*delegated_called);
}

TEST_F(RPCTest, debug_trace_call_uses_debug_trace_reader) {
  const auto caller = addr_t::random();
  auto latest_called = std::make_shared<bool>(false);
  auto account_called = std::make_shared<bool>(false);
  auto trace_called = std::make_shared<bool>(false);

  net::DebugTraceReader trace_reader;
  trace_reader.latest_finalized_block_number = [latest_called] {
    *latest_called = true;
    return EthBlockNumber(15);
  };
  trace_reader.account_at = [account_called, caller](const Address& requested_address, EthBlockNumber block_number) {
    *account_called = true;
    EXPECT_EQ(caller, requested_address);
    EXPECT_EQ(15, block_number);
    state_api::Account account;
    account.nonce = 42;
    return std::optional<state_api::Account>(account);
  };
  trace_reader.trace = [trace_called, caller](std::vector<state_api::EVMTransaction> state_trxs,
                                              std::vector<state_api::EVMTransaction> trxs, EthBlockNumber block_number,
                                              std::optional<state_api::Tracing> tracing) {
    *trace_called = true;
    EXPECT_TRUE(state_trxs.empty());
    EXPECT_EQ(1, trxs.size());
    EXPECT_EQ(caller, trxs.front().from);
    EXPECT_EQ(42, trxs.front().nonce);
    EXPECT_EQ(15, block_number);
    EXPECT_FALSE(tracing.has_value());
    return std::string(R"({"ok":true})");
  };

  net::Debug debug_rpc(nullptr, 1000000, {}, std::move(trace_reader));
  Json::Value call(Json::objectValue);
  call["from"] = dev::toJS(caller);

  const auto result = debug_rpc.debug_traceCall(call, "latest");
  EXPECT_TRUE(result["ok"].asBool());
  ASSERT_TRUE(*latest_called);
  ASSERT_TRUE(*account_called);
  ASSERT_TRUE(*trace_called);
}

TEST_F(RPCTest, taraxa_dpos_scalar_reads_use_taraxa_dpos_reader) {
  auto yield_called = std::make_shared<bool>(false);
  auto supply_called = std::make_shared<bool>(false);

  net::TaraxaDposReader reader;
  reader.dpos_yield = [yield_called](EthBlockNumber block_number) {
    *yield_called = true;
    EXPECT_EQ(12, block_number);
    return uint64_t(34);
  };
  reader.total_supply = [supply_called](EthBlockNumber block_number) {
    *supply_called = true;
    EXPECT_EQ(12, block_number);
    return u256(5600);
  };
  reader.eligible_total_vote_count = [](EthBlockNumber) { return 0; };
  reader.eligible_vote_count = [](EthBlockNumber, const addr_t&) { return 0; };

  net::Taraxa taraxa_rpc(nullptr, std::move(reader));

  EXPECT_EQ(dev::toJS(uint64_t(34)), taraxa_rpc.taraxa_yield(dev::toJS(12)));
  ASSERT_TRUE(*yield_called);
  EXPECT_EQ(dev::toJS(u256(5600)), taraxa_rpc.taraxa_totalSupply(dev::toJS(12)));
  ASSERT_TRUE(*supply_called);
}

TEST_F(RPCTest, taraxa_dag_status_reads_use_dag_status_reader) {
  auto level_called = std::make_shared<bool>(false);
  auto period_called = std::make_shared<bool>(false);

  net::TaraxaDagStatusReader dag_status_reader;
  dag_status_reader.latest_level = [level_called] {
    *level_called = true;
    return uint64_t(44);
  };
  dag_status_reader.latest_period = [period_called] {
    *period_called = true;
    return uint64_t(55);
  };

  net::Taraxa taraxa_rpc(nullptr, {}, std::move(dag_status_reader));

  EXPECT_EQ(dev::toJS(uint64_t(44)), taraxa_rpc.taraxa_dagBlockLevel());
  ASSERT_TRUE(*level_called);
  EXPECT_EQ(dev::toJS(uint64_t(55)), taraxa_rpc.taraxa_dagBlockPeriod());
  ASSERT_TRUE(*period_called);
}

TEST_F(RPCTest, taraxa_dag_block_reads_use_dag_block_reader) {
  auto transaction = samples::createSignedTrxSamples(1, 1, secret_t::random()).front();
  const auto transaction_hash = transaction->getHash();
  auto dag_block = std::make_shared<DagBlock>(blk_hash_t(9), level_t(4), vec_blk_t{}, vec_trx_t{transaction_hash},
                                              secret_t::random());
  const auto dag_hash = dag_block->getHash();

  auto by_hash_called = std::make_shared<bool>(false);
  auto by_level_called = std::make_shared<bool>(false);
  auto period_called = std::make_shared<int>(0);
  auto transaction_called = std::make_shared<int>(0);

  net::TaraxaDagBlockReader dag_block_reader;
  dag_block_reader.block_by_hash = [by_hash_called, dag_block, dag_hash](const blk_hash_t& requested_hash) {
    *by_hash_called = true;
    EXPECT_EQ(dag_hash, requested_hash);
    return dag_block;
  };
  dag_block_reader.blocks_by_level = [by_level_called, dag_block](level_t requested_level) {
    *by_level_called = true;
    EXPECT_EQ(level_t(4), requested_level);
    return std::vector<std::shared_ptr<DagBlock>>{dag_block};
  };
  dag_block_reader.period_by_hash = [period_called, dag_hash](const blk_hash_t& requested_hash) {
    ++*period_called;
    EXPECT_EQ(dag_hash, requested_hash);
    return std::optional<uint64_t>(33);
  };
  dag_block_reader.transaction_by_hash = [transaction_called, transaction,
                                          transaction_hash](const trx_hash_t& requested_hash) {
    ++*transaction_called;
    EXPECT_EQ(transaction_hash, requested_hash);
    return transaction;
  };

  net::Taraxa taraxa_rpc(nullptr, {}, {}, std::move(dag_block_reader));

  const auto by_hash = taraxa_rpc.taraxa_getDagBlockByHash(dag_hash.toString(), true);
  EXPECT_EQ(dev::toJS(uint64_t(33)), by_hash["period"].asString());
  ASSERT_EQ(Json::ArrayIndex(1), by_hash["transactions"].size());
  ASSERT_TRUE(*by_hash_called);

  const auto by_level = taraxa_rpc.taraxa_getDagBlockByLevel(dev::toJS(uint64_t(4)), true);
  ASSERT_EQ(Json::ArrayIndex(1), by_level.size());
  EXPECT_EQ(dev::toJS(uint64_t(33)), by_level[0]["period"].asString());
  ASSERT_EQ(Json::ArrayIndex(1), by_level[0]["transactions"].size());
  ASSERT_TRUE(*by_level_called);
  EXPECT_EQ(2, *period_called);
  EXPECT_EQ(2, *transaction_called);
}

TEST_F(RPCTest, test_coin_transaction_uses_transaction_api) {
  constexpr uint64_t chain_id = 2999;
  const auto secret = secret_t::random();
  const auto sender = dev::toAddress(secret);
  const auto receiver = addr_t::random();
  auto nonce_called = std::make_shared<bool>(false);
  auto insert_called = std::make_shared<bool>(false);

  net::TestTransactionApi transaction_api;
  transaction_api.next_account_nonce = [nonce_called, sender](const addr_t& requested_sender) {
    *nonce_called = true;
    EXPECT_EQ(sender, requested_sender);
    return uint64_t(17);
  };
  transaction_api.insert_transaction = [insert_called, sender, receiver](const SharedTransaction& trx) {
    *insert_called = true;
    EXPECT_EQ(17, trx->getNonce());
    EXPECT_EQ(sender, trx->getSender());
    EXPECT_TRUE(trx->getReceiver().has_value());
    if (!trx->getReceiver()) {
      return std::pair<bool, std::string>{false, "missing receiver"};
    }
    EXPECT_EQ(receiver, *trx->getReceiver());
    return std::pair<bool, std::string>{true, ""};
  };

  net::Test test_rpc(nullptr, {}, std::move(transaction_api), chain_id);
  Json::Value params(Json::objectValue);
  params["secret"] = secret.makeInsecure().hex();
  params["value"] = "1";
  params["gasPrice"] = "2";
  params["gas"] = "21000";
  params["receiver"] = receiver.toString();

  const auto result = test_rpc.send_coin_transaction(params);
  EXPECT_FALSE(result.asString().empty());
  ASSERT_TRUE(*nonce_called);
  ASSERT_TRUE(*insert_called);
}

TEST_F(RPCTest, test_network_reads_use_network_reader) {
  auto peer_count_called = std::make_shared<bool>(false);
  auto all_nodes_called = std::make_shared<bool>(false);

  net::TestNetworkReader network_reader;
  network_reader.peer_count = [peer_count_called] {
    *peer_count_called = true;
    return uint64_t(3);
  };
  network_reader.all_nodes = [all_nodes_called] {
    *all_nodes_called = true;
    return std::vector<net::TestNetworkNodeView>{{"node-a", "127.0.0.1", 10002}, {"node-b", "127.0.0.2", 10003}};
  };

  net::Test test_rpc(nullptr, {}, {}, 1, std::move(network_reader));

  const auto peer_count = test_rpc.get_peer_count();
  EXPECT_EQ(Json::UInt64(3), peer_count["value"].asUInt64());
  ASSERT_TRUE(*peer_count_called);

  const auto all_nodes = test_rpc.get_all_nodes();
  EXPECT_EQ(Json::UInt64(2), all_nodes["nodes_count"].asUInt64());
  EXPECT_EQ("node-a", all_nodes["nodes"][0]["node_id"].asString());
  EXPECT_EQ("127.0.0.2", all_nodes["nodes"][1]["address"].asString());
  EXPECT_EQ(Json::UInt64(10003), all_nodes["nodes"][1]["listen_port"].asUInt64());
  ASSERT_TRUE(*all_nodes_called);
}

TEST_F(RPCTest, graphql_mutation_uses_transaction_api) {
  const auto trx = samples::createSignedTrxSamples(1, 1, secret_t::random()).front();
  auto insert_called = std::make_shared<bool>(false);

  graphql::taraxa::MutationTransactionApi transaction_api;
  transaction_api.insert_transaction = [insert_called, trx](const SharedTransaction& submitted_trx) {
    *insert_called = true;
    EXPECT_EQ(trx->getHash(), submitted_trx->getHash());
    return std::pair<bool, std::string>{true, ""};
  };

  graphql::taraxa::Mutation mutation(std::move(transaction_api));
  auto result = mutation.applySendRawTransaction(graphql::response::Value(dev::toJS(trx->rlp())));

  EXPECT_EQ(dev::toJS(trx->getHash()), result.get<std::string>());
  ASSERT_TRUE(*insert_called);
}

TEST_F(RPCTest, graphql_transaction_uses_receipt_reader) {
  const auto trx = samples::createSignedTrxSamples(1, 1, secret_t::random()).front();
  auto location_called = std::make_shared<bool>(false);
  auto receipt_called = std::make_shared<bool>(false);

  graphql::taraxa::TransactionReceiptReader reader;
  reader.location = [location_called, trx](const trx_hash_t& hash) {
    *location_called = true;
    EXPECT_EQ(trx->getHash(), hash);
    return std::optional<TransactionLocation>(TransactionLocation{9, 2, false});
  };
  reader.receipt = [receipt_called, trx](EthBlockNumber period, uint32_t position, const trx_hash_t& hash) {
    *receipt_called = true;
    EXPECT_EQ(9, period);
    EXPECT_EQ(2, position);
    EXPECT_EQ(trx->getHash(), hash);
    TransactionReceipt receipt;
    receipt.status_code = 1;
    receipt.gas_used = 21000;
    receipt.cumulative_gas_used = 42000;
    return std::optional<TransactionReceipt>(receipt);
  };

  graphql::taraxa::Transaction transaction(std::move(reader), nullptr, [](EthBlockNumber) { return nullptr; }, trx);

  EXPECT_EQ(2, transaction.getIndex());
  EXPECT_EQ(1, transaction.getStatus()->get<int>());
  EXPECT_EQ(21000, transaction.getGasUsed()->get<int>());
  EXPECT_EQ(42000, transaction.getCumulativeGasUsed()->get<int>());
  ASSERT_TRUE(*location_called);
  ASSERT_TRUE(*receipt_called);
}

TEST_F(RPCTest, eth_estimateGas) {
  auto node_cfg = make_isolated_node_cfgs(1);
  auto nodes = launch_nodes(node_cfg);
  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.chain_id = node_cfg.front().genesis.chain_id;
  eth_rpc_params.gas_limit = node_cfg.front().genesis.dag.gas_limit;
  eth_rpc_params.final_chain = nodes.front()->getFinalChain();
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  const auto from = dev::toHex(dev::toAddress(node_cfg.front().getFirstWallet().node_secret));
  auto check_estimation_is_in_range = [&](const Json::Value& trx, const std::string& e) {
    auto estimate = dev::jsToInt(eth_json_rpc->eth_estimateGas(trx, ""));
    auto expected = dev::jsToInt(e);
    EXPECT_GE(estimate, expected);
    EXPECT_GE(expected / 20, estimate - expected);
  };

  // Contract creation estimations with author + without author
  {
    Json::Value trx(Json::objectValue);
    trx["data"] = samples::greeter_contract_code;
    check_estimation_is_in_range(trx, "0x5dcc5");
    trx["from"] = from;
    check_estimation_is_in_range(trx, "0x5dcc5");
  }

  // Contract creation with value
  {
    Json::Value trx(Json::objectValue);
    trx["value"] = 1;
    trx["data"] = samples::greeter_contract_code;
    check_estimation_is_in_range(trx, "0x5dcc5");
  }

  // Simple transfer estimations with author + without author
  {
    Json::Value trx(Json::objectValue);
    trx["value"] = 1;
    trx["to"] = dev::toHex(addr_t::random());
    check_estimation_is_in_range(trx, "0x5208");  // 21k
    trx["from"] = from;
    check_estimation_is_in_range(trx, "0x5208");  // 21k
  }

  // Test throw on failed transaction
  {
    Json::Value trx(Json::objectValue);
    trx["value"] = 1000;
    trx["to"] = dev::toHex(addr_t::random());
    trx["from"] = dev::toHex(addr_t::random());
    EXPECT_THROW(eth_json_rpc->eth_estimateGas(trx, ""), std::exception);
  }
}

TEST_F(RPCTest, eth_call) {
  auto node_cfg = make_isolated_node_cfgs(1);
  auto nodes = launch_nodes(node_cfg);
  const auto final_chain = nodes.front()->getFinalChain();

  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.chain_id = node_cfg.front().genesis.chain_id;
  eth_rpc_params.gas_limit = node_cfg.front().genesis.dag.gas_limit;
  eth_rpc_params.final_chain = final_chain;
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  const auto last_block_num = final_chain->lastBlockNumber();
  const u256 total_eligible = final_chain->dposEligibleTotalVoteCount(last_block_num);
  const auto total_eligible_str = dev::toHexPrefixed(dev::toBigEndian(total_eligible));

  const auto empty_address = dev::KeyPair::create().address().toString();
  // check that balance of empty_address is 0
  ASSERT_EQ(eth_json_rpc->eth_getBalance(empty_address, dev::toCompactHexPrefixed(last_block_num)), "0x0");

  const std::string get_total_eligible_method("0xde8e4b50");
  const auto dpos_contract("0x00000000000000000000000000000000000000FE");
  {
    // Tested on Ethereum mainnet node. OK.
    // Sending some value to random address(not contract). from == zeroAddress
    // '{"method":"eth_call","params":[{"to":"0xeEA2524616B61E12c0Cb00a41dA78Ded1635F566","value":"0x100"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["to"] = empty_address;
    trx["value"] = "0x100";
    EXPECT_EQ(eth_json_rpc->eth_call(trx, "latest"), "0x");
  }

  {
    // Tested on Ethereum mainnet node.
    // ERROR: insufficient funds for gas * price + value.
    // Sending some value from account with no funds to other address.
    // '{"method":"eth_call","params":[{"from":"0x46dE41a622e679B8CDa5B76942a2e3Df5Ba023db","to":"0xeEA2524616B61E12c0Cb00a41dA78Ded1635F566","value":"0x100"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["from"] = empty_address;
    trx["to"] = dev::KeyPair::create().address().toString();
    trx["value"] = "0x100";
    EXPECT_THROW_WITH(eth_json_rpc->eth_call(trx, "latest"), jsonrpc::JsonRpcException,
                      "insufficient balance for transfer");
  }

  {
    // Tested on Ethereum mainnet node. OK
    // '{"method":"eth_call","params":[{"to":"0xdac17f958d2ee523a2206206994597c13d831ec7","data":"0x3eaaf86b"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["to"] = dpos_contract;
    trx["data"] = get_total_eligible_method;
    EXPECT_EQ(eth_json_rpc->eth_call(trx, "latest"), total_eligible_str);
  }

  {
    // Tested on Ethereum mainnet node. OK
    // '{"method":"eth_call","params":[{"to":"0xdac17f958d2ee523a2206206994597c13d831ec7","gas":"0x100000","data":"0x3eaaf86b"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["to"] = dpos_contract;
    trx["gas"] = "0x100000";
    trx["data"] = get_total_eligible_method;
    EXPECT_EQ(eth_json_rpc->eth_call(trx, "latest"), total_eligible_str);
  }

  {
    // Tested on Ethereum mainnet node. OK.
    // gas * gasPrice balance check eliminated for `from == ZeroAddress`.
    // '{"method":"eth_call","params":[{"to":"0xdac17f958d2ee523a2206206994597c13d831ec7","gas":"0x100000","gasPrice":"0x241268485270","data":"0x3eaaf86b"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["to"] = dpos_contract;
    trx["gas"] = "0x100000";
    trx["gasPrice"] = "0x241268485270";
    trx["data"] = get_total_eligible_method;
    EXPECT_EQ(eth_json_rpc->eth_call(trx, "latest"), total_eligible_str);
  }

  {
    // Tested on Ethereum mainnet node.
    // ERROR: insufficient funds for gas * price + value.
    // Sending from address with no funds, so can't pay gas * gas_price
    // '{"method":"eth_call","params":[{"from":"0xeEA2524616B61E12c0Cb00a41dA78Ded1635F566","to":"0xdac17f958d2ee523a2206206994597c13d831ec7","gas":"0x100000","gasPrice":"0x241268485270","data":"0x3eaaf86b"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["from"] = empty_address;
    trx["to"] = dpos_contract;
    trx["gas"] = "0x100000";
    trx["gasPrice"] = "0x241268485270";
    trx["data"] = get_total_eligible_method;
    EXPECT_THROW_WITH(eth_json_rpc->eth_call(trx, "latest"), jsonrpc::JsonRpcException,
                      "insufficient balance to pay for gas");
  }

  {
    // Tested on Ethereum mainnet node. OK.
    // Sending from address with no funds. Default `gasPrice == 0`, no funds needed
    // '{"method":"eth_call","params":[{"from":"0xeEA2524616B61E12c0Cb00a41dA78Ded1635F566","to":"0xdac17f958d2ee523a2206206994597c13d831ec7","data":"0x3eaaf86b"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["from"] = empty_address;
    trx["to"] = dpos_contract;
    trx["data"] = get_total_eligible_method;
    EXPECT_EQ(eth_json_rpc->eth_call(trx, "latest"), total_eligible_str);
  }

  {
    // Tested on Ethereum mainnet node. OK.
    // Sending from address with no funds. Default `gasPrice == 0`, no funds needed. Custom sufficient gas value is ok
    // '{"method":"eth_call","params":[{"from":"0xeEA2524616B61E12c0Cb00a41dA78Ded1635F566","to":"0xdac17f958d2ee523a2206206994597c13d831ec7","gas":"0x100000","data":"0x3eaaf86b"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["from"] = empty_address;
    trx["to"] = dpos_contract;
    trx["gas"] = "0x100000";
    trx["data"] = get_total_eligible_method;
    EXPECT_EQ(eth_json_rpc->eth_call(trx, "latest"), total_eligible_str);
  }

  {
    // Tested on Ethereum mainnet node.
    // ERROR: intrinsic gas too low
    // Sending from address with no funds. Default `gasPrice == 0`, no funds needed. Gas value lower then intrinsic gas
    // '{"method":"eth_call","params":[{"from":"0xeEA2524616B61E12c0Cb00a41dA78Ded1635F566","to":"0xdac17f958d2ee523a2206206994597c13d831ec7","gas":"0x1000","data":"0x3eaaf86b"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["from"] = empty_address;
    trx["to"] = dpos_contract;
    trx["gas"] = "0x1000";
    trx["data"] = get_total_eligible_method;
    EXPECT_THROW_WITH(eth_json_rpc->eth_call(trx, "latest"), jsonrpc::JsonRpcException, "intrinsic gas too low");
  }

  {
    // Tested on Ethereum mainnet node.
    // ERROR: out of gas
    // Sending from address with no funds. Default `gasPrice == 0`, no funds needed.
    // Gas value lower then needed intrinsic gas, but lower then total required gas
    // '{"method":"eth_call","params":[{"from":"0xeEA2524616B61E12c0Cb00a41dA78Ded1635F566","to":"0xdac17f958d2ee523a2206206994597c13d831ec7","gas":"0x5250","data":"0x3eaaf86b"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["from"] = empty_address;
    trx["to"] = dpos_contract;
    trx["gas"] = "0x5330";
    trx["data"] = get_total_eligible_method;
    EXPECT_THROW_WITH(eth_json_rpc->eth_call(trx, "latest"), jsonrpc::JsonRpcException, "out of gas");
  }

  {
    // Tested on Ethereum mainnet node. OK.
    // Sending from address with no funds. default `gasPrice == 0`, so no funds needed. Ok with custom gas value
    // '{"method":"eth_call","params":[{"from":"0xeEA2524616B61E12c0Cb00a41dA78Ded1635F566","to":"0xdac17f958d2ee523a2206206994597c13d831ec7","gas":"0x100000","data":"0x3eaaf86b"},"latest"]}'
    Json::Value trx(Json::objectValue);
    trx["from"] = empty_address;
    trx["to"] = dpos_contract;
    trx["gas"] = "0x100000";
    trx["data"] = get_total_eligible_method;
    EXPECT_EQ(eth_json_rpc->eth_call(trx, "latest"), total_eligible_str);
  }
}

TEST_F(RPCTest, eth_getBlock) {
  auto node_cfg = make_isolated_node_cfgs(1, 1, 10);
  // Enable rewards distribution
  node_cfg[0].genesis.state.dpos.yield_percentage = 10;
  node_cfg[0].genesis.state.hardforks.cacti_hf.block_num = -1;
  auto nodes = launch_nodes(node_cfg);
  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.chain_id = node_cfg.front().genesis.chain_id;
  eth_rpc_params.gas_limit = node_cfg.front().genesis.dag.gas_limit;
  eth_rpc_params.final_chain = nodes.front()->getFinalChain();
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  wait({10s, 500ms}, [&](auto& ctx) { WAIT_EXPECT_EQ(ctx, 5, nodes[0]->getFinalChain()->lastBlockNumber()); });
  auto block = eth_json_rpc->eth_getBlockByNumber("0x4", false);

  EXPECT_EQ(4, dev::jsToU256(block["number"].asString()));
  EXPECT_GT(dev::jsToU256(block["totalReward"].asString()), 0);
}

TEST_F(RPCTest, eip_1898) {
  auto node_cfg = make_isolated_node_cfgs(1);
  auto nodes = launch_nodes(node_cfg);
  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.chain_id = node_cfg.front().genesis.chain_id;
  eth_rpc_params.gas_limit = node_cfg.front().genesis.dag.gas_limit;
  eth_rpc_params.final_chain = nodes.front()->getFinalChain();
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  const auto from = dev::toHex(dev::toAddress(node_cfg.front().getFirstWallet().node_secret));

  Json::Value zero_block(Json::objectValue);
  zero_block["blockNumber"] = dev::toJS(0);
  EXPECT_EQ(eth_json_rpc->eth_getBalance(from, "0x0"), eth_json_rpc->eth_getBalance(from, zero_block));

  Json::Value genesis_block(Json::objectValue);
  genesis_block["blockHash"] = dev::toJS(*nodes.front()->getFinalChain()->blockHash(0));
  EXPECT_EQ(eth_json_rpc->eth_getBalance(from, "0x0"), eth_json_rpc->eth_getBalance(from, genesis_block));
}

#ifdef RUSTAXA_ENABLE
TEST_F(RPCTest, eth_account_state_uses_query_callbacks) {
  const auto address = dev::KeyPair::create().address();
  const auto block_hash = h256(0x1234);
  constexpr EthBlockNumber kLatestBlock = 11;
  constexpr EthBlockNumber kHashBlock = 7;

  state_api::Account account;
  account.balance = 12345;
  account.nonce = 9;
  account.storage_root_hash = h256(0x55);

  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.query_final_chain_last_block_number = [] { return kLatestBlock; };
  eth_rpc_params.query_final_chain_block_number_by_hash = [block_hash](const h256& hash) {
    rustaxa::FinalChainBlockNumberLookup lookup;
    lookup.found = hash == block_hash;
    lookup.value = kHashBlock;
    return lookup;
  };
  eth_rpc_params.query_account = [address, account](const Address& requested_address, EthBlockNumber block_number) {
    EXPECT_EQ(address, requested_address);
    EXPECT_TRUE(block_number == kLatestBlock || block_number == kHashBlock);
    return std::optional<state_api::Account>(account);
  };
  eth_rpc_params.query_account_storage = [address, kLatestBlock](const Address& requested_address, const u256& key,
                                                                 EthBlockNumber block_number) {
    EXPECT_EQ(address, requested_address);
    EXPECT_EQ(u256(3), key);
    EXPECT_EQ(kLatestBlock, block_number);
    return h256(0x99);
  };
  eth_rpc_params.query_account_code = [address, kLatestBlock](const Address& requested_address,
                                                              EthBlockNumber block_number) {
    EXPECT_EQ(address, requested_address);
    EXPECT_EQ(kLatestBlock, block_number);
    return bytes{0x60, 0x01};
  };
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  EXPECT_EQ(dev::toJS(account.balance), eth_json_rpc->eth_getBalance(address.toString(), "latest"));
  EXPECT_EQ(dev::toJS(account.nonce), eth_json_rpc->eth_getTransactionCount(address.toString(), "latest"));
  EXPECT_EQ(dev::toJS(account.storage_root_eth()), eth_json_rpc->eth_getStorageRoot(address.toString(), "latest"));
  EXPECT_EQ(dev::toJS(h256(0x99)), eth_json_rpc->eth_getStorageAt(address.toString(), "0x3", "latest"));
  EXPECT_EQ(dev::toJS(bytes{0x60, 0x01}), eth_json_rpc->eth_getCode(address.toString(), "latest"));

  Json::Value hash_block(Json::objectValue);
  hash_block["blockHash"] = dev::toJS(block_hash);
  EXPECT_EQ(dev::toJS(account.balance), eth_json_rpc->eth_getBalance(address.toString(), hash_block));
}

TEST_F(RPCTest, eth_installed_log_filter_uses_log_replay_api) {
  const auto log_address = dev::KeyPair::create().address();
  const auto topic = h256(0x2222);
  const auto block_hash = h256(0x3333);
  constexpr EthBlockNumber kLatestBlock = 5;
  constexpr EthBlockNumber kFilterBlock = 2;

  const auto trx = samples::createSignedTrxSamples(0, 0, secret_t::random()).front();
  TransactionReceipt receipt;
  receipt.status_code = 1;
  receipt.gas_used = 21000;
  receipt.cumulative_gas_used = 21000;
  receipt.logs.push_back(LogEntry{log_address, {topic}, bytes{0xaa, 0xbb}});

  auto to_rust_bytes = [](const bytes& input) {
    rust::Vec<uint8_t> output;
    for (const auto byte : input) {
      output.push_back(byte);
    }
    return output;
  };

  const auto trx_hash = trx->getHash();
  const auto trx_rlp = trx->rlp();
  const auto receipt_rlp = util::rlp_enc(receipt);

  auto bloom_queries = std::make_shared<size_t>(0);
  auto receipt_queries = std::make_shared<size_t>(0);
  net::rpc::eth::EthParams eth_rpc_params;
  net::rpc::eth::FinalizedLogReplayApi log_replay_api;
  log_replay_api.latest_finalized_block_number = [] { return kLatestBlock; };
  log_replay_api.blocks_with_bloom = [bloom_queries, kFilterBlock](const std::array<uint8_t, 256>&, EthBlockNumber from,
                                                                   EthBlockNumber to) {
    ++*bloom_queries;
    EXPECT_EQ(kFilterBlock, from);
    EXPECT_EQ(kFilterBlock, to);
    rust::Vec<uint64_t> blocks;
    blocks.push_back(kFilterBlock);
    return blocks;
  };
  log_replay_api.transaction_receipts_by_block_number = [receipt_queries, trx_hash, block_hash, trx_rlp, receipt_rlp,
                                                         to_rust_bytes, kLatestBlock](EthBlockNumber requested_block) {
    ++*receipt_queries;
    rust::Vec<rustaxa::TransactionReceiptPublicView> receipts;
    if (requested_block == kFilterBlock) {
      rustaxa::TransactionReceiptPublicView receipt_view;
      receipt_view.found = true;
      receipt_view.transaction_hash = trx_hash.asArray();
      receipt_view.transaction_source = 2;
      receipt_view.transaction_rlp = to_rust_bytes(trx_rlp);
      receipt_view.receipt_rlp = to_rust_bytes(receipt_rlp);
      receipt_view.block_number = kFilterBlock;
      receipt_view.transaction_index = 0;
      receipt_view.is_system = false;
      receipt_view.block_hash_found = true;
      receipt_view.block_hash = block_hash.asArray();
      receipts.push_back(std::move(receipt_view));
    } else {
      EXPECT_EQ(kLatestBlock, requested_block);
    }
    return receipts;
  };
  eth_rpc_params.query_log_replay = std::move(log_replay_api);
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  Json::Value filter(Json::objectValue);
  filter["fromBlock"] = dev::toJS(kFilterBlock);
  filter["toBlock"] = dev::toJS(kFilterBlock);
  filter["address"] = dev::toJS(log_address);
  filter["topics"] = Json::Value(Json::arrayValue);
  filter["topics"].append(dev::toJS(topic));

  const auto filter_id = eth_json_rpc->eth_newFilter(filter);
  auto logs = eth_json_rpc->eth_getFilterLogs(filter_id);
  ASSERT_EQ(1, logs.size());
  EXPECT_EQ(dev::toJS(log_address), logs[0]["address"].asString());
  EXPECT_EQ(dev::toJS(topic), logs[0]["topics"][0].asString());
  EXPECT_EQ(dev::toJS(bytes{0xaa, 0xbb}), logs[0]["data"].asString());
  EXPECT_EQ(dev::toJS(kFilterBlock), logs[0]["blockNumber"].asString());
  EXPECT_EQ(dev::toJS(block_hash), logs[0]["blockHash"].asString());
  EXPECT_EQ(dev::toJS(trx->getHash()), logs[0]["transactionHash"].asString());
  EXPECT_EQ(1, *bloom_queries);
  EXPECT_EQ(1, *receipt_queries);

  Json::Value default_from_filter(Json::objectValue);
  const auto default_filter_id = eth_json_rpc->eth_newFilter(default_from_filter);
  EXPECT_TRUE(eth_json_rpc->eth_getFilterLogs(default_filter_id).empty());
  EXPECT_EQ(1, *bloom_queries);
  EXPECT_EQ(2, *receipt_queries);
}
#endif

TEST_F(RPCTest, graphql_account_uses_query_callbacks) {
  const auto address = dev::KeyPair::create().address();
  constexpr EthBlockNumber kLatestBlock = 9;
  constexpr EthBlockNumber kAccountBlock = 3;

  state_api::Account account;
  account.balance = 1001;
  account.nonce = 17;

  graphql::taraxa::AccountStateReader reader;
  reader.account_at = [address, account, kAccountBlock](const dev::Address& requested_address,
                                                        std::optional<EthBlockNumber> block_number) {
    EXPECT_EQ(address, requested_address);
    EXPECT_EQ(kAccountBlock, block_number.value());
    return std::optional<state_api::Account>(account);
  };
  reader.storage_at = [address, kAccountBlock](const dev::Address& requested_address, const dev::u256& key,
                                               std::optional<EthBlockNumber> block_number) {
    EXPECT_EQ(address, requested_address);
    EXPECT_EQ(dev::u256(4), key);
    EXPECT_EQ(kAccountBlock, block_number.value());
    return dev::h256(0x44);
  };
  reader.code_at = [address, kAccountBlock](const dev::Address& requested_address,
                                            std::optional<EthBlockNumber> block_number) {
    EXPECT_EQ(address, requested_address);
    EXPECT_EQ(kAccountBlock, block_number.value());
    return dev::bytes{0x60, 0x02};
  };
  reader.latest_finalized_block_number = [] { return kLatestBlock; };

  graphql::taraxa::Account graphql_account(std::move(reader), address, kAccountBlock);
  EXPECT_EQ(dev::toJS(account.balance), graphql_account.getBalance().get<std::string>());
  EXPECT_EQ(static_cast<int>(account.nonce), graphql_account.getTransactionCount().get<int>());
  EXPECT_EQ(dev::toJS(dev::bytes{0x60, 0x02}), graphql_account.getCode().get<std::string>());
  EXPECT_EQ(dev::toJS(dev::h256(0x44)), graphql_account.getStorage(graphql::response::Value("0x4")).get<std::string>());
}

TEST_F(RPCTest, graphql_log_account_uses_account_reader) {
  const auto log_address = dev::KeyPair::create().address();
  auto account_called = std::make_shared<bool>(false);

  state_api::Account account;
  account.balance = 5;

  graphql::taraxa::AccountStateReader reader;
  reader.account_at = [account_called, log_address, account](const dev::Address& requested_address,
                                                             std::optional<EthBlockNumber> block_number) {
    *account_called = true;
    EXPECT_EQ(log_address, requested_address);
    EXPECT_FALSE(block_number.has_value());
    return std::optional<state_api::Account>(account);
  };
  reader.storage_at = [](const dev::Address&, const dev::u256&, std::optional<EthBlockNumber>) { return dev::h256(); };
  reader.code_at = [](const dev::Address&, std::optional<EthBlockNumber>) { return dev::bytes{}; };
  reader.latest_finalized_block_number = [] { return EthBlockNumber(0); };

  LogEntry log;
  log.address = log_address;
  graphql::taraxa::Log graphql_log(std::move(reader), nullptr, std::move(log), 0);

  ASSERT_NE(nullptr, graphql_log.getAccount(std::nullopt));
  ASSERT_TRUE(*account_called);
}

TEST_F(RPCTest, graphql_block_accounts_use_account_reader) {
  const auto miner = dev::KeyPair::create().address();
  const auto queried_address = dev::KeyPair::create().address();
  auto account_calls = std::make_shared<int>(0);

  state_api::Account account;
  account.balance = 7;

  graphql::taraxa::AccountStateReader reader;
  reader.account_at = [account_calls, miner, queried_address, account](const dev::Address& requested_address,
                                                                       std::optional<EthBlockNumber> block_number) {
    ++*account_calls;
    if (*account_calls == 1) {
      EXPECT_EQ(miner, requested_address);
      EXPECT_FALSE(block_number.has_value());
    } else {
      EXPECT_EQ(queried_address, requested_address);
      EXPECT_EQ(EthBlockNumber(12), block_number.value());
    }
    return std::optional<state_api::Account>(account);
  };
  reader.storage_at = [](const dev::Address&, const dev::u256&, std::optional<EthBlockNumber>) { return dev::h256(); };
  reader.code_at = [](const dev::Address&, std::optional<EthBlockNumber>) { return dev::bytes{}; };
  reader.latest_finalized_block_number = [] { return EthBlockNumber(0); };

  auto header = std::make_shared<final_chain::BlockHeader>();
  header->author = miner;
  header->number = 12;

  graphql::taraxa::Block block(
      std::move(reader), nullptr, [](EthBlockNumber) { return nullptr; }, blk_hash_t(1), header);

  ASSERT_NE(nullptr, block.getMiner(std::nullopt));
  ASSERT_NE(nullptr, block.getAccount(graphql::response::Value(queried_address.toString())));
  EXPECT_EQ(2, *account_calls);
}

TEST_F(RPCTest, graphql_block_transactions_use_transaction_reader) {
  auto transaction = samples::createSignedTrxSamples(1, 1, secret_t::random()).front();
  auto count_called = std::make_shared<bool>(false);
  auto transactions_called = std::make_shared<bool>(false);

  graphql::taraxa::AccountStateReader account_reader;
  account_reader.account_at = [](const dev::Address&, std::optional<EthBlockNumber>) {
    return std::optional<state_api::Account>{};
  };
  account_reader.storage_at = [](const dev::Address&, const dev::u256&, std::optional<EthBlockNumber>) {
    return dev::h256();
  };
  account_reader.code_at = [](const dev::Address&, std::optional<EthBlockNumber>) { return dev::bytes{}; };
  account_reader.latest_finalized_block_number = [] { return EthBlockNumber(0); };

  graphql::taraxa::BlockTransactionReader transaction_reader;
  transaction_reader.transaction_count = [count_called](EthBlockNumber block_number) {
    *count_called = true;
    EXPECT_EQ(EthBlockNumber(18), block_number);
    return uint64_t(1);
  };
  transaction_reader.transactions = [transactions_called, transaction](EthBlockNumber block_number) {
    *transactions_called = true;
    EXPECT_EQ(EthBlockNumber(18), block_number);
    return std::vector<std::shared_ptr<Transaction>>{transaction};
  };

  auto header = std::make_shared<final_chain::BlockHeader>();
  header->number = 18;

  graphql::taraxa::Block block(
      std::move(account_reader), std::move(transaction_reader), nullptr, [](EthBlockNumber) { return nullptr; },
      blk_hash_t(2), header);

  EXPECT_EQ(1, block.getTransactionCount().value());
  const auto graphql_transaction = block.getTransactionAt(graphql::response::IntType(0));
  ASSERT_NE(nullptr, graphql_transaction);
  ASSERT_TRUE(*count_called);
  ASSERT_TRUE(*transactions_called);
}

TEST_F(RPCTest, graphql_dag_block_author_uses_account_reader) {
  auto dag_block = std::make_shared<DagBlock>(blk_hash_t(1), level_t(1), vec_blk_t{}, vec_trx_t{}, secret_t::random());
  const auto author = dag_block->getSender();
  auto account_called = std::make_shared<bool>(false);

  state_api::Account account;
  account.balance = 9;

  graphql::taraxa::AccountStateReader reader;
  reader.account_at = [account_called, author, account](const dev::Address& requested_address,
                                                        std::optional<EthBlockNumber> block_number) {
    *account_called = true;
    EXPECT_EQ(author, requested_address);
    EXPECT_FALSE(block_number.has_value());
    return std::optional<state_api::Account>(account);
  };
  reader.storage_at = [](const dev::Address&, const dev::u256&, std::optional<EthBlockNumber>) { return dev::h256(); };
  reader.code_at = [](const dev::Address&, std::optional<EthBlockNumber>) { return dev::bytes{}; };
  reader.latest_finalized_block_number = [] { return EthBlockNumber(0); };

  graphql::taraxa::DagBlock graphql_dag_block(std::move(reader), std::move(dag_block), nullptr, nullptr,
                                              [](EthBlockNumber) { return nullptr; });

  ASSERT_NE(nullptr, graphql_dag_block.getAuthor());
  ASSERT_TRUE(*account_called);
}

TEST_F(RPCTest, graphql_dag_block_transactions_use_transaction_reader) {
  auto transaction = samples::createSignedTrxSamples(1, 1, secret_t::random()).front();
  const auto transaction_hash = transaction->getHash();
  auto transaction_called = std::make_shared<bool>(false);

  auto dag_block = std::make_shared<DagBlock>(blk_hash_t(1), level_t(1), vec_blk_t{}, vec_trx_t{transaction_hash},
                                              secret_t::random());

  graphql::taraxa::AccountStateReader account_reader;
  account_reader.account_at = [](const dev::Address&, std::optional<EthBlockNumber>) {
    return std::optional<state_api::Account>{};
  };
  account_reader.storage_at = [](const dev::Address&, const dev::u256&, std::optional<EthBlockNumber>) {
    return dev::h256();
  };
  account_reader.code_at = [](const dev::Address&, std::optional<EthBlockNumber>) { return dev::bytes{}; };
  account_reader.latest_finalized_block_number = [] { return EthBlockNumber(0); };

  graphql::taraxa::DagBlockTransactionReader transaction_reader;
  transaction_reader.transaction_by_hash = [transaction_called, transaction,
                                            transaction_hash](const trx_hash_t& requested_hash) {
    *transaction_called = true;
    EXPECT_EQ(transaction_hash, requested_hash);
    return transaction;
  };

  graphql::taraxa::DagBlock graphql_dag_block(std::move(account_reader), std::move(transaction_reader),
                                              std::move(dag_block), nullptr, nullptr,
                                              [](EthBlockNumber) { return nullptr; });

  const auto transactions = graphql_dag_block.getTransactions();
  ASSERT_TRUE(transactions.has_value());
  ASSERT_EQ(1, transactions->size());
  ASSERT_TRUE(*transaction_called);
}

TEST_F(RPCTest, graphql_dag_block_period_uses_period_reader) {
  auto dag_block = std::make_shared<DagBlock>(blk_hash_t(7), level_t(1), vec_blk_t{}, vec_trx_t{}, secret_t::random());
  const auto dag_block_hash = dag_block->getHash();
  auto period_called = std::make_shared<bool>(false);

  graphql::taraxa::AccountStateReader account_reader;
  account_reader.account_at = [](const dev::Address&, std::optional<EthBlockNumber>) {
    return std::optional<state_api::Account>{};
  };
  account_reader.storage_at = [](const dev::Address&, const dev::u256&, std::optional<EthBlockNumber>) {
    return dev::h256();
  };
  account_reader.code_at = [](const dev::Address&, std::optional<EthBlockNumber>) { return dev::bytes{}; };
  account_reader.latest_finalized_block_number = [] { return EthBlockNumber(0); };

  graphql::taraxa::DagBlockPeriodReader period_reader;
  period_reader.period_by_hash = [period_called, dag_block_hash](const blk_hash_t& requested_hash) {
    *period_called = true;
    EXPECT_EQ(dag_block_hash, requested_hash);
    return std::optional<uint64_t>(23);
  };

  graphql::taraxa::DagBlock graphql_dag_block(std::move(account_reader), graphql::taraxa::DagBlockTransactionReader{},
                                              std::move(period_reader), std::move(dag_block), nullptr, nullptr,
                                              [](EthBlockNumber) { return nullptr; });

  const auto period = graphql_dag_block.getPbftPeriod();
  ASSERT_TRUE(period.has_value());
  EXPECT_EQ(23, period->get<int>());
  ASSERT_TRUE(*period_called);
}

TEST_F(RPCTest, graphql_query_account_uses_account_reader) {
  const auto address = dev::KeyPair::create().address();
  auto account_called = std::make_shared<bool>(false);

  state_api::Account account;
  account.balance = 11;

  graphql::taraxa::AccountStateReader reader;
  reader.account_at = [account_called, address, account](const dev::Address& requested_address,
                                                         std::optional<EthBlockNumber> block_number) {
    *account_called = true;
    EXPECT_EQ(address, requested_address);
    EXPECT_EQ(EthBlockNumber(15), block_number.value());
    return std::optional<state_api::Account>(account);
  };
  reader.storage_at = [](const dev::Address&, const dev::u256&, std::optional<EthBlockNumber>) { return dev::h256(); };
  reader.code_at = [](const dev::Address&, std::optional<EthBlockNumber>) { return dev::bytes{}; };
  reader.latest_finalized_block_number = [] { return EthBlockNumber(0); };

  graphql::taraxa::Query query(std::move(reader));

  ASSERT_NE(nullptr, query.getAccount(graphql::response::Value(address.toString()), graphql::response::Value(15)));
  ASSERT_TRUE(*account_called);
}

TEST_F(RPCTest, graphql_query_dag_blocks_use_query_dag_block_reader) {
  auto transaction = samples::createSignedTrxSamples(1, 1, secret_t::random()).front();
  const auto transaction_hash = transaction->getHash();
  auto level_four_block = std::make_shared<DagBlock>(blk_hash_t(4), level_t(4), vec_blk_t{},
                                                     vec_trx_t{transaction_hash}, secret_t::random());
  auto level_five_block =
      std::make_shared<DagBlock>(blk_hash_t(5), level_t(5), vec_blk_t{}, vec_trx_t{}, secret_t::random());
  const auto level_four_hash = level_four_block->getHash();

  auto by_hash_called = std::make_shared<bool>(false);
  auto latest_level_called = std::make_shared<int>(0);
  auto latest_period_called = std::make_shared<bool>(false);
  auto level_requests = std::make_shared<std::vector<level_t>>();
  auto finalized_period_called = std::make_shared<bool>(false);
  auto transaction_called = std::make_shared<bool>(false);
  auto period_called = std::make_shared<bool>(false);

  graphql::taraxa::AccountStateReader account_reader;
  account_reader.account_at = [](const dev::Address&, std::optional<EthBlockNumber>) {
    return std::optional<state_api::Account>{};
  };
  account_reader.storage_at = [](const dev::Address&, const dev::u256&, std::optional<EthBlockNumber>) {
    return dev::h256();
  };
  account_reader.code_at = [](const dev::Address&, std::optional<EthBlockNumber>) { return dev::bytes{}; };
  account_reader.latest_finalized_block_number = [] { return EthBlockNumber(0); };

  graphql::taraxa::QueryDagBlockReader dag_reader;
  dag_reader.block_by_hash = [by_hash_called, level_four_block, level_four_hash](const blk_hash_t& requested_hash) {
    *by_hash_called = true;
    EXPECT_EQ(level_four_hash, requested_hash);
    return level_four_block;
  };
  dag_reader.latest_level = [latest_level_called] {
    ++*latest_level_called;
    return level_t(5);
  };
  dag_reader.latest_finalized_period = [latest_period_called] {
    *latest_period_called = true;
    return uint64_t(9);
  };
  dag_reader.blocks_by_level = [level_requests, level_four_block, level_five_block](level_t requested_level) {
    level_requests->push_back(requested_level);
    if (requested_level == level_t(4)) {
      return std::vector<std::shared_ptr<DagBlock>>{level_four_block};
    }
    if (requested_level == level_t(5)) {
      return std::vector<std::shared_ptr<DagBlock>>{level_five_block};
    }
    return std::vector<std::shared_ptr<DagBlock>>{};
  };
  dag_reader.finalized_blocks_by_period = [finalized_period_called, level_four_block](uint64_t requested_period) {
    *finalized_period_called = true;
    EXPECT_EQ(uint64_t(9), requested_period);
    return std::vector<std::shared_ptr<DagBlock>>{level_four_block};
  };

  graphql::taraxa::DagBlockTransactionReader transaction_reader;
  transaction_reader.transaction_by_hash = [transaction_called, transaction,
                                            transaction_hash](const trx_hash_t& requested_hash) {
    *transaction_called = true;
    EXPECT_EQ(transaction_hash, requested_hash);
    return transaction;
  };

  graphql::taraxa::DagBlockPeriodReader period_reader;
  period_reader.period_by_hash = [period_called, level_four_hash](const blk_hash_t& requested_hash) {
    *period_called = true;
    EXPECT_EQ(level_four_hash, requested_hash);
    return std::optional<uint64_t>(9);
  };

  graphql::taraxa::Query query(std::move(account_reader), 0, std::move(dag_reader), std::move(transaction_reader),
                               std::move(period_reader));

  ASSERT_NE(nullptr, query.getDagBlock(graphql::response::Value(level_four_hash.toString())));
  ASSERT_TRUE(*by_hash_called);

  ASSERT_NE(nullptr, query.getDagBlock(std::nullopt));
  ASSERT_EQ(1, *latest_level_called);
  ASSERT_EQ(std::vector<level_t>{level_t(5)}, *level_requests);

  const auto period_blocks = query.getPeriodDagBlocks(std::nullopt);
  ASSERT_EQ(1, period_blocks.size());
  ASSERT_TRUE(*latest_period_called);
  ASSERT_TRUE(*finalized_period_called);

  const auto level_blocks =
      query.getDagBlocks(graphql::response::Value(4), std::optional<int>(2), std::optional<bool>(false));
  ASSERT_EQ(2, level_blocks.size());
  ASSERT_EQ(2, *latest_level_called);
  ASSERT_EQ((std::vector<level_t>{level_t(5), level_t(4), level_t(5)}), *level_requests);
}

TEST_F(RPCTest, transaction_json) {
  auto nonce = 0;
  auto trx = std::make_shared<Transaction>(nonce, 100, 1000000000, 100000, dev::bytes(),
                                           dev::KeyPair::create().secret(), dev::KeyPair::create().address(), 841);
  const auto loc = net::rpc::eth::TransactionLocationWithBlockHash{TransactionLocation{1, 1}, h256(123)};
  const auto json = toJson(*trx, loc);

  EXPECT_EQ(json["blockHash"], dev::toJS(loc.blk_h));
  EXPECT_EQ(json["blockNumber"], dev::toJS(loc.period));
  EXPECT_EQ(json["transactionIndex"], dev::toJS(loc.position));
  EXPECT_EQ(json["from"], dev::toJS(trx->getSender()));
  EXPECT_EQ(json["gas"], dev::toJS(trx->getGas()));
  EXPECT_EQ(json["gasPrice"], dev::toJS(trx->getGasPrice()));
  EXPECT_EQ(json["hash"], dev::toJS(trx->getHash()));
  EXPECT_EQ(json["input"], dev::toJS(trx->getData()));
  EXPECT_EQ(json["nonce"], dev::toJS(trx->getNonce()));
  EXPECT_EQ(json["to"], dev::toJS(*trx->getReceiver()));
  EXPECT_EQ(json["value"], dev::toJS(trx->getValue()));
  EXPECT_EQ(json["v"], dev::toJS(trx->getVRS().v));
  EXPECT_EQ(json["r"], dev::toJS(u256(trx->getVRS().r)));
  EXPECT_EQ(json["s"], dev::toJS(u256(trx->getVRS().s)));
  EXPECT_EQ(json["chainId"], dev::toJS(trx->getChainID()));
}

TEST_F(RPCTest, u256_h256_serialization) {
  auto str = std::string("0x09cf8cb3d2b55fcbddc997b8669dd37a84699886ea2e9d7c88217c8443cfa8b0");
  h256 val(str);
  EXPECT_EQ(dev::toJS(val), str);
  // u256 should be serialized without leading 0
  EXPECT_NE(dev::toJS(dev::u256(val)), str);
  EXPECT_EQ(dev::toJS(dev::u256(val)).size(), str.size() - 1);
}

}  // namespace taraxa::core_tests

using namespace taraxa;
int main(int argc, char** argv) {
  taraxa::static_init();

  auto logging = logger::createDefaultLoggingConfig();
  logging.verbosity = logger::Verbosity::Error;
  addr_t node_addr;
  logging.InitLogging(node_addr);

  ::testing::InitGoogleTest(&argc, argv);
  return RUN_ALL_TESTS();
}
