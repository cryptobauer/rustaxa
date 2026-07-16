#pragma once

#include <memory>
#include <shared_mutex>

#include "config/genesis.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/dag_transaction_service.hpp"
#include "transaction/transaction.hpp"

namespace taraxa {

class DbStorage;
class TransactionManager;

/**
 * Rust-mode GasPricer facade.
 *
 * This class intentionally replaces the legacy `GasPricer` C++ implementation in
 * Rust-enabled builds. It preserves the upstream constructor/destructor and query
 * APIs while delegating pricing decisions to the Rust-owned transaction-manager
 * runtime.
 *
 * Behavior notes:
 * - Production instances route block and pool bids through `TransactionManager`;
 *   pool queue inspection and oracle combination remain inside its combined runtime.
 * - `update` forwards finalized-block transactions to that same runtime.
 * - A null-manager block-mode instance owns a storage-free combined runtime solely
 *   for standalone compatibility tests. It is never production authority.
 * - No fallback to the legacy C++ pricing logic is performed in Rust mode.
 */
class GasPricer {
 public:
  GasPricer(const GenesisConfig& config, bool is_light_node = false, bool is_blocks_gas_pricer = false,
            std::shared_ptr<TransactionManager> trx_mgr = nullptr, std::shared_ptr<DbStorage> db = {});
  ~GasPricer();

  GasPricer(const GasPricer&) = delete;
  GasPricer(GasPricer&&) = delete;
  GasPricer& operator=(const GasPricer&) = delete;
  GasPricer& operator=(GasPricer&&) = delete;

  /**
   * @brief returns current gas price
   *
   * @return u256 gas price
   */
  u256 bid() const;

  /**
   * @brief updates gas price after each executed block
   *
   * @param trxs from latest block
   */
  void update(const SharedTransactions& trxs);

 private:
  const bool kIsLightNode;
  const bool kBlocksGasPricer;
  std::shared_ptr<TransactionManager> trx_mgr_;

  mutable std::shared_mutex mutex_;
  SharedDagTransactionService compatibility_service_;
};

}  // namespace taraxa
