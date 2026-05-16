#pragma once

#include <unordered_set>
#include <utility>

#include "common/constants.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

/**
 * Rust-mode TransactionManager facade.
 *
 * This facade preserves the public `TransactionManager` API while moving deterministic
 * proposal selection planning and DAG-accepted transaction persistence to Rust-backed
 * planner/storage code.
 * C++ continues to own live `Transaction` objects, gas estimation, and stateful
 * transaction lifecycle cache handling.
 *
 * The facade currently inherits the legacy storage/lifecycle implementation so existing
 * runtime wiring stays intact. `packTrxs` and `saveTransactionsFromDagBlock` are redeclared
 * because proposal packing and DAG transaction persistence are the migrated surfaces for
 * this slice; remaining inherited APIs are tracked TransactionManager migration work, not a
 * permanent Rust fallback boundary.
 */
class TransactionManager : public TransactionManagerOld {
 public:
  /**
   * Rust-mode pending-transaction event surface.
   *
   * The legacy owner type keeps `emit` private to `TransactionManagerOld`, so the shim owns
   * the public event instance used by Rust-mode subscribers and emits it only from
   * shim-owned insertion paths selected by Rust admission planning.
   */
  util::event::Event<TransactionManager, const trx_hash_t &> const transaction_added_{};

  TransactionManager(const FullNodeConfig &conf, std::shared_ptr<DbStorage> db,
                     std::shared_ptr<final_chain::FinalChain> final_chain, addr_t node_addr)
      : TransactionManagerOld(conf, db, std::move(final_chain), node_addr),
        runtime_(rustaxa::create_transaction_manager_runtime(db->getStatusField(StatusDbField::TrxCount),
                                                             rustaxa::TransactionQueueConfig{
                                                                 conf.transactions_pool_size})) {}

  TransactionManager(const TransactionManager &) = delete;
  TransactionManager(TransactionManager &&) = delete;
  TransactionManager &operator=(const TransactionManager &) = delete;
  TransactionManager &operator=(TransactionManager &&) = delete;

  uint64_t estimateTransactions(const SharedTransactions &trxs, PbftPeriod proposal_period) {
    // TODO(rust-rewrite): migrate estimation orchestration to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::estimateTransactions(trxs, proposal_period);
  }

  state_api::ExecutionResult estimateTransactionGas(std::shared_ptr<Transaction> trx, PbftPeriod proposal_period) {
    // TODO(rust-rewrite): migrate estimation/cache ownership to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::estimateTransactionGas(std::move(trx), proposal_period);
  }

  /**
   * Select transactions to include in a proposed DAG block.
   *
   * Rust decides candidate scan sizing, declared-gas fit checks, invalid-estimate
   * demotion, accepted gas accumulation, and stop conditions. C++ supplies live
   * transactions and gas estimates and applies Rust's selected/demoted hash plan.
   */
  std::pair<SharedTransactions, std::vector<uint64_t>> packTrxs(PbftPeriod proposal_period, uint64_t weight_limit);

  /**
   * Return live transaction-pool groups under the transaction mutex.
   *
   * The Rust-mode shim keeps `SharedTransaction` ownership in C++ for this read-only helper.
   */
  std::vector<SharedTransactions> getAllPoolTrxs();

  /**
   * Persist transactions accepted by a DAG block.
   *
   * C++ applies live duplicate/finalized filtering and owns pool/cache mutation. Accepted
   * transaction RLP payloads and the target transaction count are committed through Rust
   * storage first; if that write fails, the live C++ transaction state is left unchanged.
   */
  void saveTransactionsFromDagBlock(const SharedTransactions &trxs);

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
   * Rust owns status planning, recently-finalized sidecar retention,
   * non-finalized sidecar removal, known-cache marking, and live queue cleanup.
   * Rust also sources finalized-account purge facts from the Rust FinalChain
   * runtime; C++ logs returned side effects.
   */
  void updateFinalizedTransactionsStatus(const PeriodData &period_data);

  /**
   * Warm Rust-owned recently-finalized sidecars from canonical period-data RLP payloads.
   */
  void initializeRecentlyFinalizedTransactions(const PeriodData &period_data);

  void removeNonFinalizedTransactions(std::unordered_set<trx_hash_t> &&transactions);

  /**
   * Erase live C++ sidecars for expired non-finalized DAG transactions.
   *
   * Rust finalization has already removed the matching payloads from
   * non-finalized storage before this method is called. This method performs no
   * DB writes and is idempotent for hashes that are no longer present in the
   * live sidecar map. The caller must hold the finalization transaction lock.
   */
  void forgetExpiredNonFinalizedTransactionSidecars(std::unordered_set<trx_hash_t> &&transactions);

  std::shared_mutex &getTransactionsMutex() {
    // TODO(rust-rewrite): migrate transaction lifecycle synchronization to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getTransactionsMutex();
  }

  std::pair<std::vector<std::shared_ptr<Transaction>>, std::vector<trx_hash_t>> getPoolTransactions(
      const std::vector<trx_hash_t> &trx_to_query) const;

  /**
   * Return whether the live transaction queue has dropped transactions.
   */
  bool transactionsDropped() const;

  /**
   * Return the live queue's minimum gas price for inclusion in the configured DAG gas limit.
   */
  val_t getMinGasPriceForBlockInclusion() const;

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
   * Return the Rust-owned Rust-mode transaction count cached by the manager.
   *
   * The count value is updated from Rust-planned persistence/finalization outcomes.
   */
  unsigned long getTransactionCount() const;

  /**
   * Rebuild non-finalized transaction sidecars from Rust-backed storage on startup.
   *
   * Rust returns persisted payloads keyed by hash and removes stale finalized rows.
   * C++ constructs survivor `Transaction` objects, validates hash/RLP consistency,
   * warms sender caches, and mutates the live sidecar map only after materialization
   * succeeds.
   */
  void recoverNonfinalizedTransactions();

  std::pair<bool, std::string> verifyTransaction(const std::shared_ptr<Transaction> &trx) const;

 private:
  friend class TransactionManagerRustShimAccess;

  /**
   * Emit the Rust-mode pending-transaction event.
   *
   * Only shim-owned insertion code calls this helper. It intentionally emits the facade event
   * rather than the inherited legacy event so subscribers attached to the Rust-mode
   * `TransactionManager` observe Rust-planned proposable admissions without a legacy owner hook.
   */
  void emitTransactionAddedForRust(const trx_hash_t &trx_hash) const { transaction_added_.emit(trx_hash); }

  /**
   * Rust-owned live TransactionManager runtime state.
   *
   * The handle owns the authoritative Rust-mode transaction count, transaction queue
   * metadata/payloads, known-admission cache, and non-finalized/recently-finalized
   * sidecars. C++ keeps object materialization, event emission, logging, gas
   * estimation, historical/proposal-period account reads, and lifecycle orchestration.
   */
  ::rust::Box<rustaxa::BridgeTransactionManagerRuntime> runtime_;
};

}  // namespace taraxa
