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
   * Rust-evaluated round-advance decision.
   *
   * Purpose:
   * - Carries the highest round in `period` that has 2t+1 next votes, plus
   *   voted-block details used by C++ callers for logging.
   *
   * Invariants:
   * - `new_round == supporting_round + 1`.
   * - `supporting_round` is the greatest round `>= current_pbft_round` that
   *   has a NextVotedBlock or NextVotedNullBlock mapping.
   */
  struct RoundAdvanceDecision {
    PbftRound new_round{0};
    PbftRound supporting_round{0};
    VotedBlock voted_block{};
  };

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
   * Outcome of one Rust-owned verified-vote add with optional threshold effects.
   *
   * Purpose:
   * - Preserves the exact live-sidecar facts C++ executors still need while
   *   exposing the flat Rust mutation report consumed by protocol planners.
   *
   * Outputs:
   * - `report` carries inserted/duplicate/conflict/threshold facts from Rust.
   * - `conflicting_vote` is populated when Rust selected an existing live vote
   *   for slashing.
   * - `votes_with_weight` is populated only after a successful insertion and
   *   mirrors the voted-value bucket used for current-round 2t+1 persistence.
   *
   * Invariants:
   * - Live sidecars are inserted only after Rust accepts the vote.
   * - Missing live sidecars for Rust-selected hashes remain hard invariant
   *   errors, matching the existing shim facade contract.
   */
  struct AddVerifiedVoteOutcome {
    rustaxa::VerifiedVoteAddOutcome report{};
    std::optional<std::shared_ptr<PbftVote>> conflicting_vote;
    std::optional<VotesWithWeight> votes_with_weight;
  };

  /**
   * Rust-evaluated threshold effects for one inserted verified vote.
   *
   * Purpose:
   * - Captures all Rust-owned threshold side effects consumed by
   *   `VoteManager::addVerifiedVote` in one decision object.
   *
   * Inputs used to derive this decision:
   * - vote metadata (`period`, `round`, `step`, type, voted block hash),
   * - voted-value `total_weight`,
   * - caller-computed `two_t_plus_one` threshold.
   *
   * Outputs:
   * - `set_network_t_plus_one_step` when this vote advanced the stored
   *   network t+1 step marker.
   * - `inserted_two_t_plus_one_voted_block_type` when this vote crossed 2t+1
   *   and inserted a new first-writer mapping for its vote type.
   *
   * Invariants:
   * - Empty `inserted_two_t_plus_one_voted_block_type` means either
   *   `total_weight < two_t_plus_one` or mapping already existed.
   * - When `inserted_two_t_plus_one_voted_block_type` is set, Rust state already
   *   contains the corresponding mapping for vote's (`period`, `round`).
   */
  struct ThresholdDecision {
    bool set_network_t_plus_one_step{false};
    std::optional<TwoTPlusOneVotedBlockType> inserted_two_t_plus_one_voted_block_type;
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
   * Atomically inserts a verified vote and applies Rust-owned threshold effects.
   *
   * Inputs:
   * - `vote`: live C++ vote sidecar with a non-zero calculated weight.
   * - `two_t_plus_one`: optional threshold for this vote's period/type. When
   *   empty, insertion still runs but threshold side effects are skipped.
   *
   * Outputs:
   * - A flat Rust mutation report plus live sidecar references needed by the
   *   C++ `VoteManager` executor.
   */
  AddVerifiedVoteOutcome addVerifiedVoteWithThreshold(const std::shared_ptr<PbftVote>& vote,
                                                      std::optional<uint64_t> two_t_plus_one);

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

  /**
   * Applies Rust-owned t+1 and 2t+1 threshold effects for one vote and
   * returns a summary decision for `VoteManager`.
   */
  ThresholdDecision decideThresholdEffects(const std::shared_ptr<PbftVote>& vote, uint64_t total_weight,
                                           uint64_t two_t_plus_one);

  /**
   * Evaluates whether the period can advance to a higher round from Rust
   * next-vote 2t+1 mappings.
   *
   * Inputs:
   * - `current_pbft_period`: period to evaluate.
   * - `current_pbft_round`: current round lower bound.
   *
   * Outputs:
   * - Empty optional when no qualifying next-vote 2t+1 mapping exists.
   * - Highest qualifying round + 1 plus supporting mapping details otherwise.
   *
   * Edge behavior:
   * - Ignores mappings from older rounds (`< current_pbft_round`).
   * - Prefers `NextVotedBlock` over `NextVotedNullBlock` when both exist for
   *   the same round to match legacy C++ selection order.
   */
  std::optional<RoundAdvanceDecision> determineRoundAdvance(PbftPeriod current_pbft_period,
                                                            PbftRound current_pbft_round) const;

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
