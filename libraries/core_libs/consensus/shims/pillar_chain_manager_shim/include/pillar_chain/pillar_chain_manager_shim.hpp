#pragma once

#include <cstdint>
#include <memory>
#include <optional>
#include <shared_mutex>
#include <vector>

#include "common/event.hpp"
#include "common/types.hpp"
#include "final_chain/data.hpp"
#include "logger/logger.hpp"
#include "pillar_chain/pillar_block.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace rustaxa {
struct BridgePillarChainStorage;
struct BridgePillarChainRuntime;
}

namespace taraxa {
class DbStorage;
class Network;
class KeyManager;
struct FicusHardforkConfig;
namespace final_chain {
class FinalChain;
}

namespace pillar_chain {

/**
 * Deterministic pillar-vote relevance plan status from the Rust planner.
 *
 * Purpose:
 * - Carries an explicit reason for the Rust-enabled
 *   `PillarChainManager::isRelevantPillarVote` compatibility path.
 *
 * Invariants:
 * - Values `0..4` intentionally match the stable Rust bridge status codes.
 * - `kUnknown` means the shim could not obtain a deterministic Rust decision;
 *   Rust mode treats that as non-relevant rather than falling back to C++.
 */
enum class PillarVoteRelevancePlanStatus : uint8_t {
  kRelevant = 0,
  kVoteAlreadyKnown = 1,
  kMissingCurrentPillarBlock = 2,
  kVotePeriodMismatch = 3,
  kVoteBlockHashMismatch = 4,
  kUnknown = 255,
};

/**
 * Result returned by one relevance-planning pass.
 *
 * Inputs/outputs:
 * - `status` identifies the accepted/rejected reason.
 * - `is_relevant` is true only when the Rust planner accepts the vote for the
 *   current local pillar-chain context.
 */
struct PillarVoteRelevancePlan {
  PillarVoteRelevancePlanStatus status{PillarVoteRelevancePlanStatus::kUnknown};
  bool is_relevant{false};
};

/**
 * Evaluates one pillar vote against current pillar-block context.
 *
 * This helper keeps all Rust bridge calls in a dedicated shim-owned file while
 * preserving the public `PillarChainManager` API in upstream code.
 *
 * Edge behavior:
 * - Bridge exceptions and invalid Rust input are mapped to `kUnknown` and
 *   `is_relevant == false`.
 */
PillarVoteRelevancePlan planPillarVoteRelevance(const FicusHardforkConfig& ficus_hf_config,
                                                const std::shared_ptr<PillarVote>& vote,
                                                const std::shared_ptr<PillarBlock>& current_pillar_block,
                                                bool vote_already_known);

/**
 * Stable logging helper for explicit reason reporting.
 */
const char* pillarVoteRelevancePlanStatusString(PillarVoteRelevancePlanStatus status);

/**
 * Deterministic pillar-vote validation plan status from Rust inspection and
 * local final-chain context.
 *
 * Purpose:
 * - Centralizes validation outcomes used by Rust-mode `validatePillarVote`.
 *
 * Invariants:
 * - `kValid` requires both bridge-inspected signature validity and current
 *   deterministic relevance/uniqueness/eligibility checks.
 */
enum class PillarVoteValidationPlanStatus : uint8_t {
  kValid = 0,
  kDuplicate = 1,
  kMissingCurrentPillarBlock = 2,
  kVotePeriodMismatch = 3,
  kVoteBlockHashMismatch = 4,
  kNotUnique = 5,
  kSignatureInvalid = 6,
  kNotEligible = 7,
  kFuturePeriod = 8,
  kInspectionFailure = 9,
  kUnknown = 255,
};

/**
 * One validation outcome for a single pillar vote.
 *
 * Inputs/outputs:
 * - `status` identifies the first deterministic reason for failure.
 * - `is_valid` is true only when all checks pass.
 * - `period` and `vote_hash` are filled from Rust inspection when available.
 * - `recovered_voter` is filled from Rust inspection when available.
 */
struct PillarVoteValidationPlan {
  PillarVoteValidationPlanStatus status{PillarVoteValidationPlanStatus::kUnknown};
  bool is_valid{false};
  PbftPeriod period{0};
  vote_hash_t vote_hash{};
  addr_t recovered_voter{};
};

/**
 * Validates one pillar vote in Rust mode with local relevance, identity, and DPoS checks.
 *
 * Purpose:
 * - Owns the Rust-enabled validation order so callers cannot accidentally pass
 *   uniqueness results computed from C++ sidecar voter recovery.
 *
 * Inputs/outputs:
 * - `pillar_votes` supplies duplicate and Rust identity uniqueness checks.
 * - Returned `period`, `vote_hash`, and `recovered_voter` are populated from
 *   Rust inspection when the vote reaches identity checks.
 *
 * Invariants:
 * - Duplicate/relevance checks run before Rust signature inspection.
 * - Signature inspection runs before identity uniqueness.
 * - Identity uniqueness must call `isUniqueVoteIdentity(period, vote_hash,
 *   recovered_voter)`, not `isUniqueVote(vote)`.
 * - The helper must not call `PillarVote::getVoterAddr()` or `verifyVote()`.
 */
PillarVoteValidationPlan validatePillarVoteWithRust(const FicusHardforkConfig& ficus_hf_config,
                                                    const std::shared_ptr<PillarVote>& vote,
                                                    const std::shared_ptr<final_chain::FinalChain>& final_chain,
                                                    const std::shared_ptr<PillarBlock>& current_pillar_block,
                                                    const ::rust::Box<rustaxa::BridgePillarChainRuntime>& runtime);

/**
 * Prepared insertion facts for one Rust-inspected pillar vote.
 *
 * Purpose:
 * - Carries the Rust-recovered identity and DPoS weight that
 *   `PillarChainManager::addVerifiedPillarVote` needs to initialize threshold
 *   state and insert into the Rust-backed `PillarVotes` index.
 *
 * Invariants:
 * - `can_insert` is true only after Rust RLP/signature inspection succeeds and
 *   FinalChain returns a non-zero DPoS vote count for `recovered_voter`.
 * - Callers must not fall back to C++ voter recovery when `can_insert` is false
 *   in Rust-enabled mode.
 */
struct AddVerifiedPillarVoteWithRustPlan {
  PillarVoteValidationPlanStatus status{PillarVoteValidationPlanStatus::kUnknown};
  bool can_insert{false};
  bool needs_threshold{false};
  PbftPeriod period{0};
  blk_hash_t block_hash{};
  vote_hash_t vote_hash{};
  addr_t recovered_voter{};
  uint64_t validator_vote_count{0};
};

/**
 * Inspects and weights one vote for Rust-mode `addVerifiedPillarVote`.
 *
 * Inputs/outputs:
 * - `vote` supplies canonical RLP bytes and live sidecar hash/block data.
 * - `final_chain` supplies DPoS vote counts at `period - 1`.
 * - Returns a plan with explicit failure status instead of falling back to C++.
 *
 * Edge behavior:
 * - Null inputs, malformed RLP, invalid signatures, future DPoS state, and
 *   zero-weight validators all return `can_insert == false`.
 */
AddVerifiedPillarVoteWithRustPlan planAddVerifiedPillarVoteWithRust(
    const std::shared_ptr<PillarVote>& vote, const std::shared_ptr<final_chain::FinalChain>& final_chain,
    const ::rust::Box<rustaxa::BridgePillarChainRuntime>& runtime);

/**
 * Inspects one vote RLP in Rust and returns decoded identity plus signature status.
 *
 * The helper must not call `PillarVote::getVoterAddr()` or `verifyVote()`.
 * Rust-enabled validation uses the recovered identity for uniqueness and DPoS
 * checks so cryptographic recovery is not silently delegated back to C++.
 */
PillarVoteValidationPlan inspectPillarVoteWithRust(const std::shared_ptr<PillarVote>& vote);

/**
 * Stable logging helper for explicit validation reason reporting.
 */
const char* pillarVoteValidationPlanStatusString(PillarVoteValidationPlanStatus status);

/**
 * Rust-mode deterministic status for one synced pillar-vote bundle.
 *
 * Purpose:
 * - Mirrors stable Rust planner status codes at the pillar-chain shim boundary.
 *
 * Invariants:
 * - Values `0..8` intentionally match Rust bridge status codes.
 * - `kUnknown` is reserved for shim/bridge failures before Rust returns a
 *   deterministic status.
 */
enum class ValidateSyncPillarVotesBundlePlanStatus : uint8_t {
  kBundleValid = 0,
  kBundleEmpty = 1,
  kVotePeriodMismatch = 2,
  kVoteBlockHashMismatch = 3,
  kPrevalidationFailed = 4,
  kZeroWeight = 5,
  kVoterConflict = 6,
  kThresholdNotReached = 7,
  kWeightOverflow = 8,
  kUnknown = 255,
};

/**
 * Deterministic Rust bundle-apply result for synced pillar votes.
 *
 * Purpose:
 * - Carries Rust's validation status, aggregate weights, and insertion status
 *   after Rust applies selected votes to the Rust-backed `PillarVotes` index.
 *
 * Edge behavior:
 * - `valid` is true only when Rust returned `kBundleValid` and all selected
 *   votes were inserted or already present.
 */
struct ValidateSyncPillarVotesBundleDeterministicallyResult {
  ValidateSyncPillarVotesBundlePlanStatus plan_status{ValidateSyncPillarVotesBundlePlanStatus::kUnknown};
  vote_hash_t first_bad_vote_hash{};
  uint64_t block_weight{0};
  uint64_t selected_weight{0};
  bool insert_failed{false};
  vote_hash_t insert_failed_vote_hash{};
  bool valid{false};
};

/**
 * PBFT sync pillar-vote validation result status.
 *
 * Purpose:
 * - Makes Rust-mode synced pillar-vote failures observable while keeping PBFT
 *   manager on a typed pillar-chain port instead of PBFT-local helper logic.
 *
 * Invariants:
 * - `kValid` is the only accepting status.
 * - `kPlanRejected` means Rust returned a deterministic rejection status.
 * - `kBridgeError` means the shim could not obtain a deterministic Rust plan.
 */
enum class ValidatePbftBlockPillarVotesWithRustStatus : uint8_t {
  kUnknown = 0,
  kValid,
  kMissingPillarChainManager,
  kMissingPbftBlock,
  kMissingPillarVotes,
  kMissingCurrentPillarBlock,
  kPillarBlockPeriodMismatch,
  kMissingThreshold,
  kBridgeError,
  kPlanRejected,
  kAcceptedVoteMissing,
  kInsertFailed,
};

/**
 * Explicit result for the Rust-mode PBFT synced pillar-vote path.
 *
 * Inputs/outputs:
 * - `status` describes the pillar-chain shim-level decision.
 * - `plan_status` preserves Rust's stable planner code for deterministic
 *   bundle rejections.
 * - `block_weight` and `selected_weight` are Rust-planned aggregate weights
 *   populated for accepted plans.
 */
struct ValidatePbftBlockPillarVotesWithRustResult {
  ValidatePbftBlockPillarVotesWithRustStatus status{ValidatePbftBlockPillarVotesWithRustStatus::kUnknown};
  uint8_t plan_status{0};
  vote_hash_t first_bad_vote_hash{};
  uint64_t block_weight{0};
  uint64_t selected_weight{0};

