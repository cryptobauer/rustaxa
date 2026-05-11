#pragma once

#include <array>
#include <exception>
#include <optional>
#include <memory>
#include <shared_mutex>
#include <thread>
#include <vector>

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
 * APIs while delegating pricing decisions to a Rust `BridgeGasPricer`.
 *
 * Behavior notes:
 * - `bid` in block mode returns Rust state from historical/rolling block gas prices.
 * - `bid` in pool mode queries `TransactionManager` for minimum inclusion gas and
 *   lets Rust compute a final pool-aware bid.
 * - `update` forwards extracted gas prices from a finalized block into Rust.
 * - No fallback to legacy `GasPricerOld` logic is performed in Rust mode.
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
  void init(const std::shared_ptr<DbStorage>& db);
  void rethrowInitError() const;

  const bool kIsLightNode;
  const bool kBlocksGasPricer;
  std::shared_ptr<TransactionManager> trx_mgr_;

  mutable std::shared_mutex mutex_;
  std::unique_ptr<std::thread> init_daemon_;
  std::optional<::rust::Box<rustaxa::BridgeGasPricer>> gas_pricer_;
  std::exception_ptr init_error_;
};

}  // namespace taraxa
