#include <algorithm>
#include <stdexcept>
#include <utility>

#include "common/constants.hpp"
#include "vote/pbft_vote.hpp"
#include "vote_manager/verified_votes.hpp"

namespace taraxa {
namespace {

std::runtime_error verifiedVotesError(const std::string& msg) { return std::runtime_error("VerifiedVotes: " + msg); }

uint64_t requireVoteWeight(const std::shared_ptr<PbftVote>& vote) {
  if (!vote || !vote->getWeight().has_value() || *vote->getWeight() == 0) {
    throw verifiedVotesError("vote weight must be present and non-zero");
  }
  return *vote->getWeight();
}

}  // namespace

std::array<uint8_t, 32> VerifiedVotes::toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }

std::array<uint8_t, 20> VerifiedVotes::toBridgeAddress(const addr_t& address) { return address.asArray(); }

uint256_hash_t VerifiedVotes::fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return uint256_hash_t(hash.data(), uint256_hash_t::ConstructFromPointer);
}

rustaxa::VerifiedVotePayload VerifiedVotes::toBridgeVotePayload(const std::shared_ptr<PbftVote>& vote) const {
  if (!vote) {
    throw verifiedVotesError("cannot bridge null vote");
  }

  return rustaxa::VerifiedVotePayload{toBridgeHash(vote->getHash()),
                                      toBridgeHash(vote->getBlockHash()),
                                      toBridgeAddress(vote->getVoterAddr()),
                                      vote->getPeriod(),
                                      vote->getRound(),
                                      vote->getStep(),
                                      static_cast<uint8_t>(vote->getType()),
                                      requireVoteWeight(vote)};
}

rustaxa::PbftVoteStorageRecord VerifiedVotes::toBridgeWeightedVoteRecord(const std::shared_ptr<PbftVote>& vote) const {
  (void)requireVoteWeight(vote);
  rustaxa::PbftVoteStorageRecord record;
  record.hash = toBridgeHash(vote->getHash());
  const auto weighted_rlp = vote->rlp(true, true);
  record.vote_rlp.reserve(weighted_rlp.size());
  for (const auto byte : weighted_rlp) {
    record.vote_rlp.push_back(byte);
  }
  return record;
}

rust::Vec<rustaxa::PbftFinalizationHash> toBridgeVoteHashes(const std::vector<vote_hash_t>& hashes) {
  rust::Vec<rustaxa::PbftFinalizationHash> out;
  out.reserve(hashes.size());
  for (const auto& hash : hashes) {
    out.push_back(rustaxa::PbftFinalizationHash{hash.asArray()});
  }
  return out;
}

std::shared_ptr<PbftVote> VerifiedVotes::materializeWeightedPayload(
    const rustaxa::PbftVoteStorageRecord& record) const {
  bytes vote_rlp;
  vote_rlp.reserve(record.vote_rlp.size());
  for (const auto byte : record.vote_rlp) {
    vote_rlp.push_back(byte);
  }

  auto vote = std::make_shared<PbftVote>(vote_rlp);
  const auto expected_hash = fromBridgeHash(record.hash);
  if (vote->getHash() != expected_hash) {
    throw verifiedVotesError("Rust retained weighted payload hash mismatches materialized vote");
  }
  if (!vote->getWeight().has_value() || *vote->getWeight() == 0) {
    throw verifiedVotesError("Rust retained weighted payload decoded without non-zero weight");
  }
  return vote;
}

std::shared_ptr<PbftVote> VerifiedVotes::materializeVoteForSnapshot(
    const rustaxa::VerifiedVotePayload& vote_data, const rustaxa::PbftVoteStorageRecord& weighted_vote) const {
  auto vote = materializeWeightedPayload(weighted_vote);
  if (vote->getBlockHash() != fromBridgeHash(vote_data.block_hash) || vote->getPeriod() != vote_data.period ||
      vote->getRound() != vote_data.round || vote->getStep() != vote_data.step ||
      static_cast<uint8_t>(vote->getType()) != vote_data.vote_type || *vote->getWeight() != vote_data.weight) {
    throw verifiedVotesError("Rust retained weighted payload mismatches verified-vote metadata");
  }
  return vote;
}

