#include <gtest/gtest.h>

#include <optional>
#include <string>
#include <type_traits>
#include <vector>

#include "pbft/pbft_block.hpp"
#include "pbft/period_data_queue.hpp"
#include "vote/pbft_vote.hpp"

namespace taraxa::core_tests {
namespace {

std::shared_ptr<PbftBlock> makeBlock(PbftPeriod period, uint64_t seed,
                                      const std::optional<PbftBlockExtraData>& extra_data = {}) {
  std::vector<vote_hash_t> reward_votes_hashes;
  return std::make_shared<PbftBlock>(blk_hash_t(seed), kNullBlockHash, kNullBlockHash, blk_hash_t(seed + 5000), period,
                                     addr_t(), dev::KeyPair::create().secret(), reward_votes_hashes, extra_data);
}

PeriodData makePeriodData(PbftPeriod period, uint64_t seed,
                          const std::vector<std::shared_ptr<PbftVote>>& previous_cert_votes = {},
                          const std::optional<PbftBlockExtraData>& extra_data = {}) {
  return PeriodData(makeBlock(period, seed, extra_data), previous_cert_votes);
}

}  // namespace

TEST(PeriodDataQueueShimTest, rustModePeriodDataQueueDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_PERIOD_DATA_QUEUE
  static_assert(!std::is_base_of_v<PeriodDataQueueOld, PeriodDataQueue>);
  SUCCEED();
#else
  GTEST_SKIP() << "PeriodDataQueue shim is disabled";
#endif
}

TEST(PeriodDataQueueShimTest, popReturnsQueueFrontAndMatchingCertVotesContract) {
  PeriodDataQueue queue;
  const dev::p2p::NodeID node1(11);
  const dev::p2p::NodeID node2(22);

  auto vote_from_next_block = std::make_shared<PbftVote>();
  auto vote_for_last_block = std::make_shared<PbftVote>();

  auto period1 = makePeriodData(1, 101);
  const auto extra_data = PbftBlockExtraData(1, 2, 3, 4, "rustaxa-test", blk_hash_t(303));
  auto period2 = makePeriodData(2, 202, {vote_from_next_block}, extra_data);
  const auto period2_hash = period2.pbft_blk->getBlockHash();
  const auto period2_final_chain_hash = period2.pbft_blk->getFinalChainHash();

  EXPECT_TRUE(queue.push(std::move(period1), node1, 0, {}));
  EXPECT_EQ(queue.size(), 0);
  EXPECT_EQ(queue.syncingPeriod(0), 1);
  EXPECT_EQ(queue.syncingPeriod(5), 5);
  EXPECT_TRUE(queue.push(std::move(period2), node2, 0, {vote_for_last_block}));
  EXPECT_EQ(queue.size(), 2);
  EXPECT_EQ(queue.syncingPeriod(1), 2);
  ASSERT_NE(queue.lastPbftBlock(), nullptr);
  EXPECT_EQ(queue.lastPbftBlock()->getPeriod(), 2);
  ASSERT_TRUE(queue.lastPbftBlockHash().has_value());
  EXPECT_EQ(*queue.lastPbftBlockHash(), period2_hash);
  EXPECT_EQ(queue.lastBlockHashOrChain(1, blk_hash_t(999)), period2_hash);
  EXPECT_EQ(queue.lastBlockHashOrChain(3, blk_hash_t(999)), blk_hash_t(999));

  auto popped1 = queue.popWithMetadata();
  EXPECT_EQ(popped1.period_data.pbft_blk->getPeriod(), 1);
  EXPECT_EQ(popped1.node_id, node1);
  EXPECT_EQ(popped1.period, 1);
  EXPECT_EQ(popped1.block_hash, popped1.period_data.pbft_blk->getBlockHash());
  EXPECT_EQ(popped1.prev_block_hash, popped1.period_data.pbft_blk->getPrevBlockHash());
  EXPECT_EQ(popped1.pivot_hash, popped1.period_data.pbft_blk->getPivotDagBlockHash());
  EXPECT_EQ(popped1.final_chain_hash, popped1.period_data.pbft_blk->getFinalChainHash());
  EXPECT_TRUE(popped1.dag_transaction_hashes.empty());
  EXPECT_TRUE(popped1.period_data_transaction_hashes.empty());
  EXPECT_FALSE(popped1.pillar_votes_present);
  EXPECT_FALSE(popped1.extra_data_present);
  EXPECT_FALSE(popped1.extra_data_pillar_block_hash_present);
  ASSERT_EQ(popped1.cert_votes.size(), 1);
  EXPECT_EQ(popped1.cert_votes[0].get(), vote_from_next_block.get());
  EXPECT_EQ(queue.size(), 1);
  EXPECT_EQ(queue.getPeriod(), 2);

  auto popped2 = queue.popWithMetadata();
  EXPECT_EQ(popped2.period_data.pbft_blk->getPeriod(), 2);
  EXPECT_EQ(popped2.node_id, node2);
  EXPECT_EQ(popped2.final_chain_hash, period2_final_chain_hash);
  EXPECT_TRUE(popped2.extra_data_present);
  EXPECT_TRUE(popped2.extra_data_pillar_block_hash_present);
  ASSERT_EQ(popped2.cert_votes.size(), 1);
  EXPECT_EQ(popped2.cert_votes[0].get(), vote_for_last_block.get());
  EXPECT_TRUE(queue.empty());
  EXPECT_FALSE(queue.lastPbftBlockHash().has_value());
  EXPECT_EQ(queue.lastBlockHashOrChain(1, blk_hash_t(999)), blk_hash_t(999));
  EXPECT_EQ(queue.size(), 0);
  EXPECT_EQ(queue.getPeriod(), 0);
}

TEST(PeriodDataQueueShimTest, periodAdmissionAndCleanupBehaviorMatchesLegacyContract) {
  PeriodDataQueue queue;
  const dev::p2p::NodeID node(33);

  EXPECT_FALSE(queue.push(makePeriodData(3, 303), node, 0, {}));
  EXPECT_TRUE(queue.push(makePeriodData(2, 202), node, 0, {}));
  EXPECT_FALSE(queue.push(makePeriodData(4, 404), node, 1, {}));

  EXPECT_FALSE(queue.empty());
  queue.cleanOldData(3);
  EXPECT_TRUE(queue.empty());
  // Legacy contract keeps tracked period until explicit reset.
  EXPECT_EQ(queue.getPeriod(), 2);

  queue.clear();
  EXPECT_EQ(queue.getPeriod(), 0);
  EXPECT_EQ(queue.syncingPeriod(7), 7);
  EXPECT_TRUE(queue.push(makePeriodData(1, 101), node, 0, {}));
}

}  // namespace taraxa::core_tests
