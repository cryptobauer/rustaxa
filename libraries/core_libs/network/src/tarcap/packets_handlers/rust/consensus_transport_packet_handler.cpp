#include "network/tarcap/packets_handlers/rust/consensus_transport_packet_handler.hpp"

#include <algorithm>
#include <stdexcept>

#include "network/tarcap/packets/latest/get_next_votes_bundle_packet.hpp"
#include "network/tarcap/packets/latest/get_pbft_sync_packet.hpp"
#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {

constexpr uint32_t kPendingDagTransportLane = 0;
constexpr uint8_t kEffectSendPacket = 0;
constexpr uint8_t kEffectGossipPacket = 1;
constexpr uint8_t kEffectMarkPeerKnown = 2;
constexpr uint8_t kEffectRequestSync = 3;
constexpr uint8_t kEffectReportPeer = 4;
constexpr uint8_t kEffectDisconnectPeer = 5;
constexpr uint8_t kObjectPbftVote = 0;
constexpr uint8_t kObjectPbftBlock = 1;
constexpr uint8_t kObjectPillarVote = 5;
constexpr uint8_t kSyncPbftChain = 0;
constexpr uint8_t kSyncPbftNextVotes = 1;
constexpr uint32_t kPacketPbftVote = 1;

network::ConsensusPeerCandidate toPeerCandidate(const std::shared_ptr<TaraxaPeer>& peer) {
  return {peer->getId().asArray(),
          peer->pbft_chain_size_.load(),
          peer->dag_level_.load(),
          peer->peer_light_node.load(),
          peer->peer_light_node_history.load(),
          peer->peer_dag_synced_.load(),
          peer->peer_dag_syncing_.load(),
          peer->dagSyncingAllowed()};
}

}  // namespace

RustConsensusTransportPacketHandler::RustConsensusTransportPacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, net::ConsensusQueryClient consensus_query,
    network::ConsensusLiveStatusProvider consensus_status, network::ConsensusNetworkApiShared consensus_network_api,
    const addr_t& node_addr, const std::string& log_channel)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, log_channel),
      pbft_chain_(std::move(consensus_query)),
      consensus_status_(std::move(consensus_status)),
      rust_consensus_network_api_(std::move(consensus_network_api)) {
  if (!rust_consensus_network_api_) {
    throw std::invalid_argument("Rust consensus transport handler requires the application network API");
  }
}

RustConsensusTransportPacketHandler::~RustConsensusTransportPacketHandler() = default;

void RustConsensusTransportPacketHandler::requestPendingDagBlocks(std::shared_ptr<TaraxaPeer> peer) {
  std::vector<network::ConsensusPeerCandidate> candidates;
  if (peer) {
    candidates.push_back(toPeerCandidate(peer));
  } else {
    for (const auto& entry : peers_state_->getAllPeers()) {
      candidates.push_back(toPeerCandidate(entry.second));
    }
  }
  if (candidates.empty()) {
    return;
  }

  const auto outcome = rust_consensus_network_api_->requestPendingDagBlocks(
      kPendingDagTransportLane, consensus_status_().syncing_period, candidates,
      network::PendingDagBlocksExecutor{[this](const auto& peer_id, const auto& payload, uint64_t /* period */) {
        const auto target =
            peers_state_->getPeer(dev::p2p::NodeID(peer_id.data(), dev::p2p::NodeID::ConstructFromPointer));
        if (!target) {
          return false;
        }
        bool expected = false;
        if (!target->peer_dag_syncing_.compare_exchange_strong(expected, true)) {
          return false;
        }
        const auto sent = sealAndSend(target->getId(), SubprotocolPacketType::kGetDagSyncPacket,
                                      dev::bytes(payload.begin(), payload.end()));
        if (!sent) {
          target->peer_dag_syncing_ = false;
        }
        return sent;
      }});
  if (outcome.queued_effect_count == 0) {
    LOG(log_dg_) << "Native pending-DAG request skipped: " << outcome.error_code;
  }
}

bool RustConsensusTransportPacketHandler::requestPbftNextVotesAtPeriodRound(const dev::p2p::NodeID& peer_id,
                                                                            PbftPeriod peer_pbft_period,
                                                                            PbftRound peer_pbft_round) {
  return sealAndSend(peer_id, SubprotocolPacketType::kGetNextVotesSyncPacket,
                     encodePacketRlp(GetNextVotesBundlePacket{peer_pbft_period, peer_pbft_round}));
}

