#include "Taraxa.h"

#include <json/reader.h>
#include <jsonrpccpp/common/exception.h>
#include <libdevcore/CommonData.h>
#include <libdevcore/CommonJS.h>
#include <libp2p/Common.h>

#include <stdexcept>

#include "config/version.hpp"
#include "dag/dag_manager.hpp"
#include "pbft/pbft_manager.hpp"
#include "pillar_chain/pillar_block.hpp"
#include "transaction/transaction_manager.hpp"

#ifdef RUSTAXA_ENABLE
#include "transaction/system_transaction.hpp"
#endif

using namespace std;
using namespace jsonrpc;
using namespace dev;
using namespace taraxa;
using namespace ::taraxa::final_chain;

namespace taraxa::net {

namespace {
TaraxaDposReader makeTaraxaDposReader(std::weak_ptr<taraxa::AppBase> app) {
  TaraxaDposReader reader;
  reader.eligible_total_vote_count = [app](EthBlockNumber block_number) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DPOS_READER_APP_EXPIRED");
    }
    return node->getFinalChain()->dposEligibleTotalVoteCount(block_number);
  };
  reader.eligible_vote_count = [app](EthBlockNumber block_number, const addr_t& address) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DPOS_READER_APP_EXPIRED");
    }
    return node->getFinalChain()->dposEligibleVoteCount(block_number, address);
  };
  reader.dpos_yield = [app](EthBlockNumber block_number) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DPOS_READER_APP_EXPIRED");
    }
    return node->getFinalChain()->dposYield(block_number);
  };
  reader.total_supply = [app](EthBlockNumber block_number) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DPOS_READER_APP_EXPIRED");
    }
    return node->getFinalChain()->dposTotalSupply(block_number);
  };
  return reader;
}

TaraxaDagStatusReader makeTaraxaDagStatusReader(std::weak_ptr<taraxa::AppBase> app) {
  TaraxaDagStatusReader reader;
  reader.latest_level = [app] {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DAG_STATUS_READER_APP_EXPIRED");
    }
    return static_cast<uint64_t>(node->getDagManager()->getMaxLevel());
  };
  reader.latest_period = [app] {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DAG_STATUS_READER_APP_EXPIRED");
    }
    return static_cast<uint64_t>(node->getDagManager()->getLatestPeriod());
  };
  return reader;
}

TaraxaDagBlockReader makeTaraxaDagBlockReader(std::weak_ptr<taraxa::AppBase> app) {
  TaraxaDagBlockReader reader;
  reader.block_by_hash = [app](const blk_hash_t& hash) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DAG_BLOCK_READER_APP_EXPIRED");
    }
    return node->getDagManager()->getDagBlock(hash);
  };
  reader.blocks_by_level = [app](level_t level) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DAG_BLOCK_READER_APP_EXPIRED");
    }
    return node->getDB()->getDagBlocksAtLevel(level, 1);  // RUSTAXA_QUERY_COMPAT_READ
  };
  reader.period_by_hash = [app](const blk_hash_t& hash) -> std::optional<uint64_t> {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DAG_BLOCK_READER_APP_EXPIRED");
    }
    const auto period = node->getPbftManager()->getDagBlockPeriod(hash);
    if (!period.first) {
      return std::nullopt;
    }
    return period.second;
  };
  reader.transaction_by_hash = [app](const trx_hash_t& hash) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_DAG_BLOCK_READER_APP_EXPIRED");
    }
    return node->getTransactionManager()->getTransaction(hash);
  };
  return reader;
}

