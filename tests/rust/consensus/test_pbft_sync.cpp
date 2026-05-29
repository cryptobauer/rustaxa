#include <gtest/gtest.h>

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
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

PbftSyncTransactionHash tx(uint8_t last_byte) { return PbftSyncTransactionHash{h256(last_byte)}; }

std::vector<std::array<uint8_t, 32>> hashes(const rust::Vec<PbftSyncTransactionHash>& input) {
  std::vector<std::array<uint8_t, 32>> out;
  out.reserve(input.size());
  for (const auto& hash : input) {
    out.push_back(hash.hash);
  }
  return out;
}

constexpr uint8_t kPbftSyncFactValid = 0;
constexpr uint8_t kPbftSyncFactNotRequired = 2;
constexpr uint8_t kPbftSyncFactNotChecked = 3;
constexpr uint8_t kPbftSyncFinalChainHashValid = 0;
constexpr uint8_t kPbftSyncFinalChainHashMissing = 1;
constexpr uint8_t kPbftSyncRuntimeActionRunCheck = 0;
constexpr uint8_t kPbftSyncRuntimeActionAccept = 1;
constexpr uint8_t kPbftSyncRuntimeActionWaitForFinalization = 3;
constexpr uint8_t kPbftSyncNextCheckNone = 0;
constexpr uint8_t kPbftSyncNextCheckValidateFinalChainHash = 1;
constexpr uint8_t kPbftSyncNextCheckCheckRewardVotes = 2;
constexpr uint8_t kPbftSyncNextCheckCheckTransactions = 4;
constexpr uint8_t kPbftSyncTransactionWarningMissing = 1;
constexpr uint8_t kPbftSyncTransactionWarningFinalized = 2;
constexpr uint8_t kPbftFinalizationAnchorNull = 0;
constexpr uint8_t kPbftFinalizationAnchorAnchored = 1;
constexpr uint8_t kPbftFinalizationStatusAccepted = 0;
constexpr uint8_t kPbftFinalizationStatusBlockAlreadyInChain = 1;
constexpr uint8_t kPbftFinalizationStatusPillarDependencyMissing = 4;
constexpr uint8_t kPbftFinalizationStatusEmptyCertVotes = 5;
constexpr uint8_t kPbftFinalizationStatusCertVoteBlockMismatch = 6;
constexpr uint8_t kPbftFinalizationRuntimeActionPrimaryStorage = 0;
constexpr uint8_t kPbftFinalizationRuntimeActionRewardResetStorage = 1;
constexpr uint8_t kPbftFinalizationRuntimeActionSortitionStorage = 2;
constexpr uint8_t kPbftFinalizationRuntimeActionCommitRewardReset = 3;
constexpr uint8_t kPbftFinalizationRuntimeActionSetDagOrder = 4;
constexpr uint8_t kPbftFinalizationRuntimeActionUpdateTransactions = 5;
constexpr uint8_t kPbftFinalizationRuntimeActionUpdatePbftChain = 6;
constexpr uint8_t kPbftFinalizationRuntimeActionClearAnchorCache = 7;
constexpr uint8_t kPbftFinalizationRuntimeActionApplyDynamicLambda = 8;
constexpr uint8_t kPbftFinalizationRuntimeActionFinalizeFinalChain = 9;
constexpr uint8_t kPbftFinalizationRuntimeActionPersistExecutedStatus = 10;
constexpr uint8_t kPbftFinalizationRuntimeActionSetExecutedFlag = 11;
constexpr uint8_t kPbftFinalizationRuntimeActionAdvancePeriod = 12;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusApplied = 0;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusAlreadyApplied = 1;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusRejected = 2;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusMissingPayload = 3;
constexpr uint8_t kPbftMgrFieldLambda = 2;
constexpr uint8_t kPbftMgrStatusExecutedBlock = 0;
constexpr uint8_t kPbftFinalizationStorageStagePrimary = 0;
constexpr uint8_t kPbftFinalizationStorageStageDynamicLambda = 1;
constexpr uint8_t kPbftFinalizationStorageStageExecutedStatus = 2;
constexpr uint8_t kPbftFinalizationStorageStageSortition = 3;
constexpr uint8_t kPbftFinalizationStorageStageRewardReset = 4;

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

