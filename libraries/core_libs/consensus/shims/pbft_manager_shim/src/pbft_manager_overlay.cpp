#ifdef RUSTAXA_ENABLE

#include <libdevcore/SHA3.h>

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <exception>
#include <limits>
#include <optional>
#include <stdexcept>
#include <string>
#include <unordered_set>
#include <vector>

#include "common/thread_pool.hpp"
#include "config/version.hpp"
#include "dag/dag_manager.hpp"
#include "final_chain/final_chain.hpp"
#include "network/network.hpp"
#include "pbft/pbft_manager.hpp"
#include "pbft/period_data.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"
#include "transaction/transaction.hpp"
#include "vote_manager/vote_manager.hpp"

namespace taraxa {
using namespace std::chrono_literals;

constexpr std::chrono::milliseconds kPollingIntervalMs{100};
constexpr PbftStep kMaxSteps{13};  // Need to be a odd number

namespace {

constexpr std::optional<PbftPeriod> checkedNextPbftPeriod(PbftPeriod finalized_chain_size) {
  if (finalized_chain_size == std::numeric_limits<PbftPeriod>::max()) {
    return std::nullopt;
  }
  return finalized_chain_size + 1;
}

static_assert(checkedNextPbftPeriod(1) == 2);
static_assert(!checkedNextPbftPeriod(std::numeric_limits<PbftPeriod>::max()).has_value());

constexpr uint8_t kPbftSyncDposFactsReady = 0;
constexpr uint8_t kPbftSyncFactValid = 0;
constexpr uint8_t kPbftSyncFactInvalid = 1;
constexpr uint8_t kPbftSyncRuntimeCheckFinalChainHash = 1;
constexpr uint8_t kPbftSyncRuntimeCheckCertVotes = 3;
constexpr uint8_t kPbftSyncRuntimeCheckTransactions = 4;
constexpr uint8_t kPbftSyncRuntimeCheckPillarVotes = 6;

constexpr uint8_t kPbftFinalizationStatusAccepted = 0;
constexpr uint8_t kPbftFinalizationStorageStagePrimary = 0;
constexpr uint8_t kPbftFinalizationRuntimeStatusActive = 0;
constexpr uint8_t kPbftFinalizationRuntimeStatusComplete = 1;
constexpr uint8_t kPbftFinalizationRuntimeActionCommitRewardVotesReset = 3;
constexpr uint8_t kPbftFinalizationRuntimeActionSetDagBlockOrder = 4;
constexpr uint8_t kPbftFinalizationRuntimeActionUpdateFinalizedTransactions = 5;
constexpr uint8_t kPbftFinalizationRuntimeActionFinalizeFinalChain = 9;
constexpr uint8_t kPbftFinalizationRuntimeActionAdvancePeriod = 12;
constexpr uint8_t kPbftFinalizationRuntimeActionCommitSortitionRuntime = 14;
constexpr uint8_t kPbftFinalizationRuntimeActionProcessPillarBlock = 15;
constexpr uint8_t kPbftManagerRuntimeStatusActive = 0;
constexpr uint8_t kPbftManagerRuntimeStatusComplete = 1;
constexpr uint8_t kPbftManagerRuntimeActionProcessSyncedPbftBlocks = 0;
constexpr uint8_t kPbftManagerRuntimeActionMaybeBroadcastVotes = 1;
constexpr uint8_t kPbftManagerRuntimeActionTryPushCertVotesBlock = 2;
constexpr uint8_t kPbftManagerRuntimeActionTryAdvanceRound = 3;
constexpr uint8_t kPbftManagerRuntimeActionSleepIneligiblePollingInterval = 4;
constexpr uint8_t kPbftManagerRuntimeActionRunValueProposal = 5;
constexpr uint8_t kPbftManagerRuntimeActionTransitionToFilter = 6;
constexpr uint8_t kPbftManagerRuntimeActionRunFilter = 7;
constexpr uint8_t kPbftManagerRuntimeActionTransitionToCertify = 8;
constexpr uint8_t kPbftManagerRuntimeActionRunCertify = 9;
constexpr uint8_t kPbftManagerRuntimeActionTransitionToFinish = 10;
constexpr uint8_t kPbftManagerRuntimeActionDelayCertifyPoll = 11;
constexpr uint8_t kPbftManagerRuntimeActionRunFirstFinish = 12;
constexpr uint8_t kPbftManagerRuntimeActionTransitionToFinishPolling = 13;
constexpr uint8_t kPbftManagerRuntimeActionRunSecondFinish = 14;
constexpr uint8_t kPbftManagerRuntimeActionLoopBackFinish = 15;
constexpr uint8_t kPbftManagerRuntimeActionDelayFinishPoll = 16;
constexpr uint8_t kPbftManagerRuntimeActionSleepUntilNextStep = 17;
constexpr uint8_t kPbftManagerRuntimeActionResetConsensus = 18;
constexpr uint8_t kPbftManagerRuntimeResultNoProgressContinue = 0;
constexpr uint8_t kPbftManagerRuntimeResultProgressRestartLoop = 1;
constexpr uint8_t kPbftManagerRuntimeResultStateActionDone = 2;
constexpr uint8_t kPbftManagerRuntimeResultTransitionApplied = 3;
constexpr uint8_t kPbftManagerRuntimeResultSleepApplied = 4;
constexpr uint8_t kPbftManagerRuntimeResultExecutorError = 255;
constexpr uint8_t kPbftManagerRuntimeSnapshotStatusReady = 0;
constexpr uint8_t kPbftManagerStateActionIntentProposeNewBlock = 1;
constexpr uint8_t kPbftManagerStateActionIntentReproposePreviousRoundNextValue = 2;
constexpr uint8_t kPbftManagerStateActionIntentIdentifyLeaderAndSoftVote = 3;
constexpr uint8_t kPbftManagerStateActionIntentSoftVotePreviousRoundNextValue = 4;
constexpr uint8_t kPbftManagerStateActionIntentCertVoteCurrentSoftValue = 5;
constexpr uint8_t kPbftManagerStateActionIntentGoFinish = 6;
constexpr uint8_t kPbftManagerStateActionIntentNextVoteCertVotedBlock = 7;
constexpr uint8_t kPbftManagerStateActionIntentNextVoteNullBlock = 8;
constexpr uint8_t kPbftManagerStateActionIntentNextVotePreviousRoundValue = 9;
constexpr uint8_t kPbftManagerStateActionIntentNextVoteCurrentSoftValue = 10;
constexpr uint8_t kPbftManagerStateActionEffectResultApplied = 0;
constexpr uint8_t kPbftManagerStateActionEffectResultSkippedNoWork = 1;
constexpr uint8_t kPbftManagerStateActionEffectResultSkippedMissingLiveObject = 2;
constexpr uint8_t kPbftManagerStateActionEffectResultRejectedLiveCheck = 3;
constexpr uint8_t kPbftManagerStateActionEffectResultExecutorError = 255;
constexpr uint8_t kPbftManagerProposalActionBuildProposal = 1;
constexpr uint8_t kPbftManagerProposalActionSkipProposal = 2;
constexpr uint8_t kPbftManagerProposalActionContractError = 255;
constexpr uint8_t kPbftManagerProposalStatusBuildReady = 1;
constexpr uint8_t kPbftManagerBroadcastActionNoop = 0;
constexpr uint8_t kPbftManagerBroadcastActionPeriodVotes = 1;
constexpr uint8_t kPbftManagerBroadcastActionRoundVotes = 2;
constexpr uint8_t kPbftManagerBroadcastStatusReady = 0;
constexpr uint8_t kPbftSyncQueueDrainActionPopAndProcess = 1;
constexpr uint8_t kPbftSyncQueueDrainActionPushAccepted = 2;
constexpr uint8_t kPbftSyncQueueDrainActionUpdateSyncState = 3;
constexpr uint8_t kPbftSyncQueueDrainActionStop = 4;
constexpr uint8_t kPbftSyncQueueDrainStatusActive = 0;
constexpr uint8_t kPbftSyncQueueDrainStatusComplete = 1;
constexpr uint8_t kPbftSyncTransactionWarningMissingTransaction = 1;
constexpr uint8_t kPbftSyncTransactionWarningFinalizedTransaction = 2;
constexpr uint8_t kPbftSyncCertBundleActionAwaitingSlashing = 0;
constexpr uint8_t kPbftSyncCertBundleActionAccepted = 1;
constexpr uint8_t kPbftSyncCertBundleActionRejected = 2;
constexpr uint8_t kPbftSyncCertBundleCommandBegin = 0;
constexpr uint8_t kPbftSyncCertBundleCommandReportSlashing = 1;
constexpr uint8_t kPbftSyncCertBundleCommandAbort = 2;
constexpr uint8_t kPbftManagerStartupRestoreStatusReady = 0;
constexpr uint8_t kPbftManagerTransitionStorageStatusApplied = 0;
constexpr uint8_t kPbftFinalizationExecutorModeFresh = 0;
constexpr uint8_t kPbftFinalizationExecutorModeResume = 1;
constexpr uint8_t kPbftManagerAdvancePeriodActionApplyExecutedBlockReset = 1;
constexpr uint8_t kPbftManagerAdvancePeriodActionSetVoteManagerPeriodRound = 2;
constexpr uint8_t kPbftManagerAdvancePeriodActionResetCurrentRoundTimer = 3;
constexpr uint8_t kPbftManagerAdvancePeriodActionResetRewardVoteCounters = 4;
constexpr uint8_t kPbftManagerAdvancePeriodActionResetPeriodTimer = 5;

constexpr uint8_t kPbftManagerAdvancePeriodActionUpdateWalletEligibility = 6;
constexpr uint8_t kPbftManagerTransitionResetConsensus = 0;
constexpr uint8_t kPbftManagerTransitionToFilter = 1;
constexpr uint8_t kPbftManagerTransitionToCertify = 2;
constexpr uint8_t kPbftManagerTransitionToFinish = 3;
constexpr uint8_t kPbftManagerTransitionToFinishPolling = 4;
constexpr uint8_t kPbftManagerTransitionLoopBackFinish = 5;
constexpr uint8_t kPbftManagerTransitionDelayCertifyPoll = 6;
constexpr uint8_t kPbftManagerTransitionDelayFinishPoll = 7;
constexpr uint8_t kPbftManagerCandidateAdmissionValidationNotChecked = 0;
constexpr uint8_t kPbftManagerCandidateAdmissionValidationValid = 1;
constexpr uint8_t kPbftManagerCandidateAdmissionValidationInvalid = 2;
constexpr uint8_t kPbftManagerCandidateAdmissionActionRequestLookup = 0;
constexpr uint8_t kPbftManagerCandidateAdmissionActionRequestValidation = 1;
constexpr uint8_t kPbftManagerCandidateAdmissionActionAccept = 2;
constexpr uint8_t kPbftManagerCandidateAdmissionActionReject = 3;
constexpr uint8_t kPbftManagerCandidateAdmissionActionDeferMissingBlock = 4;
constexpr uint8_t kPbftManagerCandidateAdmissionActionContractError = 255;
constexpr uint8_t kPbftManagerBlockValidationActionAccept = 1;
constexpr uint8_t kPbftManagerBlockValidationActionReject = 2;
constexpr uint8_t kPbftManagerBlockValidationActionWaitForFinalization = 3;
constexpr uint8_t kPbftManagerBlockValidationActionContractError = 255;
constexpr uint8_t kPillarAnchorDecisionSelectPreviousPeriod = 1;
constexpr uint8_t kPillarAnchorDecisionRestartPostProcessing = 2;

// Returns the shared PBFT service after pillar bootstrap has completed.
// Production decision paths fail explicitly instead of constructing or
// falling back to a separate runtime.
const rustaxa::BridgePbftService &requireReadyPillarService(const SharedPbftService &service) {
  if (!service || !service->service().pbft_service_pillar_ready()) {
    throw std::runtime_error("PBFT_SERVICE_PILLAR_UNAVAILABLE");
  }
  return service->service();
}

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t &hash) { return hash.asArray(); }

template <size_t N, typename FixedHash>
std::array<uint8_t, N> toBridgeFixedBytes(const FixedHash &value) {
  return value.asArray();
}

uint256_hash_t fromBridgeHash(const std::array<uint8_t, 32> &hash) {
  return uint256_hash_t(hash.data(), uint256_hash_t::ConstructFromPointer);
}

rust::Vec<rustaxa::PbftFinalizationHash> toBridgeHashes(const std::vector<blk_hash_t> &hashes) {
  rust::Vec<rustaxa::PbftFinalizationHash> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    rustaxa::PbftFinalizationHash bridge_hash;
    bridge_hash.hash = toBridgeHash(hash);
    out.push_back(bridge_hash);
  }
  return out;
}

uint64_t toBroadcastElapsedMs(std::chrono::milliseconds elapsed) {
  if (elapsed.count() <= 0) {
    return 0;
  }
  return static_cast<uint64_t>(elapsed.count());
}

uint64_t rustFinalChainLastBlockNumber(const std::shared_ptr<final_chain::FinalChain> &final_chain) {
  if (!final_chain) {
    throw std::runtime_error("PBFT manager requires FinalChain for Rust FinalChain height facts");
  }
  return final_chain->lastBlockNumber();
}

uint8_t toPbftManagerRuntimeState(PbftStates state) {
  switch (state) {
    case value_proposal_state:
      return 0;
    case filter_state:
      return 1;
    case certify_state:
      return 2;
    case finish_state:
      return 3;
    case finish_polling_state:
      return 4;
  }
  return 254;
}

PbftStates fromPbftManagerRuntimeState(uint8_t state) {
  switch (state) {
    case 0:
      return value_proposal_state;
    case 1:
      return filter_state;
    case 2:
      return certify_state;
    case 3:
      return finish_state;
    case 4:
      return finish_polling_state;
    default:
      throw std::runtime_error("Unsupported Rust PBFT manager state code " + std::to_string(state));
  }
}

bool pbftManagerRuntimeActionMatchesLiveState(uint8_t action, PbftStates state) {
  switch (action) {
    case kPbftManagerRuntimeActionRunValueProposal:
      return state == value_proposal_state;
    case kPbftManagerRuntimeActionRunFilter:
      return state == filter_state;
    case kPbftManagerRuntimeActionRunCertify:
      return state == certify_state;
    case kPbftManagerRuntimeActionRunFirstFinish:
      return state == finish_state;
    case kPbftManagerRuntimeActionRunSecondFinish:
      return state == finish_polling_state;
    default:
      return true;
  }
}

void applyPbftManagerRuntimeSnapshot(const rustaxa::PbftManagerRuntimeSnapshot &snapshot, std::atomic<PbftRound> &round,
                                     PbftStep &step, PbftStates &state, std::chrono::milliseconds &current_round_lambda,
                                     std::chrono::milliseconds &next_step_time, uint32_t &rounds_count_dynamic_lambda,
                                     uint32_t &dynamic_lambda, bool &executed_pbft_block,
                                     bool &already_next_voted_value, bool &already_next_voted_null_block_hash,
                                     uint32_t &broadcast_votes_counter, uint32_t &rebroadcast_votes_counter,
                                     uint32_t &broadcast_reward_votes_counter,
                                     uint32_t &rebroadcast_reward_votes_counter) {
  if (snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
    throw std::runtime_error("Rust PBFT manager snapshot rejected: " + static_cast<std::string>(snapshot.error_code));
  }

  round = snapshot.round;
  step = snapshot.step;
  state = fromPbftManagerRuntimeState(snapshot.state);
  current_round_lambda = std::chrono::milliseconds(snapshot.current_round_lambda_ms);
  next_step_time = std::chrono::milliseconds(snapshot.next_step_time_ms);
  rounds_count_dynamic_lambda = snapshot.rounds_count_dynamic_lambda;
  dynamic_lambda = snapshot.dynamic_lambda_ms;
  executed_pbft_block = snapshot.executed_pbft_block;
  already_next_voted_value = snapshot.already_next_voted_value;
  already_next_voted_null_block_hash = snapshot.already_next_voted_null;
  broadcast_votes_counter = snapshot.broadcast_votes_counter;
  rebroadcast_votes_counter = snapshot.rebroadcast_votes_counter;
  broadcast_reward_votes_counter = snapshot.broadcast_reward_votes_counter;
  rebroadcast_reward_votes_counter = snapshot.rebroadcast_reward_votes_counter;
}

void ensurePbftManagerRuntimeSnapshotReady(const rustaxa::PbftManagerRuntimeSnapshot &snapshot, const char *operation) {
  if (snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
    throw std::runtime_error(std::string(operation) + " rejected by Rust PBFT manager runtime: " +
                             static_cast<std::string>(snapshot.error_code));
  }
}

rustaxa::PbftManagerLifecycleTransitionRequest makePbftManagerLifecycleTransitionRequest(
    uint8_t kind, PbftPeriod target_period, PbftRound target_round, const VoteManager &vote_mgr,
    const rustaxa::PbftManagerRuntimeSnapshot &snapshot) {
  rustaxa::PbftManagerLifecycleTransitionRequest request{};
  request.kind = kind;
  request.target_period = target_period;
  request.target_round = target_round;
  const auto next_step =
      kind == kPbftManagerTransitionResetConsensus
          ? PbftStep{1}
          : (kind == kPbftManagerTransitionDelayCertifyPoll || kind == kPbftManagerTransitionDelayFinishPoll
                 ? static_cast<PbftStep>(snapshot.step)
                 : static_cast<PbftStep>(snapshot.step + 1));
  if (next_step >= kMaxSteps && next_step % 2) {
    request.has_network_next_voting_step = true;
    request.network_next_voting_step =
        vote_mgr.getNetworkTplusOneNextVotingStep(target_period, static_cast<PbftRound>(snapshot.round));
  }
  return request;
}

rustaxa::PbftManagerStateActionFact makePbftManagerStateActionFact(
    PbftStates state, PbftPeriod period, PbftRound round, PbftStep step, std::chrono::milliseconds elapsed,
    std::chrono::milliseconds deadline, std::chrono::milliseconds current_round_lambda, const VoteManager &vote_mgr,
    bool has_cert_voted_block, const blk_hash_t &cert_voted_block_hash, bool already_next_voted_value,
    bool already_next_voted_null_block_hash) {
  rustaxa::PbftManagerStateActionFact fact{};
  fact.state = toPbftManagerRuntimeState(state);
  fact.period = period;
  fact.round = round;
  fact.step = step;
  fact.elapsed_round_ms = static_cast<uint64_t>(elapsed.count());
  fact.deadline_ms = static_cast<uint64_t>(deadline.count());
  fact.current_round_lambda_ms = static_cast<uint64_t>(current_round_lambda.count());
  fact.polling_interval_ms = static_cast<uint64_t>(kPollingIntervalMs.count());

  const auto certify_vote_window_started = elapsed >= current_round_lambda * 2;
  const auto certify_finish_deadline = deadline > kPollingIntervalMs ? deadline - kPollingIntervalMs : 0ms;
  const auto certify_will_finish = elapsed > certify_finish_deadline;
  const auto needs_previous_round_next_null =
      state == value_proposal_state || state == filter_state || state == finish_state || state == finish_polling_state;
  const auto needs_previous_round_next_value =
      state == value_proposal_state || state == filter_state || state == finish_state;
  const auto needs_current_round_soft =
      state == finish_polling_state || (state == certify_state && certify_vote_window_started && !certify_will_finish);

  const auto vote_facts = vote_mgr.stateActionVoteFacts(period, round, needs_previous_round_next_null,
                                                        needs_previous_round_next_value, needs_current_round_soft);
  fact.has_previous_round_next_null = vote_facts.has_previous_round_next_null;
  fact.has_previous_round_next_value = vote_facts.has_previous_round_next_value;
  fact.previous_round_next_value_hash = toBridgeHash(vote_facts.previous_round_next_value_hash);
  fact.has_current_round_soft_value = vote_facts.has_current_round_soft_value;
  fact.current_round_soft_value_hash = toBridgeHash(vote_facts.current_round_soft_value_hash);

  if (has_cert_voted_block) {
    fact.has_cert_voted_block = true;
    fact.cert_voted_block_hash = toBridgeHash(cert_voted_block_hash);
  }
  fact.already_next_voted_value = already_next_voted_value;
  fact.already_next_voted_null = already_next_voted_null_block_hash;
  return fact;
}

template <typename Executor, typename Logger>
rustaxa::PbftManagerStateActionSessionStep executeStateActionEffectSession(
    const rustaxa::BridgePbftService &runtime, const rustaxa::PbftManagerStateActionFact &fact, Executor &&executor,
    Logger &log_er) {
  rustaxa::pbft_manager_runtime_begin_state_action_effect_session(runtime, fact);
  auto step = rustaxa::pbft_manager_runtime_state_action_effect_session_next(runtime);
  while (step.has_effect) {
    rustaxa::PbftManagerStateActionEffectReport report{};
    report.cursor = step.cursor;
    report.intent = step.effect.intent;
    report.result = kPbftManagerStateActionEffectResultExecutorError;
    try {
      report.result = executor(step.effect);
    } catch (const std::exception &e) {
      report.error_code = std::string("PBFT_MANAGER_STATE_ACTION_EFFECT_EXCEPTION: ") + e.what();
    } catch (...) {
      report.error_code = "PBFT_MANAGER_STATE_ACTION_EFFECT_UNKNOWN_EXCEPTION";
    }
    step = rustaxa::pbft_manager_runtime_state_action_effect_session_report(runtime, report);
  }
  if (!step.can_continue) {
    LOG(log_er) << "Rust PBFT manager state-action effect session stopped, status "
                << static_cast<uint32_t>(step.status) << ", cursor " << step.cursor << ", error "
                << static_cast<std::string>(step.error_code);
  }
  return step;
}

