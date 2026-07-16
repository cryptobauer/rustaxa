#include <gtest/gtest.h>

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <initializer_list>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

namespace {

std::array<uint8_t, 32> h256(uint8_t last_byte) {
  std::array<uint8_t, 32> hash{};
  hash[31] = last_byte;
  return hash;
}

rust::Box<BridgePbftStorageQueries> pbftQueries(const rust::Box<BridgeStorage>& storage) {
  return create_pbft_storage_queries(*storage);
}
constexpr uint8_t kPbftFinalizationAnchorNull = 0;
constexpr uint8_t kPbftFinalizationAnchorAnchored = 1;
constexpr uint8_t kPbftFinalizationStatusAccepted = 0;
constexpr uint8_t kPbftFinalizationStatusBlockAlreadyInChain = 1;
constexpr uint8_t kPbftFinalizationStatusPillarDependencyMissing = 4;
constexpr uint8_t kPbftFinalizationStatusEmptyCertVotes = 5;
constexpr uint8_t kPbftFinalizationStatusCertVoteBlockMismatch = 6;
constexpr uint8_t kPbftFinalizationRuntimeActionFinalizeFinalChain = 9;
constexpr uint8_t kPbftFinalizationRuntimeActionCommitSortitionRuntime = 14;
constexpr uint8_t kPbftFinalizationRuntimeStatusActive = 0;
constexpr uint8_t kPbftFinalizationRuntimeStatusActionMismatch = 3;
constexpr uint8_t kPbftFinalizationRuntimeStatusActionFailed = 4;
constexpr uint8_t kPbftFinalizationExecutorModeFresh = 0;
constexpr uint8_t kPbftFinalizationExecutorModeResume = 1;
constexpr uint8_t kPbftMgrFieldRound = 0;
constexpr uint8_t kPbftMgrFieldStep = 1;
constexpr uint8_t kPbftMgrFieldLambda = 2;
constexpr uint8_t kPbftMgrStatusExecutedBlock = 0;
constexpr uint8_t kPbftMgrStatusNextVotedValue = 2;
constexpr uint8_t kPbftFinalizationStorageStagePrimary = 0;
constexpr uint8_t kPbftFinalizationStorageStageDynamic = 1;
constexpr uint8_t kPbftManagerStartupStatusReady = 0;
constexpr uint8_t kPbftManagerRuntimeStateValueProposal = 0;
constexpr uint8_t kPbftManagerRuntimeStateFinish = 3;
constexpr uint8_t kPbftManagerRuntimeStateCertify = 2;
constexpr uint8_t kPbftManagerRuntimeActionProcessSyncedBlocks = 0;
constexpr uint8_t kPbftManagerRuntimeActionMaybeBroadcastVotes = 1;
constexpr uint8_t kPbftManagerRuntimeActionTryPushCertVotesBlock = 2;
constexpr uint8_t kPbftManagerRuntimeActionTryAdvanceRound = 3;
constexpr uint8_t kPbftManagerRuntimeActionRunValueProposal = 5;
constexpr uint8_t kPbftManagerRuntimeActionTransitionToFilter = 6;
constexpr uint8_t kPbftManagerRuntimeActionRunCertify = 9;
constexpr uint8_t kPbftManagerRuntimeActionTransitionToFinish = 10;
constexpr uint8_t kPbftManagerRuntimeActionSleepUntilNextStep = 17;
constexpr uint8_t kPbftManagerRuntimeActionResetConsensus = 18;
constexpr uint8_t kPbftManagerRuntimeStatusActive = 0;
constexpr uint8_t kPbftManagerRuntimeStatusComplete = 1;
constexpr uint8_t kPbftManagerRuntimeStatusActionMismatch = 3;
constexpr uint8_t kPbftManagerRuntimeStatusInvalidReport = 5;
constexpr uint8_t kPbftManagerRuntimeResultNoProgress = 0;
constexpr uint8_t kPbftManagerRuntimeResultProgressRestart = 1;
constexpr uint8_t kPbftManagerRuntimeResultStateDone = 2;
constexpr uint8_t kPbftManagerRuntimeResultTransition = 3;
constexpr uint8_t kPbftManagerRuntimeResultSleepApplied = 4;
constexpr uint8_t kPbftManagerStateActionNextVoteNullBlock = 8;
constexpr uint8_t kPbftManagerStateActionNextVoteCurrentSoftValue = 10;
constexpr uint8_t kPbftManagerStateActionSessionActive = 0;
constexpr uint8_t kPbftManagerStateActionSessionComplete = 1;
constexpr uint8_t kPbftManagerStateActionEffectApplied = 0;
constexpr uint8_t kPbftManagerAdvanceActionSetVoteManagerPeriodRound = 2;
constexpr uint8_t kPbftManagerAdvanceActionResetCurrentRoundTimer = 3;
constexpr uint8_t kPbftManagerAdvanceActionResetRewardVoteCounters = 4;
constexpr uint8_t kPbftManagerAdvanceActionResetPeriodTimer = 5;
constexpr uint8_t kPbftManagerAdvanceActionUpdateWalletEligibility = 6;
constexpr uint8_t kPbftManagerAdvanceActionCleanupVotes = 7;
constexpr uint8_t kPbftManagerAdvanceActionCleanupProposedBlocks = 8;

PbftFinalizationStorageWriteStage finalizationStorageStage(uint8_t stage) {
  PbftFinalizationStorageWriteStage write_stage{};
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

PbftFinalizationStorageWriteStage dynamicLambdaFinalizationStorageStage(const PbftFinalizationIntentPlan& plan) {
  auto write_stage = finalizationStorageStage(kPbftFinalizationStorageStageDynamic);
  write_stage.rounds_count_dynamic_lambda = plan.storage_write_intent.rounds_count_dynamic_lambda;
  write_stage.dynamic_lambda = plan.storage_write_intent.dynamic_lambda;
  return write_stage;
}

rust::Vec<PbftFinalizationStorageWriteStage> storageStages(
    std::initializer_list<PbftFinalizationStorageWriteStage> stages) {
  rust::Vec<PbftFinalizationStorageWriteStage> out;
  for (auto stage : stages) {
    out.push_back(std::move(stage));
  }
  return out;
}

PbftManagerFinalizationExecutorState startFreshFinalizationExecutor(
    BridgePbftService& runtime, const PbftFinalizationIntentPlan& plan,
    rust::Vec<PbftFinalizationStorageWriteStage> primary_stages) {
  PbftFinalizationExecutorStartRequest request{};
  request.mode = kPbftFinalizationExecutorModeFresh;
  request.plan = plan;
  request.primary_stages = std::move(primary_stages);
  request.sync = false;
  request.final_chain_last_block = 0;
  return pbft_manager_runtime_start_finalization_executor(runtime, request);
}

PbftManagerFinalizationExecutorState startResumeFinalizationExecutor(BridgePbftService& runtime,
                                                                     const PbftFinalizationIntentPlan& plan,
                                                                     uint64_t final_chain_last_block) {
  PbftFinalizationExecutorStartRequest request{};
  request.mode = kPbftFinalizationExecutorModeResume;
  request.plan = plan;
  request.sync = false;
  request.final_chain_last_block = final_chain_last_block;
  return pbft_manager_runtime_start_finalization_executor(runtime, request);
}

PbftFinalizationIntentFact makeFinalizationFact() {
  PbftFinalizationIntentFact fact;
  fact.block_hash = h256(9);
  fact.pbft_head_hash = h256(8);
  fact.block_period = 101;
  fact.block_prev_hash = h256(1);
  fact.chain_last_hash = h256(1);
  fact.chain_last_period = 100;
  fact.block_in_chain = false;
  fact.pivot_dag_anchor_hash = h256(8);
  fact.has_pillar_block = false;
  fact.pillar_block_finalized = false;
  fact.request_dynamic_lambda_update = true;
  fact.cert_vote_count = 3;
  fact.sample_cert_vote_block_hash = h256(9);
  fact.sample_cert_vote_period = 101;
  fact.sample_cert_vote_round = 2;
  fact.sample_cert_vote_step = 5;
  fact.block_lambda = 1500;
  fact.last_saved_period_lambda_found = false;
  fact.last_saved_period_lambda = 0;
  fact.dynamic_blocks_per_year = 1000;
  fact.dpos_blocks_per_year = 500;
  fact.pbft_head_payload = {'{', '"', 'h', 'e', 'a', 'd', '"', ':', 't', 'r', 'u', 'e', '}'};
  fact.period_data_rlp = {0xc0};
  fact.ordered_dag_block_hashes = {PbftFinalizationHash{h256(2)}, PbftFinalizationHash{h256(3)}};
  fact.ordered_transaction_hashes = {PbftFinalizationHash{h256(4)}};
  return fact;
}

PbftDynamicLambdaFact makeDynamicLambdaFact() {
  PbftDynamicLambdaConfig config;
  config.cacti_block_num = 10;
  config.lambda_min = 500;
  config.lambda_max = 1500;
  config.lambda_default = 2000;
  config.lambda_change_interval = 10;
  config.lambda_change = 10;
  config.consensus_delay = 400;
  config.dpos_blocks_per_year = 500;

  PbftDynamicLambdaFact fact;
  fact.dynamic_lambda_active = true;
  fact.finalized_period = 20;
  fact.finalized_round = 1;
  fact.pre_adjust_rounds_count_dynamic_lambda = 9;
  fact.pre_adjust_dynamic_lambda = 1500;
  fact.config = config;
  return fact;
}

PbftManagerRuntimeTickFact makePbftManagerRuntimeTick(uint8_t state) {
  PbftManagerRuntimeTickFact fact;
  fact.tick_id = 77;
  fact.state = state;
  fact.period = 10;
  fact.round = 2;
  fact.step = 3;
  fact.network_available = true;
  fact.network_pbft_syncing = false;
  fact.has_eligible_wallet = true;
  return fact;
}

PbftServiceConfig makePbftServiceConfig(bool cacti_active = true) {
  PbftServiceConfig config;
  config.genesis_lambda_ms = 100;
  config.cacti_lambda_max_ms = 1'500;
  config.cacti_lambda_default_ms = 500;
  config.cacti_block = cacti_active ? 1 : 100;
  config.max_exponential_lambda_ms = 60'000;
  config.max_steps = 13;
  config.deadline_ms = 1'000;
  config.polling_interval_ms = 100;
  return config;
}

rust::Vec<uint8_t> bridgeBytes(std::string_view input) {
  rust::Vec<uint8_t> out;
  out.reserve(input.size());
  for (const auto ch : input) {
    out.push_back(static_cast<uint8_t>(ch));
  }
  return out;
}

void seedPbftChainPeriod(const rust::Box<BridgeStorage>& storage, uint64_t current_period = 10) {
  std::ostringstream head;
  head
      << R"({"head_hash":"0x0000000000000000000000000000000000000000000000000000000000000000","size":)"
      << current_period - 1
      << R"(,"non_empty_size":0,"last_pbft_block_hash":"0x0000000000000000000000000000000000000000000000000000000000000000"})";
  auto batch = create_storage_shim_batch(*storage);
  storage_shim_save_pbft_head(*batch, h256(0), bridgeBytes(head.str()));
  storage_shim_commit_batch(std::move(batch), false);
}

PbftManagerRuntimeActionReport managerRuntimeReport(uint32_t cursor, uint8_t action, uint8_t result) {
  PbftManagerRuntimeActionReport report;
  report.cursor = cursor;
  report.action = action;
  report.success = true;
  report.result = result;
  report.go_finish_state = false;
  report.loop_back_finish_state = false;
  report.has_eligible_wallet = true;
  report.has_new_round = false;
  report.new_round = 0;
  return report;
}

PbftManagerStateActionFact makePbftManagerStateActionFact(uint8_t state) {
  PbftManagerStateActionFact fact;
  fact.state = state;
  fact.period = 10;
  fact.round = 2;
  fact.step = 3;
  fact.elapsed_round_ms = 250;
  fact.deadline_ms = 1'000;
  fact.current_round_lambda_ms = 1'000;
  fact.polling_interval_ms = 100;
  fact.has_previous_round_next_null = false;
  fact.has_previous_round_next_value = false;
  fact.previous_round_next_value_hash = h256(0x44);
  fact.has_current_round_soft_value = false;
  fact.current_round_soft_value_hash = h256(0x55);
  fact.has_cert_voted_block = false;
  fact.cert_voted_block_hash = h256(0x66);
  fact.already_next_voted_value = false;
  fact.already_next_voted_null = false;
  return fact;
}

PbftManagerStateActionEffectReport stateActionReport(uint32_t cursor, uint8_t intent) {
  PbftManagerStateActionEffectReport report;
  report.cursor = cursor;
  report.intent = intent;
  report.result = kPbftManagerStateActionEffectApplied;
  return report;
}

std::filesystem::path uniqueTempDir(const std::string& name) {
  const auto nonce = std::chrono::steady_clock::now().time_since_epoch().count();
  auto path = std::filesystem::temp_directory_path() / (name + "_" + std::to_string(nonce));
  std::filesystem::create_directories(path);
  return path;
}

rust::Box<BridgePbftService> managerRuntimeForTick(PbftManagerRuntimeTickFact tick) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_runtime_session");
  auto storage = create_storage(test_dir.string());
  seedPbftChainPeriod(storage);
  auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig(false));
  pbft_service_complete_bootstrap(*runtime);
  pbft_manager_runtime_begin_session(*runtime, tick);
  return runtime;
}

