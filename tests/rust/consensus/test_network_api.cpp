#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <utility>

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
  config.max_payload_bytes = 1024;
  config.max_retained_payloads = 11;
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

rustaxa::NetworkIngressPacket packet(uint32_t packet_type, std::array<uint8_t, 64> peer, rust::Vec<uint8_t> payload) {
  rustaxa::NetworkIngressPacket packet{};
  packet.packet_type = packet_type;
  packet.peer_id = peer;
  packet.payload_bytes = std::move(payload);
  packet.received_at_mono_ms = 44;
  packet.source_packet_id = 99;
  return packet;
}

}  // namespace

TEST(ConsensusNetworkApiBridgeTest, ingestPacketStoresCanonicalBytesThroughDirectBridge) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());
  const auto peer = nodeId(0x11);

  const auto receipt = network_api->consensus_network_ingest_packet(packet(1, peer, bytes({1, 2, 3})));

  EXPECT_TRUE(receipt.accepted);
  EXPECT_EQ(receipt.payload_id, 1);
  EXPECT_EQ(receipt.status, 0);
  EXPECT_TRUE(receipt.error_code.empty());

  const auto next_votes_receipt = network_api->consensus_network_ingest_packet(packet(2, peer, bytes({4})));
  EXPECT_TRUE(next_votes_receipt.accepted);
  EXPECT_EQ(next_votes_receipt.payload_id, 2);

  const auto second_receipt = network_api->consensus_network_ingest_packet(packet(3, peer, bytes({5})));
  EXPECT_TRUE(second_receipt.accepted);
  EXPECT_EQ(second_receipt.payload_id, 3);

  const auto proposed_blocks_receipt =
      network_api->consensus_network_ingest_packet(packet(16, peer, bytes({0xC0, 0x02})));
  EXPECT_TRUE(proposed_blocks_receipt.accepted);
  EXPECT_EQ(proposed_blocks_receipt.payload_id, 4);

  const auto transaction_receipt = network_api->consensus_network_ingest_packet(packet(7, peer, bytes({0xC0, 0x03})));
  EXPECT_TRUE(transaction_receipt.accepted);
  EXPECT_EQ(transaction_receipt.payload_id, 5);

  const auto get_pbft_sync_receipt =
      network_api->consensus_network_ingest_packet(packet(10, peer, bytes({0xC0, 0x04})));
  EXPECT_TRUE(get_pbft_sync_receipt.accepted);
  EXPECT_EQ(get_pbft_sync_receipt.payload_id, 6);

  const auto dag_block_receipt = network_api->consensus_network_ingest_packet(packet(5, peer, bytes({0xC0, 0x04})));
  EXPECT_TRUE(dag_block_receipt.accepted);
  EXPECT_EQ(dag_block_receipt.payload_id, 7);

  const auto dag_sync_receipt = network_api->consensus_network_ingest_packet(packet(6, peer, bytes({0xC0, 0x05})));
  EXPECT_TRUE(dag_sync_receipt.accepted);
  EXPECT_EQ(dag_sync_receipt.payload_id, 8);

  const auto pillar_vote_receipt = network_api->consensus_network_ingest_packet(packet(13, peer, bytes({0xC0, 0x06})));
  EXPECT_TRUE(pillar_vote_receipt.accepted);
  EXPECT_EQ(pillar_vote_receipt.payload_id, 9);

  const auto get_pillar_votes_bundle_receipt =
      network_api->consensus_network_ingest_packet(packet(14, peer, bytes({0xC0, 0x07})));
  EXPECT_TRUE(get_pillar_votes_bundle_receipt.accepted);
  EXPECT_EQ(get_pillar_votes_bundle_receipt.payload_id, 10);

  const auto pillar_votes_bundle_receipt =
      network_api->consensus_network_ingest_packet(packet(15, peer, bytes({0xC0, 0x07})));
  EXPECT_TRUE(pillar_votes_bundle_receipt.accepted);
  EXPECT_EQ(pillar_votes_bundle_receipt.payload_id, 11);
}

