#include "network/tarcap/packets_handlers/latest/common/ext_pillar_vote_packet_handler.hpp"

#include <cassert>
#include <exception>
#include <stdexcept>

#include "pillar_chain/pillar_chain_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindPillarVote = 5;
constexpr uint32_t kNetworkPacketKindPillarVote = 13;
constexpr uint32_t kNetworkPacketKindPillarVotesBundle = 15;
constexpr uint8_t kPillarVoteRelevanceStatusRelevant = 0;
constexpr uint8_t kPillarVoteRelevanceStatusVoteAlreadyKnown = 1;
constexpr uint8_t kPillarVoteRelevanceStatusMissingCurrentPillarBlock = 2;
constexpr uint8_t kPillarVoteRelevanceStatusVotePeriodMismatch = 3;
constexpr uint8_t kPillarVoteRelevanceStatusVoteBlockHashMismatch = 4;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

rust::Vec<uint8_t> toBridgeBytes(const bytes &input) {
  rust::Vec<uint8_t> output;
  output.reserve(input.size());
  for (const auto byte : input) {
    output.push_back(static_cast<uint8_t>(byte));
  }
  return output;
}

uint32_t expectedPillarVotePacketKind(SubprotocolPacketType packet_type) {
  switch (packet_type) {
    case SubprotocolPacketType::kPillarVotePacket:
      return kNetworkPacketKindPillarVote;
    case SubprotocolPacketType::kPillarVotesBundlePacket:
      return kNetworkPacketKindPillarVotesBundle;
    default:
      throw std::runtime_error("Network API pillar vote admission received unsupported packet type");
  }
}

}  // namespace

struct ExtPillarVotePacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

