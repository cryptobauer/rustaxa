#include "pillar_chain/pillar_chain_manager_shim.hpp"

#include <algorithm>
#include <array>
#include <cassert>
#include <exception>
#include <libff/common/profiling.hpp>
#include <unordered_map>

#include "config/hardfork.hpp"
#include "final_chain/final_chain.hpp"
#include "key_manager/key_manager.hpp"
#include "network/network.hpp"
#include "pillar_chain/pillar_block.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"
#include "vote/pillar_vote.hpp"
#include "vote/votes_bundle_rlp.hpp"

namespace taraxa::pillar_chain {
namespace {
static constexpr uint16_t PILLAR_VOTES_POS_IN_PERIOD_DATA = 4;
static constexpr uint8_t kPbftFinalChainFactStatusReady = 0;
static constexpr uint8_t kPillarFinalizationReady = 0;
static constexpr uint8_t kPillarFinalizationMissingCurrentBlock = 1;
static constexpr uint8_t kPillarFinalizationCurrentBlockHashMismatch = 2;
static constexpr uint8_t kPillarFinalizationMissingVotes = 3;
static constexpr uint8_t kPillarFinalizationAlreadyFinalized = 4;

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }
std::array<uint8_t, 20> toBridgeAddress(const addr_t& address) { return address.asArray(); }
addr_t fromBridgeAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

blk_hash_t fromBridgeBlockHash(const std::array<uint8_t, 32>& hash) {
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

h256 fromBridgeH256(const std::array<uint8_t, 32>& hash) { return h256(hash.data(), h256::ConstructFromPointer); }

vote_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return vote_hash_t(hash.data(), vote_hash_t::ConstructFromPointer);
}

rust::Slice<const uint8_t> toBridgeBytes(const bytes& input) {
  return rust::Slice<const uint8_t>(input.data(), input.size());
}

rust::Vec<uint8_t> toRustBytes(const bytes& input) {
  rust::Vec<uint8_t> out;
  out.reserve(input.size());
  for (auto byte : input) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

bytes fromRustBytes(const rust::Vec<uint8_t>& input) { return bytes(input.begin(), input.end()); }

rustaxa::PbftFinalChainFacts collectPillarDposFacts(const std::shared_ptr<final_chain::FinalChain>& final_chain,
                                                    PbftPeriod dpos_period, bool collect_total_vote_count,
                                                    const std::vector<addr_t>& voters) {
  rustaxa::PbftFinalChainFactRequest request{};
  request.period = dpos_period;
  request.collect_total_vote_count = collect_total_vote_count;
  request.collect_address_vote_counts = !voters.empty();
  request.addresses.reserve(voters.size());
  for (const auto& voter : voters) {
    request.addresses.push_back(rustaxa::PbftFinalChainFactAddress{toBridgeAddress(voter)});
  }
  return final_chain->collectPbftFinalChainFacts(std::move(request));
}

bool finalChainFactReady(uint8_t status) { return status == kPbftFinalChainFactStatusReady; }

bool firstAddressFactReady(const rustaxa::PbftFinalChainFacts& facts) {
  return !facts.address_facts.empty() && finalChainFactReady(facts.address_facts[0].status);
}

std::shared_ptr<PillarBlock> decodePillarBlockFromRustBytes(const rust::Vec<uint8_t>& data) {
  if (data.empty()) {
    return {};
  }

  auto bytes = fromRustBytes(data);
  return std::make_shared<PillarBlock>(dev::RLP(bytes));
}

std::shared_ptr<PillarVote> decodePillarVoteFromRustBytes(const rust::Vec<uint8_t>& data) {
  if (data.empty()) {
    return {};
  }

  auto bytes = fromRustBytes(data);
  return std::make_shared<PillarVote>(dev::RLP(bytes));
}

std::optional<CurrentPillarBlockDataDb> decodeCurrentPillarBlockDataFromRustBytes(const rust::Vec<uint8_t>& data) {
  if (data.empty()) {
    return {};
  }

  auto bytes = fromRustBytes(data);
  return util::rlp_dec<CurrentPillarBlockDataDb>(dev::RLP(bytes));
}

std::vector<std::shared_ptr<PillarVote>> decodePeriodPillarVotesFromRustBytes(const rust::Vec<uint8_t>& data) {
  if (data.empty()) {
    return {};
  }

  auto bytes = fromRustBytes(data);
  const auto period_data_rlp = dev::RLP(bytes);
  if (period_data_rlp.itemCount() <= PILLAR_VOTES_POS_IN_PERIOD_DATA) {
    return {};
  }

  return decodePillarVotesBundleRlp(period_data_rlp[PILLAR_VOTES_POS_IN_PERIOD_DATA]);
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

ValidateSyncPillarVotesBundlePlanStatus toSyncPillarVotesBundlePlanStatus(uint8_t status) {
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

uint8_t toSyncPillarVotesBundlePlanStatusCode(ValidateSyncPillarVotesBundlePlanStatus status) {
  return static_cast<uint8_t>(status);
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

rustaxa::PillarValidatorVoteCount toBridgeVoteCount(const state_api::ValidatorVoteCount& vote_count) {
  rustaxa::PillarValidatorVoteCount out{};
  out.address = toBridgeAddress(vote_count.addr);
  out.vote_count = vote_count.vote_count;
  return out;
}

rust::Vec<rustaxa::PillarValidatorVoteCount> toBridgeVoteCounts(
    const std::vector<state_api::ValidatorVoteCount>& vote_counts) {
  rust::Vec<rustaxa::PillarValidatorVoteCount> out;
  out.reserve(vote_counts.size());
  for (const auto& vote_count : vote_counts) {
    out.push_back(toBridgeVoteCount(vote_count));
  }
  return out;
}

std::vector<state_api::ValidatorVoteCount> loadPillarValidatorVoteCounts(
    const std::shared_ptr<final_chain::FinalChain>& final_chain, PbftPeriod period) {
  auto rust_vote_counts = final_chain->dposValidatorsEligibleVoteCounts(period);
  std::vector<state_api::ValidatorVoteCount> out;
  out.reserve(rust_vote_counts.size());
  for (const auto& vote_count : rust_vote_counts) {
    out.push_back(vote_count);
  }
  return out;
}

std::vector<PillarBlock::ValidatorVoteCountChange> fromBridgeVoteCountChanges(
    const rust::Vec<rustaxa::PillarValidatorVoteCountChange>& changes) {
  std::vector<PillarBlock::ValidatorVoteCountChange> out;
  out.reserve(changes.size());
  for (const auto& change : changes) {
    out.emplace_back(fromBridgeAddress(change.address), change.vote_count_change);
  }
  return out;
}

std::runtime_error pillarVotesError(const std::string& message) {
  return std::runtime_error("PillarChainManager: " + message);
}

rustaxa::PillarVotePayload toBridgeVotePayload(const std::shared_ptr<PillarVote>& vote, const addr_t& recovered_voter,
                                              uint64_t validator_vote_count) {
  if (!vote) {
    throw pillarVotesError("cannot bridge null pillar vote");
  }
  if (validator_vote_count == 0) {
    throw pillarVotesError("validator vote count must be non-zero");
  }

  return rustaxa::PillarVotePayload{
      toBridgeHash(vote->getHash()), toBridgeHash(vote->getBlockHash()), toBridgeAddress(recovered_voter),
      vote->getPeriod(), validator_vote_count, toRustBytes(vote->rlp())};
}

rustaxa::PillarVoteIdentityPayload toBridgeVoteIdentityPayload(PbftPeriod period, const vote_hash_t& vote_hash,
                                                              const addr_t& recovered_voter) {
  return rustaxa::PillarVoteIdentityPayload{toBridgeHash(vote_hash), toBridgeAddress(recovered_voter), period};
}

std::shared_ptr<PillarVote> materializePillarVoteRecord(const rustaxa::PillarVoteRecord& vote_record) {
  bytes vote_rlp;
  vote_rlp.reserve(vote_record.vote_rlp.size());
  for (const auto& byte : vote_record.vote_rlp) {
    vote_rlp.push_back(byte);
  }

  auto vote = std::make_shared<PillarVote>(dev::RLP(vote_rlp));
  const auto vote_hash = fromBridgeHash(vote_record.vote_hash);
  if (vote->getHash() != vote_hash) {
    throw pillarVotesError("rust retained pillar vote hash mismatch when materializing vote payload");
  }
  return vote;
}

std::vector<std::shared_ptr<PillarVote>> materializePillarVotes(const rustaxa::PillarVotesPayloadLookup& lookup) {
  std::vector<std::shared_ptr<PillarVote>> votes;
  votes.reserve(lookup.votes.size());
  for (const auto& vote_record : lookup.votes) {
    votes.push_back(materializePillarVoteRecord(vote_record));
  }
  return votes;
}

bool pillarVoteExistsByLookup(const rustaxa::BridgePillarVotes& pillar_votes, const std::shared_ptr<PillarVote>& vote) {
  if (!vote) {
    return false;
  }
  try {
    const auto lookup = pillar_votes.pillar_votes_get_verified_vote_payloads(
        vote->getPeriod(), toBridgeHash(vote->getBlockHash()), false);
    const auto vote_hash = toBridgeHash(vote->getHash());
    return std::any_of(lookup.votes.begin(), lookup.votes.end(),
                       [&](const auto& record) { return record.vote_hash == vote_hash; });
  } catch (const std::exception&) {
    return false;
  }
}

std::optional<uint64_t> getPillarVoteWeight(const std::shared_ptr<final_chain::FinalChain>& final_chain,
                                            const addr_t& voter, PbftPeriod required_votes_period) {
  try {
    rustaxa::PbftFinalChainFactRequest request{};
    request.period = required_votes_period - 1;
    request.collect_address_vote_counts = true;
    request.addresses.push_back(rustaxa::PbftFinalChainFactAddress{toBridgeAddress(voter)});

    const auto facts = final_chain->collectPbftFinalChainFacts(std::move(request));
    if (facts.address_facts.empty() || !finalChainFactReady(facts.address_facts[0].status) ||
        facts.address_facts[0].vote_count == 0) {
      return std::nullopt;
    }
    return facts.address_facts[0].vote_count;
  } catch (const std::exception&) {
    return std::nullopt;
  }
}

rustaxa::PillarBlockLinkageFact toBridgeLinkageFact(const FicusHardforkConfig& ficus_hf_config,
                                                    const std::shared_ptr<PillarBlock>& pillar_block,
                                                    const std::shared_ptr<PillarBlock>& last_finalized_pillar_block) {
  rustaxa::PillarBlockLinkageFact fact{};
  fact.pillar_block_period = pillar_block->getPeriod();
  fact.pillar_block_previous_hash = toBridgeHash(pillar_block->getPreviousBlockHash());
  fact.first_pillar_block_period = ficus_hf_config.firstPillarBlockPeriod();
  fact.pillar_blocks_interval = ficus_hf_config.pillar_blocks_interval;
  fact.has_last_finalized_pillar_block = static_cast<bool>(last_finalized_pillar_block);
  if (last_finalized_pillar_block) {
    fact.last_finalized_period = last_finalized_pillar_block->getPeriod();
    fact.last_finalized_hash = toBridgeHash(last_finalized_pillar_block->getHash());
  }
  return fact;
}

rustaxa::PillarBlockCreationFact toBridgeCreationFact(
    const FicusHardforkConfig& ficus_hf_config, PbftPeriod period,
    const std::shared_ptr<const final_chain::BlockHeader>& block_header, const h256& bridge_root,
    const h256& bridge_epoch, const std::shared_ptr<PillarBlock>& last_finalized_pillar_block) {
  rustaxa::PillarBlockCreationFact fact{};
  fact.pillar_block_period = period;
  fact.state_root = toBridgeHash(block_header->state_root);
  fact.bridge_root = toBridgeHash(bridge_root);
  fact.bridge_epoch = toBridgeHash(bridge_epoch);
  fact.first_pillar_block_period = ficus_hf_config.firstPillarBlockPeriod();
  fact.pillar_blocks_interval = ficus_hf_config.pillar_blocks_interval;
  fact.has_last_finalized_pillar_block = static_cast<bool>(last_finalized_pillar_block);
  if (last_finalized_pillar_block) {
    fact.last_finalized_period = last_finalized_pillar_block->getPeriod();
    fact.last_finalized_hash = toBridgeHash(last_finalized_pillar_block->getHash());
  }
  return fact;
}

rustaxa::PillarBlockFinalizationFact toBridgeFinalizationFact(
    const blk_hash_t& requested_pillar_block_hash, const std::shared_ptr<PillarBlock>& current_pillar_block,
    const rustaxa::PillarVotesPayloadLookup& vote_lookup,
    const std::shared_ptr<PillarBlock>& last_finalized_pillar_block) {
  rustaxa::PillarBlockFinalizationFact fact{};
  fact.requested_pillar_block_hash = toBridgeHash(requested_pillar_block_hash);
  fact.has_current_pillar_block = static_cast<bool>(current_pillar_block);
  if (current_pillar_block) {
    fact.current_period = current_pillar_block->getPeriod();
    fact.current_hash = toBridgeHash(current_pillar_block->getHash());
  }
  fact.threshold_met = vote_lookup.threshold_met;
  fact.block_weight = vote_lookup.block_weight;
  fact.selected_weight = vote_lookup.selected_weight;
  fact.selected_vote_count = vote_lookup.votes.size();
  fact.has_last_finalized_pillar_block = static_cast<bool>(last_finalized_pillar_block);
  if (last_finalized_pillar_block) {
    fact.last_finalized_hash = toBridgeHash(last_finalized_pillar_block->getHash());
  }
  return fact;
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
                                                    const ::rust::Box<rustaxa::BridgePillarVotes>& pillar_votes) {
  if (!vote || !final_chain) {
    return {PillarVoteValidationPlanStatus::kInspectionFailure, false, 0, {}, {}};
  }

  try {
    const auto vote_already_known = pillarVoteExistsByLookup(*pillar_votes, vote);
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

  try {
    if (!pillar_votes
             ->pillar_votes_is_unique_identity(
                 toBridgeVoteIdentityPayload(inspection.period, inspection.vote_hash, recovered_voter))
             .is_unique) {
      return {PillarVoteValidationPlanStatus::kNotUnique, false, inspection.period, inspection.vote_hash, recovered_voter};
    }
  } catch (const std::exception&) {
    return {PillarVoteValidationPlanStatus::kNotUnique, false, inspection.period, inspection.vote_hash,
            recovered_voter};
  }

  try {
    const auto dpos_facts = collectPillarDposFacts(final_chain, inspection.period - 1, false, {recovered_voter});
    if (!firstAddressFactReady(dpos_facts)) {
      return {PillarVoteValidationPlanStatus::kFuturePeriod, false, inspection.period, inspection.vote_hash,
              recovered_voter};
    }
    if (!dpos_facts.address_facts[0].eligible) {
      return {PillarVoteValidationPlanStatus::kNotEligible, false, inspection.period, inspection.vote_hash,
              recovered_voter};
    }
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
    const auto dpos_facts =
        collectPillarDposFacts(final_chain, inspection.period - 1, false, {inspection.recovered_voter});
    if (!firstAddressFactReady(dpos_facts)) {
      return {PillarVoteValidationPlanStatus::kFuturePeriod,
              false,
              inspection.period,
              inspection.vote_hash,
              inspection.recovered_voter,
              0};
    }

    const auto validator_vote_count = dpos_facts.address_facts[0].vote_count;
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

ValidateSyncPillarVotesBundleDeterministicallyResult validateSyncPillarVotesBundleDeterministically(
    const std::vector<bytes>& pillar_vote_rlps, PbftPeriod required_votes_period,
    const blk_hash_t& required_pillar_block_hash, uint64_t required_threshold,
    const std::shared_ptr<final_chain::FinalChain>& final_chain) {
  if (!final_chain || required_votes_period == 0) {
    return {ValidateSyncPillarVotesBundlePlanStatus::kUnknown, {}, 0, 0, {}, false};
  }

  if (pillar_vote_rlps.empty()) {
    return {ValidateSyncPillarVotesBundlePlanStatus::kBundleEmpty, {}, 0, 0, {}, false};
  }

  rust::Vec<rustaxa::PillarVoteBundleFact> facts;
  facts.reserve(pillar_vote_rlps.size());

  std::unordered_map<vote_hash_t, uint64_t> vote_weights;
  vote_weights.reserve(pillar_vote_rlps.size());
  std::unordered_map<vote_hash_t, addr_t> vote_recovered_voters;
  vote_recovered_voters.reserve(pillar_vote_rlps.size());

  for (const auto& vote_rlp : pillar_vote_rlps) {
    rustaxa::PillarVoteInspection inspection;
    try {
      inspection = rustaxa::pillar_vote_inspect(toBridgeBytes(vote_rlp));
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

    const auto plan_status = toSyncPillarVotesBundlePlanStatus(plan.status);
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

PillarChainManager::PillarChainManager(const FicusHardforkConfig& ficus_hf_config, std::shared_ptr<DbStorage> db,
                                       std::shared_ptr<final_chain::FinalChain> final_chain,
                                       std::shared_ptr<KeyManager> key_manager, addr_t node_addr)
    : kFicusHfConfig(ficus_hf_config),
      rust_storage_(rustaxa::create_pillar_chain_storage(db->rustStorage())),
      network_{},
      final_chain_{std::move(final_chain)},
      key_manager_(std::move(key_manager)),
      node_addr_(node_addr),
      last_finalized_pillar_block_{},
      current_pillar_block_{},
      current_pillar_block_vote_counts_{},
      pillar_votes_{rustaxa::create_pillar_votes_index()},
      mutex_{} {
  LOG_OBJECTS_CREATE("PILLAR_CHAIN");

  if (const auto vote = decodePillarVoteFromRustBytes(rust_storage_->pillar_chain_storage_load_own_vote()); vote) {
    addVerifiedPillarVote(vote);
  }

  if (auto&& current_pillar_block_data =
          decodeCurrentPillarBlockDataFromRustBytes(rust_storage_->pillar_chain_storage_load_current_block_data());
      current_pillar_block_data.has_value()) {
    current_pillar_block_ = std::move(current_pillar_block_data->pillar_block);
    current_pillar_block_vote_counts_ = std::move(current_pillar_block_data->vote_counts);
  }

  if (auto&& latest_pillar_block =
          decodePillarBlockFromRustBytes(rust_storage_->pillar_chain_storage_load_latest_block());
      latest_pillar_block) {
    last_finalized_pillar_block_ = std::move(latest_pillar_block);

    const auto last_finalized_pillar_block_votes = decodePeriodPillarVotesFromRustBytes(
        rust_storage_->pillar_chain_storage_load_period_data(last_finalized_pillar_block_->getPeriod() + 1));
    // There should always be pillar votes stored in period data for finalized pillar block
    assert(!last_finalized_pillar_block_votes.empty());
    for (const auto& pillar_vote : last_finalized_pillar_block_votes) {
      addVerifiedPillarVote(pillar_vote);
    }
  }
}

std::shared_ptr<PillarBlock> PillarChainManager::createPillarBlock(
    PbftPeriod period, const std::shared_ptr<const final_chain::BlockHeader>& block_header, const h256& bridge_root,
    const h256& bridge_epoch) {
  auto new_vote_counts = loadPillarValidatorVoteCounts(final_chain_, period);
  std::vector<PillarBlock::ValidatorVoteCountChange> votes_count_changes;
  std::shared_ptr<PillarBlock> last_finalized_pillar_block;

  // First ever pillar block
  if (period == kFicusHfConfig.firstPillarBlockPeriod()) {
    try {
      rust::Vec<rustaxa::PillarValidatorVoteCount> empty_previous_vote_counts;
      votes_count_changes = fromBridgeVoteCountChanges(rustaxa::plan_pillar_vote_count_changes(
          toBridgeVoteCounts(new_vote_counts), std::move(empty_previous_vote_counts)));
    } catch (const std::exception& e) {
      LOG(log_er_) << "Unable to plan first pillar block vote-count changes in Rust for period " << period << ": "
                   << e.what();
      return nullptr;
    }
  } else {
    last_finalized_pillar_block = getLastFinalizedPillarBlock();
    // This should never happen !!!
    if (!last_finalized_pillar_block) {
      LOG(log_er_) << "Empty last finalized pillar block, new pillar block period " << period;
      assert(false);
      return nullptr;
    }

    // !!!Note: No need to protect current_pillar_block_vote_counts_ as it is read & written only in this function,
    // which is always called once in a time
    // This should never happen !!!
    if (current_pillar_block_vote_counts_.empty()) {
      LOG(log_er_) << "Empty current pillar block vote counts, new pillar block period " << period;
      assert(false);
      return nullptr;
    }

    // Get validators vote counts changes between the current and previous pillar block
    try {
      votes_count_changes = fromBridgeVoteCountChanges(rustaxa::plan_pillar_vote_count_changes(
          toBridgeVoteCounts(new_vote_counts), toBridgeVoteCounts(current_pillar_block_vote_counts_)));
    } catch (const std::exception& e) {
      LOG(log_er_) << "Unable to plan pillar block vote-count changes in Rust for period " << period << ": "
                   << e.what();
      return nullptr;
    }
  }

  rustaxa::PillarBlockCreationPlan creation_plan{};
  try {
    creation_plan = rustaxa::plan_pillar_block_creation(toBridgeCreationFact(
        kFicusHfConfig, period, block_header, bridge_root, bridge_epoch, last_finalized_pillar_block));
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to plan pillar block creation in Rust for period " << period << ": " << e.what();
    return nullptr;
  }
  if (!creation_plan.valid) {
    LOG(log_er_) << "Invalid pillar block creation plan for period " << period << ", linkage status "
                 << static_cast<uint64_t>(creation_plan.status) << ", expected previous period "
                 << creation_plan.expected_previous_period;
    return nullptr;
  }

  const auto pillar_block = std::make_shared<PillarBlock>(
      period, fromBridgeH256(creation_plan.state_root), fromBridgeBlockHash(creation_plan.previous_pillar_block_hash),
      fromBridgeH256(creation_plan.bridge_root), fromBridgeH256(creation_plan.bridge_epoch),
      std::move(votes_count_changes));

  // Check if some pillar block was not skipped
  if (!isValidPillarBlock(pillar_block)) {
    LOG(log_er_) << "Newly created pillar block " << pillar_block->getHash() << "with period "
                 << pillar_block->getPeriod() << " is invalid";
    return nullptr;
  }

  saveNewPillarBlock(pillar_block, std::move(new_vote_counts));
  LOG(log_nf_) << "New pillar block " << pillar_block->getHash() << " with period " << pillar_block->getPeriod()
               << " created";

  return pillar_block;
}

void PillarChainManager::saveNewPillarBlock(const std::shared_ptr<PillarBlock>& pillar_block,
                                            std::vector<state_api::ValidatorVoteCount>&& new_vote_counts) {
  std::scoped_lock<std::shared_mutex> lock(mutex_);
  rust_storage_->pillar_chain_storage_apply_current_block_data(
      toRustBytes(util::rlp_enc(CurrentPillarBlockDataDb{pillar_block, new_vote_counts})));
  current_pillar_block_ = pillar_block;
  current_pillar_block_vote_counts_ = std::move(new_vote_counts);
}

std::shared_ptr<PillarVote> PillarChainManager::genAndPlacePillarVote(PbftPeriod period,
                                                                      const blk_hash_t& pillar_block_hash,
                                                                      const secret_t& node_sk, bool broadcast_vote) {
  const auto vote = std::make_shared<PillarVote>(node_sk, period, pillar_block_hash);

  // Broadcasts pillar vote
  const auto vote_weight = addVerifiedPillarVote(vote);
  if (!vote_weight) {
    LOG(log_er_) << "Unable to gen pillar vote. Vote was not added to the verified votes. Vote hash "
                 << vote->getHash();
    return nullptr;
  }
  rust_storage_->pillar_chain_storage_apply_own_vote(toRustBytes(util::rlp_enc(vote)));

  if (auto net = network_.lock(); net && broadcast_vote) {
    net->gossipPillarBlockVote(vote);
    LOG(log_nf_) << "Placed pillar vote " << vote->getHash() << " for block " << vote->getBlockHash() << ", period "
                 << vote->getPeriod() << ", weight " << vote_weight;
  } else {
    LOG(log_nf_) << "Created pillar vote " << vote->getHash() << " for block " << vote->getBlockHash() << ", period "
                 << vote->getPeriod() << ", weight " << vote_weight;
  }

  return vote;
}

std::vector<std::shared_ptr<PillarVote>> PillarChainManager::finalizePillarBlock(const blk_hash_t& pillar_block_hash) {
  const auto current_pillar_block = getCurrentPillarBlock();
  rustaxa::PillarVotesPayloadLookup pillar_vote_lookup;
  std::vector<std::shared_ptr<PillarVote>> pillar_votes;
  if (current_pillar_block && current_pillar_block->getHash() == pillar_block_hash) {
    try {
      pillar_vote_lookup = pillar_votes_->pillar_votes_get_verified_vote_payloads(
          current_pillar_block->getPeriod() + 1, toBridgeHash(pillar_block_hash), true /* above_threshold */);
      pillar_votes = materializePillarVotes(pillar_vote_lookup);
    } catch (const std::exception& e) {
      LOG(log_er_) << "Unable to materialize verified pillar votes for finalized block " << pillar_block_hash << ": "
                   << e.what();
      return {};
    }
  }
  const auto last_finalized_pillar_block = getLastFinalizedPillarBlock();

  rustaxa::PillarBlockFinalizationPlan finalization_plan{};
  try {
    finalization_plan = rustaxa::plan_pillar_block_finalization(
        toBridgeFinalizationFact(pillar_block_hash, current_pillar_block, pillar_vote_lookup,
                                 last_finalized_pillar_block));
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to plan pillar block finalization in Rust for " << pillar_block_hash << ": " << e.what();
    return {};
  }

  switch (finalization_plan.status) {
    case kPillarFinalizationReady:
      break;
    case kPillarFinalizationMissingCurrentBlock:
      LOG(log_er_) << "Cannot finalize pillar block " << pillar_block_hash << ". Empty current pillar block";
      return {};
    case kPillarFinalizationCurrentBlockHashMismatch:
      LOG(log_er_) << "Cannot finalize pillar block " << pillar_block_hash << ". Provided pillar block hash "
                   << pillar_block_hash << " != current pillar block hash " << current_pillar_block->getHash();
      return {};
    case kPillarFinalizationMissingVotes:
      LOG(log_er_) << "Cannot finalize pillar block " << pillar_block_hash
                   << ". Not enough pillar votes for pillar block. Request it";
      if (finalization_plan.should_request_votes) {
        if (auto net = network_.lock()) {
          net->requestPillarBlockVotesBundle(current_pillar_block->getPeriod() + 1, pillar_block_hash);
        }
      }
      return {};
    case kPillarFinalizationAlreadyFinalized:
      LOG(log_er_) << "Pillar block already " << pillar_block_hash << " already finalized";
      return finalization_plan.return_votes ? pillar_votes : std::vector<std::shared_ptr<PillarVote>>{};
    default:
      LOG(log_er_) << "Unable to plan pillar block finalization for " << pillar_block_hash
                   << ". Unknown Rust status " << static_cast<uint64_t>(finalization_plan.status);
      return {};
  }

  if (finalization_plan.should_persist) {
    rust_storage_->pillar_chain_storage_apply_finalized_block(finalization_plan.current_period,
                                                              toRustBytes(current_pillar_block->getRlp()));
  }
  LOG(log_nf_) << "Pillar block " << pillar_block_hash << " with period " << finalization_plan.current_period
               << " finalized";

  {
    std::scoped_lock<std::shared_mutex> lock(mutex_);
    last_finalized_pillar_block_ = current_pillar_block;

    // Erase votes that are no longer needed
    try {
      pillar_votes_->pillar_votes_cleanup_votes_by_period(last_finalized_pillar_block_->getPeriod() + 1);
    } catch (const std::exception&) {
      LOG(log_er_) << "Unable to cleanup pillar votes for finalized period "
                   << last_finalized_pillar_block_->getPeriod() + 1;
      return {};
    }
  }
  if (finalization_plan.should_emit) {
    pillar_block_finalized_emitter_.emit(PillarBlockData{current_pillar_block, pillar_votes});
  }

  return finalization_plan.return_votes ? pillar_votes : std::vector<std::shared_ptr<PillarVote>>{};
}

PillarChainManager::FinalizePillarBlockPreflightResult PillarChainManager::finalizePillarBlockForPbftPreflight(
    const blk_hash_t& pillar_block_hash) {
  FinalizePillarBlockPreflightResult result;
  result.pillar_votes = finalizePillarBlock(pillar_block_hash);
  result.pillar_vote_count = result.pillar_votes.size();
  result.success = !result.pillar_votes.empty();
  return result;
}

bool PillarChainManager::isPillarBlockLatestFinalized(const blk_hash_t& block_hash) const {
  std::shared_lock<std::shared_mutex> lock(mutex_);

  // Current pillar block was already pushed into the pillar chain
  if (last_finalized_pillar_block_ && last_finalized_pillar_block_->getHash() == block_hash) {
    return true;
  }

  return false;
}

std::shared_ptr<PillarBlock> PillarChainManager::getLastFinalizedPillarBlock() const {
  std::shared_lock<std::shared_mutex> lock(mutex_);
  return last_finalized_pillar_block_;
}

std::shared_ptr<PillarBlock> PillarChainManager::getCurrentPillarBlock() const {
  std::shared_lock<std::shared_mutex> lock(mutex_);
  return current_pillar_block_;
}

PillarChainManager::CurrentPillarBlockAnchor PillarChainManager::currentPillarBlockAnchor() const {
  std::shared_lock<std::shared_mutex> lock(mutex_);
  CurrentPillarBlockAnchor anchor;
  if (!current_pillar_block_) {
    return anchor;
  }
  anchor.found = true;
  anchor.period = current_pillar_block_->getPeriod();
  anchor.hash = current_pillar_block_->getHash();
  return anchor;
}

PillarChainManager::PbftBlockPillarAnchorValidation PillarChainManager::validatePbftBlockPillarAnchor(
    const blk_hash_t& pbft_block_hash, PbftPeriod pbft_period,
    const std::optional<blk_hash_t>& pillar_block_hash) const {
  std::shared_lock<std::shared_mutex> lock(mutex_);
  PbftBlockPillarAnchorValidation result;
  if (!current_pillar_block_) {
    result.missing_current_anchor = true;
    LOG(log_er_) << "Unable to validate PBFT block " << pbft_block_hash << ", period " << pbft_period
                 << ". No current pillar block present in node";
    return result;
  }

  result.current_pillar_period = current_pillar_block_->getPeriod();
  result.current_pillar_hash = current_pillar_block_->getHash();
  if (!pillar_block_hash.has_value() || *pillar_block_hash != result.current_pillar_hash) {
    LOG(log_er_) << "PBFT block " << pbft_block_hash << " with period " << pbft_period << " contains pillar block hash "
                 << pillar_block_hash.value_or(kNullBlockHash) << ", which is different than the local current pillar "
                 << "block " << result.current_pillar_hash << " with period " << result.current_pillar_period;
    return result;
  }

  result.valid = true;
  return result;
}

PillarChainManager::PbftExtraDataPillarAnchor PillarChainManager::pbftExtraDataPillarAnchor(
    PbftPeriod pbft_period) const {
  std::shared_lock<std::shared_mutex> lock(mutex_);
  PbftExtraDataPillarAnchor result;
  if (!current_pillar_block_) {
    LOG(log_er_) << "Missing pillar block, pbft period " << pbft_period;
    return result;
  }

  result.current_pillar_period = current_pillar_block_->getPeriod();
  if (result.current_pillar_period != pbft_period - 1) {
    LOG(log_er_) << "Wrong pillar block period: " << result.current_pillar_period << ", pbft period: " << pbft_period;
    return result;
  }

  result.available = true;
  result.pillar_block_hash = current_pillar_block_->getHash();
  return result;
}

PillarChainManager::LocalPillarVoteAnchor PillarChainManager::localPillarVoteAnchorForPbftPeriod(
    PbftPeriod pbft_period) const {
  std::shared_lock<std::shared_mutex> lock(mutex_);
  LocalPillarVoteAnchor result;
  if (!current_pillar_block_) {
    return result;
  }

  result.current_pillar_period = current_pillar_block_->getPeriod();
  if (result.current_pillar_period != pbft_period - 1) {
    return result;
  }

  result.should_vote = true;
  result.pillar_block_hash = current_pillar_block_->getHash();
  return result;
}

PillarChainManager::RestartPillarPostProcessingDecision PillarChainManager::restartPillarPostProcessingDecision(
    PbftPeriod pbft_period) const {
  std::shared_lock<std::shared_mutex> lock(mutex_);
  RestartPillarPostProcessingDecision decision;
  if (!current_pillar_block_) {
    return decision;
  }

  decision.current_pillar_period = current_pillar_block_->getPeriod();
  decision.should_process =
      pbft_period == decision.current_pillar_period + kFicusHfConfig.pillar_blocks_interval;
  if (decision.should_process) {
    LOG(log_er_) << "Pillar block was not processed before restart, current period: " << pbft_period
                 << ", current pillar block period: " << decision.current_pillar_period;
  }
  return decision;
}

bool PillarChainManager::isRelevantPillarVote(const std::shared_ptr<PillarVote> vote) const {
  const auto vote_exists = pillarVoteExistsByLookup(*pillar_votes_, vote);
  const auto current_pillar_block = getCurrentPillarBlock();
  const auto relevance_plan = planPillarVoteRelevance(kFicusHfConfig, vote, current_pillar_block, vote_exists);

  if (!relevance_plan.is_relevant) {
    switch (relevance_plan.status) {
      case pillar_chain::PillarVoteRelevancePlanStatus::kVoteAlreadyKnown:
        LOG(log_dg_) << "Received vote " << vote->getHash() << " already saved";
        return false;
      case pillar_chain::PillarVoteRelevancePlanStatus::kMissingCurrentPillarBlock:
        LOG(log_nf_) << "Received vote's period " << vote->getPeriod()
                     << ", no pillar block created yet. Accepting votes with "
                     << kFicusHfConfig.firstPillarBlockPeriod() + 1 << " period";
        return false;
      case pillar_chain::PillarVoteRelevancePlanStatus::kVotePeriodMismatch:
        if (!current_pillar_block) {
          LOG(log_nf_) << "Received vote's period " << vote->getPeriod() << ", current pillar block missing";
        } else {
          LOG(log_nf_) << "Received vote's period " << vote->getPeriod() << ", current pillar block period "
                       << current_pillar_block->getPeriod();
        }
        return false;
      case pillar_chain::PillarVoteRelevancePlanStatus::kVoteBlockHashMismatch:
        LOG(log_nf_) << "Received vote's block hash " << vote->getBlockHash() << " != current pillar block hash "
                     << current_pillar_block->getHash();
        return false;
      case pillar_chain::PillarVoteRelevancePlanStatus::kUnknown:
        [[fallthrough]];
      default:
        LOG(log_wr_) << "Unable to evaluate pillar vote relevance for " << vote->getHash() << ": "
                     << pillarVoteRelevancePlanStatusString(relevance_plan.status);
        return false;
    }
  }

  return true;
}

bool PillarChainManager::validatePillarVote(const std::shared_ptr<PillarVote> vote) const {
  if (!vote) {
    LOG(log_er_) << "Unable to validate pillar vote: null vote pointer";
    return false;
  }

  const auto current_pillar_block = getCurrentPillarBlock();
  const auto validation_plan =
      validatePillarVoteWithRust(kFicusHfConfig, vote, final_chain_, current_pillar_block, pillar_votes_);
  const auto vote_period = validation_plan.period;

  if (!validation_plan.is_valid) {
    switch (validation_plan.status) {
      case pillar_chain::PillarVoteValidationPlanStatus::kDuplicate:
        LOG(log_dg_) << "Received vote " << vote->getHash() << " already saved";
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kMissingCurrentPillarBlock:
        LOG(log_nf_) << "Received vote's period " << vote_period
                     << ", no pillar block created yet. Accepting votes with "
                     << kFicusHfConfig.firstPillarBlockPeriod() + 1 << " period";
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kVotePeriodMismatch:
        if (!current_pillar_block) {
          LOG(log_nf_) << "Received vote's period " << vote_period << ", current pillar block missing";
        } else {
          LOG(log_nf_) << "Received vote's period " << vote_period << ", current pillar block period "
                       << current_pillar_block->getPeriod();
        }
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kVoteBlockHashMismatch:
        LOG(log_nf_) << "Received vote's block hash " << vote->getBlockHash() << " != current pillar block hash "
                     << current_pillar_block->getHash();
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kNotUnique:
        LOG(log_er_) << "Pillar vote " << vote->getHash() << " is not unique per period & validator";
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kNotEligible:
        LOG(log_er_) << "Validator is not eligible. Pillar vote " << vote->getHash();
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kFuturePeriod:
        LOG(log_wr_) << "Period " << vote_period << " is too far ahead of DPOS. Pillar vote " << vote->getHash();
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kSignatureInvalid:
        LOG(log_er_) << "Invalid pillar vote " << vote->getHash();
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kInspectionFailure:
      case pillar_chain::PillarVoteValidationPlanStatus::kUnknown:
        [[fallthrough]];
      default:
        LOG(log_wr_) << "Unable to validate pillar vote " << vote->getHash() << ": "
                     << pillarVoteValidationPlanStatusString(validation_plan.status);
        return false;
    }
  }

  return true;
}

uint64_t PillarChainManager::addVerifiedPillarVote(const std::shared_ptr<PillarVote>& vote) {
  const auto add_plan = planAddVerifiedPillarVoteWithRust(vote, final_chain_);
  if (!add_plan.can_insert) {
    LOG(log_er_) << "Unable to add pillar vote in Rust production path: "
                 << pillarVoteValidationPlanStatusString(add_plan.status);
    return 0;
  }

  if (!pillar_votes_->pillar_votes_period_data_initialized(add_plan.period)) {
    const auto threshold = getPillarConsensusThreshold(add_plan.period - 1);
    if (!threshold) {
      LOG(log_er_) << "Unable to get pillar consensus threshold for period " << add_plan.period - 1;
      return 0;
    }
    pillar_votes_->pillar_votes_init_period_data(add_plan.period, *threshold);
  }

  rustaxa::PillarVoteInsertOutcome insert_outcome;
  try {
    insert_outcome = pillar_votes_->pillar_votes_insert_vote(
        toBridgeVotePayload(vote, add_plan.recovered_voter, add_plan.validator_vote_count));
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to insert pillar vote " << add_plan.vote_hash << ", period " << add_plan.period << ", validator "
                 << add_plan.recovered_voter << ": " << e.what();
    return 0;
  }
  if (insert_outcome.conflict_found) {
    LOG(log_er_) << "Non-unique pillar vote " << add_plan.vote_hash << ", period " << add_plan.period << ", validator "
                 << add_plan.recovered_voter;
    return 0;
  }
  if (!insert_outcome.accepted && !insert_outcome.duplicate) {
    LOG(log_er_) << "Unable to insert pillar vote " << add_plan.vote_hash << ", period " << add_plan.period
                 << ", validator " << add_plan.recovered_voter << ", unexpected Rust insert outcome";
    return 0;
  }

  LOG(log_nf_) << "Added pillar vote " << add_plan.vote_hash << ", period " << add_plan.period << ", pillar block hash "
               << vote->getBlockHash();
  return add_plan.validator_vote_count;
}

bool PillarChainManager::addPlannedVerifiedPillarVoteForRust(const std::shared_ptr<PillarVote>& vote,
                                                            uint64_t period_threshold, uint64_t validator_vote_count,
                                                            const addr_t& recovered_voter) {
  if (!vote || period_threshold == 0 || validator_vote_count == 0) {
    LOG(log_er_) << "Unable to add planned pillar vote: missing vote, zero threshold, or zero validator vote count";
    return false;
  }

  if (!pillar_votes_->pillar_votes_period_data_initialized(vote->getPeriod())) {
    pillar_votes_->pillar_votes_init_period_data(vote->getPeriod(), period_threshold);
  }

  rustaxa::PillarVoteInsertOutcome insert_outcome;
  try {
    insert_outcome =
        pillar_votes_->pillar_votes_insert_vote(toBridgeVotePayload(vote, recovered_voter, validator_vote_count));
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to insert planned pillar vote " << vote->getHash() << " for period " << vote->getPeriod()
                 << ", validator " << recovered_voter << ": " << e.what();
    return false;
  }
  if (insert_outcome.conflict_found) {
    LOG(log_er_) << "Unable to insert planned pillar vote " << vote->getHash() << " for period " << vote->getPeriod()
                 << " in Rust sync path";
    return false;
  }
  if (!insert_outcome.accepted && !insert_outcome.duplicate) {
    LOG(log_er_) << "Unable to insert planned pillar vote " << vote->getHash() << " for period " << vote->getPeriod()
                 << " due to invalid Rust insert outcome";
    return false;
  }

  LOG(log_nf_) << "Inserted planned pillar vote " << vote->getHash() << " for block " << vote->getBlockHash()
               << ", period " << vote->getPeriod();
  return true;
}

ValidatePbftBlockPillarVotesWithRustResult PillarChainManager::validatePbftBlockPillarVotesWithRust(
    PbftPeriod required_votes_period, const std::vector<bytes>& pillar_vote_rlps,
    const std::optional<std::vector<std::shared_ptr<PillarVote>>>& live_pillar_votes) {
  if (pillar_vote_rlps.empty()) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingPillarVotes, 0, {}, 0, 0};
  }

  const auto current_pillar_block = getCurrentPillarBlock();
  if (!current_pillar_block) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingCurrentPillarBlock, 0, {}, 0, 0};
  }
  if (current_pillar_block->getPeriod() + 1 != required_votes_period) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kPillarBlockPeriodMismatch, 0, {}, 0, 0};
  }

  const auto pillar_consensus_threshold = getPillarConsensusThreshold(required_votes_period - 1);
  if (!pillar_consensus_threshold) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingThreshold, 0, {}, 0, 0};
  }

  const auto sync_plan = validateSyncPillarVotesBundleDeterministically(
      pillar_vote_rlps, required_votes_period, current_pillar_block->getHash(), *pillar_consensus_threshold,
      final_chain_);
  if (!sync_plan.valid) {
    ValidatePbftBlockPillarVotesWithRustStatus status = ValidatePbftBlockPillarVotesWithRustStatus::kPlanRejected;
    if (sync_plan.plan_status == ValidateSyncPillarVotesBundlePlanStatus::kUnknown) {
      status = ValidatePbftBlockPillarVotesWithRustStatus::kBridgeError;
    }
    return {status, toSyncPillarVotesBundlePlanStatusCode(sync_plan.plan_status), sync_plan.first_bad_vote_hash,
            sync_plan.block_weight, sync_plan.selected_weight};
  }

  std::unordered_map<vote_hash_t, std::shared_ptr<PillarVote>> vote_by_hash;
  if (!live_pillar_votes.has_value() || live_pillar_votes->empty()) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kAcceptedVoteMissing,
            toSyncPillarVotesBundlePlanStatusCode(sync_plan.plan_status), sync_plan.first_bad_vote_hash,
            sync_plan.block_weight, sync_plan.selected_weight};
  }
  vote_by_hash.reserve(live_pillar_votes->size());
  for (const auto& vote : *live_pillar_votes) {
    if (!vote) {
      return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingPillarVotes, 0, {}, 0, 0};
    }
    vote_by_hash[vote->getHash()] = vote;
  }

