#pragma once
#ifdef RUSTAXA_ENABLE
#include "config/config.hpp"
#include "rustaxa-native-config/src/lib.rs.h"
#include <algorithm>
#include <array>
#include <cstddef>
#include <cstdint>
#include <stdexcept>

namespace taraxa {
namespace production_config_detail {
template <typename Value>
std::array<uint8_t, 32> toBridgeU256(const Value& value) {
  std::array<uint8_t, 32> out{};
  const auto bytes = dev::toBigEndian(value);
  if (bytes.size() > out.size()) {
    throw std::runtime_error("u256 value exceeds 32 bytes");
  }
  std::copy(bytes.begin(), bytes.end(), out.begin() + static_cast<std::ptrdiff_t>(out.size() - bytes.size()));
  return out;
}
}  // namespace production_config_detail
rust::Vec<rustaxa::GenesisAccount> makeGenesisAccounts(const state_api::Config& config);
rust::Vec<rustaxa::GenesisValidator> makeGenesisValidators(const state_api::Config& config);
rustaxa::GenesisDposConfig makeGenesisDposConfig(const state_api::DPOSConfig& config,
                                              uint64_t dag_vdf_sortition_total_vote_count_until_period);
rustaxa::FinalChainRewardsConfig makeFinalChainRewardsConfig(const FullNodeConfig& config);
rustaxa::ConsensusApplicationBootstrapConfig makeConsensusBootstrapConfig(
    const FullNodeConfig& config, std::uint64_t committed_period,
    const std::array<std::uint8_t, 32>& committed_root);
}  // namespace taraxa
#endif
