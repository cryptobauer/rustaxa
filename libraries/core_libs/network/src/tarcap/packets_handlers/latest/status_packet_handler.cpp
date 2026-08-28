#include "network/tarcap/packets_handlers/latest/status_packet_handler.hpp"

#include "config/version.hpp"
#include "network/consensus_query.hpp"
#include "network/tarcap/packets/latest/status_packet.hpp"
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets/latest/get_next_votes_bundle_packet.hpp"
#include "pbft/pbft_manager.hpp"
#endif
#include "vote_manager/vote_manager.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkStatusPlanStatusChainIdMismatch = 6;
constexpr uint8_t kNetworkStatusPlanStatusGenesisMismatch = 7;
constexpr uint8_t kNetworkStatusPlanStatusLightNodeHistoryUnavailable = 8;

}  // namespace
#endif

StatusPacketHandler::StatusPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                         std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
                                         std::shared_ptr<PbftSyncingState> pbft_syncing_state,
#endif
                                         net::ConsensusQueryClient pbft_chain,
#ifndef RUSTAXA_ENABLE
                                         std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<DagManager> dag_mgr,
                                         std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY:
                                                                         // legacy status handler.
#else
                                         network::ConsensusLiveStatusProvider consensus_status,
                                         network::ConsensusNetworkApiShared consensus_network_api,
#endif
                                         h256 genesis_hash, const addr_t& node_addr, const std::string& logs_prefix)
    : ISyncPacketHandler(conf, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
                         std::move(pbft_syncing_state),
#endif
                         std::move(pbft_chain),
#ifndef RUSTAXA_ENABLE
                         std::move(pbft_mgr), std::move(dag_mgr), std::move(db),
#else
                         std::move(consensus_status), std::move(consensus_network_api),
#endif
                         node_addr, logs_prefix + "STATUS_PH"),
      kGenesisHash(genesis_hash) {
}

void StatusPacketHandler::process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<StatusPacket>(packet_data.rlp_);

  // Important !!! Use only "selected_peer" and not "peer" in this function as "peer" might be nullptr
  auto selected_peer = peer;
#ifdef RUSTAXA_ENABLE
  // Keep every decision made for this packet on one application-root epoch.
  const auto consensus_status = consensus_status_();
#endif
  const auto pbft_synced_period =
#ifdef RUSTAXA_ENABLE
      consensus_status.syncing_period;
#else
      pbft_mgr_->pbftSyncingPeriod();
