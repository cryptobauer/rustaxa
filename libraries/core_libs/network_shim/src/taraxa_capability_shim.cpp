#include "network/tarcap/taraxa_capability_shim.hpp"

namespace taraxa::network::tarcap {

namespace {
rust::Vec<uint8_t> toRustVec(const dev::bytes& bytes) {
  rust::Vec<uint8_t> vec;
  vec.reserve(bytes.size());
  for (auto const& byte : bytes) {
    vec.push_back(static_cast<uint8_t>(byte));
  }
  return vec;
}
}  // namespace

RustaxaNetworkShim::RustaxaNetworkShim(size_t queue_size)
    : packet_arena_(rustaxa::create_packet_arena(1024)),
      network_(rustaxa::create_network(*packet_arena_, queue_size)) {
        network_->start_network();
      }

bool RustaxaNetworkShim::queueIsFull() {
  return network_->queue_is_full();
}

bool RustaxaNetworkShim::ingestPacket(SubprotocolPacketType packet_type, const dev::p2p::NodeID& node_id,
                                      const dev::RLP& rlp) {
  return network_->ingest_network_packet(static_cast<uint8_t>(packet_type), node_id.asArray(),
                                         toRustVec(rlp.data().toBytes()));
}

bool RustaxaNetworkShim::connectPeer(const dev::p2p::NodeID& node_id) {
  return network_->connect_peer(node_id.asArray());
}

void RustaxaNetworkShim::disconnectPeer(const dev::p2p::NodeID& node_id) {
  return network_->disconnect_peer(node_id.asArray());
}

}  // namespace taraxa::network::tarcap
