#include <gtest/gtest.h>

#include <array>
#include <filesystem>
#include <memory>

#include "consensus/consensus_application.hpp"
#include "consensus/consensus_host_ports.hpp"
#include "final_chain/final_chain.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"

namespace taraxa::final_chain {
namespace {

/**
 * Owns the production-shaped application root and exact concrete-EVM leaf used
 * by Rust-enabled host-port tests. Each test receives an isolated native store
 * and FinalChain state database; construction failures fail the fixture.
 */
class ExternalEvmPortTest : public ::testing::Test {
 protected:
  void SetUp() override {
    test_dir = std::filesystem::temp_directory_path() / "rustaxa_external_evm_port_test";
    std::filesystem::remove_all(test_dir);
  }

  void TearDown() override { std::filesystem::remove_all(test_dir); }

  void initialize() {
    config.genesis.state.dpos.yield_percentage = 0;
    config.db_path = test_dir / "db";
    application = createConsensusApplication(config);
    final_chain =
        std::make_shared<FinalChain>(config.db_path / "state_db", config, addr_t{}, application);
  }

  std::filesystem::path test_dir;
  FullNodeConfig config;
  SharedConsensusApplication application;
  std::shared_ptr<FinalChain> final_chain;
};

TEST_F(ExternalEvmPortTest, AccountFactsPreserveIdentityOrderAndMissingRows) {
  const auto present = addr_t::random();
  const auto missing = addr_t::random();
  config.genesis.state.initial_balances = {{present, 123}};
  initialize();

  ExternalEvmPort port(final_chain);
  rustaxa::HostFinalChainAccountFactsRequest request{};
  request.effect_id = {.generation = 7, .sequence = 9};
  request.addresses.push_back({.bytes = present.asArray()});
  request.addresses.push_back({.bytes = missing.asArray()});
  const auto report = port.consensusLoadFinalChainAccountFacts(request);

  EXPECT_TRUE(report.succeeded);
  EXPECT_EQ(report.effect_id.generation, 7);
  EXPECT_EQ(report.effect_id.sequence, 9);
  EXPECT_EQ(report.observed_block, 0);
  ASSERT_EQ(report.accounts.size(), 2);
  const std::array<uint8_t, 32> zero_word{};
  EXPECT_EQ(report.accounts[0].address, present.asArray());
  EXPECT_TRUE(report.accounts[0].found);
  EXPECT_EQ(report.accounts[0].nonce, zero_word);
  EXPECT_EQ(report.accounts[0].balance[31], 123);
  EXPECT_EQ(report.accounts[1].address, missing.asArray());
  EXPECT_FALSE(report.accounts[1].found);
}

TEST_F(ExternalEvmPortTest, DagGasEstimatePreservesIdentityAndCanonicalResult) {
  const auto sender = dev::KeyPair::create();
  config.genesis.state.initial_balances = {{sender.address(), 1000000000}};
  initialize();

  const auto transaction = std::make_shared<Transaction>(
      0, 1, 1, 100000, dev::bytes{}, sender.secret(), addr_t::random(), config.genesis.chain_id);
  ExternalEvmPort port(final_chain);
  rustaxa::HostDagGasBatch request{};
  request.effect_id = {.generation = 11, .sequence = 13};
  request.proposal_period = 0;
  request.transaction_hashes.push_back({.hash = transaction->getHash().asArray()});
  rustaxa::CanonicalBytes input_rlp{};
  const auto rlp = transaction->rlp();
  input_rlp.data.reserve(rlp.size());
  for (const auto byte : rlp) input_rlp.data.push_back(byte);
  request.transaction_rlps.push_back(std::move(input_rlp));

  const auto report = port.consensusEstimateDagTransactionGas(request);

  ASSERT_TRUE(report.succeeded) << std::string(report.error_code);
  EXPECT_EQ(report.effect_id.generation, 11);
  EXPECT_EQ(report.effect_id.sequence, 13);
  EXPECT_EQ(report.observed_block, 0);
  EXPECT_EQ(report.proposal_period, request.proposal_period);
  ASSERT_EQ(report.transaction_hashes.size(), 1);
  ASSERT_EQ(report.gas_used.size(), 1);
  ASSERT_EQ(report.result_rlps.size(), 1);
  EXPECT_EQ(report.transaction_hashes[0].hash, transaction->getHash().asArray());
  EXPECT_GT(report.gas_used[0], 0);
  EXPECT_FALSE(report.result_rlps[0].data.empty());
}

}  // namespace
}  // namespace taraxa::final_chain
