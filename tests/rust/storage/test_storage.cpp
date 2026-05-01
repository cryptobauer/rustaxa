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

  std::filesystem::path test_dir;
};

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
