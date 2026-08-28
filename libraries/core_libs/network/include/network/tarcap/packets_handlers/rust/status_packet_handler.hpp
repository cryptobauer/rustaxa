#pragma once

#include "network/tarcap/packets_handlers/rust/consensus_transport_packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/** Rust-mode status adapter that applies only native-selected peer bookkeeping and exact transport leaves. */
class RustStatusPacketHandler final : public RustConsensusTransportPacketHandler {
 public:
  RustStatusPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                          std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                          net::ConsensusQueryClient consensus_query,
                          network::ConsensusLiveStatusProvider consensus_status,
                          network::ConsensusNetworkApiShared consensus_network_api, const addr_t& node_addr,
                          const std::string& logs_prefix = "");

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kStatusPacket;

  /** Builds and sends one exact native status packet to a connected or pending peer. */
  bool sendStatus(const dev::p2p::NodeID& node_id, bool initial);
  /** Advances native sync inactivity and sends one periodic status packet to each ready peer. */
  void sendStatusToPeers();

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;
};

}  // namespace taraxa::network::tarcap
