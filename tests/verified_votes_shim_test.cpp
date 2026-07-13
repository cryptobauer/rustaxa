#include <gtest/gtest.h>

#include <type_traits>
#include <utility>

#include "vote_manager/verified_votes.hpp"
#include "vote_manager/vote_manager.hpp"

namespace taraxa::core_tests {

TEST(VerifiedVotesShimTest, rustModeVerifiedVotesDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_VERIFIED_VOTES
  static_assert(!std::is_base_of_v<VerifiedVotesOld, VerifiedVotes>);
  SUCCEED();
#else
  GTEST_SKIP() << "VerifiedVotes shim is disabled";
#endif
}

TEST(VerifiedVotesShimTest, rustModeVoteManagerDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE
  static_assert(!std::is_base_of_v<VoteManagerOld, VoteManager>);
  SUCCEED();
#else
  GTEST_SKIP() << "VoteManager shim is disabled";
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
  const auto egress_plan = verified_votes.planNextVotesBundleEgress(1, 1);
  EXPECT_EQ(egress_plan.status, 0);
  EXPECT_FALSE(egress_plan.next_votes.found);
  EXPECT_FALSE(egress_plan.next_null_votes.found);

  rustaxa::PbftOptimizedVoteBundleBuildRequest request{};
  request.kind = static_cast<uint8_t>(TwoTPlusOneVotedBlockType::NextVotedBlock);
  request.period = 1;
  request.round = 1;
  const auto empty_build = verified_votes.buildOptimizedVotesBundleEgress(std::move(request));
  EXPECT_EQ(empty_build.status, 2);
  EXPECT_TRUE(empty_build.votes_bundle_rlp.empty());

  verified_votes.cleanupVotesByPeriod(10);
  EXPECT_EQ(verified_votes.size(), 0);
}

}  // namespace taraxa::core_tests
