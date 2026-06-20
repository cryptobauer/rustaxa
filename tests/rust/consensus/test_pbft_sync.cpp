#include <gtest/gtest.h>

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <initializer_list>
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

rust::Box<BridgePbftVoteStorageQueries> voteQueries(const rust::Box<BridgeStorage>& storage) {
  return create_pbft_vote_storage_queries(*storage);
}

rust::Box<BridgePbftStorageQueries> pbftQueries(const rust::Box<BridgeStorage>& storage) {
  return create_pbft_storage_queries(*storage);
}

rust::Box<BridgeMetadataStorageQueries> metadataQueries(const rust::Box<BridgeStorage>& storage) {
  return create_metadata_storage_queries(*storage);
}

rust::Box<BridgeDagStorageQueries> dagQueries(const rust::Box<BridgeStorage>& storage) {
  return create_dag_storage_queries(*storage);
}

rust::Box<BridgeTransactionStorageQueries> transactionQueries(const rust::Box<BridgeStorage>& storage) {
  return create_transaction_storage_queries(*storage);
}

rust::Box<BridgePeriodStorageQueries> periodQueries(const rust::Box<BridgeStorage>& storage) {
  return create_period_storage_queries(*storage);
}

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
constexpr uint8_t kPbftSyncNextCheckValidateCertVotes = 3;
constexpr uint8_t kPbftSyncNextCheckCheckTransactions = 4;
constexpr uint8_t kPbftSyncNextCheckValidatePillarData = 5;
constexpr uint8_t kPbftSyncNextCheckValidatePillarVotes = 6;
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
constexpr uint8_t kPbftFinalizationRuntimeActionCommitSortitionRuntime = 14;
constexpr uint8_t kPbftFinalizationRuntimeStatusActive = 0;
constexpr uint8_t kPbftFinalizationRuntimeStatusComplete = 1;
constexpr uint8_t kPbftFinalizationRuntimeStatusActionMismatch = 3;
constexpr uint8_t kPbftFinalizationRuntimeStatusActionFailed = 4;
constexpr uint8_t kPbftFinalizationLiveMutationStatusAccepted = 0;
constexpr uint8_t kPbftFinalizationLiveMutationStatusDagCountMismatch = 7;
constexpr uint8_t kPbftFinalizationResumeStatusNotPersisted = 0;
constexpr uint8_t kPbftFinalizationResumeStatusComplete = 1;
constexpr uint8_t kPbftFinalizationResumeStatusNeedsFinalChainReplay = 2;
constexpr uint8_t kPbftFinalizationResumeStatusNeedsDynamicLambda = 6;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusApplied = 0;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusAlreadyApplied = 1;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusRejected = 2;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusMissingPayload = 3;
constexpr uint8_t kPbftMgrFieldRound = 0;
constexpr uint8_t kPbftMgrFieldStep = 1;
constexpr uint8_t kPbftMgrFieldLambda = 2;
constexpr uint8_t kPbftMgrStatusExecutedBlock = 0;
constexpr uint8_t kPbftMgrStatusNextVotedValue = 2;
constexpr uint8_t kPbftFinalizationStorageStagePrimary = 0;
constexpr uint8_t kPbftFinalizationStorageStageDynamicLambda = 1;
constexpr uint8_t kPbftFinalizationStorageStageExecutedStatus = 2;
constexpr uint8_t kPbftFinalizationStorageStageSortition = 3;
constexpr uint8_t kPbftFinalizationStorageStageRewardReset = 4;
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
constexpr uint8_t kPbftManagerStateActionStatusReady = 0;
constexpr uint8_t kPbftManagerStateActionNextVoteNullBlock = 8;
constexpr uint8_t kPbftManagerStateActionNextVoteCurrentSoftValue = 10;
constexpr uint8_t kPbftManagerStateActionSessionActive = 0;
constexpr uint8_t kPbftManagerStateActionSessionComplete = 1;
constexpr uint8_t kPbftManagerStateActionEffectApplied = 0;
constexpr uint8_t kPbftManagerTransitionStatusReady = 0;
constexpr uint8_t kPbftManagerTransitionKindResetConsensus = 0;
constexpr uint8_t kPbftManagerRuntimeStateValueProposalCode = 0;
constexpr uint8_t kPbftManagerAdvanceActionResetConsensus = 0;
constexpr uint8_t kPbftManagerAdvanceActionExecutedBlockReset = 1;
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

