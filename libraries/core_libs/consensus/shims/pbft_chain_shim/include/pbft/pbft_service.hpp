#pragma once

#include <memory>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

/**
 * Shared C++ lifetime owner for the Rust PBFT application service.
 *
 * One instance owns one `BridgePbftService` box. `App` shares this holder with
 * the retained manager and chain facades so neither facade borrows a nested
 * Rust reference or restores an independent production runtime. Rust owns the
 * synchronization of the service's manager and chain lock domains; this holder
 * only provides RAII lifetime sharing.
 */
class PbftService final {
 public:
  /** Takes exclusive ownership of a fully restored Rust PBFT service. */
  explicit PbftService(rust::Box<rustaxa::BridgePbftService> service) : service_(std::move(service)) {}

  PbftService(const PbftService&) = delete;
  PbftService(PbftService&&) = delete;
  PbftService& operator=(const PbftService&) = delete;
  PbftService& operator=(PbftService&&) = delete;

  /** Returns the shared service receiver while this holder remains alive. */
  const rustaxa::BridgePbftService& service() const noexcept { return *service_; }

 private:
  rust::Box<rustaxa::BridgePbftService> service_;
};

using SharedPbftService = std::shared_ptr<PbftService>;

}  // namespace taraxa
