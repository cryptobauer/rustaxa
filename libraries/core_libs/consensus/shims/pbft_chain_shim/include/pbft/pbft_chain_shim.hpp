#pragma once

#include <iostream>
#include <memory>
#include <optional>
#include <shared_mutex>
#include <string>

#include "logger/logger.hpp"
#include "pbft/pbft_block.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

/** @addtogroup PBFT
 * @{
 */

class DbStorage;
class Vote;
class DagBlock;
struct Transaction;

/**
 * Rust-mode PBFT chain facade.
 *
 * This class preserves the public C++ `PbftChain` API while routing deterministic head state transitions and validation
 * checks to Rust. It is a standalone facade and must not inherit from or delegate to `PbftChainOld`.
 *
 * Invariants:
 * - Rust restores persisted PBFT head JSON and recovers the hidden last non-null DAG anchor from native storage
 * - public PBFT head JSON formatting remains owned by the C++ shim for compatibility with existing callers
 * - Rust owns in-memory size, non-empty-size, latest block hash, and latest non-null DAG anchor state
 * - `getJsonStrForBlock` is a pure preview and does not mutate state or write storage
 * - `updatePbftChain` mutates only in-memory state; `PbftManager` remains responsible for batched persistence
 */
class PbftChain {
 public:
  /**
   * Creates a Rust-backed PBFT chain and restores head state through `rustaxa-storage`.
   *
   * If no persisted head exists, Rust initializes the legacy zero-head record through the native storage module.
   */
  explicit PbftChain([[maybe_unused]] addr_t node_addr, std::shared_ptr<DbStorage> db);
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
   * Throws `std::runtime_error` if the hash is not present, rather than falling back to legacy `PbftChainOld`.
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
   * Updates the in-memory Rust PBFT chain head and returns a Rust-verifiable PBFT finalization live-action report.
   *
   * Inputs are the finalized block hash, finalized anchor hash, and the Rust-planned finalization write intent.
   * The returned report carries post-mutation chain size/head/anchor facts that Rust validates before advancing.
   */
  rustaxa::PbftFinalizationLiveMutationReport updatePbftChainForPbftFinalization(
      blk_hash_t const& pbft_block_hash, blk_hash_t const& anchor_hash,
      const rustaxa::PbftFinalizationStorageWritePlan& write_intent);

  /**
   * Verifies that `pbft_block` extends the current Rust PBFT head.
   *
   * Returns false for period or previous-hash mismatch and throws only for unexpected bridge errors.
   */
  bool checkPbftBlockValidation(const std::shared_ptr<PbftBlock>& pbft_block) const;

 private:
  mutable std::shared_mutex chain_head_access_;
  std::shared_ptr<DbStorage> db_ = nullptr;
  std::optional<::rust::Box<rustaxa::BridgePbftChain>> rust_chain_;

  LOG_OBJECTS_DEFINE
};

std::ostream& operator<<(std::ostream& strm, PbftChain const& pbft_chain);

/** @}*/

}  // namespace taraxa
