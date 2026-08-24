#pragma once

#include <libdevcore/Address.h>
#include <libdevcrypto/Common.h>

#include <memory>

#include "config/config.hpp"

namespace taraxa {
struct FullNodeConfig;
class Network;
#ifndef RUSTAXA_ENABLE
class TransactionManager;
class DagManager;
#endif
class DbStorage;
#ifdef RUSTAXA_ENABLE
class ConsensusApplication;
#endif
#ifndef RUSTAXA_ENABLE
class PbftManager;
class VoteManager;
class PbftChain;
#endif
#ifndef RUSTAXA_ENABLE
class DagBlockProposer;
#endif
#ifndef RUSTAXA_ENABLE
class GasPricer;
#endif
class Plugin;

/** Read-only live PBFT progress exposed without a consensus manager facade. */
struct PbftProgress {
  uint64_t finalized_period = 0;
  uint64_t non_empty_finalized_periods = 0;
};

namespace final_chain {
class FinalChain;
}
namespace pillar_chain {
class PillarChainManager;
}

namespace metrics {
class MetricsService;
}

class AppBase {
 public:
  AppBase() {}

  virtual ~AppBase() = default;

  virtual const FullNodeConfig &getConfig() const = 0;
  virtual FullNodeConfig &getMutableConfig() = 0;
  virtual std::shared_ptr<Network> getNetwork() const = 0;
#ifndef RUSTAXA_ENABLE
  virtual std::shared_ptr<TransactionManager> getTransactionManager() const = 0;
  virtual std::shared_ptr<DagManager> getDagManager() const = 0;
#endif
  virtual std::shared_ptr<DbStorage> getDB() const = 0;
#ifdef RUSTAXA_ENABLE
  /** Returns the native application root for Rust-mode fixtures and named executors. */
  virtual std::shared_ptr<ConsensusApplication> getConsensusApplication() const = 0;
  /** Starts only the App-owned native consensus process. */
  virtual void startConsensus() = 0;
  /** Stops only the App-owned native consensus process and joins its worker. */
  virtual void stopConsensus() = 0;
#endif
#ifndef RUSTAXA_ENABLE
  virtual std::shared_ptr<PbftManager> getPbftManager() const = 0;
  virtual std::shared_ptr<VoteManager> getVoteManager() const = 0;
  virtual std::shared_ptr<PbftChain> getPbftChain() const = 0;
#endif
  /** Returns one coherent live PBFT progress snapshot. */
  virtual PbftProgress getPbftProgress() const = 0;
  virtual std::shared_ptr<final_chain::FinalChain> getFinalChain() const = 0;
  virtual std::shared_ptr<metrics::MetricsService> getMetrics() const = 0;
#ifndef RUSTAXA_ENABLE
  // used only in pure-C++ reference tests
  virtual std::shared_ptr<DagBlockProposer> getDagBlockProposer() const = 0;
  virtual std::shared_ptr<GasPricer> getGasPricer() const = 0;
#endif

  const dev::Address &getAddress() const { return conf_.getFirstWallet().node_addr; }
  const Secret &getSecretKey() const { return conf_.getFirstWallet().node_secret; }
  vrf_wrapper::vrf_sk_t getVrfSecretKey() const { return conf_.getFirstWallet().vrf_secret; }

  virtual std::shared_ptr<pillar_chain::PillarChainManager> getPillarChainManager() const = 0;

  virtual std::shared_ptr<Plugin> getPlugin(const std::string &name) const = 0;

  bool isStarted() const { return started_; }

  virtual void start() = 0;

 protected:
  // configuration
  FullNodeConfig conf_;

  std::atomic_bool started_ = 0;
  std::atomic_bool stopped_ = true;
};

}  // namespace taraxa
