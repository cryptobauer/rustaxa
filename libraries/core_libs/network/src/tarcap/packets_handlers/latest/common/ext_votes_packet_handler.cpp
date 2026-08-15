#include "network/tarcap/packets_handlers/latest/common/ext_votes_packet_handler.hpp"

#include <algorithm>

#include "network/tarcap/packets/latest/get_next_votes_bundle_packet.hpp"
#include "network/tarcap/packets/latest/vote_packet.hpp"
#include "network/tarcap/packets_handlers/latest/common/exceptions.hpp"
#include "pbft/pbft_manager.hpp"
#include "vote/pbft_vote.hpp"
#include "vote/votes_bundle_rlp.hpp"
#ifndef RUSTAXA_ENABLE
#include "vote_manager/vote_manager.hpp"
#endif
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindSendPacket = 0;
constexpr uint8_t kNetworkEffectKindGossipPacket = 1;
constexpr uint8_t kNetworkEffectKindMarkPeerKnown = 2;
constexpr uint8_t kNetworkEffectKindRequestSync = 3;
constexpr uint8_t kNetworkEffectKindReportPeer = 4;
constexpr uint8_t kNetworkEffectKindDisconnectPeer = 5;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkSyncKindPbftChain = 0;
constexpr uint8_t kNetworkSyncKindPbftNextVotes = 1;
constexpr uint8_t kNetworkObjectKindPbftVote = 0;
constexpr uint8_t kNetworkObjectKindPbftBlock = 1;
constexpr uint32_t kNetworkPacketKindPbftVote = 1;
constexpr uint32_t kNetworkPacketKindPbftVotesBundle = 3;

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes &input) {
  rust::Vec<uint8_t> out;
  out.reserve(input.size());
  for (const auto byte : input) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

rust::Vec<rustaxa::SlashingSubmitterIdentity> makeSlashingSubmitters(const FullNodeConfig &config) {
  rust::Vec<rustaxa::SlashingSubmitterIdentity> submitters;
  submitters.reserve(config.wallets.size());
  for (size_t index = 0; index < config.wallets.size(); ++index) {
    rustaxa::SlashingSubmitterIdentity submitter{};
    submitter.wallet_index = index;
    submitter.address = config.wallets[index].node_addr.asArray();
    submitters.push_back(std::move(submitter));
  }
  return submitters;
}

}  // namespace
#endif

ExtVotesPacketHandler::ExtVotesPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
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
                                             const addr_t &node_addr, const std::string &log_channel_name)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, log_channel_name),
      last_votes_sync_request_time_(std::chrono::system_clock::now()),
      last_pbft_block_sync_request_time_(std::chrono::system_clock::now()),
      pbft_mgr_(std::move(pbft_mgr)),
      pbft_chain_(std::move(pbft_chain))
#ifndef RUSTAXA_ENABLE
      ,
      vote_mgr_(std::move(vote_mgr)),
      slashing_manager_(std::move(slashing_manager))
#else
      ,
      rust_consensus_network_api_(std::move(consensus_network_api)),
      trx_mgr_(std::move(trx_mgr)),
      transport_lane_(transport_lane)
#endif
{
}

ExtVotesPacketHandler::~ExtVotesPacketHandler() = default;

