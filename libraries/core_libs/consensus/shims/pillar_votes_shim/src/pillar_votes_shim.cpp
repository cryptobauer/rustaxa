#include <mutex>
#include <shared_mutex>
#include <stdexcept>
#include <unordered_set>

#include "pillar_chain/pillar_votes.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa::pillar_chain {
namespace {

std::runtime_error pillarVotesError(const std::string& msg) { return std::runtime_error("PillarVotes: " + msg); }

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }

std::array<uint8_t, 20> toBridgeAddress(const addr_t& address) { return address.asArray(); }

vote_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return vote_hash_t(hash.data(), vote_hash_t::ConstructFromPointer);
}

rust::Vec<uint8_t> toBridgeBytes(const bytes& input) {
  rust::Vec<uint8_t> out;
  out.reserve(input.size());
  for (const auto byte : input) {
    out.push_back(byte);
  }
  return out;
}

rustaxa::PillarVotePayload toBridgePayload(const std::shared_ptr<PillarVote>& vote, uint64_t validator_vote_count,
                                           const std::array<uint8_t, 20>& voter) {
  if (!vote) {
    throw pillarVotesError("cannot bridge null vote pointer");
  }
  if (validator_vote_count == 0) {
    throw pillarVotesError("validator vote count must be non-zero");
  }

  return rustaxa::PillarVotePayload{
      toBridgeHash(vote->getHash()), toBridgeHash(vote->getBlockHash()), voter, vote->getPeriod(),
      validator_vote_count,          toBridgeBytes(vote->rlp())};
}

rustaxa::PillarVotePayload toBridgePayload(const std::shared_ptr<PillarVote>& vote, uint64_t validator_vote_count,
                                           bool include_voter) {
  return toBridgePayload(vote, validator_vote_count,
                         include_voter ? toBridgeAddress(vote->getVoterAddr()) : std::array<uint8_t, 20>{});
}

}  // namespace

PillarVotes::PillarVotes() : rust_pillar_votes_(rustaxa::create_pillar_votes_index()) {}

const std::shared_ptr<PillarVote>& PillarVotes::requireLiveVote(const vote_hash_t& vote_hash) const {
  const auto found = live_votes_.find(vote_hash);
  if (found == live_votes_.end()) {
    throw pillarVotesError("missing live vote sidecar for hash " + vote_hash.hex().substr(0, 16));
  }
  return found->second;
}

std::shared_ptr<PillarVote> PillarVotes::materializeVoteRecord(const rustaxa::PillarVoteRecord& record) const {
  bytes vote_rlp;
  vote_rlp.reserve(record.vote_rlp.size());
  for (const auto byte : record.vote_rlp) {
    vote_rlp.push_back(byte);
  }
  auto vote = std::make_shared<PillarVote>(dev::RLP(vote_rlp));
  const auto expected_hash = fromBridgeHash(record.vote_hash);
  if (vote->getHash() != expected_hash) {
    throw pillarVotesError("Rust retained pillar vote payload hash mismatches materialized vote");
  }
  return vote;
}

void PillarVotes::trackVote(const std::shared_ptr<PillarVote>& vote) { live_votes_[vote->getHash()] = vote; }

void PillarVotes::pruneLiveVotesToSnapshotLocked() {
  std::unordered_set<vote_hash_t> keep;
  const auto snapshot = rust_pillar_votes_->pillar_votes_snapshot_refs();
  keep.reserve(snapshot.size());
  for (const auto& vote_ref : snapshot) {
    keep.insert(fromBridgeHash(vote_ref.vote_hash));
  }

  for (auto it = live_votes_.begin(); it != live_votes_.end();) {
    if (!keep.contains(it->first)) {
      it = live_votes_.erase(it);
    } else {
      ++it;
    }
  }
}

std::array<uint8_t, 32> PillarVotes::toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }

std::array<uint8_t, 20> PillarVotes::toBridgeAddress(const addr_t& address) { return address.asArray(); }

vote_hash_t PillarVotes::fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return taraxa::pillar_chain::fromBridgeHash(hash);
}