rust::Box<BridgePbftService> managerRuntimeForFinalizationSession() {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_finalization_session");
  auto storage = create_storage(test_dir.string());
  seedPbftChainPeriod(storage);
  return create_pbft_service_from_storage(*storage, makePbftServiceConfig(false));
}

PbftFinalizationIntentPlan finalizationIntentPlan(PbftFinalizationIntentFact fact) {
  auto runtime = managerRuntimeForFinalizationSession();
  return pbft_manager_runtime_plan_finalization_intent(*runtime, std::move(fact));
}

void expectNoFinalizationCleanup(const PbftFinalizationCleanupPlan& cleanup) {
  EXPECT_FALSE(cleanup.persist_pbft_block_metadata);
  EXPECT_FALSE(cleanup.reset_reward_votes);
  EXPECT_FALSE(cleanup.set_dag_block_order);
  EXPECT_FALSE(cleanup.update_sortition_params);
  EXPECT_FALSE(cleanup.update_finalized_transactions_status);
  EXPECT_FALSE(cleanup.update_pbft_chain);
  EXPECT_FALSE(cleanup.clear_anchor_dag_cache);
  EXPECT_FALSE(cleanup.finalize_final_chain);
  EXPECT_FALSE(cleanup.maybe_update_dynamic_lambda);
  EXPECT_FALSE(cleanup.advance_period);
}

