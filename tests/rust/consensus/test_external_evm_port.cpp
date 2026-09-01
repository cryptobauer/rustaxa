#include <gtest/gtest.h>

#include <array>
#include <filesystem>
#include <memory>
#include <string_view>

#include "common/encoding_rlp.hpp"
#include "common/encoding_solidity.hpp"
#include "config/config.hpp"
#include "consensus/consensus_application.hpp"
#include "consensus/consensus_host_ports.hpp"
#include "final_chain/state_api.hpp"
#include "rustaxa-bridge/application_host_ffi.rs.h"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"

namespace taraxa::final_chain {
namespace {

/** Test-only decoder for the concrete StateAPI identity embedded in provenance and stage markers. */
struct TestConcreteStateIdentity {
  uint64_t policy_version = 0;
  h256 database_identity;
  h256 chain_identity;

  RLP_FIELDS_DEFINE_INPLACE(policy_version, database_identity, chain_identity)
};

/** Test-only decoder for the exact committed provenance returned by the concrete-EVM leaf. */
struct TestConcreteStateProvenance {
  TestConcreteStateIdentity identity;
  uint64_t generation = 0;
  h256 plan_hash;
  state_api::StateDescriptor committed_state;
  h256 transactions_hash;
  h256 rewards_hash;
  h256 projection_hash;
  h256 catalog_hash;

  RLP_FIELDS_DEFINE_INPLACE(identity, generation, plan_hash, committed_state, transactions_hash, rewards_hash,
                            projection_hash, catalog_hash)
};

/** Test-only exact stage marker used to hold StateAPI in its durable pending lifecycle. */
struct TestConcreteExecutionMarker {
  TestConcreteStateIdentity identity;
  uint64_t generation = 0;
  h256 plan_hash;
  EthBlockNumber period = 0;
  state_api::StateDescriptor prior_state;
  h256 transactions_hash;
  h256 rewards_hash;

  RLP_FIELDS_DEFINE_INPLACE(identity, generation, plan_hash, period, prior_state, transactions_hash, rewards_hash)
};

std::array<uint8_t, 32> concreteChainIdentity(const FullNodeConfig& config) {
  constexpr std::string_view kDomain = "rustaxa-final-chain-concrete-chain-identity-v1";
  bytes preimage(kDomain.begin(), kDomain.end());
  const auto genesis_hash = config.genesis.genesisHash();
  preimage.insert(preimage.end(), genesis_hash.begin(), genesis_hash.end());
  return dev::sha3(preimage).asArray();
}

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
  }

  std::filesystem::path test_dir;
  FullNodeConfig config;
  SharedConsensusApplication application;
};

