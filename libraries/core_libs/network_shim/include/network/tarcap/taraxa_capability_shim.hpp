#pragma once

#include <libdevcore/RLP.h>
#include <libp2p/Common.h>

#include <cstddef>

#include "network/tarcap/packet_types.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa::network::tarcap {

class RustaxaNetworkShim {
 public:
  explicit RustaxaNetworkShim(size_t queue_size);

  bool ingestPacket(SubprotocolPacketType packet_type, const dev::p2p::NodeID& node_id, const dev::RLP& rlp);

 private:
  rust::Box<rustaxa::BridgePacketArena> packet_arena_;
  rust::Box<rustaxa::BridgeNetwork> network_;
};

}  // namespace taraxa::network::tarcap
