#include <gtest/gtest.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <utility>
#include <vector>

#include "rustaxa-bridge/ffi.rs.h"

namespace {

std::array<uint8_t, 64> nodeId(uint8_t byte) {
  std::array<uint8_t, 64> id{};
  id.fill(byte);
  return id;
}

rust::Vec<uint8_t> bytes(std::initializer_list<uint8_t> values) {
  rust::Vec<uint8_t> out;
  for (auto value : values) {
    out.push_back(value);
  }
  return out;
}

rustaxa::NetworkApiConfig defaultConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_effects_per_drain = 8;
  return config;
}

rustaxa::PbftVoteIngressFact voteFact(uint64_t period, uint64_t round, uint64_t step, uint8_t vote_type) {
  rustaxa::PbftVoteIngressFact fact{};
  fact.period = period;
  fact.round = round;
  fact.step = step;
  fact.vote_type = vote_type;
  return fact;
}

rustaxa::PbftVoteIngressContext voteContext() {
  rustaxa::PbftVoteIngressContext context{};
  context.current_period = 10;
  context.current_round = 3;
  context.current_step = 2;
  context.max_future_period_delta = 2;
  context.max_future_round_delta = 2;
  context.max_future_step_delta = 2;
  context.validate_max_round_step = true;
  context.source_peer_is_voter = true;
  context.can_request_pbft_sync = true;
  context.can_request_next_votes_sync = true;
  return context;
}

rustaxa::NetworkPbftVoteIngressContext networkVoteContext() {
  rustaxa::NetworkPbftVoteIngressContext context{};
  context.ingress = voteContext();
  context.transport_lane = 6;
  context.peer_id = nodeId(0x44);
  context.peer_pbft_chain_size = 11;
  context.source_payload_id = 99;
  return context;
}

std::array<uint8_t, 32> hash(uint8_t byte) {
  std::array<uint8_t, 32> id{};
  id.fill(byte);
  return id;
}

rustaxa::NetworkEffectResult successfulResult(const rustaxa::NetworkEffect& effect) {
  rustaxa::NetworkEffectResult result{};
  result.effect_id = effect.effect_id;
  result.kind = effect.kind;
  result.peer_id = effect.peer_id;
  result.packet_kind = effect.packet_kind;
  result.object_kind = effect.object_kind;
  result.object_hash = effect.object_hash;
  result.status = 0;
  return result;
}

}  // namespace

TEST(ConsensusNetworkApiBridgeTest, drainWorkAndReportResultsExposeExecutorContract) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto batch = network_api->consensus_network_drain_work(6, 10);
  EXPECT_EQ(batch.status, 0);
  EXPECT_TRUE(batch.effects.empty());
  EXPECT_FALSE(batch.more_available);
  EXPECT_TRUE(batch.error_code.empty());

  rust::Vec<rustaxa::NetworkEffectResult> results;
  const auto ack = network_api->consensus_network_report_effect_results(std::move(results));
  EXPECT_EQ(ack.status, 0);
  EXPECT_EQ(ack.accepted_results, 0);
  EXPECT_EQ(ack.failed_results, 0);
  EXPECT_TRUE(ack.error_code.empty());
}