void expectNoFinalizationStorageWrites(const PbftFinalizationStorageWritePlan& storage) {
  EXPECT_FALSE(storage.persist_pbft_head);
  EXPECT_FALSE(storage.persist_period_data);
  EXPECT_FALSE(storage.reset_reward_votes);
  EXPECT_FALSE(storage.update_sortition_params);
  EXPECT_FALSE(storage.apply_dynamic_lambda_update);
  EXPECT_FALSE(storage.persist_period_lambda);
  EXPECT_FALSE(storage.persist_executed_pbft_status);
  EXPECT_TRUE(storage.pbft_head_payload.empty());
  EXPECT_TRUE(storage.period_data_rlp.empty());
  EXPECT_TRUE(storage.dag_block_period_writes.empty());
  EXPECT_TRUE(storage.transaction_location_writes.empty());
}

}  // namespace

TEST(RustPbftSyncTest, ManagerRuntimeOrdersOneValueProposalTick) {
  auto runtime = managerRuntimeForTick(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));
  std::vector<uint8_t> actions;

  while (true) {
    auto step = pbft_manager_runtime_session_next(*runtime);
    if (!step.has_action) {
      EXPECT_EQ(step.status, kPbftManagerRuntimeStatusComplete);
      EXPECT_TRUE(step.complete);
      EXPECT_FALSE(step.restart_loop);
      break;
    }

    actions.push_back(step.action);
    uint8_t result = kPbftManagerRuntimeResultStateDone;
    if (step.action == kPbftManagerRuntimeActionTryPushCertVotesBlock ||
        step.action == kPbftManagerRuntimeActionTryAdvanceRound) {
      result = kPbftManagerRuntimeResultNoProgress;
    } else if (step.action == kPbftManagerRuntimeActionTransitionToFilter) {
      result = kPbftManagerRuntimeResultTransition;
    } else if (step.action == kPbftManagerRuntimeActionSleepUntilNextStep) {
      result = kPbftManagerRuntimeResultSleepApplied;
    }
    step = pbft_manager_runtime_session_report(*runtime, managerRuntimeReport(step.cursor, step.action, result));
    EXPECT_TRUE(step.can_continue);
  }

  EXPECT_EQ(actions, (std::vector<uint8_t>{
                         kPbftManagerRuntimeActionProcessSyncedBlocks,
                         kPbftManagerRuntimeActionMaybeBroadcastVotes,
                         kPbftManagerRuntimeActionTryPushCertVotesBlock,
                         kPbftManagerRuntimeActionTryAdvanceRound,
                         kPbftManagerRuntimeActionRunValueProposal,
                         kPbftManagerRuntimeActionTransitionToFilter,
                         kPbftManagerRuntimeActionSleepUntilNextStep,
                     }));
}

