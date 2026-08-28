#include "network/tarcap/packets_handlers/rust/transaction_packet_handler.hpp"

#include <stdexcept>

#include "network/tarcap/packets_handlers/latest/common/exceptions.hpp"
#include "network/tarcap/taraxa_peer.hpp"

namespace taraxa::network::tarcap {
namespace {

constexpr uint8_t kTransactionEgressFamily = 4;
constexpr uint8_t kTransactionPacketMalformed = 24;
constexpr uint8_t kTransactionPacketTooLarge = 25;
constexpr uint8_t kTransactionRejected = 26;

}  // namespace

RustTransactionPacketHandler::RustTransactionPacketHandler(const FullNodeConfig& conf,
                                                           std::shared_ptr<PeersState> peers_state,
                                                           std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                           network::ConsensusNetworkApiShared consensus_network_api,
                                                           TarcapVersion transport_lane, const addr_t& node_addr,
                                                           const std::string& logs_prefix)
    : ITransactionPacketHandler(conf, std::move(peers_state), std::move(packets_stats), consensus_network_api,
                                node_addr, logs_prefix + "TRANSACTION_PH"),
      consensus_network_api_(std::move(consensus_network_api)),
      transport_lane_(transport_lane) {
  if (!consensus_network_api_) {
    throw std::invalid_argument("Rust transaction packet handler requires consensus network API");
  }
}

void RustTransactionPacketHandler::process(const threadpool::PacketData& packet_data,
                                           const std::shared_ptr<TaraxaPeer>& peer) {
  const auto packet_rlp = packet_data.rlp_.data().toBytes();
  const auto outcome =
      consensus_network_api_->ingestTransactionPacket(transport_lane_, peer->getId().asArray(), packet_data.id_,
                                                      packet_rlp, kConf, consensusTransportExecutor());

  if (outcome.status == kTransactionPacketMalformed || outcome.status == kTransactionPacketTooLarge ||
      outcome.status == kTransactionRejected) {
    throw MaliciousPeerException("Native transaction packet admission rejected peer payload: " + outcome.error_code);
  }
  LOG(log_dg_) << "Native transaction packet admitted " << outcome.admitted_transaction_count << " transactions from "
               << peer->getId().abridged();
}

void RustTransactionPacketHandler::periodicSendTransactions() {
  network::ConsensusEgressRequest request{};
  request.family = kTransactionEgressFamily;
  request.transport_lane = transport_lane_;
  routeConsensusEgress(std::move(request));
}

}  // namespace taraxa::network::tarcap
