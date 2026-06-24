#pragma once

#include "dag/dag_manager.hpp"
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#include "packet_handler.hpp"
#include "pbft/pbft_chain.hpp"
#include "pbft/pbft_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa {
#ifndef RUSTAXA_ENABLE
class DbStorage;
#endif
class PbftManager;
}  // namespace taraxa

namespace taraxa::network::tarcap {

/**
 * @brief ExtSyncingPacketHandler is extended abstract PacketHandler with added functions that are used in packet
 *        handlers that need to interact with syncing process in some way
 */
class ExtSyncingPacketHandler : public PacketHandler {
 public:
  ExtSyncingPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                          std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                          std::shared_ptr<PbftSyncingState> pbft_syncing_state, std::shared_ptr<PbftChain> pbft_chain,
                          std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<DagManager> dag_mgr,
#ifndef RUSTAXA_ENABLE
                          std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy sync handler.
#endif
                          const addr_t &node_addr, const std::string &log_channel_name);

  virtual ~ExtSyncingPacketHandler();
  ExtSyncingPacketHandler &operator=(const ExtSyncingPacketHandler &) = delete;
  ExtSyncingPacketHandler &operator=(ExtSyncingPacketHandler &&) = delete;

  void requestDagBlocks(const dev::p2p::NodeID &_nodeID, std::vector<blk_hash_t> &&blocks, PbftPeriod period);
  void requestPendingDagBlocks(std::shared_ptr<TaraxaPeer> peer = nullptr);

 protected:
 #ifdef RUSTAXA_ENABLE
  void requestPbftNextVotesAtPeriodRound(const dev::p2p::NodeID &peer_id, PbftPeriod peer_pbft_period,
                                        PbftRound peer_pbft_round);
 #endif

  std::shared_ptr<PbftSyncingState> pbft_syncing_state_{nullptr};

  std::shared_ptr<PbftChain> pbft_chain_{nullptr};
  std::shared_ptr<PbftManager> pbft_mgr_{nullptr};
  std::shared_ptr<DagManager> dag_mgr_{nullptr};
#ifndef RUSTAXA_ENABLE
  std::shared_ptr<DbStorage> db_{nullptr};  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy sync handler storage.
#else
  struct RustConsensusNetworkApiHolder {
    RustConsensusNetworkApiHolder();
    rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
  };
  std::unique_ptr<RustConsensusNetworkApiHolder> rust_consensus_network_api_;
#endif
};

}  // namespace taraxa::network::tarcap
