#include "network/tarcap/packets_handlers/rust/get_dag_sync_packet_handler.hpp"

#include <chrono>
#include <stdexcept>

#include "network/tarcap/packets_handlers/latest/common/exceptions.hpp"
#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {

constexpr uint8_t kDagSyncRequestMalformed = 27;
constexpr uint8_t kDagSyncRequestThrottled = 28;

}  // namespace

RustGetDagSyncPacketHandler::RustGetDagSyncPacketHandler(const FullNodeConfig& conf,
                                                         std::shared_ptr<PeersState> peers_state,
                                                         std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                         network::ConsensusNetworkApiShared consensus_network_api,
                                                         TarcapVersion transport_lane, const addr_t& node_addr,
                                                         const std::string& logs_prefix)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, logs_prefix + "GET_DAG_SYNC_PH"),
      consensus_network_api_(std::move(consensus_network_api)),
      transport_lane_(transport_lane) {
  if (!consensus_network_api_) {
    throw std::invalid_argument("Rust get-DAG-sync handler requires consensus network API");
  }
}

void RustGetDagSyncPacketHandler::process(const threadpool::PacketData& packet_data,
                                          const std::shared_ptr<TaraxaPeer>& peer) {
  const auto request_rlp = packet_data.rlp_.data().toBytes();
  const auto outcome = consensus_network_api_->serveGetDagSyncRequest(
      transport_lane_, peer->getId().asArray(), packet_data.id_, peer->requestDagSyncingAllowed(), request_rlp,
      network::GetDagSyncExecutor{
          [this, peer](const auto& response_rlp, uint64_t request_period, uint64_t response_period) {
            std::unique_lock lock(peer->mutex_for_sending_dag_blocks_);
            if (request_period == response_period) {
              peer->syncing_ = false;
              peer->peer_requested_dag_syncing_ = true;
              peer->peer_requested_dag_syncing_time_ =
                  std::chrono::duration_cast<std::chrono::seconds>(std::chrono::system_clock::now().time_since_epoch())
                      .count();
            }
            return sealAndSend(peer->getId(), SubprotocolPacketType::kDagSyncPacket,
                               dev::bytes(response_rlp.begin(), response_rlp.end()));
          }});
  if (outcome.status == kDagSyncRequestMalformed || outcome.status == kDagSyncRequestThrottled) {
    throw MaliciousPeerException("Native get-DAG-sync admission rejected peer request: " + outcome.error_code);
  }
}

}  // namespace taraxa::network::tarcap
