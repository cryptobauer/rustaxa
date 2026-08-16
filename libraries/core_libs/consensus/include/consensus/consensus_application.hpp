#pragma once

#include <memory>
#include <mutex>
#include <optional>
#include <utility>

#include "common/types.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

class App;

class DagManager;
class DbStorage;
class Network;
class PeriodData;
class TransactionManager;
struct FullNodeConfig;

namespace final_chain {
class FinalChain;
}

namespace pillar_chain {
class PillarChainManager;
}

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

  /** Installs the process-local transport, timer, signing, and EVM effect executor once. */
  void initializeHost(const FullNodeConfig& config, std::shared_ptr<DbStorage> db,
                      std::shared_ptr<DagManager> dag_manager, std::shared_ptr<TransactionManager> transaction_manager,
                      std::shared_ptr<final_chain::FinalChain> final_chain,
                      std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager);

  /** Attaches the physical transport executor without transferring network ownership. */
  void attachNetwork(std::weak_ptr<Network> network);
  /** Starts native consensus scheduling. Initialization and startup failures propagate. */
  void startConsensus();
  /** Stops native consensus scheduling and joins its executor thread. Idempotent. */
  void stopConsensus();
  /** Returns one coherent application-root runtime status snapshot. */
  ConsensusRuntimeStatus runtimeStatus() const;
  /** Resolves the current node's DPoS votes for diagnostic metrics only. */
  std::optional<uint64_t> currentNodeVotesCount() const;
  /** Resolves total eligible DPoS votes for diagnostic metrics only. */
  std::optional<uint64_t> currentDposTotalVotesCount() const;

 private:
  friend class App;

  /**
   * Permanently detaches process-local services during App teardown.
   *
   * This is intentionally unavailable to ordinary consensus clients: unlike
   * stopConsensus(), shutdown is not restartable. It breaks the host-service
   * ownership cycle before App destroys the configuration and services that
   * Runtime references. Calls requiring the host fail after shutdown.
   */
  void shutdownHost();

  class Runtime;
  rust::Box<rustaxa::BridgeConsensusApplication> service_;
  mutable std::mutex host_mutex_;
  std::unique_ptr<Runtime> runtime_;
  bool host_shutdown_{false};
};

using SharedConsensusApplication = std::shared_ptr<ConsensusApplication>;

}  // namespace taraxa
