#pragma once

#include <mutex>
#include <unordered_map>
#include <utility>

#include "common/types.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {
namespace fs = std::filesystem;

// Rust-mode storage shim facade.
// Public APIs with existing Rust shims are explicitly redeclared here so callers
// go through this layer first. Redeclared APIs are implemented in storage_shim.cpp
// and call Rust storage directly.
//
// Compatibility boundary markers used by the consensus rewrite:
// - RUSTAXA_QUERY_COMPAT_READ marks public/debug/network query surfaces that
//   temporarily materialize legacy C++ objects from Rust-backed storage.
// - RUSTAXA_ADMIN_COMPAT_UNSUPPORTED marks app/admin/lifecycle maintenance APIs
//   that are intentionally unsupported in Rust shim mode until that subsystem is
//   rewritten.
// - RUSTAXA_ADMIN_COMPAT_LEGACY_ONLY marks legacy/reference migration behavior
//   that must not become production Rust consensus storage authority.
class DbStorage : public DbStorageOld {
 public:
  explicit DbStorage(fs::path const& path, uint32_t db_snapshot_each_n_pbft_block = 0, uint32_t max_open_files = 0,
                     uint32_t db_max_snapshots = 0, PbftPeriod db_revert_to_period = 0, addr_t node_addr = addr_t(),
                     bool rebuild = false);

  ~DbStorage();

  static Batch createWriteBatch();
  void commitWriteBatch(Batch& write_batch, const rocksdb::WriteOptions& opts);
  void commitWriteBatch(Batch& write_batch);
  void updateDbVersions();
  void DeleteRange(const Column& col, uint64_t begin, uint64_t end);
  void CompactRange(const Column& col, uint64_t begin, uint64_t end);
  void deleteColumnData(const Column& c);
  bool createSnapshot(PbftPeriod period);
  void disableSnapshots();
  void enableSnapshots();
  uint32_t getMajorVersion() const;
  std::unique_ptr<rocksdb::Iterator> getColumnIterator(const Column& c);
  std::unique_ptr<rocksdb::Iterator> getColumnIterator(rocksdb::ColumnFamilyHandle* c);

  template <typename K>
  std::string lookup(K const& key, Column const& column) const {
    auto const key_slice = toSlice(key);
    if (column.ordinal_ == Columns::final_chain_meta.ordinal_) {
      return lookupFinalChainMeta(key_slice);
    }
    if (column.ordinal_ == Columns::final_chain_blk_by_number.ordinal_) {
      return lookupFinalChainBlockByNumber(key_slice);
    }
    if (column.ordinal_ == Columns::final_chain_blk_hash_by_number.ordinal_) {
      return lookupFinalChainBlockHashByNumber(key_slice);
    }
    if (column.ordinal_ == Columns::final_chain_blk_number_by_hash.ordinal_) {
      return lookupFinalChainBlockNumberByHash(key_slice);
    }
    if (column.ordinal_ == Columns::final_chain_log_blooms_index.ordinal_) {
      return lookupFinalChainLogBloomsChunk(key_slice);
    }
    if (column.ordinal_ == Columns::final_chain_receipt_by_trx_hash.ordinal_) {
      return lookupFinalChainReceiptByTrxHash(key_slice);
    }
    if (column.ordinal_ == Columns::migrations.ordinal_) {
      // RUSTAXA_ADMIN_COMPAT_LEGACY_ONLY: migration execution stays C++-owned and out of Rust shim scope. Returning a
      // truthy bool payload makes migration::Base::isApplied() skip migration work without adding Rust support for the
      // migrations column.
      return std::string(1, '\1');
    }
    throw DbException("DbStorage::lookup unsupported column in Rust shim mode: " + column.name());
  }

  template <typename Int, typename K>
  auto lookup_int(K const& key, Column const& column) -> std::enable_if_t<std::is_integral_v<Int>, std::optional<Int>> {
    auto str = lookup(key, column);
    if (str.empty()) {
      return std::nullopt;
    }
    return *reinterpret_cast<Int*>(str.data());
  }

