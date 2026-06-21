#pragma once

#include <array>
#include <memory>
#include <shared_mutex>
#include <unordered_map>
#include <vector>

#include "common/types.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "vote/pillar_vote.hpp"

namespace taraxa::pillar_chain {

/**
 * Rust-mode pillar-votes facade.
 *
 * This class preserves the public C++ `PillarVotes` API while delegating
 * deterministic vote validation and aggregation to Rust. C++ keeps a temporary
 * live `PillarVote` sidecar map only for insertion-time compatibility; returned
 * vote objects are materialized from Rust-retained payloads at public edges.
 *
 * No fallback to `PillarVotesOld` is used for production logic in this
 * Rust-enabled shim mode.
 */
class PillarVotes {
 public:
  /**
   * Legacy-compatible block vote bucket shape.
   *
   * The shim does not use this as authoritative state; it remains part of the
   * public type surface for existing C++ code that names `PillarVotes::WeightVotes`.
   */
  struct WeightVotes {
    std::unordered_map<vote_hash_t, std::pair<std::shared_ptr<PillarVote>, uint64_t /* vote weight */>> votes;
    uint64_t weight{0};
  };

  /**
   * Legacy-compatible period bucket shape.
   *
   * Rust owns the live threshold, uniqueness, and block aggregation state in
   * shim mode. This type is retained for source compatibility with the original
   * `PillarVotes` API.
   */
  struct PeriodVotes {
    std::unordered_map<blk_hash_t, WeightVotes> pillar_block_votes;
    std::unordered_map<addr_t, vote_hash_t> unique_voters;
    uint64_t threshold{0};
  };

  /**
   * Constructs an empty `PillarVotes` index.
   */
  PillarVotes();

  /**
   * Checks whether a vote hash exists in the Rust-backed index.
   */
  bool voteExists(const std::shared_ptr<PillarVote> vote) const;

  /**
   * Checks whether a vote is unique per period & voter.
   */
  bool isUniqueVote(const std::shared_ptr<PillarVote> vote) const;

#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  /**
   * Checks whether a Rust-recovered `(period, voter, vote_hash)` identity is unique.
   */
  bool isUniqueVoteIdentity(PbftPeriod period, const vote_hash_t& vote_hash, const addr_t& voter) const;
#endif

  /**
   * Checks if specified period data have been initialized.
   */
  bool periodDataInitialized(PbftPeriod period) const;

  /**
   * Initializes period data in the Rust index.
   */
  void initializePeriodData(PbftPeriod period, uint64_t threshold);

  /**
   * Adds one verified pillar vote.
   *
   * Returns true for accepted or duplicate votes for the same hash; false for
   * conflicting voter attempts.
   */
  bool addVerifiedVote(const std::shared_ptr<PillarVote>& vote, uint64_t validator_vote_count);
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  /**
   * Adds one verified pillar vote using a Rust-recovered voter.
   *
   * The caller supplies the voter identity produced by Rust inspection; this
   * path must not recover the voter from the C++ `PillarVote` sidecar.
   *
   * Returns true for accepted or duplicate votes for the same hash; false for
   * conflicting voter attempts.
   */
  bool addVerifiedVoteWithRecoveredVoter(const std::shared_ptr<PillarVote>& vote, uint64_t validator_vote_count,
                                         const addr_t& recovered_voter);
#endif

  /**
   * Returns all votes for `pillar_block_hash`, optionally threshold-filtered.
   */
  std::vector<std::shared_ptr<PillarVote>> getVerifiedVotes(PbftPeriod period, const blk_hash_t& pillar_block_hash,
                                                            bool above_threshold = false) const;

  /**
   * Removes all vote data for periods lower than `min_period`.
   */
  void eraseVotes(PbftPeriod min_period);

 private:
  static std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash);
  static std::array<uint8_t, 20> toBridgeAddress(const addr_t& address);
  static vote_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash);
  static rustaxa::PillarVotePayload toBridgePayload(const std::shared_ptr<PillarVote>& vote,
                                                    uint64_t validator_vote_count);
  static rustaxa::PillarVotePayload toBridgeLookupPayload(const std::shared_ptr<PillarVote>& vote);

  const std::shared_ptr<PillarVote>& requireLiveVote(const vote_hash_t& vote_hash) const;
  std::shared_ptr<PillarVote> materializeVoteRecord(const rustaxa::PillarVoteRecord& record) const;
  void trackVote(const std::shared_ptr<PillarVote>& vote);
  void pruneLiveVotesToSnapshotLocked();

  mutable std::shared_mutex mutex_;
  ::rust::Box<rustaxa::BridgePillarVotes> rust_pillar_votes_;
  std::unordered_map<vote_hash_t, std::shared_ptr<PillarVote>> live_votes_;
};

}  // namespace taraxa::pillar_chain
