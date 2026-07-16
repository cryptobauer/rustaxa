#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>
#include <mutex>
#include <shared_mutex>
#include <string>
#include <unordered_set>
#include <utility>
#include <vector>

#include "common/constants.hpp"
#include "common/event.hpp"
#include "common/thread_pool.hpp"
#include "common/util.hpp"
#include "final_chain/final_chain.hpp"
#include "logger/logger.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"
#include "transaction/dag_transaction_service.hpp"
#include "transaction/transaction.hpp"
#include "transaction/transaction_manager_bridge_types.hpp"

namespace taraxa {

enum class TransactionStatus { Inserted = 0, InsertedNonProposable, Known, Overflow };

struct FullNodeConfig;
class DagBlock;
class DagManager;

/**
 * Rust-mode TransactionManager facade.
 *
 * This facade preserves the public `TransactionManager` API while moving deterministic
 * proposal selection planning and DAG-accepted transaction persistence to Rust-backed
 * planner/storage code.
 * C++ continues to own live `Transaction` materialization, EVM gas-estimation
 * execution, event dispatch, and logging.
 *
 * The facade is independent of the legacy implementation. Rust owns queue,
 * sidecar, transaction-count, gas-cache policy, and persistence state; C++ owns
 * only public object materialization plus event, logging, thread-pool, and
 * FinalChain/EVM executor boundaries.
 */
class TransactionManager : public std::enable_shared_from_this<TransactionManager> {
 public:
  /**
   * Rust-mode pending-transaction event surface.
   *
   * The shim owns the public event instance used by Rust-mode subscribers and
   * emits it only from shim-owned insertion paths selected by Rust admission
   * planning.
   */
  util::event::Event<TransactionManager, const trx_hash_t &> const transaction_added_{};

  TransactionManager(const FullNodeConfig &conf, std::shared_ptr<DbStorage> db,
                     std::shared_ptr<final_chain::FinalChain> final_chain, addr_t node_addr);

  /**
   * Constructs the production facade over an application-owned composed service.
   *
   * `dag_transaction_service` must be the same holder passed to `DagManager`.
   * The retained `db` argument preserves the public construction shape while
   * storage restoration has already completed in the Rust service factory.
   */
  TransactionManager(const FullNodeConfig &conf, std::shared_ptr<DbStorage> db,
                     std::shared_ptr<final_chain::FinalChain> final_chain, addr_t node_addr,
                     SharedDagTransactionService dag_transaction_service);

  TransactionManager(const TransactionManager &) = delete;
  TransactionManager(TransactionManager &&) = delete;
  TransactionManager &operator=(const TransactionManager &) = delete;
  TransactionManager &operator=(TransactionManager &&) = delete;

  /**
   * Estimate total gas for a transaction list through Rust-owned cache decisions.
   *
   * Rust owns declared-gas shortcut decisions, cache lookup, and cache insertion
   * policy. C++ keeps EVM execution and `ExecutionResult` materialization.
   */
  uint64_t estimateTransactions(const SharedTransactions &trxs, PbftPeriod proposal_period);

  /**
   * Estimate one transaction's gas through the Rust runtime cache.
   *
   * Rust decides whether declared gas or a cached opaque `ExecutionResult` can
   * satisfy the request. C++ calls FinalChain/EVM only on a Rust cache miss.
   */
  state_api::ExecutionResult estimateTransactionGas(std::shared_ptr<Transaction> trx, PbftPeriod proposal_period);

  /**
   * Select transactions to include in a proposed DAG block.
   *
   * Rust decides candidate scan sizing, declared-gas fit checks, invalid-estimate
   * demotion, accepted gas accumulation, and stop conditions. C++ supplies live
   * transactions and gas estimates and applies Rust's selected/demoted hash plan.
   */
  std::pair<SharedTransactions, std::vector<uint64_t>> packTrxs(PbftPeriod proposal_period, uint64_t weight_limit);

