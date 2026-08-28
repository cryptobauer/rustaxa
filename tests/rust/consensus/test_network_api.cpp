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

#include "consensus_application_test.hpp"
#include "config/config.hpp"
#include "consensus/consensus_application.hpp"
#include "consensus/consensus_host_ports.hpp"
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
  auto application = std::make_shared<taraxa::ConsensusApplication>(
      rustaxa::test::createConsensusApplication(directory.path, serviceConfig()));
  taraxa::FullNodeConfig config{};
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
  auto application = std::make_shared<taraxa::ConsensusApplication>(
      rustaxa::test::createConsensusApplication(directory.path, serviceConfig()));
  taraxa::FullNodeConfig config{};
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

  rustaxa::NetworkStatusEgressRequest status{};
  status.initial = true;
  status.local_pbft_chain_size = 3;
  status.local_pbft_round = 2;
  status.local_dag_level = 7;
  const auto egress = network_api->consensus_network_status_egress(status);
  EXPECT_TRUE(egress.peer_syncing);
  EXPECT_TRUE(egress.include_initial_data);

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
