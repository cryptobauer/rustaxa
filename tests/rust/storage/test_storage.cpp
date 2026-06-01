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

  static rust::Vec<uint8_t> one_byte_key(uint8_t v) {
    rust::Vec<uint8_t> out;
    out.push_back(v);
    return out;
  }

  static rust::Vec<uint8_t> u64_le(uint64_t value) {
    rust::Vec<uint8_t> out;
    out.reserve(8);
    for (size_t i = 0; i < 8; ++i) {
      out.push_back(static_cast<uint8_t>((value >> (8 * i)) & 0xFF));
    }
    return out;
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

  std::filesystem::path test_dir;
};

constexpr uint8_t kPbftVotePersistenceApplied = 0;
constexpr uint8_t kPbftVotePersistenceRejected = 1;

TEST_F(StorageTest, CreateStorage) {
  auto storage = create_storage(test_dir.string());
  // rust::Box cannot be null, so this is effectively a constructor smoke test
  SUCCEED();
}

TEST_F(StorageTest, BatchPutCommitAndReadBackStatusField) {
  constexpr uint8_t kStatusColumn = 8;
  auto storage = create_storage(test_dir.string());

  auto batch_id = storage->create_write_batch();
  storage->batch_put(batch_id, kStatusColumn, one_byte_key(0), u64_le(123));
  storage->commit_write_batch(batch_id, false);

  EXPECT_EQ(storage->get_status_field(0), 123u);
}

TEST_F(StorageTest, DroppedBatchDoesNotPersistWrites) {
  constexpr uint8_t kStatusColumn = 8;
  auto storage = create_storage(test_dir.string());

  auto batch_id = storage->create_write_batch();
  storage->batch_put(batch_id, kStatusColumn, one_byte_key(1), u64_le(77));
  storage->drop_write_batch(batch_id);

  EXPECT_EQ(storage->get_status_field(1), 0u);
}

TEST_F(StorageTest, BatchDeleteRemovesStatusFieldValue) {
  constexpr uint8_t kStatusColumn = 8;
  auto storage = create_storage(test_dir.string());

  storage->save_status_field(2, 55);
  EXPECT_EQ(storage->get_status_field(2), 55u);

  auto batch_id = storage->create_write_batch();
  storage->batch_delete(batch_id, kStatusColumn, one_byte_key(2));
  storage->commit_write_batch(batch_id, false);

  EXPECT_EQ(storage->get_status_field(2), 0u);
}

TEST_F(StorageTest, UnknownBatchIdThrows) {
  constexpr uint8_t kStatusColumn = 8;
  auto storage = create_storage(test_dir.string());

  EXPECT_THROW(storage->batch_put(999999, kStatusColumn, one_byte_key(0), u64_le(1)), std::exception);
  EXPECT_THROW(storage->commit_write_batch(999999, false), std::exception);
}

TEST_F(StorageTest, DropBatchIsIdempotent) {
  auto storage = create_storage(test_dir.string());
  auto batch_id = storage->create_write_batch();

  EXPECT_NO_THROW(storage->drop_write_batch(batch_id));
  EXPECT_NO_THROW(storage->drop_write_batch(batch_id));
}

TEST_F(StorageTest, MissingDagBlockReturnsEmptyPayload) {
  auto storage = create_storage(test_dir.string());
  const auto hash = h256(0x11);
  auto value = storage->get_dag_block(hash);
  EXPECT_TRUE(value.empty());
}

TEST_F(StorageTest, DagBlockPeriodLookupReflectsFoundState) {
  auto storage = create_storage(test_dir.string());
  const auto missing = h256(0x22);
  auto lookup = storage->get_dag_block_period_lookup(missing);
  EXPECT_FALSE(lookup.found);

  const auto existing = h256(0x33);
  storage->save_dag_block_period(existing, 7, 4);
  auto found = storage->get_dag_block_period_lookup(existing);
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

  auto reward_votes = storage->get_reward_votes();
  ASSERT_EQ(reward_votes.size(), 1u);
  EXPECT_EQ(to_std_vec(reward_votes[0].data), std::vector<uint8_t>({0x71}));

  auto two_t_plus_one_votes = storage->get_all_two_t_plus_one_votes();
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
  EXPECT_TRUE(storage->get_all_two_t_plus_one_votes().empty());
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
  EXPECT_TRUE(storage->get_all_two_t_plus_one_votes().empty());
}

TEST_F(StorageTest, AppendClearOwnVerifiedVotesWaitsForCallerBatchCommit) {
  auto storage = create_storage(test_dir.string());
  auto own_vote_hash = h256(0x66);
  storage->save_own_verified_vote(own_vote_hash, bytes({0x72}));
  ASSERT_EQ(storage->get_own_verified_votes().size(), 1u);

  rust::Vec<PbftFinalizationHash> vote_hashes;
  vote_hashes.push_back(PbftFinalizationHash{own_vote_hash});
  auto batch_id = storage->create_write_batch();
  auto result = storage->append_clear_own_verified_votes(batch_id, std::move(vote_hashes));
  EXPECT_EQ(result.status, kPbftVotePersistenceApplied);
  EXPECT_EQ(result.applied_writes, 1u);
  EXPECT_EQ(storage->get_own_verified_votes().size(), 1u);

  storage->commit_write_batch(batch_id, false);
  EXPECT_TRUE(storage->get_own_verified_votes().empty());
}

TEST_F(StorageTest, AppendClearOwnVerifiedVotesDropLeavesVotes) {
  auto storage = create_storage(test_dir.string());
  auto own_vote_hash = h256(0x77);
  storage->save_own_verified_vote(own_vote_hash, bytes({0x73}));

  rust::Vec<PbftFinalizationHash> vote_hashes;
  vote_hashes.push_back(PbftFinalizationHash{own_vote_hash});
  auto batch_id = storage->create_write_batch();
  auto result = storage->append_clear_own_verified_votes(batch_id, std::move(vote_hashes));
  EXPECT_EQ(result.status, kPbftVotePersistenceApplied);

  storage->drop_write_batch(batch_id);
  ASSERT_EQ(storage->get_own_verified_votes().size(), 1u);
  EXPECT_EQ(to_std_vec(storage->get_own_verified_votes()[0].data), std::vector<uint8_t>({0x73}));
}

TEST_F(StorageTest, AppendClearOwnVerifiedVotesRejectsUnknownBatch) {
  auto storage = create_storage(test_dir.string());

  rust::Vec<PbftFinalizationHash> vote_hashes;
  vote_hashes.push_back(PbftFinalizationHash{h256(0x88)});
  auto result = storage->append_clear_own_verified_votes(999999, std::move(vote_hashes));
  EXPECT_EQ(result.status, kPbftVotePersistenceRejected);
  EXPECT_FALSE(result.error_code.empty());
}