  /**
   * Select transactions for one DAG proposer shard.
   *
   * Rust owns the deterministic shard filter, candidate scan, gas-fit planning,
   * invalid-estimate demotion, accepted ordering, and stop conditions. C++ still
   * supplies live transaction materialization and FinalChain/EVM gas estimates.
   */
  std::pair<SharedTransactions, std::vector<uint64_t>> packShardedTrxs(PbftPeriod proposal_period,
                                                                       uint64_t weight_limit, uint16_t total_shards,
                                                                       uint16_t node_trx_shard,
                                                                       uint64_t shard_period_interval);

  /**
   * Executes the external EVM boundary for one Rust-owned DAG proposer pack.
   *
   * Rust derives all proposal and shard parameters from `session_id`. C++ keeps
   * the pack executor serialized across prepare, EVM estimation, and finalize,
   * and aborts the matching pack if an external estimate throws.
   */
  rustaxa::DagProposerSessionStep executeDagProposerTransactionPack(uint64_t session_id, bool network_throttled);

  /**
   * Return live transaction-pool groups under the transaction mutex.
   *
   * The Rust-mode shim keeps `SharedTransaction` ownership in C++ for this read-only helper.
   */
  std::vector<SharedTransactions> getAllPoolTrxs();

  /**
   * Persist transactions accepted by a DAG block.
   *
   * Compatibility adapter for callers that still hold public transaction objects.
   * C++ extracts only hash/RLP edge facts, then routes through the Rust-owned
   * canonical payload command path for inspection, filtering, persistence, and
   * live queue/sidecar mutation.
   */
  void saveTransactionsFromDagBlock(const SharedTransactions &trxs);

  /**
   * Persist transactions accepted by a DAG block from canonical payload facts.
   *
   * Rust inspects the supplied transaction RLP payloads, verifies each payload
   * against the supplied DAG transaction hash, owns duplicate/finalized
   * filtering, and commits the accepted storage/sidecar mutation. This avoids
   * materializing live `Transaction` objects for proposed DAG blocks whose
   * manager path already carries canonical transaction bytes.
   */
  void saveTransactionPayloadsFromDagBlock(const vec_trx_t &transaction_hashes,
                                           const std::vector<dev::bytes> &transaction_rlps);

  std::pair<bool, std::string> insertTransaction(const std::shared_ptr<Transaction> &trx);

  /**
   * Notify the Rust-owned transaction queue that an Ethereum block has finalized.
   *
   * Rust mode owns non-proposable expiry and returns explicit cleanup output while the shim keeps the public C++ API.
   */
  void blockFinalized(EthBlockNumber block_number);

  /**
   * Execute validated transaction admission through the Rust runtime.
   *
   * Rust sources latest account facts from the Rust FinalChain handle, decides known/proposable/non-proposable
   * admission, mutates the Rust-owned queue, and returns explicit status/event/log side effects. The shim keeps
   * `SharedTransaction` materialization, locking, logging mechanics, and event dispatch in C++.
   */
  TransactionStatus insertValidatedTransaction(std::shared_ptr<Transaction> &&tx, bool insert_non_proposable = true);

  /**
   * Query Rust-owned known-transaction admission state for one hash under the transaction mutex.
   */
  bool isTransactionKnown(const trx_hash_t &trx_hash);

  /**
   * Return the current live transaction-pool size.
   */
  size_t getTransactionPoolSize() const;

  /**
   * Return whether the live transaction pool has reached the configured percentage of capacity.
   */
  bool isTransactionPoolFull(size_t percentage = 100) const;

  /**
   * Return whether the live non-proposable queue has exceeded its configured limit.
   */
  bool nonProposableTransactionsOverTheLimit() const;

  /**
   * Return the number of Rust-owned live non-finalized DAG transaction sidecars.
   */
  size_t getNonfinalizedTrxSize() const;

  /**
   * Resolve hashes from Rust-owned live non-finalized DAG sidecars, preserving input order for hits.
   */
  std::vector<std::shared_ptr<Transaction>> getNonfinalizedTrx(const std::vector<trx_hash_t> &hashes);

  /**
   * Return hashes that are not finalized in the recent cache or Rust-backed transaction storage.
   *
   * The shim supplies recent-cache facts; Rust performs storage checks and returns indexed actions.
   */
  std::unordered_set<trx_hash_t> excludeFinalizedTransactions(const std::vector<trx_hash_t> &hashes);

