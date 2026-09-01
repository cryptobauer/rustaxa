#pragma once

#include "common/thread_pool.hpp"
#include "network/consensus_network_api.hpp"
#include "network/tarcap/packets_handlers/rust/consensus_transport_packet_handler.hpp"

namespace taraxa::network::tarcap {

/**
 * Rust-mode PBFT sync packet transport/executor facade.
 *
 * Native consensus inspects the original packet bytes and owns deterministic
 * chain, queue-link, certificate, pillar-schedule, and DAG-order decisions.
 * This facade retains peer state, slashing-transaction execution, pacing
 * timers, packet sends, and sync lifecycle publication.
 */
class RustPbftSyncPacketHandler final : public RustConsensusTransportPacketHandler {
 public:
  RustPbftSyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                            std::shared_ptr<TimePeriodPacketsStats> packets_stats, net::ConsensusQueryClient pbft_chain,
                            network::ConsensusLiveStatusProvider consensus_status,
                            SharedConsensusApplication consensus_application,
                            network::ConsensusNetworkApiShared consensus_network_api, const addr_t& node_addr,
                            const std::string& logs_prefix = "");
  ~RustPbftSyncPacketHandler() override;

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kPbftSyncPacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;
  void pbftSyncComplete(uint64_t generation, std::array<uint8_t, 64> peer_id);
  void delayedPbftSync(uint32_t counter, uint64_t generation, std::array<uint8_t, 64> peer_id);
  void stopPbftSync(uint64_t generation, const std::array<uint8_t, 64>& peer_id, uint8_t reason) const;

  static constexpr uint32_t kDelayedPbftSyncDelayMs = 10;

  bool executeSlashingTransaction(const network::PbftSyncSlashingTransaction& effect) const;

  SharedConsensusApplication consensus_application_;
  network::ConsensusNetworkApiShared consensus_network_api_;
  util::ThreadPool periodic_events_tp_;
};

}  // namespace taraxa::network::tarcap