ExtVotesPacketHandler::VoteProcessingResult ExtVotesPacketHandler::processVote(
    const std::shared_ptr<PbftVote> &vote, const std::shared_ptr<PbftBlock> &pbft_block,
    const std::shared_ptr<TaraxaPeer> &peer, bool validate_max_round_step, [[maybe_unused]] bool allow_gossip) {
  if (pbft_block && !validateVoteAndBlock(vote, pbft_block)) {
    throw MaliciousPeerException("Received vote's voted value != received pbft block");
  }

#ifdef RUSTAXA_ENABLE
  // Rust owns ingress, application-leaf ordering, and every dependent routing
  // decision. Tarcap supplies the decoded canonical payload and executes the
  // typed VoteManager/transport effects returned by the shared network root.
  const auto [current_pbft_round, current_pbft_period] = pbft_mgr_->getPbftRoundAndPeriod();
  rustaxa::PbftVoteIngressFact ingress_fact{};
  ingress_fact.period = vote->getPeriod();
  ingress_fact.round = vote->getRound();
  ingress_fact.step = vote->getStep();
  ingress_fact.vote_type = static_cast<uint8_t>(vote->getType());

  rustaxa::PbftVoteIngressContext ingress_context{};
  ingress_context.current_period = current_pbft_period;
  ingress_context.current_round = current_pbft_round;
  ingress_context.current_step = pbft_mgr_->getPbftStep();
  ingress_context.max_future_period_delta = this->kConf.network.ddos_protection.vote_accepting_periods;
  ingress_context.max_future_round_delta = this->kConf.network.ddos_protection.vote_accepting_rounds;
  ingress_context.max_future_step_delta = this->kConf.network.ddos_protection.vote_accepting_steps;
  ingress_context.validate_max_round_step = validate_max_round_step;
  ingress_context.source_peer_is_voter = vote->getVoter() == peer->getId();
  ingress_context.can_request_pbft_sync =
      std::chrono::system_clock::now() - last_pbft_block_sync_request_time_ > kSyncRequestInterval;
  ingress_context.can_request_next_votes_sync =
      std::chrono::system_clock::now() - last_votes_sync_request_time_ > kSyncRequestInterval;

  rustaxa::NetworkPbftVoteIngressContext network_ingress_context{};
  network_ingress_context.ingress = ingress_context;
  network_ingress_context.transport_lane = transport_lane_;
  network_ingress_context.peer_id = peer->getId().asArray();
  network_ingress_context.peer_pbft_chain_size = peer->pbft_chain_size_.load();
  network_ingress_context.source_payload_id = 0;
  network_ingress_context.enqueue_admission = true;
  network_ingress_context.allow_gossip = allow_gossip;
  network_ingress_context.vote_hash = vote->getHash().asArray();
  network_ingress_context.vote_rlp = toBridgeBytes(vote->rlp(true, false));
  if (pbft_block) {
    network_ingress_context.pbft_block_rlp = toBridgeBytes(pbft_block->rlp(true));
    network_ingress_context.pbft_block_hash = pbft_block->getBlockHash().asArray();
    network_ingress_context.pbft_block_period = pbft_block->getPeriod();
  }
  auto lane_execution_lock = rust_consensus_network_api_->lockTransportLane(transport_lane_);
  const auto ingress = ingestPbftVote(ingress_fact, network_ingress_context);
  const auto &ingress_decision = ingress.decision;
  if (ingress_decision.status != 0) {
    executeConsensusNetworkEffects(16);
  }
  if (!ingress_decision.routed || ingress_decision.status != 0) {
    LOG(this->log_wr_) << "Network API rejected vote " << vote->getHash()
                       << ". Status: " << static_cast<uint32_t>(ingress_decision.status)
                       << ", error: " << static_cast<std::string>(ingress_decision.error_code)
                       << ", queued effects: " << ingress_decision.queued_effect_count;
    return {};
  }

  const auto result = consumePbftVoteAdmission(ingress);
  executeConsensusNetworkEffects(16);

  if (!result.accepted && !result.already_present) {
    LOG(this->log_dg_) << "Vote " << vote->getHash() << " was not admitted by Rust vote transition";
    return result;
  }

  return result;
#else

  if (vote_mgr_->voteInVerifiedMap(vote)) {
    LOG(this->log_dg_) << "Vote " << vote->getHash() << " already inserted in verified queue";
    return {};
  }

  // Validate vote's period, round and step min/max values
  if (const auto vote_valid = validateVotePeriodRoundStep(vote, peer, validate_max_round_step); !vote_valid.first) {
    LOG(this->log_wr_) << "Vote period/round/step " << vote->getHash()
                       << " validation failed. Err: " << vote_valid.second;
    return {};
  }

  // Check if vote is unique per period, round & step & voter -> each address can generate just 1 vote
  // (for a value that isn't NBH) per period, round & step
  if (auto vote_valid = vote_mgr_->isUniqueVote(vote); !vote_valid.first) {
    // Create double voting proof
    slashing_manager_->submitDoubleVotingProof(vote, vote_valid.second);
    throw MaliciousPeerException("Received double vote", vote->getVoter());
  }

  // Validate vote's signature, vrf, etc...
  if (const auto vote_valid = vote_mgr_->validateVote(vote); !vote_valid.first) {
    LOG(this->log_wr_) << "Vote " << vote->getHash() << " validation failed. Err: " << vote_valid.second;
    return {};
  }

  if (!vote_mgr_->addVerifiedVote(vote)) {
    LOG(this->log_dg_) << "Vote " << vote->getHash() << " already inserted in verified queue(race condition)";
    return {};
  }

  if (pbft_block) {
    pbft_mgr_->processProposedBlock(pbft_block);
  }

  return {.accepted = true, .mark_vote_known = true, .gossip_vote = true};
#endif
}

