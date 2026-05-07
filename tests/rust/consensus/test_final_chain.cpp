#include <gtest/gtest.h>

#include <algorithm>
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

  static rust::Vec<uint8_t> get_validator_input(std::array<uint8_t, 20> validator_address) {
    rust::Vec<uint8_t> input;
    input.push_back(0x19);
    input.push_back(0x04);
    input.push_back(0xbb);
    input.push_back(0x2e);
    for (auto i = 0; i < 12; ++i) {
      input.push_back(0);
    }
    for (const auto byte : validator_address) {
      input.push_back(byte);
    }
    return input;
  }

  static uint64_t abi_word_u64(const rust::Vec<uint8_t>& data, size_t offset) {
    uint64_t value = 0;
    for (auto i = offset + 24; i < offset + 32; ++i) {
      value = (value << 8) | data[i];
    }
    return value;
  }

  static std::array<uint8_t, 32> abi_address_word(std::array<uint8_t, 20> address) {
    std::array<uint8_t, 32> word{};
    std::copy(address.begin(), address.end(), word.begin() + 12);
    return word;
  }

  static std::string abi_string_at(const rust::Vec<uint8_t>& data, size_t tuple_start, size_t offset) {
    const auto tail_start = tuple_start + offset;
    const auto size = abi_word_u64(data, tail_start);
    return std::string(data.begin() + tail_start + 32, data.begin() + tail_start + 32 + size);
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
    validator.owner = address(0x11);
    validator.vrf_key = vrf_key(0xA0);
    validator.commission = 12;
    validator.description = rust::String("bridge validator metadata");
    validator.endpoint = rust::String("https://validator.example");
    validator.total_stake = u64_be(10000);
    validators.push_back(std::move(validator));
    return validators;
  }

  static FinalChainCall dpos_call(uint64_t block_number, rust::Vec<uint8_t> input) {
    FinalChainCall call;
    call.block_number = block_number;
    call.sender = {};
    call.receiver_found = true;
    call.receiver = {};
    call.receiver[19] = 0xfe;
    call.value = {};
    call.gas_price = {};
    call.gas_limit = 1'000'000;
    call.input = std::move(input);
    return call;
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

TEST_F(RustFinalChainTest, DposCallReturnsGenesisValidatorMetadata) {
  const auto validator_address = address(0x10);
  const auto owner = address(0x11);
  auto storage = create_storage(test_dir.string());
  auto final_chain = create_final_chain(*storage, 0, 0, genesis_accounts(), genesis_validators(validator_address),
                                        genesis_dpos_config());

  auto outcome = final_chain->call(dpos_call(0, get_validator_input(validator_address)));
  const auto owner_word = abi_address_word(owner);

  ASSERT_EQ(std::string(outcome.code_err), "");
  ASSERT_EQ(std::string(outcome.consensus_err), "");
  ASSERT_EQ(outcome.code_retval.size(), 416u);
  EXPECT_EQ(abi_word_u64(outcome.code_retval, 0), 32u);
  EXPECT_EQ(abi_word_u64(outcome.code_retval, 32), 10'000u);
  EXPECT_EQ(abi_word_u64(outcome.code_retval, 64), 0u);
  EXPECT_EQ(abi_word_u64(outcome.code_retval, 96), 12u);
  EXPECT_EQ(std::vector<uint8_t>(outcome.code_retval.begin() + 192, outcome.code_retval.begin() + 224),
            std::vector<uint8_t>(owner_word.begin(), owner_word.end()));
  EXPECT_EQ(abi_word_u64(outcome.code_retval, 224), 256u);
  EXPECT_EQ(abi_word_u64(outcome.code_retval, 256), 320u);
  EXPECT_EQ(abi_string_at(outcome.code_retval, 32, 256), "bridge validator metadata");
  EXPECT_EQ(abi_string_at(outcome.code_retval, 32, 320), "https://validator.example");
}
