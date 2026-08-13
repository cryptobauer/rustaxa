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

rust::Vec<uint8_t> bytes(std::initializer_list<uint8_t> values) {
  rust::Vec<uint8_t> out;
  for (auto value : values) {
    out.push_back(value);
  }
  return out;
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
      : storage(rustaxa::create_storage(directory.path.string())),
        service(rustaxa::test::createConsensusApplication(*storage, serviceConfig())),
        network_api(rustaxa::create_consensus_network_api(*service)) {
    service->pbft_service_complete_pillar_bootstrap();
  }

  rustaxa::BridgeConsensusNetworkApi* operator->() { return &*network_api; }

  TemporaryStorageDirectory directory;
  rust::Box<rustaxa::BridgeStorage> storage;
  rust::Box<rustaxa::BridgeConsensusApplication> service;
  rust::Box<rustaxa::BridgeConsensusNetworkApi> network_api;
};

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

std::array<uint8_t, 32> hash(uint8_t byte);

rustaxa::NetworkPbftVoteIngressContext networkVoteContext() {
  rustaxa::NetworkPbftVoteIngressContext context{};
  context.ingress = voteContext();
  context.transport_lane = 6;
  context.peer_id = nodeId(0x44);
  context.peer_pbft_chain_size = 11;
  context.source_payload_id = 99;
  context.enqueue_admission = false;
  context.allow_gossip = true;
  context.vote_hash = hash(0xEF);
  context.vote_rlp = bytes({0xC3, 1, 2, 3});
  context.pbft_block_rlp = bytes({0xC2, 4, 5});
  context.pbft_block_hash = hash(0xAB);
  context.pbft_block_period = 42;
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

rustaxa::NetworkEffectResult voteAdmissionResult(const rustaxa::NetworkEffect& effect, bool admission_accepted,
                                                 bool admission_already_present, bool admission_mark_vote_known,
                                                 bool admission_gossip_vote) {
  rustaxa::NetworkEffectResult result = successfulResult(effect);
  result.admission_accepted = admission_accepted;
  result.admission_already_present = admission_already_present;
  result.admission_mark_vote_known = admission_mark_vote_known;
  result.admission_gossip_vote = admission_gossip_vote;
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

TEST(ConsensusNetworkApiBridgeTest, adaptersCloneThePbftRootOwnedNetworkService) {
  auto first = NetworkApiFixture{};
  auto second = rustaxa::create_consensus_network_api(*first.service);

  auto context = networkVoteContext();
  context.enqueue_admission = true;
  const auto decision = first->consensus_network_ingest_pbft_vote(voteFact(14, 3, 1, 2), context);
  ASSERT_EQ(decision.queued_effect_count, 1);

  const auto batch = second->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].source_payload_id, context.source_payload_id);
}