  [[nodiscard]] bool valid() const { return status == ValidatePbftBlockPillarVotesWithRustStatus::kValid; }
};

/**
 * Returns a stable string for the pillar-chain shim-level synced validation status.
 */
const char* validatePbftBlockPillarVotesWithRustStatusString(ValidatePbftBlockPillarVotesWithRustStatus status);

/** @addtogroup PILLAR_CHAIN
 * @{
 */

/**
 * Rust-mode PillarChainManager facade.
 *
 * Purpose:
 * - Preserves the public C++ `PillarChainManager` API while keeping all
 *   Rust-enabled routing in shim-owned files.
 * - Owns the pillar-vote Rust identity/relevance/validation insertion paths so
 *   upstream-owned `pillar_chain_manager.cpp` remains pure legacy C++.
 *
 * Invariants:
 * - This class is the Rust-mode production surface; it must not silently
 *   delegate deterministic vote validation or insertion to `PillarChainManagerOld`.
 * - Existing C++ storage, networking, and block lifecycle calls remain stable
 *   while deterministic vote identity logic is moved behind Rust bridge helpers.
 */
class PillarChainManager {
 private:
  const util::event::EventEmitter<const PillarBlockData&> pillar_block_finalized_emitter_{};

 public:
  const decltype(pillar_block_finalized_emitter_)::Subscriber& pillar_block_finalized_ =
      pillar_block_finalized_emitter_;

