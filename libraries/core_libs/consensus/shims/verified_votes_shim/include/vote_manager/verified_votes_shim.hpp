#pragma once

#include <array>
#include <optional>
#include <shared_mutex>
#include <unordered_map>
#include <vector>

#include "common/types.hpp"
#include "logger/logger.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

class PbftVote;

/**
 * Rust-mode verified-votes facade.
 *
 * This class preserves the public C++ `VerifiedVotes` API used by `VoteManager`
 * while replacing the legacy concrete type with a shim-local implementation in
 * Rust-enabled builds. The shim is standalone and must not inherit from or
 * delegate/fallback to `VerifiedVotesOld`.
 *
 * Ownership and invariants:
 * - C++ owns live `PbftVote` objects and map containers referenced by vote
 *   processing code.
 * - The facade owns synchronization (`verified_votes_access_`) and provides the
 *   same read/write locking boundary as the legacy implementation.
 * - All period/round/step uniqueness and vote-weight accumulation semantics
 *   remain API-compatible with existing `VoteManager` call sites.
 */
class VerifiedVotes {
 public:
  /**
   * Outcome of one atomic verified-vote insert attempt.
   *
   * Exactly one of these states is expected:
   * - `conflicting_vote` set when unique-voter conflict is detected.
   * - `votes_with_weight` set when vote was inserted into voted-value bucket.
   * - both empty when vote hash already exists in voted-value bucket.
   */
  struct AtomicInsertOutcome {
    std::optional<std::shared_ptr<PbftVote>> conflicting_vote;
    std::optional<VotesWithWeight> votes_with_weight;
  };

  /**
   * Constructs an empty verified-votes index.
   *
   * Inputs:
   * - `node_addr`: retained only for API compatibility with existing call sites.
   *
   * Outputs:
   * - Initializes an empty, thread-safe vote index.
   *
   * Edge behavior:
   * - No storage is read or written during construction.
   */
  explicit VerifiedVotes(addr_t node_addr);

  /**
   * Returns total verified-vote count across all periods/rounds/steps.
   */
  uint64_t size() const;

  /**
   * Returns flattened verified-vote objects from all indexed voted values.
   */
  std::vector<std::shared_ptr<PbftVote>> votes() const;

  /**
   * Returns all round votes for `period`, or empty optional when absent.
   */
  std::optional<const RoundVerifiedVotesMap> getPeriodVotes(PbftPeriod period) const;

  /**
   * Returns round votes for (`period`, `round`), or empty optional when absent.
   */
  std::optional<const RoundVerifiedVotes> getRoundVotes(PbftPeriod period, PbftRound round) const;

  /**
   * Returns step votes for (`period`, `round`, `step`), or empty optional when absent.
   */
  std::optional<const StepVotes> getStepVotes(PbftPeriod period, PbftRound round, PbftStep step) const;

  /**
   * Returns the tracked 2t+1 voted block for (`period`, `round`, `type`) when available.
   */
  std::optional<VotedBlock> getTwoTPlusOneVotedBlock(PbftPeriod period, PbftRound round,
                                                     TwoTPlusOneVotedBlockType type) const;

  /**
   * Returns all votes for the selected 2t+1 voted block, or empty vector when missing.
   */
  std::vector<std::shared_ptr<PbftVote>> getTwoTPlusOneVotedBlockVotes(PbftPeriod period, PbftRound round,
                                                                       TwoTPlusOneVotedBlockType type) const;

  /**
   * Removes votes for periods older than `pbft_period`.
   */
  void cleanupVotesByPeriod(PbftPeriod pbft_period);

  /**
   * Enforces unique voter rules for (`period`, `round`, `step`).
   *
   * Returns:
   * - `std::nullopt` when vote is accepted as unique (including second next-vote
   *   null/non-null pair edge case).
   * - Existing conflicting vote when duplicate-voter conflict is detected.
   */
  std::optional<std::shared_ptr<PbftVote>> insertUniqueVoter(const std::shared_ptr<PbftVote>& vote);

  /**
   * Inserts vote into voted-value bucket and accumulates total bucket weight.
   *
   * Returns updated bucket data when inserted, or empty optional when vote hash
   * is already present.
   */
  std::optional<VotesWithWeight> insertVotedValue(const std::shared_ptr<PbftVote>& vote);

  /**
   * Atomically performs unique-voter and voted-value inserts for `vote`.
   *
   * This preserves one lock boundary across both Rust index updates so
   * `VoteManager` can process one consistent insertion outcome.
   */
  AtomicInsertOutcome insertVerifiedVoteAtomic(const std::shared_ptr<PbftVote>& vote);

  /**
   * Sets `network_t_plus_one_step` for vote's (`period`, `round`) entry when present.
   */
  void setNetworkTPlusOneStep(std::shared_ptr<PbftVote> vote);

  /**
   * Stores 2t+1 voted block hash/step marker for vote's (`period`, `round`).
   *
   * Returns:
   * - `true` when this call inserted a new mapping.
   * - `false` when mapping already existed or round is missing.
   */
  bool insertTwoTPlusOneVotedBlock(TwoTPlusOneVotedBlockType type, std::shared_ptr<PbftVote> vote);

 private:
  static std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash);
  static std::array<uint8_t, 20> toBridgeAddress(const addr_t& address);
  static uint256_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash);
  rustaxa::VerifiedVotePayload toBridgeVotePayload(const std::shared_ptr<PbftVote>& vote) const;
  const std::shared_ptr<PbftVote>& requireLiveVote(const vote_hash_t& vote_hash) const;
  VotesWithWeight requireInsertedVotesWithWeightLocked(const std::shared_ptr<PbftVote>& vote,
                                                       uint64_t total_weight) const;
  PeriodVerifiedVotesMap buildSnapshotState() const;
  void pruneLiveVotesToSnapshotLocked();

  mutable std::shared_mutex verified_votes_access_;
  ::rust::Box<rustaxa::BridgeVerifiedVotes> rust_verified_votes_;
  std::unordered_map<vote_hash_t, std::shared_ptr<PbftVote>> live_votes_;

  LOG_OBJECTS_DEFINE
};

}  // namespace taraxa
