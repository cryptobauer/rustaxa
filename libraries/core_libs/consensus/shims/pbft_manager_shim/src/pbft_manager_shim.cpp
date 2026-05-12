#include "pbft/pbft_manager_shim.hpp"

#include <array>
#include <exception>
#include <optional>
#include <unordered_map>
#include <utility>
#include <vector>

#include "pbft/period_data.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

namespace {

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }

addr_t fromBridgeAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

vote_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return vote_hash_t(hash.data(), vote_hash_t::ConstructFromPointer);
}

ValidateSyncPillarVotesBundlePlanStatus toPlanStatus(uint8_t status) {
  switch (status) {
    case 0:
      return ValidateSyncPillarVotesBundlePlanStatus::kBundleValid;
    case 1:
      return ValidateSyncPillarVotesBundlePlanStatus::kBundleEmpty;
    case 2:
      return ValidateSyncPillarVotesBundlePlanStatus::kVotePeriodMismatch;
    case 3:
      return ValidateSyncPillarVotesBundlePlanStatus::kVoteBlockHashMismatch;
    case 4:
      return ValidateSyncPillarVotesBundlePlanStatus::kPrevalidationFailed;
    case 5:
      return ValidateSyncPillarVotesBundlePlanStatus::kZeroWeight;
    case 6:
      return ValidateSyncPillarVotesBundlePlanStatus::kVoterConflict;
    case 7:
      return ValidateSyncPillarVotesBundlePlanStatus::kThresholdNotReached;
    case 8:
      return ValidateSyncPillarVotesBundlePlanStatus::kWeightOverflow;
    default:
      return ValidateSyncPillarVotesBundlePlanStatus::kUnknown;
  }
}

uint8_t toPlanStatusCode(ValidateSyncPillarVotesBundlePlanStatus status) { return static_cast<uint8_t>(status); }

std::optional<uint64_t> getPillarVoteWeight(const std::shared_ptr<final_chain::FinalChain>& final_chain,
                                            const addr_t& voter, PbftPeriod required_votes_period) {
  try {
    const auto weight = final_chain->dposEligibleVoteCount(required_votes_period - 1, voter);
    if (weight == 0) {
      return std::nullopt;
    }
    return weight;
  } catch (const state_api::ErrFutureBlock&) {
    return std::nullopt;
  } catch (const std::exception&) {
    return std::nullopt;
  }
}

}  // namespace

const char* validateSyncPillarVotesBundlePlanStatusString(ValidateSyncPillarVotesBundlePlanStatus status) {
  switch (status) {
    case ValidateSyncPillarVotesBundlePlanStatus::kBundleValid:
      return "valid";
    case ValidateSyncPillarVotesBundlePlanStatus::kBundleEmpty:
      return "empty bundle";
    case ValidateSyncPillarVotesBundlePlanStatus::kVotePeriodMismatch:
      return "vote period mismatch";
    case ValidateSyncPillarVotesBundlePlanStatus::kVoteBlockHashMismatch:
      return "vote block hash mismatch";
    case ValidateSyncPillarVotesBundlePlanStatus::kPrevalidationFailed:
      return "prevalidation failed";
    case ValidateSyncPillarVotesBundlePlanStatus::kZeroWeight:
      return "zero weight";
    case ValidateSyncPillarVotesBundlePlanStatus::kVoterConflict:
      return "voter conflict";
    case ValidateSyncPillarVotesBundlePlanStatus::kThresholdNotReached:
      return "threshold not reached";
    case ValidateSyncPillarVotesBundlePlanStatus::kWeightOverflow:
      return "weight overflow";
    case ValidateSyncPillarVotesBundlePlanStatus::kUnknown:
      return "unknown";
  }
  return "unknown";
}

const char* validatePbftBlockPillarVotesWithRustStatusString(ValidatePbftBlockPillarVotesWithRustStatus status) {
  switch (status) {
    case ValidatePbftBlockPillarVotesWithRustStatus::kUnknown:
      return "unknown";
    case ValidatePbftBlockPillarVotesWithRustStatus::kValid:
      return "valid";
    case ValidatePbftBlockPillarVotesWithRustStatus::kMissingPillarChainManager:
      return "missing pillar chain manager";
    case ValidatePbftBlockPillarVotesWithRustStatus::kMissingPbftBlock:
      return "missing pbft block";
    case ValidatePbftBlockPillarVotesWithRustStatus::kMissingPillarVotes:
      return "missing pillar votes";
    case ValidatePbftBlockPillarVotesWithRustStatus::kMissingCurrentPillarBlock:
      return "missing current pillar block";
    case ValidatePbftBlockPillarVotesWithRustStatus::kPillarBlockPeriodMismatch:
      return "pillar block period mismatch";
    case ValidatePbftBlockPillarVotesWithRustStatus::kMissingThreshold:
      return "missing threshold";
    case ValidatePbftBlockPillarVotesWithRustStatus::kBridgeError:
      return "bridge error";
    case ValidatePbftBlockPillarVotesWithRustStatus::kPlanRejected:
      return "plan rejected";
    case ValidatePbftBlockPillarVotesWithRustStatus::kAcceptedVoteMissing:
      return "accepted vote missing";
    case ValidatePbftBlockPillarVotesWithRustStatus::kInsertFailed:
      return "insert failed";
  }
  return "unknown";
}

