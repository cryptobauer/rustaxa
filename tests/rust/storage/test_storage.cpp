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

  static rust::Box<BridgeStorageQueries> dagQueries(const rust::Box<BridgeConsensusApplication>& application) {
    return create_dag_storage_queries(*application);
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
  auto storage = application();
  const auto hash = h256(0x11);
  auto value = dagQueries(storage)->get_dag_block(hash);
  EXPECT_TRUE(value.empty());
}

TEST_F(StorageTest, DagBlockPeriodLookupReflectsFoundState) {
  auto storage = application();
  const auto missing = h256(0x22);
  auto lookup = dagQueries(storage)->get_dag_block_period_lookup(missing);
  EXPECT_FALSE(lookup.found);

  const auto existing = h256(0x33);
  auto seed_batch = create_storage_shim_batch(*storage);
  storage_shim_save_dag_block_period(*seed_batch, existing, 7, 4);
  storage_shim_commit_batch(std::move(seed_batch), false);
  auto found = dagQueries(storage)->get_dag_block_period_lookup(existing);
  EXPECT_TRUE(found.found);
  EXPECT_EQ(found.period, 7u);
  EXPECT_EQ(found.position, 4u);
}
