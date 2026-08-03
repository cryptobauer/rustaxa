#include <gtest/gtest.h>

#include <cstdint>
#include <filesystem>
#include <string_view>

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

  static rust::Vec<uint8_t> bytes(std::initializer_list<uint8_t> values) {
    rust::Vec<uint8_t> out;
    out.reserve(values.size());
    for (auto value : values) {
      out.push_back(value);
    }
    return out;
  }

  static rust::Vec<uint8_t> bytes(std::string_view value) {
    rust::Vec<uint8_t> out;
    out.reserve(value.size());
    for (const auto ch : value) {
      out.push_back(static_cast<uint8_t>(ch));
    }
    return out;
  }

  static std::vector<uint8_t> to_std_vec(const rust::Vec<uint8_t>& values) {
    return std::vector<uint8_t>(values.begin(), values.end());
  }

  static rust::Box<BridgePbftVoteStorageQueries> voteQueries(const rust::Box<BridgeStorage>& storage) {
    return create_pbft_vote_storage_queries(*storage);
  }

  static rust::Box<BridgePbftStorageQueries> pbftQueries(const rust::Box<BridgeStorage>& storage) {
    return create_pbft_storage_queries(*storage);
  }

  static rust::Box<BridgeDagStorageQueries> dagQueries(const rust::Box<BridgeStorage>& storage) {
    return create_dag_storage_queries(*storage);
  }

  static rust::Box<BridgePbftService> pbftService(const rust::Box<BridgeStorage>& storage) {
    return create_pbft_service_from_storage(*storage, makePbftServiceConfig());
  }

  std::filesystem::path test_dir;
};

constexpr uint8_t kPbftVotePersistenceApplied = 0;
constexpr uint8_t kPbftVotePersistenceRejected = 1;
constexpr uint8_t kPbftManagerTransitionStorageApplied = 0;

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
  return config;
}

TEST_F(StorageTest, CreateStorage) {
  auto storage = create_storage(test_dir.string());
  // rust::Box cannot be null, so this is effectively a constructor smoke test
  SUCCEED();
}

TEST_F(StorageTest, MissingDagBlockReturnsEmptyPayload) {
  auto storage = create_storage(test_dir.string());
  const auto hash = h256(0x11);
  auto value = dagQueries(storage)->get_dag_block(hash);
  EXPECT_TRUE(value.empty());
}