rustaxa::PbftManagerLifecycleTransitionResult executePbftManagerLifecycleTransition(
    rustaxa::PbftManagerLifecycleTransitionRequest request, const rustaxa::BridgePbftService &runtime,
    std::atomic<PbftRound> &round, PbftStep &step, PbftStates &state, std::chrono::milliseconds &current_round_lambda,
    std::chrono::milliseconds &next_step_time, uint32_t &rounds_count_dynamic_lambda, uint32_t &dynamic_lambda,
    bool &executed_pbft_block, std::optional<std::shared_ptr<PbftBlock>> &cert_voted_block_for_round,
    std::map<blk_hash_t, std::vector<PbftStep>> &current_round_broadcasted_votes, uint32_t &broadcast_votes_counter,
    uint32_t &rebroadcast_votes_counter, uint32_t &broadcast_reward_votes_counter,
    uint32_t &rebroadcast_reward_votes_counter, bool &already_next_voted_value,
    bool &already_next_voted_null_block_hash, bool &print_cert_step_info, bool &print_second_finish_step_info,
    std::chrono::system_clock::time_point &current_round_start_datetime,
    std::chrono::system_clock::time_point &second_finish_step_start_datetime, bool apply_current_round_timer = true) {
  auto result = rustaxa::pbft_manager_runtime_execute_lifecycle_transition(runtime, std::move(request));
  if (result.status != kPbftManagerTransitionStorageStatusApplied) {
    throw std::runtime_error("Rust PBFT manager lifecycle transition failed: " +
                             static_cast<std::string>(result.error_code));
  }

  applyPbftManagerRuntimeSnapshot(result.snapshot, round, step, state, current_round_lambda, next_step_time,
                                  rounds_count_dynamic_lambda, dynamic_lambda, executed_pbft_block,
                                  already_next_voted_value, already_next_voted_null_block_hash, broadcast_votes_counter,
                                  rebroadcast_votes_counter, broadcast_reward_votes_counter,
                                  rebroadcast_reward_votes_counter);

  if (result.remove_cert_voted_sidecar) {
    cert_voted_block_for_round.reset();
  }
  if (result.clear_broadcasted_vote_sidecars) {
    current_round_broadcasted_votes.clear();
  }
  if (result.reset_current_round_timer && apply_current_round_timer) {
    current_round_start_datetime = std::chrono::system_clock::now();
  }
  if (result.reset_second_finish_timer) {
    second_finish_step_start_datetime = std::chrono::system_clock::now();
  }
  if (result.print_cert_step_info) {
    print_cert_step_info = true;
  }
  if (result.print_second_finish_step_info) {
    print_second_finish_step_info = true;
  }
  return result;
}

template <typename Bytes>
rust::Vec<uint8_t> toBridgeBytes(const Bytes &bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

dev::bytes fromBridgeBytes(const rust::Vec<uint8_t> &bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

template <typename Hash>
rust::Vec<rustaxa::PbftFinalizationHash> toBridgeFinalizationHashes(const std::vector<Hash> &hashes) {
  rust::Vec<rustaxa::PbftFinalizationHash> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.push_back(rustaxa::PbftFinalizationHash{toBridgeHash(hash)});
  }
  return out;
}

template <typename Hashes>
rust::Vec<rustaxa::PbftSyncTransactionHash> toBridgeTransactionHashes(const Hashes &hashes) {
  rust::Vec<rustaxa::PbftSyncTransactionHash> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.push_back(rustaxa::PbftSyncTransactionHash{toBridgeHash(hash)});
  }
  return out;
}

rustaxa::PbftFinalizationStorageWriteStage makeFinalizationStorageStage(uint8_t stage) {
  rustaxa::PbftFinalizationStorageWriteStage write_stage{};
  write_stage.stage = stage;
  write_stage.rounds_count_dynamic_lambda = 0;
  write_stage.dynamic_lambda = 0;
  write_stage.has_sortition_params_change = false;
  write_stage.sortition_params_change_period = 0;
  write_stage.sortition_params_change_interval_efficiency = 0;
  write_stage.sortition_params_change_threshold_upper = 0;
  write_stage.has_prepared_pillar_block = false;
  write_stage.prepared_pillar_block_period = 0;
  write_stage.prepared_pillar_block_rlp = rust::Vec<uint8_t>{};

  return write_stage;
}

std::vector<trx_hash_t> fromBridgeTransactionHashes(const rust::Vec<rustaxa::PbftSyncTransactionHash> &hashes) {
  std::vector<trx_hash_t> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.emplace_back(hash.hash.data(), trx_hash_t::ConstructFromPointer);
  }
  return out;
}

trx_hash_t fromBridgeTransactionHash(const std::array<uint8_t, 32> &hash) {
  return trx_hash_t(hash.data(), trx_hash_t::ConstructFromPointer);
}

std::runtime_error periodDataQueueError(const std::string &message) {
  return std::runtime_error("PBFT manager period-data queue: " + message);
}

std::vector<bytes> fromBridgePillarVoteRlps(const rust::Vec<rustaxa::PillarVoteRlpPayload> &payloads) {
  std::vector<bytes> out;
  out.reserve(payloads.size());
  for (const auto &payload : payloads) {
    out.emplace_back(payload.vote_rlp.begin(), payload.vote_rlp.end());
  }
  return out;
}

rust::Vec<rustaxa::PillarVoteRlpPayload> toBridgePillarVoteRlps(const std::vector<bytes> &vote_rlps) {
  rust::Vec<rustaxa::PillarVoteRlpPayload> payloads;
  payloads.reserve(vote_rlps.size());
  for (const auto &vote_rlp : vote_rlps) {
    rustaxa::PillarVoteRlpPayload payload;
    payload.vote_rlp = toBridgeBytes(vote_rlp);
    payloads.push_back(std::move(payload));
  }
  return payloads;
}

std::vector<bytes> fromBridgeTransactionRlps(const rust::Vec<rustaxa::PeriodDataQueueTransactionPayload> &payloads) {
  std::vector<bytes> out;
  out.reserve(payloads.size());
  for (const auto &payload : payloads) {
    out.emplace_back(payload.transaction_rlp.begin(), payload.transaction_rlp.end());
  }
  return out;
}

std::vector<std::shared_ptr<PbftVote>> fromBridgePbftVotes(const rust::Vec<rustaxa::PbftCertVoteRlp> &payloads) {
  std::vector<std::shared_ptr<PbftVote>> out;
  out.reserve(payloads.size());
  for (const auto &payload : payloads) {
    out.push_back(std::make_shared<PbftVote>(bytes(payload.vote_rlp.begin(), payload.vote_rlp.end())));
  }
  return out;
}

u256 fromBridgeU256(const std::array<uint8_t, 32> &value) {
  return dev::fromBigEndian<u256>(dev::bytes(value.begin(), value.end()));
}

addr_t fromBridgeAddress(const std::array<uint8_t, 20> &address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

SharedTransactions materializeTransactionsFromQueuedRlps(const std::vector<bytes> &transaction_rlps,
                                                         const std::vector<trx_hash_t> &expected_hashes) {
  if (transaction_rlps.size() != expected_hashes.size()) {
    throw std::runtime_error("queued transaction RLP count does not match queued transaction hash count");
  }

  SharedTransactions transactions;
  transactions.reserve(transaction_rlps.size());
  for (size_t i = 0; i < transaction_rlps.size(); ++i) {
    auto transaction = std::make_shared<Transaction>(transaction_rlps[i]);
    if (transaction->getHash() != expected_hashes[i]) {
      throw std::runtime_error("queued transaction RLP hash does not match queued transaction hash fact");
    }
    transactions.emplace_back(std::move(transaction));
  }
  return transactions;
}

void materializeCachedCandidateDag(const rustaxa::DagManagerNonFinalizedSyncPayload &payload, PeriodData &period_data) {
  period_data.dag_blocks.reserve(payload.blocks.size());
  for (const auto &entry : payload.blocks) {
    const bytes block_rlp(entry.block_rlp.begin(), entry.block_rlp.end());
    dev::RLP decoded(dev::bytesConstRef(block_rlp.data(), block_rlp.size()));
    auto block = std::make_shared<DagBlock>(decoded);
    if (block->getHash() != fromBridgeHash(entry.hash)) {
      throw std::runtime_error("native PBFT candidate DAG block payload hash mismatch");
    }
    period_data.dag_blocks.emplace_back(std::move(block));
  }

  period_data.transactions.reserve(payload.transactions.size());
  for (const auto &entry : payload.transactions) {
    if (!entry.found) {
      continue;
    }
    auto transaction = std::make_shared<Transaction>(bytes(entry.tx_rlp.begin(), entry.tx_rlp.end()));
    if (transaction->getHash() != fromBridgeHash(entry.hash)) {
      throw std::runtime_error("native PBFT candidate transaction payload hash mismatch");
    }
    period_data.transactions.emplace_back(std::move(transaction));
  }
}

rustaxa::PbftFinalizationIntentFact makePbftFinalizationIntentFact(
    const PeriodData &period_data, bool block_in_chain, bool pillar_block_finalized,
    bool request_dynamic_lambda_update, uint64_t cert_vote_count, const blk_hash_t &sample_cert_vote_block_hash,
    PbftPeriod sample_cert_vote_period, PbftRound sample_cert_vote_round, PbftStep sample_cert_vote_step,
    uint32_t block_lambda, bool last_saved_period_lambda_found, uint32_t last_saved_period_lambda,
    uint32_t dynamic_blocks_per_year, uint32_t rounds_count_dynamic_lambda, uint32_t dynamic_lambda,
    uint32_t dpos_blocks_per_year, const std::vector<blk_hash_t> &dag_blocks_order,
    const std::vector<trx_hash_t> &transaction_order, bool process_pillar_block_after_advance) {
  rustaxa::PbftFinalizationIntentFact fact;
  fact.block_hash = toBridgeHash(period_data.pbft_blk->getBlockHash());
  fact.block_period = period_data.pbft_blk->getPeriod();
  fact.block_prev_hash = toBridgeHash(period_data.pbft_blk->getPrevBlockHash());
  fact.pivot_dag_anchor_hash = toBridgeHash(period_data.pbft_blk->getPivotDagBlockHash());
  fact.has_pillar_block =
      period_data.pbft_blk->getExtraData() && period_data.pbft_blk->getExtraData()->getPillarBlockHash().has_value();
  fact.pillar_block_finalized = pillar_block_finalized;
  fact.request_dynamic_lambda_update = request_dynamic_lambda_update;
  fact.cert_vote_count = cert_vote_count;
  fact.sample_cert_vote_block_hash = toBridgeHash(sample_cert_vote_block_hash);
  fact.sample_cert_vote_period = sample_cert_vote_period;
  fact.sample_cert_vote_round = sample_cert_vote_round;
  fact.sample_cert_vote_step = sample_cert_vote_step;
  fact.block_lambda = block_lambda;
  fact.last_saved_period_lambda_found = last_saved_period_lambda_found;
  fact.last_saved_period_lambda = last_saved_period_lambda;
  fact.dynamic_blocks_per_year = dynamic_blocks_per_year;
  fact.rounds_count_dynamic_lambda = rounds_count_dynamic_lambda;
  fact.dynamic_lambda = dynamic_lambda;
  fact.dpos_blocks_per_year = dpos_blocks_per_year;
  fact.period_data_rlp = block_in_chain ? rust::Vec<uint8_t>() : toBridgeBytes(period_data.rlp());
  fact.ordered_dag_block_hashes = toBridgeFinalizationHashes(dag_blocks_order);
  fact.ordered_transaction_hashes = toBridgeFinalizationHashes(transaction_order);
  fact.process_pillar_block_after_advance = process_pillar_block_after_advance;
  return fact;
}

rustaxa::PbftDynamicLambdaFact makePbftDynamicLambdaFact(const HardforksConfig &hardforks,
                                                         uint32_t dpos_blocks_per_year, bool dynamic_lambda_active,
                                                         PbftPeriod finalized_period, PbftRound finalized_round,
                                                         uint32_t rounds_count_dynamic_lambda,
                                                         uint32_t dynamic_lambda) {
  const auto &cacti_hf = hardforks.cacti_hf;
  rustaxa::PbftDynamicLambdaConfig config{};
  config.cacti_block_num = cacti_hf.block_num;
  config.lambda_min = cacti_hf.lambda_min;
  config.lambda_max = cacti_hf.lambda_max;
  config.lambda_default = cacti_hf.lambda_default;
  config.lambda_change_interval = cacti_hf.lambda_change_interval;
  config.lambda_change = cacti_hf.lambda_change;
  config.consensus_delay = cacti_hf.consensus_delay;
  config.dpos_blocks_per_year = dpos_blocks_per_year;

  rustaxa::PbftDynamicLambdaFact fact{};
  fact.dynamic_lambda_active = dynamic_lambda_active;
  fact.finalized_period = finalized_period;
  fact.finalized_round = finalized_round;
  fact.pre_adjust_rounds_count_dynamic_lambda = rounds_count_dynamic_lambda;
  fact.pre_adjust_dynamic_lambda = dynamic_lambda;
  fact.config = config;
  return fact;
}

}  // namespace

PbftManager::PbftManager(const FullNodeConfig &conf, std::shared_ptr<DbStorage> db, SharedPbftService pbft_service,
                         SharedDagTransactionService dag_transaction_service, std::shared_ptr<PbftChain> pbft_chain,
                         std::shared_ptr<VoteManager> vote_mgr, std::shared_ptr<DagManager> dag_mgr,
                         std::shared_ptr<TransactionManager> trx_mgr,
                         std::shared_ptr<final_chain::FinalChain> final_chain,
                         std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_mgr)
    : db_(std::move(db)),
      pbft_service_(std::move(pbft_service)),
      dag_transaction_service_(std::move(dag_transaction_service)),
      pbft_chain_(std::move(pbft_chain)),
      vote_mgr_(std::move(vote_mgr)),
      dag_mgr_(std::move(dag_mgr)),
      trx_mgr_(std::move(trx_mgr)),
      final_chain_(std::move(final_chain)),
      pillar_chain_mgr_(std::move(pillar_chain_mgr)),
      kSyncingThreadPoolSize(std::thread::hardware_concurrency() / 2),
      sync_thread_pool_(std::make_shared<util::ThreadPool>(kSyncingThreadPoolSize)),
      rounds_count_dynamic_lambda_(0),
      dynamic_lambda_(conf.genesis.state.hardforks.cacti_hf.lambda_max),
      dag_genesis_block_hash_(conf.genesis.dag_genesis_block.getHash()),
      kGenesisConfig(conf.genesis),
      eligible_wallets_(conf.wallets) {
  if (!pbft_service_) {
    throw std::invalid_argument("PBFT manager requires a shared PBFT service");
  }
  if (!dag_transaction_service_) {
    throw std::invalid_argument("PBFT manager requires a shared DAG/transaction service");
  }
  // Use first wallet as default node_addr
  const auto &node_addr = dev::toAddress(conf.getFirstWallet().node_secret);
  LOG_OBJECTS_CREATE("PBFT_MGR");

  rustaxa::PbftManagerStartupReplayRangeFact startup_replay_fact;
  startup_replay_fact.final_chain_last_block = rustFinalChainLastBlockNumber(final_chain_);
  startup_replay_fact.pbft_chain_size = pbft_chain_->getPbftChainSize();
  startup_replay_fact.delegation_delay = final_chain_->delegationDelay();
  startup_replay_fact.recently_finalized_factor = kRecentlyFinalizedTransactionsFactor;
  const auto startup_replay_plan = rustaxa::plan_pbft_manager_startup_replay_ranges(startup_replay_fact);
  if (!startup_replay_plan.accepted) {
    LOG(log_er_) << "Rust PBFT manager startup replay planner rejected facts, error "
                 << static_cast<std::string>(startup_replay_plan.error_code);
    assert(false);
  }

  if (startup_replay_plan.has_finalization_range) {
    for (auto period = startup_replay_plan.finalization_from_period;
         period <= startup_replay_plan.finalization_to_period; ++period) {
      const auto replay_period = rustaxa::pbft_manager_runtime_load_startup_replay_period(
          pbft_service_->service(), period, kGenesisConfig.state.hardforks.isOnCactiHardfork(period));
      if (!replay_period.found) {
        LOG(log_er_) << "DB corrupted - Cannot find PBFT block in period " << period
                     << " in PBFT chain DB pbft_blocks.";
        assert(false);
      }
      auto period_data =
          PeriodData{dev::bytes(replay_period.period_data_rlp.begin(), replay_period.period_data_rlp.end())};

      if (period_data.pbft_blk->getPeriod() != period) {
        LOG(log_er_) << "DB corrupted - PBFT block hash " << period_data.pbft_blk->getBlockHash()
                     << " has different period " << period_data.pbft_blk->getPeriod()
                     << " in block data than in block order db: " << period;
        assert(false);
      }

      // We need this section because votes need to be verified for reward distribution
      const auto replay_vote_validation = vote_mgr_->validateStartupReplayVotes(period_data.previous_block_cert_votes);
      if (!replay_vote_validation.accepted) {
        LOG(log_er_) << "DB corrupted - Cannot validate startup replay cert vote "
                     << replay_vote_validation.first_bad_vote_hash << " in period " << period
                     << ". Err: " << replay_vote_validation.validation_error;
        assert(false);
      }

      uint32_t blocks_per_year{0};
      // Dynamic lambda was introduced in cacti hardfork -> it affects the number of blocks generated per year, which
      // affects rewards distribution
      if (kGenesisConfig.state.hardforks.isOnCactiHardfork(period)) {
        if (!replay_period.has_period_lambda) {
          LOG(log_er_) << "DB corrupted - no dynamic lambda saved for period " << period;
          assert(false);
        }

        blocks_per_year = kGenesisConfig.calcBlocksPerYear(
            replay_period.period_lambda,
            kGenesisConfig.state.hardforks.cacti_hf
                .consensus_delay /* approx time it takes to receive 2t+1 soft and cert votes after 2*lambda */);
      } else {
        blocks_per_year = kGenesisConfig.state.dpos.blocks_per_year;
      }

      std::vector<blk_hash_t> finalized_dag_hashes;
      finalized_dag_hashes.reserve(replay_period.finalized_dag_hashes.size());
      for (const auto &hash : replay_period.finalized_dag_hashes) {
        finalized_dag_hashes.push_back(fromBridgeHash(hash.hash));
      }

      finalize_(std::move(period_data), std::move(finalized_dag_hashes), blocks_per_year,
                period == startup_replay_plan.finalization_to_period);
    }
  }

  for (PbftPeriod period = startup_replay_plan.recent_from_period; period <= startup_replay_plan.recent_to_period;
       period++) {
    const auto replay_period =
        rustaxa::pbft_manager_runtime_load_startup_replay_period(pbft_service_->service(), period, false);
    if (!replay_period.found) {
      LOG(log_er_) << "DB corrupted - Cannot find PBFT block in period " << period << " in PBFT chain DB pbft_blocks.";
      assert(false);
    }
    trx_mgr_->initializeRecentlyFinalizedTransactions(
        PeriodData{dev::bytes(replay_period.period_data_rlp.begin(), replay_period.period_data_rlp.end())});
  }

  // Initialize PBFT status
  initialState();

  // Update wallets eligibility, call after initialState (waitForPeriodFinalization)
  eligible_wallets_.updateWalletsEligibility(pbft_chain_->getPbftChainSize(), pbft_service_, final_chain_);

  // Note: processPillarBlock must be called after eligible_wallets_.updateWalletsEligibility
  auto current_pbft_period = pbft_chain_->getPbftChainSize();
  if (kGenesisConfig.state.hardforks.ficus_hf.isPillarBlockPeriod(current_pbft_period)) {
    rustaxa::PillarCurrentAnchorDecisionRequest request{};
    request.operation = kPillarAnchorDecisionRestartPostProcessing;
    request.pbft_period = current_pbft_period;
    request.pillar_blocks_interval = kGenesisConfig.state.hardforks.ficus_hf.pillar_blocks_interval;
    const auto pillar_restart =
        requireReadyPillarService(pbft_service_).pbft_service_pillar_plan_current_anchor_decision(request);
    if (pillar_restart.selected) {
      LOG(log_er_) << "Pillar block was not processed before restart, current period: " << current_pbft_period
                   << ", current pillar block period: " << pillar_restart.current_period;
      processPillarBlock(current_pbft_period);
    }
  }

  // Release live manager commands only after replay, compatibility mirrors, wallet eligibility, and restart
  // post-processing are coherent. This is a one-way Rust-owned bootstrap transition.
  rustaxa::pbft_service_complete_bootstrap(pbft_service_->service());
}

PbftManager::~PbftManager() { stop(); }

PbftManager::PoppedPeriodDataPayload PbftManager::popPeriodDataQueueWithMetadata() {
  if (!pbft_service_) {
    throw periodDataQueueError("PBFT manager runtime is not initialized");
  }

  rustaxa::PeriodDataQueuePopPlan plan;
  try {
    plan = rustaxa::pbft_manager_runtime_period_data_queue_pop(pbft_service_->service());
  } catch (const std::exception &e) {
    throw periodDataQueueError(e.what());
  } catch (...) {
    throw periodDataQueueError("Rust pop failed");
  }

  PoppedPeriodDataPayload payload;
  payload.period_data = PeriodData{dev::bytes(plan.period_data_rlp.begin(), plan.period_data_rlp.end())};
  payload.period_data.previous_block_cert_votes = fromBridgePbftVotes(plan.previous_cert_vote_rlps);
  payload.cert_vote_rlps = std::move(plan.cert_vote_rlps);
  payload.node_id = dev::p2p::NodeID(plan.source_peer_id.data(), dev::p2p::NodeID::ConstructFromPointer);
  payload.period = plan.entry_period;
  payload.block_hash = blk_hash_t(plan.block_hash.data(), blk_hash_t::ConstructFromPointer);
  payload.prev_block_hash = blk_hash_t(plan.prev_block_hash.data(), blk_hash_t::ConstructFromPointer);
  payload.pivot_hash = blk_hash_t(plan.pivot_hash.data(), blk_hash_t::ConstructFromPointer);
  payload.final_chain_hash = blk_hash_t(plan.final_chain_hash.data(), blk_hash_t::ConstructFromPointer);
  payload.reward_vote_hashes = fromBridgeTransactionHashes(plan.reward_vote_hashes);
  payload.pillar_vote_rlps = fromBridgePillarVoteRlps(plan.pillar_vote_rlps);
  payload.transaction_rlps = fromBridgeTransactionRlps(plan.transaction_rlps);
  payload.dag_transaction_hashes = fromBridgeTransactionHashes(plan.dag_transaction_hashes);
  payload.period_data_transaction_hashes = fromBridgeTransactionHashes(plan.period_data_transaction_hashes);
  payload.period_data_transaction_identities = std::move(plan.period_data_transaction_identities);
  payload.previous_cert_votes_present = plan.previous_cert_votes_present;
  payload.previous_cert_first_vote_has_weight = plan.previous_cert_first_vote_has_weight;
  payload.pillar_votes_present = plan.pillar_votes_present;
  payload.extra_data_present = plan.extra_data_present;
  payload.extra_data_pillar_block_hash_present = plan.extra_data_pillar_block_hash_present;

  return PoppedPeriodDataPayload{std::move(payload.period_data),
                                 std::move(payload.cert_vote_rlps),
                                 payload.node_id,
                                 plan.entry_period,
                                 payload.block_hash,
                                 payload.prev_block_hash,
                                 payload.pivot_hash,
                                 payload.final_chain_hash,
                                 std::move(payload.reward_vote_hashes),
                                 std::move(payload.pillar_vote_rlps),
                                 std::move(payload.transaction_rlps),
                                 std::move(payload.dag_transaction_hashes),
                                 std::move(payload.period_data_transaction_hashes),
                                 std::move(payload.period_data_transaction_identities),
                                 payload.previous_cert_votes_present,
                                 payload.previous_cert_first_vote_has_weight,
                                 payload.pillar_votes_present,
                                 payload.extra_data_present,
                                 payload.extra_data_pillar_block_hash_present};
}

