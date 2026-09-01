#include <gtest/gtest.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <filesystem>
#include <limits>
#include <string>
#include <utility>
#include <vector>

#include "config/config.hpp"
#include "consensus/consensus_application.hpp"
#include "consensus_application_test.hpp"
#include "final_chain/state_api.hpp"
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

  static std::vector<uint8_t> u256_be(uint64_t value) {
    std::vector<uint8_t> out(32, 0);
    for (auto index = 0U; index < sizeof(value); ++index) {
      out[31 - index] = static_cast<uint8_t>(value & 0xFF);
      value >>= 8;
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

  static uint64_t abi_word_u64(const dev::bytes& data, size_t offset) {
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

  static std::string abi_string_at(const dev::bytes& data, size_t tuple_start, size_t offset) {
    const auto tail_start = tuple_start + offset;
    const auto size = abi_word_u64(data, tail_start);
    return std::string(data.begin() + tail_start + 32, data.begin() + tail_start + 32 + size);
  }

  static std::vector<std::array<uint8_t, 20>> validator_page_addresses(const dev::bytes& data) {
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

  static taraxa::state_api::ValidatorInfo genesis_validator(std::array<uint8_t, 20> validator_address,
                                                            std::array<uint8_t, 20> owner = address(0x11),
                                                            std::string description = "bridge validator metadata") {
    const auto validator_vrf_key = vrf_key(0xA0);
    taraxa::state_api::ValidatorInfo validator{
        taraxa::addr_t(validator_address.data(), taraxa::addr_t::ConstructFromPointer),
        taraxa::addr_t(owner.data(), taraxa::addr_t::ConstructFromPointer),
        taraxa::vrf_wrapper::vrf_pk_t(validator_vrf_key.data(), taraxa::vrf_wrapper::vrf_pk_t::ConstructFromPointer),
        12,
        "https://validator.example",
        std::move(description),
        {},
    };
    validator.delegations.emplace(validator.owner, 10'000);
    return validator;
  }

  static std::vector<taraxa::state_api::ValidatorInfo> genesis_validators(
      std::array<uint8_t, 20> validator_address) {
    std::vector<taraxa::state_api::ValidatorInfo> validators;
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

  taraxa::SharedConsensusApplication create_final_chain_for_test(
      std::vector<taraxa::state_api::ValidatorInfo> validators) {
    taraxa::FullNodeConfig config{};
    config.db_path = test_dir;
    config.genesis.chain_id = 1;
    config.genesis.state.initial_balances.clear();
    for (const auto& validator : validators) {
      for (const auto& [delegator, stake] : validator.delegations) {
        config.genesis.state.initial_balances[delegator] += stake;
      }
    }
    config.genesis.state.dpos.eligibility_balance_threshold = 1'000;
    config.genesis.state.dpos.vote_eligibility_balance_step = 1'000;
    config.genesis.state.dpos.validator_maximum_stake = 30'000;
    config.genesis.state.dpos.commission_change_delta = 500;
    config.genesis.state.dpos.commission_change_frequency = 7;
    config.genesis.state.dpos.delegation_delay = 0;
    config.genesis.state.dpos.yield_percentage = 0;
    config.genesis.state.hardforks.aspen_hf.block_num_part_one = std::numeric_limits<uint64_t>::max();
    config.genesis.state.hardforks.aspen_hf.block_num_part_two = std::numeric_limits<uint64_t>::max();
    config.genesis.state.dpos.initial_validators = std::move(validators);
    return taraxa::createConsensusApplication(config);
  }

  std::filesystem::path test_dir;
};

TEST_F(RustFinalChainTest, DposQueriesUseGenesisSnapshotAtBlockZero) {
  const auto validator_address = address(0x10);
  const auto unknown_address = address(0x20);
  auto final_chain = create_final_chain_for_test(genesis_validators(validator_address));
  const auto query = final_chain->queryClient();

  EXPECT_EQ((*query)->consensus_query_final_chain_dpos_eligible_total_vote_count(0), 10u);
  EXPECT_EQ((*query)->consensus_query_final_chain_dpos_eligible_vote_count(0, validator_address), 10u);
  EXPECT_GT((*query)->consensus_query_final_chain_dpos_eligible_vote_count(0, validator_address), 0);
  EXPECT_EQ((*query)->consensus_query_final_chain_dpos_eligible_vote_count(0, unknown_address), 0u);
  EXPECT_EQ((*query)->consensus_query_final_chain_dpos_eligible_vote_count(0, unknown_address), 0);

  const auto stakes = (*query)->consensus_query_final_chain_dpos_validators_total_stakes(0);
  ASSERT_EQ(stakes.size(), 1u);
  EXPECT_EQ(stakes[0].address, validator_address);
  EXPECT_EQ(bytes(stakes[0].stake), u256_be(10000));
  EXPECT_EQ(bytes((*query)->consensus_query_final_chain_dpos_total_amount_delegated(0)), bytes(u64_be(10000)));
  EXPECT_EQ((*query)->consensus_query_final_chain_dpos_yield(0), 0u);
  EXPECT_TRUE((*query)->consensus_query_final_chain_dpos_total_supply(0).empty());

}

TEST_F(RustFinalChainTest, DposQueriesRejectMissingNonGenesisSnapshot) {
  const auto validator_address = address(0x10);
  auto final_chain = create_final_chain_for_test(genesis_validators(validator_address));
  const auto query = final_chain->queryClient();

  EXPECT_THROW((*query)->consensus_query_final_chain_dpos_eligible_total_vote_count(1), std::exception);
  EXPECT_THROW((*query)->consensus_query_final_chain_dpos_eligible_vote_count(1, validator_address), std::exception);
  EXPECT_THROW((*query)->consensus_query_final_chain_dpos_eligible_vote_count(1, validator_address), std::exception);
  EXPECT_THROW((*query)->consensus_query_final_chain_dpos_validators_total_stakes(1), std::exception);
}