ValidateSyncPillarVotesBundleDeterministicallyResult validateSyncPillarVotesBundleDeterministically(
    const std::vector<std::shared_ptr<PillarVote>>& pillar_votes, PbftPeriod required_votes_period,
    const blk_hash_t& required_pillar_block_hash, uint64_t required_threshold,
    const std::shared_ptr<final_chain::FinalChain>& final_chain) {
  if (!final_chain || required_votes_period == 0) {
    return {ValidateSyncPillarVotesBundlePlanStatus::kUnknown, {}, 0, 0, {}, false};
  }

  if (pillar_votes.empty()) {
    return {ValidateSyncPillarVotesBundlePlanStatus::kBundleEmpty, {}, 0, 0, {}, false};
  }

  rust::Vec<rustaxa::PillarVoteBundleFact> facts;
  facts.reserve(pillar_votes.size());

  std::unordered_map<vote_hash_t, uint64_t> vote_weights;
  vote_weights.reserve(pillar_votes.size());
  std::unordered_map<vote_hash_t, addr_t> vote_recovered_voters;
  vote_recovered_voters.reserve(pillar_votes.size());

  for (const auto& vote : pillar_votes) {
    if (!vote) {
      return {ValidateSyncPillarVotesBundlePlanStatus::kUnknown, {}, 0, 0, {}, false};
    }

    rustaxa::PillarVoteInspection inspection;
    try {
      const auto rlp = vote->rlp();
      inspection = rustaxa::pillar_vote_inspect(rust::Slice<const uint8_t>(rlp.data(), rlp.size()));
    } catch (const std::exception&) {
      return {ValidateSyncPillarVotesBundlePlanStatus::kPrevalidationFailed, {}, 0, 0, {}, false};
    }

    const auto vote_hash = fromBridgeHash(inspection.vote_hash);
    if (!inspection.signature_valid) {
      return {ValidateSyncPillarVotesBundlePlanStatus::kPrevalidationFailed, vote_hash, 0, 0, {}, false};
    }

    const auto voter = fromBridgeAddress(inspection.voter);
    const auto weight = getPillarVoteWeight(final_chain, voter, required_votes_period);
    if (!weight) {
      return {ValidateSyncPillarVotesBundlePlanStatus::kZeroWeight, vote_hash, 0, 0, {}, false};
    }

    vote_recovered_voters[vote_hash] = voter;

    rustaxa::PillarVoteBundleFact fact{};
    fact.vote_hash = inspection.vote_hash;
    fact.block_hash = inspection.block_hash;
    fact.voter = inspection.voter;
    fact.period = inspection.period;
    fact.weight = *weight;
    fact.prevalidated = true;

    facts.push_back(fact);
    vote_weights[vote_hash] = *weight;
  }

  try {
    const auto plan = rustaxa::plan_pillar_vote_bundle(std::move(facts), required_votes_period,
                                                       toBridgeHash(required_pillar_block_hash), required_threshold);

    const auto plan_status = toPlanStatus(plan.status);
    ValidateSyncPillarVotesBundleDeterministicallyResult result;
    result.plan_status = plan_status;
    result.first_bad_vote_hash = fromBridgeHash(plan.first_bad_vote_hash);
    result.block_weight = plan.block_weight;
    result.selected_weight = plan.selected_weight;

    if (plan_status != ValidateSyncPillarVotesBundlePlanStatus::kBundleValid) {
      return result;
    }

    result.accepted_votes.reserve(plan.accepted_votes.size());
    for (const auto& accepted_vote : plan.accepted_votes) {
      const auto accepted_vote_hash = fromBridgeHash(accepted_vote.vote_hash);
      const auto recovered_voter_it = vote_recovered_voters.find(accepted_vote_hash);
      if (!vote_weights.contains(accepted_vote_hash) || recovered_voter_it == vote_recovered_voters.end()) {
        return {ValidateSyncPillarVotesBundlePlanStatus::kUnknown, accepted_vote_hash, 0, 0, {}, false};
      }
      result.accepted_votes.push_back({accepted_vote_hash, accepted_vote.weight, recovered_voter_it->second});
    }

    result.valid = true;
    return result;

  } catch (const std::exception&) {
    return {ValidateSyncPillarVotesBundlePlanStatus::kUnknown, {}, 0, 0, {}, false};
  }
}

