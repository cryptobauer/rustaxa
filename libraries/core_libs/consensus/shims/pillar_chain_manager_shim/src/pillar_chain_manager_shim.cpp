#include "pillar_chain/pillar_chain_manager_shim.hpp"

#include <algorithm>
#include <array>
#include <cassert>
#include <exception>
#include <libff/common/profiling.hpp>

#include "config/hardfork.hpp"
#include "final_chain/final_chain.hpp"
#include "key_manager/key_manager.hpp"
#include "network/network.hpp"
#include "pillar_chain/pillar_block.hpp"
#include "pillar_chain/pillar_votes.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"
#include "vote/pillar_vote.hpp"
#include "vote/votes_bundle_rlp.hpp"

namespace taraxa::pillar_chain {
namespace {
static constexpr uint16_t PILLAR_VOTES_POS_IN_PERIOD_DATA = 4;

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }
std::array<uint8_t, 20> toBridgeAddress(const addr_t& address) { return address.asArray(); }
addr_t fromBridgeAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

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

std::vector<PillarBlock::ValidatorVoteCountChange> fromBridgeVoteCountChanges(
    const rust::Vec<rustaxa::PillarValidatorVoteCountChange>& changes) {
  std::vector<PillarBlock::ValidatorVoteCountChange> out;
  out.reserve(changes.size());
  for (const auto& change : changes) {
    out.emplace_back(fromBridgeAddress(change.address), change.vote_count_change);
  }
  return out;
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

PillarChainManager::PillarChainManager(const FicusHardforkConfig& ficus_hf_config, std::shared_ptr<DbStorage> db,
                                       std::shared_ptr<final_chain::FinalChain> final_chain,
                                       std::shared_ptr<KeyManager> key_manager, addr_t node_addr)
    : kFicusHfConfig(ficus_hf_config),
      storage_owner_(std::move(db)),
      rust_storage_(&storage_owner_->rustStorage()),
      network_{},
      final_chain_{std::move(final_chain)},
      key_manager_(std::move(key_manager)),
      node_addr_(node_addr),
      last_finalized_pillar_block_{},
      current_pillar_block_{},
      current_pillar_block_vote_counts_{},
      pillar_votes_{},
      mutex_{} {
  LOG_OBJECTS_CREATE("PILLAR_CHAIN");

  if (const auto vote = decodePillarVoteFromRustBytes(rustaxa::load_pillar_own_vote_storage(*rust_storage_)); vote) {
    addVerifiedPillarVote(vote);
  }

  if (auto&& current_pillar_block_data =
          decodeCurrentPillarBlockDataFromRustBytes(rustaxa::load_pillar_current_block_data_storage(*rust_storage_));
      current_pillar_block_data.has_value()) {
    current_pillar_block_ = std::move(current_pillar_block_data->pillar_block);
    current_pillar_block_vote_counts_ = std::move(current_pillar_block_data->vote_counts);
  }

  if (auto&& latest_pillar_block =
          decodePillarBlockFromRustBytes(rustaxa::load_latest_pillar_block_storage(*rust_storage_));
      latest_pillar_block) {
    last_finalized_pillar_block_ = std::move(latest_pillar_block);

    const auto last_finalized_pillar_block_votes = decodePeriodPillarVotesFromRustBytes(
        rustaxa::load_pillar_period_data_storage(*rust_storage_, last_finalized_pillar_block_->getPeriod() + 1));
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
  blk_hash_t previous_pillar_block_hash{};  // null block hash
  auto new_vote_counts = final_chain_->dposValidatorsEligibleVoteCounts(period);
  std::vector<PillarBlock::ValidatorVoteCountChange> votes_count_changes;

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
    const auto last_finalized_pillar_block = getLastFinalizedPillarBlock();
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

    previous_pillar_block_hash = last_finalized_pillar_block->getHash();

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

  const auto pillar_block = std::make_shared<PillarBlock>(period, block_header->state_root, previous_pillar_block_hash,
                                                          bridge_root, bridge_epoch, std::move(votes_count_changes));

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
  rustaxa::apply_pillar_current_block_data_storage(
      *rust_storage_, toRustBytes(util::rlp_enc(CurrentPillarBlockDataDb{pillar_block, new_vote_counts})));
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
  rustaxa::apply_pillar_own_vote_storage(*rust_storage_, toRustBytes(util::rlp_enc(vote)));

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
  // Compare provided pillar block hash to the current pillar block
  const auto current_pillar_block = getCurrentPillarBlock();
  if (!current_pillar_block) {
    // This should never happen
    LOG(log_er_) << "Cannot finalize pillar block " << pillar_block_hash << ". Empty current pillar block";
    return {};
  }

  if (current_pillar_block->getHash() != pillar_block_hash) {
    // This should never happen
    LOG(log_er_) << "Cannot finalize pillar block " << pillar_block_hash << ". Provided pillar block hash "
                 << pillar_block_hash << " != current pillar block hash " << current_pillar_block->getHash();
    return {};
  }

  auto pillar_votes =
      getVerifiedPillarVotes(current_pillar_block->getPeriod() + 1, pillar_block_hash, true /* above_threshold */);
  if (pillar_votes.empty()) {
    LOG(log_er_) << "Cannot finalize pillar block " << pillar_block_hash
                 << ". Not enough pillar votes for pillar block. Request it";
    if (auto net = network_.lock()) {
      net->requestPillarBlockVotesBundle(current_pillar_block->getPeriod() + 1, pillar_block_hash);
    }

    return {};
  }

  if (isPillarBlockLatestFinalized(pillar_block_hash)) {
    // This should never happen
    LOG(log_er_) << "Pillar block already " << pillar_block_hash << " already finalized";
    return pillar_votes;
  }

  rustaxa::apply_finalized_pillar_block_storage(*rust_storage_, current_pillar_block->getPeriod(),
                                                toRustBytes(current_pillar_block->getRlp()));
  LOG(log_nf_) << "Pillar block " << pillar_block_hash << " with period " << current_pillar_block->getPeriod()
               << " finalized";

  {
    std::scoped_lock<std::shared_mutex> lock(mutex_);
    last_finalized_pillar_block_ = current_pillar_block;

    // Erase votes that are no longer needed
    pillar_votes_.eraseVotes(last_finalized_pillar_block_->getPeriod() + 1);
  }
  pillar_block_finalized_emitter_.emit(PillarBlockData{current_pillar_block, pillar_votes});

  return pillar_votes;
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

bool PillarChainManager::isRelevantPillarVote(const std::shared_ptr<PillarVote> vote) const {
  const auto vote_exists = pillar_votes_.voteExists(vote);
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

  if (!pillar_votes_.periodDataInitialized(add_plan.period)) {
    const auto threshold = getPillarConsensusThreshold(add_plan.period - 1);
    if (!threshold) {
      LOG(log_er_) << "Unable to get pillar consensus threshold for period " << add_plan.period - 1;
      return 0;
    }
    pillar_votes_.initializePeriodData(add_plan.period, *threshold);
  }

  if (!pillar_votes_.addVerifiedVoteWithRecoveredVoter(vote, add_plan.validator_vote_count, add_plan.recovered_voter)) {
    LOG(log_er_) << "Non-unique pillar vote " << add_plan.vote_hash << ", period " << add_plan.period << ", validator "
                 << add_plan.recovered_voter;
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

  if (!pillar_votes_.periodDataInitialized(vote->getPeriod())) {
    pillar_votes_.initializePeriodData(vote->getPeriod(), period_threshold);
  }

  if (!pillar_votes_.addVerifiedVoteWithRecoveredVoter(vote, validator_vote_count, recovered_voter)) {
    LOG(log_er_) << "Unable to insert planned pillar vote " << vote->getHash() << " for period " << vote->getPeriod()
                 << " in Rust sync path";
    return false;
  }

  LOG(log_nf_) << "Inserted planned pillar vote " << vote->getHash() << " for block " << vote->getBlockHash()
               << ", period " << vote->getPeriod();
  return true;
}

std::vector<std::shared_ptr<PillarVote>> PillarChainManager::getVerifiedPillarVotes(PbftPeriod period,
                                                                                    const blk_hash_t pillar_block_hash,
                                                                                    bool above_threshold) const {
  auto pillar_votes = pillar_votes_.getVerifiedVotes(period, pillar_block_hash, above_threshold);

  // No votes returned from memory, try db
  if (pillar_votes.empty()) {
    pillar_votes =
        decodePeriodPillarVotesFromRustBytes(rustaxa::load_pillar_period_data_storage(*rust_storage_, period));
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
    // Pillar chain consensus threshold = total votes count / 2 + 1
    threshold = final_chain_->dposEligibleTotalVoteCount(period) / 2 + 1;
  } catch (state_api::ErrFutureBlock& e) {
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