std::shared_ptr<PbftVote> VerifiedVotes::materializeConflictVote(const rustaxa::PbftVoteStorageRecord& record,
                                                                 const std::array<uint8_t, 32>& expected_hash) const {
  auto vote = materializeWeightedPayload(record);
  if (record.hash != expected_hash || toBridgeHash(vote->getHash()) != expected_hash) {
    throw verifiedVotesError("Rust conflict payload mismatches selected conflict hash");
  }
  return vote;
}

VotesWithWeight VerifiedVotes::materializeInsertedVotesWithWeight(const rustaxa::VerifiedVotePayload& vote_data,
                                                                  const rustaxa::VerifiedStepVotePayloadEntry& bucket,
                                                                  uint64_t total_weight,
                                                                  bool allow_later_bucket_growth) const {
  VotesWithWeight value{};
  if (bucket.block_hash != vote_data.block_hash) {
    throw verifiedVotesError("Rust inserted voted value returned a mismatched block bucket");
  }
  if (bucket.total_weight != total_weight && (!allow_later_bucket_growth || bucket.total_weight < total_weight)) {
    throw verifiedVotesError("Rust inserted voted value weight mismatches mutation outcome");
  }
  value.weight = bucket.total_weight;

  bool current_vote_found = false;
  for (const auto& weighted_vote : bucket.votes) {
    const auto hash = fromBridgeHash(weighted_vote.hash);
    auto stored_vote = materializeWeightedPayload(weighted_vote);
    if (stored_vote->getPeriod() != vote_data.period || stored_vote->getRound() != vote_data.round ||
        stored_vote->getStep() != vote_data.step || stored_vote->getBlockHash() != fromBridgeHash(bucket.block_hash)) {
      throw verifiedVotesError("Rust inserted voted value returned a mismatched weighted payload");
    }
    current_vote_found = current_vote_found || weighted_vote.hash == vote_data.vote_hash;
    value.votes.insert({hash, std::move(stored_vote)});
  }
  if (!current_vote_found) {
    throw verifiedVotesError("Rust inserted current vote without a retained weighted payload");
  }
  return value;
}

PeriodVerifiedVotesMap VerifiedVotes::buildSnapshotState() const {
  PeriodVerifiedVotesMap state;

  const auto snapshot = pbft_service_->service().pbft_service_verified_votes_state_snapshot();
  for (const auto& entry : snapshot.votes) {
    const auto& vote_data = entry.vote;
    const auto vote_hash = fromBridgeHash(vote_data.vote_hash);
    const auto block_hash = fromBridgeHash(vote_data.block_hash);
    auto vote = materializeVoteForSnapshot(vote_data, entry.weighted_vote);

    auto& round_votes = state[vote_data.period][static_cast<PbftRound>(vote_data.round)];
    auto& step_votes = round_votes.step_votes[static_cast<PbftStep>(vote_data.step)];

    auto& voted_value = step_votes.votes[block_hash];
    voted_value.votes.insert({vote_hash, vote});
    voted_value.weight += vote_data.weight;

    auto& unique_votes = step_votes.unique_voters[vote->getVoterAddr()];
    if (!unique_votes.first) {
      unique_votes.first = vote;
    } else if (unique_votes.first->getHash() != vote_hash) {
      if (!unique_votes.second) {
        const auto first_is_null = unique_votes.first->getBlockHash() == kNullBlockHash;
        const auto second_is_null = vote->getBlockHash() == kNullBlockHash;
        if (vote->getType() == PbftVoteTypes::next_vote && (vote->getStep() % 2) && (first_is_null != second_is_null)) {
          unique_votes.second = vote;
        }
      } else if (unique_votes.second->getHash() != vote_hash) {
        throw verifiedVotesError("unexpected unique-voter snapshot conflict for voter " + vote->getVoterAddr().hex());
      }
    }
  }

  for (const auto& marker : snapshot.round_markers) {
    auto& round_votes = state[marker.period][static_cast<PbftRound>(marker.round)];
    round_votes.network_t_plus_one_step = static_cast<PbftStep>(marker.network_t_plus_one_step);
  }

  for (const auto& entry : snapshot.two_t_plus_one) {
    auto& round_votes = state[entry.period][static_cast<PbftRound>(entry.round)];
    round_votes.two_t_plus_one_voted_blocks_[static_cast<TwoTPlusOneVotedBlockType>(entry.kind)] =
        VotedBlock{fromBridgeHash(entry.block_hash), static_cast<PbftStep>(entry.step)};
  }

  return state;
}

