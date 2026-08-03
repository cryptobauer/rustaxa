#include "network/tarcap/packets_handlers/rust/get_next_votes_bundle_packet_handler.hpp"

#include <stdexcept>

#include "network/tarcap/packets/latest/votes_bundle_packet.hpp"
#include "vote/votes_bundle_rlp.hpp"

namespace taraxa::network::tarcap {

RustGetNextVotesBundlePacketHandler::RustGetNextVotesBundlePacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, network::ConsensusNetworkApiShared consensus_network_api,
    TarcapVersion transport_lane, const addr_t& node_addr, const std::string& logs_prefix)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                    logs_prefix + "GET_NEXT_VOTES_BUNDLE_PH"),
      consensus_network_api_(std::move(consensus_network_api)),
      transport_lane_(transport_lane) {
  if (!consensus_network_api_) {
    throw std::invalid_argument("Rust next-votes handler requires consensus network API");
  }
}

RustGetNextVotesBundlePacketHandler::~RustGetNextVotesBundlePacketHandler() = default;

void RustGetNextVotesBundlePacketHandler::process(const threadpool::PacketData& packet_data,
                                                  const std::shared_ptr<TaraxaPeer>& peer) {
  const auto packet = decodePacketRlp<GetNextVotesBundlePacket>(packet_data.rlp_);
  const auto peer_id = peer->getId();
  const auto outcome = consensus_network_api_->servePbftNextVotesBundleRequest(
      static_cast<uint32_t>(transport_lane_), peer_id.asArray(), packet.peer_pbft_period, packet.peer_pbft_round,
      packet_data.id_,
      network::PbftNextVotesBundleExecutor{
          .send_bundle =
              [this, peer_id](const std::vector<uint8_t>& payload) {
                const dev::bytes optimized_bundle_rlp(payload.begin(), payload.end());
                auto votes = decodePbftVotesBundleRlp(dev::RLP(optimized_bundle_rlp));
                auto target = peers_state_->getPeer(peer_id);
                if (!target) {
                  return false;
                }
                auto packet = VotesBundlePacket{OptimizedPbftVotesBundle{.votes = std::move(votes)}};
                if (!sealAndSend(peer_id, SubprotocolPacketType::kVotesBundlePacket, encodePacketRlp(packet))) {
                  return false;
                }
                for (const auto& vote : packet.votes_bundle.votes) {
                  target->markPbftVoteAsKnown(vote->getHash());
                }
                return true;
              },
      });

  if (outcome.status != 0 && outcome.queued_effect_count == 0) {
    LOG(log_dg_) << "Native next-votes request produced no network work: " << outcome.error_code;
  }
}

}  // namespace taraxa::network::tarcap
