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
 * - C++ owns live `PeriodData`, `PbftVote`, and peer `NodeID` objects until those model types are ported.
 *
 * Invariants:
 * - all Rust queue calls and live payload mutation are guarded by `queue_access_`
 * - `queued_payloads_` order and ids mirror Rust queue metadata exactly
 * - `last_block_cert_votes_` is used only when Rust pop planning selects the final queued block side-car votes
 */
class PeriodDataQueue {
 public:
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
   * Pops and returns front period data, cert votes for processing, and sender node id.
   *
   * Throws `std::runtime_error` if called while no raw queue entry is available.
   */
  std::tuple<PeriodData, std::vector<std::shared_ptr<PbftVote>>, dev::p2p::NodeID> pop();

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
   * Returns last queued PBFT block, or nullptr when queue is empty.
   */
  std::shared_ptr<PbftBlock> lastPbftBlock() const;

  /**
   * Returns last queued PBFT block hash from Rust-owned queue metadata, or nullopt when queue is empty.
   *
   * This avoids materializing the live `PeriodData` payload when PBFT manager only needs the compact chain-link fact.
   */
  std::optional<blk_hash_t> lastPbftBlockHash() const;

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
  const QueuedPayload& frontPayload(uint64_t expected_entry_id) const;
  const QueuedPayload& backPayload(uint64_t expected_entry_id) const;

  mutable std::shared_mutex queue_access_;
  ::rust::Box<rustaxa::BridgePeriodDataQueue> rust_queue_;
  std::deque<QueuedPayload> queued_payloads_;
  std::vector<std::shared_ptr<PbftVote>> last_block_cert_votes_;
  uint64_t next_entry_id_{1};
};

/** @}*/

}  // namespace taraxa