  template <typename K>
  bool exist(K const& key, Column const& column) {
    (void)key;
    (void)column;
    throw DbException(
        "DbStorage::exist is a RUSTAXA_QUERY_COMPAT_READ boundary without a generic Rust shim implementation");
  }

  void setGenesisHash(const h256& genesis_hash);
  std::optional<h256> getGenesisHash();

  std::shared_ptr<DagBlock> getDagBlock(blk_hash_t const& hash);
  bool dagBlockInDb(blk_hash_t const& hash);
  std::set<blk_hash_t> getBlocksByLevel(level_t level);
  level_t getLastBlocksLevel() const;
  std::vector<std::shared_ptr<DagBlock>> getDagBlocksAtLevel(level_t level, int number_of_levels);
  std::map<level_t, std::vector<std::shared_ptr<DagBlock>>> getNonfinalizedDagBlocks();
  SharedTransactions getAllNonfinalizedTransactions();
  void removeDagBlock(blk_hash_t const& hash);
  void updateDagBlockCounters(std::vector<std::shared_ptr<DagBlock>> blks);
  void mirrorDagBlockCounters(uint64_t dag_blocks_count, uint64_t dag_edge_count);
  uint64_t getDagBlocksCount() const;
  uint64_t getDagEdgeCount() const;
  void saveDagBlock(const std::shared_ptr<DagBlock>& blk, Batch* write_batch_p = nullptr);

  void saveSortitionParamsChange(PbftPeriod period, const SortitionParamsChange& params, Batch& batch);
  std::deque<SortitionParamsChange> getLastSortitionParams(size_t count);
  std::optional<SortitionParamsChange> getParamsChangeForPeriod(PbftPeriod period);

  void savePeriodData(const PeriodData& period_data, Batch& write_batch);
  dev::bytes getPeriodDataRaw(PbftPeriod period) const;
  std::optional<PeriodData> getPeriodData(PbftPeriod period) const;
  std::optional<PbftBlock> getPbftBlock(PbftPeriod period) const;
  std::vector<std::shared_ptr<PbftVote>> getPeriodCertVotes(PbftPeriod period) const;
  blk_hash_t getPeriodBlockHash(PbftPeriod period) const;
  SharedTransactions transactionsFromPeriodDataRlp(PbftPeriod period, const dev::RLP& period_data_rlp) const;
  std::optional<SharedTransactions> getPeriodTransactions(PbftPeriod period) const;
  std::vector<std::shared_ptr<PillarVote>> getPeriodPillarVotes(PbftPeriod period) const;
  uint64_t getEarliestBlockNumber() const;

  void savePillarBlock(const std::shared_ptr<pillar_chain::PillarBlock>& pillar_block);
  std::shared_ptr<pillar_chain::PillarBlock> getPillarBlock(PbftPeriod period) const;
  std::shared_ptr<pillar_chain::PillarBlock> getLatestPillarBlock() const;
  void saveOwnPillarBlockVote(const std::shared_ptr<PillarVote>& vote);
  std::shared_ptr<PillarVote> getOwnPillarBlockVote() const;
  void saveCurrentPillarBlockData(const pillar_chain::CurrentPillarBlockDataDb& current_pillar_block_data);
  std::optional<pillar_chain::CurrentPillarBlockDataDb> getCurrentPillarBlockData() const;

  void addTransactionLocationToBatch(Batch& write_batch, trx_hash_t const& trx_hash, PbftPeriod period,
                                     uint32_t position, bool is_system = false);
  std::optional<TransactionLocation> getTransactionLocation(trx_hash_t const& hash) const;
  std::vector<bool> transactionsFinalized(std::vector<trx_hash_t> const& trx_hashes);
  std::unordered_map<trx_hash_t, PbftPeriod> getAllTransactionPeriod();

  void saveProposedPbftBlock(const std::shared_ptr<PbftBlock>& block);
  void removeProposedPbftBlock(const blk_hash_t& block_hash, Batch& write_batch);
  std::vector<std::shared_ptr<PbftBlock>> getProposedPbftBlocks();