  /**
   * Verify that all transactions are absent from recent-finalized cache and Rust-backed finalized storage.
   *
   * C++ supplies live transaction identities and senders; Rust sources latest account nonces and short-circuits on the
   * first finalized hash.
   */
  bool verifyTransactionsNotFinalized(const SharedTransactions &trxs);

  /**
   * Verify transaction identity facts and return Rust's typed finalized outcome.
   *
   * Purpose:
   * - Lets PBFT sync admission forward hash-specific finalized transaction
   *   warnings into the Rust PBFT sync planner instead of collapsing the result
   *   to a legacy boolean.
   *
   * Outputs:
   * - `is_finalized == false` when all input facts are accepted.
   * - Otherwise `hash`, `input_index`, and `source` identify the first
   *   finalized transaction selected by Rust.
   *
   * Edge behavior:
   * - Throws `DbException` if Rust returns an out-of-range index or mismatched
   *   hash for the supplied facts.
   */
  rustaxa::TransactionManagerVerifyNotFinalizedOutcome verifyTransactionsNotFinalizedDetailed(
      std::vector<TransactionManagerVerifyNotFinalizedInput> &&facts);

  /**
   * Materialize DAG block transactions from live C++ views and Rust-backed storage.
   *
   * C++ preserves live pool identity and materializes non-finalized/recently-finalized
   * sidecar hits from Rust-retained canonical RLP. Rust resolves storage misses
   * and classifies regular versus system finalized payloads; C++ constructs the
   * transaction objects and applies proposal-period nonce filtering.
   */
  SharedTransactions getBlockTransactions(const DagBlock &blk, PbftPeriod proposal_period);

  /**
   * Materialize ordered transaction hashes from live C++ views and Rust-backed storage.
   *
   * Missing hashes are skipped. Storage or RLP corruption throws before any live
   * sidecar mutation because this lookup is read-only.
   */
  SharedTransactions getTransactions(const vec_trx_t &trxs_hashes, PbftPeriod proposal_period);

  /**
   * Apply finalized-status transitions using Rust planner and sidecar state.
   *
   * Compatibility adapter for callers that still hold `PeriodData`.
   * C++ extracts only finalized transaction hashes/RLP payloads and the period,
   * then routes through Rust-owned payload inspection, status planning,
   * recently-finalized sidecar retention, non-finalized sidecar removal,
   * known-cache marking, and live queue cleanup.
   */
  void updateFinalizedTransactionsStatus(const PeriodData &period_data);

  /**
   * Apply finalized-status transitions and return the Rust-owned command report.
   *
   * This is the same Rust-backed mutation as `updateFinalizedTransactionsStatus`,
   * but exposes the typed report so PBFT finalization can prove the live mutation
   * back to its Rust runtime cursor before advancing.
   */
  rustaxa::TransactionManagerFinalizedStatusCommandReport updateFinalizedTransactionsStatusForPbftFinalization(
      const PeriodData &period_data);

  /**
   * Warm Rust-owned recently-finalized sidecars from canonical period-data RLP payloads.
   *
   * `PeriodData` is only an edge adapter: C++ extracts hashes/RLP bytes, Rust
   * re-inspects payloads and mutates sidecar state from canonical facts.
   */
  void initializeRecentlyFinalizedTransactions(const PeriodData &period_data);

  void removeNonFinalizedTransactions(std::unordered_set<trx_hash_t> &&transactions);

  std::shared_mutex &getTransactionsMutex();

  std::pair<std::vector<std::shared_ptr<Transaction>>, std::vector<trx_hash_t>> getPoolTransactions(
      const std::vector<trx_hash_t> &trx_to_query) const;

  /**
   * Return whether the live transaction queue has dropped transactions.
   */
  bool transactionsDropped() const;

  /**
   * Return the live queue's minimum gas price for compatibility callers.
   *
   * New Rust-mode pricing must use `gasPriceBid`, which keeps the queue floor
   * and oracle combination inside the combined runtime. This method exposes the
   * raw queue floor only to preserve the stable public API.
   */
  val_t getMinGasPriceForBlockInclusion() const;

