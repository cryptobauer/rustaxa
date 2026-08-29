#include "network/tarcap/packets_handlers/rust/get_pillar_votes_bundle_packet_handler.hpp"

namespace taraxa::network::tarcap {
namespace {

constexpr uint8_t kMalformedPacket = 11;

}  // namespace

RustGetPillarVotesBundlePacketHandler::RustGetPillarVotesBundlePacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, net::ConsensusQueryClient consensus_query,
    network::ConsensusLiveStatusProvider consensus_status, network::ConsensusNetworkApiShared consensus_network_api,
    TarcapVersion transport_lane, const addr_t& node_addr, const std::string& logs_prefix)
    : RustConsensusTransportPacketHandler(conf, std::move(peers_state), std::move(packets_stats),
                                          std::move(consensus_query), std::move(consensus_status),
                                          std::move(consensus_network_api), node_addr,
                                          logs_prefix + "GET_PILLAR_VOTES_BUNDLE_PH"),
      transport_lane_(transport_lane) {}

RustGetPillarVotesBundlePacketHandler::~RustGetPillarVotesBundlePacketHandler() = default;

void RustGetPillarVotesBundlePacketHandler::process(const threadpool::PacketData& packet_data,
                                                    const std::shared_ptr<TaraxaPeer>& peer) {
  const auto outcome = rust_consensus_network_api_->ingestGetPillarVotesBundleRequest(
      static_cast<uint32_t>(transport_lane_), peer->getId().asArray(), packet_data.id_,
      packet_data.rlp_.data().toBytes(), consensusTransportExecutor());
  if (outcome.status == kMalformedPacket) {
    throw PacketProcessingException(
        "Native get-pillar-votes admission rejected malformed packet: " + outcome.error_code,
        dev::p2p::DisconnectReason::BadProtocol);
  }
  if (outcome.status != 0 && outcome.queued_effect_count == 0) {
    LOG(log_dg_) << "Native pillar-votes request produced no network work: " << outcome.error_code;
  }
}

}  // namespace taraxa::network::tarcap
