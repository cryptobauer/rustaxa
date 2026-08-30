#include "network/consensus_network_api.hpp"

#include <algorithm>
#include <atomic>
#include <iterator>
#include <mutex>
#include <stdexcept>
#include <unordered_map>
#include <unordered_set>
#include <utility>

#ifdef RUSTAXA_ENABLE
#include "consensus/consensus_application.hpp"
#include "consensus/consensus_host_ports.hpp"
#include "final_chain/final_chain.hpp"
#include "network/consensus_query.hpp"
#include "rustaxa-bridge/application_host_ffi.rs.h"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"

namespace taraxa::network {

namespace {

constexpr uint8_t kEffectResultOk = 0;
constexpr uint8_t kEffectResultFailed = 1;
constexpr uint8_t kEffectSendPacket = 0;
constexpr uint8_t kEffectMarkPeerKnown = 2;
constexpr uint8_t kEffectReportPeer = 4;
constexpr uint8_t kEffectDisconnectPeer = 5;
constexpr uint8_t kEffectClearPeerSyncing = 9;
constexpr uint8_t kObjectTransaction = 2;
constexpr uint8_t kObjectDagBlock = 3;
constexpr uint32_t kPacketPbftSync = 11;
constexpr uint32_t kPacketPbftBlocksBundle = 16;
constexpr uint32_t kPacketDagBlock = 5;
constexpr uint32_t kPacketDagSync = 6;
constexpr uint32_t kPacketGetDagSync = 12;
constexpr uint8_t kEgressFamilyPillarVotesRequest = 5;
constexpr uint32_t kEffectDrainBudget = 1024;
constexpr uint8_t kPbftSyncIngressContinue = 0;
constexpr uint8_t kPbftSyncIngressDuplicate = 1;
constexpr uint8_t kPbftSyncIngressComplete = 2;
constexpr uint8_t kPbftSyncIngressDrop = 3;
constexpr uint8_t kPbftSyncIngressStop = 4;
constexpr uint8_t kPbftSyncIngressMalicious = 5;
constexpr uint8_t kPbftSyncIngressQueueRejected = 6;
constexpr uint8_t kPbftSyncIngressAwaitingSlashing = 7;
constexpr uint8_t kPbftSyncCommandAdmitSource = 0;
constexpr uint8_t kPbftSyncCommandRecordActivity = 1;
constexpr uint8_t kPbftSyncCommandStop = 2;
constexpr uint8_t kPbftSyncCommandDisconnect = 3;
constexpr uint8_t kPbftSyncCommandTick = 4;
constexpr uint8_t kPbftSyncCommandComplete = 5;
constexpr uint8_t kPbftSyncCommandPlanLastBlock = 6;
constexpr uint8_t kPbftSyncCommandPlanDelayed = 7;

SharedConsensusApplication requireConsensusApplication(SharedConsensusApplication consensus_application) {
  if (!consensus_application) {
    throw std::invalid_argument("Consensus network API requires a shared consensus application");
  }
  return consensus_application;
}

rust::Vec<uint8_t> toRustBytes(const dev::bytes& value) {
  rust::Vec<uint8_t> out;
  out.reserve(value.size());
  std::copy(value.begin(), value.end(), std::back_inserter(out));
  return out;
}

template <typename Value>
std::array<uint8_t, 32> toBridgeU256(const Value& value) {
  std::array<uint8_t, 32> out{};
  const auto bytes = dev::toBigEndian(value);
  if (bytes.size() > out.size()) {
    throw std::runtime_error("u256 value exceeds 32 bytes");
  }
  std::copy(bytes.begin(), bytes.end(), out.begin() + static_cast<std::ptrdiff_t>(out.size() - bytes.size()));
  return out;
}

PbftSyncLifecycleOutcome toPbftSyncLifecycleOutcome(const rustaxa::NetworkPbftSyncCommandOutcome& value) {
  return {value.accepted,
          value.active,
          value.stopped,
          value.expired,
          value.restart_sync,
          value.retry,
          value.request_next,
          value.request_pending_dag_if_idle,
          value.deep_syncing,
          value.generation,
          static_cast<std::string>(value.error_code)};
}

rustaxa::NetworkConsensusPacketRequest toNativeConsensusPacketRequest(const ConsensusPacketRequest& request) {
  rustaxa::NetworkConsensusPacketRequest native{};
  native.transport_lane = request.transport_lane;
  native.peer_id = request.peer_id;
  native.peer_pbft_chain_size = request.peer_pbft_chain_size;
  native.source_payload_id = request.source_payload_id;
  native.packet_rlp.reserve(request.packet_rlp.size());
  for (const auto byte : request.packet_rlp) {
    native.packet_rlp.push_back(byte);
  }
  native.current_period = request.current_period;
  native.current_round = request.current_round;
  native.current_step = request.current_step;
  native.max_future_period_delta = request.max_future_period_delta;
  native.max_future_round_delta = request.max_future_round_delta;
  native.max_future_step_delta = request.max_future_step_delta;
  native.validate_max_round_step = request.validate_max_round_step;
  native.can_request_pbft_sync = request.can_request_pbft_sync;
  native.can_request_next_votes_sync = request.can_request_next_votes_sync;
  return native;
}

rust::Vec<rustaxa::SlashingSubmitterIdentity> makeSlashingSubmitters(const FullNodeConfig& config) {
  rust::Vec<rustaxa::SlashingSubmitterIdentity> submitters;
  submitters.reserve(config.wallets.size());
  for (size_t index = 0; index < config.wallets.size(); ++index) {
    rustaxa::SlashingSubmitterIdentity submitter{};
    submitter.wallet_index = index;
    submitter.address = config.wallets[index].node_addr.asArray();
    submitters.push_back(std::move(submitter));
  }
  return submitters;
}

}  // namespace

class ConsensusNetworkApi::Impl final {
 public:
  Impl(SharedConsensusApplication consensus_application, std::shared_ptr<final_chain::FinalChain> final_chain,
       ConsensusNetworkObservers observers)
      : consensus_application(requireConsensusApplication(std::move(consensus_application))),
        final_chain(std::move(final_chain)),
        external_evm(this->final_chain),
        api(rustaxa::create_consensus_network_api(this->consensus_application->service())),
        query(this->consensus_application->queryClient()),
        observers(std::move(observers)) {}