 public:
  /**
   * Constructs the Rust-mode pillar-chain manager and loads persisted local
   * pillar block/vote state.
   *
   * Inputs:
   * - `ficus_hf_config` supplies pillar period configuration.
   * - `db` supplies persisted pillar blocks and votes.
   * - `final_chain` supplies DPoS vote counts and eligibility.
   * - `key_manager` and `node_addr` preserve the legacy construction contract.
   *
   * Edge behavior:
   * - Persisted votes are reinserted through the Rust-mode verified-vote path.
   */
  PillarChainManager(const FicusHardforkConfig& ficus_hf_config, std::shared_ptr<DbStorage> db,
                     std::shared_ptr<final_chain::FinalChain> final_chain, std::shared_ptr<KeyManager> key_manager,
                     addr_t node_addr);

  /**
   * Creates and persists a new current pillar block for `period`.
   *
   * Returns:
   * - The created block when vote-count deltas and parent linkage are valid.
   * - `nullptr` when local finalized/current pillar state is inconsistent.
   */
  std::shared_ptr<PillarBlock> createPillarBlock(PbftPeriod period,
                                                 const std::shared_ptr<const final_chain::BlockHeader>& block_header,
                                                 const h256& bridge_root, const h256& bridge_epoch);

  /**
   * Generates, stores, and optionally broadcasts this node's pillar vote.
   *
   * Returns:
   * - The created vote when Rust-mode verified insertion succeeds.
   * - `nullptr` when the vote cannot be weighted or inserted.
   */
  std::shared_ptr<PillarVote> genAndPlacePillarVote(PbftPeriod period, const blk_hash_t& pillar_block_hash,
                                                    const secret_t& node_sk, bool broadcast_vote);

