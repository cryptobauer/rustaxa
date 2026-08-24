#include "network/tarcap/packets_handlers/rust/dag_block_packet_handler.hpp"

#include <stdexcept>

#include "network/tarcap/packets/latest/dag_block_packet.hpp"
#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {

constexpr uint8_t kDagRejectionNone = 0;
constexpr uint8_t kDagRejectionIgnore = 1;
constexpr uint8_t kDagRejectionRequestSync = 2;
constexpr uint8_t kDagRejectionRequestPending = 3;
constexpr uint8_t kDagRejectionDisconnect = 4;
constexpr uint8_t kDagRejectionMalicious = 5;

dev::p2p::NodeID toNodeId(const std::array<uint8_t, 64>& peer_id) {
  return dev::p2p::NodeID(peer_id.data(), dev::p2p::NodeID::ConstructFromPointer);
}

}  // namespace

RustDagBlockPacketHandler::RustDagBlockPacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, std::shared_ptr<PbftSyncingState> pbft_syncing_state,
    net::ConsensusQueryClient consensus_query, network::ConsensusLiveStatusProvider consensus_status,
    network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane, const addr_t& node_addr,
    const std::string& logs_prefix)
    : IDagBlockPacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_syncing_state),
                             std::move(consensus_query), std::move(consensus_status), consensus_network_api, node_addr,
                             logs_prefix + "DAG_BLOCK_PH"),
      consensus_network_api_(std::move(consensus_network_api)),
      transport_lane_(transport_lane) {}

void RustDagBlockPacketHandler::process(const threadpool::PacketData& packet_data,
                                        const std::shared_ptr<TaraxaPeer>& peer) {
  const auto packet_rlp = packet_data.rlp_.data().toBytes();
  const auto transaction_status = (*pbft_chain_)->consensus_query_live_transaction_status();
  const auto outcome = consensus_network_api_->ingestDagBlockPacket(
      transport_lane_, peer->getId().asArray(), packet_data.id_, packet_rlp, true,
      network::DagBlockPeerFacts{peer->peer_dag_synced_.load(), peer->dagSyncingAllowed(),
                                 transaction_status.transactions_dropped, peer->peer_dag_syncing_.load(),
                                 consensus_status_().syncing_period != 0},
      kConf,
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
          [this](const auto& block_hash) {
            const blk_hash_t typed_hash(block_hash.data(), blk_hash_t::ConstructFromPointer);
            std::vector<network::DagGossipPeer> candidates;
            for (const auto& peer_entry : peers_state_->getAllPeers()) {
              const auto& target = peer_entry.second;
              candidates.push_back(network::DagGossipPeer{target->getId().asArray(), target->syncing_.load(),
                                                          target->isDagBlockKnown(typed_hash)});
            }
            return candidates;
          },
          [this](const auto& peer_id, const auto& payload) {
            const auto target = peers_state_->getPeer(toNodeId(peer_id));
            if (!target) {
              return false;
            }
            std::unique_lock lock(target->mutex_for_sending_dag_blocks_);
            return sealAndSend(target->getId(), SubprotocolPacketType::kDagBlockPacket,
                               dev::bytes(payload.begin(), payload.end()));
          }});
  if (outcome.admission && outcome.admission->block_level > peer->dag_level_) {
    peer->dag_level_ = outcome.admission->block_level;
  }
  switch (outcome.rejection_action) {
    case kDagRejectionNone:
    case kDagRejectionIgnore:
      return;
    case kDagRejectionRequestSync:
      peer->peer_dag_synced_ = false;
      requestPendingDagBlocks(peer);
      return;
    case kDagRejectionRequestPending:
      requestPendingDagBlocks(peer);
      return;
    case kDagRejectionDisconnect:
      disconnect(peer->getId(), dev::p2p::UserReason);
      return;
    case kDagRejectionMalicious:
      throw MaliciousPeerException("Native DAG-block peer policy rejected payload: " + outcome.error_code);
    default:
      throw std::runtime_error("Native DAG-block peer policy returned an unknown rejection action");
  }
}

void RustDagBlockPacketHandler::sendBlockWithTransactions(const std::shared_ptr<TaraxaPeer>& peer,
                                                          const std::shared_ptr<DagBlock>& block,
                                                          SharedTransactions&& transactions) {
  std::unique_lock lock(peer->mutex_for_sending_dag_blocks_);
  const DagBlockPacket packet{.transactions = std::move(transactions), .dag_block = block};
  if (sealAndSend(peer->getId(), SubprotocolPacketType::kDagBlockPacket, encodePacketRlp(packet))) {
    peer->markDagBlockAsKnown(block->getHash());
  }
}

}  // namespace taraxa::network::tarcap