  SharedConsensusApplication consensus_application;
  std::shared_ptr<final_chain::FinalChain> final_chain;
  ExternalEvmPort external_evm;
  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
  taraxa::net::ConsensusQueryClient query;
  ConsensusNetworkObservers observers;
  std::mutex lanes_mutex;
  std::unordered_map<uint32_t, std::unique_ptr<std::mutex>> lane_execution_mutexes;
  std::atomic<uint64_t> next_egress_payload_id{uint64_t{1} << 63};
};

ConsensusNetworkApi::ConsensusNetworkApi(SharedConsensusApplication consensus_application,
                                         std::shared_ptr<final_chain::FinalChain> final_chain,
                                         ConsensusNetworkObservers observers)
    : impl_(std::make_unique<Impl>(std::move(consensus_application), std::move(final_chain), std::move(observers))) {}
ConsensusNetworkApi::~ConsensusNetworkApi() = default;

PbftSyncStatus ConsensusNetworkApi::pbftSyncStatus(uint64_t now_ms) const {
  const auto view = (*impl_->query)->consensus_query_pbft_sync_status(now_ms);
  return {view.active,        view.deep_syncing,     view.generation,        view.has_peer,        view.peer_id,
          view.has_last_peer, view.last_peer_id,     view.target_chain_size, view.current_period,  view.request_period,
          view.started_at_ms, view.last_activity_ms, view.elapsed_ms,        view.inactive_for_ms, view.start_count,
          view.stop_count,    view.inactivity_count, view.disconnect_count,  view.last_stop_reason};
}

PbftSyncLifecycleOutcome ConsensusNetworkApi::admitPbftSyncSource(const std::array<uint8_t, 64>& peer_id,
                                                                  PbftSyncResponseSource source) const {
  rustaxa::NetworkPbftSyncCommand command{};
  command.kind = kPbftSyncCommandAdmitSource;
  command.peer_id = peer_id;
  command.source = static_cast<uint8_t>(source);
  return toPbftSyncLifecycleOutcome(impl_->api->consensus_network_apply_pbft_sync_command(command));
}

PbftSyncLifecycleOutcome ConsensusNetworkApi::recordPbftSyncActivity(uint64_t now_ms, uint64_t generation,
                                                                     const std::array<uint8_t, 64>& peer_id) const {
  rustaxa::NetworkPbftSyncCommand command{};
  command.kind = kPbftSyncCommandRecordActivity;
  command.now_ms = now_ms;
  command.generation = generation;
  command.peer_id = peer_id;
  return toPbftSyncLifecycleOutcome(impl_->api->consensus_network_apply_pbft_sync_command(command));
}

PbftSyncLifecycleOutcome ConsensusNetworkApi::stopPbftSync(uint64_t generation, const std::array<uint8_t, 64>& peer_id,
                                                           uint8_t reason) const {
  rustaxa::NetworkPbftSyncCommand command{};
  command.kind = kPbftSyncCommandStop;
  command.generation = generation;
  command.peer_id = peer_id;
  command.reason = reason;
  return toPbftSyncLifecycleOutcome(impl_->api->consensus_network_apply_pbft_sync_command(command));
}

PbftSyncLifecycleOutcome ConsensusNetworkApi::handlePbftSyncDisconnect(uint64_t generation,
                                                                       const std::array<uint8_t, 64>& peer_id) const {
  rustaxa::NetworkPbftSyncCommand command{};
  command.kind = kPbftSyncCommandDisconnect;
  command.generation = generation;
  command.peer_id = peer_id;
  return toPbftSyncLifecycleOutcome(impl_->api->consensus_network_apply_pbft_sync_command(command));
}

PbftSyncLifecycleOutcome ConsensusNetworkApi::tickPbftSync(uint64_t now_ms, uint64_t generation) const {
  rustaxa::NetworkPbftSyncCommand command{};
  command.kind = kPbftSyncCommandTick;
  command.now_ms = now_ms;
  command.generation = generation;
  return toPbftSyncLifecycleOutcome(impl_->api->consensus_network_apply_pbft_sync_command(command));
}

PbftSyncLifecycleOutcome ConsensusNetworkApi::completePbftSync(uint64_t now_ms, uint64_t generation,
                                                               const std::array<uint8_t, 64>& peer_id,
                                                               uint64_t sync_queue_size) const {
  rustaxa::NetworkPbftSyncCommand command{};
  command.kind = kPbftSyncCommandComplete;
  command.now_ms = now_ms;
  command.generation = generation;
  command.peer_id = peer_id;
  command.sync_queue_size = sync_queue_size;
  return toPbftSyncLifecycleOutcome(impl_->api->consensus_network_apply_pbft_sync_command(command));
}

PbftSyncLifecycleOutcome ConsensusNetworkApi::planPbftSyncLastBlock(uint64_t now_ms, uint64_t generation,
                                                                    const std::array<uint8_t, 64>& peer_id,
                                                                    uint64_t syncing_period, uint64_t finalized_period,
                                                                    uint64_t remote_period,
                                                                    uint64_t sync_level_size) const {
  rustaxa::NetworkPbftSyncCommand command{};
  command.kind = kPbftSyncCommandPlanLastBlock;
  command.now_ms = now_ms;
  command.generation = generation;
  command.peer_id = peer_id;
  command.syncing_period = syncing_period;
  command.finalized_period = finalized_period;
  command.remote_period = remote_period;
  command.sync_level_size = sync_level_size;
  return toPbftSyncLifecycleOutcome(impl_->api->consensus_network_apply_pbft_sync_command(command));
}

PbftSyncLifecycleOutcome ConsensusNetworkApi::planDelayedPbftSync(uint64_t now_ms, uint64_t generation,
                                                                  const std::array<uint8_t, 64>& peer_id,
                                                                  uint64_t syncing_period, uint64_t finalized_period,
                                                                  uint64_t sync_level_size, uint32_t retry_count,
                                                                  uint64_t retry_delay_ms) const {
  rustaxa::NetworkPbftSyncCommand command{};
  command.kind = kPbftSyncCommandPlanDelayed;
  command.now_ms = now_ms;
  command.generation = generation;
  command.peer_id = peer_id;
  command.syncing_period = syncing_period;
  command.finalized_period = finalized_period;
  command.sync_level_size = sync_level_size;
  command.retry_count = retry_count;
  command.retry_delay_ms = retry_delay_ms;
  return toPbftSyncLifecycleOutcome(impl_->api->consensus_network_apply_pbft_sync_command(command));
}

PbftSyncStartOutcome ConsensusNetworkApi::beginPbftSync(const PbftSyncStartRequest& request) const {
  rustaxa::NetworkPbftSyncStartRequest native{};
  native.start = request.start;
  native.now_ms = request.now_ms;
  native.local_pbft_synced_period = request.local_pbft_synced_period;
  native.local_pbft_chain_size = request.local_pbft_chain_size;
  native.candidates.reserve(request.candidates.size());
  for (const auto& candidate : request.candidates) {
    rustaxa::NetworkPbftSyncPeerCandidate peer{};
    peer.peer_id = candidate.peer_id;
    peer.pbft_chain_size = candidate.pbft_chain_size;
    peer.dag_level = candidate.dag_level;
    peer.is_light_node = candidate.is_light_node;
    peer.light_node_history = candidate.light_node_history;
    peer.peer_dag_synced = candidate.peer_dag_synced;
    peer.peer_dag_syncing = candidate.peer_dag_syncing;
    peer.dag_sync_allowed = candidate.dag_sync_allowed;
    native.candidates.push_back(std::move(peer));
  }
  const auto outcome = impl_->api->consensus_network_begin_pbft_sync(native);
  return {outcome.status,         static_cast<std::string>(outcome.error_code),
          outcome.started,        outcome.has_peer,
          outcome.peer_id,        outcome.peer_pbft_chain_size,
          outcome.request_period, outcome.generation,
          outcome.deep_syncing,   outcome.enable_snapshot_creation};
}

StatusPacketReport ConsensusNetworkApi::ingestStatusPacket(const StatusPacketRequest& request) const {
  rustaxa::NetworkStatusPacketRequest native{};
  native.peer_id = request.peer_id;
  native.packet_rlp.reserve(request.packet_rlp.size());
  std::copy(request.packet_rlp.begin(), request.packet_rlp.end(), std::back_inserter(native.packet_rlp));
  native.source_peer_ready = request.source_peer_ready;
  native.local_pbft_synced_period = request.local_pbft_synced_period;
  native.local_pbft_period = request.local_pbft_period;
  native.local_pbft_round = request.local_pbft_round;
  native.peer_dag_synced = request.peer_dag_synced;
  const auto outcome = impl_->api->consensus_network_ingest_status_packet(std::move(native));
  StatusPacketReport report{};
  report.status = outcome.status;
  report.error_code = static_cast<std::string>(outcome.error_code);
  report.malicious = outcome.malicious;
  report.initial = outcome.initial;
  report.accept_peer = outcome.accept_peer;
  report.disconnect_peer = outcome.disconnect_peer;
  report.peer_pbft_chain_size = outcome.peer_pbft_chain_size;
  report.peer_pbft_period = outcome.peer_pbft_period;
  report.peer_pbft_round = outcome.peer_pbft_round;
  report.peer_dag_level = outcome.peer_dag_level;
  report.peer_syncing = outcome.peer_syncing;
  report.peer_is_light_node = outcome.peer_is_light_node;
  report.peer_light_node_history = outcome.peer_light_node_history;
  report.node_major_version = outcome.node_major_version;
  report.node_minor_version = outcome.node_minor_version;
  report.node_patch_version = outcome.node_patch_version;
  report.request_pbft_sync = outcome.request_pbft_sync;
  report.request_pending_dag_blocks = outcome.request_pending_dag_blocks;
  report.request_next_votes = outcome.request_next_votes;
  report.next_votes_period = outcome.next_votes_period;
  report.next_votes_round = outcome.next_votes_round;
  report.next_votes_request_rlp.assign(outcome.next_votes_request_rlp.begin(), outcome.next_votes_request_rlp.end());
  report.sync_generation = outcome.sync_generation;
  return report;
}

StatusPacketBuildReport ConsensusNetworkApi::buildStatusPacket(const StatusPacketBuildRequest& request) const {
  rustaxa::NetworkStatusPacketBuildRequest native{};
  native.initial = request.initial;
  native.local_pbft_chain_size = request.local_pbft_chain_size;
  native.local_pbft_round = request.local_pbft_round;
  native.local_dag_level = request.local_dag_level;
  const auto outcome = impl_->api->consensus_network_build_status_packet(native);
  return {outcome.status, static_cast<std::string>(outcome.error_code),
          std::vector<uint8_t>(outcome.packet_rlp.begin(), outcome.packet_rlp.end())};
}

PbftSyncIngressOutcome ConsensusNetworkApi::admitPbftSyncPacket(
    const std::vector<uint8_t>& packet_rlp, uint64_t source_payload_id, const std::array<uint8_t, 64>& source_peer_id,
    const std::vector<PbftSyncSlashingSubmitterFact>& slashing_submitters, const PbftSyncIngressExecutor& executor) {
  rust::Vec<rustaxa::SlashingSubmitterIdentity> native_submitters;
  native_submitters.reserve(slashing_submitters.size());
  for (const auto& submitter : slashing_submitters) {
    rustaxa::SlashingSubmitterIdentity native{};
    native.wallet_index = submitter.wallet_index;
    native.nonce = submitter.nonce;
    native.balance = submitter.balance;
    native_submitters.push_back(std::move(native));
  }
  auto step = rustaxa::pbft_service_begin_pbft_sync_ingress(
      impl_->consensus_application->service(), rust::Slice<const uint8_t>(packet_rlp.data(), packet_rlp.size()),
      source_payload_id, source_peer_id, std::move(native_submitters));
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
    step = rustaxa::pbft_service_report_pbft_sync_ingress_slashing(
        impl_->consensus_application->service(), step.slashing_transaction_effect.proof_hash, transaction_inserted);
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

bool ConsensusNetworkApi::reportPbftVoteSlashingSubmission(const std::array<uint8_t, 32>& proof_hash,
                                                           bool transaction_inserted) {
  return impl_->consensus_application->service().pbft_service_verified_votes_report_slashing_transaction_submission(
      proof_hash, transaction_inserted);
}

bool ConsensusNetworkApi::executePbftVoteSlashingTransaction(const PbftVoteSlashingTransaction& effect,
                                                             const FullNodeConfig& config) {
  if (effect.status != 0 || effect.wallet_index >= config.wallets.size()) {
    throw std::runtime_error("Native network vote admission returned an invalid slashing transaction effect");
  }
  const auto inserted = submitSlashingTransaction(effect.wallet_index, effect.nonce, effect.contract_address,
                                                  effect.value, effect.gas_limit, effect.call_data, config);
  return reportPbftVoteSlashingSubmission(effect.proof_hash, inserted);
}

bool ConsensusNetworkApi::executePbftSyncSlashingTransaction(const PbftSyncSlashingTransaction& effect,
                                                             const FullNodeConfig& config) {
  if (effect.status != 0 || effect.wallet_index >= config.wallets.size()) {
    throw std::runtime_error("Native PBFT-sync admission returned an invalid slashing transaction effect");
  }
  return submitSlashingTransaction(effect.wallet_index, effect.nonce, effect.contract_address, effect.value,
                                   effect.gas_limit, effect.call_data, config);
}

ConsensusPacketOutcome ConsensusNetworkApi::ingestPbftVotePacket(const ConsensusPacketRequest& request,
                                                                 const FullNodeConfig& config,
                                                                 const ConsensusTransportExecutor& executor) {
  auto lane_lock = lockTransportLane(request.transport_lane);
  const auto report = impl_->api->consensus_network_ingest_pbft_vote_packet(toNativeConsensusPacketRequest(request),
                                                                            makeSlashingSubmitters(config));
  ConsensusPacketOutcome outcome{};
  outcome.status = report.status;
  outcome.malicious = report.malicious;
  outcome.error_code = static_cast<std::string>(report.error_code);
  outcome.has_peer_pbft_chain_size = report.has_peer_pbft_chain_size;
  outcome.peer_pbft_chain_size = report.peer_pbft_chain_size;
  outcome.egress_payload_bytes.assign(report.egress_payload_bytes.begin(), report.egress_payload_bytes.end());
  for (const auto& member : report.outcomes) {
    outcome.queued_effect_count += member.decision.queued_effect_count;
    if (outcome.status == 0 && member.decision.status != 0) {
      outcome.status = member.decision.status;
      outcome.error_code = static_cast<std::string>(member.decision.error_code);
    }
    if (!member.has_admission) {
      ++outcome.rejected_count;
    } else if (member.accepted) {
      ++outcome.accepted_count;
    } else if (member.already_present) {
      ++outcome.duplicate_count;
    } else {
      ++outcome.rejected_count;
    }
    if (member.has_slashing_transaction_effect) {
      const auto& effect = member.slashing_transaction_effect;
      (void)executePbftVoteSlashingTransaction(
          PbftVoteSlashingTransaction{effect.status, effect.proof_hash, effect.wallet_index, effect.nonce,
                                      effect.contract_address, effect.value, effect.gas_limit,
                                      std::vector<uint8_t>(effect.call_data.begin(), effect.call_data.end())},
          config);
    }
  }
  drainAndExecuteTransportEffects(request.transport_lane, request.source_payload_id, true, executor);
  return outcome;
}

ConsensusPacketOutcome ConsensusNetworkApi::ingestPbftVotesBundlePacket(const ConsensusPacketRequest& request,
                                                                        const FullNodeConfig& config,
                                                                        const ConsensusTransportExecutor& executor) {
  auto lane_lock = lockTransportLane(request.transport_lane);
  const auto report = impl_->api->consensus_network_ingest_pbft_votes_bundle_packet(
      toNativeConsensusPacketRequest(request), makeSlashingSubmitters(config));
  ConsensusPacketOutcome outcome{};
  outcome.status = report.status;
  outcome.malicious = report.malicious;
  outcome.error_code = static_cast<std::string>(report.error_code);
  outcome.has_peer_pbft_chain_size = report.has_peer_pbft_chain_size;
  outcome.peer_pbft_chain_size = report.peer_pbft_chain_size;
  outcome.egress_payload_bytes.assign(report.egress_payload_bytes.begin(), report.egress_payload_bytes.end());
  for (const auto& member : report.outcomes) {
    outcome.queued_effect_count += member.decision.queued_effect_count;
    if (outcome.status == 0 && member.decision.status != 0) {
      outcome.status = member.decision.status;
      outcome.error_code = static_cast<std::string>(member.decision.error_code);
    }
    if (!member.has_admission) {
      ++outcome.rejected_count;
    } else if (member.accepted) {
      ++outcome.accepted_count;
    } else if (member.already_present) {
      ++outcome.duplicate_count;
    } else {
      ++outcome.rejected_count;
    }
    if (member.has_slashing_transaction_effect) {
      const auto& effect = member.slashing_transaction_effect;
      (void)executePbftVoteSlashingTransaction(
          PbftVoteSlashingTransaction{effect.status, effect.proof_hash, effect.wallet_index, effect.nonce,
                                      effect.contract_address, effect.value, effect.gas_limit,
                                      std::vector<uint8_t>(effect.call_data.begin(), effect.call_data.end())},
          config);
    }
  }
  drainAndExecuteTransportEffects(request.transport_lane, request.source_payload_id, true, executor);
  return outcome;
}

ConsensusPacketOutcome ConsensusNetworkApi::ingestPillarVotePacket(const ConsensusPacketRequest& request,
                                                                   const ConsensusTransportExecutor& executor) {
  auto lane_lock = lockTransportLane(request.transport_lane);
  const auto report = impl_->api->consensus_network_ingest_pillar_vote_packet(toNativeConsensusPacketRequest(request));
  ConsensusPacketOutcome outcome{};
  outcome.status = report.status;
  outcome.malicious = report.malicious;
  outcome.error_code = static_cast<std::string>(report.error_code);
  for (const auto& member : report.outcomes) {
    outcome.queued_effect_count += member.decision.queued_effect_count;
    if (outcome.status == 0 && member.decision.status != 0) {
      outcome.status = member.decision.status;
      outcome.error_code = static_cast<std::string>(member.decision.error_code);
    }
    if (static_cast<std::string>(member.decision.error_code) == "PILLAR_VOTE_INGRESS_MALFORMED_RLP") {
      outcome.malicious = true;
    }
    if (member.accepted) {
      ++outcome.accepted_count;
    } else if (member.duplicate) {
      ++outcome.duplicate_count;
    } else {
      ++outcome.rejected_count;
    }
  }
  drainAndExecuteTransportEffects(request.transport_lane, request.source_payload_id, true, executor);
  return outcome;
}

ConsensusPacketOutcome ConsensusNetworkApi::ingestPillarVotesBundlePacket(const ConsensusPacketRequest& request,
                                                                          const ConsensusTransportExecutor& executor) {
  auto lane_lock = lockTransportLane(request.transport_lane);
  const auto report =
      impl_->api->consensus_network_ingest_pillar_votes_bundle_packet(toNativeConsensusPacketRequest(request));
  ConsensusPacketOutcome outcome{};
  outcome.status = report.status;
  outcome.malicious = report.malicious;
  outcome.error_code = static_cast<std::string>(report.error_code);
  for (const auto& member : report.outcomes) {
    outcome.queued_effect_count += member.decision.queued_effect_count;
    if (outcome.status == 0 && member.decision.status != 0) {
      outcome.status = member.decision.status;
      outcome.error_code = static_cast<std::string>(member.decision.error_code);
    }
    if (static_cast<std::string>(member.decision.error_code) == "PILLAR_VOTE_INGRESS_MALFORMED_RLP") {
      outcome.malicious = true;
    }
    if (member.accepted) {
      ++outcome.accepted_count;
    } else if (member.duplicate) {
      ++outcome.duplicate_count;
    } else {
      ++outcome.rejected_count;
    }
  }
  drainAndExecuteTransportEffects(request.transport_lane, request.source_payload_id, true, executor);
  return outcome;
}

ConsensusPacketOutcome ConsensusNetworkApi::routeConsensusEgress(
    const ConsensusEgressRequest& request, const ConsensusEgressPeerSnapshotProvider& peer_snapshot_provider,
    const ConsensusTransportExecutor& executor) {
  if (!peer_snapshot_provider) {
    throw std::invalid_argument("Consensus egress requires an immutable peer snapshot provider");
  }
  auto lane_lock = lockTransportLane(request.transport_lane);
  const auto source_payload_id = request.source_payload_id == 0
                                     ? impl_->next_egress_payload_id.fetch_add(1, std::memory_order_relaxed)
                                     : request.source_payload_id;
  rustaxa::NetworkEgressPrepareRequest native_request{};
  native_request.family = request.family;
  native_request.transport_lane = request.transport_lane;
  native_request.source_payload_id = source_payload_id;
  native_request.source_peer_id = request.source_peer_id;
  native_request.rebroadcast = request.rebroadcast;
  native_request.object_hash = request.object_hash;
  native_request.payload_bytes.reserve(request.payload_bytes.size());
  for (const auto byte : request.payload_bytes) {
    native_request.payload_bytes.push_back(byte);
  }
  native_request.related_payload_bytes.reserve(request.related_payload_bytes.size());
  for (const auto byte : request.related_payload_bytes) {
    native_request.related_payload_bytes.push_back(byte);
  }

  const auto preparation =
      impl_->api->consensus_network_prepare_egress(impl_->consensus_application->service(), std::move(native_request));
  try {
    std::vector<ConsensusEgressProbe> probes;
    probes.reserve(preparation.probes.size());
    for (const auto& probe : preparation.probes) {
      probes.push_back({probe.probe_id, probe.object_kind, probe.object_hash});
    }
    const auto peers = peer_snapshot_provider(probes);
    rust::Vec<rustaxa::NetworkEgressPeerSnapshot> plan_peers;
    plan_peers.reserve(peers.size());
    for (const auto& peer : peers) {
      rustaxa::NetworkEgressPeerSnapshot native_peer{};
      native_peer.transport_lane = peer.transport_lane == 0 ? request.transport_lane : peer.transport_lane;
      native_peer.peer_id = peer.peer_id;
      native_peer.syncing = peer.syncing;
      native_peer.pbft_chain_size = peer.pbft_chain_size;
      native_peer.dag_level = peer.dag_level;
      native_peer.is_light_node = peer.is_light_node;
      native_peer.light_node_history = peer.light_node_history;
      native_peer.known_probe_ids.reserve(peer.known_probe_ids.size());
      for (const auto probe_id : peer.known_probe_ids) {
        native_peer.known_probe_ids.push_back(probe_id);
      }
      plan_peers.push_back(std::move(native_peer));
    }
    const auto decision = impl_->api->consensus_network_plan_egress(preparation.token, std::move(plan_peers));
    const auto execution = drainAndExecuteTransportEffects(request.transport_lane, source_payload_id, true, executor);
    const auto status = execution.failed_effect_count == 0 ? decision.status : uint8_t{1};
    const auto error_code = execution.failed_effect_count == 0 ? static_cast<std::string>(decision.error_code)
                                                               : std::string{"NETWORK_EGRESS_TRANSPORT_PARTIAL"};
    return ConsensusPacketOutcome{status, false, decision.queued_effect_count, 0, 0, 0, false, 0, error_code, {}};
  } catch (...) {
    try {
      impl_->api->consensus_network_cancel_egress(preparation.token);
    } catch (...) {
      // Preserve the original snapshot, planning, or transport failure.
    }
    throw;
  }
}

ConsensusPacketOutcome ConsensusNetworkApi::requestPillarVotesBundle(
    uint64_t local_pbft_syncing_period, uint64_t period, const std::array<uint8_t, 32>& pillar_block_hash,
    const ConsensusEgressPeerSnapshotProvider& peer_snapshot_provider, const ConsensusTransportExecutor& executor) {
  if (!peer_snapshot_provider) {
    throw std::invalid_argument("Pillar-vote request requires an immutable peer snapshot provider");
  }
  const auto source_payload_id = impl_->next_egress_payload_id.fetch_add(1, std::memory_order_relaxed);
  rustaxa::NetworkEgressPrepareRequest request{};
  request.family = kEgressFamilyPillarVotesRequest;
  request.source_payload_id = source_payload_id;
  request.object_hash = pillar_block_hash;
  for (int shift = 56; shift >= 0; shift -= 8) {
    request.payload_bytes.push_back(static_cast<uint8_t>(period >> shift));
    request.related_payload_bytes.push_back(static_cast<uint8_t>(local_pbft_syncing_period >> shift));
  }
  const auto preparation =
      impl_->api->consensus_network_prepare_egress(impl_->consensus_application->service(), std::move(request));
  try {
    const auto peers = peer_snapshot_provider({});
    rust::Vec<rustaxa::NetworkEgressPeerSnapshot> native_peers;
    native_peers.reserve(peers.size());
    std::vector<uint32_t> lanes;
    for (const auto& peer : peers) {
      rustaxa::NetworkEgressPeerSnapshot native_peer{};
      native_peer.transport_lane = peer.transport_lane;
      native_peer.peer_id = peer.peer_id;
      native_peer.syncing = peer.syncing;
      native_peer.pbft_chain_size = peer.pbft_chain_size;
      native_peer.dag_level = peer.dag_level;
      native_peer.is_light_node = peer.is_light_node;
      native_peer.light_node_history = peer.light_node_history;
      native_peers.push_back(std::move(native_peer));
      if (std::find(lanes.begin(), lanes.end(), peer.transport_lane) == lanes.end()) {
        lanes.push_back(peer.transport_lane);
      }
    }
    const auto decision = impl_->api->consensus_network_plan_egress(preparation.token, std::move(native_peers));
    uint32_t failed_effect_count = 0;
    for (const auto lane : lanes) {
      auto lane_lock = lockTransportLane(lane);
      failed_effect_count +=
          drainAndExecuteTransportEffects(lane, source_payload_id, true, executor).failed_effect_count;
    }
    return ConsensusPacketOutcome{
        failed_effect_count == 0 ? decision.status : uint8_t{1},
        false,
        decision.queued_effect_count,
        0,
        0,
        0,
        false,
        0,
        failed_effect_count == 0 ? static_cast<std::string>(decision.error_code) : "NETWORK_EGRESS_TRANSPORT_PARTIAL",
        {}};
  } catch (...) {
    try {
      impl_->api->consensus_network_cancel_egress(preparation.token);
    } catch (...) {
      // Preserve the original planning or transport failure.
    }
    throw;
  }
}

TransactionPacketOutcome ConsensusNetworkApi::ingestTransactionPacket(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t source_payload_id,
    const std::vector<uint8_t>& packet_rlp, const FullNodeConfig& config, const ConsensusTransportExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto last_block_number = (*impl_->query)->consensus_query_final_chain_last_block_number();
  rustaxa::NetworkTransactionPacketRequest request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.packet_rlp.reserve(packet_rlp.size());
  for (const auto byte : packet_rlp) {
    request.packet_rlp.push_back(byte);
  }
  request.expected_chain_id = config.genesis.chain_id;
  request.maximum_gas_limit = config.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
  request.minimum_gas_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  request.last_block_number = last_block_number;
  request.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(last_block_number);
  const auto report = rustaxa::consensus_network_ingest_transaction_packet(
      *impl_->api, impl_->consensus_application->service(), std::move(request), impl_->external_evm);

  for (const auto& member : report.transactions) {
    if (member.observe_transaction && impl_->observers.transaction_observed) {
      try {
        impl_->observers.transaction_observed(
            trx_hash_t(member.submission.transaction_hash.data(), trx_hash_t::ConstructFromPointer));
      } catch (const std::exception&) {
        // Public observation is post-commit and best-effort. It must never
        // change native admission or transport acknowledgement.
      }
    }
  }

  drainAndExecuteTransportEffects(transport_lane, source_payload_id, true, executor);

  return TransactionPacketOutcome{report.decision.status, report.decision.queued_effect_count,
                                  report.transactions.size(), static_cast<std::string>(report.decision.error_code)};
}

GetDagSyncOutcome ConsensusNetworkApi::serveGetDagSyncRequest(uint32_t transport_lane,
                                                              const std::array<uint8_t, 64>& peer_id,
                                                              uint64_t source_payload_id, bool request_allowed,
                                                              const std::vector<uint8_t>& request_rlp,
                                                              const GetDagSyncExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  rustaxa::NetworkGetDagSyncRequest request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.request_allowed = request_allowed;
  request.request_rlp.reserve(request_rlp.size());
  for (const auto byte : request_rlp) {
    request.request_rlp.push_back(byte);
  }
  const auto decision = impl_->api->consensus_network_ingest_get_dag_sync_request(
      impl_->consensus_application->service(), std::move(request));
  drainAndExecuteTransportEffects(
      transport_lane, source_payload_id, true,
      ConsensusTransportExecutor{[&executor, &peer_id](const ConsensusTransportEffect& effect) {
        if (effect.peer_id != peer_id || effect.kind != kEffectSendPacket || effect.packet_kind != kPacketDagSync) {
          throw std::runtime_error("Get-DAG-sync executor received a mismatched effect");
        }
        if (!executor.send_response(effect.payload_bytes, effect.sync_start, effect.period)) {
          throw std::runtime_error("DAG-sync response transport failed");
        }
        return ConsensusTransportExecutionResult{};
      }});
  return GetDagSyncOutcome{decision.status, decision.queued_effect_count,
                           static_cast<std::string>(decision.error_code)};
}

DagBlockPacketOutcome ConsensusNetworkApi::ingestDagBlockPacket(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t source_payload_id,
    const std::vector<uint8_t>& packet_rlp, bool rebroadcast, const DagBlockPeerFacts& peer_facts,
    const FullNodeConfig& config, const DagPacketExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto last_block_number = (*impl_->query)->consensus_query_final_chain_last_block_number();
  rustaxa::NetworkDagPacketRequest request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.packet_rlp.reserve(packet_rlp.size());
  for (const auto byte : packet_rlp) {
    request.packet_rlp.push_back(byte);
  }
  request.expected_chain_id = config.genesis.chain_id;
  request.maximum_gas_limit = config.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
  request.minimum_gas_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  request.last_block_number = last_block_number;
  request.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(last_block_number);
  request.rebroadcast = rebroadcast;
  request.peer_dag_synced = peer_facts.peer_dag_synced;
  request.dag_sync_allowed = peer_facts.dag_sync_allowed;
  request.transactions_dropped = peer_facts.transactions_dropped;
  request.pending_dag_request = peer_facts.pending_dag_request;
  request.local_pbft_syncing = peer_facts.local_pbft_syncing;
  const auto report = rustaxa::consensus_network_ingest_dag_block_packet(
      *impl_->api, impl_->consensus_application->service(), std::move(request), impl_->external_evm);

  if (report.admission_found && report.admission.observe_block && impl_->observers.dag_block_observed) {
    try {
      impl_->observers.dag_block_observed(
          std::vector<uint8_t>(report.admission.block_rlp.begin(), report.admission.block_rlp.end()));
    } catch (const std::exception&) {
      // Public observation is post-commit and best-effort.
    }
  }
  uint32_t queued_effect_count = report.decision.queued_effect_count;
  const ConsensusTransportExecutor transport_executor{[&executor](const ConsensusTransportEffect& effect) {
    if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectTransaction) {
      executor.mark_transaction_known(effect.peer_id, effect.object_hash);
    } else if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectDagBlock) {
      executor.mark_dag_block_known(effect.peer_id, effect.object_hash);
    } else if (effect.kind == kEffectSendPacket && effect.packet_kind == kPacketDagBlock) {
      if (!executor.send_packet(effect.peer_id, effect.payload_bytes)) {
        throw std::runtime_error("DAG-block transport failed");
      }
    } else {
      throw std::runtime_error("DAG-block executor received an unsupported effect");
    }
    return ConsensusTransportExecutionResult{};
  }};
  if (report.admission_found && report.admission.accepted && report.admission.gossip_block && rebroadcast) {
    ConsensusEgressRequest egress{};
    egress.family = 3;
    egress.transport_lane = transport_lane;
    egress.source_payload_id = source_payload_id;
    egress.source_peer_id = peer_id;
    egress.rebroadcast = true;
    egress.object_hash = report.admission.block_hash;
    egress.payload_bytes.assign(report.admission.block_rlp.begin(), report.admission.block_rlp.end());
    const ConsensusEgressPeerSnapshotProvider snapshot_provider{[&executor](const auto& probes) {
      return executor.gossip_snapshot ? executor.gossip_snapshot(probes) : std::vector<ConsensusEgressPeerSnapshot>{};
    }};
    lane_lock.unlock();
    const auto gossip_outcome = routeConsensusEgress(egress, snapshot_provider, transport_executor);
    queued_effect_count += gossip_outcome.queued_effect_count;
  } else {
    drainAndExecuteTransportEffects(transport_lane, source_payload_id, true, transport_executor);
  }
  std::optional<DagBlockAdmissionOutcome> admission;
  if (report.admission_found) {
    admission =
        DagBlockAdmissionOutcome{report.admission.block_hash, report.admission.block_level, report.admission.accepted,
                                 report.admission.duplicate, report.admission.reject_code};
  }
  return DagBlockPacketOutcome{report.decision.status, queued_effect_count, report.rejection_action,
                               static_cast<std::string>(report.decision.error_code), std::move(admission)};
}