  std::shared_ptr<Transaction> getTransaction(trx_hash_t const& hash) const;
  std::shared_ptr<Transaction> getTransaction(PbftPeriod period, uint32_t position) const;
  uint64_t getTransactionCount(PbftPeriod period) const;
  SharedTransactions getFinalizedTransactions(std::vector<trx_hash_t> const& trx_hashes) const;

  void addSystemTransactionToBatch(Batch& write_batch, SharedTransaction trx);
  std::shared_ptr<Transaction> getSystemTransaction(const trx_hash_t& hash) const;
  void addPeriodSystemTransactions(Batch& write_batch, SharedTransactions trxs, PbftPeriod period);
  std::vector<trx_hash_t> getPeriodSystemTransactionsHashes(PbftPeriod period) const;

  SharedTransactionReceipts getBlockReceipts(PbftPeriod period) const;

  void addTransactionToBatch(Transaction const& trx, Batch& write_batch);
  void removeTransactionToBatch(trx_hash_t const& trx, Batch& write_batch);
  bool transactionInDb(trx_hash_t const& hash);
  bool transactionFinalized(trx_hash_t const& hash);

  uint64_t getStatusField(StatusDbField const& field);
  void saveStatusField(StatusDbField const& field, uint64_t value);
  void addStatusFieldToBatch(StatusDbField const& field, uint64_t value, Batch& write_batch);
  uint64_t getNumTransactionExecuted();
  uint64_t getNumTransactionInDag();
  uint64_t getNumBlockExecuted();

  uint32_t getPbftMgrField(PbftMgrField field);
  void savePbftMgrField(PbftMgrField field, uint32_t value);
  void addPbftMgrFieldToBatch(PbftMgrField field, uint32_t value, Batch& write_batch);

  bool getPbftMgrStatus(PbftMgrStatus field);
  void savePbftMgrStatus(PbftMgrStatus field, bool const& value);
  void addPbftMgrStatusToBatch(PbftMgrStatus field, bool const& value, Batch& write_batch);

  void saveCertVotedBlockInRound(PbftRound round, const std::shared_ptr<PbftBlock>& block);
  std::optional<std::pair<PbftRound, std::shared_ptr<PbftBlock>>> getCertVotedBlockInRound() const;
  void removeCertVotedBlockInRound(Batch& write_batch);

  std::optional<PbftBlock> getPbftBlock(blk_hash_t const& hash);
  bool pbftBlockInDb(blk_hash_t const& hash);

  std::string getPbftHead(blk_hash_t const& hash);
  void savePbftHead(blk_hash_t const& hash, std::string const& pbft_chain_head_str);
  void addPbftHeadToBatch(taraxa::blk_hash_t const& head_hash, std::string const& head_str, Batch& write_batch);

