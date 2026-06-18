#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <exception>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

namespace {

std::array<uint8_t, 32> hashFor(uint8_t seed) {
  std::array<uint8_t, 32> hash{};
  hash.fill(seed);
  return hash;
}

}  // namespace

TEST(RustPeriodDataQueueTest, PushPopAndLastEntryFollowLegacyRules) {
  auto queue = create_period_data_queue();

  auto first = queue->period_data_queue_push(11, 1, hashFor(0x11), 0, 1);
  ASSERT_TRUE(first.accepted);
  EXPECT_FALSE(first.clear_existing);
  EXPECT_EQ(queue->period_data_queue_period(), 1u);
  EXPECT_EQ(queue->period_data_queue_size(), 1u);

  auto second = queue->period_data_queue_push(22, 2, hashFor(0x22), 0, 1);
  ASSERT_TRUE(second.accepted);
  EXPECT_EQ(queue->period_data_queue_size(), 2u);

  auto last = queue->period_data_queue_last_entry();
  ASSERT_TRUE(last.found);
  EXPECT_EQ(last.entry_id, 22u);
  EXPECT_EQ(last.period, 2u);
  EXPECT_EQ(last.block_hash, hashFor(0x22));

  auto pop_first = queue->period_data_queue_pop();
  EXPECT_EQ(pop_first.entry_id, 11u);
  EXPECT_FALSE(pop_first.use_last_block_cert_votes);
  EXPECT_EQ(pop_first.next_entry_id, 22u);
  EXPECT_EQ(pop_first.current_period, 2u);
  EXPECT_EQ(pop_first.effective_size, 1u);

  auto pop_second = queue->period_data_queue_pop();
  EXPECT_EQ(pop_second.entry_id, 22u);
  EXPECT_TRUE(pop_second.use_last_block_cert_votes);
  EXPECT_EQ(pop_second.next_entry_id, 0u);
  EXPECT_EQ(pop_second.current_period, 0u);
  EXPECT_EQ(pop_second.effective_size, 0u);

  EXPECT_TRUE(queue->period_data_queue_empty());
  auto no_last = queue->period_data_queue_last_entry();
  EXPECT_FALSE(no_last.found);
}

TEST(RustPeriodDataQueueTest, SizeHidesTailWhenLastCertVotesMissing) {
  auto queue = create_period_data_queue();

  auto outcome = queue->period_data_queue_push(31, 1, hashFor(0x31), 0, 0);
  ASSERT_TRUE(outcome.accepted);
  EXPECT_FALSE(queue->period_data_queue_empty());
  EXPECT_EQ(queue->period_data_queue_size(), 0u);
}

TEST(RustPeriodDataQueueTest, PushRejectsInvalidPeriodSequenceAndAllowsQueueEmptyBackfill) {
  auto queue = create_period_data_queue();

  auto rejected = queue->period_data_queue_push(41, 3, hashFor(0x41), 0, 1);
  EXPECT_FALSE(rejected.accepted);
  EXPECT_EQ(rejected.expected_next_period, 1u);
  EXPECT_EQ(rejected.actual_period, 3u);

  auto backfill = queue->period_data_queue_push(42, 2, hashFor(0x42), 0, 1);
  EXPECT_TRUE(backfill.accepted);

  auto sequential = queue->period_data_queue_push(43, 3, hashFor(0x43), 1, 1);
  EXPECT_TRUE(sequential.accepted);

  auto rejected_gap = queue->period_data_queue_push(44, 5, hashFor(0x44), 3, 1);
  EXPECT_FALSE(rejected_gap.accepted);
  EXPECT_EQ(queue->period_data_queue_period(), 3u);
}

TEST(RustPeriodDataQueueTest, CleanOldDataAndClear) {
  auto queue = create_period_data_queue();

  ASSERT_TRUE(queue->period_data_queue_push(51, 5, hashFor(0x51), 4, 1).accepted);
  ASSERT_TRUE(queue->period_data_queue_push(52, 6, hashFor(0x52), 4, 1).accepted);

  auto removed = queue->period_data_queue_clean_old_data(6);
  ASSERT_EQ(removed.size(), 1u);
  EXPECT_EQ(removed[0].entry_id, 51u);
  EXPECT_EQ(removed[0].period, 5u);
  EXPECT_EQ(removed[0].block_hash, hashFor(0x51));

  EXPECT_EQ(queue->period_data_queue_period(), 6u);
  EXPECT_EQ(queue->period_data_queue_size(), 1u);

  auto remaining = queue->period_data_queue_pop();
  EXPECT_EQ(remaining.entry_id, 52u);
  EXPECT_TRUE(remaining.use_last_block_cert_votes);

  EXPECT_THROW((void)queue->period_data_queue_pop(), std::exception);

  ASSERT_TRUE(queue->period_data_queue_push(53, 1, hashFor(0x53), 0, 1).accepted);
  queue->period_data_queue_clear();
  EXPECT_EQ(queue->period_data_queue_period(), 0u);
  EXPECT_TRUE(queue->period_data_queue_empty());
  EXPECT_EQ(queue->period_data_queue_size(), 0u);
}

TEST(RustPeriodDataQueueTest, PushCanSignalQueueResetAfterChainProgress) {
  auto queue = create_period_data_queue();

  ASSERT_TRUE(queue->period_data_queue_push(61, 2, hashFor(0x61), 0, 1).accepted);

  auto outcome = queue->period_data_queue_push(64, 4, hashFor(0x64), 3, 1);
  ASSERT_TRUE(outcome.accepted);
  EXPECT_TRUE(outcome.clear_existing);

  auto last = queue->period_data_queue_last_entry();
  ASSERT_TRUE(last.found);
  EXPECT_EQ(last.entry_id, 64u);
  EXPECT_EQ(last.period, 4u);
  EXPECT_EQ(last.block_hash, hashFor(0x64));
}