void PbftManager::setNetwork(std::weak_ptr<Network> network) { network_ = std::move(network); }

void PbftManager::start() {
  if (bool b = true; !stopped_.compare_exchange_strong(b, !b)) {
    return;
  }

  daemon_ = std::make_unique<std::thread>([this]() { run(); });
  LOG(log_dg_) << "PBFT daemon initiated ...";
}

void PbftManager::stop() {
  if (bool b = false; !stopped_.compare_exchange_strong(b, !b)) {
    return;
  }

  {
    std::unique_lock<std::mutex> lock(stop_mtx_);
    stop_cv_.notify_all();
  }

  daemon_->join();
  final_chain_->stop();

  LOG(log_dg_) << "PBFT daemon terminated ...";
}

/* When a node starts up it has to sync to the current phase (type of block
 * being generated) and step (within the block generation round)
 * Five step loop for block generation over three phases of blocks
 * User's credential, sigma_i_p for a round p is sig_i(R, p)
 * Leader l_i_p = min ( H(sig_j(R,p) ) over set of j in S_i where S_i is set of
 * users from which have received valid round p credentials
 */
void PbftManager::run() {
  uint64_t rust_runtime_tick_id = 0;
  while (!stopped_) {
    const auto runtime_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
    const auto period = getPbftPeriod();
    const auto round = static_cast<PbftRound>(runtime_snapshot.round);
    const auto snapshot_step = static_cast<PbftStep>(runtime_snapshot.step);
    LOG(log_tr_) << "PBFT current period: " << period << ", round: " << round << ", step " << snapshot_step;

    auto net = network_.lock();
    const auto &wallets = eligible_wallets_.getWallets(period);
    const bool has_eligible_wallet =
        std::any_of(wallets.cbegin(), wallets.cend(), [](const auto &wallet) { return wallet.first; });

    rustaxa::PbftManagerRuntimeTickFact fact{};
    fact.tick_id = ++rust_runtime_tick_id;
    fact.state = runtime_snapshot.state;
    fact.period = period;
    fact.round = round;
    fact.step = snapshot_step;
    fact.network_available = static_cast<bool>(net);
    fact.network_pbft_syncing = net && net->pbft_syncing();
    fact.has_eligible_wallet = has_eligible_wallet;
    fact.polling_interval_ms = static_cast<uint64_t>(kPollingIntervalMs.count());

    rustaxa::pbft_manager_runtime_begin_session(pbft_service_->service(), fact);
    auto report_action = [&](const rustaxa::PbftManagerRuntimeSessionStep &step, uint8_t result, bool success = true,
                             const std::string &error_code = "", bool has_new_round = false, PbftRound new_round = 0) {
      const auto current_period = getPbftPeriod();
      const auto &current_wallets = eligible_wallets_.getWallets(current_period);
      rustaxa::PbftManagerRuntimeActionReport report{};
      report.cursor = step.cursor;
      report.action = step.action;
      report.success = success;
      report.result = result;
      report.go_finish_state = go_finish_state_;
      report.loop_back_finish_state = loop_back_finish_state_;
      report.has_eligible_wallet = std::any_of(current_wallets.cbegin(), current_wallets.cend(),
                                               [](const auto &wallet) { return wallet.first; });
      report.has_new_round = has_new_round;
      report.new_round = new_round;
      report.error_code = error_code;
      return rustaxa::pbft_manager_runtime_session_report(pbft_service_->service(), std::move(report));
    };
    auto apply_delay_transition = [&](uint8_t kind) {
      const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
      const auto current_period = getPbftPeriod();
      const auto request =
          makePbftManagerLifecycleTransitionRequest(kind, current_period, 0, *vote_mgr_, transition_snapshot);
      executePbftManagerLifecycleTransition(
          request, pbft_service_->service(), round_, step_, state_, current_round_lambda_, next_step_time_ms_,
          rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_, cert_voted_block_for_round_,
          current_round_broadcasted_votes_, broadcast_votes_counter_, rebroadcast_votes_counter_,
          broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_, already_next_voted_value_,
          already_next_voted_null_block_hash_, printCertStepInfo_, printSecondFinishStepInfo_,
          current_round_start_datetime_, second_finish_step_start_datetime_);
      return true;
    };

    bool restart_loop = false;
    while (!stopped_) {
      auto step = rustaxa::pbft_manager_runtime_session_next(pbft_service_->service());
      if (step.status == kPbftManagerRuntimeStatusComplete || step.complete) {
        restart_loop = step.restart_loop;
        break;
      }

      if (step.status != kPbftManagerRuntimeStatusActive || !step.has_action) {
        LOG(log_er_) << "Rust PBFT manager runtime rejected tick " << step.tick_id << ", status "
                     << static_cast<uint32_t>(step.status) << ", error " << static_cast<std::string>(step.error_code);
        rustaxa::abort_pbft_manager_runtime_session(pbft_service_->service());
        assert(false);
        restart_loop = true;
        break;
      }

      const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
      const auto action_state = fromPbftManagerRuntimeState(action_snapshot.state);
      if (!pbftManagerRuntimeActionMatchesLiveState(step.action, action_state)) {
        LOG(log_dg_) << "Rust PBFT manager runtime action " << static_cast<uint32_t>(step.action)
                     << " no longer matches Rust PBFT state " << static_cast<uint32_t>(action_state)
                     << "; restarting daemon loop";
        rustaxa::abort_pbft_manager_runtime_session(pbft_service_->service());
        restart_loop = true;
        break;
      }

      switch (step.action) {
        case kPbftManagerRuntimeActionProcessSyncedPbftBlocks:
          pushSyncedPbftBlocksIntoChain();
          step = report_action(step, kPbftManagerRuntimeResultStateActionDone);
          break;
        case kPbftManagerRuntimeActionMaybeBroadcastVotes:
          broadcastVotes();
          step = report_action(step, kPbftManagerRuntimeResultStateActionDone);
          break;
        case kPbftManagerRuntimeActionTryPushCertVotesBlock:
          step = report_action(step, tryPushCertVotesBlock() ? kPbftManagerRuntimeResultProgressRestartLoop
                                                             : kPbftManagerRuntimeResultNoProgressContinue);
          break;
        case kPbftManagerRuntimeActionTryAdvanceRound: {
          const auto [current_round, current_period] = getPbftRoundAndPeriod();
          const auto round_advance = vote_mgr_->roundAdvanceDecision(current_period, current_round);
          step = report_action(step, kPbftManagerRuntimeResultNoProgressContinue, true, "", round_advance.has_new_round,
                               round_advance.new_round);
          break;
        }
        case kPbftManagerRuntimeActionResetConsensus:
          if (!step.has_target_round || step.target_round == 0) {
            step = report_action(step, kPbftManagerRuntimeResultExecutorError, false,
                                 "PBFT_MANAGER_RESET_CONSENSUS_MISSING_TARGET_ROUND");
            break;
          }
          resetPbftConsensus(step.target_round);
          LOG(log_nf_) << "Round advanced to: " << step.target_round << ", period " << getPbftPeriod() << ", step "
                       << getPbftStep();
          step = report_action(step, kPbftManagerRuntimeResultTransitionApplied);
          break;
        case kPbftManagerRuntimeActionSleepIneligiblePollingInterval:
          std::this_thread::sleep_for(std::chrono::milliseconds(step.sleep_ms));
          step = report_action(step, kPbftManagerRuntimeResultSleepApplied);
          break;
        case kPbftManagerRuntimeActionRunValueProposal:
          proposeBlock_();
          step = report_action(step, kPbftManagerRuntimeResultStateActionDone);
          break;
        case kPbftManagerRuntimeActionTransitionToFilter:
          setFilterState_();
          step = report_action(step, kPbftManagerRuntimeResultTransitionApplied);
          break;
        case kPbftManagerRuntimeActionRunFilter:
          identifyBlock_();
          step = report_action(step, kPbftManagerRuntimeResultStateActionDone);
          break;
        case kPbftManagerRuntimeActionTransitionToCertify:
          setCertifyState_();
          step = report_action(step, kPbftManagerRuntimeResultTransitionApplied);
          break;
        case kPbftManagerRuntimeActionRunCertify:
          certifyBlock_();
          step = report_action(step, kPbftManagerRuntimeResultStateActionDone);
          break;
        case kPbftManagerRuntimeActionTransitionToFinish:
          setFinishState_();
          step = report_action(step, kPbftManagerRuntimeResultTransitionApplied);
          break;
        case kPbftManagerRuntimeActionDelayCertifyPoll: {
          const auto applied = apply_delay_transition(kPbftManagerTransitionDelayCertifyPoll);
          step = report_action(step,
                               applied ? kPbftManagerRuntimeResultSleepApplied : kPbftManagerRuntimeResultExecutorError,
                               applied, applied ? "" : "PBFT_MANAGER_DELAY_CERTIFY_TRANSITION_FAILED");
          break;
        }
        case kPbftManagerRuntimeActionRunFirstFinish:
          firstFinish_();
          step = report_action(step, kPbftManagerRuntimeResultStateActionDone);
          break;
        case kPbftManagerRuntimeActionTransitionToFinishPolling:
          setFinishPollingState_();
          step = report_action(step, kPbftManagerRuntimeResultTransitionApplied);
          break;
        case kPbftManagerRuntimeActionRunSecondFinish:
          secondFinish_();
          step = report_action(step, kPbftManagerRuntimeResultStateActionDone);
          break;
        case kPbftManagerRuntimeActionLoopBackFinish:
          loopBackFinishState_();
          printVotingSummary();
          step = report_action(step, kPbftManagerRuntimeResultTransitionApplied);
          break;
        case kPbftManagerRuntimeActionDelayFinishPoll: {
          const auto applied = apply_delay_transition(kPbftManagerTransitionDelayFinishPoll);
          step = report_action(step,
                               applied ? kPbftManagerRuntimeResultSleepApplied : kPbftManagerRuntimeResultExecutorError,
                               applied, applied ? "" : "PBFT_MANAGER_DELAY_FINISH_TRANSITION_FAILED");
          break;
        }
        case kPbftManagerRuntimeActionSleepUntilNextStep:
          LOG(log_tr_) << "next step time(ms): "
                       << rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service()).next_step_time_ms
                       << ", step " << getPbftStep();
          sleep_();
          step = report_action(step, kPbftManagerRuntimeResultSleepApplied);
          break;
        default:
          LOG(log_er_) << "Unknown Rust PBFT manager runtime action " << static_cast<uint32_t>(step.action);
          step = report_action(step, kPbftManagerRuntimeResultExecutorError, false,
                               "PBFT_MANAGER_RUNTIME_UNKNOWN_CPP_ACTION");
          break;
      }

      if (!step.can_continue) {
        LOG(log_er_) << "Rust PBFT manager runtime failed tick " << step.tick_id << ", status "
                     << static_cast<uint32_t>(step.status) << ", action " << static_cast<uint32_t>(step.action)
                     << ", error " << static_cast<std::string>(step.error_code);
        rustaxa::abort_pbft_manager_runtime_session(pbft_service_->service());
        assert(false);
        restart_loop = true;
        break;
      }

      if (step.complete) {
        restart_loop = step.restart_loop;
        break;
      }
    }

    if (restart_loop) {
      continue;
    }
  }
}

std::pair<bool, PbftPeriod> PbftManager::getDagBlockPeriod(const blk_hash_t &hash) {
  if (!pbft_service_) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before reading DAG block period");
  }
  const auto lookup = rustaxa::pbft_manager_runtime_dag_block_period(pbft_service_->service(), toBridgeHash(hash));
  if (!lookup.found) {
    return {false, PbftPeriod{0}};
  }
  return {true, static_cast<PbftPeriod>(lookup.period)};
}

PbftPeriod PbftManager::getPbftPeriod() const { return pbft_chain_->getPbftChainSize() + 1; }

PbftRound PbftManager::getPbftRound() const {
  if (!pbft_service_) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before reading PBFT round");
  }
  const auto snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  if (snapshot.status != kPbftManagerStartupRestoreStatusReady) {
    throw std::runtime_error("PBFT manager Rust runtime snapshot is not ready: " +
                             static_cast<std::string>(snapshot.error_code));
  }
  return static_cast<PbftRound>(snapshot.round);
}

std::pair<PbftRound, PbftPeriod> PbftManager::getPbftRoundAndPeriod() const {
  return {getPbftRound(), getPbftPeriod()};
}

PbftStep PbftManager::getPbftStep() const {
  if (!pbft_service_) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before reading PBFT step");
  }
  const auto snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  if (snapshot.status != kPbftManagerStartupRestoreStatusReady) {
    throw std::runtime_error("PBFT manager Rust runtime snapshot is not ready: " +
                             static_cast<std::string>(snapshot.error_code));
  }
  return static_cast<PbftStep>(snapshot.step);
}

void PbftManager::setPbftRound(PbftRound round) {
  if (!pbft_service_) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before persisting round");
  }
  const auto snapshot = rustaxa::pbft_manager_runtime_apply_cursor_field(
      pbft_service_->service(), static_cast<uint8_t>(PbftMgrField::Round), static_cast<uint32_t>(round));
  round_ = static_cast<PbftRound>(snapshot.round);
}

void PbftManager::waitForPeriodFinalization() {
  do {
    // we need to be sure we finalized at least block with num lower by delegation_delay
    rustaxa::PbftManagerFinalizationWaitFact fact{};
    fact.pbft_chain_size = pbft_chain_->getPbftChainSize();
    fact.final_chain_last_block = rustFinalChainLastBlockNumber(final_chain_);
    fact.delegation_delay = final_chain_->delegationDelay();
    fact.polling_interval_ms = static_cast<uint64_t>(kPollingIntervalMs.count());
    const auto plan = rustaxa::plan_pbft_manager_finalization_wait(fact);
    if (!plan.accepted) {
      throw std::runtime_error("Rust PBFT manager finalization wait planner rejected facts: " +
                               static_cast<std::string>(plan.error_code));
    }
    if (!plan.should_wait) {
      break;
    }
    thisThreadSleepForMilliSeconds(plan.sleep_ms);
  } while (!stopped_);
}

std::optional<uint64_t> PbftManager::getCurrentDposTotalVotesCount() const {
  try {
    const auto period = pbft_chain_->getPbftChainSize();
    rustaxa::PbftFinalChainDposTotalVoteCountRequest request;
    request.period = period;
    const auto facts =
        pbft_service_->service().pbft_service_collect_dpos_total_vote_count(final_chain_->rustFinalChain(), request);
    if (facts.status == kPbftSyncDposFactsReady && facts.has_total_vote_count) {
      return facts.total_vote_count;
    }
    LOG(log_wr_) << "Unable to get CurrentDposTotalVotesCount for period: " << period
                 << ". Period is too far ahead of actual finalized pbft chain size (" << facts.last_block_number
                 << "). Err msg: " << static_cast<std::string>(facts.error_code);
  } catch (const std::exception &e) {
    LOG(log_wr_) << "Unable to get CurrentDposTotalVotesCount for period: " << pbft_chain_->getPbftChainSize()
                 << ". Period is too far ahead of actual finalized pbft chain size (" << final_chain_->lastBlockNumber()
                 << "). Err msg: " << e.what();
  }

  return {};
}

std::optional<uint64_t> PbftManager::getCurrentNodeVotesCount() const {
  // Note: There is a race condition in eligible_wallets_.getWalletsEligiblePeriod(). This method works only if
  // wallets eligible period == pbft chain size. This race condition is handled within pbft manager but
  // getCurrentNodeVotesCount() is called externally from standalone thread and in some edge cases we need to wait until
  // period in eligible_wallets_ is updated according to the latest chain size
  while (true) {
    rustaxa::PbftManagerEligibleWalletPeriodWaitFact fact{};
    fact.eligible_wallet_period = eligible_wallets_.getWalletsEligiblePeriod();
    fact.pbft_chain_size = pbft_chain_->getPbftChainSize();
    fact.polling_interval_ms = 10;
    const auto plan = rustaxa::plan_pbft_manager_eligible_wallet_period_wait(fact);
    if (!plan.should_wait) {
      break;
    }

    thisThreadSleepForMilliSeconds(plan.sleep_ms);
  }

  try {
    const auto period = pbft_chain_->getPbftChainSize();
    rustaxa::PbftFinalChainDposWalletAggregateVoteCountRequest request;
    request.period = period;
    const auto &wallets = eligible_wallets_.getWallets(getPbftPeriod());
    request.addresses.reserve(wallets.size());
    for (const auto &wallet : wallets) {
      if (!wallet.first) {
        continue;
      }
      rustaxa::PbftFinalChainDposAddress bridge_address;
      bridge_address.address = toBridgeFixedBytes<20>(wallet.second.node_addr);
      request.addresses.push_back(bridge_address);
    }

    const auto facts = pbft_service_->service().pbft_service_collect_dpos_wallet_aggregate_vote_count(
        final_chain_->rustFinalChain(), request);
    if (facts.status == kPbftSyncDposFactsReady && facts.has_aggregate_vote_count) {
      return facts.aggregate_vote_count;
    }
    LOG(log_wr_) << "Rust FinalChain PBFT node-vote fact collection failed for period " << period
                 << ". Period is too far ahead of actual finalized pbft chain size (" << facts.last_block_number
                 << "). Err msg: " << static_cast<std::string>(facts.error_code);
  } catch (const std::exception &e) {
    LOG(log_wr_) << "Rust FinalChain PBFT node-vote fact collection failed for period "
                 << pbft_chain_->getPbftChainSize() << ". Period is too far ahead of actual finalized pbft chain size ("
                 << final_chain_->lastBlockNumber() << "). Err msg: " << e.what();
  }

  return {};
}

bool PbftManager::tryPushCertVotesBlock() {
  const auto [current_pbft_round, current_pbft_period] = getPbftRoundAndPeriod();

  auto cert_voted_block = vote_mgr_->certVotedBlockSelection(current_pbft_period, current_pbft_round);
  if (!cert_voted_block.found) {
    return false;
  }
  const auto certified_block_hash = cert_voted_block.block_hash;

  LOG(log_nf_) << "Found enough cert votes for PBFT block " << certified_block_hash << ", period "
               << current_pbft_period << ", round " << current_pbft_round;

  auto pbft_block = getValidPbftProposedBlock(current_pbft_period, certified_block_hash);
  if (!pbft_block) {
    LOG(log_er_) << "Invalid certified block " << certified_block_hash;
    return false;
  }

  // Push pbft block into chain
  if (!pushCertVotedPbftBlockIntoChain_(pbft_block, std::move(cert_voted_block.votes))) {
    return false;
  }

  return true;
}

bool PbftManager::advancePeriod() {
  const auto chain_size = pbft_chain_->getPbftChainSize();
  return applyRustPlannedAdvancePeriod_(chain_size);
}

bool PbftManager::applyRustPlannedAdvancePeriod_(PbftPeriod finalized_chain_size) {
  const auto chain_size = finalized_chain_size;
  if (chain_size == 0) {
    LOG(log_er_) << "Rust PBFT manager advance-period rejected empty finalized chain before lifecycle reset";
    return false;
  }
  const auto new_period = checkedNextPbftPeriod(chain_size);
  if (!new_period.has_value()) {
    LOG(log_er_) << "Rust PBFT manager advance-period rejected finalized-chain overflow before lifecycle reset";
    return false;
  }
  printVotingSummary();
  const auto transition_result =
      applyLifecycleTransition_(kPbftManagerTransitionResetConsensus, *new_period, 1 /* round */, false);
  return applyRustPlannedAdvancePeriod_(chain_size, transition_result);
}

bool PbftManager::applyRustPlannedAdvancePeriod_(
    PbftPeriod finalized_chain_size, const rustaxa::PbftManagerLifecycleTransitionResult &transition_result) {
  const auto advance_plan =
      rustaxa::pbft_manager_runtime_plan_advance_period_after_reset(pbft_service_->service(), finalized_chain_size);
  if (!advance_plan.accepted) {
    LOG(log_er_) << "Rust PBFT manager advance-period planner rejected facts, chain size " << finalized_chain_size
                 << ", error " << static_cast<std::string>(advance_plan.error_code);
    return false;
  }

  uint64_t action_index = 0;
  for (const auto action : advance_plan.actions) {
    switch (action) {
      case kPbftManagerAdvancePeriodActionApplyExecutedBlockReset: {
        waitForPeriodFinalization();
        const auto reset_result = rustaxa::pbft_manager_runtime_apply_executed_block_reset(pbft_service_->service());
        if (reset_result.status != kPbftManagerTransitionStorageStatusApplied) {
          throw std::runtime_error("Rust PBFT manager executed-block reset failed: " +
                                   static_cast<std::string>(reset_result.error_code));
        }
        applyPbftManagerRuntimeSnapshot(
            reset_result.snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
            rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_, already_next_voted_value_,
            already_next_voted_null_block_hash_, broadcast_votes_counter_, rebroadcast_votes_counter_,
            broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_);
        break;
      }
      case kPbftManagerAdvancePeriodActionSetVoteManagerPeriodRound:
        vote_mgr_->applyRustPlannedPeriodRound(advance_plan.new_period, transition_result.snapshot.round);
        break;
      case kPbftManagerAdvancePeriodActionResetCurrentRoundTimer:
        current_round_start_datetime_ = std::chrono::system_clock::now();
        break;
      case kPbftManagerAdvancePeriodActionResetRewardVoteCounters: {
        const auto broadcast_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
        const auto reset_snapshot = rustaxa::pbft_manager_runtime_apply_broadcast_counters(
            pbft_service_->service(), broadcast_snapshot.broadcast_votes_counter,
            broadcast_snapshot.rebroadcast_votes_counter, 1, 1);
        if (reset_snapshot.status != kPbftManagerStartupRestoreStatusReady) {
          LOG(log_er_) << "Rust PBFT manager reward-vote counter reset rejected, error "
                       << static_cast<std::string>(reset_snapshot.error_code);
          return false;
        }
        applyPbftManagerRuntimeSnapshot(
            reset_snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
            rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_, already_next_voted_value_,
            already_next_voted_null_block_hash_, broadcast_votes_counter_, rebroadcast_votes_counter_,
            broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_);
        break;
      }
      case kPbftManagerAdvancePeriodActionResetPeriodTimer:
        current_period_start_datetime_ = std::chrono::system_clock::now();
        break;
      case kPbftManagerAdvancePeriodActionUpdateWalletEligibility:
        eligible_wallets_.updateWalletsEligibility(advance_plan.finalized_chain_size, pbft_service_, final_chain_);
        break;
      default:
        LOG(log_er_) << "Rust PBFT manager advance-period planner returned unknown action "
                     << static_cast<uint32_t>(action);
        return false;
    }
    rustaxa::PbftManagerAdvancePeriodActionReport action_report{};
    action_report.action_index = action_index;
    action_report.action = action;
    action_report.succeeded = true;
    const auto action_validation =
        rustaxa::validate_pbft_manager_advance_period_action_report(advance_plan, action_report);
    if (!action_validation.accepted) {
      LOG(log_er_) << "Rust PBFT manager advance-period action report rejected at index " << action_index << ", action "
                   << static_cast<uint32_t>(action) << ", status " << static_cast<uint32_t>(action_validation.status)
                   << ", error " << static_cast<std::string>(action_validation.error_code);
      return false;
    }
    ++action_index;
  }

  std::optional<rustaxa::PbftManagerRuntimeSnapshot> period_snapshot;
  try {
    period_snapshot =
        rustaxa::pbft_manager_runtime_apply_period_advance(pbft_service_->service(), advance_plan.new_period);
  } catch (const std::exception &e) {
    LOG(log_er_) << "Rust PBFT manager period-advance commit failed for new period " << advance_plan.new_period
                 << ", error " << e.what();
    return false;
  }
  if (period_snapshot->status != kPbftManagerStartupRestoreStatusReady) {
    LOG(log_er_) << "Rust PBFT manager period-advance runtime rejected new period " << advance_plan.new_period
                 << ", error " << static_cast<std::string>(period_snapshot->error_code);
    return false;
  }
  applyPbftManagerRuntimeSnapshot(*period_snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
                                  rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_,
                                  already_next_voted_value_, already_next_voted_null_block_hash_,
                                  broadcast_votes_counter_, rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                  rebroadcast_reward_votes_counter_);

  LOG(log_nf_) << "Period advanced to: " << advance_plan.new_period << ", round and step reset to 1";

  // Restart while loop...
  return true;
}

