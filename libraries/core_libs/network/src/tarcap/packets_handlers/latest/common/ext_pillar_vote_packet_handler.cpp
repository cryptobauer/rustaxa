#include "network/tarcap/packets_handlers/latest/common/ext_pillar_vote_packet_handler.hpp"

#include <algorithm>
#include <cassert>
#include <optional>
#include <stdexcept>

#include "network/tarcap/packets/latest/pillar_vote_packet.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
#include "vote/pillar_vote.hpp"
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
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindPillarVote = 5;
constexpr uint32_t kNetworkPacketKindPillarVote = 13;

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& input) {
  rust::Vec<uint8_t> out;
  out.reserve(input.size());
  for (const auto byte : input) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

}  // namespace
#endif

ExtPillarVotePacketHandler::ExtPillarVotePacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
#ifdef RUSTAXA_ENABLE
    network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
#endif
    const addr_t& node_addr, const std::string& log_channel)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, log_channel),
      pillar_chain_manager_{std::move(pillar_chain_manager)}
#ifdef RUSTAXA_ENABLE
      ,
      rust_consensus_network_api_(std::move(consensus_network_api)),
      transport_lane_(transport_lane)
#endif
{
}

bool ExtPillarVotePacketHandler::processPillarVote(const std::shared_ptr<PillarVote>& vote,
                                                   const std::shared_ptr<TaraxaPeer>& peer,
                                                   SubprotocolPacketType packet_type) {
  const auto results = processPillarVotes({vote}, peer, packet_type);
  return results.size() == 1 && results.front();
}

std::vector<bool> ExtPillarVotePacketHandler::processPillarVotes(const std::vector<std::shared_ptr<PillarVote>>& votes,
                                                                 const std::shared_ptr<TaraxaPeer>& peer,
                                                                 SubprotocolPacketType packet_type) {
  std::vector<bool> accepted(votes.size(), false);

#ifdef RUSTAXA_ENABLE
  rust::Vec<rustaxa::PillarVoteRlpPayload> payloads;
  payloads.reserve(votes.size());
  for (const auto& vote : votes) {
    rustaxa::PillarVoteRlpPayload payload;
    payload.vote_rlp = toBridgeBytes(vote->rlp());
    payloads.push_back(std::move(payload));
  }

  auto lane_execution_lock = rust_consensus_network_api_->lockTransportLane(static_cast<uint32_t>(transport_lane_));
  const auto decisions =
      ingestPillarVotes(std::move(payloads), peer, packet_type == SubprotocolPacketType::kPillarVotePacket);
  if (decisions.size() != votes.size()) {
    throw std::runtime_error("Network API pillar-vote ingress returned an invalid decision count");
  }

  std::vector<uint64_t> application_effect_ids;
  std::vector<size_t> routed_vote_indices;
  application_effect_ids.reserve(decisions.size());
  routed_vote_indices.reserve(decisions.size());
  for (size_t index = 0; index < decisions.size(); ++index) {
    const auto& decision = decisions[index];
    if (!decision.routed || decision.status != 0 || decision.application_effect_id == 0) {
      LOG(this->log_dg_) << "Network API rejected pillar vote " << votes[index]->getHash()
                         << ". Status: " << static_cast<uint32_t>(decision.status)
                         << ", error: " << static_cast<std::string>(decision.error_code);
      continue;
    }
    application_effect_ids.push_back(decision.application_effect_id);
    routed_vote_indices.push_back(index);
  }

  const auto routed_results = executeConsensusNetworkEffects(application_effect_ids);
  for (size_t index = 0; index < routed_results.size(); ++index) {
    accepted[routed_vote_indices[index]] = routed_results[index];
  }
  return accepted;
#else
  static_cast<void>(packet_type);
  for (size_t index = 0; index < votes.size(); ++index) {
    const auto& vote = votes[index];
    if (!pillar_chain_manager_->isRelevantPillarVote(vote)) {
      LOG(this->log_dg_) << "Drop irrelevant pillar vote " << vote->getHash() << ", period " << vote->getPeriod()
                         << " from peer " << peer->getId();
      continue;
    }

    if (!pillar_chain_manager_->validatePillarVote(vote)) {
      // TODO: enable malicious-peer reporting for mainnet.
      continue;
    }

    pillar_chain_manager_->addVerifiedPillarVote(vote);
    peer->markPillarVoteAsKnown(vote->getHash());
    accepted[index] = true;
  }
  return accepted;
#endif
}

