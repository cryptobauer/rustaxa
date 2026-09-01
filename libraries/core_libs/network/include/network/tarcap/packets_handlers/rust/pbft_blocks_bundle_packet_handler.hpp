#pragma once

#include "network/consensus_network_api.hpp"
#include "network/tarcap/packets_handlers/latest/common/packet_handler.hpp"

namespace taraxa::network::tarcap {

/**
 * Rust-mode PbftBlocksBundle transport adapter.
 *
 * The handler retains only last-sync-peer gating and error execution. Canonical
 * packet decoding, relevance, author uniqueness, DPoS eligibility, and proposal
 * publication are owned by the native consensus network service.
 */
class RustPbftBlocksBundlePacketHandler final : public PacketHandler {
 public:
  RustPbftBlocksBundlePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                    network::ConsensusNetworkApiShared consensus_network_api, const addr_t& node_addr,
                                    const std::string& logs_prefix = "");
  ~RustPbftBlocksBundlePacketHandler() override;

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kPbftBlocksBundlePacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

 private:
  network::ConsensusNetworkApiShared consensus_network_api_;
};

}  // namespace taraxa::network::tarcap
