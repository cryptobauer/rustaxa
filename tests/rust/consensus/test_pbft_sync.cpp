#include <gtest/gtest.h>
#include <libdevcore/RLP.h>

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <initializer_list>
#include <string>
#include <utility>
#include <vector>

#include "rustaxa-bridge/ffi.rs.h"
#include "vote/pbft_vote.hpp"

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
constexpr uint8_t kPbftFinalizationRuntimeActionCommitRewardVotesResetRuntime = 3;
constexpr uint8_t kPbftFinalizationRuntimeActionSetDagBlockOrder = 4;
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
constexpr uint8_t kPbftManagerStartupStatusInvalidFact = 1;
constexpr uint8_t kPbftManagerRuntimeStateFinish = 3;
constexpr uint8_t kPbftManagerAdvanceActionSetVoteManagerPeriodRound = 2;
constexpr uint8_t kPbftManagerAdvanceActionResetCurrentRoundTimer = 3;
constexpr uint8_t kPbftManagerAdvanceActionResetRewardVoteCounters = 4;
constexpr uint8_t kPbftManagerAdvanceActionResetPeriodTimer = 5;
constexpr uint8_t kPbftManagerAdvanceActionUpdateWalletEligibility = 6;

PbftFinalizationStorageWriteStage finalizationStorageStage(uint8_t stage) {
  PbftFinalizationStorageWriteStage write_stage{};
  write_stage.stage = stage;
  write_stage.rounds_count_dynamic_lambda = 0;
  write_stage.dynamic_lambda = 0;
  write_stage.has_sortition_params_change = false;
  write_stage.sortition_params_change_period = 0;
  write_stage.sortition_params_change_interval_efficiency = 0;
  write_stage.sortition_params_change_threshold_upper = 0;
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
    BridgePbftService& runtime, BridgeDagTransactionService& dag_transaction_service,
    const PbftFinalizationIntentPlan& plan, rust::Vec<PbftFinalizationStorageWriteStage> primary_stages) {
  PbftFinalizationExecutorStartRequest request{};
  request.mode = kPbftFinalizationExecutorModeFresh;
  request.plan = plan;
  request.primary_stages = std::move(primary_stages);
  request.sync = false;
  request.final_chain_last_block = 0;
  return pbft_manager_runtime_start_finalization_executor(runtime, dag_transaction_service, request);
}

PbftManagerFinalizationExecutorState startResumeFinalizationExecutor(
    BridgePbftService& runtime, BridgeDagTransactionService& dag_transaction_service,
    const PbftFinalizationIntentPlan& plan, uint64_t final_chain_last_block) {
  PbftFinalizationExecutorStartRequest request{};
  request.mode = kPbftFinalizationExecutorModeResume;
  request.plan = plan;
  request.sync = false;
  request.final_chain_last_block = final_chain_last_block;
  return pbft_manager_runtime_start_finalization_executor(runtime, dag_transaction_service, request);
}

PbftFinalizationIntentFact makeFinalizationFact() {
  PbftFinalizationIntentFact fact;
  fact.block_hash = h256(9);
  fact.block_period = 1;
  fact.block_prev_hash = h256(0);
  fact.block_in_chain = false;
  fact.pivot_dag_anchor_hash = h256(8);
  fact.has_pillar_block = false;
  fact.pillar_block_finalized = false;
  fact.request_dynamic_lambda_update = true;
  fact.cert_vote_count = 3;
  fact.sample_cert_vote_block_hash = h256(9);
  fact.sample_cert_vote_period = 1;
  fact.sample_cert_vote_round = 2;
  fact.sample_cert_vote_step = 3;
  fact.block_lambda = 1500;
  fact.last_saved_period_lambda_found = false;
  fact.last_saved_period_lambda = 0;
  fact.dynamic_blocks_per_year = 1000;
  fact.dpos_blocks_per_year = 500;
  dev::RLPStream period_data;
  period_data.appendList(4).appendList(8)
      << dev::h256(1) << dev::h256(8) << dev::h256(2) << dev::h256(3) << uint64_t{1} << uint64_t{123};
  period_data.appendList(0) << dev::bytes(65, 0);
  period_data << dev::bytes{};
  period_data.appendList(3).appendList(0).appendList(0).appendList(0);
  period_data.appendList(0);
  const auto encoded_period_data = period_data.out();
  fact.period_data_rlp.reserve(encoded_period_data.size());
  for (const auto byte : encoded_period_data) {
    fact.period_data_rlp.push_back(byte);
  }
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
  config.ficus_activation_period = 0;
  config.pillar_blocks_interval = 10;
  config.sync_level_size = 10;
  config.is_light_node = false;
  config.light_node_history = 0;
  config.committee_size = 5;
  config.number_of_proposers = 20;
  return config;
}

