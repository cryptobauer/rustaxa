#include <algorithm>
#include <mutex>
#include <stdexcept>
#include <unordered_set>
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
    const rustaxa::VerifiedVotePayload& vote_data) const {
  const auto vote_hash = fromBridgeHash(vote_data.vote_hash);
  const auto payload_lookup = rust_verified_votes_->verified_votes_weighted_payload(vote_data.vote_hash);
  if (payload_lookup.found) {
    auto vote = materializeWeightedPayload(payload_lookup.vote);
    if (vote->getBlockHash() != fromBridgeHash(vote_data.block_hash) || vote->getPeriod() != vote_data.period ||
        vote->getRound() != vote_data.round || vote->getStep() != vote_data.step ||
        static_cast<uint8_t>(vote->getType()) != vote_data.vote_type || *vote->getWeight() != vote_data.weight) {
      throw verifiedVotesError("Rust retained weighted payload mismatches verified-vote metadata");
    }
    return vote;
  }

  // TODO(rustaxa): remove this compatibility fallback once low-level bridge test helpers stop inserting
  // verified-vote metadata without retaining weighted payload bytes through the admission runtime.
  const auto live_vote = live_votes_.find(vote_hash);
  if (live_vote == live_votes_.end()) {
    throw verifiedVotesError("missing Rust retained weighted payload for vote " + vote_hash.hex().substr(0, 16));
  }
  return live_vote->second;
}

const std::shared_ptr<PbftVote>& VerifiedVotes::requireLiveVote(const vote_hash_t& vote_hash) const {
  const auto found = live_votes_.find(vote_hash);
  if (found == live_votes_.end()) {
    throw verifiedVotesError("missing live vote sidecar for hash " + vote_hash.hex().substr(0, 16));
  }
  return found->second;
}

VotesWithWeight VerifiedVotes::requireInsertedVotesWithWeightLocked(const std::shared_ptr<PbftVote>& vote,
                                                                    uint64_t total_weight,
                                                                    bool allow_later_bucket_growth) const {
  VotesWithWeight value{};
  const auto step_votes =
      rust_verified_votes_->verified_votes_get_step_votes(vote->getPeriod(), vote->getRound(), vote->getStep());
  if (!step_votes.found) {
    throw verifiedVotesError("Rust inserted voted value but Rust step lookup has no matching step");
  }

  bool found_block = false;
  for (const auto& entry : step_votes.entries) {
    if (fromBridgeHash(entry.block_hash) != vote->getBlockHash()) {
      continue;
    }
    found_block = true;
    if (entry.total_weight != total_weight && (!allow_later_bucket_growth || entry.total_weight < total_weight)) {
      throw verifiedVotesError("Rust inserted voted value weight mismatches Rust step lookup");
    }
    value.weight = entry.total_weight;

    // TODO(rustaxa): delete this live-sidecar reconstruction once PBFT vote progress no longer returns
    // `VotesWithWeight` for compatibility callers. Rust owns threshold and persistence payloads on the production path.
    for (const auto& vote_hash : entry.vote_hashes) {
      const auto hash = fromBridgeHash(vote_hash.hash);
      const auto live_vote = live_votes_.find(hash);
      if (live_vote != live_votes_.end()) {
        value.votes.insert({hash, live_vote->second});
      } else if (hash == vote->getHash()) {
        throw verifiedVotesError("Rust inserted current vote but C++ live sidecar is missing");
      }
    }
    break;
  }
  if (!found_block) {
    throw verifiedVotesError("Rust inserted voted value but Rust step lookup has no matching block bucket");
  }
  return value;
}