TEST(ConsensusNetworkApiBridgeTest, reportEffectResultsAcceptsMatchingEffectIdentity) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(14, 3, 1, 2), networkVoteContext());
  ASSERT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(6, 10);
  ASSERT_EQ(batch.effects.size(), 1);

  rustaxa::NetworkEffectResult result{};
  result.effect_id = batch.effects[0].effect_id;
  result.kind = batch.effects[0].kind;
  result.peer_id = batch.effects[0].peer_id;
  result.packet_kind = batch.effects[0].packet_kind;
  result.object_kind = batch.effects[0].object_kind;
  result.object_hash = batch.effects[0].object_hash;
  result.status = 0;

  rust::Vec<rustaxa::NetworkEffectResult> results;
  results.push_back(std::move(result));
  const auto ack = network_api->consensus_network_report_effect_results(std::move(results));
  EXPECT_EQ(ack.status, 0);
  EXPECT_EQ(ack.accepted_results, 1);
  EXPECT_EQ(ack.failed_results, 0);
  EXPECT_TRUE(ack.error_code.empty());
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteIngressQueuesSyncEffectThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(14, 3, 1, 2), networkVoteContext());

  EXPECT_TRUE(decision.routed);
  EXPECT_TRUE(decision.payload_accepted);
  EXPECT_EQ(decision.payload_id, 99);
  EXPECT_EQ(decision.status, 3);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(6, 10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 3);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x44));
  EXPECT_EQ(batch.effects[0].sync_kind, 0);
  EXPECT_EQ(batch.effects[0].sync_start, 13);
  EXPECT_EQ(batch.effects[0].source_payload_id, 99);
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteIngressAcceptsCurrentVoteWithoutNetworkEffects) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(10, 3, 2, 2), networkVoteContext());

  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_TRUE(decision.error_code.empty());
  EXPECT_EQ(decision.queued_effect_count, 0);

  const auto batch = network_api->consensus_network_drain_work(6, 10);
  EXPECT_TRUE(batch.effects.empty());
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteBundleIngressQueuesReportAndDisconnectEffects) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto decision = network_api->consensus_network_ingest_pbft_vote_bundle_member(
      voteFact(10, 3, 2, 1), voteFact(10, 3, 2, 1), networkVoteContext());

  EXPECT_EQ(decision.status, 7);
  EXPECT_EQ(decision.queued_effect_count, 2);

  const auto batch = network_api->consensus_network_drain_work(6, 10);
  ASSERT_EQ(batch.effects.size(), 2);
  EXPECT_EQ(batch.effects[0].kind, 4);
  EXPECT_EQ(batch.effects[0].reason_code, 0);
  EXPECT_EQ(batch.effects[1].kind, 5);
  EXPECT_EQ(batch.effects[1].reason_code, 0);
}

TEST(ConsensusNetworkApiBridgeTest, pillarVoteRelevancePlanningRoutesThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::PillarVoteRelevanceFact fact{};
  fact.vote_period = 21;
  fact.vote_block_hash = hash(0x71);
  fact.current_pillar_block_period = 20;
  fact.current_pillar_block_hash = hash(0x71);
  fact.has_current_pillar_block = true;
  fact.first_pillar_block_period = 10;
  fact.pillar_blocks_interval = 10;
  fact.vote_already_known = false;

  const auto accepted = network_api->consensus_network_plan_pillar_vote_relevance(fact);
  EXPECT_EQ(accepted.status, 0);
  EXPECT_TRUE(accepted.is_relevant);

  fact.vote_block_hash = hash(0x72);
  const auto rejected = network_api->consensus_network_plan_pillar_vote_relevance(fact);
  EXPECT_EQ(rejected.status, 4);
  EXPECT_FALSE(rejected.is_relevant);
}

TEST(ConsensusNetworkApiBridgeTest, statusSyncPlanningRoutesThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkStatusSyncFacts facts{};
  facts.local_pbft_syncing = false;
  facts.local_pbft_synced_period = 10;
  facts.local_pbft_period = 11;
  facts.local_pbft_round = 2;
  facts.peer_pbft_chain_size = 13;
  facts.peer_pbft_period = 14;
  facts.peer_pbft_round = 2;
  facts.peer_dag_synced = true;
  facts.peer_last_status_pbft_chain_size = 10;

  auto plan = network_api->consensus_network_plan_status_sync(facts);
  EXPECT_TRUE(plan.request_pbft_sync);
  EXPECT_FALSE(plan.request_pending_dag_blocks);
  EXPECT_FALSE(plan.request_next_votes);

  facts.peer_pbft_chain_size = 10;
  facts.peer_pbft_period = 11;
  facts.peer_pbft_round = 4;
  facts.peer_dag_synced = false;

  plan = network_api->consensus_network_plan_status_sync(facts);
  EXPECT_FALSE(plan.request_pbft_sync);
  EXPECT_TRUE(plan.request_pending_dag_blocks);
  EXPECT_TRUE(plan.request_next_votes);
  EXPECT_EQ(plan.next_votes_period, 11);
  EXPECT_EQ(plan.next_votes_round, 2);
}

