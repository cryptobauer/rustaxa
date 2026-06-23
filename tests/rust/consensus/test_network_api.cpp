#include "rustaxa-bridge/ffi.rs.h"

#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <utility>

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
  config.max_retained_payloads = 8;
  config.max_effects_per_drain = 8;
  return config;
}

rustaxa::NetworkIngressPacket packet(uint32_t packet_type, std::array<uint8_t, 64> peer,
                                     rust::Vec<uint8_t> payload) {
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

  const auto second_receipt = network_api->consensus_network_ingest_packet(packet(3, peer, bytes({4})));
  EXPECT_TRUE(second_receipt.accepted);
  EXPECT_EQ(second_receipt.payload_id, 2);
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
