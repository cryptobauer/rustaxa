#include <gtest/gtest.h>

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

#include "common/encoding_rlp.hpp"
#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

namespace {

std::filesystem::path uniqueTempDir(const std::string& name) {
  const auto nonce = std::chrono::steady_clock::now().time_since_epoch().count();
  auto path = std::filesystem::temp_directory_path() / (name + "_" + std::to_string(nonce));
  std::filesystem::create_directories(path);
  return path;
}

std::array<uint8_t, 20> addr(uint8_t value) {
  std::array<uint8_t, 20> out{};
  out.fill(value);
  return out;
}

std::array<uint8_t, 32> hash(uint8_t value) {
  std::array<uint8_t, 32> out{};
  out[31] = value;
  return out;
}

rust::Vec<uint8_t> bytes(std::initializer_list<uint8_t> values) {
  rust::Vec<uint8_t> out;
  out.reserve(values.size());
  for (auto value : values) {
    out.push_back(value);
  }
  return out;
}

RewardsStatsConfig rewardsConfig() {
  RewardsStatsConfig config{};
  config.committee_size = 100;
  config.magnolia_period = 1;
  config.aspen_part_one_period = 10;
  return config;
}

RewardsStatsProcessFact rewardsFact(uint64_t period) {
  RewardsStatsProcessFact fact{};
  fact.period = period;
  fact.block_author = addr(1);
  fact.blocks_per_year = 1234;
  fact.dpos_eligible_total_vote_count = 80;

  RewardsTransactionFact tx{};
  tx.hash = hash(7);
  tx.gas_price_be = bytes({2});
  tx.gas_used = 5;
  fact.transactions.push_back(std::move(tx));

  RewardsDagBlockFact dag_block{};
  dag_block.author = addr(2);
  dag_block.difficulty = 3;
  dag_block.transaction_hashes.push_back(RewardsHash{hash(7)});
  fact.dag_blocks.push_back(std::move(dag_block));

  RewardsCertVoteFact vote{};
  vote.voter = addr(3);
  vote.weight = 11;
  vote.period = period - 1;
  fact.cert_votes.push_back(vote);

  return fact;
}

dev::bytes toDevBytes(const rust::Vec<uint8_t>& input) {
  return dev::bytes(input.begin(), input.end());
}

}  // namespace

TEST(RustRewardsStatsBridgeTest, producedRlpMatchesLegacyBlockStatsShape) {
  const auto test_dir = uniqueTempDir("rustaxa_rewards_stats_rlp");
  auto storage = create_storage(test_dir.string());
  rust::Vec<RewardsFrequencyRule> frequency_rules;
  auto runtime = create_rewards_stats_runtime(*storage, rewardsConfig(), std::move(frequency_rules), 0);

  const auto result = runtime->process_finalized_period_rewards_stats(rewardsFact(1));

  ASSERT_EQ(result.status, 0);
  ASSERT_EQ(result.distribution_stats.size(), 1);
  const auto bytes = toDevBytes(result.current_block_stats_rlp);
  const dev::RLP stats(bytes);

  ASSERT_EQ(stats.itemCount(), 6);
  EXPECT_EQ(stats[1].toInt<uint32_t>(), 1234);
  const auto validators = stats[2];
  ASSERT_EQ(validators.itemCount(), 2);
  EXPECT_EQ(stats[3].toInt<uint32_t>(), 1);
  EXPECT_EQ(stats[4].toInt<uint64_t>(), 11);
  EXPECT_EQ(stats[5].toInt<uint64_t>(), 80);

  bool saw_dag_author = false;
  bool saw_vote_author = false;
  for (const auto& validator : validators) {
    ASSERT_EQ(validator.itemCount(), 2);
    const auto validator_stats = validator[1];
    ASSERT_EQ(validator_stats.itemCount(), 3);
    if (validator_stats[0].toInt<uint32_t>() == 1) {
      saw_dag_author = true;
      EXPECT_EQ(validator_stats[2].toInt<uint64_t>(), 10);
    }
    if (validator_stats[1].toInt<uint64_t>() == 11) {
      saw_vote_author = true;
    }
  }
  EXPECT_TRUE(saw_dag_author);
  EXPECT_TRUE(saw_vote_author);
}

TEST(RustRewardsStatsBridgeTest, appendsCacheWritesAndBoundaryClearToStorageBatch) {
  const auto test_dir = uniqueTempDir("rustaxa_rewards_stats_storage");
  auto storage = create_storage(test_dir.string());

  rust::Vec<RewardsFrequencyRule> frequency_rules;
  RewardsFrequencyRule rule{};
  rule.from_period = 0;
  rule.frequency = 3;
  frequency_rules.push_back(rule);
  auto runtime = create_rewards_stats_runtime(*storage, rewardsConfig(), std::move(frequency_rules), 0);

  auto cache_plan = runtime->process_finalized_period_rewards_stats(rewardsFact(1));
  ASSERT_TRUE(cache_plan.cache_current_period);
  auto batch_id = storage->create_write_batch();
  auto apply_result = append_rewards_stats_storage_writes(*storage, batch_id, cache_plan);
  ASSERT_EQ(apply_result.status, 0);
  EXPECT_TRUE(apply_result.wrote_current_period);
  storage->commit_write_batch(batch_id, false);
  ASSERT_EQ(storage->get_blocks_rewards_stats().size(), 1);

  auto boundary_plan = runtime->process_finalized_period_rewards_stats(rewardsFact(3));
  ASSERT_TRUE(boundary_plan.clear_cached_stats);
  ASSERT_EQ(boundary_plan.distribution_stats.size(), 2);
  batch_id = storage->create_write_batch();
  apply_result = append_rewards_stats_storage_writes(*storage, batch_id, boundary_plan);
  ASSERT_EQ(apply_result.status, 0);
  EXPECT_TRUE(apply_result.cleared_cached_stats);
  storage->commit_write_batch(batch_id, false);
  runtime->rewards_stats_runtime_clear_committed(3);

  EXPECT_TRUE(storage->get_blocks_rewards_stats().empty());
}