  for (const auto& accepted_vote : sync_plan.accepted_votes) {
    const auto& vote_hash = accepted_vote.vote_hash;
    const auto vote_it = vote_by_hash.find(vote_hash);
    if (vote_it == vote_by_hash.end()) {
      return {ValidatePbftBlockPillarVotesWithRustStatus::kAcceptedVoteMissing,
              toSyncPillarVotesBundlePlanStatusCode(sync_plan.plan_status), vote_hash, sync_plan.block_weight,
              sync_plan.selected_weight};
    }

    if (!addPlannedVerifiedPillarVoteForRust(vote_it->second, *pillar_consensus_threshold, accepted_vote.weight,
                                             accepted_vote.recovered_voter)) {
      return {ValidatePbftBlockPillarVotesWithRustStatus::kInsertFailed,
              toSyncPillarVotesBundlePlanStatusCode(sync_plan.plan_status), vote_hash, sync_plan.block_weight,
              sync_plan.selected_weight};
    }
  }

  return {ValidatePbftBlockPillarVotesWithRustStatus::kValid,
          toSyncPillarVotesBundlePlanStatusCode(sync_plan.plan_status), sync_plan.first_bad_vote_hash,
          sync_plan.block_weight, sync_plan.selected_weight};
}

std::vector<std::shared_ptr<PillarVote>> PillarChainManager::getVerifiedPillarVotes(PbftPeriod period,
                                                                                    const blk_hash_t pillar_block_hash,
                                                                                    bool above_threshold) const {
  std::vector<std::shared_ptr<PillarVote>> pillar_votes;
  try {
    pillar_votes = materializePillarVotes(
        pillar_votes_->pillar_votes_get_verified_vote_payloads(period, toBridgeHash(pillar_block_hash), above_threshold));
  } catch (const std::exception&) {
    // Fall back to persisted sidecar bytes when the in-memory votes are unavailable.
  }

  // No votes returned from memory, try db
  if (pillar_votes.empty()) {
    pillar_votes = decodePeriodPillarVotesFromRustBytes(rust_storage_->pillar_chain_storage_load_period_data(period));
  }

  return pillar_votes;
}