TEST(ConsensusNetworkApiBridgeTest, reportEffectResultsAcceptsMatchingEffectIdentity) {
  auto network_api = NetworkApiFixture{};

  auto context = networkVoteContext();
  context.enqueue_admission = true;
  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(14, 3, 1, 2), context);
  ASSERT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(6, 0, false, 10);
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
  auto network_api = NetworkApiFixture{};

  auto context = networkVoteContext();
  context.enqueue_admission = true;
  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(14, 3, 1, 2), context);

  EXPECT_TRUE(decision.routed);
  EXPECT_TRUE(decision.payload_accepted);
  EXPECT_EQ(decision.payload_id, 99);
  EXPECT_EQ(decision.status, 3);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 3);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x44));
  EXPECT_EQ(batch.effects[0].sync_kind, 0);
  EXPECT_EQ(batch.effects[0].sync_start, 13);
  EXPECT_EQ(batch.effects[0].source_payload_id, 99);
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteIngressAcceptsCurrentVoteWithoutNetworkEffects) {
  auto network_api = NetworkApiFixture{};

  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(10, 3, 2, 2), networkVoteContext());

  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_TRUE(decision.error_code.empty());
  EXPECT_EQ(decision.queued_effect_count, 0);

  const auto batch = network_api->consensus_network_drain_work(6, 0, false, 10);
  EXPECT_TRUE(batch.effects.empty());
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteBundleIngressQueuesReportAndDisconnectEffects) {
  auto network_api = NetworkApiFixture{};

  rust::Vec<rustaxa::PbftVoteIngressFact> votes;
  votes.push_back(voteFact(10, 3, 2, 1));
  rust::Vec<rustaxa::NetworkPbftVoteIngressContext> contexts;
  contexts.push_back(networkVoteContext());
  const auto decisions = network_api->consensus_network_ingest_pbft_vote_bundle(voteFact(10, 3, 2, 1), std::move(votes),
                                                                                std::move(contexts));
  ASSERT_EQ(decisions.size(), 1);
  const auto& decision = decisions.front();

  EXPECT_EQ(decision.status, 7);
  EXPECT_EQ(decision.queued_effect_count, 2);

  const auto batch = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(batch.effects.size(), 2);
  EXPECT_EQ(batch.effects[0].kind, 4);
  EXPECT_EQ(batch.effects[0].reason_code, 0);
  EXPECT_EQ(batch.effects[1].kind, 5);
  EXPECT_EQ(batch.effects[1].reason_code, 0);
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

TEST(ConsensusNetworkApiBridgeTest, pbftVoteAdmissionOrdersBlockPublicationBeforeGossip) {
  auto network_api = NetworkApiFixture{};

  auto context = networkVoteContext();
  context.enqueue_admission = true;
  context.allow_gossip = true;
  context.peer_id = nodeId(0x77);
  context.source_payload_id = 103;

  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(10, 3, 2, 2), context);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto admission = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(admission.effects.size(), 1);
  EXPECT_EQ(decision.application_effect_id, admission.effects[0].effect_id);
  EXPECT_EQ(admission.effects[0].kind, 8);
  EXPECT_EQ(admission.effects[0].object_kind, 0);
  EXPECT_EQ(admission.effects[0].object_hash, hash(0xEF));
  EXPECT_EQ(std::vector<uint8_t>(admission.effects[0].payload_bytes.begin(), admission.effects[0].payload_bytes.end()),
            (std::vector<uint8_t>{0xC3, 1, 2, 3}));
  rust::Vec<rustaxa::NetworkEffectResult> admission_result;
  admission_result.push_back(voteAdmissionResult(admission.effects[0], true, false, false, true));
  EXPECT_EQ(network_api->consensus_network_report_effect_results(std::move(admission_result)).status, 0);

  const auto block_publication = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(block_publication.effects.size(), 1);
  EXPECT_EQ(block_publication.effects[0].kind, 8);
  EXPECT_EQ(block_publication.effects[0].object_kind, 1);
  EXPECT_EQ(block_publication.effects[0].object_hash, hash(0xAB));
  rust::Vec<rustaxa::NetworkEffectResult> block_result;
  block_result.push_back(successfulResult(block_publication.effects[0]));
  EXPECT_EQ(network_api->consensus_network_report_effect_results(std::move(block_result)).status, 0);

  const auto dependents = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(dependents.effects.size(), 2);
  EXPECT_EQ(dependents.effects[0].kind, 2);
  EXPECT_EQ(dependents.effects[0].dependency_id, block_publication.effects[0].effect_id);
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
  auto network_api = NetworkApiFixture{};
  auto context = networkVoteContext();
  context.enqueue_admission = true;
  context.peer_id = nodeId(0x77);
  context.vote_hash = hash(0xEF);
  context.vote_rlp = bytes({0xC3, 1, 2, 3});
  context.pbft_block_rlp = bytes({0xC2, 4, 5});
  context.pbft_block_hash = hash(0xAB);
  context.pbft_block_period = 42;

  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(10, 3, 2, 2), context);
  EXPECT_EQ(decision.queued_effect_count, 1);
  const auto admission = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(admission.effects.size(), 1);
  rust::Vec<rustaxa::NetworkEffectResult> admission_result;
  admission_result.push_back(voteAdmissionResult(admission.effects[0], false, true, false, false));
  EXPECT_EQ(network_api->consensus_network_report_effect_results(std::move(admission_result)).status, 0);

  const auto first = network_api->consensus_network_drain_work(6, 0, false, 10);
  EXPECT_TRUE(
      std::none_of(first.effects.begin(), first.effects.end(), [](const auto& effect) { return effect.kind == 1; }));
  EXPECT_TRUE(std::any_of(first.effects.begin(), first.effects.end(),
                          [](const auto& effect) { return effect.kind == 8 && effect.object_kind == 1; }));
  EXPECT_TRUE(
      std::any_of(first.effects.begin(), first.effects.end(), [](const auto& effect) { return effect.kind == 2; }));
}

