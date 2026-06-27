#include <gtest/gtest.h>

#include <cstdint>
#include <exception>
#include <filesystem>
#include <string>
#include <string_view>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

namespace {

SortitionRuntimeConfig runtime_config() {
  SortitionRuntimeConfig config;
  config.threshold_upper = 2000;
  config.difficulty_min = 1;
  config.difficulty_max = 2;
  config.difficulty_stale = 3;
  config.lambda_bound = 1500;
  config.changes_count_for_average = 3;
  config.dag_efficiency_target_low = 48 * 100;
  config.dag_efficiency_target_high = 52 * 100;
  config.changing_interval = 2;
  config.computation_interval = 2;
  return config;
}

rust::Box<BridgeSortitionParamsManager> create_manager(std::string_view name) {
  const auto test_dir = std::filesystem::temp_directory_path() / std::string(name);
  if (std::filesystem::exists(test_dir)) {
    std::filesystem::remove_all(test_dir);
  }
  auto storage = create_storage(test_dir.string());
  auto manager = create_sortition_params_manager_from_storage(runtime_config(), *storage);
  std::filesystem::remove_all(test_dir);
  return manager;
}

}  // namespace

TEST(RustSortitionTest, ManagerEmitsThresholdChangeForInterval) {
  auto manager = create_manager("rustaxa_consensus_sortition_threshold_change");

  auto first = manager->sortition_record_finalized_period(1, true, 25, 100, 1);
  EXPECT_FALSE(first.changed);

  auto second = manager->sortition_record_finalized_period(2, true, 25, 100, 2);
  ASSERT_TRUE(second.changed);
  EXPECT_EQ(second.period, 2);
  EXPECT_EQ(second.interval_efficiency, 25 * 100);
  EXPECT_EQ(second.threshold_upper, 0x50);

  auto params = manager->sortition_current_params();
  EXPECT_EQ(params.threshold_upper, second.threshold_upper);
  EXPECT_EQ(params.lambda_bound, 1500);

  auto changes = manager->sortition_params_changes();
  ASSERT_EQ(changes.size(), 2);
  EXPECT_EQ(changes[1].period, 2);
  EXPECT_EQ(changes[1].threshold_upper, second.threshold_upper);
}

TEST(RustSortitionTest, PreviewDoesNotPublishThresholdUntilCommit) {
  auto manager = create_manager("rustaxa_consensus_sortition_preview_commit");

  auto first_preview = manager->sortition_preview_finalized_period(1, true, 25, 100, 1);
  EXPECT_FALSE(first_preview.changed);
  EXPECT_EQ(manager->sortition_current_params().threshold_upper, 2000);

  SortitionParamsChangePayload no_change;
  auto first_commit = manager->sortition_commit_finalized_period(1, true, 25, 100, 1, false, no_change);
  EXPECT_FALSE(first_commit.changed);

  auto second_preview = manager->sortition_preview_finalized_period(2, true, 25, 100, 2);
  ASSERT_TRUE(second_preview.changed);
  EXPECT_EQ(manager->sortition_current_params().threshold_upper, 2000);

  SortitionParamsChangePayload expected;
  expected.period = second_preview.period;
  expected.interval_efficiency = second_preview.interval_efficiency;
  expected.threshold_upper = second_preview.threshold_upper;
  auto second_commit = manager->sortition_commit_finalized_period(2, true, 25, 100, 2, true, expected);

  ASSERT_TRUE(second_commit.changed);
  EXPECT_EQ(second_commit.period, second_preview.period);
  EXPECT_EQ(manager->sortition_current_params().threshold_upper, second_preview.threshold_upper);
}

TEST(RustSortitionTest, CommitRejectsPreviewMismatch) {
  auto manager = create_manager("rustaxa_consensus_sortition_preview_mismatch");
  auto preview = manager->sortition_preview_finalized_period(1, true, 25, 100, 1);
  EXPECT_FALSE(preview.changed);

  SortitionParamsChangePayload unexpected;
  unexpected.period = 1;
  unexpected.interval_efficiency = 25 * 100;
  unexpected.threshold_upper = 1234;
  EXPECT_THROW(manager->sortition_commit_finalized_period(1, true, 25, 100, 1, true, unexpected), std::exception);
}

TEST(RustSortitionTest, ParamsForPeriodAppliesStorageChangePayload) {
  auto manager = create_manager("rustaxa_consensus_sortition_params_for_period");
  SortitionParamsChangePayload change;
  change.period = 10;
  change.interval_efficiency = 75 * 100;
  change.threshold_upper = 4444;

  auto params = manager->sortition_params_for_period(true, change);

  EXPECT_EQ(params.threshold_upper, 4444);
  EXPECT_EQ(params.difficulty_min, 1);
  EXPECT_EQ(params.difficulty_max, 2);
  EXPECT_EQ(params.difficulty_stale, 3);
}
