#pragma once

#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include "common/util.hpp"
#include "common/vrf_wrapper.hpp"
#include "final_chain/final_chain.hpp"
#include "key_manager/key_manager.hpp"
#include "pbft/pbft_chain.hpp"
#include "pbft/pbft_service.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "vote/pbft_vote.hpp"
#include "vote_manager/verified_votes.hpp"

namespace taraxa {

class Network;
class PbftBlock;
class PbftService;
class SlashingManager;
struct SlashingDoubleVoteEvidence;

/**
 * Reward-vote-owned result after applying live metadata for a PBFT-finalization reset.
 *
 * Inputs are the Rust-planned reset identity after the finalization storage batch has committed. Outputs carry only
 * reward-vote facts: the accepted period, round, block hash, and storage-authenticated reset generation. PBFT manager
 * code passes these facts through the typed reward-reset bridge at the manager boundary.
 */
struct RewardVotesFinalizationResetReport {
  PbftPeriod period = 0;
  PbftRound round = 0;
  blk_hash_t block_hash;
  uint64_t reward_votes_reset_generation = 0;
};

/**
 * Rust-mode VoteManager overlay.
 *
 * Purpose:
 * - Owns the Rust-mode compatibility state directly while routing PBFT vote
 *   persistence through Rust-owned storage bridge operations.
 *
 * Inputs:
 * - Constructor arguments preserve the upstream public API.
 *
 * Outputs and invariants:
 * - All live compatibility state is shim-owned; no production behavior
 *   inherits from or delegates to the legacy implementation.
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
 * - Rust appender rejection returns a rejected result and leaves reward metadata unchanged.
 */
class VoteManager {
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
   * Synced cert-vote bundle validation result for PBFT sync admission.
   *
   * Purpose:
   * - Keeps PBFT manager from owning live cert-vote sidecar validation,
   *   verified-vote insertion, and Rust threshold fact construction.
   *
   * Outputs:
   * - `accepted` is Rust's terminal bundle decision after VoteManager executor
   *   validation and insertion effects have run.
   * - `status`, `first_bad_vote_hash`, `total_weight`, and `two_t_plus_one`
   *   mirror the Rust cert-vote bundle validator diagnostics.
   * - `validation_error` carries the temporary VoteManager executor rejection
   *   string when live vote validation fails before threshold planning.
   *
   * Invariants:
   * - VoteManager remains the only C++ owner of verified-vote mutation.
   * - PBFT manager receives diagnostics only and does not inspect accepted vote
   *   payloads for protocol decisions.
   */
  struct SyncedCertVoteValidationResult {
    bool accepted = false;
    uint8_t status = 0;
    vote_hash_t first_bad_vote_hash;
    uint64_t total_weight = 0;
    uint64_t two_t_plus_one = 0;
    std::string validation_error;
  };

  /**
   * Startup replay vote-validation result for PBFT finalization recovery.
   *
   * Purpose:
   * - Keeps PBFT manager from validating replayed reward-distribution votes one
   *   by one while replaying recently finalized period data.
   *
   * Outputs:
   * - `accepted == true` when all replayed votes were accepted by the
   *   VoteManager validation boundary.
   * - `first_bad_vote_hash` and `validation_error` identify the first rejected
   *   replay vote.
   *
   * Invariants:
   * - VoteManager owns the legacy validation compatibility call until replayed
   *   vote hydration is fully Rust-owned.
   * - The method does not mutate verified-vote collections; it only hydrates
   *   validation/weight facts required by reward distribution.
   */
  struct StartupReplayVoteValidationResult {
    bool accepted = false;
    vote_hash_t first_bad_vote_hash;
    std::string validation_error;
  };

