#include "network/tarcap/packets_handlers/rust/pillar_vote_packet_handler.hpp"

#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {
constexpr uint8_t kPillarVoteEgressFamily = 2;
}

RustPillarVotePacketHandler::RustPillarVotePacketHandler(const FullNodeConfig& conf,
                                                         std::shared_ptr<PeersState> peers_state,
                                                         std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                         network::ConsensusNetworkApiShared consensus_network_api,
                                                         TarcapVersion transport_lane, const addr_t& node_addr,
                                                         const std::string& logs_prefix)
    : IPillarVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats), consensus_network_api,
                               transport_lane, node_addr, logs_prefix + "PILLAR_VOTE_PH"),
      transport_lane_(transport_lane) {}

network::ConsensusPacketOutcome RustPillarVotePacketHandler::gossipCanonicalPillarVote(
    const std::vector<uint8_t>& pillar_vote_rlp, bool rebroadcast, uint64_t source_payload_id) {
  return routeConsensusEgress(network::ConsensusEgressRequest{
      kPillarVoteEgressFamily, transport_lane_, source_payload_id, {}, rebroadcast, {}, pillar_vote_rlp, {}});
}

void RustPillarVotePacketHandler::process(const threadpool::PacketData& packet_data,
                                          const std::shared_ptr<TaraxaPeer>& peer) {
  const auto outcome = rust_consensus_network_api_->ingestPillarVotePacket(
      consensusPacketRequest(packet_data, peer, transport_lane_, false), consensusTransportExecutor());
  if (outcome.malicious) {
    throw MaliciousPeerException("Native pillar-vote packet rejected peer payload: " + outcome.error_code);
  }
  if (outcome.accepted_count != 0) {
    routeConsensusEgress(network::ConsensusEgressRequest{kPillarVoteEgressFamily,
                                                         transport_lane_,
                                                         packet_data.id_,
                                                         peer->getId().asArray(),
                                                         false,
                                                         {},
                                                         packet_data.rlp_.data().toBytes(),
                                                         {}});
  }
  if (outcome.status != 0) {
    LOG(log_dg_) << "Native pillar-vote ingress skipped packet: " << outcome.error_code;
  }
}

}  // namespace taraxa::network::tarcap
