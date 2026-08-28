#include "network/tarcap/packets_handlers/rust/dag_sync_packet_handler.hpp"

#include <stdexcept>

#include "network/tarcap/packets_handlers/latest/common/exceptions.hpp"
#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {

constexpr uint8_t kDagPacketMalformed = 29;
constexpr uint8_t kDagBlockRejected = 30;
constexpr uint8_t kDagSyncPeriodAhead = 31;
constexpr uint8_t kDagSyncPeriodBehind = 32;

dev::p2p::NodeID toNodeId(const std::array<uint8_t, 64>& peer_id) {
  return dev::p2p::NodeID(peer_id.data(), dev::p2p::NodeID::ConstructFromPointer);
}

}  // namespace

RustDagSyncPacketHandler::RustDagSyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                                   std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                   net::ConsensusQueryClient consensus_query,
                                                   network::ConsensusLiveStatusProvider consensus_status,
                                                   network::ConsensusNetworkApiShared consensus_network_api,
                                                   TarcapVersion transport_lane, const addr_t& node_addr,
                                                   const std::string& logs_prefix)
    : ISyncPacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(consensus_query),
                         std::move(consensus_status), consensus_network_api, node_addr,
                         logs_prefix + "DAG_SYNC_PH"),
      consensus_network_api_(std::move(consensus_network_api)),
      transport_lane_(transport_lane) {}

void RustDagSyncPacketHandler::process(const threadpool::PacketData& packet_data,
                                       const std::shared_ptr<TaraxaPeer>& peer) {
  const auto packet_rlp = packet_data.rlp_.data().toBytes();
  const auto outcome = consensus_network_api_->ingestDagSyncPacket(
      transport_lane_, peer->getId().asArray(), packet_data.id_, packet_rlp, kConf,
      network::DagPacketExecutor{
          [this](const auto& peer_id, const auto& hash) {
            if (const auto target = peers_state_->getPeer(toNodeId(peer_id)); target) {
              target->markTransactionAsKnown(trx_hash_t(hash.data(), trx_hash_t::ConstructFromPointer));
            }
          },
          [this](const auto& peer_id, const auto& hash) {
            if (const auto target = peers_state_->getPeer(toNodeId(peer_id)); target) {
              target->markDagBlockAsKnown(blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer));
            }
          },
          {},
          {}});
  if (outcome.status == kDagSyncPeriodAhead) {
    if (peer->pbft_chain_size_ < outcome.response_period) {
      peer->pbft_chain_size_ = outcome.response_period;
    }
    peer->peer_dag_syncing_ = false;
    startSyncingPbft();
    return;
  }
  if (outcome.status == kDagPacketMalformed || outcome.status == kDagBlockRejected ||
      outcome.status == kDagSyncPeriodBehind) {
    throw MaliciousPeerException("Native DAG-sync admission rejected peer payload: " + outcome.error_code);
  }
  for (const auto& block : outcome.blocks) {
    if (block.block_level > peer->dag_level_) {
      peer->dag_level_ = block.block_level;
    }
  }
  peer->peer_dag_synced_ = true;
  peer->peer_dag_synced_time_ =
      std::chrono::duration_cast<std::chrono::seconds>(std::chrono::system_clock::now().time_since_epoch()).count();
  peer->peer_dag_syncing_ = false;
}

}  // namespace taraxa::network::tarcap