ExtPillarVotePacketHandler::ExtPillarVotePacketHandler(
    const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager, const addr_t &node_addr,
    const std::string &log_channel)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, log_channel),
      pillar_chain_manager_{std::move(pillar_chain_manager)} {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

bool ExtPillarVotePacketHandler::processPillarVote(const std::shared_ptr<PillarVote> &vote,
                                                   const std::shared_ptr<TaraxaPeer> &peer,
                                                   SubprotocolPacketType packet_type) {
#ifdef RUSTAXA_ENABLE
  const auto relevance_plan = planPillarVoteRelevance(vote);
  if (!relevance_plan.is_relevant) {
    const auto current_pillar_block = pillar_chain_manager_->getCurrentPillarBlock();
    switch (relevance_plan.status) {
      case kPillarVoteRelevanceStatusVoteAlreadyKnown:
        LOG(this->log_dg_) << "Received vote " << vote->getHash() << " already saved";
        return false;
      case kPillarVoteRelevanceStatusMissingCurrentPillarBlock:
        LOG(this->log_nf_) << "Received vote's period " << vote->getPeriod()
                           << ", no pillar block created yet. Accepting votes with "
                           << pillar_chain_manager_->kFicusHfConfig.firstPillarBlockPeriod() + 1 << " period";
        return false;
      case kPillarVoteRelevanceStatusVotePeriodMismatch:
        if (!current_pillar_block) {
          LOG(this->log_nf_) << "Received vote's period " << vote->getPeriod() << ", current pillar block missing";
        } else {
          LOG(this->log_nf_) << "Received vote's period " << vote->getPeriod() << ", current pillar block period "
                             << current_pillar_block->getPeriod();
        }
        return false;
      case kPillarVoteRelevanceStatusVoteBlockHashMismatch:
        LOG(this->log_nf_) << "Received vote's block hash " << vote->getBlockHash() << " != current pillar block hash "
                           << current_pillar_block->getHash();
        return false;
      case kPillarVoteRelevanceStatusRelevant:
        break;
      default:
        LOG(this->log_wr_) << "Unable to evaluate pillar vote relevance for " << vote->getHash()
                           << ": network api status " << static_cast<uint32_t>(relevance_plan.status);
        return false;
    }
  }
#else
  if (!pillar_chain_manager_->isRelevantPillarVote(vote)) {
    LOG(this->log_dg_) << "Drop irrelevant pillar vote " << vote->getHash() << ", period " << vote->getPeriod()
                       << " from peer " << peer->getId();
    return false;
  }
#endif

  if (!pillar_chain_manager_->validatePillarVote(vote)) {
    // TODO: enable for mainnet
    // std::ostringstream err_msg;
    // err_msg << "Invalid pillar vote " << vote->getHash() << " from peer " << peer->getId();
    // throw MaliciousPeerException(err_msg.str());
    return false;
  }

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkPillarVoteAdmissionRequestEffects effects{};
  effects.peer_id = peer->getId().asArray();
  effects.vote_hash = vote->getHash().asArray();
  effects.period = vote->getPeriod();
  effects.vote_rlp = toBridgeBytes(vote->rlp());
  effects.source_payload_id = 0;
  effects.admit_vote = true;
  (void)queuePillarVoteAdmissionRequestEffects(effects, packet_type);
  executePillarVoteAdmissionEffect(vote, peer, packet_type);
#else
  pillar_chain_manager_->addVerifiedPillarVote(vote);

  // Mark pillar vote as known for peer
  peer->markPillarVoteAsKnown(vote->getHash());
#endif
  return true;
}

#ifdef RUSTAXA_ENABLE
rustaxa::PillarVoteRelevancePlan ExtPillarVotePacketHandler::planPillarVoteRelevance(
    const std::shared_ptr<PillarVote> &vote) const {
  assert(rust_consensus_network_api_);
  rustaxa::PillarVoteRelevanceFact fact{};
  fact.vote_period = vote->getPeriod();
  fact.vote_block_hash = vote->getBlockHash().asArray();
  fact.first_pillar_block_period = pillar_chain_manager_->kFicusHfConfig.firstPillarBlockPeriod();
  fact.pillar_blocks_interval = pillar_chain_manager_->kFicusHfConfig.pillar_blocks_interval;
  // Duplicate rejection remains covered by validatePillarVote during this slice
  // because tarcap cannot inspect the Rust-backed pillar vote index directly.
  fact.vote_already_known = false;

  if (const auto current_pillar_block = pillar_chain_manager_->getCurrentPillarBlock(); current_pillar_block) {
    fact.has_current_pillar_block = true;
    fact.current_pillar_block_period = current_pillar_block->getPeriod();
    fact.current_pillar_block_hash = current_pillar_block->getHash().asArray();
  }

  return rust_consensus_network_api_->api->consensus_network_plan_pillar_vote_relevance(fact);
}

rustaxa::NetworkIngressDecision ExtPillarVotePacketHandler::queuePillarVoteAdmissionRequestEffects(
    const rustaxa::NetworkPillarVoteAdmissionRequestEffects &effects, SubprotocolPacketType packet_type) {
  assert(rust_consensus_network_api_);
  switch (packet_type) {
    case SubprotocolPacketType::kPillarVotePacket:
      return rust_consensus_network_api_->api->consensus_network_queue_pillar_vote_admission_request_effects(effects);
    case SubprotocolPacketType::kPillarVotesBundlePacket:
      return rust_consensus_network_api_->api
          ->consensus_network_queue_pillar_vote_bundle_member_admission_request_effects(effects);
    default:
      throw std::runtime_error("Network API pillar vote admission received unsupported packet type");
  }
}

void ExtPillarVotePacketHandler::executePillarVoteAdmissionEffect(const std::shared_ptr<PillarVote> &vote,
                                                                  const std::shared_ptr<TaraxaPeer> &peer,
                                                                  SubprotocolPacketType packet_type) {
  assert(rust_consensus_network_api_);
  const auto batch = rust_consensus_network_api_->api->consensus_network_drain_work(1);
  rust::Vec<rustaxa::NetworkEffectResult> results;
  results.reserve(batch.effects.size());
  std::exception_ptr pending_exception;

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
      const auto expected_packet_kind = expectedPillarVotePacketKind(packet_type);
      const auto effect_payload = bytes(effect.payload_bytes.begin(), effect.payload_bytes.end());
      if (effect.kind != kNetworkEffectKindRecordConsensusObject ||
          effect.object_kind != kNetworkObjectKindPillarVote || effect.packet_kind != expected_packet_kind ||
          effect.peer_id != peer->getId().asArray() || !vote || vote->getHash().asArray() != effect.object_hash ||
          vote->getPeriod() != effect.period || vote->rlp() != effect_payload) {
        throw std::runtime_error("Network API pillar vote admission effect missing matching live vote");
      }

      pillar_chain_manager_->addVerifiedPillarVote(vote);
      peer->markPillarVoteAsKnown(vote->getHash());
    } catch (const std::exception &e) {
      result.status = kNetworkEffectResultStatusFailed;
      result.diagnostic = e.what();
      pending_exception = std::current_exception();
    }

    results.push_back(std::move(result));
  }

  if (!results.empty()) {
    (void)rust_consensus_network_api_->api->consensus_network_report_effect_results(std::move(results));
  }

  if (pending_exception) {
    std::rethrow_exception(pending_exception);
  }
}
#endif

ExtPillarVotePacketHandler::~ExtPillarVotePacketHandler() = default;

}  // namespace taraxa::network::tarcap
