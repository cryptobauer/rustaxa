#include <gtest/gtest.h>
#include <libdevcore/SHA3.h>

#include <type_traits>

#include "common/init.hpp"
#include "logger/logger.hpp"
#include "network/network.hpp"
#include "network/tarcap/packets_handlers/latest/vote_packet_handler.hpp"
#include "pbft/pbft_manager.hpp"
#include "test_util/test_util.hpp"

#ifndef RUSTAXA_ENABLE
namespace taraxa::core_tests {
using namespace vrf_wrapper;

auto g_vrf_sk = Lazy([] {
  return vrf_sk_t(
      "0b6627a6680e01cea3d9f36fa797f7f34e8869c3a526d9ed63ed8170e35542aad05dc12c"
      "1df1edc9f3367fba550b7971fc2de6c5998d8784051c5be69abc9644");
});
auto g_sk = Lazy([] {
  return secret_t("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                  dev::Secret::ConstructFromStringType::FromHex);
});
struct VoteTest : NodesTest {};

TEST(VoteManagerCarrierTest, verifiedVoteViewValuesShapesAndDefaultsRemainStable) {
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

  const VotedBlock voted_block{};
  EXPECT_EQ(voted_block.hash, blk_hash_t{});
  EXPECT_EQ(voted_block.step, PbftStep{});
  const VotesWithWeight votes_with_weight{};
  EXPECT_EQ(votes_with_weight.weight, 0);
  EXPECT_TRUE(votes_with_weight.votes.empty());
  const StepVotes step_votes{};
  EXPECT_TRUE(step_votes.votes.empty());
  EXPECT_TRUE(step_votes.unique_voters.empty());
  const RoundVerifiedVotes round_votes{};
  EXPECT_TRUE(round_votes.two_t_plus_one_voted_blocks_.empty());
  EXPECT_TRUE(round_votes.step_votes.empty());
  EXPECT_EQ(round_votes.network_t_plus_one_step, PbftStep{});
}

TEST_F(VoteTest, verified_votes) {
  auto node = create_nodes(1, true /*start*/).front();

  // stop PBFT manager, that will place vote
  node->getPbftManager()->stop();

  auto [period, round] = clearAllVotes({node});
  std::cout << "[TODO REMOVE] Clear all votes returned period " << period << ", round " << round << std::endl;

  // Generate a vote
  blk_hash_t blockhash(1);
  PbftVoteTypes type = PbftVoteTypes::soft_vote;
  PbftStep step = 2;
  auto vote =
      node->getVoteManager()->generateVote(blockhash, type, period, round, step, node->getConfig().getFirstWallet());
  vote->calculateWeight(1, 1, 1);

  auto vote_mgr = node->getVoteManager();
  vote_mgr->addVerifiedVote(vote);
  EXPECT_TRUE(vote_mgr->voteInVerifiedMap(vote));
  // Test same vote cannot add twice
  vote_mgr->addVerifiedVote(vote);
  EXPECT_EQ(vote_mgr->getVerifiedVotesSize(), 1);
  EXPECT_EQ(vote_mgr->getVerifiedVotes().size(), 1);

  auto [period2, round2] = clearAllVotes({node});
  std::cout << "[TODO REMOVE] Clear all votes returned period " << period2 << ", round " << round2 << std::endl;

  EXPECT_FALSE(vote_mgr->voteInVerifiedMap(vote));
  EXPECT_EQ(vote_mgr->getVerifiedVotesSize(), 0);
  EXPECT_EQ(vote_mgr->getVerifiedVotes().size(), 0);
}

TEST_F(VoteTest, add_preverified_weight_vote) {
  auto node = create_nodes(1, true /*start*/).front();
  auto vote_mgr = node->getVoteManager();
  auto pbft_mgr = node->getPbftManager();
  pbft_mgr->stop();

  clearAllVotes({node});
  const auto [current_round, current_period] = pbft_mgr->getPbftRoundAndPeriod();
  const auto &wallet = node->getConfig().getFirstWallet();

  auto weighted_vote = vote_mgr->generateVoteWithWeight(blk_hash_t(89), PbftVoteTypes::propose_vote, current_period,
                                                        current_round, 1, wallet);
  ASSERT_NE(weighted_vote, nullptr);
  ASSERT_TRUE(weighted_vote->getWeight().has_value());
  ASSERT_GT(*weighted_vote->getWeight(), 0);
  EXPECT_TRUE(vote_mgr->addVerifiedVote(weighted_vote));
}

TEST_F(VoteTest, round_determine_from_next_votes) {
  auto node = create_nodes(1, true /*start*/).front();

  auto pbft_mgr = node->getPbftManager();
  auto vote_mgr = node->getVoteManager();

  // stop PBFT manager, that will place vote
  pbft_mgr->stop();
  clearAllVotes({node});

  const auto [current_round, current_period] = pbft_mgr->getPbftRoundAndPeriod();

  // Generate votes for a few future rounds
  blk_hash_t voted_block_hash(1);
  PbftVoteTypes type = PbftVoteTypes::next_vote;
  const PbftRound kMaxRound = current_round + 3;
  PbftStep step = 5;
  for (PbftRound round = current_round; round <= kMaxRound; round++) {
    auto vote =
        vote_mgr->generateVote(voted_block_hash, type, current_period, round, step, node->getConfig().getFirstWallet());
    vote->calculateWeight(3, 3, 3);
    vote_mgr->addVerifiedVote(vote);
  }

  auto new_round = vote_mgr->determineNewRound(current_period, kMaxRound);
  EXPECT_EQ(new_round.has_value(), true);
  EXPECT_EQ(*new_round, kMaxRound + 1);
}

TEST_F(VoteTest, reconstruct_votes) {
  public_t pk(12345);
  sig_t sortition_sig(1234567);
  sig_t vote_sig(9878766);
  blk_hash_t propose_blk_hash(111111);
  PbftVoteTypes type(PbftVoteTypes::propose_vote);
  PbftPeriod period(999);
  PbftRound round(999);
  PbftStep step(1);
  VrfPbftMsg msg(type, period, round, step);
  VrfPbftSortition vrf_sortition(g_vrf_sk, msg);
  PbftVote vote1(g_sk, vrf_sortition, propose_blk_hash);
  auto rlp = vote1.rlp();
  PbftVote vote2(rlp);
  EXPECT_EQ(vote1, vote2);
}

TEST_F(VoteTest, rust_generated_own_vote_materializes_persists_and_reloads) {
  auto node = create_nodes(1, true /*start*/).front();
  node->getPbftManager()->stop();
  clearAllVotes({node});

  auto vote = node->getVoteManager()->generateVoteWithWeight(blk_hash_t(9), PbftVoteTypes::propose_vote, 1, 1, 1,
                                                             node->getConfig().getFirstWallet());
  ASSERT_NE(vote, nullptr);
  ASSERT_TRUE(vote->getWeight().has_value());
  EXPECT_GT(*vote->getWeight(), 0);
  EXPECT_TRUE(vote->getCredential());
  const auto weighted_vote_rlp = vote->rlp(true, true);
  const auto gossip_vote_rlp = vote->rlp(true, false);
  EXPECT_NE(gossip_vote_rlp, weighted_vote_rlp);

  node->getVoteManager()->saveOwnVerifiedVote(vote);
  const auto own_votes = node->getVoteManager()->getOwnVerifiedVotes();
  ASSERT_EQ(own_votes.size(), 1);
  EXPECT_EQ(own_votes[0]->rlp(true, true), weighted_vote_rlp);

  const auto reloaded_votes = node->getDB()->getOwnVerifiedVotes();
  ASSERT_EQ(reloaded_votes.size(), 1);
  EXPECT_EQ(reloaded_votes[0]->rlp(true, true), weighted_vote_rlp);
  ASSERT_TRUE(reloaded_votes[0]->getWeight().has_value());
  EXPECT_EQ(*reloaded_votes[0]->getWeight(), *vote->getWeight());
}

TEST_F(VoteTest, rust_reward_vote_check_accepts_reverse_round_fallback) {
  auto node = create_nodes(1, true /*start*/).front();
  node->getPbftManager()->stop();
  clearAllVotes({node});

  constexpr PbftPeriod reward_period = 1;
  constexpr PbftRound preferred_round = 1;
  constexpr PbftRound fallback_round = 2;
  const auto cert_step = static_cast<PbftStep>(PbftVoteTypes::cert_vote);
  const blk_hash_t reward_block_hash(10);
  auto vote_mgr = node->getVoteManager();
  const auto &wallet = node->getConfig().getFirstWallet();

  auto preferred_vote = genDummyVote(PbftVoteTypes::cert_vote, reward_period, preferred_round, cert_step,
                                     reward_block_hash, vote_mgr, wallet);
  auto fallback_vote = genDummyVote(PbftVoteTypes::cert_vote, reward_period, fallback_round, cert_step,
                                    reward_block_hash, vote_mgr, wallet);
  ASSERT_TRUE(vote_mgr->addVerifiedVote(preferred_vote));
  ASSERT_TRUE(vote_mgr->addVerifiedVote(fallback_vote));

  auto batch = DbStorage::createWriteBatch();
  vote_mgr->resetRewardVotes(reward_period, preferred_round, cert_step, reward_block_hash, batch);

  std::vector<vote_hash_t> reward_vote_hashes{fallback_vote->getHash()};
  auto pbft_block = std::make_shared<PbftBlock>(blk_hash_t(1), blk_hash_t(2), blk_hash_t(3), blk_hash_t(4), 2,
                                                wallet.node_addr, wallet.node_secret, reward_vote_hashes);

  auto [valid_without_copy, no_votes] = vote_mgr->checkRewardVotes(pbft_block, false);
  EXPECT_TRUE(valid_without_copy);
  EXPECT_TRUE(no_votes.empty());

  auto [valid_with_copy, copied_votes] = vote_mgr->checkRewardVotes(pbft_block, true);
  ASSERT_TRUE(valid_with_copy);
  ASSERT_EQ(copied_votes.size(), 1);
  EXPECT_EQ(copied_votes[0]->getHash(), fallback_vote->getHash());

  std::vector<vote_hash_t> missing_reward_vote_hashes{vote_hash_t(99)};
  auto missing_reward_block =
      std::make_shared<PbftBlock>(blk_hash_t(1), blk_hash_t(2), blk_hash_t(3), blk_hash_t(4), 2, wallet.node_addr,
                                  wallet.node_secret, missing_reward_vote_hashes);
  auto [valid_missing, missing_votes] = vote_mgr->checkRewardVotes(missing_reward_block, true);
  EXPECT_FALSE(valid_missing);
  EXPECT_TRUE(missing_votes.empty());
}

#ifdef RUSTAXA_ENABLE
TEST_F(VoteTest, rust_validate_vote_composes_final_chain_and_hydrates_weight) {
  auto node = create_nodes(1, true /*start*/).front();
  auto vote_mgr = node->getVoteManager();
  auto pbft_mgr = node->getPbftManager();
  pbft_mgr->stop();

  clearAllVotes({node});
  const auto [current_round, current_period] = pbft_mgr->getPbftRoundAndPeriod();
  const auto &wallet = node->getConfig().getFirstWallet();

  auto weighted_vote = vote_mgr->generateVoteWithWeight(blk_hash_t(77), PbftVoteTypes::soft_vote, current_period,
                                                        current_round, 2, wallet);
  ASSERT_NE(weighted_vote, nullptr);
  ASSERT_TRUE(weighted_vote->getWeight().has_value());
  ASSERT_GT(*weighted_vote->getWeight(), 0);

  auto unweighted_vote = std::make_shared<PbftVote>(weighted_vote->rlp(true, false));
  ASSERT_FALSE(unweighted_vote->getWeight().has_value());
  const auto [valid_unweighted, unweighted_err] = vote_mgr->validateVote(unweighted_vote);
  EXPECT_TRUE(valid_unweighted) << unweighted_err;
  ASSERT_TRUE(unweighted_vote->getWeight().has_value());
  EXPECT_EQ(*unweighted_vote->getWeight(), *weighted_vote->getWeight());
  EXPECT_EQ(unweighted_vote->rlp(true, true), weighted_vote->rlp(true, true));

  const auto [valid_weighted, weighted_err] = vote_mgr->validateVote(weighted_vote);
  EXPECT_TRUE(valid_weighted) << weighted_err;
}

TEST_F(VoteTest, rust_add_vote_with_missing_weight_composes_final_chain) {
  auto node = create_nodes(1, true /*start*/).front();
  auto vote_mgr = node->getVoteManager();
  auto pbft_mgr = node->getPbftManager();
  pbft_mgr->stop();

  clearAllVotes({node});
  const auto [current_round, current_period] = pbft_mgr->getPbftRoundAndPeriod();
  const auto &wallet = node->getConfig().getFirstWallet();

  auto weighted_vote = vote_mgr->generateVoteWithWeight(blk_hash_t(88), PbftVoteTypes::soft_vote, current_period,
                                                        current_round, 2, wallet);
  ASSERT_NE(weighted_vote, nullptr);
  auto unweighted_vote = std::make_shared<PbftVote>(weighted_vote->rlp(true, false));
  ASSERT_FALSE(unweighted_vote->getWeight().has_value());

  const auto report = vote_mgr->addVerifiedVoteWithReport(unweighted_vote);
  EXPECT_TRUE(report.accepted);
  EXPECT_TRUE(unweighted_vote->getWeight().has_value());
  EXPECT_GT(*unweighted_vote->getWeight(), 0);
  EXPECT_EQ(*unweighted_vote->getWeight(), *weighted_vote->getWeight());

  const auto duplicate_report = vote_mgr->addVerifiedVoteWithReport(unweighted_vote);
  EXPECT_FALSE(duplicate_report.accepted);
  EXPECT_TRUE(duplicate_report.already_present);
}

TEST_F(VoteTest, rust_generate_weighted_vote_is_deterministic) {
  auto node = create_nodes(1, true /*start*/).front();
  auto pbft_mgr = node->getPbftManager();
  auto vote_mgr = node->getVoteManager();
  pbft_mgr->stop();

  clearAllVotes({node});
  const auto [current_round, current_period] = pbft_mgr->getPbftRoundAndPeriod();
  const auto &wallet = node->getConfig().getFirstWallet();

  auto block_hash = blk_hash_t(77);
  auto vote_a =
      vote_mgr->generateVoteWithWeight(block_hash, PbftVoteTypes::soft_vote, current_period, current_round, 2, wallet);
  auto vote_b =
      vote_mgr->generateVoteWithWeight(block_hash, PbftVoteTypes::soft_vote, current_period, current_round, 2, wallet);

  ASSERT_NE(vote_a, nullptr);
  ASSERT_NE(vote_b, nullptr);
  EXPECT_EQ(vote_a->getHash(), vote_b->getHash());
  EXPECT_EQ(vote_a->rlp(true, true), vote_b->rlp(true, true));
}

TEST_F(VoteTest, rust_generate_weighted_vote_rejects_far_future_period) {
  auto node = create_nodes(1, true /*start*/).front();
  auto pbft_mgr = node->getPbftManager();
  auto vote_mgr = node->getVoteManager();
  pbft_mgr->stop();

  clearAllVotes({node});
  const auto [current_round, current_period] = pbft_mgr->getPbftRoundAndPeriod();
  const auto &wallet = node->getConfig().getFirstWallet();

  auto too_far_period = current_period + 1000;
  auto vote = vote_mgr->generateVoteWithWeight(blk_hash_t(11), PbftVoteTypes::propose_vote, too_far_period,
                                               current_round, 1, wallet);
  EXPECT_EQ(vote, nullptr);
}

TEST_F(VoteTest, rust_generate_and_validate_proposer_sortition_is_deterministic) {
  auto node = create_nodes(1, true /*start*/).front();
  auto pbft_mgr = node->getPbftManager();
  auto vote_mgr = node->getVoteManager();
  pbft_mgr->stop();

  clearAllVotes({node});
  const auto [current_round, current_period] = pbft_mgr->getPbftRoundAndPeriod();
  const auto &wallet = node->getConfig().getFirstWallet();

  const auto first = vote_mgr->genAndValidateVrfSortition(current_period, current_round, wallet);
  const auto second = vote_mgr->genAndValidateVrfSortition(current_period, current_round, wallet);

  EXPECT_TRUE(first);
  EXPECT_EQ(second, first);
}

TEST_F(VoteTest, rust_generate_and_validate_proposer_sortition_rejects_far_future_period) {
  auto node = create_nodes(1, true /*start*/).front();
  auto pbft_mgr = node->getPbftManager();
  auto vote_mgr = node->getVoteManager();
  pbft_mgr->stop();

  clearAllVotes({node});
  const auto [current_round, current_period] = pbft_mgr->getPbftRoundAndPeriod();
  const auto &wallet = node->getConfig().getFirstWallet();

  EXPECT_FALSE(vote_mgr->genAndValidateVrfSortition(current_period + 1000, current_round, wallet));
}
#endif

TEST_F(VoteTest, proposer_sortition_matches_legacy_nontrivial_weight) {
  auto node_cfgs = make_node_cfgs(2, 2);
  node_cfgs[0].genesis.pbft.number_of_proposers = 1;
  auto node = create_node(node_cfgs[0], true);
  auto pbft_mgr = node->getPbftManager();
  auto vote_mgr = node->getVoteManager();
  pbft_mgr->stop();

  clearAllVotes({node});
  const auto [current_round, current_period] = pbft_mgr->getPbftRoundAndPeriod();
  const auto &wallet = node->getConfig().getFirstWallet();
  const auto voter_votes = node->getFinalChain()->dposEligibleVoteCount(current_period - 1, wallet.node_addr);
  const auto total_votes = node->getFinalChain()->dposEligibleTotalVoteCount(current_period - 1);
  const auto threshold = std::min<uint64_t>(node->getConfig().genesis.pbft.number_of_proposers, total_votes);
  ASSERT_LT(threshold, total_votes);

  VrfPbftSortition legacy_sortition(wallet.vrf_secret, {PbftVoteTypes::propose_vote, current_period, current_round, 1});
  const auto legacy_weight = legacy_sortition.calculateWeight(voter_votes, total_votes, threshold, wallet.node_pk);
  EXPECT_EQ(legacy_weight, 0);
  const auto rust_or_legacy_result = vote_mgr->genAndValidateVrfSortition(current_period, current_round, wallet);

  EXPECT_EQ(rust_or_legacy_result, legacy_weight != 0);
}

// Generate a vote, send the vote from node2 to node1
TEST_F(VoteTest, transfer_vote) {
  auto node_cfgs = make_node_cfgs(2);

  auto nodes = launch_nodes(node_cfgs);
  auto &node1 = nodes[0];
  auto &node2 = nodes[1];
  std::shared_ptr<Network> nw1 = node1->getNetwork();
  std::shared_ptr<Network> nw2 = node2->getNetwork();

  // stop PBFT manager, that will place vote
  node1->getPbftManager()->stop();
  node2->getPbftManager()->stop();

  clearAllVotes({node1, node2});

  // generate a vote far ahead (never exist in PBFT manager)
  blk_hash_t propose_block_hash(11);
  PbftVoteTypes type = PbftVoteTypes::propose_vote;
  PbftPeriod period = 1;
  PbftRound round = 1;
  PbftStep step = 1;
  auto vote = node1->getVoteManager()->generateVote(propose_block_hash, type, period, round, step,
                                                    node1->getConfig().getFirstWallet());

  nw1->getSpecificHandler<network::tarcap::IVotePacketHandler>(network::SubprotocolPacketType::kVotePacket)
      ->sendPbftVote(nw1->getPeer(nw2->getNodeId()), vote, nullptr);

  auto vote_mgr1 = node1->getVoteManager();
  auto vote_mgr2 = node2->getVoteManager();
  EXPECT_HAPPENS({60s, 100ms}, [&](auto &ctx) { WAIT_EXPECT_EQ(ctx, vote_mgr2->getVerifiedVotesSize(), 1) });
  EXPECT_EQ(vote_mgr1->getVerifiedVotesSize(), 0);
}

TEST_F(VoteTest, vote_broadcast) {
  auto node_cfgs = make_node_cfgs(3);
  auto nodes = launch_nodes(node_cfgs);
  auto &node1 = nodes[0];
  auto &node2 = nodes[1];
  auto &node3 = nodes[2];

  // stop PBFT manager, that will place vote
  std::shared_ptr<PbftManager> pbft_mgr1 = node1->getPbftManager();
  std::shared_ptr<PbftManager> pbft_mgr2 = node2->getPbftManager();
  std::shared_ptr<PbftManager> pbft_mgr3 = node3->getPbftManager();
  pbft_mgr1->stop();
  pbft_mgr2->stop();
  pbft_mgr3->stop();

  auto vote_mgr1 = node1->getVoteManager();
  auto vote_mgr2 = node2->getVoteManager();
  auto vote_mgr3 = node3->getVoteManager();

  auto [period, round] = clearAllVotes({node1, node2, node3});

  EXPECT_EQ(vote_mgr1->getVerifiedVotesSize(), 0);
  EXPECT_EQ(vote_mgr2->getVerifiedVotesSize(), 0);
  EXPECT_EQ(vote_mgr3->getVerifiedVotesSize(), 0);

  // generate a vote far ahead (never exist in PBFT manager)
  auto vote = vote_mgr1->generateVote(blk_hash_t(1), PbftVoteTypes::soft_vote, period, round, 2,
                                      node1->getConfig().getFirstWallet());

  node1->getNetwork()
      ->getSpecificHandler<network::tarcap::IVotePacketHandler>(network::SubprotocolPacketType::kVotePacket)
      ->onNewPbftVote(vote, nullptr);

  EXPECT_HAPPENS({60s, 100ms}, [&](auto &ctx) {
    WAIT_EXPECT_EQ(ctx, vote_mgr2->getVerifiedVotesSize(), 1)
    WAIT_EXPECT_EQ(ctx, vote_mgr3->getVerifiedVotesSize(), 1)
  });
}

TEST_F(VoteTest, two_t_plus_one_votes) {
  auto node_cfgs = make_node_cfgs(1);
  auto nodes = launch_nodes(node_cfgs);
  auto &node = nodes[0];

  // stop PBFT manager, that will place vote
  node->getPbftManager()->stop();

  auto vote_mgr = node->getVoteManager();

  // Clear unverfied/verified table/DB
  clearAllVotes({node});

  const auto chain_size = node->getPbftProgress().finalized_period;
  auto pbft_2t_plus_1 = vote_mgr->getPbftTwoTPlusOne(chain_size, PbftVoteTypes::cert_vote).value();
  EXPECT_EQ(pbft_2t_plus_1, 1);

  // Generate a vote voted at kNullBlockHash
  PbftPeriod period = 1;
  PbftRound round = 1;

  vote_mgr->addVerifiedVote(genDummyVote(PbftVoteTypes::soft_vote, period, round, 2, blk_hash_t(1), vote_mgr,
                                         node->getConfig().getFirstWallet()));
  EXPECT_TRUE(vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::SoftVotedBlock).has_value());
  EXPECT_FALSE(
      vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::CertVotedBlock).has_value());
  EXPECT_FALSE(
      vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::NextVotedBlock).has_value());
  EXPECT_FALSE(
      vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::NextVotedNullBlock).has_value());

  vote_mgr->addVerifiedVote(genDummyVote(PbftVoteTypes::cert_vote, period, round, 3, blk_hash_t(1), vote_mgr,
                                         node->getConfig().getFirstWallet()));
  EXPECT_TRUE(vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::SoftVotedBlock).has_value());
  EXPECT_TRUE(vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::CertVotedBlock).has_value());
  EXPECT_FALSE(
      vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::NextVotedBlock).has_value());
  EXPECT_FALSE(
      vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::NextVotedNullBlock).has_value());

  vote_mgr->addVerifiedVote(genDummyVote(PbftVoteTypes::next_vote, period, round, 4, blk_hash_t(1), vote_mgr,
                                         node->getConfig().getFirstWallet()));
  EXPECT_TRUE(vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::SoftVotedBlock).has_value());
  EXPECT_TRUE(vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::CertVotedBlock).has_value());
  EXPECT_TRUE(vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::NextVotedBlock).has_value());
  EXPECT_FALSE(
      vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::NextVotedNullBlock).has_value());

  vote_mgr->addVerifiedVote(genDummyVote(PbftVoteTypes::next_vote, period, round, 5, kNullBlockHash, vote_mgr,
                                         node->getConfig().getFirstWallet()));
  EXPECT_TRUE(vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::SoftVotedBlock).has_value());
  EXPECT_TRUE(vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::CertVotedBlock).has_value());
  EXPECT_TRUE(vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::NextVotedBlock).has_value());
  EXPECT_TRUE(
      vote_mgr->getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::NextVotedNullBlock).has_value());
}