rustaxa::PillarVotePayload PillarVotes::toBridgePayload(const std::shared_ptr<PillarVote>& vote,
                                                        uint64_t validator_vote_count) {
  return taraxa::pillar_chain::toBridgePayload(vote, validator_vote_count, true);
}

rustaxa::PillarVotePayload PillarVotes::toBridgeLookupPayload(const std::shared_ptr<PillarVote>& vote) {
  return taraxa::pillar_chain::toBridgePayload(vote, 1, false);
}

bool PillarVotes::voteExists(const std::shared_ptr<PillarVote> vote) const {
  std::shared_lock lock(mutex_);
  return rust_pillar_votes_->pillar_votes_vote_exists(toBridgeLookupPayload(vote));
}

bool PillarVotes::isUniqueVote(const std::shared_ptr<PillarVote> vote) const {
  std::shared_lock lock(mutex_);
  return rust_pillar_votes_->pillar_votes_is_unique_vote(toBridgePayload(vote, 1)).is_unique;
}

#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
bool PillarVotes::isUniqueVoteIdentity(PbftPeriod period, const vote_hash_t& vote_hash, const addr_t& voter) const {
  std::shared_lock lock(mutex_);
  rustaxa::PillarVoteIdentityPayload payload{toBridgeHash(vote_hash), toBridgeAddress(voter), period};
  return rust_pillar_votes_->pillar_votes_is_unique_identity(payload).is_unique;
}
#endif

bool PillarVotes::periodDataInitialized(PbftPeriod period) const {
  std::shared_lock lock(mutex_);
  return rust_pillar_votes_->pillar_votes_period_data_initialized(period);
}

void PillarVotes::initializePeriodData(PbftPeriod period, uint64_t threshold) {
  std::scoped_lock lock(mutex_);
  rust_pillar_votes_->pillar_votes_init_period_data(period, threshold);
}

bool PillarVotes::addVerifiedVote(const std::shared_ptr<PillarVote>& vote, uint64_t validator_vote_count) {
  std::scoped_lock lock(mutex_);
  const auto outcome = rust_pillar_votes_->pillar_votes_insert_vote(toBridgePayload(vote, validator_vote_count));
  if (outcome.conflict_found) {
    return false;
  }

  if (!outcome.accepted && !outcome.duplicate) {
    throw pillarVotesError("Rust insert returned neither accepted, duplicate, nor conflict");
  }

  trackVote(vote);
  return true;
}

#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
bool PillarVotes::addVerifiedVoteWithRecoveredVoter(const std::shared_ptr<PillarVote>& vote,
                                                    uint64_t validator_vote_count, const addr_t& recovered_voter) {
  std::scoped_lock lock(mutex_);
  const auto outcome = rust_pillar_votes_->pillar_votes_insert_vote(
      taraxa::pillar_chain::toBridgePayload(vote, validator_vote_count, toBridgeAddress(recovered_voter)));
  if (outcome.conflict_found) {
    return false;
  }

  if (!outcome.accepted && !outcome.duplicate) {
    throw pillarVotesError("Rust insert returned neither accepted, duplicate, nor conflict");
  }

  trackVote(vote);
  return true;
}
#endif

std::vector<std::shared_ptr<PillarVote>> PillarVotes::getVerifiedVotes(PbftPeriod period,
                                                                       const blk_hash_t& pillar_block_hash,
                                                                       bool above_threshold) const {
  std::shared_lock lock(mutex_);

  std::vector<std::shared_ptr<PillarVote>> votes;
  const auto bridge_block_hash = toBridgeHash(pillar_block_hash);
  const auto vote_data =
      rust_pillar_votes_->pillar_votes_get_verified_vote_payloads(period, bridge_block_hash, above_threshold);

  if (vote_data.votes.empty()) {
    return votes;
  }

  votes.reserve(vote_data.votes.size());
  for (const auto& vote_record : vote_data.votes) {
    votes.push_back(materializeVoteRecord(vote_record));
  }

  return votes;
}

void PillarVotes::eraseVotes(PbftPeriod min_period) {
  std::scoped_lock lock(mutex_);
  rust_pillar_votes_->pillar_votes_cleanup_votes_by_period(min_period);
  pruneLiveVotesToSnapshotLocked();
}

}  // namespace taraxa::pillar_chain
