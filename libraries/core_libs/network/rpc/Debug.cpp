#include "Debug.h"

#include <libdevcore/CommonData.h>
#include <libdevcore/CommonJS.h>

#include "common/jsoncpp.hpp"
#include "common/rpc_utils.hpp"
#include "final_chain/state_api_data.hpp"
#include "network/rpc/eth/data.hpp"
#include "transaction/system_transaction.hpp"
#include "transaction/transaction.hpp"
#include "vote_manager/vote_manager.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

using namespace std;
using namespace dev;
using namespace jsonrpc;
using namespace taraxa;

namespace taraxa::net {

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

std::shared_ptr<Transaction> materializeReceiptTransactionView(const rustaxa::TransactionReceiptPublicView& view) {
  if (!view.found) {
    return nullptr;
  }

  std::shared_ptr<Transaction> transaction;
  if (view.transaction_source == kConsensusQueryTransactionSourceFinalizedSystem) {
    transaction = std::make_shared<SystemTransaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.transaction_source == kConsensusQueryTransactionSourcePending ||
             view.transaction_source == kConsensusQueryTransactionSourceFinalizedRegular) {
    transaction = std::make_shared<Transaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.transaction_source != kConsensusQueryTransactionSourceMissing) {
    throw std::runtime_error("CONSENSUS_QUERY_DEBUG_RECEIPT_TRANSACTION_UNKNOWN_SOURCE");
  }

  if (transaction && transaction->getHash() != hashFromBridge(view.transaction_hash)) {
    throw std::runtime_error("CONSENSUS_QUERY_DEBUG_RECEIPT_TRANSACTION_HASH_MISMATCH");
  }
  return transaction;
}

rpc::eth::ExtendedTransactionLocation receiptLocationFromView(const rustaxa::TransactionReceiptPublicView& view) {
  if (!view.block_hash_found) {
    throw std::runtime_error("CONSENSUS_QUERY_DEBUG_RECEIPT_BLOCK_HASH_MISSING");
  }

  rpc::eth::ExtendedTransactionLocation location;
  location.period = view.block_number;
  location.position = view.transaction_index;
  location.is_system = view.is_system;
  location.blk_h = hashFromBridge(view.block_hash);
  location.trx_hash = hashFromBridge(view.transaction_hash);
  return location;
}

Json::Value dagBlockPublicViewToJson(const rustaxa::DagBlockPublicView& view, uint64_t period) {
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
    json["vdf"] = std::move(vdf);
  }
  json["period"] = dev::toJS(period);
  return json;
}
}  // namespace
#endif

Json::Value Debug::debug_traceCall(const Json::Value& call_params, const std::string& blk_num) {
  Json::Value res;
  const auto block = parse_blk_num(blk_num);
  auto trx = to_eth_trx(call_params, block);
  if (auto node = app_.lock()) {
    return util::readJsonFromString(node->getFinalChain()->trace({}, {std::move(trx)}, block));
  }
  return res;
}

Json::Value Debug::trace_call(const Json::Value& call_params, const Json::Value& trace_params,
                              const std::string& blk_num) {
  Json::Value res;
  const auto block = parse_blk_num(blk_num);
  auto params = parse_tracking_parms(trace_params);
  if (auto node = app_.lock()) {
    return util::readJsonFromString(
        node->getFinalChain()->trace({}, {to_eth_trx(call_params, block)}, block, std::move(params)));
  }
  return res;
}

