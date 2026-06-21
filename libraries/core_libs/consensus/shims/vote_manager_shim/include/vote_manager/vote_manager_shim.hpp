#pragma once

#include <string>

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
   * Network-facing executor report for Rust PBFT vote admission.
   *
   * Purpose:
   * - Carries the Rust-planned ingress/network effects that are safe for the
   *   temporary packet-handler executor to apply after `VoteManager` has
   *   validated, inserted, and persisted the vote through Rust-owned runtime
   *   state.
   *
   * Invariants:
   * - `accepted` means the vote was admitted and any VoteManager-owned storage
   *   or slashing side effects have already been applied.
   * - Hash fields are populated only when their matching boolean is true.
   * - Slashing submission remains owned by `VoteManager` in this slice; the
   *   `report_slashing` flag lets packet handlers apply peer-action policy.
   */
  struct PbftVoteAdmissionReport {
    bool accepted = false;
    bool mark_vote_known = false;
    vote_hash_t mark_vote_known_hash;
    bool gossip_vote = false;
    vote_hash_t gossip_vote_hash;
    bool report_slashing = false;
    bool drive_pbft_progress = false;
    PbftPeriod progress_period = 0;
    PbftRound progress_round = 0;
  };

  /**
   * Detailed Rust-planned reward-vote validation result.
   *
   * Purpose:
   * - Preserves Rust's reward-vote selection status at PBFT manager call sites
   *   instead of collapsing deterministic selection failures into an opaque
   *   boolean.
   *
   * Inputs:
   * - Produced by `checkRewardVotesDetailed` from compact PBFT block facts and
   *   live verified-vote membership snapshots.
   *
   * Outputs and invariants:
   * - `accepted` is Rust's terminal reward-vote selection decision.
   * - `status` and `error_code` are stable Rust bridge diagnostics.
   * - `votes` contains temporary C++ `PbftVote` sidecars only when requested
   *   by `copy_votes` and Rust accepted the selected hashes.
   * - C++ does not decide preferred-round or reverse-round fallback policy.
   */
  struct RewardVoteValidationResult {
    bool accepted = false;
    uint8_t status = 0;
    std::string error_code;
    PbftPeriod selected_period = 0;
    PbftRound selected_round = 0;
    blk_hash_t selected_block_hash;
    vote_hash_t missing_vote_hash;
    std::vector<std::shared_ptr<PbftVote>> votes;
  };

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
  /**
   * Validates and admits a PBFT vote through the Rust-owned admission runtime.
   *
   * Inputs:
   * - `vote` is the temporary live C++ sidecar whose canonical RLP is inspected
   *   and validated by Rust before any verified-vote runtime mutation.
   *
   * Outputs:
   * - Returns the terminal Rust admission outcome plus packet-handler effects
   *   that C++ may execute after admission.
   *
   * Edge behavior:
   * - Duplicate, invalid, or non-admitted votes return `accepted == false`.
   * - Double-vote evidence is submitted inside `VoteManager`; callers use
   *   `report_slashing` only for peer-action policy.
   */
  PbftVoteAdmissionReport addVerifiedVoteWithReport(const std::shared_ptr<PbftVote>& vote);
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
  RewardVoteValidationResult checkRewardVotesDetailed(const std::shared_ptr<PbftBlock>& pbft_block, bool copy_votes);

  /**
   * Validates and optionally materializes reward votes referenced by compact PBFT block facts.
   *
   * Inputs:
   * - `block_period`, `block_hash`, and `prev_block_hash`: PBFT block facts supplied by the caller, typically from
   *   Rust-owned sync queue metadata.
   * - `reward_vote_hashes`: reward-vote hashes referenced by the PBFT block in block order.
   * - `copy_votes`: when true, return selected live `PbftVote` sidecars in the same order as `reward_vote_hashes`.
   *
   * Outputs and invariants match `checkRewardVotes(pbft_block, copy_votes)`, but callers that already have compact
   * block facts do not need to materialize or reopen a live `PbftBlock` sidecar.
   */
  std::pair<bool, std::vector<std::shared_ptr<PbftVote>>> checkRewardVotes(
      PbftPeriod block_period, const blk_hash_t& block_hash, const blk_hash_t& prev_block_hash,
      const std::vector<vote_hash_t>& reward_vote_hashes, bool copy_votes);
  RewardVoteValidationResult checkRewardVotesDetailed(PbftPeriod block_period, const blk_hash_t& block_hash,
                                                      const blk_hash_t& prev_block_hash,
                                                      const std::vector<vote_hash_t>& reward_vote_hashes,
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
  /**
   * Clears only the Rust-mode live own-vote sidecar after Rust storage has
   * already committed the matching `latest_round_own_votes` deletes.
   *
   * Inputs:
   * - None. The durable delete set is supplied to the PBFT manager Rust
   *   transition apply path before this method is called.
   *
   * Outputs:
   * - Empties the in-memory own-vote vector.
   *
   * Invariants and edge behavior:
   * - This method must only be called after the Rust storage transition apply
   *   reports success. It intentionally performs no storage writes.
   */
  void clearOwnVerifiedVotesAfterRustPersistence();
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
  /**
   * Compact verified-vote facts consumed by PBFT manager state-action planning.
   *
   * Purpose:
   * - Keeps PBFT manager from directly querying `2t+1` voted-block sidecar
   *   families while preserving the existing Rust state-action fact shape.
   *
   * Outputs:
   * - Previous-round next-null and next-value status.
   * - Current-round soft-value status.
   *
   * Invariants:
   * - VoteManager/VerifiedVotes owns the Rust-backed `2t+1` lookup details.
   * - PBFT manager receives only compact booleans and hashes for planner input.
   */
  struct StateActionVoteFacts {
    bool has_previous_round_next_null = false;
    bool has_previous_round_next_value = false;
    blk_hash_t previous_round_next_value_hash;
    bool has_current_round_soft_value = false;
    blk_hash_t current_round_soft_value_hash;
  };
  StateActionVoteFacts stateActionVoteFacts(PbftPeriod period, PbftRound round, bool needs_previous_round_next_null,
                                            bool needs_previous_round_next_value,
                                            bool needs_current_round_soft) const;

  /**
   * Previous-round next-vote facts for PBFT manager transition logging.
   *
   * Purpose:
   * - Keeps PBFT manager from querying individual verified-vote sidecar families
   *   when it only needs compact facts for legacy-compatible logging.
   *
   * Outputs:
   * - Optional previous-round next-voted block hash.
   * - Boolean previous-round next-voted null status.
   *
   * Invariants:
   * - Does not materialize vote payloads or perform network/slashing side effects.
   * - Absent facts are represented as empty optional/false.
   */
  struct PreviousRoundNextVoteLogFacts {
    std::optional<blk_hash_t> next_voted_block;
    bool next_voted_null_block = false;
  };
  PreviousRoundNextVoteLogFacts previousRoundNextVoteLogFacts(PbftPeriod period, PbftRound previous_round) const;

  /**
   * Current-round cert-voted block selection for PBFT manager execution.
   *
   * Purpose:
   * - Keeps PBFT manager from deriving the certified block hash by peeking into
   *   the first materialized `PbftVote` sidecar.
   *
   * Outputs:
   * - `found == true` when Rust-backed verified-vote state has both the
   *   current-round cert-voted block hash and retained cert-vote payloads.
   * - `block_hash` is the Rust-selected certified block identity.
   * - `votes` are temporary C++ payloads for the PBFT-chain push executor.
   *
   * Invariants:
   * - An empty payload vector with a found block is treated as missing
   *   executor materialization and returns `found == false`.
   */
  struct CertVotedBlockSelection {
    bool found = false;
    blk_hash_t block_hash;
    std::vector<std::shared_ptr<PbftVote>> votes;
  };
  CertVotedBlockSelection certVotedBlockSelection(PbftPeriod period, PbftRound round) const;
  /**
   * Stuck-round vote payload groups for PBFT manager network rebroadcast.
   *
   * Purpose:
   * - Keeps PBFT manager from selecting individual verified-vote sidecar families
   *   while preserving network egress as a temporary C++ boundary.
   *
   * Outputs:
   * - Current-round soft-vote payloads.
   * - Previous-round next-vote and next-null-vote payloads when `round > 1`.
   *
   * Invariants:
   * - Returned `PbftVote` objects are executor payloads for gossip only, not
   *   protocol-decision inputs.
   * - Empty vectors represent absent egress payloads.
   */
  struct StuckRoundVoteBroadcastPayloads {
    std::vector<std::shared_ptr<PbftVote>> soft_votes;
    std::vector<std::shared_ptr<PbftVote>> previous_round_next_votes;
    std::vector<std::shared_ptr<PbftVote>> previous_round_next_null_votes;
  };
  StuckRoundVoteBroadcastPayloads stuckRoundVoteBroadcastPayloads(PbftPeriod period, PbftRound round) const;
  std::optional<blk_hash_t> getTwoTPlusOneVotedBlock(PbftPeriod period, PbftRound round,
                                                     TwoTPlusOneVotedBlockType type) const;
  std::vector<std::shared_ptr<PbftVote>> getTwoTPlusOneVotedBlockVotes(PbftPeriod period, PbftRound round,
                                                                       TwoTPlusOneVotedBlockType type) const;
  /**
   * Plans previous-round next-vote bundle egress from Rust-owned vote payload metadata.
   *
   * Inputs:
   * - `period` and `round`: PBFT round requested by get-next-votes sync.
   *
   * Outputs:
   * - Ordered next and next-null vote-hash plans without materializing
   *   `PbftVote` objects.
   *
   * Invariants:
   * - Network code must still filter per-peer known votes and build/send
   *   chunked tarcap packets at the boundary.
   */
  rustaxa::PbftNextVotesBundleEgressPlan planNextVotesBundleEgress(PbftPeriod period, PbftRound round) const;
  /**
   * Builds one optimized PBFT votes-bundle payload from a peer-filtered Rust egress request.
   *
   * Outputs:
   * - On status 0, returns inner optimized votes-bundle RLP and included hashes
   *   in send order; non-zero statuses must not be sent.
   */
  rustaxa::PbftOptimizedVoteBundleBuildResult buildOptimizedVotesBundleEgress(
      rustaxa::PbftOptimizedVoteBundleBuildRequest request) const;
  StepVotes getStepVotes(PbftPeriod period, PbftRound round, PbftStep step) const;
  void setCurrentPbftPeriodAndRound(PbftPeriod pbft_period, PbftRound pbft_round);
  PbftStep getNetworkTplusOneNextVotingStep(PbftPeriod period, PbftRound round) const;

  /**
   * Finalization-aware reward-vote reset handoff.
   *
   * Inputs:
   * - `write_intent`: Rust-planned PBFT finalization storage intent carrying
   *   reward vote period, round, step, and block hash facts.
   * - `batch`: legacy API parameter retained for upstream signature
   *   compatibility. Rust mode commits this isolated compatibility reset through
   *   a Rust-owned batch instead of appending to the caller batch.
   *
   * Outputs:
   * - Rust-owned apply status for the reward-vote reset stage.
   *
   * Invariants:
   * - The certified-vote bundle is selected from inherited live
   *   `VerifiedVotes` state.
   * - Inherited reward metadata and stale extra-reward tracking are mutated only
   *   after Rust commits and returns `Applied` or `AlreadyApplied`.
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
