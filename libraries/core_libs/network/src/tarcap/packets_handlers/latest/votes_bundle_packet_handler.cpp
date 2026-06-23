#include "network/tarcap/packets_handlers/latest/votes_bundle_packet_handler.hpp"

#include "pbft/pbft_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif
#include "vote/votes_bundle_rlp.hpp"
#include "vote_manager/vote_manager.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kPbftVoteIngressStatusUnsupportedBundleProposeVote = 7;
constexpr uint8_t kPbftVoteIngressStatusBundleVoteMismatch = 8;

rustaxa::PbftVoteIngressFact makeVoteIngressFact(const std::shared_ptr<PbftVote> &vote) {
  rustaxa::PbftVoteIngressFact fact{};
  fact.period = vote->getPeriod();
  fact.round = vote->getRound();
  fact.step = vote->getStep();
  fact.vote_type = static_cast<uint8_t>(vote->getType());
  return fact;
}

}  // namespace
#endif

VotesBundlePacketHandler::VotesBundlePacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                                   std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                   std::shared_ptr<PbftManager> pbft_mgr,
                                                   std::shared_ptr<PbftChain> pbft_chain,
                                                   std::shared_ptr<VoteManager> vote_mgr,
                                                   std::shared_ptr<SlashingManager> slashing_manager,
                                                   const addr_t &node_addr, const std::string &logs_prefix)
    : IVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_mgr),
                         std::move(pbft_chain), std::move(vote_mgr), std::move(slashing_manager), node_addr,
                         logs_prefix + "VOTES_BUNDLE_PH") {}

