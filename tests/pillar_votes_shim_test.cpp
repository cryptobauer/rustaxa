#include <gtest/gtest.h>

#include <type_traits>

#include "pillar_chain/pillar_votes.hpp"

namespace taraxa::core_tests {

TEST(PillarVotesShimTest, rustModePillarVotesDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_PILLAR_VOTES
  static_assert(!std::is_base_of_v<taraxa::pillar_chain::PillarVotesOld, taraxa::pillar_chain::PillarVotes>);
  SUCCEED();
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
