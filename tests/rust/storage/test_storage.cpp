#include <gtest/gtest.h>

#include <cstdint>
#include <filesystem>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

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

  std::filesystem::path test_dir;
};

constexpr uint8_t kPbftVotePersistenceApplied = 0;
constexpr uint8_t kPbftVotePersistenceRejected = 1;
constexpr uint8_t kPbftManagerTransitionStorageApplied = 0;

PbftManagerStartupFact makePbftManagerStartupFact() {
  PbftManagerStartupFact fact{};
  fact.current_period = 10;
  fact.cacti_active_at_chain_size = false;
  fact.genesis_lambda_ms = 100;
  fact.cacti_lambda_max_ms = 1500;
  fact.cacti_lambda_default_ms = 500;
  return fact;
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
  storage->save_dag_block_period(existing, 7, 4);
  auto found = dagQueries(storage)->get_dag_block_period_lookup(existing);
  EXPECT_TRUE(found.found);
  EXPECT_EQ(found.period, 7u);
  EXPECT_EQ(found.position, 4u);
}

TEST_F(StorageTest, PersistPbftVoteProgressGroupsRewardAndTwoTPlusOneWrites) {
  auto storage = create_storage(test_dir.string());

  PbftVoteProgressPersistenceWrite write{};
  write.has_extra_reward_vote = true;
  write.extra_reward_vote.hash = h256(0x44);
  write.extra_reward_vote.vote_rlp = bytes({0x71});
  write.has_two_t_plus_one_bundle = true;
  write.two_t_plus_one_bundle.kind = 0;
  write.two_t_plus_one_bundle.period = 10;
  write.two_t_plus_one_bundle.round = 2;
  write.two_t_plus_one_bundle.step = 3;
  write.two_t_plus_one_bundle.block_hash = h256(0x55);
  write.two_t_plus_one_bundle.votes_bundle_rlp = bytes({0xC2, 0x01, 0x02});

  auto result = storage->persist_pbft_vote_progress(write);
  EXPECT_EQ(result.status, kPbftVotePersistenceApplied);
  EXPECT_EQ(result.applied_writes, 2u);
  EXPECT_TRUE(result.error_code.empty());

  auto vote_queries = voteQueries(storage);
  auto reward_votes = vote_queries->get_reward_votes();
  ASSERT_EQ(reward_votes.size(), 1u);
  EXPECT_EQ(to_std_vec(reward_votes[0].data), std::vector<uint8_t>({0x71}));

  auto two_t_plus_one_votes = vote_queries->get_all_two_t_plus_one_votes();
  ASSERT_EQ(two_t_plus_one_votes.size(), 2u);
  EXPECT_EQ(to_std_vec(two_t_plus_one_votes[0].data), std::vector<uint8_t>({0x01}));
  EXPECT_EQ(to_std_vec(two_t_plus_one_votes[1].data), std::vector<uint8_t>({0x02}));
}

TEST_F(StorageTest, PersistPbftVoteProgressRejectsInvalidTwoTPlusOneKind) {
  auto storage = create_storage(test_dir.string());

  PbftVoteProgressPersistenceWrite write{};
  write.has_two_t_plus_one_bundle = true;
  write.two_t_plus_one_bundle.kind = 99;
  write.two_t_plus_one_bundle.votes_bundle_rlp = bytes({0xC1, 0x01});

  auto result = storage->persist_pbft_vote_progress(write);
  EXPECT_EQ(result.status, kPbftVotePersistenceRejected);
  EXPECT_FALSE(result.error_code.empty());
  auto vote_queries = voteQueries(storage);
  EXPECT_TRUE(vote_queries->get_all_two_t_plus_one_votes().empty());
}

TEST_F(StorageTest, PersistPbftVoteProgressRejectsMalformedTwoTPlusOneBundle) {
  auto storage = create_storage(test_dir.string());

  PbftVoteProgressPersistenceWrite write{};
  write.has_two_t_plus_one_bundle = true;
  write.two_t_plus_one_bundle.kind = 0;
  write.two_t_plus_one_bundle.votes_bundle_rlp = bytes({0x01});

  auto result = storage->persist_pbft_vote_progress(write);
  EXPECT_EQ(result.status, kPbftVotePersistenceRejected);
  EXPECT_FALSE(result.error_code.empty());
  auto vote_queries = voteQueries(storage);
  EXPECT_TRUE(vote_queries->get_all_two_t_plus_one_votes().empty());
}