void PbftManager::resetPbftConsensus(PbftRound round) {
  // Print node's broadcasted votes for current round
  printVotingSummary();

  const auto period = getPbftPeriod();
  const auto result = applyLifecycleTransition_(kPbftManagerTransitionResetConsensus, period, round, false);

  if (result.reset_executed_block_follow_up) {
    waitForPeriodFinalization();
    const auto reset_result = rustaxa::pbft_manager_runtime_apply_executed_block_reset(pbft_service_->service());
    if (reset_result.status != kPbftManagerTransitionStorageStatusApplied) {
      throw std::runtime_error("Rust PBFT manager executed-block reset failed: " +
                               static_cast<std::string>(reset_result.error_code));
    }
    applyPbftManagerRuntimeSnapshot(
        reset_result.snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
        rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_, already_next_voted_value_,
        already_next_voted_null_block_hash_, broadcast_votes_counter_, rebroadcast_votes_counter_,
        broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_);
  }
  if (result.set_vote_manager_period_round) {
    vote_mgr_->applyRustPlannedPeriodRound(period, result.snapshot.round);
  }
  if (result.reset_current_round_timer) {
    current_round_start_datetime_ = std::chrono::system_clock::now();
  }

  const auto reset_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  ensurePbftManagerRuntimeSnapshotReady(reset_snapshot, "PBFT consensus reset log");
  LOG(log_nf_) << "Reset PBFT consensus to: period " << period << ", round " << reset_snapshot.round << ", step "
               << reset_snapshot.step << ", lambda " << reset_snapshot.current_round_lambda_ms << " [ms]";
}

std::chrono::milliseconds PbftManager::elapsedTimeInMs(const time_point &start_time) {
  return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::system_clock::now() - start_time);
}

void PbftManager::sleep_() {
  // Run "wait_for" sleep in loop due to potential spurious wakeup on lock
  if (!pbft_service_) {
    throw std::runtime_error("PBFT manager runtime must be initialized before sleep");
  }
  while (!stopped_) {
    const auto round_elapsed_time = elapsedTimeInMs(current_round_start_datetime_);
    rustaxa::PbftManagerSleepPlan sleep_plan =
        rustaxa::plan_pbft_manager_runtime_sleep_until_next_step(pbft_service_->service(), round_elapsed_time.count());
    if (!sleep_plan.accepted) {
      throw std::runtime_error("PBFT manager Rust sleep plan rejected: " +
                               static_cast<std::string>(sleep_plan.error_code));
    }
    if (!sleep_plan.should_sleep) {
      return;
    }

    const auto time_to_sleep_for_ms = std::chrono::milliseconds(sleep_plan.sleep_ms);
    const auto [round, period] = getPbftRoundAndPeriod();
    LOG(log_tr_) << "Sleep " << time_to_sleep_for_ms.count() << " [ms] before going into the next step. Period "
                 << period << ", round " << round << ", step " << static_cast<PbftStep>(sleep_plan.step);
    std::unique_lock<std::mutex> lock(stop_mtx_);
    stop_cv_.wait_for(lock, time_to_sleep_for_ms);
  }
}

void PbftManager::initialState() {
  // Initial PBFT state

  // Time constants...
  const auto current_pbft_period = getPbftPeriod();
  const auto now = std::chrono::system_clock::now();

  if (!pbft_service_) {
    LOG(log_er_) << "Rust PBFT manager runtime was not provided before initialState";
    assert(false);
  }
  const auto startup_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  applyPbftManagerRuntimeSnapshot(startup_snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
                                  rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_,
                                  already_next_voted_value_, already_next_voted_null_block_hash_,
                                  broadcast_votes_counter_, rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                  rebroadcast_reward_votes_counter_);
  if (startup_snapshot.reset_second_finish_start) {
    second_finish_step_start_datetime_ = now;
  }
  const auto current_pbft_round = static_cast<PbftRound>(startup_snapshot.round);
  const auto current_pbft_step = static_cast<PbftStep>(startup_snapshot.step);

  // Load proposed-block startup metadata through the Rust-owned proposed-block
  // index. This preserves canonical block bytes for later network/public
  // materialization without scanning `DbStorage` into live C++ `PbftBlock`
  // objects during PBFT manager startup.
  // Process saved cert voted block from Rust storage through the PBFT runtime.
  const auto cert_voted_block_payload =
      rustaxa::pbft_manager_runtime_cert_voted_block_in_round(pbft_service_->service());
  if (!cert_voted_block_payload.empty()) {
    const auto payload_bytes = dev::bytes(cert_voted_block_payload.begin(), cert_voted_block_payload.end());
    const auto payload_rlp = dev::RLP(payload_bytes);
    assert(payload_rlp.itemCount() == 2);
    const auto cert_voted_block_round = payload_rlp[0].toInt<PbftRound>();
    const auto cert_voted_block = std::make_shared<PbftBlock>(payload_rlp[1]);
    if (publishProposedBlock(cert_voted_block)) {
      LOG(log_nf_) << "Last cert voted block " << cert_voted_block->getBlockHash() << " with period "
                   << cert_voted_block->getPeriod() << ", round " << cert_voted_block_round
                   << " pushed into proposed blocks";
    }

    // Set cert_voted_block_for_round_ only if round and period match. Note: could differ in edge case when node
    // crashed, new period/round was already saved in db but cert voted block was not cleared yet
    if (current_pbft_period == cert_voted_block->getPeriod() && current_pbft_round == cert_voted_block_round) {
      const auto cert_voted_snapshot = rustaxa::pbft_manager_runtime_apply_cert_voted_block_metadata(
          pbft_service_->service(), cert_voted_block->getPeriod(), cert_voted_block_round,
          toBridgeHash(cert_voted_block->getBlockHash()));
      if (cert_voted_snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
        throw std::runtime_error("Rust PBFT manager cert-voted metadata restore rejected: " +
                                 static_cast<std::string>(cert_voted_snapshot.error_code));
      }
      cert_voted_block_for_round_ = cert_voted_block;
      LOG(log_nf_) << "Init last cert voted block in round to " << cert_voted_block->getBlockHash() << ", period "
                   << current_pbft_period << ", round " << current_pbft_round;
    }
  }

  current_round_start_datetime_ = now;
  current_period_start_datetime_ = now;

  waitForPeriodFinalization();

  const auto previous_round_next_vote_facts =
      vote_mgr_->applyStartupPeriodRoundAndLogFacts(current_pbft_period, current_pbft_round);

  LOG(log_nf_) << "Node initialize at period " << current_pbft_period << ", round " << current_pbft_round << ", step "
               << current_pbft_step << ". Previous round 2t+1 next voted null block: " << std::boolalpha
               << previous_round_next_vote_facts.next_voted_null_block << ", previous round 2t+1 next voted block "
               << (previous_round_next_vote_facts.next_voted_block.has_value()
                       ? previous_round_next_vote_facts.next_voted_block->abridged()
                       : "no value");
}

rustaxa::PbftManagerLifecycleTransitionResult PbftManager::applyLifecycleTransition_(uint8_t kind,
                                                                                     PbftPeriod target_period,
                                                                                     PbftRound target_round,
                                                                                     bool apply_current_round_timer) {
  const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  auto request =
      makePbftManagerLifecycleTransitionRequest(kind, target_period, target_round, *vote_mgr_, transition_snapshot);
  return executePbftManagerLifecycleTransition(
      std::move(request), pbft_service_->service(), round_, step_, state_, current_round_lambda_, next_step_time_ms_,
      rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_, cert_voted_block_for_round_,
      current_round_broadcasted_votes_, broadcast_votes_counter_, rebroadcast_votes_counter_,
      broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_, already_next_voted_value_,
      already_next_voted_null_block_hash_, printCertStepInfo_, printSecondFinishStepInfo_,
      current_round_start_datetime_, second_finish_step_start_datetime_, apply_current_round_timer);
}

void PbftManager::setFilterState_() { applyLifecycleTransition_(kPbftManagerTransitionToFilter, getPbftPeriod()); }

void PbftManager::setCertifyState_() { applyLifecycleTransition_(kPbftManagerTransitionToCertify, getPbftPeriod()); }

void PbftManager::setFinishState_() {
  LOG(log_dg_) << "Will go to first finish State";
  applyLifecycleTransition_(kPbftManagerTransitionToFinish, getPbftPeriod());
}

void PbftManager::setFinishPollingState_() {
  applyLifecycleTransition_(kPbftManagerTransitionToFinishPolling, getPbftPeriod());
}

void PbftManager::loopBackFinishState_() {
  applyLifecycleTransition_(kPbftManagerTransitionLoopBackFinish, getPbftPeriod());
}

void PbftManager::broadcastVotes() {
  auto net = network_.lock();
  if (!net) {
    LOG(log_er_) << "Unable to broadcast votes -> cant obtain net ptr";
    return;
  }

  // Send votes to the other peers
  auto gossipVotes = [this, &net](const std::vector<std::shared_ptr<PbftVote>> &votes,
                                  const std::string &votes_type_str, bool rebroadcast) {
    if (!votes.empty()) {
      LOG(log_dg_) << "Broadcast " << votes_type_str << " for period " << votes.back()->getPeriod() << ", round "
                   << votes.back()->getRound();
      net->gossipVotesBundle(votes, rebroadcast);
    }
  };

  // (Re)broadcast reward votes + all own pbft and pillar votes
  auto stuckPeriodBroadcastVotes =
      [this, &net, &gossipVotes](const VoteManager::StuckRoundVoteBroadcastPayloads &vote_payloads, bool rebroadcast) {
        auto [round, period] = getPbftRoundAndPeriod();

        gossipVotes(vote_payloads.reward_votes, "Reward votes", rebroadcast);

        // Broadcast own pbft votes - send votes by one as they have different type, period, round, step
        if (!vote_payloads.own_votes.empty()) {
          for (const auto &vote : vote_payloads.own_votes) {
            net->gossipVote(vote, getPbftProposedBlock(vote->getPeriod(), vote->getBlockHash()), rebroadcast);
          }

          LOG(log_dg_) << "Broadcast own votes for period " << period << ", round " << round << ", rebroadcast "
                       << rebroadcast;
        }

        // Broadcast own pillar vote
        const auto own_pillar_vote_rlp = rustaxa::pbft_manager_runtime_own_pillar_block_vote(pbft_service_->service());
        if (!own_pillar_vote_rlp.empty()) {
          const auto payload_bytes = dev::bytes(own_pillar_vote_rlp.begin(), own_pillar_vote_rlp.end());
          const auto own_pillar_vote = std::make_shared<PillarVote>(dev::RLP(payload_bytes));
          net->gossipPillarBlockVote(own_pillar_vote, rebroadcast);
        }
      };

  // (Re)broadcast 2t+1 soft/reward/previous round next votes + all own votes
  auto stuckRoundBroadcastVotes = [this, &gossipVotes, &stuckPeriodBroadcastVotes](bool rebroadcast) {
    auto [round, period] = getPbftRoundAndPeriod();

    auto vote_payloads = vote_mgr_->stuckRoundVoteBroadcastPayloads(period, round);
    stuckPeriodBroadcastVotes(vote_payloads, rebroadcast);

    // Broadcast 2t+1 soft votes
    gossipVotes(std::move(vote_payloads.soft_votes), "2t+1 soft votes", rebroadcast);
    // Broadcast previous round 2t+1 next votes
    if (round > 1) {
      gossipVotes(std::move(vote_payloads.previous_round_next_votes), "2t+1 next votes", rebroadcast);
      gossipVotes(std::move(vote_payloads.previous_round_next_null_votes), "2t+1 next null votes", rebroadcast);
    }
  };

  const auto round_elapsed_time = elapsedTimeInMs(current_round_start_datetime_);
  const auto period_elapsed_time = elapsedTimeInMs(current_period_start_datetime_);
  const auto broadcast_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  if (broadcast_snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
    LOG(log_er_) << "Rust PBFT broadcast snapshot rejected, error "
                 << static_cast<std::string>(broadcast_snapshot.error_code);
    assert(false);
    return;
  }

  rustaxa::PbftManagerBroadcastFact fact;
  fact.round_elapsed_ms = toBroadcastElapsedMs(round_elapsed_time);
  fact.period_elapsed_ms = toBroadcastElapsedMs(period_elapsed_time);
  fact.current_round_lambda_ms = broadcast_snapshot.current_round_lambda_ms;
  fact.broadcast_lambda_threshold = kBroadcastVotesLambdaTime;
  fact.rebroadcast_lambda_threshold = kRebroadcastVotesLambdaTime;
  fact.broadcast_votes_counter = broadcast_snapshot.broadcast_votes_counter;
  fact.rebroadcast_votes_counter = broadcast_snapshot.rebroadcast_votes_counter;
  fact.broadcast_reward_votes_counter = broadcast_snapshot.broadcast_reward_votes_counter;
  fact.rebroadcast_reward_votes_counter = broadcast_snapshot.rebroadcast_reward_votes_counter;

  auto plan = rustaxa::plan_pbft_manager_broadcast(fact);
  if (plan.status != kPbftManagerBroadcastStatusReady) {
    LOG(log_er_) << "Rust PBFT broadcast planner rejected facts, status " << static_cast<uint32_t>(plan.status)
                 << ", error " << static_cast<std::string>(plan.error_code);
    assert(false);
    return;
  }

  if (plan.action == kPbftManagerBroadcastActionNoop) {
    return;
  }

  rustaxa::PbftManagerBroadcastReport report;
  report.action = plan.action;
  report.rebroadcast = plan.rebroadcast;
  report.success = false;
  try {
    if (plan.action == kPbftManagerBroadcastActionRoundVotes) {
      stuckRoundBroadcastVotes(plan.rebroadcast);
      report.success = true;
    } else if (plan.action == kPbftManagerBroadcastActionPeriodVotes) {
      auto [round, period] = getPbftRoundAndPeriod();
      auto vote_payloads = vote_mgr_->stuckRoundVoteBroadcastPayloads(period, round);
      stuckPeriodBroadcastVotes(vote_payloads, plan.rebroadcast);
      report.success = true;
    } else {
      report.error_code = "PBFT_MANAGER_BROADCAST_UNSUPPORTED_ACTION";
    }
  } catch (const std::exception &e) {
    report.error_code = std::string("PBFT_MANAGER_BROADCAST_EXECUTOR_EXCEPTION: ") + e.what();
  } catch (...) {
    report.error_code = "PBFT_MANAGER_BROADCAST_EXECUTOR_UNKNOWN_EXCEPTION";
  }

  const auto result = rustaxa::report_pbft_manager_broadcast(std::move(plan), report);
  if (result.status != kPbftManagerBroadcastStatusReady) {
    LOG(log_er_) << "Rust PBFT broadcast report rejected, status " << static_cast<uint32_t>(result.status) << ", error "
                 << static_cast<std::string>(result.error_code);
    assert(result.status != 3);
    return;
  }

  if (result.apply_counters) {
    const auto counter_snapshot = rustaxa::pbft_manager_runtime_apply_broadcast_counters(
        pbft_service_->service(), result.broadcast_votes_counter, result.rebroadcast_votes_counter,
        result.broadcast_reward_votes_counter, result.rebroadcast_reward_votes_counter);
    applyPbftManagerRuntimeSnapshot(counter_snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
                                    rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_,
                                    already_next_voted_value_, already_next_voted_null_block_hash_,
                                    broadcast_votes_counter_, rebroadcast_votes_counter_,
                                    broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_);
  }
}

void PbftManager::testBroadcastVotesFunctionality() {
  // Set these variables to force broadcastVotes() send votes
  current_round_start_datetime_ = time_point{};
  current_period_start_datetime_ = time_point{};
  const auto counter_snapshot = rustaxa::pbft_manager_runtime_apply_broadcast_counters(
      pbft_service_->service(), kBroadcastVotesLambdaTime, kRebroadcastVotesLambdaTime, kBroadcastVotesLambdaTime,
      kRebroadcastVotesLambdaTime);
  applyPbftManagerRuntimeSnapshot(counter_snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
                                  rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_,
                                  already_next_voted_value_, already_next_voted_null_block_hash_,
                                  broadcast_votes_counter_, rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                  rebroadcast_reward_votes_counter_);

  broadcastVotes();
}

void PbftManager::printVotingSummary() const {
  const auto [round, period] = getPbftRoundAndPeriod();
  Json::Value json_obj;

  json_obj["period"] = Json::UInt64(period - 1);
  json_obj["round"] = Json::UInt64(round);
  auto &steps_voted_blocks_json = json_obj["voted_blocks_steps"] = Json::Value(Json::arrayValue);

  for (const auto &voted_blocks_steps : current_round_broadcasted_votes_) {
    const auto voted_block_hash = voted_blocks_steps.first;
    auto &voted_blocks_steps_json = steps_voted_blocks_json.append(Json::Value(Json::objectValue));
    auto &steps_json = voted_blocks_steps_json[voted_block_hash.abridged().substr(0, 8)] =
        Json::Value(Json::arrayValue);
    for (const auto &step : voted_blocks_steps.second) {
      steps_json.append(step);
    }
  }

  LOG(log_nf_) << "Voting summary: " << jsonToUnstyledString(json_obj);
}

bool PbftManager::publishProposedBlock(const std::shared_ptr<PbftBlock> &proposed_block) {
  if (!proposed_block) {
    throw std::runtime_error("Cannot publish null proposed PBFT block");
  }
  return pbft_service_->service().pbft_service_publish_proposed_block(
      proposed_block->getPeriod(), toBridgeHash(proposed_block->getBlockHash()),
      toBridgeHash(proposed_block->getPivotDagBlockHash()), toBridgeBytes(proposed_block->rlp(true)));
}

std::shared_ptr<PbftBlock> PbftManager::getValidPbftProposedBlock(PbftPeriod period, const blk_hash_t &block_hash) {
  rustaxa::PbftManagerCandidateAdmissionFact fact;
  fact.period = period;
  fact.block_hash = toBridgeHash(block_hash);
  fact.lookup_performed = false;
  fact.proposed_block_found = false;
  fact.proposed_block_already_valid = false;
  fact.validation_status = kPbftManagerCandidateAdmissionValidationNotChecked;

  std::shared_ptr<PbftBlock> block;
  std::optional<rustaxa::ProposedBlockLookup> lookup;
  while (true) {
    const auto plan = rustaxa::plan_pbft_manager_candidate_admission(fact);
    if (plan.action == kPbftManagerCandidateAdmissionActionAccept) {
      if (!block) {
        // Rust admission decisions use owned proposed-block lookup facts. C++ materializes the accepted block only at
        // this vote-generation/executor boundary.
        if (!lookup.has_value() || !lookup->found) {
          throw std::runtime_error("Rust PBFT proposed-block admission accepted missing materialized block");
        }
        block = std::make_shared<PbftBlock>(fromBridgeBytes(lookup->block_rlp));
      }
      if (plan.mark_valid) {
        pbft_service_->service().pbft_service_proposed_blocks_mark_valid(period, toBridgeHash(block_hash));
      }
      return block;
    }
    if (plan.action == kPbftManagerCandidateAdmissionActionReject) {
      LOG(log_er_) << "Proposed block " << block_hash << " rejected by Rust admission planner, period " << period
                   << ", status " << static_cast<uint64_t>(plan.status) << ", code " << std::string(plan.error_code);
      return nullptr;
    }
    if (plan.action == kPbftManagerCandidateAdmissionActionDeferMissingBlock) {
      LOG(log_dg_) << "Proposed block " << block_hash << " deferred by Rust admission planner, period " << period
                   << ", status " << static_cast<uint64_t>(plan.status) << ", code " << std::string(plan.error_code);
      return nullptr;
    }
    if (plan.action == kPbftManagerCandidateAdmissionActionContractError) {
      throw std::runtime_error("Rust PBFT proposed-block admission planner rejected bridge facts: " +
                               std::string(plan.error_code));
    }

    if (plan.action == kPbftManagerCandidateAdmissionActionRequestLookup) {
      lookup = pbft_service_->service().pbft_service_proposed_blocks_get(period, toBridgeHash(block_hash));
      fact.lookup_performed = true;
      if (!lookup->found) {
        LOG(log_er_) << "Unable to find proposed block " << block_hash << ", period " << period;
        fact.proposed_block_found = false;
        continue;
      }

      fact.proposed_block_found = true;
      fact.proposed_block_already_valid = lookup->is_valid;
      continue;
    }

    if (plan.action == kPbftManagerCandidateAdmissionActionRequestValidation) {
      if (!block) {
        if (!lookup.has_value() || !lookup->found) {
          LOG(log_er_) << "Unable to materialize proposed block " << block_hash << " for validation, period " << period;
          fact.validation_status = kPbftManagerCandidateAdmissionValidationInvalid;
          continue;
        }
        block = std::make_shared<PbftBlock>(fromBridgeBytes(lookup->block_rlp));
      }
      if (!validatePbftBlock(block)) {
        LOG(log_er_) << "Proposed block " << block_hash << " failed validation, period " << period;
        fact.validation_status = kPbftManagerCandidateAdmissionValidationInvalid;
      } else {
        fact.validation_status = kPbftManagerCandidateAdmissionValidationValid;
      }
      continue;
    }

    throw std::runtime_error("Rust PBFT proposed-block admission planner returned unknown action");
  }
}

