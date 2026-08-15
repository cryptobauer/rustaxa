#include "network/tarcap/packets_handlers/latest/votes_bundle_packet_handler.hpp"

#include <algorithm>

#include "pbft/pbft_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif
#include "vote/votes_bundle_rlp.hpp"
#ifndef RUSTAXA_ENABLE
#include "vote_manager/vote_manager.hpp"
#endif

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kPbftVoteIngressStatusUnsupportedBundleProposeVote = 7;
constexpr uint8_t kPbftVoteIngressStatusBundleVoteMismatch = 8;
constexpr uint8_t kPbftVoteIngressStatusAccepted = 0;

rustaxa::PbftVoteIngressFact makeVoteIngressFact(const std::shared_ptr<PbftVote> &vote) {
  rustaxa::PbftVoteIngressFact fact{};
  fact.period = vote->getPeriod();
  fact.round = vote->getRound();
  fact.step = vote->getStep();
  fact.vote_type = static_cast<uint8_t>(vote->getType());
  return fact;
}

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes &input) {
  rust::Vec<uint8_t> out;
  out.reserve(input.size());
  for (const auto byte : input) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

}  // namespace
#endif

VotesBundlePacketHandler::VotesBundlePacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                                   std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                   std::shared_ptr<PbftManager> pbft_mgr,
                                                   net::ConsensusQueryClient pbft_chain,
#ifndef RUSTAXA_ENABLE
                                                   std::shared_ptr<VoteManager> vote_mgr,
                                                   std::shared_ptr<SlashingManager> slashing_manager,
#else
                                                   std::shared_ptr<TransactionManager> trx_mgr,
                                                   network::ConsensusNetworkApiShared consensus_network_api,
                                                   TarcapVersion transport_lane,
#endif
                                                   const addr_t &node_addr, const std::string &logs_prefix)
    : IVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_mgr),
                         std::move(pbft_chain),
#ifndef RUSTAXA_ENABLE
                         std::move(vote_mgr), std::move(slashing_manager),
#else
                         std::move(trx_mgr), std::move(consensus_network_api), transport_lane,
