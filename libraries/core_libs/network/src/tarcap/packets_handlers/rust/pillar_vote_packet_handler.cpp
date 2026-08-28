#include "network/tarcap/packets_handlers/rust/pillar_vote_packet_handler.hpp"

#include "network/tarcap/packets/latest/pillar_vote_packet.hpp"
#include "network/tarcap/taraxa_peer.hpp"
#include "vote/pillar_vote.hpp"

namespace taraxa::network::tarcap {

RustPillarVotePacketHandler::RustPillarVotePacketHandler(const FullNodeConfig& conf,
                                                         std::shared_ptr<PeersState> peers_state,
                                                         std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                         network::ConsensusNetworkApiShared consensus_network_api,
                                                         TarcapVersion transport_lane, const addr_t& node_addr,
                                                         const std::string& logs_prefix)
    : IPillarVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats), consensus_network_api,
                               transport_lane, node_addr, logs_prefix + "PILLAR_VOTE_PH"),
      transport_lane_(transport_lane) {}

void RustPillarVotePacketHandler::process(const threadpool::PacketData& packet_data,
                                          const std::shared_ptr<TaraxaPeer>& peer) {
  const auto outcome = rust_consensus_network_api_->ingestPillarVotePacket(
      consensusPacketRequest(packet_data, peer, transport_lane_, false, true), consensusTransportExecutor());
  if (outcome.malicious) {
    throw MaliciousPeerException("Native pillar-vote packet rejected peer payload: " + outcome.error_code);
  }
  if (outcome.status != 0) {
    LOG(log_dg_) << "Native pillar-vote ingress skipped packet: " << outcome.error_code;
  }
}

void RustPillarVotePacketHandler::sendPillarVote(const std::shared_ptr<TaraxaPeer>& peer,
                                                 const std::shared_ptr<PillarVote>& vote) {
  if (sealAndSend(peer->getId(), SubprotocolPacketType::kPillarVotePacket, encodePacketRlp(PillarVotePacket(vote)))) {
    peer->markPillarVoteAsKnown(vote->getHash());
  }
}

}  // namespace taraxa::network::tarcap
