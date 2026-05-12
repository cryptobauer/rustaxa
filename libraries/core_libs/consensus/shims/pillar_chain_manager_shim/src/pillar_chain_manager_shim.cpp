#include "pillar_chain/pillar_chain_manager_shim.hpp"

#include <array>
#include <exception>

#include "config/hardfork.hpp"
#include "final_chain/final_chain.hpp"
#include "pillar_chain/pillar_block.hpp"
#include "pillar_chain/pillar_votes.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "vote/pillar_vote.hpp"

namespace taraxa::pillar_chain {
namespace {

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }
addr_t fromBridgeAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

vote_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return vote_hash_t(hash.data(), vote_hash_t::ConstructFromPointer);
}

rust::Slice<const uint8_t> toBridgeBytes(const bytes& input) {
  return rust::Slice<const uint8_t>(input.data(), input.size());
}

PillarVoteRelevancePlanStatus fromStatusCode(uint8_t status) {
  switch (status) {
    case 0:
      return PillarVoteRelevancePlanStatus::kRelevant;
    case 1:
      return PillarVoteRelevancePlanStatus::kVoteAlreadyKnown;
    case 2:
      return PillarVoteRelevancePlanStatus::kMissingCurrentPillarBlock;
    case 3:
      return PillarVoteRelevancePlanStatus::kVotePeriodMismatch;
    case 4:
      return PillarVoteRelevancePlanStatus::kVoteBlockHashMismatch;
    default:
      return PillarVoteRelevancePlanStatus::kUnknown;
  }
}

PillarVoteValidationPlanStatus fromRelevanceStatus(PillarVoteRelevancePlanStatus status) {
  switch (status) {
    case PillarVoteRelevancePlanStatus::kRelevant:
      return PillarVoteValidationPlanStatus::kValid;
    case PillarVoteRelevancePlanStatus::kVoteAlreadyKnown:
      return PillarVoteValidationPlanStatus::kDuplicate;
    case PillarVoteRelevancePlanStatus::kMissingCurrentPillarBlock:
      return PillarVoteValidationPlanStatus::kMissingCurrentPillarBlock;
    case PillarVoteRelevancePlanStatus::kVotePeriodMismatch:
      return PillarVoteValidationPlanStatus::kVotePeriodMismatch;
    case PillarVoteRelevancePlanStatus::kVoteBlockHashMismatch:
      return PillarVoteValidationPlanStatus::kVoteBlockHashMismatch;
    default:
      return PillarVoteValidationPlanStatus::kUnknown;
  }
}

}  // namespace

const char* pillarVoteValidationPlanStatusString(PillarVoteValidationPlanStatus status) {
  switch (status) {
    case PillarVoteValidationPlanStatus::kValid:
      return "valid";
    case PillarVoteValidationPlanStatus::kDuplicate:
      return "vote already known";
    case PillarVoteValidationPlanStatus::kMissingCurrentPillarBlock:
      return "missing current pillar block";
    case PillarVoteValidationPlanStatus::kVotePeriodMismatch:
      return "vote period mismatch";
    case PillarVoteValidationPlanStatus::kVoteBlockHashMismatch:
      return "vote block hash mismatch";
    case PillarVoteValidationPlanStatus::kNotUnique:
      return "vote not unique";
    case PillarVoteValidationPlanStatus::kSignatureInvalid:
      return "invalid signature";
    case PillarVoteValidationPlanStatus::kNotEligible:
      return "validator not eligible";
    case PillarVoteValidationPlanStatus::kFuturePeriod:
      return "period too far ahead of DPOS";
    case PillarVoteValidationPlanStatus::kInspectionFailure:
      return "inspection failure";
    case PillarVoteValidationPlanStatus::kUnknown:
      return "unknown";
  }
  return "unknown";
}

const char* pillarVoteRelevancePlanStatusString(PillarVoteRelevancePlanStatus status) {
  switch (status) {
    case PillarVoteRelevancePlanStatus::kRelevant:
      return "relevant";
    case PillarVoteRelevancePlanStatus::kVoteAlreadyKnown:
      return "vote already known";
    case PillarVoteRelevancePlanStatus::kMissingCurrentPillarBlock:
      return "missing current pillar block";
    case PillarVoteRelevancePlanStatus::kVotePeriodMismatch:
      return "vote period mismatch";
    case PillarVoteRelevancePlanStatus::kVoteBlockHashMismatch:
      return "vote block hash mismatch";
    case PillarVoteRelevancePlanStatus::kUnknown:
      return "unknown";
  }
  return "unknown";
}

PillarVoteRelevancePlan planPillarVoteRelevance(const FicusHardforkConfig& ficus_hf_config,
                                                const std::shared_ptr<PillarVote>& vote,
                                                const std::shared_ptr<PillarBlock>& current_pillar_block,
                                                bool vote_already_known) {
  rustaxa::PillarVoteRelevanceFact fact{};
  fact.vote_period = vote->getPeriod();
  fact.vote_block_hash = toBridgeHash(vote->getBlockHash());
  fact.first_pillar_block_period = ficus_hf_config.firstPillarBlockPeriod();
  fact.pillar_blocks_interval = ficus_hf_config.pillar_blocks_interval;
  fact.vote_already_known = vote_already_known;

  if (current_pillar_block) {
    fact.current_pillar_block_period = current_pillar_block->getPeriod();
    fact.current_pillar_block_hash = toBridgeHash(current_pillar_block->getHash());
    fact.has_current_pillar_block = true;
  }

  try {
    const auto plan = rustaxa::plan_pillar_vote_relevance(fact);
    return {fromStatusCode(plan.status), plan.is_relevant};
  } catch (const std::exception&) {
    return {PillarVoteRelevancePlanStatus::kUnknown, false};
  }
}