TEST_F(ExternalEvmPortTest, MixedOrphanedConcreteStageIsDiscardedOnApplicationRestart) {
  const auto sender = dev::KeyPair::create();
  const auto transfer_receiver = addr_t::random();
  const auto arbitrary_evm_receiver = addr_t::random();
  constexpr uint64_t kGenesisBalance = 1'000'000'000;
  config.genesis.state.initial_balances[sender.address()] = kGenesisBalance;
  ASSERT_NO_THROW(initialize());
  ASSERT_NE(application, nullptr);

  auto port = std::make_unique<ExternalEvmPort>(application);
  rustaxa::HostFinalChainPreflightRequest preflight_request{};
  preflight_request.request_id[0] = 1;
  preflight_request.concrete_chain_identity = concreteChainIdentity(config);
  const auto before = port->consensusLoadFinalChainCommittedState(preflight_request);
  ASSERT_TRUE(before.succeeded) << std::string(before.error_code);
  TestConcreteStateProvenance before_provenance;
  const auto before_provenance_rlp =
      dev::bytes(before.concrete_provenance_rlp.begin(), before.concrete_provenance_rlp.end());
  util::rlp(dev::RLP(before_provenance_rlp), before_provenance);

  const auto marker_rlp = util::rlp_enc(TestConcreteExecutionMarker{
      .identity = before_provenance.identity,
      .generation = before_provenance.generation + 1,
      .plan_hash = h256::random(),
      .period = before.committed_period + 1,
      .prior_state = {before.committed_period, h256(before.committed_state_root.data(), h256::ConstructFromPointer)},
      .transactions_hash = h256::random(),
      .rewards_hash = h256::random(),
  });
  rustaxa::HostFinalChainExecutionRequest execution_request{};
  execution_request.concrete_marker_rlp.reserve(marker_rlp.size());
  for (const auto byte : marker_rlp) execution_request.concrete_marker_rlp.push_back(byte);
  execution_request.block_author = sender.address().asArray();
  execution_request.block_gas_limit = config.genesis.pbft.gas_limit;
  execution_request.timestamp = 1;

  rustaxa::HostFinalChainTransactionInput transfer{};
  transfer.sender = sender.address().asArray();
  transfer.receiver_found = true;
  transfer.receiver = transfer_receiver.asArray();
  transfer.value.push_back(100);
  transfer.gas_limit = 100'000;
  execution_request.transactions.push_back(std::move(transfer));

  rustaxa::HostFinalChainTransactionInput dpos_read{};
  dpos_read.sender = sender.address().asArray();
  dpos_read.receiver_found = true;
  dpos_read.receiver = addr_t("0x00000000000000000000000000000000000000FE").asArray();
  dpos_read.nonce.push_back(1);
  dpos_read.gas_limit = 1'000'000;
  const auto dpos_input = util::EncodingSolidity::packFunctionCall("getTotalEligibleVotesCount()");
  for (const auto byte : dpos_input) dpos_read.data.push_back(byte);
  execution_request.transactions.push_back(std::move(dpos_read));

  rustaxa::HostFinalChainTransactionInput arbitrary_evm{};
  arbitrary_evm.sender = sender.address().asArray();
  arbitrary_evm.receiver_found = true;
  arbitrary_evm.receiver = arbitrary_evm_receiver.asArray();
  arbitrary_evm.nonce.push_back(2);
  arbitrary_evm.gas_limit = 100'000;
  arbitrary_evm.data.push_back(1);
  execution_request.transactions.push_back(std::move(arbitrary_evm));

  const auto staged = port->consensusExecuteFinalChainTransactions(execution_request);
  ASSERT_EQ(staged.results.size(), 3);
  EXPECT_EQ(staged.results[0].status, 1);
  EXPECT_EQ(staged.results[1].status, 1);
  EXPECT_EQ(staged.results[2].status, 1);
  preflight_request.request_id[0] = 2;
  const auto pending = port->consensusLoadFinalChainCommittedState(preflight_request);
  ASSERT_TRUE(pending.succeeded) << std::string(pending.error_code);
  EXPECT_EQ(dev::bytes(pending.pending_concrete_marker_rlp.begin(), pending.pending_concrete_marker_rlp.end()),
            marker_rlp);

  // Simulate a process crash after concrete execution but before Rust writes
  // its pending-publication marker. Startup must discard this orphaned stage.
  port.reset();
  application.reset();
  ASSERT_NO_THROW(application = createConsensusApplication(config));
  ASSERT_NE(application, nullptr);

  ExternalEvmPort reopened_port(application);
  preflight_request.request_id[0] = 3;
  const auto reopened = reopened_port.consensusLoadFinalChainCommittedState(preflight_request);
  ASSERT_TRUE(reopened.succeeded) << std::string(reopened.error_code);
  EXPECT_TRUE(reopened.pending_concrete_marker_rlp.empty());
  EXPECT_EQ(reopened.committed_period, before.committed_period);
  EXPECT_EQ(reopened.committed_state_root, before.committed_state_root);
  TestConcreteStateProvenance reopened_provenance;
  const auto reopened_provenance_rlp =
      dev::bytes(reopened.concrete_provenance_rlp.begin(), reopened.concrete_provenance_rlp.end());
  util::rlp(dev::RLP(reopened_provenance_rlp), reopened_provenance);
  EXPECT_EQ(reopened_provenance.identity.database_identity, before_provenance.identity.database_identity);
  EXPECT_EQ(reopened_provenance.identity.chain_identity, before_provenance.identity.chain_identity);
  EXPECT_EQ(reopened_provenance.catalog_hash, before_provenance.catalog_hash);

  const auto sender_account = application->getAccount(sender.address());
  ASSERT_TRUE(sender_account);
  EXPECT_EQ(sender_account->nonce, 0);
  EXPECT_EQ(sender_account->balance, kGenesisBalance);
  EXPECT_FALSE(application->getAccount(transfer_receiver));
  EXPECT_FALSE(application->getAccount(arbitrary_evm_receiver));
}

