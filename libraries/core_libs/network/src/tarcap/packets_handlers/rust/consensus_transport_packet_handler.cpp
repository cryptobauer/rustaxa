#include "network/tarcap/packets_handlers/rust/consensus_transport_packet_handler.hpp"

#include <stdexcept>

#include "network/tarcap/packets/latest/get_pbft_sync_packet.hpp"
#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {

constexpr uint32_t kPendingDagTransportLane = 0;
constexpr uint8_t kEffectSendPacket = 0;
constexpr uint8_t kEffectMarkPeerKnown = 2;
constexpr uint8_t kEffectRequestSync = 3;
constexpr uint8_t kEffectReportPeer = 4;
constexpr uint8_t kEffectDisconnectPeer = 5;
constexpr uint8_t kObjectPbftVote = 0;
constexpr uint8_t kObjectPbftBlock = 1;
constexpr uint8_t kObjectTransaction = 2;
constexpr uint8_t kObjectDagBlock = 3;
constexpr uint8_t kObjectPillarVote = 5;
constexpr uint8_t kSyncPbftChain = 0;
constexpr uint8_t kSyncPbftNextVotes = 1;
constexpr uint32_t kPacketDagBlock = 5;
constexpr uint8_t kNetworkStatusPlanStatusNoEligiblePeer = 2;
constexpr uint8_t kNetworkStatusPlanStatusSyncNotNeeded = 3;
constexpr uint8_t kNetworkPbftSyncStopReasonTransportFailed = 4;

uint64_t monotonicMilliseconds() {
  return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
      .count();
}

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

void RustConsensusTransportPacketHandler::startSyncingPbft() {
  const auto consensus_status = consensus_status_();
  network::PbftSyncStartRequest request{};
  request.start = true;
  request.now_ms = monotonicMilliseconds();
  request.local_pbft_synced_period = consensus_status.syncing_period;
  request.local_pbft_chain_size = net::consensusPbftProgress(pbft_chain_).finalized_period;
  for (const auto& [peer_id, peer] : peers_state_->getAllPeers()) {
    auto candidate = toPeerCandidate(peer);
    candidate.peer_id = peer_id.asArray();
    request.candidates.push_back(std::move(candidate));
  }

  const auto outcome = rust_consensus_network_api_->beginPbftSync(request);
  if (!outcome.started) {
    if (outcome.status == kNetworkStatusPlanStatusNoEligiblePeer) {
      LOG(log_nf_) << "Restarting syncing PBFT not possible since no connected peers";
    } else if (outcome.status == kNetworkStatusPlanStatusSyncNotNeeded) {
      LOG(log_nf_) << "Restarting syncing PBFT not needed since our pbft chain size: "
                   << request.local_pbft_synced_period << "(" << request.local_pbft_chain_size
                   << ") is greater or equal than max node pbft chain size:" << outcome.peer_pbft_chain_size;
    } else {
      LOG(log_dg_) << "startSyncingPbft skipped with status " << static_cast<uint32_t>(outcome.status) << ", error "
                   << outcome.error_code;
    }
    return;
  }

  const auto selected_peer =
      peers_state_->getPeer(dev::p2p::NodeID(outcome.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer));
  if (!selected_peer) {
    rust_consensus_network_api_->stopPbftSync(outcome.generation, outcome.peer_id,
                                              kNetworkPbftSyncStopReasonTransportFailed);
    return;
  }
  if (selected_peer->dagSyncingAllowed()) {
    selected_peer->peer_dag_synced_ = false;
  }
  if (outcome.request_period <= selected_peer->pbft_chain_size_ && syncPeerPbft(outcome.request_period)) {
    return;
  }
  rust_consensus_network_api_->stopPbftSync(outcome.generation, outcome.peer_id,
                                            kNetworkPbftSyncStopReasonTransportFailed);
}