DagSyncPacketOutcome ConsensusNetworkApi::ingestDagSyncPacket(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t source_payload_id,
    const std::vector<uint8_t>& packet_rlp, const FullNodeConfig& config, const DagPacketExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto last_block_number = (*impl_->query)->consensus_query_final_chain_last_block_number();
  rustaxa::NetworkDagPacketRequest request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.packet_rlp.reserve(packet_rlp.size());
  for (const auto byte : packet_rlp) {
    request.packet_rlp.push_back(byte);
  }
  request.expected_chain_id = config.genesis.chain_id;
  request.maximum_gas_limit = config.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
  request.minimum_gas_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  request.last_block_number = last_block_number;
  request.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(last_block_number);
  request.rebroadcast = false;
  request.local_pbft_syncing = false;
  const auto report = rustaxa::consensus_network_ingest_dag_sync_packet(
      *impl_->api, impl_->consensus_application->service(), std::move(request), impl_->external_evm);

  for (const auto& transaction : report.transactions) {
    if (transaction.observe_transaction && impl_->observers.transaction_observed) {
      try {
        impl_->observers.transaction_observed(
            trx_hash_t(transaction.submission.transaction_hash.data(), trx_hash_t::ConstructFromPointer));
      } catch (const std::exception&) {
        // Public observation is post-commit and best-effort.
      }
    }
  }
  for (const auto& block : report.blocks) {
    if (block.observe_block && impl_->observers.dag_block_observed) {
      try {
        impl_->observers.dag_block_observed(std::vector<uint8_t>(block.block_rlp.begin(), block.block_rlp.end()));
      } catch (const std::exception&) {
        // Public observation is post-commit and best-effort.
      }
    }
  }
  drainAndExecuteTransportEffects(
      transport_lane, source_payload_id, true,
      ConsensusTransportExecutor{[&executor](const ConsensusTransportEffect& effect) {
        if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectTransaction) {
          executor.mark_transaction_known(effect.peer_id, effect.object_hash);
        } else if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectDagBlock) {
          executor.mark_dag_block_known(effect.peer_id, effect.object_hash);
        } else {
          throw std::runtime_error("DAG-sync executor received an unsupported effect");
        }
        return ConsensusTransportExecutionResult{};
      }});
  std::vector<DagBlockAdmissionOutcome> blocks;
  blocks.reserve(report.blocks.size());
  for (const auto& block : report.blocks) {
    blocks.push_back(DagBlockAdmissionOutcome{block.block_hash, block.block_level, block.accepted, block.duplicate,
                                              block.reject_code});
  }
  return DagSyncPacketOutcome{report.decision.status,
                              report.decision.queued_effect_count,
                              static_cast<std::string>(report.decision.error_code),
                              report.request_period,
                              report.response_period,
                              std::move(blocks)};
}

