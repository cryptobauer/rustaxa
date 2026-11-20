#include <algorithm>
#include <array>
#include <cstdint>
#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <iterator>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#include "app/production_consensus_application.hpp"
#include "cli/config.hpp"
#include "common/jsoncpp.hpp"
#include "consensus/native_config_projection.hpp"
#include "libdevcrypto/Common.h"
#include "rustaxa-bridge/src/ffi.rs.h"
#include "rustaxa-bridge/src/production_ffi.rs.h"
#include "rustaxa-native-config/src/lib.rs.h"
#include "transaction/transaction.hpp"

#ifdef RUSTAXA_ACTUAL_APP_TEST
#include <chrono>
#include <thread>

#include "app/app.hpp"
#include "consensus/consensus_application.hpp"
#include "network/rpc/Debug.h"
#endif

namespace {
void require_task_path(const std::filesystem::path& root, const std::filesystem::path& path) {
  const auto expected = std::filesystem::weakly_canonical(root);
  const auto actual = std::filesystem::weakly_canonical(path);
  const auto mismatch = std::mismatch(expected.begin(), expected.end(), actual.begin(), actual.end());
  if (mismatch.first != expected.end() || actual == expected) {
    throw std::runtime_error("PRODUCTION_TEST_DATABASE_OUTSIDE_TASK_ROOT");
  }
}

rust::String check_common_clients(const rustaxa::BridgeConsensusApplication& application,
                                  const taraxa::FullNodeConfig& config) {
  auto query = rustaxa::create_consensus_query_api(application);
  auto network = rustaxa::create_consensus_network_api(application);
  if (query->consensus_query_final_chain_last_block_number() != 0 ||
      query->consensus_query_live_transaction_status().queue_size != 0)
    throw std::runtime_error("COMMON_CLIENT_INITIAL_STATE_MISMATCH");
  const auto& policy = config.genesis.state.hardforks.soleirolia_hf;
  const taraxa::val_t price = taraxa::val_t(policy.trx_min_gas_price) + 1;
  const taraxa::Transaction transaction(0, 1, price, 21000, {}, config.getFirstWallet().node_secret,
                                        config.getFirstWallet().node_addr, config.genesis.chain_id);
  const auto request = [&]() {
    rustaxa::PublicTransactionSubmissionRequest value;
    for (const auto byte : transaction.rlp()) value.transaction_rlp.push_back(byte);
    value.expected_chain_id = config.genesis.chain_id;
    value.maximum_gas_limit = policy.trx_max_gas_limit;
    value.minimum_gas_price = taraxa::production_config_detail::toBridgeU256(taraxa::val_t(policy.trx_min_gas_price));
    value.last_block_number = 0;
    value.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(0);
    return value;
  };
  bool rejected = false;
  try {
    auto wrong = request();
    ++wrong.maximum_gas_limit;
    rustaxa::consensus_application_submit_transaction(application, std::move(wrong));
  } catch (const rust::Error&) {
    rejected = true;
  }
  if (!rejected || query->consensus_query_live_transaction_status().queue_size != 0)
    throw std::runtime_error("COMMON_CLIENT_CONFIG_MISMATCH_NOT_REJECTED");
  const auto admitted = rustaxa::consensus_application_submit_transaction(application, request());
  if (!admitted.accepted || !admitted.transaction_observed ||
      query->consensus_query_live_transaction_status().queue_size != 1)
    throw std::runtime_error("COMMON_CLIENT_TRANSACTION_NOT_ADMITTED");
  rustaxa::consensus_network_submit_transaction(*network, request());
  if (rustaxa::consensus_network_transaction_gas_price_bid(*network) !=
      query->consensus_query_live_transaction_status().gas_price_bid)
    throw std::runtime_error("COMMON_CLIENT_GAS_BID_MISMATCH");
  // The packet entry must link without the application host unit.
  auto volatile packet_entry = &rustaxa::consensus_network_ingest_transaction_packet;
  if (!packet_entry) throw std::runtime_error("COMMON_CLIENT_PACKET_ENTRY_MISSING");
  network->consensus_network_drain_work(0, 0, false, 1);
  if (query->consensus_query_live_transaction_status().queue_size != 1 ||
      query->consensus_query_final_chain_last_block_number() != 0)
    throw std::runtime_error("COMMON_CLIENT_DUPLICATE_OR_HEAD_MISMATCH");
  bool legacy_rejected = false;
  try {
    rustaxa::consensus_application_prune_light_history(application, {});
  } catch (const rust::Error& error) {
    legacy_rejected = std::string(error.what()).find("PRODUCTION_LEGACY_OPERATION_UNAVAILABLE") != std::string::npos;
  }
  if (!legacy_rejected) throw std::runtime_error("COMMON_CLIENT_LEGACY_ROUTE_AVAILABLE");
  return rust::String("COMMON_PRODUCTION_CLIENTS_PASSED");
}

using rustaxa::AppFixtureDiskStep;
using rustaxa::AppFixtureLiveStep;

class FixtureConfig : public taraxa::cli::Config {
 public:
  void schedulePillarTest(bool with_bridge) {
    auto& ficus = node_config_.genesis.state.hardforks.ficus_hf;
    ficus.block_num = with_bridge ? 3 : 2;
    ficus.pillar_blocks_interval = ficus.block_num;
    if (with_bridge) {
      const auto sender = node_config_.getFirstWallet().node_addr;
      ficus.bridge_contract_address = dev::right160(dev::sha3(dev::rlpList(sender, 0)));
    }
  }
};

rustaxa::AppFixtureDiskReport fixture_disk_step(
    const taraxa::cli::Config& cli, rustaxa::AppFixtureDiskRequest request,
    std::unique_ptr<rustaxa::NativeSelectedProductionConfig> foreign = nullptr) {
  return rustaxa::production_fixture_disk_step(taraxa::loadSelectedProductionConfig(cli), std::move(foreign),
                                               std::move(request));
}

#ifdef RUSTAXA_ACTUAL_APP_TEST
rust::String check_app_interrupted_recovery(const taraxa::cli::Config& cli, const std::array<std::uint8_t, 32>& secret,
                                            const std::array<std::uint8_t, 64>& vrf, std::uint8_t stop,
                                            bool pillar = false) {
  fixture_disk_step(cli, {AppFixtureDiskStep::SeedPredecessor, secret, vrf, 0, false});
  const auto transactions =
      fixture_disk_step(cli, {AppFixtureDiskStep::InterruptPublication, secret, vrf, stop, false}).transactions;
  if (transactions.size() != 3) throw std::runtime_error("APP_INTERRUPT_ORDERED_TRANSACTION_COUNT");
  for (const bool second_open : {false, true}) {
    auto app = std::make_shared<taraxa::App>();
    app->init(cli);
    const auto owner = app->getConsensusApplication();
    const auto query = owner->queryClient();
    if (!owner->usesRustExecution() ||
        (*query)->consensus_query_final_chain_last_block_number() != (stop == 1 ? 1 : 2) ||
        !(*query)->consensus_query_final_chain_block_by_number(1).found)
      throw std::runtime_error("APP_INTERRUPT_RECOVERY_HEAD");
    for (const auto& input : transactions) {
      const taraxa::Transaction transaction(taraxa::bytes(input.rlp.begin(), input.rlp.end()));
      const auto receipt = (*query)->consensus_query_transaction_receipt_by_hash(transaction.getHash().asArray());
      if (receipt.found != (stop == 3) || (stop == 3 && receipt.block_number != 2))
        throw std::runtime_error("APP_INTERRUPT_RECOVERY_RECEIPT");
    }
    rustaxa::production_fixture_live_step(owner->service(), taraxa::loadSelectedProductionConfig(cli),
                                          {AppFixtureLiveStep::CheckInterruptedRecovery, {}, {}, second_open, {}});
    if (pillar && second_open) {
      const auto has_vote = [&] {
        return !rustaxa::production_fixture_live_step(owner->service(), taraxa::loadSelectedProductionConfig(cli),
                                                      {AppFixtureLiveStep::CheckScheduledPillarVote, {}, {}, false, {}})
                    .empty();
      };
      if (has_vote()) throw std::runtime_error("APP_TEST_PILLAR_VOTE_WRITTEN_BEFORE_RECOVERY");
      app->startConsensus();
      const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);
      while (!has_vote() && std::chrono::steady_clock::now() < deadline)
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
      app->stopConsensus();
      if (!has_vote()) throw std::runtime_error("APP_TEST_RECOVERED_PILLAR_VOTE_MISSING");
    }
  }
  return rust::String(pillar      ? "ACTUAL_APP_PILLAR_INTERRUPT_RESTART_PASSED"
                      : stop == 1 ? "ACTUAL_APP_INTENT_ONLY_RESTART_PASSED"
                                  : "ACTUAL_APP_CONCRETE_COMMIT_RESTART_PASSED");
}