  /**
   * Constructs the Rust-mode VoteManager overlay.
   *
   * Rust mode receives the application-owned PBFT service instead of extracting
   * an independent vote runtime from `DbStorage`. The overlay initializes
   * shim-owned compatibility state while the shared service owns PBFT vote
   * validation and admission state.
   *
   * Invariants:
   * - Public C++ API remains identical during the rewrite.
   * - Unported methods keep explicit shim-local forwarding TODOs.
   */
  VoteManager(const FullNodeConfig& config, SharedPbftService pbft_service, std::shared_ptr<PbftChain> pbft_chain,
              std::shared_ptr<final_chain::FinalChain> final_chain, std::shared_ptr<KeyManager> key_manager,
              std::shared_ptr<SlashingManager> slashing_manager);
  ~VoteManager() = default;
  VoteManager(const VoteManager&) = delete;
  VoteManager(VoteManager&&) = delete;
  VoteManager& operator=(const VoteManager&) = delete;
  VoteManager& operator=(VoteManager&&) = delete;

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
   * - Required progress-persistence failure throws before network, slashing, or PBFT-progress intents are exposed.
   * - Double-vote evidence is submitted inside `VoteManager`; callers use
   *   `report_slashing` only for peer-action policy.
   */
  PbftVoteAdmissionReport addVerifiedVoteWithReport(const std::shared_ptr<PbftVote>& vote);
  bool addVerifiedVote(const std::shared_ptr<PbftVote>& vote);
  /**
   * Admits and persists one locally generated PBFT vote.
   *
   * Purpose:
   * - Gives PBFT manager a local-signing executor boundary instead of making it
   *   orchestrate verified-vote admission and own-vote persistence separately.
   *
   * Inputs:
   * - `vote` is a signed local vote sidecar generated by this node.
   *
   * Outputs:
   * - Returns true only after Rust-backed admission accepts the vote and
   *   Rust-backed own-vote persistence commits the weighted payload.
   *
   * Edge behavior:
   * - Admission rejections return false.
   * - Own-vote storage failures propagate as exceptions, matching
   *   `saveOwnVerifiedVote`.
   *
   * Invariants:
   * - Network gossip remains outside VoteManager.
   * - PBFT manager must not call `saveOwnVerifiedVote` separately for votes
   *   accepted by this method.
   */
  bool addLocallyGeneratedVote(const std::shared_ptr<PbftVote>& vote);
  SyncedCertVoteValidationResult validateSyncedCertVoteBundle(PbftPeriod block_period, const blk_hash_t& block_hash,
                                                              const std::vector<std::shared_ptr<PbftVote>>& cert_votes);
  StartupReplayVoteValidationResult validateStartupReplayVotes(
      const std::vector<std::shared_ptr<PbftVote>>& replay_votes) const;
  bool voteInVerifiedMap(std::shared_ptr<PbftVote> const& vote) const;
  std::pair<bool, std::shared_ptr<PbftVote>> isUniqueVote(const std::shared_ptr<PbftVote>& vote) const;
  std::vector<std::shared_ptr<PbftVote>> getVerifiedVotes() const;
  uint64_t getVerifiedVotesSize() const;
  void cleanupVotesByPeriod(PbftPeriod pbft_period);
  std::vector<std::shared_ptr<PbftVote>> getProposalVotes(PbftPeriod period, PbftRound round) const;
  /**
   * Selects the filtering-step leader from VoteManager-owned proposal votes.
   *
   * Purpose:
   * - Keeps PBFT manager from pulling live proposal-vote sidecars only to build
   *   Rust leader-candidate facts.
   * - Reuses the Rust leader-candidate planner for status derivation,
   *   mark-valid commands, and deterministic ranking.
   *
   * Inputs:
   * - `period` and `round` identify the authoritative Rust-owned proposal snapshot.
   * - `validate_block` executes only the live block validation requests selected by Rust.
   *
   * Outputs:
   * - Returns the selected live proposed block and proposal vote when Rust
   *   selected an eligible leader.
   * - Returns empty when no proposal vote can be selected.
   *
   * Invariants:
   * - Proposal votes, chain membership, and proposed-block state are read atomically by the injected PBFT service.
   * - C++ reports identity-bound validation results against the prepared snapshot; Rust rejects stale snapshots and
   *   applies mark-valid state before returning owned selected vote/block payloads.
   */
  std::optional<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> identifyLeaderBlock(
      PbftPeriod period, PbftRound round,
      const std::function<bool(const std::shared_ptr<PbftBlock>&)>& validate_block) const;
  /**
   * Selects a leader from caller-supplied proposal votes.
   *
   * Purpose:
   * - Reuses the same Rust leader-candidate planner for locally generated
   *   proposal votes, so PBFT manager no longer owns a duplicate leader
   *   candidate fact builder.
   *
   * Inputs and outputs match the period/round overload, except tentative block/vote pairs are supplied by the caller
   * and normalized by a stateless Rust local-candidate API.
   *
   * Invariants:
   * - The caller remains responsible for local proposal vote generation and
   *   uniqueness checks.
   * - Tentative candidates never enter authoritative service or storage state.
   * - Rust still owns candidate lookup, status derivation, and deterministic leader ranking.
   */
  std::optional<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> identifyLeaderBlock(
      std::vector<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>>&& local_candidates,
      const std::function<bool(const blk_hash_t&)>& block_in_chain,
      const std::function<bool(const std::shared_ptr<PbftBlock>&)>& validate_block) const;
  /**
   * Rust-backed round-advance decision for PBFT manager runtime reports.
   *
   * Purpose:
   * - Keeps PBFT manager from calling legacy optional-return round selection
   *   APIs while the runtime session expects explicit `has_new_round` facts.
   *
   * Outputs:
   * - `has_new_round == true` when Rust verified-vote state found a valid
   *   next-round candidate.
   * - `new_round` carries that candidate round and is zero otherwise.
   *
   * Invariants:
   * - VoteManager/VerifiedVotes owns the `2t+1` next-vote lookup and
   *   preference rules.
   * - PBFT manager receives only runtime report facts and does not inspect
   *   supporting vote sidecars for this decision.
   */
  struct RoundAdvanceDecision {
    bool has_new_round = false;
    PbftRound new_round = 0;
  };
  RoundAdvanceDecision roundAdvanceDecision(PbftPeriod current_pbft_period, PbftRound current_pbft_round);
  std::optional<PbftRound> determineNewRound(PbftPeriod current_pbft_period, PbftRound current_pbft_round);

