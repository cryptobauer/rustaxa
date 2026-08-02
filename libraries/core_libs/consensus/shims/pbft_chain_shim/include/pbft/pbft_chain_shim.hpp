#pragma once

#include <iostream>
#include <memory>
#include <shared_mutex>
#include <string>

#include "logger/logger.hpp"
#include "pbft/pbft_block.hpp"
#include "pbft/pbft_service.hpp"

namespace taraxa {

/** @addtogroup PBFT
 * @{
 */

/**
 * Rust-mode PBFT chain facade.
 *
 * This class preserves the public C++ `PbftChain` API while routing deterministic head state transitions and validation
 * checks to the shared Rust `PbftService` holder. It does not inherit from or delegate to the legacy C++
 * implementation.
 *
 * Invariants:
 * - Rust owns persisted PBFT chain state, and restores it into the shared service on construction.
 * - JSON rendering for head state remains owned by the C++ shim for C++-side API compatibility.
 * - in-memory chain fields and runtime validation state are held by the shared Rust service.
 * - `getJsonStrForBlock` is a pure preview and does not mutate state or write storage
 * - `updatePbftChain` mutates only in-memory state; `PbftManager` remains responsible for batched persistence
 */
class PbftChain {
 public:
  /**
   * Creates the PBFT chain facade over an already-configured shared Rust service.
   *
   * The shared holder keeps the Rust service alive for every facade operation; no nested Rust reference is retained.
   */
  explicit PbftChain([[maybe_unused]] addr_t node_addr, SharedPbftService pbft_service);
  ~PbftChain();

  PbftChain(const PbftChain&) = delete;
  PbftChain(PbftChain&&) = delete;
  PbftChain& operator=(const PbftChain&) = delete;
  PbftChain& operator=(PbftChain&&) = delete;

  /**
   * Returns the PBFT chain head hash.
   */
  blk_hash_t getHeadHash() const;

  /**
   * Returns PBFT chain size including empty-anchor PBFT blocks.
   */
  PbftPeriod getPbftChainSize() const;

  /**
   * Returns PBFT chain size excluding empty-anchor PBFT blocks.
   */
  PbftPeriod getPbftChainSizeExcludingEmptyPbftBlocks() const;

  /**
   * Returns the latest PBFT block hash in the chain.
   */
  blk_hash_t getLastPbftBlockHash() const;

  /**
   * Returns the latest non-null DAG anchor in the chain, or zero when none exists.
   */
  blk_hash_t getLastNonNullPbftBlockAnchor() const;

  /**
   * Materializes a PBFT block by hash from Rust storage.
   *
   * Throws `std::runtime_error` if the hash is not present; no legacy storage lookup is attempted.
   */
  PbftBlock getPbftBlockInChain(blk_hash_t const& pbft_block_hash);

  /**
   * Returns the current PBFT chain head as legacy JsonCpp styled JSON.
   */
  std::string getJsonStr() const;

  /**
   * Returns the legacy JSON that would result from appending `block_hash`.
   *
   * `null_anchor` controls whether the non-empty PBFT size is incremented. This method does not mutate state.
   */
  std::string getJsonStrForBlock(blk_hash_t const& block_hash, bool null_anchor) const;

  /**
   * Returns true if Rust storage has a PBFT block-period index entry for `pbft_block_hash`.
   */
  bool findPbftBlockInChain(blk_hash_t const& pbft_block_hash);

  /**
   * Updates only in-memory Rust PBFT chain head state.
   *
   * Persistence is deliberately not performed here; callers must keep the existing batch write flow.
   */
  void updatePbftChain(blk_hash_t const& pbft_block_hash, blk_hash_t const& anchor_hash);

  /**
   * Verifies that `pbft_block` extends the current Rust PBFT head.
   *
   * Returns false for period or previous-hash mismatch and throws only for unexpected bridge errors.
   */
  bool checkPbftBlockValidation(const std::shared_ptr<PbftBlock>& pbft_block) const;

 private:
  mutable std::shared_mutex chain_head_access_;
  SharedPbftService pbft_service_;

  LOG_OBJECTS_DEFINE
};

std::ostream& operator<<(std::ostream& strm, PbftChain const& pbft_chain);

/** @}*/

}  // namespace taraxa