std::shared_ptr<PbftBlock> PbftManager::admitStateActionPbftBlock(const rustaxa::PbftManagerStateActionEffect &effect,
                                                                  std::string_view action_context) {
  if (!effect.request_proposed_block_sidecar) {
    throw std::runtime_error(std::string(action_context) +
                             ": Rust PBFT state-action effect did not request proposed-block sidecar");
  }
  const auto period = static_cast<PbftPeriod>(effect.proposed_block_sidecar_period);
  const auto block_hash = fromBridgeHash(effect.proposed_block_sidecar_hash);
  if (block_hash != fromBridgeHash(effect.hash)) {
    throw std::runtime_error(std::string(action_context) +
                             ": Rust PBFT state-action effect sidecar hash does not match effect hash");
  }

  auto block = getValidPbftProposedBlock(period, block_hash);
  if (!block) {
    LOG(log_er_) << action_context << ": Rust proposed-block admission rejected " << block_hash << ". Period " << period
                 << ", round " << getPbftRound();
    return nullptr;
  }

  assert(block->getPeriod() == period);
  assert(block->getBlockHash() == block_hash);
  return block;
}

bool PbftManager::genAndPlaceVote(PbftVoteTypes vote_type, PbftPeriod period, PbftRound round, PbftStep step,
                                  const blk_hash_t &block_hash, std::shared_ptr<PbftBlock> pbft_block) {
  if (pbft_block) {
    assert(pbft_block->getPeriod() == period);
    assert(pbft_block->getBlockHash() == block_hash);
  }

  // In case it is pbft with pillar block period and we have not voted yet, place a pillar vote (can be placed during
  // any pbft step)
  std::optional<blk_hash_t> place_pillar_vote_for_block;
  if (kGenesisConfig.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(period) &&
      last_placed_pillar_vote_period_ < period) {
    if (pbft_block) {
      // No need to check presence of extra data and pillar block hash - this was already validated in validatePbftBlock
      place_pillar_vote_for_block = pbft_block->getExtraData()->getPillarBlockHash();
    } else {
      rustaxa::PillarCurrentAnchorDecisionRequest request{};
      request.operation = kPillarAnchorDecisionSelectPreviousPeriod;
      request.pbft_period = period;
      const auto local_pillar_vote_anchor =
          requireReadyPillarService(pbft_service_).pbft_service_pillar_plan_current_anchor_decision(request);
      if (local_pillar_vote_anchor.selected && local_pillar_vote_anchor.has_current_anchor) {
        place_pillar_vote_for_block = fromBridgeHash(local_pillar_vote_anchor.current_hash);
      }
    }
  }

  bool success = false;
  std::vector<std::shared_ptr<PbftVote>> valid_votes;
  uint64_t valid_votes_weight = 0;
  for (const auto &wallet : eligible_wallets_.getWallets(period)) {
    // Wallet is not dpos eligible - do no vote
    if (!wallet.first) {
      continue;
    }

    auto local_vote_placement =
        vote_mgr_->generateAndPlaceLocalVote(block_hash, vote_type, period, round, step, wallet.second);
    if (!local_vote_placement.placed) {
      LOG(log_er_) << local_vote_placement.error;
      continue;
    }
    auto vote = std::move(local_vote_placement.vote);

    // Propose votes are sent as single packets so it is gossiped together with pbft block
    if (vote_type == PbftVoteTypes::propose_vote) {
      gossipNewOwnVote(vote, pbft_block);

      LOG(log_nf_) << "Placed and sent " << vote->getHash() << " vote for block " << block_hash << ", vote weight "
                   << *vote->getWeight() << ", period " << period << ", round " << round << ", step " << step
                   << ", validator " << wallet.second.node_addr;
    } else {
      valid_votes_weight += *vote->getWeight();
      LOG(log_nf_) << "Placed " << vote->getHash() << " vote for block " << block_hash << ", vote weight "
                   << *vote->getWeight() << ", period " << period << ", round " << round << ", step " << step
                   << ", validator " << wallet.second.node_addr;
      valid_votes.push_back(std::move(vote));
    }

    if (place_pillar_vote_for_block.has_value()) {
      const auto pillar_vote = pillar_chain_mgr_->genAndPlacePillarVote(period, *place_pillar_vote_for_block,
                                                                        wallet.second.node_secret, true);
      if (pillar_vote) {
        last_placed_pillar_vote_period_ = pillar_vote->getPeriod();
      }
    }
    success = true;
  }

  // Gossip all generated votes in single packet
  if (!valid_votes.empty()) {
    if (valid_votes.size() == 1) {
      const auto &vote = valid_votes.front();
      gossipNewOwnVote(vote, pbft_block);
      LOG(log_nf_) << "Sent " << vote->getHash() << " vote for block " << block_hash << ", vote weight "
                   << *vote->getWeight() << ", period " << period << ", round " << round << ", step " << step;
    } else {
      gossipNewOwnVotesBundle(valid_votes);
      LOG(log_nf_) << "Votes bundle with " << valid_votes.size() << " votes with overall weight " << valid_votes_weight
                   << " for block " << block_hash << ", period " << period << ", round " << round << ", step " << step
                   << " sent";
    }
  }

  return success;
}

bool PbftManager::placeStateActionVote(PbftVoteTypes vote_type, PbftPeriod period, PbftRound round, PbftStep step,
                                       const blk_hash_t &block_hash, std::shared_ptr<PbftBlock> pbft_block,
                                       std::string_view action_context, std::optional<PbftMgrStatus> next_vote_status) {
  const auto placed = genAndPlaceVote(vote_type, period, round, step, block_hash, std::move(pbft_block));
  if (!placed) {
    LOG(log_dg_) << action_context << ": failed to generate and place vote type " << static_cast<uint32_t>(vote_type)
                 << " for " << block_hash << ". Period " << period << ", round " << round << ", step " << step;
    return false;
  }

  if (next_vote_status.has_value()) {
    if (!pbft_service_) {
      throw std::runtime_error("PBFT manager runtime is required for next-voted status persistence");
    }
    const auto next_voted_snapshot = rustaxa::pbft_manager_runtime_apply_next_voted_status(
        pbft_service_->service(), static_cast<uint8_t>(*next_vote_status));
    applyPbftManagerRuntimeSnapshot(
        next_voted_snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
        rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_, already_next_voted_value_,
        already_next_voted_null_block_hash_, broadcast_votes_counter_, rebroadcast_votes_counter_,
        broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_);
  }

  return true;
}

bool PbftManager::genAndPlaceProposeVote(const std::shared_ptr<PbftBlock> &proposed_block,
                                         std::vector<std::shared_ptr<PbftVote>> &&reward_votes) {
  const auto [current_pbft_round, current_pbft_period] = getPbftRoundAndPeriod();
  const auto current_pbft_step = getPbftStep();

  if (proposed_block->getPeriod() != current_pbft_period) {
    LOG(log_er_) << "Propose block " << proposed_block->getBlockHash()
                 << " has different period than current pbft period " << current_pbft_period;
    return false;
  }

  // Broadcast reward votes - previous round 2t+1 cert votes
  if (auto net = network_.lock()) {
    LOG(log_dg_) << "Broadcast propose block reward votes for block " << proposed_block->getBlockHash()
                 << ", num of reward votes: " << reward_votes.size() << ", period " << current_pbft_period << ", round "
                 << current_pbft_round;
    net->gossipVotesBundle(reward_votes, false);
  }

  if (!genAndPlaceVote(PbftVoteTypes::propose_vote, current_pbft_period, current_pbft_round, current_pbft_step,
                       proposed_block->getBlockHash(), proposed_block)) {
    LOG(log_nf_) << "Unable to generate and place propose vote";
    return false;
  }

  return true;
}

void PbftManager::gossipNewOwnVote(const std::shared_ptr<PbftVote> &vote,
                                   const std::shared_ptr<PbftBlock> &voted_block) {
  // Always rebroadcast pbft block together with new own propose vote
  bool rebroadcast = (vote->getType() == PbftVoteTypes::propose_vote);
  gossipVote(vote, voted_block, rebroadcast);

  auto found_voted_block_it = current_round_broadcasted_votes_.find(vote->getBlockHash());
  if (found_voted_block_it == current_round_broadcasted_votes_.end()) {
    found_voted_block_it = current_round_broadcasted_votes_.insert({vote->getBlockHash(), {}}).first;
  }

  found_voted_block_it->second.emplace_back(vote->getStep());
}

void PbftManager::gossipNewOwnVotesBundle(const std::vector<std::shared_ptr<PbftVote>> &votes) {
  auto net = network_.lock();
  if (!net) {
    LOG(log_er_) << "Could not obtain net - cannot gossip new own votes bundle";
    // assert(false);
    return;
  }

  net->gossipVotesBundle(votes);

  for (const auto &vote : votes) {
    auto found_voted_block_it = current_round_broadcasted_votes_.find(vote->getBlockHash());
    if (found_voted_block_it == current_round_broadcasted_votes_.end()) {
      found_voted_block_it = current_round_broadcasted_votes_.insert({vote->getBlockHash(), {}}).first;
    }

    found_voted_block_it->second.emplace_back(vote->getStep());
  }
}

void PbftManager::gossipVote(const std::shared_ptr<PbftVote> &vote, const std::shared_ptr<PbftBlock> &voted_block,
                             bool rebroadcast) {
  assert(!voted_block || vote->getBlockHash() == voted_block->getBlockHash());

  auto net = network_.lock();
  if (!net) {
    LOG(log_er_) << "Could not obtain net - cannot gossip new own vote";
    // assert(false);
    return;
  }

  net->gossipVote(vote, voted_block, rebroadcast);
}

void PbftManager::proposeBlock_() {
  // Value Proposal
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  const auto state = fromPbftManagerRuntimeState(action_snapshot.state);
  const auto round = static_cast<PbftRound>(action_snapshot.round);
  const auto step = static_cast<PbftStep>(action_snapshot.step);
  const auto current_round_lambda = std::chrono::milliseconds(action_snapshot.current_round_lambda_ms);
  const auto period = getPbftPeriod();
  LOG(log_dg_) << "PBFT value proposal state in period " << period << ", round " << round;

  const auto fact = makePbftManagerStateActionFact(
      state, period, round, step, 0ms, getPbftDeadline(), current_round_lambda, *vote_mgr_,
      action_snapshot.has_cert_voted_block, fromBridgeHash(action_snapshot.cert_voted_block_hash),
      action_snapshot.already_next_voted_value, action_snapshot.already_next_voted_null);
  executeStateActionEffectSession(
      pbft_service_->service(), fact,
      [&](const auto &effect) {
        if (effect.intent == kPbftManagerStateActionIntentProposeNewBlock) {
          LOG(log_nf_) << " 2t+1 next voted kNullBlockHash in previous round " << round - 1;

          if (auto proposed_block_data = proposePbftBlock(); proposed_block_data.has_value()) {
            if (auto net = network_.lock()) {
              LOG(log_dg_) << "Broadcast propose block reward votes for block "
                           << proposed_block_data->pbft_block->getBlockHash()
                           << ", num of reward votes: " << proposed_block_data->reward_votes.size() << ", period "
                           << period << ", round " << round;
              net->gossipVotesBundle(proposed_block_data->reward_votes, false);
            }

            gossipNewOwnVote(proposed_block_data->vote, proposed_block_data->pbft_block);

            LOG(log_nf_) << "Placed " << proposed_block_data->vote->getHash() << " propose vote for block "
                         << proposed_block_data->pbft_block->getBlockHash() << ", vote weight "
                         << *proposed_block_data->vote->getWeight() << ", period " << period << ", round " << round
                         << ", step " << step << ", validator " << proposed_block_data->vote->getVoterAddr();
            return kPbftManagerStateActionEffectResultApplied;
          }
          return kPbftManagerStateActionEffectResultSkippedNoWork;
        }

        if (effect.intent == kPbftManagerStateActionIntentReproposePreviousRoundNextValue) {
          assert(round > 1);

          const auto next_voted_block_hash = fromBridgeHash(effect.hash);

          const auto next_voted_block = admitStateActionPbftBlock(effect, "Value proposal re-propose");
          if (!next_voted_block) {
            return kPbftManagerStateActionEffectResultSkippedMissingLiveObject;
          }

          auto block_reward_votes = vote_mgr_->collectRewardVotesForBlock(next_voted_block);
          if (!block_reward_votes.has_value()) {
            LOG(log_er_) << "Unable to re-propose previous round next voted block " << next_voted_block_hash
                         << ", period " << period << ", round " << round;
            return kPbftManagerStateActionEffectResultRejectedLiveCheck;
          }

          return genAndPlaceProposeVote(next_voted_block, std::move(*block_reward_votes))
                     ? kPbftManagerStateActionEffectResultApplied
                     : kPbftManagerStateActionEffectResultRejectedLiveCheck;
        }

        LOG(log_er_) << "Unsupported Rust PBFT value proposal effect " << static_cast<uint32_t>(effect.intent);
        assert(false);
        return kPbftManagerStateActionEffectResultExecutorError;
      },
      log_er_);
}

void PbftManager::identifyBlock_() {
  // The Filtering Step
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  const auto state = fromPbftManagerRuntimeState(action_snapshot.state);
  const auto round = static_cast<PbftRound>(action_snapshot.round);
  const auto step = static_cast<PbftStep>(action_snapshot.step);
  const auto current_round_lambda = std::chrono::milliseconds(action_snapshot.current_round_lambda_ms);
  const auto period = getPbftPeriod();
  LOG(log_dg_) << "PBFT filtering state in period: " << period << ", round: " << round;

  const auto fact = makePbftManagerStateActionFact(
      state, period, round, step, 0ms, getPbftDeadline(), current_round_lambda, *vote_mgr_,
      action_snapshot.has_cert_voted_block, fromBridgeHash(action_snapshot.cert_voted_block_hash),
      action_snapshot.already_next_voted_value, action_snapshot.already_next_voted_null);
  executeStateActionEffectSession(
      pbft_service_->service(), fact,
      [&](const auto &effect) {
        if (effect.intent == kPbftManagerStateActionIntentIdentifyLeaderAndSoftVote) {
          const auto leader_block_data = vote_mgr_->identifyLeaderBlock(
              period, round, [this](const auto &proposed_block) { return validatePbftBlock(proposed_block); });
          if (!leader_block_data.has_value()) {
            LOG(log_dg_) << "No leader block identified. Period " << period << ", round " << round;
            return kPbftManagerStateActionEffectResultSkippedNoWork;
          }

          assert(leader_block_data->first->getPeriod() == period);
          LOG(log_dg_) << "Leader block identified " << leader_block_data->first->getBlockHash() << ", period "
                       << period << ", round " << round;

          return placeStateActionVote(PbftVoteTypes::soft_vote, leader_block_data->first->getPeriod(), round, step,
                                      leader_block_data->first->getBlockHash(), leader_block_data->first,
                                      "Filter leader soft vote")
                     ? kPbftManagerStateActionEffectResultApplied
                     : kPbftManagerStateActionEffectResultRejectedLiveCheck;
        }

        if (effect.intent == kPbftManagerStateActionIntentSoftVotePreviousRoundNextValue) {
          const auto next_voted_block_hash = fromBridgeHash(effect.hash);
          const auto next_voted_block = admitStateActionPbftBlock(effect, "Filter soft-vote previous round next value");
          if (!next_voted_block) {
            return kPbftManagerStateActionEffectResultSkippedMissingLiveObject;
          }

          return placeStateActionVote(PbftVoteTypes::soft_vote, period, round, step, next_voted_block_hash,
                                      next_voted_block, "Filter previous-round soft vote")
                     ? kPbftManagerStateActionEffectResultApplied
                     : kPbftManagerStateActionEffectResultRejectedLiveCheck;
        }

        LOG(log_er_) << "Unsupported Rust PBFT filter effect " << static_cast<uint32_t>(effect.intent);
        assert(false);
        return kPbftManagerStateActionEffectResultExecutorError;
      },
      log_er_);
}

void PbftManager::certifyBlock_() {
  // The Certifying Step
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  const auto state = fromPbftManagerRuntimeState(action_snapshot.state);
  const auto round = static_cast<PbftRound>(action_snapshot.round);
  const auto step = static_cast<PbftStep>(action_snapshot.step);
  const auto current_round_lambda = std::chrono::milliseconds(action_snapshot.current_round_lambda_ms);
  const auto period = getPbftPeriod();

  if (printCertStepInfo_) {
    LOG(log_dg_) << "PBFT certifying state in period " << period << ", round " << round;
    printCertStepInfo_ = false;
  }

  const auto elapsed_time_in_round = elapsedTimeInMs(current_round_start_datetime_);
  const auto fact = makePbftManagerStateActionFact(
      state, period, round, step, elapsed_time_in_round, getPbftDeadline(), current_round_lambda, *vote_mgr_,
      action_snapshot.has_cert_voted_block, fromBridgeHash(action_snapshot.cert_voted_block_hash),
      action_snapshot.already_next_voted_value, action_snapshot.already_next_voted_null);
  const auto session_step = executeStateActionEffectSession(
      pbft_service_->service(), fact,
      [&](const auto &effect) {
        if (effect.intent == kPbftManagerStateActionIntentGoFinish) {
          LOG(log_dg_) << "Step 3 expired, will go to step 4 in period " << period << ", round " << round;

          LOG(log_dg_) << vote_mgr_->softVoteDebugMessage(period, round);

          return kPbftManagerStateActionEffectResultApplied;
        }

        if (effect.intent == kPbftManagerStateActionIntentCertVoteCurrentSoftValue) {
          const auto soft_voted_block = admitStateActionPbftBlock(effect, "Certify cert-vote current soft value");
          if (soft_voted_block == nullptr) {
            return kPbftManagerStateActionEffectResultSkippedMissingLiveObject;
          }

          // generate cert vote
          if (!placeStateActionVote(PbftVoteTypes::cert_vote, soft_voted_block->getPeriod(), round, step,
                                    soft_voted_block->getBlockHash(), soft_voted_block, "Certify cert vote")) {
            return kPbftManagerStateActionEffectResultRejectedLiveCheck;
          }

          if (!pbft_service_) {
            throw std::runtime_error(
                "PBFT manager Rust runtime must be initialized before persisting cert-voted block");
          }
          const auto cert_voted_snapshot = rustaxa::pbft_manager_runtime_save_cert_voted_block_in_round(
              pbft_service_->service(), soft_voted_block->getPeriod(), round,
              toBridgeHash(soft_voted_block->getBlockHash()), toBridgeBytes(soft_voted_block->rlp(true)));
          if (cert_voted_snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
            throw std::runtime_error("Rust PBFT manager cert-voted metadata update rejected: " +
                                     static_cast<std::string>(cert_voted_snapshot.error_code));
          }
          cert_voted_block_for_round_ = soft_voted_block;
          return kPbftManagerStateActionEffectResultApplied;
        }

        LOG(log_er_) << "Unsupported Rust PBFT certify effect " << static_cast<uint32_t>(effect.intent);
        assert(false);
        return kPbftManagerStateActionEffectResultExecutorError;
      },
      log_er_);
  go_finish_state_ = session_step.go_finish_state;
}

void PbftManager::firstFinish_() {
  // Even number steps from 4 are in first finish
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  const auto state = fromPbftManagerRuntimeState(action_snapshot.state);
  const auto round = static_cast<PbftRound>(action_snapshot.round);
  const auto step = static_cast<PbftStep>(action_snapshot.step);
  const auto current_round_lambda = std::chrono::milliseconds(action_snapshot.current_round_lambda_ms);
  const auto period = getPbftPeriod();
  LOG(log_dg_) << "PBFT first finishing state in period " << period << ", round " << round << ", step " << step;

  const auto fact = makePbftManagerStateActionFact(
      state, period, round, step, 0ms, getPbftDeadline(), current_round_lambda, *vote_mgr_,
      action_snapshot.has_cert_voted_block, fromBridgeHash(action_snapshot.cert_voted_block_hash),
      action_snapshot.already_next_voted_value, action_snapshot.already_next_voted_null);
  executeStateActionEffectSession(
      pbft_service_->service(), fact,
      [&](const auto &effect) {
        if (effect.intent == kPbftManagerStateActionIntentNextVoteCertVotedBlock) {
          if (!cert_voted_block_for_round_.has_value()) {
            if (!action_snapshot.has_cert_voted_block) {
              throw std::runtime_error(
                  "Rust PBFT first-finish requested cert-voted next vote without runtime metadata");
            }

            // Rust owns the cert-voted sidecar metadata and persisted payload. The C++ pointer is only a temporary
            // materialization cache for the legacy vote-generation executor boundary.
            const auto cert_voted_payload =
                rustaxa::pbft_manager_runtime_cert_voted_block_in_round(pbft_service_->service());
            if (cert_voted_payload.empty()) {
              throw std::runtime_error("Rust PBFT first-finish requested cert-voted next vote without runtime payload");
            }

            const auto payload_bytes = dev::bytes(cert_voted_payload.begin(), cert_voted_payload.end());
            const auto payload_rlp = dev::RLP(payload_bytes);
            if (payload_rlp.itemCount() != 2) {
              throw std::runtime_error("Rust PBFT cert-voted payload has invalid shape");
            }
            const auto cert_voted_round = payload_rlp[0].toInt<PbftRound>();
            if (cert_voted_round != round) {
              throw std::runtime_error("Rust PBFT cert-voted payload round does not match first-finish round");
            }
            const auto cert_voted_block = std::make_shared<PbftBlock>(payload_rlp[1]);
            if (cert_voted_block->getPeriod() != period) {
              throw std::runtime_error("Rust PBFT cert-voted payload period does not match first-finish period");
            }
            if (cert_voted_block->getBlockHash() != fromBridgeHash(action_snapshot.cert_voted_block_hash)) {
              throw std::runtime_error("Rust PBFT cert-voted payload hash does not match runtime metadata");
            }
            if (publishProposedBlock(cert_voted_block)) {
              LOG(log_nf_) << "Materialized Rust cert-voted block " << cert_voted_block->getBlockHash()
                           << " for first-finish next vote in period " << period << ", round " << round;
            }
            cert_voted_block_for_round_ = cert_voted_block;
          }
          const auto &cert_voted_block = *cert_voted_block_for_round_;

          // It should never happen that node moved to the next period without cert_voted_block_for_round_ reset
          assert(cert_voted_block->getPeriod() == period);
          assert(cert_voted_block->getBlockHash() == fromBridgeHash(action_snapshot.cert_voted_block_hash));

          return placeStateActionVote(PbftVoteTypes::next_vote, cert_voted_block->getPeriod(), round, step,
                                      cert_voted_block->getBlockHash(), cert_voted_block,
                                      "First finish cert-voted next vote")
                     ? kPbftManagerStateActionEffectResultApplied
                     : kPbftManagerStateActionEffectResultRejectedLiveCheck;
        }

        if (effect.intent == kPbftManagerStateActionIntentNextVoteNullBlock) {
          // Starting value in round 1 is always null block hash... So combined with other condition for next
          // voting null block hash...
          return placeStateActionVote(PbftVoteTypes::next_vote, period, round, step, kNullBlockHash, nullptr,
                                      "First finish null next vote")
                     ? kPbftManagerStateActionEffectResultApplied
                     : kPbftManagerStateActionEffectResultRejectedLiveCheck;
        }

        if (effect.intent == kPbftManagerStateActionIntentNextVotePreviousRoundValue) {
          // Rust selects the previous-round next-vote value from compact vote facts. The shim only materializes the
          // selected block for the temporary vote-generation executor boundary.
          const auto starting_value_hash = fromBridgeHash(effect.hash);
          auto block = admitStateActionPbftBlock(effect, "First finish next-vote previous round value");
          if (!block) {
            return kPbftManagerStateActionEffectResultSkippedMissingLiveObject;
          }

          return placeStateActionVote(PbftVoteTypes::next_vote, period, round, step, starting_value_hash,
                                      std::move(block), "First finish previous-round next vote")
                     ? kPbftManagerStateActionEffectResultApplied
                     : kPbftManagerStateActionEffectResultRejectedLiveCheck;
        }

        LOG(log_er_) << "Unsupported Rust PBFT first-finish effect " << static_cast<uint32_t>(effect.intent);
        assert(false);
        return kPbftManagerStateActionEffectResultExecutorError;
      },
      log_er_);
}