  /**
   * Sets the network dependency used for pillar-vote gossip and vote-bundle requests.
   */
  void setNetwork(std::weak_ptr<Network> network);

  /**
   * Checks whether a pillar vote is relevant to the local current/future pillar context.
   *
   * Invariants:
   * - Relevance is planned through Rust-compatible helper facts.
   * - Duplicate votes are rejected before insertion.
   */
  bool isRelevantPillarVote(const std::shared_ptr<PillarVote> vote) const;

  /**
   * Validates one pillar vote using Rust identity inspection plus local DPoS checks.
   *
   * Returns:
   * - `true` only when relevance, uniqueness, Rust signature recovery, and DPoS
   *   eligibility all pass.
   */
  bool validatePillarVote(const std::shared_ptr<PillarVote> vote) const;

  /**
   * Returns true when `block_hash` is already the latest finalized pillar block.
   */
  bool isPillarBlockLatestFinalized(const blk_hash_t& block_hash) const;

  /**
   * Returns the latest finalized pillar block, or `nullptr` before first finalization.
   */
  std::shared_ptr<PillarBlock> getLastFinalizedPillarBlock() const;

  /**
   * Adds one verified pillar vote to the in-memory Rust-backed vote index.
   *
   * Invariants:
   * - Voter identity is recovered from Rust inspection and passed into
   *   `PillarVotes`; this method must not call `PillarVote::getVoterAddr()`.
   * - DPoS weight is looked up for the Rust-recovered voter at `period - 1`.
   *
   * Returns:
   * - The non-zero validator vote count when inserted; otherwise 0.
   */
  uint64_t addVerifiedPillarVote(const std::shared_ptr<PillarVote>& vote);

  /**
   * Validates synced PBFT pillar-vote payloads through the Rust planner.
   *
   * Purpose:
   * - Owns the Rust-mode pillar-vote bundle validation boundary for PBFT sync
   *   so PBFT manager does not inspect live `PillarVote` sidecars for protocol
   *   decisions.
   *
   * Inputs/outputs:
   * - `required_votes_period` is the PBFT block period whose pillar votes are
   *   being admitted.
   * - `pillar_vote_rlps` are canonical vote payloads inspected by Rust.
   *
   * Invariants:
   * - Current pillar-block anchor, threshold lookup, Rust bundle planning, and
   *   verified-vote insertion are all owned by Rust-mode PillarVotes APIs.
   * - The method must not recover voters through legacy C++ vote APIs.
   */
  ValidatePbftBlockPillarVotesWithRustResult validatePbftBlockPillarVotesWithRust(
      PbftPeriod required_votes_period, const std::vector<bytes>& pillar_vote_rlps);

  /**
   * Finalizes the current pillar block when enough verified votes are present.
   *
   * Returns:
   * - Above-threshold votes used for finalization, or an empty vector when
   *   finalization cannot proceed.
   */
  std::vector<std::shared_ptr<PillarVote>> finalizePillarBlock(const blk_hash_t& pillar_block_hash);

