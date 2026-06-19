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

rust::Vec<PeriodDataQueuePillarVotePayload> pillarVoteRlps(std::initializer_list<uint8_t> seeds) {
  rust::Vec<PeriodDataQueuePillarVotePayload> payloads;
  for (auto seed : seeds) {
    PeriodDataQueuePillarVotePayload payload;
    payload.vote_rlp.push_back(seed);
    payload.vote_rlp.push_back(static_cast<uint8_t>(seed + 1));
    payloads.push_back(std::move(payload));
  }
  return payloads;
}

rust::Vec<PeriodDataQueueTransactionPayload> transactionRlps(std::initializer_list<uint8_t> seeds) {
  rust::Vec<PeriodDataQueueTransactionPayload> payloads;
  for (auto seed : seeds) {
    PeriodDataQueueTransactionPayload payload;
    payload.transaction_rlp.push_back(seed);
    payload.transaction_rlp.push_back(static_cast<uint8_t>(seed + 1));
    payloads.push_back(std::move(payload));
  }
  return payloads;
}

rust::Vec<PeriodDataQueuePbftVotePayload> pbftVoteRlps(std::initializer_list<uint8_t> seeds) {
  rust::Vec<PeriodDataQueuePbftVotePayload> payloads;
  for (auto seed : seeds) {
    PeriodDataQueuePbftVotePayload payload;
    payload.vote_rlp.push_back(seed);
    payload.vote_rlp.push_back(static_cast<uint8_t>(seed + 1));
    payloads.push_back(std::move(payload));
  }
  return payloads;
}

rust::Vec<PeriodDataQueueTransactionIdentity> txIdentities(std::initializer_list<uint8_t> seeds) {
  rust::Vec<PeriodDataQueueTransactionIdentity> identities;
  uint64_t input_index = 0;
  for (auto seed : seeds) {
    PeriodDataQueueTransactionIdentity identity;
    identity.input_index = input_index++;
    identity.hash = hashFor(seed);
    identity.transaction_nonce = hashFor(static_cast<uint8_t>(seed + 1));
    identity.sender.fill(static_cast<uint8_t>(seed + 2));
    identities.push_back(identity);
  }
  return identities;
}

}  // namespace

