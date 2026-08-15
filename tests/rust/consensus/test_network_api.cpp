#include <gtest/gtest.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <cstdint>
#include <filesystem>
#include <string>
#include <utility>
#include <vector>

#include "consensus_application_test.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "vote/pillar_vote.hpp"

namespace {

std::array<uint8_t, 64> nodeId(uint8_t byte) {
  std::array<uint8_t, 64> id{};
  id.fill(byte);
  return id;
}

rust::Vec<uint8_t> bridgeBytes(const taraxa::bytes& values) {
  rust::Vec<uint8_t> out;
  out.reserve(values.size());
  for (const auto value : values) {
    out.push_back(static_cast<uint8_t>(value));
  }
  return out;
}

rustaxa::PbftServiceConfig serviceConfig() {
  rustaxa::PbftServiceConfig config{};
  config.genesis_lambda_ms = 100;
  config.cacti_lambda_max_ms = 100;
  config.cacti_lambda_default_ms = 100;
  config.cacti_block = 100;
  config.max_exponential_lambda_ms = 60'000;
  config.max_steps = 13;
  config.deadline_ms = 400;
  config.polling_interval_ms = 100;
  config.report_malicious_behaviour = true;
  config.magnolia_activation_period = 0;
  config.ficus_activation_period = 10;
  config.pillar_blocks_interval = 10;
  config.sync_level_size = 10;
  config.is_light_node = false;
  config.light_node_history = 0;
  config.committee_size = 5;
  config.number_of_proposers = 20;
  return config;
}

struct TemporaryStorageDirectory {
  TemporaryStorageDirectory() {
    static std::atomic<uint64_t> sequence{0};
    path = std::filesystem::temp_directory_path() /
           ("rustaxa_network_api_bridge_" + std::to_string(sequence.fetch_add(1)));
    std::error_code ignored;
    std::filesystem::remove_all(path, ignored);
  }

  ~TemporaryStorageDirectory() {
    std::error_code ignored;
    std::filesystem::remove_all(path, ignored);
  }

  std::filesystem::path path;
};

struct NetworkApiFixture {
  NetworkApiFixture()
      : service(rustaxa::test::createConsensusApplication(directory.path, serviceConfig())),
        network_api(rustaxa::create_consensus_network_api(*service)) {
    service->pbft_service_complete_pillar_bootstrap();
  }

  rustaxa::BridgeConsensusNetworkApi* operator->() { return &*network_api; }

  TemporaryStorageDirectory directory;
  rust::Box<rustaxa::BridgeConsensusApplication> service;
  rust::Box<rustaxa::BridgeConsensusNetworkApi> network_api;
};

std::array<uint8_t, 32> hash(uint8_t byte);

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
  auto network_api = NetworkApiFixture{};

  const auto batch = network_api->consensus_network_drain_work(6, 0, false, 10);
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

TEST(ConsensusNetworkApiBridgeTest, pillarVoteIngressQueuesAdmissionAndAcceptedFollowUps) {
  auto network_api = NetworkApiFixture{};
  const auto secret = taraxa::secret_t("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd");
  const taraxa::PillarVote vote(secret, taraxa::PbftPeriod{21}, taraxa::blk_hash_t{456});
  rustaxa::NetworkPillarVoteIngressContext context{};
  context.transport_lane = 6;
  context.peer_id = nodeId(0x71);
  context.source_payload_id = 101;
  context.ficus_activation_period = 10;
  context.allow_gossip = true;
  rustaxa::PillarVoteRlpPayload payload;
  payload.vote_rlp = bridgeBytes(vote.rlp());
  rust::Vec<rustaxa::PillarVoteRlpPayload> payloads;
  payloads.push_back(std::move(payload));

  const auto decisions = network_api->consensus_network_ingest_pillar_vote_bundle(context, std::move(payloads));
  ASSERT_EQ(decisions.size(), 1);
  EXPECT_EQ(decisions[0].status, 0);
  EXPECT_NE(decisions[0].application_effect_id, 0);

  const auto admission = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(admission.effects.size(), 1);
  EXPECT_EQ(admission.effects[0].kind, 8);
  EXPECT_EQ(admission.effects[0].object_kind, 5);
  auto result = successfulResult(admission.effects[0]);
  result.admission_accepted = true;
  rust::Vec<rustaxa::NetworkEffectResult> results;
  results.push_back(std::move(result));
  EXPECT_EQ(network_api->consensus_network_report_effect_results(std::move(results)).status, 0);

  const auto follow_ups = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(follow_ups.effects.size(), 2);
  EXPECT_EQ(follow_ups.effects[0].kind, 2);
  EXPECT_EQ(follow_ups.effects[1].kind, 1);
  EXPECT_EQ(follow_ups.effects[1].packet_kind, 13);
}

TEST(ConsensusNetworkApiBridgeTest, statusSyncPlanningRoutesThroughNetworkApi) {
  auto network_api = NetworkApiFixture{};

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
  auto network_api = NetworkApiFixture{};

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
  auto network_api = NetworkApiFixture{};

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
  auto network_api = NetworkApiFixture{};

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
  auto network_api = NetworkApiFixture{};

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
  auto network_api = NetworkApiFixture{};

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
