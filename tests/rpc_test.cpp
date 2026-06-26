#include <gtest/gtest.h>
#include <jsonrpccpp/common/exception.h>
#include <libdevcore/Common.h>
#include <libdevcore/CommonJS.h>

#include <sstream>

#include "common/encoding_rlp.hpp"
#include "graphql/account.hpp"
#include "graphql/mutation.hpp"
#include "graphql/sync_state.hpp"
#include "network/rpc/Debug.h"
#include "network/rpc/Taraxa.h"
#include "network/rpc/Test.h"
#include "network/rpc/eth/Eth.h"
#include "network/rpc/eth/LiveLogSubscription.hpp"
#include "network/subscriptions.hpp"
#include "test_util/samples.hpp"

namespace taraxa::core_tests {

struct RPCTest : NodesTest {};

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

TEST_F(RPCTest, eth_estimateGas) {
  auto node_cfg = make_node_cfgs(1);
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
  auto node_cfg = make_node_cfgs(1);
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
  auto node_cfg = make_node_cfgs(1, 1, 10);
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
  auto node_cfg = make_node_cfgs(1);
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
