#pragma once

#include <json/value.h>

#include <cstdint>
#include <functional>
#include <optional>

namespace taraxa::net {

// LiveStatusSnapshot is the external-client view of live node progress. It keeps
// network, PBFT, vote, and mempool facts grouped at the RPC/GraphQL boundary;
// storage-backed finalized counters remain in ConsensusQueryApi.
struct LiveStatusSnapshot {
  bool pbft_syncing = false;
  uint64_t syncing_seconds = 0;
  uint64_t peer_count = 0;
  uint64_t node_count = 0;
  uint64_t pbft_chain_size = 0;
  uint64_t pbft_sync_period = 0;
  uint64_t pbft_round = 0;
  uint64_t dpos_total_votes = 0;
  uint64_t dpos_node_votes = 0;
  uint64_t dpos_quorum = 0;
  uint64_t pbft_sync_queue_size = 0;
  uint64_t transaction_pool_size = 0;
  uint64_t nonfinalized_transaction_size = 0;
  std::optional<uint64_t> max_peer_pbft_chain_size;

  // Compatibility payload for Test RPC's existing `network` field. New status
  // consumers should prefer typed fields above.
  Json::Value compatibility_network_status;
};

using LiveStatusReader = std::function<LiveStatusSnapshot()>;

}  // namespace taraxa::net