void PbftManager::secondFinish_() {
  // Odd number steps from 5 are in second finish
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  const auto state = fromPbftManagerRuntimeState(action_snapshot.state);
  const auto round = static_cast<PbftRound>(action_snapshot.round);
  const auto step = static_cast<PbftStep>(action_snapshot.step);
  const auto current_round_lambda = std::chrono::milliseconds(action_snapshot.current_round_lambda_ms);
  const auto period = getPbftPeriod();

  if (printSecondFinishStepInfo_) {
    LOG(log_dg_) << "PBFT second finishing state in period " << period << ", round " << round << ", step " << step;
    printSecondFinishStepInfo_ = false;
  }

  const auto fact = makePbftManagerStateActionFact(
      state, period, round, step, elapsedTimeInMs(second_finish_step_start_datetime_), getPbftDeadline(),
      current_round_lambda, *vote_mgr_, action_snapshot.has_cert_voted_block,
      fromBridgeHash(action_snapshot.cert_voted_block_hash), action_snapshot.already_next_voted_value,
      action_snapshot.already_next_voted_null);
  const auto session_step = executeStateActionEffectSession(
      pbft_service_->service(), fact,
      [&](const auto &effect) {
        if (effect.intent == kPbftManagerStateActionIntentNextVoteCurrentSoftValue) {
          const auto soft_voted_block_hash = fromBridgeHash(effect.hash);
          const auto soft_voted_block = admitStateActionPbftBlock(effect, "Second finish next-vote current soft value");
          if (soft_voted_block != nullptr) {
            return placeStateActionVote(PbftVoteTypes::next_vote, period, round, step, soft_voted_block_hash,
                                        soft_voted_block, "Second finish soft-value next vote",
                                        PbftMgrStatus::NextVotedSoftValue)
                       ? kPbftManagerStateActionEffectResultApplied
                       : kPbftManagerStateActionEffectResultRejectedLiveCheck;
          }
          return kPbftManagerStateActionEffectResultSkippedMissingLiveObject;
        }

        if (effect.intent == kPbftManagerStateActionIntentNextVoteNullBlock) {
          return placeStateActionVote(PbftVoteTypes::next_vote, period, round, step, kNullBlockHash, nullptr,
                                      "Second finish null next vote", PbftMgrStatus::NextVotedNullBlockHash)
                     ? kPbftManagerStateActionEffectResultApplied
                     : kPbftManagerStateActionEffectResultRejectedLiveCheck;
        }

        LOG(log_er_) << "Unsupported Rust PBFT second-finish effect " << static_cast<uint32_t>(effect.intent);
        assert(false);
        return kPbftManagerStateActionEffectResultExecutorError;
      },
      log_er_);

  loop_back_finish_state_ = session_step.loop_back_finish_state;
}

std::optional<PbftManager::ProposedBlockData> PbftManager::generatePbftBlock(
    PbftPeriod propose_period, const blk_hash_t &prev_blk_hash, const blk_hash_t &anchor_hash,
    const blk_hash_t &order_hash, const blk_hash_t &final_chain_hash,
    const std::optional<PbftBlockExtraData> &extra_data, const std::vector<WalletConfig> &eligible_wallets) {
  // Reward votes should only include those reward votes with the same round as the round last pbft block was pushed
  // into chain
  auto reward_vote_payload = vote_mgr_->proposalRewardVotesForPeriod(propose_period);
  if (!reward_vote_payload.valid) {
    LOG(log_er_) << "Unable to collect proposal reward votes for period " << propose_period << ": "
                 << reward_vote_payload.validation_error;
    assert(false);
    return {};
  }

  try {
    std::vector<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> local_candidates;

    for (const auto &wallet : eligible_wallets) {
      auto block = std::make_shared<PbftBlock>(prev_blk_hash, anchor_hash, order_hash, final_chain_hash, propose_period,
                                               wallet.node_addr, wallet.node_secret,
                                               reward_vote_payload.reward_vote_hashes, extra_data);

      const auto propose_round = getPbftRound();
      const auto propose_step = getPbftStep();
      auto propose_vote_generation = vote_mgr_->generateUniqueProposalVoteForBlock(
          block->getBlockHash(), propose_period, propose_round, propose_step, wallet);
      if (!propose_vote_generation.generated) {
        LOG(log_er_) << propose_vote_generation.error << " when generating pbft block";
        continue;
      }

      local_candidates.emplace_back(std::move(block), std::move(propose_vote_generation.vote));
    }

    // Select leader block
    auto leader_block_data = vote_mgr_->identifyLeaderBlock(
        std::move(local_candidates),
        [this](const auto &proposed_block_hash) { return pbft_chain_->findPbftBlockInChain(proposed_block_hash); },
        [this](const auto &proposed_block) { return validatePbftBlock(proposed_block); });
    if (!leader_block_data.has_value()) {
      return {};
    }

    if (!vote_mgr_->addLocallyGeneratedVote(leader_block_data->second)) {
      LOG(log_er_) << "Unable to save propose vote " << leader_block_data->second->getHash() << " for block "
                   << leader_block_data->second->getBlockHash() << ", period " << propose_period << ", round "
                   << leader_block_data->second->getRound() << ", step " << leader_block_data->second->getStep()
                   << ", validator " << leader_block_data->second->getVoterAddr();
      return {};
    }

    publishProposedBlock(leader_block_data->first);

    return PbftManager::ProposedBlockData{std::move(leader_block_data->first),
                                          std::move(reward_vote_payload.reward_votes),
                                          std::move(leader_block_data->second)};
  } catch (const std::exception &e) {
    LOG(log_er_) << "Block for period " << propose_period << " could not be proposed " << e.what();
    return {};
  }
}

void PbftManager::processProposedBlock(const std::shared_ptr<PbftBlock> &proposed_block) {
  const auto existing = pbft_service_->service().pbft_service_proposed_blocks_get(
      proposed_block->getPeriod(), toBridgeHash(proposed_block->getBlockHash()));
  if (existing.found) {
    return;
  }
  (void)publishProposedBlock(proposed_block);
}

blk_hash_t PbftManager::calculateOrderHash(const std::vector<blk_hash_t> &dag_block_hashes) {
  if (dag_block_hashes.empty()) {
    return kNullBlockHash;
  }
  dev::RLPStream order_stream(1);
  order_stream.appendList(dag_block_hashes.size());
  for (auto const &blk_hash : dag_block_hashes) {
    order_stream << blk_hash;
  }
  return dev::sha3(order_stream.out());
}

struct ProposedBlockData {
  std::shared_ptr<PbftBlock> pbft_block;
  std::vector<std::shared_ptr<PbftVote>> reward_votes;
  WalletConfig proposer_wallet;
};

std::optional<PbftManager::ProposedBlockData> PbftManager::proposePbftBlock() {
  // generates propose vote with the same block
  const auto [current_pbft_round, current_pbft_period] = getPbftRoundAndPeriod();

  const auto wallets = eligible_wallets_.getWallets(current_pbft_period);
  auto proposal_wallets = vote_mgr_->proposalWalletFacts(current_pbft_period, current_pbft_round, wallets);

  auto last_pbft_block_hash = pbft_chain_->getLastPbftBlockHash();
  auto last_period_dag_anchor_block_hash = pbft_chain_->getLastNonNullPbftBlockAnchor();
  if (last_period_dag_anchor_block_hash == kNullBlockHash) {
    last_period_dag_anchor_block_hash = dag_genesis_block_hash_;
  }

  // Creates pbft block's extra data
  std::optional<PbftBlockExtraData> extra_data;
  if (kGenesisConfig.state.hardforks.ficus_hf.isFicusHardfork(current_pbft_period)) {
    extra_data = createPbftBlockExtraData(current_pbft_period);
    if (!extra_data.has_value()) {
      LOG(log_er_) << "Unable to propose block for period " << current_pbft_period << ", round " << current_pbft_round
                   << ". Empty extra data";
      return {};
    }
  }

  auto ghost = dag_mgr_->getGhostPath(last_period_dag_anchor_block_hash);
  LOG(log_dg_) << "GHOST size " << ghost.size();

  std::optional<blk_hash_t> non_finalized_fallback_hash;
  auto non_finalized_dag_blocks = dag_mgr_->getNonFinalizedBlocks();
  if (non_finalized_dag_blocks.second.size() > 0) {
    non_finalized_fallback_hash = *non_finalized_dag_blocks.second.rbegin()->second.begin();
  }

  const auto [dag_gas_limit, pbft_gas_limit] = kGenesisConfig.getGasLimits(current_pbft_period);
  (void)dag_gas_limit;

  rustaxa::PbftManagerProposalInitialFact fact;
  fact.period = current_pbft_period;
  fact.round = current_pbft_round;
  fact.previous_pbft_block_hash = toBridgeHash(last_pbft_block_hash);
  fact.last_period_dag_anchor_hash = toBridgeHash(last_period_dag_anchor_block_hash);
  fact.dag_genesis_hash = toBridgeHash(dag_genesis_block_hash_);
  fact.dag_blocks_size = kGenesisConfig.pbft.dag_blocks_size;
  fact.ghost_path_move_back = kGenesisConfig.pbft.ghost_path_move_back;
  fact.pbft_gas_limit = pbft_gas_limit;
  fact.extra_data_required = kGenesisConfig.state.hardforks.ficus_hf.isFicusHardfork(current_pbft_period);
  fact.extra_data_available = !fact.extra_data_required || extra_data.has_value();
  fact.wallets = std::move(proposal_wallets.wallet_facts);
  fact.ghost_path = toBridgeHashes(ghost);
  fact.has_non_finalized_fallback = non_finalized_fallback_hash.has_value();
  fact.non_finalized_fallback_hash = toBridgeHash(non_finalized_fallback_hash.value_or(kNullBlockHash));

  pbft_service_->service().pbft_service_begin_proposal_session_with_final_chain(final_chain_->rustFinalChain(), fact);
  const auto step = rustaxa::pbft_manager_proposal_session_next_with_dag(pbft_service_->service(),
                                                                         dag_transaction_service_->service());

  if (step.action == kPbftManagerProposalActionBuildProposal && step.status == kPbftManagerProposalStatusBuildReady) {
    std::vector<WalletConfig> eligible_wallets;
    eligible_wallets.reserve(step.eligible_wallet_indices.size());
    for (const auto selected_wallet_index : step.eligible_wallet_indices) {
      if (selected_wallet_index >= proposal_wallets.local_wallets.size()) {
        LOG(log_er_) << "Rust PBFT proposal selected wallet index " << selected_wallet_index
                     << " outside local wallet count " << proposal_wallets.local_wallets.size();
        assert(false);
        return {};
      }
      eligible_wallets.push_back(proposal_wallets.local_wallets[selected_wallet_index]);
    }

    const auto dag_block_hash = fromBridgeHash(step.anchor_hash);
    const auto order_hash = fromBridgeHash(step.order_hash);
    if (auto proposed_block_data =
            generatePbftBlock(current_pbft_period, fromBridgeHash(step.previous_pbft_block_hash), dag_block_hash,
                              order_hash, fromBridgeHash(step.final_chain_hash), extra_data, eligible_wallets);
        proposed_block_data.has_value()) {
      LOG(log_nf_) << "Created PBFT block: " << proposed_block_data->pbft_block->getBlockHash()
                   << ", order hash:" << order_hash << ", DAG blocks included " << step.dag_blocks_included
                   << ", Rust proposal status " << static_cast<uint32_t>(step.status);
      return proposed_block_data;
    }
    return {};
  }

  if (step.action == kPbftManagerProposalActionSkipProposal) {
    LOG(log_dg_) << "Rust PBFT proposal skipped period " << current_pbft_period << ", round " << current_pbft_round
                 << ", status " << static_cast<uint32_t>(step.status) << ", error "
                 << static_cast<std::string>(step.error_code);
    return {};
  }

  LOG(log_er_) << "Rust PBFT proposal session failed period " << current_pbft_period << ", round " << current_pbft_round
               << ", action " << static_cast<uint32_t>(step.action) << ", status " << static_cast<uint32_t>(step.status)
               << ", error " << static_cast<std::string>(step.error_code);
  assert(step.action != kPbftManagerProposalActionContractError);

  return {};
}

std::optional<PbftBlockExtraData> PbftManager::createPbftBlockExtraData(PbftPeriod pbft_period) const {
  std::optional<blk_hash_t> pillar_block_hash;
  if (kGenesisConfig.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(pbft_period)) {
    // Anchor pillar block hash into the pbft block
    rustaxa::PillarCurrentAnchorDecisionRequest request{};
    request.operation = kPillarAnchorDecisionSelectPreviousPeriod;
    request.pbft_period = pbft_period;
    const auto pillar_anchor =
        requireReadyPillarService(pbft_service_).pbft_service_pillar_plan_current_anchor_decision(request);
    if (!pillar_anchor.selected || !pillar_anchor.has_current_anchor) {
      return {};
    }

    pillar_block_hash = fromBridgeHash(pillar_anchor.current_hash);
  }

  return PbftBlockExtraData{TARAXA_MAJOR_VERSION, TARAXA_MINOR_VERSION, TARAXA_PATCH_VERSION, TARAXA_NET_VERSION, "T",
                            pillar_block_hash};
}

bool PbftManager::validatePbftBlock(const std::shared_ptr<PbftBlock> &pbft_block) const {
  if (!pbft_block) {
    LOG(log_er_) << "Unable to validate pbft block - no block provided";
    return false;
  }

  auto const &pbft_block_hash = pbft_block->getBlockHash();
  const auto block_period = pbft_block->getPeriod();
  auto const &anchor_hash = pbft_block->getPivotDagBlockHash();
  const auto extra_data = pbft_block->getExtraData();
  const auto pillar_hash = extra_data ? extra_data->getPillarBlockHash() : std::nullopt;
  rustaxa::PbftManagerBlockValidationFact fact;
  fact.block_hash = toBridgeHash(pbft_block_hash);
  fact.period = block_period;
  fact.previous_pbft_block_hash = toBridgeHash(pbft_block->getPrevBlockHash());
  fact.candidate_final_chain_hash = toBridgeHash(pbft_block->getFinalChainHash());
  fact.expected_order_hash = toBridgeHash(pbft_block->getOrderHash());
  fact.pbft_gas_limit = kGenesisConfig.getGasLimits(block_period).second;
  fact.reward_vote_hashes = toBridgeHashes(pbft_block->getRewardVotes());
  fact.has_pillar_block_hash = pillar_hash.has_value();
  fact.pillar_block_hash = pillar_hash ? toBridgeHash(*pillar_hash) : std::array<uint8_t, 32>{};
  fact.pivot_hash = toBridgeHash(anchor_hash);
  fact.extra_data_required = kGenesisConfig.state.hardforks.ficus_hf.isFicusHardfork(block_period);
  fact.extra_data_present = extra_data.has_value();
  fact.extra_data_pillar_hash_present = pillar_hash.has_value();
  fact.pillar_block_required = kGenesisConfig.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(block_period);
  const auto plan = rustaxa::plan_pbft_manager_block_validation(pbft_service_->service(),
                                                               final_chain_->rustFinalChain(),
                                                               dag_transaction_service_->service(), fact);

  if (plan.action == kPbftManagerBlockValidationActionAccept) {
    return true;
  }
  if (plan.action == kPbftManagerBlockValidationActionReject ||
      plan.action == kPbftManagerBlockValidationActionWaitForFinalization) {
    return false;
  }
  if (plan.action == kPbftManagerBlockValidationActionContractError) {
    throw std::runtime_error("Rust PBFT block validation planner rejected bridge facts: " +
                             std::string(plan.error_code));
  }
  throw std::runtime_error("Rust PBFT block validation planner returned unknown action");
}

bool PbftManager::pushCertVotedPbftBlockIntoChain_(const std::shared_ptr<PbftBlock> &pbft_block,
                                                   std::vector<std::shared_ptr<PbftVote>> &&current_round_cert_votes) {
  PeriodData period_data;
  period_data.pbft_blk = pbft_block;
  if (pbft_block->getPivotDagBlockHash() != kNullBlockHash) {
    const auto payload = rustaxa::pbft_manager_runtime_cached_candidate_dag_payload(
        pbft_service_->service(), dag_transaction_service_->service(),
        toBridgeHash(pbft_block->getPivotDagBlockHash()));
    materializeCachedCandidateDag(payload, period_data);
  }

  auto reward_votes = vote_mgr_->collectRewardVotesForBlock(period_data.pbft_blk);
  if (!reward_votes.has_value()) {
    LOG(log_er_) << "Missing reward votes in cert voted block " << pbft_block->getBlockHash();
    return false;
  }
  period_data.previous_block_cert_votes = std::move(*reward_votes);

  if (!pushPbftBlock_(std::move(period_data), std::move(current_round_cert_votes))) {
    LOG(log_er_) << "Failed push cert voted block " << pbft_block->getBlockHash() << " into PBFT chain";
    return false;
  }

  return true;
}

void PbftManager::pushSyncedPbftBlocksIntoChain() {
  auto net = network_.lock();
  if (!net) {
    LOG(log_er_) << "Failed to obtain net !";
    return;
  }

  rustaxa::pbft_manager_runtime_begin_pbft_sync_queue_drain(pbft_service_->service());
  std::optional<std::pair<PeriodData, std::vector<std::shared_ptr<PbftVote>>>> accepted_period_data;

  auto report_step = [&](const rustaxa::PbftSyncQueueDrainStep &step, bool success, bool accepted) {
    rustaxa::PbftSyncQueueDrainReport report;
    report.action = step.action;
    report.success = success;
    report.accepted_period_data = accepted;
    const auto result = rustaxa::pbft_manager_runtime_pbft_sync_queue_drain_report(pbft_service_->service(), report);
    if (!result.can_continue && result.status != kPbftSyncQueueDrainStatusComplete) {
      LOG(log_er_) << "Rust PBFT sync queue drain stopped after action " << static_cast<uint32_t>(step.action)
                   << ", status " << static_cast<uint32_t>(result.status) << ", error "
                   << static_cast<std::string>(result.error_code);
    }
    return result.can_continue;
  };

  while (true) {
    const auto step = rustaxa::pbft_manager_runtime_pbft_sync_queue_drain_next(pbft_service_->service());

    if (step.action == kPbftSyncQueueDrainActionStop) {
      break;
    }
    if (step.status != kPbftSyncQueueDrainStatusActive) {
      LOG(log_er_) << "Rust PBFT sync queue drain returned non-active step, action "
                   << static_cast<uint32_t>(step.action) << ", status " << static_cast<uint32_t>(step.status)
                   << ", error " << static_cast<std::string>(step.error_code);
      break;
    }

    if (step.action == kPbftSyncQueueDrainActionPopAndProcess) {
      accepted_period_data.reset();
      bool success = true;
      try {
        accepted_period_data = processPeriodData();
      } catch (const std::exception &e) {
        success = false;
        LOG(log_er_) << "Rust PBFT sync queue drain process executor failed: " << e.what();
      } catch (...) {
        success = false;
        LOG(log_er_) << "Rust PBFT sync queue drain process executor failed with unknown exception";
      }
      if (!report_step(step, success, accepted_period_data.has_value())) {
        break;
      }
      continue;
    }

    if (step.action == kPbftSyncQueueDrainActionPushAccepted) {
      if (!accepted_period_data) {
        LOG(log_er_) << "Rust PBFT sync queue drain requested push without accepted period data";
        report_step(step, false, false);
        break;
      }

      const auto pbft_block_period = accepted_period_data->first.pbft_blk->getPeriod();
      const auto pbft_block_hash = accepted_period_data->first.pbft_blk->getBlockHash();
      LOG(log_nf_) << "Picked sync block " << pbft_block_hash << " with period " << pbft_block_period;

      const auto pushed =
          pushPbftBlock_(std::move(accepted_period_data->first), std::move(accepted_period_data->second));
      if (pushed) {
        LOG(log_dg_) << "Pushed synced PBFT block " << pbft_block_hash << " with period " << pbft_block_period;
      } else {
        LOG(log_er_) << "Failed push PBFT block " << pbft_block_hash << " with period " << pbft_block_period;
      }
      accepted_period_data.reset();
      if (!report_step(step, pushed, false)) {
        break;
      }
      continue;
    }

    if (step.action == kPbftSyncQueueDrainActionUpdateSyncState) {
      net->setSyncStatePeriod(pbftSyncingPeriod());
      if (!report_step(step, true, false)) {
        break;
      }
      continue;
    }

    LOG(log_er_) << "Rust PBFT sync queue drain returned unsupported action " << static_cast<uint32_t>(step.action)
                 << ", error " << static_cast<std::string>(step.error_code);
    break;
  }
}