TEST(RustPbftSyncTest, ManagerRuntimeCompletesWithRestartOnCertPushProgress) {
  auto runtime = managerRuntimeForTick(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));

  auto step = pbft_manager_runtime_session_next(*runtime);
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionProcessSyncedBlocks);
  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionMaybeBroadcastVotes);
  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionTryPushCertVotesBlock);

  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultProgressRestart));

  EXPECT_EQ(step.status, kPbftManagerRuntimeStatusComplete);
  EXPECT_TRUE(step.complete);
  EXPECT_TRUE(step.restart_loop);
}

TEST(RustPbftSyncTest, ManagerRuntimeAdvanceRoundCandidateRequestsResetEffect) {
  auto runtime = managerRuntimeForTick(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));

  auto step = pbft_manager_runtime_session_next(*runtime);
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionProcessSyncedBlocks);
  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionMaybeBroadcastVotes);
  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionTryPushCertVotesBlock);
  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultNoProgress));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionTryAdvanceRound);

  auto report = managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultNoProgress);
  report.has_new_round = true;
  report.new_round = 5;
  step = pbft_manager_runtime_session_report(*runtime, std::move(report));

  ASSERT_EQ(step.status, kPbftManagerRuntimeStatusActive);
  ASSERT_TRUE(step.has_action);
  EXPECT_EQ(step.action, kPbftManagerRuntimeActionResetConsensus);
  EXPECT_TRUE(step.has_target_round);
  EXPECT_EQ(step.target_round, 5);

  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultTransition));

  EXPECT_EQ(step.status, kPbftManagerRuntimeStatusComplete);
  EXPECT_TRUE(step.complete);
  EXPECT_TRUE(step.restart_loop);
}

TEST(RustPbftSyncTest, ManagerRuntimeRejectsNonIncreasingAdvanceRoundCandidate) {
  auto runtime = managerRuntimeForTick(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));

  auto step = pbft_manager_runtime_session_next(*runtime);
  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultNoProgress));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionTryAdvanceRound);

  auto report = managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultNoProgress);
  report.has_new_round = true;
  report.new_round = 2;
  step = pbft_manager_runtime_session_report(*runtime, std::move(report));

  EXPECT_EQ(step.status, kPbftManagerRuntimeStatusInvalidReport);
  EXPECT_FALSE(step.can_continue);
  EXPECT_FALSE(step.complete);
}