PendingDagBlocksOutcome ConsensusNetworkApi::requestPendingDagBlocks(
    uint32_t transport_lane, uint64_t local_pbft_syncing_period, const std::vector<ConsensusPeerCandidate>& candidates,
    const PendingDagBlocksExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto source_payload_id = impl_->next_egress_payload_id.fetch_add(1, std::memory_order_relaxed);
  rustaxa::NetworkPendingDagBlocksRequestFacts facts{};
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
  const auto decision = impl_->api->consensus_network_request_pending_dag_blocks(
      impl_->consensus_application->service(), transport_lane, source_payload_id, std::move(facts));

  drainAndExecuteTransportEffects(
      transport_lane, source_payload_id, true,
      ConsensusTransportExecutor{[&candidates, &executor](const ConsensusTransportEffect& effect) {
        if (effect.kind != kEffectSendPacket || effect.packet_kind != kPacketGetDagSync ||
            std::none_of(candidates.begin(), candidates.end(),
                         [&effect](const auto& candidate) { return candidate.peer_id == effect.peer_id; })) {
          throw std::runtime_error("Pending-DAG executor received a mismatched effect");
        }
        if (!executor.send_request(effect.peer_id, effect.payload_bytes, effect.period)) {
          throw std::runtime_error("Pending-DAG request transport failed");
        }
        return ConsensusTransportExecutionResult{};
      }});
  return PendingDagBlocksOutcome{decision.status, decision.queued_effect_count,
                                 static_cast<std::string>(decision.error_code)};
}

