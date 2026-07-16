#include <array>
#include <cassert>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <stdexcept>

#include "transaction/gas_pricer.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa {
namespace {

std::array<uint8_t, 32> to_bridge_u256(const u256& value) {
  auto bytes = dev::toBigEndian(value);
  std::array<uint8_t, 32> out{};
  if (bytes.size() > out.size()) {
    throw std::overflow_error("u256 value cannot be represented in 32 bridge bytes");
  }
  std::memcpy(out.data() + (out.size() - bytes.size()), bytes.data(), bytes.size());
  return out;
}

u256 from_bridge_u256(const std::array<uint8_t, 32>& bytes) {
  return dev::fromBigEndian<u256>(dev::bytes(bytes.begin(), bytes.end()));
}

rustaxa::GasPricerConfig to_bridge_config(const GenesisConfig& config, bool is_light_node, bool blocks_gas_pricer) {
  rustaxa::GasPricerConfig bridge_config;
  bridge_config.percentile = config.gas_price.percentile;
  bridge_config.minimum_price = to_bridge_u256(config.state.hardforks.soleirolia_hf.trx_min_gas_price);
  bridge_config.history_blocks = config.gas_price.blocks;
  bridge_config.is_light_node = is_light_node;
  bridge_config.blocks_gas_pricer = blocks_gas_pricer;
  return bridge_config;
}

rust::Vec<rustaxa::GasPricerGasPrice> extract_tx_gas_prices(const SharedTransactions& trxs) {
  rust::Vec<rustaxa::GasPricerGasPrice> gas_prices;
  gas_prices.reserve(trxs.size());
  for (const auto& trx : trxs) {
    rustaxa::GasPricerGasPrice gas_price;
    gas_price.price = to_bridge_u256(trx->getGasPrice());
    gas_prices.push_back(std::move(gas_price));
  }
  return gas_prices;
}

}  // namespace

GasPricer::GasPricer(const GenesisConfig& config, bool is_light_node, bool is_blocks_gas_pricer,
                     std::shared_ptr<TransactionManager> trx_mgr, std::shared_ptr<DbStorage> db)
    : kIsLightNode(is_light_node), kBlocksGasPricer(is_blocks_gas_pricer), trx_mgr_(std::move(trx_mgr)) {
  static_cast<void>(db);
  assert(config.gas_price.percentile <= 100);
  if (!trx_mgr_ && kBlocksGasPricer) {
    compatibility_service_ = std::make_shared<DagTransactionService>(
        rustaxa::create_dag_transaction_service_for_gas_pricer(to_bridge_config(config, kIsLightNode, true)));
  }
}

GasPricer::~GasPricer() = default;

u256 GasPricer::bid() const {
  if (trx_mgr_) {
    return trx_mgr_->gasPriceBid();
  }

  if (!kBlocksGasPricer || !compatibility_service_) {
    throw std::logic_error("GasPricer::bid requested pool price with no TransactionManager");
  }

  std::shared_lock lock(mutex_);
  return from_bridge_u256(compatibility_service_->service().transaction_manager_runtime_gas_price_bid());
}

void GasPricer::update(const SharedTransactions& trxs) {
  if (trx_mgr_) {
    trx_mgr_->updateGasPrice(trxs);
    return;
  }

  if (!kBlocksGasPricer || trxs.empty()) {
    return;
  }

  if (!compatibility_service_) {
    throw std::logic_error("GasPricer::update requested block price update with no runtime");
  }
  std::unique_lock lock(mutex_);
  compatibility_service_->service().transaction_manager_runtime_gas_price_update(extract_tx_gas_prices(trxs));
}

}  // namespace taraxa