TEST(ConsensusNetworkApiBridgeTest, statusEgressPlanningRoutesThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkStatusEgressFacts facts{};
  facts.initial = true;
  facts.local_chain_id = 7;
  facts.genesis_hash = hash(0xA0);
  facts.node_major_version = 2;
  facts.node_minor_version = 3;
  facts.node_patch_version = 4;
  facts.is_light_node = true;
  facts.light_node_history = 9;
  facts.local_pbft_chain_size = 10;
  facts.local_pbft_round = 5;
  facts.local_dag_level = 44;
  facts.pbft_syncing = true;
  facts.deep_pbft_syncing = false;

  auto plan = network_api->consensus_network_plan_status_egress(facts);
  EXPECT_EQ(plan.status, 0);
  EXPECT_EQ(plan.peer_pbft_chain_size, 10);
  EXPECT_EQ(plan.peer_pbft_round, 5);
  EXPECT_EQ(plan.peer_dag_level, 44);
  EXPECT_TRUE(plan.peer_syncing);
  EXPECT_TRUE(plan.include_initial_data);
  EXPECT_EQ(plan.chain_id, 7);
  EXPECT_EQ(plan.genesis_hash, hash(0xA0));
  EXPECT_EQ(plan.node_major_version, 2);
  EXPECT_TRUE(plan.is_light_node);
  EXPECT_EQ(plan.light_node_history, 9);

  facts.initial = false;
  facts.pbft_syncing = true;
  facts.deep_pbft_syncing = false;
  plan = network_api->consensus_network_plan_status_egress(facts);
  EXPECT_EQ(plan.status, 0);
  EXPECT_FALSE(plan.peer_syncing);
  EXPECT_FALSE(plan.include_initial_data);
  EXPECT_EQ(plan.chain_id, 0);
}

TEST(ConsensusNetworkApiBridgeTest, initialStatusPlanningRoutesThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkInitialStatusFacts facts{};
  facts.local_chain_id = 7;
  facts.peer_chain_id = 7;
  facts.expected_genesis_hash = hash(0xA1);
  facts.peer_genesis_hash = hash(0xA1);
  facts.local_pbft_synced_period = 10;
  facts.peer_pbft_chain_size = 12;
  facts.peer_is_light_node = true;
  facts.peer_light_node_history = 3;

  auto plan = network_api->consensus_network_plan_initial_status(facts);
  EXPECT_EQ(plan.status, 0);
  EXPECT_TRUE(plan.accept_peer);
  EXPECT_FALSE(plan.disconnect_peer);

  facts.peer_genesis_hash = hash(0xA2);
  plan = network_api->consensus_network_plan_initial_status(facts);
  EXPECT_EQ(plan.status, 7);
  EXPECT_FALSE(plan.accept_peer);
  EXPECT_TRUE(plan.disconnect_peer);
}

TEST(ConsensusNetworkApiBridgeTest, pbftSyncStartPlanningRoutesThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftSyncStartFacts facts{};
  facts.local_pbft_syncing = false;
  facts.local_pbft_synced_period = 10;
  facts.local_pbft_chain_size = 10;
  rustaxa::NetworkPbftSyncPeerCandidate first{};
  first.peer_id = nodeId(0x41);
  first.pbft_chain_size = 12;
  first.dag_level = 20;
  rustaxa::NetworkPbftSyncPeerCandidate second{};
  second.peer_id = nodeId(0x42);
  second.pbft_chain_size = 12;
  second.dag_level = 21;
  facts.candidates.push_back(first);
  facts.candidates.push_back(second);

  auto plan = network_api->consensus_network_plan_pbft_sync_start(facts);
  EXPECT_EQ(plan.status, 0);
  EXPECT_TRUE(plan.start_sync);
  EXPECT_TRUE(plan.has_peer);
  EXPECT_EQ(plan.peer_id, nodeId(0x42));
  EXPECT_EQ(plan.request_period, 11);
  EXPECT_FALSE(plan.enable_snapshot_creation);

  facts.local_pbft_synced_period = 13;
  facts.local_pbft_chain_size = 13;
  plan = network_api->consensus_network_plan_pbft_sync_start(facts);
  EXPECT_EQ(plan.status, 3);
  EXPECT_FALSE(plan.start_sync);
  EXPECT_TRUE(plan.enable_snapshot_creation);
}

