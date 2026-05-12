#pragma once

#include <cstdint>
#include <memory>

#include "common/types.hpp"

namespace taraxa {
struct FicusHardforkConfig;
class PillarVote;
namespace final_chain {
class FinalChain;
}

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
 * Validates one pillar vote in Rust-mode through bridge inspection + local checks.
 */
PillarVoteValidationPlan validatePillarVoteWithRust(const FicusHardforkConfig& ficus_hf_config,
                                                    const std::shared_ptr<PillarVote>& vote,
                                                    const std::shared_ptr<final_chain::FinalChain>& final_chain,
                                                    const std::shared_ptr<PillarBlock>& current_pillar_block,
                                                    bool vote_already_known, bool is_unique);

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

}  // namespace pillar_chain
}  // namespace taraxa
