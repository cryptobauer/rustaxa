#include "Net.h"

#include <jsonrpccpp/common/exception.h>
#include <jsonrpccpp/server.h>
#include <libdevcore/CommonData.h>
#include <libdevcore/CommonIO.h>
#include <libdevcore/CommonJS.h>

#include "network/network.hpp"

using namespace dev;
using namespace std;
using namespace jsonrpc;

namespace taraxa::net {

namespace {
NetReader makeNetReader(std::weak_ptr<taraxa::AppBase> app) {
  NetReader reader;
  reader.chain_id = [app] {
    auto node = app.lock();
    if (!node) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }
    return node->getConfig().genesis.chain_id;
  };
  reader.peer_count = [app] {
    auto node = app.lock();
    if (!node) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }
    return static_cast<uint64_t>(node->getNetwork()->getPeerCount());
  };
  reader.listening = [app] {
    auto node = app.lock();
    if (!node) {
      BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INTERNAL_ERROR));
    }
    return node->getNetwork()->isStarted();
  };
  return reader;
}

void fillMissingNetReaderCallbacks(NetReader& reader, std::weak_ptr<taraxa::AppBase> app) {
  auto defaults = makeNetReader(std::move(app));
  if (!reader.chain_id) {
    reader.chain_id = std::move(defaults.chain_id);
  }
  if (!reader.peer_count) {
    reader.peer_count = std::move(defaults.peer_count);
  }
  if (!reader.listening) {
    reader.listening = std::move(defaults.listening);
  }
}
}  // namespace

Net::Net(std::shared_ptr<taraxa::AppBase> const& app, NetReader reader) : app_(app), reader_(std::move(reader)) {
  fillMissingNetReaderCallbacks(reader_, app_);
}

std::string Net::net_version() { return toString(reader_.chain_id()); }

std::string Net::net_peerCount() { return toJS(reader_.peer_count()); }

bool Net::net_listening() { return reader_.listening(); }

}  // namespace taraxa::net
