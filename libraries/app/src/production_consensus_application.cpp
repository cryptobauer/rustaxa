#ifdef RUSTAXA_ENABLE
#include "cli/config.hpp"
#include "app/production_consensus_application.hpp"
#include "consensus/native_config_projection.hpp"
#include "rustaxa/selected_production_config.hpp"
#include "consensus/consensus_application.hpp"
#include <memory>
#include <stdexcept>
#include <utility>

namespace rustaxa {
class NativeSelectedProductionConfigBuilder final {
 public:
  static std::unique_ptr<NativeSelectedProductionConfig> capture(const taraxa::FullNodeConfig& config) {
    // The Rust owner loads the state descriptor from its owned stores.
    auto bootstrap = taraxa::makeConsensusBootstrapConfig(config, 0, {});
    const auto& policy = config.genesis.state.hardforks.soleirolia_hf;
    return std::unique_ptr<NativeSelectedProductionConfig>(new NativeSelectedProductionConfig(
        std::move(bootstrap), policy.trx_max_gas_limit, policy.trx_min_gas_price));
  }
};
}  // namespace rustaxa

namespace taraxa {
std::unique_ptr<rustaxa::NativeSelectedProductionConfig> loadSelectedProductionConfig(const cli::Config& cli_config) {
  if (!cli_config.nodeConfigured()) {
    throw std::runtime_error("PRODUCTION_SELECTED_NODE_CONFIG_REQUIRED");
  }
  auto effective_config = cli_config.getNodeConfiguration();
  effective_config.validate();
  const auto config = std::move(effective_config);
  if (config.db_config.rebuild_db || config.db_config.migrate_only || config.db_config.migrate_receipts_by_period ||
      config.db_config.db_revert_to_period != 0 || config.db_config.rebuild_db_period != 0) {
    throw std::runtime_error("PRODUCTION_EXISTING_STATE_REQUIRED");
  }
  if (!config.genesis.dag_genesis_block.verifySig()) {
    throw std::runtime_error("PRODUCTION_SELECTED_GENESIS_SIGNATURE_INVALID");
  }
  return rustaxa::NativeSelectedProductionConfigBuilder::capture(config);
}

rust::Box<rustaxa::BridgeConsensusApplication> createQualifiedProductionApplication(const cli::Config& cli_config) {
  auto selected = loadSelectedProductionConfig(cli_config);
  const auto config = cli_config.getNodeConfiguration();
  return openProductionConsensusApplication(std::move(selected), config.db_path / "state_db");
}
}  // namespace taraxa
#endif