TEST_F(StorageTest, DagBlockPeriodLookupReflectsFoundState) {
  auto storage = create_storage(test_dir.string());
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

TEST_F(StorageTest, PersistPbftVoteProgressRejectsMissingRetainedRewardPayload) {
  auto storage = create_storage(test_dir.string());

  PbftVoteProgressPersistenceWrite write{};
  write.has_extra_reward_vote = true;
  write.extra_reward_vote_hash = h256(0x44);

  EXPECT_THROW(pbftService(storage)->pbft_service_verified_votes_persist_pbft_vote_progress(write), std::exception);

  auto vote_queries = voteQueries(storage);
  EXPECT_TRUE(vote_queries->get_reward_votes().empty());
}

TEST_F(StorageTest, PersistPbftVoteProgressRejectsInvalidTwoTPlusOneKind) {
  auto storage = create_storage(test_dir.string());

  PbftVoteProgressPersistenceWrite write{};
  write.has_two_t_plus_one_bundle = true;
  write.two_t_plus_one_kind = 99;

  auto result = pbftService(storage)->pbft_service_verified_votes_persist_pbft_vote_progress(write);
  EXPECT_EQ(result.status, kPbftVotePersistenceRejected);
  EXPECT_FALSE(result.error_code.empty());
  auto vote_queries = voteQueries(storage);
  EXPECT_TRUE(vote_queries->get_all_two_t_plus_one_votes().empty());
}

TEST_F(StorageTest, PersistPbftVoteProgressRejectsMissingNativeTwoTPlusOneMapping) {
  auto storage = create_storage(test_dir.string());

  PbftVoteProgressPersistenceWrite write{};
  write.has_two_t_plus_one_bundle = true;
  write.two_t_plus_one_kind = 0;
  write.two_t_plus_one_period = 10;
  write.two_t_plus_one_round = 2;
  write.two_t_plus_one_step = 3;
  write.two_t_plus_one_block_hash = h256(0x55);

  EXPECT_THROW(pbftService(storage)->pbft_service_verified_votes_persist_pbft_vote_progress(write), std::exception);
  auto vote_queries = voteQueries(storage);
  EXPECT_TRUE(vote_queries->get_all_two_t_plus_one_votes().empty());
}

TEST_F(StorageTest, ClearOwnVerifiedVotesCommitsRustOwnedBatch) {
  auto storage = create_storage(test_dir.string());
  auto pbft_service = pbftService(storage);
  auto own_vote_hash = h256(0x66);
  auto seed_batch = create_storage_shim_batch(*storage);
  storage_shim_save_own_verified_vote(*seed_batch, own_vote_hash, bytes({0x72}));
  storage_shim_commit_batch(std::move(seed_batch), false);
  auto vote_queries = voteQueries(storage);
  ASSERT_EQ(vote_queries->get_own_verified_votes().size(), 1u);

  auto result = pbft_service->pbft_service_verified_votes_clear_own_verified_votes();
  EXPECT_EQ(result.status, kPbftVotePersistenceApplied);
  EXPECT_EQ(result.applied_writes, 1u);
  EXPECT_TRUE(vote_queries->get_own_verified_votes().empty());
}

TEST_F(StorageTest, ClearOwnVerifiedVotesTreatsEmptyStorageAsNoOp) {
  auto storage = create_storage(test_dir.string());

  auto result = pbftService(storage)->pbft_service_verified_votes_clear_own_verified_votes();
  EXPECT_EQ(result.status, kPbftVotePersistenceApplied);
  EXPECT_EQ(result.applied_writes, 0u);
  auto vote_queries = voteQueries(storage);
  EXPECT_TRUE(vote_queries->get_own_verified_votes().empty());
}

TEST_F(StorageTest, ApplyPbftManagerTransitionStorageCommitsCursorStatusesAndOwnVoteCleanup) {
  auto storage = create_storage(test_dir.string());
  const auto own_vote_hash = h256(0x99);

  auto seed_batch = create_storage_shim_batch(*storage);
  storage_shim_save_pbft_mgr_field(*seed_batch, 0, 1);
  storage_shim_save_pbft_mgr_field(*seed_batch, 1, 1);
  storage_shim_save_pbft_mgr_status(*seed_batch, 2, true);
  storage_shim_save_pbft_mgr_status(*seed_batch, 3, true);
  const std::string pbft_head =
      R"({"head_hash":"0x0000000000000000000000000000000000000000000000000000000000000000","size":0,"non_empty_size":0,"last_pbft_block_hash":"0x0000000000000000000000000000000000000000000000000000000000000000"})";
  storage_shim_save_pbft_head(*seed_batch, h256(0), bytes(pbft_head));
  storage_shim_commit_batch(std::move(seed_batch), false);

  auto runtime = create_pbft_service_from_storage(*storage, makePbftServiceConfig());
  auto own_vote_batch = create_storage_shim_batch(*storage);
  storage_shim_save_own_verified_vote(*own_vote_batch, own_vote_hash, bytes({0x74}));
  storage_shim_commit_batch(std::move(own_vote_batch), false);
  PbftManagerLifecycleTransitionRequest request{};
  request.kind = 0;
  request.target_period = 1;
  request.target_round = 7;
  auto result = pbft_manager_runtime_execute_lifecycle_transition(*runtime, request);

  auto pbft_queries = pbftQueries(storage);
  EXPECT_EQ(result.status, kPbftManagerTransitionStorageApplied);
  EXPECT_TRUE(result.error_code.empty());
  EXPECT_EQ(result.snapshot.round, 7u);
  EXPECT_EQ(result.snapshot.step, 1u);
  EXPECT_EQ(pbft_queries->get_pbft_mgr_field(0), 7u);
  EXPECT_EQ(pbft_queries->get_pbft_mgr_field(1), 1u);
  EXPECT_FALSE(pbft_queries->get_pbft_mgr_status(2));
  EXPECT_FALSE(pbft_queries->get_pbft_mgr_status(3));
  auto vote_queries = voteQueries(storage);
  EXPECT_TRUE(vote_queries->get_own_verified_votes().empty());
}
