#include "network/tarcap/packets_handlers/interface/sync_packet_handler.hpp"

#include <chrono>

#include "config/version.hpp"
#include "network/tarcap/packets/latest/get_pbft_sync_packet.hpp"
#include "network/tarcap/packets/latest/status_packet.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkStatusPlanStatusNoEligiblePeer = 2;
constexpr uint8_t kNetworkStatusPlanStatusSyncNotNeeded = 3;
constexpr uint8_t kNetworkPbftSyncStopReasonTransportFailed = 4;

uint64_t monotonicMilliseconds() {
  return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
      .count();
}

}  // namespace
#endif

ISyncPacketHandler::ISyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                       std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
                                       std::shared_ptr<PbftSyncingState> pbft_syncing_state,
#endif
                                       net::ConsensusQueryClient pbft_chain,
#ifndef RUSTAXA_ENABLE
                                       std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<DagManager> dag_mgr,
                                       std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY:
                                                                       // legacy sync handler.
#else
                                       network::ConsensusLiveStatusProvider consensus_status,
                                       network::ConsensusNetworkApiShared consensus_network_api,
#endif
                                       const addr_t& node_addr, const std::string& logs_prefix)
    : ExtSyncingPacketHandler(conf, std::move(peers_state), std::move(packets_stats),
#ifndef RUSTAXA_ENABLE
                              std::move(pbft_syncing_state),
#endif
                              std::move(pbft_chain),
#ifndef RUSTAXA_ENABLE
                              std::move(pbft_mgr), std::move(dag_mgr), std::move(db),
#else
                              std::move(consensus_status), std::move(consensus_network_api),
#endif
                              node_addr, logs_prefix),
      kGenesisHash(kConf.genesis.genesisHash()) {
}

