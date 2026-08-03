#include "network/consensus_network_api.hpp"

#include <mutex>
#include <unordered_map>
#include <utility>

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa::network {

class ConsensusNetworkApi::Impl final {
 public:
  Impl() : api(rustaxa::create_consensus_network_api(config())) {}

  static rustaxa::NetworkApiConfig config() {
    rustaxa::NetworkApiConfig value{};
    value.max_effects_per_drain = 1024;
    return value;
  }

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
  std::mutex lanes_mutex;
  std::unordered_map<uint32_t, std::unique_ptr<std::mutex>> lane_execution_mutexes;
};

ConsensusNetworkApi::ConsensusNetworkApi() : impl_(std::make_unique<Impl>()) {}
ConsensusNetworkApi::~ConsensusNetworkApi() = default;

rustaxa::BridgeConsensusNetworkApi& ConsensusNetworkApi::api() noexcept { return *impl_->api; }

const rustaxa::BridgeConsensusNetworkApi& ConsensusNetworkApi::api() const noexcept { return *impl_->api; }

std::unique_lock<std::mutex> ConsensusNetworkApi::lockTransportLane(uint32_t transport_lane) {
  std::mutex* lane_mutex = nullptr;
  {
    std::lock_guard lanes_lock(impl_->lanes_mutex);
    auto& stored_mutex = impl_->lane_execution_mutexes[transport_lane];
    if (!stored_mutex) {
      stored_mutex = std::make_unique<std::mutex>();
    }
    lane_mutex = stored_mutex.get();
  }
  return std::unique_lock(*lane_mutex);
}

std::optional<std::array<uint8_t, 64>> ConsensusNetworkApi::selectMaxChainPeer(
    uint64_t local_pbft_syncing_period, const std::vector<ConsensusPeerCandidate>& candidates) const {
  rustaxa::NetworkPeerSelectionFacts facts{};
  facts.local_pbft_syncing_period = local_pbft_syncing_period;
  facts.candidates.reserve(candidates.size());
  for (const auto& candidate : candidates) {
    rustaxa::NetworkPbftSyncPeerCandidate bridge_candidate{};
    bridge_candidate.peer_id = candidate.peer_id;
    bridge_candidate.pbft_chain_size = candidate.pbft_chain_size;
    bridge_candidate.dag_level = candidate.dag_level;
    bridge_candidate.is_light_node = candidate.is_light_node;
    bridge_candidate.light_node_history = candidate.light_node_history;
    bridge_candidate.peer_dag_synced = candidate.peer_dag_synced;
    bridge_candidate.peer_dag_syncing = candidate.peer_dag_syncing;
    bridge_candidate.dag_sync_allowed = candidate.dag_sync_allowed;
    facts.candidates.push_back(std::move(bridge_candidate));
  }

  const auto plan = api().consensus_network_plan_max_chain_peer_selection(facts);
  if (!plan.has_peer) {
    return std::nullopt;
  }
  return plan.peer_id;
}

}  // namespace taraxa::network
#endif