TEST(ConsensusNetworkApiBridgeTest, ingestPacketRejectsEmptyPayloadWithoutAllocatingIngress) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());
  const auto peer = nodeId(0x22);

  rust::Vec<uint8_t> empty;
  const auto receipt = network_api->consensus_network_ingest_packet(packet(1, peer, std::move(empty)));

  EXPECT_FALSE(receipt.accepted);
  EXPECT_EQ(receipt.payload_id, 0);
  EXPECT_EQ(receipt.status, 1);
  EXPECT_EQ(receipt.error_code, "NETWORK_INGRESS_REJECTED_EMPTY_PAYLOAD");
}

TEST(ConsensusNetworkApiBridgeTest, ingestPacketRejectsUnsupportedPacketType) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());
  const auto peer = nodeId(0x33);

  const auto receipt = network_api->consensus_network_ingest_packet(packet(9, peer, bytes({1})));

  EXPECT_FALSE(receipt.accepted);
  EXPECT_EQ(receipt.payload_id, 0);
  EXPECT_EQ(receipt.status, 2);
  EXPECT_EQ(receipt.error_code, "NETWORK_INGRESS_UNSUPPORTED_PACKET_TYPE");
}

TEST(ConsensusNetworkApiBridgeTest, drainWorkAndReportResultsExposeExecutorContract) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto batch = network_api->consensus_network_drain_work(10);
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

  rustaxa::NetworkPbftVoteAdmissionEffects effects{};
  effects.peer_id = nodeId(0x33);
  effects.vote_hash = hash(0x44);
  effects.source_payload_id = 105;
  effects.mark_vote_known = true;

  const auto decision = network_api->consensus_network_queue_pbft_vote_admission_effects(effects);
  ASSERT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
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

TEST(ConsensusNetworkApiBridgeTest, voteIngressPlanningRoutesThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto accepted = network_api->consensus_network_plan_pbft_vote_ingress(voteFact(10, 3, 2, 2), voteContext());
  EXPECT_TRUE(accepted.accepted);
  EXPECT_EQ(accepted.status, 0);
  EXPECT_TRUE(accepted.error_code.empty());

  const auto rejected = network_api->consensus_network_plan_pbft_vote_ingress(voteFact(14, 3, 1, 2), voteContext());
  EXPECT_FALSE(rejected.accepted);
  EXPECT_EQ(rejected.status, 3);
  EXPECT_TRUE(rejected.request_pbft_sync);
  EXPECT_EQ(rejected.error_code, "PBFT_VOTE_INGRESS_INVALID_PERIOD_TOO_BIG");
}

TEST(ConsensusNetworkApiBridgeTest, voteBundleIngressPlanningRoutesThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto plan = network_api->consensus_network_plan_pbft_vote_bundle_ingress(voteFact(10, 3, 2, 2),
                                                                                 voteFact(10, 3, 3, 2), voteContext());

  EXPECT_FALSE(plan.accepted);
  EXPECT_EQ(plan.status, 8);
  EXPECT_EQ(plan.error_code, "PBFT_VOTE_INGRESS_BUNDLE_VOTE_MISMATCH");
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteIngressQueuesSyncEffectThroughNetworkApi) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto decision = network_api->consensus_network_ingest_pbft_vote(voteFact(14, 3, 1, 2), networkVoteContext());

  EXPECT_TRUE(decision.routed);
  EXPECT_TRUE(decision.payload_accepted);
  EXPECT_EQ(decision.payload_id, 99);
  EXPECT_EQ(decision.status, 3);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 3);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x44));
  EXPECT_EQ(batch.effects[0].sync_kind, 0);
  EXPECT_EQ(batch.effects[0].sync_start, 13);
  EXPECT_EQ(batch.effects[0].source_payload_id, 99);
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteBundleIngressQueuesReportAndDisconnectEffects) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  const auto decision = network_api->consensus_network_ingest_pbft_vote_bundle_member(
      voteFact(10, 3, 2, 1), voteFact(10, 3, 2, 1), networkVoteContext());

  EXPECT_EQ(decision.status, 7);
  EXPECT_EQ(decision.queued_effect_count, 2);

  const auto batch = network_api->consensus_network_drain_work(10);
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

