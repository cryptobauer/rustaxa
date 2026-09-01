#include <gtest/gtest.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <cstdint>
#include <filesystem>
#include <future>
#include <string>
#include <utility>
#include <vector>

#include "config/config.hpp"
#include "consensus/consensus_application.hpp"
#include "consensus/consensus_host_ports.hpp"
#include "consensus_application_test.hpp"
#include "network/tarcap/packets_handlers/rust/consensus_transport_packet_handler.hpp"
#include "pillar_chain/pillar_block.hpp"
#include "rustaxa-bridge/application_host_ffi.rs.h"
#include "rustaxa-bridge/ffi.rs.h"
#include "vote/pillar_vote.hpp"

namespace {

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
  config.deep_syncing_threshold = 100;
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
        network_api(rustaxa::create_consensus_network_api(*service)) {}

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

rust::Vec<uint8_t> bridgeBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> result;
  result.reserve(bytes.size());
  for (const auto byte : bytes) result.push_back(byte);
  return result;
}

}  // namespace

TEST(ConsensusObserverBridgeTest, canonicalPillarDataPublishesThroughHostPort) {
  TemporaryStorageDirectory directory;
  taraxa::FullNodeConfig config{};
  config.db_path = directory.path;
  auto application = taraxa::createConsensusApplication(config);
  taraxa::ConsensusProcessPort process(config, application);
  auto block = std::make_shared<taraxa::pillar_chain::PillarBlock>(
      21, dev::h256(1), taraxa::blk_hash_t(2), dev::h256(3), 4,
      std::vector<taraxa::pillar_chain::PillarBlock::ValidatorVoteCountChange>{});
  auto vote = std::make_shared<taraxa::PillarVote>(taraxa::secret_t::random(), 22, block->getHash());
  const taraxa::pillar_chain::PillarBlockData block_data(block, {vote});
  auto execution_context = std::make_shared<taraxa::util::ThreadPool>(1);
  std::promise<taraxa::PbftPeriod> observed_period;
  auto observed = observed_period.get_future();
  application->pillarBlockObserved().subscribe(
      [&observed_period](const taraxa::pillar_chain::PillarBlockData& value) {
        observed_period.set_value(value.block_->getPeriod());
      },
      execution_context);

  rustaxa::HostConsensusObservationRequest request{};
  request.effect_id = rustaxa::HostEffectId{7, 7};
  request.kind = 3;
  request.hash = block->getHash().asArray();
  request.canonical_rlp = bridgeBytes(block_data.getRlp());
  const auto report = process.consensusObserve(request);

  EXPECT_EQ(report.effect_id.generation, 7);
  EXPECT_EQ(report.effect_id.sequence, 7);
  EXPECT_TRUE(report.succeeded) << std::string(report.error_code);
  ASSERT_EQ(observed.wait_for(std::chrono::seconds(1)), std::future_status::ready);
  EXPECT_EQ(observed.get(), 21);
}

TEST(ConsensusObserverBridgeTest, rejectsMismatchedAndMalformedPillarData) {
  TemporaryStorageDirectory directory;
  taraxa::FullNodeConfig config{};
  config.db_path = directory.path;
  auto application = taraxa::createConsensusApplication(config);
  taraxa::ConsensusProcessPort process(config, application);
  auto block = std::make_shared<taraxa::pillar_chain::PillarBlock>(
      21, dev::h256(1), taraxa::blk_hash_t(2), dev::h256(3), 4,
      std::vector<taraxa::pillar_chain::PillarBlock::ValidatorVoteCountChange>{});
  auto vote = std::make_shared<taraxa::PillarVote>(taraxa::secret_t::random(), 22, block->getHash());
  const taraxa::pillar_chain::PillarBlockData block_data(block, {vote});

  rustaxa::HostConsensusObservationRequest mismatch{};
  mismatch.effect_id = rustaxa::HostEffectId{7, 8};
  mismatch.kind = 3;
  mismatch.hash = hash(0xff);
  mismatch.canonical_rlp = bridgeBytes(block_data.getRlp());
  const auto mismatch_report = process.consensusObserve(mismatch);
  EXPECT_FALSE(mismatch_report.succeeded);
  EXPECT_EQ(std::string(mismatch_report.error_code), "OBSERVED_PILLAR_BLOCK_HASH_MISMATCH");

  rustaxa::HostConsensusObservationRequest malformed{};
  malformed.effect_id = rustaxa::HostEffectId{7, 9};
  malformed.kind = 3;
  malformed.hash = block->getHash().asArray();
  malformed.canonical_rlp.push_back(0x80);
  const auto malformed_report = process.consensusObserve(malformed);
  EXPECT_FALSE(malformed_report.succeeded);
  EXPECT_TRUE(std::string(malformed_report.error_code).starts_with("OBSERVATION_FAILED:"));
}

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

