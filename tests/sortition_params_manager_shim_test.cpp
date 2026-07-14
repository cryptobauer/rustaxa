#include <gtest/gtest.h>

#include "dag/dag_block.hpp"
#include "dag/sortition_params_manager.hpp"
#include "pbft/pbft_block.hpp"
#include "storage/storage.hpp"
#include "test_util/test_util.hpp"

namespace taraxa::core_tests {
namespace {

FullNodeConfig sortitionShimConfig() {
  FullNodeConfig config;
  auto& sortition = config.genesis.sortition;
  sortition.vrf.threshold_upper = 10000;
  sortition.vdf.difficulty_min = 16;
  sortition.vdf.difficulty_max = 21;
  sortition.vdf.difficulty_stale = 23;
  sortition.vdf.lambda_bound = 1500;
  sortition.changes_count_for_average = 10;
  sortition.dag_efficiency_targets = {48 * kOnePercent, 52 * kOnePercent};
  sortition.changing_interval = 1;
  sortition.computation_interval = 1;
  return config;
}

PeriodData makeSortitionPeriod(PbftPeriod period) {
  PeriodData data;
  std::vector<vote_hash_t> reward_votes_hashes;
  data.pbft_blk =
      std::make_shared<PbftBlock>(blk_hash_t(period), blk_hash_t(period + 100), kNullBlockHash, kNullBlockHash, period,
                                  addr_t(), dev::KeyPair::create().secret(), reward_votes_hashes);
  data.transactions.push_back(std::make_shared<Transaction>());
  data.dag_blocks.push_back(std::make_shared<DagBlock>(blk_hash_t(period + 200), level_t{}, vec_blk_t{},
                                                       vec_trx_t{dev::h256(1), dev::h256(2)},
                                                       dev::KeyPair::create().secret()));
  return data;
}

}  // namespace

struct SortitionParamsManagerShimDataTest : WithDataDir {};

TEST(SortitionParamsManagerShimTest, compatibilityChangeUsesCanonicalLegacyRlp) {
  VrfParams vrf;
  vrf.threshold_upper = 10000;
  const SortitionParamsChange change{42, 5000, vrf};
  const bytes expected{0xc7, 0x82, 0x27, 0x10, 0x2a, 0x82, 0x13, 0x88};

  EXPECT_EQ(change.rlp(), expected);
  const auto decoded = SortitionParamsChange::from_rlp(dev::RLP(expected));
  EXPECT_EQ(decoded.period, change.period);
  EXPECT_EQ(decoded.vrf_params.threshold_upper, change.vrf_params.threshold_upper);
  EXPECT_EQ(decoded.interval_efficiency, change.interval_efficiency);
}

TEST_F(SortitionParamsManagerShimDataTest, startupPersistsDefaultChangeThroughRustStorage) {
  auto db = std::make_shared<DbStorage>(data_dir);
  SortitionParamsManager manager({}, sortitionShimConfig(), db);

  const auto changes = db->getLastSortitionParams(10);
  ASSERT_EQ(changes.size(), 1);
  EXPECT_EQ(changes.front().period, 0);
  EXPECT_EQ(changes.front().vrf_params.threshold_upper, 10000);
  EXPECT_EQ(manager.getParamsChanges().size(), 1);
}

TEST_F(SortitionParamsManagerShimDataTest, finalizedPeriodPersistenceIgnoresCompatibilityBatch) {
  auto db = std::make_shared<DbStorage>(data_dir);
  SortitionParamsManager manager({}, sortitionShimConfig(), db);
  auto ignored_batch = db->createWriteBatch();

  manager.pbftBlockPushed(makeSortitionPeriod(9), ignored_batch, 1);

  const auto persisted_change = db->getParamsChangeForPeriod(9);
  ASSERT_TRUE(persisted_change.has_value());
  EXPECT_EQ(persisted_change->period, 9);
  EXPECT_EQ(persisted_change->interval_efficiency, 50 * kOnePercent);
  EXPECT_EQ(manager.getParamsChanges().back().period, 9);
}

}  // namespace taraxa::core_tests
