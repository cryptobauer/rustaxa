#include "network/subscriptions.hpp"

#include <libdevcore/CommonJS.h>

#include <mutex>

#include "common/jsoncpp.hpp"

namespace taraxa::net {

int Subscriptions::addSubscription(std::shared_ptr<Subscription> subscription) {
  std::lock_guard<std::mutex> lock(subscriptions_mutex_);
  subscriptions_[subscription->getId()] = subscription;
  subscriptions_by_type_[subscription->getType()].push_back(subscription->getId());
  return subscription->getId();
}

bool Subscriptions::removeSubscription(int id) {
  std::lock_guard<std::mutex> lock(subscriptions_mutex_);
  auto it = subscriptions_.find(id);
  if (it == subscriptions_.end()) {
    return false;
  }
  auto sub = it->second;
  auto& subs = subscriptions_by_type_[sub->getType()];
  subs.erase(std::remove(subs.begin(), subs.end(), id), subs.end());
  subscriptions_.erase(it);
  return true;
}

void Subscriptions::process(SubscriptionType type, const Json::Value& payload) {
  for (auto id : subscriptions_by_type_[type]) {
    send_(subscriptions_[id]->processPayload(payload));
  }
}
void Subscriptions::processLogs(const rpc::eth::LiveLogBlock& block) {
  if (!live_logs_.matching_logs) {
    return;
  }

  for (auto id : subscriptions_by_type_[SubscriptionType::LOGS]) {
    auto sub = std::dynamic_pointer_cast<LogsSubscription>(subscriptions_[id]);
    if (!sub) {
      continue;
    }

    for (const auto& log : live_logs_.matching_logs(sub->getFilter(), block)) {
      send_(sub->processPayload(toJson(log)));
    }
  }
}

std::string makeEthSubscriptionResponse(int id, const Json::Value& payload) {
  Json::Value params;
  params["result"] = payload;
  params["subscription"] = dev::toJS(id);
  Json::Value res;
  res["jsonrpc"] = "2.0";
  res["method"] = "eth_subscription";
  res["params"] = params;

  return util::to_string(res);
}

std::string HeadsSubscription::processPayload(Json::Value payload) const {
  return makeEthSubscriptionResponse(id_, payload);
}

std::string DagBlocksSubscription::processPayload(Json::Value payload) const {
  if (!full_data_) {
    payload = payload["hash"];
  }
  return makeEthSubscriptionResponse(id_, payload);
}

std::string TransactionsSubscription::processPayload(Json::Value payload) const {
  return makeEthSubscriptionResponse(id_, payload);
}

std::string DagBlockFinalizedSubscription::processPayload(Json::Value payload) const {
  return makeEthSubscriptionResponse(id_, payload);
}

std::string PbftBlockExecutedSubscription::processPayload(Json::Value payload) const {
  if (!full_block_) {
    payload = payload["block_hash"];
  }
  return makeEthSubscriptionResponse(id_, payload);
}

std::string PillarBlockSubscription::processPayload(Json::Value payload) const {
  if (!include_signatures_) {
    payload.removeMember("signatures");
  }
  return makeEthSubscriptionResponse(id_, payload);
}

std::string LogsSubscription::processPayload(Json::Value payload) const {
  return makeEthSubscriptionResponse(id_, payload);
}

}  // namespace taraxa::net
