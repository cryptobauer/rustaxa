#pragma once

#include <mutex>
#include <unordered_set>
#include <utility>
#include <vector>

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
   * Rust-selected proposal transaction payloads.
   *
   * Rust returns the authoritative accepted ordering as hashes, canonical transaction RLP payloads, and gas estimates
   * from one runtime packing session. Consumers should use these facts for deterministic proposal planning and only
   * materialize live `Transaction` instances at temporary compatibility sidecars.
   */
  struct PackedProposalTransactions {
    std::vector<trx_hash_t> transaction_hashes;
    std::vector<dev::bytes> transaction_rlps;
    std::vector<uint64_t> gas_estimations;
  };

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
        runtime_(rustaxa::create_transaction_manager_runtime_from_storage(
            db->rustStorage(), db->getStatusField(StatusDbField::TrxCount),
            rustaxa::TransactionQueueConfig{conf.transactions_pool_size})) {}

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
   * Select proposer transactions and preserve Rust-owned canonical payload facts.
   *
   * The returned hashes, RLP payloads, and gas estimates come directly from the Rust packing session. The live
   * transaction objects are a temporary compatibility sidecar for DAG insertion paths that have not yet moved to block
   * intents over canonical payloads.
   */
  PackedProposalTransactions packShardedTransactionPayloads(PbftPeriod proposal_period, uint64_t weight_limit,
                                                            uint16_t total_shards, uint16_t node_trx_shard,
                                                            uint64_t shard_period_interval);

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
   * Verify transaction identity facts retained by Rust sync queue metadata.
   *
   * This is the same Rust runtime decision as `verifyTransactionsNotFinalized`,
   * but the caller supplies pre-inspected hash/sender/nonce facts so PBFT sync
   * admission does not reopen live `Transaction` objects only to build those
   * facts.
   */
  bool verifyTransactionsNotFinalized(
      ::rust::Vec<rustaxa::TransactionManagerVerifyNotFinalizedRuntimeFact> &&facts);
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
      ::rust::Vec<rustaxa::TransactionManagerVerifyNotFinalizedRuntimeFact> &&facts);

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
   * Apply finalized-status transitions and return the Rust-owned command report.
   *
   * This is the same Rust-backed mutation as `updateFinalizedTransactionsStatus`,
   * but exposes the typed report so PBFT finalization can prove the live mutation
   * back to its Rust runtime cursor before advancing.
   */
  rustaxa::TransactionManagerFinalizedStatusCommandReport updateFinalizedTransactionsStatusForPbftFinalization(
      const PeriodData &period_data);

  /**
   * Apply finalized-status transitions and return a Rust-verifiable PBFT finalization live-action report.
   *
   * Inputs are the finalized period data and the Rust-planned finalization write intent. The returned report carries
   * post-mutation transaction counts that Rust validates before the PBFT runtime cursor advances.
   */
  rustaxa::PbftFinalizationLiveMutationReport updateFinalizedTransactionsStatusForPbftFinalization(
      const PeriodData &period_data, const rustaxa::PbftFinalizationStorageWritePlan &write_intent);

  /**
   * Warm Rust-owned recently-finalized sidecars from canonical period-data RLP payloads.
   */
  void initializeRecentlyFinalizedTransactions(const PeriodData &period_data);

  void removeNonFinalizedTransactions(std::unordered_set<trx_hash_t> &&transactions);

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
   * sidecars, and gas-estimation cache policy. C++ keeps object materialization,
   * event emission, logging, EVM estimation execution, historical/proposal-period
   * account reads, and lifecycle orchestration.
   */
  ::rust::Box<rustaxa::BridgeTransactionManagerRuntime> runtime_;
  mutable std::mutex pack_mutex_;
};

}  // namespace taraxa