PbftFinalizationStorageWriteStage sortitionFinalizationStorageStage(uint64_t period, uint16_t interval_efficiency,
                                                                    uint16_t threshold_upper) {
  auto stage = finalizationStorageStage(kPbftFinalizationStorageStageSortition);
  stage.has_sortition_params_change = true;
  stage.sortition_params_change_period = period;
  stage.sortition_params_change_interval_efficiency = interval_efficiency;
  stage.sortition_params_change_threshold_upper = threshold_upper;
  return stage;
}

PbftFinalizationStorageWriteStage rewardResetFinalizationStorageStage(
    rust::Vec<uint8_t> cert_votes_bundle_rlp,
    const std::vector<std::array<uint8_t, 32>>& stale_extra_reward_vote_hashes) {
  auto stage = finalizationStorageStage(kPbftFinalizationStorageStageRewardReset);
  stage.has_reward_votes_reset = true;
  stage.reward_votes_bundle_rlp = std::move(cert_votes_bundle_rlp);
  stage.extra_reward_vote_hashes.reserve(stale_extra_reward_vote_hashes.size());
  for (const auto& hash : stale_extra_reward_vote_hashes) {
    stage.extra_reward_vote_hashes.push_back(PbftFinalizationHash{hash});
  }
  return stage;
}

PbftSyncPeriodAdmissionFact makeAdmissionFact() {
  PbftSyncPeriodAdmissionFact fact;
  fact.block_period = 101;
  fact.block_prev_hash = h256(1);
  fact.chain_last_hash = h256(1);
  fact.chain_last_period = 100;
  fact.block_in_chain = false;
  fact.final_chain_hash_status = kPbftSyncFinalChainHashValid;
  fact.reward_votes_status = kPbftSyncFactValid;
  fact.cert_votes_status = kPbftSyncFactValid;
  fact.contains_finalized_transactions = false;
  fact.pillar_data_status = kPbftSyncFactValid;
  fact.pillar_votes_status = kPbftSyncFactNotRequired;
  return fact;
}

