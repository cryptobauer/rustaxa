#pragma once

#include "network/tarcap/packets_handlers/interface/vote_packet_handler.hpp"

namespace taraxa::network::tarcap {

/** Rust-mode PBFT vote adapter that forwards complete canonical packet bytes to the application network service. */
class RustVotePacketHandler final : public IVotePacketHandler {
 public:
  RustVotePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                        std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                        network::ConsensusLiveStatusProvider consensus_status,
                        net::ConsensusQueryClient consensus_query,
                        network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
                        const addr_t& node_addr, const std::string& logs_prefix = "");

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kVotePacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;
  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
