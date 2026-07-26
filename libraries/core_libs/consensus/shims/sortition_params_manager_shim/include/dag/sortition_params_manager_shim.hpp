#pragma once

#include <libdevcore/RLP.h>

#include <cstdint>
#include <deque>
#include <map>
#include <memory>
#include <optional>

#include "common/types.hpp"
#include "config/config.hpp"
#include "pbft/period_data.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"
#include "transaction/dag_transaction_service.hpp"
#include "vdf/config.hpp"

namespace taraxa {

using EfficienciesMap = std::map<uint16_t, int32_t>;

/**
 * A persisted change to the DAG sortition threshold.
 *
 * The period identifies when the new VRF parameters become active,
 * interval_efficiency records the fixed-point efficiency that caused the
 * change, and vrf_params carries the resulting threshold. RLP encoding uses
 * the stable legacy field order: threshold, period, efficiency. Decoding
 * expects all three fields and propagates malformed-RLP failures.
 */
struct SortitionParamsChange {
  PbftPeriod period = 0;
  VrfParams vrf_params;
  uint16_t interval_efficiency = 0;

  SortitionParamsChange() = default;
  SortitionParamsChange(PbftPeriod period, uint16_t efficiency, const VrfParams& vrf);
  static SortitionParamsChange from_rlp(const dev::RLP& rlp);
  bytes rlp() const;
};

/**
 * Rust-mode sortition parameter manager facade.
 *
 * The facade preserves the public C++ API while routing deterministic
 * efficiency sampling, VRF threshold updates, startup replay, and historical
 * parameter lookups to Rust. The legacy `pbftBlockPushed` API still receives a
 * C++ `Batch&`; Rust mode keeps that argument only for API compatibility and
 * persists emitted sortition changes through the Rust manager's native storage
 * handle before live state is updated.
 *
 * Inputs and outputs match the legacy SortitionParamsManager surface. PeriodData
 * is reduced to a pivot flag, finalized unique transaction count, and total DAG
 * transaction references before crossing the bridge. SortitionParamsChange
 * values are stored through Rust storage APIs. Unsupported protected helper
 * calls throw locally instead of invoking a legacy implementation.
 *
 * Invariants and edge behavior:
 * - Empty storage is initialized with the genesis VRF threshold at period 0.
 * - The application-owned DAG/transaction service owns interval counters and
 *   threshold-policy state behind its sortition lock domain.
 * - getParamsChanges returns the Rust-backed cache after each state transition.
 * - This facade has no legacy implementation dependency or independent Rust
 *   sortition handle.
 */
class SortitionParamsManager {
 public:
  /**
   * Creates a compatibility facade with a fully restored application service.
   *
   * This overload preserves the public construction API for standalone C++
   * callers. Production DagManager wiring uses the shared-service overload so
   * all DAG consumers observe one sortition runtime.
   */
  SortitionParamsManager([[maybe_unused]] const addr_t& node_addr, const FullNodeConfig& config,
                         std::shared_ptr<DbStorage> db);
  /**
   * Creates a facade over the canonical DAG/transaction application service.
   *
   * The service must be non-null and expose sortition capability. `db` remains
   * in the signature for source compatibility but the service owns native
   * storage.
   */
  SortitionParamsManager([[maybe_unused]] const addr_t& node_addr, const FullNodeConfig& config,
                         std::shared_ptr<DbStorage> db, SharedDagTransactionService dag_transaction_service);

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
   * Rust updates the interval state and persists any emitted parameter change
   * through native Rust storage. The batch argument is ignored in Rust mode and
   * remains only to preserve the public C++ API during migration.
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
  std::deque<SortitionParamsChange> params_changes_;
  SharedDagTransactionService dag_transaction_service_;

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
