#include <cassert>
#include <stdexcept>

#include "transaction/gas_pricer.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa {

GasPricer::GasPricer(const GenesisConfig& config, bool is_light_node, bool is_blocks_gas_pricer,
                     std::shared_ptr<TransactionManager> trx_mgr, std::shared_ptr<DbStorage> db)
    : kIsLightNode(is_light_node), kBlocksGasPricer(is_blocks_gas_pricer), trx_mgr_(std::move(trx_mgr)) {
  static_cast<void>(db);
  assert(config.gas_price.percentile <= 100);
}

GasPricer::~GasPricer() = default;

u256 GasPricer::bid() const {
  if (trx_mgr_) {
    return trx_mgr_->gasPriceBid();
  }

  if (!kBlocksGasPricer) {
    throw std::logic_error("GasPricer::bid requested pool price with no TransactionManager");
  }
  throw std::logic_error("GasPricer::bid requested block price with no TransactionManager");
}

void GasPricer::update(const SharedTransactions& trxs) {
  if (trx_mgr_) {
    trx_mgr_->updateGasPrice(trxs);
    return;
  }

  if (trxs.empty()) {
    return;
  }

  if (!kBlocksGasPricer) {
    throw std::logic_error("GasPricer::update requested pool price update with no TransactionManager");
  }
  throw std::logic_error("GasPricer::update requested block price update with no TransactionManager");
}

}  // namespace taraxa