#ifdef RUSTAXA_ENABLE
ExtVotesPacketHandler::VoteProcessingResult ExtVotesPacketHandler::consumePbftVoteAdmission(
    const rustaxa::NetworkPbftVoteAdmissionOutcome &outcome) {
  VoteProcessingResult result{.accepted = outcome.accepted,
                              .already_present = outcome.already_present,
                              .mark_vote_known = outcome.mark_vote_known,
                              .gossip_vote = outcome.gossip_vote,
                              .report_slashing = outcome.report_slashing,
                              .cancelled = !outcome.has_admission};
  if (!outcome.has_slashing_transaction_effect) {
    return result;
  }
  const auto &effect = outcome.slashing_transaction_effect;
  (void)rust_consensus_network_api_->executePbftVoteSlashingTransaction(
      network::PbftVoteSlashingTransaction{effect.status, effect.proof_hash, effect.wallet_index, effect.nonce,
                                           effect.contract_address, effect.value, effect.gas_limit,
                                           std::vector<uint8_t>(effect.call_data.begin(), effect.call_data.end())},
      kConf, *trx_mgr_);
  return result;
}
#endif

std::pair<bool, std::string> ExtVotesPacketHandler::validateVotePeriodRoundStep(const std::shared_ptr<PbftVote> &vote,
                                                                                const std::shared_ptr<TaraxaPeer> &peer,
                                                                                bool validate_max_round_step) {
  const auto [current_pbft_round, current_pbft_period] = pbft_mgr_->getPbftRoundAndPeriod();

  auto genErrMsg = [period = current_pbft_period, round = current_pbft_round,
                    step = pbft_mgr_->getPbftStep()](const std::shared_ptr<PbftVote> &vote) -> std::string {
    std::stringstream err;
    err << "Vote " << vote->getHash() << " (period, round, step) = (" << vote->getPeriod() << ", " << vote->getRound()
        << ", " << vote->getStep() << "). Current PBFT (period, round, step) = (" << period << ", " << round << ", "
        << step << ")";
    return err.str();
  };

  // Period validation
  // vote->getPeriod() == current_pbft_period - 1 && cert_vote -> potential reward vote
  if (vote->getPeriod() < current_pbft_period - 1 ||
      (vote->getPeriod() == current_pbft_period - 1 && vote->getType() != PbftVoteTypes::cert_vote)) {
    return {false, "Invalid period(too small): " + genErrMsg(vote)};
  } else if (this->kConf.network.ddos_protection.vote_accepting_periods &&
             vote->getPeriod() - 1 > current_pbft_period + this->kConf.network.ddos_protection.vote_accepting_periods) {
    // skip this check if kConf.network.ddos_protection.vote_accepting_periods == 0
    // vote->getPeriod() - 1 is here because votes are validated against vote_period - 1 in dpos contract
    // Do not request round sync too often here
    if (vote->getVoter() == peer->getId() &&
        std::chrono::system_clock::now() - last_pbft_block_sync_request_time_ > kSyncRequestInterval) {
      // request PBFT chain sync from this node
      this->sealAndSend(
          peer->getId(), SubprotocolPacketType::kGetPbftSyncPacket,
          encodePacketRlp(GetPbftSyncPacket{std::max(vote->getPeriod() - 1, peer->pbft_chain_size_.load())}));
      last_pbft_block_sync_request_time_ = std::chrono::system_clock::now();
    }

    return {false, "Invalid period(too big): " + genErrMsg(vote)};
  }

  // Round validation
  auto checking_round = current_pbft_round;
  // If period is not the same we assume current round is equal to 1
  // So we won't accept votes for future period with round bigger than kConf.network.vote_accepting_steps
  if (current_pbft_period != vote->getPeriod()) {
    checking_round = 1;
  }

  // vote->getRound() == checking_round - 1 && next_vote -> previous round next vote
  if (vote->getRound() < checking_round - 1 ||
      (vote->getRound() == checking_round - 1 && vote->getType() != PbftVoteTypes::next_vote)) {
    return {false, "Invalid round(too small): " + genErrMsg(vote)};
  } else if (validate_max_round_step && this->kConf.network.ddos_protection.vote_accepting_rounds &&
             vote->getRound() >= checking_round + this->kConf.network.ddos_protection.vote_accepting_rounds) {
    // skip this check if kConf.network.vote_accepting_rounds == 0
    // Trigger votes(round) syncing only if we are in sync in terms of period
    if (current_pbft_period == vote->getPeriod()) {
      // Do not request round sync too often here
      if (vote->getVoter() == peer->getId() &&
          std::chrono::system_clock::now() - last_votes_sync_request_time_ > kSyncRequestInterval) {
        // request round votes sync from this node
        this->requestPbftNextVotesAtPeriodRound(peer->getId(), current_pbft_period, current_pbft_round);
        last_votes_sync_request_time_ = std::chrono::system_clock::now();
      }
    }

    return {false, "Invalid round(too big): " + genErrMsg(vote)};
  }

  // Step validation
  auto checking_step = pbft_mgr_->getPbftStep();
  // If period or round is not the same we assume current step is equal to 1
  // So we won't accept votes for future rounds with step bigger than kConf.network.vote_accepting_steps
  if (current_pbft_period != vote->getPeriod() || current_pbft_round != vote->getRound()) {
    checking_step = 1;
  }

  // skip check if kConf.network.vote_accepting_steps == 0
  if (validate_max_round_step && this->kConf.network.ddos_protection.vote_accepting_steps &&
      vote->getStep() >= checking_step + this->kConf.network.ddos_protection.vote_accepting_steps) {
    return {false, "Invalid step(too big): " + genErrMsg(vote)};
  }

  return {true, ""};
}

