#if defined(RUSTAXA_ENABLE_PILLAR_VOTES) || defined(RUSTAXA_ENABLE_PROPOSED_BLOCKS)

#include <libdevcore/SHA3.h>

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <optional>
#include <stdexcept>
#include <string>
#include <unordered_set>
#include <vector>

#include "config/version.hpp"
#include "dag/dag.hpp"
#include "dag/dag_manager.hpp"
#include "final_chain/final_chain.hpp"
#include "pbft/pbft_manager.hpp"
#include "pbft/period_data.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"
#include "vote_manager/vote_manager.hpp"

namespace taraxa {
using namespace std::chrono_literals;

constexpr std::chrono::milliseconds kPollingIntervalMs{100};
constexpr PbftStep kMaxSteps{13};  // Need to be a odd number

namespace {

constexpr uint8_t kPbftSyncFinalChainValid = 0;
constexpr uint8_t kPbftSyncFinalChainMissing = 1;
constexpr uint8_t kPbftSyncFinalChainInvalid = 2;
constexpr uint8_t kPbftSyncFinalChainNotChecked = 3;
constexpr uint8_t kPbftSyncFactValid = 0;
constexpr uint8_t kPbftSyncFactInvalid = 1;
constexpr uint8_t kPbftSyncFactNotRequired = 2;
constexpr uint8_t kPbftSyncFactNotChecked = 3;

constexpr uint8_t kPbftSyncStatusBlockAlreadyInChain = 1;
constexpr uint8_t kPbftSyncStatusStalePeriod = 2;
constexpr uint8_t kPbftSyncStatusPreviousHashMismatch = 3;
constexpr uint8_t kPbftSyncStatusCertVotesInvalid = 7;
constexpr uint8_t kPbftSyncStatusPillarDataInvalid = 10;
constexpr uint8_t kPbftSyncStatusPillarVotesInvalid = 11;
constexpr uint8_t kPbftSyncRuntimeActionContractError = 5;
constexpr uint8_t kPbftFinalizationStatusAccepted = 0;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusApplied = 0;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusAlreadyApplied = 1;
constexpr uint8_t kPbftFinalizationStorageStagePrimary = 0;
constexpr uint8_t kPbftFinalizationStorageStageDynamicLambda = 1;
constexpr uint8_t kPbftFinalizationStorageStageExecutedStatus = 2;
constexpr uint8_t kPbftFinalizationStorageStageSortition = 3;
constexpr uint8_t kPbftFinalizationRuntimeStatusActive = 0;
constexpr uint8_t kPbftFinalizationRuntimeStatusComplete = 1;
constexpr uint8_t kPbftFinalizationResumeStatusNeedsFinalChainReplay = 2;
constexpr uint8_t kPbftFinalizationResumeStatusNeedsExecutedStatusPersistence = 3;
constexpr uint8_t kPbftFinalizationResumeStatusNeedsDynamicLambdaPersistence = 6;
constexpr uint8_t kPbftFinalizationResumeStatusNeedsPillarPostProcessingReplay = 7;
constexpr uint8_t kPbftFinalizationRuntimeActionApplyPrimaryStorage = 0;
constexpr uint8_t kPbftFinalizationRuntimeActionCommitRewardVotesReset = 3;
constexpr uint8_t kPbftFinalizationRuntimeActionSetDagBlockOrder = 4;
constexpr uint8_t kPbftFinalizationRuntimeActionUpdateFinalizedTransactions = 5;
constexpr uint8_t kPbftFinalizationRuntimeActionUpdatePbftChain = 6;
constexpr uint8_t kPbftFinalizationRuntimeActionClearAnchorDagCache = 7;
constexpr uint8_t kPbftFinalizationRuntimeActionApplyDynamicLambda = 8;
constexpr uint8_t kPbftFinalizationRuntimeActionFinalizeFinalChain = 9;
constexpr uint8_t kPbftFinalizationRuntimeActionPersistExecutedStatus = 10;
constexpr uint8_t kPbftFinalizationRuntimeActionSetExecutedFlag = 11;
constexpr uint8_t kPbftFinalizationRuntimeActionAdvancePeriod = 12;
constexpr uint8_t kPbftFinalizationRuntimeActionCommitSortitionRuntime = 14;
constexpr uint8_t kPbftFinalizationRuntimeActionProcessPillarBlock = 15;
constexpr uint8_t kPbftFinalizationPillarPreflightActionFinalizePillarBlock = 1;
constexpr uint8_t kPbftFinalizationPillarPreflightStatusAccepted = 0;
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
constexpr uint8_t kPbftManagerStateActionStatusReady = 0;
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
constexpr uint8_t kPbftManagerProposalActionRequestDagOrder = 0;
constexpr uint8_t kPbftManagerProposalActionBuildProposal = 1;
constexpr uint8_t kPbftManagerProposalActionSkipProposal = 2;
constexpr uint8_t kPbftManagerProposalActionContractError = 255;
constexpr uint8_t kPbftManagerProposalStatusBuildReady = 1;
constexpr uint8_t kPbftManagerBroadcastActionNoop = 0;
constexpr uint8_t kPbftManagerBroadcastActionPeriodVotes = 1;
constexpr uint8_t kPbftManagerBroadcastActionRoundVotes = 2;
constexpr uint8_t kPbftManagerBroadcastStatusReady = 0;
constexpr uint8_t kPbftSyncQueueDrainActionCleanOldData = 0;
constexpr uint8_t kPbftSyncQueueDrainActionPopAndProcess = 1;
constexpr uint8_t kPbftSyncQueueDrainActionPushAccepted = 2;
constexpr uint8_t kPbftSyncQueueDrainActionUpdateSyncState = 3;
constexpr uint8_t kPbftSyncQueueDrainActionStop = 4;
constexpr uint8_t kPbftSyncQueueDrainStatusActive = 0;
constexpr uint8_t kPbftSyncQueueDrainStatusComplete = 1;
constexpr uint8_t kPbftSyncTransactionWarningMissingTransaction = 1;
constexpr uint8_t kPbftSyncTransactionWarningFinalizedTransaction = 2;
constexpr uint8_t kPbftManagerStartupRestoreStatusReady = 0;
constexpr uint8_t kPbftManagerTransitionStatusReady = 0;
constexpr uint8_t kPbftManagerTransitionStorageStatusApplied = 0;
constexpr uint8_t kPbftManagerAdvancePeriodActionApplyResetConsensusTransition = 0;
constexpr uint8_t kPbftManagerAdvancePeriodActionApplyExecutedBlockReset = 1;
constexpr uint8_t kPbftManagerAdvancePeriodActionSetVoteManagerPeriodRound = 2;
constexpr uint8_t kPbftManagerAdvancePeriodActionResetCurrentRoundTimer = 3;
constexpr uint8_t kPbftManagerAdvancePeriodActionResetRewardVoteCounters = 4;
constexpr uint8_t kPbftManagerAdvancePeriodActionResetPeriodTimer = 5;
constexpr uint8_t kPbftManagerAdvancePeriodActionUpdateWalletEligibility = 6;
constexpr uint8_t kPbftManagerAdvancePeriodActionCleanupVotes = 7;
constexpr uint8_t kPbftManagerAdvancePeriodActionCleanupProposedBlocks = 8;
constexpr uint8_t kPbftManagerTransitionResetConsensus = 0;
constexpr uint8_t kPbftManagerTransitionToFilter = 1;
constexpr uint8_t kPbftManagerTransitionToCertify = 2;
constexpr uint8_t kPbftManagerTransitionToFinish = 3;
constexpr uint8_t kPbftManagerTransitionToFinishPolling = 4;
constexpr uint8_t kPbftManagerTransitionLoopBackFinish = 5;
constexpr uint8_t kPbftManagerTransitionDelayCertifyPoll = 6;
constexpr uint8_t kPbftManagerTransitionDelayFinishPoll = 7;
constexpr uint8_t kPbftManagerLeaderBlockAlreadyValid = 0;
constexpr uint8_t kPbftManagerLeaderBlockValidated = 1;
constexpr uint8_t kPbftManagerLeaderBlockRejected = 2;
constexpr uint8_t kPbftManagerCandidateAdmissionValidationNotChecked = 0;
constexpr uint8_t kPbftManagerCandidateAdmissionValidationValid = 1;
constexpr uint8_t kPbftManagerCandidateAdmissionValidationInvalid = 2;
constexpr uint8_t kPbftManagerCandidateAdmissionActionRequestLookup = 0;
constexpr uint8_t kPbftManagerCandidateAdmissionActionRequestValidation = 1;
constexpr uint8_t kPbftManagerCandidateAdmissionActionAccept = 2;
constexpr uint8_t kPbftManagerCandidateAdmissionActionReject = 3;
constexpr uint8_t kPbftManagerCandidateAdmissionActionDeferMissingBlock = 4;
constexpr uint8_t kPbftManagerCandidateAdmissionActionContractError = 255;
constexpr uint8_t kPbftManagerBlockValidationFactNotChecked = 0;
constexpr uint8_t kPbftManagerBlockValidationFactValid = 1;
constexpr uint8_t kPbftManagerBlockValidationFactInvalid = 2;
constexpr uint8_t kPbftManagerBlockValidationFactMissing = 3;
constexpr uint8_t kPbftManagerBlockValidationFactNotRequired = 4;
constexpr uint8_t kPbftManagerBlockValidationActionRunCheck = 0;
constexpr uint8_t kPbftManagerBlockValidationActionAccept = 1;
constexpr uint8_t kPbftManagerBlockValidationActionReject = 2;
constexpr uint8_t kPbftManagerBlockValidationActionWaitForFinalization = 3;
constexpr uint8_t kPbftManagerBlockValidationActionContractError = 255;
constexpr uint8_t kPbftManagerBlockValidationStatusFinalChainHashInvalid = 4;
constexpr uint8_t kPbftManagerBlockValidationStatusRewardVotesInvalid = 5;
constexpr uint8_t kPbftManagerBlockValidationStatusExtraDataInvalid = 6;
constexpr uint8_t kPbftManagerBlockValidationCheckPbftChain = 0;
constexpr uint8_t kPbftManagerBlockValidationCheckFinalChainHash = 1;
constexpr uint8_t kPbftManagerBlockValidationCheckRewardVotes = 2;
constexpr uint8_t kPbftManagerBlockValidationCheckExtraData = 3;
constexpr uint8_t kPbftManagerBlockValidationCheckPillarBlock = 4;
constexpr uint8_t kPbftManagerBlockValidationCheckDagOrder = 5;
constexpr uint8_t kPbftManagerBlockValidationCheckDagWeight = 6;

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t &hash) { return hash.asArray(); }

std::array<uint8_t, 20> toBridgeAddress(const addr_t &address) { return address.asArray(); }

template <size_t N, typename FixedHash>
std::array<uint8_t, N> toBridgeFixedBytes(const FixedHash &value) {
  return value.asArray();
}

uint256_hash_t fromBridgeHash(const std::array<uint8_t, 32> &hash) {
  return uint256_hash_t(hash.data(), uint256_hash_t::ConstructFromPointer);
}

rust::Vec<rustaxa::PbftFinalChainFactAddress> toBridgeAddresses(const std::vector<addr_t> &addresses) {
  rust::Vec<rustaxa::PbftFinalChainFactAddress> out;
  out.reserve(addresses.size());
  for (const auto &address : addresses) {
    rustaxa::PbftFinalChainFactAddress fact_address;
    fact_address.address = toBridgeAddress(address);
    out.push_back(fact_address);
  }
  return out;
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

rustaxa::PbftFinalChainFactRequest makePbftFinalChainFactRequest(
    PbftPeriod period, const blk_hash_t &candidate_final_chain_hash, bool collect_final_chain_hash,
    bool validate_candidate_final_chain_hash, bool collect_total_vote_count, bool collect_address_vote_counts,
    std::vector<addr_t> addresses = {}) {
  rustaxa::PbftFinalChainFactRequest request;
  request.period = period;
  request.candidate_final_chain_hash = toBridgeHash(candidate_final_chain_hash);
  request.collect_final_chain_hash = collect_final_chain_hash;
  request.validate_candidate_final_chain_hash = validate_candidate_final_chain_hash;
  request.collect_total_vote_count = collect_total_vote_count;
  request.collect_address_vote_counts = collect_address_vote_counts;
  request.addresses = toBridgeAddresses(addresses);
  return request;
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

void ensurePbftManagerRuntimeSnapshotReady(const rustaxa::PbftManagerRuntimeSnapshot &snapshot,
                                           const char *operation) {
  if (snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
    throw std::runtime_error(std::string(operation) + " rejected by Rust PBFT manager runtime: " +
                             static_cast<std::string>(snapshot.error_code));
  }
}

rustaxa::PbftManagerTransitionFact makePbftManagerTransitionFact(
    uint8_t kind, PbftPeriod period, PbftRound round, PbftStep step, PbftRound target_round,
    std::chrono::milliseconds current_round_lambda, std::chrono::milliseconds target_round_lambda,
    std::chrono::milliseconds default_lambda, std::chrono::milliseconds max_exponential_lambda,
    std::chrono::milliseconds deadline, std::chrono::milliseconds next_step_time, const VoteManager &vote_mgr,
    bool cacti_hardfork, bool has_cert_voted_block, bool executed_pbft_block) {
  rustaxa::PbftManagerTransitionFact fact{};
  fact.kind = kind;
  fact.period = period;
  fact.round = round;
  fact.step = step;
  fact.target_round = target_round;
  fact.current_round_lambda_ms = static_cast<uint64_t>(current_round_lambda.count());
  fact.target_round_lambda_ms = static_cast<uint64_t>(target_round_lambda.count());
  fact.default_lambda_ms = static_cast<uint64_t>(default_lambda.count());
  fact.max_exponential_lambda_ms = static_cast<uint64_t>(max_exponential_lambda.count());
  fact.max_steps = kMaxSteps;
  const auto next_step =
      kind == kPbftManagerTransitionResetConsensus
          ? PbftStep{1}
          : (kind == kPbftManagerTransitionDelayCertifyPoll || kind == kPbftManagerTransitionDelayFinishPoll
                 ? step
                 : step + 1);
  if (next_step >= kMaxSteps && next_step % 2) {
    fact.network_next_voting_step = vote_mgr.getNetworkTplusOneNextVotingStep(period, round);
  }
  fact.deadline_ms = static_cast<uint64_t>(deadline.count());
  fact.polling_interval_ms = static_cast<uint64_t>(kPollingIntervalMs.count());
  fact.next_step_time_ms = static_cast<uint64_t>(next_step_time.count());
  fact.cacti_hardfork = cacti_hardfork;
  fact.has_cert_voted_block = has_cert_voted_block;
  fact.executed_pbft_block = executed_pbft_block;
  return fact;
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

template <typename Logger>
bool ensureStateActionPlanReady(const rustaxa::PbftManagerStateActionPlan &plan, Logger &log_er) {
  if (plan.status == kPbftManagerStateActionStatusReady) {
    return true;
  }
  LOG(log_er) << "Rust PBFT manager state-action planner rejected facts, status " << static_cast<uint32_t>(plan.status)
              << ", error " << static_cast<std::string>(plan.error_code);
  assert(false);
  return false;
}

template <typename Executor, typename Logger>
rustaxa::PbftManagerStateActionSessionStep executeStateActionEffectSession(
    const rustaxa::PbftManagerStateActionFact &fact, Executor &&executor, Logger &log_er) {
  auto session = rustaxa::create_pbft_manager_state_action_effect_session(fact);
  auto step = session->pbft_manager_state_action_effect_session_next();
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
    step = session->pbft_manager_state_action_effect_session_report(report);
  }
  if (!step.can_continue) {
    LOG(log_er) << "Rust PBFT manager state-action effect session stopped, status "
                << static_cast<uint32_t>(step.status) << ", cursor " << step.cursor << ", error "
                << static_cast<std::string>(step.error_code);
  }
  return step;
}

template <typename Logger>
bool ensureTransitionPlanReady(const rustaxa::PbftManagerTransitionPlan &plan, Logger &log_er) {
  if (plan.status == kPbftManagerTransitionStatusReady) {
    return true;
  }
  LOG(log_er) << "Rust PBFT manager transition planner rejected facts, status " << static_cast<uint32_t>(plan.status)
              << ", error " << static_cast<std::string>(plan.error_code);
  assert(false);
  return false;
}

void applyPbftManagerTransitionPlan(const rustaxa::PbftManagerTransitionPlan &plan,
                                    rustaxa::BridgePbftManagerRuntime &runtime,
                                    const std::shared_ptr<VoteManager> &vote_mgr, std::atomic<PbftRound> &round,
                                    PbftStep &step, PbftStates &state, std::chrono::milliseconds &current_round_lambda,
                                    std::chrono::milliseconds &next_step_time, uint32_t &rounds_count_dynamic_lambda,
                                    uint32_t &dynamic_lambda, bool &executed_pbft_block,
                                    std::optional<std::shared_ptr<PbftBlock>> &cert_voted_block_for_round,
                                    std::map<blk_hash_t, std::vector<PbftStep>> &current_round_broadcasted_votes,
                                    uint32_t &broadcast_votes_counter, uint32_t &rebroadcast_votes_counter,
                                    uint32_t &broadcast_reward_votes_counter,
                                    uint32_t &rebroadcast_reward_votes_counter,
                                    bool &already_next_voted_value, bool &already_next_voted_null_block_hash,
                                    bool &print_cert_step_info, bool &print_second_finish_step_info,
                                    std::chrono::system_clock::time_point &second_finish_step_start_datetime) {
  rust::Vec<rustaxa::PbftFinalizationHash> own_vote_hashes;
  if (plan.clear_own_votes) {
    const auto own_verified_votes = vote_mgr->getOwnVerifiedVotes();
    own_vote_hashes.reserve(own_verified_votes.size());
    for (const auto &vote : own_verified_votes) {
      if (!vote) {
        throw std::runtime_error("PBFT manager transition cannot clear a null own verified vote");
      }
      rustaxa::PbftFinalizationHash hash{};
      hash.hash = toBridgeHash(vote->getHash());
      own_vote_hashes.push_back(hash);
    }
  }

  const auto storage_result =
      rustaxa::pbft_manager_runtime_apply_transition_storage_write(runtime, plan, std::move(own_vote_hashes));
  if (storage_result.status != kPbftManagerTransitionStorageStatusApplied) {
    throw std::runtime_error("Rust PBFT manager transition storage apply failed: " +
                             static_cast<std::string>(storage_result.error_code));
  }

  applyPbftManagerRuntimeSnapshot(storage_result.snapshot, round, step, state, current_round_lambda, next_step_time,
                                  rounds_count_dynamic_lambda, dynamic_lambda, executed_pbft_block,
                                  already_next_voted_value, already_next_voted_null_block_hash,
                                  broadcast_votes_counter, rebroadcast_votes_counter, broadcast_reward_votes_counter,
                                  rebroadcast_reward_votes_counter);

  if (plan.remove_cert_voted_block) {
    cert_voted_block_for_round.reset();
  }
  if (plan.clear_own_votes) {
    vote_mgr->clearOwnVerifiedVotesAfterRustPersistence();
  }
  if (plan.clear_broadcasted_votes) {
    current_round_broadcasted_votes.clear();
  }
  if (plan.reset_second_finish_start) {
    second_finish_step_start_datetime = std::chrono::system_clock::now();
  }
  if (plan.print_cert_step_info) {
    print_cert_step_info = true;
  }
  if (plan.print_second_finish_step_info) {
    print_second_finish_step_info = true;
  }
}

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes &bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

rust::Vec<uint8_t> toBridgeBytes(const std::string &bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

template <typename Hash>
rust::Vec<rustaxa::PbftFinalizationHash> toBridgeFinalizationHashes(const std::vector<Hash> &hashes) {
  rust::Vec<rustaxa::PbftFinalizationHash> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.push_back(rustaxa::PbftFinalizationHash{toBridgeHash(hash)});
  }
  return out;
}

rust::Vec<rustaxa::PbftSyncTransactionHash> toBridgeTransactionHashes(const std::unordered_set<trx_hash_t> &hashes) {
  rust::Vec<rustaxa::PbftSyncTransactionHash> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.push_back(rustaxa::PbftSyncTransactionHash{toBridgeHash(hash)});
  }
  return out;
}

rust::Vec<rustaxa::PbftSyncTransactionHash> toBridgeTransactionHashes(const std::vector<trx_hash_t> &hashes) {
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
  write_stage.has_reward_votes_reset = false;

  return write_stage;
}

rustaxa::PbftFinalizationStorageWriteStage makeSortitionFinalizationStorageStage(const SortitionParamsChange &change) {
  auto write_stage = makeFinalizationStorageStage(kPbftFinalizationStorageStageSortition);
  write_stage.has_sortition_params_change = true;
  write_stage.sortition_params_change_period = change.period;
  write_stage.sortition_params_change_interval_efficiency = change.interval_efficiency;
  write_stage.sortition_params_change_threshold_upper = change.vrf_params.threshold_upper;
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

rustaxa::PbftSyncProcessPeriodDataRuntimeFact makePbftSyncProcessPeriodDataRuntimeFact(
    PbftPeriod block_period, const blk_hash_t &block_prev_hash,
    const std::vector<trx_hash_t> &dag_transaction_hashes,
    const std::vector<trx_hash_t> &period_data_transaction_hashes, const blk_hash_t &last_pbft_block_hash,
    PbftPeriod last_pbft_block_period, bool block_in_chain, uint8_t final_chain_hash_status,
    uint8_t reward_votes_status, uint8_t cert_votes_status, uint8_t transactions_status,
    const std::unordered_set<trx_hash_t> &missing_transaction_hashes, bool contains_finalized_transactions,
    uint8_t pillar_data_status, bool pillar_votes_required, uint8_t pillar_votes_status,
    bool previous_cert_votes_present, bool previous_cert_first_vote_has_weight, bool extra_data_required,
    bool extra_data_present, bool extra_data_pillar_block_hash_present, bool pillar_votes_present) {
  rustaxa::PbftSyncProcessPeriodDataRuntimeFact fact;
  fact.block_period = block_period;
  fact.block_prev_hash = toBridgeHash(block_prev_hash);
  fact.chain_last_hash = toBridgeHash(last_pbft_block_hash);
  fact.chain_last_period = last_pbft_block_period;
  fact.block_in_chain = block_in_chain;
  fact.final_chain_hash_status = final_chain_hash_status;
  fact.reward_votes_status = reward_votes_status;
  fact.cert_votes_status = cert_votes_status;
  fact.transactions_status = transactions_status;
  fact.dag_transaction_hashes = toBridgeTransactionHashes(dag_transaction_hashes);
  fact.period_data_transaction_hashes = toBridgeTransactionHashes(period_data_transaction_hashes);
  fact.missing_transaction_hashes = toBridgeTransactionHashes(missing_transaction_hashes);
  // TODO(RUSTAXA): populate hash-specific finalized transaction warnings when the transaction-manager executor returns
  // finalized hashes instead of only the legacy boolean `verifyTransactionsNotFinalized` result.
  fact.finalized_transaction_hashes = rust::Vec<rustaxa::PbftSyncTransactionHash>();
  fact.contains_finalized_transactions = contains_finalized_transactions;
  fact.pillar_data_status = pillar_data_status;
  fact.extra_data_required = extra_data_required;
  fact.extra_data_present = extra_data_present;
  fact.extra_data_pillar_block_hash_present = extra_data_pillar_block_hash_present;
  fact.pillar_votes_required = pillar_votes_required;
  fact.pillar_votes_present = pillar_votes_present;
  fact.pillar_votes_status = pillar_votes_status;
  fact.previous_cert_votes_present = previous_cert_votes_present;
  fact.previous_cert_first_vote_has_weight = previous_cert_first_vote_has_weight;
  return fact;
}

rustaxa::PbftFinalizationIntentFact makePbftFinalizationIntentFact(
    const PeriodData &period_data, const blk_hash_t &pbft_head_hash, const blk_hash_t &last_pbft_block_hash,
    PbftPeriod last_pbft_block_period, bool block_in_chain, bool pillar_block_finalized,
    bool request_dynamic_lambda_update, uint64_t cert_vote_count, const blk_hash_t &sample_cert_vote_block_hash,
    PbftPeriod sample_cert_vote_period, PbftRound sample_cert_vote_round, PbftStep sample_cert_vote_step,
    uint32_t block_lambda, bool last_saved_period_lambda_found, uint32_t last_saved_period_lambda,
    uint32_t dynamic_blocks_per_year, uint32_t rounds_count_dynamic_lambda, uint32_t dynamic_lambda,
    uint32_t dpos_blocks_per_year, const std::vector<blk_hash_t> &dag_blocks_order,
    const std::vector<trx_hash_t> &transaction_order, const std::string &pbft_head_payload,
    bool process_pillar_block_after_advance) {
  rustaxa::PbftFinalizationIntentFact fact;
  fact.block_hash = toBridgeHash(period_data.pbft_blk->getBlockHash());
  fact.pbft_head_hash = toBridgeHash(pbft_head_hash);
  fact.block_period = period_data.pbft_blk->getPeriod();
  fact.block_prev_hash = toBridgeHash(period_data.pbft_blk->getPrevBlockHash());
  fact.chain_last_hash = toBridgeHash(last_pbft_block_hash);
  fact.chain_last_period = last_pbft_block_period;
  fact.block_in_chain = block_in_chain;
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
  fact.pbft_head_payload = block_in_chain ? rust::Vec<uint8_t>() : toBridgeBytes(pbft_head_payload);
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

PbftManager::PbftManager(const FullNodeConfig &conf, std::shared_ptr<DbStorage> db,
                         rust::Box<rustaxa::BridgePbftManagerRuntime> pbft_manager_runtime,
                         std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<VoteManager> vote_mgr,
                         std::shared_ptr<DagManager> dag_mgr, std::shared_ptr<TransactionManager> trx_mgr,
                         std::shared_ptr<final_chain::FinalChain> final_chain,
                         std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_mgr)
    : db_(std::move(db)),
      pbft_manager_runtime_(std::move(pbft_manager_runtime)),
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
      proposed_blocks_(db_),
      eligible_wallets_(conf.wallets) {
  // Use first wallet as default node_addr
  const auto &node_addr = dev::toAddress(conf.getFirstWallet().node_secret);
  LOG_OBJECTS_CREATE("PBFT_MGR");

  rustaxa::PbftManagerStartupReplayRangeFact startup_replay_fact;
  startup_replay_fact.final_chain_last_block = final_chain_->lastBlockNumber();
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
    for (auto period = startup_replay_plan.finalization_from_period; period <= startup_replay_plan.finalization_to_period;
         ++period) {
      const auto replay_period = rustaxa::pbft_manager_runtime_load_startup_replay_period(
          *pbft_manager_runtime_.value(), period, kGenesisConfig.state.hardforks.isOnCactiHardfork(period));
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
      for (const auto &v : period_data.previous_block_cert_votes) {
        vote_mgr_->validateVote(v);
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
        rustaxa::pbft_manager_runtime_load_startup_replay_period(*pbft_manager_runtime_.value(), period, false);
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
  eligible_wallets_.updateWalletsEligibility(pbft_chain_->getPbftChainSize(), final_chain_);

  // Note: processPillarBlock must be called after eligible_wallets_.updateWalletsEligibility
  auto current_pbft_period = pbft_chain_->getPbftChainSize();
  if (kGenesisConfig.state.hardforks.ficus_hf.isPillarBlockPeriod(current_pbft_period)) {
    const auto current_pillar_block = pillar_chain_mgr_->getCurrentPillarBlock();
    // There is a race condition where pbt block could have been saved and node stopped before saving pillar block
    if (current_pbft_period ==
        current_pillar_block->getPeriod() + kGenesisConfig.state.hardforks.ficus_hf.pillar_blocks_interval)
      LOG(log_er_) << "Pillar block was not processed before restart, current period: " << current_pbft_period
                   << ", current pillar block period: " << current_pillar_block->getPeriod();
    processPillarBlock(current_pbft_period);
  }
}

PbftManager::~PbftManager() { stop(); }

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
    const auto runtime_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
    const auto period = getPbftPeriod();
    const auto round = static_cast<PbftRound>(runtime_snapshot.round);
    const auto step = static_cast<PbftStep>(runtime_snapshot.step);
    LOG(log_tr_) << "PBFT current period: " << period << ", round: " << round << ", step " << step;

    auto net = network_.lock();
    const auto &wallets = eligible_wallets_.getWallets(period);
    const bool has_eligible_wallet =
        std::any_of(wallets.cbegin(), wallets.cend(), [](const auto &wallet) { return wallet.first; });

    rustaxa::PbftManagerRuntimeTickFact fact{};
    fact.tick_id = ++rust_runtime_tick_id;
    fact.state = runtime_snapshot.state;
    fact.period = period;
    fact.round = round;
    fact.step = step;
    fact.network_available = static_cast<bool>(net);
    fact.network_pbft_syncing = net && net->pbft_syncing();
    fact.has_eligible_wallet = has_eligible_wallet;

    auto runtime_session = rustaxa::create_pbft_manager_runtime_session(fact);
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
      return runtime_session->pbft_manager_runtime_session_report(std::move(report));
    };
    auto apply_delay_transition = [&](uint8_t kind) {
      const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
      const auto current_round = static_cast<PbftRound>(transition_snapshot.round);
      const auto current_step = static_cast<PbftStep>(transition_snapshot.step);
      const auto current_period = getPbftPeriod();
      const auto plan = rustaxa::plan_pbft_manager_transition(makePbftManagerTransitionFact(
          kind, current_period, current_round, current_step, 0,
          std::chrono::milliseconds(transition_snapshot.current_round_lambda_ms),
          std::chrono::milliseconds(getRoundLambda(current_round)),
          std::chrono::milliseconds(kGenesisConfig.pbft.lambda_ms), kMaxExponentialLambda, getPbftDeadline(),
          std::chrono::milliseconds(transition_snapshot.next_step_time_ms), *vote_mgr_,
          kGenesisConfig.state.hardforks.isOnCactiHardfork(current_period), transition_snapshot.has_cert_voted_block,
          transition_snapshot.executed_pbft_block));
      if (!ensureTransitionPlanReady(plan, log_er_)) {
        return false;
      }
      applyPbftManagerTransitionPlan(
          plan, *pbft_manager_runtime_.value(), vote_mgr_, round_, step_, state_, current_round_lambda_,
          next_step_time_ms_, rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_,
          cert_voted_block_for_round_, current_round_broadcasted_votes_, broadcast_votes_counter_,
          rebroadcast_votes_counter_, broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_,
          already_next_voted_value_, already_next_voted_null_block_hash_, printCertStepInfo_, printSecondFinishStepInfo_,
          second_finish_step_start_datetime_);
      return true;
    };

    bool restart_loop = false;
    while (!stopped_) {
      auto step = runtime_session->pbft_manager_runtime_session_next();
      if (step.status == kPbftManagerRuntimeStatusComplete || step.complete) {
        restart_loop = step.restart_loop;
        break;
      }

      if (step.status != kPbftManagerRuntimeStatusActive || !step.has_action) {
        LOG(log_er_) << "Rust PBFT manager runtime rejected tick " << step.tick_id << ", status "
                     << static_cast<uint32_t>(step.status) << ", error " << static_cast<std::string>(step.error_code);
        runtime_session->abort_pbft_manager_runtime_session();
        assert(false);
        restart_loop = true;
        break;
      }

      const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
      const auto action_state = fromPbftManagerRuntimeState(action_snapshot.state);
      if (!pbftManagerRuntimeActionMatchesLiveState(step.action, action_state)) {
        LOG(log_dg_) << "Rust PBFT manager runtime action " << static_cast<uint32_t>(step.action)
                     << " no longer matches Rust PBFT state " << static_cast<uint32_t>(action_state)
                     << "; restarting daemon loop";
        runtime_session->abort_pbft_manager_runtime_session();
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
          const auto new_round = vote_mgr_->determineNewRound(current_period, current_round);
          step = report_action(step, kPbftManagerRuntimeResultNoProgressContinue, true, "", new_round.has_value(),
                               new_round.value_or(0));
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
                       << step_;
          step = report_action(step, kPbftManagerRuntimeResultTransitionApplied);
          break;
        case kPbftManagerRuntimeActionSleepIneligiblePollingInterval:
          std::this_thread::sleep_for(std::chrono::milliseconds(kPollingIntervalMs));
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
                       << rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value()).next_step_time_ms
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
        runtime_session->abort_pbft_manager_runtime_session();
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
  if (!pbft_manager_runtime_.has_value()) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before reading DAG block period");
  }
  const auto lookup =
      rustaxa::pbft_manager_runtime_dag_block_period(*pbft_manager_runtime_.value(), toBridgeHash(hash));
  if (!lookup.found) {
    return {false, PbftPeriod{0}};
  }
  return {true, static_cast<PbftPeriod>(lookup.period)};
}

PbftPeriod PbftManager::getPbftPeriod() const { return pbft_chain_->getPbftChainSize() + 1; }

PbftRound PbftManager::getPbftRound() const {
  if (pbft_manager_runtime_.has_value()) {
    const auto snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
    if (snapshot.status != kPbftManagerStartupRestoreStatusReady) {
      throw std::runtime_error("PBFT manager Rust runtime snapshot is not ready: " +
                               static_cast<std::string>(snapshot.error_code));
    }
    return static_cast<PbftRound>(snapshot.round);
  }
  return round_;
}

std::pair<PbftRound, PbftPeriod> PbftManager::getPbftRoundAndPeriod() const {
  return {getPbftRound(), getPbftPeriod()};
}

PbftStep PbftManager::getPbftStep() const {
  if (pbft_manager_runtime_.has_value()) {
    const auto snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
    if (snapshot.status != kPbftManagerStartupRestoreStatusReady) {
      throw std::runtime_error("PBFT manager Rust runtime snapshot is not ready: " +
                               static_cast<std::string>(snapshot.error_code));
    }
    return static_cast<PbftStep>(snapshot.step);
  }
  return step_;
}

void PbftManager::setPbftRound(PbftRound round) {
  if (!pbft_manager_runtime_.has_value()) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before persisting round");
  }
  const auto snapshot = rustaxa::pbft_manager_runtime_apply_cursor_field(
      *pbft_manager_runtime_.value(), static_cast<uint8_t>(PbftMgrField::Round), static_cast<uint32_t>(round));
  round_ = static_cast<PbftRound>(snapshot.round);
}

void PbftManager::waitForPeriodFinalization() {
  do {
    // we need to be sure we finalized at least block with num lower by delegation_delay
    if (pbft_chain_->getPbftChainSize() <= final_chain_->lastBlockNumber() + final_chain_->delegationDelay()) {
      break;
    }
    thisThreadSleepForMilliSeconds(kPollingIntervalMs.count());
  } while (!stopped_);
}

std::optional<uint64_t> PbftManager::getCurrentDposTotalVotesCount() const {
  try {
    const auto period = pbft_chain_->getPbftChainSize();
    const auto facts = final_chain_->rustFinalChainForRust().collect_pbft_final_chain_facts(
        makePbftFinalChainFactRequest(period, kNullBlockHash, false, false, true, false));
    if (facts.has_total_vote_count && facts.total_vote_count_status == kPbftSyncFactValid) {
      return facts.total_vote_count;
    }
    LOG(log_wr_) << "Unable to get CurrentDposTotalVotesCount for period: " << pbft_chain_->getPbftChainSize()
                 << ". Period is too far ahead of actual finalized pbft chain size (" << facts.last_block_number
                 << "). Err msg: " << static_cast<std::string>(facts.error_code);
  } catch (const std::exception &e) {
    LOG(log_wr_) << "Rust FinalChain PBFT total-vote fact collection failed for period "
                 << pbft_chain_->getPbftChainSize() << ". Err msg: " << e.what();
  }

  return {};
}

std::optional<uint64_t> PbftManager::getCurrentNodeVotesCount() const {
  // Note: There is a race condition in eligible_wallets_.getWalletsEligiblePeriod(). This method works only if
  // wallets eligible period == pbft chain size. This race condition is handled within pbft manager but
  // getCurrentNodeVotesCount() is called externally from standalone thread and in some edge cases we need to wait until
  // period in eligible_wallets_ is updated according to the latest chain size
  while (true) {
    if (eligible_wallets_.getWalletsEligiblePeriod() == pbft_chain_->getPbftChainSize()) {
      break;
    }

    thisThreadSleepForMilliSeconds(10);
  }

  std::vector<addr_t> eligible_addresses;
  for (const auto &wallet : eligible_wallets_.getWallets(getPbftPeriod())) {
    // Wallet is not dpos eligible - do no vote
    if (!wallet.first) {
      continue;
    }
    eligible_addresses.emplace_back(wallet.second.node_addr);
  }

  try {
    const auto period = pbft_chain_->getPbftChainSize();
    const auto facts =
        final_chain_->rustFinalChainForRust().collect_pbft_final_chain_facts(makePbftFinalChainFactRequest(
            period, kNullBlockHash, false, false, false, true, std::move(eligible_addresses)));
    uint64_t node_votes_count = 0;
    for (const auto &address_fact : facts.address_facts) {
      if (address_fact.status != kPbftSyncFactValid) {
        LOG(log_wr_) << "Unable to get CurrentNodeVotesCount for period: " << period
                     << ". Period is too far ahead of actual finalized pbft chain size (" << facts.last_block_number
                     << "). Err msg: " << static_cast<std::string>(address_fact.error_code);
        return {};
      }
      node_votes_count += address_fact.vote_count;
    }
    return node_votes_count;
  } catch (const std::exception &e) {
    LOG(log_wr_) << "Rust FinalChain PBFT node-vote fact collection failed for period "
                 << pbft_chain_->getPbftChainSize() << ". Err msg: " << e.what();
  }

  return {};
}

void PbftManager::setPbftStep(PbftStep pbft_step) {
  if (!pbft_manager_runtime_.has_value()) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before persisting step");
  }
  const auto snapshot = rustaxa::pbft_manager_runtime_apply_cursor_field(
      *pbft_manager_runtime_.value(), static_cast<uint8_t>(PbftMgrField::Step), static_cast<uint32_t>(pbft_step));
  step_ = static_cast<PbftStep>(snapshot.step);
}

bool PbftManager::tryPushCertVotesBlock() {
  const auto [current_pbft_round, current_pbft_period] = getPbftRoundAndPeriod();

  auto cert_votes = vote_mgr_->getTwoTPlusOneVotedBlockVotes(current_pbft_period, current_pbft_round,
                                                             TwoTPlusOneVotedBlockType::CertVotedBlock);
  if (cert_votes.empty()) {
    return false;
  }
  const blk_hash_t &certified_block_hash = cert_votes[0]->getBlockHash();

  LOG(log_nf_) << "Found enough cert votes for PBFT block " << certified_block_hash << ", period "
               << current_pbft_period << ", round " << current_pbft_round;

  auto pbft_block = getValidPbftProposedBlock(proposed_blocks_, current_pbft_period, certified_block_hash);
  if (!pbft_block) {
    LOG(log_er_) << "Invalid certified block " << certified_block_hash;
    return false;
  }

  // Push pbft block into chain
  if (!pushCertVotedPbftBlockIntoChain_(pbft_block, std::move(cert_votes))) {
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
  const auto new_period = chain_size + 1;
  const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto transition_plan = rustaxa::plan_pbft_manager_transition(makePbftManagerTransitionFact(
      kPbftManagerTransitionResetConsensus, new_period, static_cast<PbftRound>(transition_snapshot.round),
      static_cast<PbftStep>(transition_snapshot.step), 1 /* round */,
      std::chrono::milliseconds(transition_snapshot.current_round_lambda_ms),
      std::chrono::milliseconds(getRoundLambda(1 /* round */)),
      std::chrono::milliseconds(kGenesisConfig.pbft.lambda_ms), kMaxExponentialLambda, getPbftDeadline(),
      std::chrono::milliseconds(transition_snapshot.next_step_time_ms), *vote_mgr_,
      kGenesisConfig.state.hardforks.isOnCactiHardfork(new_period), transition_snapshot.has_cert_voted_block,
      transition_snapshot.executed_pbft_block));
  if (!ensureTransitionPlanReady(transition_plan, log_er_)) {
    return false;
  }
  return applyRustPlannedAdvancePeriod_(chain_size, transition_plan);
}

bool PbftManager::applyRustPlannedAdvancePeriod_(PbftPeriod finalized_chain_size,
                                                 const rustaxa::PbftManagerTransitionPlan& transition_plan) {
  const auto advance_plan = rustaxa::plan_pbft_manager_advance_period(finalized_chain_size, transition_plan);
  if (!advance_plan.accepted) {
    LOG(log_er_) << "Rust PBFT manager advance-period planner rejected facts, chain size " << finalized_chain_size
                 << ", error " << static_cast<std::string>(advance_plan.error_code);
    return false;
  }

  uint64_t action_index = 0;
  for (const auto action : advance_plan.actions) {
    switch (action) {
      case kPbftManagerAdvancePeriodActionApplyResetConsensusTransition:
        printVotingSummary();
        applyPbftManagerTransitionPlan(transition_plan, *pbft_manager_runtime_.value(), vote_mgr_, round_, step_, state_,
                                       current_round_lambda_, next_step_time_ms_, rounds_count_dynamic_lambda_,
                                       dynamic_lambda_, executed_pbft_block_, cert_voted_block_for_round_,
                                       current_round_broadcasted_votes_, broadcast_votes_counter_,
                                       rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                       rebroadcast_reward_votes_counter_, already_next_voted_value_,
                                       already_next_voted_null_block_hash_, printCertStepInfo_,
                                       printSecondFinishStepInfo_, second_finish_step_start_datetime_);
        break;
      case kPbftManagerAdvancePeriodActionApplyExecutedBlockReset: {
        waitForPeriodFinalization();
        const auto reset_result =
            rustaxa::pbft_manager_runtime_apply_executed_block_reset(*pbft_manager_runtime_.value());
        if (reset_result.status != kPbftManagerTransitionStorageStatusApplied) {
          throw std::runtime_error("Rust PBFT manager executed-block reset failed: " +
                                   static_cast<std::string>(reset_result.error_code));
        }
        applyPbftManagerRuntimeSnapshot(reset_result.snapshot, round_, step_, state_, current_round_lambda_,
                                        next_step_time_ms_, rounds_count_dynamic_lambda_, dynamic_lambda_,
                                        executed_pbft_block_, already_next_voted_value_,
                                        already_next_voted_null_block_hash_, broadcast_votes_counter_,
                                        rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                        rebroadcast_reward_votes_counter_);
        break;
      }
      case kPbftManagerAdvancePeriodActionSetVoteManagerPeriodRound:
        vote_mgr_->setCurrentPbftPeriodAndRound(advance_plan.new_period, transition_plan.new_round);
        break;
      case kPbftManagerAdvancePeriodActionResetCurrentRoundTimer:
        current_round_start_datetime_ = std::chrono::system_clock::now();
        break;
      case kPbftManagerAdvancePeriodActionResetRewardVoteCounters:
      {
        const auto broadcast_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
        const auto reset_snapshot = rustaxa::pbft_manager_runtime_apply_broadcast_counters(
            *pbft_manager_runtime_.value(), broadcast_snapshot.broadcast_votes_counter,
            broadcast_snapshot.rebroadcast_votes_counter, 1, 1);
        if (reset_snapshot.status != kPbftManagerStartupRestoreStatusReady) {
          LOG(log_er_) << "Rust PBFT manager reward-vote counter reset rejected, error "
                       << static_cast<std::string>(reset_snapshot.error_code);
          return false;
        }
        applyPbftManagerRuntimeSnapshot(reset_snapshot, round_, step_, state_, current_round_lambda_,
                                        next_step_time_ms_, rounds_count_dynamic_lambda_, dynamic_lambda_,
                                        executed_pbft_block_, already_next_voted_value_,
                                        already_next_voted_null_block_hash_, broadcast_votes_counter_,
                                        rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                        rebroadcast_reward_votes_counter_);
        break;
      }
      case kPbftManagerAdvancePeriodActionResetPeriodTimer:
        current_period_start_datetime_ = std::chrono::system_clock::now();
        break;
      case kPbftManagerAdvancePeriodActionUpdateWalletEligibility:
        eligible_wallets_.updateWalletsEligibility(advance_plan.finalized_chain_size, final_chain_);
        break;
      case kPbftManagerAdvancePeriodActionCleanupVotes:
        // !!!Important: we need previous period votes to get reward votes for current period block
        vote_mgr_->cleanupVotesByPeriod(advance_plan.finalized_chain_size);
        break;
      case kPbftManagerAdvancePeriodActionCleanupProposedBlocks:
        proposed_blocks_.cleanupProposedPbftBlocksByPeriod(advance_plan.new_period);
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
      LOG(log_er_) << "Rust PBFT manager advance-period action report rejected at index " << action_index
                   << ", action " << static_cast<uint32_t>(action) << ", status "
                   << static_cast<uint32_t>(action_validation.status) << ", error "
                   << static_cast<std::string>(action_validation.error_code);
      return false;
    }
    ++action_index;
  }

  const auto period_snapshot =
      rustaxa::pbft_manager_runtime_apply_period_advance(*pbft_manager_runtime_.value(), advance_plan.new_period);
  if (period_snapshot.status != kPbftManagerStartupRestoreStatusReady) {
    LOG(log_er_) << "Rust PBFT manager period-advance runtime rejected new period " << advance_plan.new_period
                 << ", error " << static_cast<std::string>(period_snapshot.error_code);
    return false;
  }
  applyPbftManagerRuntimeSnapshot(period_snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
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

  const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto period = getPbftPeriod();
  const auto current_round = static_cast<PbftRound>(transition_snapshot.round);
  const auto current_step = static_cast<PbftStep>(transition_snapshot.step);
  const auto plan = rustaxa::plan_pbft_manager_transition(makePbftManagerTransitionFact(
      kPbftManagerTransitionResetConsensus, period, current_round, current_step, round,
      std::chrono::milliseconds(transition_snapshot.current_round_lambda_ms),
      std::chrono::milliseconds(getRoundLambda(round)), std::chrono::milliseconds(kGenesisConfig.pbft.lambda_ms),
      kMaxExponentialLambda, getPbftDeadline(), std::chrono::milliseconds(transition_snapshot.next_step_time_ms),
      *vote_mgr_,
      kGenesisConfig.state.hardforks.isOnCactiHardfork(period), transition_snapshot.has_cert_voted_block,
      transition_snapshot.executed_pbft_block));
  if (!ensureTransitionPlanReady(plan, log_er_)) {
    return;
  }

  applyPbftManagerTransitionPlan(plan, *pbft_manager_runtime_.value(), vote_mgr_, round_, step_, state_,
                                 current_round_lambda_, next_step_time_ms_, rounds_count_dynamic_lambda_,
                                 dynamic_lambda_, executed_pbft_block_, cert_voted_block_for_round_,
                                 current_round_broadcasted_votes_, broadcast_votes_counter_, rebroadcast_votes_counter_,
                                 broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_,
                                 already_next_voted_value_, already_next_voted_null_block_hash_, printCertStepInfo_,
                                 printSecondFinishStepInfo_, second_finish_step_start_datetime_);

  if (plan.reset_executed_block_status) {
    waitForPeriodFinalization();
    const auto reset_result = rustaxa::pbft_manager_runtime_apply_executed_block_reset(*pbft_manager_runtime_.value());
    if (reset_result.status != kPbftManagerTransitionStorageStatusApplied) {
      throw std::runtime_error("Rust PBFT manager executed-block reset failed: " +
                               static_cast<std::string>(reset_result.error_code));
    }
    applyPbftManagerRuntimeSnapshot(reset_result.snapshot, round_, step_, state_, current_round_lambda_,
                                    next_step_time_ms_, rounds_count_dynamic_lambda_, dynamic_lambda_,
                                    executed_pbft_block_, already_next_voted_value_,
                                    already_next_voted_null_block_hash_, broadcast_votes_counter_,
                                    rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                    rebroadcast_reward_votes_counter_);
  }
  if (plan.set_vote_manager_period_round) {
    vote_mgr_->setCurrentPbftPeriodAndRound(period, plan.new_round);
  }
  if (plan.reset_current_round_start) {
    current_round_start_datetime_ = std::chrono::system_clock::now();
  }

  LOG(log_nf_) << "Reset PBFT consensus to: period " << period << ", round " << plan.new_round << ", step "
               << plan.new_step << ", lambda " << current_round_lambda_ << " [ms]";
}

uint32_t PbftManager::getRoundLambda(PbftRound round) const {
  if (round == 1) {
    if (!pbft_manager_runtime_.has_value()) {
      throw std::runtime_error("PBFT manager Rust runtime must be initialized before reading round lambda");
    }
    const auto runtime_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
    if (runtime_snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
      throw std::runtime_error("Rust PBFT manager snapshot rejected while reading round lambda: " +
                               static_cast<std::string>(runtime_snapshot.error_code));
    }
    return runtime_snapshot.dynamic_lambda_ms;
  }

  // otherwise use default lambda
  return kGenesisConfig.state.hardforks.cacti_hf.lambda_default;
}

std::chrono::milliseconds PbftManager::elapsedTimeInMs(const time_point &start_time) {
  return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::system_clock::now() - start_time);
}

void PbftManager::sleep_() {
  // Run "wait_for" sleep in loop due to potential spurious wakeup on lock
  while (!stopped_) {
    auto next_step_time_ms = next_step_time_ms_;
    auto step = step_;
    if (pbft_manager_runtime_.has_value()) {
      const auto snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
      if (snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
        throw std::runtime_error("PBFT manager Rust runtime snapshot is not ready: " +
                                 static_cast<std::string>(snapshot.error_code));
      }
      next_step_time_ms = std::chrono::milliseconds(snapshot.next_step_time_ms);
      step = static_cast<PbftStep>(snapshot.step);
    }

    const auto round_elapsed_time = elapsedTimeInMs(current_round_start_datetime_);
    if (next_step_time_ms <= round_elapsed_time) {
      return;
    }

    const auto time_to_sleep_for_ms = next_step_time_ms - round_elapsed_time;
    const auto [round, period] = getPbftRoundAndPeriod();
    LOG(log_tr_) << "Sleep " << time_to_sleep_for_ms.count() << " [ms] before going into the next step. Period "
                 << period << ", round " << round << ", step " << step;
    std::unique_lock<std::mutex> lock(stop_mtx_);
    stop_cv_.wait_for(lock, time_to_sleep_for_ms);
  }
}

void PbftManager::initialState() {
  // Initial PBFT state

  // Time constants...
  const auto current_pbft_period = getPbftPeriod();
  const auto now = std::chrono::system_clock::now();

  if (!pbft_manager_runtime_.has_value()) {
    LOG(log_er_) << "Rust PBFT manager runtime was not provided before initialState";
    assert(false);
  }
  const auto startup_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  applyPbftManagerRuntimeSnapshot(startup_snapshot, round_, step_, state_, current_round_lambda_, next_step_time_ms_,
                                  rounds_count_dynamic_lambda_, dynamic_lambda_, executed_pbft_block_,
                                  already_next_voted_value_, already_next_voted_null_block_hash_,
                                  broadcast_votes_counter_, rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                  rebroadcast_reward_votes_counter_);
  if (startup_snapshot.reset_second_finish_start) {
    second_finish_step_start_datetime_ = now;
  }
  const auto current_pbft_round = round_.load();
  const auto current_pbft_step = step_;

  // Load proposed-block startup metadata through the Rust-owned proposed-block
  // index. This preserves canonical block bytes for later network/public
  // materialization without scanning `DbStorage` into live C++ `PbftBlock`
  // objects during PBFT manager startup.
  proposed_blocks_.restoreFromStorage();

  // TODO[2840]: remove this check if case nodes do not log the err messages after restart
  //  if (const auto &err_msg = proposed_blocks_.checkOldBlocksPresence(current_pbft_period); err_msg.has_value()) {
  //    LOG(log_er_) << "Old proposed blocks saved in db <period> -> <blocks count>: " << *err_msg;
  //  }

  // Process saved cert voted block from Rust storage through the PBFT runtime.
  const auto cert_voted_block_payload =
      rustaxa::pbft_manager_runtime_cert_voted_block_in_round(*pbft_manager_runtime_.value());
  if (!cert_voted_block_payload.empty()) {
    const auto payload_bytes = dev::bytes(cert_voted_block_payload.begin(), cert_voted_block_payload.end());
    const auto payload_rlp = dev::RLP(payload_bytes);
    assert(payload_rlp.itemCount() == 2);
    const auto cert_voted_block_round = payload_rlp[0].toInt<PbftRound>();
    const auto cert_voted_block = std::make_shared<PbftBlock>(payload_rlp[1]);
    if (proposed_blocks_.pushProposedPbftBlock(cert_voted_block)) {
      LOG(log_nf_) << "Last cert voted block " << cert_voted_block->getBlockHash() << " with period "
                   << cert_voted_block->getPeriod() << ", round " << cert_voted_block_round
                   << " pushed into proposed blocks";
    }

    // Set cert_voted_block_for_round_ only if round and period match. Note: could differ in edge case when node
    // crashed, new period/round was already saved in db but cert voted block was not cleared yet
    if (current_pbft_period == cert_voted_block->getPeriod() && current_pbft_round == cert_voted_block_round) {
      const auto cert_voted_snapshot = rustaxa::pbft_manager_runtime_apply_cert_voted_block_metadata(
          *pbft_manager_runtime_.value(), cert_voted_block->getPeriod(), cert_voted_block_round,
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

  // Set current period & round in vote manager
  vote_mgr_->setCurrentPbftPeriodAndRound(current_pbft_period, current_pbft_round);

  waitForPeriodFinalization();

  const auto previous_round_next_voted_block = vote_mgr_->getTwoTPlusOneVotedBlock(
      current_pbft_period, current_pbft_round - 1, TwoTPlusOneVotedBlockType::NextVotedBlock);
  const auto previous_round_next_voted_null_block = vote_mgr_->getTwoTPlusOneVotedBlock(
      current_pbft_period, current_pbft_round - 1, TwoTPlusOneVotedBlockType::NextVotedNullBlock);

  LOG(log_nf_) << "Node initialize at period " << current_pbft_period << ", round " << current_pbft_round << ", step "
               << current_pbft_step << ". Previous round 2t+1 next voted null block: " << std::boolalpha
               << previous_round_next_voted_null_block.has_value() << ", previous round 2t+1 next voted block "
               << (previous_round_next_voted_block.has_value() ? previous_round_next_voted_block->abridged()
                                                               : "no value");
}

void PbftManager::setFilterState_() {
  const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto round = static_cast<PbftRound>(transition_snapshot.round);
  const auto step = static_cast<PbftStep>(transition_snapshot.step);
  const auto period = getPbftPeriod();
  const auto plan = rustaxa::plan_pbft_manager_transition(makePbftManagerTransitionFact(
      kPbftManagerTransitionToFilter, period, round, step, 0,
      std::chrono::milliseconds(transition_snapshot.current_round_lambda_ms),
      std::chrono::milliseconds(getRoundLambda(round)), std::chrono::milliseconds(kGenesisConfig.pbft.lambda_ms),
      kMaxExponentialLambda, getPbftDeadline(), std::chrono::milliseconds(transition_snapshot.next_step_time_ms),
      *vote_mgr_,
      kGenesisConfig.state.hardforks.isOnCactiHardfork(period), transition_snapshot.has_cert_voted_block,
      transition_snapshot.executed_pbft_block));
  if (!ensureTransitionPlanReady(plan, log_er_)) {
    return;
  }
  applyPbftManagerTransitionPlan(plan, *pbft_manager_runtime_.value(), vote_mgr_, round_, step_, state_,
                                 current_round_lambda_, next_step_time_ms_, rounds_count_dynamic_lambda_,
                                 dynamic_lambda_, executed_pbft_block_, cert_voted_block_for_round_,
                                 current_round_broadcasted_votes_, broadcast_votes_counter_, rebroadcast_votes_counter_,
                                 broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_,
                                 already_next_voted_value_, already_next_voted_null_block_hash_, printCertStepInfo_,
                                 printSecondFinishStepInfo_, second_finish_step_start_datetime_);
}

void PbftManager::setCertifyState_() {
  const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto round = static_cast<PbftRound>(transition_snapshot.round);
  const auto step = static_cast<PbftStep>(transition_snapshot.step);
  const auto period = getPbftPeriod();
  const auto plan = rustaxa::plan_pbft_manager_transition(makePbftManagerTransitionFact(
      kPbftManagerTransitionToCertify, period, round, step, 0,
      std::chrono::milliseconds(transition_snapshot.current_round_lambda_ms),
      std::chrono::milliseconds(getRoundLambda(round)), std::chrono::milliseconds(kGenesisConfig.pbft.lambda_ms),
      kMaxExponentialLambda, getPbftDeadline(), std::chrono::milliseconds(transition_snapshot.next_step_time_ms),
      *vote_mgr_,
      kGenesisConfig.state.hardforks.isOnCactiHardfork(period), transition_snapshot.has_cert_voted_block,
      transition_snapshot.executed_pbft_block));
  if (!ensureTransitionPlanReady(plan, log_er_)) {
    return;
  }
  applyPbftManagerTransitionPlan(plan, *pbft_manager_runtime_.value(), vote_mgr_, round_, step_, state_,
                                 current_round_lambda_, next_step_time_ms_, rounds_count_dynamic_lambda_,
                                 dynamic_lambda_, executed_pbft_block_, cert_voted_block_for_round_,
                                 current_round_broadcasted_votes_, broadcast_votes_counter_, rebroadcast_votes_counter_,
                                 broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_,
                                 already_next_voted_value_, already_next_voted_null_block_hash_, printCertStepInfo_,
                                 printSecondFinishStepInfo_, second_finish_step_start_datetime_);
}

void PbftManager::setFinishState_() {
  LOG(log_dg_) << "Will go to first finish State";
  const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto round = static_cast<PbftRound>(transition_snapshot.round);
  const auto step = static_cast<PbftStep>(transition_snapshot.step);
  const auto period = getPbftPeriod();
  const auto plan = rustaxa::plan_pbft_manager_transition(makePbftManagerTransitionFact(
      kPbftManagerTransitionToFinish, period, round, step, 0,
      std::chrono::milliseconds(transition_snapshot.current_round_lambda_ms),
      std::chrono::milliseconds(getRoundLambda(round)), std::chrono::milliseconds(kGenesisConfig.pbft.lambda_ms),
      kMaxExponentialLambda, getPbftDeadline(), std::chrono::milliseconds(transition_snapshot.next_step_time_ms),
      *vote_mgr_,
      kGenesisConfig.state.hardforks.isOnCactiHardfork(period), transition_snapshot.has_cert_voted_block,
      transition_snapshot.executed_pbft_block));
  if (!ensureTransitionPlanReady(plan, log_er_)) {
    return;
  }
  applyPbftManagerTransitionPlan(plan, *pbft_manager_runtime_.value(), vote_mgr_, round_, step_, state_,
                                 current_round_lambda_, next_step_time_ms_, rounds_count_dynamic_lambda_,
                                 dynamic_lambda_, executed_pbft_block_, cert_voted_block_for_round_,
                                 current_round_broadcasted_votes_, broadcast_votes_counter_, rebroadcast_votes_counter_,
                                 broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_,
                                 already_next_voted_value_, already_next_voted_null_block_hash_, printCertStepInfo_,
                                 printSecondFinishStepInfo_, second_finish_step_start_datetime_);
}

void PbftManager::setFinishPollingState_() {
  const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto round = static_cast<PbftRound>(transition_snapshot.round);
  const auto step = static_cast<PbftStep>(transition_snapshot.step);
  const auto period = getPbftPeriod();
  const auto plan = rustaxa::plan_pbft_manager_transition(makePbftManagerTransitionFact(
      kPbftManagerTransitionToFinishPolling, period, round, step, 0,
      std::chrono::milliseconds(transition_snapshot.current_round_lambda_ms),
      std::chrono::milliseconds(getRoundLambda(round)), std::chrono::milliseconds(kGenesisConfig.pbft.lambda_ms),
      kMaxExponentialLambda, getPbftDeadline(), std::chrono::milliseconds(transition_snapshot.next_step_time_ms),
      *vote_mgr_,
      kGenesisConfig.state.hardforks.isOnCactiHardfork(period), transition_snapshot.has_cert_voted_block,
      transition_snapshot.executed_pbft_block));
  if (!ensureTransitionPlanReady(plan, log_er_)) {
    return;
  }
  applyPbftManagerTransitionPlan(plan, *pbft_manager_runtime_.value(), vote_mgr_, round_, step_, state_,
                                 current_round_lambda_, next_step_time_ms_, rounds_count_dynamic_lambda_,
                                 dynamic_lambda_, executed_pbft_block_, cert_voted_block_for_round_,
                                 current_round_broadcasted_votes_, broadcast_votes_counter_, rebroadcast_votes_counter_,
                                 broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_,
                                 already_next_voted_value_, already_next_voted_null_block_hash_, printCertStepInfo_,
                                 printSecondFinishStepInfo_, second_finish_step_start_datetime_);
}

void PbftManager::loopBackFinishState_() {
  const auto transition_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto round = static_cast<PbftRound>(transition_snapshot.round);
  const auto step = static_cast<PbftStep>(transition_snapshot.step);
  const auto period = getPbftPeriod();
  const auto plan = rustaxa::plan_pbft_manager_transition(makePbftManagerTransitionFact(
      kPbftManagerTransitionLoopBackFinish, period, round, step, 0,
      std::chrono::milliseconds(transition_snapshot.current_round_lambda_ms),
      std::chrono::milliseconds(getRoundLambda(round)), std::chrono::milliseconds(kGenesisConfig.pbft.lambda_ms),
      kMaxExponentialLambda, getPbftDeadline(), std::chrono::milliseconds(transition_snapshot.next_step_time_ms),
      *vote_mgr_,
      kGenesisConfig.state.hardforks.isOnCactiHardfork(period), transition_snapshot.has_cert_voted_block,
      transition_snapshot.executed_pbft_block));
  if (!ensureTransitionPlanReady(plan, log_er_)) {
    return;
  }
  applyPbftManagerTransitionPlan(plan, *pbft_manager_runtime_.value(), vote_mgr_, round_, step_, state_,
                                 current_round_lambda_, next_step_time_ms_, rounds_count_dynamic_lambda_,
                                 dynamic_lambda_, executed_pbft_block_, cert_voted_block_for_round_,
                                 current_round_broadcasted_votes_, broadcast_votes_counter_, rebroadcast_votes_counter_,
                                 broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_,
                                 already_next_voted_value_, already_next_voted_null_block_hash_, printCertStepInfo_,
                                 printSecondFinishStepInfo_, second_finish_step_start_datetime_);
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
  auto stuckPeriodBroadcastVotes = [this, &net, &gossipVotes](bool rebroadcast) {
    auto [round, period] = getPbftRoundAndPeriod();

    gossipVotes(vote_mgr_->getRewardVotes(), "Reward votes", rebroadcast);

    // Broadcast own pbft votes - send votes by one as they have different type, period, round, step
    if (const auto &own_votes = vote_mgr_->getOwnVerifiedVotes(); !own_votes.empty()) {
      for (const auto &vote : own_votes) {
        net->gossipVote(vote, getPbftProposedBlock(vote->getPeriod(), vote->getBlockHash()), rebroadcast);
      }

      LOG(log_dg_) << "Broadcast own votes for period " << period << ", round " << round << ", rebroadcast "
                   << rebroadcast;
    }

    // Broadcast own pillar vote
    const auto own_pillar_vote_rlp = rustaxa::pbft_manager_runtime_own_pillar_block_vote(*pbft_manager_runtime_.value());
    if (!own_pillar_vote_rlp.empty()) {
      const auto payload_bytes = dev::bytes(own_pillar_vote_rlp.begin(), own_pillar_vote_rlp.end());
      const auto own_pillar_vote = std::make_shared<PillarVote>(dev::RLP(payload_bytes));
      net->gossipPillarBlockVote(own_pillar_vote, rebroadcast);
    }
  };

  // (Re)broadcast 2t+1 soft/reward/previous round next votes + all own votes
  auto stuckRoundBroadcastVotes = [this, &gossipVotes, &stuckPeriodBroadcastVotes](bool rebroadcast) {
    auto [round, period] = getPbftRoundAndPeriod();

    stuckPeriodBroadcastVotes(rebroadcast);

    // Broadcast 2t+1 soft votes
    gossipVotes(vote_mgr_->getTwoTPlusOneVotedBlockVotes(period, round, TwoTPlusOneVotedBlockType::SoftVotedBlock),
                "2t+1 soft votes", rebroadcast);

    // Broadcast previous round 2t+1 next votes
    if (round > 1) {
      gossipVotes(
          vote_mgr_->getTwoTPlusOneVotedBlockVotes(period, round - 1, TwoTPlusOneVotedBlockType::NextVotedBlock),
          "2t+1 next votes", rebroadcast);
      gossipVotes(
          vote_mgr_->getTwoTPlusOneVotedBlockVotes(period, round - 1, TwoTPlusOneVotedBlockType::NextVotedNullBlock),
          "2t+1 next null votes", rebroadcast);
    }
  };

  const auto round_elapsed_time = elapsedTimeInMs(current_round_start_datetime_);
  const auto period_elapsed_time = elapsedTimeInMs(current_period_start_datetime_);
  const auto broadcast_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
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
      stuckPeriodBroadcastVotes(plan.rebroadcast);
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
        *pbft_manager_runtime_.value(), result.broadcast_votes_counter, result.rebroadcast_votes_counter,
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
      *pbft_manager_runtime_.value(), kBroadcastVotesLambdaTime, kRebroadcastVotesLambdaTime, kBroadcastVotesLambdaTime,
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

std::shared_ptr<PbftBlock> PbftManager::getValidPbftProposedBlock(ProposedBlocks &proposed_blocks, PbftPeriod period,
                                                                  const blk_hash_t &block_hash) {
  rustaxa::PbftManagerCandidateAdmissionFact fact;
  fact.period = period;
  fact.block_hash = toBridgeHash(block_hash);
  fact.lookup_performed = false;
  fact.proposed_block_found = false;
  fact.proposed_block_already_valid = false;
  fact.validation_status = kPbftManagerCandidateAdmissionValidationNotChecked;

  std::shared_ptr<PbftBlock> block;
  while (true) {
    const auto plan = rustaxa::plan_pbft_manager_candidate_admission(fact);
    if (plan.action == kPbftManagerCandidateAdmissionActionAccept) {
      if (!block) {
        // Rust admission decisions use compact proposed-block metadata. C++ materializes the accepted block only at this
        // vote-generation/executor boundary.
        const auto block_data = proposed_blocks.getPbftProposedBlock(period, block_hash);
        if (!block_data.has_value()) {
          throw std::runtime_error("Rust PBFT proposed-block admission accepted missing materialized block");
        }
        block = block_data->first;
      }
      if (plan.mark_valid) {
        proposed_blocks.markBlockAsValid(period, block_hash);
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
      const auto block_metadata = proposed_blocks.getPbftProposedBlockMetadata(period, block_hash);
      fact.lookup_performed = true;
      if (!block_metadata.has_value()) {
        LOG(log_er_) << "Unable to find proposed block " << block_hash << ", period " << period;
        fact.proposed_block_found = false;
        continue;
      }

      fact.proposed_block_found = true;
      fact.proposed_block_already_valid = block_metadata->is_valid;
      continue;
    }

    if (plan.action == kPbftManagerCandidateAdmissionActionRequestValidation) {
      if (!block) {
        const auto block_data = proposed_blocks.getPbftProposedBlock(period, block_hash);
        if (!block_data.has_value()) {
          LOG(log_er_) << "Unable to materialize proposed block " << block_hash << " for validation, period " << period;
          fact.validation_status = kPbftManagerCandidateAdmissionValidationInvalid;
          continue;
        }
        block = block_data->first;
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

std::shared_ptr<PbftBlock> PbftManager::admitStateActionPbftBlock(
    const rustaxa::PbftManagerStateActionEffect &effect, std::string_view action_context) {
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

  auto block = getValidPbftProposedBlock(proposed_blocks_, period, block_hash);
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
      const auto current_pillar_block = pillar_chain_mgr_->getCurrentPillarBlock();
      // Check if the latest pillar block was created
      if (current_pillar_block && current_pillar_block->getPeriod() == period - 1) {
        place_pillar_vote_for_block = current_pillar_block->getHash();
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

    const auto vote = vote_mgr_->generateVoteWithWeight(block_hash, vote_type, period, round, step, wallet.second);
    if (!vote) {
      LOG(log_dg_) << "Failed to generate vote for " << block_hash << ", period " << period << ", round " << round
                   << ", step " << step << ", validator " << wallet.second.node_addr;
      continue;
    }

    if (!vote_mgr_->addVerifiedVote(vote)) {
      LOG(log_er_) << "Unable to place vote " << vote->getHash() << " for block " << block_hash << ", period " << period
                   << ", round " << round << ", step " << step << ", validator " << wallet.second.node_addr;
      continue;
    }

    // Propose votes are sent as single packets so it is gossiped together with pbft block
    if (vote_type == PbftVoteTypes::propose_vote) {
      gossipNewOwnVote(vote, pbft_block);

      LOG(log_nf_) << "Placed and sent " << vote->getHash() << " vote for block " << block_hash << ", vote weight "
                   << *vote->getWeight() << ", period " << period << ", round " << round << ", step " << step
                   << ", validator " << wallet.second.node_addr;
    } else {
      valid_votes_weight += *vote->getWeight();
      valid_votes.push_back(std::move(vote));

      LOG(log_nf_) << "Placed " << vote->getHash() << " vote for block " << block_hash << ", vote weight "
                   << *vote->getWeight() << ", period " << period << ", round " << round << ", step " << step
                   << ", validator " << wallet.second.node_addr;
    }

    // Save own verified vote
    vote_mgr_->saveOwnVerifiedVote(vote);

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
    if (!pbft_manager_runtime_.has_value()) {
      throw std::runtime_error("PBFT manager runtime is required for next-voted status persistence");
    }
    const auto next_voted_snapshot = rustaxa::pbft_manager_runtime_apply_next_voted_status(
        *pbft_manager_runtime_.value(), static_cast<uint8_t>(*next_vote_status));
    applyPbftManagerRuntimeSnapshot(next_voted_snapshot, round_, step_, state_, current_round_lambda_,
                                    next_step_time_ms_, rounds_count_dynamic_lambda_, dynamic_lambda_,
                                    executed_pbft_block_, already_next_voted_value_,
                                    already_next_voted_null_block_hash_, broadcast_votes_counter_,
                                    rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                    rebroadcast_reward_votes_counter_);
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
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto state = fromPbftManagerRuntimeState(action_snapshot.state);
  const auto round = static_cast<PbftRound>(action_snapshot.round);
  const auto step = static_cast<PbftStep>(action_snapshot.step);
  const auto current_round_lambda = std::chrono::milliseconds(action_snapshot.current_round_lambda_ms);
  const auto period = getPbftPeriod();
  LOG(log_dg_) << "PBFT value proposal state in period " << period << ", round " << round;

  const auto fact = makePbftManagerStateActionFact(state, period, round, step, 0ms, getPbftDeadline(),
                                                   current_round_lambda, *vote_mgr_,
                                                   action_snapshot.has_cert_voted_block,
                                                   fromBridgeHash(action_snapshot.cert_voted_block_hash),
                                                   action_snapshot.already_next_voted_value,
                                                   action_snapshot.already_next_voted_null);
  executeStateActionEffectSession(fact, [&](const auto &effect) {
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

      const auto next_voted_block =
          admitStateActionPbftBlock(effect, "Value proposal re-propose");
      if (!next_voted_block) {
        return kPbftManagerStateActionEffectResultSkippedMissingLiveObject;
      }

      auto block_reward_votes = vote_mgr_->checkRewardVotesDetailed(next_voted_block, true);
      if (!block_reward_votes.accepted) {
        LOG(log_er_) << "Unable to re-propose previous round next voted block " << next_voted_block_hash << ", period "
                     << period << ", round " << round << ". Rust reward-vote validation rejected status "
                     << static_cast<uint32_t>(block_reward_votes.status) << ", error " << block_reward_votes.error_code;
        return kPbftManagerStateActionEffectResultRejectedLiveCheck;
      }

      return genAndPlaceProposeVote(next_voted_block, std::move(block_reward_votes.votes))
                 ? kPbftManagerStateActionEffectResultApplied
                 : kPbftManagerStateActionEffectResultRejectedLiveCheck;
    }

    LOG(log_er_) << "Unsupported Rust PBFT value proposal effect " << static_cast<uint32_t>(effect.intent);
    assert(false);
    return kPbftManagerStateActionEffectResultExecutorError;
  }, log_er_);
}

void PbftManager::identifyBlock_() {
  // The Filtering Step
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto state = fromPbftManagerRuntimeState(action_snapshot.state);
  const auto round = static_cast<PbftRound>(action_snapshot.round);
  const auto step = static_cast<PbftStep>(action_snapshot.step);
  const auto current_round_lambda = std::chrono::milliseconds(action_snapshot.current_round_lambda_ms);
  const auto period = getPbftPeriod();
  LOG(log_dg_) << "PBFT filtering state in period: " << period << ", round: " << round;

  const auto fact = makePbftManagerStateActionFact(state, period, round, step, 0ms, getPbftDeadline(),
                                                   current_round_lambda, *vote_mgr_,
                                                   action_snapshot.has_cert_voted_block,
                                                   fromBridgeHash(action_snapshot.cert_voted_block_hash),
                                                   action_snapshot.already_next_voted_value,
                                                   action_snapshot.already_next_voted_null);
  executeStateActionEffectSession(fact, [&](const auto &effect) {
    if (effect.intent == kPbftManagerStateActionIntentIdentifyLeaderAndSoftVote) {
      const auto leader_block_data = identifyLeaderBlock(proposed_blocks_, vote_mgr_->getProposalVotes(period, round));
      if (!leader_block_data.has_value()) {
        LOG(log_dg_) << "No leader block identified. Period " << period << ", round " << round;
        return kPbftManagerStateActionEffectResultSkippedNoWork;
      }

      assert(leader_block_data->first->getPeriod() == period);
      LOG(log_dg_) << "Leader block identified " << leader_block_data->first->getBlockHash() << ", period " << period
                   << ", round " << round;

      return placeStateActionVote(PbftVoteTypes::soft_vote, leader_block_data->first->getPeriod(), round, step,
                                  leader_block_data->first->getBlockHash(), leader_block_data->first,
                                  "Filter leader soft vote")
                 ? kPbftManagerStateActionEffectResultApplied
                 : kPbftManagerStateActionEffectResultRejectedLiveCheck;
    }

    if (effect.intent == kPbftManagerStateActionIntentSoftVotePreviousRoundNextValue) {
      const auto next_voted_block_hash = fromBridgeHash(effect.hash);
      const auto next_voted_block =
          admitStateActionPbftBlock(effect, "Filter soft-vote previous round next value");
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
  }, log_er_);
}

void PbftManager::certifyBlock_() {
  // The Certifying Step
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
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
  const auto session_step = executeStateActionEffectSession(fact, [&](const auto &effect) {
    if (effect.intent == kPbftManagerStateActionIntentGoFinish) {
      LOG(log_dg_) << "Step 3 expired, will go to step 4 in period " << period << ", round " << round;

      uint64_t votes_weight = 0;
      std::string debug_msg;
      auto soft_votes = vote_mgr_->getStepVotes(period, round, 2 /* soft voting step */);
      for (const auto &block_soft_votes : soft_votes.votes) {
        votes_weight += block_soft_votes.second.weight;
        debug_msg += "Block " + block_soft_votes.first.abridged() + "(votes weight " +
                     std::to_string(block_soft_votes.second.weight) + ") -> [";

        for (const auto &vote : block_soft_votes.second.votes) {
          debug_msg += vote.first.abridged() + "(voter " + vote.second->getVoterAddr().abridged() + "), ";
        }

        debug_msg += "]\n";
      }
      debug_msg += "all votes weight " + std::to_string(votes_weight) + ", 2t+1 threshold " +
                   std::to_string(vote_mgr_->getPbftTwoTPlusOne(period - 1, PbftVoteTypes::soft_vote).value());
      LOG(log_dg_) << debug_msg;

      return kPbftManagerStateActionEffectResultApplied;
    }

    if (effect.intent == kPbftManagerStateActionIntentCertVoteCurrentSoftValue) {
      const auto soft_voted_block =
          admitStateActionPbftBlock(effect, "Certify cert-vote current soft value");
      if (soft_voted_block == nullptr) {
        return kPbftManagerStateActionEffectResultSkippedMissingLiveObject;
      }

      // generate cert vote
      if (!placeStateActionVote(PbftVoteTypes::cert_vote, soft_voted_block->getPeriod(), round, step,
                                soft_voted_block->getBlockHash(), soft_voted_block, "Certify cert vote")) {
        return kPbftManagerStateActionEffectResultRejectedLiveCheck;
      }

      if (!pbft_manager_runtime_.has_value()) {
        throw std::runtime_error("PBFT manager Rust runtime must be initialized before persisting cert-voted block");
      }
      const auto cert_voted_snapshot = rustaxa::pbft_manager_runtime_save_cert_voted_block_in_round(
          *pbft_manager_runtime_.value(), soft_voted_block->getPeriod(), round,
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
  }, log_er_);
  go_finish_state_ = session_step.go_finish_state;
}

void PbftManager::firstFinish_() {
  // Even number steps from 4 are in first finish
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto state = fromPbftManagerRuntimeState(action_snapshot.state);
  const auto round = static_cast<PbftRound>(action_snapshot.round);
  const auto step = static_cast<PbftStep>(action_snapshot.step);
  const auto current_round_lambda = std::chrono::milliseconds(action_snapshot.current_round_lambda_ms);
  const auto period = getPbftPeriod();
  LOG(log_dg_) << "PBFT first finishing state in period " << period << ", round " << round << ", step " << step;

  const auto fact = makePbftManagerStateActionFact(state, period, round, step, 0ms, getPbftDeadline(),
                                                   current_round_lambda, *vote_mgr_,
                                                   action_snapshot.has_cert_voted_block,
                                                   fromBridgeHash(action_snapshot.cert_voted_block_hash),
                                                   action_snapshot.already_next_voted_value,
                                                   action_snapshot.already_next_voted_null);
  executeStateActionEffectSession(fact, [&](const auto &effect) {
    if (effect.intent == kPbftManagerStateActionIntentNextVoteCertVotedBlock) {
      if (!cert_voted_block_for_round_.has_value()) {
        if (!action_snapshot.has_cert_voted_block) {
          throw std::runtime_error("Rust PBFT first-finish requested cert-voted next vote without runtime metadata");
        }

        // Rust owns the cert-voted sidecar metadata and persisted payload. The C++ pointer is only a temporary
        // materialization cache for the legacy vote-generation executor boundary.
        const auto cert_voted_payload =
            rustaxa::pbft_manager_runtime_cert_voted_block_in_round(*pbft_manager_runtime_.value());
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
        if (proposed_blocks_.pushProposedPbftBlock(cert_voted_block)) {
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
      // TODO: We should vote for any value that we first saw 2t+1 next votes for in previous round -> in current design
      // we dont know for which value we saw 2t+1 next votes as first so we prefer specific block if possible
      const auto starting_value_hash = fromBridgeHash(effect.hash);
      auto block = admitStateActionPbftBlock(effect, "First finish next-vote previous round value");
      if (!block) {
        return kPbftManagerStateActionEffectResultSkippedMissingLiveObject;
      }

      return placeStateActionVote(PbftVoteTypes::next_vote, period, round, step, starting_value_hash, std::move(block),
                                  "First finish previous-round next vote")
                 ? kPbftManagerStateActionEffectResultApplied
                 : kPbftManagerStateActionEffectResultRejectedLiveCheck;
    }

    LOG(log_er_) << "Unsupported Rust PBFT first-finish effect " << static_cast<uint32_t>(effect.intent);
    assert(false);
    return kPbftManagerStateActionEffectResultExecutorError;
  }, log_er_);
}

void PbftManager::secondFinish_() {
  // Odd number steps from 5 are in second finish
  const auto action_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto state = fromPbftManagerRuntimeState(action_snapshot.state);
  const auto round = static_cast<PbftRound>(action_snapshot.round);
  const auto step = static_cast<PbftStep>(action_snapshot.step);
  const auto current_round_lambda = std::chrono::milliseconds(action_snapshot.current_round_lambda_ms);
  const auto period = getPbftPeriod();

  if (printSecondFinishStepInfo_) {
    LOG(log_dg_) << "PBFT second finishing state in period " << period << ", round " << round << ", step " << step;
    printSecondFinishStepInfo_ = false;
  }

  const auto fact =
      makePbftManagerStateActionFact(state, period, round, step, elapsedTimeInMs(second_finish_step_start_datetime_),
                                     getPbftDeadline(), current_round_lambda, *vote_mgr_,
                                     action_snapshot.has_cert_voted_block,
                                     fromBridgeHash(action_snapshot.cert_voted_block_hash),
                                     action_snapshot.already_next_voted_value, action_snapshot.already_next_voted_null);
  const auto session_step = executeStateActionEffectSession(fact, [&](const auto &effect) {
    if (effect.intent == kPbftManagerStateActionIntentNextVoteCurrentSoftValue) {
      const auto soft_voted_block_hash = fromBridgeHash(effect.hash);
      const auto soft_voted_block =
          admitStateActionPbftBlock(effect, "Second finish next-vote current soft value");
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
  }, log_er_);

  loop_back_finish_state_ = session_step.loop_back_finish_state;
}

std::optional<PbftManager::ProposedBlockData> PbftManager::generatePbftBlock(
    PbftPeriod propose_period, const blk_hash_t &prev_blk_hash, const blk_hash_t &anchor_hash,
    const blk_hash_t &order_hash, const blk_hash_t &final_chain_hash,
    const std::optional<PbftBlockExtraData> &extra_data, const std::vector<WalletConfig> &eligible_wallets) {
  // Reward votes should only include those reward votes with the same round as the round last pbft block was pushed
  // into chain
  auto reward_votes = vote_mgr_->getRewardVotes();
  if (propose_period > 1) [[likely]] {
    assert(!reward_votes.empty());
    if (reward_votes[0]->getPeriod() != propose_period - 1) {
      LOG(log_er_) << "Reward vote period(" << reward_votes[0]->getPeriod() << ") != propose_period - 1("
                   << propose_period - 1 << ")";
      assert(false);
      return {};
    }
  }

  std::vector<vote_hash_t> reward_votes_hashes;
  std::transform(reward_votes.begin(), reward_votes.end(), std::back_inserter(reward_votes_hashes),
                 [](const auto &v) { return v->getHash(); });

  try {
    ProposedBlocks propose_blocks{nullptr};
    std::vector<std::shared_ptr<PbftVote>> propose_votes;

    for (const auto &wallet : eligible_wallets) {
      auto block = std::make_shared<PbftBlock>(prev_blk_hash, anchor_hash, order_hash, final_chain_hash, propose_period,
                                               wallet.node_addr, wallet.node_secret, reward_votes_hashes, extra_data);

      const auto propose_round = getPbftRound();
      const auto propose_step = getPbftStep();
      auto propose_vote = vote_mgr_->generateVoteWithWeight(block->getBlockHash(), PbftVoteTypes::propose_vote,
                                                            propose_period, propose_round, propose_step, wallet);
      if (!propose_vote) {
        LOG(log_er_) << "Failed to generate propose vote for block " << block->getBlockHash() << ", period "
                     << propose_period << ", round " << propose_round << ", step " << propose_step << ", validator "
                     << wallet.node_addr << " when generating pbft block";
        continue;
      }

      if (!vote_mgr_->isUniqueVote(propose_vote).first) {
        LOG(log_er_) << "Non unique propose vote " << propose_vote->getHash() << " for block " << block->getBlockHash()
                     << ", period " << propose_period << ", round " << propose_vote->getRound() << ", step "
                     << propose_vote->getStep() << ", validator " << wallet.node_addr;
        continue;
      }

      propose_blocks.pushProposedPbftBlock(block, false);
      propose_votes.push_back(std::move(propose_vote));
    }

    // Select leader block
    auto leader_block_data = identifyLeaderBlock(propose_blocks, std::move(propose_votes));
    if (!leader_block_data.has_value()) {
      return {};
    }

    if (!vote_mgr_->addVerifiedVote(leader_block_data->second)) {
      LOG(log_er_) << "Unable to save propose vote " << leader_block_data->second->getHash() << " for block "
                   << leader_block_data->second->getBlockHash() << ", period " << propose_period << ", round "
                   << leader_block_data->second->getRound() << ", step " << leader_block_data->second->getStep()
                   << ", validator " << leader_block_data->second->getVoterAddr();
      return {};
    }

    // Save own verified vote
    proposed_blocks_.pushProposedPbftBlock(leader_block_data->first);
    vote_mgr_->saveOwnVerifiedVote(leader_block_data->second);

    return PbftManager::ProposedBlockData{std::move(leader_block_data->first), std::move(reward_votes),
                                          std::move(leader_block_data->second)};
  } catch (const std::exception &e) {
    LOG(log_er_) << "Block for period " << propose_period << " could not be proposed " << e.what();
    return {};
  }
}

void PbftManager::processProposedBlock(const std::shared_ptr<PbftBlock> &proposed_block) {
  if (proposed_blocks_.isInProposedBlocks(proposed_block->getPeriod(), proposed_block->getBlockHash())) {
    return;
  }

  proposed_blocks_.pushProposedPbftBlock(proposed_block);
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

blk_hash_t PbftManager::calculateOrderHash(const std::vector<std::shared_ptr<DagBlock>> &dag_blocks) {
  if (dag_blocks.empty()) {
    return kNullBlockHash;
  }
  dev::RLPStream order_stream(1);
  order_stream.appendList(dag_blocks.size());
  for (auto const &blk : dag_blocks) {
    order_stream << blk->getHash();
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

  std::vector<WalletConfig> local_wallets;
  rust::Vec<rustaxa::PbftManagerProposalWalletFact> wallet_facts;
  const auto wallets = eligible_wallets_.getWallets(current_pbft_period);
  local_wallets.reserve(wallets.size());
  wallet_facts.reserve(wallets.size());
  uint64_t wallet_index = 0;
  for (const auto &wallet : wallets) {
    local_wallets.push_back(wallet.second);
    rustaxa::PbftManagerProposalWalletFact wallet_fact;
    wallet_fact.wallet_index = wallet_index;
    wallet_fact.dpos_eligible = wallet.first;
    wallet_fact.sortition_valid = false;
    if (wallet.first) {
      wallet_fact.sortition_valid =
          vote_mgr_->genAndValidateVrfSortition(current_pbft_period, current_pbft_round, wallet.second);
      if (!wallet_fact.sortition_valid) {
        LOG(log_dg_) << "Unable to propose block for period " << current_pbft_period << ", round "
                     << current_pbft_round << ", validator " << wallet.second.node_addr << ". Invalid vrf sortition";
      }
    }
    wallet_facts.push_back(wallet_fact);
    wallet_index++;
  }

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

  const auto final_chain_facts = final_chain_->rustFinalChainForRust().collect_pbft_final_chain_facts(
      makePbftFinalChainFactRequest(current_pbft_period, kNullBlockHash, true, false, false, false));

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
  fact.final_chain_hash_valid = final_chain_facts.final_chain_hash.status == kPbftSyncFinalChainValid;
  fact.final_chain_hash = final_chain_facts.final_chain_hash.expected_hash;
  fact.wallets = std::move(wallet_facts);
  fact.ghost_path = toBridgeHashes(ghost);
  fact.has_non_finalized_fallback = non_finalized_fallback_hash.has_value();
  fact.non_finalized_fallback_hash = toBridgeHash(non_finalized_fallback_hash.value_or(kNullBlockHash));

  auto session = rustaxa::create_pbft_manager_proposal_session(fact);
  auto step = session->pbft_manager_proposal_session_next();
  while (step.action == kPbftManagerProposalActionRequestDagOrder) {
    const auto requested_anchor = fromBridgeHash(step.requested_anchor_hash);
    rustaxa::PbftManagerProposalDagOrderReport report;
    report.anchor_hash = step.requested_anchor_hash;
    const auto dag_block_order = dag_mgr_->getDagBlockOrder(requested_anchor, current_pbft_period);
    report.order_available = !dag_block_order.empty();
    report.dag_blocks.reserve(dag_block_order.size());
    for (const auto &blk_hash : dag_block_order) {
      auto dag_blk = dag_mgr_->getDagBlock(blk_hash);
      if (!dag_blk) {
        LOG(log_er_) << "DAG anchor block hash " << requested_anchor << " getDagBlock failed in propose for block "
                     << blk_hash;
        report.order_available = false;
        report.dag_blocks.clear();
        break;
      }
      rustaxa::PbftManagerProposalDagBlockFact dag_block_fact;
      dag_block_fact.hash = toBridgeHash(blk_hash);
      dag_block_fact.gas_estimation = static_cast<uint64_t>(dag_blk->getGasEstimation());
      report.dag_blocks.push_back(dag_block_fact);
    }
    step = session->pbft_manager_proposal_session_report_dag_order(report);
  }

  if (step.action == kPbftManagerProposalActionBuildProposal && step.status == kPbftManagerProposalStatusBuildReady) {
    std::vector<WalletConfig> eligible_wallets;
    eligible_wallets.reserve(step.eligible_wallet_indices.size());
    for (const auto selected_wallet_index : step.eligible_wallet_indices) {
      if (selected_wallet_index >= local_wallets.size()) {
        LOG(log_er_) << "Rust PBFT proposal selected wallet index " << selected_wallet_index
                     << " outside local wallet count " << local_wallets.size();
        assert(false);
        return {};
      }
      eligible_wallets.push_back(local_wallets[selected_wallet_index]);
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
    const auto pillar_block = pillar_chain_mgr_->getCurrentPillarBlock();
    if (!pillar_block) {
      LOG(log_er_) << "Missing pillar block, pbft period " << pbft_period;
      return {};
    }

    if (pillar_block->getPeriod() != pbft_period - 1) {
      LOG(log_er_) << "Wrong pillar block period: " << pillar_block->getPeriod() << ", pbft period: " << pbft_period;
      return {};
    }

    pillar_block_hash = pillar_block->getHash();
  }

  return PbftBlockExtraData{TARAXA_MAJOR_VERSION, TARAXA_MINOR_VERSION, TARAXA_PATCH_VERSION, TARAXA_NET_VERSION, "T",
                            pillar_block_hash};
}

std::optional<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> PbftManager::identifyLeaderBlock(
    ProposedBlocks &propose_blocks, std::vector<std::shared_ptr<PbftVote>> &&propose_votes) {
  if (propose_votes.empty()) {
    return {};
  }

  rust::Vec<rustaxa::PbftManagerLeaderCandidateInputFact> candidate_facts;
  candidate_facts.reserve(propose_votes.size());
  std::vector<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> materialized_candidates;

  for (auto &&vote : propose_votes) {
    rustaxa::PbftManagerLeaderCandidateInputFact fact;
    fact.vote_hash = toBridgeHash(vote->getHash());
    fact.block_hash = toBridgeHash(vote->getBlockHash());
    fact.period = vote->getPeriod();
    fact.credential = toBridgeFixedBytes<64>(vote->getCredential());
    fact.voter_public_key = toBridgeFixedBytes<64>(vote->getVoter());
    fact.weight_found = false;
    fact.weight = 0;
    fact.block_in_chain = false;
    fact.proposed_block_found = false;
    fact.block_validation_status = kPbftManagerLeaderBlockAlreadyValid;
    fact.pivot_hash = toBridgeHash(kNullBlockHash);

    const auto weight = vote->getWeight();
    if (!weight.has_value() || *weight == 0) {
      candidate_facts.push_back(fact);
      continue;
    }
    fact.weight_found = true;
    fact.weight = *weight;

    const auto proposed_block_hash = vote->getBlockHash();
    if (proposed_block_hash == kNullBlockHash) {
      LOG(log_er_) << "Propose block hash should not be NULL. Vote " << vote;
      candidate_facts.push_back(fact);
      continue;
    }

    if (pbft_chain_->findPbftBlockInChain(proposed_block_hash)) {
      fact.block_in_chain = true;
      candidate_facts.push_back(fact);
      continue;
    }

    const auto block_metadata = propose_blocks.getPbftProposedBlockMetadata(vote->getPeriod(), proposed_block_hash);
    if (!block_metadata.has_value()) {
      LOG(log_er_) << "Unable to get proposed block " << proposed_block_hash;
      candidate_facts.push_back(fact);
      continue;
    }

    fact.proposed_block_found = true;
    fact.pivot_hash = toBridgeHash(block_metadata->pivot_hash);
    if (block_metadata->is_valid) {
      fact.block_validation_status = kPbftManagerLeaderBlockAlreadyValid;
      candidate_facts.push_back(fact);
      continue;
    }

    const auto block_data = propose_blocks.getPbftProposedBlock(vote->getPeriod(), proposed_block_hash);
    if (!block_data.has_value()) {
      LOG(log_er_) << "Unable to materialize proposed block " << proposed_block_hash;
      fact.block_validation_status = kPbftManagerLeaderBlockRejected;
      candidate_facts.push_back(fact);
      continue;
    }

    const auto leader_block = block_data->first;
    assert(leader_block != nullptr);
    if (validatePbftBlock(leader_block)) {
      fact.block_validation_status = kPbftManagerLeaderBlockValidated;
      materialized_candidates.emplace_back(leader_block, vote);
    } else {
      LOG(log_er_) << "Proposed block " << proposed_block_hash << " failed validation, period " << vote->getPeriod();
      fact.block_validation_status = kPbftManagerLeaderBlockRejected;
    }
    candidate_facts.push_back(fact);
  }

  const auto plan = rustaxa::plan_pbft_manager_leader_candidates(std::move(candidate_facts));
  if (!plan.selected) {
    return {};
  }

  for (const auto &command : plan.valid_blocks) {
    const auto command_block_hash = fromBridgeHash(command.block_hash);
    try {
      propose_blocks.markBlockAsValid(command.period, command_block_hash);
    } catch (const std::exception &e) {
      LOG(log_er_) << "Rust PBFT leader candidate plan failed to mark valid proposed block " << command_block_hash
                   << ", period " << command.period << ": " << e.what();
      return {};
    }
  }

  const auto selected_vote_hash = fromBridgeHash(plan.selected_vote_hash);
  const auto selected_block_hash = fromBridgeHash(plan.selected_block_hash);
  for (auto &candidate : materialized_candidates) {
    if (candidate.second->getHash() == selected_vote_hash && candidate.first->getBlockHash() == selected_block_hash) {
      return std::make_pair(candidate.first, candidate.second);
    }
  }

  const auto selected_block_data = propose_blocks.getPbftProposedBlock(plan.selected_period, selected_block_hash);
  if (!selected_block_data.has_value()) {
    LOG(log_er_) << "Rust PBFT leader selection returned missing live candidate block " << selected_block_hash
                 << ", period " << plan.selected_period;
    return {};
  }

  for (auto &vote : propose_votes) {
    if (vote->getHash() == selected_vote_hash && vote->getBlockHash() == selected_block_hash) {
      return std::make_pair(selected_block_data->first, vote);
    }
  }

  LOG(log_er_) << "Rust PBFT leader selection returned missing live candidate vote " << selected_vote_hash << " block "
               << selected_block_hash;
  return {};
}

PbftStateRootValidation PbftManager::validateFinalChainHash(const std::shared_ptr<PbftBlock> &pbft_block) const {
  const auto period = pbft_block->getPeriod();
  const auto &pbft_block_hash = pbft_block->getBlockHash();

  const auto facts = final_chain_->rustFinalChainForRust().collect_pbft_final_chain_facts(
      makePbftFinalChainFactRequest(period, pbft_block->getFinalChainHash(), true, true, false, false));
  if (facts.final_chain_hash.status == kPbftSyncFinalChainMissing) {
    LOG(log_wr_) << "Block " << pbft_block_hash << " could not be validated as we are behind";
    return PbftStateRootValidation::Missing;
  }
  if (facts.final_chain_hash.status == kPbftSyncFinalChainInvalid) {
    LOG(log_er_) << "Block " << period << " hash " << pbft_block_hash << " state root "
                 << pbft_block->getFinalChainHash() << " isn't matching actual "
                 << fromBridgeHash(facts.final_chain_hash.expected_hash);
    return PbftStateRootValidation::Invalid;
  }

  return PbftStateRootValidation::Valid;
}

bool PbftManager::validatePbftBlockExtraData(const std::shared_ptr<PbftBlock> &pbft_block) const {
  const auto extra_data = pbft_block->getExtraData();
  const auto block_period = pbft_block->getPeriod();
  if (kGenesisConfig.state.hardforks.ficus_hf.isFicusHardfork(block_period)) {
    if (!extra_data.has_value()) {
      LOG(log_er_) << "PBFT block " << pbft_block->getBlockHash() << ", period " << block_period
                   << " does not contain extra data";
      return false;
    }

    // Validate optional pillar block hash
    const auto pillar_block_hash = extra_data->getPillarBlockHash();
    if (kGenesisConfig.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(block_period)) {
      if (!pillar_block_hash.has_value()) {
        LOG(log_er_) << "PBFT block " << pbft_block->getBlockHash() << ", period " << block_period
                     << " does not contain pillar block hash";
        return false;
      }
    } else if (pillar_block_hash.has_value()) {
      LOG(log_er_) << "PBFT block " << pbft_block->getBlockHash() << ", period " << block_period
                   << " contains pillar block hash even though it should not";
      return false;
    }

  } else if (extra_data.has_value()) {
    LOG(log_er_) << "PBFT block " << pbft_block->getBlockHash() << ", period " << block_period
                 << " contains extra data even though it should not";
    return false;
  }

  return true;
}

bool PbftManager::validatePillarDataInPeriodData(const PeriodData &period_data) const {
  if (!validatePbftBlockExtraData(period_data.pbft_blk)) {
    return false;
  }

  const auto block_period = period_data.pbft_blk->getPeriod();

  // Validate optional pillar votes presence
  if (kGenesisConfig.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(block_period)) {
    if (!period_data.pillar_votes_.has_value()) {
      LOG(log_er_) << "Sync PBFT block " << period_data.pbft_blk->getBlockHash() << ", period " << block_period
                   << " does not contain pillar votes";
      return false;
    }
  } else if (period_data.pillar_votes_.has_value()) {
    LOG(log_er_) << "Sync PBFT block " << period_data.pbft_blk->getBlockHash() << ", period "
                 << period_data.pbft_blk->getPeriod() << " contains pillar votes even though it should not";
    return false;
  }

  return true;
}

bool PbftManager::validatePbftBlock(const std::shared_ptr<PbftBlock> &pbft_block) const {
  if (!pbft_block) {
    LOG(log_er_) << "Unable to validate pbft block - no block provided";
    return false;
  }

  auto const &pbft_block_hash = pbft_block->getBlockHash();
  const auto block_period = pbft_block->getPeriod();
  auto const &anchor_hash = pbft_block->getPivotDagBlockHash();
  rustaxa::PbftManagerBlockValidationFact fact;
  fact.block_hash = toBridgeHash(pbft_block_hash);
  fact.period = block_period;
  fact.pivot_hash = toBridgeHash(anchor_hash);
  fact.pivot_is_null = anchor_hash == kNullBlockHash;
  fact.dag_order_cached = rustaxa::pbft_manager_runtime_has_cached_anchor_dag_order(
      *pbft_manager_runtime_.value(), toBridgeHash(anchor_hash));
  fact.dag_order_required = true;
  fact.pillar_block_required = kGenesisConfig.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(block_period);
  fact.dag_weight_check_required = false;
  fact.pbft_chain_status = kPbftManagerBlockValidationFactNotChecked;
  fact.final_chain_hash_status = kPbftManagerBlockValidationFactNotChecked;
  fact.reward_votes_status = kPbftManagerBlockValidationFactNotChecked;
  fact.extra_data_status = kPbftManagerBlockValidationFactNotChecked;
  fact.pillar_block_status = fact.pillar_block_required ? kPbftManagerBlockValidationFactNotChecked
                                                        : kPbftManagerBlockValidationFactNotRequired;
  fact.dag_order_status = kPbftManagerBlockValidationFactNotChecked;
  fact.dag_weight_status = kPbftManagerBlockValidationFactNotChecked;

  auto validation_session = rustaxa::create_pbft_manager_block_validation_session(fact);
  auto plan = validation_session->pbft_manager_block_validation_session_next();
  while (true) {
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
    if (plan.action != kPbftManagerBlockValidationActionRunCheck) {
      throw std::runtime_error("Rust PBFT block validation planner returned unknown action");
    }

    if (plan.next_check == kPbftManagerBlockValidationCheckPbftChain) {
      plan = validation_session->pbft_manager_block_validation_session_report(
          pbft_chain_->checkPbftBlockValidation(pbft_block) ? kPbftManagerBlockValidationFactValid
                                                            : kPbftManagerBlockValidationFactInvalid,
          false);
      continue;
    }

    if (plan.next_check == kPbftManagerBlockValidationCheckFinalChainHash) {
      const auto validation_result = validateFinalChainHash(pbft_block);
      if (validation_result == PbftStateRootValidation::Valid) {
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactValid,
                                                                                false);
      } else if (validation_result == PbftStateRootValidation::Missing) {
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactMissing,
                                                                                false);
      } else {
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactInvalid,
                                                                                false);
      }
      continue;
    }

    if (plan.next_check == kPbftManagerBlockValidationCheckRewardVotes) {
      const auto reward_votes = vote_mgr_->checkRewardVotesDetailed(pbft_block, false);
      if (!reward_votes.accepted) {
        LOG(log_er_) << "Failed verifying reward votes for proposed PBFT block " << pbft_block_hash
                     << ", Rust status " << static_cast<uint32_t>(reward_votes.status) << ", error "
                     << reward_votes.error_code;
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactInvalid,
                                                                                false);
      } else {
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactValid,
                                                                                false);
      }
      continue;
    }

    if (plan.next_check == kPbftManagerBlockValidationCheckExtraData) {
      plan = validation_session->pbft_manager_block_validation_session_report(
          validatePbftBlockExtraData(pbft_block) ? kPbftManagerBlockValidationFactValid
                                                 : kPbftManagerBlockValidationFactInvalid,
          false);
      continue;
    }

    if (plan.next_check == kPbftManagerBlockValidationCheckPillarBlock) {
      const auto current_pillar_block = pillar_chain_mgr_->getCurrentPillarBlock();
      if (!current_pillar_block) {
        // This should never happen
        LOG(log_er_) << "Unable to validate PBFT block " << pbft_block_hash << ", period " << block_period
                     << ". No current pillar block present in node";
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactInvalid,
                                                                                false);
        continue;
      }

      if (!pbft_block->getExtraData().has_value() || !pbft_block->getExtraData()->getPillarBlockHash().has_value() ||
          *pbft_block->getExtraData()->getPillarBlockHash() != current_pillar_block->getHash()) {
        LOG(log_er_) << "PBFT block " << pbft_block_hash << " with period " << pbft_block->getPeriod()
                     << " contains pillar block hash "
                     << (pbft_block->getExtraData().has_value() &&
                                 pbft_block->getExtraData()->getPillarBlockHash().has_value()
                             ? *pbft_block->getExtraData()->getPillarBlockHash()
                             : kNullBlockHash)
                     << ", which is different than the local current pillar block" << current_pillar_block->getHash()
                     << " with period " << current_pillar_block->getPeriod();
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactInvalid,
                                                                                false);
      } else {
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactValid,
                                                                                false);
      }
      continue;
    }

    if (plan.next_check == kPbftManagerBlockValidationCheckDagOrder) {
      auto dag_blocks_order = dag_mgr_->getDagBlockOrder(anchor_hash, pbft_block->getPeriod());
      if (dag_blocks_order.empty()) {
        LOG(log_er_) << "Missing dag blocks for proposed PBFT block " << pbft_block_hash;
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactMissing,
                                                                                false);
        continue;
      }

      auto calculated_order_hash = calculateOrderHash(dag_blocks_order);
      if (calculated_order_hash != pbft_block->getOrderHash()) {
        LOG(log_er_) << "Order hash incorrect. Pbft block: " << pbft_block_hash
                     << ". Order hash: " << pbft_block->getOrderHash() << " . Calculated hash:" << calculated_order_hash
                     << ". Dag order: " << dag_blocks_order;
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactInvalid,
                                                                                false);
        continue;
      }

      anchor_dag_block_order_cache_[anchor_hash].reserve(dag_blocks_order.size());
      for (auto const &dag_blk_hash : dag_blocks_order) {
        auto dag_block = dag_mgr_->getDagBlock(dag_blk_hash);
        assert(dag_block);
        anchor_dag_block_order_cache_[anchor_hash].emplace_back(std::move(dag_block));
      }
      const auto record_cache_snapshot = rustaxa::pbft_manager_runtime_record_cached_anchor_dag_order(
          *pbft_manager_runtime_.value(), toBridgeHash(anchor_hash));
      ensurePbftManagerRuntimeSnapshotReady(record_cache_snapshot, "Record cached PBFT DAG order anchor");

      auto last_pbft_block_hash = pbft_chain_->getLastPbftBlockHash();
      bool dag_weight_check_required = false;
      if (last_pbft_block_hash) {
        auto prev_pbft_block = pbft_chain_->getPbftBlockInChain(last_pbft_block_hash);
        auto ghost = dag_mgr_->getGhostPath(prev_pbft_block.getPivotDagBlockHash());
        dag_weight_check_required = ghost.size() > 1 && anchor_hash != ghost[1];
      }
      plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactValid,
                                                                              dag_weight_check_required);
      continue;
    }

    if (plan.next_check == kPbftManagerBlockValidationCheckDagWeight) {
      if (!checkBlockWeight(anchor_dag_block_order_cache_[anchor_hash], block_period)) {
        LOG(log_er_) << "PBFT block " << pbft_block_hash << " weight exceeded max limit";
        anchor_dag_block_order_cache_.erase(anchor_hash);
        const auto remove_cache_snapshot = rustaxa::pbft_manager_runtime_remove_cached_anchor_dag_order(
            *pbft_manager_runtime_.value(), toBridgeHash(anchor_hash));
        ensurePbftManagerRuntimeSnapshotReady(remove_cache_snapshot, "Remove cached PBFT DAG order anchor");
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactInvalid,
                                                                                false);
      } else {
        plan = validation_session->pbft_manager_block_validation_session_report(kPbftManagerBlockValidationFactValid,
                                                                                false);
      }
      continue;
    }

    throw std::runtime_error("Rust PBFT block validation planner returned unknown next check");
  }
}

bool PbftManager::pushCertVotedPbftBlockIntoChain_(const std::shared_ptr<PbftBlock> &pbft_block,
                                                   std::vector<std::shared_ptr<PbftVote>> &&current_round_cert_votes) {
  PeriodData period_data;
  period_data.pbft_blk = pbft_block;
  if (pbft_block->getPivotDagBlockHash() != kNullBlockHash) {
    auto dag_order_it = anchor_dag_block_order_cache_.find(pbft_block->getPivotDagBlockHash());
    assert(dag_order_it != anchor_dag_block_order_cache_.end());
    std::unordered_set<trx_hash_t> trx_set;
    std::vector<trx_hash_t> transactions_to_query;
    period_data.dag_blocks.reserve(dag_order_it->second.size());
    for (const auto &dag_blk : dag_order_it->second) {
      for (const auto &trx_hash : dag_blk->getTrxs()) {
        if (trx_set.insert(trx_hash).second) {
          transactions_to_query.emplace_back(trx_hash);
        }
      }
      period_data.dag_blocks.emplace_back(dag_blk);
    }
    period_data.transactions = trx_mgr_->getNonfinalizedTrx(transactions_to_query);
  }

  auto reward_votes = vote_mgr_->checkRewardVotesDetailed(period_data.pbft_blk, true);
  if (!reward_votes.accepted) {
    LOG(log_er_) << "Missing reward votes in cert voted block " << pbft_block->getBlockHash() << ", Rust status "
                 << static_cast<uint32_t>(reward_votes.status) << ", error " << reward_votes.error_code;
    return false;
  }
  period_data.previous_block_cert_votes = std::move(reward_votes.votes);

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

  auto drain_session = rustaxa::create_pbft_sync_queue_drain_session();
  std::optional<std::pair<PeriodData, std::vector<std::shared_ptr<PbftVote>>>> accepted_period_data;

  auto report_step = [&](const rustaxa::PbftSyncQueueDrainStep &step, bool success, bool accepted) {
    rustaxa::PbftSyncQueueDrainReport report;
    report.action = step.action;
    report.success = success;
    report.accepted_period_data = accepted;
    const auto result = rustaxa::pbft_sync_queue_drain_session_report(*drain_session, report);
    if (!result.can_continue && result.status != kPbftSyncQueueDrainStatusComplete) {
      LOG(log_er_) << "Rust PBFT sync queue drain stopped after action " << static_cast<uint32_t>(step.action)
                   << ", status " << static_cast<uint32_t>(result.status) << ", error "
                   << static_cast<std::string>(result.error_code);
    }
    return result.can_continue;
  };

  while (true) {
    const auto step = rustaxa::pbft_sync_queue_drain_session_next(*drain_session, periodDataQueueSize(),
                                                                 static_cast<uint64_t>(getPbftPeriod()));

    if (step.action == kPbftSyncQueueDrainActionStop) {
      break;
    }
    if (step.status != kPbftSyncQueueDrainStatusActive) {
      LOG(log_er_) << "Rust PBFT sync queue drain returned non-active step, action "
                   << static_cast<uint32_t>(step.action) << ", status " << static_cast<uint32_t>(step.status)
                   << ", error " << static_cast<std::string>(step.error_code);
      break;
    }

    if (step.action == kPbftSyncQueueDrainActionCleanOldData) {
      sync_queue_.cleanOldData(step.clean_before_period);
      if (!report_step(step, true, false)) {
        break;
      }
      continue;
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

      const auto pushed = pushPbftBlock_(std::move(accepted_period_data->first), std::move(accepted_period_data->second));
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

void PbftManager::finalize_(PeriodData &&period_data, std::vector<h256> &&finalized_dag_blk_hashes,
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
}

bool PbftManager::pushPbftBlock_(PeriodData &&period_data, std::vector<std::shared_ptr<PbftVote>> &&cert_votes) {
  auto const &pbft_block_hash = period_data.pbft_blk->getBlockHash();
  if (!pbft_manager_runtime_.has_value()) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before reading PBFT block existence");
  }
  const auto push_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto block_in_chain =
      rustaxa::pbft_manager_runtime_pbft_block_in_db(*pbft_manager_runtime_.value(), toBridgeHash(pbft_block_hash));
  if (block_in_chain && cert_votes.empty()) {
    LOG(log_nf_) << "PBFT block: " << pbft_block_hash << " in DB already.";
    LOG(log_dg_) << "Rust PBFT finalization resume classifier cannot inspect duplicate block " << pbft_block_hash
                 << " because certified-vote facts are unavailable.";
    if (push_snapshot.has_cert_voted_block && fromBridgeHash(push_snapshot.cert_voted_block_hash) == pbft_block_hash) {
      LOG(log_er_) << "Last cert voted value should be kNullBlockHash. Block hash "
                   << pbft_block_hash << " has been pushed into chain already";
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
  const auto dynamic_lambda_runtime_snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
  const auto dynamic_lambda_plan = rustaxa::plan_pbft_dynamic_lambda(makePbftDynamicLambdaFact(
      kGenesisConfig.state.hardforks, kGenesisConfig.state.dpos.blocks_per_year, dynamic_lambda_enabled,
      block_pbft_period, block_pbft_round, dynamic_lambda_runtime_snapshot.rounds_count_dynamic_lambda,
      dynamic_lambda_runtime_snapshot.dynamic_lambda_ms));
  if (dynamic_lambda_plan.status != kPbftFinalizationStatusAccepted) {
    LOG(log_er_) << "Rust PBFT dynamic-lambda planner rejected block " << pbft_block_hash << ", period "
                 << block_pbft_period << ", round " << block_pbft_round << ", error "
                 << static_cast<std::string>(dynamic_lambda_plan.error_code);
    return false;
  }
  const uint32_t block_lambda = dynamic_lambda_plan.period_lambda;
  rustaxa::PeriodLambda last_saved_period_lambda{};
  if (dynamic_lambda_enabled) {
    last_saved_period_lambda = rustaxa::pbft_manager_runtime_load_finalization_last_period_lambda(
        *pbft_manager_runtime_.value(), block_pbft_period - 1);
  }
  const uint32_t dynamic_blocks_per_year = dynamic_lambda_enabled ? dynamic_lambda_plan.blocks_per_year : 0;
  bool pillar_block_finalized = false;
  const auto pillar_block_hash =
      period_data.pbft_blk->getExtraData() ? period_data.pbft_blk->getExtraData()->getPillarBlockHash()
                                           : std::optional<blk_hash_t>();
  const auto pillar_finalization_required =
      kGenesisConfig.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(block_pbft_period);
  rustaxa::PbftFinalizationPillarPreflightFact pillar_preflight_fact;
  pillar_preflight_fact.pbft_block_hash = toBridgeHash(pbft_block_hash);
  pillar_preflight_fact.block_period = block_pbft_period;
  pillar_preflight_fact.block_in_chain = block_in_chain;
  pillar_preflight_fact.pillar_finalization_required = pillar_finalization_required;
  pillar_preflight_fact.has_pillar_block_hash = pillar_block_hash.has_value();
  pillar_preflight_fact.pillar_block_hash =
      pillar_block_hash ? toBridgeHash(*pillar_block_hash) : toBridgeHash(kNullBlockHash);
  pillar_preflight_fact.pillar_block_finalized = false;
  const auto pillar_preflight_plan = rustaxa::plan_pbft_finalization_pillar_preflight(pillar_preflight_fact);
  if (!pillar_preflight_plan.accepted) {
    LOG(log_er_) << "Rust PBFT pillar preflight rejected block " << pbft_block_hash << ", period "
                 << block_pbft_period << ", action " << static_cast<uint32_t>(pillar_preflight_plan.action)
                 << ", status " << static_cast<uint32_t>(pillar_preflight_plan.status) << ", error "
                 << static_cast<std::string>(pillar_preflight_plan.error_code);
    return false;
  }
  if (pillar_preflight_plan.action == kPbftFinalizationPillarPreflightActionFinalizePillarBlock) {
    assert(pillar_block_hash.has_value());
    auto above_threshold_pillar_votes = pillar_chain_mgr_->finalizePillarBlock(*pillar_block_hash);
    rustaxa::PbftFinalizationPillarPreflightReport pillar_preflight_report;
    pillar_preflight_report.action = pillar_preflight_plan.action;
    pillar_preflight_report.success = !above_threshold_pillar_votes.empty();
    pillar_preflight_report.status =
        pillar_preflight_report.success ? kPbftFinalizationPillarPreflightStatusAccepted : 255;
    pillar_preflight_report.error_code =
        pillar_preflight_report.success ? "" : "PBFT_FINALIZE_PILLAR_PREFLIGHT_EMPTY_VOTES";
    pillar_preflight_report.block_period = block_pbft_period;
    pillar_preflight_report.pbft_block_hash = toBridgeHash(pbft_block_hash);
    pillar_preflight_report.pillar_block_hash = toBridgeHash(*pillar_block_hash);
    pillar_preflight_report.pillar_vote_count = above_threshold_pillar_votes.size();
    const auto pillar_preflight_result =
        rustaxa::report_pbft_finalization_pillar_preflight(pillar_preflight_plan, pillar_preflight_report);
    if (!pillar_preflight_result.accepted) {
      LOG(log_er_) << "Rust PBFT pillar preflight report rejected block " << pbft_block_hash << ", period "
                   << block_pbft_period << ", action " << static_cast<uint32_t>(pillar_preflight_result.action)
                   << ", status " << static_cast<uint32_t>(pillar_preflight_result.status) << ", error "
                   << static_cast<std::string>(pillar_preflight_result.error_code);
      return false;
    }
    period_data.pillar_votes_ = std::move(above_threshold_pillar_votes);
    pillar_block_finalized = true;
  }

  auto null_anchor = period_data.pbft_blk->getPivotDagBlockHash() == kNullBlockHash;

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

  const auto planner_chain_last_hash =
      block_in_chain ? period_data.pbft_blk->getPrevBlockHash() : pbft_chain_->getHeadHash();
  const auto planner_chain_last_period = block_in_chain ? block_pbft_period - 1 : pbft_chain_->getPbftChainSize();
  const auto planner_pillar_block_finalized = block_in_chain ? true : pillar_block_finalized;
  const auto finalization_plan = rustaxa::plan_pbft_finalization_intent(makePbftFinalizationIntentFact(
      period_data, planner_chain_last_hash, pbft_chain_->getLastPbftBlockHash(), planner_chain_last_period, false,
      planner_pillar_block_finalized, dynamic_lambda_enabled, cert_votes.size(), sample_cert_vote->getBlockHash(),
      sample_cert_vote->getPeriod(), sample_cert_vote->getRound(), sample_cert_vote->getStep(), block_lambda,
      last_saved_period_lambda.found, last_saved_period_lambda.value, dynamic_blocks_per_year,
      dynamic_lambda_plan.rounds_count_dynamic_lambda, dynamic_lambda_plan.dynamic_lambda,
      kGenesisConfig.state.dpos.blocks_per_year, dag_blocks_order, transaction_order,
      pbft_chain_->getJsonStrForBlock(pbft_block_hash, null_anchor),
      kGenesisConfig.state.hardforks.ficus_hf.isPillarBlockPeriod(block_pbft_period)));
  if (!finalization_plan.finalize_block || finalization_plan.status != kPbftFinalizationStatusAccepted) {
    LOG(log_er_) << "Rust PBFT finalization planner rejected block " << pbft_block_hash << ", period "
                 << block_pbft_period << ", round " << block_pbft_round << ", status "
                 << static_cast<uint32_t>(finalization_plan.status);
    return false;
  }
  if (block_in_chain) {
    bool resume_executed = false;
    try {
      const auto resume_plan = rustaxa::pbft_manager_runtime_inspect_finalization_resume(
          *pbft_manager_runtime_.value(), finalization_plan.storage_write_intent, final_chain_->lastBlockNumber());
      LOG(log_nf_) << "PBFT block: " << pbft_block_hash << " in DB already.";
      LOG(log_dg_) << "Rust PBFT finalization resume classified duplicate block " << pbft_block_hash << ", period "
                   << block_pbft_period << ", status " << static_cast<uint32_t>(resume_plan.status) << ", complete "
                   << resume_plan.complete << ", replay actions " << resume_plan.replay_actions.size() << ", error "
                   << static_cast<std::string>(resume_plan.error_code);
      if (resume_plan.status == kPbftFinalizationResumeStatusNeedsPillarPostProcessingReplay) {
        LOG(log_er_) << "Rust PBFT finalization resume requires pillar post-processing for block " << pbft_block_hash
                     << ", period " << block_pbft_period
                     << ", but no durable pillar post-processing proof exists yet. Error "
                     << static_cast<std::string>(resume_plan.error_code);
      } else if (resume_plan.status == kPbftFinalizationResumeStatusNeedsDynamicLambdaPersistence ||
                 resume_plan.status == kPbftFinalizationResumeStatusNeedsFinalChainReplay ||
                 resume_plan.status == kPbftFinalizationResumeStatusNeedsExecutedStatusPersistence) {
        auto resume_runtime_session = rustaxa::create_pbft_finalization_resume_runtime_session(resume_plan);
        auto begin_resume_action = [&](uint8_t expected_action, rustaxa::PbftFinalizationRuntimeSessionStep &step) {
          step = resume_runtime_session->pbft_finalization_runtime_session_next();
          if (!step.has_action || step.action != expected_action ||
              step.status != kPbftFinalizationRuntimeStatusActive) {
            LOG(log_er_) << "Rust PBFT finalization resume expected action " << static_cast<uint32_t>(expected_action)
                         << " for block " << pbft_block_hash << ", period " << block_pbft_period << ", got action "
                         << static_cast<uint32_t>(step.action) << ", status " << static_cast<uint32_t>(step.status)
                         << ", error " << static_cast<std::string>(step.error_code);
            resume_runtime_session->abort_pbft_finalization_runtime_session();
            return false;
          }
          return true;
        };
        auto report_resume_action_detail = [&](const rustaxa::PbftFinalizationRuntimeSessionStep &step, bool success,
                                               uint8_t action_status, std::string error_code) {
          rustaxa::PbftFinalizationRuntimeActionReport report;
          report.cursor = step.cursor;
          report.action = step.action;
          report.success = success;
          report.status = action_status;
          report.error_code = std::move(error_code);
          const auto next_step = resume_runtime_session->pbft_finalization_runtime_session_report_action(report);
          if (!success || (next_step.status != kPbftFinalizationRuntimeStatusActive &&
                           next_step.status != kPbftFinalizationRuntimeStatusComplete)) {
            LOG(log_er_) << "Rust PBFT finalization resume action " << static_cast<uint32_t>(step.action)
                         << " failed for block " << pbft_block_hash << ", period " << block_pbft_period << ", status "
                         << static_cast<uint32_t>(next_step.status) << ", error "
                         << static_cast<std::string>(next_step.error_code);
            return false;
          }
          return true;
        };
        auto report_resume_action = [&](const rustaxa::PbftFinalizationRuntimeSessionStep &step, bool success,
                                        uint8_t action_status) {
          return report_resume_action_detail(step, success, action_status,
                                             success ? std::string{} : "PBFT_FINALIZE_RESUME_ACTION_FAILED");
        };

        rustaxa::PbftFinalizationRuntimeSessionStep resume_step{};
        if (resume_plan.status == kPbftFinalizationResumeStatusNeedsDynamicLambdaPersistence) {
          if (!begin_resume_action(kPbftFinalizationRuntimeActionApplyDynamicLambda, resume_step)) {
            return false;
          }
          rustaxa::PbftFinalizedPeriodApplyResult dynamic_lambda_result{};
          try {
            auto dynamic_lambda_stage = makeFinalizationStorageStage(kPbftFinalizationStorageStageDynamicLambda);
            dynamic_lambda_stage.rounds_count_dynamic_lambda =
                finalization_plan.storage_write_intent.rounds_count_dynamic_lambda;
            dynamic_lambda_stage.dynamic_lambda = finalization_plan.storage_write_intent.dynamic_lambda;
            rust::Vec<rustaxa::PbftFinalizationStorageWriteStage> dynamic_lambda_stages;
            dynamic_lambda_stages.push_back(std::move(dynamic_lambda_stage));
            dynamic_lambda_result = rustaxa::pbft_manager_runtime_apply_finalization_storage_writes(
                *pbft_manager_runtime_.value(), finalization_plan.storage_write_intent,
                std::move(dynamic_lambda_stages), false);
          } catch (const std::exception &e) {
            LOG(log_er_) << "Rust PBFT resume dynamic-lambda storage appender failed for block " << pbft_block_hash
                         << ", period " << block_pbft_period << ": " << e.what();
            report_resume_action(resume_step, false, 255);
            return false;
          }
          if (dynamic_lambda_result.status != kPbftFinalizedPeriodApplyStatusApplied &&
              dynamic_lambda_result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
            LOG(log_er_) << "Rust PBFT resume dynamic-lambda storage appender rejected block " << pbft_block_hash
                         << ", period " << block_pbft_period << ", status "
                         << static_cast<uint32_t>(dynamic_lambda_result.status) << ", error "
                         << static_cast<std::string>(dynamic_lambda_result.error_code);
            report_resume_action(resume_step, false, dynamic_lambda_result.status);
            return false;
          }
          const auto dynamic_lambda_snapshot = rustaxa::pbft_manager_runtime_apply_dynamic_lambda(
              *pbft_manager_runtime_.value(), finalization_plan.storage_write_intent.rounds_count_dynamic_lambda,
              finalization_plan.storage_write_intent.dynamic_lambda);
          if (dynamic_lambda_snapshot.status != kPbftManagerStartupRestoreStatusReady) {
            LOG(log_er_) << "Rust PBFT resume dynamic-lambda live-state update failed for block " << pbft_block_hash
                         << ", period " << block_pbft_period << ", status "
                         << static_cast<uint32_t>(dynamic_lambda_snapshot.status);
            report_resume_action(resume_step, false, 255);
            return false;
          }
          applyPbftManagerRuntimeSnapshot(dynamic_lambda_snapshot, round_, step_, state_, current_round_lambda_,
                                          next_step_time_ms_, rounds_count_dynamic_lambda_, dynamic_lambda_,
                                          executed_pbft_block_, already_next_voted_value_,
                                          already_next_voted_null_block_hash_, broadcast_votes_counter_,
                                          rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                          rebroadcast_reward_votes_counter_);
          rustaxa::PbftFinalizationLiveMutationReport dynamic_lambda_report{};
          dynamic_lambda_report.action = kPbftFinalizationRuntimeActionApplyDynamicLambda;
          dynamic_lambda_report.block_period = finalization_plan.storage_write_intent.block_period;
          dynamic_lambda_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
          dynamic_lambda_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
          dynamic_lambda_report.rounds_count_dynamic_lambda = dynamic_lambda_snapshot.rounds_count_dynamic_lambda;
          dynamic_lambda_report.dynamic_lambda = dynamic_lambda_snapshot.dynamic_lambda_ms;
          const auto dynamic_lambda_validation =
              rustaxa::validate_pbft_finalization_live_mutation_report(finalization_plan, dynamic_lambda_report);
          if (!dynamic_lambda_validation.accepted) {
            LOG(log_er_) << "Rust PBFT finalization resume dynamic-lambda live mutation rejected for block "
                         << pbft_block_hash << ", period " << block_pbft_period << ", status "
                         << static_cast<uint32_t>(dynamic_lambda_validation.status) << ", error "
                         << static_cast<std::string>(dynamic_lambda_validation.error_code);
          }
          const auto action_status =
              dynamic_lambda_validation.accepted ? dynamic_lambda_result.status : dynamic_lambda_validation.status;
          const auto action_error = dynamic_lambda_validation.accepted
                                        ? std::string{}
                                        : static_cast<std::string>(dynamic_lambda_validation.error_code);
          if (!report_resume_action_detail(resume_step, dynamic_lambda_validation.accepted, action_status,
                                           action_error)) {
            return false;
          }
        }

        if (resume_runtime_session->pbft_finalization_runtime_session_next().action ==
            kPbftFinalizationRuntimeActionFinalizeFinalChain) {
          if (final_chain_->lastBlockNumber() + 1 != block_pbft_period) {
            LOG(log_er_) << "Rust PBFT finalization resume refused non-sequential FinalChain replay for block "
                         << pbft_block_hash << ", period " << block_pbft_period << ", FinalChain last block "
                         << final_chain_->lastBlockNumber();
            resume_runtime_session->abort_pbft_finalization_runtime_session();
            return false;
          }
          if (!begin_resume_action(kPbftFinalizationRuntimeActionFinalizeFinalChain, resume_step)) {
            return false;
          }
          finalize_(std::move(period_data), std::move(dag_blocks_order),
                    finalization_plan.storage_write_intent.blocks_per_year);
          if (final_chain_->lastBlockNumber() < block_pbft_period) {
            report_resume_action(resume_step, false, 255);
            return false;
          }
          rustaxa::PbftFinalizationLiveMutationReport final_chain_report{};
          final_chain_report.action = kPbftFinalizationRuntimeActionFinalizeFinalChain;
          final_chain_report.block_period = finalization_plan.storage_write_intent.block_period;
          final_chain_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
          final_chain_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
          final_chain_report.final_chain_dispatched = true;
          final_chain_report.final_chain_blocks_per_year = finalization_plan.storage_write_intent.blocks_per_year;
          final_chain_report.final_chain_last_block = final_chain_->lastBlockNumber();
          const auto final_chain_validation =
              rustaxa::validate_pbft_finalization_live_mutation_report(finalization_plan, final_chain_report);
          if (!final_chain_validation.accepted) {
            LOG(log_er_) << "Rust PBFT finalization resume FinalChain dispatch report rejected for block "
                         << pbft_block_hash << ", period " << block_pbft_period << ", status "
                         << static_cast<uint32_t>(final_chain_validation.status) << ", error "
                         << static_cast<std::string>(final_chain_validation.error_code);
          }
          if (!report_resume_action_detail(resume_step, final_chain_validation.accepted, final_chain_validation.status,
                                           static_cast<std::string>(final_chain_validation.error_code))) {
            return false;
          }
        }

        if (resume_runtime_session->pbft_finalization_runtime_session_next().action ==
            kPbftFinalizationRuntimeActionPersistExecutedStatus) {
          if (!begin_resume_action(kPbftFinalizationRuntimeActionPersistExecutedStatus, resume_step)) {
            return false;
          }
          rustaxa::PbftFinalizedPeriodApplyResult executed_status_result{};
          try {
            rust::Vec<rustaxa::PbftFinalizationStorageWriteStage> executed_status_stages;
            executed_status_stages.push_back(makeFinalizationStorageStage(kPbftFinalizationStorageStageExecutedStatus));
            executed_status_result = rustaxa::pbft_manager_runtime_apply_finalization_storage_writes(
                *pbft_manager_runtime_.value(), finalization_plan.storage_write_intent,
                std::move(executed_status_stages), false);
          } catch (const std::exception &e) {
            LOG(log_er_) << "Rust PBFT resume executed-status storage appender failed for block " << pbft_block_hash
                         << ", period " << block_pbft_period << ": " << e.what();
            report_resume_action(resume_step, false, 255);
            return false;
          }
          if (executed_status_result.status != kPbftFinalizedPeriodApplyStatusApplied &&
              executed_status_result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
            LOG(log_er_) << "Rust PBFT resume executed-status storage appender rejected block " << pbft_block_hash
                         << ", period " << block_pbft_period << ", status "
                         << static_cast<uint32_t>(executed_status_result.status) << ", error "
                         << static_cast<std::string>(executed_status_result.error_code);
            report_resume_action(resume_step, false, executed_status_result.status);
            return false;
          }
          if (!report_resume_action(resume_step, true, executed_status_result.status)) {
            return false;
          }
        }

        if (resume_runtime_session->pbft_finalization_runtime_session_next().action ==
            kPbftFinalizationRuntimeActionSetExecutedFlag) {
          if (!begin_resume_action(kPbftFinalizationRuntimeActionSetExecutedFlag, resume_step)) {
            return false;
          }
          const auto executed_status_snapshot = rustaxa::pbft_manager_runtime_apply_finalization_executed_status(
              *pbft_manager_runtime_.value(), finalization_plan.storage_write_intent);
          applyPbftManagerRuntimeSnapshot(executed_status_snapshot, round_, step_, state_, current_round_lambda_,
                                          next_step_time_ms_, rounds_count_dynamic_lambda_, dynamic_lambda_,
                                          executed_pbft_block_, already_next_voted_value_,
                                          already_next_voted_null_block_hash_, broadcast_votes_counter_,
                                          rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                          rebroadcast_reward_votes_counter_);
          rustaxa::PbftFinalizationLiveMutationReport executed_report{};
          executed_report.action = kPbftFinalizationRuntimeActionSetExecutedFlag;
          executed_report.block_period = finalization_plan.storage_write_intent.block_period;
          executed_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
          executed_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
          executed_report.executed_pbft_block = executed_status_snapshot.executed_pbft_block;
          const auto executed_validation =
              rustaxa::validate_pbft_finalization_live_mutation_report(finalization_plan, executed_report);
          if (!executed_validation.accepted) {
            LOG(log_er_) << "Rust PBFT finalization resume executed-flag live mutation rejected for block "
                         << pbft_block_hash << ", period " << block_pbft_period << ", status "
                         << static_cast<uint32_t>(executed_validation.status) << ", error "
                         << static_cast<std::string>(executed_validation.error_code);
          }
          if (!report_resume_action_detail(resume_step, executed_validation.accepted, executed_validation.status,
                                           static_cast<std::string>(executed_validation.error_code))) {
            return false;
          }
          if (!begin_resume_action(kPbftFinalizationRuntimeActionAdvancePeriod, resume_step)) {
            return false;
          }
          if (!applyRustPlannedAdvancePeriod_(finalization_plan.storage_write_intent.block_period)) {
            report_resume_action(resume_step, false, 255);
            return false;
          }
          rustaxa::PbftFinalizationLiveMutationReport advance_report{};
          advance_report.action = kPbftFinalizationRuntimeActionAdvancePeriod;
          advance_report.block_period = finalization_plan.storage_write_intent.block_period;
          advance_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
          advance_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
          advance_report.manager_period =
              rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value()).period;
          const auto advance_validation =
              rustaxa::validate_pbft_finalization_live_mutation_report(finalization_plan, advance_report);
          if (!advance_validation.accepted) {
            LOG(log_er_) << "Rust PBFT finalization resume advance-period live mutation rejected for block "
                         << pbft_block_hash << ", period " << block_pbft_period << ", status "
                         << static_cast<uint32_t>(advance_validation.status) << ", error "
                         << static_cast<std::string>(advance_validation.error_code);
          }
          if (!report_resume_action_detail(resume_step, advance_validation.accepted, advance_validation.status,
                                           static_cast<std::string>(advance_validation.error_code))) {
            return false;
          }
        }

        if (resume_runtime_session->pbft_finalization_runtime_session_next().action ==
            kPbftFinalizationRuntimeActionProcessPillarBlock) {
          if (!begin_resume_action(kPbftFinalizationRuntimeActionProcessPillarBlock, resume_step)) {
            return false;
          }
          assert(block_pbft_period == pbft_chain_->getPbftChainSize());
          const auto pillar_request_period = block_pbft_period - final_chain_->delegationDelay();
          processPillarBlock(block_pbft_period);
          rustaxa::PbftFinalizationLiveMutationReport pillar_report{};
          pillar_report.action = kPbftFinalizationRuntimeActionProcessPillarBlock;
          pillar_report.block_period = finalization_plan.storage_write_intent.block_period;
          pillar_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
          pillar_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
          pillar_report.manager_period = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value()).period;
          pillar_report.pillar_processed_period = block_pbft_period;
          pillar_report.pillar_request_period = pillar_request_period;
          const auto pillar_validation =
              rustaxa::validate_pbft_finalization_live_mutation_report(finalization_plan, pillar_report);
          if (!pillar_validation.accepted) {
            LOG(log_er_) << "Rust PBFT finalization resume pillar live mutation rejected for block " << pbft_block_hash
                         << ", period " << block_pbft_period << ", status "
                         << static_cast<uint32_t>(pillar_validation.status) << ", error "
                         << static_cast<std::string>(pillar_validation.error_code);
          }
          if (!report_resume_action_detail(resume_step, pillar_validation.accepted, pillar_validation.status,
                                           static_cast<std::string>(pillar_validation.error_code))) {
            return false;
          }
        }

        const auto final_resume_step = resume_runtime_session->pbft_finalization_runtime_session_next();
        if (!final_resume_step.complete || final_resume_step.status != kPbftFinalizationRuntimeStatusComplete) {
          LOG(log_er_) << "Rust PBFT finalization resume runtime did not complete for block " << pbft_block_hash
                       << ", period " << block_pbft_period << ", status "
                       << static_cast<uint32_t>(final_resume_step.status) << ", action "
                       << static_cast<uint32_t>(final_resume_step.action) << ", error "
                       << static_cast<std::string>(final_resume_step.error_code);
          resume_runtime_session->abort_pbft_finalization_runtime_session();
          return false;
        }
        resume_executed = true;
      }
    } catch (const std::exception &e) {
      LOG(log_er_) << "Rust PBFT finalization resume inspection failed for duplicate block " << pbft_block_hash
                   << ", period " << block_pbft_period << ": " << e.what();
    }
    if (push_snapshot.has_cert_voted_block && fromBridgeHash(push_snapshot.cert_voted_block_hash) == pbft_block_hash) {
      LOG(log_er_) << "Last cert voted value should be kNullBlockHash. Block hash "
                   << pbft_block_hash << " has been pushed into chain already";
      assert(false);
    }
    return resume_executed;
  }
  const auto finalization_runtime_plan = rustaxa::plan_pbft_finalization_runtime(finalization_plan);
  if (!finalization_runtime_plan.finalize_block ||
      finalization_runtime_plan.status != kPbftFinalizationStatusAccepted ||
      finalization_runtime_plan.actions.empty()) {
    LOG(log_er_) << "Rust PBFT finalization runtime planner rejected block " << pbft_block_hash << ", period "
                 << block_pbft_period << ", round " << block_pbft_round << ", status "
                 << static_cast<uint32_t>(finalization_runtime_plan.status) << ", error "
                 << static_cast<std::string>(finalization_runtime_plan.error_code);
    return false;
  }
  if (finalization_plan.storage_write_intent.apply_dynamic_lambda_update !=
          dynamic_lambda_plan.apply_dynamic_lambda_update ||
      finalization_plan.storage_write_intent.period_lambda != dynamic_lambda_plan.period_lambda ||
      finalization_plan.storage_write_intent.blocks_per_year != dynamic_lambda_plan.blocks_per_year) {
    LOG(log_er_) << "Rust PBFT finalization dynamic-lambda facts diverged for block " << pbft_block_hash << ", period "
                 << block_pbft_period;
    return false;
  }
  auto finalization_runtime_session = rustaxa::create_pbft_finalization_runtime_session(finalization_plan);
  auto begin_runtime_action = [&](uint8_t expected_action, rustaxa::PbftFinalizationRuntimeSessionStep &step) {
    step = finalization_runtime_session->pbft_finalization_runtime_session_next();
    if (!step.has_action || step.action != expected_action || step.status != kPbftFinalizationRuntimeStatusActive) {
      LOG(log_er_) << "Rust PBFT finalization runtime expected action " << static_cast<uint32_t>(expected_action)
                   << " for block " << pbft_block_hash << ", period " << block_pbft_period << ", got action "
                   << static_cast<uint32_t>(step.action) << ", status " << static_cast<uint32_t>(step.status)
                   << ", error " << static_cast<std::string>(step.error_code);
      finalization_runtime_session->abort_pbft_finalization_runtime_session();
      return false;
    }
    return true;
  };
  auto report_runtime_action = [&](const rustaxa::PbftFinalizationRuntimeSessionStep &step, bool success,
                                   uint8_t action_status) {
    rustaxa::PbftFinalizationRuntimeActionReport report;
    report.cursor = step.cursor;
    report.action = step.action;
    report.success = success;
    report.status = action_status;
    report.error_code = success ? "" : "PBFT_FINALIZE_RUNTIME_ACTION_FAILED";
    const auto next_step = finalization_runtime_session->pbft_finalization_runtime_session_report_action(report);
    if (!success) {
      LOG(log_er_) << "Rust PBFT finalization runtime action " << static_cast<uint32_t>(step.action)
                   << " failed for block " << pbft_block_hash << ", period " << block_pbft_period << ", status "
                   << static_cast<uint32_t>(next_step.status) << ", error "
                   << static_cast<std::string>(next_step.error_code);
      return false;
    }
    if (next_step.status != kPbftFinalizationRuntimeStatusActive &&
        next_step.status != kPbftFinalizationRuntimeStatusComplete) {
      LOG(log_er_) << "Rust PBFT finalization runtime rejected action " << static_cast<uint32_t>(step.action)
                   << " for block " << pbft_block_hash << ", period " << block_pbft_period << ", status "
                   << static_cast<uint32_t>(next_step.status) << ", error "
                   << static_cast<std::string>(next_step.error_code);
      return false;
    }
    return true;
  };
  auto report_runtime_action_detail = [&](const rustaxa::PbftFinalizationRuntimeSessionStep &step, bool success,
                                          uint8_t action_status, std::string error_code) {
    rustaxa::PbftFinalizationRuntimeActionReport report;
    report.cursor = step.cursor;
    report.action = step.action;
    report.success = success;
    report.status = action_status;
    report.error_code = std::move(error_code);
    const auto next_step = finalization_runtime_session->pbft_finalization_runtime_session_report_action(report);
    if (!success) {
      LOG(log_er_) << "Rust PBFT finalization runtime action " << static_cast<uint32_t>(step.action)
                   << " failed for block " << pbft_block_hash << ", period " << block_pbft_period << ", status "
                   << static_cast<uint32_t>(next_step.status) << ", error "
                   << static_cast<std::string>(next_step.error_code);
      return false;
    }
    if (next_step.status != kPbftFinalizationRuntimeStatusActive &&
        next_step.status != kPbftFinalizationRuntimeStatusComplete) {
      LOG(log_er_) << "Rust PBFT finalization runtime rejected action " << static_cast<uint32_t>(step.action)
                   << " for block " << pbft_block_hash << ", period " << block_pbft_period << ", status "
                   << static_cast<uint32_t>(next_step.status) << ", error "
                   << static_cast<std::string>(next_step.error_code);
      return false;
    }
    return true;
  };
  auto validate_live_mutation = [&](const rustaxa::PbftFinalizationLiveMutationReport &report) {
    return rustaxa::validate_pbft_finalization_live_mutation_report(finalization_plan, report);
  };

  rustaxa::PbftFinalizationRuntimeSessionStep runtime_step{};
  if (!begin_runtime_action(kPbftFinalizationRuntimeActionApplyPrimaryStorage, runtime_step)) {
    return false;
  }
  rust::Vec<rustaxa::PbftFinalizationStorageWriteStage> first_persistence_stages;
  first_persistence_stages.push_back(makeFinalizationStorageStage(kPbftFinalizationStorageStagePrimary));

  // Replace current reward votes
  bool should_commit_reward_vote_metadata = false;
  if (finalization_plan.storage_write_intent.reset_reward_votes) {
    try {
      first_persistence_stages.push_back(
          vote_mgr_->rewardVotesResetStageForFinalization(finalization_plan.storage_write_intent));
      should_commit_reward_vote_metadata = true;
    } catch (const std::exception &e) {
      LOG(log_er_) << "Rust PBFT finalized-period reward-vote reset facts failed for block " << pbft_block_hash
                   << ", period " << block_pbft_period << ": " << e.what();
      report_runtime_action(runtime_step, false, 255);
      return false;
    }
  }

  // pass pbft with dag blocks and transactions to adjust difficulty
  std::optional<SortitionParamsChange> prepared_sortition_params_change;
  bool should_commit_sortition_runtime = false;
  if (finalization_plan.storage_write_intent.update_sortition_params) {
    prepared_sortition_params_change = dag_mgr_->sortitionParamsManager().prepareBlockForSortitionFinalization(
        period_data, pbft_chain_->getPbftChainSizeExcludingEmptyPbftBlocks() + 1);
    should_commit_sortition_runtime = true;
    if (prepared_sortition_params_change.has_value()) {
      first_persistence_stages.push_back(makeSortitionFinalizationStorageStage(*prepared_sortition_params_change));
    }
  }

  {
    // This makes sure that no DAG block or transaction can be added or change state in transaction and dag manager
    // when finalizing pbft block with dag blocks and transactions
    std::unique_lock dag_lock(dag_mgr_->getDagMutex());
    std::unique_lock trx_lock(trx_mgr_->getTransactionsMutex());

    rustaxa::PbftFinalizedPeriodApplyResult primary_storage_result{};
    try {
      primary_storage_result = rustaxa::pbft_manager_runtime_apply_finalization_storage_writes(
          *pbft_manager_runtime_.value(), finalization_plan.storage_write_intent, std::move(first_persistence_stages),
          false);
    } catch (const std::exception &e) {
      LOG(log_er_) << "Rust PBFT finalized-period storage apply failed for block " << pbft_block_hash << ", period "
                   << block_pbft_period << ": " << e.what();
      report_runtime_action(runtime_step, false, 255);
      return false;
    }
    if (primary_storage_result.status != kPbftFinalizedPeriodApplyStatusApplied &&
        primary_storage_result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
      LOG(log_er_) << "Rust PBFT finalized-period storage apply rejected block " << pbft_block_hash << ", period "
                   << block_pbft_period << ", status " << static_cast<uint32_t>(primary_storage_result.status)
                   << ", error " << static_cast<std::string>(primary_storage_result.error_code);
      report_runtime_action(runtime_step, false, primary_storage_result.status);
      return false;
    }
    if (!report_runtime_action(runtime_step, true, primary_storage_result.status)) {
      return false;
    }
    if (should_commit_sortition_runtime) {
      if (!begin_runtime_action(kPbftFinalizationRuntimeActionCommitSortitionRuntime, runtime_step)) {
        return false;
      }
      const auto sortition_report = dag_mgr_->sortitionParamsManager().commitPreparedBlockForSortitionFinalization(
          period_data, pbft_chain_->getPbftChainSizeExcludingEmptyPbftBlocks() + 1, prepared_sortition_params_change,
          finalization_plan.storage_write_intent);
      const auto live_validation = validate_live_mutation(sortition_report);
      if (!live_validation.accepted) {
        LOG(log_er_) << "Rust PBFT finalization sortition live mutation rejected for block " << pbft_block_hash
                     << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                     << ", error " << static_cast<std::string>(live_validation.error_code);
      }
      if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                        static_cast<std::string>(live_validation.error_code))) {
        return false;
      }
    }
    if (should_commit_reward_vote_metadata) {
      if (!begin_runtime_action(kPbftFinalizationRuntimeActionCommitRewardVotesReset, runtime_step)) {
        return false;
      }
      const auto reward_votes_report =
          vote_mgr_->commitRewardVotesResetForFinalization(finalization_plan.storage_write_intent);
      const auto live_validation = validate_live_mutation(reward_votes_report);
      if (!live_validation.accepted) {
        LOG(log_er_) << "Rust PBFT finalization reward-vote live mutation rejected for block " << pbft_block_hash
                     << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                     << ", error " << static_cast<std::string>(live_validation.error_code);
      }
      if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                        static_cast<std::string>(live_validation.error_code))) {
        return false;
      }
    }

    // Set DAG blocks period
    auto const &anchor_hash = period_data.pbft_blk->getPivotDagBlockHash();
    if (finalization_plan.cleanup.set_dag_block_order) {
      if (!begin_runtime_action(kPbftFinalizationRuntimeActionSetDagBlockOrder, runtime_step)) {
        return false;
      }
      const auto dag_report = dag_mgr_->setDagBlockOrderForPbftFinalization(
          anchor_hash, block_pbft_period, dag_blocks_order, finalization_plan.storage_write_intent);
      const auto live_validation = validate_live_mutation(dag_report);
      if (!live_validation.accepted) {
        LOG(log_er_) << "Rust PBFT finalization DAG live mutation rejected for block " << pbft_block_hash << ", period "
                     << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status) << ", error "
                     << static_cast<std::string>(live_validation.error_code);
      }
      if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                        static_cast<std::string>(live_validation.error_code))) {
        return false;
      }
    }

    if (finalization_plan.cleanup.update_finalized_transactions_status) {
      if (!begin_runtime_action(kPbftFinalizationRuntimeActionUpdateFinalizedTransactions, runtime_step)) {
        return false;
      }
      const auto finalized_transaction_report = trx_mgr_->updateFinalizedTransactionsStatusForPbftFinalization(
          period_data, finalization_plan.storage_write_intent);
      const auto live_validation = validate_live_mutation(finalized_transaction_report);
      if (!live_validation.accepted) {
        LOG(log_er_) << "Rust PBFT finalization transaction live mutation rejected for block " << pbft_block_hash
                     << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                     << ", error " << static_cast<std::string>(live_validation.error_code);
      }
      if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                        static_cast<std::string>(live_validation.error_code))) {
        return false;
      }
    }

    // update PBFT chain size
    if (finalization_plan.cleanup.update_pbft_chain) {
      if (!begin_runtime_action(kPbftFinalizationRuntimeActionUpdatePbftChain, runtime_step)) {
        return false;
      }
      const auto pbft_chain_report =
          pbft_chain_->updatePbftChainForPbftFinalization(finalization_plan.storage_write_intent);
      const auto live_validation = validate_live_mutation(pbft_chain_report);
      if (!live_validation.accepted) {
        LOG(log_er_) << "Rust PBFT finalization PBFT-chain live mutation rejected for block " << pbft_block_hash
                     << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                     << ", error " << static_cast<std::string>(live_validation.error_code);
      }
      if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                        static_cast<std::string>(live_validation.error_code))) {
        return false;
      }
    }
  }