TEST(ConsensusNetworkApiBridgeTest, failedAdmissionCancelsDependentKnownAndGossipEffects) {
  auto network_api = NetworkApiFixture{};
  auto context = networkVoteContext();
  context.enqueue_admission = true;
  context.peer_id = nodeId(0x77);
  context.pbft_block_rlp = bytes({0xC2, 4, 5});
  context.pbft_block_hash = hash(0xAB);
  context.pbft_block_period = 42;

  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(10, 3, 2, 2), context);
  EXPECT_EQ(decision.queued_effect_count, 1);
  const auto admission = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(admission.effects.size(), 1);
  rust::Vec<rustaxa::NetworkEffectResult> admission_result;
  admission_result.push_back(successfulResult(admission.effects[0]));
  admission_result[0].status = 1;
  EXPECT_EQ(network_api->consensus_network_report_effect_results(std::move(admission_result)).status, 0);

  const auto cancelled = network_api->consensus_network_drain_work(6, 0, false, 10);
  EXPECT_TRUE(cancelled.effects.empty());
  EXPECT_FALSE(cancelled.more_available);
}

TEST(ConsensusNetworkApiBridgeTest, gossipPayloadsSurviveProducerScopeAndDrainFifo) {
  auto network_api = NetworkApiFixture{};

  for (uint8_t byte : {1, 2}) {
    auto context = networkVoteContext();
    context.enqueue_admission = true;
    context.peer_id = nodeId(byte);
    context.vote_hash = hash(byte);
    context.vote_rlp = bytes({0xC1, byte});
    context.pbft_block_rlp = rust::Vec<uint8_t>();
    context.pbft_block_hash = std::array<uint8_t, 32>{};
    context.pbft_block_period = 0;
    context.source_payload_id = byte;
    context.allow_gossip = true;
    const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(10, 3, 2, 2), context);
    EXPECT_TRUE(decision.routed);
    EXPECT_EQ(decision.queued_effect_count, 1);
  }

  const auto first = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(first.effects.size(), 2);
  EXPECT_EQ(first.effects[0].kind, 8);
  EXPECT_EQ(first.effects[1].kind, 8);
  EXPECT_TRUE(first.effects[0].object_hash == hash(1) || first.effects[0].object_hash == hash(2));

  rust::Vec<rustaxa::NetworkEffectResult> admission_results;
  admission_results.push_back(voteAdmissionResult(first.effects[0], true, false, false, true));
  admission_results.push_back(voteAdmissionResult(first.effects[1], true, false, false, true));
  EXPECT_EQ(network_api->consensus_network_report_effect_results(std::move(admission_results)).status, 0);

  const auto second = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(second.effects.size(), 2);
  EXPECT_EQ(second.effects[0].kind, 1);
  EXPECT_EQ(second.effects[1].kind, 1);
  EXPECT_TRUE(std::vector<uint8_t>(second.effects[0].payload_bytes.begin(), second.effects[0].payload_bytes.end()) ==
              (std::vector<uint8_t>{0xC1, 1}));
  EXPECT_TRUE(std::vector<uint8_t>(second.effects[1].payload_bytes.begin(), second.effects[1].payload_bytes.end()) ==
              (std::vector<uint8_t>{0xC1, 2}));
}

TEST(ConsensusNetworkApiBridgeTest, sharedRootDrainsOnlyRequestedTransportLane) {
  auto network_api = NetworkApiFixture{};
  const auto enqueue_gossip = [&network_api](uint32_t transport_lane, uint8_t byte) {
    auto context = networkVoteContext();
    context.enqueue_admission = true;
    context.transport_lane = transport_lane;
    context.peer_id = nodeId(byte);
    context.vote_hash = hash(byte);
    context.vote_rlp = bytes({0xC1, byte});
    context.source_payload_id = byte;
    context.pbft_block_rlp = rust::Vec<uint8_t>();
    context.pbft_block_hash = std::array<uint8_t, 32>{};
    context.pbft_block_period = 0;
    const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(10, 3, 2, 2), context);
    return decision;
  };

  EXPECT_TRUE(enqueue_gossip(6, 1).routed);
  EXPECT_TRUE(enqueue_gossip(5, 2).routed);
  EXPECT_TRUE(enqueue_gossip(6, 3).routed);
  EXPECT_TRUE(enqueue_gossip(5, 4).routed);

  const auto first_v5 = network_api->consensus_network_drain_work(5, 0, false, 1);
  ASSERT_EQ(first_v5.effects.size(), 1);
  EXPECT_EQ(first_v5.effects[0].effect_id, 2);
  EXPECT_EQ(first_v5.effects[0].transport_lane, 5);
  EXPECT_TRUE(first_v5.more_available);

  const auto latest = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(latest.effects.size(), 2);
  EXPECT_EQ(latest.effects[0].effect_id, 1);
  EXPECT_EQ(latest.effects[1].effect_id, 3);
  EXPECT_EQ(latest.effects[0].transport_lane, 6);
  EXPECT_EQ(latest.effects[1].transport_lane, 6);
  EXPECT_FALSE(latest.more_available);

  const auto second_v5 = network_api->consensus_network_drain_work(5, 0, false, 10);
  ASSERT_EQ(second_v5.effects.size(), 1);
  EXPECT_EQ(second_v5.effects[0].effect_id, 4);
  EXPECT_EQ(second_v5.effects[0].transport_lane, 5);
  EXPECT_FALSE(second_v5.more_available);
}