ValidatePbftBlockPillarVotesWithRustResult validatePbftBlockPillarVotesWithRust(
    const PeriodData& period_data, const std::shared_ptr<pillar_chain::PillarChainManager>& pillar_chain_mgr,
    const std::shared_ptr<final_chain::FinalChain>& final_chain) {
  if (!pillar_chain_mgr) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingPillarChainManager, 0, {}, 0, 0};
  }
  if (!period_data.pbft_blk) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingPbftBlock, 0, {}, 0, 0};
  }
  if (!period_data.pillar_votes_.has_value() || period_data.pillar_votes_->empty()) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingPillarVotes, 0, {}, 0, 0};
  }

  const auto required_votes_period = period_data.pbft_blk->getPeriod();
  const auto current_pillar_block = pillar_chain_mgr->getCurrentPillarBlock();
  if (!current_pillar_block) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingCurrentPillarBlock, 0, {}, 0, 0};
  }
  if (current_pillar_block->getPeriod() + 1 != required_votes_period) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kPillarBlockPeriodMismatch, 0, {}, 0, 0};
  }

  const auto pillar_consensus_threshold = pillar_chain_mgr->getPillarConsensusThreshold(required_votes_period - 1);
  if (!pillar_consensus_threshold) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingThreshold, 0, {}, 0, 0};
  }

  const auto sync_plan = validateSyncPillarVotesBundleDeterministically(
      *period_data.pillar_votes_, required_votes_period, current_pillar_block->getHash(), *pillar_consensus_threshold,
      final_chain);
  if (!sync_plan.valid) {
    ValidatePbftBlockPillarVotesWithRustStatus status = ValidatePbftBlockPillarVotesWithRustStatus::kPlanRejected;
    if (sync_plan.plan_status == ValidateSyncPillarVotesBundlePlanStatus::kUnknown) {
      status = ValidatePbftBlockPillarVotesWithRustStatus::kBridgeError;
    }
    return {status, toPlanStatusCode(sync_plan.plan_status), sync_plan.first_bad_vote_hash, sync_plan.block_weight,
            sync_plan.selected_weight};
  }

  std::unordered_map<vote_hash_t, std::shared_ptr<PillarVote>> vote_by_hash;
  vote_by_hash.reserve(period_data.pillar_votes_->size());
  for (const auto& vote : *period_data.pillar_votes_) {
    if (!vote) {
      return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingPillarVotes, 0, {}, 0, 0};
    }
    vote_by_hash[vote->getHash()] = vote;
  }

  for (const auto& accepted_vote : sync_plan.accepted_votes) {
    const auto& vote_hash = accepted_vote.vote_hash;
    const auto vote_it = vote_by_hash.find(vote_hash);
    if (vote_it == vote_by_hash.end()) {
      return {ValidatePbftBlockPillarVotesWithRustStatus::kAcceptedVoteMissing, toPlanStatusCode(sync_plan.plan_status),
              vote_hash, sync_plan.block_weight, sync_plan.selected_weight};
    }

#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
    if (!pillar_chain_mgr->addPlannedVerifiedPillarVoteForRust(vote_it->second, *pillar_consensus_threshold,
                                                               accepted_vote.weight, accepted_vote.recovered_voter)) {
      return {ValidatePbftBlockPillarVotesWithRustStatus::kInsertFailed, toPlanStatusCode(sync_plan.plan_status),
              vote_hash, sync_plan.block_weight, sync_plan.selected_weight};
    }
#else
    (void)vote_it;
    (void)accepted_vote;
    return {ValidatePbftBlockPillarVotesWithRustStatus::kInsertFailed, toPlanStatusCode(sync_plan.plan_status),
            vote_hash, sync_plan.block_weight, sync_plan.selected_weight};
#endif
  }

  return {ValidatePbftBlockPillarVotesWithRustStatus::kValid, toPlanStatusCode(sync_plan.plan_status),
          sync_plan.first_bad_vote_hash, sync_plan.block_weight, sync_plan.selected_weight};
}

}  // namespace taraxa
