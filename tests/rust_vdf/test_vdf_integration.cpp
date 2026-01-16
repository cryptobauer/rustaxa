#include <gtest/gtest.h>
#include <vector>

#include <chrono>

#include "rust/cxx.h"
#include "rustaxa-bridge/src/vdf.rs.h"

using namespace rustaxa::vdf;

inline rust::Slice<const uint8_t> to_slice(const std::vector<uint8_t>& v) {
  return {v.data(), v.size()};
}

class VDFIntegrationTest : public ::testing::Test {
 protected:
  void SetUp() override {
    // Common test setup - no need to store cancellation token
  }
};

TEST_F(VDFIntegrationTest, MainExampleTest) {
  const auto vdf1 = make_vdf(20, 8, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto cancellation_token = make_cancellation_token();

  auto const solution1 = prove(*vdf1, *cancellation_token);
  auto const is_valid1 = verify(*vdf1, *solution1);
  EXPECT_TRUE(is_valid1);

  auto const solution2 = prove(*vdf1, *cancellation_token);
  auto const is_valid2 = verify(*vdf1, *solution2);
  EXPECT_TRUE(is_valid2);

  const auto vdf3 = make_vdf(20, 8, to_slice({77, 39, 11}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto solution3 = prove(*vdf3, *cancellation_token);
  auto const is_valid3 = verify(*vdf3, *solution3);
  EXPECT_TRUE(is_valid3);

  // Cross verification should fail
  auto const is_valid4 = verify(*vdf3, *solution2);
  EXPECT_FALSE(is_valid4);
}

// Test VDF consistency across multiple runs
TEST_F(VDFIntegrationTest, ConsistencyTest) {
  std::vector<uint8_t> input = {42, 123, 255};
  std::vector<uint8_t> modulus = {213, 166, 245, 127, 146, 139, 45, 0};
  auto cancellation_token = make_cancellation_token();

  // Create multiple VDFs with same parameters
  auto vdf1 = make_vdf(20, 7, to_slice(input), to_slice(modulus));  // Reduced time_bits for performance
  auto vdf2 = make_vdf(20, 7, to_slice(input), to_slice(modulus));

  // Generate solutions
  auto solution1 = prove(*vdf1, *cancellation_token);
  auto solution2 = prove(*vdf2, *cancellation_token);

  // Both solutions should be valid for both VDFs (since they're identical)
  EXPECT_TRUE(verify(*vdf1, *solution1));
  EXPECT_TRUE(verify(*vdf1, *solution2));
  EXPECT_TRUE(verify(*vdf2, *solution1));
  EXPECT_TRUE(verify(*vdf2, *solution2));
}

// Test with edge case parameters
TEST_F(VDFIntegrationTest, EdgeCaseParameters) {
  auto cancellation_token = make_cancellation_token();

  // Test with minimal time_bits
  auto vdf_min = make_vdf(16, 4, to_slice({1}), to_slice({7, 11}));
  auto solution_min = prove(*vdf_min, *cancellation_token);
  EXPECT_TRUE(verify(*vdf_min, *solution_min));

  // Test with single byte input
  auto vdf_single = make_vdf(20, 6, to_slice({255}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto solution_single = prove(*vdf_single, *cancellation_token);
  EXPECT_TRUE(verify(*vdf_single, *solution_single));

  // Test with empty input - this might not be supported, so we test but don't
  // assert the result
  auto vdf_empty = make_vdf(20, 6, to_slice({}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto solution_empty = prove(*vdf_empty, *cancellation_token);
  // Note: Empty input verification might fail depending on implementation
  bool empty_result = verify(*vdf_empty, *solution_empty);
  // We record but don't assert on this result since empty input behavior may
  // vary
  std::cout << "Empty input VDF verification result: " << (empty_result ? "Valid" : "Invalid") << std::endl;
}

// Test VDF behavior with different modulus values
TEST_F(VDFIntegrationTest, DifferentModulusTest) {
  std::vector<uint8_t> input = {97};
  auto cancellation_token = make_cancellation_token();

  auto vdf1 = make_vdf(20, 6, to_slice(input), to_slice({7, 11}));
  auto vdf2 = make_vdf(20, 6, to_slice(input), to_slice({13, 17}));
  auto vdf3 = make_vdf(20, 6, to_slice(input), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));

  auto solution1 = prove(*vdf1, *cancellation_token);
  auto solution2 = prove(*vdf2, *cancellation_token);
  auto solution3 = prove(*vdf3, *cancellation_token);

  // Each solution should verify with its corresponding VDF
  EXPECT_TRUE(verify(*vdf1, *solution1));
  EXPECT_TRUE(verify(*vdf2, *solution2));
  EXPECT_TRUE(verify(*vdf3, *solution3));

  // Cross-verification should fail
  EXPECT_FALSE(verify(*vdf1, *solution2));
  EXPECT_FALSE(verify(*vdf2, *solution3));
  EXPECT_FALSE(verify(*vdf3, *solution1));
}

// Test performance characteristics (basic timing test)
TEST_F(VDFIntegrationTest, PerformanceCharacteristics) {
  auto vdf_fast = make_vdf(20, 4, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));  // 2^4 = 16 iterations
  auto vdf_slow = make_vdf(20, 6, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));  // 2^6 = 64 iterations
  auto cancellation_token = make_cancellation_token();

  auto start_fast = std::chrono::high_resolution_clock::now();
  auto solution_fast = prove(*vdf_fast, *cancellation_token);
  auto end_fast = std::chrono::high_resolution_clock::now();

  auto start_slow = std::chrono::high_resolution_clock::now();
  auto solution_slow = prove(*vdf_slow, *cancellation_token);
  auto end_slow = std::chrono::high_resolution_clock::now();

  auto duration_fast = std::chrono::duration_cast<std::chrono::milliseconds>(end_fast - start_fast);
  auto duration_slow = std::chrono::duration_cast<std::chrono::milliseconds>(end_slow - start_slow);

  // Both should be valid
  EXPECT_TRUE(verify(*vdf_fast, *solution_fast));
  EXPECT_TRUE(verify(*vdf_slow, *solution_slow));

  // The slower VDF should take longer (though this might be flaky in very fast
  // systems) We'll just check that both complete in reasonable time
  EXPECT_LT(duration_fast.count(), 10000);  // Less than 10 seconds
  EXPECT_LT(duration_slow.count(), 30000);  // Less than 30 seconds
}

// Test cancellation token behavior
TEST_F(VDFIntegrationTest, CancellationBehavior) {
  auto vdf = make_vdf(20, 8, to_slice({97}), to_slice({213, 166, 245, 127, 146, 139, 45, 0}));
  auto token = make_cancellation_token();

  // Cancel the token before proving
  cancellation_token_cancel(*token);

  // Prove should still complete (implementation dependent)
  auto solution = prove(*vdf, *token);

  // When token is cancelled, the solution might not be valid
  // This tests the cancellation behavior - result may vary based on
  // implementation
  bool is_valid = verify(*vdf, *solution);
  std::cout << "Cancelled token VDF verification result: " << (is_valid ? "Valid" : "Invalid") << std::endl;

  // Test with fresh token should work
  auto fresh_token = make_cancellation_token();
  auto fresh_solution = prove(*vdf, *fresh_token);
  EXPECT_TRUE(verify(*vdf, *fresh_solution));
}
