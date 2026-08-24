#pragma once

#include <memory>
#include <optional>

#include "common/event.hpp"
#include "common/types.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"

namespace taraxa {

struct FullNodeConfig;
namespace final_chain {
class FinalChain;
}
class DagBlock;

/**
 * Stable public result for one native transaction-submission operation.
 *
 * The hash always identifies the decoded request. `accepted` and `message` are
 * the native admission outcome; `transaction_observed` is true only when that
 * operation committed a queue observation. Invalid C++ inputs fail before a
 * result is produced, while native admission rejection is represented here.
 */
struct PublicTransactionSubmissionResult {
  trx_hash_t transaction_hash;
  bool accepted{false};
  std::string message;
  bool transaction_observed{false};
};

/** Shared lifetime handle for the root-bound, read-only native public query API. */
using ConsensusQueryClient = std::shared_ptr<rust::Box<rustaxa::BridgeConsensusQueryApi>>;

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

  /** Returns the root-bound public query client without exposing native services. */
  ConsensusQueryClient queryClient() const noexcept { return query_client_; }

  /** Subscribable best-effort notification emitted after a native transaction commit requests public observation. */
  const auto& transactionObserved() const noexcept { return transaction_observed_; }
  /** Subscribable best-effort notification emitted after a native DAG commit requests public observation. */
  const auto& dagBlockObserved() const noexcept { return dag_block_observed_; }

  /** Publishes a post-commit transaction notification selected by a native network operation. */
  void publishTransactionObserved(const trx_hash_t& transaction_hash) const {
    transaction_observed_.emit(transaction_hash);
  }
  /** Publishes a post-commit DAG notification selected by a native network operation. */
  void publishDagBlockObserved(const std::shared_ptr<DagBlock>& block) const { dag_block_observed_.emit(block); }

  /**
   * Validates and admits one signed public transaction through the native application operation.
   *
   * Immutable chain policy and concrete EVM account facts are sampled at the call boundary. The returned observer bit
   * is set only after an accepted queue mutation and may be used for best-effort public notification.
   */
  PublicTransactionSubmissionResult submitTransaction(const SharedTransaction& transaction,
                                                      const FullNodeConfig& config,
                                                      const final_chain::FinalChain& final_chain) const;

  /** Returns one coherent application-root runtime status snapshot. */
  ConsensusRuntimeStatus runtimeStatus() const;
  /** Resolves the current node's DPoS votes for diagnostic metrics only. */
  std::optional<uint64_t> currentNodeVotesCount() const;
  /** Resolves total eligible DPoS votes for diagnostic metrics only. */
  std::optional<uint64_t> currentDposTotalVotesCount() const;

 private:
  rust::Box<rustaxa::BridgeConsensusApplication> service_;
  ConsensusQueryClient query_client_;
  util::event::Event<ConsensusApplication, trx_hash_t> transaction_observed_;
  util::event::Event<ConsensusApplication, std::shared_ptr<DagBlock>> dag_block_observed_;
};

using SharedConsensusApplication = std::shared_ptr<ConsensusApplication>;

/** Builds the sole native application root from immutable node configuration. */
SharedConsensusApplication createConsensusApplication(const FullNodeConfig& config);

}  // namespace taraxa