  /**
   * Typed PBFT finalization preflight result for pillar-block finalization.
   *
   * Purpose:
   * - Keeps PBFT manager from interpreting raw pillar-vote payload vectors as
   *   pillar-finalization status while preserving those vectors as executor
   *   payloads for PeriodData.
   *
   * Outputs:
   * - `success == true` when pillar finalization produced above-threshold votes.
   * - `pillar_vote_count` mirrors the payload count reported back to Rust.
   * - `pillar_votes` are temporary C++ executor payloads for PeriodData only.
   *
   * Invariants:
   * - PillarChainManager owns the current-block and threshold checks.
   * - PBFT manager must not inspect `pillar_votes` for protocol decisions.
   */
  struct FinalizePillarBlockPreflightResult {
    bool success = false;
    uint64_t pillar_vote_count = 0;
    std::vector<std::shared_ptr<PillarVote>> pillar_votes;
  };
  FinalizePillarBlockPreflightResult finalizePillarBlockForPbftPreflight(const blk_hash_t& pillar_block_hash);

  /**
   * Returns the current local pillar block, or `nullptr` before one is created.
   */
  std::shared_ptr<PillarBlock> getCurrentPillarBlock() const;

  /**
   * Current pillar-block anchor facts for PBFT block construction.
   *
   * Purpose:
   * - Lets PBFT manager validate and embed the current pillar-block anchor
   *   without consuming a live `PillarBlock` sidecar for protocol decisions.
   *
   * Outputs:
   * - `found == true` when a current pillar block exists.
   * - `period` and `hash` describe the current pillar block.
   *
   * Invariants:
   * - Does not mutate pillar-chain state.
   * - PBFT manager remains responsible for checking whether a pillar anchor is
   *   required for the candidate PBFT period.
   */
  struct CurrentPillarBlockAnchor {
    bool found = false;
    PbftPeriod period = 0;
    blk_hash_t hash;
  };
  CurrentPillarBlockAnchor currentPillarBlockAnchor() const;

  /**
   * Validates a PBFT block's pillar-anchor extra-data against the current
   * PillarChainManager anchor.
   *
   * Purpose:
   * - Keeps PBFT manager from owning pillar sidecar comparison rules while it
   *   reports only a typed check result into the Rust PBFT block-validation
   *   session.
   *
   * Inputs:
   * - `pbft_block_hash` and `pbft_period` identify the PBFT block being
   *   checked and are used for diagnostics.
   * - `pillar_block_hash` is the optional pillar hash carried by the PBFT
   *   block extra-data.
   *
   * Outputs:
   * - `valid` is true only when a current pillar block exists and its hash
   *   matches the PBFT block's pillar hash.
   * - `missing_current_anchor` distinguishes missing local pillar state from a
   *   hash mismatch.
   *
   * Invariants and edge behavior:
   * - Does not mutate pillar state or materialize additional pillar sidecars.
   * - Missing PBFT extra-data pillar hash is reported as invalid, not missing
   *   current anchor.
   */
  struct PbftBlockPillarAnchorValidation {
    bool valid = false;
    bool missing_current_anchor = false;
    PbftPeriod current_pillar_period = 0;
    blk_hash_t current_pillar_hash;
  };
  PbftBlockPillarAnchorValidation validatePbftBlockPillarAnchor(
      const blk_hash_t& pbft_block_hash, PbftPeriod pbft_period,
      const std::optional<blk_hash_t>& pillar_block_hash) const;

  /**
   * Selects the pillar anchor hash that must be embedded in locally proposed
   * PBFT block extra-data.
   *
   * Purpose:
   * - Keeps PBFT manager from reading the current pillar block sidecar to
   *   decide proposal extra-data eligibility.
   *
   * Inputs:
   * - `pbft_period` is the PBFT block period being proposed.
   *
   * Outputs:
   * - `available` is true only when the current pillar block exists for
   *   `pbft_period - 1`.
   * - `pillar_block_hash` is the selected anchor hash when available.
   *
   * Invariants and edge behavior:
   * - Does not mutate pillar state.
   * - Missing or wrong-period current pillar anchors are logged and returned as
   *   unavailable.
   */
  struct PbftExtraDataPillarAnchor {
    bool available = false;
    PbftPeriod current_pillar_period = 0;
    blk_hash_t pillar_block_hash;
  };
  PbftExtraDataPillarAnchor pbftExtraDataPillarAnchor(PbftPeriod pbft_period) const;

