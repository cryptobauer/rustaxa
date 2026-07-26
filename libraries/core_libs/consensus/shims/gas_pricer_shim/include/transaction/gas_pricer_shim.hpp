#pragma once

#include <memory>

#include "config/genesis.hpp"
#include "rustaxa-bridge/ffi.rs.h"
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
 *   pool queue inspection and oracle combination remain inside its `TransactionManager`.
 * - `update` forwards finalized-block transactions to that same runtime.
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
};

}  // namespace taraxa