bool ExtVotesPacketHandler::validateVoteAndBlock(const std::shared_ptr<PbftVote> &vote,
                                                 const std::shared_ptr<PbftBlock> &pbft_block) const {
  if (pbft_block->getPeriod() != vote->getPeriod()) {
    LOG(this->log_er_) << "Vote " << vote->getHash() << " period " << vote->getPeriod() << " != pbft block period "
                       << pbft_block->getPeriod();
    return false;
  }

  if (pbft_block->getBlockHash() != vote->getBlockHash()) {
    LOG(this->log_er_) << "Vote " << vote->getHash() << " voted block " << vote->getBlockHash() << " != actual block "
                       << pbft_block->getBlockHash();
    return false;
  }

  return true;
}

bool ExtVotesPacketHandler::isPbftRelevantVote(const std::shared_ptr<PbftVote> &vote) const {
  const auto [current_pbft_round, current_pbft_period] = pbft_mgr_->getPbftRoundAndPeriod();

  if (vote->getPeriod() >= current_pbft_period && vote->getRound() >= current_pbft_round) {
    // Standard current or future vote
    return true;
  } else if (vote->getPeriod() == current_pbft_period && vote->getRound() == (current_pbft_round - 1) &&
             vote->getType() == PbftVoteTypes::next_vote) {
    // Previous round next vote
    return true;
  } else if (vote->getPeriod() == current_pbft_period - 1 && vote->getType() == PbftVoteTypes::cert_vote) {
    // Previous period cert vote - potential reward vote
    return true;
  }

  return false;
}

void ExtVotesPacketHandler::requestPbftNextVotesAtPeriodRound(const dev::p2p::NodeID &peerID, PbftPeriod pbft_period,
                                                              PbftRound pbft_round) {
  LOG(log_dg_) << "Sending GetNextVotesSyncPacket with period:" << pbft_period << ", round:" << pbft_round;
  const auto packet = GetNextVotesBundlePacket{.peer_pbft_period = pbft_period, .peer_pbft_round = pbft_round};
  sealAndSend(peerID, SubprotocolPacketType::kGetNextVotesSyncPacket, encodePacketRlp(packet));
}

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkPbftVoteAdmissionOutcome ExtVotesPacketHandler::ingestPbftVote(
    const rustaxa::PbftVoteIngressFact &fact, const rustaxa::NetworkPbftVoteIngressContext &context) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api().consensus_network_admit_pbft_vote(fact, context,
                                                                              makeSlashingSubmitters(kConf));
}

