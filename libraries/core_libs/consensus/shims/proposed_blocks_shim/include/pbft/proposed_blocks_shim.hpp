#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <map>
#include <memory>
#include <optional>
#include <shared_mutex>
#include <vector>

#include "common/types.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

class DbStorage;
class PbftBlock;

/**
 * Rust-mode proposed PBFT block cache facade.
 *
 * This class preserves the `ProposedBlocks` API required by current call sites while routing deterministic period/hash
 * membership, cleanup, and validation-flag state to Rust. The unused legacy-only `checkOldBlocksPresence` diagnostic is
 * intentionally not exposed. This is a standalone facade and must not inherit from or delegate to the legacy C++
 * implementation.
 *
 * Invariants:
 * - Rust owns the canonical `(period, block hash) -> validation flag` index
 * - Rust retains canonical block RLP and metadata; C++ materializes temporary `PbftBlock` objects only for
 * compatibility return values
 * - every instance is constructed with Rust storage, which owns proposed-block save, startup restore, and
 *   stale-proposal cleanup
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
   * Creates an empty storage-backed Rust proposed-block index.
   *
   * `db` must be non-null and expose a Rust storage handle. Construction throws `std::runtime_error` when that
   * precondition is not met. The Rust index retains the storage ownership needed by later persistence, restore, and
   * cleanup operations; this facade does not retain a redundant C++ `DbStorage` owner.
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
   * When `save_to_db` is true, Rust persists the block before duplicate detection to match legacy behavior. Returns
   * true only when the Rust in-memory proposal index did not already contain the period/hash.
   */
  bool pushProposedPbftBlock(const std::shared_ptr<PbftBlock>& proposed_block, bool save_to_db = true);

  /**
   * Marks a proposed block as valid.
   *
   * Throws `std::runtime_error` if the block is not present in the Rust index.
   */
  void markBlockAsValid(const std::shared_ptr<PbftBlock>& proposed_block);

  /**
   * Marks a proposed block as valid by compact Rust-owned identity.
   *
   * Purpose:
   * - Lets Rust-planned PBFT manager mark-valid commands mutate the proposed-block
   *   index without reusing a materialized C++ `PbftBlock` object as the identity carrier.
   *
   * Inputs/outputs:
   * - `period` and `block_hash` identify an entry already retained by the Rust index.
   * - The Rust validation flag is set for that entry.
   *
   * Edge behavior:
   * - Throws `std::runtime_error` when the period/hash entry is missing or the bridge fails.
   */
  void markBlockAsValid(PbftPeriod period, const blk_hash_t& block_hash);

  /**
   * Restores the Rust index for proposed PBFT blocks from storage.
   *
   * Purpose:
   * - Hydrates Rust-owned proposed-block metadata from persisted RLP without constructing
   *   live C++ `PbftBlock` objects during PBFT startup.
   *
   * Inputs/outputs:
   * - Uses the Rust storage handle retained by the index at construction.
   * - Returns the number of persisted proposals newly inserted into the Rust index.
   *
   * Edge behavior:
   * - Throws `std::runtime_error` on storage failures, corrupt PBFT block RLP, or storage key/hash mismatch.
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
   * - Rust deletes stale storage keys in one batch before mutating the Rust index.
   *
   * Edge behavior:
   * - Throws `std::runtime_error` on Rust storage or bridge failures.
   */
  void cleanupProposedPbftBlocksByPeriod(PbftPeriod period);

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
  ::rust::Box<rustaxa::BridgeProposedBlocks> rust_blocks_;
};

}  // namespace taraxa
