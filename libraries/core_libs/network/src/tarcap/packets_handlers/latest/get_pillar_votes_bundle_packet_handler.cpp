#include "network/tarcap/packets_handlers/latest/get_pillar_votes_bundle_packet_handler.hpp"

#include <sstream>
#include <stdexcept>

#include "network/tarcap/packets/latest/pillar_votes_bundle_packet.hpp"
#include "network/tarcap/packets_handlers/latest/pillar_votes_bundle_packet_handler.hpp"

namespace taraxa::network::tarcap {

GetPillarVotesBundlePacketHandler::GetPillarVotesBundlePacketHandler(
    const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
#else
    network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
#endif
    const addr_t &node_addr, const std::string &logs_prefix)
    : IGetPillarVotesBundlePacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                                         logs_prefix + "GET_PILLAR_VOTES_BUNDLE_PH")
#ifndef RUSTAXA_ENABLE
      ,
      pillar_chain_manager_(std::move(pillar_chain_manager))
#else
      ,
      rust_consensus_network_api_(std::move(consensus_network_api)),
      transport_lane_(transport_lane)
#endif
{
#ifdef RUSTAXA_ENABLE
  if (!rust_consensus_network_api_) {
    throw std::invalid_argument("Rust consensus network API must be provided");
  }
#endif
}

GetPillarVotesBundlePacketHandler::~GetPillarVotesBundlePacketHandler() = default;

void GetPillarVotesBundlePacketHandler::process(const threadpool::PacketData &packet_data,
                                                const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<GetPillarVotesBundlePacket>(packet_data.rlp_);

  LOG(log_dg_) << "GetPillarVotesBundlePacketHandler received from peer " << peer->getId();

#ifdef RUSTAXA_ENABLE
  const auto outcome = rust_consensus_network_api_->servePillarVotesBundleRequest(
      static_cast<uint32_t>(transport_lane_), peer->getId().asArray(), packet.period,
      packet.pillar_block_hash.asArray(), packet_data.id_,
      network::PillarVotesBundleExecutor{
          .send_bundle =
              [this, &peer](const std::vector<uint8_t> &payload) {
                dev::RLPStream packet_rlp(1);
                packet_rlp.appendRaw(dev::bytes(payload.begin(), payload.end()));
                return sealAndSend(peer->getId(), SubprotocolPacketType::kPillarVotesBundlePacket,
                                   packet_rlp.invalidate());
              },
          .mark_vote_known =
              [&peer](const std::array<uint8_t, 32> &hash) {
                peer->markPillarVoteAsKnown(vote_hash_t(hash.data(), vote_hash_t::ConstructFromPointer));
              },
          .report_peer =
              [this, &peer](uint8_t reason) {
                peers_state_->set_peer_malicious(peer->getId());
                LOG(log_wr_) << "Network API reported malicious pillar-vote requester " << peer->getId()
                             << " with reason: " << static_cast<uint32_t>(reason);
              },
          .disconnect_peer = [this, &peer] { disconnect(peer->getId(), dev::p2p::UserReason); },
      });

  if (outcome.status != 0 && outcome.queued_effect_count == 0) {
    LOG(log_dg_) << "Native pillar-vote bundle request produced no network work: " << outcome.error_code;
  }
#else
  if (!kConf.genesis.state.hardforks.ficus_hf.isFicusHardfork(packet.period)) {
    std::ostringstream err_msg;
    err_msg << "Pillar votes bundle request for period " << packet.period << ", ficus hardfork block num "
            << kConf.genesis.state.hardforks.ficus_hf.block_num;
    throw MaliciousPeerException(err_msg.str());
  }

  if (!kConf.genesis.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(packet.period)) {
    std::ostringstream err_msg;
    err_msg << "Pillar votes bundle request for period " << packet.period << ". Wrong requested period";
    throw MaliciousPeerException(err_msg.str());
  }

  const auto votes = pillar_chain_manager_->getVerifiedPillarVotes(packet.period, packet.pillar_block_hash);
  if (votes.empty()) {
    LOG(log_dg_) << "No pillar votes for period " << packet.period << "and pillar block hash "
                 << packet.pillar_block_hash;
    return;
  }

  const size_t total_votes = votes.size();
  size_t votes_sent = 0;
  while (votes_sent < total_votes) {
    const size_t chunk_size =
        std::min(PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp, total_votes - votes_sent);

    std::vector<std::shared_ptr<PillarVote> > pillar_votes;
    pillar_votes.reserve(chunk_size);
    for (size_t i = 0; i < chunk_size; ++i) {
      pillar_votes.emplace_back(votes[votes_sent + i]);
    }
    PillarVotesBundlePacket pillar_votes_bundle_packet(OptimizedPillarVotesBundle{std::move(pillar_votes)});

    if (sealAndSend(peer->getId(), SubprotocolPacketType::kPillarVotesBundlePacket,
                    encodePacketRlp(pillar_votes_bundle_packet))) {
      for (size_t i = 0; i < chunk_size; ++i) {
        peer->markPillarVoteAsKnown(votes[votes_sent + i]->getHash());
      }

      LOG(log_nf_) << "Pillar votes bundle for period " << packet.period << ", hash " << packet.pillar_block_hash
                   << " sent to " << peer->getId() << " (Chunk "
                   << (votes_sent / PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp) + 1 << "/"
                   << (total_votes + PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp - 1) /
                          PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp
                   << ")";
    }

    votes_sent += chunk_size;
  }
#endif
}

void GetPillarVotesBundlePacketHandler::requestPillarVotesBundle(PbftPeriod period, const blk_hash_t &pillar_block_hash,
                                                                 const std::shared_ptr<TaraxaPeer> &peer) {
  if (sealAndSend(peer->getId(), SubprotocolPacketType::kGetPillarVotesBundlePacket,
                  encodePacketRlp(GetPillarVotesBundlePacket(period, pillar_block_hash)))) {
    LOG(log_nf_) << "Requested pillar votes bundle for period " << period << " and pillar block " << pillar_block_hash
                 << " from peer " << peer->getId();
  } else {
    LOG(log_er_) << "Unable to send pillar votes bundle request for period " << period << " and pillar block "
                 << pillar_block_hash << " to peer " << peer->getId();
  }
}

}  // namespace taraxa::network::tarcap