TEST(ConsensusNetworkApiBridgeTest, pbftVoteAdmissionEffectsQueueMarkKnownEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftVoteAdmissionEffects effects{};
  effects.peer_id = nodeId(0x55);
  effects.vote_hash = hash(0xAB);
  effects.source_payload_id = 101;
  effects.mark_vote_known = true;

  const auto decision = network_api->consensus_network_queue_pbft_vote_admission_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 2);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x55));
  EXPECT_EQ(batch.effects[0].object_kind, 0);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xAB));
  EXPECT_EQ(batch.effects[0].source_payload_id, 101);
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteAdmissionRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftVoteAdmissionRequestEffects effects{};
  effects.peer_id = nodeId(0x56);
  effects.vote_hash = hash(0xAC);
  effects.source_payload_id = 106;
  effects.admit_vote = true;

  const auto decision = network_api->consensus_network_queue_pbft_vote_admission_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x56));
  EXPECT_EQ(batch.effects[0].packet_kind, 1);
  EXPECT_EQ(batch.effects[0].object_kind, 0);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xAC));
  EXPECT_EQ(batch.effects[0].source_payload_id, 106);
}

TEST(ConsensusNetworkApiBridgeTest, pbftBlockAdmissionEffectsQueueMarkKnownEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftBlockAdmissionEffects effects{};
  effects.peer_id = nodeId(0x66);
  effects.block_hash = hash(0xCD);
  effects.source_payload_id = 102;
  effects.mark_block_known = true;

  const auto decision = network_api->consensus_network_queue_pbft_block_admission_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 2);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x66));
  EXPECT_EQ(batch.effects[0].object_kind, 1);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xCD));
  EXPECT_EQ(batch.effects[0].source_payload_id, 102);
}

TEST(ConsensusNetworkApiBridgeTest, pbftVoteGossipEffectsQueueGossipPacketEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftVoteGossipEffects effects{};
  effects.peer_id = nodeId(0x77);
  effects.vote_hash = hash(0xEF);
  effects.source_payload_id = 103;
  effects.gossip_vote = true;

  const auto decision = network_api->consensus_network_queue_pbft_vote_gossip_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 1);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x77));
  EXPECT_EQ(batch.effects[0].packet_kind, 1);
  ASSERT_EQ(batch.effects[0].exclude_peers.size(), 1);
  EXPECT_EQ(batch.effects[0].exclude_peers[0].id, nodeId(0x77));
  EXPECT_EQ(batch.effects[0].object_kind, 0);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xEF));
  EXPECT_EQ(batch.effects[0].source_payload_id, 103);
}

TEST(ConsensusNetworkApiBridgeTest, pbftNextVotesBundleEgressRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftNextVotesBundleEgressRequestEffects effects{};
  effects.peer_id = nodeId(0x87);
  effects.period = 50;
  effects.round = 7;
  effects.source_payload_id = 104;
  effects.request_bundle = true;

  const auto decision = network_api->consensus_network_queue_pbft_next_votes_bundle_egress_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x87));
  EXPECT_EQ(batch.effects[0].packet_kind, 2);
  EXPECT_TRUE(batch.effects[0].payload_bytes.empty());
  EXPECT_EQ(batch.effects[0].object_kind, 7);
  EXPECT_EQ(batch.effects[0].object_hash[7], 50);
  EXPECT_EQ(batch.effects[0].object_hash[15], 7);
  EXPECT_EQ(batch.effects[0].object_hash[23], 104);
  EXPECT_EQ(batch.effects[0].period, 50);
  EXPECT_EQ(batch.effects[0].round, 7);
  EXPECT_EQ(batch.effects[0].source_payload_id, 104);
}

