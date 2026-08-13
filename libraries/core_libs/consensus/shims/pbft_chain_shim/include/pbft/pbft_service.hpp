#pragma once

#include <memory>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

/**
 * Shared C++ lifetime owner for the Rust consensus application.
 *
 * One instance owns one `BridgeConsensusApplication` root. `App` shares this holder with retained manager, chain,
 * proposed-block, verified-vote, pillar-chain, and network facades so no shim borrows a nested Rust reference. Rust
 * owns runtime synchronization and lock-domain partitioning; this holder only provides RAII lifetime sharing.
 */
class ConsensusApplication final {
 public:
  /** Takes exclusive ownership of a fully restored Rust consensus application root. */
  explicit ConsensusApplication(rust::Box<rustaxa::BridgeConsensusApplication> service)
      : service_(std::move(service)) {}

  ConsensusApplication(const ConsensusApplication&) = delete;
  ConsensusApplication(ConsensusApplication&&) = delete;
  ConsensusApplication& operator=(const ConsensusApplication&) = delete;
  ConsensusApplication& operator=(ConsensusApplication&&) = delete;

  /** Returns the shared service receiver while this holder remains alive. */
  const rustaxa::BridgeConsensusApplication& service() const noexcept { return *service_; }

 private:
  rust::Box<rustaxa::BridgeConsensusApplication> service_;
};

using SharedConsensusApplication = std::shared_ptr<ConsensusApplication>;

}  // namespace taraxa
