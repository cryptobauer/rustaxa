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

rust::Box<rustaxa::BridgePbftService> createReadyPillarService(const rustaxa::BridgeStorage& storage) {
  auto service = rustaxa::create_pillar_capable_pbft_service_for_compatibility(storage);
  service->pbft_service_complete_pillar_bootstrap();
  return service;
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
  auto pillar_service = createReadyPillarService(*storage);
  pillar_service->pbft_service_pillar_apply_current_block_data(makeBytes(current_anchor.current_data_rlp));
  const auto plan = pillar_service->pbft_service_pillar_prepare_weighted_rlp_bundle(std::move(votes), period);

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
  auto pillar_service = createReadyPillarService(*storage);

  rustaxa::PillarCurrentAnchorDecisionRequest request{};
  request.operation = 0;
  request.has_candidate_hash = true;
  request.candidate_hash = current_anchor.hash.asArray();
  auto decision = pillar_service->pbft_service_pillar_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 1);
  EXPECT_FALSE(decision.selected);
  EXPECT_FALSE(decision.has_current_anchor);

  pillar_service->pbft_service_pillar_apply_current_block_data(makeBytes(current_anchor.current_data_rlp));
  decision = pillar_service->pbft_service_pillar_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 0);
  EXPECT_TRUE(decision.selected);
  EXPECT_TRUE(decision.has_current_anchor);
  EXPECT_EQ(decision.current_period, current_period);
  EXPECT_EQ(decision.current_hash, current_anchor.hash.asArray());
  EXPECT_NE(decision.anchor_generation, 0);

  request = {};
  request.operation = 1;
  request.pbft_period = current_period + 1;
  decision = pillar_service->pbft_service_pillar_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 0);
  EXPECT_TRUE(decision.selected);

  request.pbft_period = 0;
  decision = pillar_service->pbft_service_pillar_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 4);
  EXPECT_FALSE(decision.selected);

  request = {};
  request.operation = 2;
  request.pbft_period = current_period + 10;
  request.pillar_blocks_interval = 10;
  decision = pillar_service->pbft_service_pillar_plan_current_anchor_decision(request);
  EXPECT_EQ(decision.status, 0);
  EXPECT_TRUE(decision.selected);

  EXPECT_EQ(pillar_service->pbft_service_pillar_consensus_threshold(0), 1);
  EXPECT_EQ(pillar_service->pbft_service_pillar_consensus_threshold(10), 6);
  EXPECT_EQ(pillar_service->pbft_service_pillar_consensus_threshold(std::numeric_limits<uint64_t>::max()),
            std::numeric_limits<uint64_t>::max() / 2 + 1);
  std::filesystem::remove_all(test_dir);
}

TEST(PillarVoteBundleBridgeTest, preparePillarFinalizationReturnsMissingCurrentBlock) {
  const auto test_dir = tempStoragePath("rustaxa_pillar_finalize_prepare_missing_current");
  auto storage = rustaxa::create_storage(test_dir.string());
  auto pillar_service = createReadyPillarService(*storage);

  rustaxa::PillarBlockFinalizationRequest request{};
  request.requested_pillar_block_hash = {};

  const auto prepare = pillar_service->pbft_service_pillar_prepare_finalized_block_for_pbft(request);
  EXPECT_EQ(prepare.status, 1);
  EXPECT_FALSE(prepare.success);
  EXPECT_FALSE(prepare.should_emit);
  EXPECT_FALSE(prepare.has_prepared_pillar_block);
  EXPECT_FALSE(prepare.preparation_anchor_generation);
  EXPECT_FALSE(prepare.preparation_token);
  EXPECT_EQ(prepare.selected_vote_count, 0);
  EXPECT_FALSE(prepare.should_request_votes);
  EXPECT_FALSE(prepare.has_request_votes_period);
  std::filesystem::remove_all(test_dir);
}

TEST(PillarVoteBundleBridgeTest, preparePillarFinalizationWithCurrentBlockCanReachHashMismatchPath) {
  const auto current_anchor = makeCurrentPillarAnchor(100);
  const auto test_dir = tempStoragePath("rustaxa_pillar_finalize_prepare_hash_mismatch");
  auto storage = rustaxa::create_storage(test_dir.string());
  auto pillar_service = createReadyPillarService(*storage);
  pillar_service->pbft_service_pillar_apply_current_block_data(makeBytes(current_anchor.current_data_rlp));

  rustaxa::PillarBlockFinalizationRequest request{};
  request.requested_pillar_block_hash = current_anchor.hash.asArray();
  request.requested_pillar_block_hash[0] ^= 0xFF;
  const auto prepare = pillar_service->pbft_service_pillar_prepare_finalized_block_for_pbft(request);

  EXPECT_EQ(prepare.status, 2);
  EXPECT_FALSE(prepare.success);
  EXPECT_FALSE(prepare.should_emit);
  EXPECT_FALSE(prepare.has_prepared_pillar_block);
  EXPECT_FALSE(prepare.should_request_votes);
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
  auto pillar_service = createReadyPillarService(*storage);
  pillar_service->pbft_service_pillar_apply_current_block_data(makeBytes(current_anchor.current_data_rlp));

  rust::Vec<rustaxa::PillarVoteRlpPayload> vote_rlps;
  rustaxa::PillarVoteRlpPayload first_rlp;
  first_rlp.vote_rlp = makeBytes(first_vote.rlp());
  vote_rlps.push_back(std::move(first_rlp));
  rustaxa::PillarVoteRlpPayload second_rlp;
  second_rlp.vote_rlp = makeBytes(second_vote.rlp());
  vote_rlps.push_back(std::move(second_rlp));
  const auto prepared = pillar_service->pbft_service_pillar_prepare_weighted_rlp_bundle(std::move(vote_rlps), period);
  ASSERT_TRUE(prepared.can_query_dpos);

  rustaxa::PillarVoteWeightedBundleApplyInput input;
  input.votes = std::move(votes);
  input.required_votes_period = period;
  input.threshold = 7;
  input.anchor_generation = prepared.anchor_generation;
  const auto plan = pillar_service->pbft_service_pillar_apply_weighted_rlp_bundle(std::move(input));

  EXPECT_EQ(plan.status, 0);
  EXPECT_EQ(plan.block_weight, 7);
  EXPECT_EQ(plan.selected_weight, 7);
  EXPECT_FALSE(plan.insert_failed);
  EXPECT_EQ(plan.applied_votes, 2);

  const auto lookup =
      pillar_service->pbft_service_pillar_get_verified_vote_payloads(period, block_hash.asArray(), true);
  EXPECT_TRUE(lookup.threshold_met);
  EXPECT_EQ(lookup.selected_weight, 7);
  ASSERT_EQ(lookup.votes.size(), 2);
  EXPECT_EQ(lookup.votes[0].weight + lookup.votes[1].weight, 7);
  std::filesystem::remove_all(test_dir);
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
