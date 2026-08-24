#pragma once

#include "network/consensus_network_api.hpp"
#include "network/tarcap/packets_handlers/interface/sync_packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/** Rust-mode DAG-sync packet transport adapter over native sequential admission. */
class RustDagSyncPacketHandler final : public ISyncPacketHandler {
 public:
  RustDagSyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                           std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                           std::shared_ptr<PbftSyncingState> pbft_syncing_state,
                           net::ConsensusQueryClient consensus_query,
                           network::ConsensusLiveStatusProvider consensus_status,
                           network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
                           const addr_t& node_addr, const std::string& logs_prefix = "");

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kDagSyncPacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

  network::ConsensusNetworkApiShared consensus_network_api_;
  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
