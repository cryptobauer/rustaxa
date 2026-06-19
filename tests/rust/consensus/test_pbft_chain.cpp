#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <exception>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

class RustPbftChainTest : public ::testing::Test {
 protected:
  static std::array<uint8_t, 32> h256(uint8_t last_byte) {
    std::array<uint8_t, 32> hash{};
    hash[31] = last_byte;
    return hash;
  }

  static PbftChainHeadPayload head(uint64_t size, uint64_t non_empty_size, uint8_t last_block, uint8_t last_anchor) {
    PbftChainHeadPayload payload{};
    payload.head_hash = h256(0);
    payload.size = size;
    payload.non_empty_size = non_empty_size;
    payload.last_pbft_block_hash = h256(last_block);
    payload.last_non_null_anchor_hash = h256(last_anchor);
    return payload;
  }
};

TEST_F(RustPbftChainTest, UpdatesHeadStateForNonNullAndNullAnchors) {
  auto chain = create_pbft_chain(head(1, 0, 11, 0));
  EXPECT_FALSE(chain->pbft_chain_initialized_default());

  auto projected = chain->pbft_chain_project_update(h256(12), h256(99));
  EXPECT_EQ(projected.size, 2);
  EXPECT_EQ(projected.non_empty_size, 1);
  EXPECT_EQ(projected.last_pbft_block_hash, h256(12));
  EXPECT_EQ(projected.last_non_null_anchor_hash, h256(99));

  auto current = chain->pbft_chain_head();
  EXPECT_EQ(current.size, 1);
  EXPECT_EQ(current.last_pbft_block_hash, h256(11));

  current = chain->pbft_chain_update(h256(12), h256(99));
  EXPECT_EQ(current.size, 2);
  EXPECT_EQ(current.non_empty_size, 1);
  EXPECT_EQ(current.last_pbft_block_hash, h256(12));
  EXPECT_EQ(current.last_non_null_anchor_hash, h256(99));

  current = chain->pbft_chain_update(h256(13), h256(0));
  EXPECT_EQ(current.size, 3);
  EXPECT_EQ(current.non_empty_size, 1);
  EXPECT_EQ(current.last_pbft_block_hash, h256(13));
  EXPECT_EQ(current.last_non_null_anchor_hash, h256(99));
}

TEST_F(RustPbftChainTest, ProjectsLegacyJsonHeadWithoutMutatingCurrentHead) {
  auto chain = create_pbft_chain(head(4, 2, 44, 77));

  auto projected = chain->pbft_chain_project_legacy_json_head(h256(45), true);
  EXPECT_EQ(projected.size, 5);
  EXPECT_EQ(projected.non_empty_size, 3);
  EXPECT_EQ(projected.last_pbft_block_hash, h256(45));
  EXPECT_EQ(projected.last_non_null_anchor_hash, h256(77));

  auto current = chain->pbft_chain_head();
  EXPECT_EQ(current.size, 4);
  EXPECT_EQ(current.non_empty_size, 2);
  EXPECT_EQ(current.last_pbft_block_hash, h256(44));
}

TEST_F(RustPbftChainTest, ReportsPeriodAndPreviousHashValidationFailures) {
  auto chain = create_pbft_chain(head(3, 2, 33, 22));

  auto valid = chain->pbft_chain_validate_block(4, h256(33));
  EXPECT_TRUE(valid.ok);
  EXPECT_EQ(valid.code, 0);

  auto period_mismatch = chain->pbft_chain_validate_block(5, h256(33));
  EXPECT_FALSE(period_mismatch.ok);
  EXPECT_EQ(period_mismatch.code, 1);
  EXPECT_EQ(period_mismatch.expected_period, 4);
  EXPECT_EQ(period_mismatch.actual_period, 5);

  auto prev_hash_mismatch = chain->pbft_chain_validate_block(4, h256(99));
  EXPECT_FALSE(prev_hash_mismatch.ok);
  EXPECT_EQ(prev_hash_mismatch.code, 2);
  EXPECT_EQ(prev_hash_mismatch.expected_prev_hash, h256(33));
  EXPECT_EQ(prev_hash_mismatch.actual_prev_hash, h256(99));
}

TEST_F(RustPbftChainTest, RejectsImpossibleRecoveredHead) {
  EXPECT_THROW((void)create_pbft_chain(head(1, 2, 11, 0)), std::exception);
}
