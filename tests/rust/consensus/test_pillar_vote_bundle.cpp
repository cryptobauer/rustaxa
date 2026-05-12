#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"

namespace rustaxa::core_tests {

namespace {

std::array<uint8_t, 32> makeHash(uint64_t value) {
  std::array<uint8_t, 32> out{};
  for (auto index = 0; index < 8; ++index) {
    out[31 - index] = static_cast<uint8_t>(value & 0xff);
    value >>= 8;
  }
  return out;
}

std::array<uint8_t, 20> makeVoter(uint8_t first_byte) {
  std::array<uint8_t, 20> out{};
  out.fill(first_byte);
  return out;
}

rustaxa::PillarVoteBundleFact makeFact(uint64_t vote_hash, uint64_t block_hash, uint8_t voter, uint64_t period,
                                       uint64_t weight, bool prevalidated = true) {
  rustaxa::PillarVoteBundleFact fact{};
  fact.vote_hash = makeHash(vote_hash);
  fact.block_hash = makeHash(block_hash);
  fact.voter = makeVoter(voter);
  fact.period = period;
  fact.weight = weight;
  fact.prevalidated = prevalidated;
  return fact;
}

}  // namespace

TEST(PillarVoteBundleBridgeTest, planPillarVoteBundleMatchesExpectedStatuses) {
  rust::Vec<rustaxa::PillarVoteBundleFact> facts;
  facts.reserve(3);
  facts.push_back(makeFact(11, 1234, 1, 40, 4));
  facts.push_back(makeFact(12, 1234, 2, 40, 3));
  facts.push_back(makeFact(13, 1234, 3, 40, 2));

  auto plan = rustaxa::plan_pillar_vote_bundle(std::move(facts), 40, makeHash(1234), 7);

  EXPECT_EQ(plan.status, 0);
  EXPECT_EQ(plan.block_weight, 9);
  EXPECT_EQ(plan.selected_weight, 7);
  EXPECT_EQ(plan.accepted_votes.size(), 3);
  EXPECT_EQ(plan.accepted_votes[0].vote_hash, makeHash(11));
  EXPECT_EQ(plan.accepted_votes[1].vote_hash, makeHash(12));
  EXPECT_EQ(plan.accepted_votes[2].vote_hash, makeHash(13));
}

TEST(PillarVoteBundleBridgeTest, planPillarVoteBundleRejectsMismatchedBlockHash) {
  rust::Vec<rustaxa::PillarVoteBundleFact> facts;
  facts.push_back(makeFact(21, 5555, 1, 40, 1));

  auto plan = rustaxa::plan_pillar_vote_bundle(std::move(facts), 40, makeHash(1234), 10);

  EXPECT_EQ(plan.status, 3);
  EXPECT_EQ(plan.first_bad_vote_hash, makeHash(21));
}

TEST(PillarVoteBundleBridgeTest, planPillarVoteBundleRejectsFailedPrevalidation) {
  rust::Vec<rustaxa::PillarVoteBundleFact> facts;
  facts.push_back(makeFact(31, 7777, 1, 42, 3, false));
  facts.push_back(makeFact(32, 7777, 2, 42, 2));

  auto plan = rustaxa::plan_pillar_vote_bundle(std::move(facts), 42, makeHash(7777), 10);

  EXPECT_EQ(plan.status, 4);
  EXPECT_EQ(plan.block_weight, 0);
  EXPECT_EQ(plan.selected_weight, 0);
  EXPECT_TRUE(plan.accepted_votes.empty());
  EXPECT_EQ(plan.first_bad_vote_hash, makeHash(31));
}

TEST(PillarVoteBundleBridgeTest, planPillarVoteBundleDoesNotRecountDuplicateVoteHash) {
  rust::Vec<rustaxa::PillarVoteBundleFact> facts;
  facts.push_back(makeFact(41, 9000, 1, 50, 5));
  facts.push_back(makeFact(41, 9000, 1, 50, 5));

  auto plan = rustaxa::plan_pillar_vote_bundle(std::move(facts), 50, makeHash(9000), 10);

  EXPECT_EQ(plan.status, 7);
  EXPECT_EQ(plan.block_weight, 5);
  EXPECT_EQ(plan.selected_weight, 0);
  EXPECT_TRUE(plan.accepted_votes.empty());
}

TEST(PillarVoteBundleBridgeTest, planPillarVoteBundleRejectsSameVoterConflict) {
  rust::Vec<rustaxa::PillarVoteBundleFact> facts;
  facts.push_back(makeFact(51, 9001, 1, 51, 5));
  facts.push_back(makeFact(52, 9001, 1, 51, 5));

  auto plan = rustaxa::plan_pillar_vote_bundle(std::move(facts), 51, makeHash(9001), 10);

  EXPECT_EQ(plan.status, 6);
  EXPECT_EQ(plan.first_bad_vote_hash, makeHash(52));
  EXPECT_EQ(plan.block_weight, 0);
}

}  // namespace rustaxa::core_tests
