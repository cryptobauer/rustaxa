#include <gtest/gtest.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <utility>
#include <vector>

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

rustaxa::PillarValidatorVoteCount makeVoteCount(uint8_t address, uint64_t vote_count) {
  rustaxa::PillarValidatorVoteCount fact{};
  fact.address = makeVoter(address);
  fact.vote_count = vote_count;
  return fact;
}

rustaxa::PillarBlockLinkageFact makeLinkageFact(uint64_t period, uint64_t previous_hash, bool has_last_finalized,
                                                uint64_t last_period, uint64_t last_hash) {
  rustaxa::PillarBlockLinkageFact fact{};
  fact.pillar_block_period = period;
  fact.pillar_block_previous_hash = makeHash(previous_hash);
  fact.first_pillar_block_period = 10;
  fact.pillar_blocks_interval = 10;
  fact.has_last_finalized_pillar_block = has_last_finalized;
  fact.last_finalized_period = last_period;
  fact.last_finalized_hash = makeHash(last_hash);
  return fact;
}

rust::Slice<const uint8_t> makeSlice(const taraxa::bytes& bytes) {
  return rust::Slice<const uint8_t>(bytes.data(), bytes.size());
}

rust::Vec<uint8_t> makeBytes(const taraxa::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
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

TEST(PillarVoteBundleBridgeTest, inspectPillarVoteBundleRlpsReturnsRecoveredVoters) {
  const auto first_secret = taraxa::secret_t("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd");
  const auto second_secret = taraxa::secret_t("0b8f2d8f2b753f9d6eebcc334d79c8d0e9cfdd4457f0327f3a30a2d8a7f1f7cd");
  const taraxa::PbftPeriod period{123};
  const taraxa::blk_hash_t block_hash{456};
  const taraxa::PillarVote first_vote(first_secret, period, block_hash);
  const taraxa::PillarVote second_vote(second_secret, period, block_hash);
  rust::Vec<rustaxa::PillarVoteRlpPayload> votes;
  votes.reserve(2);
  rustaxa::PillarVoteRlpPayload first_payload;
  first_payload.vote_rlp = makeBytes(first_vote.rlp());
  votes.push_back(std::move(first_payload));
  rustaxa::PillarVoteRlpPayload second_payload;
  second_payload.vote_rlp = makeBytes(second_vote.rlp());
  votes.push_back(std::move(second_payload));

  const auto plan = rustaxa::inspect_pillar_vote_bundle_rlps(std::move(votes));

  EXPECT_EQ(plan.status, 0);
  ASSERT_EQ(plan.inspections.size(), 2);
  EXPECT_EQ(plan.inspections[0].vote_hash, first_vote.getHash().asArray());
  EXPECT_EQ(plan.inspections[0].voter, first_vote.getVoterAddr().asArray());
  EXPECT_EQ(plan.inspections[1].vote_hash, second_vote.getHash().asArray());
  EXPECT_EQ(plan.inspections[1].voter, second_vote.getVoterAddr().asArray());
}

TEST(PillarVoteBundleBridgeTest, planPillarVoteBundleFromWeightedRlpsReturnsAcceptedVoters) {
  const auto first_secret = taraxa::secret_t("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd");
  const auto second_secret = taraxa::secret_t("0b8f2d8f2b753f9d6eebcc334d79c8d0e9cfdd4457f0327f3a30a2d8a7f1f7cd");
  const taraxa::PbftPeriod period{124};
  const taraxa::blk_hash_t block_hash{457};
  const taraxa::PillarVote first_vote(first_secret, period, block_hash);
  const taraxa::PillarVote second_vote(second_secret, period, block_hash);
  rust::Vec<rustaxa::PillarVoteWeightedRlpPayload> votes;
  votes.reserve(2);
  rustaxa::PillarVoteWeightedRlpPayload first_payload;
  first_payload.vote_rlp = makeBytes(first_vote.rlp());
  first_payload.weight = 4;
  votes.push_back(std::move(first_payload));
  rustaxa::PillarVoteWeightedRlpPayload second_payload;
  second_payload.vote_rlp = makeBytes(second_vote.rlp());
  second_payload.weight = 3;
  votes.push_back(std::move(second_payload));

  const auto plan =
      rustaxa::plan_pillar_vote_bundle_from_weighted_rlps(std::move(votes), period, block_hash.asArray(), 7);

  EXPECT_EQ(plan.status, 0);
  EXPECT_EQ(plan.block_weight, 7);
  EXPECT_EQ(plan.selected_weight, 7);
  ASSERT_EQ(plan.accepted_votes.size(), 2);
  EXPECT_EQ(plan.accepted_votes[0].weight + plan.accepted_votes[1].weight, 7);
  std::vector<std::array<uint8_t, 20>> voters{plan.accepted_votes[0].voter, plan.accepted_votes[1].voter};
  EXPECT_TRUE(std::find(voters.begin(), voters.end(), first_vote.getVoterAddr().asArray()) != voters.end());
  EXPECT_TRUE(std::find(voters.begin(), voters.end(), second_vote.getVoterAddr().asArray()) != voters.end());
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

TEST(PillarChainPlanningBridgeTest, planPillarVoteCountChangesMatchesLegacyOrdering) {
  rust::Vec<rustaxa::PillarValidatorVoteCount> current;
  current.push_back(makeVoteCount(3, 9));
  current.push_back(makeVoteCount(1, 3));
  current.push_back(makeVoteCount(4, 4));

  rust::Vec<rustaxa::PillarValidatorVoteCount> previous;
  previous.push_back(makeVoteCount(3, 5));
  previous.push_back(makeVoteCount(2, 8));
  previous.push_back(makeVoteCount(1, 3));

  const auto changes = rustaxa::plan_pillar_vote_count_changes(std::move(current), std::move(previous));

  ASSERT_EQ(changes.size(), 3);
  EXPECT_EQ(changes[0].address, makeVoter(2));
  EXPECT_EQ(changes[0].vote_count_change, -8);
  EXPECT_EQ(changes[1].address, makeVoter(3));
  EXPECT_EQ(changes[1].vote_count_change, 4);
  EXPECT_EQ(changes[2].address, makeVoter(4));
  EXPECT_EQ(changes[2].vote_count_change, 4);
}

TEST(PillarChainPlanningBridgeTest, planPillarVoteCountChangesPreservesFirstBlockOrder) {
  rust::Vec<rustaxa::PillarValidatorVoteCount> current;
  current.push_back(makeVoteCount(3, 9));
  current.push_back(makeVoteCount(1, 3));

  rust::Vec<rustaxa::PillarValidatorVoteCount> previous;

  const auto changes = rustaxa::plan_pillar_vote_count_changes(std::move(current), std::move(previous));

  ASSERT_EQ(changes.size(), 2);
  EXPECT_EQ(changes[0].address, makeVoter(3));
  EXPECT_EQ(changes[0].vote_count_change, 9);
  EXPECT_EQ(changes[1].address, makeVoter(1));
  EXPECT_EQ(changes[1].vote_count_change, 3);
}

TEST(PillarChainPlanningBridgeTest, planPillarBlockLinkageReportsStatus) {
  const auto first = rustaxa::plan_pillar_block_linkage(makeLinkageFact(10, 0, false, 0, 0));
  EXPECT_TRUE(first.valid);
  EXPECT_EQ(first.status, 1);

  const auto valid_next = rustaxa::plan_pillar_block_linkage(makeLinkageFact(20, 777, true, 10, 777));
  EXPECT_TRUE(valid_next.valid);
  EXPECT_EQ(valid_next.status, 0);
  EXPECT_EQ(valid_next.expected_previous_period, 20);

  const auto wrong_hash = rustaxa::plan_pillar_block_linkage(makeLinkageFact(20, 778, true, 10, 777));
  EXPECT_FALSE(wrong_hash.valid);
  EXPECT_EQ(wrong_hash.status, 4);
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