TEST(ConsensusNetworkApiBridgeTest, pbftSyncEgressRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftSyncEgressRequestEffects effects{};
  effects.peer_id = nodeId(0x89);
  effects.from_period = 60;
  effects.blocks_to_transfer = 9;
  effects.pbft_chain_synced = true;
  effects.source_payload_id = 105;
  effects.request_sync = true;

  const auto decision = network_api->consensus_network_queue_pbft_sync_egress_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x89));
  EXPECT_EQ(batch.effects[0].packet_kind, 10);
  EXPECT_TRUE(batch.effects[0].payload_bytes.empty());
  EXPECT_EQ(batch.effects[0].object_kind, 8);
  EXPECT_EQ(batch.effects[0].object_hash[7], 60);
  EXPECT_EQ(batch.effects[0].object_hash[15], 9);
  EXPECT_EQ(batch.effects[0].object_hash[23], 105);
  EXPECT_EQ(batch.effects[0].sync_kind, 0);
  EXPECT_EQ(batch.effects[0].sync_start, 60);
  EXPECT_EQ(batch.effects[0].reason_code, 1);
  EXPECT_EQ(batch.effects[0].dependency_id, 9);
  EXPECT_EQ(batch.effects[0].period, 60);
  EXPECT_EQ(batch.effects[0].source_payload_id, 105);
}

TEST(ConsensusNetworkApiBridgeTest, pillarVotesBundleEgressRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPillarVotesBundleEgressRequestEffects effects{};
  effects.peer_id = nodeId(0x8A);
  effects.period = 61;
  effects.pillar_block_hash = hash(0xAB);
  effects.source_payload_id = 106;
  effects.request_bundle = true;

  const auto decision = network_api->consensus_network_queue_pillar_votes_bundle_egress_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x8A));
  EXPECT_EQ(batch.effects[0].packet_kind, 14);
  EXPECT_TRUE(batch.effects[0].payload_bytes.empty());
  EXPECT_EQ(batch.effects[0].object_kind, 9);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xAB));
  EXPECT_EQ(batch.effects[0].period, 61);
  EXPECT_EQ(batch.effects[0].source_payload_id, 106);
}

TEST(ConsensusNetworkApiBridgeTest, pbftProposedBlockSidecarQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftProposedBlockSidecarEffects effects{};
  effects.peer_id = nodeId(0x88);
  effects.period = 42;
  effects.block_hash = hash(0xA1);
  effects.pivot_hash = hash(0xB2);
  effects.block_rlp = bytes({0xC0, 0x01});
  effects.source_payload_id = 107;
  effects.record_block = true;

  const auto decision = network_api->consensus_network_queue_pbft_proposed_block_sidecar_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x88));
  EXPECT_EQ(batch.effects[0].packet_kind, 1);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x01);
  EXPECT_EQ(batch.effects[0].object_kind, 1);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xA1));
  EXPECT_EQ(batch.effects[0].period, 42);
  EXPECT_EQ(batch.effects[0].source_payload_id, 107);
}

TEST(ConsensusNetworkApiBridgeTest, pbftProposedBlockBundleQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftProposedBlockSidecarEffects effects{};
  effects.peer_id = nodeId(0x99);
  effects.period = 43;
  effects.block_hash = hash(0xC1);
  effects.pivot_hash = hash(0xD2);
  effects.block_rlp = bytes({0xC0, 0x02});
  effects.source_payload_id = 105;
  effects.record_block = true;

  const auto decision = network_api->consensus_network_queue_pbft_proposed_block_bundle_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x99));
  EXPECT_EQ(batch.effects[0].packet_kind, 16);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x02);
  EXPECT_EQ(batch.effects[0].object_kind, 1);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xC1));
  EXPECT_EQ(batch.effects[0].period, 43);
  EXPECT_EQ(batch.effects[0].source_payload_id, 105);
}

