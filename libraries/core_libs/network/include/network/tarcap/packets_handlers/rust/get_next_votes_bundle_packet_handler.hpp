#pragma once

#include "network/tarcap/packets_handlers/rust/consensus_transport_packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/**
 * Rust-mode previous-round next-vote transport adapter.
 *
 * The adapter passes exact canonical bytes to native consensus. The native network
 * service owns strict decoding, the shared manager period/round snapshot, eligibility,
 * vote lookup, canonical bundle validation, chunking, and ordered sends.
 * It deliberately holds no PBFT chain, vote manager, slashing manager, or
 * legacy vote-handler runtime.
 */
class RustGetNextVotesBundlePacketHandler final : public RustConsensusTransportPacketHandler {
 public:
  RustGetNextVotesBundlePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                      std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                      net::ConsensusQueryClient consensus_query,
                                      network::ConsensusLiveStatusProvider consensus_status,
                                      network::ConsensusNetworkApiShared consensus_network_api,
                                      TarcapVersion transport_lane, const addr_t& node_addr,
                                      const std::string& logs_prefix = "");
  ~RustGetNextVotesBundlePacketHandler() override;

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kGetNextVotesSyncPacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