bool PillarChainManager::isValidPillarBlock(const std::shared_ptr<PillarBlock>& pillar_block) const {
  if (!pillar_block) {
    LOG(log_er_) << "Invalid pillar block: null block";
    return false;
  }

  const auto last_finalized_pillar_block = getLastFinalizedPillarBlock();
  try {
    const auto plan = rustaxa::plan_pillar_block_linkage(
        toBridgeLinkageFact(kFicusHfConfig, pillar_block, last_finalized_pillar_block));
    if (plan.valid) {
      return true;
    }

    if (!last_finalized_pillar_block) {
      LOG(log_er_) << "Invalid pillar block: missing last finalized pillar block, new pillar block "
                   << pillar_block->getHash() << "(" << pillar_block->getPeriod() << "), linkage status "
                   << static_cast<uint64_t>(plan.status);
    } else {
      LOG(log_er_) << "Invalid pillar block: last finalized pillar block(period): "
                   << last_finalized_pillar_block->getHash() << "(" << last_finalized_pillar_block->getPeriod()
                   << "), new pillar block: " << pillar_block->getHash() << "(" << pillar_block->getPeriod()
                   << "), parent block hash: " << pillar_block->getPreviousBlockHash() << ", expected period "
                   << plan.expected_previous_period << ", linkage status " << static_cast<uint64_t>(plan.status);
    }
    return false;
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to validate pillar block linkage in Rust for " << pillar_block->getHash() << ": "
                 << e.what();
    return false;
  }
}

