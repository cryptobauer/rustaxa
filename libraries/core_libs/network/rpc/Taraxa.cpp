#include "Taraxa.h"

#include <json/reader.h>
#include <jsonrpccpp/common/exception.h>
#include <libdevcore/CommonData.h>
#include <libdevcore/CommonJS.h>
#include <libp2p/Common.h>

#include "config/version.hpp"
#include "dag/dag_manager.hpp"
#include "pbft/pbft_manager.hpp"
#include "pillar_chain/pillar_block.hpp"
#include "transaction/transaction_manager.hpp"

using namespace std;
using namespace jsonrpc;
using namespace dev;
using namespace taraxa;
using namespace ::taraxa::final_chain;

namespace taraxa::net {

#ifdef RUSTAXA_ENABLE
namespace {
dev::h256 hashFromBridge(const std::array<uint8_t, 32>& hash) {
  return dev::h256(hash.data(), dev::h256::ConstructFromPointer);
}

dev::Address addressFromBridge(const std::array<uint8_t, 20>& address) {
  return dev::Address(address.data(), dev::Address::ConstructFromPointer);
}

dev::bytes bytesFromBridge(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

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

Taraxa::Taraxa(std::shared_ptr<AppBase> app) : app_(app) {
  Json::CharReaderBuilder builder;
  auto reader = std::unique_ptr<Json::CharReader>(builder.newCharReader());

  bool parsingSuccessful = reader->parse(kVersionJson, kVersionJson + strlen(kVersionJson), &version, nullptr);
  assert(parsingSuccessful);
}

string Taraxa::taraxa_protocolVersion() { return toJS(TARAXA_NET_VERSION); }

Json::Value Taraxa::taraxa_getVersion() { return version; }

string Taraxa::taraxa_dagBlockLevel() {
  try {
    auto app = tryGetApp();
    return toJS(app->getDagManager()->getMaxLevel());
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

string Taraxa::taraxa_dagBlockPeriod() {
  try {
    auto app = tryGetApp();
    return toJS(app->getDagManager()->getLatestPeriod());
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
    auto app = tryGetApp();
#ifdef RUSTAXA_ENABLE
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
        block_json["transactions"] = Json::Value(Json::arrayValue);
        for (auto const& t : rust_block.transactions) {
          block_json["transactions"].append(
              app->getTransactionManager()->getTransaction(hashFromBridge(t.hash))->toJSON());
        }
      }
      return block_json;
    }
    return Json::Value();
#endif
    auto block = app->getDagManager()->getDagBlock(blk_hash_t(_blockHash));
    if (block) {
      auto block_json = block->getJson();
      auto period = app->getPbftManager()->getDagBlockPeriod(block->getHash());
      if (period.first) {
        block_json["period"] = toJS(period.second);
      } else {
        block_json["period"] = "-0x1";
      }
      if (_includeTransactions) {
        block_json["transactions"] = Json::Value(Json::arrayValue);
        for (auto const& t : block->getTrxs()) {
          block_json["transactions"].append(app->getTransactionManager()->getTransaction(t)->toJSON());
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
    auto app = tryGetApp();
#ifdef RUSTAXA_ENABLE
    const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
    const auto lookup = query_api->consensus_query_pbft_block_hash_by_period(dev::jsToInt(_period));
    if (!lookup.found) {
      return {};
    }
    return toJS(hashFromBridge(lookup.hash));
#endif
    auto db = app->getDB();  // RUSTAXA_QUERY_COMPAT_READ
    auto blk = db->getPbftBlock(dev::jsToInt(_period));
    if (!blk.has_value()) {
      return {};
    }
    return toJS(blk->getBlockHash());
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
    auto period = app->getFinalChain()->lastBlockNumber();
    const uint64_t max_blocks_to_process = 6000;
    std::map<addr_t, std::string> node_version_map;
    std::multimap<std::string, std::pair<addr_t, uint64_t>> version_node_map;
    std::map<std::string, std::pair<uint32_t, uint32_t>> version_count;
#ifdef RUSTAXA_ENABLE
    const auto query_api = rustaxa::create_consensus_query_api(db->rustStorage());
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

    auto total_vote_count = app->getFinalChain()->dposEligibleTotalVoteCount(period);
    for (auto nv : node_version_map) {
      auto vote_count = app->getFinalChain()->dposEligibleVoteCount(period, nv.first);
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
    auto app = tryGetApp();
#ifdef RUSTAXA_ENABLE
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
        block_json["transactions"] = Json::Value(Json::arrayValue);
        for (auto const& t : block.transactions) {
          block_json["transactions"].append(
              app->getTransactionManager()->getTransaction(hashFromBridge(t.hash))->toJSON());
        }
      }
      rust_res.append(block_json);
    }
    return rust_res;
#endif
    auto blocks = app->getDB()->getDagBlocksAtLevel(dev::jsToInt(_blockLevel), 1);  // RUSTAXA_QUERY_COMPAT_READ
    auto res = Json::Value(Json::arrayValue);
    for (auto const& b : blocks) {
      auto block_json = b->getJson();
      auto period = app->getPbftManager()->getDagBlockPeriod(b->getHash());
      if (period.first) {
        block_json["period"] = toJS(period.second);
      } else {
        block_json["period"] = "-0x1";
      }
      if (_includeTransactions) {
        block_json["transactions"] = Json::Value(Json::arrayValue);
        for (auto const& t : b->getTrxs()) {
          block_json["transactions"].append(app->getTransactionManager()->getTransaction(t)->toJSON());
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
  if (auto app = app_.lock()) {
    res["pbft_period"] = Json::UInt64(app->getFinalChain()->lastBlockNumber());
    res["dag_blocks_executed"] = Json::UInt64(app->getDB()->getNumBlockExecuted());  // RUSTAXA_QUERY_COMPAT_READ
    res["transactions_executed"] =
        Json::UInt64(app->getDB()->getNumTransactionExecuted());  // RUSTAXA_QUERY_COMPAT_READ
  }

  return res;
}

std::string Taraxa::taraxa_yield(const std::string& _period) {
  try {
    auto app = app_.lock();
    if (!app) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }

    auto period = dev::jsToInt(_period);
    return toJS(app->getFinalChain()->dposYield(period));
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

std::string Taraxa::taraxa_totalSupply(const std::string& _period) {
  try {
    auto app = app_.lock();
    if (!app) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }

    auto period = dev::jsToInt(_period);
    return toJS(app->getFinalChain()->dposTotalSupply(period));
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
    auto app = tryGetApp();
#ifdef RUSTAXA_ENABLE
    {
      const auto query_api = rustaxa::create_consensus_query_api(app->getDB()->rustStorage());
      const auto period_lambda = query_api->consensus_query_period_lambda_by_period(dev::jsToInt(period));
      if (!period_lambda.found) {
        return {};
      }
      return toJS(period_lambda.value);
    }
#endif
    auto db = app->getDB();  // RUSTAXA_QUERY_COMPAT_READ
    auto period_lambda = db->getPeriodLambda(dev::jsToInt(period), false);
    if (!period_lambda.has_value()) {
      return {};
    }

    return toJS(*period_lambda);
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

}  // namespace taraxa::net
