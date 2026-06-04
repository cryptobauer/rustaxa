#pragma once

#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

/**
 * Rust-mode VoteManager overlay.
 *
 * Purpose:
 * - Delegates legacy-compatible vote-manager behavior to `VoteManagerOld`
 *   through inheritance while routing PBFT vote persistence through
 *   Rust-owned storage bridge operations in shim-owned code.
 *
 * Inputs:
 * - Constructor arguments are inherited from `VoteManagerOld`.
 *
 * Outputs and invariants:
 * - Inherited methods use the same base state as the overridden reset path.
 * - Own verified votes, extra reward votes, and latest-round 2t+1 bundles use
 *   `rustaxa-storage` for durable writes in Rust mode.
 * - PBFT replay protection and `2t+1` threshold cache ownership live in the
 *   same Rust runtime as verified-vote admission; C++ supplies only
 *   FinalChain/PBFT-chain scalar facts when Rust reports a cache miss.
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
  /**
   * Constructs the Rust-mode VoteManager overlay.
   *
   * Inputs mirror the legacy `VoteManager` constructor. The overlay initializes
   * the inherited legacy state machine; PBFT vote validation/admission state is
   * owned by the Rust-backed `VerifiedVotes` facade.
   *
   * Invariants:
   * - Public C++ API remains identical during the rewrite.
   * - Unported methods keep explicit shim-local forwarding TODOs.
   */
  VoteManager(const FullNodeConfig& config, std::shared_ptr<DbStorage> db, std::shared_ptr<PbftChain> pbft_chain,
              std::shared_ptr<final_chain::FinalChain> final_chain, std::shared_ptr<KeyManager> key_manager,
              std::shared_ptr<SlashingManager> slashing_manager);

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

  /**
   * Validates and optionally materializes reward votes referenced by a PBFT block.
   *
   * Inputs:
   * - `pbft_block`: block whose reward-vote hash list should be checked against
   *   live certified votes for the current reward-vote metadata.
   * - `copy_votes`: when true, return the selected live `PbftVote` sidecars in
   *   the same order as the PBFT block's reward-vote hash list.
   *
   * Outputs:
   * - `{true, votes}` when Rust accepts the reward-vote references.
   * - `{false, {}}` when the preferred round and reverse period scan cannot
   *   satisfy the requested hashes.
   *
   * Invariants and edge behavior:
   * - Rust owns the deterministic preferred-round and reverse-round selection
   *   decision from compact membership facts.
   * - C++ only snapshots live verified-vote buckets and maps accepted hashes
   *   back to temporary sidecars when requested.
   * - Period-one blocks accept with no reward votes.
   */
  std::pair<bool, std::vector<std::shared_ptr<PbftVote>>> checkRewardVotes(const std::shared_ptr<PbftBlock>& pbft_block,
                                                                           bool copy_votes);
  /**
   * Returns reward votes selected from the shim's live verified-vote sidecar.
   *
   * Inputs:
   * - None; selection uses live reward metadata restored from Rust-backed PBFT
   *   vote storage during construction and updated after finalization reset.
   *
   * Outputs:
   * - Certified 2t+1 vote sidecars for the current reward block, or an empty
   *   vector if the sidecar facts are inconsistent.
   *
   * Invariants and edge behavior:
   * - Does not read or write durable storage.
   * - Asserts on impossible block-hash mismatches to preserve legacy behavior.
   */
  std::vector<std::shared_ptr<PbftVote>> getRewardVotes();
  /**
   * Returns the PBFT period associated with the current reward-vote sidecar.
   *
   * Inputs:
   * - None.
   *
   * Outputs:
   * - The period loaded from Rust-backed vote persistence or installed after a
   *   successful Rust finalization reward-vote reset.
   *
   * Invariants:
   * - The metadata is protected by `reward_votes_info_mutex_`.
   */
  PbftPeriod getRewardVotesPbftBlockPeriod();
  /**
   * Persists a locally generated verified PBFT vote through Rust storage.
   *
   * Inputs:
   * - `vote`: non-null weighted PBFT vote sidecar generated by the local node.
   *
   * Outputs:
   * - Writes `PbftVote::rlp(true, true)` into the Rust-backed
   *   `latest_round_own_votes` column and then records the live sidecar.
   *
   * Invariants and edge behavior:
   * - Durable Rust storage succeeds before the in-memory own-vote vector is
   *   mutated.
   * - Storage rejection propagates as an exception to the caller.
   */
  void saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote);
  /**
   * Returns live own verified PBFT vote sidecars for the current round.
   *
   * Outputs:
   * - The sidecars restored from Rust-backed storage at construction plus any
   *   successfully persisted local votes generated since startup.
   */
  std::vector<std::shared_ptr<PbftVote>> getOwnVerifiedVotes();
  /**
   * Appends own verified vote cleanup to the caller-owned Rust storage batch.
   *
   * Inputs:
   * - `write_batch`: PBFT manager batch that owns the eventual atomic commit.
   *
   * Outputs:
   * - Appends exact vote-hash deletes through the Rust storage batch registry
   *   and clears the live own-vote sidecar only after Rust accepts the appends.
   *
   * Invariants and edge behavior:
   * - No legacy C++ batch contents are created by this method in Rust mode.
   * - Unknown Rust batch ids or storage errors propagate as exceptions.
   */
  void clearOwnVerifiedVotes(Batch& write_batch);
  std::shared_ptr<PbftVote> generateVoteWithWeight(const blk_hash_t& blockhash, PbftVoteTypes vote_type,
                                                   PbftPeriod period, PbftRound round, PbftStep step,
                                                   const WalletConfig& wallet);
  std::shared_ptr<PbftVote> generateVote(const blk_hash_t& blockhash, PbftVoteTypes type, PbftPeriod period,
                                         PbftRound round, PbftStep step, const WalletConfig& wallet);
  std::pair<bool, std::string> validateVote(const std::shared_ptr<PbftVote>& vote, bool strict = true) const;
  /**
   * Returns the PBFT `2t+1` threshold for an eligibility period and vote type.
   *
   * Inputs:
   * - `pbft_period`: eligibility period used for the FinalChain total DPoS
   *   vote-count lookup.
   * - `vote_type`: proposal/soft/cert/next vote family selecting the Rust
   *   sortition threshold rule.
   *
   * Outputs:
   * - Threshold value when Rust has a cache hit or accepts the supplied
   *   FinalChain total-vote fact.
   * - Empty optional when FinalChain is behind, the vote type is invalid, or
   *   the threshold runtime rejects the supplied facts.
   *
   * Invariants and edge behavior:
   * - Rust owns cache lookup and update policy; C++ does not read or mutate the
   *   inherited legacy threshold cache in Rust mode.
   * - Only current PBFT-chain-size thresholds are cached, matching legacy
   *   VoteManager behavior.
   */
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
