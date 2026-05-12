#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"
#include "vote/pillar_vote.hpp"

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

rustaxa::PillarVoteRelevanceFact makeRelevanceFact(uint64_t vote_period, uint64_t vote_block_hash,
                                                   bool has_current_pillar_block, uint64_t current_pillar_block_period,
                                                   uint64_t current_pillar_block_hash,
                                                   bool vote_already_known = false) {
  rustaxa::PillarVoteRelevanceFact fact{};
  fact.vote_period = vote_period;
  fact.vote_block_hash = makeHash(vote_block_hash);
  fact.has_current_pillar_block = has_current_pillar_block;
  fact.current_pillar_block_period = current_pillar_block_period;
  fact.current_pillar_block_hash = makeHash(current_pillar_block_hash);
  fact.first_pillar_block_period = 10;
  fact.pillar_blocks_interval = 10;
  fact.vote_already_known = vote_already_known;
  return fact;
}

rust::Slice<const uint8_t> makeSlice(const taraxa::bytes& bytes) {
  return rust::Slice<const uint8_t>(bytes.data(), bytes.size());
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
  EXPECT_EQ(plan.accepted_votes[0].weight, 4);
  EXPECT_EQ(plan.accepted_votes[1].vote_hash, makeHash(12));
  EXPECT_EQ(plan.accepted_votes[1].weight, 3);
  EXPECT_EQ(plan.accepted_votes[2].vote_hash, makeHash(13));
  EXPECT_EQ(plan.accepted_votes[2].weight, 2);
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

TEST(PillarVoteRelevanceBridgeTest, planPillarVoteRelevanceMatchesManagerPeriodRules) {
  auto first_vote = rustaxa::plan_pillar_vote_relevance(makeRelevanceFact(11, 1111, false, 0, 0));
  EXPECT_TRUE(first_vote.is_relevant);
  EXPECT_EQ(first_vote.status, 0);

  auto wrong_first_vote = rustaxa::plan_pillar_vote_relevance(makeRelevanceFact(12, 1111, false, 0, 0));
  EXPECT_FALSE(wrong_first_vote.is_relevant);
  EXPECT_EQ(wrong_first_vote.status, 2);

  auto next_period_wrong_hash = rustaxa::plan_pillar_vote_relevance(makeRelevanceFact(21, 2222, true, 20, 3333));
  EXPECT_FALSE(next_period_wrong_hash.is_relevant);
  EXPECT_EQ(next_period_wrong_hash.status, 4);

  auto future_period = rustaxa::plan_pillar_vote_relevance(makeRelevanceFact(31, 2222, true, 20, 3333));
  EXPECT_TRUE(future_period.is_relevant);
  EXPECT_EQ(future_period.status, 0);
}

TEST(PillarVoteRelevanceBridgeTest, planPillarVoteRelevanceRejectsKnownVote) {
  auto known_vote = rustaxa::plan_pillar_vote_relevance(makeRelevanceFact(31, 2222, true, 20, 3333, true));

  EXPECT_FALSE(known_vote.is_relevant);
  EXPECT_EQ(known_vote.status, 1);
}

TEST(PillarVoteInspectionBridgeTest, inspectPillarVoteRecoversSameVoterAsCpp) {
  const auto secret = taraxa::secret_t("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd");
  const taraxa::PbftPeriod period{123};
  const taraxa::blk_hash_t block_hash{456};
  const taraxa::PillarVote vote(secret, period, block_hash);
  const auto vote_rlp = vote.rlp();

  const auto inspection = rustaxa::pillar_vote_inspect(makeSlice(vote_rlp));

  EXPECT_EQ(inspection.status, 0);
  EXPECT_TRUE(inspection.signature_valid);
  EXPECT_EQ(inspection.period, period);
  EXPECT_EQ(inspection.block_hash, block_hash.asArray());
  EXPECT_EQ(inspection.vote_hash, vote.getHash().asArray());
  EXPECT_EQ(inspection.voter, vote.getVoterAddr().asArray());
}

TEST(PillarVoteInspectionBridgeTest, inspectPillarVoteRejectsOutOfRangeRecoveryId) {
  const auto secret = taraxa::secret_t("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd");
  taraxa::PillarVote vote(secret, 124, taraxa::blk_hash_t{457});
  auto signature = vote.getVoteSignature();
  signature[64] = 4;
  const taraxa::PillarVote malformed_vote(vote.getPeriod(), vote.getBlockHash(), std::move(signature));
  const auto vote_rlp = malformed_vote.rlp();

  const auto inspection = rustaxa::pillar_vote_inspect(makeSlice(vote_rlp));

  EXPECT_EQ(inspection.status, 1);
  EXPECT_FALSE(inspection.signature_valid);
  const std::array<uint8_t, 20> zero_address{};
  EXPECT_EQ(inspection.voter, zero_address);
}

}  // namespace rustaxa::core_tests