void ISyncPacketHandler::startSyncingPbft() {
#ifndef RUSTAXA_ENABLE
  if (pbft_syncing_state_->isPbftSyncing()) {
    LOG(this->log_dg_) << "startSyncingPbft called but syncing_ already true";
    return;
  }
#endif

#ifdef RUSTAXA_ENABLE
  const auto consensus_status = consensus_status_();
  rustaxa::NetworkPbftSyncStartRequest request{};
  request.start = true;
  request.now_ms = monotonicMilliseconds();
  request.local_pbft_synced_period = consensus_status.syncing_period;
  request.local_pbft_chain_size = net::consensusPbftProgress(pbft_chain_).finalized_period;
  for (const auto& peer_entry : peers_state_->getAllPeers()) {
    rustaxa::NetworkPbftSyncPeerCandidate candidate{};
    candidate.peer_id = peer_entry.first.asArray();
    candidate.pbft_chain_size = peer_entry.second->pbft_chain_size_.load();
    candidate.dag_level = peer_entry.second->dag_level_.load();
    candidate.is_light_node = peer_entry.second->peer_light_node.load();
    candidate.light_node_history = peer_entry.second->peer_light_node_history.load();
    request.candidates.push_back(candidate);
  }

  const auto outcome = rust_consensus_network_api_->api().consensus_network_begin_pbft_sync(request);
  if (!outcome.started) {
    if (outcome.status == kNetworkStatusPlanStatusNoEligiblePeer) {
      LOG(this->log_nf_) << "Restarting syncing PBFT not possible since no connected peers";
    } else if (outcome.status == kNetworkStatusPlanStatusSyncNotNeeded) {
      LOG(this->log_nf_) << "Restarting syncing PBFT not needed since our pbft chain size: "
                         << request.local_pbft_synced_period << "(" << request.local_pbft_chain_size << ")"
                         << " is greater or equal than max node pbft chain size:" << outcome.peer_pbft_chain_size;
    } else {
      LOG(this->log_dg_) << "startSyncingPbft skipped with status " << static_cast<uint32_t>(outcome.status)
                         << ", error " << static_cast<std::string>(outcome.error_code);
    }
    return;
  }

  const auto selected_peer =
      peers_state_->getPeer(dev::p2p::NodeID(outcome.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer));
  if (!selected_peer) {
    LOG(this->log_nf_) << "Restarting syncing PBFT not possible since selected peer is no longer connected";
    rust_consensus_network_api_->stopPbftSync(outcome.generation, outcome.peer_id,
                                              kNetworkPbftSyncStopReasonTransportFailed);
    return;
  }

  // PBFT sync invalidates the transport peer's prior DAG-complete observation.
  // This is peer bookkeeping only; native state owns the subsequent DAG-sync decision.
  if (selected_peer->dagSyncingAllowed()) {
    selected_peer->peer_dag_synced_ = false;
  }

  LOG(this->log_si_) << "Restarting syncing PBFT from peer " << selected_peer->getId().abridged()
                     << ", peer PBFT chain size " << selected_peer->pbft_chain_size_.load()
                     << ", own PBFT chain synced at period " << consensus_status.syncing_period;

  if (outcome.request_period > selected_peer->pbft_chain_size_) {
    LOG(this->log_wr_) << "Unable to start PBFT sync from peer " << selected_peer->getId().abridged()
                       << ", peer chain size " << selected_peer->pbft_chain_size_.load() << ", requested period "
                       << outcome.request_period;
  } else if (syncPeerPbft(outcome.request_period)) {
    return;
  }
  rust_consensus_network_api_->stopPbftSync(outcome.generation, outcome.peer_id,
                                            kNetworkPbftSyncStopReasonTransportFailed);
  return;
#else

  std::shared_ptr<TaraxaPeer> peer = peers_state_->getMaxChainPeer(pbft_mgr_);
  if (!peer) {
    LOG(this->log_nf_) << "Restarting syncing PBFT not possible since no connected peers";
    return;
  }

  auto pbft_sync_period = pbft_mgr_->pbftSyncingPeriod();
  if (peer->pbft_chain_size_ > pbft_sync_period) {
    auto peer_id = peer->getId().abridged();
    auto peer_pbft_chain_size = peer->pbft_chain_size_.load();
    if (!pbft_syncing_state_->setPbftSyncing(true, pbft_sync_period, std::move(peer))) {
      LOG(this->log_dg_) << "startSyncingPbft called but syncing_ already true";
      return;
    }
    LOG(this->log_si_) << "Restarting syncing PBFT from peer " << peer_id << ", peer PBFT chain size "
                       << peer_pbft_chain_size << ", own PBFT chain synced at period " << pbft_sync_period;

    if (syncPeerPbft(pbft_sync_period + 1)) {
      // Disable snapshots only if are syncing from scratch
      if (pbft_syncing_state_->isDeepPbftSyncing()) {
#ifdef RUSTAXA_ENABLE
        pbft_mgr_->setPbftSyncSnapshotCreationEnabled(false);
#else
        db_->disableSnapshots();  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy snapshot lifecycle.
#endif
      }
    } else {
      pbft_syncing_state_->setPbftSyncing(false);
    }
  } else {
    LOG(this->log_nf_) << "Restarting syncing PBFT not needed since our pbft chain size: " << pbft_sync_period << "("
                       << net::consensusPbftProgress(pbft_chain_).finalized_period << ")"
                       << " is greater or equal than max node pbft chain size:" << peer->pbft_chain_size_;
#ifdef RUSTAXA_ENABLE
    pbft_mgr_->setPbftSyncSnapshotCreationEnabled(true);
#else
    db_->enableSnapshots();  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy snapshot lifecycle.
#endif
  }
#endif
}

bool ISyncPacketHandler::syncPeerPbft(PbftPeriod request_period) {
#ifdef RUSTAXA_ENABLE
  const auto sync = rust_consensus_network_api_->pbftSyncStatus(monotonicMilliseconds());
  if (!sync.active || !sync.has_peer) {
    LOG(this->log_er_) << "Unable to send GetPbftSyncPacket. No syncing peer set.";
    return false;
  }
  const auto syncing_peer =
      peers_state_->getPeer(dev::p2p::NodeID(sync.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer));
#else
  const auto syncing_peer = pbft_syncing_state_->syncingPeer();
#endif
  if (!syncing_peer) {
    LOG(this->log_er_) << "Unable to send GetPbftSyncPacket. No syncing peer set.";
    return false;
  }

  if (request_period > syncing_peer->pbft_chain_size_) {
    LOG(this->log_wr_) << "Invalid syncPeerPbft argument. Node " << syncing_peer->getId() << " chain size "
                       << syncing_peer->pbft_chain_size_ << ", requested period " << request_period;
    return false;
  }

  LOG(this->log_nf_) << "Send GetPbftSyncPacket with period " << request_period << " to node " << syncing_peer->getId();
  return this->sealAndSend(syncing_peer->getId(), SubprotocolPacketType::kGetPbftSyncPacket,
                           encodePacketRlp(GetPbftSyncPacket{request_period}));
}

