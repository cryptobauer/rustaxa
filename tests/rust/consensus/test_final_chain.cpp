#include <gtest/gtest.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <filesystem>
#include <string>
#include <utility>
#include <vector>

#include "consensus_application_test.hpp"
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

  static rust::Vec<uint8_t> get_total_delegation_input(std::array<uint8_t, 20> delegator_address) {
    rust::Vec<uint8_t> input;
    input.push_back(0xfc);
    input.push_back(0x5e);
    input.push_back(0x7e);
    input.push_back(0x09);
    for (auto i = 0; i < 12; ++i) {
      input.push_back(0);
    }
    for (const auto byte : delegator_address) {
      input.push_back(byte);
    }
    return input;
  }

  static rust::Vec<uint8_t> get_delegate_input(std::array<uint8_t, 20> validator_address) {
    rust::Vec<uint8_t> input;
    input.push_back(0x5c);
    input.push_back(0x19);
    input.push_back(0xa9);
    input.push_back(0x5c);
    for (auto i = 0; i < 12; ++i) {
      input.push_back(0);
    }
    for (const auto byte : validator_address) {
      input.push_back(byte);
    }
    return input;
  }

  static rust::Vec<uint8_t> get_delegations_input(std::array<uint8_t, 20> delegator_address, uint32_t batch) {
    rust::Vec<uint8_t> input;
    input.push_back(0x8b);
    input.push_back(0x49);
    input.push_back(0xd3);
    input.push_back(0x94);
    for (auto i = 0; i < 12; ++i) {
      input.push_back(0);
    }
    for (const auto byte : delegator_address) {
      input.push_back(byte);
    }
    for (auto i = 0; i < 28; ++i) {
      input.push_back(0);
    }
    input.push_back(static_cast<uint8_t>((batch >> 24) & 0xff));
    input.push_back(static_cast<uint8_t>((batch >> 16) & 0xff));
    input.push_back(static_cast<uint8_t>((batch >> 8) & 0xff));
    input.push_back(static_cast<uint8_t>(batch & 0xff));
    return input;
  }

  static rust::Vec<uint8_t> get_validators_input(uint32_t batch) {
    rust::Vec<uint8_t> input;
    input.push_back(0x19);
    input.push_back(0xd8);
    input.push_back(0x02);
    input.push_back(0x4f);
    for (auto i = 0; i < 28; ++i) {
      input.push_back(0);
    }
    input.push_back(static_cast<uint8_t>((batch >> 24) & 0xff));
    input.push_back(static_cast<uint8_t>((batch >> 16) & 0xff));
    input.push_back(static_cast<uint8_t>((batch >> 8) & 0xff));
    input.push_back(static_cast<uint8_t>(batch & 0xff));
    return input;
  }

  static rust::Vec<uint8_t> get_validators_for_input(std::array<uint8_t, 20> owner, uint32_t batch) {
    rust::Vec<uint8_t> input;
    input.push_back(0x72);
    input.push_back(0x4a);
    input.push_back(0xc6);
    input.push_back(0xb0);
    for (auto i = 0; i < 12; ++i) {
      input.push_back(0);
    }
    for (const auto byte : owner) {
      input.push_back(byte);
    }
    for (auto i = 0; i < 28; ++i) {
      input.push_back(0);
    }
    input.push_back(static_cast<uint8_t>((batch >> 24) & 0xff));
    input.push_back(static_cast<uint8_t>((batch >> 16) & 0xff));
    input.push_back(static_cast<uint8_t>((batch >> 8) & 0xff));
    input.push_back(static_cast<uint8_t>(batch & 0xff));
    return input;
  }

  static rust::Vec<uint8_t> get_claim_rewards_input(std::array<uint8_t, 20> validator) {
    rust::Vec<uint8_t> input;
    input.push_back(0xef);
    input.push_back(0x5c);
    input.push_back(0xfb);
    input.push_back(0x8c);
    for (auto i = 0; i < 12; ++i) {
      input.push_back(0);
    }
    for (const auto byte : validator) {
      input.push_back(byte);
    }
    return input;
  }

  static rust::Vec<uint8_t> get_claim_all_rewards_input() {
    rust::Vec<uint8_t> input;
    input.push_back(0x0b);
    input.push_back(0x83);
    input.push_back(0xa7);
    input.push_back(0x27);
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

  static std::vector<std::array<uint8_t, 20>> validator_page_addresses(const rust::Vec<uint8_t>& data) {
    const auto array_start = static_cast<size_t>(abi_word_u64(data, 0));
    const auto size = static_cast<size_t>(abi_word_u64(data, array_start));
    std::vector<std::array<uint8_t, 20>> addresses;
    addresses.reserve(size);
    for (auto i = 0u; i < size; ++i) {
      const auto offset = static_cast<size_t>(abi_word_u64(data, array_start + 32 + i * 32));
      const auto payload_start = array_start + 32 + offset;
      std::array<uint8_t, 20> validator{};
      std::copy(data.begin() + payload_start + 12, data.begin() + payload_start + 32, validator.begin());
      addresses.push_back(validator);
    }
    return addresses;
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
    config.commission_change_delta = 500;
    config.commission_change_frequency = 7;
    config.dag_vdf_sortition_total_vote_count_until_period = 0;
    return config;
  }

  static FinalChainRewardsConfig default_rewards_config() {
    FinalChainRewardsConfig config;
    config.committee_size = 0;
    config.magnolia_period = 0;
    config.aspen_part_one_period = UINT64_MAX;
    config.fix_claim_all_block_num = UINT64_MAX;
    config.aspen_part_two_period = 0;
    config.max_block_author_reward_percent = 0;
    config.dag_proposers_reward_percent = 0;
    config.yield_percentage = 0;
    config.dpos_blocks_per_year = 0;
    config.dpos_delegation_locking_period = 0;
    config.cornus_period = 0;
    config.cornus_delegation_locking_period = 0;
    config.genesis_balance_sum = {};
    config.aspen_max_supply = {};
    config.aspen_generated_rewards = {};
    config.cacti_period = 0;
    config.cacti_delegation_locking_period = 0;
    config.magnolia_jail_time = 0;
    config.cacti_jail_time = 0;
    config.frequency_rules = {};
    return config;
  }

  static GenesisValidator genesis_validator(std::array<uint8_t, 20> validator_address,
                                            std::array<uint8_t, 20> owner = address(0x11),
                                            std::string description = "bridge validator metadata") {
    GenesisValidator validator;
    validator.address = validator_address;
    validator.owner = owner;
    validator.vrf_key = vrf_key(0xA0);
    validator.commission = 12;
    validator.description = rust::String(description);
    validator.endpoint = rust::String("https://validator.example");
    validator.total_stake = u64_be(10000);
    return validator;
  }

  static rust::Vec<GenesisValidator> genesis_validators(std::array<uint8_t, 20> validator_address) {
    rust::Vec<GenesisValidator> validators;
    validators.push_back(genesis_validator(validator_address));
    return validators;
  }

  static PbftServiceConfig pbft_config() {
    PbftServiceConfig config{};
    config.genesis_lambda_ms = 100;
    config.cacti_lambda_max_ms = 100;
    config.cacti_lambda_default_ms = 100;
    config.max_exponential_lambda_ms = 60'000;
    config.max_steps = 13;
    config.deadline_ms = 400;
    config.polling_interval_ms = 100;
    config.pillar_blocks_interval = 10;
    config.sync_level_size = 10;
    config.committee_size = 5;
    config.number_of_proposers = 20;
    return config;
  }

  rust::Box<BridgeConsensusApplication> create_final_chain_for_test(rust::Vec<GenesisValidator> validators) {
    return test::createConsensusApplication(test_dir, pbft_config(), 32, 10, genesis_accounts(), std::move(validators),
                                            genesis_dpos_config(), default_rewards_config());
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
  auto final_chain = create_final_chain_for_test(genesis_validators(validator_address));

  EXPECT_EQ(final_chain->get_dpos_eligible_total_vote_count(0), 10u);
  EXPECT_EQ(final_chain->get_dpos_eligible_vote_count(0, validator_address), 10u);
  EXPECT_GT(final_chain->get_dpos_eligible_vote_count(0, validator_address), 0);
  EXPECT_EQ(final_chain->get_dpos_eligible_vote_count(0, unknown_address), 0u);
  EXPECT_EQ(final_chain->get_dpos_eligible_vote_count(0, unknown_address), 0);

  const auto stakes = final_chain->get_dpos_validators_total_stakes(0);
  ASSERT_EQ(stakes.size(), 1u);
  EXPECT_EQ(stakes[0].address, validator_address);
  EXPECT_EQ(bytes(stakes[0].stake), bytes(u64_be(10000)));
  EXPECT_EQ(bytes(final_chain->get_dpos_total_amount_delegated(0)), bytes(u64_be(10000)));
  EXPECT_EQ(final_chain->get_dpos_yield(0), 0u);
  EXPECT_TRUE(final_chain->get_dpos_total_supply(0).empty());

  auto total_delegation = final_chain->call(dpos_call(0, get_total_delegation_input(validator_address)));
  ASSERT_EQ(std::string(total_delegation.code_err), "");
  ASSERT_EQ(std::string(total_delegation.consensus_err), "");
  EXPECT_EQ(abi_word_u64(total_delegation.code_retval, 0), 10'000u);

  auto delegations = final_chain->call(dpos_call(0, get_delegations_input(validator_address, 0)));
  ASSERT_EQ(std::string(delegations.code_err), "");
  ASSERT_EQ(std::string(delegations.consensus_err), "");
  ASSERT_EQ(delegations.code_retval.size(), 192u);
  EXPECT_EQ(abi_word_u64(delegations.code_retval, 0), 64u);
  EXPECT_EQ(abi_word_u64(delegations.code_retval, 32), 1u);
  EXPECT_EQ(abi_word_u64(delegations.code_retval, 64), 1u);
  const auto validator_word = abi_address_word(validator_address);
  EXPECT_EQ(std::vector<uint8_t>(delegations.code_retval.begin() + 96, delegations.code_retval.begin() + 128),
            std::vector<uint8_t>(validator_word.begin(), validator_word.end()));
  EXPECT_EQ(abi_word_u64(delegations.code_retval, 128), 10'000u);
  EXPECT_EQ(abi_word_u64(delegations.code_retval, 160), 0u);
}

TEST_F(RustFinalChainTest, DposQueriesRejectMissingNonGenesisSnapshot) {
  const auto validator_address = address(0x10);
  auto final_chain = create_final_chain_for_test(genesis_validators(validator_address));

  EXPECT_THROW(final_chain->get_dpos_eligible_total_vote_count(1), std::exception);
  EXPECT_THROW(final_chain->get_dpos_eligible_vote_count(1, validator_address), std::exception);
  EXPECT_THROW(final_chain->get_dpos_eligible_vote_count(1, validator_address), std::exception);
  EXPECT_THROW(final_chain->get_dpos_validators_total_stakes(1), std::exception);
}

TEST_F(RustFinalChainTest, DposCallReturnsGenesisValidatorMetadata) {
  const auto validator_address = address(0x10);
  const auto owner = address(0x11);
  auto final_chain = create_final_chain_for_test(genesis_validators(validator_address));

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

TEST_F(RustFinalChainTest, DposCallReturnsGenesisValidatorPages) {
  const auto first_validator = address(0x30);
  const auto second_validator = address(0x10);
  const auto first_owner = address(0x11);
  const auto second_owner = address(0x22);
  rust::Vec<GenesisValidator> validators;
  validators.push_back(genesis_validator(first_validator, first_owner, "first"));
  validators.push_back(genesis_validator(second_validator, second_owner, "second"));

  auto final_chain = create_final_chain_for_test(std::move(validators));

  auto all = final_chain->call(dpos_call(0, get_validators_input(0)));
  ASSERT_EQ(std::string(all.code_err), "");
  ASSERT_EQ(std::string(all.consensus_err), "");
  EXPECT_EQ(all.gas_used, 10'000u);
  EXPECT_EQ(abi_word_u64(all.code_retval, 32), 1u);
  EXPECT_EQ(validator_page_addresses(all.code_retval), (std::vector{first_validator, second_validator}));

  auto by_owner = final_chain->call(dpos_call(0, get_validators_for_input(first_owner, 0)));
  ASSERT_EQ(std::string(by_owner.code_err), "");
  ASSERT_EQ(std::string(by_owner.consensus_err), "");
  EXPECT_EQ(by_owner.gas_used, 100'000u);
  EXPECT_EQ(validator_page_addresses(by_owner.code_retval), (std::vector{first_validator}));
}

TEST_F(RustFinalChainTest, DposCallExecutesMutationsTransientlyAndReturnsLogs) {
  const auto validator_address = address(0x10);
  auto final_chain = create_final_chain_for_test(genesis_validators(validator_address));

  auto claim_rewards_outcome = final_chain->call(dpos_call(0, get_claim_rewards_input(validator_address)));
  ASSERT_EQ(std::string(claim_rewards_outcome.code_err), "Delegation does not exist");
  EXPECT_EQ(std::string(claim_rewards_outcome.consensus_err), "");
  EXPECT_TRUE(claim_rewards_outcome.code_retval.empty());
  EXPECT_TRUE(claim_rewards_outcome.logs.empty());

  auto delegate_call = dpos_call(0, get_delegate_input(validator_address));
  delegate_call.value = u64_be(1'000);
  auto delegate_outcome = final_chain->call(std::move(delegate_call));
  ASSERT_EQ(std::string(delegate_outcome.code_err), "");
  ASSERT_EQ(std::string(delegate_outcome.consensus_err), "");
  ASSERT_EQ(delegate_outcome.logs.size(), 1u);
  EXPECT_EQ(delegate_outcome.logs[0].address[19], 0xfe);
  EXPECT_EQ(delegate_outcome.logs[0].topics.size(), 3u);
  EXPECT_EQ(delegate_outcome.logs[0].data.size(), 32u);

  auto claim_all_outcome = final_chain->call(dpos_call(0, get_claim_all_rewards_input()));
  ASSERT_EQ(std::string(claim_all_outcome.code_err), "");
  EXPECT_EQ(std::string(claim_all_outcome.consensus_err), "");
  EXPECT_TRUE(claim_all_outcome.code_retval.empty());
  EXPECT_TRUE(claim_all_outcome.logs.empty());

  auto delegation_after_call = final_chain->call(dpos_call(0, get_total_delegation_input(address(0x00))));
  EXPECT_EQ(abi_word_u64(delegation_after_call.code_retval, 0), 0u);
}