void check_historical_trace_rpc(const std::shared_ptr<taraxa::App>& app, const std::vector<taraxa::trx_hash_t>& hashes,
                                const taraxa::addr_t& wallet) {
  taraxa::net::Debug rpc(app, 1000000, {}, {}, {}, {}, {}, {}, app->getConsensusApplication()->queryClient());
  Json::Value options(Json::arrayValue);
  options.append("trace");
  options.append("vmTrace");
  const auto block = rpc.trace_replayBlockTransactions("0x2", options);
  if (!block.isArray() || block.size() != hashes.size()) throw std::runtime_error("APP_TEST_REPLAY_BLOCK_TRACES");
  for (Json::ArrayIndex index = 0; index < block.size(); ++index) {
    const auto hash = dev::toJS(hashes[index]);
    const auto replay = rpc.trace_replayTransaction(hash, options);
    if (!replay.isArray() || replay.size() != 1 || replay[0] != block[index] ||
        replay[0]["trace"][0]["traceAddress"] != Json::Value(Json::arrayValue))
      throw std::runtime_error("APP_TEST_REPLAY_TRANSACTION_TRACE");
  }
  Json::Value call(Json::objectValue);
  call["from"] = dev::toJS(wallet);
  call["gas"] = "0x186a0";
  call["nonce"] = "0x3";
  call["data"] = "0xfe";
  call["code"] = "0x602a60005260206000f3";
  const auto traced = rpc.trace_call(call, options, "0x2");
  const auto debug = rpc.debug_traceCall(call, "0x2");
  const std::string output = std::string(62, '0') + "2a";
  if (!traced.isArray() || traced.size() != 1 || traced[0]["output"].asString() != "0x" + output ||
      traced[0]["trace"][0]["type"].asString() != "create" || traced[0]["vmTrace"]["ops"].size() != 6 ||
      !debug.isArray() || debug.size() != 1 || debug[0]["returnValue"].asString() != output ||
      debug[0]["structLogs"].size() != 6)
    throw std::runtime_error("APP_TEST_TRACE_CALL_FORMATS");
  const auto historical = rpc.debug_traceTransaction(dev::toJS(hashes.back()));
  if (!historical.isArray() || historical.size() != 1 || historical[0]["gas"].asUInt64() != 21000)
    throw std::runtime_error("APP_TEST_DEBUG_TRANSACTION_TRACE");
  std::cout << "ACTUAL_APP_HISTORICAL_TRACE_RPC_PASSED\n";
}