void ISyncPacketHandler::sendStatusToPeers() {
  auto host = peers_state_->host_.lock();
  if (!host) {
    LOG(log_er_) << "Unavailable host during checkLiveness";
    return;
  }

#ifdef RUSTAXA_ENABLE
  const auto now_ms = monotonicMilliseconds();
  const auto sync = rust_consensus_network_api_->pbftSyncStatus(now_ms);
  if (sync.active) {
    const auto outcome = rust_consensus_network_api_->tickPbftSync(now_ms, sync.generation);
    if (outcome.restart_sync) {
      LOG(log_nf_) << "Restart PBFT/DAG syncing after native inactivity timeout.";
      startSyncingPbft();
    }
  }
#endif

  for (auto const& peer : peers_state_->getAllPeers()) {
    sendStatus(peer.first, false);
  }
}

bool ISyncPacketHandler::sendStatus(const dev::p2p::NodeID& node_id, bool initial) {
  bool success = false;
  std::string status_packet_type = initial ? "initial" : "standard";

  LOG(log_dg_) << "Sending " << status_packet_type << " status message to " << node_id << ", protocol version "
               << TARAXA_NET_VERSION << ", network id " << kConf.genesis.chain_id << ", genesis " << kGenesisHash
               << ", node version " << TARAXA_VERSION;

  const auto dag_max_level =
#ifdef RUSTAXA_ENABLE
      (*pbft_chain_)->consensus_query_live_dag_status().max_level;
#else
      dag_mgr_->getMaxLevel();
#endif
  auto pbft_chain_size = net::consensusPbftProgress(pbft_chain_).finalized_period;
  const auto pbft_round =
#ifdef RUSTAXA_ENABLE
      consensus_status_().round;
#else
      pbft_mgr_->getPbftRound();
#endif

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkStatusEgressRequest request{};
  request.initial = initial;
  request.local_chain_id = kConf.genesis.chain_id;
  request.genesis_hash = kGenesisHash.asArray();
  request.node_major_version = TARAXA_MAJOR_VERSION;
  request.node_minor_version = TARAXA_MINOR_VERSION;
  request.node_patch_version = TARAXA_PATCH_VERSION;
  request.is_light_node = kConf.is_light_node;
  request.light_node_history = kConf.light_node_history;
  request.local_pbft_chain_size = pbft_chain_size;
  request.local_pbft_round = pbft_round;
  request.local_dag_level = dag_max_level;
  const auto outcome = rust_consensus_network_api_->api().consensus_network_status_egress(request);
  if (outcome.include_initial_data) {
    success = sealAndSend(
        node_id, SubprotocolPacketType::kStatusPacket,
        encodePacketRlp(StatusPacket(
            outcome.peer_pbft_chain_size, outcome.peer_pbft_round, outcome.peer_dag_level, outcome.peer_syncing,
            StatusPacket::InitialData{outcome.chain_id,
                                      blk_hash_t(outcome.genesis_hash.data(), blk_hash_t::ConstructFromPointer),
                                      outcome.node_major_version, outcome.node_minor_version,
                                      outcome.node_patch_version, outcome.is_light_node, outcome.light_node_history})));
  } else {
    success = sealAndSend(node_id, SubprotocolPacketType::kStatusPacket,
                          encodePacketRlp(StatusPacket(outcome.peer_pbft_chain_size, outcome.peer_pbft_round,
                                                       outcome.peer_dag_level, outcome.peer_syncing)));
  }

  return success;
#endif

#ifndef RUSTAXA_ENABLE
  if (initial) {
    success = sealAndSend(
        node_id, SubprotocolPacketType::kStatusPacket,
        encodePacketRlp(StatusPacket(
            pbft_chain_size, pbft_round, dag_max_level, pbft_syncing_state_->isPbftSyncing(),
            StatusPacket::InitialData{kConf.genesis.chain_id, kGenesisHash, TARAXA_MAJOR_VERSION, TARAXA_MINOR_VERSION,
                                      TARAXA_PATCH_VERSION, kConf.is_light_node, kConf.light_node_history})));
  } else {
    success = sealAndSend(node_id, SubprotocolPacketType::kStatusPacket,
                          encodePacketRlp(StatusPacket(pbft_chain_size, pbft_round, dag_max_level,
                                                       pbft_syncing_state_->isDeepPbftSyncing())));
  }

  return success;
#endif
}

}  // namespace taraxa::network::tarcap
