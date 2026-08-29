#pragma once

#include "network/tarcap/packets_handlers/rust/consensus_transport_packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/** Rust-mode adapter for canonical get-pillar-votes-bundle requests. */
class RustGetPillarVotesBundlePacketHandler final : public RustConsensusTransportPacketHandler {
 public:
  RustGetPillarVotesBundlePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                        std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                        net::ConsensusQueryClient consensus_query,
                                        network::ConsensusLiveStatusProvider consensus_status,
                                        network::ConsensusNetworkApiShared consensus_network_api,
                                        TarcapVersion transport_lane, const addr_t& node_addr,
                                        const std::string& logs_prefix = "");
  ~RustGetPillarVotesBundlePacketHandler() override;

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kGetPillarVotesBundlePacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
