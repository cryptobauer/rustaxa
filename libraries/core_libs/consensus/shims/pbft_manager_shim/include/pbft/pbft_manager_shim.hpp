#pragma once

#include <cstdint>
#include <memory>
#include <vector>

#include "common/types.hpp"
#include "final_chain/final_chain.hpp"
#include "vote/pillar_vote.hpp"

namespace taraxa {

class PeriodData;

namespace pillar_chain {
class PillarChainManager;
}

/**
 * Rust-mode deterministic status for one planned pillar-vote bundle.
 *
 * Purpose:
 * - Mirrors the stable Rust planner status codes at the C++ shim boundary.
 *
 * Inputs/outputs:
 * - Produced by `validateSyncPillarVotesBundleDeterministically` after C++
 *   supplies vote context, signature prevalidation, and DPoS weights.
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
 * One Rust-planned pillar-vote insertion fact.
 *
 * Purpose:
 * - Carries the exact vote hash and DPoS weight selected by Rust so C++ can
 *   resolve a live `PillarVote` sidecar and insert it without re-querying
 *   FinalChain.
 */
struct ValidateSyncPillarVotesBundleAcceptedVote {
  vote_hash_t vote_hash{};
  uint64_t weight{0};
};

/**
 * One deterministic Rust bundle-planning result.
 *
 * Purpose:
 * - Carries Rust's validation status, aggregate weights, and accepted vote
 *   insertion facts for C++ side effects.
 *
 * Edge behavior:
 * - `valid` is true only when Rust returned `kBundleValid` and every accepted
 *   vote could be represented as a C++ insertion fact.
 */
struct ValidateSyncPillarVotesBundleDeterministicallyResult {
  ValidateSyncPillarVotesBundlePlanStatus plan_status{ValidateSyncPillarVotesBundlePlanStatus::kUnknown};
  vote_hash_t first_bad_vote_hash{};
  uint64_t block_weight{0};
  uint64_t selected_weight{0};
  std::vector<ValidateSyncPillarVotesBundleAcceptedVote> accepted_votes;
  bool valid{false};
};

/**
 * PBFT sync pillar-vote validation result status.
 *
 * Purpose:
 * - Makes Rust-mode sync failures observable without changing the public
 *   `PbftManager::validatePbftBlockPillarVotes` boolean API.
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
 * Explicit result for the Rust-mode PBFT pillar-vote sync path.
 *
 * Inputs/outputs:
 * - `status` describes the shim-level decision.
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
 * Returns a stable string for a Rust bundle-planning status.
 */
const char* validateSyncPillarVotesBundlePlanStatusString(ValidateSyncPillarVotesBundlePlanStatus status);

/**
 * Returns a stable string for the PBFT shim-level validation status.
 */
const char* validatePbftBlockPillarVotesWithRustStatusString(ValidatePbftBlockPillarVotesWithRustStatus status);

/**
 * @brief Deterministic helper used by C++ sync flow to pre-validate synced pillar vote bundles
 * in Rust-enabled pillar-votes shim mode.
 */
ValidateSyncPillarVotesBundleDeterministicallyResult validateSyncPillarVotesBundleDeterministically(
    const std::vector<std::shared_ptr<PillarVote>>& pillar_votes, PbftPeriod required_votes_period,
    const blk_hash_t& required_pillar_block_hash, uint64_t required_threshold,
    const std::shared_ptr<final_chain::FinalChain>& final_chain);

/**
 * @brief Rust-enabled PBFT pillar-vote validation path owned by the shim layer.
 *
 * This helper mirrors the legacy sync validation side effects while routing
 * deterministic bundle acceptance through Rust. It exists so the upstream-owned
 * `PbftManager` body can keep a narrow early-return hook until a complete
 * `PbftManager` overlay owns this method directly.
 */
ValidatePbftBlockPillarVotesWithRustResult validatePbftBlockPillarVotesWithRust(
    const PeriodData& period_data, const std::shared_ptr<pillar_chain::PillarChainManager>& pillar_chain_mgr,
    const std::shared_ptr<final_chain::FinalChain>& final_chain);

}  // namespace taraxa
