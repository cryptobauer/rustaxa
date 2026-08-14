#pragma once

#include <algorithm>
#include <array>
#include <cstdint>
#include <filesystem>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"

namespace rustaxa::test {

inline rust::Vec<uint8_t> u64Be(uint64_t value) {
  rust::Vec<uint8_t> bytes;
  while (value != 0) {
    bytes.push_back(static_cast<uint8_t>(value & 0xff));
    value >>= 8;
  }
  std::reverse(bytes.begin(), bytes.end());
  return bytes;
}

/** Returns the minimal valid DPoS policy used by CXX application-root fixtures. */
inline GenesisDposConfig genesisDposConfig() {
  GenesisDposConfig config{};
  config.eligibility_balance_threshold = u64Be(1'000);
  config.vote_eligibility_balance_step = u64Be(1'000);
  config.validator_maximum_stake = u64Be(30'000);
  config.minimum_deposit = {};
  config.commission_change_delta = 500;
  config.commission_change_frequency = 7;
  config.delegation_delay = 0;
  config.dag_vdf_sortition_total_vote_count_until_period = 0;
  return config;
}

/** Returns a reward policy with no scheduled emissions for boundary-only fixtures. */
inline FinalChainRewardsConfig finalChainRewardsConfig() {
  FinalChainRewardsConfig config{};
  config.aspen_part_one_period = UINT64_MAX;
  config.fix_claim_all_block_num = UINT64_MAX;
  return config;
}

/**
 * Constructs one complete production-shaped application root for a fixture directory.
 *
 * The returned root exclusively owns native storage and FinalChain state. Tests must
 * derive query, batch, and consensus handles from this root instead of creating
 * independent storage or FinalChain bootstrap handles.
 */
inline rust::Box<BridgeConsensusApplication> createConsensusApplication(
    const std::filesystem::path& storage_path, PbftServiceConfig pbft_config, uint32_t dag_expiry_limit = 32,
    uint16_t changing_interval = 10, rust::Vec<GenesisAccount> genesis_accounts = {},
    rust::Vec<GenesisValidator> genesis_validators = {}, GenesisDposConfig dpos_config = genesisDposConfig(),
    FinalChainRewardsConfig rewards_config = finalChainRewardsConfig()) {
  SortitionRuntimeConfig sortition{};
  sortition.threshold_upper = 0x100;
  sortition.difficulty_min = 1;
  sortition.difficulty_max = 10;
  sortition.difficulty_stale = 5;
  sortition.lambda_bound = 100;
  sortition.changes_count_for_average = 8;
  sortition.dag_efficiency_target_low = 5000;
  sortition.dag_efficiency_target_high = 10000;
  sortition.changing_interval = changing_interval;
  sortition.computation_interval = changing_interval;

  GasPricerConfig gas_pricer{};
  gas_pricer.percentile = 50;
  std::array<uint8_t, 32> storage_genesis{};
  storage_genesis[31] = 1;
  std::array<uint8_t, 32> dag_genesis{};
  dag_genesis[31] = 2;
  return create_consensus_application(storage_path.string(), 1, 0, storage_genesis, dag_genesis, dag_expiry_limit, 100,
                                      sortition, TransactionQueueConfig{16}, gas_pricer, 1'000'000,
                                      std::move(pbft_config), 1'000'000, 0, std::move(genesis_accounts),
                                      std::move(genesis_validators), std::move(dpos_config), std::move(rewards_config));
}

}  // namespace rustaxa::test
