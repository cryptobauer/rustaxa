#include "network/tarcap/packets_handlers/latest/get_pillar_votes_bundle_packet_handler.hpp"

#include <sstream>

#include "network/tarcap/packets/latest/pillar_votes_bundle_packet.hpp"
#include "network/tarcap/packets_handlers/latest/pillar_votes_bundle_packet_handler.hpp"

namespace taraxa::network::tarcap {

GetPillarVotesBundlePacketHandler::GetPillarVotesBundlePacketHandler(
    const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager, const addr_t &node_addr,
    const std::string &logs_prefix)
    : IGetPillarVotesBundlePacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                                         logs_prefix + "GET_PILLAR_VOTES_BUNDLE_PH"),
      pillar_chain_manager_(std::move(pillar_chain_manager)) {}

GetPillarVotesBundlePacketHandler::~GetPillarVotesBundlePacketHandler() = default;

void GetPillarVotesBundlePacketHandler::process(const threadpool::PacketData &packet_data,
                                                const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<GetPillarVotesBundlePacket>(packet_data.rlp_);

  LOG(log_dg_) << "GetPillarVotesBundlePacketHandler received from peer " << peer->getId();

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

#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  const auto chunks = pillar_chain_manager_->buildVerifiedPillarVoteNetworkBundles(
      packet.period, packet.pillar_block_hash, PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp);
  if (chunks.empty()) {
    LOG(log_dg_) << "No pillar votes for period " << packet.period << "and pillar block hash "
                 << packet.pillar_block_hash;
    return;
  }

  for (size_t chunk_index = 0; chunk_index < chunks.size(); ++chunk_index) {
    const auto &chunk = chunks[chunk_index];
    dev::RLPStream packet_rlp(1);
    packet_rlp.appendRaw(chunk.optimized_bundle_rlp);

    if (sealAndSend(peer->getId(), SubprotocolPacketType::kPillarVotesBundlePacket, packet_rlp.invalidate())) {
      for (const auto &vote_hash : chunk.vote_hashes) {
        peer->markPillarVoteAsKnown(vote_hash);
      }

      LOG(log_nf_) << "Pillar votes bundle for period " << packet.period << ", hash " << packet.pillar_block_hash
                   << " sent to " << peer->getId() << " (Chunk " << chunk_index + 1 << "/" << chunks.size() << ")";
    }
  }
#else
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