VerifiedVotes::VerifiedVotes(addr_t node_addr, SharedPbftService pbft_service)
    : pbft_service_(std::move(pbft_service)) {
  if (!pbft_service_) {
    throw std::invalid_argument("VerifiedVotes requires a shared PBFT service");
  }
  LOG_OBJECTS_CREATE("VERIFIED_VOTES");
}

rust::Vec<rustaxa::PbftVoteStorageRecord> VerifiedVotes::ownVoteRecords() const {
  return pbft_service_->service().pbft_service_verified_votes_own_vote_records();
}

rustaxa::PbftVotePersistenceResult VerifiedVotes::saveOwnVerifiedVote(rustaxa::PbftVoteStorageRecord record) const {
  return pbft_service_->service().pbft_service_verified_votes_save_own_verified_vote(std::move(record));
}

rustaxa::PbftVotePersistenceResult VerifiedVotes::clearOwnVerifiedVotes() const {
  return pbft_service_->service().pbft_service_verified_votes_clear_own_verified_votes();
}

rustaxa::PbftVotePersistenceResult VerifiedVotes::persistPbftVoteProgress(
    rustaxa::PbftVoteProgressPersistenceWrite write) const {
  return pbft_service_->service().pbft_service_verified_votes_persist_pbft_vote_progress(std::move(write));
}

rustaxa::PbftFinalizedPeriodApplyResult VerifiedVotes::applyPbftFinalizationStorageWrites(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent,
    rust::Vec<rustaxa::PbftFinalizationStorageWriteStage> stages, bool sync) const {
  return pbft_service_->service().pbft_service_verified_votes_apply_pbft_finalization_storage_writes(
      write_intent, std::move(stages), sync);
}

rustaxa::PbftFinalizationStorageWriteStage VerifiedVotes::prepareRewardVotesResetStage(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent) const {
  return pbft_service_->service().pbft_service_verified_votes_prepare_reward_votes_reset_stage(write_intent);
}

rustaxa::PbftFinalizedPeriodApplyResult VerifiedVotes::applyRewardVotesReset(
    rustaxa::PbftRewardVotesResetRequest request) const {
  return pbft_service_->service().pbft_service_verified_votes_apply_reward_votes_reset(std::move(request));
}

uint64_t VerifiedVotes::size() const { return pbft_service_->service().pbft_service_verified_votes_size(); }

bool VerifiedVotes::replayContains(const vote_hash_t& vote_hash) const {
  const auto bridge_hash = toBridgeHash(vote_hash);
  return pbft_service_->service().pbft_service_verified_votes_replay_contains(bridge_hash);
}

bool VerifiedVotes::replayInsert(const vote_hash_t& vote_hash) const {
  const auto bridge_hash = toBridgeHash(vote_hash);
  return pbft_service_->service().pbft_service_verified_votes_replay_insert(bridge_hash);
}

rustaxa::PbftTwoTPlusOneThresholdPlan VerifiedVotes::twoTPlusOneThreshold(
    const rustaxa::PbftTwoTPlusOneThresholdFact& fact) const {
  return pbft_service_->service().pbft_service_verified_votes_two_t_plus_one_threshold(fact);
}

rustaxa::PbftVoteRuntimeValidationResult VerifiedVotes::validateCanonicalVote(
    rust::Slice<const uint8_t> canonical_vote_rlp, rustaxa::PbftVoteValidationExternalFacts validation_facts) const {
  return pbft_service_->service().pbft_service_verified_votes_validate_canonical_vote(canonical_vote_rlp,
                                                                                      validation_facts);
}

