#pragma once

#include "network/tarcap/packets_handlers/latest/common/ext_syncing_packet_handler.hpp"
#include "transaction/transaction.hpp"

namespace taraxa::network::tarcap {

class IDagBlockPacketHandler : public ExtSyncingPacketHandler {
 public:
  IDagBlockPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                         std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
                         std::shared_ptr<PbftSyncingState> pbft_syncing_state, net::ConsensusQueryClient pbft_chain,
#else
                         net::ConsensusQueryClient pbft_chain,
#endif
#ifndef RUSTAXA_ENABLE
                         std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<DagManager> dag_mgr,
                         std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy DAG handler.
#else
                         network::ConsensusLiveStatusProvider consensus_status,
                         network::ConsensusNetworkApiShared consensus_network_api,
#endif
                         const addr_t &node_addr, const std::string &logs_prefix);

  void onNewBlockVerified(const std::shared_ptr<DagBlock> &block, bool proposed, const SharedTransactions &trxs);
  virtual void sendBlockWithTransactions(const std::shared_ptr<TaraxaPeer> &peer,
                                         const std::shared_ptr<DagBlock> &block, SharedTransactions &&trxs) = 0;

  // Note: Used only in tests
  void requestDagBlocks(std::shared_ptr<TaraxaPeer> peer);
};

}  // namespace taraxa::network::tarcap
