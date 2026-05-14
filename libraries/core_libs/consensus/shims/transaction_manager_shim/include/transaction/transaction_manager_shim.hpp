#pragma once

#include <utility>

#include "common/constants.hpp"

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
  TransactionManager(const FullNodeConfig &conf, std::shared_ptr<DbStorage> db,
                     std::shared_ptr<final_chain::FinalChain> final_chain, addr_t node_addr)
      : TransactionManagerOld(conf, std::move(db), std::move(final_chain), node_addr) {}

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

  std::vector<SharedTransactions> getAllPoolTrxs() {
    // TODO(rust-rewrite): migrate pool grouping to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getAllPoolTrxs();
  }

  /**
   * Persist transactions accepted by a DAG block.
   *
   * C++ applies live duplicate/finalized filtering and owns pool/cache mutation. Accepted
   * transaction RLP payloads and the target transaction count are committed through Rust
   * storage first; if that write fails, the live C++ transaction state is left unchanged.
   */
  void saveTransactionsFromDagBlock(const SharedTransactions &trxs);

  std::pair<bool, std::string> insertTransaction(const std::shared_ptr<Transaction> &trx) {
    // TODO(rust-rewrite): migrate transaction verification/insertion orchestration to Rust instead of
    // TransactionManagerOld.
    return TransactionManagerOld::insertTransaction(trx);
  }

  void blockFinalized(EthBlockNumber block_number) {
    // TODO(rust-rewrite): migrate finalized-block queue updates to Rust instead of TransactionManagerOld.
    TransactionManagerOld::blockFinalized(block_number);
  }

  TransactionStatus insertValidatedTransaction(std::shared_ptr<Transaction> &&tx, bool insert_non_proposable = true) {
    // TODO(rust-rewrite): migrate validated transaction insertion to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::insertValidatedTransaction(std::move(tx), insert_non_proposable);
  }

  bool isTransactionKnown(const trx_hash_t &trx_hash) {
    // TODO(rust-rewrite): migrate known-transaction checks to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::isTransactionKnown(trx_hash);
  }

  size_t getTransactionPoolSize() const {
    // TODO(rust-rewrite): migrate transaction pool sizing to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getTransactionPoolSize();
  }

  bool isTransactionPoolFull(size_t percentage = 100) const {
    // TODO(rust-rewrite): migrate pool fullness checks to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::isTransactionPoolFull(percentage);
  }

  bool nonProposableTransactionsOverTheLimit() const {
    // TODO(rust-rewrite): migrate non-proposable limit checks to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::nonProposableTransactionsOverTheLimit();
  }

  size_t getNonfinalizedTrxSize() const {
    // TODO(rust-rewrite): migrate non-finalized transaction sizing to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getNonfinalizedTrxSize();
  }

  std::vector<std::shared_ptr<Transaction>> getNonfinalizedTrx(const std::vector<trx_hash_t> &hashes) {
    // TODO(rust-rewrite): migrate non-finalized transaction lookup to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getNonfinalizedTrx(hashes);
  }

  std::unordered_set<trx_hash_t> excludeFinalizedTransactions(const std::vector<trx_hash_t> &hashes) {
    // TODO(rust-rewrite): migrate finalized transaction filtering to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::excludeFinalizedTransactions(hashes);
  }

  bool verifyTransactionsNotFinalized(const SharedTransactions &trxs) {
    // TODO(rust-rewrite): migrate finalized transaction verification to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::verifyTransactionsNotFinalized(trxs);
  }

  SharedTransactions getBlockTransactions(const DagBlock &blk, PbftPeriod proposal_period) {
    // TODO(rust-rewrite): migrate DAG block transaction materialization to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getBlockTransactions(blk, proposal_period);
  }

  SharedTransactions getTransactions(const vec_trx_t &trxs_hashes, PbftPeriod proposal_period) {
    // TODO(rust-rewrite): migrate transaction materialization to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getTransactions(trxs_hashes, proposal_period);
  }

  void updateFinalizedTransactionsStatus(const PeriodData &period_data) {
    // TODO(rust-rewrite): migrate finalized transaction status updates to Rust instead of TransactionManagerOld.
    TransactionManagerOld::updateFinalizedTransactionsStatus(period_data);
  }

  void initializeRecentlyFinalizedTransactions(const PeriodData &period_data) {
    // TODO(rust-rewrite): migrate recently-finalized transaction cache to Rust instead of TransactionManagerOld.
    TransactionManagerOld::initializeRecentlyFinalizedTransactions(period_data);
  }

  void removeNonFinalizedTransactions(std::unordered_set<trx_hash_t> &&transactions) {
    // TODO(rust-rewrite): migrate non-finalized transaction removal to Rust instead of TransactionManagerOld.
    TransactionManagerOld::removeNonFinalizedTransactions(std::move(transactions));
  }

  std::shared_mutex &getTransactionsMutex() {
    // TODO(rust-rewrite): migrate transaction lifecycle synchronization to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getTransactionsMutex();
  }

  std::pair<std::vector<std::shared_ptr<Transaction>>, std::vector<trx_hash_t>> getPoolTransactions(
      const std::vector<trx_hash_t> &trx_to_query) const {
    // TODO(rust-rewrite): migrate pool transaction lookup to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getPoolTransactions(trx_to_query);
  }

  bool transactionsDropped() const {
    // TODO(rust-rewrite): migrate dropped-transaction reporting to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::transactionsDropped();
  }

  val_t getMinGasPriceForBlockInclusion() const {
    // TODO(rust-rewrite): migrate minimum inclusion gas-price lookup to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getMinGasPriceForBlockInclusion();
  }

  std::shared_ptr<Transaction> getTransaction(const trx_hash_t &hash) const {
    // TODO(rust-rewrite): migrate transaction lookup to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getTransaction(hash);
  }

  std::shared_ptr<Transaction> getNonFinalizedTransaction(const trx_hash_t &hash) const {
    // TODO(rust-rewrite): migrate non-finalized transaction lookup to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getNonFinalizedTransaction(hash);
  }

  unsigned long getTransactionCount() const {
    // TODO(rust-rewrite): migrate transaction count reads to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::getTransactionCount();
  }

  void recoverNonfinalizedTransactions() {
    // TODO(rust-rewrite): migrate non-finalized transaction recovery to Rust instead of TransactionManagerOld.
    TransactionManagerOld::recoverNonfinalizedTransactions();
  }

  std::pair<bool, std::string> verifyTransaction(const std::shared_ptr<Transaction> &trx) const {
    // TODO(rust-rewrite): migrate transaction verification to Rust instead of TransactionManagerOld.
    return TransactionManagerOld::verifyTransaction(trx);
  }
};

}  // namespace taraxa
