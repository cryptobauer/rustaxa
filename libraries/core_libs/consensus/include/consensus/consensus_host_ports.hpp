#pragma once

#include <cstdint>
#include <memory>

namespace rustaxa {
struct HostEvmFinalizationReport;
struct HostEvmFinalizationRequest;
struct HostFinalChainAccountFactsReport;
struct HostFinalChainAccountFactsRequest;
struct HostGossipPillarVoteRequest;
struct HostGossipVoteBundleRequest;
struct HostGossipVoteRequest;
struct HostMaliciousPeerRequest;
struct HostPillarAnchorStateReport;
struct HostPillarAnchorStateRequest;
struct HostSetSyncPeriodRequest;
struct HostSignReport;
struct HostSignRequest;
struct HostTransportReport;
struct HostTransportStatus;
struct HostVrfReport;
struct HostVrfRequest;
struct HostWaitReport;
struct HostWaitRequest;
}  // namespace rustaxa

namespace taraxa {

class ConsensusApplication;
class FullNodeConfig;
class Network;

namespace final_chain {
class FinalChain;
}

/**
 * Interruptible clock and wait leaf used by the native consensus runner.
 *
 * The port owns only monotonic-clock and condition-variable process mechanics.
 * Every wait echoes its native generation/effect identity, and stop wakes an
 * active wait without retaining protocol phase, timer, or cursor state.
 */
class ConsensusProcessPort final {
 public:
  ConsensusProcessPort();
  ~ConsensusProcessPort();

  ConsensusProcessPort(const ConsensusProcessPort&) = delete;
  ConsensusProcessPort(ConsensusProcessPort&&) = delete;
  ConsensusProcessPort& operator=(const ConsensusProcessPort&) = delete;
  ConsensusProcessPort& operator=(ConsensusProcessPort&&) = delete;

  rustaxa::HostWaitReport consensusWait(const rustaxa::HostWaitRequest& request) const;
  bool consensusStopRequested(uint64_t generation) const;
  uint64_t consensusNowMillis() const;
  uint64_t consensusUnixTimeSeconds() const;

 private:
  friend class ConsensusProcess;
  class Impl;
  std::unique_ptr<Impl> impl_;
};

/**
 * Exact key-custody leaf for native consensus signing requests.
 *
 * Wallet secrets remain in the C++ configuration. Requests select a stable
 * wallet index and supply only the digest or VRF message; reports return the
 * signature/proof and the exact effect identity or an explicit error.
 */
class ConsensusSignerPort final {
 public:
  explicit ConsensusSignerPort(const FullNodeConfig& config);
  ~ConsensusSignerPort();

  ConsensusSignerPort(const ConsensusSignerPort&) = delete;
  ConsensusSignerPort(ConsensusSignerPort&&) = delete;
  ConsensusSignerPort& operator=(const ConsensusSignerPort&) = delete;
  ConsensusSignerPort& operator=(ConsensusSignerPort&&) = delete;

  rustaxa::HostSignReport consensusSignDigest(const rustaxa::HostSignRequest& request) const;
  rustaxa::HostVrfReport consensusProveVrf(const rustaxa::HostVrfRequest& request) const;

 private:
  class Impl;
  std::unique_ptr<Impl> impl_;
};

/**
 * Physical tarcap leaf for canonical native consensus egress.
 *
 * The port may decode canonical payloads only to satisfy the existing network
 * packet API. It owns no routing policy, peer selection, rebroadcast counters,
 * consensus queues, or protocol state. An unavailable network is reported as
 * an exact failed effect.
 */
class ConsensusTransportPort final {
 public:
  ConsensusTransportPort();
  ~ConsensusTransportPort();

  ConsensusTransportPort(const ConsensusTransportPort&) = delete;
  ConsensusTransportPort(ConsensusTransportPort&&) = delete;
  ConsensusTransportPort& operator=(const ConsensusTransportPort&) = delete;
  ConsensusTransportPort& operator=(ConsensusTransportPort&&) = delete;

  rustaxa::HostTransportReport consensusGossipVote(const rustaxa::HostGossipVoteRequest& request) const;
  rustaxa::HostTransportReport consensusGossipVoteBundle(const rustaxa::HostGossipVoteBundleRequest& request) const;
  rustaxa::HostTransportReport consensusGossipPillarVote(const rustaxa::HostGossipPillarVoteRequest& request) const;
  rustaxa::HostTransportReport consensusSetSyncPeriod(const rustaxa::HostSetSyncPeriodRequest& request) const;
  rustaxa::HostTransportStatus consensusTransportStatus() const;
  rustaxa::HostTransportReport consensusReportMaliciousPeer(const rustaxa::HostMaliciousPeerRequest& request) const;

 private:
  friend class ConsensusProcess;
  class Impl;
  std::unique_ptr<Impl> impl_;
};

/**
 * Concrete FinalChain/StateAPI execution leaf retained outside native consensus.
 *
 * Rust supplies one canonical, effect-identified finalization request. The port
 * performs only the existing concrete execution call and returns its exact
 * result; planning, ordering, retries, and cursor advancement remain native.
 */
class ExternalEvmPort final {
 public:
  explicit ExternalEvmPort(std::shared_ptr<final_chain::FinalChain> final_chain);
  ~ExternalEvmPort();

  ExternalEvmPort(const ExternalEvmPort&) = delete;
  ExternalEvmPort(ExternalEvmPort&&) = delete;
  ExternalEvmPort& operator=(const ExternalEvmPort&) = delete;
  ExternalEvmPort& operator=(ExternalEvmPort&&) = delete;

  rustaxa::HostEvmFinalizationReport consensusExecuteFinalization(
      const rustaxa::HostEvmFinalizationRequest& request) const;
  /** Loads the exact finalized header/bridge facts needed for pillar restart recovery. */
  rustaxa::HostPillarAnchorStateReport consensusLoadPillarAnchorState(
      const rustaxa::HostPillarAnchorStateRequest& request) const;
  /** Loads an ordered account batch from one exact FinalChain snapshot. */
  rustaxa::HostFinalChainAccountFactsReport consensusLoadFinalChainAccountFacts(
      const rustaxa::HostFinalChainAccountFactsRequest& request) const;

 private:
  class Impl;
  std::unique_ptr<Impl> impl_;
};

/**
 * App-owned process shell for the blocking native consensus runner.
 *
 * This type owns one worker thread plus the four exact host ports above. It
 * contains no PBFT actions, phases, manager references, protocol mirrors, or
 * materialized-object caches. Stop is interruptible, idempotent, and joins the
 * worker without holding the worker-state mutex. Concurrent lifecycle calls
 * are linearized so a start cannot overwrite a joinable worker or lose a stop.
 */
class ConsensusProcess final {
 public:
  ConsensusProcess(std::shared_ptr<ConsensusApplication> application, const FullNodeConfig& config,
                   std::shared_ptr<final_chain::FinalChain> final_chain);
  ~ConsensusProcess();

  ConsensusProcess(const ConsensusProcess&) = delete;
  ConsensusProcess(ConsensusProcess&&) = delete;
  ConsensusProcess& operator=(const ConsensusProcess&) = delete;
  ConsensusProcess& operator=(ConsensusProcess&&) = delete;

  /** Attaches the non-owning physical transport used by subsequent effects. */
  void attachNetwork(std::weak_ptr<Network> network);
  /** Starts one blocking native runner thread; concurrent starts are linearized and idempotent. */
  void start();
  /** Requests stop and joins the worker; concurrent lifecycle transitions are linearized. */
  void stop() noexcept;

 private:
  class Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace taraxa
