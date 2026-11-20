#include "rustaxa/selected_production_config.hpp"
#include "rustaxa-native-config/src/lib.rs.h"
#include <utility>

namespace rustaxa {
NativeSelectedProductionConfig::NativeSelectedProductionConfig(
    ConsensusApplicationBootstrapConfig bootstrap, std::uint64_t transaction_max_gas,
    std::uint64_t transaction_min_gas_price)
    : bootstrap_(std::make_unique<const ConsensusApplicationBootstrapConfig>(std::move(bootstrap))),
      transaction_max_gas_(transaction_max_gas), transaction_min_gas_price_(transaction_min_gas_price) {}
NativeSelectedProductionConfig::~NativeSelectedProductionConfig() = default;
const ConsensusApplicationBootstrapConfig& NativeSelectedProductionConfig::native_bootstrap() const noexcept {
  return *bootstrap_;
}
std::uint64_t NativeSelectedProductionConfig::native_transaction_max_gas() const noexcept {
  return transaction_max_gas_;
}
std::uint64_t NativeSelectedProductionConfig::native_transaction_min_gas_price() const noexcept {
  return transaction_min_gas_price_;
}
}  // namespace rustaxa