TEST(ConsensusNetworkApiBridgeTest, maxChainPeerSelectionRoutesThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPeerSelectionFacts facts{};
  facts.local_pbft_syncing_period = 10;
  rustaxa::NetworkPbftSyncPeerCandidate light{};
  light.peer_id = nodeId(0x49);
  light.pbft_chain_size = 20;
  light.dag_level = 50;
  light.is_light_node = true;
  light.light_node_history = 4;
  rustaxa::NetworkPbftSyncPeerCandidate selected{};
  selected.peer_id = nodeId(0x4A);
  selected.pbft_chain_size = 12;
  selected.dag_level = 21;
  facts.candidates.push_back(light);
  facts.candidates.push_back(selected);

  const auto plan = network_api->consensus_network_plan_max_chain_peer_selection(facts);

  EXPECT_EQ(plan.status, 0);
  EXPECT_TRUE(plan.has_peer);
  EXPECT_EQ(plan.peer_id, nodeId(0x4A));
  EXPECT_EQ(plan.peer_pbft_chain_size, 12);
}

TEST(ConsensusNetworkApiBridgeTest, pendingDagBlocksRequestPlanningRoutesThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPendingDagBlocksRequestFacts facts{};
  facts.local_pbft_syncing_period = 10;
  facts.has_explicit_peer = false;
  rustaxa::NetworkPbftSyncPeerCandidate already_synced{};
  already_synced.peer_id = nodeId(0x51);
  already_synced.pbft_chain_size = 12;
  already_synced.dag_level = 30;
  already_synced.peer_dag_synced = true;
  already_synced.dag_sync_allowed = true;
  rustaxa::NetworkPbftSyncPeerCandidate selected{};
  selected.peer_id = nodeId(0x52);
  selected.pbft_chain_size = 10;
  selected.dag_level = 7;
  selected.dag_sync_allowed = true;
  facts.candidates.push_back(already_synced);
  facts.candidates.push_back(selected);

  auto plan = network_api->consensus_network_plan_pending_dag_blocks_request(facts);
  EXPECT_EQ(plan.status, 0);
  EXPECT_TRUE(plan.request_pending_dag_blocks);
  EXPECT_TRUE(plan.has_peer);
  EXPECT_EQ(plan.peer_id, nodeId(0x52));
  EXPECT_EQ(plan.request_period, 10);

  facts.local_pbft_syncing_period = 9;
  plan = network_api->consensus_network_plan_pending_dag_blocks_request(facts);
  EXPECT_EQ(plan.status, 5);
  EXPECT_FALSE(plan.request_pending_dag_blocks);
  EXPECT_TRUE(plan.has_peer);
  EXPECT_EQ(plan.peer_id, nodeId(0x52));
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteAdmissionOrdersBlockPublicationBeforeGossip) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  {
    rustaxa::NetworkPbftVoteAdmissionEffects effects{};
    effects.transport_lane = 6;
    effects.peer_id = nodeId(0x77);
    effects.vote_hash = hash(0xEF);
    effects.vote_rlp = bytes({0xC3, 1, 2, 3});
    effects.accepted = true;
    effects.gossip_vote = true;
    effects.pbft_block_rlp = bytes({0xC2, 4, 5});
    effects.pbft_block_hash = hash(0xAB);
    effects.pbft_block_period = 42;
    effects.source_payload_id = 103;

    const auto decision = network_api->consensus_network_route_pbft_vote_admission(effects);
    EXPECT_TRUE(decision.routed);
    EXPECT_EQ(decision.status, 0);
    EXPECT_EQ(decision.queued_effect_count, 3);
  }

  const auto publication = network_api->consensus_network_drain_work(6, 10);
  ASSERT_EQ(publication.effects.size(), 1);
  EXPECT_EQ(publication.effects[0].kind, 8);
  EXPECT_EQ(publication.effects[0].object_kind, 1);
  EXPECT_EQ(publication.effects[0].object_hash, hash(0xAB));
  rust::Vec<rustaxa::NetworkEffectResult> publication_result;
  publication_result.push_back(successfulResult(publication.effects[0]));
  EXPECT_EQ(network_api->consensus_network_report_effect_results(std::move(publication_result)).status, 0);

  const auto dependents = network_api->consensus_network_drain_work(6, 10);
  ASSERT_EQ(dependents.effects.size(), 2);
  EXPECT_EQ(dependents.effects[1].kind, 1);
  EXPECT_EQ(dependents.effects[1].peer_id, nodeId(0x77));
  EXPECT_EQ(dependents.effects[1].packet_kind, 1);
  ASSERT_EQ(dependents.effects[1].exclude_peers.size(), 1);
  EXPECT_EQ(dependents.effects[1].exclude_peers[0].id, nodeId(0x77));
  EXPECT_EQ(dependents.effects[1].object_kind, 0);
  EXPECT_EQ(dependents.effects[1].object_hash, hash(0xEF));
  EXPECT_EQ(
      std::vector<uint8_t>(dependents.effects[1].payload_bytes.begin(), dependents.effects[1].payload_bytes.end()),
      (std::vector<uint8_t>{0xC3, 1, 2, 3}));
  EXPECT_EQ(std::vector<uint8_t>(dependents.effects[1].related_payload_bytes.begin(),
                                 dependents.effects[1].related_payload_bytes.end()),
            (std::vector<uint8_t>{0xC2, 4, 5}));
  EXPECT_EQ(dependents.effects[1].source_payload_id, 103);
}