TEST_F(VoteTest, vote_count_compare) {
  auto vote_count_old = [](u256 balance, u256 threshold) { return u256(balance / threshold); };
  auto vote_count_new = [](u256 balance, u256 threshold, u256 step) {
    // the same logic as new GO method. Result should be the same as for the old method if threshold == step
    u256 res = 0;
    if (balance >= threshold) {
      res = balance - threshold;
      res /= step;
      res += 1;
    }
    return res;
  };

  {
    auto balance = 1000000;
    auto threshold = 100000;
    EXPECT_EQ(vote_count_old(balance, threshold), vote_count_new(balance, threshold, threshold));
  }

  {
    auto balance = 1000000000000;
    auto threshold = 100000;
    EXPECT_EQ(vote_count_old(balance, threshold), vote_count_new(balance, threshold, threshold));
  }

  {
    auto balance = u256("10000000000000000000000000");
    auto threshold = 100000;
    EXPECT_EQ(vote_count_old(balance, threshold), vote_count_new(balance, threshold, threshold));
  }

  {
    auto step = 100000;
    auto threshold = 1000000;
    auto balance = u256(7 * step);
    EXPECT_EQ(vote_count_old(balance, threshold), vote_count_new(balance, threshold, threshold));
  }
}

}  // namespace taraxa::core_tests
#endif

using namespace taraxa;
int main(int argc, char **argv) {
  taraxa::static_init();
  auto logging = logger::createDefaultLoggingConfig();
  logging.verbosity = logger::Verbosity::Error;

  addr_t node_addr;
  logger::InitLogging(logging, node_addr);

  ::testing::InitGoogleTest(&argc, argv);
  return RUN_ALL_TESTS();
}