PbftSyncProcessPeriodDataRuntimeFact makeRuntimeFact() {
  PbftSyncProcessPeriodDataRuntimeFact fact;
  fact.block_period = 101;
  fact.block_prev_hash = h256(1);
  fact.chain_last_hash = h256(1);
  fact.chain_last_period = 100;
  fact.block_in_chain = false;
  fact.final_chain_hash_status = kPbftSyncFactNotChecked;
  fact.reward_votes_status = kPbftSyncFactNotChecked;
  fact.cert_votes_status = kPbftSyncFactNotChecked;
  fact.transactions_status = kPbftSyncFactNotChecked;
  fact.dag_transaction_hashes = {tx(1), tx(2), tx(1)};
  fact.period_data_transaction_hashes = {tx(2)};
  fact.contains_finalized_transactions = false;
  fact.pillar_data_status = kPbftSyncFactNotChecked;
  fact.pillar_votes_required = true;
  fact.pillar_votes_status = kPbftSyncFactNotChecked;
  fact.previous_cert_votes_present = true;
  fact.previous_cert_first_vote_has_weight = false;
  return fact;
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

std::filesystem::path uniqueTempDir(const std::string& name) {
  const auto nonce = std::chrono::steady_clock::now().time_since_epoch().count();
  auto path = std::filesystem::temp_directory_path() / (name + "_" + std::to_string(nonce));
  std::filesystem::create_directories(path);
  return path;
}

rust::Vec<uint8_t> bytes(std::initializer_list<uint8_t> values) {
  rust::Vec<uint8_t> out;
  out.reserve(values.size());
  for (auto value : values) {
    out.push_back(value);
  }
  return out;
}

rust::Vec<uint8_t> hashBytes(const std::array<uint8_t, 32>& hash) {
  rust::Vec<uint8_t> out;
  out.reserve(hash.size());
  for (auto value : hash) {
    out.push_back(value);
  }
  return out;
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

TEST(RustPbftSyncTest, TransactionQueryPlansUniqueMissingDagTransactionsInOrder) {
  PbftSyncTransactionQueryFact fact;
  fact.dag_transaction_hashes.push_back(tx(1));
  fact.dag_transaction_hashes.push_back(tx(2));
  fact.dag_transaction_hashes.push_back(tx(1));
  fact.dag_transaction_hashes.push_back(tx(3));
  fact.dag_transaction_hashes.push_back(tx(4));
  fact.period_data_transaction_hashes.push_back(tx(2));
  fact.period_data_transaction_hashes.push_back(tx(4));

  const auto plan = plan_pbft_sync_transaction_query(std::move(fact));

  EXPECT_EQ(hashes(plan.finalized_lookup_hashes), (std::vector{h256(1), h256(3)}));
}

TEST(RustPbftSyncTest, PeriodAdmissionPlanAcceptsWithRuntimeWarnings) {
  auto fact = makeAdmissionFact();
  fact.missing_transaction_hashes = {tx(2), tx(4)};
  fact.finalized_transaction_hashes = {tx(7)};
  fact.contains_finalized_transactions = true;

  const auto plan = plan_pbft_sync_period_admission(std::move(fact));

  EXPECT_TRUE(plan.accept_period_data);
  EXPECT_FALSE(plan.clear_sync_queue);
  EXPECT_FALSE(plan.report_malicious_peer);
  EXPECT_FALSE(plan.wait_for_finalization);
  EXPECT_EQ(plan.warnings.size(), 3);
  EXPECT_EQ(plan.warnings[0].kind, kPbftSyncTransactionWarningMissing);
  EXPECT_EQ(plan.warnings[2].kind, kPbftSyncTransactionWarningFinalized);
  EXPECT_TRUE(plan.contains_finalized_transaction_warning);
}

TEST(RustPbftSyncTest, PeriodAdmissionPlanWaitsWhenFinalChainHashIsMissing) {
  auto fact = makeAdmissionFact();
  fact.final_chain_hash_status = kPbftSyncFinalChainHashMissing;

  const auto plan = plan_pbft_sync_period_admission(std::move(fact));

  EXPECT_TRUE(plan.wait_for_finalization);
  EXPECT_FALSE(plan.accept_period_data);
  EXPECT_FALSE(plan.clear_sync_queue);
}

TEST(RustPbftSyncTest, PeriodAdmissionPlanClearsAndReportsForPrevHashMismatch) {
  auto fact = makeAdmissionFact();
  fact.block_prev_hash = h256(2);

  const auto plan = plan_pbft_sync_period_admission(std::move(fact));

  EXPECT_FALSE(plan.accept_period_data);
  EXPECT_TRUE(plan.clear_sync_queue);
  EXPECT_TRUE(plan.report_malicious_peer);
}

TEST(RustPbftSyncTest, ProcessPeriodRuntimeRequestsChecksInOrder) {
  auto fact = makeRuntimeFact();
  auto plan = plan_pbft_sync_process_period_data_runtime(std::move(fact));

  EXPECT_EQ(plan.runtime_action, kPbftSyncRuntimeActionRunCheck);
  EXPECT_EQ(plan.next_check, kPbftSyncNextCheckValidateFinalChainHash);
  EXPECT_TRUE(plan.replace_previous_block_cert_votes);

  fact = makeRuntimeFact();
  fact.final_chain_hash_status = kPbftSyncFinalChainHashValid;
  plan = plan_pbft_sync_process_period_data_runtime(std::move(fact));
  EXPECT_EQ(plan.runtime_action, kPbftSyncRuntimeActionRunCheck);
  EXPECT_EQ(plan.next_check, kPbftSyncNextCheckCheckRewardVotes);

  fact = makeRuntimeFact();
  fact.final_chain_hash_status = kPbftSyncFinalChainHashValid;
  fact.reward_votes_status = kPbftSyncFactValid;
  fact.cert_votes_status = kPbftSyncFactValid;
  plan = plan_pbft_sync_process_period_data_runtime(std::move(fact));
  EXPECT_EQ(plan.runtime_action, kPbftSyncRuntimeActionRunCheck);
  EXPECT_EQ(plan.next_check, kPbftSyncNextCheckCheckTransactions);
  EXPECT_EQ(hashes(plan.transaction_query_plan.finalized_lookup_hashes), (std::vector{h256(1)}));
}

TEST(RustPbftSyncTest, ProcessPeriodRuntimeWaitsAndAccepts) {
  auto fact = makeRuntimeFact();
  fact.final_chain_hash_status = kPbftSyncFinalChainHashMissing;
  auto plan = plan_pbft_sync_process_period_data_runtime(std::move(fact));
  EXPECT_EQ(plan.runtime_action, kPbftSyncRuntimeActionWaitForFinalization);
  EXPECT_TRUE(plan.wait_for_finalization);
  EXPECT_TRUE(plan.retry_same_candidate);

  fact = makeRuntimeFact();
  fact.final_chain_hash_status = kPbftSyncFinalChainHashValid;
  fact.reward_votes_status = kPbftSyncFactValid;
  fact.cert_votes_status = kPbftSyncFactValid;
  fact.transactions_status = kPbftSyncFactValid;
  fact.missing_transaction_hashes = {tx(1)};
  fact.contains_finalized_transactions = true;
  fact.pillar_data_status = kPbftSyncFactValid;
  fact.pillar_votes_status = kPbftSyncFactValid;
  plan = plan_pbft_sync_process_period_data_runtime(std::move(fact));

  EXPECT_EQ(plan.runtime_action, kPbftSyncRuntimeActionAccept);
  EXPECT_EQ(plan.next_check, kPbftSyncNextCheckNone);
  EXPECT_TRUE(plan.accept_period_data);
  ASSERT_EQ(plan.warnings.size(), 1);
  EXPECT_EQ(plan.warnings[0].kind, kPbftSyncTransactionWarningMissing);
  EXPECT_TRUE(plan.contains_finalized_transaction_warning);
}

TEST(RustPbftSyncTest, FinalizationIntentAcceptsAnchoredBlockAndMapsCleanup) {
  const auto plan = plan_pbft_finalization_intent(makeFinalizationFact());

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

TEST(RustPbftSyncTest, FinalizationIntentRejectsAlreadyPersistedBlock) {
  auto fact = makeFinalizationFact();
  fact.block_in_chain = true;

  const auto plan = plan_pbft_finalization_intent(std::move(fact));

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

  auto plan = plan_pbft_finalization_intent(std::move(fact));

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
  plan = plan_pbft_finalization_intent(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusBlockAlreadyInChain);
  EXPECT_FALSE(plan.cleanup.advance_period);
  expectNoFinalizationStorageWrites(plan.storage_write_intent);

  fact = makeFinalizationFact();
  fact.has_pillar_block = true;
  fact.pillar_block_finalized = false;
  plan = plan_pbft_finalization_intent(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusPillarDependencyMissing);
  EXPECT_FALSE(plan.cleanup.finalize_final_chain);
  expectNoFinalizationCleanup(plan.cleanup);
  expectNoFinalizationStorageWrites(plan.storage_write_intent);
}

TEST(RustPbftSyncTest, FinalizationIntentRejectsMalformedCertVoteFacts) {
  auto fact = makeFinalizationFact();
  fact.cert_vote_count = 0;

  auto plan = plan_pbft_finalization_intent(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusEmptyCertVotes);
  expectNoFinalizationStorageWrites(plan.storage_write_intent);

  fact = makeFinalizationFact();
  fact.sample_cert_vote_block_hash = h256(10);
  plan = plan_pbft_finalization_intent(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusCertVoteBlockMismatch);
  expectNoFinalizationStorageWrites(plan.storage_write_intent);
}

TEST(RustPbftSyncTest, FinalizationRuntimePlanOrdersMixedExecutorActions) {
  const auto intent = plan_pbft_finalization_intent(makeFinalizationFact());
  const auto runtime = plan_pbft_finalization_runtime(intent);

  EXPECT_TRUE(runtime.finalize_block);
  EXPECT_EQ(runtime.status, kPbftFinalizationStatusAccepted);
  const std::vector<uint8_t> actions(runtime.actions.begin(), runtime.actions.end());
  EXPECT_EQ(actions, (std::vector<uint8_t>{
                         kPbftFinalizationRuntimeActionPrimaryStorage,
                         kPbftFinalizationRuntimeActionRewardResetStorage,
                         kPbftFinalizationRuntimeActionSortitionStorage,
                         kPbftFinalizationRuntimeActionCommitRewardReset,
                         kPbftFinalizationRuntimeActionSetDagOrder,
                         kPbftFinalizationRuntimeActionUpdateTransactions,
                         kPbftFinalizationRuntimeActionUpdatePbftChain,
                         kPbftFinalizationRuntimeActionClearAnchorCache,
                         kPbftFinalizationRuntimeActionApplyDynamicLambda,
                         kPbftFinalizationRuntimeActionFinalizeFinalChain,
                         kPbftFinalizationRuntimeActionPersistExecutedStatus,
                         kPbftFinalizationRuntimeActionSetExecutedFlag,
                         kPbftFinalizationRuntimeActionAdvancePeriod,
                     }));

  auto rejected_fact = makeFinalizationFact();
  rejected_fact.block_in_chain = true;
  const auto rejected_intent = plan_pbft_finalization_intent(std::move(rejected_fact));
  const auto rejected_runtime = plan_pbft_finalization_runtime(rejected_intent);
  EXPECT_FALSE(rejected_runtime.finalize_block);
  EXPECT_TRUE(rejected_runtime.actions.empty());
}

TEST(RustPbftSyncTest, DynamicLambdaPlannerMatchesCactiAdjustmentPolicy) {
  auto plan = plan_pbft_dynamic_lambda(makeDynamicLambdaFact());

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
  plan = plan_pbft_dynamic_lambda(fact);
  EXPECT_EQ(plan.period_lambda, 2000);
  EXPECT_EQ(plan.rounds_count_dynamic_lambda, 5);
  EXPECT_EQ(plan.dynamic_lambda, 1500);
  EXPECT_FALSE(plan.decreased_dynamic_lambda);
  EXPECT_TRUE(plan.increased_dynamic_lambda);

  fact = makeDynamicLambdaFact();
  fact.dynamic_lambda_active = false;
  plan = plan_pbft_dynamic_lambda(fact);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusAccepted);
  EXPECT_FALSE(plan.apply_dynamic_lambda_update);
  EXPECT_EQ(plan.blocks_per_year, 500);
  EXPECT_EQ(plan.dynamic_lambda, 1500);
}

TEST(RustPbftSyncTest, FinalizedPeriodStorageAppenderWritesPrimaryBatch) {
  constexpr uint8_t kDagBlocksColumn = 4;
  constexpr uint8_t kTransactionsColumn = 6;
  const auto test_dir = uniqueTempDir("rustaxa_pbft_finalized_period_apply");

  auto storage = create_storage(test_dir.string());
  auto seed_batch = storage->create_write_batch();
  storage->batch_put(seed_batch, kDagBlocksColumn, hashBytes(h256(2)), bytes({0xda}));
  storage->batch_put(seed_batch, kTransactionsColumn, hashBytes(h256(4)), bytes({0xd0}));
  storage->commit_write_batch(seed_batch, false);

  const auto plan = plan_pbft_finalization_intent(makeFinalizationFact());
  auto batch_id = storage->create_write_batch();
  const auto result = append_pbft_finalization_storage_write(
      *storage, batch_id, plan.storage_write_intent, finalizationStorageStage(kPbftFinalizationStorageStagePrimary));

  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusApplied);
  EXPECT_TRUE(result.wrote_pbft_head);
  EXPECT_TRUE(result.wrote_period_data);
  EXPECT_EQ(result.dag_index_writes, 2);
  EXPECT_EQ(result.transaction_location_writes, 1);

  storage->commit_write_batch(batch_id, false);

  const auto pbft_head = storage->get_pbft_head(h256(8));
  EXPECT_EQ(std::vector<uint8_t>(pbft_head.begin(), pbft_head.end()),
            (std::vector<uint8_t>{'{', '"', 'h', 'e', 'a', 'd', '"', ':', 't', 'r', 'u', 'e', '}'}));
  const auto period_data = storage->get_period_data_raw(101);
  EXPECT_EQ(std::vector<uint8_t>(period_data.begin(), period_data.end()), (std::vector<uint8_t>{0xc0}));
  EXPECT_TRUE(storage->get_transaction(h256(4)).empty());

  const auto dag_lookup = storage->get_dag_block_period_lookup(h256(2));
  EXPECT_TRUE(dag_lookup.found);
  EXPECT_EQ(dag_lookup.period, 101);
  EXPECT_EQ(dag_lookup.position, 0);
  EXPECT_FALSE(storage->get_transaction_location(h256(4)).empty());
  const auto period_lambda = storage->get_period_lambda(101, false);
  EXPECT_FALSE(period_lambda.found);
  EXPECT_FALSE(storage->get_pbft_mgr_status(kPbftMgrStatusExecutedBlock));

  auto sortition_batch_id = storage->create_write_batch();
  const auto sortition_result = append_pbft_finalization_storage_write(
      *storage, sortition_batch_id, plan.storage_write_intent, sortitionFinalizationStorageStage(101, 2500, 1300));

  EXPECT_EQ(sortition_result.status, kPbftFinalizedPeriodApplyStatusApplied);
  EXPECT_FALSE(sortition_result.wrote_pbft_head);
  EXPECT_FALSE(sortition_result.wrote_period_data);

  storage->commit_write_batch(sortition_batch_id, false);
  EXPECT_FALSE(storage->get_params_change_for_period(101).empty());

  auto reward_reset_batch_id = storage->create_write_batch();
  const auto reward_reset_result =
      append_pbft_finalization_storage_write(*storage, reward_reset_batch_id, plan.storage_write_intent,
                                             rewardResetFinalizationStorageStage(bytes({0xc2, 0x01, 0x02}), {}));

  EXPECT_EQ(reward_reset_result.status, kPbftFinalizedPeriodApplyStatusApplied);
  EXPECT_FALSE(reward_reset_result.wrote_pbft_head);
  EXPECT_FALSE(reward_reset_result.wrote_period_data);

  storage->commit_write_batch(reward_reset_batch_id, false);
  const auto reward_votes = storage->get_all_two_t_plus_one_votes();
  ASSERT_EQ(reward_votes.size(), 2);
  EXPECT_EQ(std::vector<uint8_t>(reward_votes[0].data.begin(), reward_votes[0].data.end()),
            (std::vector<uint8_t>{0x01}));
  EXPECT_EQ(std::vector<uint8_t>(reward_votes[1].data.begin(), reward_votes[1].data.end()),
            (std::vector<uint8_t>{0x02}));

  auto reward_reset_retry_batch_id = storage->create_write_batch();
  const auto reward_reset_retry_result =
      append_pbft_finalization_storage_write(*storage, reward_reset_retry_batch_id, plan.storage_write_intent,
                                             rewardResetFinalizationStorageStage(bytes({0xc2, 0x01, 0x02}), {}));

  EXPECT_EQ(reward_reset_retry_result.status, kPbftFinalizedPeriodApplyStatusAlreadyApplied);
  storage->drop_write_batch(reward_reset_retry_batch_id);

  auto dynamic_lambda_batch_id = storage->create_write_batch();
  auto dynamic_lambda_stage = finalizationStorageStage(kPbftFinalizationStorageStageDynamicLambda);
  dynamic_lambda_stage.rounds_count_dynamic_lambda = 7;
  dynamic_lambda_stage.dynamic_lambda = 1450;
  const auto dynamic_lambda_result = append_pbft_finalization_storage_write(
      *storage, dynamic_lambda_batch_id, plan.storage_write_intent, dynamic_lambda_stage);

  EXPECT_EQ(dynamic_lambda_result.status, kPbftFinalizedPeriodApplyStatusApplied);
  EXPECT_FALSE(dynamic_lambda_result.wrote_pbft_head);
  EXPECT_FALSE(dynamic_lambda_result.wrote_period_data);

  storage->commit_write_batch(dynamic_lambda_batch_id, false);

  const auto persisted_period_lambda = storage->get_period_lambda(101, false);
  EXPECT_TRUE(persisted_period_lambda.found);
  EXPECT_EQ(persisted_period_lambda.value, 1500);
  EXPECT_EQ(storage->get_rounds_count_dynamic_lambda(), 7);
  EXPECT_EQ(storage->get_pbft_mgr_field(kPbftMgrFieldLambda), 1450);

  auto executed_status_batch_id = storage->create_write_batch();
  const auto executed_status_result =
      append_pbft_finalization_storage_write(*storage, executed_status_batch_id, plan.storage_write_intent,
                                             finalizationStorageStage(kPbftFinalizationStorageStageExecutedStatus));

  EXPECT_EQ(executed_status_result.status, kPbftFinalizedPeriodApplyStatusApplied);

  storage->commit_write_batch(executed_status_batch_id, false);
  EXPECT_EQ(storage->get_pbft_mgr_status(kPbftMgrStatusExecutedBlock), plan.storage_write_intent.executed_pbft_status);

  std::filesystem::remove_all(test_dir);
}

TEST(RustPbftSyncTest, FinalizedPeriodStorageApplyCommitsOwnedBatch) {
  constexpr uint8_t kDagBlocksColumn = 4;
  constexpr uint8_t kTransactionsColumn = 6;
  const auto test_dir = uniqueTempDir("rustaxa_pbft_finalized_period_owned_apply");

  auto storage = create_storage(test_dir.string());
  auto seed_batch = storage->create_write_batch();
  storage->batch_put(seed_batch, kDagBlocksColumn, hashBytes(h256(2)), bytes({0xda}));
  storage->batch_put(seed_batch, kTransactionsColumn, hashBytes(h256(4)), bytes({0xd0}));
  storage->commit_write_batch(seed_batch, false);
  storage->save_extra_reward_vote(h256(12), bytes({0xee}));

  const auto plan = plan_pbft_finalization_intent(makeFinalizationFact());
  rust::Vec<PbftFinalizationStorageWriteStage> stages;
  stages.push_back(finalizationStorageStage(kPbftFinalizationStorageStagePrimary));
  stages.push_back(rewardResetFinalizationStorageStage(bytes({0xc2, 0x01, 0x02}), {h256(12)}));
  stages.push_back(sortitionFinalizationStorageStage(101, 2500, 1300));

  const auto result =
      apply_pbft_finalization_storage_writes(*storage, plan.storage_write_intent, std::move(stages), false);

  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusApplied);
  EXPECT_TRUE(result.wrote_pbft_head);
  EXPECT_TRUE(result.wrote_period_data);
  EXPECT_EQ(result.dag_index_writes, 2);
  EXPECT_EQ(result.transaction_location_writes, 1);
  const auto period_data = storage->get_period_data_raw(101);
  EXPECT_EQ(std::vector<uint8_t>(period_data.begin(), period_data.end()), (std::vector<uint8_t>{0xc0}));
  EXPECT_TRUE(storage->get_transaction(h256(4)).empty());
  const auto reward_votes = storage->get_all_two_t_plus_one_votes();
  ASSERT_EQ(reward_votes.size(), 2);
  EXPECT_EQ(std::vector<uint8_t>(reward_votes[0].data.begin(), reward_votes[0].data.end()),
            (std::vector<uint8_t>{0x01}));
  EXPECT_FALSE(storage->get_params_change_for_period(101).empty());

  std::filesystem::remove_all(test_dir);
}