TEST(RustPeriodDataQueueTest, PushPopAndLastEntryFollowLegacyRules) {
  auto queue = create_period_data_queue();

  auto first = queue->period_data_queue_push(11, 1, hashFor(0x11), hashFor(0xa1), hashFor(0xb1), hashFor(0xe1),
                                             txHashes({0xf1}), pillarVoteRlps({0xa1}), transactionRlps({0xb1}),
                                             pbftVoteRlps({}), txHashes({0xc1}), txHashes({0xd1}),
                                             txIdentities({0xd1}), false, false, false, false, false, 0,
                                             pbftVoteRlps({0x81}));
  ASSERT_TRUE(first.accepted);
  EXPECT_FALSE(first.clear_existing);
  EXPECT_EQ(queue->period_data_queue_period(), 1u);
  EXPECT_EQ(queue->period_data_queue_syncing_period(0), 1u);
  EXPECT_EQ(queue->period_data_queue_syncing_period(5), 5u);
  EXPECT_EQ(queue->period_data_queue_size(), 1u);

  auto second = queue->period_data_queue_push(22, 2, hashFor(0x22), hashFor(0xa2), hashFor(0xb2), hashFor(0xe2),
                                              txHashes({0xf2}), pillarVoteRlps({0xa2}), transactionRlps({0xb2}),
                                              pbftVoteRlps({0x92}), txHashes({0xc2}), txHashes({0xd2}),
                                              txIdentities({0xd2}), true, false, true, true, false, 0,
                                              pbftVoteRlps({0x82}));
  ASSERT_TRUE(second.accepted);
  EXPECT_EQ(queue->period_data_queue_size(), 2u);

  auto last = queue->period_data_queue_last_entry();
  ASSERT_TRUE(last.found);
  EXPECT_EQ(last.entry_id, 22u);
  EXPECT_EQ(last.period, 2u);
  EXPECT_EQ(last.block_hash, hashFor(0x22));
  EXPECT_EQ(last.final_chain_hash, hashFor(0xe2));
  ASSERT_EQ(last.reward_vote_hashes.size(), 1u);
  EXPECT_EQ(last.reward_vote_hashes[0].hash, hashFor(0xf2));
  ASSERT_EQ(last.pillar_vote_rlps.size(), 1u);
  ASSERT_EQ(last.pillar_vote_rlps[0].vote_rlp.size(), 2u);
  EXPECT_EQ(last.pillar_vote_rlps[0].vote_rlp[0], 0xa2);
  EXPECT_EQ(last.pillar_vote_rlps[0].vote_rlp[1], 0xa3);
  ASSERT_EQ(last.transaction_rlps.size(), 1u);
  ASSERT_EQ(last.transaction_rlps[0].transaction_rlp.size(), 2u);
  EXPECT_EQ(last.transaction_rlps[0].transaction_rlp[0], 0xb2);
  EXPECT_EQ(last.transaction_rlps[0].transaction_rlp[1], 0xb3);
  ASSERT_EQ(last.previous_cert_vote_rlps.size(), 1u);
  EXPECT_EQ(last.previous_cert_vote_rlps[0].vote_rlp[0], 0x92);
  EXPECT_EQ(last.previous_cert_vote_rlps[0].vote_rlp[1], 0x93);
  ASSERT_EQ(last.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(last.dag_transaction_hashes[0].hash, hashFor(0xc2));
  ASSERT_EQ(last.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(last.period_data_transaction_hashes[0].hash, hashFor(0xd2));
  ASSERT_EQ(last.period_data_transaction_identities.size(), 1u);
  EXPECT_EQ(last.period_data_transaction_identities[0].hash, hashFor(0xd2));
  EXPECT_EQ(last.period_data_transaction_identities[0].input_index, 0u);
  EXPECT_TRUE(last.previous_cert_votes_present);
  EXPECT_FALSE(last.previous_cert_first_vote_has_weight);
  EXPECT_TRUE(last.pillar_votes_present);
  EXPECT_TRUE(last.extra_data_present);
  EXPECT_FALSE(last.extra_data_pillar_block_hash_present);
  EXPECT_EQ(queue->period_data_queue_last_block_hash_or_chain(1, hashFor(0xee)), hashFor(0x22));
  EXPECT_EQ(queue->period_data_queue_last_block_hash_or_chain(3, hashFor(0xee)), hashFor(0xee));

  auto pop_first = queue->period_data_queue_pop();
  EXPECT_EQ(pop_first.entry_id, 11u);
  EXPECT_EQ(pop_first.entry_period, 1u);
  EXPECT_EQ(pop_first.block_hash, hashFor(0x11));
  EXPECT_EQ(pop_first.prev_block_hash, hashFor(0xa1));
  EXPECT_EQ(pop_first.pivot_hash, hashFor(0xb1));
  EXPECT_EQ(pop_first.final_chain_hash, hashFor(0xe1));
  ASSERT_EQ(pop_first.reward_vote_hashes.size(), 1u);
  EXPECT_EQ(pop_first.reward_vote_hashes[0].hash, hashFor(0xf1));
  ASSERT_EQ(pop_first.pillar_vote_rlps.size(), 1u);
  ASSERT_EQ(pop_first.pillar_vote_rlps[0].vote_rlp.size(), 2u);
  EXPECT_EQ(pop_first.pillar_vote_rlps[0].vote_rlp[0], 0xa1);
  EXPECT_EQ(pop_first.pillar_vote_rlps[0].vote_rlp[1], 0xa2);
  ASSERT_EQ(pop_first.transaction_rlps.size(), 1u);
  ASSERT_EQ(pop_first.transaction_rlps[0].transaction_rlp.size(), 2u);
  EXPECT_EQ(pop_first.transaction_rlps[0].transaction_rlp[0], 0xb1);
  EXPECT_EQ(pop_first.transaction_rlps[0].transaction_rlp[1], 0xb2);
  ASSERT_EQ(pop_first.cert_vote_rlps.size(), 1u);
  EXPECT_EQ(pop_first.cert_vote_rlps[0].vote_rlp[0], 0x92);
  EXPECT_EQ(pop_first.cert_vote_rlps[0].vote_rlp[1], 0x93);
  EXPECT_TRUE(pop_first.previous_cert_vote_rlps.empty());
  ASSERT_EQ(pop_first.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(pop_first.dag_transaction_hashes[0].hash, hashFor(0xc1));
  ASSERT_EQ(pop_first.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(pop_first.period_data_transaction_hashes[0].hash, hashFor(0xd1));
  ASSERT_EQ(pop_first.period_data_transaction_identities.size(), 1u);
  EXPECT_EQ(pop_first.period_data_transaction_identities[0].hash, hashFor(0xd1));
  EXPECT_FALSE(pop_first.previous_cert_votes_present);
  EXPECT_FALSE(pop_first.previous_cert_first_vote_has_weight);
  EXPECT_FALSE(pop_first.pillar_votes_present);
  EXPECT_FALSE(pop_first.extra_data_present);
  EXPECT_FALSE(pop_first.extra_data_pillar_block_hash_present);
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
  EXPECT_EQ(pop_second.final_chain_hash, hashFor(0xe2));
  ASSERT_EQ(pop_second.reward_vote_hashes.size(), 1u);
  EXPECT_EQ(pop_second.reward_vote_hashes[0].hash, hashFor(0xf2));
  ASSERT_EQ(pop_second.pillar_vote_rlps.size(), 1u);
  ASSERT_EQ(pop_second.pillar_vote_rlps[0].vote_rlp[0], 0xa2);
  ASSERT_EQ(pop_second.transaction_rlps.size(), 1u);
  ASSERT_EQ(pop_second.transaction_rlps[0].transaction_rlp[0], 0xb2);
  ASSERT_EQ(pop_second.cert_vote_rlps.size(), 1u);
  EXPECT_EQ(pop_second.cert_vote_rlps[0].vote_rlp[0], 0x82);
  ASSERT_EQ(pop_second.previous_cert_vote_rlps.size(), 1u);
  EXPECT_EQ(pop_second.previous_cert_vote_rlps[0].vote_rlp[0], 0x92);
  ASSERT_EQ(pop_second.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(pop_second.dag_transaction_hashes[0].hash, hashFor(0xc2));
  ASSERT_EQ(pop_second.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(pop_second.period_data_transaction_hashes[0].hash, hashFor(0xd2));
  ASSERT_EQ(pop_second.period_data_transaction_identities.size(), 1u);
  EXPECT_EQ(pop_second.period_data_transaction_identities[0].hash, hashFor(0xd2));
  EXPECT_TRUE(pop_second.previous_cert_votes_present);
  EXPECT_FALSE(pop_second.previous_cert_first_vote_has_weight);
  EXPECT_TRUE(pop_second.pillar_votes_present);
  EXPECT_TRUE(pop_second.extra_data_present);
  EXPECT_FALSE(pop_second.extra_data_pillar_block_hash_present);
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

  auto outcome = queue->period_data_queue_push(31, 1, hashFor(0x31), hashFor(0xa3), hashFor(0xb3), hashFor(0xe3),
                                               txHashes({}), pillarVoteRlps({}), transactionRlps({}), pbftVoteRlps({}),
                                               txHashes({}), txHashes({}), txIdentities({}), false, false, false,
                                               false, false, 0, pbftVoteRlps({}));
  ASSERT_TRUE(outcome.accepted);
  EXPECT_FALSE(queue->period_data_queue_empty());
  EXPECT_EQ(queue->period_data_queue_size(), 0u);
}

TEST(RustPeriodDataQueueTest, PushRejectsInvalidPeriodSequenceAndAllowsQueueEmptyBackfill) {
  auto queue = create_period_data_queue();

  auto rejected = queue->period_data_queue_push(41, 3, hashFor(0x41), hashFor(0xa4), hashFor(0xb4), hashFor(0xe4),
                                                txHashes({}), pillarVoteRlps({}), transactionRlps({}), pbftVoteRlps({}),
                                                txHashes({}), txHashes({}), txIdentities({}), false, false, false,
                                                false, false, 0, pbftVoteRlps({0x84}));
  EXPECT_FALSE(rejected.accepted);
  EXPECT_EQ(rejected.expected_next_period, 1u);
  EXPECT_EQ(rejected.actual_period, 3u);

  auto backfill = queue->period_data_queue_push(42, 2, hashFor(0x42), hashFor(0xa5), hashFor(0xb5), hashFor(0xe5),
                                                txHashes({}), pillarVoteRlps({}), transactionRlps({}), pbftVoteRlps({}),
                                                txHashes({}), txHashes({}), txIdentities({}), false, false, false,
                                                false, false, 0, pbftVoteRlps({0x85}));
  EXPECT_TRUE(backfill.accepted);

  auto sequential = queue->period_data_queue_push(43, 3, hashFor(0x43), hashFor(0xa6), hashFor(0xb6), hashFor(0xe6),
                                                  txHashes({}), pillarVoteRlps({}), transactionRlps({}), pbftVoteRlps({}),
                                                  txHashes({}), txHashes({}), txIdentities({}), false, false,
                                                  false, false, false, 1, pbftVoteRlps({0x86}));
  EXPECT_TRUE(sequential.accepted);

  auto rejected_gap = queue->period_data_queue_push(44, 5, hashFor(0x44), hashFor(0xa7), hashFor(0xb7), hashFor(0xe7),
                                                    txHashes({}), pillarVoteRlps({}), transactionRlps({}),
                                                    pbftVoteRlps({}), txHashes({}), txHashes({}), txIdentities({}),
                                                    false, false,
                                                    false, false, false, 3, pbftVoteRlps({0x87}));
  EXPECT_FALSE(rejected_gap.accepted);
  EXPECT_EQ(queue->period_data_queue_period(), 3u);
  EXPECT_EQ(queue->period_data_queue_syncing_period(1), 3u);
}

TEST(RustPeriodDataQueueTest, CleanOldDataAndClear) {
  auto queue = create_period_data_queue();

  ASSERT_TRUE(queue
                  ->period_data_queue_push(51, 5, hashFor(0x51), hashFor(0xa8), hashFor(0xb8), hashFor(0xe8),
                                           txHashes({0xf8}), pillarVoteRlps({0xa8}), transactionRlps({0xb8}),
                                           pbftVoteRlps({0x98}), txHashes({0xc8}), txHashes({0xd8}),
                                           txIdentities({0xd8}), true, true, true, true, true, 4,
                                           pbftVoteRlps({0x88}))
                  .accepted);
  ASSERT_TRUE(queue
                  ->period_data_queue_push(52, 6, hashFor(0x52), hashFor(0xa9), hashFor(0xb9), hashFor(0xe9),
                                           txHashes({}), pillarVoteRlps({}), transactionRlps({}), pbftVoteRlps({}),
                                           txHashes({0xc9}), txHashes({0xd9}), txIdentities({0xd9}), false, false,
                                           false, false, false, 4, pbftVoteRlps({0x89}))
                  .accepted);

  auto removed = queue->period_data_queue_clean_old_data(6);
  ASSERT_EQ(removed.size(), 1u);
  EXPECT_EQ(removed[0].entry_id, 51u);
  EXPECT_EQ(removed[0].period, 5u);
  EXPECT_EQ(removed[0].block_hash, hashFor(0x51));
  EXPECT_EQ(removed[0].prev_block_hash, hashFor(0xa8));
  EXPECT_EQ(removed[0].pivot_hash, hashFor(0xb8));
  EXPECT_EQ(removed[0].final_chain_hash, hashFor(0xe8));
  ASSERT_EQ(removed[0].reward_vote_hashes.size(), 1u);
  EXPECT_EQ(removed[0].reward_vote_hashes[0].hash, hashFor(0xf8));
  ASSERT_EQ(removed[0].pillar_vote_rlps.size(), 1u);
  EXPECT_EQ(removed[0].pillar_vote_rlps[0].vote_rlp[0], 0xa8);
  ASSERT_EQ(removed[0].transaction_rlps.size(), 1u);
  EXPECT_EQ(removed[0].transaction_rlps[0].transaction_rlp[0], 0xb8);
  ASSERT_EQ(removed[0].previous_cert_vote_rlps.size(), 1u);
  EXPECT_EQ(removed[0].previous_cert_vote_rlps[0].vote_rlp[0], 0x98);
  ASSERT_EQ(removed[0].dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(removed[0].dag_transaction_hashes[0].hash, hashFor(0xc8));
  ASSERT_EQ(removed[0].period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(removed[0].period_data_transaction_hashes[0].hash, hashFor(0xd8));
  ASSERT_EQ(removed[0].period_data_transaction_identities.size(), 1u);
  EXPECT_EQ(removed[0].period_data_transaction_identities[0].hash, hashFor(0xd8));
  EXPECT_TRUE(removed[0].previous_cert_votes_present);
  EXPECT_TRUE(removed[0].previous_cert_first_vote_has_weight);
  EXPECT_TRUE(removed[0].pillar_votes_present);
  EXPECT_TRUE(removed[0].extra_data_present);
  EXPECT_TRUE(removed[0].extra_data_pillar_block_hash_present);

  EXPECT_EQ(queue->period_data_queue_period(), 6u);
  EXPECT_EQ(queue->period_data_queue_syncing_period(8), 8u);
  EXPECT_EQ(queue->period_data_queue_size(), 1u);

  auto remaining = queue->period_data_queue_pop();
  EXPECT_EQ(remaining.entry_id, 52u);
  EXPECT_EQ(remaining.prev_block_hash, hashFor(0xa9));
  EXPECT_EQ(remaining.pivot_hash, hashFor(0xb9));
  EXPECT_EQ(remaining.final_chain_hash, hashFor(0xe9));
  ASSERT_EQ(remaining.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(remaining.dag_transaction_hashes[0].hash, hashFor(0xc9));
  ASSERT_EQ(remaining.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(remaining.period_data_transaction_hashes[0].hash, hashFor(0xd9));
  ASSERT_EQ(remaining.period_data_transaction_identities.size(), 1u);
  EXPECT_EQ(remaining.period_data_transaction_identities[0].hash, hashFor(0xd9));
  EXPECT_FALSE(remaining.previous_cert_votes_present);
  EXPECT_FALSE(remaining.previous_cert_first_vote_has_weight);
  EXPECT_FALSE(remaining.pillar_votes_present);
  EXPECT_FALSE(remaining.extra_data_present);
  EXPECT_FALSE(remaining.extra_data_pillar_block_hash_present);
  EXPECT_TRUE(remaining.use_last_block_cert_votes);

  EXPECT_THROW((void)queue->period_data_queue_pop(), std::exception);

  ASSERT_TRUE(queue
                  ->period_data_queue_push(53, 1, hashFor(0x53), hashFor(0xaa), hashFor(0xba), hashFor(0xea),
                                           txHashes({}), pillarVoteRlps({}), transactionRlps({}), pbftVoteRlps({}),
                                           txHashes({}), txHashes({}), txIdentities({}), false, false, false,
                                           false, false, 0, pbftVoteRlps({0x8a}))
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
                  ->period_data_queue_push(61, 2, hashFor(0x61), hashFor(0xab), hashFor(0xbb), hashFor(0xeb),
                                           txHashes({}), pillarVoteRlps({}), transactionRlps({}), pbftVoteRlps({}),
                                           txHashes({0xcb}), txHashes({0xdb}), txIdentities({0xdb}), false, false,
                                           false, false, false, 0, pbftVoteRlps({0x8b}))
                  .accepted);

  auto outcome = queue->period_data_queue_push(64, 4, hashFor(0x64), hashFor(0xac), hashFor(0xbc), hashFor(0xec),
                                               txHashes({0xfc}), pillarVoteRlps({0xac}), transactionRlps({0xbc}),
                                               pbftVoteRlps({0x9c}), txHashes({0xcc}), txHashes({0xdc}),
                                               txIdentities({0xdc}), true, true, true, true, true, 3,
                                               pbftVoteRlps({0x8c}));
  ASSERT_TRUE(outcome.accepted);
  EXPECT_TRUE(outcome.clear_existing);

  auto last = queue->period_data_queue_last_entry();
  ASSERT_TRUE(last.found);
  EXPECT_EQ(last.entry_id, 64u);
  EXPECT_EQ(last.period, 4u);
  EXPECT_EQ(last.block_hash, hashFor(0x64));
  EXPECT_EQ(last.prev_block_hash, hashFor(0xac));
  EXPECT_EQ(last.pivot_hash, hashFor(0xbc));
  EXPECT_EQ(last.final_chain_hash, hashFor(0xec));
  ASSERT_EQ(last.reward_vote_hashes.size(), 1u);
  EXPECT_EQ(last.reward_vote_hashes[0].hash, hashFor(0xfc));
  ASSERT_EQ(last.dag_transaction_hashes.size(), 1u);
  EXPECT_EQ(last.dag_transaction_hashes[0].hash, hashFor(0xcc));
  ASSERT_EQ(last.period_data_transaction_hashes.size(), 1u);
  EXPECT_EQ(last.period_data_transaction_hashes[0].hash, hashFor(0xdc));
  ASSERT_EQ(last.period_data_transaction_identities.size(), 1u);
  EXPECT_EQ(last.period_data_transaction_identities[0].hash, hashFor(0xdc));
  EXPECT_TRUE(last.previous_cert_votes_present);
  EXPECT_TRUE(last.previous_cert_first_vote_has_weight);
  EXPECT_TRUE(last.pillar_votes_present);
  EXPECT_TRUE(last.extra_data_present);
  EXPECT_TRUE(last.extra_data_pillar_block_hash_present);
}
