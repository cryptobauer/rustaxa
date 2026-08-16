#pragma once

#include <json/value.h>

#include "common/types.hpp"
#include "config/config.hpp"
#include "logger/logger.hpp"
#include "network/consensus_query.hpp"
#ifdef RUSTAXA_ENABLE
#include "network/consensus_network_api.hpp"
#endif
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa {
#ifndef RUSTAXA_ENABLE
class PbftManager;
class VoteManager;
#endif
class DagManager;
class TransactionManager;
}  // namespace taraxa

namespace taraxa::network::threadpool {
class PacketsThreadPool;
}

namespace taraxa::network::tarcap {

class TaraxaPeer;
class PbftSyncingState;
class TimePeriodPacketsStats;

class NodeStats {
 public:
  NodeStats(std::shared_ptr<PbftSyncingState> pbft_syncing_state, net::ConsensusQueryClient pbft_chain,
#ifdef RUSTAXA_ENABLE
            network::ConsensusLiveStatusProvider consensus_status,
            network::ConsensusVoteStatusProvider consensus_vote_status,
#else
            std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<VoteManager> vote_mgr,
#endif
            std::shared_ptr<DagManager> dag_mgr, std::shared_ptr<TransactionManager> trx_mgr,
            std::shared_ptr<TimePeriodPacketsStats> packets_stats,
            std::shared_ptr<const threadpool::PacketsThreadPool> thread_pool, const FullNodeConfig& config);

  void logNodeStats(const std::vector<std::shared_ptr<network::tarcap::TaraxaPeer>>& all_peers,
                    const std::vector<std::string>& nodes);
  uint64_t syncTimeSeconds() const;
  Json::Value getStatus(
      std::map<network::tarcap::TarcapVersion, std::shared_ptr<network::tarcap::TaraxaPeer>> peers) const;

 private:
  std::shared_ptr<PbftSyncingState> pbft_syncing_state_;
  net::ConsensusQueryClient pbft_chain_;
#ifdef RUSTAXA_ENABLE
  network::ConsensusLiveStatusProvider consensus_status_;
  network::ConsensusVoteStatusProvider consensus_vote_status_;
#else
  std::shared_ptr<PbftManager> pbft_mgr_;
#endif
  std::shared_ptr<DagManager> dag_mgr_;
#ifndef RUSTAXA_ENABLE
  std::shared_ptr<VoteManager> vote_mgr_;
#endif
  std::shared_ptr<TransactionManager> trx_mgr_;
  std::shared_ptr<TimePeriodPacketsStats> packets_stats_;
  std::shared_ptr<const threadpool::PacketsThreadPool> thread_pool_;

  level_t local_max_level_in_dag_prev_interval_{0};
  uint64_t local_pbft_round_prev_interval_{0};
  uint64_t local_chain_size_prev_interval_{0};
  uint64_t local_pbft_sync_period_prev_interval_{0};
  uint64_t intervals_in_sync_since_launch_{0};
  uint64_t intervals_syncing_since_launch_{0};
  uint64_t syncing_duration_seconds{0};
  uint64_t stalled_syncing_duration_seconds{0};

  // List of node addresses running on this node
  std::string node_addresses_;

  LOG_OBJECTS_DEFINE
};

}  // namespace taraxa::network::tarcap