TEST(ConsensusNetworkApiBridgeTest, duplicateVoteStillRoutesAttachedProposedBlockWithoutGossip) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());
  rustaxa::NetworkPbftVoteAdmissionEffects effects{};
  effects.transport_lane = 6;
  effects.peer_id = nodeId(0x77);
  effects.vote_hash = hash(0xEF);
  effects.vote_rlp = bytes({0xC3, 1, 2, 3});
  effects.already_present = true;
  effects.gossip_vote = true;
  effects.pbft_block_rlp = bytes({0xC2, 4, 5});
  effects.pbft_block_hash = hash(0xAB);
  effects.pbft_block_period = 42;

  const auto decision = network_api->consensus_network_route_pbft_vote_admission(effects);
  EXPECT_EQ(decision.queued_effect_count, 3);
  const auto first = network_api->consensus_network_drain_work(6, 10);
  ASSERT_EQ(first.effects.size(), 2);
  EXPECT_EQ(first.effects[0].kind, 8);
  EXPECT_EQ(first.effects[0].object_kind, 1);
  EXPECT_EQ(first.effects[1].kind, 2);
  EXPECT_EQ(first.effects[1].object_kind, 0);
  EXPECT_TRUE(
      std::none_of(first.effects.begin(), first.effects.end(), [](const auto& effect) { return effect.kind == 1; }));
}

TEST(ConsensusNetworkApiBridgeTest, sharedRootDrainsOnlyRequestedTransportLane) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());
  const auto enqueue_gossip = [&network_api](uint32_t transport_lane, uint8_t byte) {
    rustaxa::NetworkPbftVoteAdmissionEffects effects{};
    effects.transport_lane = transport_lane;
    effects.peer_id = nodeId(byte);
    effects.vote_hash = hash(byte);
    effects.vote_rlp = bytes({0xC1, byte});
    effects.accepted = true;
    effects.source_payload_id = byte;
    effects.gossip_vote = true;
    return network_api->consensus_network_route_pbft_vote_admission(effects);
  };

  EXPECT_TRUE(enqueue_gossip(6, 1).routed);
  EXPECT_TRUE(enqueue_gossip(5, 2).routed);
  EXPECT_TRUE(enqueue_gossip(6, 3).routed);
  EXPECT_TRUE(enqueue_gossip(5, 4).routed);

  const auto first_v5 = network_api->consensus_network_drain_work(5, 1);
  ASSERT_EQ(first_v5.effects.size(), 1);
  EXPECT_EQ(first_v5.effects[0].effect_id, 2);
  EXPECT_EQ(first_v5.effects[0].transport_lane, 5);
  EXPECT_TRUE(first_v5.more_available);

  const auto latest = network_api->consensus_network_drain_work(6, 10);
  ASSERT_EQ(latest.effects.size(), 2);
  EXPECT_EQ(latest.effects[0].effect_id, 1);
  EXPECT_EQ(latest.effects[1].effect_id, 3);
  EXPECT_EQ(latest.effects[0].transport_lane, 6);
  EXPECT_EQ(latest.effects[1].transport_lane, 6);
  EXPECT_FALSE(latest.more_available);

  const auto second_v5 = network_api->consensus_network_drain_work(5, 10);
  ASSERT_EQ(second_v5.effects.size(), 1);
  EXPECT_EQ(second_v5.effects[0].effect_id, 4);
  EXPECT_EQ(second_v5.effects[0].transport_lane, 5);
  EXPECT_FALSE(second_v5.more_available);
}
