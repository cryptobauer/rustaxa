#include "network/tarcap/packets_handlers/latest/common/ext_syncing_packet_handler.hpp"

#include "network/consensus_query.hpp"
#include "network/tarcap/packets/latest/get_dag_sync_packet.hpp"
#include "network/tarcap/packets/latest/get_next_votes_bundle_packet.hpp"
#include "network/tarcap/packets/latest/get_pbft_sync_packet.hpp"
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#endif
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#else
#include "pbft/pbft_manager.hpp"
#endif

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint32_t kPendingDagTransportLane = 0;

network::ConsensusPeerCandidate toConsensusPeerCandidate(const std::shared_ptr<TaraxaPeer> &peer) {
  network::ConsensusPeerCandidate candidate{};
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

#endif

ExtSyncingPacketHandler::ExtSyncingPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                                 std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
                                                 std::shared_ptr<PbftSyncingState> pbft_syncing_state,
#endif
                                                 net::ConsensusQueryClient pbft_chain,
#ifndef RUSTAXA_ENABLE
                                                 std::shared_ptr<PbftManager> pbft_mgr,
                                                 std::shared_ptr<DagManager> dag_mgr,
                                                 std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY:
                                                                                 // legacy sync handler.
#else
                                                 network::ConsensusLiveStatusProvider consensus_status,
                                                 network::ConsensusNetworkApiShared consensus_network_api,
#endif
                                                 const addr_t &node_addr, const std::string &log_channel_name)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, log_channel_name),
#ifndef RUSTAXA_ENABLE
      pbft_syncing_state_(std::move(pbft_syncing_state)),
#endif
      pbft_chain_(std::move(pbft_chain)),
#ifndef RUSTAXA_ENABLE
      pbft_mgr_(std::move(pbft_mgr)),
      dag_mgr_(std::move(dag_mgr)),
      db_(std::move(db))
#else
      consensus_status_(std::move(consensus_status)),
      rust_consensus_network_api_(std::move(consensus_network_api))
#endif
{
}

ExtSyncingPacketHandler::~ExtSyncingPacketHandler() = default;

void ExtSyncingPacketHandler::requestPendingDagBlocks(std::shared_ptr<TaraxaPeer> peer) {
#ifdef RUSTAXA_ENABLE
  std::vector<network::ConsensusPeerCandidate> candidates;
  if (peer) {
    candidates.push_back(toConsensusPeerCandidate(peer));
  } else {
    for (const auto &peer_entry : peers_state_->getAllPeers()) {
      candidates.push_back(toConsensusPeerCandidate(peer_entry.second));
    }
  }
  if (candidates.empty()) {
    LOG(this->log_nf_) << "requestPendingDagBlocks not possible since no peers are matching conditions";
    return;
  }

  const auto outcome = rust_consensus_network_api_->requestPendingDagBlocks(
      kPendingDagTransportLane, consensus_status_().syncing_period, candidates,
      network::PendingDagBlocksExecutor{[this](const auto &peer_id, const auto &payload, uint64_t /* period */) {
        const auto selected_peer = peers_state_->getPeer(
            dev::p2p::NodeID(peer_id.data(), dev::p2p::NodeID::ConstructFromPointer));
        if (!selected_peer) {
          return false;
        }
        bool expected = false;
        if (!selected_peer->peer_dag_syncing_.compare_exchange_strong(expected, true)) {
          return false;
        }
        LOG(this->log_nf_) << "Request pending blocks from peer " << selected_peer->getId();
        const auto sent = sealAndSend(selected_peer->getId(), SubprotocolPacketType::kGetDagSyncPacket,
                                      dev::bytes(payload.begin(), payload.end()));
        if (!sent) {
          selected_peer->peer_dag_syncing_ = false;
        }
        return sent;
      }});
  if (outcome.queued_effect_count == 0) {
    LOG(this->log_dg_) << "Native pending-DAG request skipped with status " << static_cast<uint32_t>(outcome.status)
                       << ", error " << outcome.error_code;
  }
  return;
#endif
#ifndef RUSTAXA_ENABLE
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
#endif
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
  const auto packet =
      GetNextVotesBundlePacket{.peer_pbft_period = peer_pbft_period, .peer_pbft_round = peer_pbft_round};
  sealAndSend(peer_id, SubprotocolPacketType::kGetNextVotesSyncPacket, encodePacketRlp(packet));
}

#endif

}  // namespace taraxa::network::tarcap