#ifdef RUSTAXA_ENABLE
rust::Vec<rustaxa::NetworkIngressDecision> ExtPillarVotePacketHandler::ingestPillarVotes(
    rust::Vec<rustaxa::PillarVoteRlpPayload> votes, const std::shared_ptr<TaraxaPeer>& peer, bool allow_gossip) const {
  assert(rust_consensus_network_api_);
  rustaxa::NetworkPillarVoteIngressContext context{};
  context.transport_lane = static_cast<uint32_t>(transport_lane_);
  context.peer_id = peer->getId().asArray();
  context.source_payload_id = 0;
  context.ficus_activation_period = kConf.genesis.state.hardforks.ficus_hf.block_num;
  context.allow_gossip = allow_gossip;
  return rust_consensus_network_api_->api().consensus_network_ingest_pillar_vote_bundle(context, std::move(votes));
}

std::vector<bool> ExtPillarVotePacketHandler::executeConsensusNetworkEffects(
    const std::vector<uint64_t>& application_effect_ids) {
  if (application_effect_ids.empty()) {
    return {};
  }

  std::vector<bool> application_completed(application_effect_ids.size(), false);
  std::vector<bool> application_accepted(application_effect_ids.size(), false);
  std::vector<std::optional<std::string>> application_failures(application_effect_ids.size());

  while (true) {
    const auto batch =
        rust_consensus_network_api_->api().consensus_network_drain_work(static_cast<uint32_t>(transport_lane_), 64);
    if (batch.effects.empty()) {
      break;
    }

    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kNetworkEffectResultStatusOk;
      const auto requested_application =
          std::find(application_effect_ids.begin(), application_effect_ids.end(), effect.effect_id);
      const auto requested_application_index =
          static_cast<size_t>(std::distance(application_effect_ids.begin(), requested_application));

      try {
        const dev::p2p::NodeID peer_id(effect.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer);
        if (effect.kind == kNetworkEffectKindRecordConsensusObject &&
            effect.object_kind == kNetworkObjectKindPillarVote) {
          if (requested_application == application_effect_ids.end()) {
            throw std::runtime_error("Pillar-vote network executor received an uncorrelated application effect");
          }
          auto vote = std::make_shared<PillarVote>(bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()));
          if (vote->getHash().asArray() != effect.object_hash || vote->getPeriod() != effect.period) {
            throw std::runtime_error("Network API pillar-vote admission effect has mismatched payload identity");
          }
          const auto report = pillar_chain_manager_->admitPillarVote(vote);
          result.admission_accepted = report.accepted;
          result.admission_already_present = report.already_present;
          application_completed[requested_application_index] = true;
          application_accepted[requested_application_index] = report.accepted;
        } else if (effect.kind == kNetworkEffectKindMarkPeerKnown &&
                   effect.object_kind == kNetworkObjectKindPillarVote) {
          if (const auto target = peers_state_->getPeer(peer_id); target) {
            target->markPillarVoteAsKnown(vote_hash_t(effect.object_hash.data(), vote_hash_t::ConstructFromPointer));
          }
        } else if (effect.kind == kNetworkEffectKindGossipPacket &&
                   effect.packet_kind == kNetworkPacketKindPillarVote) {
          auto vote = std::make_shared<PillarVote>(bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()));
          if (vote->getHash().asArray() != effect.object_hash) {
            throw std::runtime_error("Network API pillar-vote gossip effect has mismatched payload identity");
          }
          for (const auto& target : peers_state_->getAllPeers()) {
            const auto excluded = std::ranges::any_of(effect.exclude_peers, [&target](const auto& excluded_peer) {
              return dev::p2p::NodeID(excluded_peer.id.data(), dev::p2p::NodeID::ConstructFromPointer) == target.first;
            });
            if (excluded || target.second->syncing_ || target.second->isPillarVoteKnown(vote->getHash())) {
              continue;
            }
            if (sealAndSend(target.first, SubprotocolPacketType::kPillarVotePacket,
                            encodePacketRlp(PillarVotePacket(vote)))) {
              target.second->markPillarVoteAsKnown(vote->getHash());
            }
          }
        } else {
          throw std::runtime_error("Pillar-vote network executor received an unsupported effect");
        }
      } catch (const std::exception& e) {
        result.status = kNetworkEffectResultStatusFailed;
        result.diagnostic = e.what();
        if (requested_application != application_effect_ids.end()) {
          application_completed[requested_application_index] = true;
          application_failures[requested_application_index] = e.what();
        }
      }
      results.push_back(std::move(result));
    }

    const auto acknowledgement =
        rust_consensus_network_api_->api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected pillar-vote executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }

  for (size_t index = 0; index < application_effect_ids.size(); ++index) {
    if (application_failures[index]) {
      throw std::runtime_error(*application_failures[index]);
    }
    if (!application_completed[index]) {
      throw std::runtime_error("Network API did not execute a correlated pillar-vote application effect");
    }
  }
  return application_accepted;
}
#endif

ExtPillarVotePacketHandler::~ExtPillarVotePacketHandler() = default;

}  // namespace taraxa::network::tarcap
