#include <gtest/gtest.h>

#include <vector>

#include "pbft/pbft_block.hpp"
#include "pbft/pbft_service.hpp"
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

SharedPbftService makeService(const std::shared_ptr<DbStorage>& db) {
  rustaxa::PbftServiceConfig config{};
  config.genesis_lambda_ms = 1000;
  config.cacti_lambda_max_ms = 1000;
  config.cacti_lambda_default_ms = 1000;
  config.max_exponential_lambda_ms = 60000;
  config.max_steps = 13;
  config.deadline_ms = 4000;
  config.polling_interval_ms = 100;
  return std::make_shared<PbftService>(rustaxa::create_pbft_service_from_storage(db->rustStorage(), config));
}

}  // namespace

struct ProposedBlocksShimDataTest : WithDataDir {};

TEST_F(ProposedBlocksShimDataTest, nullServiceConstructionIsUnsupportedInRustMode) {
  EXPECT_THROW(ProposedBlocks(nullptr), std::runtime_error);
}

TEST_F(ProposedBlocksShimDataTest, servicePushPublishesAuthoritativeProposal) {
  auto db = std::make_shared<DbStorage>(data_dir);
  ProposedBlocks proposed_blocks(makeService(db));
  auto block = makeBlock(1, 10);

  EXPECT_TRUE(proposed_blocks.pushProposedPbftBlock(block));
  EXPECT_FALSE(proposed_blocks.pushProposedPbftBlock(block));
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

  auto identity_block = makeBlock(1, 11);
  EXPECT_TRUE(proposed_blocks.pushProposedPbftBlock(identity_block));
  proposed_blocks.markBlockAsValid(identity_block->getPeriod(), identity_block->getBlockHash());
  metadata = proposed_blocks.getPbftProposedBlockMetadata(identity_block->getPeriod(), identity_block->getBlockHash());
  ASSERT_TRUE(metadata.has_value());
  EXPECT_TRUE(metadata->is_valid);

  proposed_blocks.cleanupProposedPbftBlocksByPeriod(2);
  EXPECT_FALSE(proposed_blocks.isInProposedBlocks(block->getPeriod(), block->getBlockHash()));
  EXPECT_FALSE(proposed_blocks.isInProposedBlocks(identity_block->getPeriod(), identity_block->getBlockHash()));
  EXPECT_TRUE(db->getProposedPbftBlocks().empty());
}

TEST_F(ProposedBlocksShimDataTest, serviceConstructionRestoresStorage) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto period_one_block = makeBlock(1, 101);
  auto period_two_block = makeBlock(2, 202);

  db->saveProposedPbftBlock(period_one_block);
  db->saveProposedPbftBlock(period_two_block);

  ProposedBlocks proposed_blocks(makeService(db));
  EXPECT_TRUE(proposed_blocks.isInProposedBlocks(period_one_block->getPeriod(), period_one_block->getBlockHash()));
  EXPECT_TRUE(proposed_blocks.isInProposedBlocks(period_two_block->getPeriod(), period_two_block->getBlockHash()));

  EXPECT_EQ(proposed_blocks.restoreFromStorage(), 0);
  EXPECT_TRUE(proposed_blocks.isInProposedBlocks(period_one_block->getPeriod(), period_one_block->getBlockHash()));
  EXPECT_TRUE(proposed_blocks.isInProposedBlocks(period_two_block->getPeriod(), period_two_block->getBlockHash()));
  auto metadata =
      proposed_blocks.getPbftProposedBlockMetadata(period_two_block->getPeriod(), period_two_block->getBlockHash());
  ASSERT_TRUE(metadata.has_value());
  EXPECT_EQ(metadata->pivot_hash, period_two_block->getPivotDagBlockHash());
}

TEST_F(ProposedBlocksShimDataTest, persistenceAndCleanupUseRustIndexAndDb) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto period_one_block = makeBlock(1, 101);
  auto period_two_block = makeBlock(2, 202);

  db->saveProposedPbftBlock(period_one_block);
  db->saveProposedPbftBlock(period_two_block);

  ProposedBlocks proposed_blocks(makeService(db));
  EXPECT_EQ(proposed_blocks.restoreFromStorage(), 0);
  ASSERT_EQ(db->getProposedPbftBlocks().size(), 2);

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

TEST_F(ProposedBlocksShimDataTest, retainedRustStorageOutlivesCppDbOwner) {
  const auto block = makeBlock(1, 303);

  {
    auto db = std::make_shared<DbStorage>(data_dir);
    ProposedBlocks proposed_blocks(makeService(db));
    db.reset();

    EXPECT_TRUE(proposed_blocks.pushProposedPbftBlock(block, true));
  }

  {
    auto db = std::make_shared<DbStorage>(data_dir);
    ASSERT_EQ(db->getProposedPbftBlocks().size(), 1);
    ProposedBlocks proposed_blocks(makeService(db));
    db.reset();

    EXPECT_EQ(proposed_blocks.restoreFromStorage(), 0);
    proposed_blocks.cleanupProposedPbftBlocksByPeriod(2);
  }

  auto db = std::make_shared<DbStorage>(data_dir);
  EXPECT_TRUE(db->getProposedPbftBlocks().empty());
}

TEST_F(ProposedBlocksShimDataTest, missingMarkValidThrows) {
  auto db = std::make_shared<DbStorage>(data_dir);
  ProposedBlocks proposed_blocks(makeService(db));
  const auto block = makeBlock(7, 707);
  EXPECT_THROW(proposed_blocks.markBlockAsValid(block), std::runtime_error);
  EXPECT_THROW(proposed_blocks.markBlockAsValid(block->getPeriod(), block->getBlockHash()), std::runtime_error);
}

}  // namespace taraxa::core_tests