TEST(RustPbftSyncTest, ManagerRuntimeCertifyReportSelectsFinishTransition) {
  auto runtime = managerRuntimeForTick(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateCertify));

  while (true) {
    auto step = pbft_manager_runtime_session_next(*runtime);
    ASSERT_TRUE(step.has_action);
    if (step.action == kPbftManagerRuntimeActionRunCertify) {
      auto report = managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone);
      report.go_finish_state = true;
      step = pbft_manager_runtime_session_report(*runtime, std::move(report));
      EXPECT_EQ(step.status, kPbftManagerRuntimeStatusActive);
      EXPECT_EQ(step.action, kPbftManagerRuntimeActionTransitionToFinish);
      break;
    }

    uint8_t result = kPbftManagerRuntimeResultStateDone;
    if (step.action == kPbftManagerRuntimeActionTryPushCertVotesBlock ||
        step.action == kPbftManagerRuntimeActionTryAdvanceRound) {
      result = kPbftManagerRuntimeResultNoProgress;
    }
    pbft_manager_runtime_session_report(*runtime, managerRuntimeReport(step.cursor, step.action, result));
  }
}

TEST(RustPbftSyncTest, ManagerRuntimeRejectsCursorMismatch) {
  auto runtime = managerRuntimeForTick(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));
  auto step = pbft_manager_runtime_session_next(*runtime);

  step = pbft_manager_runtime_session_report(
      *runtime, managerRuntimeReport(step.cursor + 1, step.action, kPbftManagerRuntimeResultStateDone));

  EXPECT_EQ(step.status, kPbftManagerRuntimeStatusActionMismatch);
  EXPECT_FALSE(step.can_continue);
  EXPECT_FALSE(step.complete);
}

TEST(RustPbftSyncTest, ManagerStartupRestoreRecordsRuntimeSnapshotFromStorage) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_startup_snapshot");
  auto storage = create_storage(test_dir.string());
  auto seed_batch = create_storage_shim_batch(*storage);
  storage_shim_save_pbft_mgr_field(*seed_batch, kPbftMgrFieldRound, 2);
  storage_shim_save_pbft_mgr_field(*seed_batch, kPbftMgrFieldStep, 2);
  storage_shim_save_pbft_mgr_field(*seed_batch, kPbftMgrFieldLambda, 1'500);
  storage_shim_save_pbft_mgr_status(*seed_batch, kPbftMgrStatusExecutedBlock, true);
  storage_shim_save_pbft_mgr_status(*seed_batch, kPbftMgrStatusNextVotedValue, true);
  storage_shim_commit_batch(std::move(seed_batch), false);

  seedPbftChainPeriod(storage);
  const auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig());
  const auto snapshot = pbft_manager_runtime_snapshot(*runtime);

  EXPECT_EQ(snapshot.status, kPbftManagerStartupStatusReady);
  EXPECT_EQ(snapshot.state, kPbftManagerRuntimeStateFinish);
  EXPECT_EQ(snapshot.period, 10);
  EXPECT_EQ(snapshot.round, 2);
  EXPECT_EQ(snapshot.step, 4);
  EXPECT_EQ(snapshot.current_round_lambda_ms, 500);
  EXPECT_EQ(snapshot.dynamic_lambda_ms, 1'500);
  EXPECT_TRUE(snapshot.executed_pbft_block);
  EXPECT_TRUE(snapshot.already_next_voted_value);
  EXPECT_FALSE(snapshot.already_next_voted_null);
  EXPECT_EQ(pbftQueries(storage)->get_pbft_mgr_field(kPbftMgrFieldStep), 4);

  std::filesystem::remove_all(test_dir);
}

TEST(RustPbftSyncTest, ManagerStateActionEffectSessionRecordsFinishPollingTranscript) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_state_action_runtime");
  auto storage = create_storage(test_dir.string());
  seedPbftChainPeriod(storage);
  auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig(false));
  auto fact = makePbftManagerStateActionFact(4);
  fact.has_current_round_soft_value = true;
  fact.has_previous_round_next_null = true;

  pbft_manager_runtime_begin_state_action_effect_session(*runtime, fact);
  std::vector<uint8_t> intents;

  auto step = pbft_manager_runtime_state_action_effect_session_next(*runtime);
  ASSERT_EQ(step.status, kPbftManagerStateActionSessionActive);
  ASSERT_TRUE(step.has_effect);
  intents.push_back(step.effect.intent);
  EXPECT_EQ(step.effect.hash, h256(0x55));

  step = pbft_manager_runtime_state_action_effect_session_report(*runtime,
                                                                 stateActionReport(step.cursor, step.effect.intent));
  ASSERT_EQ(step.status, kPbftManagerStateActionSessionActive);
  ASSERT_TRUE(step.has_effect);
  intents.push_back(step.effect.intent);
  EXPECT_EQ(step.effect.hash, h256(0));

  step = pbft_manager_runtime_state_action_effect_session_report(*runtime,
                                                                 stateActionReport(step.cursor, step.effect.intent));
  EXPECT_EQ(step.status, kPbftManagerStateActionSessionComplete);
  EXPECT_TRUE(step.complete);
  EXPECT_TRUE(step.can_continue);
  EXPECT_FALSE(step.has_effect);

  EXPECT_EQ(intents, (std::vector<uint8_t>{kPbftManagerStateActionNextVoteCurrentSoftValue,
                                           kPbftManagerStateActionNextVoteNullBlock}));
  std::filesystem::remove_all(test_dir);
}

