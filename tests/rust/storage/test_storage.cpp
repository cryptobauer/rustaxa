#include <gtest/gtest.h>

#include <cstdint>
#include <filesystem>

#include "../consensus/consensus_application_test.hpp"
#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

PbftServiceConfig makePbftServiceConfig();

class StorageTest : public ::testing::Test {
 protected:
  void SetUp() override {
    test_dir = std::filesystem::temp_directory_path() / "rustaxa_storage_test";
    if (std::filesystem::exists(test_dir)) {
      std::filesystem::remove_all(test_dir);
    }
  }

  void TearDown() override {
    if (std::filesystem::exists(test_dir)) {
      std::filesystem::remove_all(test_dir);
    }
  }

  static std::array<uint8_t, 32> h256(uint8_t last_byte) {
    std::array<uint8_t, 32> hash{};
    hash[31] = last_byte;
    return hash;
  }

  rust::Box<BridgeConsensusApplication> application() const {
    return test::createConsensusApplication(test_dir, makePbftServiceConfig());
  }

  std::filesystem::path test_dir;
};

PbftServiceConfig makePbftServiceConfig() {
  PbftServiceConfig config{};
  config.genesis_lambda_ms = 100;
  config.cacti_lambda_max_ms = 1500;
  config.cacti_lambda_default_ms = 500;
  config.cacti_block = 100;
  config.max_exponential_lambda_ms = 60000;
  config.max_steps = 13;
  config.deadline_ms = 1000;
  config.polling_interval_ms = 100;
  config.ficus_activation_period = 0;
  config.pillar_blocks_interval = 10;
  config.sync_level_size = 10;
  config.is_light_node = false;
  config.light_node_history = 0;
  config.committee_size = 5;
  config.number_of_proposers = 20;
  return config;
}

TEST_F(StorageTest, CreateStorage) {
  auto storage = application();
  // rust::Box cannot be null, so this is effectively a constructor smoke test
  SUCCEED();
}

TEST_F(StorageTest, MissingDagBlockReturnsEmptyPayload) {
  auto root = application();
  auto query = create_consensus_query_api(*root);
  const auto hash = h256(0x11);
  const auto value = query->consensus_query_dag_block_by_hash(hash);
  EXPECT_FALSE(value.found);
}

TEST_F(StorageTest, LightHistoryAdminAcceptsZeroDagCutoffWithoutExposingStorage) {
  auto root = application();
  LightHistoryPruneRequest request;
  request.end_period_exclusive = 10;
  request.first_retained_dag_level = 0;
  request.live_cleanup = true;
  request.non_block_periods_to_keep = 5;
  const auto report = consensus_application_prune_light_history(*root, request);
  EXPECT_FALSE(report.changed);
  EXPECT_EQ(report.first_retained_dag_level, 0u);
}

TEST_F(StorageTest, ProductionRootConformanceBoundaryReturnsVersionedTranscript) {
  auto root = application();
  const auto observations = consensus_application_run_storage_conformance_v1(*root);
  ASSERT_EQ(observations.size(), 52u);
  EXPECT_EQ(std::string(observations.front().key), "status_default_executed_blk");
  EXPECT_EQ(std::string(observations.back().key), "final_chain_receipts_by_period_count");
}