  // anchor_dag_block_order_cache_ is valid in one period, clear when period changes
  if (finalization_plan.cleanup.clear_anchor_dag_cache) {
    if (!begin_runtime_action(kPbftFinalizationRuntimeActionClearAnchorDagCache, runtime_step)) {
      return false;
    }
    anchor_dag_block_order_cache_.clear();
    const auto clear_cache_snapshot =
        rustaxa::pbft_manager_runtime_clear_cached_anchor_dag_order(*pbft_manager_runtime_.value());
    ensurePbftManagerRuntimeSnapshotReady(clear_cache_snapshot, "Clear cached PBFT DAG order anchors");
    rustaxa::PbftFinalizationLiveMutationReport clear_cache_report{};
    clear_cache_report.action = kPbftFinalizationRuntimeActionClearAnchorDagCache;
    clear_cache_report.block_period = finalization_plan.storage_write_intent.block_period;
    clear_cache_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
    clear_cache_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
    clear_cache_report.anchor_dag_cache_count =
        rustaxa::pbft_manager_runtime_cached_anchor_dag_order_count(*pbft_manager_runtime_.value());
    const auto live_validation = validate_live_mutation(clear_cache_report);
    if (!live_validation.accepted) {
      LOG(log_er_) << "Rust PBFT finalization anchor-cache live mutation rejected for block " << pbft_block_hash
                   << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                   << ", error " << static_cast<std::string>(live_validation.error_code);
    }
    if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                      static_cast<std::string>(live_validation.error_code))) {
      return false;
    }
  }

  LOG(log_nf_) << "Pushed new PBFT block " << pbft_block_hash << " into chain. Period: " << block_pbft_period
               << ", round: " << block_pbft_round;

  uint32_t blocks_per_year{0};
  // Dynamic lambda was introduced in cacti hardfork -> it affects the number of blocks generated per year, which
  // affects rewards distribution
  if (finalization_plan.storage_write_intent.apply_dynamic_lambda_update) {
    if (!begin_runtime_action(kPbftFinalizationRuntimeActionApplyDynamicLambda, runtime_step)) {
      return false;
    }
    blocks_per_year = finalization_plan.storage_write_intent.blocks_per_year;

    rustaxa::PbftFinalizedPeriodApplyResult dynamic_lambda_result{};
    try {
      auto dynamic_lambda_stage = makeFinalizationStorageStage(kPbftFinalizationStorageStageDynamicLambda);
      dynamic_lambda_stage.rounds_count_dynamic_lambda = dynamic_lambda_plan.rounds_count_dynamic_lambda;
      dynamic_lambda_stage.dynamic_lambda = dynamic_lambda_plan.dynamic_lambda;
      rust::Vec<rustaxa::PbftFinalizationStorageWriteStage> dynamic_lambda_stages;
      dynamic_lambda_stages.push_back(std::move(dynamic_lambda_stage));
      dynamic_lambda_result = rustaxa::pbft_manager_runtime_apply_finalization_storage_writes(
          *pbft_manager_runtime_.value(), finalization_plan.storage_write_intent, std::move(dynamic_lambda_stages),
          false);
    } catch (const std::exception &e) {
      LOG(log_er_) << "Rust PBFT dynamic-lambda storage appender failed for block " << pbft_block_hash << ", period "
                   << block_pbft_period << ": " << e.what();
      report_runtime_action(runtime_step, false, 255);
      return false;
    }
    if (dynamic_lambda_result.status != kPbftFinalizedPeriodApplyStatusApplied &&
        dynamic_lambda_result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
      LOG(log_er_) << "Rust PBFT dynamic-lambda storage appender rejected block " << pbft_block_hash << ", period "
                   << block_pbft_period << ", status " << static_cast<uint32_t>(dynamic_lambda_result.status)
                   << ", error " << static_cast<std::string>(dynamic_lambda_result.error_code);
      report_runtime_action(runtime_step, false, dynamic_lambda_result.status);
      return false;
    }
    const auto dynamic_lambda_snapshot = rustaxa::pbft_manager_runtime_apply_dynamic_lambda(
        *pbft_manager_runtime_.value(), dynamic_lambda_plan.rounds_count_dynamic_lambda,
        dynamic_lambda_plan.dynamic_lambda);
    if (dynamic_lambda_snapshot.status != kPbftManagerStartupRestoreStatusReady) {
      LOG(log_er_) << "Rust PBFT dynamic-lambda live-state update failed for block " << pbft_block_hash << ", period "
                   << block_pbft_period << ", status " << static_cast<uint32_t>(dynamic_lambda_snapshot.status);
      report_runtime_action(runtime_step, false, 255);
      return false;
    }
    applyPbftManagerRuntimeSnapshot(dynamic_lambda_snapshot, round_, step_, state_, current_round_lambda_,
                                    next_step_time_ms_, rounds_count_dynamic_lambda_, dynamic_lambda_,
                                    executed_pbft_block_, already_next_voted_value_,
                                    already_next_voted_null_block_hash_, broadcast_votes_counter_,
                                    rebroadcast_votes_counter_, broadcast_reward_votes_counter_,
                                    rebroadcast_reward_votes_counter_);
    if (dynamic_lambda_plan.decreased_dynamic_lambda) {
      LOG(log_nf_) << "Decrease dynamic_lambda by " << kGenesisConfig.state.hardforks.cacti_hf.lambda_change << " to "
                   << dynamic_lambda_ << ", period " << block_pbft_period << ", round " << block_pbft_round;
    }
    if (dynamic_lambda_plan.increased_dynamic_lambda) {
      LOG(log_nf_) << "Increase dynamic_lambda by " << kGenesisConfig.state.hardforks.cacti_hf.lambda_change << " to "
                   << dynamic_lambda_ << ", period " << block_pbft_period << ", round " << block_pbft_round;
    }
    rustaxa::PbftFinalizationLiveMutationReport dynamic_lambda_report{};
    dynamic_lambda_report.action = kPbftFinalizationRuntimeActionApplyDynamicLambda;
    dynamic_lambda_report.block_period = finalization_plan.storage_write_intent.block_period;
    dynamic_lambda_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
    dynamic_lambda_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
    dynamic_lambda_report.rounds_count_dynamic_lambda = dynamic_lambda_snapshot.rounds_count_dynamic_lambda;
    dynamic_lambda_report.dynamic_lambda = dynamic_lambda_snapshot.dynamic_lambda_ms;
    const auto live_validation = validate_live_mutation(dynamic_lambda_report);
    if (!live_validation.accepted) {
      LOG(log_er_) << "Rust PBFT finalization dynamic-lambda live mutation rejected for block " << pbft_block_hash
                   << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                   << ", error " << static_cast<std::string>(live_validation.error_code);
    }
    const auto action_status = live_validation.accepted ? dynamic_lambda_result.status : live_validation.status;
    const auto action_error =
        live_validation.accepted ? std::string{} : static_cast<std::string>(live_validation.error_code);
    if (!report_runtime_action_detail(runtime_step, live_validation.accepted, action_status, action_error)) {
      return false;
    }
  } else {
    blocks_per_year = finalization_plan.storage_write_intent.blocks_per_year;
  }

  if (finalization_plan.cleanup.finalize_final_chain) {
    if (!begin_runtime_action(kPbftFinalizationRuntimeActionFinalizeFinalChain, runtime_step)) {
      return false;
    }
    finalize_(std::move(period_data), std::move(dag_blocks_order), blocks_per_year);
    rustaxa::PbftFinalizationLiveMutationReport final_chain_report{};
    final_chain_report.action = kPbftFinalizationRuntimeActionFinalizeFinalChain;
    final_chain_report.block_period = finalization_plan.storage_write_intent.block_period;
    final_chain_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
    final_chain_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
    final_chain_report.final_chain_dispatched = true;
    final_chain_report.final_chain_blocks_per_year = blocks_per_year;
    final_chain_report.final_chain_last_block = final_chain_->lastBlockNumber();
    const auto live_validation = validate_live_mutation(final_chain_report);
    if (!live_validation.accepted) {
      LOG(log_er_) << "Rust PBFT finalization FinalChain dispatch report rejected for block " << pbft_block_hash
                   << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                   << ", error " << static_cast<std::string>(live_validation.error_code);
    }
    if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                      static_cast<std::string>(live_validation.error_code))) {
      return false;
    }
  }

  if (finalization_plan.executed_pbft_block) {
    if (finalization_plan.storage_write_intent.persist_executed_pbft_status) {
      if (!begin_runtime_action(kPbftFinalizationRuntimeActionPersistExecutedStatus, runtime_step)) {
        return false;
      }
      rustaxa::PbftFinalizedPeriodApplyResult executed_status_result{};
      try {
        rust::Vec<rustaxa::PbftFinalizationStorageWriteStage> executed_status_stages;
        executed_status_stages.push_back(makeFinalizationStorageStage(kPbftFinalizationStorageStageExecutedStatus));
        executed_status_result = rustaxa::pbft_manager_runtime_apply_finalization_storage_writes(
            *pbft_manager_runtime_.value(), finalization_plan.storage_write_intent,
            std::move(executed_status_stages), false);
      } catch (const std::exception &e) {
        LOG(log_er_) << "Rust PBFT executed-status storage appender failed for block " << pbft_block_hash << ", period "
                     << block_pbft_period << ": " << e.what();
        report_runtime_action(runtime_step, false, 255);
        return false;
      }
      if (executed_status_result.status != kPbftFinalizedPeriodApplyStatusApplied &&
          executed_status_result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
        LOG(log_er_) << "Rust PBFT executed-status storage appender rejected block " << pbft_block_hash << ", period "
                     << block_pbft_period << ", status " << static_cast<uint32_t>(executed_status_result.status)
                     << ", error " << static_cast<std::string>(executed_status_result.error_code);
        report_runtime_action(runtime_step, false, executed_status_result.status);
        return false;
      }
      if (!report_runtime_action(runtime_step, true, executed_status_result.status)) {
        return false;
      }
    }
    if (!begin_runtime_action(kPbftFinalizationRuntimeActionSetExecutedFlag, runtime_step)) {
      return false;
    }
    const auto executed_status_snapshot = rustaxa::pbft_manager_runtime_apply_finalization_executed_status(
        *pbft_manager_runtime_.value(), finalization_plan.storage_write_intent);
    applyPbftManagerRuntimeSnapshot(executed_status_snapshot, round_, step_, state_, current_round_lambda_,
                                    next_step_time_ms_, rounds_count_dynamic_lambda_, dynamic_lambda_,
                                    executed_pbft_block_, already_next_voted_value_, already_next_voted_null_block_hash_,
                                    broadcast_votes_counter_, rebroadcast_votes_counter_,
                                    broadcast_reward_votes_counter_, rebroadcast_reward_votes_counter_);
    rustaxa::PbftFinalizationLiveMutationReport executed_report{};
    executed_report.action = kPbftFinalizationRuntimeActionSetExecutedFlag;
    executed_report.block_period = finalization_plan.storage_write_intent.block_period;
    executed_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
    executed_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
    executed_report.executed_pbft_block = executed_status_snapshot.executed_pbft_block;
    const auto live_validation = validate_live_mutation(executed_report);
    if (!live_validation.accepted) {
      LOG(log_er_) << "Rust PBFT finalization executed-flag live mutation rejected for block " << pbft_block_hash
                   << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                   << ", error " << static_cast<std::string>(live_validation.error_code);
    }
    if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                      static_cast<std::string>(live_validation.error_code))) {
      return false;
    }
  }

  // Advance pbft consensus period
  if (finalization_plan.cleanup.advance_period) {
    if (!begin_runtime_action(kPbftFinalizationRuntimeActionAdvancePeriod, runtime_step)) {
      return false;
    }
    if (!applyRustPlannedAdvancePeriod_(finalization_plan.storage_write_intent.block_period)) {
      report_runtime_action(runtime_step, false, 255);
      return false;
    }
    rustaxa::PbftFinalizationLiveMutationReport advance_report{};
    advance_report.action = kPbftFinalizationRuntimeActionAdvancePeriod;
    advance_report.block_period = finalization_plan.storage_write_intent.block_period;
    advance_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
    advance_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
    advance_report.manager_period = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value()).period;
    const auto live_validation = validate_live_mutation(advance_report);
    if (!live_validation.accepted) {
      LOG(log_er_) << "Rust PBFT finalization advance-period live mutation rejected for block " << pbft_block_hash
                   << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                   << ", error " << static_cast<std::string>(live_validation.error_code);
    }
    if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                      static_cast<std::string>(live_validation.error_code))) {
      return false;
    }
  }

  if (finalization_plan.cleanup.process_pillar_block) {
    if (!begin_runtime_action(kPbftFinalizationRuntimeActionProcessPillarBlock, runtime_step)) {
      return false;
    }
    assert(block_pbft_period == pbft_chain_->getPbftChainSize());
    const auto pillar_request_period = block_pbft_period - final_chain_->delegationDelay();
    processPillarBlock(block_pbft_period);
    rustaxa::PbftFinalizationLiveMutationReport pillar_report{};
    pillar_report.action = kPbftFinalizationRuntimeActionProcessPillarBlock;
    pillar_report.block_period = finalization_plan.storage_write_intent.block_period;
    pillar_report.pbft_block_hash = finalization_plan.storage_write_intent.pbft_block_hash;
    pillar_report.anchor_hash = finalization_plan.storage_write_intent.anchor_hash;
    pillar_report.manager_period = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value()).period;
    pillar_report.pillar_processed_period = block_pbft_period;
    pillar_report.pillar_request_period = pillar_request_period;
    const auto live_validation = validate_live_mutation(pillar_report);
    if (!live_validation.accepted) {
      LOG(log_er_) << "Rust PBFT finalization pillar live mutation rejected for block " << pbft_block_hash
                   << ", period " << block_pbft_period << ", status " << static_cast<uint32_t>(live_validation.status)
                   << ", error " << static_cast<std::string>(live_validation.error_code);
    }
    if (!report_runtime_action_detail(runtime_step, live_validation.accepted, live_validation.status,
                                      static_cast<std::string>(live_validation.error_code))) {
      return false;
    }
  }

  const auto final_runtime_step = finalization_runtime_session->pbft_finalization_runtime_session_next();
  if (!final_runtime_step.complete || final_runtime_step.status != kPbftFinalizationRuntimeStatusComplete) {
    LOG(log_er_) << "Rust PBFT finalization runtime did not complete for block " << pbft_block_hash << ", period "
                 << block_pbft_period << ", status " << static_cast<uint32_t>(final_runtime_step.status) << ", action "
                 << static_cast<uint32_t>(final_runtime_step.action) << ", error "
                 << static_cast<std::string>(final_runtime_step.error_code);
    finalization_runtime_session->abort_pbft_finalization_runtime_session();
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
  assert(final_chain_->lastBlockNumber() >= request_period);

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
  return sync_queue_.syncingPeriod(pbft_chain_->getPbftChainSize());
}