rust::String check_actual_app(const taraxa::cli::Config& cli, const std::array<std::uint8_t, 32>& secret,
                              const std::array<std::uint8_t, 64>& vrf, bool native_call, bool repropose = false,
                              bool native_write = false, bool pillar = false, bool system = false) {
  fixture_disk_step(cli, {AppFixtureDiskStep::SeedPredecessor, secret, vrf, static_cast<uint8_t>(system), false});
  const auto has_pillar_vote = [&](const auto& owner) {
    return !rustaxa::production_fixture_live_step(owner->service(), taraxa::loadSelectedProductionConfig(cli),
                                                  {AppFixtureLiveStep::CheckScheduledPillarVote, {}, {}, false, {}})
                .empty();
  };
  const auto has_reproposal = [&](const auto& owner) {
    return !rustaxa::production_fixture_live_step(owner->service(), taraxa::loadSelectedProductionConfig(cli),
                                                  {AppFixtureLiveStep::CheckReproposal, {}, {}, false, {}})
                .empty();
  };
  std::vector<taraxa::trx_hash_t> hashes;
  rustaxa::FinalChainBlockView predecessor;
  {
    auto app = std::make_shared<taraxa::App>();
    app->init(cli);
    const auto owner = app->getConsensusApplication();
    const auto query = owner->queryClient();
    if (!owner->usesRustExecution() || (*query)->consensus_query_final_chain_last_block_number() != 1)
      throw std::runtime_error("APP_TEST_WRONG_OWNER_OR_PREDECESSOR");
    predecessor = (*query)->consensus_query_final_chain_block_by_number(1);
    if (!predecessor.found) throw std::runtime_error("APP_TEST_PREDECESSOR_HEADER_MISSING");
    const auto& config = app->getConfig();
    const auto& wallet = config.getFirstWallet();
    const taraxa::val_t price = taraxa::val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price) + 1;
    rust::Vec<rustaxa::AppFixtureTransaction> inputs;
    for (std::uint64_t nonce = 3; nonce < 6; ++nonce) {
      const bool native = native_call && nonce == 3;
      const auto receiver = native ? taraxa::addr_t("0x00000000000000000000000000000000000000fe") : wallet.node_addr;
      taraxa::bytes call_data;
      if (native) {
        call_data = {0xde, 0x8e, 0x4b, 0x50};
        if (native_write) {
          call_data = {0xf0, 0x00, 0x32, 0x2c};
          call_data.resize(68);
          const auto address = config.genesis.state.dpos.initial_validators.front().address.asArray();
          std::copy(address.begin(), address.end(), call_data.begin() + 16);
          call_data.back() = 1;
        }
      }
      auto transaction = std::make_shared<taraxa::Transaction>(nonce, native ? 0 : 1, price, native ? 100000 : 21000,
                                                               std::move(call_data), wallet.node_secret, receiver,
                                                               config.genesis.chain_id);
      const auto result = owner->submitTransaction(transaction, config);
      if (!result.accepted || !result.transaction_observed || result.transaction_hash != transaction->getHash())
        throw std::runtime_error("APP_TEST_NORMAL_CLIENT_REJECTED");
      hashes.push_back(result.transaction_hash);
      rustaxa::AppFixtureTransaction input;
      for (const auto byte : transaction->rlp()) input.rlp.push_back(byte);
      inputs.push_back(std::move(input));
    }
    rustaxa::production_fixture_live_step(
        owner->service(), taraxa::loadSelectedProductionConfig(cli),
        {repropose ? AppFixtureLiveStep::RetainNextVotes : AppFixtureLiveStep::RetainCertificate,
         {},
         vrf,
         system,
         std::move(inputs)});
    app->startConsensus();
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);
    if (repropose) {
      while (!has_reproposal(owner) && std::chrono::steady_clock::now() < deadline)
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
      app->stopConsensus();
      if (!has_reproposal(owner)) throw std::runtime_error("APP_TEST_REPROPOSAL_MISSING");
    } else {
      while ((*query)->consensus_query_final_chain_last_block_number() == 1 &&
             std::chrono::steady_clock::now() < deadline) {
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
      }
      if (pillar) {
        while (!has_pillar_vote(owner) && std::chrono::steady_clock::now() < deadline)
          std::this_thread::sleep_for(std::chrono::milliseconds(10));
      }
      app->stopConsensus();
      if ((*query)->consensus_query_final_chain_last_block_number() != 2)
        throw std::runtime_error("APP_TEST_SCHEDULER_DID_NOT_PUBLISH");
      if (pillar && !has_pillar_vote(owner)) throw std::runtime_error("APP_TEST_PILLAR_VOTE_MISSING");
      for (std::size_t index = 0; index < hashes.size(); ++index) {
        const auto receipt = (*query)->consensus_query_transaction_receipt_by_hash(hashes[index].asArray());
        if (!receipt.found || receipt.block_number != 2 || receipt.transaction_index != index)
          throw std::runtime_error("APP_TEST_SCHEDULER_RECEIPT");
      }
    }
  }
  if (repropose) {
    auto reopened = std::make_shared<taraxa::App>();
    reopened->init(cli);
    if (!has_reproposal(reopened->getConsensusApplication())) throw std::runtime_error("APP_TEST_REPROPOSAL_RESTART");
    return rust::String("ACTUAL_APP_REPROPOSAL_PASSED");
  }
  if (!pillar) fixture_disk_step(cli, {AppFixtureDiskStep::CheckRestartImages, {}, {}, 0, false});
  {
    auto reopened = std::make_shared<taraxa::App>();
    reopened->init(cli);
    const auto owner = reopened->getConsensusApplication();
    const auto query = owner->queryClient();
    const auto previous = (*query)->consensus_query_final_chain_block_by_number(1);
    const auto committed = (*query)->consensus_query_final_chain_block_by_number(2);
    const std::array<std::uint8_t, 32> zero{};
    if (!owner->usesRustExecution() || (*query)->consensus_query_final_chain_last_block_number() != 2 ||
        !previous.found || previous.hash != predecessor.hash || !committed.found ||
        committed.parent_hash != predecessor.hash || !committed.has_pbft_hash ||
        ((native_call || system) ? committed.gas_used <= 63000 : committed.gas_used != 63000) ||
        committed.state_root == predecessor.state_root || committed.transactions_root == zero ||
        committed.receipts_root == zero ||
        (*query)->consensus_query_transaction_count_by_block({false, 2, {}}) !=
            hashes.size() + static_cast<std::size_t>(system))
      throw std::runtime_error("APP_TEST_RESTART_HEAD_OR_HISTORY");
    if (pillar && !has_pillar_vote(owner)) throw std::runtime_error("APP_TEST_RESTART_PILLAR_VOTE_MISSING");
    if (system) {
      const auto bridge = reopened->getConfig().genesis.state.hardforks.ficus_hf.bridge_contract_address;
      if (owner->getCode(bridge, 1).empty() || owner->getAccountStorage(bridge, 0, 1) != taraxa::h256{} ||
          owner->getAccountStorage(bridge, 0, 2) != taraxa::h256{1})
        throw std::runtime_error("APP_TEST_BRIDGE_SYSTEM_TRANSITION");
    }
    const auto wallet_address = reopened->getConfig().getFirstWallet().node_addr;
    const auto historical_account = owner->getAccount(wallet_address, 1);
    const auto latest_account = owner->getAccount(wallet_address, std::nullopt);
    if (!historical_account || !latest_account || historical_account->nonce != 3 || latest_account->nonce != 6 ||
        latest_account->balance >= historical_account->balance || owner->getAccount(wallet_address, 3) ||
        owner->getAccountStorage(wallet_address, 0, 1) != taraxa::h256{} ||
        !owner->getCode(wallet_address, 2).empty()) {
      throw std::runtime_error("APP_TEST_RESTART_CONCRETE_QUERIES");
    }
    if (native_write) {
      taraxa::state_api::EVMTransaction read;
      read.from = wallet_address;
      read.to = wallet_address;
      read.nonce = 3;
      read.gas = 21000;
      const auto trace = taraxa::util::readJsonFromString(owner->trace({}, {read}, 2, std::nullopt));
      if (!trace.isArray() || trace.size() != 1 || trace[0]["failed"].asBool() || trace[0]["gas"].asUInt64() != 21000 ||
          trace[0]["returnValue"].asString() != "")
        throw std::runtime_error("APP_TEST_HISTORICAL_TRACE");
      read.nonce = 0;
      const auto genesis_trace = taraxa::util::readJsonFromString(owner->trace({}, {read}, 0, std::nullopt));
      if (!genesis_trace.isArray() || genesis_trace.size() != 1 || genesis_trace[0]["failed"].asBool() ||
          genesis_trace[0]["gas"].asUInt64() != 21000)
        throw std::runtime_error("APP_TEST_GENESIS_TRACE");
      std::cout << "ACTUAL_APP_HISTORICAL_TRACE_PASSED\n";
      check_historical_trace_rpc(reopened, hashes, wallet_address);
    }
    for (std::size_t index = 0; index < hashes.size(); ++index) {
      const auto receipt = (*query)->consensus_query_transaction_receipt_by_hash(hashes[index].asArray());
      if (!receipt.found || receipt.block_number != 2 || receipt.transaction_index != index)
        throw std::runtime_error("APP_TEST_RESTART_RECEIPT");
      const taraxa::bytes receipt_rlp(receipt.receipt_rlp.begin(), receipt.receipt_rlp.end());
      if (dev::RLP(receipt_rlp)[0].toInt<unsigned>() != 1) throw std::runtime_error("APP_TEST_RECEIPT_FAILURE");
    }
  }
  if (!pillar) fixture_disk_step(cli, {AppFixtureDiskStep::CheckRestartImages, {}, {}, 0, true});
  return rust::String(system         ? "ACTUAL_APP_BRIDGE_SYSTEM_RESTART_PASSED"
                      : pillar       ? "ACTUAL_APP_PILLAR_RESTART_PASSED"
                      : native_write ? "ACTUAL_APP_NATIVE_WRITE_RESTART_PASSED"
                      : native_call  ? "ACTUAL_APP_NATIVE_SCHEDULER_RESTART_PASSED"
                                     : "ACTUAL_APP_SCHEDULER_RESTART_PASSED");
}
#endif
}  // namespace