std::filesystem::path uniqueTempDir(const std::string& name) {
  const auto nonce = std::chrono::steady_clock::now().time_since_epoch().count();
  auto path = std::filesystem::temp_directory_path() / (name + "_" + std::to_string(nonce));
  std::filesystem::create_directories(path);
  return path;
}

SortitionRuntimeConfig runtimeSortitionConfig(uint16_t changing_interval = 2) {
  SortitionRuntimeConfig config;
  config.threshold_upper = 2000;
  config.difficulty_min = 1;
  config.difficulty_max = 2;
  config.difficulty_stale = 3;
  config.lambda_bound = 1500;
  config.changes_count_for_average = 3;
  config.dag_efficiency_target_low = 48 * 100;
  config.dag_efficiency_target_high = 52 * 100;
  config.changing_interval = changing_interval;
  config.computation_interval = changing_interval;
  return config;
}

rust::Box<BridgeDagTransactionService> createDagTransactionServiceForFinalizationTest(
    const rust::Box<BridgeStorage>& storage, uint16_t changing_interval = 2) {
  std::array<uint8_t, 32> genesis{};
  genesis.fill(1);
  TransactionQueueConfig queue_config;
  queue_config.max_size = 1000;
  GasPricerConfig gas_config;
  gas_config.percentile = 50;
  gas_config.history_blocks = 10;
  return create_dag_transaction_service_from_storage(
      *storage, genesis, 32, 100, runtimeSortitionConfig(changing_interval), queue_config, gas_config, UINT64_MAX);
}

struct FinalizationServices {
  rust::Box<BridgePbftService> pbft;
  rust::Box<BridgeDagTransactionService> dag_transaction_service;

  BridgePbftService& operator*() { return *pbft; }
};

FinalizationServices managerRuntimeForFinalizationSession() {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_finalization_session");
  auto storage = create_storage(test_dir.string());
  auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig(false));
  auto dag_transaction_service = createDagTransactionServiceForFinalizationTest(storage);
  return FinalizationServices{std::move(runtime), std::move(dag_transaction_service)};
}

void seedRewardCertVote(BridgeStorage& storage, BridgePbftService& service) {
  rust::Vec<GenesisAccount> accounts;
  rust::Vec<GenesisValidator> validators;
  GenesisDposConfig dpos_config{};
  FinalChainRewardsConfig rewards_config{};
  auto final_chain = create_final_chain_with_rewards_config(storage, 0, 0, std::move(accounts), std::move(validators),
                                                            std::move(dpos_config), std::move(rewards_config));

  const taraxa::secret_t node_secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                     dev::Secret::ConstructFromStringType::FromHex);
  const taraxa::vrf_wrapper::vrf_sk_t vrf_secret(
      "0b6627a6680e01cea3d9f36fa797f7f34e8869c3a526d9ed63ed8170e35542aad05dc12c"
      "1df1edc9f3367fba550b7971fc2de6c5998d8784051c5be69abc9644");
  taraxa::VrfPbftMsg message(taraxa::PbftVoteTypes::cert_vote, 1, 2, 3);
  taraxa::PbftVote vote(node_secret, taraxa::VrfPbftSortition(vrf_secret, message), taraxa::blk_hash_t(9));
  const auto vote_rlp = vote.rlp();

  PbftVoteAdmissionValidationRequest validation{};
  validation.strict_vrf = false;
  validation.committee_size = 1;
  validation.number_of_proposers = 1;
  validation.has_preverified_weight = true;
  validation.preverified_weight = 1;
  PbftVoteEventFactFlags flags{};
  flags.carries_proposed_block = true;
  PbftVoteProgressContext context{};
  context.current_period = 1;
  context.current_round = 2;
  context.has_two_t_plus_one_threshold = true;
  context.two_t_plus_one_threshold = 1;
  context.slashing_enabled = true;
  const auto result = service.pbft_service_verified_votes_admit_and_persist_with_final_chain(
      *final_chain, rust::Slice<const uint8_t>(vote_rlp.data(), vote_rlp.size()), validation, flags, context,
      rust::Vec<SlashingSubmitterIdentity>{});
  ASSERT_TRUE(result.transition_published);
}

PbftFinalizationIntentPlan finalizationIntentPlan(PbftFinalizationIntentFact fact) {
  auto runtime = managerRuntimeForFinalizationSession();
  return pbft_manager_runtime_plan_finalization_intent(*runtime, std::move(fact));
}