TEST(RustPbftSyncTest, ManagerAdvancePeriodRecordsEffectTranscript) {
  auto runtime = managerRuntimeForFinalizationSession();
  PbftManagerLifecycleTransitionRequest request{};
  request.kind = 0;
  request.target_period = 13;
  request.target_round = 1;
  const auto reset = pbft_manager_runtime_execute_lifecycle_transition(*runtime, request);
  ASSERT_EQ(reset.status, 0);
  const auto plan = pbft_manager_runtime_plan_advance_period_after_reset(*runtime, 12);

  std::vector<uint8_t> actions;
  actions.reserve(plan.actions.size());
  for (const auto action : plan.actions) {
    actions.push_back(action);
  }

  EXPECT_TRUE(plan.accepted);
  EXPECT_EQ(plan.finalized_chain_size, 12);
  EXPECT_EQ(plan.new_period, 13);
  EXPECT_EQ(actions,
            (std::vector<uint8_t>{
                kPbftManagerAdvanceActionSetVoteManagerPeriodRound, kPbftManagerAdvanceActionResetCurrentRoundTimer,
                kPbftManagerAdvanceActionResetRewardVoteCounters, kPbftManagerAdvanceActionResetPeriodTimer,
                kPbftManagerAdvanceActionUpdateWalletEligibility, kPbftManagerAdvanceActionCleanupVotes,
                kPbftManagerAdvanceActionCleanupProposedBlocks}));
}

TEST(RustPbftSyncTest, FinalizationIntentAcceptsAnchoredBlockAndMapsCleanup) {
  const auto plan = finalizationIntentPlan(makeFinalizationFact());

  EXPECT_TRUE(plan.finalize_block);
  EXPECT_EQ(plan.anchor, kPbftFinalizationAnchorAnchored);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusAccepted);
  EXPECT_TRUE(plan.executed_pbft_block);
  EXPECT_TRUE(plan.cleanup.persist_pbft_block_metadata);
  EXPECT_TRUE(plan.cleanup.reset_reward_votes);
  EXPECT_TRUE(plan.cleanup.set_dag_block_order);
  EXPECT_TRUE(plan.cleanup.update_sortition_params);
  EXPECT_TRUE(plan.cleanup.update_finalized_transactions_status);
  EXPECT_TRUE(plan.cleanup.update_pbft_chain);
  EXPECT_TRUE(plan.cleanup.clear_anchor_dag_cache);
  EXPECT_TRUE(plan.cleanup.finalize_final_chain);
  EXPECT_TRUE(plan.cleanup.maybe_update_dynamic_lambda);
  EXPECT_TRUE(plan.cleanup.advance_period);
  EXPECT_TRUE(plan.storage_write_intent.persist_pbft_head);
  EXPECT_TRUE(plan.storage_write_intent.persist_period_data);
  EXPECT_TRUE(plan.storage_write_intent.reset_reward_votes);
  EXPECT_TRUE(plan.storage_write_intent.update_sortition_params);
  EXPECT_TRUE(plan.storage_write_intent.apply_dynamic_lambda_update);
  EXPECT_TRUE(plan.storage_write_intent.persist_period_lambda);
  EXPECT_TRUE(plan.storage_write_intent.persist_executed_pbft_status);
  EXPECT_EQ(plan.storage_write_intent.pbft_block_hash, h256(9));
  EXPECT_EQ(plan.storage_write_intent.pbft_head_hash, h256(8));
  EXPECT_EQ(plan.storage_write_intent.block_period, 101);
  EXPECT_FALSE(plan.storage_write_intent.null_anchor);
  EXPECT_EQ(plan.storage_write_intent.anchor_hash, h256(8));
  EXPECT_EQ(plan.storage_write_intent.reward_vote_period, 101);
  EXPECT_EQ(plan.storage_write_intent.reward_vote_round, 2);
  EXPECT_EQ(plan.storage_write_intent.reward_vote_step, 5);
  EXPECT_EQ(plan.storage_write_intent.reward_vote_block_hash, h256(9));
  EXPECT_EQ(plan.storage_write_intent.period_lambda, 1500);
  EXPECT_EQ(plan.storage_write_intent.blocks_per_year, 1000);
  EXPECT_TRUE(plan.storage_write_intent.executed_pbft_status);
  EXPECT_EQ(std::vector<uint8_t>(plan.storage_write_intent.pbft_head_payload.begin(),
                                 plan.storage_write_intent.pbft_head_payload.end()),
            (std::vector<uint8_t>{'{', '"', 'h', 'e', 'a', 'd', '"', ':', 't', 'r', 'u', 'e', '}'}));
  ASSERT_EQ(plan.storage_write_intent.period_data_rlp.size(), 1);
  EXPECT_EQ(plan.storage_write_intent.period_data_rlp[0], 0xc0);
  ASSERT_EQ(plan.storage_write_intent.dag_block_period_writes.size(), 2);
  EXPECT_EQ(plan.storage_write_intent.dag_block_period_writes[0].hash, h256(2));
  EXPECT_EQ(plan.storage_write_intent.dag_block_period_writes[0].position, 0);
  EXPECT_EQ(plan.storage_write_intent.dag_block_period_writes[1].hash, h256(3));
  EXPECT_EQ(plan.storage_write_intent.dag_block_period_writes[1].position, 1);
  ASSERT_EQ(plan.storage_write_intent.transaction_location_writes.size(), 1);
  EXPECT_EQ(plan.storage_write_intent.transaction_location_writes[0].hash, h256(4));
  EXPECT_EQ(plan.storage_write_intent.transaction_location_writes[0].position, 0);
}

