#include "network/tarcap/packets_handlers/rust/status_packet_handler.hpp"

#include <chrono>

#include "network/tarcap/packets_handlers/latest/common/exceptions.hpp"

namespace taraxa::network::tarcap {
namespace {

uint64_t monotonicMilliseconds() {
  return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
      .count();
}

}  // namespace

RustStatusPacketHandler::RustStatusPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                                 std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                 net::ConsensusQueryClient consensus_query,
                                                 network::ConsensusLiveStatusProvider consensus_status,
                                                 network::ConsensusNetworkApiShared consensus_network_api,
                                                 const addr_t& node_addr, const std::string& logs_prefix)
    : RustConsensusTransportPacketHandler(conf, std::move(peers_state), std::move(packets_stats),
                                          std::move(consensus_query), std::move(consensus_status),
                                          std::move(consensus_network_api), node_addr, logs_prefix + "STATUS_PH") {}

void RustStatusPacketHandler::process(const threadpool::PacketData& packet_data,
                                      const std::shared_ptr<TaraxaPeer>& peer) {
  const auto local = consensus_status_();
  network::StatusPacketRequest request{};
  request.peer_id = peer->getId().asArray();
  request.packet_rlp = packet_data.rlp_.data().toBytes();
  request.source_peer_ready = peers_state_->getPeer(peer->getId()) != nullptr;
  request.local_pbft_synced_period = local.syncing_period;
  request.local_pbft_period = local.period;
  request.local_pbft_round = local.round;
  request.peer_dag_synced = peer->peer_dag_synced_.load();
  const auto report = rust_consensus_network_api_->ingestStatusPacket(request);

  if (report.malicious) {
    throw PacketProcessingException("Native status admission rejected malformed packet: " + report.error_code,
                                    dev::p2p::DisconnectReason::BadProtocol);
  }
  if (!report.accept_peer) {
    if (report.disconnect_peer) {
      disconnect(peer->getId(), dev::p2p::UserReason);
    }
    return;
  }

  peer->dag_level_ = report.peer_dag_level;
  peer->pbft_chain_size_ = report.peer_pbft_chain_size;
  peer->pbft_period_ = report.peer_pbft_period;
  peer->pbft_round_ = report.peer_pbft_round;
  peer->syncing_ = report.peer_syncing;
  if (report.initial && report.peer_is_light_node) {
    peer->peer_light_node = true;
    peer->peer_light_node_history = report.peer_light_node_history;
  }
  if (report.initial) {
    peers_state_->setPeerAsReadyToSendMessages(peer->getId(), peer);
  }

  if (report.request_pbft_sync) {
    startSyncingPbft();
  } else if (report.request_pending_dag_blocks) {
    requestPendingDagBlocks(peer);
  }
  if (report.request_next_votes) {
    if (!sealAndSend(peer->getId(), SubprotocolPacketType::kGetNextVotesSyncPacket,
                     dev::bytes(report.next_votes_request_rlp.begin(), report.next_votes_request_rlp.end()))) {
      LOG(log_wr_) << "Unable to send native status-selected next-votes request";
    }
  }
}

bool RustStatusPacketHandler::sendStatus(const dev::p2p::NodeID& node_id, bool initial) {
  const auto status = consensus_status_();
  network::StatusPacketBuildRequest request{};
  request.initial = initial;
  request.local_pbft_chain_size = net::consensusPbftProgress(pbft_chain_).finalized_period;
  request.local_pbft_round = status.round;
  request.local_dag_level = (*pbft_chain_)->consensus_query_live_dag_status().max_level;
  const auto report = rust_consensus_network_api_->buildStatusPacket(request);
  if (report.status != 0 || report.packet_rlp.empty()) {
    LOG(log_wr_) << "Native status packet construction failed: " << report.error_code;
    return false;
  }
  return sealAndSend(node_id, SubprotocolPacketType::kStatusPacket,
                     dev::bytes(report.packet_rlp.begin(), report.packet_rlp.end()));
}

void RustStatusPacketHandler::sendStatusToPeers() {
  const auto now_ms = monotonicMilliseconds();
  const auto sync = rust_consensus_network_api_->pbftSyncStatus(now_ms);
  if (sync.active) {
    const auto outcome = rust_consensus_network_api_->tickPbftSync(now_ms, sync.generation);
    if (outcome.restart_sync) {
      startSyncingPbft();
    }
  }
  for (const auto& [peer_id, peer] : peers_state_->getAllPeers()) {
    static_cast<void>(peer);
    sendStatus(peer_id, false);
  }
}

}  // namespace taraxa::network::tarcap