TEST(ConsensusNetworkApiBridgeTest, pbftSyncPeriodDataAdmissionRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPbftSyncPeriodDataAdmissionRequestEffects effects{};
  effects.peer_id = nodeId(0x9A);
  effects.block_hash = hash(0xC2);
  effects.period = 44;
  effects.period_data_rlp = bytes({0xC0, 0x06});
  effects.current_block_cert_vote_count = 4;
  effects.source_payload_id = 106;
  effects.admit_period_data = true;

  const auto decision = network_api->consensus_network_queue_pbft_sync_period_data_admission_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x9A));
  EXPECT_EQ(batch.effects[0].packet_kind, 11);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x06);
  EXPECT_EQ(batch.effects[0].object_kind, 4);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xC2));
  EXPECT_EQ(batch.effects[0].period, 44);
  EXPECT_EQ(batch.effects[0].dependency_id, 4);
  EXPECT_EQ(batch.effects[0].source_payload_id, 106);
}

TEST(ConsensusNetworkApiBridgeTest, pillarVoteAdmissionRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPillarVoteAdmissionRequestEffects effects{};
  effects.peer_id = nodeId(0x9B);
  effects.vote_hash = hash(0xC3);
  effects.period = 45;
  effects.vote_rlp = bytes({0xC0, 0x07});
  effects.source_payload_id = 108;
  effects.admit_vote = true;

  const auto decision = network_api->consensus_network_queue_pillar_vote_admission_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x9B));
  EXPECT_EQ(batch.effects[0].packet_kind, 13);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x07);
  EXPECT_EQ(batch.effects[0].object_kind, 5);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xC3));
  EXPECT_EQ(batch.effects[0].period, 45);
  EXPECT_EQ(batch.effects[0].source_payload_id, 108);
}

TEST(ConsensusNetworkApiBridgeTest, pillarVoteBundleMemberAdmissionRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPillarVoteAdmissionRequestEffects effects{};
  effects.peer_id = nodeId(0x9C);
  effects.vote_hash = hash(0xC4);
  effects.period = 46;
  effects.vote_rlp = bytes({0xC0, 0x08});
  effects.source_payload_id = 109;
  effects.admit_vote = true;

  const auto decision =
      network_api->consensus_network_queue_pillar_vote_bundle_member_admission_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x9C));
  EXPECT_EQ(batch.effects[0].packet_kind, 15);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x08);
  EXPECT_EQ(batch.effects[0].object_kind, 5);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xC4));
  EXPECT_EQ(batch.effects[0].period, 46);
  EXPECT_EQ(batch.effects[0].source_payload_id, 109);
}

TEST(ConsensusNetworkApiBridgeTest, pillarVoteValidationRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPillarVoteValidationRequestEffects effects{};
  effects.peer_id = nodeId(0x9D);
  effects.vote_hash = hash(0xC5);
  effects.period = 47;
  effects.vote_rlp = bytes({0xC0, 0x09});
  effects.source_payload_id = 110;
  effects.validate_vote = true;

  const auto decision = network_api->consensus_network_queue_pillar_vote_validation_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x9D));
  EXPECT_EQ(batch.effects[0].packet_kind, 13);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x09);
  EXPECT_EQ(batch.effects[0].object_kind, 6);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xC5));
  EXPECT_EQ(batch.effects[0].period, 47);
  EXPECT_EQ(batch.effects[0].source_payload_id, 110);
}