rust::Vec<rustaxa::NetworkPbftVoteAdmissionOutcome> ExtVotesPacketHandler::ingestPbftVoteBundle(
    const rustaxa::PbftVoteIngressFact &reference, rust::Vec<rustaxa::PbftVoteIngressFact> votes,
    rust::Vec<rustaxa::NetworkPbftVoteIngressContext> contexts) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api().consensus_network_admit_pbft_vote_bundle(
      reference, std::move(votes), std::move(contexts), makeSlashingSubmitters(kConf));
}

void ExtVotesPacketHandler::executeConsensusNetworkEffects(size_t budget, std::optional<uint64_t> source_payload_id) {
  assert(rust_consensus_network_api_);
  while (true) {
    const auto batch = rust_consensus_network_api_->api().consensus_network_drain_work(
        static_cast<uint32_t>(transport_lane_), source_payload_id.value_or(0), source_payload_id.has_value(),
        static_cast<uint32_t>(budget));
    if (batch.effects.empty()) {
      break;
    }
    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto &effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kNetworkEffectResultStatusOk;
      try {
        const dev::p2p::NodeID peer_id(effect.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer);
        if (effect.kind == kNetworkEffectKindRequestSync && effect.sync_kind == kNetworkSyncKindPbftChain) {
          sealAndSend(peer_id, SubprotocolPacketType::kGetPbftSyncPacket,
                      encodePacketRlp(GetPbftSyncPacket{effect.sync_start}));
          last_pbft_block_sync_request_time_ = std::chrono::system_clock::now();
        } else if (effect.kind == kNetworkEffectKindSendPacket &&
                   effect.packet_kind == kNetworkPacketKindPbftVotesBundle) {
          const auto optimized_bundle_rlp = bytes(effect.payload_bytes.begin(), effect.payload_bytes.end());
          auto votes = decodePbftVotesBundleRlp(dev::RLP(optimized_bundle_rlp));
          auto target = peers_state_->getPeer(peer_id);
          if (!target) {
            throw std::runtime_error("Network API PBFT votes-bundle send target is no longer connected");
          }
          auto packet = VotesBundlePacket{OptimizedPbftVotesBundle{.votes = std::move(votes)}};
          if (!sealAndSend(peer_id, SubprotocolPacketType::kVotesBundlePacket, encodePacketRlp(packet))) {
            throw std::runtime_error("Network API PBFT votes-bundle transport send failed");
          }
          for (const auto &vote : packet.votes_bundle.votes) {
            target->markPbftVoteAsKnown(vote->getHash());
          }
        } else if (effect.kind == kNetworkEffectKindGossipPacket && effect.packet_kind == kNetworkPacketKindPbftVote) {
          auto gossip_vote =
              std::make_shared<PbftVote>(bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()));
          if (gossip_vote->getHash().asArray() != effect.object_hash) {
            throw std::runtime_error("Network API PBFT vote gossip effect has mismatched vote payload");
          }
          std::shared_ptr<PbftBlock> gossip_block;
          if (!effect.related_payload_bytes.empty()) {
            gossip_block = std::make_shared<PbftBlock>(
                bytes(effect.related_payload_bytes.begin(), effect.related_payload_bytes.end()));
            if (gossip_block->getBlockHash() != gossip_vote->getBlockHash()) {
              throw std::runtime_error("Network API PBFT vote gossip effect has mismatched PBFT block payload");
            }
          }
          for (const auto &peer : peers_state_->getAllPeers()) {
            if (peer.second->syncing_) {
              LOG(log_dg_) << " PBFT vote " << gossip_vote->getHash() << " not sent to " << peer.first
                           << " peer syncing";
              continue;
            }

            bool excluded = false;
            for (const auto &excluded_peer : effect.exclude_peers) {
              if (dev::p2p::NodeID(excluded_peer.id.data(), dev::p2p::NodeID::ConstructFromPointer) == peer.first) {
                excluded = true;
                break;
              }
            }
            if (excluded || peer.second->isPbftVoteKnown(gossip_vote->getHash())) {
              continue;
            }

            std::optional<VotePacket::OptionalData> optional_packet_data;
            if (gossip_block && !peer.second->isPbftBlockKnown(gossip_vote->getBlockHash())) {
              optional_packet_data =
                  VotePacket::OptionalData{gossip_block, net::consensusPbftProgress(pbft_chain_).finalized_period};
            }

            if (sealAndSend(peer.first, SubprotocolPacketType::kVotePacket,
                            encodePacketRlp(VotePacket(gossip_vote, std::move(optional_packet_data))))) {
              peer.second->markPbftVoteAsKnown(gossip_vote->getHash());
              if (optional_packet_data.has_value()) {
                peer.second->markPbftBlockAsKnown(gossip_block->getBlockHash());
                LOG(log_dg_) << " PBFT vote " << gossip_vote->getHash() << " together with block "
                             << gossip_block->getBlockHash() << " sent to " << peer.first;
              } else {
                LOG(log_dg_) << " PBFT vote " << gossip_vote->getHash() << " sent to " << peer.first;
              }
            }
          }
        } else if (effect.kind == kNetworkEffectKindGossipPacket &&
                   effect.packet_kind == kNetworkPacketKindPbftVotesBundle) {
          const auto optimized_bundle_rlp = bytes(effect.payload_bytes.begin(), effect.payload_bytes.end());
          const auto votes = decodePbftVotesBundleRlp(dev::RLP(optimized_bundle_rlp));
          for (const auto &target : peers_state_->getAllPeers()) {
            if (target.second->syncing_ || target.first == peer_id) {
              continue;
            }

            std::vector<std::shared_ptr<PbftVote>> unknown_votes;
            unknown_votes.reserve(votes.size());
            for (const auto &vote : votes) {
              if (!target.second->isPbftVoteKnown(vote->getHash())) {
                unknown_votes.push_back(vote);
              }
            }
            if (unknown_votes.empty()) {
              continue;
            }

            auto packet = VotesBundlePacket{OptimizedPbftVotesBundle{.votes = std::move(unknown_votes)}};
            if (sealAndSend(target.first, SubprotocolPacketType::kVotesBundlePacket, encodePacketRlp(packet))) {
              for (const auto &vote : packet.votes_bundle.votes) {
                target.second->markPbftVoteAsKnown(vote->getHash());
              }
            }
          }
        } else if (effect.kind == kNetworkEffectKindRecordConsensusObject &&
                   effect.object_kind == kNetworkObjectKindPbftBlock) {
          auto proposed_block =
              std::make_shared<PbftBlock>(bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()));
          if (proposed_block->getPeriod() != effect.period ||
              proposed_block->getBlockHash().asArray() != effect.object_hash) {
            throw std::runtime_error("Network API proposed PBFT block sidecar effect has mismatched block payload");
          }
          pbft_mgr_->processProposedBlock(proposed_block);
        } else if (effect.kind == kNetworkEffectKindRequestSync && effect.sync_kind == kNetworkSyncKindPbftNextVotes) {
          requestPbftNextVotesAtPeriodRound(peer_id, effect.period, effect.round);
          last_votes_sync_request_time_ = std::chrono::system_clock::now();
        } else if (effect.kind == kNetworkEffectKindMarkPeerKnown && effect.object_kind == kNetworkObjectKindPbftVote) {
          const auto peer = peers_state_->getPeer(peer_id);
          if (peer) {
            peer->markPbftVoteAsKnown(vote_hash_t(effect.object_hash.data(), vote_hash_t::ConstructFromPointer));
          }
        } else if (effect.kind == kNetworkEffectKindMarkPeerKnown &&
                   effect.object_kind == kNetworkObjectKindPbftBlock) {
          const auto peer = peers_state_->getPeer(peer_id);
          if (peer) {
            peer->markPbftBlockAsKnown(blk_hash_t(effect.object_hash.data(), blk_hash_t::ConstructFromPointer));
          }
        } else if (effect.kind == kNetworkEffectKindDisconnectPeer) {
          disconnect(peer_id, dev::p2p::UserReason);
        } else if (effect.kind == kNetworkEffectKindReportPeer) {
          LOG(log_wr_) << "Network API reported peer " << peer_id
                       << " with reason: " << static_cast<uint32_t>(effect.reason_code);
        }
      } catch (const std::exception &e) {
        result.status = kNetworkEffectResultStatusFailed;
        result.diagnostic = e.what();
      }

      results.push_back(std::move(result));
    }

    const auto acknowledgement =
        rust_consensus_network_api_->api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected an executor result batch: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }
}

#endif

}  // namespace taraxa::network::tarcap