void PbftManager::reorderTransactions(SharedTransactions &transactions) {
  // DAG reordering can cause transactions from same sender to be reordered by nonce. If this is the case only
  // transactions from these accounts are sorted and reordered, all other transactions keep the order
  SharedTransactions ordered_transactions;

  // Account with reverse order nonce, the value in a map is a position of last instance
  // of transaction with this account
  std::unordered_map<addr_t, uint32_t> account_reverse_order;

  // While iterating over transactions, account_nonce will keep the last nonce for the account
  std::unordered_map<addr_t, val_t> account_nonce;

  // Find accounts that need reordering and place in account_reverse_order set
  for (uint32_t i = 0; i < transactions.size(); i++) {
    const auto &t = transactions[i];
    auto ro_it = account_reverse_order.find(t->getSender());
    if (ro_it == account_reverse_order.end()) {
      auto it = account_nonce.find(t->getSender());
      if (it == account_nonce.end() || it->second < t->getNonce()) {
        account_nonce[t->getSender()] = t->getNonce();
      } else if (it->second > t->getNonce()) {
        // Nonce of the transaction is smaller than previous nonce, this account transactions will need reordering
        account_reverse_order.insert({t->getSender(), i});
      }
    } else {
      ro_it->second = i;
    }
  }

  // If account_reverse_order size is 0, there is no need to reorder transactions
  if (account_reverse_order.size() > 0) {
    std::unordered_map<addr_t, std::multimap<val_t, std::shared_ptr<Transaction>>> account_nonce_transactions;
    // Keep the order for all transactions that do not need reordering
    for (uint32_t i = 0; i < transactions.size(); i++) {
      const auto &t = transactions[i];
      auto ro_it = account_reverse_order.find(t->getSender());
      if (ro_it != account_reverse_order.end()) {
        account_nonce_transactions[t->getSender()].insert({t->getNonce(), t});
        if (ro_it->second == i) {
          // This is the last instance of transaction for this account, place all the reordered transactions for this
          // account at this position
          for (const auto &nonce : account_nonce_transactions[t->getSender()]) {
            ordered_transactions.push_back(nonce.second);
          }
        }
      } else {
        ordered_transactions.push_back(t);
      }
    }
    transactions = ordered_transactions;
  }
}

uint64_t PbftManager::finalize_(PeriodData &&period_data, std::vector<h256> &&finalized_dag_blk_hashes,
                                uint32_t blocks_per_year, bool synchronous_processing) {
  std::shared_ptr<DagBlock> anchor_block = nullptr;

  if (const auto anchor = period_data.pbft_blk->getPivotDagBlockHash()) {
    anchor_block = dag_mgr_->getDagBlock(anchor);
    if (!anchor_block) {
      LOG(log_er_) << "DB corrupted - Cannot find anchor block: " << anchor << " in DB.";
      assert(false);
    }
  }

  auto result = final_chain_->finalize(std::move(period_data), std::move(finalized_dag_blk_hashes), blocks_per_year,
                                       std::move(anchor_block));

  if (synchronous_processing) {
    result.wait();
  }

  return rustFinalChainLastBlockNumber(final_chain_);
}

bool PbftManager::pushPbftBlock_(PeriodData &&period_data, std::vector<std::shared_ptr<PbftVote>> &&cert_votes) {
  auto const &pbft_block_hash = period_data.pbft_blk->getBlockHash();
  if (!pbft_service_) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before reading PBFT block existence");
  }
  const auto push_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  const auto block_in_chain = pbft_service_->service().pbft_chain_block_exists(toBridgeHash(pbft_block_hash));
  if (block_in_chain && cert_votes.empty()) {
    LOG(log_nf_) << "PBFT block: " << pbft_block_hash << " in DB already.";
    LOG(log_dg_) << "Rust PBFT finalization resume classifier cannot inspect duplicate block " << pbft_block_hash
                 << " because certified-vote facts are unavailable.";
    if (push_snapshot.has_cert_voted_block && fromBridgeHash(push_snapshot.cert_voted_block_hash) == pbft_block_hash) {
      LOG(log_er_) << "Last cert voted value should be kNullBlockHash. Block hash " << pbft_block_hash
                   << " has been pushed into chain already";
      assert(false);
    }
    return false;
  }

  assert(cert_votes.empty() == false);
  const auto sample_cert_vote = cert_votes[0];
  assert(pbft_block_hash == sample_cert_vote->getBlockHash());

  const auto block_pbft_period = period_data.pbft_blk->getPeriod();
  const auto block_pbft_round = sample_cert_vote->getRound();
  const auto dynamic_lambda_enabled = kGenesisConfig.state.hardforks.isOnCactiHardfork(block_pbft_period);
  const auto dynamic_lambda_runtime_snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  const auto dynamic_lambda_plan = rustaxa::pbft_manager_runtime_plan_finalization_dynamic_lambda(
      pbft_service_->service(),
      makePbftDynamicLambdaFact(kGenesisConfig.state.hardforks, kGenesisConfig.state.dpos.blocks_per_year,
                                dynamic_lambda_enabled, block_pbft_period, block_pbft_round,
                                dynamic_lambda_runtime_snapshot.rounds_count_dynamic_lambda,
                                dynamic_lambda_runtime_snapshot.dynamic_lambda_ms));
  if (dynamic_lambda_plan.status != kPbftFinalizationStatusAccepted) {
    LOG(log_er_) << "Rust PBFT dynamic-lambda planner rejected block " << pbft_block_hash << ", period "
                 << block_pbft_period << ", round " << block_pbft_round << ", error "
                 << static_cast<std::string>(dynamic_lambda_plan.error_code);
    return false;
  }
  const uint32_t block_lambda = dynamic_lambda_plan.period_lambda;
  const uint32_t dynamic_blocks_per_year = dynamic_lambda_enabled ? dynamic_lambda_plan.blocks_per_year : 0;
  bool pillar_block_finalized = false;
  std::optional<pillar_chain::PillarChainManager::FinalizePillarBlockPreflightResult> pillar_preflight;
  const auto pillar_block_hash = period_data.pbft_blk->getExtraData()
                                     ? period_data.pbft_blk->getExtraData()->getPillarBlockHash()
                                     : std::optional<blk_hash_t>();
  const auto pillar_finalization_required =
      kGenesisConfig.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(block_pbft_period);
  if (!block_in_chain && pillar_finalization_required) {
    if (!pillar_block_hash.has_value()) {
      LOG(log_er_) << "PBFT block " << pbft_block_hash << ", period " << block_pbft_period
                   << " requires pillar finalization but has no pillar block hash";
      return false;
    }
    auto pillar_finalization = pillar_chain_mgr_->finalizePillarBlockForPbftPreflight(*pillar_block_hash);
    if (!pillar_finalization.success) {
      LOG(log_er_) << "PBFT block " << pbft_block_hash << ", period " << block_pbft_period
                   << " could not finalize pillar block " << *pillar_block_hash;
      return false;
    }
    pillar_preflight = std::move(pillar_finalization);
    // Retain the shared-pointer payload for compatibility emission after the
    // primary batch commits and the protected DAG/transaction locks release.
    period_data.pillar_votes_ = pillar_preflight->pillar_votes;
    pillar_block_finalized = true;
  }

  LOG(log_dg_) << "Storing pbft blk " << pbft_block_hash << " cert votes: " << cert_votes;

  vec_blk_t dag_blocks_order;
  dag_blocks_order.reserve(period_data.dag_blocks.size());
  std::transform(period_data.dag_blocks.begin(), period_data.dag_blocks.end(), std::back_inserter(dag_blocks_order),
                 [](const auto &dag_block) { return dag_block->getHash(); });

  // We need to reorder transactions before saving them
  reorderTransactions(period_data.transactions);

  std::vector<trx_hash_t> transaction_order;
  transaction_order.reserve(period_data.transactions.size());
  for (const auto &transaction : period_data.transactions) {
    transaction_order.emplace_back(transaction->getHash());
  }

  const auto planner_pillar_block_finalized = block_in_chain ? true : pillar_block_finalized;
  const auto finalization_intent_fact = makePbftFinalizationIntentFact(
      period_data, block_in_chain, planner_pillar_block_finalized, dynamic_lambda_enabled, cert_votes.size(),
      sample_cert_vote->getBlockHash(), sample_cert_vote->getPeriod(), sample_cert_vote->getRound(),
      sample_cert_vote->getStep(), block_lambda, dynamic_lambda_plan.last_saved_period_lambda_found,
      dynamic_lambda_plan.last_saved_period_lambda, dynamic_blocks_per_year,
      dynamic_lambda_plan.rounds_count_dynamic_lambda, dynamic_lambda_plan.dynamic_lambda,
      kGenesisConfig.state.dpos.blocks_per_year, dag_blocks_order, transaction_order,
      kGenesisConfig.state.hardforks.ficus_hf.isPillarBlockPeriod(block_pbft_period));
  const auto finalization_plan =
      rustaxa::pbft_manager_runtime_plan_finalization_intent(pbft_service_->service(), finalization_intent_fact);
  if (!finalization_plan.finalize_block || finalization_plan.status != kPbftFinalizationStatusAccepted) {
    LOG(log_er_) << "Rust PBFT finalization planner rejected block " << pbft_block_hash << ", period "
                 << block_pbft_period << ", round " << block_pbft_round << ", status "
                 << static_cast<uint32_t>(finalization_plan.status);
    return false;
  }
  auto apply_boundary_snapshot = [&](const rustaxa::PbftManagerFinalizationExecutorState &boundary) {
    if (boundary.has_snapshot) {
      applyPbftManagerRuntimeSnapshot(
          boundary.snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
          rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_, already_next_voted_value_,
          already_next_voted_null_block_hash_, broadcast_votes_counter_, rebroadcast_votes_counter_,
          broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_);
    }
  };
  auto fail_boundary = [&](const char *context, const rustaxa::PbftManagerFinalizationExecutorState &boundary) {
    LOG(log_er_) << "Rust PBFT finalization boundary failed for block " << pbft_block_hash << ", period "
                 << block_pbft_period << ", context " << context << ", status "
                 << static_cast<uint32_t>(boundary.status) << ", action " << static_cast<uint32_t>(boundary.action)
                 << ", error " << static_cast<std::string>(boundary.error_code);
    return false;
  };
  auto report_failure_boundary = [&](const rustaxa::PbftManagerFinalizationExecutorState &boundary_state,
                                     const char *error_code) {
    const auto boundary = rustaxa::pbft_manager_runtime_fail_finalization_external_effect(
        pbft_service_->service(), boundary_state.cursor, 255, error_code);
    apply_boundary_snapshot(boundary);
    return boundary;
  };
  auto report_finalization_action = [&](rustaxa::PbftManagerFinalizationExecutorState &boundary, const char *context,
                                        rust::Vec<rustaxa::TransactionQueueAccountNonceFact> nonce_facts,
                                        uint64_t last_block, uint64_t request_period, uint64_t retention_window,
                                        bool apply_dag_compatibility_effects = false, bool terminate_on_error = false) {
    try {
      boundary = rustaxa::pbft_manager_runtime_advance_finalization_action(
          pbft_service_->service(), dag_transaction_service_->service(), boundary.cursor, boundary.action, last_block,
          request_period, retention_window, std::move(nonce_facts));
    } catch (const std::exception &e) {
      LOG(log_er_) << "Rust PBFT finalization boundary report threw for block " << pbft_block_hash << ", period "
                   << block_pbft_period << ", context " << context << ": " << e.what();
      if (terminate_on_error) {
        std::terminate();
      }
      return false;
    }
    apply_boundary_snapshot(boundary);
    if (apply_dag_compatibility_effects) {
      vec_blk_t expired_hashes;
      expired_hashes.reserve(boundary.expired_dag_hashes.size());
      for (const auto &hash : boundary.expired_dag_hashes) {
        expired_hashes.push_back(fromBridgeHash(hash.hash));
      }
      dag_mgr_->applyFinalizationDagOrderCompatibilityEffects(expired_hashes, boundary.refresh_dag_counters);
    }
    if (!boundary.can_continue) {
      return fail_boundary(context, boundary);
    }
    return true;
  };
  auto apply_advance_period_for_finalization = [&]() {
    return applyRustPlannedAdvancePeriod_(finalization_plan.storage_write_intent.block_period);
  };
  auto process_pillar_block_for_finalization = [&]() -> std::optional<uint64_t> {
    assert(block_pbft_period == pbft_chain_->getPbftChainSize());
    const auto delegation_delay = final_chain_->delegationDelay();
    if (delegation_delay >= block_pbft_period) {
      return std::nullopt;
    }
    const auto pillar_request_period = block_pbft_period - delegation_delay;
    processPillarBlock(block_pbft_period);
    return pillar_request_period;
  };

  bool dag_order_payload_available = true;
  bool transaction_status_action_available = true;
  bool final_chain_payload_available = true;
  bool advance_period_payload_available = true;
  bool pillar_payload_available = true;

  enum class FinalizationDispatchResult { kComplete, kReleaseProtectedLocks, kFailed };
  auto dispatch_finalization_actions = [&](rustaxa::PbftManagerFinalizationExecutorState &boundary,
                                           bool protected_locks_held, bool resume_mode) -> FinalizationDispatchResult {
    auto fail_action = [&](const char *context, const char *error_code) {
      boundary = report_failure_boundary(boundary, error_code);
      fail_boundary(context, boundary);
      return FinalizationDispatchResult::kFailed;
    };

    while (boundary.has_action) {
      if (boundary.status != kPbftFinalizationRuntimeStatusActive) {
        fail_boundary("action dispatch", boundary);
        return FinalizationDispatchResult::kFailed;
      }

      switch (boundary.action) {
        case kPbftFinalizationRuntimeActionCommitSortitionRuntime: {
          if (!protected_locks_held && !resume_mode) {
            return fail_action("sortition runtime commit", "PBFT_FINALIZE_PROTECTED_ACTION_OUTSIDE_LOCKS");
          }
          if (!report_finalization_action(boundary, "sortition runtime commit", {}, 0, 0, 0, false, true)) {
            return FinalizationDispatchResult::kFailed;
          }
          break;
        }
        case kPbftFinalizationRuntimeActionCommitRewardVotesReset: {
          if (!protected_locks_held && !resume_mode) {
            return fail_action("reward-vote reset", "PBFT_FINALIZE_PROTECTED_ACTION_OUTSIDE_LOCKS");
          }
          if (!report_finalization_action(boundary, "reward-vote reset", {}, 0, 0, 0)) {
            return FinalizationDispatchResult::kFailed;
          }
          break;
        }
        case kPbftFinalizationRuntimeActionSetDagBlockOrder: {
          if (!protected_locks_held) {
            return fail_action("DAG block order", "PBFT_FINALIZE_PROTECTED_ACTION_OUTSIDE_LOCKS");
          }
          if (!dag_order_payload_available) {
            return fail_action("DAG block order", "PBFT_FINALIZE_DAG_ORDER_PAYLOAD_UNAVAILABLE");
          }
          dag_order_payload_available = false;
          if (!report_finalization_action(boundary, "DAG block order", {}, 0, 0, 0, true)) {
            return FinalizationDispatchResult::kFailed;
          }
          break;
        }
        case kPbftFinalizationRuntimeActionUpdateFinalizedTransactions: {
          if (!protected_locks_held) {
            return fail_action("transaction finalized-status update", "PBFT_FINALIZE_PROTECTED_ACTION_OUTSIDE_LOCKS");
          }
          if (!transaction_status_action_available) {
            return fail_action("transaction finalized-status update",
                               "PBFT_FINALIZE_TRANSACTION_STATUS_ALREADY_CALLED");
          }
          transaction_status_action_available = false;
          if (!report_finalization_action(boundary, "transaction finalized-status update",
                                          trx_mgr_->finalizedStatusAccountNonceFacts(), 0, 0,
                                          kRecentlyFinalizedTransactionsFactor * final_chain_->delegationDelay())) {
            return FinalizationDispatchResult::kFailed;
          }
          break;
        }
        case kPbftFinalizationRuntimeActionFinalizeFinalChain: {
          if (protected_locks_held) {
            return FinalizationDispatchResult::kReleaseProtectedLocks;
          }
          if (!final_chain_payload_available) {
            return fail_action("FinalChain dispatch", "PBFT_FINALIZE_FINAL_CHAIN_PAYLOAD_UNAVAILABLE");
          }
          const auto final_chain_last_block = rustFinalChainLastBlockNumber(final_chain_);
          if (final_chain_last_block + 1 != block_pbft_period) {
            return fail_action("FinalChain dispatch", "PBFT_FINALIZE_NON_SEQUENTIAL_FINAL_CHAIN");
          }
          final_chain_payload_available = false;
          const auto last_block = finalize_(std::move(period_data), std::move(dag_blocks_order),
                                            finalization_plan.storage_write_intent.blocks_per_year);
          if (last_block < block_pbft_period) {
            return fail_action("FinalChain dispatch", "PBFT_FINALIZE_FINAL_CHAIN_ACTION_FAILED");
          }
          if (!report_finalization_action(boundary, "FinalChain dispatch", {}, last_block, 0, 0)) {
            return FinalizationDispatchResult::kFailed;
          }
          break;
        }
        case kPbftFinalizationRuntimeActionAdvancePeriod: {
          if (protected_locks_held) {
            return FinalizationDispatchResult::kReleaseProtectedLocks;
          }
          if (!advance_period_payload_available) {
            return fail_action("advance period", "PBFT_FINALIZE_ADVANCE_PERIOD_PAYLOAD_UNAVAILABLE");
          }
          advance_period_payload_available = false;
          if (!apply_advance_period_for_finalization()) {
            return fail_action("advance period", "PBFT_FINALIZE_ADVANCE_PERIOD_ACTION_FAILED");
          }
          if (!report_finalization_action(boundary, "advance period", {}, 0, 0, 0)) {
            return FinalizationDispatchResult::kFailed;
          }
          break;
        }
        case kPbftFinalizationRuntimeActionProcessPillarBlock: {
          if (protected_locks_held) {
            return FinalizationDispatchResult::kReleaseProtectedLocks;
          }
          if (!pillar_payload_available) {
            return fail_action("pillar post-processing", "PBFT_FINALIZE_PILLAR_PAYLOAD_UNAVAILABLE");
          }
          pillar_payload_available = false;
          const auto report = process_pillar_block_for_finalization();
          if (!report.has_value()) {
            return fail_action("pillar post-processing", "PBFT_FINALIZE_PILLAR_ACTION_FAILED");
          }
          if (!report_finalization_action(boundary, "pillar post-processing", {}, 0, *report, 0)) {
            return FinalizationDispatchResult::kFailed;
          }
          break;
        }
        default:
          return fail_action(protected_locks_held ? "protected action dispatch" : "action dispatch",
                             protected_locks_held ? "PBFT_FINALIZE_UNKNOWN_PROTECTED_ACTION"
                             : resume_mode        ? "PBFT_FINALIZE_PROTECTED_ACTION_ON_RESUME"
                                                  : "PBFT_FINALIZE_UNKNOWN_UNPROTECTED_ACTION");
      }
    }

    apply_boundary_snapshot(boundary);
    if (!boundary.complete || boundary.status != kPbftFinalizationRuntimeStatusComplete) {
      fail_boundary("completion", boundary);
      return FinalizationDispatchResult::kFailed;
    }
    return FinalizationDispatchResult::kComplete;
  };
  if (block_in_chain) {
    bool resume_executed = false;
    try {
      LOG(log_nf_) << "PBFT block: " << pbft_block_hash << " in DB already.";
      rustaxa::PbftManagerFinalizationExecutorState resume_boundary{};
      try {
        rustaxa::PbftFinalizationExecutorStartRequest start_request{};
        start_request.mode = kPbftFinalizationExecutorModeResume;
        start_request.plan = finalization_plan;
        start_request.final_chain_last_block = rustFinalChainLastBlockNumber(final_chain_);
        resume_boundary = rustaxa::pbft_manager_runtime_start_finalization_executor(
            pbft_service_->service(), dag_transaction_service_->service(), start_request);
      } catch (const std::exception &e) {
        LOG(log_er_) << "Rust PBFT finalization resume boundary begin threw for block " << pbft_block_hash
                     << ", period " << block_pbft_period << ": " << e.what();
        return false;
      }
      apply_boundary_snapshot(resume_boundary);
      LOG(log_dg_) << "Rust PBFT finalization resume started for duplicate block " << pbft_block_hash << ", period "
                   << block_pbft_period << ", status " << static_cast<uint32_t>(resume_boundary.status) << ", complete "
                   << resume_boundary.complete << ", action " << static_cast<uint32_t>(resume_boundary.action)
                   << ", error " << static_cast<std::string>(resume_boundary.error_code);

      if (dispatch_finalization_actions(resume_boundary, false, true) != FinalizationDispatchResult::kComplete) {
        return false;
      }
      resume_executed = true;
    } catch (const std::exception &e) {
      LOG(log_er_) << "Rust PBFT finalization resume failed for duplicate block " << pbft_block_hash << ", period "
                   << block_pbft_period << ": " << e.what();
    }
    if (push_snapshot.has_cert_voted_block && fromBridgeHash(push_snapshot.cert_voted_block_hash) == pbft_block_hash) {
      LOG(log_er_) << "Last cert voted value should be kNullBlockHash. Block hash " << pbft_block_hash
                   << " has been pushed into chain already";
      assert(false);
    }
    return resume_executed;
  }
  if (finalization_plan.storage_write_intent.apply_dynamic_lambda_update !=
          dynamic_lambda_plan.apply_dynamic_lambda_update ||
      finalization_plan.storage_write_intent.period_lambda != dynamic_lambda_plan.period_lambda ||
      finalization_plan.storage_write_intent.blocks_per_year != dynamic_lambda_plan.blocks_per_year) {
    LOG(log_er_) << "Rust PBFT finalization dynamic-lambda facts diverged for block " << pbft_block_hash << ", period "
                 << block_pbft_period;
    return false;
  }
  rust::Vec<rustaxa::PbftFinalizationStorageWriteStage> first_persistence_stages;
  auto primary_storage_stage = makeFinalizationStorageStage(kPbftFinalizationStorageStagePrimary);
  if (pillar_preflight.has_value() && pillar_preflight->has_prepared_pillar_block) {
    primary_storage_stage.has_prepared_pillar_block = true;
    primary_storage_stage.prepared_pillar_block_period = pillar_preflight->prepared_pillar_block_period;
    primary_storage_stage.prepared_pillar_block_rlp.reserve(pillar_preflight->prepared_pillar_block_rlp.size());
    for (const auto block_data_byte : pillar_preflight->prepared_pillar_block_rlp) {
      primary_storage_stage.prepared_pillar_block_rlp.push_back(block_data_byte);
    }
  }
  first_persistence_stages.push_back(std::move(primary_storage_stage));

  rustaxa::PbftManagerFinalizationExecutorState boundary{};
  bool dispatch_complete = false;
  FinalizationDispatchResult protected_dispatch = FinalizationDispatchResult::kReleaseProtectedLocks;
  {
    // This makes sure that no DAG block or transaction can be added or change state in transaction and dag manager
    // when finalizing pbft block with dag blocks and transactions
    std::unique_lock dag_lock(dag_mgr_->getDagMutex());
    std::unique_lock trx_lock(trx_mgr_->getTransactionsMutex());

    try {
      rustaxa::PbftFinalizationExecutorStartRequest start_request{};
      start_request.mode = kPbftFinalizationExecutorModeFresh;
      start_request.plan = finalization_plan;
      start_request.primary_stages = std::move(first_persistence_stages);
      start_request.sync = false;
      boundary = rustaxa::pbft_manager_runtime_start_finalization_executor(
          pbft_service_->service(), dag_transaction_service_->service(), start_request);
    } catch (const std::exception &e) {
      LOG(log_er_) << "Rust PBFT finalization boundary begin failed for block " << pbft_block_hash << ", period "
                   << block_pbft_period << ": " << e.what();
      return false;
    }
    apply_boundary_snapshot(boundary);
    if (!boundary.can_continue) {
      return fail_boundary("primary storage", boundary);
    }
    try {
      protected_dispatch = dispatch_finalization_actions(boundary, true, false);
    } catch (const std::exception &e) {
      LOG(log_er_) << "Protected PBFT finalization action threw after primary storage for block " << pbft_block_hash
                   << ", period " << block_pbft_period << ": " << e.what();
      protected_dispatch = FinalizationDispatchResult::kFailed;
    }
    if (protected_dispatch == FinalizationDispatchResult::kComplete) {
      dispatch_complete = true;
    }
  }

  // The Rust executor has committed the primary PBFT batch at this point, and
  // the protected DAG/transaction locks are no longer held. Only now may the
  // pillar runtime publish its latest snapshot, clean votes, and emit the
  // compatibility event.
  if (pillar_preflight.has_value() && !pillar_chain_mgr_->acknowledgePillarBlockForPbft(
                                          pillar_preflight->preparation_anchor_generation,
                                          pillar_preflight->preparation_token, pillar_preflight->pillar_votes)) {
    LOG(log_er_) << "Pillar finalization acknowledge failed for PBFT block " << pbft_block_hash << ", period "
                 << block_pbft_period << ", pillar block hash " << *pillar_block_hash;
    return false;
  }

  // A protected action can fail after the primary batch has committed. Defer
  // that failure return until after pillar reconciliation so the durable row
  // cannot leave the runtime preparation pending until restart.
  if (protected_dispatch == FinalizationDispatchResult::kFailed) {
    return false;
  }

  LOG(log_nf_) << "Pushed new PBFT block " << pbft_block_hash << " into chain. Period: " << block_pbft_period
               << ", round: " << block_pbft_round;

  if (finalization_plan.storage_write_intent.apply_dynamic_lambda_update) {
    if (dynamic_lambda_plan.decreased_dynamic_lambda) {
      LOG(log_nf_) << "Decrease dynamic_lambda by " << kGenesisConfig.state.hardforks.cacti_hf.lambda_change << " to "
                   << dynamic_lambda_ << ", period " << block_pbft_period << ", round " << block_pbft_round;
    }
    if (dynamic_lambda_plan.increased_dynamic_lambda) {
      LOG(log_nf_) << "Increase dynamic_lambda by " << kGenesisConfig.state.hardforks.cacti_hf.lambda_change << " to "
                   << dynamic_lambda_ << ", period " << block_pbft_period << ", round " << block_pbft_round;
    }
  }

  if (!dispatch_complete &&
      dispatch_finalization_actions(boundary, false, false) != FinalizationDispatchResult::kComplete) {
    return false;
  }

  return true;
}

