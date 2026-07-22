#include "pillar_chain/pillar_chain_manager_shim.hpp"

#include <algorithm>
#include <array>
#include <cassert>
#include <exception>
#include <libff/common/profiling.hpp>
#include <mutex>
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
static constexpr uint8_t kPillarFinalizationReady = 0;
static constexpr std::size_t kMaxExternallyValidatedVoteReceipts = 4096;
static constexpr uint8_t kPillarFinalizationMissingCurrentBlock = 1;
static constexpr uint8_t kPillarFinalizationCurrentBlockHashMismatch = 2;
static constexpr uint8_t kPillarFinalizationMissingVotes = 3;
static constexpr uint8_t kPillarFinalizationAlreadyFinalized = 4;

enum class CurrentAnchorDecisionOperation : uint8_t {
  kValidateCandidate = 0,
  kSelectPreviousPeriod = 1,
  kRestartPostProcessing = 2,
};

enum class CurrentAnchorDecisionStatus : uint8_t {
  kMissingCurrentAnchor = 1,
};

enum class WeightedBundlePrepareStatus : uint8_t {
  kEmpty = 1,
  kMissingCurrentAnchor = 2,
  kCurrentPeriodMismatch = 3,
  kInspectionFailure = 4,
};

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }
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

PillarVoteValidationPlanStatus toPillarVoteValidationStatus(uint8_t status) {
  switch (status) {
    case 0:
      return PillarVoteValidationPlanStatus::kValid;
    case 1:
      return PillarVoteValidationPlanStatus::kDuplicate;
    case 2:
      return PillarVoteValidationPlanStatus::kMissingCurrentPillarBlock;
    case 3:
      return PillarVoteValidationPlanStatus::kVotePeriodMismatch;
    case 4:
      return PillarVoteValidationPlanStatus::kVoteBlockHashMismatch;
    case 5:
      return PillarVoteValidationPlanStatus::kNotUnique;
    case 6:
      return PillarVoteValidationPlanStatus::kSignatureInvalid;
    case 7:
      return PillarVoteValidationPlanStatus::kNotEligible;
    case 8:
      return PillarVoteValidationPlanStatus::kFuturePeriod;
    case 9:
      return PillarVoteValidationPlanStatus::kInspectionFailure;
    case 10:
      return PillarVoteValidationPlanStatus::kStaleAnchor;
    case 11:
      return PillarVoteValidationPlanStatus::kMissingPreparation;
    default:
      return PillarVoteValidationPlanStatus::kUnknown;
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
    case 9:
      return ValidateSyncPillarVotesBundlePlanStatus::kStaleAnchor;
    default:
      return ValidateSyncPillarVotesBundlePlanStatus::kUnknown;
  }
}