std::optional<uint64_t> PillarChainManager::getPillarConsensusThreshold(PbftPeriod period) const {
  std::optional<uint64_t> threshold;

  try {
    const auto dpos_facts = collectPillarDposFacts(final_chain_, period, true, {});
    if (!finalChainFactReady(dpos_facts.total_vote_count_status) || !dpos_facts.has_total_vote_count) {
      LOG(log_er_) << "Unable to get dpos total votes count for period " << period
                   << " to calculate pillar consensus threshold: " << static_cast<std::string>(dpos_facts.error_code);
      return threshold;
    }

    // Pillar chain consensus threshold = total votes count / 2 + 1
    threshold = dpos_facts.total_vote_count / 2 + 1;
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to get dpos total votes count for period " << period
                 << " to calculate pillar consensus threshold: " << e.what();
  }

  return threshold;
}

std::vector<PillarBlock::ValidatorVoteCountChange> PillarChainManager::getOrderedValidatorsVoteCountsChanges(
    const std::vector<state_api::ValidatorVoteCount>& current_vote_counts,
    const std::vector<state_api::ValidatorVoteCount>& previous_pillar_block_vote_counts) {
  return fromBridgeVoteCountChanges(rustaxa::plan_pillar_vote_count_changes(
      toBridgeVoteCounts(current_vote_counts), toBridgeVoteCounts(previous_pillar_block_vote_counts)));
}

void PillarChainManager::setNetwork(std::weak_ptr<Network> network) { network_ = std::move(network); }

}  // namespace taraxa::pillar_chain
