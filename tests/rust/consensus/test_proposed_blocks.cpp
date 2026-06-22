#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <vector>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

class RustProposedBlocksTest : public ::testing::Test {
 protected:
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
  auto proposed_blocks = create_proposed_blocks_index();

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
}

TEST_F(RustProposedBlocksTest, CleanupCandidatesAndRemovePeriodMatchLegacyBehavior) {
  auto proposed_blocks = create_proposed_blocks_index();

  proposed_blocks->proposed_blocks_push(1, h256(0x21), h256(0xa1), bytes({0x01}));
  proposed_blocks->proposed_blocks_push(1, h256(0x22), h256(0xa2), bytes({0x02}));
  proposed_blocks->proposed_blocks_push(2, h256(0x31), h256(0xa3), bytes({0x03}));
  proposed_blocks->proposed_blocks_push(3, h256(0x41), h256(0xa4), bytes({0x04}));

  auto cleanup = proposed_blocks->proposed_blocks_cleanup_candidates(3);
  ASSERT_EQ(cleanup.size(), 2);
  EXPECT_EQ(cleanup[0].period, 1u);
  EXPECT_EQ(cleanup[0].block_hashes.size(), 2);
  EXPECT_EQ(cleanup[1].period, 2u);
  EXPECT_EQ(cleanup[1].block_hashes.size(), 1);

  for (const auto& removed : cleanup) {
    proposed_blocks->proposed_blocks_remove_period(removed.period);
  }

  EXPECT_FALSE(proposed_blocks->proposed_blocks_contains(1, h256(0x21)));
  EXPECT_FALSE(proposed_blocks->proposed_blocks_contains(2, h256(0x31)));
  EXPECT_TRUE(proposed_blocks->proposed_blocks_contains(3, h256(0x41)));

  auto grouped = proposed_blocks->proposed_blocks_snapshot();
  ASSERT_EQ(grouped.size(), 1);
  EXPECT_EQ(grouped[0].period, 3u);
  ASSERT_EQ(grouped[0].block_hashes.size(), 1);
  EXPECT_EQ(grouped[0].block_hashes[0].hash, h256(0x41));
}

TEST_F(RustProposedBlocksTest, MarkValidThrowsForMissingBlock) {
  auto proposed_blocks = create_proposed_blocks_index();

  EXPECT_THROW(proposed_blocks->proposed_blocks_mark_valid(9, h256(0x90)), std::exception);
}