PbftFinalizationStorageWriteStage sortitionFinalizationStorageStage(uint64_t period, uint16_t interval_efficiency,
                                                                    uint16_t threshold_upper) {
  auto stage = finalizationStorageStage(kPbftFinalizationStorageStageSortition);
  stage.has_sortition_params_change = true;
  stage.sortition_params_change_period = period;
  stage.sortition_params_change_interval_efficiency = interval_efficiency;
  stage.sortition_params_change_threshold_upper = threshold_upper;
  return stage;
}

rust::Vec<PbftFinalizationStorageWriteStage> storageStages(
    std::initializer_list<PbftFinalizationStorageWriteStage> stages) {
  rust::Vec<PbftFinalizationStorageWriteStage> out;
  for (auto stage : stages) {
    out.push_back(std::move(stage));
  }
  return out;
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

PbftManagerStartupFact makePbftManagerStartupFact() {
  PbftManagerStartupFact fact;
  fact.current_period = 10;
  fact.cacti_active_at_chain_size = true;
  fact.genesis_lambda_ms = 100;
  fact.cacti_lambda_max_ms = 1'500;
  fact.cacti_lambda_default_ms = 500;
  return fact;
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

PbftManagerTransitionPlan resetTransitionPlanForAdvancePeriod() {
  PbftManagerTransitionPlan plan;
  plan.status = kPbftManagerTransitionStatusReady;
  plan.kind = kPbftManagerTransitionKindResetConsensus;
  plan.new_state = kPbftManagerRuntimeStateValueProposalCode;
  plan.new_round = 1;
  plan.new_step = 1;
  plan.current_round_lambda_ms = 100;
  plan.next_step_time_ms = 1'000;
  plan.persist_round = true;
  plan.persist_step = true;
  plan.reset_next_voted_statuses = true;
  plan.remove_cert_voted_block = true;
  plan.clear_own_votes = true;
  plan.clear_broadcasted_votes = true;
  plan.reset_broadcast_counters = true;
  plan.reset_executed_block_status = true;
  plan.set_vote_manager_period_round = true;
  plan.reset_current_round_start = true;
  plan.reset_second_finish_start = false;
  plan.print_cert_step_info = false;
  plan.print_second_finish_step_info = false;
  return plan;
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

TEST(RustPbftSyncTest, ProcessPeriodRuntimeRecordsAcceptTranscript) {
  auto fact = makeRuntimeFact();
  std::vector<uint8_t> checks;

  auto plan = plan_pbft_sync_process_period_data_runtime(fact);
  ASSERT_EQ(plan.runtime_action, kPbftSyncRuntimeActionRunCheck);
  checks.push_back(plan.next_check);

  fact.final_chain_hash_status = kPbftSyncFinalChainHashValid;
  plan = plan_pbft_sync_process_period_data_runtime(fact);
  ASSERT_EQ(plan.runtime_action, kPbftSyncRuntimeActionRunCheck);
  checks.push_back(plan.next_check);

  fact.reward_votes_status = kPbftSyncFactValid;
  plan = plan_pbft_sync_process_period_data_runtime(fact);
  ASSERT_EQ(plan.runtime_action, kPbftSyncRuntimeActionRunCheck);
  checks.push_back(plan.next_check);

  fact.cert_votes_status = kPbftSyncFactValid;
  plan = plan_pbft_sync_process_period_data_runtime(fact);
  ASSERT_EQ(plan.runtime_action, kPbftSyncRuntimeActionRunCheck);
  checks.push_back(plan.next_check);
  EXPECT_EQ(hashes(plan.transaction_query_plan.finalized_lookup_hashes), (std::vector{h256(1)}));

  fact.transactions_status = kPbftSyncFactValid;
  plan = plan_pbft_sync_process_period_data_runtime(fact);
  ASSERT_EQ(plan.runtime_action, kPbftSyncRuntimeActionRunCheck);
  checks.push_back(plan.next_check);

  fact.pillar_data_status = kPbftSyncFactValid;
  plan = plan_pbft_sync_process_period_data_runtime(fact);
  ASSERT_EQ(plan.runtime_action, kPbftSyncRuntimeActionRunCheck);
  checks.push_back(plan.next_check);

  fact.pillar_votes_status = kPbftSyncFactValid;
  plan = plan_pbft_sync_process_period_data_runtime(fact);

  EXPECT_EQ(checks,
            (std::vector<uint8_t>{kPbftSyncNextCheckValidateFinalChainHash, kPbftSyncNextCheckCheckRewardVotes,
                                  kPbftSyncNextCheckValidateCertVotes, kPbftSyncNextCheckCheckTransactions,
                                  kPbftSyncNextCheckValidatePillarData, kPbftSyncNextCheckValidatePillarVotes}));
  EXPECT_EQ(plan.runtime_action, kPbftSyncRuntimeActionAccept);
  EXPECT_EQ(plan.next_check, kPbftSyncNextCheckNone);
  EXPECT_TRUE(plan.accept_period_data);
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

TEST(RustPbftSyncTest, ManagerRuntimeOrdersOneValueProposalTick) {
  auto session = create_pbft_manager_runtime_session(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));
  std::vector<uint8_t> actions;

  while (true) {
    auto step = session->pbft_manager_runtime_session_next();
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
    step = session->pbft_manager_runtime_session_report(managerRuntimeReport(step.cursor, step.action, result));
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
  auto session = create_pbft_manager_runtime_session(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));

  auto step = session->pbft_manager_runtime_session_next();
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionProcessSyncedBlocks);
  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionMaybeBroadcastVotes);
  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionTryPushCertVotesBlock);

  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultProgressRestart));

  EXPECT_EQ(step.status, kPbftManagerRuntimeStatusComplete);
  EXPECT_TRUE(step.complete);
  EXPECT_TRUE(step.restart_loop);
}