TEST_F(ExternalEvmPortTest, PruneFailsClosedWhileConcreteExecutionIsStaged) {
  initialize();

  ExternalEvmPort port(application);
  rustaxa::HostFinalChainPreflightRequest preflight_request{};
  preflight_request.request_id[0] = 1;
  preflight_request.concrete_chain_identity = concreteChainIdentity(config);
  const auto preflight = port.consensusLoadFinalChainCommittedState(preflight_request);
  ASSERT_TRUE(preflight.succeeded) << std::string(preflight.error_code);

  TestConcreteStateProvenance provenance;
  const auto provenance_rlp =
      dev::bytes(preflight.concrete_provenance_rlp.begin(), preflight.concrete_provenance_rlp.end());
  util::rlp(dev::RLP(provenance_rlp), provenance);
  const auto marker_rlp = util::rlp_enc(TestConcreteExecutionMarker{
      .identity = provenance.identity,
      .generation = provenance.generation + 1,
      .plan_hash = h256::random(),
      .period = preflight.committed_period + 1,
      .prior_state = {preflight.committed_period,
                      h256(preflight.committed_state_root.data(), h256::ConstructFromPointer)},
      .transactions_hash = h256::random(),
      .rewards_hash = h256::random(),
  });

  rustaxa::HostFinalChainExecutionRequest execution_request{};
  execution_request.concrete_marker_rlp.reserve(marker_rlp.size());
  for (const auto byte : marker_rlp) execution_request.concrete_marker_rlp.push_back(byte);
  execution_request.block_gas_limit = config.genesis.pbft.gas_limit;
  execution_request.timestamp = 1;
  const auto execution = port.consensusExecuteFinalChainTransactions(execution_request);
  EXPECT_TRUE(execution.results.empty());

  try {
    application->pruneFinalChain(0);
    FAIL() << "pruning must reject a durable staged concrete execution";
  } catch (const std::exception& error) {
    EXPECT_STREQ(error.what(), "FINAL_CHAIN_CONCRETE_STATE_STAGED");
  }

  preflight_request.request_id[0] = 2;
  const auto staged = port.consensusLoadFinalChainCommittedState(preflight_request);
  ASSERT_TRUE(staged.succeeded) << std::string(staged.error_code);
  EXPECT_EQ(dev::bytes(staged.pending_concrete_marker_rlp.begin(), staged.pending_concrete_marker_rlp.end()),
            marker_rlp);

  rustaxa::CanonicalBytes exact_marker{};
  exact_marker.data.reserve(marker_rlp.size());
  for (const auto byte : marker_rlp) exact_marker.data.push_back(byte);
  const auto discarded = port.consensusDiscardFinalChainState(exact_marker);
  ASSERT_TRUE(discarded.succeeded) << std::string(discarded.error_code);
  EXPECT_NO_THROW(application->pruneFinalChain(0));
}

