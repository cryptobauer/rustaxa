#pragma once

#include "network/tarcap/packets/latest/get_pillar_votes_bundle_packet.hpp"
#include "network/tarcap/packets_handlers/interface/get_pillar_votes_bundle_packet_handler.hpp"
#ifdef RUSTAXA_ENABLE
#include "network/consensus_network_api.hpp"
#include "network/tarcap/tarcap_version.hpp"
#else
#include "pillar_chain/pillar_chain_manager.hpp"
#endif

namespace taraxa::network::tarcap {

class GetPillarVotesBundlePacketHandler : public IGetPillarVotesBundlePacketHandler {
 public:
  GetPillarVotesBundlePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
                                    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
#else
                                    network::ConsensusNetworkApiShared consensus_network_api,
                                    TarcapVersion transport_lane,
#endif
                                    const addr_t& node_addr, const std::string& logs_prefix = "");
  ~GetPillarVotesBundlePacketHandler() override;

  void requestPillarVotesBundle(PbftPeriod period, const blk_hash_t& pillar_block_hash,
                                const std::shared_ptr<TaraxaPeer>& peer) override;

  // Packet type that is processed by this handler
  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kGetPillarVotesBundlePacket;

 private:
  virtual void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

 protected:
#ifndef RUSTAXA_ENABLE
  std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager_;
#else
  network::ConsensusNetworkApiShared rust_consensus_network_api_;
  const TarcapVersion transport_lane_;
#endif
};

}  // namespace taraxa::network::tarcap
