#include <gtest/gtest.h>

#include <type_traits>
#include <vector>

#include "pbft/pbft_block.hpp"
#include "pbft/proposed_blocks.hpp"
#include "storage/storage.hpp"
#include "test_util/test_util.hpp"

namespace taraxa::core_tests {
namespace {

std::shared_ptr<PbftBlock> makeBlock(PbftPeriod period, uint64_t seed) {
  std::vector<vote_hash_t> reward_votes_hashes;
  return std::make_shared<PbftBlock>(blk_hash_t(seed), kNullBlockHash, kNullBlockHash, kNullBlockHash, period, addr_t(),
                                     dev::KeyPair::create().secret(), reward_votes_hashes);
}

}  // namespace

TEST(ProposedBlocksShimTest, rustModeProposedBlocksDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_PROPOSED_BLOCKS
  static_assert(!std::is_base_of_v<ProposedBlocksOld, ProposedBlocks>);
  SUCCEED();
#else
  GTEST_SKIP() << "ProposedBlocks shim is disabled";
#endif
}

struct ProposedBlocksShimDataTest : WithDataDir {};

TEST_F(ProposedBlocksShimDataTest, nullDbWorksWhenPersistenceIsDisabled) {
  ProposedBlocks proposed_blocks(nullptr);
  auto block = makeBlock(1, 10);

  EXPECT_TRUE(proposed_blocks.pushProposedPbftBlock(block, false));
  EXPECT_FALSE(proposed_blocks.pushProposedPbftBlock(block, false));
  EXPECT_TRUE(proposed_blocks.isInProposedBlocks(block->getPeriod(), block->getBlockHash()));

  auto found = proposed_blocks.getPbftProposedBlock(block->getPeriod(), block->getBlockHash());
  ASSERT_TRUE(found.has_value());
  EXPECT_EQ(found->first->rlp(true), block->rlp(true));
  EXPECT_FALSE(found->second);

  proposed_blocks.markBlockAsValid(block);
  found = proposed_blocks.getPbftProposedBlock(block->getPeriod(), block->getBlockHash());
  ASSERT_TRUE(found.has_value());
  EXPECT_TRUE(found->second);
}

TEST_F(ProposedBlocksShimDataTest, persistenceAndCleanupUseRustIndex) {
  auto db = std::make_shared<DbStorage>(data_dir);
  ProposedBlocks proposed_blocks(db);
  auto period_one_block = makeBlock(1, 101);
  auto period_two_block = makeBlock(2, 202);

  EXPECT_TRUE(proposed_blocks.pushProposedPbftBlock(period_one_block));
  EXPECT_TRUE(proposed_blocks.pushProposedPbftBlock(period_two_block));
  EXPECT_EQ(db->getProposedPbftBlocks().size(), 2);
  EXPECT_EQ(proposed_blocks.checkOldBlocksPresence(2), std::make_optional(std::string("1 -> 1. ")));

  const auto snapshot = proposed_blocks.getProposedBlocks();
  ASSERT_EQ(snapshot.size(), 2);
  ASSERT_EQ(snapshot.at(1).size(), 1);
  EXPECT_EQ(snapshot.at(1)[0]->rlp(true), period_one_block->rlp(true));

  proposed_blocks.cleanupProposedPbftBlocksByPeriod(2);
  EXPECT_FALSE(proposed_blocks.isInProposedBlocks(1, period_one_block->getBlockHash()));
  EXPECT_TRUE(proposed_blocks.isInProposedBlocks(2, period_two_block->getBlockHash()));
  EXPECT_EQ(db->getProposedPbftBlocks().size(), 1);
  EXPECT_EQ(db->getProposedPbftBlocks()[0]->rlp(true), period_two_block->rlp(true));
}

TEST_F(ProposedBlocksShimDataTest, missingMarkValidThrows) {
  ProposedBlocks proposed_blocks(nullptr);
  EXPECT_THROW(proposed_blocks.markBlockAsValid(makeBlock(7, 707)), std::runtime_error);
}

}  // namespace taraxa::core_tests