TEST(RustPbftSyncTest, FinalizedPeriodStorageAppenderRejectsRewardResetMissingFacts) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_reward_reset_stage_rejected");
  auto storage = create_storage(test_dir.string());
  auto plan = plan_pbft_finalization_intent(makeFinalizationFact());

  auto batch_id = storage->create_write_batch();
  auto stage = finalizationStorageStage(kPbftFinalizationStorageStageRewardReset);
  const auto result = append_pbft_finalization_storage_write(*storage, batch_id, plan.storage_write_intent, stage);

  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusRejected);
  EXPECT_EQ(std::string(result.error_code), "PBFT_FINALIZE_MISSING_REWARD_VOTES_RESET_FACTS");
  storage->drop_write_batch(batch_id);

  std::filesystem::remove_all(test_dir);
}

TEST(RustPbftSyncTest, FinalizedPeriodStorageAppenderRejectsMissingPayload) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_finalized_period_missing_payload");
  auto storage = create_storage(test_dir.string());
  auto plan = plan_pbft_finalization_intent(makeFinalizationFact());
  plan.storage_write_intent.pbft_head_payload.clear();

  auto batch_id = storage->create_write_batch();
  const auto result = append_pbft_finalization_storage_write(
      *storage, batch_id, plan.storage_write_intent, finalizationStorageStage(kPbftFinalizationStorageStagePrimary));

  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusMissingPayload);
  EXPECT_FALSE(result.wrote_pbft_head);
  EXPECT_FALSE(result.wrote_period_data);
  storage->drop_write_batch(batch_id);

  std::filesystem::remove_all(test_dir);
}