TEST(RustPbftSyncTest, ManagerRuntimeAdvanceRoundCandidateRequestsResetEffect) {
  auto session = create_pbft_manager_runtime_session(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));

  auto step = session->pbft_manager_runtime_session_next();
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionProcessSyncedBlocks);
  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionMaybeBroadcastVotes);
  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionTryPushCertVotesBlock);
  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultNoProgress));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionTryAdvanceRound);

  auto report = managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultNoProgress);
  report.has_new_round = true;
  report.new_round = 5;
  step = session->pbft_manager_runtime_session_report(std::move(report));

  ASSERT_EQ(step.status, kPbftManagerRuntimeStatusActive);
  ASSERT_TRUE(step.has_action);
  EXPECT_EQ(step.action, kPbftManagerRuntimeActionResetConsensus);
  EXPECT_TRUE(step.has_target_round);
  EXPECT_EQ(step.target_round, 5);

  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultTransition));

  EXPECT_EQ(step.status, kPbftManagerRuntimeStatusComplete);
  EXPECT_TRUE(step.complete);
  EXPECT_TRUE(step.restart_loop);
}

TEST(RustPbftSyncTest, ManagerRuntimeRejectsNonIncreasingAdvanceRoundCandidate) {
  auto session = create_pbft_manager_runtime_session(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));

  auto step = session->pbft_manager_runtime_session_next();
  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone));
  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultNoProgress));
  ASSERT_EQ(step.action, kPbftManagerRuntimeActionTryAdvanceRound);

  auto report = managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultNoProgress);
  report.has_new_round = true;
  report.new_round = 2;
  step = session->pbft_manager_runtime_session_report(std::move(report));

  EXPECT_EQ(step.status, kPbftManagerRuntimeStatusInvalidReport);
  EXPECT_FALSE(step.can_continue);
  EXPECT_FALSE(step.complete);
}

TEST(RustPbftSyncTest, ManagerRuntimeCertifyReportSelectsFinishTransition) {
  auto session = create_pbft_manager_runtime_session(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateCertify));

  while (true) {
    auto step = session->pbft_manager_runtime_session_next();
    ASSERT_TRUE(step.has_action);
    if (step.action == kPbftManagerRuntimeActionRunCertify) {
      auto report = managerRuntimeReport(step.cursor, step.action, kPbftManagerRuntimeResultStateDone);
      report.go_finish_state = true;
      step = session->pbft_manager_runtime_session_report(std::move(report));
      EXPECT_EQ(step.status, kPbftManagerRuntimeStatusActive);
      EXPECT_EQ(step.action, kPbftManagerRuntimeActionTransitionToFinish);
      break;
    }

    uint8_t result = kPbftManagerRuntimeResultStateDone;
    if (step.action == kPbftManagerRuntimeActionTryPushCertVotesBlock ||
        step.action == kPbftManagerRuntimeActionTryAdvanceRound) {
      result = kPbftManagerRuntimeResultNoProgress;
    }
    session->pbft_manager_runtime_session_report(managerRuntimeReport(step.cursor, step.action, result));
  }
}