TEST(RustPbftSyncTest, FinalizationBoundaryReportsExternalActionFailure) {
  const auto plan = finalizationIntentPlan(makeFinalizationFact());
  auto runtime = managerRuntimeForFinalizationSession();
  auto boundary = startFreshFinalizationExecutor(
      *runtime, plan, storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));

  PbftManagerFinalizationSortitionCommitReport report{};
  report.changed = true;
  report.change_period = 999;
  report.change_interval_efficiency = 2500;
  report.change_threshold_upper = 1300;
  report.current_threshold_upper = 1300;
  report.params_changes_count = 1;

  boundary = pbft_manager_runtime_advance_finalization_sortition_commit(*runtime, boundary.cursor, report);
  EXPECT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActionFailed);
  EXPECT_FALSE(boundary.has_action);
  EXPECT_FALSE(boundary.can_continue);
  EXPECT_FALSE(std::string(boundary.error_code).empty());
}

TEST(RustPbftSyncTest, FinalizationBoundaryBeginsAtFirstExternalAction) {
  const auto plan = finalizationIntentPlan(makeFinalizationFact());
  auto runtime = managerRuntimeForFinalizationSession();

  const auto boundary = startFreshFinalizationExecutor(
      *runtime, plan, storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));

  EXPECT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActive);
  EXPECT_EQ(boundary.cursor, 1);
  EXPECT_TRUE(boundary.has_action);
  EXPECT_EQ(boundary.action, kPbftFinalizationRuntimeActionCommitSortitionRuntime);
  EXPECT_TRUE(boundary.can_continue);
}

TEST(RustPbftSyncTest, FinalizationIntentRejectsAlreadyPersistedBlock) {
  auto fact = makeFinalizationFact();
  fact.block_in_chain = true;

  const auto plan = finalizationIntentPlan(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusBlockAlreadyInChain);
  EXPECT_FALSE(plan.executed_pbft_block);
  expectNoFinalizationCleanup(plan.cleanup);
  expectNoFinalizationStorageWrites(plan.storage_write_intent);
}

TEST(RustPbftSyncTest, FinalizationIntentClassifiesNullAnchorAndRejectsExplicitly) {
  auto fact = makeFinalizationFact();
  fact.pivot_dag_anchor_hash = h256(0);
  fact.request_dynamic_lambda_update = false;

  auto plan = finalizationIntentPlan(std::move(fact));

  EXPECT_TRUE(plan.finalize_block);
  EXPECT_EQ(plan.anchor, kPbftFinalizationAnchorNull);
  EXPECT_FALSE(plan.cleanup.update_sortition_params);
  EXPECT_FALSE(plan.cleanup.maybe_update_dynamic_lambda);
  EXPECT_FALSE(plan.storage_write_intent.update_sortition_params);
  EXPECT_FALSE(plan.storage_write_intent.apply_dynamic_lambda_update);
  EXPECT_FALSE(plan.storage_write_intent.persist_period_lambda);
  EXPECT_TRUE(plan.storage_write_intent.null_anchor);
  EXPECT_EQ(plan.storage_write_intent.blocks_per_year, 500);

  fact = makeFinalizationFact();
  fact.block_in_chain = true;
  plan = finalizationIntentPlan(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusBlockAlreadyInChain);
  EXPECT_FALSE(plan.cleanup.advance_period);
  expectNoFinalizationStorageWrites(plan.storage_write_intent);

  fact = makeFinalizationFact();
  fact.has_pillar_block = true;
  fact.pillar_block_finalized = false;
  plan = finalizationIntentPlan(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusPillarDependencyMissing);
  EXPECT_FALSE(plan.cleanup.finalize_final_chain);
  expectNoFinalizationCleanup(plan.cleanup);
  expectNoFinalizationStorageWrites(plan.storage_write_intent);
}

TEST(RustPbftSyncTest, FinalizationIntentRejectsMalformedCertVoteFacts) {
  auto fact = makeFinalizationFact();
  fact.cert_vote_count = 0;

  auto plan = finalizationIntentPlan(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusEmptyCertVotes);
  expectNoFinalizationStorageWrites(plan.storage_write_intent);

  fact = makeFinalizationFact();
  fact.sample_cert_vote_block_hash = h256(10);
  plan = finalizationIntentPlan(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusCertVoteBlockMismatch);
  expectNoFinalizationStorageWrites(plan.storage_write_intent);
}