bool ConsensusNetworkApi::submitSlashingTransaction(size_t wallet_index, const std::array<uint8_t, 32>& nonce_bytes,
                                                    const std::array<uint8_t, 20>& contract_address,
                                                    const std::array<uint8_t, 32>& value_bytes, uint64_t gas_limit,
                                                    const std::vector<uint8_t>& call_data,
                                                    const FullNodeConfig& config) {
  const auto& wallet = config.wallets[wallet_index];
  const auto nonce = dev::fromBigEndian<u256>(dev::bytes(nonce_bytes.begin(), nonce_bytes.end()));
  const auto value = dev::fromBigEndian<u256>(dev::bytes(value_bytes.begin(), value_bytes.end()));
  const addr_t contract(contract_address.data(), addr_t::ConstructFromPointer);
  const auto gas_price_bytes =
      rustaxa::consensus_application_transaction_gas_price_bid(impl_->consensus_application->service());
  const auto gas_price = dev::fromBigEndian<u256>(dev::bytes(gas_price_bytes.begin(), gas_price_bytes.end()));
  const auto transaction = std::make_shared<Transaction>(nonce, value, gas_price, gas_limit, call_data,
                                                         wallet.node_secret, contract, config.genesis.chain_id);
  const auto last_block_number = (*impl_->query)->consensus_query_final_chain_last_block_number();
  rustaxa::PublicTransactionSubmissionRequest request{};
  request.transaction_rlp = toRustBytes(transaction->rlp());
  request.expected_chain_id = config.genesis.chain_id;
  request.maximum_gas_limit = config.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
  request.minimum_gas_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  request.last_block_number = last_block_number;
  request.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(last_block_number);
  const auto submission = rustaxa::consensus_application_submit_transaction_with_execution(
      impl_->consensus_application->service(), std::move(request), impl_->external_evm);
  return submission.accepted;
}

