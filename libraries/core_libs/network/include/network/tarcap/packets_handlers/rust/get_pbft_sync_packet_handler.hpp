#pragma once

#include "network/consensus_network_api.hpp"
#include "network/tarcap/packets_handlers/latest/common/packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/**
 * Rust-mode Get-PBFT-sync transport adapter shared by tarcap versions 5 and 6.
 *
 * The adapter forwards the canonical request RLP, peer identity, source packet
 * identity, and transport version to the application-owned native network API.
 * It holds no PBFT manager, chain, vote manager, or storage dependency. Native
 * ordered effects are executed only through tarcap packet sealing and the live
 * peer context; malformed or unsupported requests are reported and disconnected
 * according to the returned dependency plan.
 */
class RustGetPbftSyncPacketHandler final : public PacketHandler {
 public:
  RustGetPbftSyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                               std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                               network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
                               const addr_t& node_addr, const std::string& logs_prefix = "");
  ~RustGetPbftSyncPacketHandler() override;

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kGetPbftSyncPacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

  network::ConsensusNetworkApiShared consensus_network_api_;
  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
