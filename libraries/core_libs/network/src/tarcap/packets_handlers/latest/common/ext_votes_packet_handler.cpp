#include "network/tarcap/packets_handlers/latest/common/ext_votes_packet_handler.hpp"

#include "network/tarcap/packets/latest/get_next_votes_bundle_packet.hpp"
#include "network/tarcap/packets/latest/vote_packet.hpp"
#include "network/tarcap/packets_handlers/latest/common/exceptions.hpp"
#include "pbft/pbft_manager.hpp"
#include "vote/pbft_vote.hpp"
#include "vote/votes_bundle_rlp.hpp"
#include "vote_manager/vote_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindGossipPacket = 1;
constexpr uint8_t kNetworkEffectKindMarkPeerKnown = 2;
constexpr uint8_t kNetworkEffectKindRequestSync = 3;
constexpr uint8_t kNetworkEffectKindReportPeer = 4;
constexpr uint8_t kNetworkEffectKindDisconnectPeer = 5;
constexpr uint8_t kNetworkSyncKindPbftChain = 0;
constexpr uint8_t kNetworkSyncKindPbftNextVotes = 1;
constexpr uint8_t kNetworkObjectKindPbftVote = 0;
constexpr uint8_t kNetworkObjectKindPbftBlock = 1;
constexpr uint32_t kNetworkPacketKindPbftVote = 1;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

}  // namespace

struct ExtVotesPacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

ExtVotesPacketHandler::ExtVotesPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                             std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                             std::shared_ptr<PbftManager> pbft_mgr,
                                             std::shared_ptr<PbftChain> pbft_chain,
                                             std::shared_ptr<VoteManager> vote_mgr,
                                             std::shared_ptr<SlashingManager> slashing_manager, const addr_t &node_addr,
                                             const std::string &log_channel_name)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, log_channel_name),
      last_votes_sync_request_time_(std::chrono::system_clock::now()),
      last_pbft_block_sync_request_time_(std::chrono::system_clock::now()),
      pbft_mgr_(std::move(pbft_mgr)),
      pbft_chain_(std::move(pbft_chain)),
      vote_mgr_(std::move(vote_mgr)),
      slashing_manager_(std::move(slashing_manager)) {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

ExtVotesPacketHandler::~ExtVotesPacketHandler() = default;

ExtVotesPacketHandler::VoteProcessingResult ExtVotesPacketHandler::processVote(
    const std::shared_ptr<PbftVote> &vote, const std::shared_ptr<PbftBlock> &pbft_block,
    const std::shared_ptr<TaraxaPeer> &peer, bool validate_max_round_step) {
  if (pbft_block && !validateVoteAndBlock(vote, pbft_block)) {
    throw MaliciousPeerException("Received vote's voted value != received pbft block");
  }

#ifdef RUSTAXA_ENABLE
  // TODO(rustaxa): move this temporary network-handler hook into the future
  // tarcap/network pipeline overlay. Rust decides deterministic PBFT vote
  // ingress gates here; C++ still executes peer sync requests, live sidecar
  // admission, proposed-block handling, logging, and typed network effects.
  if (vote_mgr_->voteAlreadyValidated(vote->getHash())) {
    LOG(this->log_dg_) << "Received vote " << vote->getHash() << " has already been validated";
    return {};
  }

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
  network_ingress_context.peer_id = peer->getId().asArray();
  network_ingress_context.peer_pbft_chain_size = peer->pbft_chain_size_.load();
  network_ingress_context.source_payload_id = 0;

  const auto ingress_decision = ingestPbftVote(ingress_fact, network_ingress_context);
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

  const auto admission_report = vote_mgr_->addVerifiedVoteWithReport(vote);
  VoteProcessingResult result{};
  result.accepted = admission_report.accepted;
  result.mark_vote_known = admission_report.mark_vote_known;
  result.gossip_vote = admission_report.gossip_vote;
  result.report_slashing = admission_report.report_slashing;
  if (!result.accepted) {
    LOG(this->log_dg_) << "Vote " << vote->getHash() << " was not admitted by Rust vote transition";
    return result;
  }

  if (admission_report.mark_vote_known) {
    rustaxa::NetworkPbftVoteAdmissionEffects effects{};
    effects.peer_id = peer->getId().asArray();
    effects.vote_hash = vote->getHash().asArray();
    effects.source_payload_id = 0;
    effects.mark_vote_known = true;
    (void)queuePbftVoteAdmissionEffects(effects);
    executeConsensusNetworkEffects(16);
    result.mark_vote_known = false;
  }

  if (pbft_block) {
    pbft_mgr_->processProposedBlock(pbft_block);
  }

  return result;
#endif

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
}

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
rustaxa::PbftVoteIngressPlan ExtVotesPacketHandler::planPbftVoteIngress(
    const rustaxa::PbftVoteIngressFact &fact, const rustaxa::PbftVoteIngressContext &context) const {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_plan_pbft_vote_ingress(fact, context);
}

rustaxa::PbftVoteIngressPlan ExtVotesPacketHandler::planPbftVoteBundleIngress(
    const rustaxa::PbftVoteIngressFact &reference, const rustaxa::PbftVoteIngressFact &vote,
    const rustaxa::PbftVoteIngressContext &context) const {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_plan_pbft_vote_bundle_ingress(reference, vote, context);
}

rustaxa::NetworkIngressDecision ExtVotesPacketHandler::ingestPbftVote(
    const rustaxa::PbftVoteIngressFact &fact, const rustaxa::NetworkPbftVoteIngressContext &context) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_ingest_pbft_vote(fact, context);
}