PbftFinalizationIntentPlan withoutRewardVoteReset(PbftFinalizationIntentPlan plan) {
  plan.cleanup.reset_reward_votes = false;
  plan.storage_write_intent.reset_reward_votes = false;
  return plan;
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
  const auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig());
  const auto snapshot = pbft_manager_runtime_snapshot(*runtime);

  EXPECT_EQ(snapshot.status, kPbftManagerStartupStatusReady);
  EXPECT_EQ(snapshot.state, kPbftManagerRuntimeStateFinish);
  EXPECT_EQ(snapshot.period, 1);
  EXPECT_EQ(snapshot.round, 2);
  EXPECT_EQ(snapshot.step, 4);
  EXPECT_EQ(snapshot.current_round_lambda_ms, 100);
  EXPECT_EQ(snapshot.dynamic_lambda_ms, 1'500);
  EXPECT_TRUE(snapshot.executed_pbft_block);
  EXPECT_TRUE(snapshot.already_next_voted_value);
  EXPECT_FALSE(snapshot.already_next_voted_null);
  EXPECT_EQ(pbftQueries(storage)->get_pbft_mgr_field(kPbftMgrFieldStep), 4);

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
                kPbftManagerAdvanceActionUpdateWalletEligibility}));

  const auto committed = pbft_manager_runtime_apply_period_advance(*runtime, plan.new_period);
  EXPECT_EQ(committed.status, kPbftManagerStartupStatusReady);
  EXPECT_EQ(committed.period, 13);
  EXPECT_TRUE(committed.error_code.empty());

  const auto duplicate = pbft_manager_runtime_apply_period_advance(*runtime, plan.new_period);
  EXPECT_EQ(duplicate.status, kPbftManagerStartupStatusInvalidFact);
  EXPECT_EQ(duplicate.period, 13);
  EXPECT_EQ(duplicate.error_code, "PBFT_MANAGER_ADVANCE_PERIOD_NON_INCREASING_PERIOD");
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
  EXPECT_EQ(plan.storage_write_intent.pbft_head_hash, h256(0));
  EXPECT_EQ(plan.storage_write_intent.block_period, 1);
  EXPECT_FALSE(plan.storage_write_intent.null_anchor);
  EXPECT_EQ(plan.storage_write_intent.anchor_hash, h256(8));
  EXPECT_EQ(plan.storage_write_intent.reward_vote_period, 1);
  EXPECT_EQ(plan.storage_write_intent.reward_vote_round, 2);
  EXPECT_EQ(plan.storage_write_intent.reward_vote_step, 3);
  EXPECT_EQ(plan.storage_write_intent.reward_vote_block_hash, h256(9));
  EXPECT_EQ(plan.storage_write_intent.period_lambda, 1500);
  EXPECT_EQ(plan.storage_write_intent.blocks_per_year, 1000);
  EXPECT_TRUE(plan.storage_write_intent.executed_pbft_status);
  const auto expected_head_payload =
      std::string("{\n\t\"head_hash\" : \"0x") + std::string(64, '0') +
      "\",\n\t\"last_pbft_block_hash\" : \"0x" + std::string(63, '0') +
      "9\",\n\t\"non_empty_size\" : 1,\n\t\"size\" : 1\n}\n";
  EXPECT_EQ(std::vector<uint8_t>(plan.storage_write_intent.pbft_head_payload.begin(),
                                 plan.storage_write_intent.pbft_head_payload.end()),
            std::vector<uint8_t>(expected_head_payload.begin(), expected_head_payload.end()));
  EXPECT_GT(plan.storage_write_intent.period_data_rlp.size(), 1);
  ASSERT_EQ(plan.storage_write_intent.dag_block_period_writes.size(), 2);
  EXPECT_EQ(plan.storage_write_intent.dag_block_period_writes[0].hash, h256(2));
  EXPECT_EQ(plan.storage_write_intent.dag_block_period_writes[0].position, 0);
  EXPECT_EQ(plan.storage_write_intent.dag_block_period_writes[1].hash, h256(3));
  EXPECT_EQ(plan.storage_write_intent.dag_block_period_writes[1].position, 1);
  ASSERT_EQ(plan.storage_write_intent.transaction_location_writes.size(), 1);
  EXPECT_EQ(plan.storage_write_intent.transaction_location_writes[0].hash, h256(4));
  EXPECT_EQ(plan.storage_write_intent.transaction_location_writes[0].position, 0);
}