std::vector<std::shared_ptr<PbftVote>> VerifiedVotes::votes() const {
  std::vector<std::shared_ptr<PbftVote>> out;
  const auto snapshot = pbft_service_->service().pbft_service_verified_votes_state_snapshot();
  out.reserve(snapshot.votes.size());
  for (const auto& entry : snapshot.votes) {
    out.push_back(materializeVoteForSnapshot(entry.vote, entry.weighted_vote));
  }
  return out;
}

std::optional<const RoundVerifiedVotesMap> VerifiedVotes::getPeriodVotes(PbftPeriod period) const {
  auto state = buildSnapshotState();
  auto found = state.find(period);
  if (found == state.end()) {
    return std::nullopt;
  }
  return found->second;
}

std::optional<const RoundVerifiedVotes> VerifiedVotes::getRoundVotes(PbftPeriod period, PbftRound round) const {
  auto state = buildSnapshotState();
  auto period_it = state.find(period);
  if (period_it == state.end()) {
    return std::nullopt;
  }
  auto round_it = period_it->second.find(round);
  if (round_it == period_it->second.end()) {
    return std::nullopt;
  }
  return round_it->second;
}

std::optional<const StepVotes> VerifiedVotes::getStepVotes(PbftPeriod period, PbftRound round, PbftStep step) const {
  const auto lookup = pbft_service_->service().pbft_service_verified_votes_step_payloads(period, round, step);
  if (!lookup.found) {
    return std::nullopt;
  }

  StepVotes result;
  for (const auto& entry : lookup.entries) {
    const auto block_hash = fromBridgeHash(entry.block_hash);
    auto& voted_value = result.votes[block_hash];
    voted_value.weight = entry.total_weight;
    for (const auto& record : entry.votes) {
      auto vote = materializeWeightedPayload(record);
      if (vote->getPeriod() != period || vote->getRound() != round || vote->getStep() != step ||
          vote->getBlockHash() != block_hash) {
        throw verifiedVotesError("Rust step payload mismatches requested vote bucket");
      }

      const auto vote_hash = vote->getHash();
      voted_value.votes.insert({vote_hash, vote});
      auto& unique_votes = result.unique_voters[vote->getVoterAddr()];
      if (!unique_votes.first) {
        unique_votes.first = vote;
      } else if (unique_votes.first->getHash() != vote_hash) {
        if (!unique_votes.second) {
          const auto first_is_null = unique_votes.first->getBlockHash() == kNullBlockHash;
          const auto second_is_null = vote->getBlockHash() == kNullBlockHash;
          if (vote->getType() == PbftVoteTypes::next_vote && (vote->getStep() % 2) && first_is_null != second_is_null) {
            unique_votes.second = vote;
          }
        } else if (unique_votes.second->getHash() != vote_hash) {
          throw verifiedVotesError("unexpected unique-voter step conflict for voter " + vote->getVoterAddr().hex());
        }
      }
    }
  }
  return result;
}

std::optional<VotedBlock> VerifiedVotes::getTwoTPlusOneVotedBlock(PbftPeriod period, PbftRound round,
                                                                  TwoTPlusOneVotedBlockType type) const {
  const auto lookup = pbft_service_->service().pbft_service_verified_votes_get_two_t_plus_one_voted_block(
      period, round, static_cast<uint8_t>(type));
  if (!lookup.found) {
    return std::nullopt;
  }

  return VotedBlock{fromBridgeHash(lookup.block_hash), static_cast<PbftStep>(lookup.step)};
}

std::vector<std::shared_ptr<PbftVote>> VerifiedVotes::getTwoTPlusOneVotedBlockVotes(
    PbftPeriod period, PbftRound round, TwoTPlusOneVotedBlockType type) const {
  const auto lookup = pbft_service_->service().pbft_service_verified_votes_get_two_t_plus_one_voted_block_payloads(
      period, round, static_cast<uint8_t>(type));
  if (!lookup.found) {
    return {};
  }

  std::vector<std::shared_ptr<PbftVote>> out;
  out.reserve(lookup.votes.size());
  const auto expected_block_hash = fromBridgeHash(lookup.block_hash);
  for (const auto& record : lookup.votes) {
    auto vote = materializeWeightedPayload(record);
    if (vote->getPeriod() != period || vote->getRound() != round || vote->getStep() != lookup.step ||
        vote->getBlockHash() != expected_block_hash) {
      throw verifiedVotesError("Rust retained 2t+1 payload mismatches mapped voted block");
    }
    out.push_back(std::move(vote));
  }
  return out;
}