ConsensusNetworkApi::TransportDrainOutcome ConsensusNetworkApi::drainAndExecuteTransportEffects(
    uint32_t transport_lane, uint64_t source_payload_id, bool source_scoped,
    const ConsensusTransportExecutor& executor) {
  if (!executor.execute) {
    throw std::invalid_argument("Consensus transport executor has no physical execution callback");
  }

  TransportDrainOutcome outcome{};
  std::unordered_set<uint64_t> observed_effect_ids;
  while (true) {
    const auto batch =
        impl_->api->consensus_network_drain_work(transport_lane, source_payload_id, source_scoped, kEffectDrainBudget);
    if (batch.status != 0) {
      throw std::runtime_error("Network API rejected effect drain: " + static_cast<std::string>(batch.error_code));
    }
    if (batch.effects.empty()) {
      if (batch.more_available) {
        throw std::runtime_error("Network API reported more effects after returning an empty drain batch");
      }
      break;
    }

    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      if (effect.effect_id == 0 || !observed_effect_ids.insert(effect.effect_id).second) {
        throw std::runtime_error("Network API returned a missing or duplicate effect id");
      }
      if (effect.transport_lane != transport_lane || (source_scoped && effect.source_payload_id != source_payload_id)) {
        throw std::runtime_error("Network API returned an effect outside the requested lane or payload scope");
      }

      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;
      try {
        {
          ConsensusTransportEffect physical{};
          physical.effect_id = effect.effect_id;
          physical.source_payload_id = effect.source_payload_id;
          physical.transport_lane = effect.transport_lane;
          physical.kind = effect.kind;
          physical.peer_id = effect.peer_id;
          physical.packet_kind = effect.packet_kind;
          physical.payload_bytes.assign(effect.payload_bytes.begin(), effect.payload_bytes.end());
          physical.object_kind = effect.object_kind;
          physical.object_hash = effect.object_hash;
          physical.sync_kind = effect.sync_kind;
          physical.sync_start = effect.sync_start;
          physical.reason_code = effect.reason_code;
          physical.dependency_id = effect.dependency_id;
          physical.period = effect.period;
          physical.round = effect.round;
          const auto execution = executor.execute(physical);
          if (!execution.success) {
            throw std::runtime_error(execution.diagnostic.empty() ? "Consensus transport effect failed"
                                                                  : execution.diagnostic);
          }
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      if (result.status == kEffectResultOk) {
        ++outcome.successful_effect_count;
      } else {
        ++outcome.failed_effect_count;
      }
      results.push_back(std::move(result));
    }

    const auto expected_results = results.size();
    const auto acknowledgement = impl_->api->consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0 || acknowledgement.accepted_results != expected_results ||
        acknowledgement.failed_results > expected_results) {
      throw std::runtime_error("Network API rejected or incompletely acknowledged executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }
  return outcome;
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

ConsensusPacketOutcome ConsensusNetworkApi::ingestGetPillarVotesBundleRequest(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t source_payload_id,
    const std::vector<uint8_t>& packet_rlp, const ConsensusTransportExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  rustaxa::NetworkCanonicalRequestPacket request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.packet_rlp.reserve(packet_rlp.size());
  std::copy(packet_rlp.begin(), packet_rlp.end(), std::back_inserter(request.packet_rlp));
  const auto decision = impl_->api->consensus_network_ingest_pillar_votes_bundle_request(std::move(request));
  drainAndExecuteTransportEffects(transport_lane, source_payload_id, true, executor);
  return ConsensusPacketOutcome{decision.status,
                                false,
                                decision.queued_effect_count,
                                0,
                                0,
                                0,
                                false,
                                0,
                                static_cast<std::string>(decision.error_code),
                                {}};
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
  const auto decision = impl_->api->consensus_network_ingest_get_pbft_sync_request(std::move(request));

  drainAndExecuteTransportEffects(
      tarcap_version, source_payload_id, true,
      ConsensusTransportExecutor{[&executor, &peer_id](const ConsensusTransportEffect& effect) {
        if (effect.peer_id != peer_id) {
          throw std::runtime_error("PBFT sync effect targets a different peer");
        }
        if (effect.kind == kEffectSendPacket &&
            (effect.packet_kind == kPacketPbftSync || effect.packet_kind == kPacketPbftBlocksBundle)) {
          if (!executor.send_packet(effect.packet_kind, effect.payload_bytes)) {
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
        return ConsensusTransportExecutionResult{};
      }});

  return PbftSyncRequestOutcome{decision.status, decision.queued_effect_count,
                                static_cast<std::string>(decision.error_code)};
}

ConsensusPacketOutcome ConsensusNetworkApi::ingestPbftNextVotesRequest(uint32_t transport_lane,
                                                                       const std::array<uint8_t, 64>& peer_id,
                                                                       uint64_t source_payload_id,
                                                                       const std::vector<uint8_t>& packet_rlp,
                                                                       const ConsensusTransportExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  rustaxa::NetworkCanonicalRequestPacket request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.packet_rlp.reserve(packet_rlp.size());
  std::copy(packet_rlp.begin(), packet_rlp.end(), std::back_inserter(request.packet_rlp));
  const auto decision = impl_->api->consensus_network_ingest_pbft_next_votes_bundle_request(std::move(request));
  const auto execution = drainAndExecuteTransportEffects(transport_lane, source_payload_id, true, executor);
  return {decision.status,
          decision.status == 11,
          decision.queued_effect_count,
          0,
          0,
          execution.failed_effect_count,
          false,
          0,
          static_cast<std::string>(decision.error_code),
          {}};
}

PbftBlocksBundleOutcome ConsensusNetworkApi::admitPbftBlocksBundle(const std::vector<uint8_t>& packet_rlp,
                                                                   uint64_t source_payload_id) {
  rust::Vec<uint8_t> bridge_packet;
  bridge_packet.reserve(packet_rlp.size());
  std::copy(packet_rlp.begin(), packet_rlp.end(), std::back_inserter(bridge_packet));
  const auto decision = impl_->api->consensus_network_ingest_pbft_blocks_bundle(
      impl_->consensus_application->service(), std::move(bridge_packet), source_payload_id);
  return PbftBlocksBundleOutcome{decision.status, static_cast<std::string>(decision.error_code)};
}

std::optional<std::array<uint8_t, 64>> ConsensusNetworkApi::selectMaxChainPeer(
    uint64_t local_pbft_syncing_period, const std::vector<ConsensusPeerCandidate>& candidates) const {
  rustaxa::NetworkPbftSyncStartRequest request{};
  request.start = false;
  request.local_pbft_synced_period = local_pbft_syncing_period;
  request.candidates.reserve(candidates.size());
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
    request.candidates.push_back(std::move(bridge_candidate));
  }

  const auto outcome = impl_->api->consensus_network_begin_pbft_sync(request);
  if (!outcome.has_peer) {
    return std::nullopt;
  }
  return outcome.peer_id;
}

}  // namespace taraxa::network
#endif