rustaxa::NetworkIngressDecision ExtVotesPacketHandler::ingestPbftVoteBundleMember(
    const rustaxa::PbftVoteIngressFact &reference, const rustaxa::PbftVoteIngressFact &vote,
    const rustaxa::NetworkPbftVoteIngressContext &context) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_ingest_pbft_vote_bundle_member(reference, vote, context);
}

rustaxa::NetworkIngressDecision ExtVotesPacketHandler::queuePbftVoteAdmissionEffects(
    const rustaxa::NetworkPbftVoteAdmissionEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_pbft_vote_admission_effects(effects);
}

rustaxa::NetworkIngressDecision ExtVotesPacketHandler::queuePbftBlockAdmissionEffects(
    const rustaxa::NetworkPbftBlockAdmissionEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_pbft_block_admission_effects(effects);
}

rustaxa::NetworkIngressDecision ExtVotesPacketHandler::queuePbftVoteGossipEffects(
    const rustaxa::NetworkPbftVoteGossipEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_pbft_vote_gossip_effects(effects);
}

void ExtVotesPacketHandler::executeConsensusNetworkEffects(size_t budget) {
  executeConsensusNetworkEffects(budget, nullptr, nullptr);
}

void ExtVotesPacketHandler::executeConsensusNetworkEffects(size_t budget,
                                                           const std::shared_ptr<PbftVote> &gossip_vote,
                                                           const std::shared_ptr<PbftBlock> &gossip_block) {
  assert(rust_consensus_network_api_);
  const auto batch = rust_consensus_network_api_->api->consensus_network_drain_work(static_cast<uint32_t>(budget));
  rust::Vec<rustaxa::NetworkEffectResult> results;
  results.reserve(batch.effects.size());

  for (const auto &effect : batch.effects) {
    rustaxa::NetworkEffectResult result{};
    result.effect_id = effect.effect_id;
    result.status = kNetworkEffectResultStatusOk;

    try {
      const dev::p2p::NodeID peer_id(effect.peer_id);
      if (effect.kind == kNetworkEffectKindRequestSync && effect.sync_kind == kNetworkSyncKindPbftChain) {
        sealAndSend(peer_id, SubprotocolPacketType::kGetPbftSyncPacket,
                    encodePacketRlp(GetPbftSyncPacket{effect.sync_start}));
        last_pbft_block_sync_request_time_ = std::chrono::system_clock::now();
      } else if (effect.kind == kNetworkEffectKindGossipPacket && effect.packet_kind == kNetworkPacketKindPbftVote) {
        if (!gossip_vote || gossip_vote->getHash().asArray() != effect.object_hash) {
          throw std::runtime_error("Network API PBFT vote gossip effect missing matching live vote");
        }
        for (const auto &peer : peers_state_->getAllPeers()) {
          if (peer.second->syncing_) {
            LOG(log_dg_) << " PBFT vote " << gossip_vote->getHash() << " not sent to " << peer.first
                         << " peer syncing";
            continue;
          }

          bool excluded = false;
          for (const auto &excluded_peer : effect.exclude_peers) {
            if (dev::p2p::NodeID(excluded_peer.id) == peer.first) {
              excluded = true;
              break;
            }
          }
          if (excluded || peer.second->isPbftVoteKnown(gossip_vote->getHash())) {
            continue;
          }

          std::optional<VotePacket::OptionalData> optional_packet_data;
          if (gossip_block && !peer.second->isPbftBlockKnown(gossip_vote->getBlockHash())) {
            optional_packet_data = VotePacket::OptionalData{gossip_block, pbft_chain_->getPbftChainSize()};
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
      } else if (effect.kind == kNetworkEffectKindRequestSync && effect.sync_kind == kNetworkSyncKindPbftNextVotes) {
        requestPbftNextVotesAtPeriodRound(peer_id, effect.period, effect.round);
        last_votes_sync_request_time_ = std::chrono::system_clock::now();
      } else if (effect.kind == kNetworkEffectKindMarkPeerKnown && effect.object_kind == kNetworkObjectKindPbftVote) {
        const auto peer = peers_state_->getPeer(peer_id);
        if (peer) {
          peer->markPbftVoteAsKnown(vote_hash_t(effect.object_hash));
        }
      } else if (effect.kind == kNetworkEffectKindMarkPeerKnown && effect.object_kind == kNetworkObjectKindPbftBlock) {
        const auto peer = peers_state_->getPeer(peer_id);
        if (peer) {
          peer->markPbftBlockAsKnown(blk_hash_t(effect.object_hash));
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

  if (!results.empty()) {
    (void)rust_consensus_network_api_->api->consensus_network_report_effect_results(std::move(results));
  }
}
#endif

}  // namespace taraxa::network::tarcap
