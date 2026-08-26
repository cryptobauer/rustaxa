#include "network/tarcap/packets_handlers/latest/common/ext_pillar_vote_packet_handler.hpp"

#include <algorithm>
#include <cassert>
#include <stdexcept>

#include "network/tarcap/packets/latest/pillar_vote_packet.hpp"
#ifndef RUSTAXA_ENABLE
#include "pillar_chain/pillar_chain_manager.hpp"
#endif
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
constexpr uint8_t kNetworkEffectKindReportPeer = 4;
constexpr uint8_t kNetworkEffectKindDisconnectPeer = 5;
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
#ifndef RUSTAXA_ENABLE
    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
#else
    network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
#endif
    const addr_t& node_addr, const std::string& log_channel)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, log_channel)
#ifndef RUSTAXA_ENABLE
      ,
      pillar_chain_manager_{std::move(pillar_chain_manager)}
#else
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
#ifdef RUSTAXA_ENABLE
  rust::Vec<rustaxa::PillarVoteRlpPayload> payloads;
  payloads.reserve(votes.size());
  for (const auto& vote : votes) {
    rustaxa::PillarVoteRlpPayload payload;
    payload.vote_rlp = toBridgeBytes(vote->rlp());
    payloads.push_back(std::move(payload));
  }

  auto lane_execution_lock = rust_consensus_network_api_->lockTransportLane(static_cast<uint32_t>(transport_lane_));
  const auto outcomes =
      ingestPillarVotes(std::move(payloads), peer, packet_type == SubprotocolPacketType::kPillarVotePacket);
  if (outcomes.size() != votes.size()) {
    throw std::runtime_error("Network API pillar-vote ingress returned an invalid outcome count");
  }

  std::vector<std::array<uint8_t, 32>> vote_hashes;
  vote_hashes.reserve(votes.size());
  std::vector<bool> accepted(votes.size(), false);
  for (size_t index = 0; index < outcomes.size(); ++index) {
    const auto& outcome = outcomes[index];
    const auto& decision = outcome.decision;
    const auto vote_hash = votes[index]->getHash().asArray();
    vote_hashes.push_back(vote_hash);
    if (!decision.routed || decision.status != 0) {
      LOG(this->log_dg_) << "Network API rejected pillar vote " << votes[index]->getHash()
                         << ". Status: " << static_cast<uint32_t>(decision.status)
                         << ", error: " << static_cast<std::string>(decision.error_code);
      continue;
    }
    if (!outcome.has_admission) {
      throw std::runtime_error("Routed pillar vote has no native admission outcome");
    }
    if (outcome.vote_hash != vote_hash) {
      throw std::runtime_error("Native pillar-vote admission returned an unexpected vote hash");
    }
    if (!outcome.accepted && !outcome.duplicate) {
      LOG(this->log_dg_) << "Native pillar-vote admission rejected vote " << votes[index]->getHash()
                         << ". Status: " << static_cast<uint32_t>(outcome.status);
    }
    accepted[index] = outcome.accepted;
  }
  executeConsensusNetworkEffects(vote_hashes);
  return accepted;
#else
  static_cast<void>(packet_type);
  std::vector<bool> accepted(votes.size(), false);
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
rust::Vec<rustaxa::NetworkPillarVoteAdmissionOutcome> ExtPillarVotePacketHandler::ingestPillarVotes(
    rust::Vec<rustaxa::PillarVoteRlpPayload> votes, const std::shared_ptr<TaraxaPeer>& peer, bool allow_gossip) const {
  assert(rust_consensus_network_api_);
  rustaxa::NetworkPillarVoteIngressContext context{};
  context.transport_lane = static_cast<uint32_t>(transport_lane_);
  context.peer_id = peer->getId().asArray();
  context.source_payload_id = 0;
  context.allow_gossip = allow_gossip;
  return rust_consensus_network_api_->api().consensus_network_ingest_pillar_vote_bundle(context, std::move(votes));
}

void ExtPillarVotePacketHandler::executeConsensusNetworkEffects(
    const std::vector<std::array<uint8_t, 32>>& expected_vote_hashes) {
  while (true) {
    const auto batch = rust_consensus_network_api_->api().consensus_network_drain_work(
        static_cast<uint32_t>(transport_lane_), 0, false, 64);
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
      try {
        const dev::p2p::NodeID peer_id(effect.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer);
        if (effect.kind == kNetworkEffectKindMarkPeerKnown && effect.object_kind == kNetworkObjectKindPillarVote) {
          const auto expected = std::find(expected_vote_hashes.begin(), expected_vote_hashes.end(), effect.object_hash);
          if (expected == expected_vote_hashes.end()) {
            throw std::runtime_error("Pillar-vote known effect has an unexpected vote hash");
          }
          if (const auto target = peers_state_->getPeer(peer_id); target) {
            target->markPillarVoteAsKnown(vote_hash_t(effect.object_hash.data(), vote_hash_t::ConstructFromPointer));
          }
        } else if (effect.kind == kNetworkEffectKindGossipPacket &&
                   effect.packet_kind == kNetworkPacketKindPillarVote &&
                   effect.object_kind == kNetworkObjectKindPillarVote) {
          if (std::find(expected_vote_hashes.begin(), expected_vote_hashes.end(), effect.object_hash) ==
              expected_vote_hashes.end()) {
            throw std::runtime_error("Pillar-vote gossip effect has an unexpected vote hash");
          }
          for (const auto& target : peers_state_->getAllPeers()) {
            const auto excluded = std::ranges::any_of(effect.exclude_peers, [&target](const auto& excluded_peer) {
              return dev::p2p::NodeID(excluded_peer.id.data(), dev::p2p::NodeID::ConstructFromPointer) == target.first;
            });
            const vote_hash_t vote_hash(effect.object_hash.data(), vote_hash_t::ConstructFromPointer);
            if (excluded || target.second->syncing_ || target.second->isPillarVoteKnown(vote_hash)) {
              continue;
            }
            dev::RLPStream packet_rlp(1);
            packet_rlp.appendRaw(bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()));
            if (sealAndSend(target.first, SubprotocolPacketType::kPillarVotePacket, packet_rlp.invalidate())) {
              target.second->markPillarVoteAsKnown(vote_hash);
            }
          }
        } else if (effect.kind == kNetworkEffectKindReportPeer) {
          LOG(this->log_wr_) << "Network API reported peer " << peer_id
                             << " with reason: " << static_cast<uint32_t>(effect.reason_code);
        } else if (effect.kind == kNetworkEffectKindDisconnectPeer) {
          disconnect(peer_id, dev::p2p::UserReason);
        } else {
          throw std::runtime_error("Pillar-vote network executor received an unsupported effect");
        }
      } catch (const std::exception& e) {
        result.status = kNetworkEffectResultStatusFailed;
        result.diagnostic = e.what();
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
}
#endif

ExtPillarVotePacketHandler::~ExtPillarVotePacketHandler() = default;

}  // namespace taraxa::network::tarcap
