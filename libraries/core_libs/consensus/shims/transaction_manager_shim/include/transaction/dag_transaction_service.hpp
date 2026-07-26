#pragma once

#include <memory>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

struct FullNodeConfig;
class DbStorage;

/**
 * Shared C++ lifetime owner for the Rust DAG/transaction application service.
 *
 * One instance owns the composed Rust transaction, DAG, and sortition
 * runtimes. `App` creates the fully restored production service and shares
 * this holder with the retained `TransactionManager`, `DagManager`, and
 * `SortitionParamsManager` facades. Rust owns sibling runtime synchronization;
 * this class only provides stable RAII ownership.
 */
class DagTransactionService final {
 public:
  /** Takes exclusive ownership of a composed Rust DAG/transaction/sortition service. */
  explicit DagTransactionService(rust::Box<rustaxa::BridgeDagTransactionService> service)
      : service_(std::move(service)) {}

  DagTransactionService(const DagTransactionService&) = delete;
  DagTransactionService(DagTransactionService&&) = delete;
  DagTransactionService& operator=(const DagTransactionService&) = delete;
  DagTransactionService& operator=(DagTransactionService&&) = delete;

  /**
   * Returns the shared service receiver while this holder remains alive.
   *
   * All Rust mutation is synchronized inside the service's sibling lock
   * domains, so C++ never obtains an exclusive reference to the shared root.
   */
  const rustaxa::BridgeDagTransactionService& service() const noexcept { return *service_; }

 private:
  rust::Box<rustaxa::BridgeDagTransactionService> service_;
};

using SharedDagTransactionService = std::shared_ptr<DagTransactionService>;

/** Builds the fully composed, storage-restored service used by `App`. */
SharedDagTransactionService createDagTransactionService(const FullNodeConfig& config, DbStorage& db);

}  // namespace taraxa
