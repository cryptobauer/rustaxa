#include <gtest/gtest.h>

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

class RustProposedBlocksTest : public ::testing::Test {
 protected:
  static std::filesystem::path uniqueTempDir(const std::string& name) {
    const auto nonce = std::chrono::steady_clock::now().time_since_epoch().count();
    auto path = std::filesystem::temp_directory_path() / (name + "_" + std::to_string(nonce));
    std::filesystem::create_directories(path);
    return path;
  }

  static std::array<uint8_t, 32> h256(uint8_t last_byte) {
    std::array<uint8_t, 32> hash{};
    hash[31] = last_byte;
    return hash;
  }

  static rust::Vec<uint8_t> bytes(std::initializer_list<uint8_t> data) {
    rust::Vec<uint8_t> out;
    out.reserve(data.size());
    for (auto byte : data) {
      out.push_back(byte);
    }
    return out;
  }

  static std::vector<uint8_t> to_std(const rust::Vec<uint8_t>& data) {
    return {data.begin(), data.end()};
  }
};

TEST_F(RustProposedBlocksTest, PushGetMarkValidAndSnapshotEntries) {
  const auto test_dir = uniqueTempDir("rustaxa_proposed_blocks_bridge");
  auto storage = create_storage(test_dir.string());
  auto proposed_blocks = create_proposed_blocks_index_from_storage(*storage);

  EXPECT_TRUE(proposed_blocks->proposed_blocks_push(2, h256(0x11), h256(0x99), bytes({0xAA, 0xBB})));
  EXPECT_FALSE(proposed_blocks->proposed_blocks_push(2, h256(0x11), h256(0x88), bytes({0xCC})));

  auto lookup = proposed_blocks->proposed_blocks_get(2, h256(0x11));
  EXPECT_TRUE(lookup.found);
  EXPECT_FALSE(lookup.is_valid);
  EXPECT_EQ(lookup.pivot_hash, h256(0x99));
  EXPECT_EQ(to_std(lookup.block_rlp), std::vector<uint8_t>({0xAA, 0xBB}));
  auto metadata = proposed_blocks->proposed_blocks_metadata(2, h256(0x11));
  EXPECT_TRUE(metadata.found);
  EXPECT_FALSE(metadata.is_valid);
  EXPECT_EQ(metadata.pivot_hash, h256(0x99));

  EXPECT_TRUE(proposed_blocks->proposed_blocks_contains(2, h256(0x11)));
  EXPECT_FALSE(proposed_blocks->proposed_blocks_contains(2, h256(0x12)));

  proposed_blocks->proposed_blocks_mark_valid(2, h256(0x11));
  lookup = proposed_blocks->proposed_blocks_get(2, h256(0x11));
  EXPECT_TRUE(lookup.is_valid);
  metadata = proposed_blocks->proposed_blocks_metadata(2, h256(0x11));
  EXPECT_TRUE(metadata.is_valid);

  auto entries = proposed_blocks->proposed_blocks_snapshot_entries();
  ASSERT_EQ(entries.size(), 1);
  EXPECT_EQ(entries[0].period, 2u);
  EXPECT_EQ(entries[0].block_hash, h256(0x11));
  EXPECT_EQ(entries[0].pivot_hash, h256(0x99));
  EXPECT_EQ(to_std(entries[0].block_rlp), std::vector<uint8_t>({0xAA, 0xBB}));
  EXPECT_TRUE(entries[0].is_valid);

  std::filesystem::remove_all(test_dir);
}

TEST_F(RustProposedBlocksTest, MarkValidThrowsForMissingBlock) {
  const auto test_dir = uniqueTempDir("rustaxa_proposed_blocks_missing");
  auto storage = create_storage(test_dir.string());
  auto proposed_blocks = create_proposed_blocks_index_from_storage(*storage);

  EXPECT_THROW(proposed_blocks->proposed_blocks_mark_valid(9, h256(0x90)), std::exception);

  std::filesystem::remove_all(test_dir);
}
