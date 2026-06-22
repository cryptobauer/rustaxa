#pragma once

#include <libp2p/Host.h>

#include <deque>
#include <memory>
#include <optional>
#include <shared_mutex>
#include <tuple>
#include <vector>

#include "pbft/period_data.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

/** @addtogroup PBFT
 * @{
 */

class PeriodData;

/**
 * Rust-mode PBFT sync queue facade.
 *
 * This class preserves the public C++ `PeriodDataQueue` API while routing deterministic queue metadata and admission
 * rules to Rust. It is a standalone facade and must not inherit from or delegate/fallback to `PeriodDataQueueOld`.
 *
 * Ownership:
 * - Rust owns entry order, accepted period tracking, effective processable size, pop vote-source decisions, cleanup
 *   planning, and clear semantics.
 * - C++ owns live `PeriodData` and peer `NodeID` objects until those model types are ported. PBFT cert-vote payloads
 *   are retained in Rust as canonical bytes and materialized into C++ vote sidecars only at executor boundaries.
 *
 * Invariants:
 * - all Rust queue calls and live payload mutation are guarded by `queue_access_`
 * - `queued_payloads_` order and ids mirror Rust queue metadata exactly
 */
class PeriodDataQueue {
 public:
  /**
   * C++ live payload plus Rust-owned compact block-link metadata returned by a pop.
   *
   * The payload fields remain compatibility sidecars. `period`, `block_hash`, `prev_block_hash`, and `pivot_hash` are
   * copied from Rust queue metadata so PBFT manager admission facts do not need to reopen the live `PeriodData` object
   * just to recover chain-link, final-chain-hash, extra-data, transaction, previous-cert, and pillar sidecar facts.
   * Live payload objects remain temporary compatibility sidecars.
   */
  struct PoppedPeriodData {
    PeriodData period_data;
    std::vector<std::shared_ptr<PbftVote>> cert_votes;
    dev::p2p::NodeID node_id;
    uint64_t period = 0;
    blk_hash_t block_hash;
    blk_hash_t prev_block_hash;
    blk_hash_t pivot_hash;
    blk_hash_t final_chain_hash;
    std::vector<vote_hash_t> reward_vote_hashes;
    std::vector<bytes> pillar_vote_rlps;
    std::vector<bytes> transaction_rlps;
    std::vector<bytes> cert_vote_rlps;
    std::vector<trx_hash_t> dag_transaction_hashes;
    std::vector<trx_hash_t> period_data_transaction_hashes;
    ::rust::Vec<rustaxa::TransactionManagerVerifyNotFinalizedRuntimeFact> period_data_transaction_identities;
    bool previous_cert_votes_present = false;
    bool previous_cert_first_vote_has_weight = false;
    bool pillar_votes_present = false;
    bool extra_data_present = false;
    bool extra_data_pillar_block_hash_present = false;
  };

  /**
   * Creates an empty Rust-backed period-data queue.
   */
  PeriodDataQueue();
  ~PeriodDataQueue();

  PeriodDataQueue(const PeriodDataQueue&) = delete;
  PeriodDataQueue(PeriodDataQueue&&) = delete;
  PeriodDataQueue& operator=(const PeriodDataQueue&) = delete;
  PeriodDataQueue& operator=(PeriodDataQueue&&) = delete;

  /**
   * Pushes synced period data when it extends local synchronization bounds.
   *
   * Returns false for legacy period-admission rejection. Throws if the payload is malformed or Rust reports an
   * invariant error.
   */
  bool push(PeriodData&& period_data, const dev::p2p::NodeID& node_id, uint64_t max_pbft_size,
            std::vector<std::shared_ptr<PbftVote>>&& cert_votes);

  /**
   * Pops and returns front period data together with Rust-owned compact block-link metadata.
   */
  PoppedPeriodData popWithMetadata();

  /**
   * Clears all queue state and resets tracked period.
   */
  void clear();

  /**
   * Returns number of processable queue entries.
   */
  size_t size() const;

  /**
   * Returns true when queue has no period-data entries.
   */
  bool empty() const;

  /**
   * Returns newest tracked period for synchronized queue data, or 0 when reset.
   */
  uint64_t getPeriod() const;

  /**
   * Returns the Rust-owned queue-aware PBFT syncing period for network status.
   *
   * `pbft_chain_size` remains a PBFT-chain executor fact; the max calculation lives with Rust queue metadata.
   */
  uint64_t syncingPeriod(uint64_t pbft_chain_size) const;

  /**
   * Returns the PBFT block hash to use as the next chain-link fact.
   *
   * The PBFT period and chain hash remain PBFT-chain executor facts. Rust queue metadata decides whether the newest
   * queued block hash is fresh enough for that period; otherwise the supplied chain hash is returned.
   */
  blk_hash_t lastBlockHashOrChain(uint64_t current_period, const blk_hash_t& chain_last_hash) const;

  /**
   * Removes queued entries with period lower than `period`.
   */
  void cleanOldData(uint64_t period);

 private:
  struct QueuedPayload {
    uint64_t entry_id = 0;
    PeriodData period_data;
    dev::p2p::NodeID node_id;
  };

  QueuedPayload popFrontPayload(uint64_t expected_entry_id);
  const QueuedPayload& backPayload(uint64_t expected_entry_id) const;

  mutable std::shared_mutex queue_access_;
  ::rust::Box<rustaxa::BridgePeriodDataQueue> rust_queue_;
  std::deque<QueuedPayload> queued_payloads_;
  uint64_t next_entry_id_{1};
};

/** @}*/

}  // namespace taraxa
