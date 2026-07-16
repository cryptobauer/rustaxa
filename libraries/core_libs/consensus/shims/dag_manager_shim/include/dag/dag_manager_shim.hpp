#pragma once

#include <cstdint>
#include <map>
#include <memory>
#include <optional>
#include <shared_mutex>
#include <string>
#include <tuple>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>

#include "common/thread_pool.hpp"
#include "dag/dag_block.hpp"
#include "dag/sortition_params_manager.hpp"
#include "pbft/pbft_chain.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa {

class KeyManager;
class Network;

/**
 * DAG-owned result after applying finalized DAG order for PBFT finalization.
 *
 * Inputs are the finalized anchor, PBFT period, and ordered DAG blocks after the Rust DAG runtime applies the finalized
 * order. Outputs carry only DAG facts: the number of DAG blocks finalized by the mutation. PBFT manager code converts
 * this fact into its executor report at the manager boundary.
 */
struct DagFinalizationOrderReport {
  uint64_t finalized_count = 0;
};

/**
 * Rust-mode DagManager migration facade.
 *
 * This class preserves the public DagManager API while routing migrated behavior
 * through Rust-backed implementations. It owns shared-pointer identity directly
 * and does not import or delegate to the legacy DAG manager in Rust mode.
 */
class DagManager : public std::enable_shared_from_this<DagManager> {
 public:
  /**
   * Result of Rust-backed DAG block verification.
   *
   * The numeric values intentionally match the legacy public enum so tarcap and
   * tests can keep the stable `DagManager::VerifyBlockReturnType` API while
   * Rust mode owns the result type directly instead of importing the legacy manager declaration.
   */
  enum class VerifyBlockReturnType : uint32_t {
    Verified = 0,
    MissingTransaction,
    AheadBlock,
    FailedVdfVerification,
    FutureBlock,
    NotEligible,
    ExpiredBlock,
    IncorrectTransactionsEstimation,
    BlockTooBig,
    FailedTipsVerification,
    MissingTip
  };

  /**
   * Preserves standalone test construction by creating a private composed service.
   *
   * Production `App` uses the overload below so `TransactionManager` and
   * `DagManager` share one application-owned service.
   */
  explicit DagManager(const FullNodeConfig &config, addr_t node_addr, std::shared_ptr<TransactionManager> trx_mgr,
                      std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<final_chain::FinalChain> final_chain,
                      std::shared_ptr<DbStorage> db, std::shared_ptr<KeyManager> key_manager);

