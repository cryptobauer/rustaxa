#pragma once

#include <memory>

#include "network/tarcap/packets/latest/dag_sync_packet.hpp"
#include "network/tarcap/packets_handlers/interface/sync_packet_handler.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa {
class TransactionManager;
}  // namespace taraxa

namespace taraxa::network::tarcap {

class DagSyncPacketHandler : public ISyncPacketHandler {
 public:
  DagSyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                       std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                       std::shared_ptr<PbftSyncingState> pbft_syncing_state, std::shared_ptr<PbftChain> pbft_chain,
                       std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<DagManager> dag_mgr,
                       std::shared_ptr<TransactionManager> trx_mgr,
#ifndef RUSTAXA_ENABLE
                       std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy DAG sync handler.
#endif
                       const addr_t& node_addr, const std::string& logs_prefix = "");
  ~DagSyncPacketHandler() override;

  // Packet type that is processed by this handler
  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kDagSyncPacket;

 private:
  virtual void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkIngressDecision queueDagSyncBlockAdmissionRequestEffects(
      const rustaxa::NetworkDagBlockAdmissionRequestEffects& effects);
  void executeDagSyncBlockAdmissionEffect(std::shared_ptr<DagBlock>& block, const std::shared_ptr<TaraxaPeer>& peer,
                                          const std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>>& trxs);
#endif

 protected:
  std::shared_ptr<TransactionManager> trx_mgr_{nullptr};
#ifdef RUSTAXA_ENABLE
  struct RustConsensusNetworkApiHolder;
  std::unique_ptr<RustConsensusNetworkApiHolder> rust_consensus_network_api_;
#endif
};

}  // namespace taraxa::network::tarcap