rustaxa::PbftNextVotesBundleEgressPlan VerifiedVotes::planNextVotesBundleEgress(PbftPeriod period,
                                                                                PbftRound round) const {
  return pbft_service_->service().pbft_service_verified_votes_plan_next_votes_bundle_egress(period, round);
}

rustaxa::PbftOptimizedVoteBundleBuildResult VerifiedVotes::buildOptimizedVotesBundleEgress(
    rustaxa::PbftOptimizedVoteBundleBuildRequest request) const {
  return pbft_service_->service().pbft_service_verified_votes_build_optimized_votes_bundle_egress(std::move(request));
}

VerifiedVotes::RewardVotePayloadSelection VerifiedVotes::selectRewardVotePayloads(
    PbftPeriod block_period, const std::vector<vote_hash_t>& requested_vote_hashes, bool materialize_votes) const {
  auto selection = pbft_service_->service().pbft_service_verified_votes_select_reward_vote_payloads(
      block_period, toBridgeVoteHashes(requested_vote_hashes));

  RewardVotePayloadSelection out{};
  out.report = std::move(selection);
  if (!out.report.accepted || !materialize_votes) {
    return out;
  }

  out.votes.reserve(out.report.selected_records.size());
  const auto expected_block_hash = fromBridgeHash(out.report.selected_block_hash);
  for (const auto& record : out.report.selected_records) {
    auto vote = materializeWeightedPayload(record);
    if (vote->getPeriod() != out.report.selected_period || vote->getRound() != out.report.selected_round ||
        vote->getStep() != static_cast<PbftStep>(PbftVoteTypes::cert_vote) ||
        vote->getBlockHash() != expected_block_hash) {
      throw verifiedVotesError("Rust reward-vote payload selection returned mismatched weighted payload");
    }
    out.votes.push_back(std::move(vote));
  }
  return out;
}

rustaxa::RewardVoteCursorSnapshot VerifiedVotes::rewardVoteCursor() const {
  return pbft_service_->service().pbft_service_verified_votes_reward_vote_cursor();
}

PbftPeriod VerifiedVotes::rewardVotePeriod() const {
  return pbft_service_->service().pbft_service_verified_votes_reward_vote_period();
}

std::vector<std::shared_ptr<PbftVote>> VerifiedVotes::currentRewardVotes() const {
  const auto snapshot = pbft_service_->service().pbft_service_verified_votes_current_reward_snapshot();
  const auto& cursor = snapshot.cursor;
  const auto& records = snapshot.records;
  std::vector<std::shared_ptr<PbftVote>> votes;
  votes.reserve(records.size());
  for (const auto& record : records) {
    auto vote = materializeWeightedPayload(record);
    if (!cursor.found || vote->getPeriod() != cursor.period || vote->getRound() != cursor.round ||
        vote->getStep() != cursor.step || vote->getBlockHash() != fromBridgeHash(cursor.block_hash)) {
      throw verifiedVotesError("Rust current reward-vote payload mismatches authoritative cursor");
    }
    votes.push_back(std::move(vote));
  }
  return votes;
}

rustaxa::RewardVoteCursorCommitResult VerifiedVotes::commitRewardVoteCursor(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent, uint64_t reset_generation) {
  return pbft_service_->service().pbft_service_verified_votes_commit_reward_vote_cursor(write_intent, reset_generation);
}

void VerifiedVotes::cleanupVotesByPeriod(PbftPeriod pbft_period) {
  pbft_service_->service().pbft_service_verified_votes_cleanup_votes_by_period(pbft_period);
}

