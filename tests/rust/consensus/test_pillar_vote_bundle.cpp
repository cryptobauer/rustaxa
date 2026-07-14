#include <gtest/gtest.h>
#include <libdevcore/RLP.h>
#include <libdevcore/SHA3.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <filesystem>
#include <limits>
#include <string>
#include <utility>
#include <vector>

#include "common/encoding_solidity.hpp"
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

rustaxa::PillarBlockCreationFact makeCreationFact(uint64_t period, bool has_last_finalized, uint64_t last_period,
                                                  uint64_t last_hash) {
  rustaxa::PillarBlockCreationFact fact{};
  fact.pillar_block_period = period;
  fact.state_root = makeHash(0xA1);
  fact.bridge_root = makeHash(0xB2);
  fact.bridge_epoch = makeHash(0xC3);
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

struct CurrentPillarAnchorFixture {
  taraxa::blk_hash_t hash;
  taraxa::bytes current_data_rlp;
};

CurrentPillarAnchorFixture makeCurrentPillarAnchor(uint64_t period) {
  const taraxa::h256 state_root{};
  const taraxa::blk_hash_t previous_hash{};
  const taraxa::h256 bridge_root{};
  const taraxa::u256 epoch{};

  taraxa::bytes solidity;
  const auto prefix = taraxa::util::EncodingSolidity::pack(taraxa::util::EncodingSolidity::kStartPrefix);
  solidity.insert(solidity.end(), prefix.begin(), prefix.end());
  const auto body = taraxa::util::EncodingSolidity::pack(period, state_root, previous_hash, bridge_root, epoch);
  solidity.insert(solidity.end(), body.begin(), body.end());
  constexpr uint64_t kPillarBlockFields = 5;
  const auto array_position = (taraxa::util::EncodingSolidity::kStartPrefixSize + kPillarBlockFields) *
                              taraxa::util::EncodingSolidity::kWordSize;
  const auto empty_changes = taraxa::util::EncodingSolidity::pack(array_position, uint64_t{0});
  solidity.insert(solidity.end(), empty_changes.begin(), empty_changes.end());

  dev::RLPStream block(6);
  block << period << state_root << previous_hash << bridge_root << epoch;
  block.appendList(0);
  dev::RLPStream current_data(2);
  current_data.appendRaw(block.out());
  current_data.appendList(0);
  return {dev::sha3(solidity), current_data.out()};
}

std::filesystem::path tempStoragePath(const std::string& name) {
  const auto path = std::filesystem::temp_directory_path() / name;
  if (std::filesystem::exists(path)) {
    std::filesystem::remove_all(path);
  }
  return path;
}

}  // namespace

TEST(PillarVoteBundleBridgeTest, preparePillarVoteBundleReturnsRecoveredVotersAndGeneration) {
  const auto first_secret = taraxa::secret_t("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd");
  const auto second_secret = taraxa::secret_t("0b8f2d8f2b753f9d6eebcc334d79c8d0e9cfdd4457f0327f3a30a2d8a7f1f7cd");
  const taraxa::PbftPeriod period{123};
  const auto current_anchor = makeCurrentPillarAnchor(period - 1);
  const auto block_hash = current_anchor.hash;
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

  const auto test_dir = tempStoragePath("rustaxa_pillar_vote_bundle_prepare");
  auto storage = rustaxa::create_storage(test_dir.string());
  auto pillar_runtime = rustaxa::create_pillar_chain_runtime(*storage);
  pillar_runtime->pillar_chain_runtime_apply_current_block_data(makeBytes(current_anchor.current_data_rlp));
  const auto plan = pillar_runtime->pillar_chain_runtime_prepare_weighted_rlp_bundle(std::move(votes), period);

  EXPECT_EQ(plan.status, 0);
  EXPECT_TRUE(plan.can_query_dpos);
  EXPECT_NE(plan.anchor_generation, 0);
  EXPECT_EQ(plan.expected_block_hash, block_hash.asArray());
  ASSERT_EQ(plan.inspections.size(), 2);
  EXPECT_EQ(plan.inspections[0].vote_hash, first_vote.getHash().asArray());
  EXPECT_EQ(plan.inspections[0].voter, first_vote.getVoterAddr().asArray());
  EXPECT_EQ(plan.inspections[1].vote_hash, second_vote.getHash().asArray());
  EXPECT_EQ(plan.inspections[1].voter, second_vote.getVoterAddr().asArray());
  std::filesystem::remove_all(test_dir);
}

TEST(PillarVoteBundleBridgeTest, currentAnchorDecisionsAndThresholdUseRuntimeState) {
  const taraxa::PbftPeriod current_period{130};
  const auto current_anchor = makeCurrentPillarAnchor(current_period);
  const auto test_dir = tempStoragePath("rustaxa_pillar_current_anchor_decisions");
  auto storage = rustaxa::create_storage(test_dir.string());
  auto pillar_runtime = rustaxa::create_pillar_chain_runtime(*storage);

  rustaxa::PillarCurrentAnchorDecisionRequest request{};
  request.operation = 0;
  request.has_candidate_hash = true;
  request.candidate_hash = current_anchor.hash.asArray();
  auto decision = pillar_runtime->pillar_chain_runtime_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 1);
  EXPECT_FALSE(decision.selected);
  EXPECT_FALSE(decision.has_current_anchor);

  pillar_runtime->pillar_chain_runtime_apply_current_block_data(makeBytes(current_anchor.current_data_rlp));
  decision = pillar_runtime->pillar_chain_runtime_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 0);
  EXPECT_TRUE(decision.selected);
  EXPECT_TRUE(decision.has_current_anchor);
  EXPECT_EQ(decision.current_period, current_period);
  EXPECT_EQ(decision.current_hash, current_anchor.hash.asArray());
  EXPECT_NE(decision.anchor_generation, 0);

  request = {};
  request.operation = 1;
  request.pbft_period = current_period + 1;
  decision = pillar_runtime->pillar_chain_runtime_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 0);
  EXPECT_TRUE(decision.selected);

  request.pbft_period = 0;
  decision = pillar_runtime->pillar_chain_runtime_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 4);
  EXPECT_FALSE(decision.selected);

  request = {};
  request.operation = 2;
  request.pbft_period = current_period + 10;
  request.pillar_blocks_interval = 10;
  decision = pillar_runtime->pillar_chain_runtime_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 0);
  EXPECT_TRUE(decision.selected);

  EXPECT_EQ(pillar_runtime->pillar_chain_runtime_consensus_threshold(0), 1);
  EXPECT_EQ(pillar_runtime->pillar_chain_runtime_consensus_threshold(10), 6);
  EXPECT_EQ(pillar_runtime->pillar_chain_runtime_consensus_threshold(std::numeric_limits<uint64_t>::max()),
            std::numeric_limits<uint64_t>::max() / 2 + 1);
  std::filesystem::remove_all(test_dir);
}