TEST(ConsensusNetworkApiBridgeTest, pillarBundleRouteAcceptsCanonicalBytesAndRejectsMalformedPackets) {
  auto network_api = NetworkApiFixture{};
  rustaxa::NetworkCanonicalRequestPacket request{};
  request.transport_lane = 6;
  request.peer_id.fill(0x42);
  request.source_payload_id = 91;
  dev::RLPStream packet(2);
  packet << uint64_t{11} << dev::h256(0x55);
  request.packet_rlp = bridgeBytes(packet.out());

  const auto no_data = network_api->consensus_network_ingest_pillar_votes_bundle_request(std::move(request));
  EXPECT_EQ(no_data.status, 9);
  EXPECT_EQ(no_data.queued_effect_count, 0);

  rustaxa::NetworkCanonicalRequestPacket malformed{};
  malformed.transport_lane = 6;
  malformed.peer_id.fill(0x42);
  malformed.source_payload_id = 92;
  malformed.packet_rlp.push_back(0xc1);
  malformed.packet_rlp.push_back(0x0b);
  const auto rejected = network_api->consensus_network_ingest_pillar_votes_bundle_request(std::move(malformed));
  EXPECT_EQ(rejected.status, 11);
  EXPECT_EQ(rejected.queued_effect_count, 0);
  EXPECT_EQ(std::string(rejected.error_code), "NETWORK_GET_PILLAR_VOTES_BUNDLE_MALFORMED_RLP");
}

TEST(ConsensusNetworkApiBridgeTest, statusAndLifecycleCommandsShareApplicationOwnedSyncState) {
  auto network_api = NetworkApiFixture{};

  rustaxa::NetworkPbftSyncStartRequest start{};
  start.start = true;
  start.now_ms = 100;
  start.local_pbft_synced_period = 3;
  start.local_pbft_chain_size = 3;
  rustaxa::NetworkPbftSyncPeerCandidate candidate{};
  candidate.peer_id.fill(0x42);
  candidate.pbft_chain_size = 8;
  candidate.peer_dag_synced = true;
  candidate.dag_sync_allowed = true;
  start.candidates.push_back(candidate);
  const auto started = network_api->consensus_network_begin_pbft_sync(std::move(start));
  ASSERT_TRUE(started.started) << std::string(started.error_code);
  EXPECT_EQ(started.peer_id, candidate.peer_id);

  rustaxa::NetworkStatusPacketBuildRequest status{};
  status.initial = true;
  status.local_pbft_chain_size = 3;
  status.local_pbft_round = 2;
  status.local_dag_level = 7;
  const auto egress = network_api->consensus_network_build_status_packet(status);
  EXPECT_EQ(egress.status, 0);
  EXPECT_FALSE(egress.packet_rlp.empty());

  rustaxa::NetworkPbftSyncCommand activity{};
  activity.kind = 1;
  activity.now_ms = 125;
  activity.generation = started.generation;
  activity.peer_id = started.peer_id;
  const auto activity_outcome = network_api->consensus_network_apply_pbft_sync_command(activity);
  EXPECT_TRUE(activity_outcome.accepted) << std::string(activity_outcome.error_code);
  EXPECT_EQ(activity_outcome.generation, started.generation);

  activity.kind = 99;
  EXPECT_THROW(network_api->consensus_network_apply_pbft_sync_command(activity), std::exception);
}