  /**
   * Compatibility entrypoint for existing direct callers.
   *
   * Inputs:
   * - Certified vote period, round, step, block hash, and caller-owned storage
   *   batch.
   *
   * Outputs:
   * - Appends reward-vote reset persistence through Rust and updates shim-owned
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
   * Validates reward votes for a live PBFT block without returning sidecars.
   *
   * Purpose:
   * - Lets PBFT manager report a compact valid/invalid fact to Rust block
   *   validation sessions without inspecting detailed reward-vote planner
   *   status.
   *
   * Outputs:
   * - Returns true when Rust accepts the reward-vote references.
   * - Returns false after `checkRewardVotesDetailed` logs the stable Rust
   *   rejection status and error code.
   *
   * Invariants:
   * - Does not mutate verified-vote state.
   * - Does not copy selected live vote sidecars.
   */
  bool validateRewardVotesForBlock(const std::shared_ptr<PbftBlock>& pbft_block);
  /**
   * Selects reward-vote payloads for a live PBFT block.
   *
   * Purpose:
   * - Gives PBFT manager only the executor payloads needed by reproposal and
   *   finalization compatibility after Rust reward-vote selection accepts.
   *
   * Outputs:
   * - Returns selected reward-vote sidecars on acceptance.
   * - Returns empty after `checkRewardVotesDetailed` logs the Rust rejection.
   *
   * Edge behavior:
   * - Payload order matches the PBFT block reward-vote hash list.
   */
  std::optional<std::vector<std::shared_ptr<PbftVote>>> collectRewardVotesForBlock(
      const std::shared_ptr<PbftBlock>& pbft_block);
  /**
   * Selects reward-vote payloads for compact queued PBFT block facts.
   *
   * Purpose:
   * - Keeps sync admission from materializing a PBFT block or reading detailed
   *   reward-vote reports in PBFT manager when queued metadata already supplies
   *   the block identity and reward-vote hash list.
   *
   * Outputs and edge behavior match the live-block overload.
   */
  std::optional<std::vector<std::shared_ptr<PbftVote>>> collectRewardVotesForBlock(
      PbftPeriod block_period, const blk_hash_t& block_hash, const blk_hash_t& prev_block_hash,
      const std::vector<vote_hash_t>& reward_vote_hashes);
  /**
   * Validated reward-vote payloads for local PBFT block proposal.
   *
   * Purpose:
   * - Keeps PBFT manager from inspecting reward-vote sidecars only to decide
   *   whether a proposal can include them and which hashes should enter the
   *   PBFT block constructor.
   *
   * Outputs:
   * - `valid == true` when the payload is acceptable for `propose_period`.
   * - `reward_votes` are temporary executor/public payloads preserved for the
   *   proposed-block return value and network egress.
   * - `reward_vote_hashes` are compact facts PBFT manager passes into
   *   `PbftBlock` construction.
   *
   * Edge behavior:
   * - Period 1 accepts an empty reward-vote payload.
   * - Later periods require at least one reward vote whose period is
   *   `propose_period - 1`, matching the legacy proposal precondition.
   */
  struct ProposalRewardVotes {
    bool valid = false;
    std::string validation_error;
    std::vector<std::shared_ptr<PbftVote>> reward_votes;
    std::vector<vote_hash_t> reward_vote_hashes;
  };
  ProposalRewardVotes proposalRewardVotesForPeriod(PbftPeriod propose_period);
  /**
   * Returns reward votes selected by the Rust-owned reward cursor.
   *
   * Inputs:
   * - None; selection reads the current Rust cursor and canonical retained payloads.
   *
   * Outputs:
   * - Transient certified-vote sidecars materialized from canonical Rust records.
   *
   * Invariants and edge behavior:
   * - Does not retain duplicate cursor or vote ownership in C++.
   * - Asserts on impossible cursor/payload mismatches to preserve legacy behavior.
   */
  std::vector<std::shared_ptr<PbftVote>> getRewardVotes();
  /**
   * Returns the period of the authoritative Rust-owned reward cursor.
   *
   * Inputs:
   * - None.
   *
   * Outputs:
   * - The current cursor period, or zero when no cursor is installed.
   *
   * Invariants:
   * - No C++ reward cursor field or lock is consulted.
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
   *   `latest_round_own_votes` column.
   *
   * Invariants and edge behavior:
   * - Storage rejection propagates as an exception to the caller.
   */
  void saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote);
  /**
   * Materializes own verified PBFT vote sidecars for the current round.
   *
   * Outputs:
   * - Fresh sidecars decoded from the canonical weighted vote payloads in
   *   Rust-backed storage.
   *
   * Invariants and edge behavior:
   * - VoteManager does not retain a duplicate own-vote sidecar collection.
   * - Invalid or mismatched durable payloads fail the read rather than being
   *   silently omitted.
   */
  std::vector<std::shared_ptr<PbftVote>> getOwnVerifiedVotes();
  /**
   * Clears all own verified votes through authoritative Rust storage.
   *
   * Inputs:
   * - `write_batch`: retained compatibility parameter; Rust mode does not append to it.
   *
   * Outputs:
   * - Clears every durable own vote through the storage-backed Rust runtime.
   *
   * Invariants and edge behavior:
   * - No legacy C++ batch contents are created by this method in Rust mode.
   * - The compatibility batch parameter remains unused in Rust mode.
   * - Storage errors propagate as exceptions.
   */
  void clearOwnVerifiedVotes(Batch& write_batch);
  std::shared_ptr<PbftVote> generateVoteWithWeight(const blk_hash_t& blockhash, PbftVoteTypes vote_type,
                                                   PbftPeriod period, PbftRound round, PbftStep step,
                                                   const WalletConfig& wallet);
  /**
   * Generates, admits, and persists one locally weighted PBFT vote.
   *
   * Purpose:
   * - Keeps PBFT manager from sequencing Rust vote generation, Rust-backed
   *   verified-vote admission, and own-vote persistence for normal local
   *   consensus voting.
   *
   * Outputs:
   * - `placed == true` with `vote` after the weighted vote is generated,
   *   admitted, and persisted as an own verified vote.
   * - `placed == false` with `error` when generation or admission rejects the
   *   local vote.
   *
   * Invariants:
   * - Network gossip and pillar-vote placement remain PBFT manager executor
   *   responsibilities.
   * - Storage failures from own-vote persistence propagate as exceptions.
   */
  struct LocallyGeneratedVotePlacement {
    bool placed = false;
    std::shared_ptr<PbftVote> vote;
    std::string error;
  };
  LocallyGeneratedVotePlacement generateAndPlaceLocalVote(const blk_hash_t& block_hash, PbftVoteTypes vote_type,
                                                          PbftPeriod period, PbftRound round, PbftStep step,
                                                          const WalletConfig& wallet);
  /**
   * Generates a locally signed proposal vote and verifies uniqueness.
   *
   * Purpose:
   * - Keeps PBFT manager from coordinating Rust vote generation with
   *   verified-vote uniqueness sidecar checks while it builds local proposal
   *   candidates.
   *
   * Inputs:
   * - `block_hash`, `period`, `round`, and `step` identify the proposal vote.
   * - `wallet` supplies the local signing and VRF material.
   *
   * Outputs:
   * - `generated == true` with `vote` when Rust generation succeeds and the
   *   vote is unique for its voter/period/round/step.
   * - `generated == false` with `error` when generation returns no vote or the
   *   generated vote conflicts with existing verified-vote state.
   *
   * Invariants:
   * - Does not mutate verified-vote state.
   * - The returned vote still needs local admission through
   *   `addLocallyGeneratedVote` after PBFT manager selects a proposal leader.
   */
  struct LocalProposalVoteGeneration {
    bool generated = false;
    std::shared_ptr<PbftVote> vote;
    std::string error;
  };
  LocalProposalVoteGeneration generateUniqueProposalVoteForBlock(const blk_hash_t& block_hash, PbftPeriod period,
                                                                 PbftRound round, PbftStep step,
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
   *   removed legacy threshold cache in Rust mode.
   * - Only current PBFT-chain-size thresholds are cached, matching legacy
   *   VoteManager behavior.
   */
  std::optional<uint64_t> getPbftTwoTPlusOne(PbftPeriod pbft_period, PbftVoteTypes vote_type) const;
  bool voteAlreadyValidated(const vote_hash_t& vote_hash) const;
  bool genAndValidateVrfSortition(PbftPeriod pbft_period, PbftRound pbft_round, const WalletConfig& wallet) const;
  /**
   * Builds local proposal-wallet facts for the Rust PBFT proposal planner.
   *
   * Purpose:
   * - Keeps PBFT manager from looping over wallet sidecars to run proposer
   *   sortition and construct Rust proposal facts.
   *
   * Inputs:
   * - `wallets` is the current period wallet eligibility snapshot.
   *
   * Outputs:
   * - `local_wallets` preserves the wallet order used by Rust-selected wallet
   *   indices.
   * - `wallet_facts` carries DPoS eligibility and Rust-backed proposer
   *   sortition acceptance for each wallet.
   *
   * Invariants:
   * - Does not mutate vote state.
   * - Wallet indices in `wallet_facts` match positions in `local_wallets`.
   */
  struct ProposalWalletFacts {
    std::vector<WalletConfig> local_wallets;
    rust::Vec<rustaxa::PbftManagerProposalWalletFact> wallet_facts;
  };
  ProposalWalletFacts proposalWalletFacts(PbftPeriod pbft_period, PbftRound pbft_round,
                                          const std::vector<std::pair<bool, WalletConfig>>& wallets) const;
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
                                            bool needs_previous_round_next_value, bool needs_current_round_soft) const;

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
   * Applies PBFT manager startup period/round synchronization and returns the
   * previous-round facts used for legacy-compatible initialization logging.
   *
   * Purpose:
   * - Keeps PBFT manager from sequencing VoteManager startup state mutation and
   *   verified-vote fact lookup as two separate sidecar-facing operations.
   *
   * Inputs:
   * - `period` and `round` are restored from the Rust PBFT manager runtime
   *   snapshot during startup.
   *
   * Outputs:
   * - Previous-round next-vote facts for logging only.
   *
   * Invariants:
   * - Updates VoteManager's current PBFT period/round before reading
   *   previous-round facts.
   * - Does not persist votes or perform network/slashing effects.
   */
  PreviousRoundNextVoteLogFacts applyStartupPeriodRoundAndLogFacts(PbftPeriod period, PbftRound round);

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
   * - Reward-vote payloads for period-level rebroadcast.
   * - Own verified PBFT vote payloads for one-by-one gossip.
   * - Current-round soft-vote payloads.
   * - Previous-round next-vote and next-null-vote payloads when `round > 1`.
   *
   * Invariants:
   * - Returned `PbftVote` objects are executor payloads for gossip only, not
   *   protocol-decision inputs.
   * - Empty vectors represent absent egress payloads.
   */
  struct StuckRoundVoteBroadcastPayloads {
    std::vector<std::shared_ptr<PbftVote>> reward_votes;
    std::vector<std::shared_ptr<PbftVote>> own_votes;
    std::vector<std::shared_ptr<PbftVote>> soft_votes;
    std::vector<std::shared_ptr<PbftVote>> previous_round_next_votes;
    std::vector<std::shared_ptr<PbftVote>> previous_round_next_null_votes;
  };
  StuckRoundVoteBroadcastPayloads stuckRoundVoteBroadcastPayloads(PbftPeriod period, PbftRound round);
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
  /**
   * Builds certify-step soft-vote debug output from VoteManager-owned vote facts.
   *
   * Purpose:
   * - Keeps PBFT manager from iterating live soft-vote sidecar buckets and
   *   reading thresholds only to produce diagnostic output.
   *
   * Outputs:
   * - Legacy-compatible debug text describing per-block soft-vote weights,
   *   voters, total weight, and the soft-vote `2t+1` threshold.
   *
   * Invariants:
   * - Does not mutate verified-vote state.
   * - The returned string is for logging only and must not be used as a
   *   protocol decision input.
   */
  std::string softVoteDebugMessage(PbftPeriod period, PbftRound round) const;
  StepVotes getStepVotes(PbftPeriod period, PbftRound round, PbftStep step) const;
  /**
   * Applies a Rust-planned PBFT manager period/round lifecycle update.
   *
   * Purpose:
   * - Keeps PBFT manager transition executors from calling the generic
   *   VoteManager period/round mutator directly.
   *
   * Inputs:
   * - `pbft_period` and `pbft_round` are selected by the Rust PBFT manager
   *   transition or advance-period plan.
   *
   * Invariants:
   * - Performs the same live verified-vote threshold hydration as the legacy
   *   period/round setter.
   * - Does not persist votes or perform network/slashing effects.
   */
  void applyRustPlannedPeriodRound(PbftPeriod pbft_period, PbftRound pbft_round);

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
   * - The certified-vote bundle and finalized reward cursor are selected and
   *   durably committed by the Rust-owned verified-vote runtime.
   * - Live Rust cursor state is published only after the atomic reset commit
   *   returns `Applied` or `AlreadyApplied`.
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
   * - A bridge stage containing the certified-vote bundle selected by Rust.
   *
   * Invariants:
   * - Rust validates the requested identity against authoritative runtime and
   *   storage state before returning the stage.
   * - Live reward metadata is unchanged; callers must invoke
   *   `commitRewardVotesResetForFinalization` after Rust commits the stage.
   */
  rustaxa::PbftFinalizationStorageWriteStage rewardVotesResetStageForFinalization(
      const rustaxa::PbftFinalizationStorageWritePlan& write_intent);