TEST(RustPbftSyncTest, ManagerRuntimeRejectsCursorMismatch) {
  auto session = create_pbft_manager_runtime_session(makePbftManagerRuntimeTick(kPbftManagerRuntimeStateValueProposal));
  auto step = session->pbft_manager_runtime_session_next();

  step = session->pbft_manager_runtime_session_report(
      managerRuntimeReport(step.cursor + 1, step.action, kPbftManagerRuntimeResultStateDone));

  EXPECT_EQ(step.status, kPbftManagerRuntimeStatusActionMismatch);
  EXPECT_FALSE(step.can_continue);
  EXPECT_FALSE(step.complete);
}

TEST(RustPbftSyncTest, ManagerStartupRestoreRecordsRuntimeSnapshotFromStorage) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_manager_startup_snapshot");
  auto storage = create_storage(test_dir.string());
  storage->save_pbft_mgr_field(kPbftMgrFieldRound, 2);
  storage->save_pbft_mgr_field(kPbftMgrFieldStep, 2);
  storage->save_pbft_mgr_field(kPbftMgrFieldLambda, 1'500);
  storage->save_pbft_mgr_status(kPbftMgrStatusExecutedBlock, true);
  storage->save_pbft_mgr_status(kPbftMgrStatusNextVotedValue, true);

  const auto runtime = create_pbft_manager_runtime_from_storage(*storage, makePbftManagerStartupFact());
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
  auto fact = makePbftManagerStateActionFact(4);
  fact.has_current_round_soft_value = true;
  fact.has_previous_round_next_null = true;

  const auto plan = plan_pbft_manager_state_action_effects(fact);
  ASSERT_EQ(plan.status, kPbftManagerStateActionStatusReady);
  ASSERT_EQ(plan.effects.size(), 2);

  auto session = create_pbft_manager_state_action_effect_session(fact);
  std::vector<uint8_t> intents;

  auto step = session->pbft_manager_state_action_effect_session_next();
  ASSERT_EQ(step.status, kPbftManagerStateActionSessionActive);
  ASSERT_TRUE(step.has_effect);
  intents.push_back(step.effect.intent);
  EXPECT_EQ(step.effect.hash, h256(0x55));

  step = session->pbft_manager_state_action_effect_session_report(stateActionReport(step.cursor, step.effect.intent));
  ASSERT_EQ(step.status, kPbftManagerStateActionSessionActive);
  ASSERT_TRUE(step.has_effect);
  intents.push_back(step.effect.intent);
  EXPECT_EQ(step.effect.hash, h256(0));

  step = session->pbft_manager_state_action_effect_session_report(stateActionReport(step.cursor, step.effect.intent));
  EXPECT_EQ(step.status, kPbftManagerStateActionSessionComplete);
  EXPECT_TRUE(step.complete);
  EXPECT_TRUE(step.can_continue);
  EXPECT_FALSE(step.has_effect);

  EXPECT_EQ(intents, (std::vector<uint8_t>{kPbftManagerStateActionNextVoteCurrentSoftValue,
                                           kPbftManagerStateActionNextVoteNullBlock}));
}

TEST(RustPbftSyncTest, ManagerAdvancePeriodRecordsEffectTranscript) {
  const auto transition = resetTransitionPlanForAdvancePeriod();
  const auto plan = plan_pbft_manager_advance_period(12, transition);

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
                kPbftManagerAdvanceActionResetConsensus, kPbftManagerAdvanceActionExecutedBlockReset,
                kPbftManagerAdvanceActionSetVoteManagerPeriodRound, kPbftManagerAdvanceActionResetCurrentRoundTimer,
                kPbftManagerAdvanceActionResetRewardVoteCounters, kPbftManagerAdvanceActionResetPeriodTimer,
                kPbftManagerAdvanceActionUpdateWalletEligibility, kPbftManagerAdvanceActionCleanupVotes,
                kPbftManagerAdvanceActionCleanupProposedBlocks}));
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

