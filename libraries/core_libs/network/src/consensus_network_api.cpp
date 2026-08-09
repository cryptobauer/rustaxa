#include "network/consensus_network_api.hpp"

#include <mutex>
#include <stdexcept>
#include <unordered_map>
#include <utility>

#ifdef RUSTAXA_ENABLE
#include "final_chain/final_chain.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa::network {

namespace {

constexpr uint8_t kEffectResultOk = 0;
constexpr uint8_t kEffectResultFailed = 1;
constexpr uint8_t kEffectSendPacket = 0;
constexpr uint8_t kEffectMarkPeerKnown = 2;
constexpr uint8_t kEffectReportPeer = 4;
constexpr uint8_t kEffectDisconnectPeer = 5;
constexpr uint8_t kEffectClearPeerSyncing = 9;
constexpr uint8_t kObjectPillarVote = 5;
constexpr uint32_t kPacketPillarVotesBundle = 15;
constexpr uint32_t kPacketPbftSync = 11;
constexpr uint32_t kPacketPbftBlocksBundle = 16;
constexpr uint32_t kPacketPbftVotesBundle = 3;
constexpr uint32_t kEffectDrainBudget = 1024;
constexpr uint8_t kPbftSyncIngressContinue = 0;
constexpr uint8_t kPbftSyncIngressDuplicate = 1;
constexpr uint8_t kPbftSyncIngressComplete = 2;
constexpr uint8_t kPbftSyncIngressDrop = 3;
constexpr uint8_t kPbftSyncIngressStop = 4;
constexpr uint8_t kPbftSyncIngressMalicious = 5;
constexpr uint8_t kPbftSyncIngressQueueRejected = 6;
constexpr uint8_t kPbftSyncIngressAwaitingSlashing = 7;

}  // namespace

class ConsensusNetworkApi::Impl final {
 public:
  explicit Impl(const rustaxa::BridgePbftService& service)
      : service(service), api(rustaxa::create_consensus_network_api(service)) {}

  const rustaxa::BridgePbftService& service;
  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
  std::mutex lanes_mutex;
  std::unordered_map<uint32_t, std::unique_ptr<std::mutex>> lane_execution_mutexes;
};

ConsensusNetworkApi::ConsensusNetworkApi(const rustaxa::BridgePbftService& service)
    : impl_(std::make_unique<Impl>(service)) {}
ConsensusNetworkApi::~ConsensusNetworkApi() = default;

rustaxa::BridgeConsensusNetworkApi& ConsensusNetworkApi::api() noexcept { return *impl_->api; }

const rustaxa::BridgeConsensusNetworkApi& ConsensusNetworkApi::api() const noexcept { return *impl_->api; }

PbftSyncIngressOutcome ConsensusNetworkApi::admitPbftSyncPacket(const final_chain::FinalChain& final_chain,
                                                                const std::vector<uint8_t>& packet_rlp,
                                                                uint64_t source_payload_id,
                                                                const std::array<uint8_t, 64>& source_peer_id,
                                                                const PbftSyncIngressExecutor& executor) {
  auto step = rustaxa::pbft_service_begin_pbft_sync_ingress(
      impl_->service, final_chain.rustFinalChain(), rust::Slice<const uint8_t>(packet_rlp.data(), packet_rlp.size()),
      source_payload_id, source_peer_id);
  while (step.action == kPbftSyncIngressAwaitingSlashing) {
    if (!step.has_slashing_transaction_effect || !executor.submit_slashing_transaction) {
      throw std::runtime_error("Native PBFT-sync ingress paused without an executable slashing boundary");
    }
    const auto& native_effect = step.slashing_transaction_effect;
    PbftSyncSlashingTransaction transaction{
        native_effect.status,
        native_effect.wallet_index,
        native_effect.nonce,
        native_effect.contract_address,
        native_effect.value,
        native_effect.gas_limit,
        std::vector<uint8_t>(native_effect.call_data.begin(), native_effect.call_data.end())};
    const auto transaction_inserted = executor.submit_slashing_transaction(transaction);
    step = rustaxa::pbft_service_report_pbft_sync_ingress_slashing(impl_->service, final_chain.rustFinalChain(),
                                                                   step.slashing_transaction_effect.proof_hash,
                                                                   transaction_inserted);
  }

  PbftSyncIngressAction action;
  switch (step.action) {
    case kPbftSyncIngressContinue:
      action = PbftSyncIngressAction::kContinue;
      break;
    case kPbftSyncIngressDuplicate:
      action = PbftSyncIngressAction::kDuplicate;
      break;
    case kPbftSyncIngressComplete:
      action = PbftSyncIngressAction::kSyncComplete;
      break;
    case kPbftSyncIngressDrop:
      action = PbftSyncIngressAction::kDrop;
      break;
    case kPbftSyncIngressStop:
      action = PbftSyncIngressAction::kStopSyncing;
      break;
    case kPbftSyncIngressMalicious:
      action = PbftSyncIngressAction::kMalicious;
      break;
    case kPbftSyncIngressQueueRejected:
      action = PbftSyncIngressAction::kQueueRejected;
      break;
    default:
      throw std::runtime_error("Native PBFT-sync ingress returned an unknown terminal action");
  }
  return PbftSyncIngressOutcome{action,
                                static_cast<std::string>(step.error_code),
                                step.block_hash,
                                step.period,
                                step.max_dag_level,
                                step.last_block,
                                step.current_cert_present};
}

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
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
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