PeriodVerifiedVotesMap VerifiedVotes::buildSnapshotState() const {
  PeriodVerifiedVotesMap state;

  const auto votes_snapshot = rust_verified_votes_->verified_votes_snapshot_votes();
  for (const auto& vote_data : votes_snapshot) {
    const auto vote_hash = fromBridgeHash(vote_data.vote_hash);
    const auto block_hash = fromBridgeHash(vote_data.block_hash);
    auto vote = materializeVoteForSnapshot(vote_data);

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

  const auto markers = rust_verified_votes_->verified_votes_snapshot_round_markers();
  for (const auto& marker : markers) {
    auto& round_votes = state[marker.period][static_cast<PbftRound>(marker.round)];
    round_votes.network_t_plus_one_step = static_cast<PbftStep>(marker.network_t_plus_one_step);
  }

  const auto two_t_plus_one = rust_verified_votes_->verified_votes_snapshot_two_t_plus_one();
  for (const auto& entry : two_t_plus_one) {
    auto& round_votes = state[entry.period][static_cast<PbftRound>(entry.round)];
    round_votes.two_t_plus_one_voted_blocks_[static_cast<TwoTPlusOneVotedBlockType>(entry.kind)] =
        VotedBlock{fromBridgeHash(entry.block_hash), static_cast<PbftStep>(entry.step)};
  }

  return state;
}

void VerifiedVotes::pruneLiveVotesToSnapshotLocked() {
  std::unordered_set<vote_hash_t> keep;
  const auto snapshot = rust_verified_votes_->verified_votes_snapshot_votes();
  keep.reserve(snapshot.size());
  for (const auto& vote : snapshot) {
    keep.insert(fromBridgeHash(vote.vote_hash));
  }

  for (auto it = live_votes_.begin(); it != live_votes_.end();) {
    if (!keep.contains(it->first)) {
      it = live_votes_.erase(it);
    } else {
      ++it;
    }
  }
}

VerifiedVotes::VerifiedVotes(addr_t node_addr) : rust_verified_votes_(rustaxa::create_verified_votes_index()) {
  (void)node_addr;
  LOG_OBJECTS_CREATE("VERIFIED_VOTES");
}

uint64_t VerifiedVotes::size() const {
  std::shared_lock lock(verified_votes_access_);
  return rust_verified_votes_->verified_votes_size();
}

bool VerifiedVotes::replayContains(const vote_hash_t& vote_hash) const {
  std::shared_lock lock(verified_votes_access_);
  const auto bridge_hash = toBridgeHash(vote_hash);
  return rust_verified_votes_->verified_votes_replay_contains(bridge_hash);
}

bool VerifiedVotes::replayInsert(const vote_hash_t& vote_hash) const {
  std::scoped_lock lock(verified_votes_access_);
  const auto bridge_hash = toBridgeHash(vote_hash);
  return rust_verified_votes_->verified_votes_replay_insert(bridge_hash);
}

rustaxa::PbftTwoTPlusOneThresholdPlan VerifiedVotes::twoTPlusOneThreshold(
    const rustaxa::PbftTwoTPlusOneThresholdFact& fact) const {
  std::scoped_lock lock(verified_votes_access_);
  return rust_verified_votes_->verified_votes_two_t_plus_one_threshold(fact);
}

rustaxa::PbftVoteRuntimeValidationResult VerifiedVotes::validateCanonicalVote(
    rust::Slice<const uint8_t> canonical_vote_rlp, rustaxa::PbftVoteValidationExternalFacts validation_facts) const {
  std::scoped_lock lock(verified_votes_access_);
  return rust_verified_votes_->verified_votes_validate_canonical_vote(canonical_vote_rlp, validation_facts);
}

std::vector<std::shared_ptr<PbftVote>> VerifiedVotes::votes() const {
  std::shared_lock lock(verified_votes_access_);

  std::vector<std::shared_ptr<PbftVote>> out;
  const auto snapshot = rust_verified_votes_->verified_votes_snapshot_votes();
  out.reserve(snapshot.size());
  for (const auto& vote_data : snapshot) {
    out.push_back(materializeVoteForSnapshot(vote_data));
  }
  return out;
}

std::optional<const RoundVerifiedVotesMap> VerifiedVotes::getPeriodVotes(PbftPeriod period) const {
  std::shared_lock lock(verified_votes_access_);
  auto state = buildSnapshotState();
  auto found = state.find(period);
  if (found == state.end()) {
    return std::nullopt;
  }
  return found->second;
}

std::optional<const RoundVerifiedVotes> VerifiedVotes::getRoundVotes(PbftPeriod period, PbftRound round) const {
  std::shared_lock lock(verified_votes_access_);
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
  std::shared_lock lock(verified_votes_access_);
  auto state = buildSnapshotState();
  auto period_it = state.find(period);
  if (period_it == state.end()) {
    return std::nullopt;
  }
  auto round_it = period_it->second.find(round);
  if (round_it == period_it->second.end()) {
    return std::nullopt;
  }
  auto step_it = round_it->second.step_votes.find(step);
  if (step_it == round_it->second.step_votes.end()) {
    return std::nullopt;
  }
  return step_it->second;
}

std::optional<VotedBlock> VerifiedVotes::getTwoTPlusOneVotedBlock(PbftPeriod period, PbftRound round,
                                                                  TwoTPlusOneVotedBlockType type) const {
  std::shared_lock lock(verified_votes_access_);
  const auto lookup =
      rust_verified_votes_->verified_votes_get_two_t_plus_one_voted_block(period, round, static_cast<uint8_t>(type));
  if (!lookup.found) {
    return std::nullopt;
  }

  return VotedBlock{fromBridgeHash(lookup.block_hash), static_cast<PbftStep>(lookup.step)};
}

std::vector<std::shared_ptr<PbftVote>> VerifiedVotes::getTwoTPlusOneVotedBlockVotes(
    PbftPeriod period, PbftRound round, TwoTPlusOneVotedBlockType type) const {
  std::shared_lock lock(verified_votes_access_);
  // TODO(rustaxa): remove this compatibility materialization API once PBFT manager/finalization callers consume
  // Rust-owned payload facts or optimized egress bytes directly.
  const auto lookup = rust_verified_votes_->verified_votes_get_two_t_plus_one_voted_block_payloads(
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
  std::shared_lock lock(verified_votes_access_);
  return rust_verified_votes_->verified_votes_plan_next_votes_bundle_egress(period, round);
}

rustaxa::PbftOptimizedVoteBundleBuildResult VerifiedVotes::buildOptimizedVotesBundleEgress(
    rustaxa::PbftOptimizedVoteBundleBuildRequest request) const {
  std::shared_lock lock(verified_votes_access_);
  return rust_verified_votes_->verified_votes_build_optimized_votes_bundle_egress(std::move(request));
}

VerifiedVotes::RewardVotePayloadSelection VerifiedVotes::selectRewardVotePayloads(
    PbftPeriod block_period, PbftPeriod reward_period, PbftRound preferred_reward_round,
    const blk_hash_t& reward_block_hash, const std::vector<vote_hash_t>& requested_vote_hashes,
    bool materialize_votes) const {
  std::shared_lock lock(verified_votes_access_);
  auto selection = rust_verified_votes_->verified_votes_select_reward_vote_payloads(
      block_period, reward_period, preferred_reward_round, toBridgeHash(reward_block_hash),
      toBridgeVoteHashes(requested_vote_hashes));

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

void VerifiedVotes::cleanupVotesByPeriod(PbftPeriod pbft_period) {
  std::scoped_lock lock(verified_votes_access_);
  rust_verified_votes_->verified_votes_cleanup_votes_by_period(pbft_period);
  pruneLiveVotesToSnapshotLocked();
}

std::optional<std::shared_ptr<PbftVote>> VerifiedVotes::insertUniqueVoter(const std::shared_ptr<PbftVote>& vote) {
  std::scoped_lock lock(verified_votes_access_);
  const auto outcome = rust_verified_votes_->verified_votes_insert_unique_voter(toBridgeVotePayload(vote));
  if (outcome.accepted) {
    return std::nullopt;
  }
  if (!outcome.conflict_found) {
    throw verifiedVotesError("Rust rejected unique voter insert without conflict hash");
  }
  const auto conflict_hash = fromBridgeHash(outcome.conflicting_vote_hash);
  return requireLiveVote(conflict_hash);
}

std::optional<VotesWithWeight> VerifiedVotes::insertVotedValue(const std::shared_ptr<PbftVote>& vote) {
  std::scoped_lock lock(verified_votes_access_);
  const auto outcome = rust_verified_votes_->verified_votes_insert_voted_value(toBridgeVotePayload(vote));
  if (!outcome.inserted) {
    return {};
  }

  live_votes_[vote->getHash()] = vote;
  return requireInsertedVotesWithWeightLocked(vote, outcome.total_weight, false);
}

VerifiedVotes::AtomicInsertOutcome VerifiedVotes::insertVerifiedVoteAtomic(const std::shared_ptr<PbftVote>& vote) {
  std::scoped_lock lock(verified_votes_access_);
  const auto payload = toBridgeVotePayload(vote);

  const auto outcome = rust_verified_votes_->verified_votes_insert_vote_atomic(payload);
  if (outcome.conflict_found) {
    const auto conflict_hash = fromBridgeHash(outcome.conflicting_vote_hash);
    return AtomicInsertOutcome{requireLiveVote(conflict_hash), std::nullopt};
  }

  if (!outcome.inserted) {
    return AtomicInsertOutcome{std::nullopt, std::nullopt};
  }

  live_votes_[vote->getHash()] = vote;
  return AtomicInsertOutcome{std::nullopt, requireInsertedVotesWithWeightLocked(vote, outcome.total_weight)};
}

VerifiedVotes::AddVerifiedVoteOutcome VerifiedVotes::addVerifiedVoteWithThreshold(
    const std::shared_ptr<PbftVote>& vote, std::optional<uint64_t> two_t_plus_one) {
  std::scoped_lock lock(verified_votes_access_);
  const auto payload = toBridgeVotePayload(vote);
  const auto outcome = rust_verified_votes_->verified_votes_add_verified_vote(payload, two_t_plus_one.value_or(0),
                                                                              two_t_plus_one.has_value());

  AddVerifiedVoteOutcome result{};
  result.report = outcome;

  if (outcome.conflict_found) {
    const auto conflict_hash = fromBridgeHash(outcome.conflicting_vote_hash);
    result.conflicting_vote = requireLiveVote(conflict_hash);
    return result;
  }

  if (!outcome.inserted) {
    return result;
  }

  live_votes_[vote->getHash()] = vote;
  result.votes_with_weight = requireInsertedVotesWithWeightLocked(vote, outcome.total_weight);
  return result;
}

rustaxa::PbftVoteAdmissionRuntimeResult VerifiedVotes::admitValidatedVote(
    rust::Slice<const uint8_t> canonical_vote_rlp, rustaxa::PbftVoteValidationExternalFacts validation_facts,
    rustaxa::PbftVoteEventFactFlags flags, rustaxa::PbftVoteProgressContext context) {
  std::scoped_lock lock(verified_votes_access_);
  return rust_verified_votes_->verified_votes_admit_validated_vote(canonical_vote_rlp, validation_facts, flags,
                                                                   context);
}

std::optional<VotesWithWeight> VerifiedVotes::attachRuntimeAcceptedVote(
    const std::shared_ptr<PbftVote>& vote, const rustaxa::PbftVoteAdmissionRuntimeResult& result) {
  if (!vote) {
    throw verifiedVotesError("cannot attach null runtime-accepted vote");
  }
  if (!result.accepted || !result.has_verified_vote_add || !result.verified_vote_add.inserted) {
    return std::nullopt;
  }
  if (result.vote.vote_hash != toBridgeHash(vote->getHash()) ||
      result.verified_vote_add.vote.vote_hash != toBridgeHash(vote->getHash())) {
    throw verifiedVotesError("runtime admission accepted a different vote hash than the live sidecar");
  }
  if (!vote->getWeight().has_value() || *vote->getWeight() != result.verified_vote_add.vote.weight) {
    throw verifiedVotesError("runtime admission weight mismatches the live sidecar");
  }

  std::scoped_lock lock(verified_votes_access_);
  live_votes_[vote->getHash()] = vote;
  return requireInsertedVotesWithWeightLocked(vote, result.verified_vote_add.total_weight, true);
}

void VerifiedVotes::setNetworkTPlusOneStep(std::shared_ptr<PbftVote> vote) {
  std::scoped_lock lock(verified_votes_access_);
  rust_verified_votes_->verified_votes_set_network_t_plus_one_step(vote->getPeriod(), vote->getRound(),
                                                                   vote->getStep());
}

bool VerifiedVotes::insertTwoTPlusOneVotedBlock(TwoTPlusOneVotedBlockType type, std::shared_ptr<PbftVote> vote) {
  std::scoped_lock lock(verified_votes_access_);
  const auto block_hash = toBridgeHash(vote->getBlockHash());
  const auto outcome = rust_verified_votes_->verified_votes_insert_two_t_plus_one_voted_block(
      vote->getPeriod(), vote->getRound(), static_cast<uint8_t>(type), block_hash, vote->getStep());
  return outcome.inserted;
}

VerifiedVotes::ThresholdDecision VerifiedVotes::decideThresholdEffects(const std::shared_ptr<PbftVote>& vote,
                                                                       uint64_t total_weight, uint64_t two_t_plus_one) {
  if (!vote) {
    throw verifiedVotesError("cannot derive threshold decision from null vote");
  }

  std::scoped_lock lock(verified_votes_access_);
  const auto outcome = rust_verified_votes_->verified_votes_apply_threshold_decision(toBridgeVotePayload(vote),
                                                                                     total_weight, two_t_plus_one);
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
  std::shared_lock lock(verified_votes_access_);
  const auto outcome =
      rust_verified_votes_->verified_votes_determine_new_round(current_pbft_period, current_pbft_round);
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