  void saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote);
  std::vector<std::shared_ptr<PbftVote>> getOwnVerifiedVotes();
  void clearOwnVerifiedVotes(Batch& write_batch, const std::vector<std::shared_ptr<PbftVote>>& own_verified_votes);

  void replaceTwoTPlusOneVotes(TwoTPlusOneVotedBlockType type, const std::vector<std::shared_ptr<PbftVote>>& votes);
  void replaceTwoTPlusOneVotesToBatch(TwoTPlusOneVotedBlockType type,
                                      const std::vector<std::shared_ptr<PbftVote>>& votes, Batch& write_batch);
  std::vector<std::shared_ptr<PbftVote>> getAllTwoTPlusOneVotes();

  void removeExtraRewardVotes(const std::vector<vote_hash_t>& votes, Batch& write_batch);
  void saveExtraRewardVote(const std::shared_ptr<PbftVote>& vote);
  std::vector<std::shared_ptr<PbftVote>> getRewardVotes();

  void addPbftBlockPeriodToBatch(PbftPeriod period, taraxa::blk_hash_t const& pbft_block_hash, Batch& write_batch);
  std::pair<bool, PbftPeriod> getPeriodFromPbftHash(taraxa::blk_hash_t const& pbft_block_hash);
  std::shared_ptr<std::pair<PbftPeriod, uint32_t>> getDagBlockPeriod(blk_hash_t const& hash);
  void addDagBlockPeriodToBatch(blk_hash_t const& hash, PbftPeriod period, uint32_t position, Batch& write_batch);
  std::vector<blk_hash_t> getFinalizedDagBlockHashesByPeriod(PbftPeriod period);
  std::vector<std::shared_ptr<DagBlock>> getFinalizedDagBlockByPeriod(PbftPeriod period);

  std::optional<uint64_t> getProposalPeriodForDagLevel(uint64_t level);
  void saveProposalPeriodDagLevelsMap(uint64_t level, PbftPeriod period);
  void addProposalPeriodDagLevelsMapToBatch(uint64_t level, PbftPeriod period, Batch& write_batch);

  void savePeriodLambda(PbftPeriod period, uint32_t period_lambda, Batch& write_batch);
  std::optional<uint32_t> getPeriodLambda(PbftPeriod period, bool find_closest);

  void saveRoundsCountDynamicLambda(uint32_t rounds_count, Batch& write_batch);
  uint32_t getRoundsCountDynamicLambda();

  std::unordered_map<PbftPeriod, rewards::BlockStats> getBlocksRewardsStats() const;
  void saveBlockRewardsStats(uint64_t period, const rewards::BlockStats& stats, Batch& write_batch);
  bool hasMajorVersionChanged();
  void compactColumn(Column const& column);
  rustaxa::BridgeStorage& rustStorage();
  const rustaxa::BridgeStorage& rustStorage() const;

  template <typename T>
  void clearColumnHistory(std::unordered_set<T>& to_keep, Column c) {
    (void)to_keep;
    (void)c;
    throw DbException("DbStorage::clearColumnHistory is a RUSTAXA_ADMIN_COMPAT_UNSUPPORTED boundary in Rust shim mode");
  }

 private:
  rustaxa::BridgeStorageBatch& getOrCreateRustBatch(Batch& batch);
  template <typename Append>
  void commitImmediateRustBatch(Append&& append) {
    auto write_batch = createWriteBatch();
    try {
      std::forward<Append>(append)(getOrCreateRustBatch(write_batch));
      commitWriteBatch(write_batch);
    } catch (...) {
      std::lock_guard<std::mutex> lock(rust_batches_mutex_);
      rust_batches_.erase(&write_batch);
      throw;
    }
  }
  std::string lookupFinalChainMeta(const Slice& key) const;
  std::string lookupFinalChainBlockByNumber(const Slice& key) const;
  std::string lookupFinalChainBlockHashByNumber(const Slice& key) const;
  std::string lookupFinalChainBlockNumberByHash(const Slice& key) const;
  std::string lookupFinalChainLogBloomsChunk(const Slice& key) const;
  std::string lookupFinalChainReceiptByTrxHash(const Slice& key) const;

  std::optional<::rust::Box<rustaxa::BridgeStorage>> rust_storage_;
  std::optional<::rust::Box<rustaxa::BridgeDagStorageQueries>> dag_queries_;
  std::optional<::rust::Box<rustaxa::BridgePbftStorageQueries>> pbft_queries_;
  std::optional<::rust::Box<rustaxa::BridgePbftVoteStorageQueries>> pbft_vote_queries_;
  std::optional<::rust::Box<rustaxa::BridgeTransactionStorageQueries>> transaction_queries_;
  std::optional<::rust::Box<rustaxa::BridgeFinalChainStorageQueries>> final_chain_queries_;
  std::optional<::rust::Box<rustaxa::BridgePeriodStorageQueries>> period_queries_;
  std::unordered_map<Batch*, ::rust::Box<rustaxa::BridgeStorageBatch>> rust_batches_;
  std::mutex rust_batches_mutex_;
};

}  // namespace taraxa
