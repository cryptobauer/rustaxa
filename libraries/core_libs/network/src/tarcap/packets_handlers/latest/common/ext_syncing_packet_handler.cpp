#include "network/tarcap/packets_handlers/latest/common/ext_syncing_packet_handler.hpp"

#include "network/tarcap/packets/latest/get_dag_sync_packet.hpp"
#include "network/tarcap/packets/latest/get_pbft_sync_packet.hpp"
#include "network/tarcap/packets/latest/get_next_votes_bundle_packet.hpp"
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#include "pbft/pbft_chain.hpp"
#include "pbft/pbft_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkStatusPlanStatusNoEligiblePeer = 2;
constexpr uint8_t kNetworkStatusPlanStatusDagAlreadySynced = 4;
constexpr uint8_t kNetworkStatusPlanStatusDagPeriodMismatch = 5;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

rustaxa::NetworkPbftSyncPeerCandidate toNetworkSyncPeerCandidate(const std::shared_ptr<TaraxaPeer> &peer) {
  rustaxa::NetworkPbftSyncPeerCandidate candidate{};
  candidate.peer_id = peer->getId().asArray();
  candidate.pbft_chain_size = peer->pbft_chain_size_.load();
  candidate.dag_level = peer->dag_level_.load();
  candidate.is_light_node = peer->peer_light_node.load();
  candidate.light_node_history = peer->peer_light_node_history.load();
  candidate.peer_dag_synced = peer->peer_dag_synced_.load();
  candidate.peer_dag_syncing = peer->peer_dag_syncing_.load();
  candidate.dag_sync_allowed = peer->dagSyncingAllowed();
  return candidate;
}

}  // namespace

ExtSyncingPacketHandler::RustConsensusNetworkApiHolder::RustConsensusNetworkApiHolder()
    : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}
#endif

ExtSyncingPacketHandler::ExtSyncingPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                                 std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                 std::shared_ptr<PbftSyncingState> pbft_syncing_state,
                                                 std::shared_ptr<PbftChain> pbft_chain,
                                                 std::shared_ptr<PbftManager> pbft_mgr,
                                                 std::shared_ptr<DagManager> dag_mgr,
#ifndef RUSTAXA_ENABLE
                                                 std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY:
                                                                                 // legacy sync handler.
#endif
                                                 const addr_t &node_addr, const std::string &log_channel_name)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, log_channel_name),
      pbft_syncing_state_(std::move(pbft_syncing_state)),
      pbft_chain_(std::move(pbft_chain)),
      pbft_mgr_(std::move(pbft_mgr)),
      dag_mgr_(std::move(dag_mgr))
#ifndef RUSTAXA_ENABLE
      ,
      db_(std::move(db))
