#pragma once

#ifdef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/rust/consensus_transport_packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"
#include "vote/pillar_vote.hpp"
#else
#include "network/tarcap/packets_handlers/latest/common/ext_pillar_vote_packet_handler.hpp"
#endif

namespace taraxa::network::tarcap {

class IPillarVotePacketHandler : public
#ifdef RUSTAXA_ENABLE
                                 RustConsensusTransportPacketHandler
#else
                                 ExtPillarVotePacketHandler
#endif
{
 public:
  IPillarVotePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                           std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
                           std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
#else
                           network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
#endif
                           const addr_t& node_addr, const std::string& logs_prefix);

#ifndef RUSTAXA_ENABLE
  void onNewPillarVote(const std::shared_ptr<PillarVote>& vote, bool rebroadcast = false);
  virtual void sendPillarVote(const std::shared_ptr<TaraxaPeer>& peer, const std::shared_ptr<PillarVote>& vote) = 0;
#endif
};

}  // namespace taraxa::network::tarcap
