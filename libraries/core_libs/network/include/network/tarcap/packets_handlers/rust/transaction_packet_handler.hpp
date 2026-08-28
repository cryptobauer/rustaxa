#pragma once

#include "network/consensus_network_api.hpp"
#include "network/tarcap/packets_handlers/interface/transaction_packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/**
 * Rust-mode transaction packet transport adapter.
 *
 * Complete canonical packet bytes cross once into the application-root transaction pipeline. Rust owns decoding,
 * limits, verification, admission, and gossip selection; this adapter executes only peer-known and packet-send leaves.
 */
class RustTransactionPacketHandler final : public ITransactionPacketHandler {
 public:
  RustTransactionPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                               std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                               network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
                               const addr_t& node_addr, const std::string& logs_prefix = "");

  void periodicSendTransactions() override;

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kTransactionPacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

  network::ConsensusNetworkApiShared consensus_network_api_;
  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
