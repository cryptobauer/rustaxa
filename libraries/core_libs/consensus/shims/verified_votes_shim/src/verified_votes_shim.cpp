#include "vote_manager/verified_votes.hpp"

#include <algorithm>
#include <mutex>
#include <stdexcept>
#include <unordered_set>
#include <utility>

#include "common/constants.hpp"
#include "vote/pbft_vote.hpp"

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

  return rustaxa::VerifiedVotePayload{
      toBridgeHash(vote->getHash()), toBridgeHash(vote->getBlockHash()), toBridgeAddress(vote->getVoterAddr()),
      vote->getPeriod(),           vote->getRound(),                    vote->getStep(),
      static_cast<uint8_t>(vote->getType()), requireVoteWeight(vote)};
}

const std::shared_ptr<PbftVote>& VerifiedVotes::requireLiveVote(const vote_hash_t& vote_hash) const {
  const auto found = live_votes_.find(vote_hash);
  if (found == live_votes_.end()) {
    throw verifiedVotesError("missing live vote sidecar for hash " + vote_hash.hex().substr(0, 16));
  }
  return found->second;
}

VotesWithWeight VerifiedVotes::requireInsertedVotesWithWeightLocked(const std::shared_ptr<PbftVote>& vote,
                                                                    uint64_t total_weight) const {
  VotesWithWeight value{};
  value.weight = total_weight;
  const auto state = buildSnapshotState();
  const auto period_it = state.find(vote->getPeriod());
  if (period_it == state.end()) {
    throw verifiedVotesError("Rust inserted voted value but C++ snapshot has no matching period");
  }
  const auto round_it = period_it->second.find(vote->getRound());
  if (round_it == period_it->second.end()) {
    throw verifiedVotesError("Rust inserted voted value but C++ snapshot has no matching round");
  }
  const auto step_it = round_it->second.step_votes.find(vote->getStep());
  if (step_it == round_it->second.step_votes.end()) {
    throw verifiedVotesError("Rust inserted voted value but C++ snapshot has no matching step");
  }
  const auto found = step_it->second.votes.find(vote->getBlockHash());
  if (found == step_it->second.votes.end()) {
    throw verifiedVotesError("Rust inserted voted value but C++ snapshot has no matching block bucket");
  }
  value.votes = found->second.votes;
  return value;
}

PeriodVerifiedVotesMap VerifiedVotes::buildSnapshotState() const {
  PeriodVerifiedVotesMap state;

  const auto votes_snapshot = rust_verified_votes_->verified_votes_snapshot_votes();
  for (const auto& vote_data : votes_snapshot) {
    const auto vote_hash = fromBridgeHash(vote_data.vote_hash);
    const auto block_hash = fromBridgeHash(vote_data.block_hash);
    auto vote = requireLiveVote(vote_hash);

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

std::vector<std::shared_ptr<PbftVote>> VerifiedVotes::votes() const {
  std::shared_lock lock(verified_votes_access_);

  std::vector<std::shared_ptr<PbftVote>> out;
  const auto snapshot = rust_verified_votes_->verified_votes_snapshot_votes();
  out.reserve(snapshot.size());
  for (const auto& vote : snapshot) {
    out.push_back(requireLiveVote(fromBridgeHash(vote.vote_hash)));
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
  const auto lookup = rust_verified_votes_->verified_votes_get_two_t_plus_one_voted_block(
      period, round, static_cast<uint8_t>(type));
  if (!lookup.found) {
    return std::nullopt;
  }

  return VotedBlock{fromBridgeHash(lookup.block_hash), static_cast<PbftStep>(lookup.step)};
}

std::vector<std::shared_ptr<PbftVote>> VerifiedVotes::getTwoTPlusOneVotedBlockVotes(
    PbftPeriod period, PbftRound round, TwoTPlusOneVotedBlockType type) const {
  std::shared_lock lock(verified_votes_access_);
  const auto lookup = rust_verified_votes_->verified_votes_get_two_t_plus_one_voted_block_votes(
      period, round, static_cast<uint8_t>(type));
  if (!lookup.found) {
    return {};
  }

  std::vector<std::shared_ptr<PbftVote>> out;
  out.reserve(lookup.vote_hashes.size());
  for (const auto& vote_hash : lookup.vote_hashes) {
    out.push_back(requireLiveVote(fromBridgeHash(vote_hash.hash)));
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
  return requireInsertedVotesWithWeightLocked(vote, outcome.total_weight);
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

}  // namespace taraxa
