#include "network/tarcap/packets_handlers/latest/vote_packet_handler.hpp"

#include "network/tarcap/packets/latest/vote_packet.hpp"
#ifndef RUSTAXA_ENABLE
#include "pbft/pbft_manager.hpp"
#include "vote_manager/vote_manager.hpp"
#endif

namespace taraxa::network::tarcap {

VotePacketHandler::VotePacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                     std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
                                     std::shared_ptr<PbftManager> pbft_mgr, net::ConsensusQueryClient pbft_chain,
                                     std::shared_ptr<VoteManager> vote_mgr,
                                     std::shared_ptr<SlashingManager> slashing_manager,
#else
                                     network::ConsensusLiveStatusProvider consensus_status,
                                     net::ConsensusQueryClient pbft_chain,
                                     network::ConsensusNetworkApiShared consensus_network_api,
                                     TarcapVersion transport_lane,
#endif
                                     const addr_t &node_addr, const std::string &logs_prefix)
    : IVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats),
#ifndef RUSTAXA_ENABLE
                         std::move(pbft_mgr), std::move(pbft_chain), std::move(vote_mgr), std::move(slashing_manager),
#else
                         std::move(consensus_status), std::move(pbft_chain), std::move(consensus_network_api),
                         transport_lane,
#endif
                         node_addr, logs_prefix + "PBFT_VOTE_PH") {
}

void VotePacketHandler::process(const threadpool::PacketData &packet_data, const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<VotePacket>(packet_data.rlp_);

  if (packet.optional_data.has_value()) {
    LOG(log_dg_) << "Received vote " << packet.vote->getHash().abridged() << ", period " << packet.vote->getPeriod()
                 << ", round " << packet.vote->getRound() << ", step " << packet.vote->getStep() << ", voter "
                 << packet.vote->getVoterAddr() << " with PBFT block "
                 << packet.optional_data->pbft_block->getBlockHash();

    // Update peer's max chain size
    if (packet.optional_data->peer_chain_size > peer->pbft_chain_size_) {
      peer->pbft_chain_size_ = packet.optional_data->peer_chain_size;
    }
  } else {
    LOG(log_dg_) << "Received vote " << packet.vote->getHash().abridged() << ", period " << packet.vote->getPeriod()
                 << ", round " << packet.vote->getRound() << ", step " << packet.vote->getStep() << ", voter "
                 << packet.vote->getVoterAddr();
  }

#ifndef RUSTAXA_ENABLE
  const auto vote_hash = packet.vote->getHash();
  const auto [current_pbft_round, current_pbft_period] = pbft_mgr_->getPbftRoundAndPeriod();
  if (!isPbftRelevantVote(packet.vote)) {
    LOG(log_dg_) << "Drop irrelevant vote " << vote_hash << " for current pbft state. Vote (period, round, step) = ("
                 << packet.vote->getPeriod() << ", " << packet.vote->getRound() << ", " << packet.vote->getStep()
                 << "). Current PBFT (period, round, step) = (" << current_pbft_period << ", " << current_pbft_round
                 << ", " << pbft_mgr_->getPbftStep() << ")";
    return;
  }

  // Do not process vote that has already been validated
  if (vote_mgr_->voteAlreadyValidated(vote_hash)) {
    LOG(log_dg_) << "Received vote " << vote_hash << " has already been validated";
    return;
  }
#endif

  std::shared_ptr<PbftBlock> pbft_block;
  if (packet.optional_data.has_value()) {
    if (packet.optional_data->pbft_block->getBlockHash() != packet.vote->getBlockHash()) {
      std::ostringstream err_msg;
      err_msg << "Vote " << packet.vote->getHash().abridged() << " voted block "
              << packet.vote->getBlockHash().abridged() << " != actual block "
              << packet.optional_data->pbft_block->getBlockHash().abridged();
      throw MaliciousPeerException(err_msg.str());
    }

#ifndef RUSTAXA_ENABLE
    peer->markPbftBlockAsKnown(packet.optional_data->pbft_block->getBlockHash());
#endif
    pbft_block = packet.optional_data->pbft_block;
  }

  const auto process_result = processVote(packet.vote, pbft_block, peer, true, true);
  if (process_result.report_slashing) {
    throw MaliciousPeerException("Received double vote", packet.vote->getVoter());
  }
  if (!process_result.accepted) {
    return;
  }

  // Do not mark it before, as peers have small caches of known votes. Only mark gossiping votes
#ifndef RUSTAXA_ENABLE
  if (process_result.mark_vote_known) {
    peer->markPbftVoteAsKnown(vote_hash);
  }
#endif

#ifndef RUSTAXA_ENABLE
  if (process_result.gossip_vote) {
    pbft_mgr_->gossipVote(packet.vote, pbft_block);
  }
#endif
}

}  // namespace taraxa::network::tarcap
