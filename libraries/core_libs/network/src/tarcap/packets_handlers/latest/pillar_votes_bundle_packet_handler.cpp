#include "network/tarcap/packets_handlers/latest/pillar_votes_bundle_packet_handler.hpp"

#include "vote/pillar_vote.hpp"

namespace taraxa::network::tarcap {

PillarVotesBundlePacketHandler::PillarVotesBundlePacketHandler(
    const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
#else
    network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
#endif
    const addr_t &node_addr, const std::string &logs_prefix)
    : ExtPillarVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats),
#ifndef RUSTAXA_ENABLE
                                 std::move(pillar_chain_manager),
#else
                                 std::move(consensus_network_api), transport_lane,
#endif
                                 node_addr, logs_prefix + "PILLAR_VOTES_BUNDLE_PH") {
}

void PillarVotesBundlePacketHandler::process(const threadpool::PacketData &packet_data,
                                             const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<PillarVotesBundlePacket>(packet_data.rlp_);

  if (packet.pillar_votes_bundle.pillar_votes.size() == 0 ||
      packet.pillar_votes_bundle.pillar_votes.size() > kMaxPillarVotesInBundleRlp) {
    throw InvalidRlpItemsCountException("PillarVotesBundlePacket", packet.pillar_votes_bundle.pillar_votes.size(),
                                        kMaxPillarVotesInBundleRlp);
  }

  // TODO[2744]: there could be the same protection as in pbft syncing that only requested bundle packet is accepted
  LOG(log_dg_) << "PillarVotesBundlePacket received from peer " << peer->getId();

#ifdef RUSTAXA_ENABLE
  processPillarVotes(packet.pillar_votes_bundle.pillar_votes, peer, kPacketType_);
#else
  for (const auto &pillar_vote : packet.pillar_votes_bundle.pillar_votes) {
    if (!kConf.genesis.state.hardforks.ficus_hf.isFicusHardfork(pillar_vote->getPeriod())) {
      std::ostringstream err_msg;
      err_msg << "Synced pillar vote " << pillar_vote->getHash() << ", period " << pillar_vote->getPeriod()
              << " < ficus hardfork block num";
      throw MaliciousPeerException(err_msg.str());
    }

    processPillarVote(pillar_vote, peer, kPacketType_);
  }
#endif
}

}  // namespace taraxa::network::tarcap