#endif

  // Initial status packet
  if (packet.initial_data.has_value()) {
    if (!selected_peer) {
      selected_peer = peers_state_->getPendingPeer(peer->getId());
      if (!selected_peer) {
        LOG(log_wr_) << "Peer " << peer->getId().abridged()
                     << " missing in both peers and pending peers map - will be disconnected.";
        disconnect(peer->getId(), dev::p2p::UserReason);
        return;
      }
    }

#ifdef RUSTAXA_ENABLE
    network::InitialStatusRequest initial_status_request{};
    initial_status_request.local_chain_id = kConf.genesis.chain_id;
    initial_status_request.peer_chain_id = packet.initial_data->peer_chain_id;
    initial_status_request.expected_genesis_hash = kGenesisHash.asArray();
    initial_status_request.peer_genesis_hash = packet.initial_data->genesis_hash.asArray();
    initial_status_request.local_pbft_synced_period = pbft_synced_period;
    initial_status_request.peer_pbft_chain_size = packet.peer_pbft_chain_size;
    initial_status_request.peer_is_light_node = packet.initial_data->is_light_node;
    initial_status_request.peer_light_node_history = packet.initial_data->node_history;
    const auto initial_status_plan = rust_consensus_network_api_->admitInitialStatus(initial_status_request);
    if (!initial_status_plan.accept_peer) {
      if (initial_status_plan.status == kNetworkStatusPlanStatusChainIdMismatch) {
        LOG((peers_state_->getPeersCount()) ? log_nf_ : log_er_)
            << "Incorrect network id " << packet.initial_data->peer_chain_id << ", host " << peer->getId().abridged()
            << " will be disconnected";
      } else if (initial_status_plan.status == kNetworkStatusPlanStatusGenesisMismatch) {
        LOG((peers_state_->getPeersCount()) ? log_nf_ : log_wr_)
            << "Incorrect genesis hash " << packet.initial_data->genesis_hash << ", host " << peer->getId().abridged()
            << " will be disconnected";
      } else if (initial_status_plan.status == kNetworkStatusPlanStatusLightNodeHistoryUnavailable) {
        selected_peer->peer_light_node = true;
        selected_peer->peer_light_node_history = packet.initial_data->node_history;
        LOG((peers_state_->getPeersCount()) ? log_nf_ : log_er_)
            << "Light node " << peer->getId().abridged() << " would not be able to serve our syncing request. "
            << "Current synced period " << pbft_synced_period << ", peer synced period " << packet.peer_pbft_chain_size
            << ", peer light node history " << packet.initial_data->node_history << ". Peer will be disconnected";
      } else {
        LOG(log_wr_) << "Initial status rejected with status " << static_cast<uint32_t>(initial_status_plan.status)
                     << ", error " << static_cast<std::string>(initial_status_plan.error_code) << ". Host "
                     << peer->getId().abridged() << " will be disconnected";
      }
      if (initial_status_plan.disconnect_peer) {
        disconnect(peer->getId(), dev::p2p::UserReason);
      }
      return;
    }

    if (packet.initial_data->is_light_node) {
      selected_peer->peer_light_node = true;
      selected_peer->peer_light_node_history = packet.initial_data->node_history;
    }
#else
    if (packet.initial_data->peer_chain_id != kConf.genesis.chain_id) {
      LOG((peers_state_->getPeersCount()) ? log_nf_ : log_er_)
          << "Incorrect network id " << packet.initial_data->peer_chain_id << ", host " << peer->getId().abridged()
          << " will be disconnected";
      disconnect(peer->getId(), dev::p2p::UserReason);
      return;
    }

    if (packet.initial_data->genesis_hash != kGenesisHash) {
      LOG((peers_state_->getPeersCount()) ? log_nf_ : log_wr_)
          << "Incorrect genesis hash " << packet.initial_data->genesis_hash << ", host " << peer->getId().abridged()
          << " will be disconnected";
      disconnect(peer->getId(), dev::p2p::UserReason);
      return;
    }

    // If this is a light node and it cannot serve our sync request disconnect from it
    if (packet.initial_data->is_light_node) {
      selected_peer->peer_light_node = true;
      selected_peer->peer_light_node_history = packet.initial_data->node_history;
      if (pbft_synced_period + packet.initial_data->node_history < packet.peer_pbft_chain_size) {
        LOG((peers_state_->getPeersCount()) ? log_nf_ : log_er_)
            << "Light node " << peer->getId().abridged() << " would not be able to serve our syncing request. "
            << "Current synced period " << pbft_synced_period << ", peer synced period " << packet.peer_pbft_chain_size
            << ", peer light node history " << packet.initial_data->node_history << ". Peer will be disconnected";
        disconnect(peer->getId(), dev::p2p::UserReason);
        return;
      }
    }
#endif

    selected_peer->dag_level_ = packet.peer_dag_level;
    selected_peer->pbft_chain_size_ = packet.peer_pbft_chain_size;
    selected_peer->syncing_ = packet.peer_syncing;
    selected_peer->pbft_period_ = packet.peer_pbft_chain_size + 1;
    selected_peer->pbft_round_ = packet.peer_pbft_round;

    peers_state_->setPeerAsReadyToSendMessages(peer->getId(), selected_peer);

    LOG(log_dg_) << "Received initial status message from " << peer->getId() << ", network id "
                 << packet.initial_data->peer_chain_id << ", peer DAG max level " << selected_peer->dag_level_
                 << ", genesis " << packet.initial_data->genesis_hash << ", peer pbft chain size "
                 << selected_peer->pbft_chain_size_ << ", peer syncing " << std::boolalpha << selected_peer->syncing_
                 << ", peer pbft period " << selected_peer->pbft_period_ << ", peer pbft round "
                 << selected_peer->pbft_round_ << ", node major version" << packet.initial_data->node_major_version
                 << ", node minor version" << packet.initial_data->node_minor_version << ", node patch version"
                 << packet.initial_data->node_patch_version;

  } else {  // Standard status packet
    if (!selected_peer) {
      LOG(log_er_) << "Received standard status packet from " << peer->getId().abridged()
                   << ", without previously received initial status packet. Will be disconnected";
      disconnect(peer->getId(), dev::p2p::UserReason);
      return;
    }

    selected_peer->dag_level_ = packet.peer_dag_level;
    selected_peer->pbft_chain_size_ = packet.peer_pbft_chain_size;
    selected_peer->pbft_period_ = selected_peer->pbft_chain_size_ + 1;
    selected_peer->syncing_ = packet.peer_syncing;
    selected_peer->pbft_round_ = packet.peer_pbft_round;

#ifdef RUSTAXA_ENABLE
    network::StatusFollowupRequest followup_request{};
    followup_request.peer_id = selected_peer->getId().asArray();
    followup_request.local_pbft_synced_period = pbft_synced_period;
    followup_request.local_pbft_period = consensus_status.period;
    followup_request.local_pbft_round = consensus_status.round;
    followup_request.peer_pbft_chain_size = selected_peer->pbft_chain_size_.load();
    followup_request.peer_pbft_period = selected_peer->pbft_period_;
    followup_request.peer_pbft_round = selected_peer->pbft_round_;
    followup_request.peer_dag_synced = selected_peer->peer_dag_synced_;
    const auto followup = rust_consensus_network_api_->planStatusFollowup(followup_request);
    if (followup.request_pbft_sync) {
      LOG(log_nf_) << "Restart PBFT chain syncing. Own synced PBFT at period " << pbft_synced_period
                   << ", peer PBFT chain size " << selected_peer->pbft_chain_size_;
      startSyncingPbft();
    } else if (followup.request_pending_dag_blocks) {
      requestPendingDagBlocks(selected_peer);
    }

    if (followup.request_next_votes) {
      requestPbftNextVotesAtPeriodRound(selected_peer->getId(), followup.next_votes_period, followup.next_votes_round);
    }
#else
    // TODO: Address malicious status
    if (!pbft_syncing_state_->isPbftSyncing()) {
      if (pbft_synced_period < selected_peer->pbft_chain_size_) {
        LOG(log_nf_) << "Restart PBFT chain syncing. Own synced PBFT at period " << pbft_synced_period
                     << ", peer PBFT chain size " << selected_peer->pbft_chain_size_;
        if (pbft_synced_period + 1 < selected_peer->pbft_chain_size_) {
          startSyncingPbft();
        } else {
          // If we are behind by only one block wait for two status messages before syncing because nodes are not always
          // in perfect sync
          if (selected_peer->last_status_pbft_chain_size_ == selected_peer->pbft_chain_size_) {
            startSyncingPbft();
          }
        }
      } else if (pbft_synced_period == selected_peer->pbft_chain_size_ && !selected_peer->peer_dag_synced_) {
        // if not syncing and the peer period is matching our period request any pending dag blocks
        requestPendingDagBlocks(selected_peer);
      }

      const auto [pbft_current_round, pbft_current_period] = pbft_mgr_->getPbftRoundAndPeriod();
      if (pbft_current_period == selected_peer->pbft_period_ && pbft_current_round < selected_peer->pbft_round_) {
        // TODO: this functionality is implemented in requestPbftNextVotesAtPeriodRound in ExtVotesPacketHandler
        const auto get_next_votes_packet =
            GetNextVotesBundlePacket{.peer_pbft_period = pbft_current_period, .peer_pbft_round = pbft_current_round};
        sealAndSend(selected_peer->getId(), SubprotocolPacketType::kGetNextVotesSyncPacket,
                    encodePacketRlp(get_next_votes_packet));
      }
    }
#endif
#ifndef RUSTAXA_ENABLE
    selected_peer->last_status_pbft_chain_size_ = selected_peer->pbft_chain_size_.load();
#endif

    LOG(log_dg_) << "Received status message from " << peer->getId() << ", peer DAG max level "
                 << selected_peer->dag_level_ << ", peer pbft chain size " << selected_peer->pbft_chain_size_
                 << ", peer syncing " << std::boolalpha << selected_peer->syncing_ << ", peer pbft round "
                 << selected_peer->pbft_round_;
  }
}

}  // namespace taraxa::network::tarcap