PbftSyncRequestOutcome ConsensusNetworkApi::servePbftSyncRequest(uint32_t tarcap_version,
                                                                 const std::array<uint8_t, 64>& peer_id,
                                                                 const std::vector<uint8_t>& request_rlp,
                                                                 uint64_t source_payload_id,
                                                                 const PbftSyncRequestExecutor& executor) {
  auto lane_lock = lockTransportLane(tarcap_version);
  rustaxa::NetworkGetPbftSyncRequest request{};
  request.tarcap_version = tarcap_version;
  request.peer_id = peer_id;
  request.request_rlp.reserve(request_rlp.size());
  for (const auto byte : request_rlp) {
    request.request_rlp.push_back(byte);
  }
  request.source_payload_id = source_payload_id;
  const auto decision = api().consensus_network_ingest_get_pbft_sync_request(std::move(request));

  while (true) {
    const auto batch = api().consensus_network_drain_work(tarcap_version, source_payload_id, true, kEffectDrainBudget);
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
          throw std::runtime_error("PBFT sync effect targets a different peer");
        }
        if (effect.kind == kEffectSendPacket &&
            (effect.packet_kind == kPacketPbftSync || effect.packet_kind == kPacketPbftBlocksBundle)) {
          if (!executor.send_packet(effect.packet_kind,
                                    std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
            throw std::runtime_error("PBFT sync transport send failed");
          }
        } else if (effect.kind == kEffectClearPeerSyncing) {
          executor.clear_peer_syncing();
        } else if (effect.kind == kEffectReportPeer) {
          executor.report_peer(effect.reason_code);
        } else if (effect.kind == kEffectDisconnectPeer) {
          executor.disconnect_peer();
        } else {
          throw std::runtime_error("PBFT sync executor received an unsupported effect");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }

    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected PBFT sync executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }

  return PbftSyncRequestOutcome{decision.status, decision.queued_effect_count,
                                static_cast<std::string>(decision.error_code)};
}

PbftNextVotesBundleRequestOutcome ConsensusNetworkApi::servePbftNextVotesBundleRequest(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t peer_period, uint64_t peer_round,
    uint64_t source_payload_id, const PbftNextVotesBundleExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto decision = api().consensus_network_ingest_pbft_next_votes_bundle_request(
      transport_lane, peer_id, peer_period, peer_round, source_payload_id);

  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
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
          throw std::runtime_error("Next-votes bundle effect targets a different peer");
        }
        if (effect.kind != kEffectSendPacket || effect.packet_kind != kPacketPbftVotesBundle) {
          throw std::runtime_error("Next-votes bundle executor received an unsupported effect");
        }
        if (!executor.send_bundle(std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
          throw std::runtime_error("Next-votes bundle transport send failed");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }

    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected next-votes bundle executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }

  return PbftNextVotesBundleRequestOutcome{decision.status, decision.queued_effect_count,
                                           static_cast<std::string>(decision.error_code)};
}

PbftBlocksBundleOutcome ConsensusNetworkApi::admitPbftBlocksBundle(const final_chain::FinalChain& final_chain,
                                                                   const std::vector<uint8_t>& packet_rlp,
                                                                   uint64_t source_payload_id) {
  rust::Vec<uint8_t> bridge_packet;
  bridge_packet.reserve(packet_rlp.size());
  for (const auto byte : packet_rlp) {
    bridge_packet.push_back(byte);
  }
  const auto decision = api().consensus_network_ingest_pbft_blocks_bundle(final_chain.rustFinalChain(),
                                                                          std::move(bridge_packet), source_payload_id);
  return PbftBlocksBundleOutcome{decision.status, static_cast<std::string>(decision.error_code)};
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