network::ConsensusTransportExecutor RustConsensusTransportPacketHandler::consensusTransportExecutor() {
  return {[this](const network::ConsensusTransportEffect& effect) -> network::ConsensusTransportExecutionResult {
    try {
      const auto peer_id = dev::p2p::NodeID(effect.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer);
      if (effect.kind == kEffectRequestSync && effect.sync_kind == kSyncPbftChain) {
        if (!tryReservePbftSyncRequest()) {
          return {};
        }
        if (!sealAndSend(peer_id, SubprotocolPacketType::kGetPbftSyncPacket,
                         encodePacketRlp(GetPbftSyncPacket{effect.sync_start}))) {
          throw std::runtime_error("PBFT sync request transport failed");
        }
      } else if (effect.kind == kEffectRequestSync && effect.sync_kind == kSyncPbftNextVotes) {
        if (!tryReserveNextVotesSyncRequest()) {
          return {};
        }
        if (!requestPbftNextVotesAtPeriodRound(peer_id, effect.period, effect.round)) {
          throw std::runtime_error("PBFT next-votes sync request transport failed");
        }
      } else if (effect.kind == kEffectMarkPeerKnown) {
        if (const auto peer = peers_state_->getPeer(peer_id); peer) {
          const vote_hash_t hash(effect.object_hash.data(), vote_hash_t::ConstructFromPointer);
          if (effect.object_kind == kObjectPbftVote) {
            peer->markPbftVoteAsKnown(hash);
          } else if (effect.object_kind == kObjectPbftBlock) {
            peer->markPbftBlockAsKnown(blk_hash_t(effect.object_hash.data(), blk_hash_t::ConstructFromPointer));
          } else if (effect.object_kind == kObjectPillarVote) {
            peer->markPillarVoteAsKnown(hash);
          } else {
            throw std::runtime_error("Unsupported consensus known-object effect");
          }
        }
      } else if (effect.kind == kEffectDisconnectPeer) {
        disconnect(peer_id, dev::p2p::UserReason);
      } else if (effect.kind == kEffectReportPeer) {
        peers_state_->set_peer_malicious(peer_id);
        LOG(log_wr_) << "Native consensus network reported peer " << peer_id << ", reason "
                     << static_cast<uint32_t>(effect.reason_code);
      } else if (effect.kind == kEffectSendPacket || effect.kind == kEffectGossipPacket) {
        auto wrap_payload = [&effect, this](bool include_related_payload) {
          dev::RLPStream packet;
          if (effect.packet_kind == kPacketPbftVote) {
            packet.appendList(2);
            packet.appendRaw(dev::bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()));
            if (!include_related_payload || effect.related_payload_bytes.empty()) {
              packet.append(uint64_t{0});
            } else {
              packet.appendList(2);
              packet.appendRaw(dev::bytes(effect.related_payload_bytes.begin(), effect.related_payload_bytes.end()));
              packet.append(pbft_chain_ ? net::consensusPbftProgress(pbft_chain_).finalized_period : uint64_t{0});
            }
          } else {
            packet.appendList(1);
            packet.appendRaw(dev::bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()));
          }
          return packet.invalidate();
        };
        const auto packet_type = static_cast<SubprotocolPacketType>(effect.packet_kind);
        if (effect.kind == kEffectSendPacket) {
          const auto packet_rlp = wrap_payload(true);
          if (!sealAndSend(peer_id, packet_type, dev::bytes(packet_rlp.begin(), packet_rlp.end()))) {
            throw std::runtime_error("Consensus packet send failed");
          }
        } else {
          const auto related_block_hash =
              effect.related_payload_bytes.empty()
                  ? blk_hash_t{}
                  : dev::sha3(dev::bytes(effect.related_payload_bytes.begin(), effect.related_payload_bytes.end()));
          for (const auto& entry : peers_state_->getAllPeers()) {
            const auto excluded = std::ranges::any_of(effect.excluded_peers,
                                                      [&entry](const auto& id) { return id == entry.first.asArray(); });
            if (excluded || entry.second->syncing_) {
              continue;
            }
            const vote_hash_t object_hash(effect.object_hash.data(), vote_hash_t::ConstructFromPointer);
            if ((effect.object_kind == kObjectPbftVote && entry.second->isPbftVoteKnown(object_hash)) ||
                (effect.object_kind == kObjectPillarVote && entry.second->isPillarVoteKnown(object_hash))) {
              continue;
            }
            const auto include_related_block = effect.packet_kind == kPacketPbftVote &&
                                               !effect.related_payload_bytes.empty() &&
                                               !entry.second->isPbftBlockKnown(related_block_hash);
            const auto packet_rlp = wrap_payload(include_related_block);
            if (!sealAndSend(entry.first, packet_type, dev::bytes(packet_rlp.begin(), packet_rlp.end()))) {
              LOG(log_wr_) << "Consensus packet gossip skipped disconnected peer " << entry.first;
              continue;
            }
            if (effect.object_kind == kObjectPbftVote) {
              entry.second->markPbftVoteAsKnown(object_hash);
              if (include_related_block) {
                entry.second->markPbftBlockAsKnown(related_block_hash);
              }
            } else if (effect.object_kind == kObjectPillarVote) {
              entry.second->markPillarVoteAsKnown(object_hash);
            }
          }
        }
      } else {
        throw std::runtime_error("Unsupported native consensus transport effect");
      }
      return {};
    } catch (const std::exception& error) {
      return {false, error.what()};
    }
  }};
}

network::ConsensusPacketRequest RustConsensusTransportPacketHandler::consensusPacketRequest(
    const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer, uint32_t transport_lane,
    bool validate_max_round_step, bool allow_gossip) const {
  const auto status = consensus_status_ ? consensus_status_() : network::ConsensusLiveStatus{};
  return {transport_lane,
          peer->getId().asArray(),
          peer->pbft_chain_size_.load(),
          packet_data.id_,
          packet_data.rlp_.data().toBytes(),
          status.period,
          status.round,
          status.step,
          kConf.network.ddos_protection.vote_accepting_periods,
          kConf.network.ddos_protection.vote_accepting_rounds,
          kConf.network.ddos_protection.vote_accepting_steps,
          validate_max_round_step,
          true,
          true,
          allow_gossip};
}

bool RustConsensusTransportPacketHandler::tryReservePbftSyncRequest() {
  const std::lock_guard lock(sync_request_mutex_);
  const auto now = std::chrono::steady_clock::now();
  if (now - last_pbft_sync_request_ <= kVoteSyncRequestInterval) {
    return false;
  }
  last_pbft_sync_request_ = now;
  return true;
}

bool RustConsensusTransportPacketHandler::tryReserveNextVotesSyncRequest() {
  const std::lock_guard lock(sync_request_mutex_);
  const auto now = std::chrono::steady_clock::now();
  if (now - last_next_votes_sync_request_ <= kVoteSyncRequestInterval) {
    return false;
  }
  last_next_votes_sync_request_ = now;
  return true;
}

}  // namespace taraxa::network::tarcap