TEST(RustPbftSyncTest, FinalizationLiveMutationReportsValidateAgainstPlan) {
  const auto plan = plan_pbft_finalization_intent(makeFinalizationFact());

  PbftFinalizationLiveMutationReport report{};
  report.action = kPbftFinalizationRuntimeActionSetDagOrder;
  report.block_period = 101;
  report.pbft_block_hash = h256(9);
  report.anchor_hash = h256(8);
  report.dag_finalized_count = 2;

  auto validation = validate_pbft_finalization_live_mutation_report(plan, report);
  EXPECT_TRUE(validation.accepted);
  EXPECT_EQ(validation.status, kPbftFinalizationLiveMutationStatusAccepted);
  EXPECT_EQ(validation.action, kPbftFinalizationRuntimeActionSetDagOrder);

  report.dag_finalized_count = 1;
  validation = validate_pbft_finalization_live_mutation_report(plan, report);
  EXPECT_FALSE(validation.accepted);
  EXPECT_EQ(validation.status, kPbftFinalizationLiveMutationStatusDagCountMismatch);
  EXPECT_EQ(std::string(validation.error_code), "PBFT_FINALIZE_LIVE_MUTATION_DAG_COUNT_MISMATCH");
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
                         kPbftFinalizationRuntimeActionCommitSortitionRuntime,
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

TEST(RustPbftSyncTest, FinalizationRuntimeSessionOwnsCursorAndCompletion) {
  const auto intent = plan_pbft_finalization_intent(makeFinalizationFact());
  auto session = create_pbft_finalization_runtime_session(intent);

  auto step = session->pbft_finalization_runtime_session_next();
  EXPECT_EQ(step.status, kPbftFinalizationRuntimeStatusActive);
  EXPECT_TRUE(step.has_action);
  EXPECT_EQ(step.cursor, 0);
  EXPECT_EQ(step.action, kPbftFinalizationRuntimeActionPrimaryStorage);
  EXPECT_FALSE(step.complete);

  std::vector<uint8_t> actions;
  while (step.has_action) {
    actions.push_back(step.action);
    step = session->pbft_finalization_runtime_session_report(step.cursor, step.action, true, 0);
  }

  EXPECT_TRUE(step.complete);
  EXPECT_EQ(step.status, kPbftFinalizationRuntimeStatusComplete);
  EXPECT_EQ(actions, (std::vector<uint8_t>{
                         kPbftFinalizationRuntimeActionPrimaryStorage,
                         kPbftFinalizationRuntimeActionCommitSortitionRuntime,
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
}

TEST(RustPbftSyncTest, FinalizationRuntimeSessionStopsOnFailureOrMismatch) {
  const auto intent = plan_pbft_finalization_intent(makeFinalizationFact());
  auto session = create_pbft_finalization_runtime_session(intent);

  auto failed =
      session->pbft_finalization_runtime_session_report(0, kPbftFinalizationRuntimeActionPrimaryStorage, false, 77);
  EXPECT_EQ(failed.status, kPbftFinalizationRuntimeStatusActionFailed);
  EXPECT_FALSE(failed.has_action);
  EXPECT_EQ(failed.cursor, 0);
  EXPECT_EQ(std::string(failed.error_code), "PBFT_FINALIZE_RUNTIME_ACTION_STATUS_77");

  session = create_pbft_finalization_runtime_session(intent);
  auto mismatch =
      session->pbft_finalization_runtime_session_report(1, kPbftFinalizationRuntimeActionPrimaryStorage, true, 0);
  EXPECT_EQ(mismatch.status, kPbftFinalizationRuntimeStatusActionMismatch);
  EXPECT_FALSE(mismatch.has_action);
  EXPECT_EQ(std::string(mismatch.error_code), "PBFT_FINALIZE_RUNTIME_CURSOR_MISMATCH");
}

TEST(RustPbftSyncTest, FinalizationResumeRuntimeSessionOwnsTailReplayCursor) {
  PbftFinalizationResumePlan resume;
  resume.status = kPbftFinalizationResumeStatusNeedsFinalChainReplay;
  resume.duplicate_classified = true;
  resume.complete = false;
  resume.replay_actions.push_back(kPbftFinalizationRuntimeActionFinalizeFinalChain);
  resume.replay_actions.push_back(kPbftFinalizationRuntimeActionPersistExecutedStatus);
  resume.replay_actions.push_back(kPbftFinalizationRuntimeActionSetExecutedFlag);
  resume.replay_actions.push_back(kPbftFinalizationRuntimeActionAdvancePeriod);
  resume.error_code = "PBFT_FINALIZE_RESUME_NEEDS_FINAL_CHAIN_REPLAY";
  auto session = create_pbft_finalization_resume_runtime_session(resume);

  auto step = session->pbft_finalization_runtime_session_next();
  std::vector<uint8_t> actions;
  while (step.has_action) {
    actions.push_back(step.action);
    step = session->pbft_finalization_runtime_session_report(step.cursor, step.action, true, 0);
  }

  EXPECT_TRUE(step.complete);
  EXPECT_EQ(step.status, kPbftFinalizationRuntimeStatusComplete);
  EXPECT_EQ(actions, (std::vector<uint8_t>{
                         kPbftFinalizationRuntimeActionFinalizeFinalChain,
                         kPbftFinalizationRuntimeActionPersistExecutedStatus,
                         kPbftFinalizationRuntimeActionSetExecutedFlag,
                         kPbftFinalizationRuntimeActionAdvancePeriod,
                     }));
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

TEST(RustPbftSyncTest, FinalizedPeriodStorageApplyWritesPrimaryBatch) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_finalized_period_apply");

  auto storage = create_storage(test_dir.string());
  storage->save_dag_block(h256(2), 1, 0, bytes({0xda}));
  storage->save_transaction(h256(4), bytes({0xd0}));
  auto period_queries = periodQueries(storage);

  const auto plan = plan_pbft_finalization_intent(makeFinalizationFact());
  const auto result = apply_pbft_finalization_storage_writes(
      *storage, plan.storage_write_intent,
      storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}), false);

  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusApplied);
  EXPECT_TRUE(result.wrote_pbft_head);
  EXPECT_TRUE(result.wrote_period_data);
  EXPECT_EQ(result.dag_index_writes, 2);
  EXPECT_EQ(result.transaction_location_writes, 1);

  auto pbft_queries = pbftQueries(storage);
  const auto pbft_head = pbft_queries->get_pbft_head(h256(8));
  EXPECT_EQ(std::vector<uint8_t>(pbft_head.begin(), pbft_head.end()),
            (std::vector<uint8_t>{'{', '"', 'h', 'e', 'a', 'd', '"', ':', 't', 'r', 'u', 'e', '}'}));
  const auto period_data = period_queries->get_period_data_raw(101);
  EXPECT_EQ(std::vector<uint8_t>(period_data.begin(), period_data.end()), (std::vector<uint8_t>{0xc0}));
  auto transaction_queries = transactionQueries(storage);
  EXPECT_TRUE(transaction_queries->get_transaction(h256(4)).empty());

  const auto dag_lookup = dagQueries(storage)->get_dag_block_period_lookup(h256(2));
  EXPECT_TRUE(dag_lookup.found);
  EXPECT_EQ(dag_lookup.period, 101);
  EXPECT_EQ(dag_lookup.position, 0);
  EXPECT_FALSE(transaction_queries->get_transaction_location(h256(4)).empty());
  auto metadata_queries = metadataQueries(storage);
  const auto period_lambda = metadata_queries->get_period_lambda(101, false);
  EXPECT_FALSE(period_lambda.found);
  EXPECT_FALSE(pbft_queries->get_pbft_mgr_status(kPbftMgrStatusExecutedBlock));

  const auto sortition_result = apply_pbft_finalization_storage_writes(
      *storage, plan.storage_write_intent, storageStages({sortitionFinalizationStorageStage(101, 2500, 1300)}), false);

  EXPECT_EQ(sortition_result.status, kPbftFinalizedPeriodApplyStatusApplied);
  EXPECT_FALSE(sortition_result.wrote_pbft_head);
  EXPECT_FALSE(sortition_result.wrote_period_data);

  EXPECT_FALSE(metadata_queries->get_params_change_for_period(101).empty());

  const auto reward_reset_result = apply_pbft_finalization_storage_writes(
      *storage, plan.storage_write_intent,
      storageStages({rewardResetFinalizationStorageStage(bytes({0xc2, 0x01, 0x02}), {})}), false);

  EXPECT_EQ(reward_reset_result.status, kPbftFinalizedPeriodApplyStatusApplied);
  EXPECT_FALSE(reward_reset_result.wrote_pbft_head);
  EXPECT_FALSE(reward_reset_result.wrote_period_data);

  const auto reward_votes = voteQueries(storage)->get_all_two_t_plus_one_votes();
  ASSERT_EQ(reward_votes.size(), 2);
  EXPECT_EQ(std::vector<uint8_t>(reward_votes[0].data.begin(), reward_votes[0].data.end()),
            (std::vector<uint8_t>{0x01}));
  EXPECT_EQ(std::vector<uint8_t>(reward_votes[1].data.begin(), reward_votes[1].data.end()),
            (std::vector<uint8_t>{0x02}));

  const auto reward_reset_retry_result = apply_pbft_finalization_storage_writes(
      *storage, plan.storage_write_intent,
      storageStages({rewardResetFinalizationStorageStage(bytes({0xc2, 0x01, 0x02}), {})}), false);

  EXPECT_EQ(reward_reset_retry_result.status, kPbftFinalizedPeriodApplyStatusAlreadyApplied);

  auto dynamic_lambda_stage = finalizationStorageStage(kPbftFinalizationStorageStageDynamicLambda);
  dynamic_lambda_stage.rounds_count_dynamic_lambda = 7;
  dynamic_lambda_stage.dynamic_lambda = 1450;
  const auto dynamic_lambda_result = apply_pbft_finalization_storage_writes(
      *storage, plan.storage_write_intent, storageStages({dynamic_lambda_stage}), false);

  EXPECT_EQ(dynamic_lambda_result.status, kPbftFinalizedPeriodApplyStatusApplied);
  EXPECT_FALSE(dynamic_lambda_result.wrote_pbft_head);
  EXPECT_FALSE(dynamic_lambda_result.wrote_period_data);

  const auto persisted_period_lambda = metadata_queries->get_period_lambda(101, false);
  EXPECT_TRUE(persisted_period_lambda.found);
  EXPECT_EQ(persisted_period_lambda.value, 1500);
  EXPECT_EQ(metadata_queries->get_rounds_count_dynamic_lambda(), 7);
  EXPECT_EQ(pbftQueries(storage)->get_pbft_mgr_field(kPbftMgrFieldLambda), 1450);

  const auto executed_status_result = apply_pbft_finalization_storage_writes(
      *storage, plan.storage_write_intent,
      storageStages({finalizationStorageStage(kPbftFinalizationStorageStageExecutedStatus)}), false);

  EXPECT_EQ(executed_status_result.status, kPbftFinalizedPeriodApplyStatusApplied);

  EXPECT_EQ(pbftQueries(storage)->get_pbft_mgr_status(kPbftMgrStatusExecutedBlock),
            plan.storage_write_intent.executed_pbft_status);

  std::filesystem::remove_all(test_dir);
}

TEST(RustPbftSyncTest, FinalizedPeriodStorageApplyCommitsOwnedBatch) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_finalized_period_owned_apply");

  auto storage = create_storage(test_dir.string());
  storage->save_dag_block(h256(2), 1, 0, bytes({0xda}));
  storage->save_transaction(h256(4), bytes({0xd0}));
  storage->save_extra_reward_vote(h256(12), bytes({0xee}));
  auto period_queries = periodQueries(storage);

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
  const auto period_data = period_queries->get_period_data_raw(101);
  EXPECT_EQ(std::vector<uint8_t>(period_data.begin(), period_data.end()), (std::vector<uint8_t>{0xc0}));
  EXPECT_TRUE(transactionQueries(storage)->get_transaction(h256(4)).empty());
  const auto reward_votes = voteQueries(storage)->get_all_two_t_plus_one_votes();
  ASSERT_EQ(reward_votes.size(), 2);
  EXPECT_EQ(std::vector<uint8_t>(reward_votes[0].data.begin(), reward_votes[0].data.end()),
            (std::vector<uint8_t>{0x01}));
  EXPECT_FALSE(metadataQueries(storage)->get_params_change_for_period(101).empty());

  std::filesystem::remove_all(test_dir);
}

