#pragma once

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <unordered_map>
#include <vector>

#include "config/hardfork.hpp"
#include "rewards/block_stats.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"

namespace taraxa::rewards {

struct FinalChainPublicationRewardsStats {
  std::vector<BlockStats> distribution_stats;
  rustaxa::FinalChainExternalEvmRewardsStatsUpdate storage_update;
};

/**
 * Rust-mode rewards statistics facade.
 *
 * The facade preserves the public `rewards::Stats` API while routing
 * deterministic per-period rewards-stat calculation, interval caching, and
 * block-reward stats cache persistence planning through Rust. C++ keeps
 * `BlockStats` only as the legacy edge adapter passed to
 * `StateAPI::distribute_rewards`. Rust owns the authoritative interval cache
 * and storage payload bytes.
 *
 * Inputs:
 * - constructor receives committee size, hardfork frequency rules, storage, and
 *   the DPoS total-vote callback used to source previous-cert-vote weights
 * - `processStats` receives finalized `PeriodData`, block-rate facts, gas used
 *   by executed transactions, and the caller's finalization batch
 *
 * Outputs:
 * - `processStats` returns legacy-compatible `BlockStats` instances decoded
 *   from Rust-produced RLP when the current period is a distribution boundary
 * - non-boundary cache write intents are appended to the caller's Rust-backed
 *   storage batch and are not committed by this class
 *
 * Invariants and edge behavior:
 * - no production path forwards to the legacy C++ implementation
 * - gas-used vectors must be empty or contain at least one entry per finalized
 *   transaction; extra entries are ignored for system-transaction compatibility
 * - `clear` updates storage, the Rust runtime, and the mirror cache only after
 *   the caller has committed the finalization batch, preserving FinalChain
 *   commit ordering
 */
class Stats {
 public:
  Stats(uint32_t committee_size, const HardforksConfig& hardforks, std::shared_ptr<DbStorage> db,
        std::function<uint64_t(EthBlockNumber)>&& dpos_eligible_total_vote_count, EthBlockNumber last_blk_num = 0);
  ~Stats();

  Stats(const Stats&) = delete;
  Stats(Stats&&) = delete;
  Stats& operator=(const Stats&) = delete;
  Stats& operator=(Stats&&) = delete;

  /**
   * Processes finalized rewards statistics for one PBFT period through Rust.
   *
   * The returned vector is non-empty only when rewards must be distributed at
   * the current period. Non-boundary cache writes are appended to `write_batch`
   * before the vector is returned. Distribution-boundary cache clearing remains
   * in `clear` so it runs after the caller commits state and the finalization
   * batch.
   */
  std::vector<BlockStats> processStats(const PeriodData& current_blk, uint32_t blocks_per_year,
                                       const std::vector<gas_t>& trxs_gas_used, Batch& write_batch);

  /**
   * Processes rewards stats for Rust-owned external-EVM FinalChain publication.
   *
   * The returned `distribution_stats` are passed to `StateAPI::distribute_rewards`, while `storage_update` must be
   * attached to the Rust external-EVM publication plan so cache rows are committed in the same FinalChain storage batch
   * as the block header and indexes. This method does not append writes to a C++ batch.
   */
  FinalChainPublicationRewardsStats processStatsForFinalChainPublication(const PeriodData& current_blk,
                                                                         uint32_t blocks_per_year,
                                                                         const std::vector<gas_t>& trxs_gas_used);

  /**
   * Commits the previously previewed publication rewards-stat plan after the
   * surrounding FinalChain storage publication succeeds.
   */
  void commitStatsAfterFinalChainPublication();

  /**
   * Clears the runtime cache after the surrounding finalization commit has
   * completed on a rewards distribution boundary.
   */
  void clear(uint64_t current_period);

  /**
   * Clears only in-memory rewards caches after Rust publication has already
   * committed any required rewards-stat storage mutation.
   */
  void clearCommittedAfterFinalChainPublication(uint64_t current_period);

 protected:
  /**
   * Recovers current-interval rewards stats from Rust storage.
   */
  void recoverFromDb(EthBlockNumber last_blk_num);

  const uint32_t kCommitteeSize;
  const HardforksConfig kHardforksConfig;
  std::shared_ptr<DbStorage> db_;
  const std::function<uint64_t(EthBlockNumber)> dpos_eligible_total_vote_count_;
  // Legacy decoded view retained for public/test adapters that still inspect
  // `BlockStats`. It is not the authoritative rewards-stat cache in Rust mode.
  std::unordered_map<PbftPeriod, BlockStats> blocks_stats_;

 private:
  rustaxa::RewardsStatsProcessFact makeProcessFact(const PeriodData& current_blk, uint32_t blocks_per_year,
                                                   const std::vector<gas_t>& trxs_gas_used) const;
  std::vector<BlockStats> decodeDistributionStats(const rust::Vec<rustaxa::PeriodRlp>& stats) const;
  BlockStats decodeBlockStats(const rust::Vec<uint8_t>& stats_rlp) const;
  void cacheStatsView(PbftPeriod period, const rust::Vec<uint8_t>& stats_rlp);
  void replaceCacheView(const rust::Vec<rustaxa::PeriodRlp>& stats);
  void appendStorageWrites(const rustaxa::RewardsStatsProcessResult& plan, Batch& write_batch);

  rust::Box<rustaxa::BridgeRewardsStatsRuntime> rust_stats_;
  std::optional<rustaxa::RewardsStatsProcessResult> pending_publication_plan_;
};

}  // namespace taraxa::rewards