#endif
{
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

ExtSyncingPacketHandler::~ExtSyncingPacketHandler() = default;

void ExtSyncingPacketHandler::requestPendingDagBlocks(std::shared_ptr<TaraxaPeer> peer) {
#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkPendingDagBlocksRequestFacts facts{};
  facts.local_pbft_syncing_period = pbft_mgr_->pbftSyncingPeriod();
  if (peer) {
    facts.has_explicit_peer = true;
    facts.explicit_peer = toNetworkSyncPeerCandidate(peer);
  } else {
    for (const auto &peer_entry : peers_state_->getAllPeers()) {
      facts.candidates.push_back(toNetworkSyncPeerCandidate(peer_entry.second));
    }
  }

  const auto dag_request_plan =
      rust_consensus_network_api_->api->consensus_network_plan_pending_dag_blocks_request(facts);
  if (!dag_request_plan.request_pending_dag_blocks) {
    if (dag_request_plan.status == kNetworkStatusPlanStatusNoEligiblePeer) {
      LOG(this->log_nf_) << "requestPendingDagBlocks not possible since no peers are matching conditions";
    } else if (dag_request_plan.status == kNetworkStatusPlanStatusDagAlreadySynced) {
      LOG(this->log_nf_) << "requestPendingDagBlocks not possible since already requested for peer";
    } else if (dag_request_plan.status == kNetworkStatusPlanStatusDagPeriodMismatch) {
      LOG(this->log_nf_) << "requestPendingDagBlocks not possible since PBFT periods are not matching";
    } else {
      LOG(this->log_dg_) << "requestPendingDagBlocks skipped with status "
                         << static_cast<uint32_t>(dag_request_plan.status) << ", error "
                         << static_cast<std::string>(dag_request_plan.error_code);
    }
    return;
  }

  auto selected_peer =
      peer ? peer : peers_state_->getPeer(
                         dev::p2p::NodeID(dag_request_plan.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer));
  if (!selected_peer) {
    LOG(this->log_nf_) << "requestPendingDagBlocks not possible since no connected peers";
    return;
  }

  // This prevents parallel requests. Rust owns deterministic request planning,
  // while the live tarcap peer keeps the atomic executor-side reservation.
  bool expected = false;
  if (!selected_peer->peer_dag_syncing_.compare_exchange_strong(expected, true)) {
    LOG(this->log_nf_) << "requestPendingDagBlocks not possible since already requesting for peer";
    return;
  }

  std::vector<blk_hash_t> known_non_finalized_blocks;
  auto [_, blocks] = dag_mgr_->getNonFinalizedBlocks();
  for (auto &level_blocks : blocks) {
    for (auto &block : level_blocks.second) {
      known_non_finalized_blocks.emplace_back(block);
    }
  }

  LOG(this->log_nf_) << "Request pending blocks from peer " << selected_peer->getId();
  requestDagBlocks(selected_peer->getId(), std::move(known_non_finalized_blocks), dag_request_plan.request_period);
#endif

  if (!peer) {
    peer = peers_state_->getMaxChainPeer(pbft_mgr_, [](const std::shared_ptr<TaraxaPeer> &peer) {
      if (peer->peer_dag_synced_ || !peer->dagSyncingAllowed()) {
        return false;
      }
      return true;
    });
    if (!peer) {
      LOG(this->log_nf_) << "requestPendingDagBlocks not possible since no peers are matching conditions";
      return;
    }
  }

  if (!peer) {
    LOG(this->log_nf_) << "requestPendingDagBlocks not possible since no connected peers";
    return;
  }

  // This prevents ddos requesting dag blocks. We can only request this one time from one peer.
  if (peer->peer_dag_synced_) {
    LOG(this->log_nf_) << "requestPendingDagBlocks not possible since already requested for peer";
    return;
  }

  // Only request dag blocks if periods are matching
  auto pbft_sync_period = pbft_mgr_->pbftSyncingPeriod();
  if (pbft_sync_period == peer->pbft_chain_size_) {
    // This prevents parallel requests
    if (bool b = false; !peer->peer_dag_syncing_.compare_exchange_strong(b, !b)) {
      LOG(this->log_nf_) << "requestPendingDagBlocks not possible since already requesting for peer";
      return;
    }
    LOG(this->log_nf_) << "Request pending blocks from peer " << peer->getId();
    std::vector<blk_hash_t> known_non_finalized_blocks;
    auto [period, blocks] = dag_mgr_->getNonFinalizedBlocks();
    for (auto &level_blocks : blocks) {
      for (auto &block : level_blocks.second) {
        known_non_finalized_blocks.emplace_back(block);
      }
    }

    requestDagBlocks(peer->getId(), std::move(known_non_finalized_blocks), period);
  }
}

void ExtSyncingPacketHandler::requestDagBlocks(const dev::p2p::NodeID &_nodeID, std::vector<blk_hash_t> &&blocks,
                                               PbftPeriod period) {
  this->sealAndSend(_nodeID, SubprotocolPacketType::kGetDagSyncPacket,
                    encodePacketRlp(GetDagSyncPacket{period, std::move(blocks)}));
}

#ifdef RUSTAXA_ENABLE
void ExtSyncingPacketHandler::requestPbftNextVotesAtPeriodRound(const dev::p2p::NodeID &peer_id,
                                                              PbftPeriod peer_pbft_period,
                                                              PbftRound peer_pbft_round) {
  LOG(log_dg_) << "Sending GetNextVotesSyncPacket with period:" << peer_pbft_period << ", round:" << peer_pbft_round;
  const auto packet = GetNextVotesBundlePacket{.peer_pbft_period = peer_pbft_period, .peer_pbft_round = peer_pbft_round};
  sealAndSend(peer_id, SubprotocolPacketType::kGetNextVotesSyncPacket, encodePacketRlp(packet));
}

#endif

}  // namespace taraxa::network::tarcap