TEST(RustPbftSyncTest, FinalizationResumeInspectorClassifiesCrashWindows) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_finalization_resume");
  auto storage = create_storage(test_dir.string());
  auto plan = plan_pbft_finalization_intent(makeFinalizationFact());

  auto resume = inspect_pbft_finalization_resume(*storage, plan.storage_write_intent, 100);
  EXPECT_EQ(resume.status, kPbftFinalizationResumeStatusNotPersisted);
  EXPECT_FALSE(resume.duplicate_classified);

  rust::Vec<PbftFinalizationStorageWriteStage> primary_stages;
  primary_stages.push_back(finalizationStorageStage(kPbftFinalizationStorageStagePrimary));
  auto result =
      apply_pbft_finalization_storage_writes(*storage, plan.storage_write_intent, std::move(primary_stages), false);
  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusApplied);

  resume = inspect_pbft_finalization_resume(*storage, plan.storage_write_intent, 100);
  EXPECT_EQ(resume.status, kPbftFinalizationResumeStatusNeedsDynamicLambda);
  EXPECT_EQ(resume.replay_actions.size(), 1);
  EXPECT_EQ(resume.replay_actions[0], kPbftFinalizationRuntimeActionApplyDynamicLambda);

  auto dynamic_lambda_stage = finalizationStorageStage(kPbftFinalizationStorageStageDynamicLambda);
  dynamic_lambda_stage.rounds_count_dynamic_lambda = 7;
  dynamic_lambda_stage.dynamic_lambda = 1450;
  rust::Vec<PbftFinalizationStorageWriteStage> dynamic_stages;
  dynamic_stages.push_back(std::move(dynamic_lambda_stage));
  result =
      apply_pbft_finalization_storage_writes(*storage, plan.storage_write_intent, std::move(dynamic_stages), false);
  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusApplied);

  resume = inspect_pbft_finalization_resume(*storage, plan.storage_write_intent, 100);
  EXPECT_EQ(resume.status, kPbftFinalizationResumeStatusNeedsFinalChainReplay);
  const std::vector<uint8_t> replay_actions(resume.replay_actions.begin(), resume.replay_actions.end());
  EXPECT_EQ(replay_actions, (std::vector<uint8_t>{
                                kPbftFinalizationRuntimeActionFinalizeFinalChain,
                                kPbftFinalizationRuntimeActionPersistExecutedStatus,
                                kPbftFinalizationRuntimeActionSetExecutedFlag,
                                kPbftFinalizationRuntimeActionAdvancePeriod,
                            }));

  rust::Vec<PbftFinalizationStorageWriteStage> executed_stages;
  executed_stages.push_back(finalizationStorageStage(kPbftFinalizationStorageStageExecutedStatus));
  result =
      apply_pbft_finalization_storage_writes(*storage, plan.storage_write_intent, std::move(executed_stages), false);
  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusApplied);

  resume = inspect_pbft_finalization_resume(*storage, plan.storage_write_intent, 101);
  EXPECT_EQ(resume.status, kPbftFinalizationResumeStatusComplete);
  EXPECT_TRUE(resume.duplicate_classified);
  EXPECT_TRUE(resume.complete);
  EXPECT_TRUE(resume.replay_actions.empty());

  std::filesystem::remove_all(test_dir);
}