TEST(RustPbftSyncTest, FinalizationBoundaryAcceptsCompatibleDagService) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_finalization_sortition_invariant");
  auto storage = create_storage(test_dir.string());
  auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig(false));
  const auto plan =
      withoutRewardVoteReset(pbft_manager_runtime_plan_finalization_intent(*runtime, makeFinalizationFact()));
  rust::Box<BridgeDagTransactionService> dag_transaction_service;
  try {
    dag_transaction_service = createDagTransactionServiceForFinalizationTest(storage);
  } catch (const std::exception& error) {
    FAIL() << "first DagTransactionService restore failed: " << error.what();
    return;
  }

  rust::Box<BridgeDagTransactionService> boundary_state;
  try {
    boundary_state = startFreshFinalizationExecutor(*runtime, *dag_transaction_service, plan,
                                                  storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));
  } catch (const std::exception& error) {
    FAIL() << "startFreshFinalizationExecutor failed: " << error.what();
    return;
  }

  rust::Box<BridgeDagTransactionService> compatible_dag_transaction_service;
  try {
    compatible_dag_transaction_service = createDagTransactionServiceForFinalizationTest(storage);
  } catch (const std::exception& error) {
    FAIL() << "compatible DagTransactionService restore failed: " << error.what();
    return;
  }

  const auto boundary = std::move(boundary_state);
  const auto state = pbft_manager_runtime_advance_finalization_action(*runtime, *compatible_dag_transaction_service,
                                                                     boundary.cursor, boundary.action, 0, 0, 0, {});
  EXPECT_EQ(state.status, kPbftFinalizationRuntimeStatusActive);
  EXPECT_EQ(state.action, kPbftFinalizationRuntimeActionSetDagBlockOrder);
  EXPECT_TRUE(state.can_continue);
}

TEST(RustPbftSyncTest, FinalizationBoundaryBeginsAtFirstExternalAction) {
  const auto plan = withoutRewardVoteReset(finalizationIntentPlan(makeFinalizationFact()));
  auto runtime = managerRuntimeForFinalizationSession();

  const auto boundary =
      startFreshFinalizationExecutor(*runtime, *runtime.dag_transaction_service, plan,
                                     storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));

  EXPECT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActive);
  EXPECT_EQ(boundary.cursor, 1);
  EXPECT_TRUE(boundary.has_action);
  EXPECT_EQ(boundary.action, kPbftFinalizationRuntimeActionCommitSortitionRuntime);
  EXPECT_TRUE(boundary.can_continue);
}

TEST(RustPbftSyncTest, FinalizationBoundaryRejectsMissingNativeRewardVotes) {
  const auto plan = finalizationIntentPlan(makeFinalizationFact());
  auto runtime = managerRuntimeForFinalizationSession();

  try {
    startFreshFinalizationExecutor(*runtime, *runtime.dag_transaction_service, plan,
                                   storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));
    FAIL() << "missing native reward votes must reject fresh finalization";
  } catch (const std::exception& error) {
    EXPECT_EQ(std::string(error.what()), "PBFT_REWARD_VOTES_RESET_CERT_MAPPING_MISSING");
  }
}

TEST(RustPbftSyncTest, FinalizationBoundaryAdvancesNativeRewardVotes) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_finalization_reward_success");
  auto storage = create_storage(test_dir.string());
  auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig(false));
  auto dag_transaction_service = createDagTransactionServiceForFinalizationTest(storage);
  seedRewardCertVote(*storage, *runtime);
  const auto plan = pbft_manager_runtime_plan_finalization_intent(*runtime, makeFinalizationFact());

  auto boundary =
      startFreshFinalizationExecutor(*runtime, *dag_transaction_service, plan,
                                     storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));
  boundary = pbft_manager_runtime_advance_finalization_action(*runtime, *dag_transaction_service, boundary.cursor,
                                                              boundary.action, 0, 0, 0, {});
  ASSERT_EQ(boundary.action, kPbftFinalizationRuntimeActionCommitRewardVotesResetRuntime);
  boundary = pbft_manager_runtime_advance_finalization_action(*runtime, *dag_transaction_service, boundary.cursor,
                                                              boundary.action, 0, 0, 0, {});
  EXPECT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActive) << std::string(boundary.error_code);
  EXPECT_EQ(boundary.action, kPbftFinalizationRuntimeActionSetDagBlockOrder);
  EXPECT_TRUE(boundary.can_continue);

  const auto stale = pbft_manager_runtime_advance_finalization_action(
      *runtime, *dag_transaction_service, boundary.cursor - 1, boundary.action, 0, 0, 0, {});
  EXPECT_EQ(stale.status, kPbftFinalizationRuntimeStatusActionMismatch);
  EXPECT_FALSE(stale.has_action);
}

