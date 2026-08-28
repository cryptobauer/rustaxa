#include "network/tarcap/packets_handlers/rust/pillar_votes_bundle_packet_handler.hpp"

#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {

RustPillarVotesBundlePacketHandler::RustPillarVotesBundlePacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, network::ConsensusNetworkApiShared consensus_network_api,
    TarcapVersion transport_lane, const addr_t& node_addr, const std::string& logs_prefix)
    : RustConsensusTransportPacketHandler(conf, std::move(peers_state), std::move(packets_stats), {}, {},
                                          consensus_network_api, node_addr, logs_prefix + "PILLAR_VOTES_BUNDLE_PH"),
      transport_lane_(transport_lane) {}

void RustPillarVotesBundlePacketHandler::process(const threadpool::PacketData& packet_data,
                                                 const std::shared_ptr<TaraxaPeer>& peer) {
  const auto outcome = rust_consensus_network_api_->ingestPillarVotesBundlePacket(
      consensusPacketRequest(packet_data, peer, transport_lane_, false), consensusTransportExecutor());
  if (outcome.malicious) {
    throw MaliciousPeerException("Native pillar-vote bundle packet rejected peer payload: " + outcome.error_code);
  }
  if (outcome.status != 0) {
    LOG(log_dg_) << "Native pillar-vote bundle ingress skipped packet: " << outcome.error_code;
  }
}

}  // namespace taraxa::network::tarcap
