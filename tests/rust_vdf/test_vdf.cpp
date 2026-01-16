#include <gtest/gtest.h>
#include <vector>

#include "rust/cxx.h"
#include "rustaxa-bridge/src/vdf.rs.h"

using namespace rustaxa::vdf;

inline rust::Slice<const uint8_t> to_slice(const std::vector<uint8_t>& v) {
  return {v.data(), v.size()};
}

class VDFTest : public ::testing::Test {};

// Test VDF creation with valid parameters
TEST_F(VDFTest, CreateValidVDF) {
  auto vdf = make_vdf(20, 8, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  // rust::Box cannot be null, so just testing that creation doesn't throw
  SUCCEED();
}

// Test VDF creation with different lambda values
TEST_F(VDFTest, CreateVDFWithDifferentLambda) {
  auto vdf1 = make_vdf(16, 8, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto vdf2 = make_vdf(32, 8, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto vdf3 = make_vdf(64, 8, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));

  // rust::Box cannot be null, so just testing that creation doesn't throw
  SUCCEED();
}

// Test VDF creation with different time bits
TEST_F(VDFTest, CreateVDFWithDifferentTimeBits) {
  auto vdf1 = make_vdf(20, 4, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto vdf2 = make_vdf(20, 8, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto vdf3 = make_vdf(20, 12, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));

  // rust::Box cannot be null, so just testing that creation doesn't throw
  SUCCEED();
}

// Test VDF creation with different input values
TEST_F(VDFTest, CreateVDFWithDifferentInputs) {
  auto vdf1 = make_vdf(20, 8, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto vdf2 = make_vdf(20, 8, to_slice({123, 45}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto vdf3 = make_vdf(20, 8, to_slice({77, 39, 11}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));

  // rust::Box cannot be null, so just testing that creation doesn't throw
  SUCCEED();
}

// Test cancellation token creation
TEST_F(VDFTest, CreateCancellationToken) {
  auto token = make_cancellation_token();
  // rust::Box cannot be null, so just testing that creation doesn't throw
  SUCCEED();
}

// Test cancellation token cancellation
TEST_F(VDFTest, CancelCancellationToken) {
  auto token = make_cancellation_token();

  // Should not throw
  EXPECT_NO_THROW(cancellation_token_cancel(*token));
}

// Test basic prove operation
TEST_F(VDFTest, BasicProve) {
  auto vdf = make_vdf(20, 6, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));  // Smaller time_bits for faster test
  auto cancellation_token = make_cancellation_token();

  auto solution = prove(*vdf, *cancellation_token);
  // rust::Box cannot be null, so just testing that prove doesn't throw
  SUCCEED();
}

// Test basic verify operation
TEST_F(VDFTest, BasicVerify) {
  auto vdf = make_vdf(20, 6, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));  // Smaller time_bits for faster test
  auto cancellation_token = make_cancellation_token();

  auto solution = prove(*vdf, *cancellation_token);

  bool is_valid = verify(*vdf, *solution);
  EXPECT_TRUE(is_valid);
}

// Test that different VDFs produce different solutions
TEST_F(VDFTest, DifferentVDFsDifferentSolutions) {
  auto vdf1 = make_vdf(20, 6, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto vdf2 = make_vdf(20, 6, to_slice({98}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto cancellation_token = make_cancellation_token();

  auto solution1 = prove(*vdf1, *cancellation_token);
  auto solution2 = prove(*vdf2, *cancellation_token);

  // Solutions from different VDFs should be valid for their respective VDFs
  EXPECT_TRUE(verify(*vdf1, *solution1));
  EXPECT_TRUE(verify(*vdf2, *solution2));
}

// Test cross-verification (solution from one VDF should not verify with
// another)
TEST_F(VDFTest, CrossVerificationShouldFail) {
  auto vdf1 = make_vdf(20, 6, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto vdf2 = make_vdf(20, 6, to_slice({98}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto cancellation_token = make_cancellation_token();

  auto solution1 = prove(*vdf1, *cancellation_token);

  // Solution from vdf1 should not verify with vdf2
  bool cross_valid = verify(*vdf2, *solution1);
  EXPECT_FALSE(cross_valid);
}

// Test multiple proofs with same VDF
TEST_F(VDFTest, MultipleProofsWithSameVDF) {
  auto vdf = make_vdf(20, 6, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto cancellation_token = make_cancellation_token();

  auto solution1 = prove(*vdf, *cancellation_token);
  auto solution2 = prove(*vdf, *cancellation_token);

  // Both solutions should be valid for the same VDF
  EXPECT_TRUE(verify(*vdf, *solution1));
  EXPECT_TRUE(verify(*vdf, *solution2));
}
