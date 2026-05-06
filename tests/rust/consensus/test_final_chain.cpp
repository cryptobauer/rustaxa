#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <filesystem>
#include <string>
#include <utility>
#include <vector>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

class RustFinalChainTest : public ::testing::Test {
 protected:
  void SetUp() override {
    test_dir = std::filesystem::temp_directory_path() / "rustaxa_consensus_final_chain_test";
    if (std::filesystem::exists(test_dir)) {
      std::filesystem::remove_all(test_dir);
    }
  }

  void TearDown() override {
    if (std::filesystem::exists(test_dir)) {
      std::filesystem::remove_all(test_dir);
    }
  }

  static std::array<uint8_t, 20> address(uint8_t value) {
    std::array<uint8_t, 20> out{};
    out.fill(value);
    return out;
  }

  static std::array<uint8_t, 32> vrf_key(uint8_t value) {
    std::array<uint8_t, 32> out{};
    out.fill(value);
    return out;
  }

  static rust::Vec<uint8_t> u64_be(uint64_t value) {
    rust::Vec<uint8_t> out;
    if (value == 0) {
      return out;
    }

    bool seen_non_zero = false;
    for (int shift = 56; shift >= 0; shift -= 8) {
      const auto byte = static_cast<uint8_t>((value >> shift) & 0xFF);
      if (byte != 0 || seen_non_zero) {
        out.push_back(byte);
        seen_non_zero = true;
      }
    }
    return out;
  }

  static std::vector<uint8_t> bytes(const rust::Vec<uint8_t>& value) {
    return std::vector<uint8_t>(value.begin(), value.end());
  }

  static rust::Vec<GenesisAccount> genesis_accounts() { return {}; }

  static GenesisDposConfig genesis_dpos_config() {
    GenesisDposConfig config;
    config.eligibility_balance_threshold = u64_be(1000);
    config.vote_eligibility_balance_step = u64_be(1000);
    config.validator_maximum_stake = u64_be(30000);
    return config;
  }

  static rust::Vec<GenesisValidator> genesis_validators(std::array<uint8_t, 20> validator_address) {
    rust::Vec<GenesisValidator> validators;
    GenesisValidator validator;
    validator.address = validator_address;
    validator.vrf_key = vrf_key(0xA0);
    validator.total_stake = u64_be(10000);
    validators.push_back(std::move(validator));
    return validators;
  }

  std::filesystem::path test_dir;
};

TEST_F(RustFinalChainTest, DposQueriesUseGenesisSnapshotAtBlockZero) {
  const auto validator_address = address(0x10);
  const auto unknown_address = address(0x20);
  auto storage = create_storage(test_dir.string());
  auto final_chain = create_final_chain(*storage, 0, 0, genesis_accounts(), genesis_validators(validator_address),
                                        genesis_dpos_config());

  EXPECT_EQ(final_chain->get_dpos_eligible_total_vote_count(0), 10u);
  EXPECT_EQ(final_chain->get_dpos_eligible_vote_count(0, validator_address), 10u);
  EXPECT_TRUE(final_chain->get_dpos_is_eligible(0, validator_address));
  EXPECT_EQ(final_chain->get_dpos_eligible_vote_count(0, unknown_address), 0u);
  EXPECT_FALSE(final_chain->get_dpos_is_eligible(0, unknown_address));

  const auto stakes = final_chain->get_dpos_validators_total_stakes(0);
  ASSERT_EQ(stakes.size(), 1u);
  EXPECT_EQ(stakes[0].address, validator_address);
  EXPECT_EQ(bytes(stakes[0].stake), bytes(u64_be(10000)));

  const auto vote_counts = final_chain->get_dpos_validators_eligible_vote_counts(0);
  ASSERT_EQ(vote_counts.size(), 1u);
  EXPECT_EQ(vote_counts[0].address, validator_address);
  EXPECT_EQ(vote_counts[0].vote_count, 10u);
}

TEST_F(RustFinalChainTest, DposQueriesRejectMissingNonGenesisSnapshot) {
  const auto validator_address = address(0x10);
  auto storage = create_storage(test_dir.string());
  auto final_chain = create_final_chain(*storage, 0, 0, genesis_accounts(), genesis_validators(validator_address),
                                        genesis_dpos_config());

  EXPECT_THROW(final_chain->get_dpos_eligible_total_vote_count(1), std::exception);
  EXPECT_THROW(final_chain->get_dpos_eligible_vote_count(1, validator_address), std::exception);
  EXPECT_THROW(final_chain->get_dpos_is_eligible(1, validator_address), std::exception);
  EXPECT_THROW(final_chain->get_dpos_validators_total_stakes(1), std::exception);
  EXPECT_THROW(final_chain->get_dpos_validators_eligible_vote_counts(1), std::exception);
}
