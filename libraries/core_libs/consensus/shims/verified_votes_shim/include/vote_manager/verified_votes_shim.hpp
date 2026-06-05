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
   * Returns whether `vote_hash` is retained in Rust validation replay state.
   *
   * Purpose:
   * - Lets Rust-mode `VoteManager::voteAlreadyValidated` query the same Rust
   *   runtime that owns PBFT vote admission and verified-vote state.
   *
   * Invariants:
   * - No legacy validation cache is read in Rust mode.
   */
  bool replayContains(const vote_hash_t& vote_hash) const;

  /**
   * Inserts `vote_hash` into Rust validation replay state.
   *
   * Purpose:
   * - Preserves existing validation replay timing while moving replay storage
   *   into the same Rust runtime that owns admission.
   *
   * Outputs:
   * - true only when the hash was newly inserted.
   */
  bool replayInsert(const vote_hash_t& vote_hash) const;

  /**
   * Plans or computes the PBFT `2t+1` threshold from Rust runtime cache state.
   *
   * Inputs:
   * - `fact`: scalar PBFT-chain/FinalChain facts collected by `VoteManager`.
   *
   * Outputs:
   * - Rust threshold lookup result with stable status and cache-hit flags.
   *
   * Invariants:
   * - Cache ownership is co-located with admission and verified-vote state.
   */
  rustaxa::PbftTwoTPlusOneThresholdPlan twoTPlusOneThreshold(const rustaxa::PbftTwoTPlusOneThresholdFact& fact) const;

  /**
   * Validates canonical PBFT vote bytes through the unified Rust vote runtime.
   *
   * Inputs:
   * - `canonical_vote_rlp`: signed unweighted PBFT vote bytes.
   * - `validation_facts`: FinalChain/key/config facts supplied by `VoteManager`.
   *
   * Outputs:
   * - Rust validation result plus explicit replay-cache mutation facts.
   *
   * Invariants:
   * - Replay marking is applied inside the same Rust runtime that owns
   *   admission and verified-vote state.
   */
  rustaxa::PbftVoteRuntimeValidationResult validateCanonicalVote(
      rust::Slice<const uint8_t> canonical_vote_rlp, rustaxa::PbftVoteValidationExternalFacts validation_facts) const;

  /**
   * Returns flattened verified-vote objects from all indexed voted values.
   *
   * Invariants:
   * - Production-admitted votes are materialized from Rust-retained weighted
   *   payload bytes, not from the temporary `live_votes_` sidecar map.
   * - Compatibility/test helper inserts that do not retain weighted payloads
   *   may still use `live_votes_` until those helpers are removed.
   * - Missing both retained payload and compatibility sidecar is a hard error.
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
   *
   * Invariants:
   * - When Rust has a 2t+1 mapping, Rust must also have retained weighted
   *   payload bytes for every selected vote hash. Missing payloads are bridge
   *   invariant errors, not partial results.
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
   * Runs one Rust-owned PBFT vote admission transition against this facade's
   * verified-vote runtime.
   *
   * Inputs:
   * - `canonical_vote_rlp`: signed unweighted vote bytes for the incoming vote.
   * - `validation_facts`: FinalChain/key facts collected by `VoteManager`.
   * - `flags`: ingress and reward-vote flags for progress planning.
   * - `context`: current PBFT period/round and optional 2t+1 threshold facts.
   *
   * Outputs:
   * - A flat Rust mutation/executor report with validation, replay, insertion,
   *   peer-known, proposed-block sidecar, gossip, slashing, persistence,
   *   threshold, and PBFT-progress intents.
   *
   * Invariants:
   * - This call mutates only Rust verified-vote state and retained payload
   *   sidecars. It does not attach a live C++ `PbftVote` object or execute
   *   network/storage/slashing side effects; callers must execute returned
   *   intents and call `attachRuntimeAcceptedVote` after hydrating the accepted
   *   sidecar.
   */
  rustaxa::PbftVoteAdmissionRuntimeResult admitValidatedVote(rust::Slice<const uint8_t> canonical_vote_rlp,
                                                             rustaxa::PbftVoteValidationExternalFacts validation_facts,
                                                             rustaxa::PbftVoteEventFactFlags flags,
                                                             rustaxa::PbftVoteProgressContext context);

  /**
   * Attaches the C++ live sidecar for a vote already accepted by Rust runtime.
   *
   * Inputs:
   * - `vote`: live `PbftVote` object with calculated weight matching Rust.
   * - `result`: runtime admission result returned by `admitValidatedVote`.
   *
   * Outputs:
   * - The inserted voted-value bucket when Rust inserted the vote.
   * - Empty optional when the runtime report was not an accepted insertion.
   *
   * Error behavior:
   * - Hash/weight/report mismatches are hard invariant errors because Rust has
   *   already mutated the authoritative verified-vote state.
   */
  std::optional<VotesWithWeight> attachRuntimeAcceptedVote(const std::shared_ptr<PbftVote>& vote,
                                                           const rustaxa::PbftVoteAdmissionRuntimeResult& result);

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
  std::shared_ptr<PbftVote> materializeWeightedPayload(const rustaxa::PbftVoteStorageRecord& record) const;
  std::shared_ptr<PbftVote> materializeVoteForSnapshot(const rustaxa::VerifiedVotePayload& vote_data) const;
  const std::shared_ptr<PbftVote>& requireLiveVote(const vote_hash_t& vote_hash) const;
  VotesWithWeight requireInsertedVotesWithWeightLocked(const std::shared_ptr<PbftVote>& vote,
                                                       uint64_t total_weight) const;
  PeriodVerifiedVotesMap buildSnapshotState() const;
  void pruneLiveVotesToSnapshotLocked();

  mutable std::shared_mutex verified_votes_access_;
  mutable ::rust::Box<rustaxa::BridgeVerifiedVotes> rust_verified_votes_;
  std::unordered_map<vote_hash_t, std::shared_ptr<PbftVote>> live_votes_;

  LOG_OBJECTS_DEFINE
};

}  // namespace taraxa