  /**
   * Builds the compact identity-only Rust reward-vote reset request used by
   * direct compatibility callers outside a broader finalization batch. Rust
   * prepares the certified bundle and enumerates extra-reward keys internally.
   */
  rustaxa::PbftRewardVotesResetRequest rewardVotesResetRequestForFinalization(
      const rustaxa::PbftFinalizationStorageWritePlan& write_intent);

  /**
   * Applies live reward-vote metadata after a Rust-owned finalization batch has
   * committed the reward-vote reset stage.
   *
   * Inputs:
   * - `write_intent`: same Rust-planned intent used to build and commit the
   *   reward-vote reset stage.
   * - `reward_votes_reset_generation`: non-zero proof returned by the Rust
   *   storage apply that committed the reset.
   *
   * Invariants:
   * - Must be called only after Rust reports `Applied` or `AlreadyApplied` for
   *   the corresponding reward-vote reset stage.
   * - The manager validates the generation before accepting the executor report.
   *
   * Outputs:
   * - Reward-vote-owned cursor facts proving live metadata matches the accepted finalization plan.
   */
  RewardVotesFinalizationResetReport commitRewardVotesResetForFinalization(
      const rustaxa::PbftFinalizationStorageWritePlan& write_intent, uint64_t reward_votes_reset_generation);

