#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <exception>
#include <initializer_list>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

namespace {

std::array<uint8_t, 32> hashFor(uint8_t seed) {
  std::array<uint8_t, 32> hash{};
  hash.fill(seed);
  return hash;
}

rust::Vec<PbftSyncTransactionHash> txHashes(std::initializer_list<uint8_t> seeds) {
  rust::Vec<PbftSyncTransactionHash> hashes;
  for (auto seed : seeds) {
    hashes.push_back(PbftSyncTransactionHash{hashFor(seed)});
  }
  return hashes;
}

}  // namespace

TEST(RustPeriodDataQueueTest, PushPopAndLastEntryFollowLegacyRules) {
  auto queue = create_period_data_queue();

  auto first = queue->period_data_queue_push(11, 1, hashFor(0x11), hashFor(0xa1), hashFor(0xb1), txHashes({0xc1}),
                                             txHashes({0xd1}), false, false, 0, 1);
  ASSERT_TRUE(first.accepted);
  EXPECT_FALSE(first.clear_existing);
  EXPECT_EQ(queue->period_data_queue_period(), 1u);
  EXPECT_EQ(queue->period_data_queue_syncing_period(0), 1u);
  EXPECT_EQ(queue->period_data_queue_syncing_period(5), 5u);
  EXPECT_EQ(queue->period_data_queue_size(), 1u);

  auto second = queue->period_data_queue_push(22, 2, hashFor(0x22), hashFor(0xa2), hashFor(0xb2), txHashes({0xc2}),
                                              txHashes({0xd2}), true, false, 0, 1);
  ASSERT_TRUE(second.accepted);
  EXPECT_EQ(queue->period_data_queue_size(), 2u);

  auto last = queue->period_data_queue_last_entry();
  ASSERT_TRUE(last.found);
  EXPECT_EQ(last.entry_id, 22u);
  EXPECT_EQ(last.period, 2u);
  EXPECT_EQ(last.block_hash, hashFor(0x22));
  ASSERT_EQ(last.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(last.dag_transaction_hashes[0].hash, hashFor(0xc2));
  ASSERT_EQ(last.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(last.period_data_transaction_hashes[0].hash, hashFor(0xd2));
  EXPECT_TRUE(last.previous_cert_votes_present);
  EXPECT_FALSE(last.previous_cert_first_vote_has_weight);
  EXPECT_EQ(queue->period_data_queue_last_block_hash_or_chain(1, hashFor(0xee)), hashFor(0x22));
  EXPECT_EQ(queue->period_data_queue_last_block_hash_or_chain(3, hashFor(0xee)), hashFor(0xee));

  auto pop_first = queue->period_data_queue_pop();
  EXPECT_EQ(pop_first.entry_id, 11u);
  EXPECT_EQ(pop_first.entry_period, 1u);
  EXPECT_EQ(pop_first.block_hash, hashFor(0x11));
  EXPECT_EQ(pop_first.prev_block_hash, hashFor(0xa1));
  EXPECT_EQ(pop_first.pivot_hash, hashFor(0xb1));
  ASSERT_EQ(pop_first.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(pop_first.dag_transaction_hashes[0].hash, hashFor(0xc1));
  ASSERT_EQ(pop_first.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(pop_first.period_data_transaction_hashes[0].hash, hashFor(0xd1));
  EXPECT_FALSE(pop_first.previous_cert_votes_present);
  EXPECT_FALSE(pop_first.previous_cert_first_vote_has_weight);
  EXPECT_FALSE(pop_first.use_last_block_cert_votes);
  EXPECT_EQ(pop_first.next_entry_id, 22u);
  EXPECT_EQ(pop_first.current_period, 2u);
  EXPECT_EQ(pop_first.effective_size, 1u);

  auto pop_second = queue->period_data_queue_pop();
  EXPECT_EQ(pop_second.entry_id, 22u);
  EXPECT_EQ(pop_second.entry_period, 2u);
  EXPECT_EQ(pop_second.block_hash, hashFor(0x22));
  EXPECT_EQ(pop_second.prev_block_hash, hashFor(0xa2));
  EXPECT_EQ(pop_second.pivot_hash, hashFor(0xb2));
  ASSERT_EQ(pop_second.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(pop_second.dag_transaction_hashes[0].hash, hashFor(0xc2));
  ASSERT_EQ(pop_second.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(pop_second.period_data_transaction_hashes[0].hash, hashFor(0xd2));
  EXPECT_TRUE(pop_second.previous_cert_votes_present);
  EXPECT_FALSE(pop_second.previous_cert_first_vote_has_weight);
  EXPECT_TRUE(pop_second.use_last_block_cert_votes);
  EXPECT_EQ(pop_second.next_entry_id, 0u);
  EXPECT_EQ(pop_second.current_period, 0u);
  EXPECT_EQ(pop_second.effective_size, 0u);

  EXPECT_TRUE(queue->period_data_queue_empty());
  auto no_last = queue->period_data_queue_last_entry();
  EXPECT_FALSE(no_last.found);
  EXPECT_EQ(queue->period_data_queue_last_block_hash_or_chain(1, hashFor(0xee)), hashFor(0xee));
}

TEST(RustPeriodDataQueueTest, SizeHidesTailWhenLastCertVotesMissing) {
  auto queue = create_period_data_queue();

  auto outcome = queue->period_data_queue_push(31, 1, hashFor(0x31), hashFor(0xa3), hashFor(0xb3), txHashes({}),
                                               txHashes({}), false, false, 0, 0);
  ASSERT_TRUE(outcome.accepted);
  EXPECT_FALSE(queue->period_data_queue_empty());
  EXPECT_EQ(queue->period_data_queue_size(), 0u);
}

TEST(RustPeriodDataQueueTest, PushRejectsInvalidPeriodSequenceAndAllowsQueueEmptyBackfill) {
  auto queue = create_period_data_queue();

  auto rejected = queue->period_data_queue_push(41, 3, hashFor(0x41), hashFor(0xa4), hashFor(0xb4), txHashes({}),
                                                txHashes({}), false, false, 0, 1);
  EXPECT_FALSE(rejected.accepted);
  EXPECT_EQ(rejected.expected_next_period, 1u);
  EXPECT_EQ(rejected.actual_period, 3u);

  auto backfill = queue->period_data_queue_push(42, 2, hashFor(0x42), hashFor(0xa5), hashFor(0xb5), txHashes({}),
                                                txHashes({}), false, false, 0, 1);
  EXPECT_TRUE(backfill.accepted);

  auto sequential = queue->period_data_queue_push(43, 3, hashFor(0x43), hashFor(0xa6), hashFor(0xb6), txHashes({}),
                                                  txHashes({}), false, false, 1, 1);
  EXPECT_TRUE(sequential.accepted);

  auto rejected_gap = queue->period_data_queue_push(44, 5, hashFor(0x44), hashFor(0xa7), hashFor(0xb7), txHashes({}),
                                                    txHashes({}), false, false, 3, 1);
  EXPECT_FALSE(rejected_gap.accepted);
  EXPECT_EQ(queue->period_data_queue_period(), 3u);
  EXPECT_EQ(queue->period_data_queue_syncing_period(1), 3u);
}

TEST(RustPeriodDataQueueTest, CleanOldDataAndClear) {
  auto queue = create_period_data_queue();

  ASSERT_TRUE(queue
                  ->period_data_queue_push(51, 5, hashFor(0x51), hashFor(0xa8), hashFor(0xb8), txHashes({0xc8}),
                                           txHashes({0xd8}), true, true, 4, 1)
                  .accepted);
  ASSERT_TRUE(queue
                  ->period_data_queue_push(52, 6, hashFor(0x52), hashFor(0xa9), hashFor(0xb9), txHashes({0xc9}),
                                           txHashes({0xd9}), false, false, 4, 1)
                  .accepted);

  auto removed = queue->period_data_queue_clean_old_data(6);
  ASSERT_EQ(removed.size(), 1u);
  EXPECT_EQ(removed[0].entry_id, 51u);
  EXPECT_EQ(removed[0].period, 5u);
  EXPECT_EQ(removed[0].block_hash, hashFor(0x51));
  EXPECT_EQ(removed[0].prev_block_hash, hashFor(0xa8));
  EXPECT_EQ(removed[0].pivot_hash, hashFor(0xb8));
  ASSERT_EQ(removed[0].dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(removed[0].dag_transaction_hashes[0].hash, hashFor(0xc8));
  ASSERT_EQ(removed[0].period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(removed[0].period_data_transaction_hashes[0].hash, hashFor(0xd8));
  EXPECT_TRUE(removed[0].previous_cert_votes_present);
  EXPECT_TRUE(removed[0].previous_cert_first_vote_has_weight);

  EXPECT_EQ(queue->period_data_queue_period(), 6u);
  EXPECT_EQ(queue->period_data_queue_syncing_period(8), 8u);
  EXPECT_EQ(queue->period_data_queue_size(), 1u);

  auto remaining = queue->period_data_queue_pop();
  EXPECT_EQ(remaining.entry_id, 52u);
  EXPECT_EQ(remaining.prev_block_hash, hashFor(0xa9));
  EXPECT_EQ(remaining.pivot_hash, hashFor(0xb9));
  ASSERT_EQ(remaining.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(remaining.dag_transaction_hashes[0].hash, hashFor(0xc9));
  ASSERT_EQ(remaining.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(remaining.period_data_transaction_hashes[0].hash, hashFor(0xd9));
  EXPECT_FALSE(remaining.previous_cert_votes_present);
  EXPECT_FALSE(remaining.previous_cert_first_vote_has_weight);
  EXPECT_TRUE(remaining.use_last_block_cert_votes);

  EXPECT_THROW((void)queue->period_data_queue_pop(), std::exception);

  ASSERT_TRUE(queue
                  ->period_data_queue_push(53, 1, hashFor(0x53), hashFor(0xaa), hashFor(0xba), txHashes({}),
                                           txHashes({}), false, false, 0, 1)
                  .accepted);
  queue->period_data_queue_clear();
  EXPECT_EQ(queue->period_data_queue_period(), 0u);
  EXPECT_EQ(queue->period_data_queue_syncing_period(9), 9u);
  EXPECT_EQ(queue->period_data_queue_last_block_hash_or_chain(1, hashFor(0xee)), hashFor(0xee));
  EXPECT_TRUE(queue->period_data_queue_empty());
  EXPECT_EQ(queue->period_data_queue_size(), 0u);
}

TEST(RustPeriodDataQueueTest, PushCanSignalQueueResetAfterChainProgress) {
  auto queue = create_period_data_queue();

  ASSERT_TRUE(queue
                  ->period_data_queue_push(61, 2, hashFor(0x61), hashFor(0xab), hashFor(0xbb), txHashes({0xcb}),
                                           txHashes({0xdb}), false, false, 0, 1)
                  .accepted);

  auto outcome = queue->period_data_queue_push(64, 4, hashFor(0x64), hashFor(0xac), hashFor(0xbc), txHashes({0xcc}),
                                               txHashes({0xdc}), true, true, 3, 1);
  ASSERT_TRUE(outcome.accepted);
  EXPECT_TRUE(outcome.clear_existing);

  auto last = queue->period_data_queue_last_entry();
  ASSERT_TRUE(last.found);
  EXPECT_EQ(last.entry_id, 64u);
  EXPECT_EQ(last.period, 4u);
  EXPECT_EQ(last.block_hash, hashFor(0x64));
  EXPECT_EQ(last.prev_block_hash, hashFor(0xac));
  EXPECT_EQ(last.pivot_hash, hashFor(0xbc));
  ASSERT_EQ(last.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(last.dag_transaction_hashes[0].hash, hashFor(0xcc));
  ASSERT_EQ(last.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(last.period_data_transaction_hashes[0].hash, hashFor(0xdc));
  EXPECT_TRUE(last.previous_cert_votes_present);
  EXPECT_TRUE(last.previous_cert_first_vote_has_weight);
}
