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

class DbStorage;

/**
 * Rust-mode PBFT chain facade.
 *
 * This class preserves the public C++ `PbftChain` API while routing deterministic head state transitions and validation
 * checks to the application-owned Rust PBFT service. It does not inherit from or delegate to the legacy C++
 * implementation.
 *
 * Invariants:
 * - Rust restores persisted PBFT head JSON and recovers the hidden last non-null DAG anchor from native storage
 * - public PBFT head JSON formatting remains owned by the C++ shim for compatibility with existing callers
 * - the shared PBFT service owns in-memory size, non-empty-size, latest block hash, and latest non-null DAG anchor
 * state
 * - `getJsonStrForBlock` is a pure preview and does not mutate state or write storage
 * - `updatePbftChain` mutates only in-memory state; `PbftManager` remains responsible for batched persistence
 */
class PbftChain {
 public:
  /**
   * Creates a chain-only compatibility PBFT service and restores head state through `rustaxa-storage`.
   *
   * `db` must be non-null and expose a Rust storage handle. The Rust service clones that storage owner during
   * construction, so later block lookups do not depend on the C++ `DbStorage` lifetime.
   *
   * If no persisted head exists, Rust initializes the legacy zero-head record through the native storage module.
   */
  explicit PbftChain([[maybe_unused]] addr_t node_addr, std::shared_ptr<DbStorage> db);

  /**
   * Creates the production compatibility facade over the application-owned PBFT service.
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