std::optional<std::shared_ptr<PbftVote>> VerifiedVotes::insertUniqueVoter(const std::shared_ptr<PbftVote>& vote) {
  const auto payload = toBridgeVotePayload(vote);
  const auto outcome = pbft_service_->service().pbft_service_verified_votes_insert_unique_voter(
      payload, toBridgeWeightedVoteRecord(vote));
  if (outcome.accepted) {
    return std::nullopt;
  }
  if (!outcome.conflict_found) {
    throw verifiedVotesError("Rust rejected unique voter insert without conflict hash");
  }
  if (!outcome.conflicting_vote_found) {
    throw verifiedVotesError("Rust rejected unique voter insert without owned conflict payload");
  }
  return materializeConflictVote(outcome.conflicting_vote, outcome.conflicting_vote_hash);
}

std::optional<VotesWithWeight> VerifiedVotes::insertVotedValue(const std::shared_ptr<PbftVote>& vote) {
  const auto payload = toBridgeVotePayload(vote);
  const auto outcome = pbft_service_->service().pbft_service_verified_votes_insert_voted_value(
      payload, toBridgeWeightedVoteRecord(vote));
  if (!outcome.inserted) {
    return {};
  }
  if (!outcome.bucket_found) {
    throw verifiedVotesError("Rust inserted voted value without owned bucket payloads");
  }

  return materializeInsertedVotesWithWeight(payload, outcome.bucket, outcome.total_weight, false);
}

VerifiedVotes::AtomicInsertOutcome VerifiedVotes::insertVerifiedVoteAtomic(const std::shared_ptr<PbftVote>& vote) {
  const auto payload = toBridgeVotePayload(vote);

  const auto outcome = pbft_service_->service().pbft_service_verified_votes_insert_vote_atomic(
      payload, toBridgeWeightedVoteRecord(vote));
  if (outcome.conflict_found) {
    if (!outcome.conflicting_vote_found) {
      throw verifiedVotesError("Rust atomic vote insert returned conflict without owned payload");
    }
    return AtomicInsertOutcome{materializeConflictVote(outcome.conflicting_vote, outcome.conflicting_vote_hash),
                               std::nullopt};
  }

  if (!outcome.inserted) {
    return AtomicInsertOutcome{std::nullopt, std::nullopt};
  }

  if (!outcome.bucket_found) {
    throw verifiedVotesError("Rust atomic vote insert returned insertion without owned bucket payloads");
  }
  return AtomicInsertOutcome{std::nullopt,
                             materializeInsertedVotesWithWeight(payload, outcome.bucket, outcome.total_weight)};
}

VerifiedVotes::AddVerifiedVoteOutcome VerifiedVotes::addVerifiedVoteWithThreshold(
    const std::shared_ptr<PbftVote>& vote, std::optional<uint64_t> two_t_plus_one) {
  const auto payload = toBridgeVotePayload(vote);
  const auto outcome = pbft_service_->service().pbft_service_verified_votes_add_verified_vote(
      payload, toBridgeWeightedVoteRecord(vote), two_t_plus_one.value_or(0), two_t_plus_one.has_value());

  AddVerifiedVoteOutcome result{};
  result.report = outcome;

  if (outcome.conflict_found) {
    if (!outcome.conflicting_vote_found) {
      throw verifiedVotesError("Rust verified-vote add returned conflict without owned payload");
    }
    result.conflicting_vote = materializeConflictVote(outcome.conflicting_vote, outcome.conflicting_vote_hash);
    return result;
  }

  if (!outcome.inserted) {
    return result;
  }

  if (!outcome.bucket_found) {
    throw verifiedVotesError("Rust verified-vote add returned insertion without owned bucket payloads");
  }
  result.votes_with_weight = materializeInsertedVotesWithWeight(payload, outcome.bucket, outcome.total_weight);
  return result;
}

rustaxa::PbftVoteAdmissionRuntimeResult VerifiedVotes::admitValidatedVote(
    rust::Slice<const uint8_t> canonical_vote_rlp, rustaxa::PbftVoteValidationExternalFacts validation_facts,
    rustaxa::PbftVoteEventFactFlags flags, rustaxa::PbftVoteProgressContext context) {
  return pbft_service_->service().pbft_service_verified_votes_admit_validated_vote(canonical_vote_rlp, validation_facts,
                                                                                   flags, context);
}

