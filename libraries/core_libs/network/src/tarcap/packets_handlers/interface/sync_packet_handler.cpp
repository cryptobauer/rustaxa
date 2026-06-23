#include "network/tarcap/packets_handlers/interface/sync_packet_handler.hpp"

#include "config/version.hpp"
#include "network/tarcap/packets/latest/get_pbft_sync_packet.hpp"
#include "network/tarcap/packets/latest/status_packet.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkStatusPlanStatusNoEligiblePeer = 2;
constexpr uint8_t kNetworkStatusPlanStatusSyncNotNeeded = 3;

}  // namespace
#endif

ISyncPacketHandler::ISyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                       std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                       std::shared_ptr<PbftSyncingState> pbft_syncing_state,
                                       std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<PbftManager> pbft_mgr,
                                       std::shared_ptr<DagManager> dag_mgr,
#ifndef RUSTAXA_ENABLE
                                       std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY:
                                                                       // legacy sync handler.
#endif
                                       const addr_t& node_addr, const std::string& logs_prefix)
    : ExtSyncingPacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_syncing_state),
                              std::move(pbft_chain), std::move(pbft_mgr), std::move(dag_mgr),
#ifndef RUSTAXA_ENABLE
                              std::move(db),
#endif
                              node_addr, logs_prefix),
      kGenesisHash(kConf.genesis.genesisHash()) {
}

void ISyncPacketHandler::startSyncingPbft() {
  if (pbft_syncing_state_->isPbftSyncing()) {
    LOG(this->log_dg_) << "startSyncingPbft called but syncing_ already true";
    return;
  }

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkPbftSyncStartFacts facts{};
  facts.local_pbft_syncing = false;
  facts.local_pbft_synced_period = pbft_mgr_->pbftSyncingPeriod();
  facts.local_pbft_chain_size = pbft_chain_->getPbftChainSize();
  for (const auto& peer_entry : peers_state_->getAllPeers()) {
    rustaxa::NetworkPbftSyncPeerCandidate candidate{};
    candidate.peer_id = peer_entry.first.asArray();
    candidate.pbft_chain_size = peer_entry.second->pbft_chain_size_.load();
    candidate.dag_level = peer_entry.second->dag_level_.load();
    candidate.is_light_node = peer_entry.second->peer_light_node.load();
    candidate.light_node_history = peer_entry.second->peer_light_node_history.load();
    facts.candidates.push_back(candidate);
  }

  const auto sync_start_plan = rust_consensus_network_api_->api->consensus_network_plan_pbft_sync_start(facts);
  if (!sync_start_plan.start_sync) {
    if (sync_start_plan.enable_snapshot_creation) {
      pbft_mgr_->setPbftSyncSnapshotCreationEnabled(true);
    }
    if (sync_start_plan.status == kNetworkStatusPlanStatusNoEligiblePeer) {
      LOG(this->log_nf_) << "Restarting syncing PBFT not possible since no connected peers";
    } else if (sync_start_plan.status == kNetworkStatusPlanStatusSyncNotNeeded) {
      LOG(this->log_nf_) << "Restarting syncing PBFT not needed since our pbft chain size: "
                         << facts.local_pbft_synced_period << "(" << facts.local_pbft_chain_size << ")"
                         << " is greater or equal than max node pbft chain size:"
                         << sync_start_plan.peer_pbft_chain_size;
    } else {
      LOG(this->log_dg_) << "startSyncingPbft skipped with status " << static_cast<uint32_t>(sync_start_plan.status)
                         << ", error " << static_cast<std::string>(sync_start_plan.error_code);
    }
    return;
  }

  auto peer = peers_state_->getPeer(dev::p2p::NodeID(sync_start_plan.peer_id));
  if (!peer) {
    LOG(this->log_nf_) << "Restarting syncing PBFT not possible since selected peer is no longer connected";
    return;
  }

  auto peer_id = peer->getId().abridged();
  auto peer_pbft_chain_size = peer->pbft_chain_size_.load();
  if (!pbft_syncing_state_->setPbftSyncing(true, facts.local_pbft_synced_period, std::move(peer))) {
    LOG(this->log_dg_) << "startSyncingPbft called but syncing_ already true";
    return;
  }
  LOG(this->log_si_) << "Restarting syncing PBFT from peer " << peer_id << ", peer PBFT chain size "
                     << peer_pbft_chain_size << ", own PBFT chain synced at period " << facts.local_pbft_synced_period;

  if (syncPeerPbft(sync_start_plan.request_period)) {
    if (pbft_syncing_state_->isDeepPbftSyncing()) {
      pbft_mgr_->setPbftSyncSnapshotCreationEnabled(false);
    }
  } else {
    pbft_syncing_state_->setPbftSyncing(false);
  }
  return;
#endif

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
                       << pbft_chain_->getPbftChainSize() << ")"
                       << " is greater or equal than max node pbft chain size:" << peer->pbft_chain_size_;
#ifdef RUSTAXA_ENABLE
    pbft_mgr_->setPbftSyncSnapshotCreationEnabled(true);
#else
    db_->enableSnapshots();  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy snapshot lifecycle.
#endif
  }
}

bool ISyncPacketHandler::syncPeerPbft(PbftPeriod request_period) {
  const auto syncing_peer = pbft_syncing_state_->syncingPeer();
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

  auto dag_max_level = dag_mgr_->getMaxLevel();
  auto pbft_chain_size = pbft_chain_->getPbftChainSize();
  const auto pbft_round = pbft_mgr_->getPbftRound();

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkStatusEgressFacts facts{};
  facts.initial = initial;
  facts.local_chain_id = kConf.genesis.chain_id;
  facts.genesis_hash = kGenesisHash.asArray();
  facts.node_major_version = TARAXA_MAJOR_VERSION;
  facts.node_minor_version = TARAXA_MINOR_VERSION;
  facts.node_patch_version = TARAXA_PATCH_VERSION;
  facts.is_light_node = kConf.is_light_node;
  facts.light_node_history = kConf.light_node_history;
  facts.local_pbft_chain_size = pbft_chain_size;
  facts.local_pbft_round = pbft_round;
  facts.local_dag_level = dag_max_level;
  facts.pbft_syncing = pbft_syncing_state_->isPbftSyncing();
  facts.deep_pbft_syncing = pbft_syncing_state_->isDeepPbftSyncing();

  const auto status_plan = rust_consensus_network_api_->api->consensus_network_plan_status_egress(facts);
  if (status_plan.include_initial_data) {
    success = sealAndSend(
        node_id, SubprotocolPacketType::kStatusPacket,
        encodePacketRlp(StatusPacket(
            status_plan.peer_pbft_chain_size, status_plan.peer_pbft_round, status_plan.peer_dag_level,
            status_plan.peer_syncing,
            StatusPacket::InitialData{
                status_plan.chain_id, blk_hash_t(status_plan.genesis_hash.data(), blk_hash_t::ConstructFromPointer),
                status_plan.node_major_version, status_plan.node_minor_version, status_plan.node_patch_version,
                status_plan.is_light_node, status_plan.light_node_history})));
  } else {
    success = sealAndSend(node_id, SubprotocolPacketType::kStatusPacket,
                          encodePacketRlp(StatusPacket(status_plan.peer_pbft_chain_size, status_plan.peer_pbft_round,
                                                       status_plan.peer_dag_level, status_plan.peer_syncing)));
  }

  return success;
#endif

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
}

}  // namespace taraxa::network::tarcap