  /**
   * Return the combined runtime's current gas-price bid.
   *
   * Rust selects block-history or pool mode from the runtime-owned configuration.
   * Pool mode derives the queue floor using the runtime-owned proposal gas limit;
   * neither the mode selector nor a queue-derived scalar crosses back through C++.
   *
   * @return the current Rust-owned bid as a C++ compatibility value
   */
  val_t gasPriceBid() const;

  /**
   * Update the combined runtime's historical gas-price oracle from a finalized block.
   *
   * C++ extracts only each transaction's canonical gas price from the finalized
   * callback payload. Empty blocks are no-ops; bridge failures propagate.
   *
   * @param trxs finalized block transactions whose gas prices seed the next bid
   */
  void updateGasPrice(const SharedTransactions &trxs);

  /**
   * Lookup one transaction by hash from live C++ views or Rust-backed storage.
   *
   * Pool, non-finalized, and recently-finalized cache hits return their live
   * `SharedTransaction` object. Cache misses are resolved by Rust storage lookup;
   * missing hashes return `nullptr`, and storage/RLP corruption raises `DbException`.
   */
  std::shared_ptr<Transaction> getTransaction(const trx_hash_t &hash) const;

  /**
   * Resolve one hash from Rust-owned live non-finalized sidecars and materialize it in C++.
   */
  std::shared_ptr<Transaction> getNonFinalizedTransaction(const trx_hash_t &hash) const;

  /**
   * Return the Rust-owned Rust-mode transaction count from the manager runtime.
   *
   * The count value is seeded from storage and updated only inside Rust runtime persistence/finalization outcomes.
   */
  unsigned long getTransactionCount() const;

  /**
   * Rebuild non-finalized transaction sidecars from Rust-backed storage on startup.
   *
   * Rust loads persisted payloads keyed by hash, removes stale finalized rows,
   * validates survivor RLP hash and sender facts, and inserts survivor payloads
   * into the runtime sidecars without returning count mirrors to C++.
   */
  void recoverNonfinalizedTransactions();

  std::pair<bool, std::string> verifyTransaction(const std::shared_ptr<Transaction> &trx) const;

 private:
  friend class DagManager;
  friend class TransactionManagerRustShimAccess;

  /**
   * Resolve the active DAG verification session's transaction query through the composed Rust service.
   *
   * Rust owns the pending query hashes, proposal period, queue/sidecar/storage precedence, nonce filtering, and verify
   * cursor. This adapter first obtains non-advancing ordered views, validates and materializes every payload, then
   * reads each materialized sender's FinalChain account at that exact proposal period. Rust advances only after
   * revalidating the cursor and those narrow nonce facts. Missing views are omitted from the returned transactions;
   * malformed views, account lookup failures, or bridge/storage failures throw without advancing the transaction-query
   * step.
   */
  std::pair<rustaxa::DagVerifyBlockSessionStep, SharedTransactions> executeDagVerifyTransactionAvailability() const;

  /**
   * Emit the Rust-mode pending-transaction event.
   *
   * Only shim-owned insertion code calls this helper, so subscribers attached
   * to the Rust-mode facade observe Rust-planned proposable admissions.
   */
  void emitTransactionAddedForRust(const trx_hash_t &trx_hash) const { transaction_added_.emit(trx_hash); }

  /**
   * Application-owned composed service containing live transaction runtime state.
   *
   * The shared service owns the authoritative Rust-mode transaction count,
   * queue metadata/payloads, known-admission cache, non-finalized and
   * recently-finalized sidecars, and gas-estimation cache policy. C++ keeps
   * object materialization, event emission, logging, EVM estimation execution,
   * historical/proposal-period account reads, and lifecycle orchestration.
   */
  const FullNodeConfig &kConf;
  static constexpr uint64_t kEstimateGasLimit = 200000;
  std::shared_ptr<final_chain::FinalChain> final_chain_;
  SharedDagTransactionService dag_transaction_service_;
  util::ThreadPool estimation_thread_pool_;
  mutable std::shared_mutex transactions_mutex_;
  mutable std::mutex pack_mutex_;

  LOG_OBJECTS_DEFINE
};

}  // namespace taraxa