TaraxaPersistentReader makeTaraxaPersistentReader(std::weak_ptr<taraxa::AppBase> app) {
  TaraxaPersistentReader reader;
  reader.pbft_block_hash_by_period = [app](uint64_t period) -> std::optional<blk_hash_t> {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_PERSISTENT_READER_APP_EXPIRED");
    }
    const auto block = node->getDB()->getPbftBlock(period);  // RUSTAXA_QUERY_COMPAT_READ
    if (!block) {
      return std::nullopt;
    }
    return block->getBlockHash();
  };
  reader.chain_stats = [app] {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_PERSISTENT_READER_APP_EXPIRED");
    }
    return TaraxaChainStatsView{
        node->getFinalChain()->lastBlockNumber(),
        node->getDB()->getNumBlockExecuted(),        // RUSTAXA_QUERY_COMPAT_READ
        node->getDB()->getNumTransactionExecuted(),  // RUSTAXA_QUERY_COMPAT_READ
    };
  };
  reader.period_lambda = [app](uint64_t period) -> std::optional<uint64_t> {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("TARAXA_PERSISTENT_READER_APP_EXPIRED");
    }
    return node->getDB()->getPeriodLambda(period, false);  // RUSTAXA_QUERY_COMPAT_READ
  };
  return reader;
}

void fillMissingTaraxaDposReaderCallbacks(TaraxaDposReader& reader, std::weak_ptr<taraxa::AppBase> app) {
  auto defaults = makeTaraxaDposReader(std::move(app));
  if (!reader.eligible_total_vote_count) {
    reader.eligible_total_vote_count = std::move(defaults.eligible_total_vote_count);
  }
  if (!reader.eligible_vote_count) {
    reader.eligible_vote_count = std::move(defaults.eligible_vote_count);
  }
  if (!reader.dpos_yield) {
    reader.dpos_yield = std::move(defaults.dpos_yield);
  }
  if (!reader.total_supply) {
    reader.total_supply = std::move(defaults.total_supply);
  }
}

void fillMissingTaraxaDagStatusReaderCallbacks(TaraxaDagStatusReader& reader, std::weak_ptr<taraxa::AppBase> app) {
  auto defaults = makeTaraxaDagStatusReader(std::move(app));
  if (!reader.latest_level) {
    reader.latest_level = std::move(defaults.latest_level);
  }
  if (!reader.latest_period) {
    reader.latest_period = std::move(defaults.latest_period);
  }
}

void fillMissingTaraxaDagBlockReaderCallbacks(TaraxaDagBlockReader& reader, std::weak_ptr<taraxa::AppBase> app) {
  auto defaults = makeTaraxaDagBlockReader(std::move(app));
  if (!reader.block_by_hash) {
    reader.block_by_hash = std::move(defaults.block_by_hash);
  }
  if (!reader.blocks_by_level) {
    reader.blocks_by_level = std::move(defaults.blocks_by_level);
  }
  if (!reader.period_by_hash) {
    reader.period_by_hash = std::move(defaults.period_by_hash);
  }
  if (!reader.transaction_by_hash) {
    reader.transaction_by_hash = std::move(defaults.transaction_by_hash);
  }
}

void fillMissingTaraxaPersistentReaderCallbacks(TaraxaPersistentReader& reader, std::weak_ptr<taraxa::AppBase> app) {
  auto defaults = makeTaraxaPersistentReader(std::move(app));
  if (!reader.pbft_block_hash_by_period) {
    reader.pbft_block_hash_by_period = std::move(defaults.pbft_block_hash_by_period);
  }
  if (!reader.chain_stats) {
    reader.chain_stats = std::move(defaults.chain_stats);
  }
  if (!reader.period_lambda) {
    reader.period_lambda = std::move(defaults.period_lambda);
  }
}
}  // namespace

