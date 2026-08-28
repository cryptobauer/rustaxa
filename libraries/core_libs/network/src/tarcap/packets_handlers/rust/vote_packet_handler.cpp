#include "network/tarcap/packets_handlers/rust/vote_packet_handler.hpp"

#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {
constexpr uint8_t kPbftVoteEgressFamily = 0;
}

RustVotePacketHandler::RustVotePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                             std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                             network::ConsensusLiveStatusProvider consensus_status,
                                             net::ConsensusQueryClient consensus_query,
                                             network::ConsensusNetworkApiShared consensus_network_api,
                                             TarcapVersion transport_lane, const addr_t& node_addr,
                                             const std::string& logs_prefix)
    : IVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(consensus_status),
                         std::move(consensus_query), consensus_network_api, node_addr, logs_prefix + "PBFT_VOTE_PH"),
      transport_lane_(transport_lane) {}

network::ConsensusPacketOutcome RustVotePacketHandler::gossipCanonicalVote(
    const std::vector<uint8_t>& vote_rlp, const std::vector<uint8_t>& proposed_block_rlp, bool rebroadcast,
    uint64_t source_payload_id) {
  return routeConsensusEgress(network::ConsensusEgressRequest{
      kPbftVoteEgressFamily, transport_lane_, source_payload_id, {}, rebroadcast, {}, vote_rlp, proposed_block_rlp});
}

void RustVotePacketHandler::process(const threadpool::PacketData& packet_data,
                                    const std::shared_ptr<TaraxaPeer>& peer) {
  const auto outcome = rust_consensus_network_api_->ingestPbftVotePacket(
      consensusPacketRequest(packet_data, peer, transport_lane_, true), kConf, consensusTransportExecutor());
  if (outcome.has_peer_pbft_chain_size && outcome.peer_pbft_chain_size > peer->pbft_chain_size_) {
    peer->pbft_chain_size_ = outcome.peer_pbft_chain_size;
  }
  if (outcome.malicious) {
    throw MaliciousPeerException("Native PBFT vote packet rejected peer payload: " + outcome.error_code);
  }
  if (outcome.accepted_count != 0) {
    routeConsensusEgress(network::ConsensusEgressRequest{kPbftVoteEgressFamily,
                                                         transport_lane_,
                                                         packet_data.id_,
                                                         peer->getId().asArray(),
                                                         false,
                                                         {},
                                                         packet_data.rlp_.data().toBytes(),
                                                         {}});
  }
  if (outcome.status != 0) {
    LOG(log_dg_) << "Native PBFT vote ingress skipped packet: " << outcome.error_code;
  }
}

}  // namespace taraxa::network::tarcap
