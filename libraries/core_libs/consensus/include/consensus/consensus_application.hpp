#pragma once

#include <memory>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

/**
 * Shared C++ lifetime owner for the native Rust consensus application.
 *
 * One instance owns one opaque application root. Consumers may invoke named
 * task and client APIs but cannot retrieve, replace, or construct its private
 * storage, FinalChain, DAG, transaction, vote, or PBFT services.
 */
class ConsensusApplication final {
 public:
  /** Takes exclusive ownership of a fully restored native application root. */
  explicit ConsensusApplication(rust::Box<rustaxa::BridgeConsensusApplication> service)
      : service_(std::move(service)) {}

  ConsensusApplication(const ConsensusApplication&) = delete;
  ConsensusApplication(ConsensusApplication&&) = delete;
  ConsensusApplication& operator=(const ConsensusApplication&) = delete;
  ConsensusApplication& operator=(ConsensusApplication&&) = delete;

  /** Returns the opaque task receiver while this holder remains alive. */
  const rustaxa::BridgeConsensusApplication& service() const noexcept { return *service_; }

 private:
  rust::Box<rustaxa::BridgeConsensusApplication> service_;
};

using SharedConsensusApplication = std::shared_ptr<ConsensusApplication>;

}  // namespace taraxa