int main() {
  try {
    const auto scratch = std::getenv("RUSTAXA_OWNER_TEST_SCRATCH_ROOT");
    if (!scratch) throw std::runtime_error("PRODUCTION_TEST_SCRATCH_ROOT_REQUIRED");
#ifdef RUSTAXA_ACTUAL_APP_TEST
    for (int scenario = 11; scenario < 20; ++scenario) {
#else
    for (int scenario = 0; scenario < 11; ++scenario) {
#endif
      const bool certified = scenario != 0;
      const auto name = scenario == 10  ? "native-common-clients"
                        : scenario >= 2 ? "native-publication-" + std::to_string(scenario - 2)
                                        : (certified ? "native-certified-owner" : "native-production-owner");
      const auto root = std::filesystem::path(scratch) / name;
      if (std::filesystem::exists(root)) throw std::runtime_error("PRODUCTION_TEST_PATH_ALREADY_USED");
      std::filesystem::create_directories(root);
      const auto config_path = root / "config.json";
      const auto genesis_path = root / "genesis.json";
      const auto wallet_path = root / "wallet.json";
      const std::vector<std::string> arguments{
          "production_owner_tests",
          "--chain",
          "devnet",
          "--config",
          config_path.string(),
          "--genesis",
          genesis_path.string(),
          "--wallet",
          wallet_path.string(),
          "--data-dir",
          (root / "data").string(),
          "--node-secret",
          std::string(64, (scenario == 15 || scenario == 17 || scenario == 19) ? '6' : '5')};
      std::vector<const char*> argv;
      for (const auto& argument : arguments) argv.push_back(argument.c_str());
      taraxa::cli::Config initial;
      initial.parseCommandLine(static_cast<int>(argv.size()), argv.data());
      auto node = initial.getNodeConfiguration();
      require_task_path(root, node.db_path);
      const auto address = node.getFirstWallet().node_addr;
      if (certified) {
        if (node.genesis.state.dpos.initial_validators.empty())
          throw std::runtime_error("PRODUCTION_TEST_VALIDATOR_REQUIRED");
        auto validator = node.genesis.state.dpos.initial_validators.front();
        const dev::Secret certificate_secret(std::string(64, '6'), dev::Secret::ConstructFromStringType::FromHex);
        const auto certificate_address = dev::KeyPair(certificate_secret).address();
        validator.address = certificate_address;
        validator.owner = scenario == 16 ? address : certificate_address;
        node.genesis.state.initial_balances[certificate_address] = dev::u256("1000000000000000000");
        validator.vrf_key = node.getFirstWallet().vrf_pk;
        auto second = validator;
        const dev::Secret second_secret(std::string(64, '7'), dev::Secret::ConstructFromStringType::FromHex);
        second.address = dev::KeyPair(second_secret).address();
        second.owner = second.address;
        const std::array<std::uint8_t, 32> second_key{0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb,
                                                      0x7e, 0x28, 0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5,
                                                      0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97, 0xad, 0xe4, 0x00, 0x81};
        std::copy(second_key.begin(), second_key.end(), second.vrf_key.data());
        node.genesis.state.initial_balances[second.address] = dev::u256("1000000000000000000");
        node.genesis.state.dpos.initial_validators = {validator, second};
      } else {
        for (const auto& validator : node.genesis.state.dpos.initial_validators) {
          if (validator.address == address) throw std::runtime_error("PRODUCTION_TEST_KEY_MUST_BE_INELIGIBLE");
        }
      }
      if (scenario >= 17) {
        node.genesis.state.dpos.delegation_delay = 1;
      }
      if (scenario == 18) node.genesis.state.initial_balances[dev::ZeroAddress] = dev::u256("1000000000000000000");
      node.genesis.state.initial_balances[address] = dev::u256("1000000000000000000");
      taraxa::util::writeJsonToFile(genesis_path.string(), enc_json(node.genesis));
#ifdef RUSTAXA_ACTUAL_APP_TEST
      auto offline_config = taraxa::util::readJsonFromFile(config_path.string());
      auto& network = offline_config["network"];
      network["boot_nodes"] = Json::Value(Json::arrayValue);
      network["priority_nodes"] = Json::Value(Json::arrayValue);
      if (scenario >= 17) network["ddos_protection"]["vote_accepting_periods"] = 1;
      network["listen_ip"] = "127.0.0.1";
      network["public_ip"] = "";
      taraxa::util::writeJsonToFile(config_path.string(), offline_config);
#endif
      FixtureConfig selected_cli;
      selected_cli.parseCommandLine(static_cast<int>(argv.size()), argv.data());
      if (scenario >= 17) selected_cli.schedulePillarTest(scenario == 18);
      const auto selected_node = selected_cli.getNodeConfiguration();
      require_task_path(root, selected_node.db_path);
#ifdef RUSTAXA_ACTUAL_APP_TEST
      if (!selected_node.network.boot_nodes.empty() || !selected_node.network.trusted_nodes.empty() ||
          selected_node.network.listen_ip != "127.0.0.1" || !selected_node.network.public_ip.empty())
        throw std::runtime_error("APP_TEST_NETWORK_MUST_BE_LOCAL");
#endif
      std::cout << fixture_disk_step(selected_cli, {AppFixtureDiskStep::Prepare, {}, {}, 0, false}).message << '\n';
      std::array<std::uint8_t, 32> secret{};
      std::copy_n(selected_node.getFirstWallet().node_secret.data(), secret.size(), secret.begin());
      std::array<std::uint8_t, 64> vrf_secret{};
      std::copy_n(selected_node.getFirstWallet().vrf_secret.data(), vrf_secret.size(), vrf_secret.begin());
      const auto report = [&]() {
#ifdef RUSTAXA_ACTUAL_APP_TEST
        if (scenario == 19) return check_app_interrupted_recovery(selected_cli, secret, vrf_secret, 3, true);
        if (scenario == 18) return check_actual_app(selected_cli, secret, vrf_secret, false, false, false, false, true);
        if (scenario == 17) return check_actual_app(selected_cli, secret, vrf_secret, false, false, false, true);
        if (scenario == 16) return check_actual_app(selected_cli, secret, vrf_secret, true, false, true);
        if (scenario == 15) return check_actual_app(selected_cli, secret, vrf_secret, false, true);
        if (scenario >= 13)
          return check_app_interrupted_recovery(selected_cli, secret, vrf_secret, scenario == 13 ? 1 : 3);
        if (scenario >= 11) return check_actual_app(selected_cli, secret, vrf_secret, scenario == 12);
#endif
        if (scenario == 10) {
          auto application = taraxa::createQualifiedProductionApplication(selected_cli);
          return check_common_clients(*application, selected_node);
        }
        if (scenario >= 2) {
          std::unique_ptr<rustaxa::NativeSelectedProductionConfig> foreign;
          if (scenario == 2) {
            auto foreign_arguments = arguments;
            const auto data = std::find(foreign_arguments.begin(), foreign_arguments.end(), "--data-dir");
            if (data == foreign_arguments.end() || std::next(data) == foreign_arguments.end())
              throw std::runtime_error("PRODUCTION_TEST_DATA_ARGUMENT_REQUIRED");
            *std::next(data) = (root / "foreign" / "data").string();
            std::vector<const char*> foreign_argv;
            for (const auto& argument : foreign_arguments) foreign_argv.push_back(argument.c_str());
            taraxa::cli::Config foreign_cli;
            foreign_cli.parseCommandLine(static_cast<int>(foreign_argv.size()), foreign_argv.data());
            require_task_path(root, foreign_cli.getNodeConfiguration().db_path);
            if (foreign_cli.getNodeConfiguration().db_path == selected_node.db_path)
              throw std::runtime_error("PRODUCTION_TEST_FOREIGN_DATABASE_REQUIRED");
            std::cout << fixture_disk_step(foreign_cli, {AppFixtureDiskStep::Prepare, {}, {}, 0, false}).message
                      << '\n';
            foreign = taraxa::loadSelectedProductionConfig(foreign_cli);
          }
          return fixture_disk_step(selected_cli,
                                   {AppFixtureDiskStep::CheckPublication, secret, vrf_secret,
                                    static_cast<std::uint8_t>(scenario - 2), false},
                                   std::move(foreign))
              .message;
        }
        auto application = taraxa::createQualifiedProductionApplication(selected_cli);
        return rustaxa::production_fixture_live_step(*application, taraxa::loadSelectedProductionConfig(selected_cli),
                                                     {AppFixtureLiveStep::CheckFixture, secret, vrf_secret, false, {}});
      }();
      const std::string text(report.data(), report.size());
      const auto marker =
          scenario == 19   ? "ACTUAL_APP_PILLAR_INTERRUPT_RESTART_PASSED"
          : scenario == 18 ? "ACTUAL_APP_BRIDGE_SYSTEM_RESTART_PASSED"
          : scenario == 17 ? "ACTUAL_APP_PILLAR_RESTART_PASSED"
          : scenario == 16 ? "ACTUAL_APP_NATIVE_WRITE_RESTART_PASSED"
          : scenario == 15 ? "ACTUAL_APP_REPROPOSAL_PASSED"
          : scenario == 14 ? "ACTUAL_APP_CONCRETE_COMMIT_RESTART_PASSED"
          : scenario == 13 ? "ACTUAL_APP_INTENT_ONLY_RESTART_PASSED"
          : scenario == 12 ? "ACTUAL_APP_NATIVE_SCHEDULER_RESTART_PASSED"
          : scenario == 11 ? "ACTUAL_APP_SCHEDULER_RESTART_PASSED"
          : scenario == 10 ? "COMMON_PRODUCTION_CLIENTS_PASSED"
          : scenario >= 2  ? "CERTIFIED_PUBLICATION_RECOVERY_PASSED"
                           : (certified ? "CERTIFIED_TRANSITION_CHECKS_PASSED" : "PRODUCTION_TRANSITION_CHECKS_PASSED");
      if (text.find(marker) == std::string::npos) throw std::runtime_error("PRODUCTION_TEST_REQUIRED_BRANCH_MISSING");
      std::cout << report << '\n';
    }
    return 0;
  } catch (const std::exception& error) {
    std::cerr << error.what() << '\n';
    return 1;
  }
}