TEST(RustPbftSyncTest, FinalizedPeriodStorageApplyRejectsRewardResetMissingFacts) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_reward_reset_stage_rejected");
  auto storage = create_storage(test_dir.string());
  auto plan = plan_pbft_finalization_intent(makeFinalizationFact());

  auto stage = finalizationStorageStage(kPbftFinalizationStorageStageRewardReset);
  const auto result =
      apply_pbft_finalization_storage_writes(*storage, plan.storage_write_intent, storageStages({stage}), false);

  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusRejected);
  EXPECT_EQ(std::string(result.error_code), "PBFT_FINALIZE_MISSING_REWARD_VOTES_RESET_FACTS");

  std::filesystem::remove_all(test_dir);
}

TEST(RustPbftSyncTest, FinalizedPeriodStorageApplyRejectsMissingPayload) {
  const auto test_dir = uniqueTempDir("rustaxa_pbft_finalized_period_missing_payload");
  auto storage = create_storage(test_dir.string());
  auto plan = plan_pbft_finalization_intent(makeFinalizationFact());
  plan.storage_write_intent.pbft_head_payload.clear();

  const auto result = apply_pbft_finalization_storage_writes(
      *storage, plan.storage_write_intent,
      storageStages({finalizationStorageStage(kPbftFinalizationStorageStagePrimary)}), false);

  EXPECT_EQ(result.status, kPbftFinalizedPeriodApplyStatusMissingPayload);
  EXPECT_FALSE(result.wrote_pbft_head);
  EXPECT_FALSE(result.wrote_period_data);

  std::filesystem::remove_all(test_dir);
}
