#pragma once

#include <cstdint>
#include <memory>

#include "common/types.hpp"

namespace taraxa {
struct FicusHardforkConfig;
class PillarVote;

namespace pillar_chain {
class PillarBlock;

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

}  // namespace pillar_chain
}  // namespace taraxa
