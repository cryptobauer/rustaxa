#include "pillar_chain/pillar_chain_manager_shim.hpp"

#include <algorithm>
#include <array>
#include <cassert>
#include <exception>
#include <libff/common/profiling.hpp>
#include <mutex>
#include <string_view>
#include <unordered_map>

#include "config/hardfork.hpp"
#include "final_chain/final_chain.hpp"
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
static constexpr uint8_t kPillarFinalizationMissingCurrentBlock = 1;
static constexpr uint8_t kPillarFinalizationCurrentBlockHashMismatch = 2;
static constexpr uint8_t kPillarFinalizationMissingVotes = 3;
static constexpr uint8_t kPillarFinalizationAlreadyFinalized = 4;

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

PillarVoteRelevancePlan planPillarVoteRelevance(const FicusHardforkConfig& ficus_hf_config,
                                                const std::shared_ptr<PillarVote>& vote,
                                                const rustaxa::BridgeConsensusApplication& service) {
  if (!vote) {
    return {PillarVoteRelevancePlanStatus::kUnknown, false};
  }

  const auto context = toSingleVoteAdmissionContext(ficus_hf_config);

  try {
    const auto plan = service.pbft_service_pillar_plan_vote_relevance(toRustBytes(vote->rlp()), context);
    return {fromStatusCode(plan.status), plan.is_relevant};
  } catch (const std::exception&) {
    return {PillarVoteRelevancePlanStatus::kUnknown, false};
  }
}

bool isUnavailableDposSnapshotError(const std::exception& error) {
  const std::string_view message(error.what());
  const auto missing_snapshot = message.find("FinalChain DPoS snapshot") != std::string_view::npos &&
                                message.find("is not implemented") != std::string_view::npos;
  return missing_snapshot || message.find("Future block") != std::string_view::npos;
}

PillarVoteValidationPlan validatePillarVoteWithRust(const FicusHardforkConfig& ficus_hf_config,
                                                    const std::shared_ptr<PillarVote>& vote,
                                                    const rustaxa::BridgeConsensusApplication& service,
                                                    const final_chain::FinalChain& final_chain) {
  if (!vote) {
    return {PillarVoteValidationPlanStatus::kInspectionFailure, false, 0, {}, {}};
  }

  rustaxa::PillarVoteSingleAdmissionPreparePlan prepared{};
  try {
    prepared = service.pbft_service_pillar_prepare_single_vote_external_facts(
        toRustBytes(vote->rlp()), toSingleVoteAdmissionContext(ficus_hf_config), false);
  } catch (const std::exception&) {
    return {PillarVoteValidationPlanStatus::kUnknown, false, vote->getPeriod(), vote->getHash(), {}};
  }
  if (!prepared.can_query_dpos || prepared.period == 0) {
    const auto status = toPillarVoteValidationStatus(prepared.status);
    return {status, false, prepared.period, fromBridgeHash(prepared.vote_hash), fromBridgeAddress(prepared.voter)};
  }
  try {
    const auto validator_vote_count =
        final_chain.dposEligibleVoteCount(prepared.period - 1, fromBridgeAddress(prepared.voter));
    const auto validated = service.pbft_service_pillar_validate_prepared_single_vote_external_facts(
        std::move(prepared), validator_vote_count);
    const auto status = toPillarVoteValidationStatus(validated.status);
    return {status, status == PillarVoteValidationPlanStatus::kValid, validated.period,
            fromBridgeHash(validated.vote_hash), fromBridgeAddress(validated.voter)};
  } catch (const std::exception& error) {
    if (isUnavailableDposSnapshotError(error)) {
      return {PillarVoteValidationPlanStatus::kFuturePeriod, false, vote->getPeriod(), vote->getHash(), {}};
    }
    return {PillarVoteValidationPlanStatus::kUnknown, false, vote->getPeriod(), vote->getHash(), {}};
  }
}

PillarChainManager::PillarChainManager(const FicusHardforkConfig& ficus_hf_config, std::shared_ptr<DbStorage> /*db*/,
                                       SharedConsensusApplication pbft_service,
                                       std::shared_ptr<final_chain::FinalChain> final_chain, addr_t node_addr)
    : kFicusHfConfig(ficus_hf_config),
      pbft_service_(std::move(pbft_service)),
      network_{},
      final_chain_{std::move(final_chain)},
      node_addr_(node_addr),
      mutex_{} {
  LOG_OBJECTS_CREATE("PILLAR_CHAIN");
  if (!pbft_service_) {
    throw pillarVotesError("PBFT_SERVICE_PILLAR_UNAVAILABLE");
  }

  const auto bootstrap = pbft_service_->service().pbft_service_pillar_load_startup_bootstrap();

  if (const auto vote = decodePillarVoteFromRustBytes(bootstrap.own_vote_rlp); vote) {
    admitPillarVoteImpl(vote, true);
  }

  if (!bootstrap.latest_pillar_votes_period_data_rlp.empty()) {
    const auto last_finalized_pillar_block_votes =
        decodePeriodPillarVotesFromRustBytes(bootstrap.latest_pillar_votes_period_data_rlp);
    // There should always be pillar votes stored in period data for finalized pillar block
    assert(!last_finalized_pillar_block_votes.empty());
    for (const auto& pillar_vote : last_finalized_pillar_block_votes) {
      admitPillarVoteImpl(pillar_vote, true);
    }
  }

  pbft_service_->service().pbft_service_complete_pillar_bootstrap();
  if (!pbft_service_->service().pbft_service_pillar_ready()) {
    throw pillarVotesError("PBFT_SERVICE_PILLAR_UNAVAILABLE");
  }
}