std::tuple<std::vector<state_api::EVMTransaction>, state_api::EVMTransaction, uint64_t>
Debug::get_transaction_with_state(const std::string& transaction_hash) {
  auto node = app_.lock();
  if (!node) {
    return {};
  }

  auto final_chain = node->getFinalChain();
  auto loc = final_chain->transactionLocation(jsToFixed<32>(transaction_hash));
  if (!loc) {
    throw std::runtime_error("Transaction not found");
  }
  auto block_transactions = final_chain->transactions(loc->period);

  auto state_trxs = SharedTransactions(block_transactions.begin(), block_transactions.begin() + loc->position);

  return {to_eth_trxs(state_trxs), to_eth_trx(block_transactions[loc->position]), loc->period};
}
Json::Value Debug::debug_traceTransaction(const std::string& transaction_hash) {
  Json::Value res;
  auto [state_trxs, trx, period] = get_transaction_with_state(transaction_hash);
  if (auto node = app_.lock()) {
    return util::readJsonFromString(node->getFinalChain()->trace({}, {trx}, period));
  }
  return res;
}

Json::Value Debug::trace_replayTransaction(const std::string& transaction_hash, const Json::Value& trace_params) {
  Json::Value res;
  auto params = parse_tracking_parms(trace_params);
  auto [state_trxs, trx, period] = get_transaction_with_state(transaction_hash);
  if (auto node = app_.lock()) {
    return util::readJsonFromString(node->getFinalChain()->trace(state_trxs, {trx}, period, params));
  }
  return res;
}

bool only_transfers(const SharedTransactions& trxs) {
  return std::all_of(trxs.begin(), trxs.end(), [](const SharedTransaction& trx) {
    return trx->getReceiver().has_value() && trx->getData().empty() && trx->getGas() <= 22000;
  });
}

Json::Value Debug::trace_replayBlockTransactions(const std::string& block_num, const Json::Value& trace_params) {
  Json::Value res;
  const auto block = parse_blk_num(block_num);
  auto params = parse_tracking_parms(trace_params);
  if (auto node = app_.lock()) {
    auto transactions = node->getDB()->getPeriodTransactions(block);  // RUSTAXA_QUERY_COMPAT_READ
    if (!transactions.has_value() || transactions->empty()) {
      return Json::Value(Json::arrayValue);
    }
    if (only_transfers(*transactions)) {
      return Json::Value(Json::arrayValue);
    }
    std::vector<state_api::EVMTransaction> trxs = to_eth_trxs(*transactions);
    return util::readJsonFromString(node->getFinalChain()->trace({}, std::move(trxs), block, std::move(params)));
  }
  return res;
}