#ifdef RUSTAXA_ENABLE
namespace {
constexpr uint8_t kConsensusQueryTransactionSourceMissing = 0;
constexpr uint8_t kConsensusQueryTransactionSourcePending = 1;
constexpr uint8_t kConsensusQueryTransactionSourceFinalizedRegular = 2;
constexpr uint8_t kConsensusQueryTransactionSourceFinalizedSystem = 3;

dev::h256 hashFromBridge(const std::array<uint8_t, 32>& hash) {
  return dev::h256(hash.data(), dev::h256::ConstructFromPointer);
}

dev::Address addressFromBridge(const std::array<uint8_t, 20>& address) {
  return dev::Address(address.data(), dev::Address::ConstructFromPointer);
}

dev::bytes bytesFromBridge(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

std::shared_ptr<Transaction> materializeTransactionView(const rustaxa::TransactionPublicView& view) {
  if (!view.found) {
    return nullptr;
  }

  std::shared_ptr<Transaction> transaction;
  if (view.source == kConsensusQueryTransactionSourceFinalizedSystem) {
    transaction = std::make_shared<SystemTransaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.source == kConsensusQueryTransactionSourcePending ||
             view.source == kConsensusQueryTransactionSourceFinalizedRegular) {
    transaction = std::make_shared<Transaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.source != kConsensusQueryTransactionSourceMissing) {
    throw std::runtime_error("CONSENSUS_QUERY_TRANSACTION_UNKNOWN_SOURCE");
  }

  if (transaction && transaction->getHash() != hashFromBridge(view.hash)) {
    throw std::runtime_error("CONSENSUS_QUERY_TRANSACTION_HASH_MISMATCH");
  }
  return transaction;
}

Json::Value pbftExtraDataViewToJson(const rustaxa::PbftBlockExtraDataView& view) {
  Json::Value json;
  json["major_version"] = view.major_version;
  json["minor_version"] = view.minor_version;
  json["patch_version"] = view.patch_version;
  json["net_version"] = view.net_version;
  json["node_implementation"] = std::string(view.node_implementation);
  json["pillar_block_hash"] = view.has_pillar_block_hash ? hashFromBridge(view.pillar_block_hash).toString() : "";
  return json;
}

Json::Value pbftScheduleBlockViewToJson(const rustaxa::PbftScheduleBlockView& view) {
  Json::Value json;
  json["prev_block_hash"] = dev::toJS(hashFromBridge(view.prev_block_hash));
  json["dag_block_hash_as_pivot"] = dev::toJS(hashFromBridge(view.dag_block_hash_as_pivot));
  json["order_hash"] = dev::toJS(hashFromBridge(view.order_hash));
  json["final_chain_hash"] = dev::toJS(hashFromBridge(view.final_chain_hash));
  json["period"] = dev::toJS(view.period);
  json["timestamp"] = dev::toJS(view.timestamp);
  json["block_hash"] = dev::toJS(hashFromBridge(view.block_hash));
  json["signature"] = dev::toJS(bytesFromBridge(view.signature));
  json["beneficiary"] = dev::toJS(addressFromBridge(view.beneficiary));
  json["reward_votes"] = Json::Value(Json::arrayValue);
  for (const auto& vote_hash : view.reward_votes) {
    json["reward_votes"].append(dev::toJS(hashFromBridge(vote_hash.hash)));
  }
  json["extra_data"] = view.has_extra_data ? pbftExtraDataViewToJson(view.extra_data) : Json::Value("");

  auto& schedule_json = json["schedule"] = Json::Value(Json::objectValue);
  auto& dag_blks_json = schedule_json["dag_blocks_order"] = Json::Value(Json::arrayValue);
  for (const auto& dag_hash : view.dag_blocks_order) {
    dag_blks_json.append(dev::toJS(hashFromBridge(dag_hash.hash)));
  }
  return json;
}

Json::Value dagBlockPublicViewToJson(const rustaxa::DagBlockPublicView& view) {
  Json::Value json;
  json["pivot"] = dev::toJS(hashFromBridge(view.pivot));
  json["level"] = dev::toJS(view.level);
  json["tips"] = Json::Value(Json::arrayValue);
  for (const auto& tip : view.tips) {
    json["tips"].append(dev::toJS(hashFromBridge(tip.hash)));
  }
  json["transactions"] = Json::Value(Json::arrayValue);
  for (const auto& trx : view.transactions) {
    json["transactions"].append(dev::toJS(hashFromBridge(trx.hash)));
  }
  json["trx_estimations"] = dev::toJS(view.trx_estimations);
  json["sig"] = dev::toJS(bytesFromBridge(view.signature));
  json["hash"] = dev::toJS(hashFromBridge(view.hash));
  json["sender"] = dev::toJS(addressFromBridge(view.sender));
  json["timestamp"] = dev::toJS(view.timestamp);
  if (view.has_vdf) {
    Json::Value vdf;
    vdf["proof"] = dev::toJS(bytesFromBridge(view.vdf_proof));
    vdf["sol1"] = dev::toJS(dev::toHex(bytesFromBridge(view.vdf_sol1)));
    vdf["sol2"] = dev::toJS(dev::toHex(bytesFromBridge(view.vdf_sol2)));
    vdf["difficulty"] = dev::toJS(view.vdf_difficulty);
    json["vdf"] = vdf;
  }
  return json;
}

template <typename QueryApi>
void appendDagBlockTransactionsFromQuery(Json::Value& block_json, const rust::Vec<rustaxa::DagHash>& transaction_hashes,
                                         const QueryApi& query_api) {
  block_json["transactions"] = Json::Value(Json::arrayValue);
  for (const auto& transaction_hash : transaction_hashes) {
    auto transaction =
        materializeTransactionView(query_api->consensus_query_transaction_by_hash(transaction_hash.hash));
    if (!transaction) {
      throw std::runtime_error("CONSENSUS_QUERY_DAG_BLOCK_TRANSACTION_MISSING");
    }
    block_json["transactions"].append(transaction->toJSON());
  }
}

Json::Value pillarBlockDataViewToJson(const rustaxa::PillarBlockDataView& view, bool include_signatures) {
  Json::Value res;
  Json::Value pillar_block;
  pillar_block["pbft_period"] = dev::toJS(view.pbft_period);
  pillar_block["state_root"] = dev::toJS(hashFromBridge(view.state_root));
  pillar_block["previous_pillar_block_hash"] = dev::toJS(hashFromBridge(view.previous_pillar_block_hash));
  pillar_block["bridge_root"] = dev::toJS(hashFromBridge(view.bridge_root));
  pillar_block["epoch"] = dev::toJS(view.epoch);
  pillar_block["validators_vote_counts_changes"] = Json::Value(Json::arrayValue);
  for (const auto& change : view.validator_vote_count_changes) {
    Json::Value vote_count_change_json;
    vote_count_change_json["address"] = dev::toJS(addressFromBridge(change.address));
    vote_count_change_json["value"] = static_cast<Json::Value::Int64>(change.vote_count_change);
    pillar_block["validators_vote_counts_changes"].append(std::move(vote_count_change_json));
  }
  pillar_block["hash"] = dev::toJS(hashFromBridge(view.block_hash));
  res["pillar_block"] = std::move(pillar_block);

  if (include_signatures) {
    res["signatures"] = Json::Value(Json::arrayValue);
    for (const auto& compact : view.signatures) {
      Json::Value signature;
      signature["r"] = dev::toJS(dev::u256(hashFromBridge(compact.r)));
      signature["vs"] = dev::toJS(dev::u256(hashFromBridge(compact.vs)));
      res["signatures"].append(std::move(signature));
    }
  }

  return res;
}
}  // namespace
#endif

Taraxa::Taraxa(std::shared_ptr<AppBase> app, TaraxaDposReader dpos_reader, TaraxaDagStatusReader dag_status_reader,
               TaraxaDagBlockReader dag_block_reader, TaraxaPersistentReader persistent_reader)
    : app_(app),
      dpos_reader_(std::move(dpos_reader)),
      dag_status_reader_(std::move(dag_status_reader)),
      dag_block_reader_(std::move(dag_block_reader)),
      persistent_reader_(std::move(persistent_reader)) {
  fillMissingTaraxaDposReaderCallbacks(dpos_reader_, app_);
  fillMissingTaraxaDagStatusReaderCallbacks(dag_status_reader_, app_);
  fillMissingTaraxaDagBlockReaderCallbacks(dag_block_reader_, app_);
  fillMissingTaraxaPersistentReaderCallbacks(persistent_reader_, app_);

  Json::CharReaderBuilder builder;
  auto reader = std::unique_ptr<Json::CharReader>(builder.newCharReader());

  bool parsingSuccessful = reader->parse(kVersionJson, kVersionJson + strlen(kVersionJson), &version, nullptr);
  assert(parsingSuccessful);
}

string Taraxa::taraxa_protocolVersion() { return toJS(TARAXA_NET_VERSION); }

Json::Value Taraxa::taraxa_getVersion() { return version; }

string Taraxa::taraxa_dagBlockLevel() {
  try {
#ifdef RUSTAXA_ENABLE
    if (auto app = app_.lock()) {
      const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
      return toJS(query_api->consensus_query_status().latest_dag_level);
    }
#endif
    return toJS(dag_status_reader_.latest_level());
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

string Taraxa::taraxa_dagBlockPeriod() {
  try {
#ifdef RUSTAXA_ENABLE
    if (auto app = app_.lock()) {
      const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
      return toJS(query_api->consensus_query_status().latest_dag_period);
    }
#endif
    return toJS(dag_status_reader_.latest_period());
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

std::shared_ptr<AppBase> Taraxa::tryGetApp() {
  if (auto app = app_.lock()) {
    return app;
  }
  BOOST_THROW_EXCEPTION(jsonrpc::JsonRpcException(jsonrpc::Errors::ERROR_RPC_INTERNAL_ERROR));
}

Json::Value Taraxa::taraxa_getDagBlockByHash(const string& _blockHash, bool _includeTransactions) {
  try {
#ifdef RUSTAXA_ENABLE
    if (auto app = app_.lock()) {
      const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
      const auto rust_block = query_api->consensus_query_dag_block_by_hash(blk_hash_t(_blockHash).asArray());
      if (rust_block.found) {
        auto block_json = dagBlockPublicViewToJson(rust_block);
        if (rust_block.finalized_period_found) {
          block_json["period"] = toJS(rust_block.finalized_period);
        } else {
          block_json["period"] = "-0x1";
        }
        if (_includeTransactions) {
          appendDagBlockTransactionsFromQuery(block_json, rust_block.transactions, query_api);
        }
        return block_json;
      }
      return Json::Value();
    }
#endif
    auto block = dag_block_reader_.block_by_hash(blk_hash_t(_blockHash));
    if (block) {
      auto block_json = block->getJson();
      const auto period = dag_block_reader_.period_by_hash(block->getHash());
      if (period) {
        block_json["period"] = toJS(*period);
      } else {
        block_json["period"] = "-0x1";
      }
      if (_includeTransactions) {
        block_json["transactions"] = Json::Value(Json::arrayValue);
        for (auto const& t : block->getTrxs()) {
          auto transaction = dag_block_reader_.transaction_by_hash(t);
          if (!transaction) {
            throw std::runtime_error("TARAXA_DAG_BLOCK_TRANSACTION_MISSING");
          }
          block_json["transactions"].append(transaction->toJSON());
        }
      }
      return block_json;
    }
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
  return Json::Value();
}

std::string Taraxa::taraxa_pbftBlockHashByPeriod(const std::string& _period) {
  try {
    const auto period = dev::jsToInt(_period);
#ifdef RUSTAXA_ENABLE
    if (auto app = app_.lock()) {
      const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
      const auto lookup = query_api->consensus_query_pbft_block_hash_by_period(period);
      if (!lookup.found) {
        return {};
      }
      return toJS(hashFromBridge(lookup.hash));
    }
#endif
    const auto block_hash = persistent_reader_.pbft_block_hash_by_period(period);
    if (!block_hash) {
      return {};
    }
    return toJS(*block_hash);
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Taraxa::taraxa_getScheduleBlockByPeriod(const std::string& _period) {
  try {
    auto app = tryGetApp();
    auto period = dev::jsToInt(_period);
#ifdef RUSTAXA_ENABLE
    const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
    const auto view = query_api->consensus_query_pbft_schedule_block_by_period(period);
    if (!view.found) {
      return Json::Value();
    }
    return pbftScheduleBlockViewToJson(view);
#endif
    auto db = app->getDB();  // RUSTAXA_QUERY_COMPAT_READ
    auto blk = db->getPbftBlock(period);
    if (!blk.has_value()) {
      return Json::Value();
    }
    return PbftBlock::toJson(*blk, db->getFinalizedDagBlockHashesByPeriod(period));
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Taraxa::taraxa_getNodeVersions() {
  try {
    auto app = tryGetApp();
    auto db = app->getDB();  // RUSTAXA_QUERY_COMPAT_READ
    const uint64_t max_blocks_to_process = 6000;
    std::map<addr_t, std::string> node_version_map;
    std::multimap<std::string, std::pair<addr_t, uint64_t>> version_node_map;
    std::map<std::string, std::pair<uint32_t, uint32_t>> version_count;
#ifdef RUSTAXA_ENABLE
    const auto query_api = rustaxa::create_consensus_query_api(db->rustStorage());
    auto period = query_api->consensus_query_final_chain_last_block_number();
    for (uint64_t i = period; i > 0 && period - i < max_blocks_to_process; i--) {
      const auto version_view = query_api->consensus_query_pbft_node_version_by_period(i);
      if (!version_view.found) {
        break;
      }
      const auto beneficiary = addressFromBridge(version_view.beneficiary);
      if (!node_version_map.contains(beneficiary)) {
        node_version_map[beneficiary] = std::to_string(version_view.major_version) + "." +
                                        std::to_string(version_view.minor_version) + "." +
                                        std::to_string(version_view.patch_version);
      }
    }
#endif
#ifndef RUSTAXA_ENABLE
    auto period = app->getFinalChain()->lastBlockNumber();
    for (uint64_t i = period; i > 0 && period - i < max_blocks_to_process; i--) {
      auto blk = db->getPbftBlock(i);
      if (!blk.has_value()) {
        break;
      }
      if (!node_version_map.contains(blk->getBeneficiary())) {
        node_version_map[blk->getBeneficiary()] = blk->getExtraData()->getJson()["major_version"].asString() + "." +
                                                  blk->getExtraData()->getJson()["minor_version"].asString() + "." +
                                                  blk->getExtraData()->getJson()["patch_version"].asString();
      }
    }
#endif

    auto total_vote_count = dpos_reader_.eligible_total_vote_count(period);
    for (auto nv : node_version_map) {
      auto vote_count = dpos_reader_.eligible_vote_count(period, nv.first);
      version_node_map.insert({nv.second, {nv.first, vote_count}});
      version_count[nv.second].first++;
      version_count[nv.second].second += vote_count;
    }

    Json::Value res;
    res["nodes"] = Json::Value(Json::arrayValue);
    for (auto vn : version_node_map) {
      Json::Value node_json;
      node_json["node"] = vn.second.first.toString();
      node_json["version"] = vn.first;
      node_json["vote_count"] = vn.second.second;
      res["nodes"].append(node_json);
    }
    res["versions"] = Json::Value(Json::arrayValue);
    for (auto vc : version_count) {
      Json::Value version_json;
      version_json["version"] = vc.first;
      version_json["node_count"] = vc.second.first;
      version_json["vote_percentage"] = vc.second.second * 100 / total_vote_count;
      res["versions"].append(version_json);
    }
    return res;
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Taraxa::taraxa_getDagBlockByLevel(const string& _blockLevel, bool _includeTransactions) {
  try {
#ifdef RUSTAXA_ENABLE
    if (auto app = app_.lock()) {
      const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
      const auto rust_blocks = query_api->consensus_query_dag_blocks_by_level(dev::jsToInt(_blockLevel), 1);
      auto rust_res = Json::Value(Json::arrayValue);
      for (auto const& block : rust_blocks) {
        auto block_json = dagBlockPublicViewToJson(block);
        if (block.finalized_period_found) {
          block_json["period"] = toJS(block.finalized_period);
        } else {
          block_json["period"] = "-0x1";
        }
        if (_includeTransactions) {
          appendDagBlockTransactionsFromQuery(block_json, block.transactions, query_api);
        }
        rust_res.append(block_json);
      }
      return rust_res;
    }
#endif
    auto blocks = dag_block_reader_.blocks_by_level(dev::jsToInt(_blockLevel));
    auto res = Json::Value(Json::arrayValue);
    for (auto const& b : blocks) {
      auto block_json = b->getJson();
      const auto period = dag_block_reader_.period_by_hash(b->getHash());
      if (period) {
        block_json["period"] = toJS(*period);
      } else {
        block_json["period"] = "-0x1";
      }
      if (_includeTransactions) {
        block_json["transactions"] = Json::Value(Json::arrayValue);
        for (auto const& t : b->getTrxs()) {
          auto transaction = dag_block_reader_.transaction_by_hash(t);
          if (!transaction) {
            throw std::runtime_error("TARAXA_DAG_BLOCK_TRANSACTION_MISSING");
          }
          block_json["transactions"].append(transaction->toJSON());
        }
      }
      res.append(block_json);
    }
    return res;
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Taraxa::taraxa_getConfig() { return enc_json(tryGetApp()->getConfig().genesis); }

Json::Value Taraxa::taraxa_getChainStats() {
  Json::Value res;
#ifdef RUSTAXA_ENABLE
  if (auto app = app_.lock()) {
    const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
    const auto stats = query_api->consensus_query_chain_stats();
    res["pbft_period"] = Json::UInt64(stats.pbft_period);
    res["dag_blocks_executed"] = Json::UInt64(stats.dag_blocks_executed);
    res["transactions_executed"] = Json::UInt64(stats.transactions_executed);
    return res;
  }
#endif
  const auto stats = persistent_reader_.chain_stats();
  res["pbft_period"] = Json::UInt64(stats.pbft_period);
  res["dag_blocks_executed"] = Json::UInt64(stats.dag_blocks_executed);
  res["transactions_executed"] = Json::UInt64(stats.transactions_executed);

  return res;
}

std::string Taraxa::taraxa_yield(const std::string& _period) {
  try {
    auto period = dev::jsToInt(_period);
    return toJS(dpos_reader_.dpos_yield(period));
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

std::string Taraxa::taraxa_totalSupply(const std::string& _period) {
  try {
    auto period = dev::jsToInt(_period);
    return toJS(dpos_reader_.total_supply(period));
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Taraxa::taraxa_getPillarBlockData(const std::string& pillar_block_period, bool include_signatures) {
  try {
    auto app = app_.lock();
    if (!app) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }

    const auto pbft_period = dev::jsToInt(pillar_block_period);
    if (!app->getConfig().genesis.state.hardforks.ficus_hf.isPillarBlockPeriod(pbft_period)) {
      return {};
    }

#ifdef RUSTAXA_ENABLE
    const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
    const auto pillar_block_data = query_api->consensus_query_pillar_block_data_by_period(pbft_period);
    if (!pillar_block_data.found) {
      return {};
    }
    return pillarBlockDataViewToJson(pillar_block_data, include_signatures);
#endif

    const auto pillar_block = app->getDB()->getPillarBlock(pbft_period);  // RUSTAXA_QUERY_COMPAT_READ
    if (!pillar_block) {
      return {};
    }

    const auto& pillar_votes = app->getDB()->getPeriodPillarVotes(pbft_period + 1);  // RUSTAXA_QUERY_COMPAT_READ
    if (pillar_votes.empty()) {
      return {};
    }

    return pillar_chain::PillarBlockData{pillar_block, pillar_votes}.getJson(include_signatures);
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

std::string Taraxa::taraxa_getPeriodLambda(const std::string& period) {
  try {
    const auto period_number = dev::jsToInt(period);
#ifdef RUSTAXA_ENABLE
    if (auto app = app_.lock()) {
      const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
      const auto period_lambda = query_api->consensus_query_period_lambda_by_period(period_number);
      if (!period_lambda.found) {
        return {};
      }
      return toJS(period_lambda.value);
    }
#endif
    const auto period_lambda = persistent_reader_.period_lambda(period_number);
    if (!period_lambda) {
      return {};
    }

    return toJS(*period_lambda);
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

}  // namespace taraxa::net
