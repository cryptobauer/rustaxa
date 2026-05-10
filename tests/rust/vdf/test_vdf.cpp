#include <gtest/gtest.h>
#include <array>
#include <vector>

#include "rust/cxx.h"
#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

inline rust::Slice<const uint8_t> to_slice(const std::vector<uint8_t>& v) {
  return {v.data(), v.size()};
}

inline rust::Slice<const uint8_t> to_rust_vec_slice(const rust::Vec<uint8_t>& v) {
  return {v.data(), v.size()};
}

class VDFTest : public ::testing::Test {};

const std::array<uint8_t, 64> kVrfSecretKey = {
    0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4, 0xe0,
    0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c, 0x9a, 0x0d,
    0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28, 0x7e, 0xab, 0xba,
    0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97, 0xad, 0xe4, 0x00, 0x81};

LegacySortitionParams sortition_params() {
  return LegacySortitionParams{
      .vrf_threshold_upper = 0x05ff,
      .vdf_difficulty_min = 5,
      .vdf_difficulty_max = 10,
      .vdf_difficulty_stale = 9,
      .vdf_lambda_bound = 64,
  };
}

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

TEST_F(VDFTest, LegacyVrfSortitionBridgeRoundTrip) {
  const auto vrf_input = std::vector<uint8_t>{0xa1, 0x02, 0x03};
  const auto proof = prove_legacy_vrf_sortition(kVrfSecretKey, to_slice(vrf_input), 1000);
  ASSERT_TRUE(proof.ok) << proof.error;

  const auto verified =
      verify_legacy_vrf_sortition(proof.public_key, proof.proof, to_slice(vrf_input), 1000, true);
  EXPECT_TRUE(verified.ok) << verified.error;
  EXPECT_EQ(verified.output, proof.output);
  EXPECT_EQ(verified.threshold, proof.threshold);

  const auto wrong_input = std::vector<uint8_t>{0xff};
  const auto rejected =
      verify_legacy_vrf_sortition(proof.public_key, proof.proof, to_slice(wrong_input), 1000, true);
  EXPECT_FALSE(rejected.ok);
}

TEST_F(VDFTest, LegacyVdfSortitionBridgeRoundTrip) {
  const auto vrf_input = std::vector<uint8_t>{0xa1, 0x02, 0x03};
  const auto vdf_input = std::vector<uint8_t>{0xb1, 0x04};
  auto cancellation_token = make_cancellation_token();
  const auto vrf = prove_legacy_vrf_sortition(kVrfSecretKey, to_slice(vrf_input), 1000);
  ASSERT_TRUE(vrf.ok) << vrf.error;

  const auto proof = prove_legacy_vdf_sortition(sortition_params(), kVrfSecretKey, to_slice(vrf_input),
                                                to_slice(vdf_input), 1, 1, *cancellation_token);
  ASSERT_TRUE(proof.ok) << proof.error;

  const auto payload = VdfSortitionPayload{
      .vrf_proof = proof.vrf_proof,
      .vdf_solution_proof = proof.vdf_proof,
      .vdf_solution_output = proof.vdf_output,
      .difficulty = proof.difficulty,
  };
  const auto encoded = vdf_sortition_payload_encode(payload);
  const auto verified = verify_legacy_vdf_sortition(sortition_params(), vrf.public_key, to_rust_vec_slice(encoded),
                                                    to_slice(vrf_input), to_slice(vdf_input), 1, 1);
  EXPECT_TRUE(verified.ok) << verified.error;
  EXPECT_EQ(verified.expected_difficulty, proof.difficulty);
  EXPECT_EQ(verified.vrf_threshold, proof.vrf_threshold);
  EXPECT_EQ(verified.vrf_output, proof.vrf_output);
}