  /** Constructs the production DAG facade over the application-owned service. */
  DagManager(const FullNodeConfig &config, addr_t node_addr, std::shared_ptr<TransactionManager> trx_mgr,
             std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<final_chain::FinalChain> final_chain,
             std::shared_ptr<DbStorage> db, std::shared_ptr<KeyManager> key_manager,
             SharedDagTransactionService dag_transaction_service);
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
  /**
   * Adds a DAG block from Rust-produced canonical signed block RLP.
   *
   * `signed_block` contains the canonical RLP and matching hash finalized by Rust. `transaction_hashes` and
   * `transaction_rlps` are the corresponding live transaction-pool payloads, while `proposed` and `save` select the
   * existing event/gossip and persistence side effects. Rust decodes compact manager facts and owns add-block planning;
   * C++ materializes temporary `DagBlock` and `Transaction` objects only when compatibility side-effect APIs require
   * them.
   *
   * Returns a typed accepted/duplicate/expired/missing-reference report for the proposer session. A block hash/RLP
   * mismatch, malformed canonical bytes, missing transaction payload, or bridge/storage failure propagates as an
   * exception without holding the proposer-session DAG lock.
   */
  rustaxa::DagProposerAddBlockReport addDagBlockRlp(rustaxa::DagProposerSignedBlockIntent signed_block,
                                                    const vec_trx_t &transaction_hashes,
                                                    std::vector<dev::bytes> &&transaction_rlps, bool proposed = false,
                                                    bool save = true);
  vec_blk_t getDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period);
  /**
   * Apply a finalized DAG order through the composed Rust DAG/transaction service.
   *
   * The service commits the DAG transition and removes expired transaction sidecars as one private cross-runtime
   * operation. C++ receives only the finalized-block count and expired DAG hashes needed to maintain its public return
   * value and compatibility block cache. Invalid periods return zero without mutation; bridge or storage failures
   * propagate as exceptions.
   */
  uint setDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period, vec_blk_t const &dag_order);
  /**
   * Apply finalized DAG ordering and return DAG-owned finalization facts.
   *
   * Inputs are the finalized anchor, PBFT period, and ordered DAG blocks. The returned report carries post-mutation DAG
   * facts that the PBFT manager forwards to Rust before the PBFT runtime cursor advances.
   */
  DagFinalizationOrderReport setDagBlockOrderForPbftFinalization(blk_hash_t const &anchor, PbftPeriod period,
                                                                 vec_blk_t const &dag_order);
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
  /**
   * Selects proposer tips using metadata loaded from Rust storage.
   *
   * This backs the legacy `DagBlockProposer::selectDagBlockTips` compatibility API while keeping missing-tip handling,
   * proposer grouping, level ordering, gas-limit enforcement, and max-tip enforcement in Rust.
   */
  rustaxa::DagProposerTipSelectionPlan planProposerTipSelection(
      rustaxa::DagProposerStorageTipSelectionInput input) const;
  /**
   * Opens a runtime-owned proposer cursor for one `DagBlockProposer::proposeDagBlock` attempt.
   *
   * The input contains the configured 20-byte proposer node address, wallet VRF identity, gas/tip limits, transaction
   * shard settings, and other static proposal limits. Rust reads transaction pressure directly from the composed
   * transaction runtime, retains the configured address as the authoritative expected signer, observes its DAG frontier
   * and proposal-period mapping atomically, and retains all deterministic construction state. The returned identifier
   * is unique among live sessions. Bridge or allocation failures propagate as exceptions; no identifier is returned
   * unless the cursor was installed.
   */
  uint64_t beginProposerSession(rustaxa::DagProposerSessionBeginInput input);
  /**
   * Abort and remove one runtime-owned proposer session.
   *
   * This cleanup operation performs no proposal planning, retry-state mutation,
   * or external effects. It is idempotent: a session already removed by normal
   * completion or a previous abort is a no-op.
   *
   * @param session_id runtime-issued proposer session identifier
   * @return true when this call removed a live session, false when no session existed
   * @throws bridge or synchronization exceptions before cleanup completes
   */
  bool abortProposerSession(uint64_t session_id);
  /**
   * Returns the first requested effect for a live proposer session.
   *
   * The runtime lock is held only while Rust advances the cursor. Unknown or out-of-order session identifiers return an
   * invalid-report step; bridge failures propagate as exceptions.
   */
  rustaxa::DagProposerSessionStep proposerSessionNext(uint64_t session_id);
  /**
   * Reports FinalChain and sortition facts requested by a proposer session.
   *
   * The caller collects these facts without holding the DAG runtime lock. Rust reacquires the lock, revalidates its
   * stored DAG observation, and either advances to transaction packing or terminates a stale attempt.
   */
  rustaxa::DagProposerSessionStep reportProposerExternalProposalFacts(
      uint64_t session_id, rustaxa::DagProposerExternalProposalFactsReport report);
  /**
   * Polls whether the Rust DAG frontier invalidated an in-flight VDF proof.
   *
   * Returns a cancellation or continuation step without waiting while holding the DAG lock. Unknown or out-of-order
   * sessions return an invalid-report step.
   */
  rustaxa::DagProposerSessionStep pollProposerVdfWait(uint64_t session_id);
  /**
   * Reports the completed canonical VDF RLP for Rust-owned block construction.
   *
   * Rust validates and retains the canonical bytes, selects tips, computes the block gas and signing hash, and returns
   * a signing request or terminal/stale-proof step. VDF execution occurs outside the DAG lock. Storage, decoding, or
   * block-construction failures remove the session and propagate as bridge exceptions.
   */
  rustaxa::DagProposerSessionStep reportProposerVdfProof(uint64_t session_id,
                                                         rustaxa::DagProposerVdfProofReport report);
  /**
   * Resumes a stale-proof session after C++ performs the requested compatibility sleep.
   *
   * Rust rechecks its frontier under the DAG lock and returns either a terminal retry decision or a signing request.
   * Block-construction failures remove the session and propagate as bridge exceptions.
   */
  rustaxa::DagProposerSessionStep resumeProposerAfterStaleProofSleep(uint64_t session_id);
  /**
   * Reports the node-secret signature for the Rust-provided signing hash.
   *
   * Rust validates the signature bytes and finalizes canonical signed block RLP/hash, returned in the add-block step.
   * Signing occurs before this call and no DAG lock crosses the signing boundary. Invalid signatures or finalization
   * failures remove the session and propagate as bridge exceptions.
   */
  rustaxa::DagProposerSessionStep reportProposerSigning(uint64_t session_id, rustaxa::DagProposerSigningReport report);
  /**
   * Reports the result of executing `addDagBlockRlp` for the Rust-produced signed block.
   *
   * Rust validates the outcome, applies terminal retry/record effects, removes the session, and returns the final
   * result. Add-block execution occurs before this call and outside the proposer-session DAG lock.
   */
  rustaxa::DagProposerSessionStep reportProposerAddBlock(uint64_t session_id,
                                                         rustaxa::DagProposerAddBlockReport report);
  /**
   * Returns the PBFT block hash for a finalized period through Rust storage.
   *
   * Missing period data preserves legacy storage behavior and returns the zero
   * hash. Malformed period data propagates as a bridge exception so callers can
   * reject invalid VDF inputs.
   */
  blk_hash_t getPeriodBlockHashForDagProposal(PbftPeriod period) const;

  static dev::bytes getVdfMessage(blk_hash_t const &hash, SharedTransactions const &trxs);
  static dev::bytes getVdfMessage(blk_hash_t const &hash, std::vector<trx_hash_t> const &trx_hashes);

 private:
  void mirrorDagCountersFromRuntime() const;
  bool addBlockToRustGraphs(const std::shared_ptr<DagBlock> &blk);
  bool addBlockToRustGraphs(const rustaxa::DagManagerBlock &blk);
  std::pair<blk_hash_t, std::vector<blk_hash_t>> getRustFrontier() const;

  std::shared_ptr<TransactionManager> trx_mgr_;
  std::shared_ptr<PbftChain> pbft_chain_;
  std::shared_ptr<final_chain::FinalChain> final_chain_;
  std::shared_ptr<DbStorage> db_;
  std::shared_ptr<KeyManager> key_manager_;
  std::weak_ptr<Network> network_;
  SortitionParamsManager sortition_params_manager_;
  const GenesisConfig genesis_config_;
  const std::shared_ptr<DagBlock> genesis_block_;
  const uint32_t max_levels_per_period_;
  const uint32_t cache_max_size_ = 10000;
  const uint32_t cache_delete_step_ = 100;
  ExpirationCacheMap<blk_hash_t, std::shared_ptr<DagBlock>> seen_blocks_;
  // Serializes Rust DAG persistence/runtime mutation with compatibility-mirror
  // updates performed by this facade.
  mutable std::shared_mutex rust_order_dag_blocks_mutex_;
  mutable std::shared_mutex rust_graphs_mutex_;
  mutable std::shared_mutex dag_finalization_mutex_;
  SharedDagTransactionService dag_transaction_service_;
};

}  // namespace taraxa
