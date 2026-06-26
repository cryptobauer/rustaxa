#pragma once

#include <functional>

#include "NetFace.h"
#include "common/app_base.hpp"

namespace taraxa::net {

// NetReader is the net_* RPC boundary for chain and live network facts. The
// RPC methods own JSON-RPC formatting; the adapter supplies the facts without
// exposing AppBase or Network to public methods.
struct NetReader {
  std::function<uint64_t()> chain_id;
  std::function<uint64_t()> peer_count;
  std::function<bool()> listening;
};

class Net : public NetFace {
 public:
  explicit Net(std::shared_ptr<taraxa::AppBase> const& app, NetReader reader = {});
  virtual RPCModules implementedModules() const override { return RPCModules{RPCModule{"net", "1.0"}}; }
  virtual std::string net_version() override;
  virtual std::string net_peerCount() override;
  virtual bool net_listening() override;

 private:
  std::weak_ptr<taraxa::AppBase> app_;
  NetReader reader_;
};

}  // namespace taraxa::net
