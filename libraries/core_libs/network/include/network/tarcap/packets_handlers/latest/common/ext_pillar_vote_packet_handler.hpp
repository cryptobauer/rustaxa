#pragma once

#include "packet_handler.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "network/consensus_network_api.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

class ExtPillarVotePacketHandler : public PacketHandler {
 public:
  ExtPillarVotePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                             std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                             std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
#ifdef RUSTAXA_ENABLE
                             network::ConsensusNetworkApiShared consensus_network_api,
#endif
                             const addr_t& node_addr, const std::string& log_channel);
  ~ExtPillarVotePacketHandler() override;

 protected:
  bool processPillarVote(const std::shared_ptr<PillarVote>& vote, const std::shared_ptr<TaraxaPeer>& peer,
                         SubprotocolPacketType packet_type);

#ifdef RUSTAXA_ENABLE
  rustaxa::PillarVoteRelevancePlan planPillarVoteRelevance(const std::shared_ptr<PillarVote>& vote) const;
#endif

 protected:
  std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager_;
#ifdef RUSTAXA_ENABLE
  network::ConsensusNetworkApiShared rust_consensus_network_api_;
#endif
};

}  // namespace taraxa::network::tarcap
