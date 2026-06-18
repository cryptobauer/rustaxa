#pragma once

#include <cstddef>
#include <map>
#include <memory>
#include <optional>
#include <shared_mutex>
#include <string>
#include <vector>

#include "common/types.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

class DbStorage;
class PbftBlock;
class Vote;

/**
 * Rust-mode proposed PBFT block cache facade.
 *
 * This class preserves the public C++ `ProposedBlocks` API while routing deterministic period/hash membership, cleanup,
 * and validation-flag state to Rust. It is a standalone facade and must not inherit from or delegate to
 * `ProposedBlocksOld`.
 *
 * Invariants:
 * - Rust owns the canonical `(period, block hash) -> validation flag` index
 * - C++ owns live `PbftBlock` objects until the PBFT block model is ported
 * - Rust storage owns proposed-block save, startup restore, and stale-proposal cleanup when a storage handle is available
 */
class ProposedBlocks {
 public:
  /**
   * Compact Rust-owned metadata for one proposed block.
   *
   * `pivot_hash` and `is_valid` can be read without reconstructing a temporary C++ `PbftBlock` object from retained
   * block RLP. Callers that need validation or public return values must still materialize through
   * `getPbftProposedBlock()`.
   */
  struct ProposedBlockMetadata {
    blk_hash_t pivot_hash;
    bool is_valid = false;
  };

  /**
   * Creates an empty Rust-backed proposed-block index.
   *
   * `db` may be null for temporary/local leader selection caches that do not request persistence.
   */
  explicit ProposedBlocks(std::shared_ptr<DbStorage> db);
  ~ProposedBlocks();

  ProposedBlocks(const ProposedBlocks&) = delete;
  ProposedBlocks(ProposedBlocks&&) = delete;
  ProposedBlocks& operator=(const ProposedBlocks&) = delete;
  ProposedBlocks& operator=(ProposedBlocks&&) = delete;

  /**
   * Inserts a proposed PBFT block.
   *
   * When `save_to_db` is true, Rust persists the block before duplicate detection to match legacy behavior. Returns true
   * only when the Rust in-memory proposal index did not already contain the period/hash.
   */
  bool pushProposedPbftBlock(const std::shared_ptr<PbftBlock>& proposed_block, bool save_to_db = true);

  /**
   * Marks a proposed block as valid.
   *
   * Throws `std::runtime_error` if the block is not present in the Rust index.
   */
  void markBlockAsValid(const std::shared_ptr<PbftBlock>& proposed_block);

  /**
   * Restores the Rust index for proposed PBFT blocks from storage.
   *
   * Purpose:
   * - Hydrates Rust-owned proposed-block metadata from persisted RLP without constructing
   *   live C++ `PbftBlock` objects during PBFT startup.
   *
   * Inputs/outputs:
   * - Requires the constructor to have received a Rust storage handle owner.
   * - Returns the number of persisted proposals newly inserted into the Rust index.
   *
   * Edge behavior:
   * - Throws `std::runtime_error` on missing DB, storage failures, corrupt PBFT block RLP,
   *   or storage key/hash mismatch.
   */
  size_t restoreFromStorage();

  /**
   * Returns a proposed block and its validation flag for `period`/`block_hash`.
   */
  std::optional<std::pair<std::shared_ptr<PbftBlock>, bool>> getPbftProposedBlock(PbftPeriod period,
                                                                                  const blk_hash_t& block_hash) const;

  /**
   * Returns compact proposed-block metadata for `period`/`block_hash`.
   */
  std::optional<ProposedBlockMetadata> getPbftProposedBlockMetadata(PbftPeriod period,
                                                                    const blk_hash_t& block_hash) const;

  /**
   * Returns true if `period` contains `block_hash`.
   */
  bool isInProposedBlocks(PbftPeriod period, const blk_hash_t& block_hash) const;

  /**
   * Removes proposed blocks with period lower than `period`.
   *
   * Purpose:
   * - Keeps Rust-owned proposed-block metadata and persisted proposed-block storage in sync
   *   when PBFT advances to a newer period.
   *
   * Inputs/outputs:
   * - `period` is the first retained PBFT period.
   * - When `rust_storage_` is present, Rust deletes stale storage keys in one batch before
   *   mutating the Rust index.
   * - When `db_` is null, only the in-memory Rust index is cleaned.
   *
   * Edge behavior:
   * - Throws `std::runtime_error` on Rust storage or bridge failures.
   */
  void cleanupProposedPbftBlocksByPeriod(PbftPeriod period);

  /**
   * Returns the legacy old-blocks diagnostic message when stale proposals are present.
   */
  std::optional<std::string> checkOldBlocksPresence(PbftPeriod current_period) const;

  /**
   * Returns proposed blocks grouped by PBFT period.
   */
  std::map<PbftPeriod, std::vector<std::shared_ptr<PbftBlock>>> getProposedBlocks() const;

 private:
  static std::array<uint8_t, 32> toBridgeHash(const blk_hash_t& hash);
  static blk_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash);
  static rust::Vec<uint8_t> toBridgeBytes(const bytes& block_rlp);
  static std::shared_ptr<PbftBlock> makeBlock(const rust::Vec<uint8_t>& block_rlp);

  mutable std::shared_mutex proposed_blocks_mutex_;
  std::shared_ptr<DbStorage> storage_owner_;
  const rustaxa::BridgeStorage* rust_storage_ = nullptr;
  ::rust::Box<rustaxa::BridgeProposedBlocks> rust_blocks_;
};

}  // namespace taraxa
