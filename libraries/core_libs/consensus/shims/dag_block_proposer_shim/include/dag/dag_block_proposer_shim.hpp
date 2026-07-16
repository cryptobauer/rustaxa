#pragma once

#include <atomic>
#include <cstdint>
#include <memory>
#include <string>
#include <thread>
#include <vector>

#include "common/thread_pool.hpp"
#include "config/config.hpp"
#include "dag/dag_block.hpp"
#include "logger/logger.hpp"

namespace taraxa {

/** @addtogroup DAG
 * @{
 */
class TransactionManager;
class KeyManager;
class DagManager;
class Network;
struct FullNodeConfig;

namespace final_chain {
class FinalChain;
}

/**
 * Rust-mode DAG block proposer facade.
 *
 * The class preserves the public `DagBlockProposer` API while moving deterministic proposal facts toward Rust. C++
 * still owns worker/network orchestration, live network-throttle and VDF execution, node-secret signature execution,
 * and compatibility transaction payload materialization. Rust owns canonical proposal VRF/VDF message bytes, proposer
 * tip-pruning, unsigned block planning, signing-hash derivation, signed-block RLP finalization, VDF wait/stale-proof
 * decisions, post-boundary retry-state resets, and DPoS/VDF authorization facts through the Rust-backed FinalChain
 * shim.
 *
 * Edge behavior:
 * - proposal returns `false` when the transaction pool, DPoS facts, VRF key, or vote denominator are unavailable
 * - VDF proving keeps the legacy async cancellation behavior through Rust cancellation tokens
 * - feature-on production routing has no dependency on the legacy C++ proposer implementation
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
          trx_shard(std::stoull(wallet.node_addr.toString().substr(0, 6).c_str(), nullptr, 16) % shard) {}

    const WalletConfig wallet;
    const uint16_t max_num_tries;
    const uint16_t trx_shard;
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
   * Rust owns the runtime session and all deterministic block construction. C++ executes transaction packing, VDF,
   * node-secret signing, and add-block side effects without holding the DAG runtime lock. Every successfully opened
   * session is removed on normal completion or aborted during unwinding.
   *
   * Returns `true` when the loop should immediately try another proposal, including the cancellation case where a stale
   * in-flight proof was cancelled. Returns `false` when no block was proposed and the caller should wait. Executor,
   * bridge, or invalid-session failures propagate as exceptions after session cleanup.
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

  /**
   * Selects proposal tips through the Rust DAG proposer policy.
   *
   * This compatibility method preserves the legacy public API for tests and callers. Rust owns storage-backed tip
   * metadata loading, missing-tip handling, proposer grouping, level ordering, gas-limit enforcement, and max-tip
   * enforcement; C++ materializes only the returned hash list.
   */
  vec_blk_t selectDagBlockTips(const vec_blk_t& frontier_tips, uint64_t gas_limit) const;

 private:
  /**
   * Transactions selected for one proposer attempt.
   *
   * `transaction_hashes` and `gas_estimations` come from the Rust transaction-packing session and are the deterministic
   * proposal facts. Live C++ transaction objects are materialized only after VDF/block planning for the temporary DAG
   * add-block sidecar. `network_throttled` reports a live executor throttle distinctly from an empty eligible pack.
   */
  struct ShardedProposalTransactions {
    bool network_throttled{false};
    vec_trx_t transaction_hashes;
    std::vector<dev::bytes> transaction_rlps;
    std::vector<uint64_t> gas_estimations;
  };

  /**
   * Returns transactions, hashes, and gas estimates for the configured proposer shard.
   *
   * Rust owns the deterministic shard filter and transaction-packing planner. The selected hashes and gas estimates
   * come directly from Rust; C++ keeps live transaction materialization and EVM gas estimation until those boundaries
   * move.
   */
  ShardedProposalTransactions getShardedTrxs(PbftPeriod proposal_period, uint64_t weight_limit,
                                             const uint16_t total_trx_shards, const uint16_t node_trx_shard,
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
