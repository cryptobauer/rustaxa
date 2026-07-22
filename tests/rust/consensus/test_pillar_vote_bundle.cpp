#include <gtest/gtest.h>
#include <libdevcore/RLP.h>
#include <libdevcore/SHA3.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <filesystem>
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
