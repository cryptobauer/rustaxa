#pragma once

#include "packet_handler.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

class ExtPillarVotePacketHandler : public PacketHandler {
 public:
  ExtPillarVotePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                             std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                             std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
                             const addr_t& node_addr, const std::string& log_channel);
  ~ExtPillarVotePacketHandler() override;

 protected:
  bool processPillarVote(const std::shared_ptr<PillarVote>& vote, const std::shared_ptr<TaraxaPeer>& peer,
                         SubprotocolPacketType packet_type);

#ifdef RUSTAXA_ENABLE
  rustaxa::PillarVoteRelevancePlan planPillarVoteRelevance(const std::shared_ptr<PillarVote>& vote) const;
  rustaxa::NetworkIngressDecision queuePillarVoteValidationRequestEffects(
      const rustaxa::NetworkPillarVoteValidationRequestEffects& effects, SubprotocolPacketType packet_type);
  bool executePillarVoteValidationEffect(const std::shared_ptr<PillarVote>& vote,
                                         const std::shared_ptr<TaraxaPeer>& peer, SubprotocolPacketType packet_type);
  rustaxa::NetworkIngressDecision queuePillarVoteAdmissionRequestEffects(
      const rustaxa::NetworkPillarVoteAdmissionRequestEffects& effects, SubprotocolPacketType packet_type);
  void executePillarVoteAdmissionEffect(const std::shared_ptr<PillarVote>& vote,
                                        const std::shared_ptr<TaraxaPeer>& peer, SubprotocolPacketType packet_type);
#endif

 protected:
  std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager_;
#ifdef RUSTAXA_ENABLE
  struct RustConsensusNetworkApiHolder;
  std::unique_ptr<RustConsensusNetworkApiHolder> rust_consensus_network_api_;
#endif
};

}  // namespace taraxa::network::tarcap
