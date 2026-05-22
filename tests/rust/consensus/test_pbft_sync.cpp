#include <gtest/gtest.h>

#include <array>
#include <cstdint>
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
  fact.block_period = 101;
  fact.block_prev_hash = h256(1);
  fact.chain_last_hash = h256(1);
  fact.chain_last_period = 100;
  fact.block_in_chain = false;
  fact.pivot_dag_anchor_hash = h256(8);
  fact.has_pillar_block = false;
  fact.pillar_block_finalized = false;
  fact.request_dynamic_lambda_update = true;
  return fact;
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

  fact = makeFinalizationFact();
  fact.block_in_chain = true;
  plan = plan_pbft_finalization_intent(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusBlockAlreadyInChain);
  EXPECT_FALSE(plan.cleanup.advance_period);

  fact = makeFinalizationFact();
  fact.has_pillar_block = true;
  fact.pillar_block_finalized = false;
  plan = plan_pbft_finalization_intent(std::move(fact));

  EXPECT_FALSE(plan.finalize_block);
  EXPECT_EQ(plan.status, kPbftFinalizationStatusPillarDependencyMissing);
  EXPECT_FALSE(plan.cleanup.finalize_final_chain);
}
