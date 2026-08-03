#include <gtest/gtest.h>

#include <memory>
#include <vector>

#include "pbft/pbft_chain.hpp"
#include "pbft/pbft_service.hpp"
#include "pbft/period_data.hpp"
#include "storage/storage.hpp"
#include "test_util/test_util.hpp"

namespace taraxa::core_tests {
namespace {

SharedPbftService makeService(const std::shared_ptr<DbStorage>& db) {
  rustaxa::PbftServiceConfig config{};
  config.genesis_lambda_ms = 1000;
  config.cacti_lambda_max_ms = 1000;
  config.cacti_lambda_default_ms = 1000;
  config.max_exponential_lambda_ms = 60000;
  config.max_steps = 13;
  config.deadline_ms = 4000;
  config.polling_interval_ms = 100;
  config.ficus_activation_period = 0;
  config.pillar_blocks_interval = 10;
  return std::make_shared<PbftService>(rustaxa::create_pbft_service_from_storage(db->rustStorage(), config));
}

std::shared_ptr<PbftBlock> makeBlock(PbftPeriod period, uint64_t seed) {
  std::vector<vote_hash_t> reward_votes_hashes;
  return std::make_shared<PbftBlock>(kNullBlockHash, blk_hash_t(seed), kNullBlockHash, kNullBlockHash, period, addr_t{},
                                     dev::KeyPair::create().secret(), reward_votes_hashes);
}

}  // namespace

struct PbftChainShimDataTest : WithDataDir {};

TEST_F(PbftChainShimDataTest, retainedRustStorageOutlivesCppDbOwner) {
  const auto block = makeBlock(1, 303);
  auto db = std::make_shared<DbStorage>(data_dir);
  auto batch = db->createWriteBatch();
  db->savePeriodData(PeriodData(block, {}), batch);
  db->commitWriteBatch(batch);

  PbftChain chain(addr_t{}, makeService(db));
  db.reset();

  EXPECT_TRUE(chain.findPbftBlockInChain(block->getBlockHash()));
  EXPECT_EQ(chain.getPbftBlockInChain(block->getBlockHash()).rlp(true), block->rlp(true));

  chain.updatePbftChain(block->getBlockHash(), block->getPivotDagBlockHash());
  EXPECT_EQ(chain.getPbftChainSize(), 1);
  EXPECT_EQ(chain.getPbftChainSizeExcludingEmptyPbftBlocks(), 1);
  EXPECT_EQ(chain.getLastPbftBlockHash(), block->getBlockHash());
  EXPECT_EQ(chain.getLastNonNullPbftBlockAnchor(), block->getPivotDagBlockHash());
  EXPECT_NE(chain.getJsonStr().find(block->getBlockHash().toString()), std::string::npos);

  const auto projected = chain.getJsonStrForBlock(blk_hash_t(404), true);
  EXPECT_NE(projected.find(blk_hash_t(404).toString()), std::string::npos);
  EXPECT_EQ(chain.getPbftChainSize(), 1);
}

TEST_F(PbftChainShimDataTest, sharedServicePublishesOneChainStateAcrossFacades) {
  const auto block = makeBlock(1, 505);
  auto db = std::make_shared<DbStorage>(data_dir);
  auto service = makeService(db);

  PbftChain writer(addr_t{}, service);
  PbftChain reader(addr_t{}, service);
  writer.updatePbftChain(block->getBlockHash(), block->getPivotDagBlockHash());

  EXPECT_EQ(reader.getPbftChainSize(), 1);
  EXPECT_EQ(reader.getLastPbftBlockHash(), block->getBlockHash());
  EXPECT_EQ(reader.getLastNonNullPbftBlockAnchor(), block->getPivotDagBlockHash());

  service.reset();
  EXPECT_EQ(writer.getPbftChainSize(), 1);
  EXPECT_EQ(reader.getPbftChainSize(), 1);
}

}  // namespace taraxa::core_tests