TEST_F(StorageTest, ClearOwnVerifiedVotesCommitsRustOwnedBatch) {
  auto storage = create_storage(test_dir.string());
  auto own_vote_hash = h256(0x66);
  auto seed_batch = create_storage_shim_batch(*storage);
  storage_shim_save_own_verified_vote(*seed_batch, own_vote_hash, bytes({0x72}));
  storage_shim_commit_batch(std::move(seed_batch), false);
  auto vote_queries = voteQueries(storage);
  ASSERT_EQ(vote_queries->get_own_verified_votes().size(), 1u);

  rust::Vec<PbftFinalizationHash> vote_hashes;
  vote_hashes.push_back(PbftFinalizationHash{own_vote_hash});
  auto result = storage->clear_own_verified_votes(std::move(vote_hashes));
  EXPECT_EQ(result.status, kPbftVotePersistenceApplied);
  EXPECT_EQ(result.applied_writes, 1u);
  EXPECT_TRUE(vote_queries->get_own_verified_votes().empty());
}

TEST_F(StorageTest, ClearOwnVerifiedVotesTreatsMissingVotesAsNoOpDeletes) {
  auto storage = create_storage(test_dir.string());

  rust::Vec<PbftFinalizationHash> vote_hashes;
  vote_hashes.push_back(PbftFinalizationHash{h256(0x77)});
  auto result = storage->clear_own_verified_votes(std::move(vote_hashes));
  EXPECT_EQ(result.status, kPbftVotePersistenceApplied);
  EXPECT_EQ(result.applied_writes, 1u);
  auto vote_queries = voteQueries(storage);
  EXPECT_TRUE(vote_queries->get_own_verified_votes().empty());
}

TEST_F(StorageTest, ApplyPbftManagerTransitionStorageCommitsCursorStatusesAndOwnVoteCleanup) {
  auto storage = create_storage(test_dir.string());
  const auto own_vote_hash = h256(0x99);

  storage->save_pbft_mgr_field(0, 1);
  storage->save_pbft_mgr_field(1, 1);
  storage->save_pbft_mgr_status(2, true);
  storage->save_pbft_mgr_status(3, true);
  storage->save_cert_voted_block_in_round(2, bytes({0xC0}));
  auto seed_batch = create_storage_shim_batch(*storage);
  storage_shim_save_own_verified_vote(*seed_batch, own_vote_hash, bytes({0x74}));
  storage_shim_commit_batch(std::move(seed_batch), false);

  PbftManagerTransitionPlan plan{};
  plan.status = 0;
  plan.new_round = 7;
  plan.new_step = 4;
  plan.persist_round = true;
  plan.persist_step = true;
  plan.reset_next_voted_statuses = true;
  plan.remove_cert_voted_block = true;
  plan.clear_own_votes = true;

  rust::Vec<PbftFinalizationHash> own_vote_hashes;
  own_vote_hashes.push_back(PbftFinalizationHash{own_vote_hash});
  auto runtime = create_pbft_manager_runtime_from_storage(*storage, makePbftManagerStartupFact());
  auto result = pbft_manager_runtime_apply_transition_storage_write(*runtime, plan, std::move(own_vote_hashes));

  auto pbft_queries = pbftQueries(storage);
  EXPECT_EQ(result.status, kPbftManagerTransitionStorageApplied);
  EXPECT_EQ(result.applied_writes, 6u);
  EXPECT_TRUE(result.error_code.empty());
  EXPECT_EQ(result.snapshot.round, 7u);
  EXPECT_EQ(result.snapshot.step, 4u);
  EXPECT_EQ(pbft_queries->get_pbft_mgr_field(0), 7u);
  EXPECT_EQ(pbft_queries->get_pbft_mgr_field(1), 4u);
  EXPECT_FALSE(pbft_queries->get_pbft_mgr_status(2));
  EXPECT_FALSE(pbft_queries->get_pbft_mgr_status(3));
  EXPECT_TRUE(pbft_queries->get_cert_voted_block_in_round().empty());
  auto vote_queries = voteQueries(storage);
  EXPECT_TRUE(vote_queries->get_own_verified_votes().empty());
}
