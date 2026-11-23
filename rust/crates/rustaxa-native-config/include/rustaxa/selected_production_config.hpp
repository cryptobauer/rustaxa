#pragma once
#include <cstdint>
#include <memory>

namespace rustaxa {
struct ConsensusApplicationBootstrapConfig;
class NativeSelectedProductionConfigBuilder;

class NativeSelectedProductionConfig final {
 public:
  ~NativeSelectedProductionConfig();
  NativeSelectedProductionConfig(const NativeSelectedProductionConfig&) = delete;
  NativeSelectedProductionConfig& operator=(const NativeSelectedProductionConfig&) = delete;
  const ConsensusApplicationBootstrapConfig& native_bootstrap() const noexcept;
  std::uint64_t native_transaction_max_gas() const noexcept;
  std::uint64_t native_transaction_min_gas_price() const noexcept;

 private:
  friend class NativeSelectedProductionConfigBuilder;
  NativeSelectedProductionConfig(ConsensusApplicationBootstrapConfig bootstrap,
                                 std::uint64_t transaction_max_gas,
                                 std::uint64_t transaction_min_gas_price);
  const std::unique_ptr<const ConsensusApplicationBootstrapConfig> bootstrap_;
  const std::uint64_t transaction_max_gas_;
  const std::uint64_t transaction_min_gas_price_;
};
}  // namespace rustaxa