TEST(RustPbftSyncTest, FinalizationBoundaryRecordsExternalFailure) {
  auto runtime = managerRuntimeForFinalizationSession();
  const auto plan = finalizationIntentPlan(makeFinalizationFact());
  auto boundary = startFreshFinalizationExecutor(
      *runtime, plan, storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));
  ASSERT_EQ(boundary.action, kPbftFinalizationRuntimeActionCommitSortitionRuntime);

  boundary =
      pbft_manager_runtime_fail_finalization_external_effect(*runtime, boundary.cursor, 77, "TEST_EXTERNAL_FAILURE");
  EXPECT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActionFailed);
  EXPECT_FALSE(boundary.has_action);
  EXPECT_EQ(std::string(boundary.error_code), "TEST_EXTERNAL_FAILURE");

  const auto cleared =
      pbft_manager_runtime_fail_finalization_external_effect(*runtime, boundary.cursor, 77, "AFTER_CLEAR");
  EXPECT_EQ(cleared.status, kPbftFinalizationRuntimeStatusActionMismatch);
  EXPECT_FALSE(cleared.has_action);
  EXPECT_EQ(std::string(cleared.error_code), "PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED");
}

TEST(RustPbftSyncTest, FinalizationExecutorRejectsStaleCursor) {
  auto runtime = managerRuntimeForFinalizationSession();
  const auto plan = finalizationIntentPlan(makeFinalizationFact());
  auto state = startFreshFinalizationExecutor(
      *runtime, plan, storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));
  ASSERT_EQ(state.action, kPbftFinalizationRuntimeActionCommitSortitionRuntime);

  state = pbft_manager_runtime_fail_finalization_external_effect(*runtime, state.cursor + 1, 77, "STALE_CURSOR");
  EXPECT_EQ(state.status, kPbftFinalizationRuntimeStatusActionMismatch);
  EXPECT_FALSE(state.has_action);
  EXPECT_EQ(std::string(state.error_code), "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH");

  const auto cleared =
      pbft_manager_runtime_fail_finalization_external_effect(*runtime, state.cursor, 77, "AFTER_CLEAR");
  EXPECT_EQ(cleared.status, kPbftFinalizationRuntimeStatusActionMismatch);
  EXPECT_FALSE(cleared.has_action);
  EXPECT_EQ(std::string(cleared.error_code), "PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED");
}

TEST(RustPbftSyncTest, FinalizationResumeBoundaryOwnsManagerTailDrain) {
  const auto plan = finalizationIntentPlan(makeFinalizationFact());
  auto runtime = managerRuntimeForFinalizationSession();
  startFreshFinalizationExecutor(*runtime, plan,
                                 storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary),
                                                dynamicLambdaFinalizationStorageStage(plan)}));

  const auto boundary = startResumeFinalizationExecutor(*runtime, plan, plan.storage_write_intent.block_period - 1);

  EXPECT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActive);
  EXPECT_TRUE(boundary.has_action);
  EXPECT_EQ(boundary.action, kPbftFinalizationRuntimeActionFinalizeFinalChain);
  EXPECT_FALSE(boundary.applied_dynamic_lambda);
}

TEST(RustPbftSyncTest, DynamicLambdaPlannerMatchesCactiAdjustmentPolicy) {
  auto runtime = managerRuntimeForFinalizationSession();
  auto plan = pbft_manager_runtime_plan_finalization_dynamic_lambda(*runtime, makeDynamicLambdaFact());

  EXPECT_EQ(plan.status, kPbftFinalizationStatusAccepted);
  EXPECT_TRUE(plan.apply_dynamic_lambda_update);
  EXPECT_EQ(plan.period_lambda, 1500);
  EXPECT_EQ(plan.blocks_per_year, 9275294);
  EXPECT_EQ(plan.rounds_count_dynamic_lambda, 0);
  EXPECT_EQ(plan.dynamic_lambda, 1490);
  EXPECT_TRUE(plan.decreased_dynamic_lambda);
  EXPECT_FALSE(plan.increased_dynamic_lambda);

  auto fact = makeDynamicLambdaFact();
  fact.finalized_period = 21;
  fact.finalized_round = 2;
  fact.pre_adjust_rounds_count_dynamic_lambda = 3;
  fact.pre_adjust_dynamic_lambda = 1495;
  plan = pbft_manager_runtime_plan_finalization_dynamic_lambda(*runtime, fact);
  EXPECT_EQ(plan.period_lambda, 2000);
  EXPECT_EQ(plan.rounds_count_dynamic_lambda, 5);
  EXPECT_EQ(plan.dynamic_lambda, 1500);
  EXPECT_FALSE(plan.decreased_dynamic_lambda);
  EXPECT_TRUE(plan.increased_dynamic_lambda);

  fact = makeDynamicLambdaFact();
  fact.dynamic_lambda_active = false;
  plan = pbft_manager_runtime_plan_finalization_dynamic_lambda(*runtime, fact);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusAccepted);
  EXPECT_FALSE(plan.apply_dynamic_lambda_update);
  EXPECT_EQ(plan.blocks_per_year, 500);
  EXPECT_EQ(plan.dynamic_lambda, 1500);
}
