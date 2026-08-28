#include "network/tarcap/packets_handlers/rust/votes_bundle_packet_handler.hpp"

#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {
constexpr uint8_t kPbftVotesBundleEgressFamily = 1;
}

RustVotesBundlePacketHandler::RustVotesBundlePacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, network::ConsensusLiveStatusProvider consensus_status,
    net::ConsensusQueryClient consensus_query, network::ConsensusNetworkApiShared consensus_network_api,
    TarcapVersion transport_lane, const addr_t& node_addr, const std::string& logs_prefix)
    : IVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(consensus_status),
                         std::move(consensus_query), consensus_network_api, node_addr, logs_prefix + "VOTES_BUNDLE_PH"),
      transport_lane_(transport_lane) {}

network::ConsensusPacketOutcome RustVotesBundlePacketHandler::gossipCanonicalVotesBundle(
    const std::vector<uint8_t>& votes_bundle_rlp, bool rebroadcast, uint64_t source_payload_id) {
  return routeConsensusEgress(network::ConsensusEgressRequest{
      kPbftVotesBundleEgressFamily, transport_lane_, source_payload_id, {}, rebroadcast, {}, votes_bundle_rlp, {}});
}

void RustVotesBundlePacketHandler::process(const threadpool::PacketData& packet_data,
                                           const std::shared_ptr<TaraxaPeer>& peer) {
  const auto outcome = rust_consensus_network_api_->ingestPbftVotesBundlePacket(
      consensusPacketRequest(packet_data, peer, transport_lane_, false), kConf, consensusTransportExecutor());
  if (outcome.malicious) {
    throw MaliciousPeerException("Native PBFT vote-bundle packet rejected peer payload: " + outcome.error_code);
  }
  if (!outcome.egress_payload_bytes.empty()) {
    routeConsensusEgress(network::ConsensusEgressRequest{kPbftVotesBundleEgressFamily,
                                                         transport_lane_,
                                                         packet_data.id_,
                                                         peer->getId().asArray(),
                                                         false,
                                                         {},
                                                         outcome.egress_payload_bytes,
                                                         {}});
  }
  if (outcome.status != 0) {
    LOG(log_dg_) << "Native PBFT vote-bundle ingress skipped packet: " << outcome.error_code;
  }
}

}  // namespace taraxa::network::tarcap
