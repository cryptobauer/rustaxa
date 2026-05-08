#pragma once

#include <deque>
#include <memory>
#include <optional>

#include "config/config.hpp"
#include "pbft/period_data.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"
#include "vdf/config.hpp"

namespace taraxa {

/**
 * Rust-mode sortition parameter manager facade.
 *
 * The facade preserves the public C++ API while routing deterministic
 * efficiency sampling and VRF threshold updates to Rust. Storage ownership and
 * batch atomicity remain in C++: the shim loads historical changes, translates
 * PeriodData into compact counts, and persists Rust-emitted changes into the
 * caller-provided batch.
 *
 * Inputs and outputs match the legacy SortitionParamsManager surface. PeriodData
 * is reduced to a pivot flag, finalized unique transaction count, and total DAG
 * transaction references before crossing the bridge. SortitionParamsChange
 * values remain serialized by the existing C++ storage APIs. Unsupported
 * protected helper calls throw locally instead of falling back to
 * SortitionParamsManagerOld.
 *
 * Invariants and edge behavior:
 * - Empty storage is initialized with the genesis VRF threshold at period 0.
 * - The Rust runtime owns interval counters and threshold-policy state.
 * - getParamsChanges returns the Rust-backed cache after each state transition.
 * - This class must not inherit from or delegate to SortitionParamsManagerOld.
 */
class SortitionParamsManager {
 public:
  SortitionParamsManager(const addr_t& node_addr, const FullNodeConfig& config, std::shared_ptr<DbStorage> db);

  /**
   * Returns current sortition parameters, or parameters adjusted with the last
   * persisted change active at or before for_period.
   */
  SortitionParams getSortitionParams(std::optional<PbftPeriod> for_period = {}) const;

  /**
   * Calculates DAG efficiency for finalized PeriodData using the Rust fixed-point policy.
   *
   * The input is reduced to unique transaction count and total DAG transaction references before crossing the bridge.
   */
  uint16_t calculateDagEfficiency(const PeriodData& block) const;

  /**
   * Processes one finalized non-empty PBFT block.
   *
   * Rust updates the in-memory interval state and returns a parameter change when this block closes a changing
   * interval. The C++ facade persists that change into the provided batch.
   */
  void pbftBlockPushed(const PeriodData& block, Batch& batch, PbftPeriod non_empty_pbft_chain_size);

  /**
   * Returns the current interval average DAG efficiency.
   */
  uint16_t averageDagEfficiency();

  /**
   * Returns the Rust-backed in-memory parameter-change cache.
   */
  const std::deque<SortitionParamsChange>& getParamsChanges() const { return params_changes_; }

 protected:
  const FullNodeConfig kConfig;
  SortitionConfig sortition_config_;
  std::shared_ptr<DbStorage> db_;
  std::deque<SortitionParamsChange> params_changes_;
  std::optional<::rust::Box<rustaxa::BridgeSortitionParamsManager>> rust_sortition_params_manager_;

  /**
   * Unsupported in Rust shim mode because threshold changes are emitted by the
   * Rust runtime during pbftBlockPushed.
   */
  SortitionParamsChange calculateChange(PbftPeriod period);

  /**
   * Unsupported in Rust shim mode because historical threshold fitting is owned
   * by the Rust runtime.
   */
  EfficienciesMap getEfficienciesToUpperRange(uint16_t efficiency, int32_t threshold) const;

  /**
   * Unsupported in Rust shim mode because threshold selection is owned by the
   * Rust runtime.
   */
  int32_t getNewUpperRange(uint16_t efficiency) const;

  /**
   * Unsupported in Rust shim mode because cache cleanup is owned by the Rust
   * runtime.
   */
  void cleanup();
};

}  // namespace taraxa