Json::Value Debug::debug_getPeriodTransactionsWithReceipts(const std::string& _period) {
  try {
    auto node = app_.lock();
    if (!node) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }
    auto final_chain = node->getFinalChain();
    auto period = dev::jsToInt(_period);
#ifdef RUSTAXA_ENABLE
    const auto query_api = rustaxa::create_consensus_query_api(node->getDB()->rustStorage());
    const auto receipt_views = query_api->consensus_query_transaction_receipts_by_block_number(period);
    Json::Value result(Json::arrayValue);
    for (const auto& view : receipt_views) {
      auto trx = materializeReceiptTransactionView(view);
      if (!trx) {
        throw std::runtime_error("CONSENSUS_QUERY_DEBUG_RECEIPT_TRANSACTION_MISSING");
      }
      auto location = receiptLocationFromView(view);
      auto transaction = rpc::eth::LocalisedTransaction{trx, location};
      auto receipt_bytes = bytesFromBridge(view.receipt_rlp);
      auto receipt = rpc::eth::LocalisedTransactionReceipt{
          util::rlp_dec<TransactionReceipt>(dev::RLP(receipt_bytes)), location, trx->getSender(), trx->getReceiver()};
      auto receipt_json = rpc::eth::toJson(receipt);
      receipt_json.removeMember("transactionHash");
      result.append(util::mergeJsons(rpc::eth::toJson(transaction), std::move(receipt_json)));
    }
    return result;
#endif
    auto block_hash = final_chain->blockHash(period);
    auto trxs = node->getDB()->getPeriodTransactions(period);  // RUSTAXA_QUERY_COMPAT_READ
    if (!trxs.has_value() || trxs->empty()) {
      return Json::Value(Json::arrayValue);
    }

    auto receipts = final_chain->blockReceipts(period);

    return util::transformToJsonParallel(*trxs, [&final_chain, &receipts, &block_hash, &period](const auto& trx,
                                                                                                auto index) {
      auto hash = trx->getHash();

      auto location = rpc::eth::ExtendedTransactionLocation{{TransactionLocation{period, index}, *block_hash}, hash};
      auto transaction = rpc::eth::LocalisedTransaction{trx, location};
      rpc::eth::LocalisedTransactionReceipt receipt;
      if (!receipts) {
        receipt = rpc::eth::LocalisedTransactionReceipt{final_chain->transactionReceipt(period, index, hash).value(),
                                                        location, trx->getSender(), trx->getReceiver()};
      } else {
        receipt =
            rpc::eth::LocalisedTransactionReceipt{receipts->at(index), location, trx->getSender(), trx->getReceiver()};
      }

      auto receipt_json = rpc::eth::toJson(receipt);
      receipt_json.removeMember("transactionHash");

      return util::mergeJsons(rpc::eth::toJson(transaction), std::move(receipt_json));
    });
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Debug::debug_getPeriodDagBlocks(const std::string& _period) {
  try {
    auto node = app_.lock();
    if (!node) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }

    auto period = dev::jsToInt(_period);
#ifdef RUSTAXA_ENABLE
    const auto query_api = rustaxa::create_consensus_query_api(node->getDB()->rustStorage());
    const auto dag_views = query_api->consensus_query_finalized_dag_blocks_by_period(period);
    Json::Value res(Json::arrayValue);
    for (const auto& dag_view : dag_views) {
      res.append(dagBlockPublicViewToJson(dag_view, period));
    }
    return res;
#endif
    auto dags = node->getDB()->getFinalizedDagBlockByPeriod(period);  // RUSTAXA_QUERY_COMPAT_READ

    return util::transformToJsonParallel(dags, [&period](const auto& dag, auto) {
      auto block_json = dag->getJson();
      block_json["period"] = toJS(period);
      return block_json;
    });
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Debug::debug_getPreviousBlockCertVotes(const std::string& _period) {
  try {
    auto node = app_.lock();
    if (!node) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }

    auto final_chain = node->getFinalChain();
    auto vote_manager = node->getVoteManager();

    Json::Value res(Json::objectValue);

    auto period = dev::jsToInt(_period);
    auto votes = node->getDB()->getPeriodCertVotes(period);  // RUSTAXA_QUERY_COMPAT_READ
    if (votes.empty()) {
      return res;
    }

    const auto& front_vote = votes.front();
    const auto votes_period = front_vote->getPeriod();
    const auto round = front_vote->getRound();
    const uint64_t total_dpos_votes_count = final_chain->dposEligibleTotalVoteCount(votes_period - 1);
    res["total_votes_count"] = total_dpos_votes_count;
    res["round"] = round;
    res["votes"] = util::transformToJsonParallel(votes, [&](const auto& vote, auto) {
      vote_manager->validateVote(vote);
      return vote->toJSON();
    });
    return res;
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Debug::debug_dposValidatorTotalStakes(const std::string& _period) {
  try {
    auto node = app_.lock();
    if (!node) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }

    auto final_chain = node->getFinalChain();
    auto vote_manager = node->getVoteManager();

    auto period = dev::jsToInt(_period);
    auto validatorsStakes = final_chain->dposValidatorsTotalStakes(period);

    Json::Value res(Json::arrayValue);

    for (auto const& validatorStake : validatorsStakes) {
      Json::Value validatorStakeJson(Json::objectValue);
      validatorStakeJson["address"] = "0x" + validatorStake.addr.toString();
      validatorStakeJson["total_stake"] = validatorStake.stake.str();
      res.append(validatorStakeJson);
    }
    return res;
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Debug::debug_dposTotalAmountDelegated(const std::string& _period) {
  try {
    auto node = app_.lock();
    if (!node) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }

    auto final_chain = node->getFinalChain();

    auto period = dev::jsToInt(_period);
    auto totalAmountDelegated = final_chain->dposTotalAmountDelegated(period);

    return toJS(totalAmountDelegated);
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

state_api::Tracing Debug::parse_tracking_parms(const Json::Value& json) const {
  state_api::Tracing ret;
  if (!json.isArray() || json.empty()) {
    throw InvalidTracingParams();
  }
  for (const auto& obj : json) {
    if (obj.asString() == "trace") ret.trace = true;
    // Disabled for now
    // if (obj.asString() == "stateDiff") ret.stateDiff = true;
    if (obj.asString() == "vmTrace") ret.vmTrace = true;
  }
  return ret;
}

std::vector<state_api::EVMTransaction> Debug::to_eth_trxs(const std::vector<std::shared_ptr<Transaction>>& trxs) {
  std::vector<state_api::EVMTransaction> eth_trxs;
  eth_trxs.reserve(trxs.size());
  std::transform(trxs.begin(), trxs.end(), std::back_inserter(eth_trxs),
                 [this](auto t) { return to_eth_trx(std::move(t)); });
  return eth_trxs;
}
state_api::EVMTransaction Debug::to_eth_trx(std::shared_ptr<Transaction> t) const {
  return state_api::EVMTransaction{
      t->getSender(), t->getGasPrice(), t->getReceiver(), t->getNonce(), t->getValue(), t->getGas(), t->getData(),
  };
}

state_api::EVMTransaction Debug::to_eth_trx(const Json::Value& json, EthBlockNumber blk_num) {
  state_api::EVMTransaction trx;
  if (!json.isObject() || json.empty()) {
    return trx;
  }

  if (!json["from"].empty()) {
    trx.from = to_address(json["from"].asString());
  } else {
    trx.from = ZeroAddress;
  }

  if (!json["to"].empty() && json["to"].asString() != "0x" && !json["to"].asString().empty()) {
    trx.to = to_address(json["to"].asString());
  }

  if (!json["value"].empty()) {
    trx.value = jsToU256(json["value"].asString());
  }

  if (!json["gas"].empty()) {
    trx.gas = jsToInt(json["gas"].asString());
  } else {
    trx.gas = kGasLimit;
  }

  if (!json["gasPrice"].empty()) {
    trx.gas_price = jsToU256(json["gasPrice"].asString());
  } else {
    trx.gas_price = 0;
  }

  if (!json["data"].empty()) {
    trx.input = jsToBytes(json["data"].asString(), OnFailed::Throw);
  }
  if (!json["code"].empty()) {
    trx.input = jsToBytes(json["code"].asString(), OnFailed::Throw);
  }
  if (!json["nonce"].empty()) {
    trx.nonce = jsToU256(json["nonce"].asString());
  } else {
    if (auto node = app_.lock()) {
      trx.nonce = node->getFinalChain()->getAccount(trx.from, blk_num).value_or(state_api::ZeroAccount).nonce;
    }
  }

  return trx;
}

EthBlockNumber Debug::parse_blk_num(const std::string& blk_num_str) {
  if (blk_num_str == "latest" || blk_num_str == "pending" || blk_num_str.empty()) {
    if (auto node = app_.lock()) {
      return node->getFinalChain()->lastBlockNumber();
    }
  } else if (blk_num_str == "earliest") {
    return 0;
  }
  return jsToInt(blk_num_str);
}

Address Debug::to_address(const std::string& s) const {
  try {
    if (auto b = fromHex(s.substr(0, 2) == "0x" ? s.substr(2) : s, WhenError::Throw); b.size() == Address::size) {
      return Address(b);
    }
  } catch (BadHexCharacter&) {
  }
  throw InvalidAddress();
}

}  // namespace taraxa::net
