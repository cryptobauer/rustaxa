#include <gtest/gtest.h>

#include <cstdint>
#include <type_traits>
#include <utility>

#include "storage/storage.hpp"
#include "test_util/test_util.hpp"
#include "vote_manager/verified_votes.hpp"
#include "vote_manager/vote_manager.hpp"

namespace taraxa::core_tests {

TEST(VerifiedVotesShimTest, compatibilityCarrierValuesAndDefaultsRemainStable) {
  static_assert(static_cast<uint8_t>(TwoTPlusOneVotedBlockType::SoftVotedBlock) == 0);
  static_assert(static_cast<uint8_t>(TwoTPlusOneVotedBlockType::CertVotedBlock) == 1);
  static_assert(static_cast<uint8_t>(TwoTPlusOneVotedBlockType::NextVotedBlock) == 2);
  static_assert(static_cast<uint8_t>(TwoTPlusOneVotedBlockType::NextVotedNullBlock) == 3);
  static_assert(std::is_same_v<TwoTVotedBlockMap, std::unordered_map<TwoTPlusOneVotedBlockType, VotedBlock>>);
  static_assert(std::is_same_v<decltype(VotedBlock::hash), blk_hash_t>);
  static_assert(std::is_same_v<decltype(VotedBlock::step), PbftStep>);
  static_assert(std::is_same_v<decltype(VotesWithWeight::weight), uint64_t>);
  static_assert(
      std::is_same_v<decltype(VotesWithWeight::votes), std::unordered_map<vote_hash_t, std::shared_ptr<PbftVote>>>);
  static_assert(
      std::is_same_v<UniqueVotersMap,
                     std::unordered_map<addr_t, std::pair<std::shared_ptr<PbftVote>, std::shared_ptr<PbftVote>>>>);
  static_assert(std::is_same_v<decltype(StepVotes::votes), std::unordered_map<blk_hash_t, VotesWithWeight>>);
  static_assert(std::is_same_v<decltype(StepVotes::unique_voters), UniqueVotersMap>);
  static_assert(std::is_same_v<StepVotesMap, std::map<PbftStep, StepVotes>>);
  static_assert(std::is_same_v<decltype(RoundVerifiedVotes::two_t_plus_one_voted_blocks_), TwoTVotedBlockMap>);
  static_assert(std::is_same_v<decltype(RoundVerifiedVotes::step_votes), StepVotesMap>);
  static_assert(std::is_same_v<decltype(RoundVerifiedVotes::network_t_plus_one_step), PbftStep>);
  static_assert(std::is_same_v<RoundVerifiedVotesMap, std::map<PbftRound, RoundVerifiedVotes>>);
  static_assert(std::is_same_v<PeriodVerifiedVotesMap, std::map<PbftPeriod, RoundVerifiedVotesMap>>);

  const VotedBlock voted_block{blk_hash_t{}, PbftStep{}};
  EXPECT_EQ(voted_block.hash, blk_hash_t{});
  EXPECT_EQ(voted_block.step, PbftStep{});

  const VotesWithWeight votes_with_weight{0, {}};
  EXPECT_EQ(votes_with_weight.weight, 0);
  EXPECT_TRUE(votes_with_weight.votes.empty());

  const StepVotes step_votes{std::unordered_map<blk_hash_t, VotesWithWeight>{}, UniqueVotersMap{}};
  EXPECT_TRUE(step_votes.votes.empty());
  EXPECT_TRUE(step_votes.unique_voters.empty());

  const RoundVerifiedVotes round_votes{TwoTVotedBlockMap{}, StepVotesMap{}, PbftStep{}};
  EXPECT_TRUE(round_votes.two_t_plus_one_voted_blocks_.empty());
  EXPECT_TRUE(round_votes.step_votes.empty());
  EXPECT_EQ(round_votes.network_t_plus_one_step, PbftStep{});
}

struct VerifiedVotesShimDataTest : WithDataDir {};

TEST_F(VerifiedVotesShimDataTest, emptyStorageBackedIndexApiContract) {
#ifdef RUSTAXA_ENABLE_VERIFIED_VOTES
  auto db = std::make_shared<DbStorage>(data_dir);
  VerifiedVotes verified_votes(addr_t{}, db->rustStorage());
#else
  VerifiedVotes verified_votes(addr_t{});
#endif

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
