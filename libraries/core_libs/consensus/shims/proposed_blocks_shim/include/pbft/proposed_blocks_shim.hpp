#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <map>
#include <memory>
#include <optional>
#include <vector>

#include "common/types.hpp"
#include "pbft/pbft_service.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

class PbftBlock;

/**
 * Rust-mode proposed PBFT block cache facade.
 *
 * This class preserves the `ProposedBlocks` API required by current call sites while routing deterministic period/hash
 * membership, cleanup, and validation-flag state to the shared Rust PBFT service. The unused legacy-only
 * `checkOldBlocksPresence` diagnostic is intentionally not exposed. This facade must not inherit from or delegate to
 * the legacy C++ implementation.
 *
 * Invariants:
 * - Rust owns the canonical `(period, block hash) -> validation flag` index
 * - Rust retains canonical block RLP and metadata; C++ materializes temporary `PbftBlock` objects only for
 * compatibility return values
 * - every instance shares the application PBFT service; it never restores or owns an independent proposed-block index
 * - Rust owns synchronization, so the C++ facade adds no mutex around service calls
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
   * Creates a facade over the application PBFT service's proposed-block index.
   *
   * `pbft_service` must be non-null. The shared service has already restored persisted proposals before publication.
   */
  explicit ProposedBlocks(SharedPbftService pbft_service);
  ~ProposedBlocks();

  ProposedBlocks(const ProposedBlocks&) = delete;
  ProposedBlocks(ProposedBlocks&&) = delete;
  ProposedBlocks& operator=(const ProposedBlocks&) = delete;
  ProposedBlocks& operator=(ProposedBlocks&&) = delete;

  /**
   * Inserts a proposed PBFT block.
   *
   * `save_to_db` must be true. Rust persists and publishes the block atomically to the authoritative service index,
   * returning true only when the period/hash was not already present. Tentative local wallet candidates must use the
   * stateless leader-candidate path and are never inserted through this facade.
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
   * Compatibility no-op for the former explicit restore boundary.
   *
   * Purpose:
   * - Preserves the legacy public method while service construction now restores proposals before the service is shared
   *   with C++ facades. Production startup no longer calls this method.
   *
   * Inputs/outputs:
   * - Returns zero because there is no second restore step or independent index.
   *
   * Edge behavior:
   * - Does not mutate service state.
   */
  size_t restoreFromStorage();

  /**
   * Returns a proposed block and its validation flag for `period`/`block_hash`.
   *
   * Rust supplies canonical RLP from the service snapshot and C++ materializes a temporary `PbftBlock`. Missing entries
   * return empty; malformed retained RLP propagates the `PbftBlock` construction failure. The read does not mutate or
   * persist state.
   */
  std::optional<std::pair<std::shared_ptr<PbftBlock>, bool>> getPbftProposedBlock(PbftPeriod period,
                                                                                  const blk_hash_t& block_hash) const;

  /**
   * Returns compact proposed-block metadata for `period`/`block_hash`.
   *
   * The result contains the Rust-owned pivot hash and validation flag without materializing block RLP. Missing entries
   * return empty and the read has no side effects.
   */
  std::optional<ProposedBlockMetadata> getPbftProposedBlockMetadata(PbftPeriod period,
                                                                    const blk_hash_t& block_hash) const;

  /**
   * Returns true if `period` contains `block_hash`.
   *
   * This is a service-memory membership read. It performs no storage I/O and does not retain a Rust reference after the
   * call returns.
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
   *
   * Rust returns one owned point-in-time snapshot and C++ materializes temporary blocks grouped in deterministic period
   * order. An empty service index returns an empty map; malformed retained RLP propagates construction failure.
   */
  std::map<PbftPeriod, std::vector<std::shared_ptr<PbftBlock>>> getProposedBlocks() const;

 private:
  static std::array<uint8_t, 32> toBridgeHash(const blk_hash_t& hash);
  static blk_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash);
  static rust::Vec<uint8_t> toBridgeBytes(const bytes& block_rlp);
  static std::shared_ptr<PbftBlock> makeBlock(const rust::Vec<uint8_t>& block_rlp);

  SharedPbftService pbft_service_;
};

}  // namespace taraxa