void VerifiedVotes::verifyRuntimeAcceptedPayload(const rustaxa::PbftVoteAdmissionRuntimeResult& result) const {
  if (!result.accepted || !result.has_verified_vote_add || !result.verified_vote_add.inserted) {
    return;
  }
  if (result.vote.vote_hash != result.verified_vote_add.vote.vote_hash) {
    throw verifiedVotesError("runtime admission accepted mismatched vote hashes");
  }
  if (!result.has_storage_vote) {
    throw verifiedVotesError("runtime admission accepted without a retained weighted payload");
  }
  if (result.storage_vote.hash != result.vote.vote_hash) {
    throw verifiedVotesError("runtime admission retained weighted payload for a different vote hash");
  }
  if (result.verified_vote_add.vote.weight != result.vote.weight) {
    throw verifiedVotesError("runtime admission verified-vote weight mismatches accepted vote");
  }

  const auto retained_vote = materializeWeightedPayload(result.storage_vote);
  if (toBridgeHash(retained_vote->getHash()) != result.vote.vote_hash ||
      toBridgeHash(retained_vote->getBlockHash()) != result.vote.block_hash ||
      retained_vote->getPeriod() != result.vote.period || retained_vote->getRound() != result.vote.round ||
      retained_vote->getStep() != result.vote.step ||
      static_cast<uint8_t>(retained_vote->getType()) != result.vote.vote_type ||
      retained_vote->getWeight().value_or(0) != result.vote.weight) {
    throw verifiedVotesError("runtime admission retained payload mismatches accepted vote");
  }
}

void VerifiedVotes::setNetworkTPlusOneStep(std::shared_ptr<PbftVote> vote) {
  pbft_service_->service().pbft_service_verified_votes_set_network_t_plus_one_step(vote->getPeriod(), vote->getRound(),
                                                                                   vote->getStep());
}

bool VerifiedVotes::insertTwoTPlusOneVotedBlock(TwoTPlusOneVotedBlockType type, std::shared_ptr<PbftVote> vote) {
  const auto block_hash = toBridgeHash(vote->getBlockHash());
  const auto outcome = pbft_service_->service().pbft_service_verified_votes_insert_two_t_plus_one_voted_block(
      vote->getPeriod(), vote->getRound(), static_cast<uint8_t>(type), block_hash, vote->getStep());
  return outcome.inserted;
}

VerifiedVotes::ThresholdDecision VerifiedVotes::decideThresholdEffects(const std::shared_ptr<PbftVote>& vote,
                                                                       uint64_t total_weight, uint64_t two_t_plus_one) {
  if (!vote) {
    throw verifiedVotesError("cannot derive threshold decision from null vote");
  }

  const auto outcome = pbft_service_->service().pbft_service_verified_votes_apply_threshold_decision(
      toBridgeVotePayload(vote), total_weight, two_t_plus_one);
  ThresholdDecision decision{};
  decision.set_network_t_plus_one_step = outcome.network_t_plus_one_step_updated;
  if (outcome.two_t_plus_one_inserted && outcome.two_t_plus_one_kind_found) {
    decision.inserted_two_t_plus_one_voted_block_type =
        static_cast<TwoTPlusOneVotedBlockType>(outcome.two_t_plus_one_kind);
  }
  return decision;
}

std::optional<VerifiedVotes::RoundAdvanceDecision> VerifiedVotes::determineRoundAdvance(
    PbftPeriod current_pbft_period, PbftRound current_pbft_round) const {
  const auto outcome =
      pbft_service_->service().pbft_service_verified_votes_determine_new_round(current_pbft_period, current_pbft_round);
  if (!outcome.found) {
    return std::nullopt;
  }

  return RoundAdvanceDecision{
      static_cast<PbftRound>(outcome.new_round),
      static_cast<PbftRound>(outcome.source_round),
      VotedBlock{fromBridgeHash(outcome.block_hash), static_cast<PbftStep>(outcome.step)},
  };
}

}  // namespace taraxa