std::shared_ptr<PillarBlock> PillarChainManager::createPillarBlock(
    PbftPeriod period, const std::shared_ptr<const final_chain::BlockHeader>& block_header, const h256& bridge_root,
    const h256& bridge_epoch) {
  rustaxa::PillarBlockCreationWithVoteCountsPlan creation_plan{};
  try {
    creation_plan = pbft_service_->service().pbft_service_pillar_plan_block_creation_with_final_chain(
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

  std::vector<state_api::ValidatorVoteCount> new_vote_counts;
  new_vote_counts.reserve(creation_plan.current_vote_counts.size());
  for (const auto& vote_count : creation_plan.current_vote_counts) {
    new_vote_counts.push_back({fromBridgeAddress(vote_count.address), vote_count.vote_count});
  }
  {
    std::scoped_lock<std::shared_mutex> lock(mutex_);
    pbft_service_->service().pbft_service_pillar_apply_planned_current_block_data(
        toRustBytes(util::rlp_enc(CurrentPillarBlockDataDb{pillar_block, new_vote_counts})),
        creation_plan.anchor_generation);
  }
  LOG(log_nf_) << "New pillar block " << pillar_block->getHash() << " with period " << pillar_block->getPeriod()
               << " created";

  return pillar_block;
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
  try {
    return decodePillarBlockFromRustBytes(pbft_service_->service().pbft_service_pillar_current_block_rlp());
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to materialize current pillar block from Rust runtime: " << e.what();
    return nullptr;
  }
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
      validatePillarVoteWithRust(kFicusHfConfig, vote, pbft_service_->service(), *final_chain_);
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

  return true;
}

PillarChainManager::PillarVoteAdmissionReport PillarChainManager::admitPillarVote(
    const std::shared_ptr<PillarVote>& vote) {
  return admitPillarVoteImpl(vote, false);
}

PillarChainManager::PillarVoteAdmissionReport PillarChainManager::admitPillarVoteImpl(
    const std::shared_ptr<PillarVote>& vote, bool trusted_local_or_restore) {
  if (!vote) {
    return {};
  }
  const auto prepared = pbft_service_->service().pbft_service_pillar_prepare_single_vote_external_facts(
      toRustBytes(vote->rlp()), toSingleVoteAdmissionContext(kFicusHfConfig), trusted_local_or_restore);
  const auto vote_hash = fromBridgeHash(prepared.vote_hash);
  const auto recovered_voter = fromBridgeAddress(prepared.voter);
  auto validation_status = toPillarVoteValidationStatus(prepared.status);
  if (!prepared.can_query_dpos || prepared.period == 0) {
    if (validation_status == PillarVoteValidationPlanStatus::kDuplicate) {
      return {.already_present = true};
    }
    return {};
  }
  const auto dpos_period = prepared.period - 1;
  uint64_t validator_vote_count = 0;
  uint64_t total_eligible_vote_count = 0;
  try {
    validator_vote_count = final_chain_->dposEligibleVoteCount(dpos_period, recovered_voter);
    if (prepared.needs_threshold) {
      total_eligible_vote_count = final_chain_->dposEligibleTotalVoteCount(dpos_period);
    }
  } catch (const std::exception& error) {
    if (!isUnavailableDposSnapshotError(error)) {
      throw;
    }
    LOG(log_dg_) << "Deferring pillar vote " << vote_hash << ", period " << prepared.period
                 << " until native FinalChain DPoS period " << dpos_period << " is available";
    return {.deferred = true};
  }
  rustaxa::PillarVoteSingleAdmissionApplyInput apply_input{};
  apply_input.vote_hash = prepared.vote_hash;
  apply_input.validator_vote_count = validator_vote_count;
  apply_input.has_total_eligible_vote_count = prepared.needs_threshold;
  if (prepared.needs_threshold) {
    apply_input.total_eligible_vote_count = total_eligible_vote_count;
  }
  const auto insert_outcome =
      pbft_service_->service().pbft_service_pillar_apply_prepared_single_vote_external_facts(apply_input);
  validation_status = toPillarVoteValidationStatus(insert_outcome.status);
  if (validation_status == PillarVoteValidationPlanStatus::kDuplicate) {
    return {.already_present = true};
  }
  if (validation_status != PillarVoteValidationPlanStatus::kValid) {
    LOG(log_er_) << "Unable to insert pillar vote " << vote_hash << ", period " << prepared.period << ", validator "
                 << recovered_voter << ": " << pillarVoteValidationPlanStatusString(validation_status);
    return {};
  }
  if (insert_outcome.conflict_found) {
    LOG(log_er_) << "Non-unique pillar vote " << vote_hash << ", period " << prepared.period << ", validator "
                 << recovered_voter;
    return {.conflict = true};
  }
  if (!insert_outcome.accepted && !insert_outcome.duplicate) {
    LOG(log_er_) << "Unable to insert pillar vote " << vote_hash << ", period " << prepared.period << ", validator "
                 << recovered_voter << ", unexpected Rust insert outcome";
    return {};
  }

  if (insert_outcome.accepted) {
    LOG(log_nf_) << "Added pillar vote " << vote_hash << ", period " << prepared.period << ", pillar block hash "
                 << vote->getBlockHash();
  }
  return {.accepted = insert_outcome.accepted && !insert_outcome.duplicate,
          .already_present = insert_outcome.duplicate,
          .conflict = false,
          .validator_vote_count = validator_vote_count};
}

uint64_t PillarChainManager::addVerifiedPillarVote(const std::shared_ptr<PillarVote>& vote) {
  return admitPillarVote(vote).validator_vote_count;
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

std::optional<uint64_t> PillarChainManager::getPillarConsensusThreshold(PbftPeriod period) const {
  std::optional<uint64_t> threshold;

  try {
    const auto lookup = pbft_service_->service().pbft_service_pillar_consensus_threshold_with_final_chain(period);
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