TEST(ConsensusNetworkApiBridgeTest, failedFutureVotePbftSyncSendRollsBackNativeGeneration) {
  auto network_api = NetworkApiFixture{};
  auto query = rustaxa::create_consensus_query_api(*network_api.service);

  rustaxa::NetworkPbftSyncStartRequest native_start{};
  native_start.start = true;
  native_start.now_ms = 100;
  native_start.local_pbft_synced_period = 9;
  native_start.local_pbft_chain_size = 9;
  rustaxa::NetworkPbftSyncPeerCandidate native_candidate{};
  native_candidate.peer_id.fill(0x42);
  native_candidate.pbft_chain_size = 20;
  native_candidate.dag_level = 20;
  native_candidate.peer_dag_synced = true;
  native_candidate.dag_sync_allowed = true;
  native_start.candidates.push_back(native_candidate);
  const auto native_started = network_api->consensus_network_begin_pbft_sync(std::move(native_start));
  ASSERT_TRUE(native_started.started) << std::string(native_started.error_code);

  taraxa::network::PbftSyncStartOutcome started{
      native_started.status,         std::string(native_started.error_code),
      native_started.started,        native_started.has_peer,
      native_started.peer_id,        native_started.peer_pbft_chain_size,
      native_started.request_period, native_started.generation,
      native_started.deep_syncing,   native_started.enable_snapshot_creation,
  };
  taraxa::network::ConsensusTransportEffect effect{};
  effect.kind = 3;
  effect.sync_kind = 0;
  effect.peer_id = started.peer_id;
  effect.sync_start = started.request_period;
  effect.payload_bytes = {0xc1, static_cast<uint8_t>(started.request_period)};

  const auto execution = taraxa::network::tarcap::executePbftSyncTransportRequest(
      effect, started,
      [&network_api](uint64_t generation, const std::array<uint8_t, 64>& peer_id, uint8_t reason) {
        rustaxa::NetworkPbftSyncCommand stop{};
        stop.kind = 2;
        stop.generation = generation;
        stop.peer_id = peer_id;
        stop.reason = reason;
        const auto outcome = network_api->consensus_network_apply_pbft_sync_command(stop);
        return taraxa::network::PbftSyncLifecycleOutcome{outcome.accepted,
                                                         outcome.active,
                                                         outcome.stopped,
                                                         outcome.expired,
                                                         outcome.restart_sync,
                                                         outcome.retry,
                                                         outcome.request_next,
                                                         outcome.request_pending_dag_if_idle,
                                                         outcome.deep_syncing,
                                                         outcome.generation,
                                                         std::string(outcome.error_code)};
      },
      {{[](const taraxa::network::ConsensusTransportEffect&) {
        return taraxa::network::ConsensusTransportExecutionResult{false, "injected send failure"};
      }}});

  EXPECT_FALSE(execution.success);
  auto status = query->consensus_query_pbft_sync_status(101);
  EXPECT_FALSE(status.active);
  EXPECT_EQ(status.last_stop_reason, 4);

  rustaxa::NetworkPbftSyncStartRequest malformed_start{};
  malformed_start.start = true;
  malformed_start.now_ms = 200;
  malformed_start.local_pbft_synced_period = 9;
  malformed_start.local_pbft_chain_size = 9;
  malformed_start.candidates.push_back(native_candidate);
  const auto native_malformed_started = network_api->consensus_network_begin_pbft_sync(std::move(malformed_start));
  ASSERT_TRUE(native_malformed_started.started) << std::string(native_malformed_started.error_code);
  started = {native_malformed_started.status,         std::string(native_malformed_started.error_code),
             native_malformed_started.started,        native_malformed_started.has_peer,
             native_malformed_started.peer_id,        native_malformed_started.peer_pbft_chain_size,
             native_malformed_started.request_period, native_malformed_started.generation,
             native_malformed_started.deep_syncing,   native_malformed_started.enable_snapshot_creation};
  effect.peer_id = started.peer_id;
  effect.sync_start = started.request_period;
  effect.payload_bytes = {0x80};
  bool malformed_sent = false;
  const auto malformed = taraxa::network::tarcap::executePbftSyncTransportRequest(
      effect, started,
      [&network_api](uint64_t generation, const std::array<uint8_t, 64>& peer_id, uint8_t reason) {
        rustaxa::NetworkPbftSyncCommand stop{};
        stop.kind = 2;
        stop.generation = generation;
        stop.peer_id = peer_id;
        stop.reason = reason;
        const auto outcome = network_api->consensus_network_apply_pbft_sync_command(stop);
        return taraxa::network::PbftSyncLifecycleOutcome{outcome.accepted,
                                                         outcome.active,
                                                         outcome.stopped,
                                                         outcome.expired,
                                                         outcome.restart_sync,
                                                         outcome.retry,
                                                         outcome.request_next,
                                                         outcome.request_pending_dag_if_idle,
                                                         outcome.deep_syncing,
                                                         outcome.generation,
                                                         std::string(outcome.error_code)};
      },
      {{[&malformed_sent](const taraxa::network::ConsensusTransportEffect&) {
        malformed_sent = true;
        return taraxa::network::ConsensusTransportExecutionResult{};
      }}});
  EXPECT_FALSE(malformed.success);
  EXPECT_FALSE(malformed_sent);
  status = query->consensus_query_pbft_sync_status(201);
  EXPECT_FALSE(status.active);
  EXPECT_EQ(status.last_stop_reason, 4);
}