PbftManager::PbftSyncEgressPayload PbftManager::getPbftSyncEgressPayload(PbftPeriod period, bool last_block,
                                                                         bool pbft_chain_synced,
                                                                         bool reward_votes_present,
                                                                         PbftPeriod reward_votes_period) const {
  if (!pbft_manager_runtime_.has_value()) {
    throw std::runtime_error("PBFT manager Rust runtime must be initialized before serving PBFT sync egress payload");
  }
  const auto payload =
      rustaxa::load_pbft_sync_egress_payload(*pbft_manager_runtime_.value(), period, last_block, pbft_chain_synced,
                                             reward_votes_present, reward_votes_period);
  return {dev::bytes(payload.period_data_rlp.begin(), payload.period_data_rlp.end()), payload.attach_reward_votes};
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
  auto popped_period_data = sync_queue_.popWithMetadata();
  auto period_data = std::move(popped_period_data.period_data);
  auto cert_votes = std::move(popped_period_data.cert_votes);
  const auto node_id = popped_period_data.node_id;
  const auto pbft_block_hash = popped_period_data.block_hash;
  const auto block_period = popped_period_data.period;
  const auto block_prev_hash = popped_period_data.prev_block_hash;
  const auto anchor_hash = popped_period_data.pivot_hash;
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
  auto make_runtime_plan = [&](bool candidate_block_in_chain, uint8_t final_chain_status, uint8_t reward_votes_status,
                               uint8_t cert_votes_status, uint8_t transactions_status,
                               const std::unordered_set<trx_hash_t> &non_finalized_transactions,
                               bool contains_finalized_transactions, uint8_t pillar_data_status,
                               bool pillar_votes_required, uint8_t pillar_votes_status) {
    return rustaxa::plan_pbft_sync_process_period_data_runtime(makePbftSyncProcessPeriodDataRuntimeFact(
        block_period, block_prev_hash, dag_transaction_hashes, period_data_transaction_hashes,
        last_pbft_block_hash, last_pbft_block_period, candidate_block_in_chain, final_chain_status, reward_votes_status,
        cert_votes_status, transactions_status, non_finalized_transactions,
        contains_finalized_transactions, pillar_data_status, pillar_votes_required, pillar_votes_status,
        previous_cert_votes_present, previous_cert_first_vote_has_weight, extra_data_required, extra_data_present,
        extra_data_pillar_block_hash_present, pillar_votes_present));
  };

  auto runtime_plan =
      make_runtime_plan(block_in_chain, kPbftSyncFinalChainNotChecked, kPbftSyncFactNotChecked, kPbftSyncFactNotChecked,
                        kPbftSyncFactNotChecked, {}, false, kPbftSyncFactNotChecked, false, kPbftSyncFactNotChecked);
  auto admission_plan = runtime_plan;
  auto throw_on_runtime_contract_error = [&]() {
    if (admission_plan.runtime_action == kPbftSyncRuntimeActionContractError) {
      throw std::runtime_error("Rust PBFT sync runtime planner received invalid bridge facts");
    }
  };
  throw_on_runtime_contract_error();
  if (admission_plan.status == kPbftSyncStatusBlockAlreadyInChain) {
    LOG(log_dg_) << "PBFT block " << pbft_block_hash << " already present in chain.";
    return std::nullopt;
  }

  auto net = network_.lock();
  assert(net);  // Should never happen
  auto apply_rust_admission_side_effects = [&]() {
    if (admission_plan.clear_sync_queue) {
      sync_queue_.clear();
    }
    if (admission_plan.report_malicious_peer) {
      net->handleMaliciousSyncPeer(node_id);
    }
  };
  auto finish_non_accepting_rust_admission =
      [&]() -> std::optional<std::pair<PeriodData, std::vector<std::shared_ptr<PbftVote>>>> {
    if (admission_plan.accept_period_data) {
      throw std::runtime_error("Rust PBFT sync runtime accepted period data on a C++ rejection path");
    }
    if (admission_plan.wait_for_finalization) {
      final_chain_->waitForFinalized();
    }
    apply_rust_admission_side_effects();
    return std::nullopt;
  };

  if (admission_plan.status == kPbftSyncStatusStalePeriod) {
    return finish_non_accepting_rust_admission();
  }
  if (admission_plan.status == kPbftSyncStatusPreviousHashMismatch) {
    LOG(log_er_) << "Invalid PBFT block " << pbft_block_hash << "; prevHash: " << block_prev_hash << " from peer "
                 << node_id.abridged()
                 << " received, stop syncing.";
    return finish_non_accepting_rust_admission();
  }

  auto validate_final_chain_hash_from_queue_metadata = [&]() {
    const auto facts = final_chain_->rustFinalChainForRust().collect_pbft_final_chain_facts(
        makePbftFinalChainFactRequest(block_period, final_chain_hash, true, true, false, false));
    if (facts.final_chain_hash.status == kPbftSyncFinalChainMissing) {
      LOG(log_wr_) << "Block " << pbft_block_hash << " could not be validated as we are behind";
      return PbftStateRootValidation::Missing;
    }
    if (facts.final_chain_hash.status == kPbftSyncFinalChainInvalid) {
      LOG(log_er_) << "Block " << block_period << " hash " << pbft_block_hash << " state root " << final_chain_hash
                   << " isn't matching actual " << fromBridgeHash(facts.final_chain_hash.expected_hash);
      return PbftStateRootValidation::Invalid;
    }

    return PbftStateRootValidation::Valid;
  };

  const auto rust_pillar_data_plan =
      make_runtime_plan(false, kPbftSyncFinalChainValid, kPbftSyncFactValid, kPbftSyncFactValid, kPbftSyncFactValid,
                        {}, false, kPbftSyncFactNotChecked, pillar_votes_required, kPbftSyncFactNotRequired);
  const auto rust_pillar_data_valid = rust_pillar_data_plan.status != kPbftSyncStatusPillarDataInvalid;

  std::optional<VoteManager::RewardVoteValidationResult> reward_votes;
  auto block_validation_fact = rustaxa::PbftManagerBlockValidationFact{};
  block_validation_fact.block_hash = toBridgeHash(pbft_block_hash);
  block_validation_fact.period = block_period;
  block_validation_fact.pivot_hash = toBridgeHash(anchor_hash);
  block_validation_fact.pivot_is_null = anchor_hash == kNullBlockHash;
  block_validation_fact.dag_order_cached = rustaxa::pbft_manager_runtime_has_cached_anchor_dag_order(
      *pbft_manager_runtime_.value(), toBridgeHash(anchor_hash));
  block_validation_fact.dag_order_required = false;
  block_validation_fact.pillar_block_required = false;
  block_validation_fact.dag_weight_check_required = false;
  block_validation_fact.pbft_chain_status = kPbftManagerBlockValidationFactValid;
  block_validation_fact.final_chain_hash_status = kPbftManagerBlockValidationFactNotChecked;
  block_validation_fact.reward_votes_status = kPbftManagerBlockValidationFactNotChecked;
  block_validation_fact.extra_data_status =
      rust_pillar_data_valid ? kPbftManagerBlockValidationFactValid : kPbftManagerBlockValidationFactInvalid;
  block_validation_fact.pillar_block_status = kPbftManagerBlockValidationFactNotRequired;
  block_validation_fact.dag_order_status = kPbftManagerBlockValidationFactNotRequired;
  block_validation_fact.dag_weight_status = kPbftManagerBlockValidationFactNotRequired;

  bool retry_logged = false;
  auto block_validation_session = rustaxa::create_pbft_manager_block_validation_session(block_validation_fact);
  auto validation_plan = block_validation_session->pbft_manager_block_validation_session_next();
  while (true) {
    if (validation_plan.action == kPbftManagerBlockValidationActionAccept) {
      break;
    }
    if (validation_plan.action == kPbftManagerBlockValidationActionWaitForFinalization) {
      // If syncing and pbft manager is faster than execution a delay might be needed to allow EVM to catch up
      final_chain_->waitForFinalized();
      if (!retry_logged) {
        LOG(log_wr_) << "PBFT block " << pbft_block_hash
                     << " validation delayed, state root missing, execution is behind";
        retry_logged = true;
      }
      validation_plan = block_validation_session->pbft_manager_block_validation_session_report(
          kPbftManagerBlockValidationFactNotChecked, false);
      continue;
    }
    if (validation_plan.action == kPbftManagerBlockValidationActionReject) {
      if (validation_plan.status == kPbftManagerBlockValidationStatusFinalChainHashInvalid) {
        runtime_plan = make_runtime_plan(false, kPbftSyncFinalChainInvalid, kPbftSyncFactNotChecked,
                                         kPbftSyncFactNotChecked, kPbftSyncFactNotChecked, {}, false,
                                         kPbftSyncFactNotChecked, false, kPbftSyncFactNotChecked);
        admission_plan = runtime_plan;
        throw_on_runtime_contract_error();
        LOG(log_er_) << "Failed verifying block " << pbft_block_hash << " with invalid state root: "
                     << final_chain_hash << ". Disconnect malicious peer " << node_id.abridged();
        return finish_non_accepting_rust_admission();
      }
      if (validation_plan.status == kPbftManagerBlockValidationStatusRewardVotesInvalid) {
        runtime_plan = make_runtime_plan(false, kPbftSyncFinalChainValid, kPbftSyncFactInvalid, kPbftSyncFactNotChecked,
                                         kPbftSyncFactNotChecked, {}, false, kPbftSyncFactNotChecked, false,
                                         kPbftSyncFactNotChecked);
        admission_plan = runtime_plan;
        throw_on_runtime_contract_error();
        LOG(log_er_) << "Failed verifying reward votes for block " << pbft_block_hash << ". Disconnect malicious peer "
                     << node_id.abridged();
        return finish_non_accepting_rust_admission();
      }
      if (validation_plan.status == kPbftManagerBlockValidationStatusExtraDataInvalid) {
        runtime_plan = make_runtime_plan(false, kPbftSyncFinalChainValid, kPbftSyncFactValid, kPbftSyncFactValid,
                                         kPbftSyncFactValid, {}, false, kPbftSyncFactInvalid, pillar_votes_required,
                                         kPbftSyncFactNotChecked);
        admission_plan = runtime_plan;
        throw_on_runtime_contract_error();
        LOG(log_er_) << "Synced PBFT block " << pbft_block_hash << " has invalid pillar data";
        return finish_non_accepting_rust_admission();
      }

      throw std::runtime_error("Rust PBFT block validation planner returned unsupported sync rejection: " +
                               std::string(validation_plan.error_code));
    }
    if (validation_plan.action == kPbftManagerBlockValidationActionContractError) {
      throw std::runtime_error("Rust PBFT block validation planner rejected sync bridge facts: " +
                               std::string(validation_plan.error_code));
    }
    if (validation_plan.action != kPbftManagerBlockValidationActionRunCheck) {
      throw std::runtime_error("Rust PBFT block validation planner returned unknown sync action");
    }

    if (validation_plan.next_check == kPbftManagerBlockValidationCheckFinalChainHash) {
      const auto validation_result = validate_final_chain_hash_from_queue_metadata();
      if (validation_result == PbftStateRootValidation::Valid) {
        validation_plan = block_validation_session->pbft_manager_block_validation_session_report(
            kPbftManagerBlockValidationFactValid, false);
      } else if (validation_result == PbftStateRootValidation::Missing) {
        validation_plan = block_validation_session->pbft_manager_block_validation_session_report(
            kPbftManagerBlockValidationFactMissing, false);
      } else {
        validation_plan = block_validation_session->pbft_manager_block_validation_session_report(
            kPbftManagerBlockValidationFactInvalid, false);
      }
      continue;
    }

    if (validation_plan.next_check == kPbftManagerBlockValidationCheckRewardVotes) {
      reward_votes =
          vote_mgr_->checkRewardVotesDetailed(block_period, pbft_block_hash, block_prev_hash, reward_vote_hashes, true);
      validation_plan = block_validation_session->pbft_manager_block_validation_session_report(
          reward_votes->accepted ? kPbftManagerBlockValidationFactValid : kPbftManagerBlockValidationFactInvalid,
          false);
      continue;
    }

    if (validation_plan.next_check == kPbftManagerBlockValidationCheckExtraData) {
      validation_plan = block_validation_session->pbft_manager_block_validation_session_report(
          rust_pillar_data_valid ? kPbftManagerBlockValidationFactValid : kPbftManagerBlockValidationFactInvalid,
          false);
      continue;
    }

    throw std::runtime_error("Rust PBFT block validation planner requested unsupported sync check");
  }

  assert(reward_votes.has_value());
  runtime_plan =
      make_runtime_plan(false, kPbftSyncFinalChainValid, kPbftSyncFactValid, kPbftSyncFactNotChecked,
                        kPbftSyncFactNotChecked, {}, false, kPbftSyncFactNotChecked, false, kPbftSyncFactNotChecked);
  admission_plan = runtime_plan;
  throw_on_runtime_contract_error();

  // Special case when previous block was already in chain so we hit condition
  // pbft_chain_->findPbftBlockInChain(pbft_block_hash) and it's cert votes were not verified here, they are part of
  // vote_manager so we need to replace them as they are not verified period_data structure
  if (admission_plan.replace_previous_block_cert_votes) {
    period_data.previous_block_cert_votes = std::move(reward_votes->votes);
  }

  // Validate cert votes
  const auto cert_votes_valid = validatePbftBlockCertVotes(block_period, pbft_block_hash, cert_votes);
  runtime_plan = make_runtime_plan(
      false, kPbftSyncFinalChainValid, kPbftSyncFactValid, cert_votes_valid ? kPbftSyncFactValid : kPbftSyncFactInvalid,
      kPbftSyncFactNotChecked, {}, false, kPbftSyncFactNotChecked, false, kPbftSyncFactNotChecked);
  admission_plan = runtime_plan;
  throw_on_runtime_contract_error();
  if (admission_plan.status == kPbftSyncStatusCertVotesInvalid) {
    LOG(log_er_) << "Synced PBFT block " << pbft_block_hash
                 << " doesn't have enough valid cert votes. Clear synced PBFT blocks!";
    return finish_non_accepting_rust_admission();
  }

  // Execute the Rust-planned finalized-transaction lookup against the live transaction manager. The classification of
  // non-fatal transaction warnings is returned by the Rust admission plan after this executor reports compact facts.
  auto non_finalized_transactions = trx_mgr_->excludeFinalizedTransactions(
      fromBridgeTransactionHashes(runtime_plan.transaction_query_plan.finalized_lookup_hashes));
  const auto contains_finalized_transactions =
      !trx_mgr_->verifyTransactionsNotFinalized(std::move(period_data_transaction_identities));

  runtime_plan = make_runtime_plan(false, kPbftSyncFinalChainValid, kPbftSyncFactValid, kPbftSyncFactValid,
                                   kPbftSyncFactValid, non_finalized_transactions, contains_finalized_transactions,
                                   kPbftSyncFactNotChecked, pillar_votes_required, kPbftSyncFactNotChecked);
  admission_plan = runtime_plan;
  throw_on_runtime_contract_error();
  if (admission_plan.status == kPbftSyncStatusPillarDataInvalid) {
    LOG(log_er_) << "Synced PBFT block " << pbft_block_hash << " has invalid pillar data";
    return finish_non_accepting_rust_admission();
  }

  // Validate pillar votes
  bool pillar_votes_valid = true;
  if (pillar_votes_required) {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
    const auto rust_validation_result =
        validatePbftBlockPillarVotesWithRust(block_period, pillar_vote_rlps, period_data.pillar_votes_,
                                             pillar_chain_mgr_, final_chain_);
    if (!rust_validation_result.valid()) {
      LOG(log_er_) << "Rust sync pillar-vote validation failed, pbft block period " << block_period << ", status "
                   << validatePbftBlockPillarVotesWithRustStatusString(rust_validation_result.status)
                   << ", plan status " << static_cast<uint32_t>(rust_validation_result.plan_status)
                   << ", first bad vote " << rust_validation_result.first_bad_vote_hash;
    }
    pillar_votes_valid = rust_validation_result.valid();
#else
    pillar_votes_valid = validatePbftBlockPillarVotes(period_data);
#endif
  }
  runtime_plan = make_runtime_plan(
      false, kPbftSyncFinalChainValid, kPbftSyncFactValid, kPbftSyncFactValid, kPbftSyncFactValid,
      non_finalized_transactions, contains_finalized_transactions, kPbftSyncFactValid, pillar_votes_required,
      pillar_votes_required ? (pillar_votes_valid ? kPbftSyncFactValid : kPbftSyncFactInvalid)
                            : kPbftSyncFactNotRequired);
  admission_plan = runtime_plan;
  throw_on_runtime_contract_error();
  if (admission_plan.status == kPbftSyncStatusPillarVotesInvalid) {
    LOG(log_er_) << "Synced PBFT block " << pbft_block_hash << ", period " << block_period
                 << " doesn't have enough valid pillar votes. Clear synced PBFT blocks!";
    return finish_non_accepting_rust_admission();
  }

  if (!admission_plan.accept_period_data) {
    return finish_non_accepting_rust_admission();
  }

  for (const auto &warning : admission_plan.warnings) {
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
  if (admission_plan.contains_finalized_transaction_warning) {
    LOG(log_er_) << "Synced PBFT block " << pbft_block_hash << " has finalized transactions";
  }

  try {
    period_data.transactions = materializeTransactionsFromQueuedRlps(transaction_rlps, period_data_transaction_hashes);
  } catch (const std::exception &e) {
    LOG(log_er_) << "Synced PBFT block " << pbft_block_hash
                 << " has invalid queued transaction payload metadata: " << e.what();
    apply_rust_admission_side_effects();
    return std::nullopt;
  }

  return std::optional<std::pair<PeriodData, std::vector<std::shared_ptr<PbftVote>>>>(
      {std::move(period_data), std::move(cert_votes)});
}

bool PbftManager::validatePbftBlockCertVotes(PbftPeriod block_period, const blk_hash_t &block_hash,
                                             const std::vector<std::shared_ptr<PbftVote>> &cert_votes) const {
  // To speed up syncing/rebuilding full strict vote verification is done for all votes on every
  // full_vote_validation_interval and for a random vote for each block
  auto make_cert_vote_bundle_fact = [&](bool check_weight_threshold, bool two_t_plus_one_found,
                                        uint64_t two_t_plus_one) {
    rustaxa::PbftSyncCertVoteBundleFact fact;
    fact.block_period = block_period;
    fact.block_hash = toBridgeHash(block_hash);
    fact.check_weight_threshold = check_weight_threshold;
    fact.two_t_plus_one_found = two_t_plus_one_found;
    fact.two_t_plus_one = two_t_plus_one;
    fact.votes.reserve(cert_votes.size());
    for (const auto &vote : cert_votes) {
      rustaxa::PbftSyncCertVoteFact vote_fact;
      vote_fact.vote_hash = toBridgeHash(vote->getHash());
      vote_fact.block_hash = toBridgeHash(vote->getBlockHash());
      vote_fact.period = vote->getPeriod();
      vote_fact.round = vote->getRound();
      vote_fact.step = vote->getStep();
      vote_fact.vote_type = static_cast<uint8_t>(vote->getType());
      vote_fact.live_vote_valid = true;
      vote_fact.weight_present = vote->getWeight().has_value();
      vote_fact.weight = vote->getWeight().value_or(0);
      fact.votes.push_back(vote_fact);
    }
    return fact;
  };

  const auto shape_validation =
      rustaxa::validate_pbft_sync_cert_vote_bundle(make_cert_vote_bundle_fact(false, false, 0));
  if (!shape_validation.valid) {
    LOG(log_er_) << "Rust sync cert-vote bundle validation failed for PBFT block " << block_hash << ", period "
                 << block_period << ", status " << static_cast<uint32_t>(shape_validation.status)
                 << ", first bad vote " << fromBridgeHash(shape_validation.first_bad_vote_hash);
    return false;
  }

  const uint32_t full_vote_validation_interval = 100;
  const uint32_t vote_to_validate = std::rand() % cert_votes.size();
  const bool strict_validation = (block_period % full_vote_validation_interval == 0);

  for (uint32_t vote_counter = 0; vote_counter < cert_votes.size(); vote_counter++) {
    const auto &v = cert_votes[vote_counter];
    bool strict = strict_validation || (vote_counter == vote_to_validate);

    if (const auto ret = vote_mgr_->validateVote(v, strict); !ret.first) {
      LOG(log_er_) << "Cert vote " << v->getHash() << " validation failed. Err: " << ret.second << ", pbft block "
                   << block_hash;
      return false;
    }

    assert(v->getWeight());
    vote_mgr_->addVerifiedVote(v);
  }

  const auto two_t_plus_one = vote_mgr_->getPbftTwoTPlusOne(block_period - 1, PbftVoteTypes::cert_vote);
  const auto threshold_validation = rustaxa::validate_pbft_sync_cert_vote_bundle(
      make_cert_vote_bundle_fact(true, two_t_plus_one.has_value(), two_t_plus_one.value_or(0)));
  if (!threshold_validation.valid) {
    LOG(log_wr_) << "Rust sync cert-vote bundle threshold validation failed for PBFT block " << block_hash
                 << ", period " << block_period << ", status " << static_cast<uint32_t>(threshold_validation.status)
                 << ", votes weight " << threshold_validation.total_weight << ", two_t_plus_one "
                 << threshold_validation.two_t_plus_one << ", first bad vote "
                 << fromBridgeHash(threshold_validation.first_bad_vote_hash);
    return false;
  }

  return true;
}

bool PbftManager::validatePbftBlockPillarVotes(const PeriodData &period_data) const {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  if (!period_data.pbft_blk) {
    LOG(log_er_) << "Rust sync pillar-vote validation failed, missing pbft block";
    return false;
  }

  std::vector<bytes> pillar_vote_rlps;
  if (period_data.pillar_votes_.has_value()) {
    pillar_vote_rlps.reserve(period_data.pillar_votes_->size());
    for (const auto &pillar_vote : *period_data.pillar_votes_) {
      if (!pillar_vote) {
        LOG(log_er_) << "Rust sync pillar-vote validation failed, pbft block period "
                     << period_data.pbft_blk->getPeriod() << ", status "
                     << validatePbftBlockPillarVotesWithRustStatusString(
                            ValidatePbftBlockPillarVotesWithRustStatus::kMissingPillarVotes);
        return false;
      }
      pillar_vote_rlps.push_back(pillar_vote->rlp());
    }
  }

  const auto rust_validation_result =
      validatePbftBlockPillarVotesWithRust(period_data.pbft_blk->getPeriod(), pillar_vote_rlps,
                                           period_data.pillar_votes_, pillar_chain_mgr_, final_chain_);
  if (!rust_validation_result.valid()) {
    LOG(log_er_) << "Rust sync pillar-vote validation failed, pbft block period "
                 << (period_data.pbft_blk ? period_data.pbft_blk->getPeriod() : 0) << ", status "
                 << validatePbftBlockPillarVotesWithRustStatusString(rust_validation_result.status) << ", plan status "
                 << static_cast<uint32_t>(rust_validation_result.plan_status) << ", first bad vote "
                 << rust_validation_result.first_bad_vote_hash;
  }
  return rust_validation_result.valid();
#endif

  if (!period_data.pillar_votes_.has_value() || period_data.pillar_votes_->empty()) {
    LOG(log_er_) << "No pillar votes provided, pbft block period " << period_data.pbft_blk->getPeriod()
                 << ". The synced PBFT block comes from a malicious player";
    return false;
  }

  const auto &pbft_block_hash = period_data.pbft_blk->getBlockHash();
  const auto required_votes_period = period_data.pbft_blk->getPeriod();

  const auto current_pillar_block = pillar_chain_mgr_->getCurrentPillarBlock();
  if (current_pillar_block->getPeriod() + 1 != required_votes_period) {
    LOG(log_er_) << "Sync pillar votes required period " << required_votes_period
                 << " != " << " current pillar block period " << current_pillar_block->getPeriod() << " + 1";
    return false;
  }

  uint64_t votes_weight = 0;
  for (auto &vote : *period_data.pillar_votes_) {
    // Any info is wrong that can determine the synced PBFT block comes from a malicious player
    if (vote->getPeriod() != required_votes_period) {
      LOG(log_er_) << "Invalid sync pillar vote " << vote->getHash() << " period " << vote->getPeriod()
                   << ", PBFT block " << pbft_block_hash << ", kRequiredVotesPeriod " << required_votes_period;
      return false;
    }

    if (vote->getBlockHash() != current_pillar_block->getHash()) {
      LOG(log_er_) << "Invalid sync pillar vote " << vote->getHash() << ", vote period " << vote->getPeriod()
                   << ", vote pillar block hash " << vote->getBlockHash()
                   << ", current pillar block hash: " << current_pillar_block->getHash()
                   << ", current pillar block period " << current_pillar_block->getPeriod()
                   << ", full data: " << current_pillar_block->getJson();
      return false;
    }

    if (!pillar_chain_mgr_->validatePillarVote(vote)) {
      LOG(log_er_) << "Invalid sync pillar vote " << vote->getHash();
      return false;
    }

    if (const auto vote_weight = pillar_chain_mgr_->addVerifiedPillarVote(vote); vote_weight) {
      votes_weight += vote_weight;
    } else {
      LOG(log_er_) << "Unable to add sync pillar vote " << vote->getHash();
      return false;
    }
  }

  const auto pillar_consensus_threshold = pillar_chain_mgr_->getPillarConsensusThreshold(required_votes_period - 1);
  if (!pillar_consensus_threshold.has_value()) {
    LOG(log_er_) << "Unable to obtain pillar consensus threshold for period " << required_votes_period - 1;
    return false;
  }

  if (votes_weight < *pillar_consensus_threshold) {
    LOG(log_wr_) << "Invalid sync pillar votes weight " << votes_weight << " < threshold "
                 << *pillar_consensus_threshold << ", period " << required_votes_period - 1;
    return false;
  }

  return true;
}

bool PbftManager::canParticipateInConsensus(PbftPeriod period, const addr_t &node_addr) const {
  try {
    const auto facts = final_chain_->rustFinalChainForRust().collect_pbft_final_chain_facts(
        makePbftFinalChainFactRequest(period, kNullBlockHash, false, false, false, true, {node_addr}));
    if (!facts.address_facts.empty() && facts.address_facts[0].status == kPbftSyncFactValid) {
      return facts.address_facts[0].eligible;
    }
    LOG(log_er_) << "Unable to decide if node is consensus node or not for period: " << period
                 << ". Period is too far ahead of actual finalized pbft chain size (" << facts.last_block_number
                 << "). Err msg: "
                 << (facts.address_facts.empty() ? static_cast<std::string>(facts.error_code)
                                                 : static_cast<std::string>(facts.address_facts[0].error_code))
                 << ". Node is considered as not eligible to participate in consensus for period " << period;
  } catch (const std::exception &e) {
    LOG(log_er_) << "Rust FinalChain PBFT eligibility fact collection failed for period " << period
                 << ". Err msg: " << e.what()
                 << ". Node is considered as not eligible to participate in consensus for period " << period;
  }

  return false;
}

std::map<PbftPeriod, std::vector<std::shared_ptr<PbftBlock>>> PbftManager::getProposedBlocks() const {
  return proposed_blocks_.getProposedBlocks();
}

blk_hash_t PbftManager::lastPbftBlockHashFromQueueOrChain() {
  return sync_queue_.lastBlockHashOrChain(getPbftPeriod(), pbft_chain_->getLastPbftBlockHash());
}

bool PbftManager::periodDataQueueEmpty() const { return sync_queue_.empty(); }

void PbftManager::periodDataQueuePush(PeriodData &&period_data, dev::p2p::NodeID const &node_id,
                                      std::vector<std::shared_ptr<PbftVote>> &&current_block_cert_votes) {
  const auto period = period_data.pbft_blk->getPeriod();

  // Only do parallel transactions retrieve for blocks bigger than 100 transactions
  auto trx_size = period_data.transactions.size();
  if (trx_size > 100) {
    auto chunk_size = trx_size / kSyncingThreadPoolSize;

    std::vector<std::future<void>> futures;
    futures.reserve(kSyncingThreadPoolSize);
    // Launch tasks in parallel
    for (uint32_t i = 0; i < kSyncingThreadPoolSize; ++i) {
      futures.push_back(sync_thread_pool_->post([&period_data, i, chunk_size, trx_size]() {
        const uint32_t start = i * chunk_size;
        const uint32_t end = std::min((i + 1) * chunk_size, trx_size);
        for (uint32_t j = start; j < end; j++) period_data.transactions[j]->getSender();
      }));
    }
    for (uint32_t i = 0; i < kSyncingThreadPoolSize; ++i) {
      futures[i].get();
    }
  }

  if (!sync_queue_.push(std::move(period_data), node_id, pbft_chain_->getPbftChainSize(),
                        std::move(current_block_cert_votes))) {
    LOG(log_er_) << "Trying to push period data with " << period << " period, but current period is "
                 << sync_queue_.getPeriod();
  }
}

size_t PbftManager::periodDataQueueSize() const { return sync_queue_.size(); }

bool PbftManager::checkBlockWeight(const std::vector<std::shared_ptr<DagBlock>> &dag_blocks, PbftPeriod period) const {
  const u256 total_weight =
      std::accumulate(dag_blocks.begin(), dag_blocks.end(), u256(0),
                      [](u256 value, const auto &dag_block) { return value + dag_block->getGasEstimation(); });
  const auto pbft_gas_limit = kGenesisConfig.getGasLimits(period).second;
  if (total_weight > pbft_gas_limit) {
    return false;
  }
  return true;
}

blk_hash_t PbftManager::getLastPbftBlockHash() { return pbft_chain_->getLastPbftBlockHash(); }

std::shared_ptr<PbftBlock> PbftManager::getPbftProposedBlock(PbftPeriod period, const blk_hash_t &block_hash) const {
  auto proposed_block = proposed_blocks_.getPbftProposedBlock(period, block_hash);
  if (!proposed_block.has_value()) {
    return nullptr;
  }

  return proposed_block->first;
}

PbftManager::EligibleWallets::EligibleWallets(const std::vector<WalletConfig> &wallets) {
  wallets_.reserve(wallets.size());
  for (const auto &wallet : wallets) {
    wallets_.emplace_back(false, wallet);
  }
}

void PbftManager::EligibleWallets::updateWalletsEligibility(
    PbftPeriod period, const std::shared_ptr<final_chain::FinalChain> &final_chain) {
  assert(period > period_ || period == 0);

  std::vector<addr_t> addresses;
  addresses.reserve(wallets_.size());
  for (const auto &wallet : wallets_) {
    addresses.emplace_back(wallet.second.node_addr);
  }

  const auto facts = final_chain->rustFinalChainForRust().collect_pbft_final_chain_facts(
      makePbftFinalChainFactRequest(period, kNullBlockHash, false, false, false, true, std::move(addresses)));
  assert(period <= facts.last_block_number + final_chain->delegationDelay());
  assert(facts.address_facts.size() == wallets_.size());

  for (size_t i = 0; i < wallets_.size(); ++i) {
    wallets_[i].first = i < facts.address_facts.size() && facts.address_facts[i].status == kPbftSyncFactValid &&
                        facts.address_facts[i].eligible;
  }

  period_ = period;
}

const std::vector<std::pair<bool, WalletConfig>> &PbftManager::EligibleWallets::getWallets(
    PbftPeriod current_pbft_period) const {
  assert(period_ == current_pbft_period - 1);

  return wallets_;
}

PbftPeriod PbftManager::EligibleWallets::getWalletsEligiblePeriod() const { return period_; }

std::chrono::milliseconds PbftManager::getPbftDeadline() const {
  auto current_round_lambda = current_round_lambda_;
  if (pbft_manager_runtime_.has_value()) {
    const auto snapshot = rustaxa::pbft_manager_runtime_snapshot(*pbft_manager_runtime_.value());
    if (snapshot.status != kPbftManagerRuntimeSnapshotStatusReady) {
      throw std::runtime_error("PBFT manager Rust runtime snapshot is not ready: " +
                               static_cast<std::string>(snapshot.error_code));
    }
    current_round_lambda = std::chrono::milliseconds(snapshot.current_round_lambda_ms);
  }

  if (kGenesisConfig.state.hardforks.isOnCactiHardfork(getPbftPeriod())) {
    auto block_propagation = std::chrono::milliseconds(kGenesisConfig.state.hardforks.cacti_hf.block_propagation_min);
    if (getPbftRound() > 1) {
      block_propagation = std::chrono::milliseconds(kGenesisConfig.state.hardforks.cacti_hf.block_propagation_max);
    }

    return std::max(4 * current_round_lambda, block_propagation);
  }

  return 4 * current_round_lambda;
}

}  // namespace taraxa

#endif  // defined(RUSTAXA_ENABLE_PILLAR_VOTES) || defined(RUSTAXA_ENABLE_PROPOSED_BLOCKS)
