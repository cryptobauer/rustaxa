#include "network/consensus_network_api.hpp"

#include <mutex>
#include <stdexcept>
#include <unordered_map>
#include <utility>

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa::network {

namespace {

constexpr uint8_t kEffectResultOk = 0;
constexpr uint8_t kEffectResultFailed = 1;
constexpr uint8_t kEffectSendPacket = 0;
constexpr uint8_t kEffectMarkPeerKnown = 2;
constexpr uint8_t kEffectReportPeer = 4;
constexpr uint8_t kEffectDisconnectPeer = 5;
constexpr uint8_t kObjectPillarVote = 5;
constexpr uint32_t kPacketPillarVotesBundle = 15;
constexpr uint32_t kEffectDrainBudget = 1024;

}  // namespace

class ConsensusNetworkApi::Impl final {
 public:
  explicit Impl(const rustaxa::BridgePbftService& service) : api(rustaxa::create_consensus_network_api(service)) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
  std::mutex lanes_mutex;
  std::unordered_map<uint32_t, std::unique_ptr<std::mutex>> lane_execution_mutexes;
};

ConsensusNetworkApi::ConsensusNetworkApi(const rustaxa::BridgePbftService& service)
    : impl_(std::make_unique<Impl>(service)) {}
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

PillarVotesBundleRequestOutcome ConsensusNetworkApi::servePillarVotesBundleRequest(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t period,
    const std::array<uint8_t, 32>& pillar_block_hash, uint64_t source_payload_id,
    const PillarVotesBundleExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto decision = api().consensus_network_ingest_pillar_votes_bundle_request(
      transport_lane, peer_id, period, pillar_block_hash, source_payload_id);

  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }

    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;

      try {
        if (effect.peer_id != peer_id) {
          throw std::runtime_error("Pillar-vote bundle effect targets a different peer");
        }
        if (effect.kind == kEffectSendPacket && effect.packet_kind == kPacketPillarVotesBundle) {
          if (!executor.send_bundle(std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
            throw std::runtime_error("Pillar-vote bundle transport send failed");
          }
        } else if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectPillarVote) {
          executor.mark_vote_known(effect.object_hash);
        } else if (effect.kind == kEffectReportPeer) {
          executor.report_peer(effect.reason_code);
        } else if (effect.kind == kEffectDisconnectPeer) {
          executor.disconnect_peer();
        } else {
          throw std::runtime_error("Pillar-vote bundle executor received an unsupported effect");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }

    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected pillar-vote bundle executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }

  return PillarVotesBundleRequestOutcome{decision.status, decision.queued_effect_count,
                                         static_cast<std::string>(decision.error_code)};
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
