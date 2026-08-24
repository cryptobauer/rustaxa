#include "network/tarcap/packets_handlers/rust/transaction_packet_handler.hpp"

#include <algorithm>
#include <stdexcept>

#include "network/tarcap/packets_handlers/latest/common/exceptions.hpp"
#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {

constexpr uint8_t kTransactionPacketMalformed = 24;
constexpr uint8_t kTransactionPacketTooLarge = 25;
constexpr uint8_t kTransactionRejected = 26;

dev::p2p::NodeID toNodeId(const std::array<uint8_t, 64>& peer_id) {
  return dev::p2p::NodeID(peer_id.data(), dev::p2p::NodeID::ConstructFromPointer);
}

}  // namespace

RustTransactionPacketHandler::RustTransactionPacketHandler(const FullNodeConfig& conf,
                                                           std::shared_ptr<PeersState> peers_state,
                                                           std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                           network::ConsensusNetworkApiShared consensus_network_api,
                                                           TarcapVersion transport_lane, const addr_t& node_addr,
                                                           const std::string& logs_prefix)
    : ITransactionPacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                                logs_prefix + "TRANSACTION_PH"),
      consensus_network_api_(std::move(consensus_network_api)),
      transport_lane_(transport_lane) {
  if (!consensus_network_api_) {
    throw std::invalid_argument("Rust transaction packet handler requires consensus network API");
  }
}

void RustTransactionPacketHandler::process(const threadpool::PacketData& packet_data,
                                           const std::shared_ptr<TaraxaPeer>& peer) {
  const auto packet_rlp = packet_data.rlp_.data().toBytes();
  const auto outcome = consensus_network_api_->ingestTransactionPacket(
      transport_lane_, peer->getId().asArray(), packet_data.id_, packet_rlp, false, kConf,
      network::TransactionPacketExecutor{
          [this](const auto& peer_id, const auto& transaction_hash) {
            if (const auto target = peers_state_->getPeer(toNodeId(peer_id)); target) {
              target->markTransactionAsKnown(trx_hash_t(transaction_hash.data(), trx_hash_t::ConstructFromPointer));
            }
          },
          [this](const auto& payload, const auto& excluded) { return gossipPacket(payload, excluded); }});

  if (outcome.status == kTransactionPacketMalformed || outcome.status == kTransactionPacketTooLarge ||
      outcome.status == kTransactionRejected) {
    throw MaliciousPeerException("Native transaction packet admission rejected peer payload: " + outcome.error_code);
  }
  LOG(log_dg_) << "Native transaction packet admitted " << outcome.admitted_transaction_count << " transactions from "
               << peer->getId().abridged();
}

bool RustTransactionPacketHandler::gossipPacket(const std::vector<uint8_t>& packet_rlp,
                                                const std::vector<std::array<uint8_t, 64>>& excluded_peers) {
  bool success = true;
  for (const auto& peer_entry : peers_state_->getAllPeers()) {
    const auto& peer = peer_entry.second;
    if (peer->syncing_) {
      continue;
    }
    const auto peer_id = peer->getId().asArray();
    if (std::find(excluded_peers.begin(), excluded_peers.end(), peer_id) != excluded_peers.end()) {
      continue;
    }
    success = sealAndSend(peer->getId(), SubprotocolPacketType::kTransactionPacket,
                          dev::bytes(packet_rlp.begin(), packet_rlp.end())) &&
              success;
  }
  return success;
}

void RustTransactionPacketHandler::periodicSendTransactions() {
  const auto candidate_hashes = consensus_network_api_->transactionGossipCandidateHashes();
  std::vector<network::TransactionGossipPeer> candidates;
  for (const auto& peer_entry : peers_state_->getAllPeers()) {
    const auto& peer = peer_entry.second;
    if (peer->syncing_) {
      continue;
    }
    network::TransactionGossipPeer candidate{};
    candidate.peer_id = peer->getId().asArray();
    for (const auto& hash : candidate_hashes) {
      const trx_hash_t typed_hash(hash.data(), trx_hash_t::ConstructFromPointer);
      if (peer->isTransactionKnown(typed_hash)) {
        candidate.known_hashes.push_back(hash);
      }
    }
    candidates.push_back(std::move(candidate));
  }
  consensus_network_api_->planTransactionGossip(
      transport_lane_, candidates,
      network::TransactionGossipExecutor{
          [this](const auto& peer_id, const auto& payload) {
            return sealAndSend(toNodeId(peer_id), SubprotocolPacketType::kTransactionPacket,
                               dev::bytes(payload.begin(), payload.end()));
          },
          [this](const auto& peer_id, const auto& transaction_hash) {
            if (const auto peer = peers_state_->getPeer(toNodeId(peer_id)); peer) {
              peer->markTransactionAsKnown(trx_hash_t(transaction_hash.data(), trx_hash_t::ConstructFromPointer));
            }
          }});
}

}  // namespace taraxa::network::tarcap