bool RustConsensusTransportPacketHandler::syncPeerPbft(PbftPeriod request_period) {
  const auto sync = rust_consensus_network_api_->pbftSyncStatus(monotonicMilliseconds());
  if (!sync.active || !sync.has_peer) {
    LOG(log_er_) << "Unable to send GetPbftSyncPacket. No syncing peer set.";
    return false;
  }
  const auto syncing_peer =
      peers_state_->getPeer(dev::p2p::NodeID(sync.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer));
  if (!syncing_peer || request_period > syncing_peer->pbft_chain_size_) {
    LOG(log_wr_) << "Unable to send GetPbftSyncPacket for period " << request_period;
    return false;
  }
  return sealAndSend(syncing_peer->getId(), SubprotocolPacketType::kGetPbftSyncPacket,
                     encodePacketRlp(GetPbftSyncPacket{request_period}));
}

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

network::ConsensusTransportExecutor RustConsensusTransportPacketHandler::consensusTransportExecutor() {
  return {[this](const network::ConsensusTransportEffect& effect) -> network::ConsensusTransportExecutionResult {
    try {
      const auto peer_id = dev::p2p::NodeID(effect.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer);
      if (effect.kind == kEffectRequestSync && effect.sync_kind == kSyncPbftChain) {
        const auto peer = peers_state_->getPeer(peer_id);
        if (!peer) {
          throw std::runtime_error("PBFT sync target peer disconnected");
        }
        const auto consensus_status = consensus_status_();
        network::PbftSyncStartRequest request{};
        request.start = true;
        request.now_ms = monotonicMilliseconds();
        request.local_pbft_synced_period = consensus_status.syncing_period;
        request.local_pbft_chain_size = net::consensusPbftProgress(pbft_chain_).finalized_period;
        request.candidates.push_back(toPeerCandidate(peer));
        const auto outcome = rust_consensus_network_api_->beginPbftSync(request);
        if (!outcome.started || outcome.peer_id != effect.peer_id) {
          throw std::runtime_error("PBFT sync request did not open its exact native generation: " + outcome.error_code);
        }
        const auto expected_payload = encodePacketRlp(GetPbftSyncPacket{outcome.request_period});
        if (effect.sync_start != outcome.request_period || effect.payload_bytes != expected_payload) {
          rust_consensus_network_api_->stopPbftSync(outcome.generation, outcome.peer_id,
                                                    kNetworkPbftSyncStopReasonTransportFailed);
          throw std::runtime_error("PBFT sync effect disagrees with its native generation");
        }
        if (peer->dagSyncingAllowed()) {
          peer->peer_dag_synced_ = false;
        }
        if (effect.payload_bytes.empty() ||
            !sealAndSend(peer_id, SubprotocolPacketType::kGetPbftSyncPacket,
                         dev::bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
          rust_consensus_network_api_->stopPbftSync(outcome.generation, outcome.peer_id,
                                                    kNetworkPbftSyncStopReasonTransportFailed);
          throw std::runtime_error("PBFT sync request transport failed");
        }
      } else if (effect.kind == kEffectRequestSync && effect.sync_kind == kSyncPbftNextVotes) {
        if (effect.payload_bytes.empty() ||
            !sealAndSend(peer_id, SubprotocolPacketType::kGetNextVotesSyncPacket,
                         dev::bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
          throw std::runtime_error("PBFT next-votes sync request transport failed");
        }
      } else if (effect.kind == kEffectMarkPeerKnown) {
        if (const auto peer = peers_state_->getPeer(peer_id); peer) {
          if (effect.object_kind == kObjectPbftVote) {
            peer->markPbftVoteAsKnown(vote_hash_t(effect.object_hash.data(), vote_hash_t::ConstructFromPointer));
          } else if (effect.object_kind == kObjectPbftBlock) {
            peer->markPbftBlockAsKnown(blk_hash_t(effect.object_hash.data(), blk_hash_t::ConstructFromPointer));
          } else if (effect.object_kind == kObjectTransaction) {
            peer->markTransactionAsKnown(trx_hash_t(effect.object_hash.data(), trx_hash_t::ConstructFromPointer));
          } else if (effect.object_kind == kObjectDagBlock) {
            peer->markDagBlockAsKnown(blk_hash_t(effect.object_hash.data(), blk_hash_t::ConstructFromPointer));
          } else if (effect.object_kind == kObjectPillarVote) {
            peer->markPillarVoteAsKnown(vote_hash_t(effect.object_hash.data(), vote_hash_t::ConstructFromPointer));
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
      } else if (effect.kind == kEffectSendPacket) {
        const auto packet_type = static_cast<SubprotocolPacketType>(effect.packet_kind);
        const auto send = [&] {
          return sealAndSend(peer_id, packet_type,
                             dev::bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()));
        };
        if (effect.packet_kind == kPacketDagBlock) {
          const auto peer = peers_state_->getPeer(peer_id);
          if (!peer) {
            throw std::runtime_error("Consensus packet target peer disconnected");
          }
          std::unique_lock lock(peer->mutex_for_sending_dag_blocks_);
          if (!send()) {
            throw std::runtime_error("Consensus DAG packet send failed");
          }
        } else if (!send()) {
          throw std::runtime_error("Consensus packet send failed");
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

std::vector<network::ConsensusEgressPeerSnapshot> RustConsensusTransportPacketHandler::consensusEgressPeerSnapshots(
    const std::vector<network::ConsensusEgressProbe>& probes) const {
  const auto peers = peers_state_->getAllPeers();
  std::vector<network::ConsensusEgressPeerSnapshot> snapshots;
  snapshots.reserve(peers.size());
  for (const auto& [peer_id, peer] : peers) {
    network::ConsensusEgressPeerSnapshot snapshot{};
    snapshot.peer_id = peer_id.asArray();
    snapshot.syncing = peer->syncing_.load();
    snapshot.known_probe_ids.reserve(probes.size());
    for (const auto& probe : probes) {
      bool known = false;
      if (probe.object_kind == kObjectPbftVote) {
        known = peer->isPbftVoteKnown(vote_hash_t(probe.object_hash.data(), vote_hash_t::ConstructFromPointer));
      } else if (probe.object_kind == kObjectPbftBlock) {
        known = peer->isPbftBlockKnown(blk_hash_t(probe.object_hash.data(), blk_hash_t::ConstructFromPointer));
      } else if (probe.object_kind == kObjectTransaction) {
        known = peer->isTransactionKnown(trx_hash_t(probe.object_hash.data(), trx_hash_t::ConstructFromPointer));
      } else if (probe.object_kind == kObjectDagBlock) {
        known = peer->isDagBlockKnown(blk_hash_t(probe.object_hash.data(), blk_hash_t::ConstructFromPointer));
      } else if (probe.object_kind == kObjectPillarVote) {
        known = peer->isPillarVoteKnown(vote_hash_t(probe.object_hash.data(), vote_hash_t::ConstructFromPointer));
      } else {
        throw std::runtime_error("Native consensus egress requested an unsupported peer-known probe");
      }
      if (known) {
        snapshot.known_probe_ids.push_back(probe.probe_id);
      }
    }
    snapshots.push_back(std::move(snapshot));
  }
  return snapshots;
}

network::ConsensusPacketOutcome RustConsensusTransportPacketHandler::routeConsensusEgress(
    network::ConsensusEgressRequest request) {
  return rust_consensus_network_api_->routeConsensusEgress(
      request, [this](const auto& probes) { return consensusEgressPeerSnapshots(probes); },
      consensusTransportExecutor());
}

network::ConsensusPacketRequest RustConsensusTransportPacketHandler::consensusPacketRequest(
    const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer, uint32_t transport_lane,
    bool validate_max_round_step) const {
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
          true};
}

}  // namespace taraxa::network::tarcap
