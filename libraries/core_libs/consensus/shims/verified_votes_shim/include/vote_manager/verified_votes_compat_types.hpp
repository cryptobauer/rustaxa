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
 * PBFT threshold marker categories retained by the verified-votes facade.
 *
 * Each value identifies the vote class that established a 2t+1 block or null
 * marker. The declaration order and therefore the numeric values are part of
 * the C++ compatibility contract used by bridge requests. Callers must not
 * persist or transmit values outside this closed set.
 */
enum class TwoTPlusOneVotedBlockType {
  SoftVotedBlock = 0,
  CertVotedBlock = 1,
  NextVotedBlock = 2,
  NextVotedNullBlock = 3,
};

/**
 * Hash and PBFT step selected by one 2t+1 threshold marker.
 *
 * A value-initialized instance represents the zero hash at step zero. The
 * carrier owns no vote payload and has no error state; absence is represented
 * by the optional/map APIs that contain it.
 */
struct VotedBlock {
  blk_hash_t hash;
  PbftStep step;
};

/** Maps each established threshold category to its selected block marker. */
using TwoTVotedBlockMap = std::unordered_map<TwoTPlusOneVotedBlockType, VotedBlock>;

/**
 * Live PBFT votes and accumulated weight for one voted-value bucket.
 *
 * `weight` is the sum reported for the votes accepted into the bucket, while
 * `votes` owns the live sidecars keyed by canonical vote hash. A
 * value-initialized carrier has zero weight and no votes. Duplicate and
 * consistency handling belongs to the verified-votes facade, not this data
 * carrier.
 */
struct VotesWithWeight {
  uint64_t weight;
  std::unordered_map<vote_hash_t, std::shared_ptr<PbftVote>> votes;
};

/**
 * Maps a voter address to its first and optional paired live vote sidecars.
 *
 * The pair preserves the legacy next-vote null/non-null compatibility shape;
 * conflict validation and pair ordering are enforced by the facade.
 */
using UniqueVotersMap = std::unordered_map<addr_t, std::pair<std::shared_ptr<PbftVote>, std::shared_ptr<PbftVote>>>;

/**
 * Compatibility snapshot for all verified votes at one PBFT step.
 *
 * `votes` groups sidecars by voted block hash and `unique_voters` exposes the
 * address index used by existing C++ readers. Empty maps represent a step with
 * no materialized votes; mutation invariants remain owned by the facade.
 */
struct StepVotes {
  std::unordered_map<blk_hash_t, VotesWithWeight> votes;
  UniqueVotersMap unique_voters;
};

/** Orders compatibility step snapshots by PBFT step. */
using StepVotesMap = std::map<PbftStep, StepVotes>;

/**
 * Compatibility snapshot for one PBFT round.
 *
 * Threshold markers and step snapshots are materialized from authoritative
 * Rust state. `network_t_plus_one_step` is the greatest observed step with at
 * least t+1 next-vote weight and defaults to zero when no marker exists.
 */
struct RoundVerifiedVotes {
  TwoTVotedBlockMap two_t_plus_one_voted_blocks_;
  StepVotesMap step_votes;
  PbftStep network_t_plus_one_step{0};
};

/** Orders compatibility round snapshots by PBFT round. */
using RoundVerifiedVotesMap = std::map<PbftRound, RoundVerifiedVotes>;

/** Orders compatibility round maps by PBFT period. */
using PeriodVerifiedVotesMap = std::map<PbftPeriod, RoundVerifiedVotesMap>;

}  // namespace taraxa
