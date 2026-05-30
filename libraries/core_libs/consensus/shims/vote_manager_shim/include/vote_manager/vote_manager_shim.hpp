#pragma once

#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

/**
 * Rust-mode VoteManager overlay.
 *
 * Purpose:
 * - Delegates legacy-compatible vote-manager behavior to `VoteManagerOld`
 *   through inheritance while overriding the reward-vote reset persistence
 *   handoff in shim-owned code.
 *
 * Inputs:
 * - Constructor arguments are inherited from `VoteManagerOld`.
 *
 * Outputs and invariants:
 * - Inherited methods use the same base state as the overridden reset path.
 * - Reward-vote reset appends the stage-4 Rust finalization storage write to
 *   the caller-owned batch and mutates live reward metadata only after Rust
 *   accepts the durable write stage.
 *
 * Error and edge behavior:
 * - Missing certified-vote facts preserve upstream assert-and-return behavior.
 * - Rust appender rejection returns a rejected result and leaves reward metadata
 *   and extra reward vote tracking unchanged.
 */
class VoteManager : public VoteManagerOld {
 public:
  using VoteManagerOld::VoteManagerOld;

  void setNetwork(std::weak_ptr<Network> network);
  bool addVerifiedVote(const std::shared_ptr<PbftVote>& vote);
  bool voteInVerifiedMap(std::shared_ptr<PbftVote> const& vote) const;
  std::pair<bool, std::shared_ptr<PbftVote>> isUniqueVote(const std::shared_ptr<PbftVote>& vote) const;
  std::vector<std::shared_ptr<PbftVote>> getVerifiedVotes() const;
  uint64_t getVerifiedVotesSize() const;
  void cleanupVotesByPeriod(PbftPeriod pbft_period);
  std::vector<std::shared_ptr<PbftVote>> getProposalVotes(PbftPeriod period, PbftRound round) const;
  std::optional<PbftRound> determineNewRound(PbftPeriod current_pbft_period, PbftRound current_pbft_round);

  /**
   * Compatibility entrypoint for existing direct callers.
   *
   * Inputs:
   * - Certified vote period, round, step, block hash, and caller-owned storage
   *   batch.
   *
   * Outputs:
   * - Appends reward-vote reset persistence through Rust and updates inherited
   *   live reward state on success.
   *
   * Edge behavior:
   * - Logs and asserts on rejected Rust appender results, matching the legacy
   *   method's assert-on-missing-facts behavior.
   */
  void resetRewardVotes(PbftPeriod period, PbftRound round, PbftStep step, const blk_hash_t& block_hash, Batch& batch);

  std::pair<bool, std::vector<std::shared_ptr<PbftVote>>> checkRewardVotes(const std::shared_ptr<PbftBlock>& pbft_block,
                                                                           bool copy_votes);
  std::vector<std::shared_ptr<PbftVote>> getRewardVotes();
  PbftPeriod getRewardVotesPbftBlockPeriod();
  void saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote);
  std::vector<std::shared_ptr<PbftVote>> getOwnVerifiedVotes();
  void clearOwnVerifiedVotes(Batch& write_batch);
  std::shared_ptr<PbftVote> generateVoteWithWeight(const blk_hash_t& blockhash, PbftVoteTypes vote_type,
                                                   PbftPeriod period, PbftRound round, PbftStep step,
                                                   const WalletConfig& wallet);
  std::shared_ptr<PbftVote> generateVote(const blk_hash_t& blockhash, PbftVoteTypes type, PbftPeriod period,
                                         PbftRound round, PbftStep step, const WalletConfig& wallet);
  std::pair<bool, std::string> validateVote(const std::shared_ptr<PbftVote>& vote, bool strict = true) const;
  std::optional<uint64_t> getPbftTwoTPlusOne(PbftPeriod pbft_period, PbftVoteTypes vote_type) const;
  bool voteAlreadyValidated(const vote_hash_t& vote_hash) const;
  bool genAndValidateVrfSortition(PbftPeriod pbft_period, PbftRound pbft_round, const WalletConfig& wallet) const;
  std::optional<blk_hash_t> getTwoTPlusOneVotedBlock(PbftPeriod period, PbftRound round,
                                                     TwoTPlusOneVotedBlockType type) const;
  std::vector<std::shared_ptr<PbftVote>> getTwoTPlusOneVotedBlockVotes(PbftPeriod period, PbftRound round,
                                                                       TwoTPlusOneVotedBlockType type) const;
  StepVotes getStepVotes(PbftPeriod period, PbftRound round, PbftStep step) const;
  void setCurrentPbftPeriodAndRound(PbftPeriod pbft_period, PbftRound pbft_round);
  PbftStep getNetworkTplusOneNextVotingStep(PbftPeriod period, PbftRound round) const;

  /**
   * Finalization-aware reward-vote reset handoff.
   *
   * Inputs:
   * - `write_intent`: Rust-planned PBFT finalization storage intent carrying
   *   reward vote period, round, step, and block hash facts.
   * - `batch`: caller-owned PBFT finalization batch.
   *
   * Outputs:
   * - Rust appender status for the reward-vote reset stage.
   *
   * Invariants:
   * - The certified-vote bundle is selected from inherited live
   *   `VerifiedVotes` state.
   * - Inherited reward metadata and stale extra-reward tracking are mutated
   *   only after Rust returns `Applied` or `AlreadyApplied`.
   */
  rustaxa::PbftFinalizedPeriodApplyResult resetRewardVotesForFinalization(
      const rustaxa::PbftFinalizationStorageWritePlan& write_intent, Batch& batch);
  /**
   * Builds the Rust reward-vote reset storage stage without mutating live
   * reward metadata.
   *
   * Inputs:
   * - `write_intent`: Rust-planned finalization write intent carrying the
   *   certified vote period, round, step, and block hash.
   *
   * Outputs:
   * - A bridge stage containing the certified-vote bundle RLP and stale
   *   extra-reward vote hashes.
   *
   * Invariants:
   * - The stage is derived from inherited live `VerifiedVotes` sidecars.
   * - Missing or mismatched live state throws before the caller can commit a
   *   finalized-period persistence batch.
   * - Live reward metadata is unchanged; callers must invoke
   *   `commitRewardVotesResetForFinalization` after Rust commits the stage.
   */
  rustaxa::PbftFinalizationStorageWriteStage rewardVotesResetStageForFinalization(
      const rustaxa::PbftFinalizationStorageWritePlan& write_intent);

  /**
   * Applies live reward-vote metadata after a Rust-owned finalization batch has
   * committed the reward-vote reset stage.
   *
   * Inputs:
   * - `write_intent`: same Rust-planned intent used to build and commit the
   *   reward-vote reset stage.
   *
   * Invariants:
   * - Must be called only after Rust reports `Applied` or `AlreadyApplied` for
   *   the corresponding reward-vote reset stage.
   * - Clears stale extra-reward vote tracking to match the committed storage.
   *
   * Outputs:
   * - A Rust-verifiable post-mutation report proving the live metadata now
   *   matches the accepted finalization plan and stale extra votes were cleared.
   */
  rustaxa::PbftFinalizationLiveMutationReport commitRewardVotesResetForFinalization(
      const rustaxa::PbftFinalizationStorageWritePlan& write_intent);

 private:
  bool isValidRewardVoteForRust(const std::shared_ptr<PbftVote>& vote) const;
};

}  // namespace taraxa