void PbftManager::processPillarBlock(PbftPeriod current_pbft_chain_size) {
  // Pillar block use state from current_pbft_chain_size - final_chain_->delegationDelay(), e.g. block with period 32
  // uses state from period 27.
  PbftPeriod request_period = current_pbft_chain_size - final_chain_->delegationDelay();
  // advancePeriod() -> resetConsensus() -> waitForPeriodFinalization() makes sure block request_period was already
  // finalized
  assert(rustFinalChainLastBlockNumber(final_chain_) >= request_period);

  const auto block_header = final_chain_->blockHeader(request_period);
  const auto bridge_root = final_chain_->getBridgeRoot(request_period);
  const auto bridge_epoch = final_chain_->getBridgeEpoch(request_period);

  // Create pillar block
  const auto pillar_block =
      pillar_chain_mgr_->createPillarBlock(current_pbft_chain_size, block_header, bridge_root, bridge_epoch);

  // Optimization - creates pillar vote right after pillar block was created, otherwise pillar votes are created during
  // next period pbft voting
  if (pillar_block) {
    for (const auto &wallet : eligible_wallets_.getWallets(current_pbft_chain_size + 1)) {
      // Wallet is not dpos eligible - do no vote
      if (!wallet.first) {
        continue;
      }

      // Pillar votes are created in the next period, this is optimization to create & broadcast it a bit faster
      const auto pillar_vote = pillar_chain_mgr_->genAndPlacePillarVote(
          current_pbft_chain_size + 1, pillar_block->getHash(), wallet.second.node_secret, periodDataQueueEmpty());
      if (pillar_vote) {
        last_placed_pillar_vote_period_ = pillar_vote->getPeriod();
      }
    }
  }
}

PbftPeriod PbftManager::pbftSyncingPeriod() const {
  const auto snapshot = rustaxa::pbft_manager_runtime_period_data_queue_snapshot(pbft_service_->service());
  return snapshot.syncing_period;
}

void PbftManager::setPbftSyncSnapshotCreationEnabled(bool enabled) {
  // RUSTAXA_PBFT_LIFECYCLE_COMPAT: snapshot toggling is an app/storage-shell lifecycle control, not a consensus
  // storage read/write route.
  if (enabled) {
    db_->enableSnapshots();
    return;
  }
  db_->disableSnapshots();
}

std::optional<std::pair<PeriodData, std::vector<std::shared_ptr<PbftVote>>>> PbftManager::processPeriodData() {
  auto popped_period_data = popPeriodDataQueueWithMetadata();
  auto period_data = std::move(popped_period_data.period_data);
  auto cert_vote_rlps = std::move(popped_period_data.cert_vote_rlps);
  const auto node_id = popped_period_data.node_id;
  const auto pbft_block_hash = popped_period_data.block_hash;
  const auto block_period = popped_period_data.period;
  const auto block_prev_hash = popped_period_data.prev_block_hash;
  const auto final_chain_hash = popped_period_data.final_chain_hash;
  auto reward_vote_hashes = std::move(popped_period_data.reward_vote_hashes);
  auto pillar_vote_rlps = std::move(popped_period_data.pillar_vote_rlps);
  auto transaction_rlps = std::move(popped_period_data.transaction_rlps);
  const auto dag_transaction_hashes = std::move(popped_period_data.dag_transaction_hashes);
  const auto period_data_transaction_hashes = std::move(popped_period_data.period_data_transaction_hashes);
  auto period_data_transaction_identities = std::move(popped_period_data.period_data_transaction_identities);
  const auto previous_cert_votes_present = popped_period_data.previous_cert_votes_present;
  const auto previous_cert_first_vote_has_weight = popped_period_data.previous_cert_first_vote_has_weight;
  const auto pillar_votes_present = popped_period_data.pillar_votes_present;
  const auto extra_data_present = popped_period_data.extra_data_present;
  const auto extra_data_pillar_block_hash_present = popped_period_data.extra_data_pillar_block_hash_present;
  const auto extra_data_required = kGenesisConfig.state.hardforks.ficus_hf.isFicusHardfork(block_period);
  const auto pillar_votes_required = kGenesisConfig.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(block_period);
  LOG(log_dg_) << "Pop pbft block " << pbft_block_hash << " with period " << block_period << " from synced queue";

  const auto last_pbft_block_hash = pbft_chain_->getLastPbftBlockHash();
  const auto last_pbft_block_period = pbft_chain_->getPbftChainSize();
  const auto block_in_chain = pbft_chain_->findPbftBlockInChain(pbft_block_hash);
  auto net = network_.lock();
  assert(net);  // Should never happen
  auto apply_rust_admission_side_effects = [&](const rustaxa::PbftSyncAdmissionSessionStep &step) {
    if (step.plan.wait_for_finalization) {
      final_chain_->waitForFinalized();
    }
    if (step.plan.report_malicious_peer) {
      net->handleMaliciousSyncPeer(node_id);
    }
  };

  rustaxa::PbftSyncAdmissionInitialFact initial_fact{};
  initial_fact.block_period = block_period;
  initial_fact.block_prev_hash = toBridgeHash(block_prev_hash);
  initial_fact.chain_last_hash = toBridgeHash(last_pbft_block_hash);
  initial_fact.chain_last_period = last_pbft_block_period;
  initial_fact.block_in_chain = block_in_chain;
  initial_fact.dag_transaction_hashes = toBridgeTransactionHashes(dag_transaction_hashes);
  initial_fact.period_data_transaction_hashes = toBridgeTransactionHashes(period_data_transaction_hashes);
  initial_fact.reward_vote_hashes = toBridgeFinalizationHashes(reward_vote_hashes);
  initial_fact.candidate_final_chain_hash = toBridgeHash(final_chain_hash);
  initial_fact.extra_data_required = extra_data_required;
  initial_fact.extra_data_present = extra_data_present;
  initial_fact.extra_data_pillar_block_hash_present = extra_data_pillar_block_hash_present;
  initial_fact.pillar_votes_required = pillar_votes_required;
  initial_fact.pillar_votes_present = pillar_votes_present;
  initial_fact.previous_cert_votes_present = previous_cert_votes_present;
  initial_fact.previous_cert_first_vote_has_weight = previous_cert_first_vote_has_weight;
  auto session_step =
      rustaxa::pbft_manager_runtime_begin_pbft_sync_admission(pbft_service_->service(), std::move(initial_fact));

  std::optional<std::vector<std::shared_ptr<PbftVote>>> reward_votes;
  std::vector<std::shared_ptr<PbftVote>> cert_votes;
  std::optional<uint64_t> active_sync_cert_session;
  try {
    while (session_step.has_check) {
      if (session_step.next_check == kPbftSyncRuntimeCheckFinalChainHash) {
        if (session_step.plan.wait_for_finalization) {
          final_chain_->waitForFinalized();
        }
        session_step = rustaxa::pbft_manager_runtime_pbft_sync_admission_report_status(
            pbft_service_->service(), final_chain_->rustFinalChain(), session_step.cursor, session_step.next_check, 0);
        reward_votes = fromBridgePbftVotes(session_step.reward_vote_rlps);
        continue;
      }
      if (session_step.next_check == kPbftSyncRuntimeCheckCertVotes) {
        if (session_step.plan.replace_previous_block_cert_votes ||
            (block_period > 1 && (period_data.previous_block_cert_votes.empty() ||
                                  !period_data.previous_block_cert_votes.front()->getWeight()))) {
          assert(reward_votes.has_value());
          period_data.previous_block_cert_votes = *reward_votes;
        }
        if (block_period > 1 && period_data.previous_block_cert_votes.empty()) {
          constexpr PbftRound kMaxRecoveredCertVoteRound = 100;
          for (PbftRound round = 1; round <= kMaxRecoveredCertVoteRound; ++round) {
            auto recovered_votes = vote_mgr_->getTwoTPlusOneVotedBlockVotes(block_period - 1, round,
                                                                            TwoTPlusOneVotedBlockType::CertVotedBlock);
            if (!recovered_votes.empty() && recovered_votes.front()->getBlockHash() == block_prev_hash) {
              period_data.previous_block_cert_votes = std::move(recovered_votes);
              break;
            }
          }
        }
        rustaxa::PbftSyncCertBundleCommand cert_command{};
        cert_command.action = kPbftSyncCertBundleCommandBegin;
        cert_command.block_period = block_period;
        cert_command.block_hash = toBridgeHash(pbft_block_hash);
        cert_command.cert_vote_rlps = std::move(cert_vote_rlps);
        auto cert_vote_step = rustaxa::pbft_service_pbft_sync_cert_bundle_session(
            pbft_service_->service(), final_chain_->rustFinalChain(), std::move(cert_command));
        if (cert_vote_step.action == kPbftSyncCertBundleActionAwaitingSlashing) {
          active_sync_cert_session = cert_vote_step.session_id;
        }
        while (cert_vote_step.action == kPbftSyncCertBundleActionAwaitingSlashing) {
          if (!cert_vote_step.has_slashing_effect) {
            throw std::runtime_error("Rust sync cert-vote session requested slashing without an executable effect");
          }
          const auto &effect = cert_vote_step.slashing_transaction_effect;
          if (effect.status != 0) {
            throw std::runtime_error("Rust sync cert-vote session returned a non-executable slashing effect");
          }

          const auto &wallet = eligible_wallets_.getSigningWallet(effect.wallet_index);
          const auto proof_hash = effect.proof_hash;
          bytes call_data(effect.call_data.begin(), effect.call_data.end());
          auto transaction = std::make_shared<Transaction>(
              fromBridgeU256(effect.nonce), fromBridgeU256(effect.value), trx_mgr_->gasPriceBid(), effect.gas_limit,
              std::move(call_data), wallet.node_secret, fromBridgeAddress(effect.contract_address),
              kGenesisConfig.chain_id);
          const bool transaction_inserted = trx_mgr_->insertTransaction(transaction).first;
          rustaxa::PbftSyncCertBundleCommand report_command{};
          report_command.action = kPbftSyncCertBundleCommandReportSlashing;
          report_command.session_id = cert_vote_step.session_id;
          report_command.effect_id = cert_vote_step.effect_id;
          report_command.proof_hash = proof_hash;
          report_command.transaction_inserted = transaction_inserted;
          cert_vote_step = rustaxa::pbft_service_pbft_sync_cert_bundle_session(
              pbft_service_->service(), final_chain_->rustFinalChain(), std::move(report_command));
        }
        active_sync_cert_session.reset();

        if (cert_vote_step.has_slashing_effect) {
          throw std::runtime_error("Rust sync cert-vote session terminated with a pending slashing effect");
        }
        const bool cert_votes_accepted = cert_vote_step.action == kPbftSyncCertBundleActionAccepted;
        if (cert_votes_accepted) {
          cert_votes = fromBridgePbftVotes(cert_vote_step.weighted_vote_rlps);
        } else if (cert_vote_step.action != kPbftSyncCertBundleActionRejected) {
          throw std::runtime_error("Rust sync cert-vote session returned an unknown terminal action");
        } else if (!cert_vote_step.error_code.empty()) {
          LOG(log_er_) << "Cert vote " << fromBridgeHash(cert_vote_step.first_bad_vote_hash)
                       << " validation failed. Err: " << static_cast<std::string>(cert_vote_step.error_code)
                       << ", pbft block " << pbft_block_hash;
        } else {
          LOG(log_wr_) << "Rust sync cert-vote bundle admission failed for PBFT block " << pbft_block_hash
                       << ", period " << block_period << ", status " << static_cast<uint32_t>(cert_vote_step.status)
                       << ", votes weight " << cert_vote_step.total_weight << ", two_t_plus_one "
                       << cert_vote_step.two_t_plus_one << ", first bad vote "
                       << fromBridgeHash(cert_vote_step.first_bad_vote_hash);
        }
        session_step = rustaxa::pbft_manager_runtime_pbft_sync_admission_report_status(
            pbft_service_->service(), final_chain_->rustFinalChain(), session_step.cursor, session_step.next_check,
            cert_votes_accepted ? kPbftSyncFactValid : kPbftSyncFactInvalid);
        continue;
      }
      if (session_step.next_check == kPbftSyncRuntimeCheckTransactions) {
        session_step = rustaxa::pbft_manager_runtime_pbft_sync_admission_validate_transactions(
            pbft_service_->service(), dag_transaction_service_->service(), final_chain_->rustFinalChain(),
            std::move(period_data_transaction_identities));
        continue;
      }
      if (session_step.next_check == kPbftSyncRuntimeCheckPillarVotes) {
        session_step = rustaxa::pbft_manager_runtime_pbft_sync_admission_validate_pillar_votes(
            pbft_service_->service(), final_chain_->rustFinalChain(), toBridgePillarVoteRlps(pillar_vote_rlps));
        continue;
      }
      rustaxa::abort_pbft_manager_runtime_pbft_sync_admission(pbft_service_->service());
      throw std::runtime_error("Rust PBFT sync admission requested unsupported external check");
    }
  } catch (...) {
    if (active_sync_cert_session) {
      try {
        rustaxa::PbftSyncCertBundleCommand abort_command{};
        abort_command.action = kPbftSyncCertBundleCommandAbort;
        abort_command.session_id = *active_sync_cert_session;
        rustaxa::pbft_service_pbft_sync_cert_bundle_session(pbft_service_->service(), final_chain_->rustFinalChain(),
                                                            std::move(abort_command));
      } catch (...) {
        // Preserve the original executor exception; the exact-session abort is best-effort on lock failure.
      }
    }
    rustaxa::abort_pbft_manager_runtime_pbft_sync_admission(pbft_service_->service());
    throw;
  }

  if (!session_step.can_continue) {
    throw std::runtime_error("Rust PBFT sync admission contract failed: " +
                             static_cast<std::string>(session_step.error_code));
  }
  apply_rust_admission_side_effects(session_step);
  if (!session_step.plan.accept_period_data) {
    return std::nullopt;
  }

  for (const auto &warning : session_step.plan.warnings) {
    if (warning.kind == kPbftSyncTransactionWarningMissingTransaction) {
      LOG(log_er_) << "Synced PBFT block " << pbft_block_hash << " has missing transaction "
                   << fromBridgeTransactionHash(warning.hash);
      continue;
    }
    if (warning.kind == kPbftSyncTransactionWarningFinalizedTransaction) {
      LOG(log_er_) << "Synced PBFT block " << pbft_block_hash << " has finalized transaction "
                   << fromBridgeTransactionHash(warning.hash);
      continue;
    }
    throw std::runtime_error("Rust PBFT sync runtime returned unknown transaction warning");
  }
  if (session_step.plan.contains_finalized_transaction_warning) {
    LOG(log_er_) << "Synced PBFT block " << pbft_block_hash << " has finalized transactions";
  }

  try {
    period_data.transactions = materializeTransactionsFromQueuedRlps(transaction_rlps, period_data_transaction_hashes);
  } catch (const std::exception &e) {
    LOG(log_er_) << "Synced PBFT block " << pbft_block_hash
                 << " has invalid queued transaction payload metadata: " << e.what();
    rustaxa::abort_pbft_manager_runtime_pbft_sync_admission(pbft_service_->service());
    return std::nullopt;
  }

  return std::optional<std::pair<PeriodData, std::vector<std::shared_ptr<PbftVote>>>>(
      {std::move(period_data), std::move(cert_votes)});
}

bool PbftManager::periodDataQueueEmpty() const {
  const auto snapshot = rustaxa::pbft_manager_runtime_period_data_queue_snapshot(pbft_service_->service());
  return snapshot.empty;
}

size_t PbftManager::periodDataQueueSize() const {
  const auto snapshot = rustaxa::pbft_manager_runtime_period_data_queue_snapshot(pbft_service_->service());
  return snapshot.size;
}

std::shared_ptr<PbftBlock> PbftManager::getPbftProposedBlock(PbftPeriod period, const blk_hash_t &block_hash) const {
  auto proposed_block = pbft_service_->service().pbft_service_proposed_blocks_get(period, toBridgeHash(block_hash));
  if (!proposed_block.found) {
    return nullptr;
  }

  return std::make_shared<PbftBlock>(fromBridgeBytes(proposed_block.block_rlp));
}

PbftManager::EligibleWallets::EligibleWallets(const std::vector<WalletConfig> &wallets) {
  wallets_.reserve(wallets.size());
  for (const auto &wallet : wallets) {
    wallets_.emplace_back(false, wallet);
  }
}

void PbftManager::EligibleWallets::updateWalletsEligibility(
    PbftPeriod period, const SharedPbftService &pbft_service,
    const std::shared_ptr<final_chain::FinalChain> &final_chain) {
  assert(period > period_ || period == 0);
  assert(period <= final_chain->lastBlockNumber() + final_chain->delegationDelay());

  rustaxa::PbftFinalChainDposWalletEligibilityBatchRequest request;
  request.period = period;
  request.addresses.reserve(wallets_.size());
  for (const auto &wallet : wallets_) {
    rustaxa::PbftFinalChainDposAddress bridge_address;
    bridge_address.address = toBridgeFixedBytes<20>(wallet.second.node_addr);
    request.addresses.push_back(bridge_address);
  }

  const auto facts = pbft_service->service().pbft_service_collect_dpos_wallet_eligibility_batch(
      final_chain->rustFinalChain(), request);
  if (facts.status != kPbftSyncDposFactsReady) {
    throw std::runtime_error("Rust FinalChain PBFT wallet-eligibility batch fact collection failed: " +
                             static_cast<std::string>(facts.error_code));
  }
  if (facts.address_facts.size() != request.addresses.size()) {
    throw std::runtime_error("Rust FinalChain PBFT wallet-eligibility batch size mismatch");
  }

  std::vector<bool> next_eligibility;
  next_eligibility.reserve(wallets_.size());
  for (size_t index = 0; index < wallets_.size(); ++index) {
    const auto &address_fact = facts.address_facts[index];
    if (address_fact.status != kPbftSyncDposFactsReady || address_fact.address != request.addresses[index].address) {
      throw std::runtime_error("Rust FinalChain PBFT wallet-eligibility batch order mismatch");
    }
    next_eligibility.push_back(address_fact.eligible);
  }
  for (size_t index = 0; index < wallets_.size(); ++index) {
    wallets_[index].first = next_eligibility[index];
  }
  period_ = period;
}

const std::vector<std::pair<bool, WalletConfig>> &PbftManager::EligibleWallets::getWallets(
    PbftPeriod current_pbft_period) const {
  assert(period_ == current_pbft_period - 1);

  return wallets_;
}

const WalletConfig &PbftManager::EligibleWallets::getSigningWallet(size_t wallet_index) const {
  return wallets_.at(wallet_index).second;
}

PbftPeriod PbftManager::EligibleWallets::getWalletsEligiblePeriod() const { return period_; }

std::chrono::milliseconds PbftManager::getPbftDeadline() const {
  if (!pbft_service_) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before reading PBFT deadline");
  }
  const auto snapshot = rustaxa::pbft_manager_runtime_snapshot(pbft_service_->service());
  if (snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
    throw std::runtime_error("PBFT manager Rust runtime snapshot is not ready: " +
                             static_cast<std::string>(snapshot.error_code));
  }
  const auto current_round_lambda = std::chrono::milliseconds(snapshot.current_round_lambda_ms);
  const auto current_round = static_cast<PbftRound>(snapshot.round);

  if (kGenesisConfig.state.hardforks.isOnCactiHardfork(getPbftPeriod())) {
    auto block_propagation = std::chrono::milliseconds(kGenesisConfig.state.hardforks.cacti_hf.block_propagation_min);
    if (current_round > 1) {
      block_propagation = std::chrono::milliseconds(kGenesisConfig.state.hardforks.cacti_hf.block_propagation_max);
    }

    return std::max(4 * current_round_lambda, block_propagation);
  }

  return 4 * current_round_lambda;
}

}  // namespace taraxa

#endif  // RUSTAXA_ENABLE