TEST(PillarVoteBundleBridgeTest, applyPillarVoteBundleFromWeightedRlpsInsertsAcceptedVotes) {
  const auto first_secret = taraxa::secret_t("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd");
  const auto second_secret = taraxa::secret_t("0b8f2d8f2b753f9d6eebcc334d79c8d0e9cfdd4457f0327f3a30a2d8a7f1f7cd");
  const taraxa::PbftPeriod period{124};
  const auto current_anchor = makeCurrentPillarAnchor(period - 1);
  const auto block_hash = current_anchor.hash;
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

  const auto test_dir = tempStoragePath("rustaxa_pillar_vote_bundle_runtime");
  auto storage = rustaxa::create_storage(test_dir.string());
  auto pillar_runtime = rustaxa::create_pillar_chain_runtime(*storage);
  pillar_runtime->pillar_chain_runtime_apply_current_block_data(makeBytes(current_anchor.current_data_rlp));

  rust::Vec<rustaxa::PillarVoteRlpPayload> vote_rlps;
  rustaxa::PillarVoteRlpPayload first_rlp;
  first_rlp.vote_rlp = makeBytes(first_vote.rlp());
  vote_rlps.push_back(std::move(first_rlp));
  rustaxa::PillarVoteRlpPayload second_rlp;
  second_rlp.vote_rlp = makeBytes(second_vote.rlp());
  vote_rlps.push_back(std::move(second_rlp));
  const auto prepared = pillar_runtime->pillar_chain_runtime_prepare_weighted_rlp_bundle(std::move(vote_rlps), period);
  ASSERT_TRUE(prepared.can_query_dpos);

  rustaxa::PillarVoteWeightedBundleApplyInput input;
  input.votes = std::move(votes);
  input.required_votes_period = period;
  input.threshold = 7;
  input.anchor_generation = prepared.anchor_generation;
  const auto plan = pillar_runtime->pillar_chain_runtime_apply_weighted_rlp_bundle(std::move(input));

  EXPECT_EQ(plan.status, 0);
  EXPECT_EQ(plan.block_weight, 7);
  EXPECT_EQ(plan.selected_weight, 7);
  EXPECT_FALSE(plan.insert_failed);
  EXPECT_EQ(plan.applied_votes, 2);

  const auto lookup =
      pillar_runtime->pillar_chain_runtime_get_verified_vote_payloads(period, block_hash.asArray(), true);
  EXPECT_TRUE(lookup.threshold_met);
  EXPECT_EQ(lookup.selected_weight, 7);
  ASSERT_EQ(lookup.votes.size(), 2);
  EXPECT_EQ(lookup.votes[0].weight + lookup.votes[1].weight, 7);
  std::filesystem::remove_all(test_dir);
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

TEST(PillarChainPlanningBridgeTest, planPillarBlockCreationCombinesShellAndVoteCountChanges) {
  rust::Vec<rustaxa::PillarValidatorVoteCount> current;
  current.push_back(makeVoteCount(3, 9));
  current.push_back(makeVoteCount(1, 3));
  current.push_back(makeVoteCount(4, 4));

  rust::Vec<rustaxa::PillarValidatorVoteCount> previous;
  previous.push_back(makeVoteCount(3, 5));
  previous.push_back(makeVoteCount(2, 8));
  previous.push_back(makeVoteCount(1, 3));

  const auto plan = rustaxa::plan_pillar_block_creation_with_vote_counts(makeCreationFact(20, true, 10, 777),
                                                                         std::move(current), std::move(previous));

  EXPECT_TRUE(plan.valid);
  EXPECT_EQ(plan.status, 0);
  EXPECT_EQ(plan.previous_pillar_block_hash, makeHash(777));
  EXPECT_EQ(plan.state_root, makeHash(0xA1));
  EXPECT_EQ(plan.bridge_root, makeHash(0xB2));
  EXPECT_EQ(plan.bridge_epoch, makeHash(0xC3));
  ASSERT_EQ(plan.vote_count_changes.size(), 3);
  EXPECT_EQ(plan.vote_count_changes[0].address, makeVoter(2));
  EXPECT_EQ(plan.vote_count_changes[0].vote_count_change, -8);
  EXPECT_EQ(plan.vote_count_changes[1].address, makeVoter(3));
  EXPECT_EQ(plan.vote_count_changes[1].vote_count_change, 4);
  EXPECT_EQ(plan.vote_count_changes[2].address, makeVoter(4));
  EXPECT_EQ(plan.vote_count_changes[2].vote_count_change, 4);
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