  /**
   * Selects the current pillar block hash for local pillar-vote generation
   * during PBFT voting.
   *
   * Purpose:
   * - Keeps PBFT manager from inspecting current pillar sidecar facts while it
   *   remains the executor for local vote signing and gossip.
   *
   * Outputs:
   * - `should_vote` is true only when the current pillar block is for
   *   `pbft_period - 1`.
   * - `pillar_block_hash` is the block that should receive the local pillar
   *   vote when `should_vote` is true.
   *
   * Invariants:
   * - Does not generate, persist, or gossip a pillar vote.
   */
  struct LocalPillarVoteAnchor {
    bool should_vote = false;
    PbftPeriod current_pillar_period = 0;
    blk_hash_t pillar_block_hash;
  };
  LocalPillarVoteAnchor localPillarVoteAnchorForPbftPeriod(PbftPeriod pbft_period) const;

  /**
   * Classifies whether PBFT startup should rerun pillar post-processing for
   * the current PBFT period.
   *
   * Purpose:
   * - Keeps PBFT manager from inspecting current pillar-block sidecar facts
   *   during restart recovery.
   *
   * Inputs:
   * - `pbft_period` is the restored PBFT chain size.
   *
   * Outputs:
   * - `should_process` is true when a current pillar block exists and its
   *   period indicates the node may have stopped after persisting the PBFT
   *   block but before processing the pillar block.
   *
   * Invariants:
   * - Does not mutate pillar state.
   * - Logs the legacy restart diagnostic when the recovery condition is met.
   */
  struct RestartPillarPostProcessingDecision {
    bool should_process = false;
    PbftPeriod current_pillar_period = 0;
  };
  RestartPillarPostProcessingDecision restartPillarPostProcessingDecision(PbftPeriod pbft_period) const;

  /**
   * Retrieves verified votes for one pillar period and block hash.
   *
   * Inputs:
   * - `above_threshold` requests the minimum sorted above-threshold vote set.
   *
   * Edge behavior:
   * - Falls back to persisted period votes only when the in-memory index has no
   *   entries for the requested key.
   */
  std::vector<std::shared_ptr<PillarVote>> getVerifiedPillarVotes(PbftPeriod period, const blk_hash_t pillar_block_hash,
                                                                  bool above_threshold = false) const;

  /**
   * Checks whether a proposed pillar block properly links to the finalized pillar chain.
   */
  bool isValidPillarBlock(const std::shared_ptr<PillarBlock>& pillar_block) const;

  /**
   * Calculates the pillar consensus threshold for a DPoS period.
   *
   * Returns:
   * - `total_eligible_vote_count / 2 + 1`, or empty when FinalChain cannot
   *   provide the period state.
   */
  std::optional<uint64_t> getPillarConsensusThreshold(PbftPeriod period) const;

 private:
  /**
   * Computes ordered validator vote-count deltas between the current and previous pillar block snapshots.
   */
  std::vector<PillarBlock::ValidatorVoteCountChange> getOrderedValidatorsVoteCountsChanges(
      const std::vector<state_api::ValidatorVoteCount>& current_vote_counts,
      const std::vector<state_api::ValidatorVoteCount>& previous_pillar_block_vote_counts);

  /**
   * Persists and installs a new current pillar block snapshot.
   */
  void saveNewPillarBlock(const std::shared_ptr<PillarBlock>& pillar_block,
                          std::vector<state_api::ValidatorVoteCount>&& new_vote_counts);

 private:
  const FicusHardforkConfig& kFicusHfConfig;

  ::rust::Box<rustaxa::BridgePillarChainStorage> rust_storage_;
  ::rust::Box<rustaxa::BridgePillarChainRuntime> pillar_runtime_;
  std::weak_ptr<Network> network_;
  std::shared_ptr<final_chain::FinalChain> final_chain_;
  std::shared_ptr<KeyManager> key_manager_;

  const addr_t node_addr_;

  std::shared_ptr<PillarBlock> last_finalized_pillar_block_;
  std::shared_ptr<PillarBlock> current_pillar_block_;
  std::vector<state_api::ValidatorVoteCount> current_pillar_block_vote_counts_;

  mutable std::shared_mutex mutex_;

  LOG_OBJECTS_DEFINE
};

/** @}*/

}  // namespace pillar_chain
}  // namespace taraxa
