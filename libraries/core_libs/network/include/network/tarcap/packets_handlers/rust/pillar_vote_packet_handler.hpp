#pragma once

#include "network/tarcap/packets_handlers/interface/pillar_vote_packet_handler.hpp"

namespace taraxa::network::tarcap {

/** Rust-mode pillar-vote adapter over complete canonical packet bytes. */
class RustPillarVotePacketHandler final : public IPillarVotePacketHandler {
 public:
  RustPillarVotePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                              std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                              network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
                              const addr_t& node_addr, const std::string& logs_prefix = "");

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kPillarVotePacket;

  /** Routes one canonical pillar vote through native exact-target egress. */
  network::ConsensusPacketOutcome gossipCanonicalPillarVote(const std::vector<uint8_t>& pillar_vote_rlp,
                                                            bool rebroadcast, uint64_t source_payload_id);

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;
  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