TEST_F(ExternalEvmPortTest, ExecutionReportPreservesExactStateApiCodeRetval) {
  const auto sender = dev::KeyPair::create();
  config.genesis.state.initial_balances[sender.address()] = 1'000'000'000;
  initialize();

  ExternalEvmPort port(application);
  rustaxa::HostFinalChainPreflightRequest preflight_request{};
  preflight_request.request_id[0] = 1;
  preflight_request.concrete_chain_identity = concreteChainIdentity(config);
  const auto preflight = port.consensusLoadFinalChainCommittedState(preflight_request);
  ASSERT_TRUE(preflight.succeeded) << std::string(preflight.error_code);

  TestConcreteStateProvenance provenance;
  const auto provenance_rlp =
      dev::bytes(preflight.concrete_provenance_rlp.begin(), preflight.concrete_provenance_rlp.end());
  util::rlp(dev::RLP(provenance_rlp), provenance);
  const auto marker_rlp = util::rlp_enc(TestConcreteExecutionMarker{
      .identity = provenance.identity,
      .generation = provenance.generation + 1,
      .plan_hash = h256::random(),
      .period = preflight.committed_period + 1,
      .prior_state = {preflight.committed_period,
                      h256(preflight.committed_state_root.data(), h256::ConstructFromPointer)},
      .transactions_hash = h256::random(),
      .rewards_hash = h256::random(),
  });

  rustaxa::HostFinalChainExecutionRequest request{};
  request.concrete_marker_rlp.reserve(marker_rlp.size());
  for (const auto byte : marker_rlp) request.concrete_marker_rlp.push_back(byte);
  request.block_author = sender.address().asArray();
  request.block_gas_limit = config.genesis.pbft.gas_limit;
  request.timestamp = 1;
  rustaxa::HostFinalChainTransactionInput transaction{};
  transaction.sender = sender.address().asArray();
  transaction.receiver_found = true;
  transaction.receiver = addr_t("0x00000000000000000000000000000000000000FE").asArray();
  transaction.gas_price.push_back(1);
  transaction.gas_limit = 1'000'000;
  const auto input = util::EncodingSolidity::packFunctionCall("getTotalEligibleVotesCount()");
  transaction.data.reserve(input.size());
  for (const auto byte : input) transaction.data.push_back(byte);
  request.transactions.push_back(std::move(transaction));

  const auto execution = port.consensusExecuteFinalChainTransactions(request);
  ASSERT_EQ(execution.results.size(), 1);
  ASSERT_EQ(execution.results[0].status, 1);
  const auto expected_output = util::EncodingSolidity::pack(uint64_t{0});
  EXPECT_EQ(dev::bytes(execution.results[0].output.begin(), execution.results[0].output.end()), expected_output);

  rustaxa::CanonicalBytes exact_marker{};
  exact_marker.data.reserve(marker_rlp.size());
  for (const auto byte : marker_rlp) exact_marker.data.push_back(byte);
  const auto discarded = port.consensusDiscardFinalChainState(exact_marker);
  ASSERT_TRUE(discarded.succeeded) << std::string(discarded.error_code);
}

TEST_F(ExternalEvmPortTest, ApplicationAccountQueriesPreservePresentAndMissingRows) {
  const auto present = addr_t::random();
  const auto missing = addr_t::random();
  config.genesis.state.initial_balances = {{present, 123}};
  initialize();

  const auto present_account = application->getAccount(present);
  ASSERT_TRUE(present_account);
  EXPECT_EQ(present_account->nonce, 0);
  EXPECT_EQ(present_account->balance, 123);
  EXPECT_FALSE(application->getAccount(missing));
}

TEST_F(ExternalEvmPortTest, DagGasEstimatePreservesIdentityAndCanonicalResult) {
  const auto sender = dev::KeyPair::create();
  config.genesis.state.initial_balances = {{sender.address(), 1000000000}};
  initialize();

  const auto transaction = std::make_shared<Transaction>(0, 1, 1, 100000, dev::bytes{}, sender.secret(),
                                                         addr_t::random(), config.genesis.chain_id);
  ExternalEvmPort port(application);
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
