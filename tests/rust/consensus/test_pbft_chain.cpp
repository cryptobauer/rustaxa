#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <iomanip>
#include <sstream>
#include <string>
#include <string_view>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

class RustPbftChainTest : public ::testing::Test {
 protected:
  static rustaxa::PbftServiceConfig makePbftServiceConfig() {
    rustaxa::PbftServiceConfig config{};
    config.genesis_lambda_ms = 100;
    config.cacti_lambda_max_ms = 1500;
    config.cacti_lambda_default_ms = 500;
    config.cacti_block = 100;
    config.max_exponential_lambda_ms = 60000;
    config.max_steps = 13;
    config.deadline_ms = 1000;
    config.polling_interval_ms = 100;
    config.ficus_activation_period = 0;
    config.pillar_blocks_interval = 10;
    config.sync_level_size = 10;
    config.is_light_node = false;
    config.light_node_history = 0;
    config.committee_size = 5;
    config.number_of_proposers = 20;
    return config;
  }

  static std::array<uint8_t, 32> h256(uint8_t last_byte) {
    std::array<uint8_t, 32> hash{};
    hash[31] = last_byte;
    return hash;
  }

  static std::string hash_json(uint8_t last_byte) {
    std::ostringstream out;
    out << "0x";
    for (size_t i = 0; i < 31; ++i) {
      out << "00";
    }
    out << std::hex << std::setw(2) << std::setfill('0') << static_cast<unsigned>(last_byte);
    return out.str();
  }

  static std::string head_json(uint64_t size, uint64_t non_empty_size, uint8_t last_block) {
    std::ostringstream out;
    out << R"({"head_hash":")" << hash_json(0) << R"(","size":)" << size << R"(,"non_empty_size":)" << non_empty_size
        << R"(,"last_pbft_block_hash":")" << hash_json(last_block) << R"("})";
    return out.str();
  }

  static rust::Vec<uint8_t> bytes(std::string_view input) {
    rust::Vec<uint8_t> out;
    out.reserve(input.size());
    for (auto ch : input) {
      out.push_back(static_cast<uint8_t>(ch));
    }
    return out;
  }

  static rust::Box<BridgePbftService> create_chain(std::string_view name, uint64_t size, uint64_t non_empty_size,
                                                   uint8_t last_block) {
    const auto test_dir = std::filesystem::temp_directory_path() / std::string(name);
    if (std::filesystem::exists(test_dir)) {
      std::filesystem::remove_all(test_dir);
    }
    auto storage = create_storage(test_dir.string());
    auto batch = create_storage_shim_batch(*storage);
    const auto head = head_json(size, non_empty_size, last_block);
    storage_shim_save_pbft_head(*batch, h256(0), bytes(head));
    storage_shim_commit_batch(std::move(batch), false);
    auto chain = create_pbft_service_from_storage(*storage, makePbftServiceConfig());
    std::filesystem::remove_all(test_dir);
    return chain;
  }
};

TEST_F(RustPbftChainTest, UpdatesHeadStateForNonNullAndNullAnchors) {
  auto chain = create_chain("rustaxa_consensus_pbft_chain_update", 0, 0, 0);
  EXPECT_FALSE(chain->pbft_chain_initialized_default());

  auto current = chain->pbft_chain_head();
  EXPECT_EQ(current.size, 0);
  EXPECT_EQ(current.non_empty_size, 0);
  EXPECT_EQ(current.last_pbft_block_hash, h256(0));

  current = chain->pbft_chain_update(h256(12), h256(99));
  EXPECT_EQ(current.size, 1);
  EXPECT_EQ(current.non_empty_size, 1);
  EXPECT_EQ(current.last_pbft_block_hash, h256(12));
  EXPECT_EQ(current.last_non_null_anchor_hash, h256(99));

  current = chain->pbft_chain_update(h256(13), h256(0));
  EXPECT_EQ(current.size, 2);
  EXPECT_EQ(current.non_empty_size, 1);
  EXPECT_EQ(current.last_pbft_block_hash, h256(13));
  EXPECT_EQ(current.last_non_null_anchor_hash, h256(99));
}

TEST_F(RustPbftChainTest, RejectsImpossibleRecoveredHead) {
  EXPECT_THROW((void)create_chain("rustaxa_consensus_pbft_chain_invalid_head", 1, 2, 0), std::exception);
}
