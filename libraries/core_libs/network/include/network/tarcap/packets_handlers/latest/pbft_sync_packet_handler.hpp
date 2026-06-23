#pragma once

#include <memory>

#include "common/thread_pool.hpp"
#include "network/tarcap/packets/latest/pbft_sync_packet.hpp"
#include "network/tarcap/packets_handlers/interface/sync_packet_handler.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif
#include "vote_manager/vote_manager.hpp"

namespace taraxa::network::tarcap {

class PbftSyncPacketHandler : public ISyncPacketHandler {
 public:
  PbftSyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                        std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                        std::shared_ptr<PbftSyncingState> pbft_syncing_state, std::shared_ptr<PbftChain> pbft_chain,
                        std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<DagManager> dag_mgr,
                        std::shared_ptr<VoteManager> vote_mgr,
#ifndef RUSTAXA_ENABLE
                        std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy PBFT sync handler.
#endif
                        const addr_t& node_addr, const std::string& logs_prefix = "");
  ~PbftSyncPacketHandler() override;

  // Packet type that is processed by this handler
  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kPbftSyncPacket;

 private:
  virtual void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkIngressDecision queuePbftSyncPeriodDataAdmissionRequestEffects(
      const rustaxa::NetworkPbftSyncPeriodDataAdmissionRequestEffects& effects);
  void executePbftSyncPeriodDataAdmissionEffect(PeriodData& period_data, const dev::bytes& period_data_rlp,
                                                const std::shared_ptr<TaraxaPeer>& peer,
                                                std::vector<std::shared_ptr<PbftVote>>& current_block_cert_votes);
#endif

 protected:
  virtual PeriodData decodePeriodData(const dev::RLP& period_data_rlp) const;
  virtual std::vector<std::shared_ptr<PbftVote>> decodeVotesBundle(const dev::RLP& votes_bundle_rlp) const;

  void pbftSyncComplete();
  void delayedPbftSync(uint32_t counter);

  static constexpr uint32_t kDelayedPbftSyncDelayMs = 10;

  std::shared_ptr<VoteManager> vote_mgr_;
  util::ThreadPool periodic_events_tp_;
#ifdef RUSTAXA_ENABLE
  struct RustConsensusNetworkApiHolder;
  std::unique_ptr<RustConsensusNetworkApiHolder> rust_consensus_network_api_;
#endif
};

}  // namespace taraxa::network::tarcap
