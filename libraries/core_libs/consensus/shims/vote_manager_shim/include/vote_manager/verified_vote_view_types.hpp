#pragma once

#include <cstdint>
#include <map>
#include <memory>
#include <unordered_map>
#include <utility>

#include "common/types.hpp"

namespace taraxa {

class PbftVote;

/**
 * PBFT threshold marker categories exposed by the stable VoteManager API.
 *
 * The declaration order is the bridge-compatible wire value used by native
 * verified-vote lookups. Values outside this closed set are invalid.
 */
enum class TwoTPlusOneVotedBlockType {
  SoftVotedBlock = 0,
  CertVotedBlock = 1,
  NextVotedBlock = 2,
  NextVotedNullBlock = 3,
};

/** Hash and PBFT step selected by one native 2t+1 threshold marker. */
struct VotedBlock {
  blk_hash_t hash;
  PbftStep step;
};

/** Maps each established threshold category to its selected block marker. */
using TwoTVotedBlockMap = std::unordered_map<TwoTPlusOneVotedBlockType, VotedBlock>;

/**
 * Materialized live PBFT votes and accumulated native weight for one block.
 *
 * These sidecars are a temporary executor/debug view. Native Rust owns
 * admission, uniqueness, accumulation, and retention invariants.
 */
struct VotesWithWeight {
  uint64_t weight{0};
  std::unordered_map<vote_hash_t, std::shared_ptr<PbftVote>> votes;
};

/** Materialized first and optional paired next-vote sidecars by voter. */
using UniqueVotersMap = std::unordered_map<addr_t, std::pair<std::shared_ptr<PbftVote>, std::shared_ptr<PbftVote>>>;

/**
 * Stable VoteManager view for one PBFT step.
 *
 * Empty maps represent a missing step. This carrier never owns authoritative
 * verified-vote state and is rebuilt from native owned payload results.
 */
struct StepVotes {
  std::unordered_map<blk_hash_t, VotesWithWeight> votes;
  UniqueVotersMap unique_voters;
};

using StepVotesMap = std::map<PbftStep, StepVotes>;

/** Stable VoteManager view for one PBFT round. */
struct RoundVerifiedVotes {
  TwoTVotedBlockMap two_t_plus_one_voted_blocks_;
  StepVotesMap step_votes;
  PbftStep network_t_plus_one_step{0};
};

using RoundVerifiedVotesMap = std::map<PbftRound, RoundVerifiedVotes>;
using PeriodVerifiedVotesMap = std::map<PbftPeriod, RoundVerifiedVotesMap>;

}  // namespace taraxa
