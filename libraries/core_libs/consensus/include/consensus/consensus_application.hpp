#pragma once

#include <memory>
#include <optional>
#include <vector>

#include "common/event.hpp"
#include "common/types.hpp"
#include "final_chain/state_api_data.hpp"
#include "pillar_chain/pillar_block.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"

namespace rustaxa {
struct HostFinalChainFinalizeReport;
struct HostFinalChainFinalizeTask;
}  // namespace rustaxa

namespace taraxa {

struct FullNodeConfig;
class DagBlock;
class ExternalEvmPort;
class ExternalEvmStateOwner;

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

/** Durable FinalChain identity emitted after native publication. */
struct FinalizedBlockObservation {
  PbftPeriod period{0};
  blk_hash_t block_hash;
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
  ConsensusApplication(rust::Box<rustaxa::BridgeConsensusApplication> service,
                       std::shared_ptr<ExternalEvmStateOwner> external_evm_state);
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
  /** Subscribable best-effort notification emitted after native pillar finalization is durably acknowledged. */
  const auto& pillarBlockObserved() const noexcept { return pillar_block_observed_; }
  /** Subscribable durable block identity; payloads are loaded through the query client. */
  const auto& finalizedBlockObserved() const noexcept { return finalized_block_observed_; }

  /** Publishes a post-commit transaction notification selected by a native network operation. */
  void publishTransactionObserved(const trx_hash_t& transaction_hash) const {
    transaction_observed_.emit(transaction_hash);
  }
  /** Publishes a post-commit DAG notification selected by a native network operation. */
  void publishDagBlockObserved(const std::shared_ptr<DagBlock>& block) const { dag_block_observed_.emit(block); }
  /** Publishes canonical finalized pillar data after native durability and hash validation. */
  void publishPillarBlockObserved(const pillar_chain::PillarBlockData& block_data) const {
    pillar_block_observed_.emit(block_data);
  }
  /** Publishes a durable block identity without materializing consensus objects. */
  void publishFinalizedBlockObserved(PbftPeriod period, const blk_hash_t& block_hash) const {
    finalized_block_observed_.emit(FinalizedBlockObservation{period, block_hash});
  }

  /**
   * Validates and admits one signed public transaction through the native application operation.
   *
   * Immutable chain policy and concrete EVM account facts are sampled at the call boundary. The returned observer bit
   * is set only after an accepted queue mutation and may be used for best-effort public notification.
   */
  PublicTransactionSubmissionResult submitTransaction(const SharedTransaction& transaction,
                                                      const FullNodeConfig& config) const;

  /** Reads an account through the serialized concrete-EVM state owner. */
  std::optional<state_api::Account> getAccount(const addr_t& address,
                                               std::optional<EthBlockNumber> block_number = {}) const;
  /** Reads one contract-storage slot through the serialized concrete-EVM state owner. */
  h256 getAccountStorage(const addr_t& address, const u256& key, std::optional<EthBlockNumber> block_number = {}) const;
  /** Reads account code through the serialized concrete-EVM state owner. */
  bytes getCode(const addr_t& address, std::optional<EthBlockNumber> block_number = {}) const;
  /** Executes one read-only concrete/native EVM call without exposing StateAPI. */
  state_api::ExecutionResult call(const state_api::EVMTransaction& transaction,
                                  std::optional<EthBlockNumber> block_number = {}) const;
  /** Traces one exact transaction sequence against committed concrete state. */
  std::string trace(std::vector<state_api::EVMTransaction> state_transactions,
                    std::vector<state_api::EVMTransaction> transactions, EthBlockNumber block_number,
                    std::optional<state_api::Tracing> params = {}) const;
  /** Prunes concrete state and native finalized indexes at one application operation boundary. */
  void pruneFinalChain(EthBlockNumber block_number) const;

  /** Executes one canonical FinalChain operation through the native application root and exact concrete-EVM leaf. */
  rustaxa::HostFinalChainFinalizeReport finalize(ExternalEvmPort& external_evm,
                                                 rustaxa::HostFinalChainFinalizeTask task) const;

  /** Returns one coherent application-root runtime status snapshot. */
  ConsensusRuntimeStatus runtimeStatus() const;
  /** Resolves the current node's DPoS votes for diagnostic metrics only. */
  std::optional<uint64_t> currentNodeVotesCount() const;
  /** Resolves total eligible DPoS votes for diagnostic metrics only. */
  std::optional<uint64_t> currentDposTotalVotesCount() const;

  /** Atomically prunes native finalized light-node history while preserving the legacy retained-DAG-level boundary. */
  void pruneLightHistory(PbftPeriod end_period_exclusive, uint64_t dag_level_to_keep, bool live_cleanup,
                         uint64_t non_block_periods_to_keep) const;

 private:
  friend class ExternalEvmPort;
  friend std::shared_ptr<ConsensusApplication> createConsensusApplication(const FullNodeConfig& config);

  /** Completes a crash-interrupted concrete-EVM publication before the application escapes bootstrap. */
  void recoverExternalEvmPendingPublication(const std::shared_ptr<ConsensusApplication>& self);
  std::shared_ptr<ExternalEvmStateOwner> externalEvmStateOwner() const noexcept { return external_evm_state_; }

  rust::Box<rustaxa::BridgeConsensusApplication> service_;
  ConsensusQueryClient query_client_;
  std::shared_ptr<ExternalEvmStateOwner> external_evm_state_;
  util::event::Event<ConsensusApplication, trx_hash_t> transaction_observed_;
  util::event::Event<ConsensusApplication, std::shared_ptr<DagBlock>> dag_block_observed_;
  util::event::Event<ConsensusApplication, pillar_chain::PillarBlockData> pillar_block_observed_;
  util::event::Event<ConsensusApplication, FinalizedBlockObservation> finalized_block_observed_;
};

using SharedConsensusApplication = std::shared_ptr<ConsensusApplication>;

/** Builds the sole native application root from immutable node configuration. */
SharedConsensusApplication createConsensusApplication(const FullNodeConfig& config);

}  // namespace taraxa