uint8_t toSyncPillarVotesBundlePlanStatusCode(ValidateSyncPillarVotesBundlePlanStatus status) {
  return static_cast<uint8_t>(status);
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

rustaxa::PillarVoteSingleAdmissionContext toSingleVoteAdmissionContext(const FicusHardforkConfig& ficus_hf_config) {
  rustaxa::PillarVoteSingleAdmissionContext context{};
  context.first_pillar_block_period = ficus_hf_config.firstPillarBlockPeriod();
  context.pillar_blocks_interval = ficus_hf_config.pillar_blocks_interval;
  return context;
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

std::vector<std::shared_ptr<PillarVote>> materializePillarVotes(
    const rust::Vec<rustaxa::PillarVoteRecord>& vote_records) {
  std::vector<std::shared_ptr<PillarVote>> votes;
  votes.reserve(vote_records.size());
  for (const auto& vote_record : vote_records) {
    votes.push_back(materializePillarVoteRecord(vote_record));
  }
  return votes;
}

rustaxa::PillarBlockLinkageRequest toBridgeLinkageRequest(const FicusHardforkConfig& ficus_hf_config,
                                                          const std::shared_ptr<PillarBlock>& pillar_block) {
  rustaxa::PillarBlockLinkageRequest request{};
  request.pillar_block_period = pillar_block->getPeriod();
  request.pillar_block_previous_hash = toBridgeHash(pillar_block->getPreviousBlockHash());
  request.first_pillar_block_period = ficus_hf_config.firstPillarBlockPeriod();
  request.pillar_blocks_interval = ficus_hf_config.pillar_blocks_interval;
  return request;
}

rustaxa::PillarBlockCreationRequest toBridgeCreationRequest(
    const FicusHardforkConfig& ficus_hf_config, PbftPeriod period,
    const std::shared_ptr<const final_chain::BlockHeader>& block_header, const h256& bridge_root,
    const h256& bridge_epoch) {
  rustaxa::PillarBlockCreationRequest request{};
  request.pillar_block_period = period;
  request.state_root = toBridgeHash(block_header->state_root);
  request.bridge_root = toBridgeHash(bridge_root);
  request.bridge_epoch = toBridgeHash(bridge_epoch);
  request.first_pillar_block_period = ficus_hf_config.firstPillarBlockPeriod();
  request.pillar_blocks_interval = ficus_hf_config.pillar_blocks_interval;
  return request;
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
    case PillarVoteValidationPlanStatus::kStaleAnchor:
      return "stale current pillar anchor";
    case PillarVoteValidationPlanStatus::kMissingPreparation:
      return "missing one-time pillar vote preparation";
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
    case ValidatePbftBlockPillarVotesWithRustStatus::kStaleAnchor:
      return "stale current pillar anchor";
  }
  return "unknown";
}

PillarVoteRelevancePlan planPillarVoteRelevance(const FicusHardforkConfig& ficus_hf_config,
                                                const std::shared_ptr<PillarVote>& vote,
                                                const rustaxa::BridgePbftService& service) {
  if (!vote) {
    return {PillarVoteRelevancePlanStatus::kUnknown, false};
  }

  rustaxa::PillarVoteRuntimeRelevanceContext context{};
  context.first_pillar_block_period = ficus_hf_config.firstPillarBlockPeriod();
  context.pillar_blocks_interval = ficus_hf_config.pillar_blocks_interval;

  try {
    const auto plan = service.pbft_service_pillar_plan_vote_relevance(toRustBytes(vote->rlp()), context);
    return {fromStatusCode(plan.status), plan.is_relevant};
  } catch (const std::exception&) {
    return {PillarVoteRelevancePlanStatus::kUnknown, false};
  }
}

PillarVoteValidationPlan validatePillarVoteWithRust(const FicusHardforkConfig& ficus_hf_config,
                                                    const std::shared_ptr<PillarVote>& vote,
                                                    const rustaxa::BridgeFinalChain& final_chain,
                                                    const rustaxa::BridgePbftService& service) {
  if (!vote) {
    return {PillarVoteValidationPlanStatus::kInspectionFailure, false, 0, {}, {}};
  }

  rustaxa::PillarVoteSingleAdmissionPreparePlan prepared{};
  try {
    prepared = service.pbft_service_pillar_validate_single_vote_with_final_chain(
        final_chain, toRustBytes(vote->rlp()), toSingleVoteAdmissionContext(ficus_hf_config));
  } catch (const std::exception&) {
    return {PillarVoteValidationPlanStatus::kUnknown, false, vote->getPeriod(), vote->getHash(), {}};
  }
  const auto status = toPillarVoteValidationStatus(prepared.status);
  return {status, status == PillarVoteValidationPlanStatus::kValid, prepared.period, fromBridgeHash(prepared.vote_hash),
          fromBridgeAddress(prepared.voter)};
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
    const rustaxa::BridgeFinalChain& final_chain, const rustaxa::BridgePbftService& service) {
  ValidateSyncPillarVotesBundleDeterministicallyResult result;
  if (required_votes_period == 0) {
    return result;
  }

  rust::Vec<rustaxa::PillarVoteRlpPayload> rlp_payloads;
  rlp_payloads.reserve(pillar_vote_rlps.size());
  for (const auto& vote_rlp : pillar_vote_rlps) {
    rustaxa::PillarVoteRlpPayload payload;
    payload.vote_rlp = toRustBytes(vote_rlp);
    rlp_payloads.push_back(std::move(payload));
  }

  try {
    const auto plan = service.pbft_service_pillar_apply_rlp_bundle_with_final_chain(
        final_chain, std::move(rlp_payloads), required_votes_period);

    const auto plan_status = toSyncPillarVotesBundlePlanStatus(plan.status);
    result.prepare_status = plan.prepare_status;
    result.missing_threshold = plan.missing_threshold;
    result.plan_status = plan_status;
    result.first_bad_vote_hash = fromBridgeHash(plan.first_bad_vote_hash);
    result.block_weight = plan.block_weight;
    result.selected_weight = plan.selected_weight;
    result.insert_failed = plan.insert_failed;
    result.insert_failed_vote_hash = fromBridgeHash(plan.insert_failed_vote_hash);

    if (plan_status != ValidateSyncPillarVotesBundlePlanStatus::kBundleValid) {
      return result;
    }

    result.valid = !plan.insert_failed;
    return result;
  } catch (const std::exception&) {
    return result;
  }
}

PillarChainManager::PillarChainManager(const FicusHardforkConfig& ficus_hf_config, std::shared_ptr<DbStorage> /*db*/,
                                       SharedPbftService pbft_service,
                                       std::shared_ptr<final_chain::FinalChain> final_chain,
                                       std::shared_ptr<KeyManager> key_manager, addr_t node_addr)
    : kFicusHfConfig(ficus_hf_config),
      pbft_service_(std::move(pbft_service)),
      network_{},
      final_chain_{std::move(final_chain)},
      key_manager_(std::move(key_manager)),
      node_addr_(node_addr),
      current_pillar_block_{},
      mutex_{} {
  LOG_OBJECTS_CREATE("PILLAR_CHAIN");
  if (!pbft_service_ || !pbft_service_->service().pbft_service_has_pillar()) {
    throw pillarVotesError("PBFT_SERVICE_PILLAR_UNAVAILABLE");
  }

  const auto bootstrap = pbft_service_->service().pbft_service_pillar_load_startup_bootstrap();

  if (const auto vote = decodePillarVoteFromRustBytes(bootstrap.own_vote_rlp); vote) {
    addVerifiedPillarVote(vote);
  }

  if (auto&& current_pillar_block_data = decodeCurrentPillarBlockDataFromRustBytes(bootstrap.current_block_data_rlp);
      current_pillar_block_data.has_value()) {
    current_pillar_block_ = std::move(current_pillar_block_data->pillar_block);
  }

  if (!bootstrap.latest_pillar_votes_period_data_rlp.empty()) {
    const auto last_finalized_pillar_block_votes =
        decodePeriodPillarVotesFromRustBytes(bootstrap.latest_pillar_votes_period_data_rlp);
    // There should always be pillar votes stored in period data for finalized pillar block
    assert(!last_finalized_pillar_block_votes.empty());
    for (const auto& pillar_vote : last_finalized_pillar_block_votes) {
      addVerifiedPillarVote(pillar_vote);
    }
  }

  pbft_service_->service().pbft_service_complete_pillar_bootstrap();
  if (!pbft_service_->service().pbft_service_pillar_ready()) {
    throw pillarVotesError("PBFT_SERVICE_PILLAR_UNAVAILABLE");
  }
}

PillarChainManager::PillarChainManager(const FicusHardforkConfig& ficus_hf_config, std::shared_ptr<DbStorage> db,
                                       std::shared_ptr<final_chain::FinalChain> final_chain,
                                       std::shared_ptr<KeyManager> key_manager, addr_t node_addr)
    : PillarChainManager(ficus_hf_config, db,
                         std::make_shared<PbftService>(
                             rustaxa::create_pillar_capable_pbft_service_for_compatibility(db->rustStorage())),
                         std::move(final_chain), std::move(key_manager), node_addr) {}

std::shared_ptr<PillarBlock> PillarChainManager::createPillarBlock(
    PbftPeriod period, const std::shared_ptr<const final_chain::BlockHeader>& block_header, const h256& bridge_root,
    const h256& bridge_epoch) {
  rustaxa::PillarBlockCreationWithVoteCountsPlan creation_plan{};
  try {
    creation_plan = pbft_service_->service().pbft_service_pillar_plan_block_creation_with_final_chain(
        final_chain_->rustFinalChain(),
        toBridgeCreationRequest(kFicusHfConfig, period, block_header, bridge_root, bridge_epoch));
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
      fromBridgeVoteCountChanges(creation_plan.vote_count_changes));

  // Check if some pillar block was not skipped
  if (!isValidPillarBlock(pillar_block)) {
    LOG(log_er_) << "Newly created pillar block " << pillar_block->getHash() << "with period "
                 << pillar_block->getPeriod() << " is invalid";
    return nullptr;
  }

  std::vector<state_api::ValidatorVoteCount> new_vote_counts;
  new_vote_counts.reserve(creation_plan.current_vote_counts.size());
  for (const auto& vote_count : creation_plan.current_vote_counts) {
    new_vote_counts.push_back({fromBridgeAddress(vote_count.address), vote_count.vote_count});
  }
  saveNewPillarBlock(pillar_block, std::move(new_vote_counts), creation_plan.anchor_generation);
  LOG(log_nf_) << "New pillar block " << pillar_block->getHash() << " with period " << pillar_block->getPeriod()
               << " created";

  return pillar_block;
}

void PillarChainManager::saveNewPillarBlock(const std::shared_ptr<PillarBlock>& pillar_block,
                                            std::vector<state_api::ValidatorVoteCount>&& new_vote_counts,
                                            uint64_t expected_anchor_generation) {
  // Operations that touch both representations always acquire the C++
  // compatibility mutex before entering the Rust runtime. Finalization uses
  // the same order and releases this mutex before invoking external effects.
  std::scoped_lock<std::shared_mutex> lock(mutex_);
  pbft_service_->service().pbft_service_pillar_apply_planned_current_block_data(
      toRustBytes(util::rlp_enc(CurrentPillarBlockDataDb{pillar_block, new_vote_counts})), expected_anchor_generation);
  current_pillar_block_ = pillar_block;
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
  pbft_service_->service().pbft_service_pillar_apply_own_vote(toRustBytes(util::rlp_enc(vote)));

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

std::vector<std::shared_ptr<PillarVote>> PillarChainManager::finalizePillarBlock(const blk_hash_t&) {
  throw std::runtime_error(
      "Direct pillar finalization is unsupported in Rust-mode compatibility path. "
      "Use finalizePillarBlockForPbftPreflight() + pbft manager acknowledge path.");
}

PillarChainManager::FinalizePillarBlockPreflightResult PillarChainManager::finalizePillarBlockForPbftPreflight(
    const blk_hash_t& pillar_block_hash) {
  FinalizePillarBlockPreflightResult result;
  rustaxa::PillarBlockFinalizationRequest finalization_request{};
  finalization_request.requested_pillar_block_hash = toBridgeHash(pillar_block_hash);

  rustaxa::PillarBlockFinalizationPrepareResult preflight_result{};
  std::unique_lock<std::shared_mutex> compatibility_lock(mutex_);
  try {
    preflight_result =
        pbft_service_->service().pbft_service_pillar_prepare_finalized_block_for_pbft(std::move(finalization_request));
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to prepare pillar block finalization in Rust for " << pillar_block_hash << ": " << e.what();
    return result;
  }

  result.success = preflight_result.success;
  result.should_request_votes = preflight_result.should_request_votes;
  result.has_request_votes_period = preflight_result.has_request_votes_period;
  result.request_votes_period = preflight_result.request_votes_period;
  result.should_emit = preflight_result.should_emit;
  result.pillar_vote_count = preflight_result.selected_vote_count;
  result.prepared_pillar_block_period = preflight_result.prepared_pillar_block_period;
  result.prepared_pillar_block_rlp = fromRustBytes(preflight_result.prepared_pillar_block_rlp);
  result.has_prepared_pillar_block = preflight_result.has_prepared_pillar_block;
  result.preparation_anchor_generation = preflight_result.preparation_anchor_generation;
  result.preparation_token = preflight_result.preparation_token;

  if (preflight_result.success && !preflight_result.votes.empty()) {
    try {
      result.pillar_votes = materializePillarVotes(preflight_result.votes);
    } catch (const std::exception& e) {
      LOG(log_er_) << "Unable to materialize verified pillar votes for finalized block " << pillar_block_hash << ": "
                   << e.what();
      result.success = false;
      result.pillar_votes.clear();
      return result;
    }
  }

  switch (preflight_result.status) {
    case kPillarFinalizationReady:
      break;
    case kPillarFinalizationMissingCurrentBlock:
      LOG(log_er_) << "Cannot prepare pillar finalization for block " << pillar_block_hash
                   << ". Empty current pillar block";
      return result;
    case kPillarFinalizationCurrentBlockHashMismatch:
      LOG(log_er_) << "Cannot prepare pillar finalization for block " << pillar_block_hash << ". Requested block hash "
                   << pillar_block_hash << " != current pillar block hash "
                   << fromBridgeBlockHash(preflight_result.current_hash);
      return result;
    case kPillarFinalizationMissingVotes:
      LOG(log_er_) << "Cannot prepare pillar finalization for block " << pillar_block_hash
                   << ". Not enough pillar votes for pillar block. Request it";
      {
        const auto should_request_votes =
            preflight_result.should_request_votes && preflight_result.has_request_votes_period;
        const auto request_votes_period = preflight_result.request_votes_period;
        if (should_request_votes) {
          compatibility_lock.unlock();
          if (auto net = network_.lock()) {
            net->requestPillarBlockVotesBundle(request_votes_period, pillar_block_hash);
          }
        }
      }
      return result;
    case kPillarFinalizationAlreadyFinalized:
      LOG(log_nf_) << "Pillar block already " << pillar_block_hash << " already finalized";
      return result;
    default:
      LOG(log_er_) << "Unable to prepare pillar finalization for block " << pillar_block_hash
                   << ". Unknown Rust status " << static_cast<uint64_t>(preflight_result.status);
      result.success = false;
      return result;
  }

  if (result.should_emit) {
    LOG(log_nf_) << "Pillar block finalization prepared for block " << pillar_block_hash << " with period "
                 << preflight_result.current_period;
  }

  return result;
}

bool PillarChainManager::acknowledgePillarBlockForPbft(uint64_t anchor_generation, uint64_t preparation_token,
                                                       const std::vector<std::shared_ptr<PillarVote>>& pillar_votes) {
  // Serialize acknowledgment with current/latest compatibility reads. Keep
  // the lock through materialization and identity verification, but never
  // invoke the compatibility emitter while holding it.
  std::unique_lock<std::shared_mutex> compatibility_lock(mutex_);
  rustaxa::PillarBlockFinalizationAcknowledgeRequest request{};
  request.anchor_generation = anchor_generation;
  request.preparation_token = preparation_token;

  rustaxa::PillarBlockFinalizationAcknowledgeResult ack_result{};
  try {
    ack_result = pbft_service_->service().pbft_service_pillar_ack_finalize_block_for_pbft(std::move(request));
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to acknowledge pillar finalization for anchor generation " << anchor_generation
                 << ", token " << preparation_token << ": " << e.what();
    return false;
  }

  if (!ack_result.should_emit) {
    LOG(log_dg_) << "Rust anchor ack for generation " << anchor_generation << ", token " << preparation_token
                 << " completed without event emission";
    return true;
  }

  std::shared_ptr<PillarBlock> latest_pillar_block;
  try {
    latest_pillar_block =
        decodePillarBlockFromRustBytes(pbft_service_->service().pbft_service_pillar_latest_finalized_block_rlp());
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to materialize latest finalized pillar block from Rust runtime: " << e.what();
    return false;
  }
  if (!latest_pillar_block) {
    LOG(log_er_) << "Unable to emit PillarBlockData in compatibility layer: latest finalized pillar block unavailable";
    return false;
  }

  if (latest_pillar_block->getPeriod() != ack_result.latest_finalized_period ||
      latest_pillar_block->getHash() != fromBridgeBlockHash(ack_result.latest_finalized_hash)) {
    LOG(log_er_) << "Compatibility pillar finalization identity changed after Rust ack: expected period "
                 << ack_result.latest_finalized_period << " hash "
                 << fromBridgeBlockHash(ack_result.latest_finalized_hash) << ", runtime reports "
                 << latest_pillar_block->getPeriod() << "/" << latest_pillar_block->getHash();
    return false;
  }

  compatibility_lock.unlock();
  pillar_block_finalized_emitter_.emit(PillarBlockData{latest_pillar_block, pillar_votes});
  return true;
}

bool PillarChainManager::isPillarBlockLatestFinalized(const blk_hash_t& block_hash) const {
  const auto latest = getLastFinalizedPillarBlock();
  return latest && latest->getHash() == block_hash;
}

std::shared_ptr<PillarBlock> PillarChainManager::getLastFinalizedPillarBlock() const {
  // Preserve the compatibility publication boundary: finalization holds this
  // mutex until the Rust snapshot and the C++ current-block mirror agree.
  std::shared_lock<std::shared_mutex> lock(mutex_);
  try {
    return decodePillarBlockFromRustBytes(pbft_service_->service().pbft_service_pillar_latest_finalized_block_rlp());
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to materialize latest finalized pillar block from Rust runtime: " << e.what();
    return nullptr;
  }
}

std::shared_ptr<PillarBlock> PillarChainManager::getCurrentPillarBlock() const {
  std::shared_lock<std::shared_mutex> lock(mutex_);
  return current_pillar_block_;
}

PillarChainManager::PbftBlockPillarAnchorValidation PillarChainManager::validatePbftBlockPillarAnchor(
    const blk_hash_t& pbft_block_hash, PbftPeriod pbft_period,
    const std::optional<blk_hash_t>& pillar_block_hash) const {
  PbftBlockPillarAnchorValidation result;
  rustaxa::PillarCurrentAnchorDecisionRequest request{};
  request.operation = static_cast<uint8_t>(CurrentAnchorDecisionOperation::kValidateCandidate);
  request.has_candidate_hash = pillar_block_hash.has_value();
  if (pillar_block_hash) {
    request.candidate_hash = toBridgeHash(*pillar_block_hash);
  }

  try {
    const auto plan = pbft_service_->service().pbft_service_pillar_plan_current_anchor_decision(request);
    result.valid = plan.selected;
    result.missing_current_anchor =
        plan.status == static_cast<uint8_t>(CurrentAnchorDecisionStatus::kMissingCurrentAnchor);
    if (plan.has_current_anchor) {
      result.current_pillar_period = plan.current_period;
      result.current_pillar_hash = fromBridgeBlockHash(plan.current_hash);
    }
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to validate PBFT block " << pbft_block_hash << ", period " << pbft_period
                 << " against the Rust pillar anchor: " << e.what();
    return result;
  }

  if (result.missing_current_anchor) {
    LOG(log_er_) << "Unable to validate PBFT block " << pbft_block_hash << ", period " << pbft_period
                 << ". No current pillar block present in Rust runtime";
  } else if (!result.valid) {
    LOG(log_er_) << "PBFT block " << pbft_block_hash << " with period " << pbft_period << " contains pillar block hash "
                 << pillar_block_hash.value_or(kNullBlockHash) << ", which is different than the local current pillar "
                 << "block " << result.current_pillar_hash << " with period " << result.current_pillar_period;
  }
  return result;
}

PillarChainManager::PbftExtraDataPillarAnchor PillarChainManager::pbftExtraDataPillarAnchor(
    PbftPeriod pbft_period) const {
  PbftExtraDataPillarAnchor result;
  rustaxa::PillarCurrentAnchorDecisionRequest request{};
  request.operation = static_cast<uint8_t>(CurrentAnchorDecisionOperation::kSelectPreviousPeriod);
  request.pbft_period = pbft_period;
  try {
    const auto plan = pbft_service_->service().pbft_service_pillar_plan_current_anchor_decision(request);
    result.available = plan.selected;
    if (plan.has_current_anchor) {
      result.current_pillar_period = plan.current_period;
      result.pillar_block_hash = fromBridgeBlockHash(plan.current_hash);
    }
    if (!result.available) {
      LOG(log_er_) << "Unable to select Rust pillar anchor for pbft period " << pbft_period << ", current period "
                   << result.current_pillar_period << ", status " << static_cast<uint64_t>(plan.status);
    }
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to select Rust pillar anchor for pbft period " << pbft_period << ": " << e.what();
    return result;
  }
  return result;
}

PillarChainManager::LocalPillarVoteAnchor PillarChainManager::localPillarVoteAnchorForPbftPeriod(
    PbftPeriod pbft_period) const {
  LocalPillarVoteAnchor result;
  rustaxa::PillarCurrentAnchorDecisionRequest request{};
  request.operation = static_cast<uint8_t>(CurrentAnchorDecisionOperation::kSelectPreviousPeriod);
  request.pbft_period = pbft_period;
  try {
    const auto plan = pbft_service_->service().pbft_service_pillar_plan_current_anchor_decision(request);
    result.should_vote = plan.selected;
    if (plan.has_current_anchor) {
      result.current_pillar_period = plan.current_period;
      result.pillar_block_hash = fromBridgeBlockHash(plan.current_hash);
    }
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to select Rust pillar-vote anchor for pbft period " << pbft_period << ": " << e.what();
    return result;
  }
  return result;
}

PillarChainManager::RestartPillarPostProcessingDecision PillarChainManager::restartPillarPostProcessingDecision(
    PbftPeriod pbft_period) const {
  RestartPillarPostProcessingDecision decision;
  rustaxa::PillarCurrentAnchorDecisionRequest request{};
  request.operation = static_cast<uint8_t>(CurrentAnchorDecisionOperation::kRestartPostProcessing);
  request.pbft_period = pbft_period;
  request.pillar_blocks_interval = kFicusHfConfig.pillar_blocks_interval;
  try {
    const auto plan = pbft_service_->service().pbft_service_pillar_plan_current_anchor_decision(request);
    decision.should_process = plan.selected;
    if (plan.has_current_anchor) {
      decision.current_pillar_period = plan.current_period;
    }
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to plan Rust pillar restart post-processing for pbft period " << pbft_period << ": "
                 << e.what();
    return decision;
  }
  if (decision.should_process) {
    LOG(log_er_) << "Pillar block was not processed before restart, current period: " << pbft_period
                 << ", current pillar block period: " << decision.current_pillar_period;
  }
  return decision;
}

bool PillarChainManager::isRelevantPillarVote(const std::shared_ptr<PillarVote> vote) const {
  const auto relevance_plan = planPillarVoteRelevance(kFicusHfConfig, vote, pbft_service_->service());

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
        LOG(log_nf_) << "Received vote's period " << vote->getPeriod()
                     << " does not match the Rust current pillar anchor";
        return false;
      case pillar_chain::PillarVoteRelevancePlanStatus::kVoteBlockHashMismatch:
        LOG(log_nf_) << "Received vote's block hash " << vote->getBlockHash()
                     << " does not match the Rust current pillar anchor";
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

  const auto validation_plan =
      validatePillarVoteWithRust(kFicusHfConfig, vote, final_chain_->rustFinalChain(), pbft_service_->service());
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
        LOG(log_nf_) << "Received vote's period " << vote_period << " does not match the Rust current pillar anchor";
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kVoteBlockHashMismatch:
        LOG(log_nf_) << "Received vote's block hash " << vote->getBlockHash()
                     << " does not match the Rust current pillar anchor";
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
      case pillar_chain::PillarVoteValidationPlanStatus::kStaleAnchor:
        LOG(log_wr_) << "Pillar anchor changed while validating vote " << vote->getHash();
        return false;
      case pillar_chain::PillarVoteValidationPlanStatus::kMissingPreparation:
        LOG(log_wr_) << "Pillar vote preparation was consumed before validation completed " << vote->getHash();
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

  {
    std::unique_lock lock(mutex_);
    const auto vote_hash = validation_plan.vote_hash;
    if (!externally_validated_vote_receipts_.contains(vote_hash)) {
      if (externally_validated_vote_receipts_.size() >= kMaxExternallyValidatedVoteReceipts) {
        LOG(log_wr_) << "Too many externally validated pillar votes awaiting insertion";
        return false;
      }
      externally_validated_vote_receipts_.insert(vote_hash);
    }
  }

  return true;
}

uint64_t PillarChainManager::addVerifiedPillarVote(const std::shared_ptr<PillarVote>& vote) {
  if (!vote) {
    return 0;
  }
  bool externally_validated = false;
  {
    std::shared_lock lock(mutex_);
    externally_validated = externally_validated_vote_receipts_.contains(vote->getHash());
  }
  rustaxa::PillarVoteSingleAdmissionWithFinalChainPlan insert_outcome;
  try {
    insert_outcome = pbft_service_->service().pbft_service_pillar_apply_single_vote_with_final_chain(
        final_chain_->rustFinalChain(), toRustBytes(vote->rlp()), toSingleVoteAdmissionContext(kFicusHfConfig),
        !externally_validated);
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to insert pillar vote " << vote->getHash() << ", period " << vote->getPeriod() << ": "
                 << e.what();
    return 0;
  }
  const auto vote_hash = fromBridgeHash(insert_outcome.vote_hash);
  const auto recovered_voter = fromBridgeAddress(insert_outcome.voter);
  if (toPillarVoteValidationStatus(insert_outcome.status) != PillarVoteValidationPlanStatus::kValid) {
    LOG(log_er_) << "Unable to insert pillar vote " << vote_hash << ", period " << insert_outcome.period
                 << ", validator " << recovered_voter << ": "
                 << pillarVoteValidationPlanStatusString(toPillarVoteValidationStatus(insert_outcome.status));
    return 0;
  }
  if (insert_outcome.conflict_found) {
    LOG(log_er_) << "Non-unique pillar vote " << vote_hash << ", period " << insert_outcome.period << ", validator "
                 << recovered_voter;
    return 0;
  }
  if (!insert_outcome.accepted && !insert_outcome.duplicate) {
    LOG(log_er_) << "Unable to insert pillar vote " << vote_hash << ", period " << insert_outcome.period
                 << ", validator " << recovered_voter << ", unexpected Rust insert outcome";
    return 0;
  }

  LOG(log_nf_) << "Added pillar vote " << vote_hash << ", period " << insert_outcome.period << ", pillar block hash "
               << vote->getBlockHash();
  if (externally_validated) {
    // Consume the routing receipt only after successful checked admission.
    // Failed or racing retries therefore remain on the checked path and can
    // never fall through to trusted local/restart preparation.
    std::unique_lock lock(mutex_);
    // One successful insertion makes every duplicate delivery safe: Rust now
    // owns the vote and subsequent apply handles the exact duplicate idempotently.
    externally_validated_vote_receipts_.erase(vote->getHash());
  }
  return insert_outcome.validator_vote_count;
}

ValidatePbftBlockPillarVotesWithRustResult PillarChainManager::validatePbftBlockPillarVotesWithRust(
    PbftPeriod required_votes_period, const std::vector<bytes>& pillar_vote_rlps) {
  if (pillar_vote_rlps.empty()) {
    return {ValidatePbftBlockPillarVotesWithRustStatus::kMissingPillarVotes, 0, {}, 0, 0};
  }

  const auto sync_plan = validateSyncPillarVotesBundleDeterministically(
      pillar_vote_rlps, required_votes_period, final_chain_->rustFinalChain(), pbft_service_->service());
  if (!sync_plan.valid) {
    auto status = ValidatePbftBlockPillarVotesWithRustStatus::kPlanRejected;
    if (sync_plan.prepare_status == static_cast<uint8_t>(WeightedBundlePrepareStatus::kMissingCurrentAnchor)) {
      status = ValidatePbftBlockPillarVotesWithRustStatus::kMissingCurrentPillarBlock;
    } else if (sync_plan.prepare_status == static_cast<uint8_t>(WeightedBundlePrepareStatus::kCurrentPeriodMismatch)) {
      status = ValidatePbftBlockPillarVotesWithRustStatus::kPillarBlockPeriodMismatch;
    } else if (sync_plan.missing_threshold) {
      status = ValidatePbftBlockPillarVotesWithRustStatus::kMissingThreshold;
    } else if (sync_plan.insert_failed) {
      status = ValidatePbftBlockPillarVotesWithRustStatus::kInsertFailed;
    } else if (sync_plan.plan_status == ValidateSyncPillarVotesBundlePlanStatus::kStaleAnchor) {
      status = ValidatePbftBlockPillarVotesWithRustStatus::kStaleAnchor;
    } else if (sync_plan.plan_status == ValidateSyncPillarVotesBundlePlanStatus::kUnknown) {
      status = ValidatePbftBlockPillarVotesWithRustStatus::kBridgeError;
    }
    const auto bad_vote_hash =
        sync_plan.insert_failed ? sync_plan.insert_failed_vote_hash : sync_plan.first_bad_vote_hash;
    return {status, toSyncPillarVotesBundlePlanStatusCode(sync_plan.plan_status), bad_vote_hash, sync_plan.block_weight,
            sync_plan.selected_weight};
  }

  return {ValidatePbftBlockPillarVotesWithRustStatus::kValid,
          toSyncPillarVotesBundlePlanStatusCode(sync_plan.plan_status), sync_plan.first_bad_vote_hash,
          sync_plan.block_weight, sync_plan.selected_weight};
}

std::vector<std::shared_ptr<PillarVote>> PillarChainManager::getVerifiedPillarVotes(PbftPeriod period,
                                                                                    const blk_hash_t pillar_block_hash,
                                                                                    bool above_threshold) const {
  try {
    return materializePillarVotes(pbft_service_->service().pbft_service_pillar_get_verified_vote_payloads(
        period, toBridgeHash(pillar_block_hash), above_threshold));
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to load verified pillar votes for period " << period << ", block " << pillar_block_hash
                 << ": " << e.what();
  }

  return {};
}

std::vector<PillarChainManager::PillarVoteNetworkBundleChunk> PillarChainManager::buildVerifiedPillarVoteNetworkBundles(
    PbftPeriod period, const blk_hash_t& pillar_block_hash, size_t max_votes_per_bundle) const {
  std::vector<PillarVoteNetworkBundleChunk> chunks;
  try {
    const auto lookup = pbft_service_->service().pbft_service_pillar_build_verified_vote_network_bundles(
        period, toBridgeHash(pillar_block_hash), max_votes_per_bundle);
    chunks.reserve(lookup.chunks.size());
    for (const auto& bridge_chunk : lookup.chunks) {
      PillarVoteNetworkBundleChunk chunk;
      chunk.optimized_bundle_rlp = fromRustBytes(bridge_chunk.votes_bundle_rlp);
      chunk.vote_hashes.reserve(bridge_chunk.vote_hashes.size());
      for (const auto& bridge_hash : bridge_chunk.vote_hashes) {
        chunk.vote_hashes.emplace_back(fromBridgeHash(bridge_hash.hash));
      }
      chunks.emplace_back(std::move(chunk));
    }
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to build pillar vote network bundle chunks for period " << period << ", block "
                 << pillar_block_hash << ": " << e.what();
  }
  return chunks;
}

bool PillarChainManager::isValidPillarBlock(const std::shared_ptr<PillarBlock>& pillar_block) const {
  if (!pillar_block) {
    LOG(log_er_) << "Invalid pillar block: null block";
    return false;
  }

  try {
    const auto plan = pbft_service_->service().pbft_service_pillar_plan_block_linkage(
        toBridgeLinkageRequest(kFicusHfConfig, pillar_block));
    if (plan.valid) {
      return true;
    }

    const auto last_finalized_pillar_block = getLastFinalizedPillarBlock();
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
    const auto lookup = pbft_service_->service().pbft_service_pillar_consensus_threshold_with_final_chain(
        final_chain_->rustFinalChain(), period);
    if (!lookup.available) {
      LOG(log_er_) << "Unable to get dpos total votes count for period " << period
                   << " to calculate pillar consensus threshold: " << static_cast<std::string>(lookup.error_code);
      return threshold;
    }
    threshold = lookup.threshold;
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to get dpos total votes count for period " << period
                 << " to calculate pillar consensus threshold: " << e.what();
  }

  return threshold;
}

void PillarChainManager::setNetwork(std::weak_ptr<Network> network) { network_ = std::move(network); }

}  // namespace taraxa::pillar_chain