#endif
                         node_addr, logs_prefix + "VOTES_BUNDLE_PH") {
}

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

  // Validate the complete bundle shape before any member reaches the
  // application leaf. The operation-specific Rust call queues admissions only
  // after this preflight succeeds for the whole bundle.

  auto lane_execution_lock = rust_consensus_network_api_->lockTransportLane(transport_lane_);
  rust::Vec<rustaxa::PbftVoteIngressFact> bundle_facts;
  rust::Vec<rustaxa::NetworkPbftVoteIngressContext> bundle_contexts;
  bundle_facts.reserve(packet.votes_bundle.votes.size());
  bundle_contexts.reserve(packet.votes_bundle.votes.size());
  for (const auto &vote : packet.votes_bundle.votes) {
    bundle_facts.push_back(makeVoteIngressFact(vote));
    rustaxa::NetworkPbftVoteIngressContext member_context{};
    member_context.ingress = ingress_context;
    member_context.transport_lane = transport_lane_;
    member_context.peer_id = peer->getId().asArray();
    member_context.peer_pbft_chain_size = peer->pbft_chain_size_.load();
    member_context.source_payload_id = packet_data.id_;
    member_context.enqueue_admission = true;
    member_context.allow_gossip = false;
    member_context.vote_hash = vote->getHash().asArray();
    member_context.vote_rlp = toBridgeBytes(vote->rlp(true, false));
    bundle_contexts.push_back(std::move(member_context));
  }
  const auto bundle_decisions =
      ingestPbftVoteBundle(makeVoteIngressFact(reference_vote), std::move(bundle_facts), std::move(bundle_contexts));
  const bool bundle_preflight_accepted =
      bundle_decisions.size() == packet.votes_bundle.votes.size() &&
      std::all_of(bundle_decisions.begin(), bundle_decisions.end(),
                  [](const auto &outcome) { return outcome.decision.status == kPbftVoteIngressStatusAccepted; });
  if (!bundle_preflight_accepted) {
    if (bundle_decisions.empty()) {
      throw std::runtime_error("Rust network API rejected malformed PBFT bundle admission inputs");
    }
    const auto &ingress_decision = bundle_decisions.front().decision;
    executeConsensusNetworkEffects(16, packet_data.id_);
    if (ingress_decision.status == kPbftVoteIngressStatusUnsupportedBundleProposeVote) {
      LOG(log_er_) << "Dropping votes bundle packet due to received \"propose\" votes from " << peer->getId()
                   << ". The peer may be a malicious player, will be disconnected";
      return;
    }
    if (ingress_decision.status == kPbftVoteIngressStatusBundleVoteMismatch) {
      throw MaliciousPeerException("Received PBFT votes bundle with mixed vote identity");
    }

    LOG(log_wr_) << "Drop votes sync bundle as Rust ingress plan rejected it. Votes (period, round, step) = ("
                 << reference_vote->getPeriod() << ", " << reference_vote->getRound() << ", "
                 << reference_vote->getStep() << "). Current PBFT (period, round, step) = (" << current_pbft_period
                 << ", " << current_pbft_round << ", " << pbft_mgr_->getPbftStep()
                 << "), status: " << static_cast<uint32_t>(ingress_decision.status)
                 << ", error: " << static_cast<std::string>(ingress_decision.error_code);
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
#ifndef RUSTAXA_ENABLE
  bool check_max_round_step = true;
  if (votes_bundle_votes_type == PbftVoteTypes::cert_vote || votes_bundle_votes_type == PbftVoteTypes::next_vote) {
    check_max_round_step = false;
  }
#endif

  size_t processed_votes_count = 0;
#ifdef RUSTAXA_ENABLE
  size_t bundle_member_index = 0;
#endif
  for (const auto &vote : packet.votes_bundle.votes) {
#ifndef RUSTAXA_ENABLE
    peer->markPbftVoteAsKnown(vote->getHash());
#endif

#ifndef RUSTAXA_ENABLE
    // Do not process vote that has already been validated. Rust-enabled builds
    // route duplicate handling through the common network API admission path.
    if (vote_mgr_->voteAlreadyValidated(vote->getHash())) {
      LOG(log_dg_) << "Received vote " << vote->getHash() << " has already been validated";
      continue;
    }
#endif

    LOG(log_dg_) << "Received vote " << vote->getHash().abridged() << ", period " << vote->getPeriod() << ", round "
                 << vote->getRound() << ", step " << vote->getStep() << ", voter " << vote->getVoterAddr()
                 << " as part of votes bundle";

#ifdef RUSTAXA_ENABLE
    const auto process_result = consumePbftVoteAdmission(bundle_decisions[bundle_member_index]);
    ++bundle_member_index;
    if (process_result.cancelled) {
      LOG(log_dg_) << "Rust network API cancelled the remaining PBFT vote-bundle admissions";
      break;
    }
#else
    const auto process_result = processVote(vote, nullptr, peer, check_max_round_step, false);
#endif
    if (process_result.report_slashing) {
      throw MaliciousPeerException("Received double vote", vote->getVoter());
    }
    if (!process_result.accepted) {
      continue;
    }

    processed_votes_count++;
  }

  LOG(log_nf_) << "Received " << packet.votes_bundle.votes.size() << " (processed " << processed_votes_count
               << " ) sync votes from peer " << peer->getId() << ". Votes period " << reference_vote->getPeriod()
               << ", round " << reference_vote->getRound() << ", step " << reference_vote->getStep();

#ifndef RUSTAXA_ENABLE
  onNewPbftVotesBundle(packet.votes_bundle.votes, false, peer->getId());
#else
  executeConsensusNetworkEffects(16, packet_data.id_);
#endif
}

}  // namespace taraxa::network::tarcap
