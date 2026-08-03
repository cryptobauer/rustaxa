#pragma once

#include <vector>

#include "packet_handler.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "network/consensus_network_api.hpp"
#include "network/tarcap/tarcap_version.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

class ExtPillarVotePacketHandler : public PacketHandler {
 public:
  ExtPillarVotePacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                             std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                             std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
#ifdef RUSTAXA_ENABLE
                             network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
#endif
                             const addr_t& node_addr, const std::string& log_channel);
  ~ExtPillarVotePacketHandler() override;

 protected:
  /**
   * Processes one decoded pillar vote through the packet-family route.
   * Rust mode returns true only for a newly admitted vote; pure-C++ mode
   * preserves the legacy relevance, validation, insertion, and peer sequence.
   */
  bool processPillarVote(const std::shared_ptr<PillarVote>& vote, const std::shared_ptr<TaraxaPeer>& peer,
                         SubprotocolPacketType packet_type);

  /**
   * Processes one complete decoded pillar-vote packet in input order.
   *
   * The result has one newly-admitted flag per vote. Rust-mode bridge or
   * executor failures throw after acknowledgement; ordinary rejection or
   * duplication returns false. Pure-C++ mode remains member-at-a-time.
   */
  std::vector<bool> processPillarVotes(const std::vector<std::shared_ptr<PillarVote>>& votes,
                                       const std::shared_ptr<TaraxaPeer>& peer, SubprotocolPacketType packet_type);

#ifdef RUSTAXA_ENABLE
  /** Converts canonical packet payloads and requests atomic native preflight. */
  rust::Vec<rustaxa::NetworkIngressDecision> ingestPillarVotes(rust::Vec<rustaxa::PillarVoteRlpPayload> votes,
                                                               const std::shared_ptr<TaraxaPeer>& peer,
                                                               bool allow_gossip) const;

  /**
   * Executes and acknowledges all queued effects for the correlated ids.
   * Returns newly-admitted outcomes in id order and throws for a missing,
   * mismatched, unsupported, or failed application effect.
   */
  std::vector<bool> executeConsensusNetworkEffects(const std::vector<uint64_t>& application_effect_ids);
#endif

 protected:
  std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager_;
#ifdef RUSTAXA_ENABLE
  network::ConsensusNetworkApiShared rust_consensus_network_api_;
  TarcapVersion transport_lane_;
#endif
};

}  // namespace taraxa::network::tarcap