 private:
  /** Selects from isolated tentative wallet candidates without mutating authoritative service state. */
  std::optional<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>>
  identifyLeaderBlockFromLocalCandidates(
      std::vector<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>>&& local_candidates,
      const std::function<bool(const blk_hash_t&)>& block_in_chain,
      const std::function<bool(const std::shared_ptr<PbftBlock>&)>& validate_block) const;

  /**
   * Executes a Rust-planned double-vote slashing proof submission.
   *
   * Purpose:
   * - Isolates the remaining C++ `SlashingManager` executor boundary from the
   *   Rust-owned PBFT vote admission state machine.
   *
   * Inputs:
   * - `evidence` is the Rust-normalized double-vote payload selected by
   *   verified-vote admission.
   *
   * Outputs:
   * - Returns the SlashingManager submission result.
   *
   * Invariants:
   * - Does not mutate verified-vote state.
   * - This is a temporary executor boundary until slashing proof submission has
   *   a Rust-owned port.
   */
  bool submitRustPlannedSlashingProof(const SlashingDoubleVoteEvidence& evidence);
  const PbftConfig& kPbftConfig;
  std::shared_ptr<PbftChain> pbft_chain_;
  std::shared_ptr<final_chain::FinalChain> final_chain_;
  std::shared_ptr<KeyManager> key_manager_;
  std::weak_ptr<Network> network_;
  std::shared_ptr<SlashingManager> slashing_manager_;
  SharedPbftService pbft_service_;

  std::atomic<PbftPeriod> current_pbft_period_{0};
  std::atomic<PbftRound> current_pbft_round_{0};
  VerifiedVotes verified_votes_;

  LOG_OBJECTS_DEFINE
};

}  // namespace taraxa
