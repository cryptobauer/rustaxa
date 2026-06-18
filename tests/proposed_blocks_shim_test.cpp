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
  auto metadata = proposed_blocks.getPbftProposedBlockMetadata(block->getPeriod(), block->getBlockHash());
  ASSERT_TRUE(metadata.has_value());
  EXPECT_EQ(metadata->pivot_hash, block->getPivotDagBlockHash());
  EXPECT_FALSE(metadata->is_valid);

  auto found = proposed_blocks.getPbftProposedBlock(block->getPeriod(), block->getBlockHash());
  ASSERT_TRUE(found.has_value());
  EXPECT_EQ(found->first->rlp(true), block->rlp(true));
  EXPECT_FALSE(found->second);

  proposed_blocks.markBlockAsValid(block);
  metadata = proposed_blocks.getPbftProposedBlockMetadata(block->getPeriod(), block->getBlockHash());
  ASSERT_TRUE(metadata.has_value());
  EXPECT_TRUE(metadata->is_valid);
  found = proposed_blocks.getPbftProposedBlock(block->getPeriod(), block->getBlockHash());
  ASSERT_TRUE(found.has_value());
  EXPECT_TRUE(found->second);

  proposed_blocks.cleanupProposedPbftBlocksByPeriod(2);
  EXPECT_FALSE(proposed_blocks.isInProposedBlocks(block->getPeriod(), block->getBlockHash()));
}

TEST_F(ProposedBlocksShimDataTest, restoreFromStorageHydratesRustIndex) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto period_one_block = makeBlock(1, 101);
  auto period_two_block = makeBlock(2, 202);

  db->saveProposedPbftBlock(period_one_block);
  db->saveProposedPbftBlock(period_two_block);

  ProposedBlocks proposed_blocks(db);
  EXPECT_FALSE(proposed_blocks.isInProposedBlocks(period_one_block->getPeriod(), period_one_block->getBlockHash()));
  EXPECT_FALSE(proposed_blocks.isInProposedBlocks(period_two_block->getPeriod(), period_two_block->getBlockHash()));

  EXPECT_EQ(proposed_blocks.restoreFromStorage(), 2);
  EXPECT_TRUE(proposed_blocks.isInProposedBlocks(period_one_block->getPeriod(), period_one_block->getBlockHash()));
  EXPECT_TRUE(proposed_blocks.isInProposedBlocks(period_two_block->getPeriod(), period_two_block->getBlockHash()));
  auto metadata = proposed_blocks.getPbftProposedBlockMetadata(period_two_block->getPeriod(),
                                                               period_two_block->getBlockHash());
  ASSERT_TRUE(metadata.has_value());
  EXPECT_EQ(metadata->pivot_hash, period_two_block->getPivotDagBlockHash());
}

TEST_F(ProposedBlocksShimDataTest, restoreFromStorageRequiresDb) {
  ProposedBlocks proposed_blocks(nullptr);
  EXPECT_THROW(proposed_blocks.restoreFromStorage(), std::runtime_error);
}

TEST_F(ProposedBlocksShimDataTest, persistenceAndCleanupUseRustIndexAndDb) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto period_one_block = makeBlock(1, 101);
  auto period_two_block = makeBlock(2, 202);

  db->saveProposedPbftBlock(period_one_block);
  db->saveProposedPbftBlock(period_two_block);

  ProposedBlocks proposed_blocks(db);
  EXPECT_EQ(proposed_blocks.restoreFromStorage(), 2);
  ASSERT_EQ(db->getProposedPbftBlocks().size(), 2);
  EXPECT_EQ(proposed_blocks.checkOldBlocksPresence(2), std::make_optional(std::string("1 -> 1. ")));

  const auto snapshot = proposed_blocks.getProposedBlocks();
  ASSERT_EQ(snapshot.size(), 2);
  ASSERT_EQ(snapshot.at(1).size(), 1);
  EXPECT_EQ(snapshot.at(1)[0]->rlp(true), period_one_block->rlp(true));

  proposed_blocks.cleanupProposedPbftBlocksByPeriod(2);
  EXPECT_FALSE(proposed_blocks.isInProposedBlocks(1, period_one_block->getBlockHash()));
  EXPECT_TRUE(proposed_blocks.isInProposedBlocks(2, period_two_block->getBlockHash()));
  const auto persisted = db->getProposedPbftBlocks();
  ASSERT_EQ(persisted.size(), 1);
  EXPECT_EQ(persisted[0]->getPeriod(), period_two_block->getPeriod());
  EXPECT_EQ(persisted[0]->rlp(true), period_two_block->rlp(true));
}

TEST_F(ProposedBlocksShimDataTest, missingMarkValidThrows) {
  ProposedBlocks proposed_blocks(nullptr);
  EXPECT_THROW(proposed_blocks.markBlockAsValid(makeBlock(7, 707)), std::runtime_error);
}

}  // namespace taraxa::core_tests