TEST(RustPbftSyncTest, FinalizationResumeReplaysAuthenticatedRewardPublication) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_finalization_reward_resume");
  auto storage = create_storage(test_dir.string());
  auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig(false));
  auto dag_transaction_service = createDagTransactionServiceForFinalizationTest(storage);
  seedRewardCertVote(*storage, *runtime);
  auto fact = makeFinalizationFact();
  fact.request_dynamic_lambda_update = false;
  const auto plan = pbft_manager_runtime_plan_finalization_intent(*runtime, std::move(fact));

  auto boundary =
      startFreshFinalizationExecutor(*runtime, *dag_transaction_service, plan,
                                     storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));
  boundary = pbft_manager_runtime_advance_finalization_action(*runtime, *dag_transaction_service, boundary.cursor,
                                                              boundary.action, 0, 0, 0, {});
  ASSERT_EQ(boundary.action, kPbftFinalizationRuntimeActionCommitRewardVotesResetRuntime);

  boundary = startResumeFinalizationExecutor(*runtime, *dag_transaction_service, plan, 0);
  ASSERT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActive);
  ASSERT_EQ(boundary.action, kPbftFinalizationRuntimeActionCommitRewardVotesResetRuntime);

  boundary = pbft_manager_runtime_advance_finalization_action(*runtime, *dag_transaction_service, boundary.cursor,
                                                              boundary.action, 0, 0, 0, {});
  EXPECT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActive) << std::string(boundary.error_code);
  EXPECT_EQ(boundary.action, kPbftFinalizationRuntimeActionFinalizeFinalChain);
  EXPECT_TRUE(boundary.can_continue);
}

TEST(RustPbftSyncTest, FinalizationResumeReplaysAuthenticatedSortitionPublication) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_finalization_sortition_resume");
  auto storage = create_storage(test_dir.string());
  auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig(false));
  auto dag_transaction_service = createDagTransactionServiceForFinalizationTest(storage, 1);
  auto fact = makeFinalizationFact();
  fact.request_dynamic_lambda_update = false;
  const auto plan = withoutRewardVoteReset(pbft_manager_runtime_plan_finalization_intent(*runtime, std::move(fact)));

  auto boundary =
      startFreshFinalizationExecutor(*runtime, *dag_transaction_service, plan,
                                     storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));
  ASSERT_EQ(boundary.action, kPbftFinalizationRuntimeActionCommitSortitionRuntime);

  boundary = startResumeFinalizationExecutor(*runtime, *dag_transaction_service, plan, 0);
  ASSERT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActive);
  ASSERT_EQ(boundary.action, kPbftFinalizationRuntimeActionCommitSortitionRuntime);

  boundary = pbft_manager_runtime_advance_finalization_action(*runtime, *dag_transaction_service, boundary.cursor,
                                                              boundary.action, 0, 0, 0, {});
  EXPECT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActive) << std::string(boundary.error_code);
  EXPECT_EQ(boundary.action, kPbftFinalizationRuntimeActionFinalizeFinalChain);
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
  const auto plan = withoutRewardVoteReset(finalizationIntentPlan(makeFinalizationFact()));
  auto boundary =
      startFreshFinalizationExecutor(*runtime, *runtime.dag_transaction_service, plan,
                                     storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));
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
  const auto plan = withoutRewardVoteReset(finalizationIntentPlan(makeFinalizationFact()));
  auto state =
      startFreshFinalizationExecutor(*runtime, *runtime.dag_transaction_service, plan,
                                     storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}));
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
  const auto plan = withoutRewardVoteReset(finalizationIntentPlan(makeFinalizationFact()));
  auto runtime = managerRuntimeForFinalizationSession();
  startFreshFinalizationExecutor(*runtime, *runtime.dag_transaction_service, plan,
                                 storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary),
                                                dynamicLambdaFinalizationStorageStage(plan)}));

  const auto boundary = startResumeFinalizationExecutor(*runtime, *runtime.dag_transaction_service, plan,
                                                        plan.storage_write_intent.block_period - 1);

  EXPECT_EQ(boundary.status, kPbftFinalizationRuntimeStatusActive);
  EXPECT_TRUE(boundary.has_action);
  EXPECT_EQ(boundary.action, kPbftFinalizationRuntimeActionFinalizeFinalChain);
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
