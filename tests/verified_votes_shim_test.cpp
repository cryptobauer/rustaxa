#include <gtest/gtest.h>

#include <type_traits>

#include "vote_manager/verified_votes.hpp"

namespace taraxa::core_tests {

TEST(VerifiedVotesShimTest, rustModeVerifiedVotesDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_VERIFIED_VOTES
  static_assert(!std::is_base_of_v<VerifiedVotesOld, VerifiedVotes>);
  SUCCEED();
#else
  GTEST_SKIP() << "VerifiedVotes shim is disabled";
#endif
}

TEST(VerifiedVotesShimTest, emptyIndexApiContract) {
  VerifiedVotes verified_votes(addr_t{});

  EXPECT_EQ(verified_votes.size(), 0);
  EXPECT_TRUE(verified_votes.votes().empty());
  EXPECT_FALSE(verified_votes.getPeriodVotes(1).has_value());
  EXPECT_FALSE(verified_votes.getRoundVotes(1, 1).has_value());
  EXPECT_FALSE(verified_votes.getStepVotes(1, 1, 1).has_value());
  EXPECT_FALSE(verified_votes.getTwoTPlusOneVotedBlock(1, 1, TwoTPlusOneVotedBlockType::SoftVotedBlock).has_value());
  EXPECT_TRUE(verified_votes.getTwoTPlusOneVotedBlockVotes(1, 1, TwoTPlusOneVotedBlockType::SoftVotedBlock).empty());

  verified_votes.cleanupVotesByPeriod(10);
  EXPECT_EQ(verified_votes.size(), 0);
}

}  // namespace taraxa::core_tests

