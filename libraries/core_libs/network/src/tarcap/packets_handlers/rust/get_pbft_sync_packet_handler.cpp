#include "network/tarcap/packets_handlers/rust/get_pbft_sync_packet_handler.hpp"

#include <stdexcept>

namespace taraxa::network::tarcap {

RustGetPbftSyncPacketHandler::RustGetPbftSyncPacketHandler(const FullNodeConfig& conf,
                                                           std::shared_ptr<PeersState> peers_state,
                                                           std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                           network::ConsensusNetworkApiShared consensus_network_api,
                                                           TarcapVersion transport_lane, const addr_t& node_addr,
                                                           const std::string& logs_prefix)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                    logs_prefix + "GET_PBFT_SYNC_PH"),
      consensus_network_api_(std::move(consensus_network_api)),
      transport_lane_(transport_lane) {
  if (!consensus_network_api_) {
    throw std::invalid_argument("Rust consensus network API must be provided");
  }
}

RustGetPbftSyncPacketHandler::~RustGetPbftSyncPacketHandler() = default;

void RustGetPbftSyncPacketHandler::process(const threadpool::PacketData& packet_data,
                                           const std::shared_ptr<TaraxaPeer>& peer) {
  const auto request_rlp = packet_data.rlp_.data().toBytes();
  const auto outcome = consensus_network_api_->servePbftSyncRequest(
      static_cast<uint32_t>(transport_lane_), peer->getId().asArray(), request_rlp, packet_data.id_,
      network::PbftSyncRequestExecutor{
          .send_packet =
              [this, &peer](uint32_t packet_kind, const std::vector<uint8_t>& payload) {
                return sealAndSend(peer->getId(), static_cast<SubprotocolPacketType>(packet_kind),
                                   dev::bytes(payload.begin(), payload.end()));
              },
          .clear_peer_syncing = [&peer] { peer->syncing_ = false; },
          .report_peer =
              [this, &peer](uint8_t reason) {
                peers_state_->set_peer_malicious(peer->getId());
                LOG(log_wr_) << "Network API reported malicious PBFT sync requester " << peer->getId()
                             << " with reason: " << static_cast<uint32_t>(reason);
              },
          .disconnect_peer = [this, &peer] { disconnect(peer->getId(), dev::p2p::UserReason); },
      });

  if (outcome.status != 0 && outcome.queued_effect_count == 0) {
    LOG(log_dg_) << "Native PBFT sync request produced no network work: " << outcome.error_code;
  }
}

}  // namespace taraxa::network::tarcap