PillarVoteValidationPlan validatePillarVoteWithRust(const FicusHardforkConfig& ficus_hf_config,
                                                    const std::shared_ptr<PillarVote>& vote,
                                                    const std::shared_ptr<final_chain::FinalChain>& final_chain,
                                                    const std::shared_ptr<PillarBlock>& current_pillar_block,
                                                    const PillarVotes& pillar_votes) {
  if (!vote || !final_chain) {
    return {PillarVoteValidationPlanStatus::kInspectionFailure, false, 0, {}, {}};
  }

  try {
    const auto vote_already_known = pillar_votes.voteExists(vote);
    const auto relevance_plan =
        planPillarVoteRelevance(ficus_hf_config, vote, current_pillar_block, vote_already_known);
    if (!relevance_plan.is_relevant) {
      return {fromRelevanceStatus(relevance_plan.status), false, vote->getPeriod(), vote->getHash(), {}};
    }
  } catch (...) {
    return {PillarVoteValidationPlanStatus::kUnknown, false, vote->getPeriod(), vote->getHash(), {}};
  }

  auto inspection = inspectPillarVoteWithRust(vote);
  if (!inspection.is_valid) {
    return inspection;
  }
  auto recovered_voter = inspection.recovered_voter;

  if (!pillar_votes.isUniqueVoteIdentity(inspection.period, inspection.vote_hash, recovered_voter)) {
    return {PillarVoteValidationPlanStatus::kNotUnique, false, inspection.period, inspection.vote_hash,
            recovered_voter};
  }

  try {
    if (!final_chain->dposIsEligible(inspection.period - 1, recovered_voter)) {
      return {PillarVoteValidationPlanStatus::kNotEligible, false, inspection.period, inspection.vote_hash,
              recovered_voter};
    }
  } catch (state_api::ErrFutureBlock&) {
    return {PillarVoteValidationPlanStatus::kFuturePeriod, false, inspection.period, inspection.vote_hash,
            recovered_voter};
  } catch (...) {
    return {PillarVoteValidationPlanStatus::kUnknown, false, inspection.period, inspection.vote_hash, recovered_voter};
  }

  return {PillarVoteValidationPlanStatus::kValid, true, inspection.period, inspection.vote_hash, recovered_voter};
}

AddVerifiedPillarVoteWithRustPlan planAddVerifiedPillarVoteWithRust(
    const std::shared_ptr<PillarVote>& vote, const std::shared_ptr<final_chain::FinalChain>& final_chain) {
  if (!vote || !final_chain) {
    return {PillarVoteValidationPlanStatus::kInspectionFailure, false, 0, {}, {}, 0};
  }

  const auto inspection = inspectPillarVoteWithRust(vote);
  if (!inspection.is_valid || inspection.period == 0) {
    return {inspection.status, false, inspection.period, inspection.vote_hash, inspection.recovered_voter, 0};
  }

  try {
    const auto validator_vote_count =
        final_chain->dposEligibleVoteCount(inspection.period - 1, inspection.recovered_voter);
    if (validator_vote_count == 0) {
      return {PillarVoteValidationPlanStatus::kNotEligible,
              false,
              inspection.period,
              inspection.vote_hash,
              inspection.recovered_voter,
              0};
    }

    return {PillarVoteValidationPlanStatus::kValid,
            true,
            inspection.period,
            inspection.vote_hash,
            inspection.recovered_voter,
            validator_vote_count};
  } catch (state_api::ErrFutureBlock&) {
    return {PillarVoteValidationPlanStatus::kFuturePeriod,
            false,
            inspection.period,
            inspection.vote_hash,
            inspection.recovered_voter,
            0};
  } catch (...) {
    return {PillarVoteValidationPlanStatus::kUnknown,
            false,
            inspection.period,
            inspection.vote_hash,
            inspection.recovered_voter,
            0};
  }
}

PillarVoteValidationPlan inspectPillarVoteWithRust(const std::shared_ptr<PillarVote>& vote) {
  if (!vote) {
    return {PillarVoteValidationPlanStatus::kInspectionFailure, false, 0, {}, {}};
  }

  try {
    const auto inspection = rustaxa::pillar_vote_inspect(toBridgeBytes(vote->rlp()));
    const auto vote_hash = fromBridgeHash(inspection.vote_hash);
    const auto voter = fromBridgeAddress(inspection.voter);
    if (!inspection.signature_valid) {
      return {PillarVoteValidationPlanStatus::kSignatureInvalid, false, inspection.period, vote_hash, voter};
    }
    return {PillarVoteValidationPlanStatus::kValid, true, inspection.period, vote_hash, voter};
  } catch (const std::exception&) {
    return {PillarVoteValidationPlanStatus::kInspectionFailure, false, 0, {}, {}};
  }
}

}  // namespace taraxa::pillar_chain
