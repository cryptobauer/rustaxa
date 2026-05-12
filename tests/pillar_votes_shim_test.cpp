#include <gtest/gtest.h>

#include <algorithm>
#include <type_traits>

#include "pillar_chain/pillar_votes.hpp"

namespace taraxa::core_tests {

namespace {
struct PeriodInitializationResult {
  bool before_init;
  bool after_init;
};

template <typename VotesT>
PeriodInitializationResult runPeriodInitializationScenario(PbftPeriod period = PbftPeriod{10}, uint64_t threshold = 4) {
  VotesT votes;
  const auto before_init = votes.periodDataInitialized(period);
  votes.initializePeriodData(period, threshold);
  return {before_init, votes.periodDataInitialized(period)};
}

struct DuplicateVoteResult {
  bool first_unique;
  bool first_added;
  bool duplicate_unique;
  bool duplicate_added;
  bool duplicate_hash_match;
  size_t all_votes_for_block;
};

template <typename VotesT>
DuplicateVoteResult runDuplicateVoteScenario(PbftPeriod period = PbftPeriod{11}, uint64_t threshold = 3,
                                             uint64_t weight = 2) {
  VotesT votes;
  votes.initializePeriodData(period, threshold);

  const auto voter_secret = secret_t::random();
  const auto vote_a = std::make_shared<PillarVote>(voter_secret, period, blk_hash_t(1));
  const auto vote_b = std::make_shared<PillarVote>(voter_secret, period, blk_hash_t(1));

  const auto first_unique = votes.isUniqueVote(vote_a);
  const auto first_added = votes.addVerifiedVote(vote_a, weight);
  const auto duplicate_unique = votes.isUniqueVote(vote_b);
  const auto duplicate_added = votes.addVerifiedVote(vote_b, weight);
  const auto all_votes_for_block = votes.getVerifiedVotes(period, blk_hash_t(1), false);

  return {first_unique, first_added, duplicate_unique, duplicate_added, vote_a->getHash() == vote_b->getHash(),
          all_votes_for_block.size()};
}

struct SameVoterConflictResult {
  bool first_unique;
  bool second_unique;
  bool first_added;
  bool second_added;
  size_t votes_for_first_block;
  size_t votes_for_second_block;
};

template <typename VotesT>
SameVoterConflictResult runSameVoterConflictScenario(PbftPeriod period = PbftPeriod{12}, uint64_t threshold = 3,
                                                     uint64_t weight = 1) {
  VotesT votes;
  votes.initializePeriodData(period, threshold);

  const auto voter_secret = secret_t::random();
  const auto first_vote = std::make_shared<PillarVote>(voter_secret, period, blk_hash_t(1));
  const auto second_vote = std::make_shared<PillarVote>(voter_secret, period, blk_hash_t(2));

  const auto first_unique = votes.isUniqueVote(first_vote);
  const auto first_added = votes.addVerifiedVote(first_vote, weight);
  const auto second_unique = votes.isUniqueVote(second_vote);
  const auto second_added = votes.addVerifiedVote(second_vote, weight);

  const auto first_votes = votes.getVerifiedVotes(period, blk_hash_t(1), false);
  const auto second_votes = votes.getVerifiedVotes(period, blk_hash_t(2), false);

  return {first_unique, second_unique, first_added, second_added, first_votes.size(), second_votes.size()};
}

struct AboveThresholdResult {
  size_t all_votes;
  size_t above_threshold_votes;
  bool contains_heaviest_votes;
  bool excludes_lowest_vote;
};

template <typename VotesT>
AboveThresholdResult runAboveThresholdScenario(PbftPeriod period = PbftPeriod{13}, uint64_t threshold = 7) {
  VotesT votes;
  votes.initializePeriodData(period, threshold);

  const auto heavy_vote = std::make_shared<PillarVote>(secret_t::random(), period, blk_hash_t(3));
  const auto medium_vote = std::make_shared<PillarVote>(secret_t::random(), period, blk_hash_t(3));
  const auto light_vote = std::make_shared<PillarVote>(secret_t::random(), period, blk_hash_t(3));

  votes.addVerifiedVote(heavy_vote, 5);
  votes.addVerifiedVote(medium_vote, 4);
  votes.addVerifiedVote(light_vote, 1);

  const auto all_votes = votes.getVerifiedVotes(period, blk_hash_t(3), false);
  const auto selected_votes = votes.getVerifiedVotes(period, blk_hash_t(3), true);

  const auto contains_heavy = std::any_of(selected_votes.begin(), selected_votes.end(),
                                          [&](const auto& vote) { return vote->getHash() == heavy_vote->getHash(); });
  const auto contains_medium = std::any_of(selected_votes.begin(), selected_votes.end(),
                                           [&](const auto& vote) { return vote->getHash() == medium_vote->getHash(); });
  const auto contains_light = std::any_of(selected_votes.begin(), selected_votes.end(),
                                         [&](const auto& vote) { return vote->getHash() == light_vote->getHash(); });

  return {all_votes.size(), selected_votes.size(), contains_heavy && contains_medium, !contains_light};
}

struct CleanupResult {
  bool previous_period_before_cleanup;
  bool previous_period_after_cleanup;
  bool current_period_after_cleanup;
  bool previous_period_votes_removed;
  bool current_period_votes_kept;
};

template <typename VotesT>
CleanupResult runCleanupScenario(PbftPeriod period = PbftPeriod{14}, uint64_t threshold = 3, uint64_t min_period = 15) {
  VotesT votes;
  votes.initializePeriodData(period, threshold);
  votes.initializePeriodData(min_period, threshold);

  const auto vote_prev_period = std::make_shared<PillarVote>(secret_t::random(), period, blk_hash_t(4));
  const auto vote_current_period = std::make_shared<PillarVote>(secret_t::random(), min_period, blk_hash_t(5));
  votes.addVerifiedVote(vote_prev_period, 1);
  votes.addVerifiedVote(vote_current_period, 1);

  const auto previous_initialized = votes.periodDataInitialized(period);
  const auto previous_votes = votes.getVerifiedVotes(period, blk_hash_t(4), false);

  votes.eraseVotes(min_period);

  const auto previous_period_after_cleanup = votes.periodDataInitialized(period);
  const auto current_period_after_cleanup = votes.periodDataInitialized(min_period);
  const auto previous_votes_after_cleanup = votes.getVerifiedVotes(period, blk_hash_t(4), false);
  const auto current_votes_after_cleanup = votes.getVerifiedVotes(min_period, blk_hash_t(5), false);

  return {previous_initialized, previous_period_after_cleanup, current_period_after_cleanup,
          !previous_votes.empty() && previous_votes_after_cleanup.empty(), !current_votes_after_cleanup.empty()};
}

}  // namespace

TEST(PillarVotesShimTest, rustModePillarVotesDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  static_assert(!std::is_base_of_v<taraxa::pillar_chain::PillarVotesOld, taraxa::pillar_chain::PillarVotes>);
  SUCCEED();
#else
  GTEST_SKIP() << "PillarVotes shim is disabled";
#endif
}

TEST(PillarVotesShimTest, parityPeriodInitialization) {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  const auto legacy = runPeriodInitializationScenario<taraxa::pillar_chain::PillarVotesOld>();
  const auto shim = runPeriodInitializationScenario<taraxa::pillar_chain::PillarVotes>();

  EXPECT_FALSE(legacy.before_init);
  EXPECT_TRUE(legacy.after_init);
  EXPECT_FALSE(shim.before_init);
  EXPECT_TRUE(shim.after_init);

  EXPECT_EQ(legacy.before_init, shim.before_init);
  EXPECT_EQ(legacy.after_init, shim.after_init);
#else
  GTEST_SKIP() << "PillarVotes shim is disabled";
#endif
}

TEST(PillarVotesShimTest, parityDuplicateVoteIdempotence) {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  const auto legacy = runDuplicateVoteScenario<taraxa::pillar_chain::PillarVotesOld>();
  const auto shim = runDuplicateVoteScenario<taraxa::pillar_chain::PillarVotes>();

  EXPECT_TRUE(legacy.first_unique);
  EXPECT_TRUE(legacy.first_added);
  EXPECT_TRUE(legacy.duplicate_unique);
  EXPECT_TRUE(legacy.duplicate_added);
  EXPECT_TRUE(legacy.duplicate_hash_match);
  EXPECT_EQ(legacy.all_votes_for_block, 1u);

  EXPECT_TRUE(shim.first_unique);
  EXPECT_TRUE(shim.first_added);
  EXPECT_TRUE(shim.duplicate_unique);
  EXPECT_TRUE(shim.duplicate_added);
  EXPECT_TRUE(shim.duplicate_hash_match);
  EXPECT_EQ(shim.all_votes_for_block, 1u);

  EXPECT_EQ(legacy.first_unique, shim.first_unique);
  EXPECT_EQ(legacy.first_added, shim.first_added);
  EXPECT_EQ(legacy.duplicate_unique, shim.duplicate_unique);
  EXPECT_EQ(legacy.duplicate_added, shim.duplicate_added);
  EXPECT_EQ(legacy.duplicate_hash_match, shim.duplicate_hash_match);
  EXPECT_EQ(legacy.all_votes_for_block, shim.all_votes_for_block);
#else
  GTEST_SKIP() << "PillarVotes shim is disabled";
#endif
}

TEST(PillarVotesShimTest, paritySameVoterConflictingVoteRejection) {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  const auto legacy = runSameVoterConflictScenario<taraxa::pillar_chain::PillarVotesOld>();
  const auto shim = runSameVoterConflictScenario<taraxa::pillar_chain::PillarVotes>();

  EXPECT_TRUE(legacy.first_unique);
  EXPECT_FALSE(legacy.second_unique);
  EXPECT_TRUE(legacy.first_added);
  EXPECT_FALSE(legacy.second_added);
  EXPECT_EQ(legacy.votes_for_first_block, 1u);
  EXPECT_EQ(legacy.votes_for_second_block, 0u);

  EXPECT_TRUE(shim.first_unique);
  EXPECT_FALSE(shim.second_unique);
  EXPECT_TRUE(shim.first_added);
  EXPECT_FALSE(shim.second_added);
  EXPECT_EQ(shim.votes_for_first_block, 1u);
  EXPECT_EQ(shim.votes_for_second_block, 0u);

  EXPECT_EQ(legacy.first_unique, shim.first_unique);
  EXPECT_EQ(legacy.second_unique, shim.second_unique);
  EXPECT_EQ(legacy.first_added, shim.first_added);
  EXPECT_EQ(legacy.second_added, shim.second_added);
  EXPECT_EQ(legacy.votes_for_first_block, shim.votes_for_first_block);
  EXPECT_EQ(legacy.votes_for_second_block, shim.votes_for_second_block);
#else
  GTEST_SKIP() << "PillarVotes shim is disabled";
#endif
}

TEST(PillarVotesShimTest, parityDistinctWeightAboveThresholdSelection) {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  const auto legacy = runAboveThresholdScenario<taraxa::pillar_chain::PillarVotesOld>();
  const auto shim = runAboveThresholdScenario<taraxa::pillar_chain::PillarVotes>();

  EXPECT_EQ(legacy.all_votes, 3u);
  EXPECT_EQ(shim.all_votes, 3u);
  EXPECT_EQ(legacy.above_threshold_votes, 2u);
  EXPECT_EQ(shim.above_threshold_votes, 2u);
  EXPECT_TRUE(legacy.contains_heaviest_votes);
  EXPECT_TRUE(shim.contains_heaviest_votes);
  EXPECT_TRUE(legacy.excludes_lowest_vote);
  EXPECT_TRUE(shim.excludes_lowest_vote);

  EXPECT_EQ(legacy.all_votes, shim.all_votes);
  EXPECT_EQ(legacy.above_threshold_votes, shim.above_threshold_votes);
  EXPECT_EQ(legacy.contains_heaviest_votes, shim.contains_heaviest_votes);
  EXPECT_EQ(legacy.excludes_lowest_vote, shim.excludes_lowest_vote);
#else
  GTEST_SKIP() << "PillarVotes shim is disabled";
#endif
}

TEST(PillarVotesShimTest, parityCleanup) {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  const auto legacy = runCleanupScenario<taraxa::pillar_chain::PillarVotesOld>();
  const auto shim = runCleanupScenario<taraxa::pillar_chain::PillarVotes>();

  EXPECT_TRUE(legacy.previous_period_before_cleanup);
  EXPECT_FALSE(legacy.previous_period_after_cleanup);
  EXPECT_TRUE(legacy.current_period_after_cleanup);
  EXPECT_TRUE(legacy.previous_period_votes_removed);
  EXPECT_TRUE(legacy.current_period_votes_kept);

  EXPECT_TRUE(shim.previous_period_before_cleanup);
  EXPECT_FALSE(shim.previous_period_after_cleanup);
  EXPECT_TRUE(shim.current_period_after_cleanup);
  EXPECT_TRUE(shim.previous_period_votes_removed);
  EXPECT_TRUE(shim.current_period_votes_kept);

  EXPECT_EQ(legacy.previous_period_before_cleanup, shim.previous_period_before_cleanup);
  EXPECT_EQ(legacy.previous_period_after_cleanup, shim.previous_period_after_cleanup);
  EXPECT_EQ(legacy.current_period_after_cleanup, shim.current_period_after_cleanup);
  EXPECT_EQ(legacy.previous_period_votes_removed, shim.previous_period_votes_removed);
  EXPECT_EQ(legacy.current_period_votes_kept, shim.current_period_votes_kept);
#else
  GTEST_SKIP() << "PillarVotes shim is disabled";
#endif
}

TEST(PillarVotesShimTest, emptyIndexApiContractAndCleanup) {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  taraxa::pillar_chain::PillarVotes pillar_votes;
  const auto period = PbftPeriod{10};
  const auto threshold = uint64_t{3};
  const secret_t validator_secret = secret_t::random();

  EXPECT_FALSE(pillar_votes.periodDataInitialized(period));
  pillar_votes.initializePeriodData(period, threshold);
  EXPECT_TRUE(pillar_votes.periodDataInitialized(period));

  const auto vote_a = std::make_shared<PillarVote>(validator_secret, period, blk_hash_t(1));
  const auto vote_b = std::make_shared<PillarVote>(secret_t::random(), period, blk_hash_t(1));
  const auto vote_c = std::make_shared<PillarVote>(validator_secret, period, blk_hash_t(2));

  EXPECT_TRUE(pillar_votes.isUniqueVote(vote_a));
  EXPECT_TRUE(pillar_votes.addVerifiedVote(vote_a, 2));
  EXPECT_TRUE(pillar_votes.voteExists(vote_a));

  EXPECT_TRUE(pillar_votes.isUniqueVote(vote_b));
  EXPECT_TRUE(pillar_votes.addVerifiedVote(vote_b, 1));
  EXPECT_TRUE(pillar_votes.voteExists(vote_b));

  const auto votes = pillar_votes.getVerifiedVotes(period, blk_hash_t(1), false);
  EXPECT_EQ(votes.size(), 2);

  const auto above_threshold_votes = pillar_votes.getVerifiedVotes(period, blk_hash_t(1), true);
  EXPECT_EQ(above_threshold_votes.size(), 2);

  EXPECT_FALSE(pillar_votes.isUniqueVote(vote_c));
  EXPECT_FALSE(pillar_votes.addVerifiedVote(vote_c, 1));

  pillar_votes.eraseVotes(period + 1);
  EXPECT_FALSE(pillar_votes.periodDataInitialized(period));
  EXPECT_FALSE(pillar_votes.voteExists(vote_a));
  EXPECT_FALSE(pillar_votes.voteExists(vote_b));
  EXPECT_TRUE(pillar_votes.getVerifiedVotes(period, blk_hash_t(1), false).empty());

  EXPECT_FALSE(pillar_votes.periodDataInitialized(period + 1));
  EXPECT_TRUE(pillar_votes.getVerifiedVotes(period + 1, blk_hash_t(2), false).empty());
#else
  GTEST_SKIP() << "PillarVotes shim is disabled";
#endif
}

}  // namespace taraxa::core_tests
