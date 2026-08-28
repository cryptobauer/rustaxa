#pragma once

#include "network/tarcap/packets_handlers/rust/consensus_transport_packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/** Rust-mode pillar-vote bundle adapter over complete canonical packet bytes. */
class RustPillarVotesBundlePacketHandler final : public RustConsensusTransportPacketHandler {
 public:
  RustPillarVotesBundlePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                     std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                     network::ConsensusNetworkApiShared consensus_network_api,
                                     TarcapVersion transport_lane, const addr_t& node_addr,
                                     const std::string& logs_prefix = "");

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kPillarVotesBundlePacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;
  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