TEST(ConsensusNetworkApiBridgeTest, sourceScopedDrainTreatsZeroAsAValidSourceId) {
  auto network_api = NetworkApiFixture{};
  const auto enqueue_admission = [&network_api](uint64_t source_payload_id, uint8_t byte) {
    auto context = networkVoteContext();
    context.enqueue_admission = true;
    context.transport_lane = 6;
    context.peer_id = nodeId(byte);
    context.vote_hash = hash(byte);
    context.source_payload_id = source_payload_id;
    const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(10, 3, 2, 2), context);
    ASSERT_TRUE(decision.routed);
    ASSERT_EQ(decision.queued_effect_count, 1);
  };

  enqueue_admission(0, 1);
  enqueue_admission(7, 2);

  const auto zero_source = network_api->consensus_network_drain_work(6, 0, true, 10);
  ASSERT_EQ(zero_source.effects.size(), 1);
  EXPECT_EQ(zero_source.effects[0].source_payload_id, 0);
  EXPECT_FALSE(zero_source.more_available);

  const auto other_source = network_api->consensus_network_drain_work(6, 7, true, 10);
  ASSERT_EQ(other_source.effects.size(), 1);
  EXPECT_EQ(other_source.effects[0].source_payload_id, 7);
  EXPECT_FALSE(other_source.more_available);
}

TEST(ConsensusNetworkApiBridgeTest, drainWorkIsolatesInterleavedTransportLanes) {
  auto network_api = NetworkApiFixture{};
  const auto enqueue_gossip = [&network_api](uint32_t transport_lane, uint8_t byte) {
    auto context = networkVoteContext();
    context.enqueue_admission = true;
    context.transport_lane = transport_lane;
    context.peer_id = nodeId(byte);
    context.vote_hash = hash(byte);
    context.vote_rlp = bytes({0xC1, byte});
    context.source_payload_id = byte;
    context.pbft_block_rlp = rust::Vec<uint8_t>();
    context.pbft_block_hash = std::array<uint8_t, 32>{};
    context.pbft_block_period = 0;
    const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(10, 3, 2, 2), context);
    return decision;
  };

  EXPECT_TRUE(enqueue_gossip(6, 1).routed);
  EXPECT_TRUE(enqueue_gossip(5, 2).routed);
  EXPECT_TRUE(enqueue_gossip(6, 3).routed);
  EXPECT_TRUE(enqueue_gossip(5, 4).routed);

  const auto first_v5 = network_api->consensus_network_drain_work(5, 0, false, 1);
  ASSERT_EQ(first_v5.effects.size(), 1);
  EXPECT_EQ(first_v5.effects[0].effect_id, 2);
  EXPECT_EQ(first_v5.effects[0].transport_lane, 5);
  EXPECT_TRUE(first_v5.more_available);

  const auto latest = network_api->consensus_network_drain_work(6, 0, false, 10);
  ASSERT_EQ(latest.effects.size(), 2);
  EXPECT_EQ(latest.effects[0].effect_id, 1);
  EXPECT_EQ(latest.effects[1].effect_id, 3);
  EXPECT_EQ(latest.effects[0].transport_lane, 6);
  EXPECT_EQ(latest.effects[1].transport_lane, 6);
  EXPECT_FALSE(latest.more_available);

  const auto second_v5 = network_api->consensus_network_drain_work(5, 0, false, 10);
  ASSERT_EQ(second_v5.effects.size(), 1);
  EXPECT_EQ(second_v5.effects[0].effect_id, 4);
  EXPECT_EQ(second_v5.effects[0].transport_lane, 5);
  EXPECT_FALSE(second_v5.more_available);
}
