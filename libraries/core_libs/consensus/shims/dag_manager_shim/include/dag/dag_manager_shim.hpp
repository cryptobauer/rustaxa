#pragma once

namespace taraxa {

/**
 * Rust-mode DagManager migration facade.
 *
 * This class preserves the public DagManager API while individual methods move
 * from legacy C++ behavior into Rust-backed implementations. Methods that have
 * not been migrated yet explicitly delegate to `DagManagerOld` at the call site
 * with a local TODO comment so remaining migration work stays visible.
 */
class DagManager : public DagManagerOld {
  struct RustDagManagerGraphs;

 public:
  using VerifyBlockReturnType = DagManagerOld::VerifyBlockReturnType;

  explicit DagManager(const FullNodeConfig &config, addr_t node_addr, std::shared_ptr<TransactionManager> trx_mgr,
                      std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<final_chain::FinalChain> final_chain,
                      std::shared_ptr<DbStorage> db, std::shared_ptr<KeyManager> key_manager);
  ~DagManager();

  DagManager(const DagManager &) = delete;
  DagManager(DagManager &&) = delete;
  DagManager &operator=(const DagManager &) = delete;
  DagManager &operator=(DagManager &&) = delete;

  std::shared_ptr<DagManager> getShared();

  void setNetwork(std::weak_ptr<Network> network);

  bool isDagBlockKnown(const blk_hash_t &hash) const;
  std::shared_ptr<DagBlock> getDagBlock(const blk_hash_t &hash) const;
  std::pair<VerifyBlockReturnType, SharedTransactions> verifyBlock(
      const std::shared_ptr<DagBlock> &blk,
      const std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>> &trxs = {});
  std::pair<bool, std::vector<blk_hash_t>> pivotAndTipsAvailable(const std::shared_ptr<DagBlock> &blk);
  std::pair<bool, std::vector<blk_hash_t>> addDagBlock(const std::shared_ptr<DagBlock> &blk,
                                                       SharedTransactions &&trxs = {}, bool proposed = false,
                                                       bool save = true);
  vec_blk_t getDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period);
  uint setDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period, vec_blk_t const &dag_order);
  std::optional<std::pair<blk_hash_t, std::vector<blk_hash_t>>> getLatestPivotAndTips() const;
  std::vector<blk_hash_t> getGhostPath(const blk_hash_t &source) const;
  std::vector<blk_hash_t> getGhostPath() const;

  void drawTotalGraph(std::string const &str) const;
  void drawPivotGraph(std::string const &str) const;
  void drawGraph(std::string const &dotfile) const;

  std::pair<uint64_t, uint64_t> getNumVerticesInDag() const;
  std::pair<uint64_t, uint64_t> getNumEdgesInDag() const;
  level_t getMaxLevel() const;
  PbftPeriod getLatestPeriod() const;
  std::pair<blk_hash_t, blk_hash_t> getAnchors() const;
  uint32_t getDagExpiryLimit() const;
  const std::pair<PbftPeriod, std::map<uint64_t, std::unordered_set<blk_hash_t>>> getNonFinalizedBlocks() const;
  const std::tuple<PbftPeriod, std::vector<std::shared_ptr<DagBlock>>, SharedTransactions>
  getNonFinalizedBlocksWithTransactions(const std::unordered_set<blk_hash_t> &known_hashes) const;
  DagFrontier getDagFrontier();
  std::pair<size_t, size_t> getNonFinalizedBlocksSize() const;
  uint32_t getNonFinalizedBlocksMinDifficulty() const;

  util::event::Event<DagManager, std::shared_ptr<DagBlock>> const block_verified_{};

  std::shared_mutex &getDagMutex();
  SortitionParamsManager &sortitionParamsManager();
  const DagConfig &getDagConfig() const;
  uint64_t getDagExpiryLevel() const;
  uint64_t getMaxLevelsPerPeriod() const;

  static dev::bytes getVdfMessage(blk_hash_t const &hash, SharedTransactions const &trxs);
  static dev::bytes getVdfMessage(blk_hash_t const &hash, std::vector<trx_hash_t> const &trx_hashes);

 private:
  void rebuildRustGraphsFromOld();
  bool addBlockToRustGraphs(const std::shared_ptr<DagBlock> &blk);
  std::pair<blk_hash_t, std::vector<blk_hash_t>> getRustFrontier() const;

  std::shared_ptr<TransactionManager> trx_mgr_;
  std::weak_ptr<Network> network_;
  const std::shared_ptr<DagBlock> genesis_block_;
  const uint32_t max_levels_per_period_;
  const uint32_t cache_max_size_ = 10000;
  const uint32_t cache_delete_step_ = 100;
  ExpirationCacheMap<blk_hash_t, std::shared_ptr<DagBlock>> seen_blocks_;
  // Serializes Rust DAG persistence/runtime mutation with compatibility-mirror
  // updates and finalization-triggered Rust rebuilds.
  mutable std::shared_mutex rust_order_dag_blocks_mutex_;
  mutable std::shared_mutex rust_graphs_mutex_;
  std::unique_ptr<RustDagManagerGraphs> rust_graphs_;
};

}  // namespace taraxa
