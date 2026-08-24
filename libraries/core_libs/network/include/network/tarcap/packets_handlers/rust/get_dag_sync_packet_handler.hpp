#pragma once

#include "network/consensus_network_api.hpp"
#include "network/tarcap/packets_handlers/latest/common/packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/** Transport-only Rust-mode adapter for canonical get-DAG-sync requests. */
class RustGetDagSyncPacketHandler final : public PacketHandler {
 public:
  RustGetDagSyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                              std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                              network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
                              const addr_t& node_addr, const std::string& logs_prefix = "");

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kGetDagSyncPacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

  network::ConsensusNetworkApiShared consensus_network_api_;
  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