void VotesBundlePacketHandler::process(const threadpool::PacketData &packet_data,
                                       const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<VotesBundlePacket>(packet_data.rlp_);

  if (packet.votes_bundle.votes.size() == 0 || packet.votes_bundle.votes.size() > kMaxVotesInBundleRlp) {
    throw InvalidRlpItemsCountException("VotesBundlePacket", packet.votes_bundle.votes.size(), kMaxVotesInBundleRlp);
  }

  const auto [current_pbft_round, current_pbft_period] = pbft_mgr_->getPbftRoundAndPeriod();

  const auto &reference_vote = packet.votes_bundle.votes.front();
  const auto votes_bundle_votes_type = reference_vote->getType();

#ifdef RUSTAXA_ENABLE
  rustaxa::PbftVoteIngressContext ingress_context{};
  ingress_context.current_period = current_pbft_period;
  ingress_context.current_round = current_pbft_round;
  ingress_context.current_step = pbft_mgr_->getPbftStep();
  ingress_context.max_future_period_delta = this->kConf.network.ddos_protection.vote_accepting_periods;
  ingress_context.max_future_round_delta = this->kConf.network.ddos_protection.vote_accepting_rounds;
  ingress_context.max_future_step_delta = this->kConf.network.ddos_protection.vote_accepting_steps;
  ingress_context.validate_max_round_step =
      !(votes_bundle_votes_type == PbftVoteTypes::cert_vote || votes_bundle_votes_type == PbftVoteTypes::next_vote);
  ingress_context.source_peer_is_voter = reference_vote->getVoter() == peer->getId();
  ingress_context.can_request_pbft_sync =
      std::chrono::system_clock::now() - last_pbft_block_sync_request_time_ > kSyncRequestInterval;
  ingress_context.can_request_next_votes_sync =
      std::chrono::system_clock::now() - last_votes_sync_request_time_ > kSyncRequestInterval;

  const auto reference_fact = makeVoteIngressFact(reference_vote);
  for (const auto &vote : packet.votes_bundle.votes) {
    const auto ingress_plan = planPbftVoteBundleIngress(reference_fact, makeVoteIngressFact(vote), ingress_context);
    if (ingress_plan.accepted) {
      continue;
    }

    if (ingress_plan.status == kPbftVoteIngressStatusUnsupportedBundleProposeVote) {
      LOG(log_er_) << "Dropping votes bundle packet due to received \"propose\" votes from " << peer->getId()
                   << ". The peer may be a malicious player, will be disconnected";
      disconnect(peer->getId(), dev::p2p::UserReason);
      return;
    }
    if (ingress_plan.status == kPbftVoteIngressStatusBundleVoteMismatch) {
      throw MaliciousPeerException("Received PBFT votes bundle with mixed vote identity");
    }

    LOG(log_wr_) << "Drop votes sync bundle as Rust ingress plan rejected it. Votes (period, round, step) = ("
                 << reference_vote->getPeriod() << ", " << reference_vote->getRound() << ", "
                 << reference_vote->getStep() << "). Current PBFT (period, round, step) = (" << current_pbft_period
                 << ", " << current_pbft_round << ", " << pbft_mgr_->getPbftStep()
                 << "), status: " << static_cast<uint32_t>(ingress_plan.status)
                 << ", error: " << static_cast<std::string>(ingress_plan.error_code);
    return;
  }
#else
  // Votes sync bundles are allowed to contain only votes bundles of the same type, period, round and step so if first
  // vote is irrelevant, all of them are
  if (!isPbftRelevantVote(packet.votes_bundle.votes[0])) {
    LOG(log_wr_) << "Drop votes sync bundle as it is irrelevant for current pbft state. Votes (period, round, step) = ("
                 << reference_vote->getPeriod() << ", " << reference_vote->getRound() << ", "
                 << reference_vote->getStep() << "). Current PBFT (period, round, step) = (" << current_pbft_period
                 << ", " << current_pbft_round << ", " << pbft_mgr_->getPbftStep() << ")";
    return;
  }

  // VotesBundlePacket does not support propose votes
  if (reference_vote->getType() == PbftVoteTypes::propose_vote) {
    LOG(log_er_) << "Dropping votes bundle packet due to received \"propose\" votes from " << peer->getId()
                 << ". The peer may be a malicious player, will be disconnected";
    disconnect(peer->getId(), dev::p2p::UserReason);
    return;
  }
#endif

  // Process processStandardVote is called with false in case of next votes bundle -> does not check max boundaries
  // for round and step to actually being able to sync the current round in case network is stalled
  bool check_max_round_step = true;
  if (votes_bundle_votes_type == PbftVoteTypes::cert_vote || votes_bundle_votes_type == PbftVoteTypes::next_vote) {
    check_max_round_step = false;
  }

  size_t processed_votes_count = 0;
#ifdef RUSTAXA_ENABLE
  std::vector<std::shared_ptr<PbftVote>> processed_votes;
  processed_votes.reserve(packet.votes_bundle.votes.size());
#endif
  for (const auto &vote : packet.votes_bundle.votes) {
#ifndef RUSTAXA_ENABLE
    peer->markPbftVoteAsKnown(vote->getHash());
#endif

    // Do not process vote that has already been validated
    if (vote_mgr_->voteAlreadyValidated(vote->getHash())) {
      LOG(log_dg_) << "Received vote " << vote->getHash() << " has already been validated";
      continue;
    }

    LOG(log_dg_) << "Received vote " << vote->getHash().abridged() << ", period " << vote->getPeriod() << ", round "
                 << vote->getRound() << ", step " << vote->getStep() << ", voter " << vote->getVoterAddr()
                 << " as part of votes bundle";

    const auto process_result = processVote(vote, nullptr, peer, check_max_round_step);
    if (process_result.report_slashing) {
      throw MaliciousPeerException("Received double vote", vote->getVoter());
    }
    if (!process_result.accepted) {
      continue;
    }

#ifdef RUSTAXA_ENABLE
    if (process_result.mark_vote_known) {
      peer->markPbftVoteAsKnown(vote->getHash());
    }

    processed_votes.emplace_back(vote);
#endif
    processed_votes_count++;
  }

  LOG(log_nf_) << "Received " << packet.votes_bundle.votes.size() << " (processed " << processed_votes_count
               << " ) sync votes from peer " << peer->getId() << ". Votes period " << reference_vote->getPeriod()
               << ", round " << reference_vote->getRound() << ", step " << reference_vote->getStep();

#ifdef RUSTAXA_ENABLE
  onNewPbftVotesBundle(processed_votes, false, peer->getId());
#else
  onNewPbftVotesBundle(packet.votes_bundle.votes, false, peer->getId());
#endif
}

}  // namespace taraxa::network::tarcap
