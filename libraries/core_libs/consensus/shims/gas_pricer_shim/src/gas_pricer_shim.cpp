#include <algorithm>
#include <array>
#include <cassert>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <stdexcept>

#include "storage/storage.hpp"
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
  assert(config.gas_price.percentile <= 100);
  gas_pricer_ = rustaxa::create_gas_pricer(to_bridge_config(config, kIsLightNode, kBlocksGasPricer));

  if (kBlocksGasPricer && db) {
    init_daemon_ = std::make_unique<std::thread>([this, db_ = std::move(db)]() { init(db_); });
  }
}

GasPricer::~GasPricer() {
  if (init_daemon_ && init_daemon_->joinable()) {
    init_daemon_->join();
  }
}

void GasPricer::init(const std::shared_ptr<DbStorage>& db) {
  if (!db || !kBlocksGasPricer) {
    return;
  }
  try {
    gas_pricer_.value()->gas_pricer_init_from_storage(db->rustStorage());
  } catch (...) {
    std::unique_lock lock(mutex_);
    init_error_ = std::current_exception();
  }
}

u256 GasPricer::bid() const {
  std::shared_lock lock(mutex_);
  rethrowInitError();
  if (!gas_pricer_) {
    return 0;
  }

  if (kBlocksGasPricer) {
    return from_bridge_u256(gas_pricer_.value()->gas_pricer_bid());
  }

  if (!trx_mgr_) {
    throw std::logic_error("GasPricer::bid requested pool price with no TransactionManager");
  }
  return from_bridge_u256(
      gas_pricer_.value()->gas_pricer_bid_from_pool(to_bridge_u256(trx_mgr_->getMinGasPriceForBlockInclusion())));
}

void GasPricer::update(const SharedTransactions& trxs) {
  if (!kBlocksGasPricer || trxs.empty()) {
    return;
  }
  std::unique_lock lock(mutex_);
  rethrowInitError();
  gas_pricer_.value()->gas_pricer_update(extract_tx_gas_prices(trxs));
}

void GasPricer::rethrowInitError() const {
  if (init_error_) {
    std::rethrow_exception(init_error_);
  }
}

}  // namespace taraxa
