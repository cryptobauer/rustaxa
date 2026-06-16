#pragma once

#include <atomic>
#include <thread>
#include <vector>

#include "dag/dag_block.hpp"
#include "logger/logger.hpp"
#include "network/network.hpp"

namespace taraxa {

/** @addtogroup DAG
 * @{
 */
class TransactionManager;
class KeyManager;
class DagManager;
struct FullNodeConfig;

namespace final_chain {
class FinalChain;
}

/**
 * Rust-mode DAG block proposer facade.
 *
 * The class preserves the public `DagBlockProposer` API while moving deterministic proposal facts toward Rust. C++ still
 * owns worker-thread orchestration, live transaction selection, live tip metadata materialization, and final `DagBlock`
 * construction. Rust owns canonical proposal VRF/VDF message byte construction, proposer VDF proof generation,
 * proposer tip-pruning/block-construction planning, VDF wait/stale-proof decisions, post-boundary retry-state resets,
 * and DPoS/VDF authorization facts through the Rust-backed FinalChain shim.
 *
 * Edge behavior:
 * - proposal returns `false` when the transaction pool, DPoS facts, VRF key, or vote denominator are unavailable
 * - VDF proving keeps the legacy async cancellation behavior through Rust cancellation tokens
 * - no method delegates production routing to `DagBlockProposerOld`
 */
class DagBlockProposer {
 public:
  /**
   * Per-wallet proposer state.
   *
   * Inputs are the configured wallet, retry budget, and normalized non-zero transaction shard count. The derived retry
   * budget and shard are deterministic and match the legacy public behavior for configured shard counts greater than
   * zero.
   */
  struct NodeDagProposerData {
    NodeDagProposerData(const WalletConfig& wallet, const uint16_t max_tries, const uint16_t shard)
        : wallet(wallet),
          max_num_tries(max_tries + (wallet.node_addr[0] % (10 * max_tries))),
          trx_shard(std::stoull(wallet.node_addr.toString().substr(0, 6).c_str(), NULL, 16) % shard) {}

    const WalletConfig wallet;
    const uint16_t max_num_tries;
    const uint16_t trx_shard;

    uint16_t num_tries{0};
    uint64_t last_propose_level{0};
  };

 public:
  DagBlockProposer(const FullNodeConfig& config, std::shared_ptr<DagManager> dag_mgr,
                   std::shared_ptr<TransactionManager> trx_mgr, std::shared_ptr<final_chain::FinalChain> final_chain,
                   std::shared_ptr<KeyManager> key_manager);
  ~DagBlockProposer() { stop(); }
  DagBlockProposer(const DagBlockProposer&) = delete;
  DagBlockProposer(DagBlockProposer&&) = delete;
  DagBlockProposer& operator=(const DagBlockProposer&) = delete;
  DagBlockProposer& operator=(DagBlockProposer&&) = delete;

  /**
   * Starts proposer worker threads for all configured wallets.
   */
  void start();

  /**
   * Stops all proposer worker threads.
   */
  void stop();

  /**
   * Attempts to propose one DAG block for `node_dag_proposer_data`.
   *
   * Returns `true` when the loop should immediately try another proposal, including the legacy cancellation case where a
   * stale in-flight proof was cancelled. Returns `false` when no block was proposed and the caller should wait.
   */
  bool proposeDagBlock(const std::shared_ptr<NodeDagProposerData>& node_dag_proposer_data);

  /**
   * Sets the optional network view used for sync and packet-pressure checks.
   */
  void setNetwork(std::weak_ptr<Network> network);

  /**
   * Returns the number of proposed blocks since the last start.
   */
  uint64_t getProposedBlocksCount() const { return proposed_blocks_count_; }

 private:
  /**
   * Creates a signed DAG block from already selected proposal data.
   *
   * Inputs are the current frontier, computed level, chosen transactions and gas estimates, completed VDF sortition, and
   * node signing secret. Rust plans the block gas estimate and selected tips, while the returned block still uses
   * existing C++ `DagBlock` construction.
   */
  std::shared_ptr<DagBlock> createDagBlock(DagFrontier&& frontier, level_t level, const SharedTransactions& trxs,
                                           std::vector<uint64_t>&& estimations, vdf_sortition::VdfSortition&& vdf,
                                           const dev::Secret& node_secret) const;

  /**
   * Returns transactions and gas estimates for the configured proposer shard.
   *
   * Rust owns the deterministic shard filter and transaction-packing planner;
   * C++ keeps live transaction materialization and EVM gas estimation until
   * those boundaries move.
   */
  std::pair<SharedTransactions, std::vector<uint64_t>> getShardedTrxs(PbftPeriod proposal_period, uint64_t weight_limit,
                                                                      const uint16_t total_trx_shards,
                                                                      const uint16_t node_trx_shard,
                                                                      uint64_t shard_period_interval) const;

 private:
  const uint16_t max_num_tries_{20};
  util::ThreadPool executor_{1};

  std::atomic<uint64_t> proposed_blocks_count_{0};
  std::atomic<bool> stopped_{true};

  const uint16_t total_trx_shards_;

  std::shared_ptr<DagManager> dag_mgr_;
  std::shared_ptr<TransactionManager> trx_mgr_;
  std::shared_ptr<final_chain::FinalChain> final_chain_;
  std::vector<std::thread> proposer_workers_;
  std::weak_ptr<Network> network_;

  std::vector<std::shared_ptr<NodeDagProposerData>> nodes_dag_proposers_data_;

  const uint64_t kDagProposeGasLimit;
  const uint64_t kPbftGasLimit;
  const uint64_t kDagGasLimit;

  const uint64_t kShardProposePeriodInterval = 10;

  LOG_OBJECTS_DEFINE
};

/**
 * @}
 */

}  // namespace taraxa