TEST(ConsensusNetworkApiBridgeTest, pillarVoteBundleMemberValidationRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkPillarVoteValidationRequestEffects effects{};
  effects.peer_id = nodeId(0x9E);
  effects.vote_hash = hash(0xC6);
  effects.period = 48;
  effects.vote_rlp = bytes({0xC0, 0x0A});
  effects.source_payload_id = 111;
  effects.validate_vote = true;

  const auto decision =
      network_api->consensus_network_queue_pillar_vote_bundle_member_validation_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0x9E));
  EXPECT_EQ(batch.effects[0].packet_kind, 15);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x0A);
  EXPECT_EQ(batch.effects[0].object_kind, 6);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xC6));
  EXPECT_EQ(batch.effects[0].period, 48);
  EXPECT_EQ(batch.effects[0].source_payload_id, 111);
}

TEST(ConsensusNetworkApiBridgeTest, transactionAdmissionRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkTransactionAdmissionRequestEffects effects{};
  effects.peer_id = nodeId(0xAA);
  effects.transaction_hash = hash(0xE1);
  effects.transaction_rlp = bytes({0xC0, 0x03});
  effects.source_payload_id = 107;
  effects.admit_transaction = true;

  const auto decision = network_api->consensus_network_queue_transaction_admission_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0xAA));
  EXPECT_EQ(batch.effects[0].packet_kind, 7);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x03);
  EXPECT_EQ(batch.effects[0].object_kind, 2);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xE1));
  EXPECT_EQ(batch.effects[0].source_payload_id, 107);
}

TEST(ConsensusNetworkApiBridgeTest, dagBlockAdmissionRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkDagBlockAdmissionRequestEffects effects{};
  effects.peer_id = nodeId(0xAB);
  effects.block_hash = hash(0xE2);
  effects.block_rlp = bytes({0xC0, 0x04});
  effects.transaction_count = 2;
  effects.source_payload_id = 107;
  effects.admit_block = true;

  const auto decision = network_api->consensus_network_queue_dag_block_admission_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0xAB));
  EXPECT_EQ(batch.effects[0].packet_kind, 5);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x04);
  EXPECT_EQ(batch.effects[0].object_kind, 3);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xE2));
  EXPECT_EQ(batch.effects[0].source_payload_id, 107);
  EXPECT_EQ(batch.effects[0].dependency_id, 2);
}

TEST(ConsensusNetworkApiBridgeTest, dagSyncBlockAdmissionRequestQueuesRecordObjectEffect) {
  auto network_api = rustaxa::create_consensus_network_api(defaultConfig());

  rustaxa::NetworkDagBlockAdmissionRequestEffects effects{};
  effects.peer_id = nodeId(0xAC);
  effects.block_hash = hash(0xE3);
  effects.block_rlp = bytes({0xC0, 0x05});
  effects.transaction_count = 3;
  effects.source_payload_id = 108;
  effects.admit_block = true;

  const auto decision = network_api->consensus_network_queue_dag_sync_block_admission_request_effects(effects);
  EXPECT_TRUE(decision.routed);
  EXPECT_EQ(decision.status, 0);
  EXPECT_EQ(decision.queued_effect_count, 1);

  const auto batch = network_api->consensus_network_drain_work(10);
  ASSERT_EQ(batch.effects.size(), 1);
  EXPECT_EQ(batch.effects[0].kind, 8);
  EXPECT_EQ(batch.effects[0].peer_id, nodeId(0xAC));
  EXPECT_EQ(batch.effects[0].packet_kind, 6);
  ASSERT_EQ(batch.effects[0].payload_bytes.size(), 2);
  EXPECT_EQ(batch.effects[0].payload_bytes[0], 0xC0);
  EXPECT_EQ(batch.effects[0].payload_bytes[1], 0x05);
  EXPECT_EQ(batch.effects[0].object_kind, 3);
  EXPECT_EQ(batch.effects[0].object_hash, hash(0xE3));
  EXPECT_EQ(batch.effects[0].source_payload_id, 108);
  EXPECT_EQ(batch.effects[0].dependency_id, 3);
}
