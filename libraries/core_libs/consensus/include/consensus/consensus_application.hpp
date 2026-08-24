#pragma once

#include <memory>
#include <optional>

#include "common/types.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

struct FullNodeConfig;

/** Coherent live protocol counters exposed to application scheduling and diagnostics. */
struct ConsensusRuntimeStatus {
  PbftPeriod period{0};
  PbftRound round{0};
  PbftStep step{0};
  PbftPeriod finalized_chain_size{0};
  PbftPeriod syncing_period{0};
  size_t sync_queue_size{0};
  bool syncQueueEmpty() const noexcept { return sync_queue_size == 0; }
};

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
  explicit ConsensusApplication(rust::Box<rustaxa::BridgeConsensusApplication> service);
  ~ConsensusApplication();

  ConsensusApplication(const ConsensusApplication&) = delete;
  ConsensusApplication(ConsensusApplication&&) = delete;
  ConsensusApplication& operator=(const ConsensusApplication&) = delete;
  ConsensusApplication& operator=(ConsensusApplication&&) = delete;

  /** Returns the opaque task receiver while this holder remains alive. */
  const rustaxa::BridgeConsensusApplication& service() const noexcept { return *service_; }

  /** Returns one coherent application-root runtime status snapshot. */
  ConsensusRuntimeStatus runtimeStatus() const;
  /** Resolves the current node's DPoS votes for diagnostic metrics only. */
  std::optional<uint64_t> currentNodeVotesCount() const;
  /** Resolves total eligible DPoS votes for diagnostic metrics only. */
  std::optional<uint64_t> currentDposTotalVotesCount() const;

 private:
  rust::Box<rustaxa::BridgeConsensusApplication> service_;
};

using SharedConsensusApplication = std::shared_ptr<ConsensusApplication>;

/** Builds the sole native application root from immutable node configuration. */
SharedConsensusApplication createConsensusApplication(const FullNodeConfig& config);

}  // namespace taraxa
