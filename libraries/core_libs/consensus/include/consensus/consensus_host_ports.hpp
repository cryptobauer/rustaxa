#pragma once

#include <cstdint>
#include <memory>

namespace rustaxa {
struct HostFinalChainExecutionReport;
struct HostFinalChainExecutionRequest;
struct HostFinalChainPreflightReport;
struct HostFinalChainPreflightRequest;
struct HostFinalChainRewardsReport;
struct HostFinalChainRewardsRequest;
struct HostFinalChainStateCommitReport;
struct HostFinalChainStateCommitRequest;
struct HostFinalChainSystemFactsReport;
struct HostFinalChainSystemFactsRequest;
struct HostDagGasBatch;
struct HostFinalChainAccountFactsReport;
struct HostFinalChainAccountFactsRequest;
struct HostGossipPillarVoteRequest;
struct HostGossipDagBlockRequest;
struct HostGossipVoteBundleRequest;
struct HostGossipVoteRequest;
struct HostDagVdfRequest;
struct HostDagVdfStartReport;
struct HostDagVdfJobRequest;
struct HostDagVdfPollReport;
struct HostDagVdfCancelReport;
struct HostConsensusObservationReport;
struct HostConsensusObservationRequest;
struct HostMaliciousPeerRequest;
struct HostPillarAnchorStateReport;
struct HostPillarAnchorStateRequest;
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
 * Existing host-process bridge handle used by the native consensus runner.
 *
 * The port owns monotonic-clock and condition-variable process mechanics plus
 * private, operation-specific VDF and public-observer executors. Sharing this
 * one CXX handle does not merge their state: every operation echoes its native
 * effect/job identity, stop wakes active waits, VDF cancellation joins its
 * exact job, and observation remains best-effort. No protocol phase, queue,
 * timer cursor, DAG state, or public-event ordering cursor is retained here.
 */
class ConsensusProcessPort final {
 public:
  /** Constructs clock/wait mechanics only for focused host-process tests. */
  ConsensusProcessPort();
  ConsensusProcessPort(const FullNodeConfig& config, std::shared_ptr<ConsensusApplication> application);
  ~ConsensusProcessPort();

  ConsensusProcessPort(const ConsensusProcessPort&) = delete;
  ConsensusProcessPort(ConsensusProcessPort&&) = delete;
  ConsensusProcessPort& operator=(const ConsensusProcessPort&) = delete;
  ConsensusProcessPort& operator=(ConsensusProcessPort&&) = delete;

  rustaxa::HostWaitReport consensusWait(const rustaxa::HostWaitRequest& request) const;
  bool consensusStopRequested(uint64_t generation) const;
  uint64_t consensusNowMillis() const;
  uint64_t consensusUnixTimeSeconds() const;
  /** Starts one exact asynchronous DAG-proposer VDF job. */
  rustaxa::HostDagVdfStartReport consensusStartDagVdf(const rustaxa::HostDagVdfRequest& request) const;
  /** Polls one exact asynchronous DAG-proposer VDF job without blocking. */
  rustaxa::HostDagVdfPollReport consensusPollDagVdf(const rustaxa::HostDagVdfJobRequest& request) const;
  /** Cancels and joins one exact asynchronous DAG-proposer VDF job. */
  rustaxa::HostDagVdfCancelReport consensusCancelDagVdf(const rustaxa::HostDagVdfJobRequest& request) const;
  /** Publishes one best-effort post-commit public observation. */
  rustaxa::HostConsensusObservationReport consensusObserve(
      const rustaxa::HostConsensusObservationRequest& request) const;

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
  rustaxa::HostTransportReport consensusGossipDagBlock(const rustaxa::HostGossipDagBlockRequest& request) const;
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
  explicit ExternalEvmPort(final_chain::FinalChain& final_chain);
  ~ExternalEvmPort();

  ExternalEvmPort(const ExternalEvmPort&) = delete;
  ExternalEvmPort(ExternalEvmPort&&) = delete;
  ExternalEvmPort& operator=(const ExternalEvmPort&) = delete;
  ExternalEvmPort& operator=(ExternalEvmPort&&) = delete;

  rustaxa::HostFinalChainSystemFactsReport consensusLoadFinalChainSystemFacts(
      const rustaxa::HostFinalChainSystemFactsRequest& request) const;
  rustaxa::HostFinalChainPreflightReport consensusLoadFinalChainCommittedState(
      const rustaxa::HostFinalChainPreflightRequest& request) const;
  rustaxa::HostFinalChainExecutionReport consensusExecuteFinalChainTransactions(
      const rustaxa::HostFinalChainExecutionRequest& request) const;
  rustaxa::HostFinalChainRewardsReport consensusDistributeFinalChainRewards(
      const rustaxa::HostFinalChainRewardsRequest& request) const;
  rustaxa::HostFinalChainStateCommitReport consensusCommitFinalChainState(
      const rustaxa::HostFinalChainStateCommitRequest& request) const;
  /** Loads the exact finalized header/bridge facts needed for pillar restart recovery. */
  rustaxa::HostPillarAnchorStateReport consensusLoadPillarAnchorState(
      const rustaxa::HostPillarAnchorStateRequest& request) const;
  /** Loads an ordered account batch from one exact FinalChain snapshot. */
  rustaxa::HostFinalChainAccountFactsReport consensusLoadFinalChainAccountFacts(
      const rustaxa::HostFinalChainAccountFactsRequest& request) const;
  /**
   * Estimates one ordered canonical transaction batch against concrete FinalChain state.
   *
   * The report echoes the exact effect identity and sampled FinalChain block, preserves input hash order, and carries
   * each complete ExecutionResult as canonical RLP. Malformed payloads, hash mismatches, unavailable periods, and EVM
   * failures produce a failed report without partial estimates.
   */
  rustaxa::HostDagGasBatch consensusEstimateDagTransactionGas(const rustaxa::HostDagGasBatch& request) const;

 private:
  class Impl;
  std::unique_ptr<Impl> impl_;
};

/**
 * App-owned process shell for the blocking native consensus runner.
 *
 * This type owns one worker thread plus the exact host ports above. It
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
