#pragma once

#include "network/consensus_network_api.hpp"
#include "network/tarcap/packets/latest/get_next_votes_bundle_packet.hpp"
#include "network/tarcap/packets_handlers/latest/common/packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/**
 * Rust-mode previous-round next-vote transport adapter.
 *
 * The adapter decodes only the two scalar request fields. The native network
 * service reads the shared manager period/round snapshot and owns eligibility,
 * vote lookup, canonical bundle validation, chunking, and ordered sends.
 * It deliberately holds no PBFT chain, vote manager, slashing manager, or
 * legacy vote-handler runtime.
 */
class RustGetNextVotesBundlePacketHandler final : public PacketHandler {
 public:
  RustGetNextVotesBundlePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                      std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                      network::ConsensusNetworkApiShared consensus_network_api,
                                      TarcapVersion transport_lane, const addr_t& node_addr,
                                      const std::string& logs_prefix = "");
  ~RustGetNextVotesBundlePacketHandler() override;

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kGetNextVotesSyncPacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

  network::ConsensusNetworkApiShared consensus_network_api_;
  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
