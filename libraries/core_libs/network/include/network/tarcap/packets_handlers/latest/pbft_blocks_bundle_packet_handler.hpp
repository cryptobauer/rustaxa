#pragma once

#include <memory>

#include "common/packet_handler.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa {
class PbftManager;
namespace final_chain {
class FinalChain;
}

}  // namespace taraxa

namespace taraxa::network::tarcap {

class PbftSyncingState;

class PbftBlocksBundlePacketHandler : public PacketHandler {
 public:
  PbftBlocksBundlePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                std::shared_ptr<PbftManager> pbft_mgr,
                                std::shared_ptr<final_chain::FinalChain> final_chain,
                                std::shared_ptr<PbftSyncingState> syncing_state, const addr_t& node_addr,
                                const std::string& logs_prefix = "");
  ~PbftBlocksBundlePacketHandler() override;

  // Packet type that is processed by this handler
  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kPbftBlocksBundlePacket;
  static constexpr size_t kMaxBlocksInPacket = 10;

 private:
  virtual void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkIngressDecision queuePbftProposedBlockBundleEffects(
      const rustaxa::NetworkPbftProposedBlockSidecarEffects& effects);
  void executeConsensusNetworkEffects(size_t budget);
#endif

  std::shared_ptr<PbftManager> pbft_mgr_;
  std::shared_ptr<final_chain::FinalChain> final_chain_;
  std::shared_ptr<PbftSyncingState> pbft_syncing_state_;
#ifdef RUSTAXA_ENABLE
  struct RustConsensusNetworkApiHolder;
  std::unique_ptr<RustConsensusNetworkApiHolder> rust_consensus_network_api_;
#endif
};

}  // namespace taraxa::network::tarcap
