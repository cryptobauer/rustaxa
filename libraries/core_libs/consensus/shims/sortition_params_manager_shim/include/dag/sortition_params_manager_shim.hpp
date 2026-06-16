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
 * calls throw locally instead of falling back to SortitionParamsManagerOld.
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
   * Returns Rust-native sortition runtime parameters for a proposal period.
   *
   * Inputs:
   * - for_period selects the historical sortition parameters active for DAG
   *   proposal and VDF proof planning.
   *
   * Outputs:
   * - The Rust bridge DTO containing VRF threshold and VDF difficulty bounds.
   *
   * Invariants and edge behavior:
   * - Reads directly through the Rust sortition runtime's rustaxa-storage
   *   handle.
   * - Does not materialize or mutate the C++ SortitionParams compatibility DTO.
   * - Propagates Rust runtime storage/decoding failures as bridge exceptions.
   */
  rustaxa::SortitionRuntimeParams rustSortitionParamsForRust(PbftPeriod for_period) const;

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
   * Rust-only entry for PBFT finalization updates.
   *
   * Updates live Rust sortition state for the finalized PBFT block and returns a change payload when a threshold
   * interval boundary is crossed.
   *
   * This method does not persist anything into C++ storage; PBFT finalization routes the returned change through the
   * Rust staged finalization storage appender while preserving the caller's batch commit boundary.
   */
  std::optional<SortitionParamsChange> applyBlockForSortitionRuntime(const PeriodData& block,
                                                                     PbftPeriod non_empty_pbft_chain_size);

  /**
   * Preview a PBFT-finalization sortition update without mutating live Rust state.
   *
   * Inputs:
   * - Finalized period data and the post-finalization non-empty PBFT-chain size.
   *
   * Outputs:
   * - Optional sortition parameter change that must be persisted in the primary
   *   PBFT finalization batch before the live sortition runtime is committed.
   *
   * Invariants and edge behavior:
   * - Does not publish threshold/counter changes to live callers.
   * - Throws on malformed efficiency facts before storage stages are built.
   */
  std::optional<SortitionParamsChange> prepareBlockForSortitionFinalization(
      const PeriodData& block, PbftPeriod non_empty_pbft_chain_size);

  /**
   * Commit a previously previewed PBFT-finalization sortition update.
   *
   * Inputs:
   * - The same finalized period data and post-finalization non-empty PBFT-chain
   *   size used for the preview.
   * - The optional previewed change that was included in the committed primary
   *   finalization storage batch.
   * - The Rust-planned PBFT finalization write intent used for live proof identity.
   *
   * Outputs:
   * - A Rust-verifiable PBFT finalization live-mutation report.
   *
   * Invariants and edge behavior:
   * - Mutates live sortition state only after the caller has committed primary
   *   finalization storage.
   * - Throws if the live Rust transition diverges from the previewed change.
   */
  rustaxa::PbftFinalizationLiveMutationReport commitPreparedBlockForSortitionFinalization(
      const PeriodData& block, PbftPeriod non_empty_pbft_chain_size,
      const std::optional<SortitionParamsChange>& prepared_change,
      const rustaxa::PbftFinalizationStorageWritePlan& write_intent);

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
  std::shared_ptr<DbStorage> batch_owner_;
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
