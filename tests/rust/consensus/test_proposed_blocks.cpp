#include <gtest/gtest.h>

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

#include <libdevcore/RLP.h>
#include <libdevcore/SHA3.h>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

class RustProposedBlocksTest : public ::testing::Test {
 protected:
  static std::filesystem::path uniqueTempDir(const std::string& name) {
    const auto nonce = std::chrono::steady_clock::now().time_since_epoch().count();
    auto path = std::filesystem::temp_directory_path() / (name + "_" + std::to_string(nonce));
    std::filesystem::create_directories(path);
    return path;
  }

  static std::array<uint8_t, 32> h256(uint8_t last_byte) {
    std::array<uint8_t, 32> hash{};
    hash[31] = last_byte;
    return hash;
  }

  static rust::Vec<uint8_t> bytes(const dev::bytes& data) {
    rust::Vec<uint8_t> out;
    out.reserve(data.size());
    for (auto byte : data) {
      out.push_back(byte);
    }
    return out;
  }

  static std::vector<uint8_t> to_std(const rust::Vec<uint8_t>& data) {
    return {data.begin(), data.end()};
  }

  static rust::Vec<uint8_t> copy(const rust::Vec<uint8_t>& data) {
    rust::Vec<uint8_t> out;
    out.reserve(data.size());
    for (const auto byte : data) {
      out.push_back(byte);
    }
    return out;
  }

  struct ProposedBlockInput {
    std::array<uint8_t, 32> block_hash;
    std::array<uint8_t, 32> pivot_hash;
    rust::Vec<uint8_t> block_rlp;
  };

  static ProposedBlockInput proposedBlock(uint64_t period, uint8_t pivot_last_byte) {
    dev::RLPStream stream(8);
    stream << dev::h256(1) << dev::h256(pivot_last_byte) << dev::h256(2) << dev::h256(3) << period << uint64_t{11}
           << dev::h256(4) << dev::bytes(65, 0);
    auto block_rlp = stream.out();
    return ProposedBlockInput{dev::sha3(block_rlp).asArray(), dev::h256(pivot_last_byte).asArray(), bytes(block_rlp)};
  }

  static PbftServiceConfig serviceConfig() {
    PbftServiceConfig config{};
    config.genesis_lambda_ms = 1000;
    config.cacti_lambda_max_ms = 1000;
    config.cacti_lambda_default_ms = 1000;
    config.max_exponential_lambda_ms = 60000;
    config.max_steps = 13;
    config.deadline_ms = 4000;
    config.polling_interval_ms = 100;
    return config;
  }
};

TEST_F(RustProposedBlocksTest, PushGetMarkValidAndSnapshotEntries) {
  const auto test_dir = uniqueTempDir("rustaxa_proposed_blocks_bridge");
  auto storage = create_storage(test_dir.string());
  auto service = create_pbft_service_from_storage(*storage, serviceConfig());
  auto block = proposedBlock(2, 0x99);

  EXPECT_TRUE(service->pbft_service_proposed_blocks_push_with_storage(
      2, block.block_hash, block.pivot_hash, copy(block.block_rlp)));
  EXPECT_FALSE(service->pbft_service_proposed_blocks_push_with_storage(
      2, block.block_hash, block.pivot_hash, copy(block.block_rlp)));

  auto lookup = service->pbft_service_proposed_blocks_get(2, block.block_hash);
  EXPECT_TRUE(lookup.found);
  EXPECT_FALSE(lookup.is_valid);
  EXPECT_EQ(lookup.pivot_hash, block.pivot_hash);
  EXPECT_EQ(to_std(lookup.block_rlp), to_std(block.block_rlp));
  auto metadata = service->pbft_service_proposed_blocks_metadata(2, block.block_hash);
  EXPECT_TRUE(metadata.found);
  EXPECT_FALSE(metadata.is_valid);
  EXPECT_EQ(metadata.pivot_hash, block.pivot_hash);

  EXPECT_TRUE(service->pbft_service_proposed_blocks_contains(2, block.block_hash));
  EXPECT_FALSE(service->pbft_service_proposed_blocks_contains(2, h256(0x12)));

  service->pbft_service_proposed_blocks_mark_valid(2, block.block_hash);
  lookup = service->pbft_service_proposed_blocks_get(2, block.block_hash);
  EXPECT_TRUE(lookup.is_valid);
  metadata = service->pbft_service_proposed_blocks_metadata(2, block.block_hash);
  EXPECT_TRUE(metadata.is_valid);

  auto entries = service->pbft_service_proposed_blocks_snapshot_entries();
  ASSERT_EQ(entries.size(), 1);
  EXPECT_EQ(entries[0].period, 2u);
  EXPECT_EQ(entries[0].block_hash, block.block_hash);
  EXPECT_EQ(entries[0].pivot_hash, block.pivot_hash);
  EXPECT_EQ(to_std(entries[0].block_rlp), to_std(block.block_rlp));
  EXPECT_TRUE(entries[0].is_valid);

  std::filesystem::remove_all(test_dir);
}

TEST_F(RustProposedBlocksTest, MarkValidThrowsForMissingBlock) {
  const auto test_dir = uniqueTempDir("rustaxa_proposed_blocks_missing");
  auto storage = create_storage(test_dir.string());
  auto service = create_pbft_service_from_storage(*storage, serviceConfig());

  EXPECT_THROW(service->pbft_service_proposed_blocks_mark_valid(9, h256(0x90)), std::exception);

  std::filesystem::remove_all(test_dir);
}
