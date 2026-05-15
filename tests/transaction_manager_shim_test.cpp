#include <gtest/gtest.h>

#include <limits>
#include <mutex>
#include <shared_mutex>
#include <type_traits>
#include <unordered_set>

#include "common/init.hpp"
#include "final_chain/final_chain.hpp"
#include "test_util/samples.hpp"
#include "transaction/transaction.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa::core_tests {

TEST(TransactionManagerShimTest, rustModeTransactionManagerIsDistinctFromLegacyType) {
#ifdef RUSTAXA_ENABLE
  static_assert(!std::is_same_v<TransactionManagerOld, TransactionManager>);
  SUCCEED();
#else
  GTEST_SKIP() << "TransactionManager shim is disabled";
#endif
}

#ifdef RUSTAXA_ENABLE
struct TransactionManagerShimFixture : NodesTest {};

TEST_F(TransactionManagerShimFixture, rustPlannerPreservesPackTrxsSelectionAndEstimations) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 4,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  for (const auto& trx : transactions) {
    ASSERT_TRUE(trx_mgr.insertTransaction(trx).first);
  }

  auto [packed, estimations] = trx_mgr.packTrxs(1, 250000);

  ASSERT_EQ(packed.size(), 2);
  ASSERT_EQ(estimations.size(), packed.size());
  EXPECT_EQ(estimations[0], packed[0]->getGas());
  EXPECT_EQ(estimations[1], packed[1]->getGas());
}

TEST_F(TransactionManagerShimFixture, rustStoragePersistsDagTransactionsBeforeLiveCacheMutation) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 4,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  for (const auto& trx : transactions) {
    ASSERT_TRUE(trx_mgr.insertTransaction(trx).first);
  }

  const auto initial_count = trx_mgr.getTransactionCount();
  ASSERT_EQ(trx_mgr.getTransactionPoolSize(), transactions.size());

  trx_mgr.saveTransactionsFromDagBlock({transactions[0], transactions[1], transactions[1]});

  EXPECT_EQ(trx_mgr.getTransactionCount(), initial_count + 2);
  EXPECT_EQ(db->getStatusField(StatusDbField::TrxCount), initial_count + 2);
  EXPECT_EQ(trx_mgr.getNonfinalizedTrxSize(), 2);
  EXPECT_EQ(trx_mgr.getTransactionPoolSize(), transactions.size() - 2);

  const auto persisted_0 = db->getTransaction(transactions[0]->getHash());
  const auto persisted_1 = db->getTransaction(transactions[1]->getHash());
  ASSERT_TRUE(persisted_0);
  ASSERT_TRUE(persisted_1);
  EXPECT_EQ(persisted_0->rlp(), transactions[0]->rlp());
  EXPECT_EQ(persisted_1->rlp(), transactions[1]->rlp());

  trx_mgr.saveTransactionsFromDagBlock({transactions[0], transactions[1]});

  EXPECT_EQ(trx_mgr.getTransactionCount(), initial_count + 2);
  EXPECT_EQ(db->getStatusField(StatusDbField::TrxCount), initial_count + 2);
  EXPECT_EQ(trx_mgr.getNonfinalizedTrxSize(), 2);
  EXPECT_EQ(trx_mgr.getTransactionPoolSize(), transactions.size() - 2);
}

TEST_F(TransactionManagerShimFixture, rustGetTransactionPrefersLiveCachesThenStorage) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 1,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  ASSERT_TRUE(trx_mgr.insertTransaction(transactions[0]).first);

  const auto from_pool = trx_mgr.getTransaction(transactions[0]->getHash());
  ASSERT_TRUE(from_pool);
  EXPECT_EQ(from_pool.get(), transactions[0].get());

  trx_mgr.saveTransactionsFromDagBlock({transactions[0]});

  const auto from_cache = trx_mgr.getTransaction(transactions[0]->getHash());
  ASSERT_TRUE(from_cache);
  const auto cached_view = trx_mgr.getNonfinalizedTrx({transactions[0]->getHash()});
  ASSERT_EQ(cached_view.size(), 1);
  EXPECT_EQ(from_cache.get(), cached_view.front().get());

  TransactionManager restart_trx_mgr(cfg, db, final_chain, addr_t());
  const auto from_storage = restart_trx_mgr.getTransaction(transactions[0]->getHash());
  ASSERT_TRUE(from_storage);
  EXPECT_NE(from_storage.get(), transactions[0].get());
  EXPECT_EQ(from_storage->getHash(), transactions[0]->getHash());
  EXPECT_EQ(from_storage->rlp(), transactions[0]->rlp());
}

TEST_F(TransactionManagerShimFixture, rustGetTransactionsCombinesLiveAndRustStorageLookups) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 2,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  {
    TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
    ASSERT_TRUE(trx_mgr.insertTransaction(transactions[0]).first);
    trx_mgr.saveTransactionsFromDagBlock({transactions[0]});
  }

  TransactionManager restart_trx_mgr(cfg, db, final_chain, addr_t());
  ASSERT_TRUE(restart_trx_mgr.insertTransaction(transactions[1]).first);

  const auto materialized = restart_trx_mgr.getTransactions(
      {transactions[0]->getHash(), trx_hash_t::random(), transactions[1]->getHash()}, 0);

  ASSERT_EQ(materialized.size(), 2);
  EXPECT_EQ(materialized[0]->getHash(), transactions[0]->getHash());
  EXPECT_EQ(materialized[1].get(), transactions[1].get());
}

TEST_F(TransactionManagerShimFixture, rustDagTransactionPersistenceFailureDoesNotMutateLiveState) {
  auto db = std::make_shared<DbStorage>(data_dir);
  db->saveStatusField(StatusDbField::TrxCount, std::numeric_limits<uint64_t>::max());
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 1,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));
  ASSERT_TRUE(trx_mgr.insertTransaction(transactions[0]).first);

  EXPECT_THROW(trx_mgr.saveTransactionsFromDagBlock({transactions[0]}), DbException);

  EXPECT_EQ(trx_mgr.getTransactionCount(), std::numeric_limits<uint64_t>::max());
  EXPECT_EQ(db->getStatusField(StatusDbField::TrxCount), std::numeric_limits<uint64_t>::max());
  EXPECT_EQ(trx_mgr.getNonfinalizedTrxSize(), 0);
  EXPECT_EQ(trx_mgr.getTransactionPoolSize(), transactions.size());
  EXPECT_FALSE(db->getTransaction(transactions[0]->getHash()));
}

TEST_F(TransactionManagerShimFixture, rustRecoverNonfinalizedTransactionsSkipsFinalizedPayloads) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 2,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  {
    TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
    for (const auto& trx : transactions) {
      ASSERT_TRUE(trx_mgr.insertTransaction(trx).first);
    }
    trx_mgr.saveTransactionsFromDagBlock(transactions);

    auto batch = db->createWriteBatch();
    db->addTransactionLocationToBatch(batch, transactions[0]->getHash(), 0, 0);
    db->commitWriteBatch(batch);
  }

  TransactionManager restart_trx_mgr(cfg, db, final_chain, addr_t());
  EXPECT_EQ(restart_trx_mgr.getNonfinalizedTrxSize(), 0);
  restart_trx_mgr.recoverNonfinalizedTransactions();
  EXPECT_EQ(restart_trx_mgr.getNonfinalizedTrxSize(), 1);

  const auto recovered_only =
      restart_trx_mgr.getNonfinalizedTrx({transactions[0]->getHash(), transactions[1]->getHash()});
  ASSERT_EQ(recovered_only.size(), 1);
  EXPECT_EQ(recovered_only.front()->getHash(), transactions[1]->getHash());
  EXPECT_FALSE(db->getTransaction(transactions[0]->getHash()));
  ASSERT_TRUE(db->getTransaction(transactions[1]->getHash()));
}

TEST_F(TransactionManagerShimFixture, expiredNonFinalizedSidecarCleanupDoesNotTouchStorage) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 1,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));
  ASSERT_TRUE(trx_mgr.insertTransaction(transactions[0]).first);
  trx_mgr.saveTransactionsFromDagBlock({transactions[0]});
  ASSERT_EQ(trx_mgr.getNonfinalizedTrxSize(), 1);
  ASSERT_TRUE(db->getTransaction(transactions[0]->getHash()));

  std::unordered_set<trx_hash_t> expired_hashes{transactions[0]->getHash()};
  std::unique_lock lock(trx_mgr.getTransactionsMutex());
  trx_mgr.forgetExpiredNonFinalizedTransactionSidecars(std::move(expired_hashes));
  lock.unlock();

  EXPECT_EQ(trx_mgr.getNonfinalizedTrxSize(), 0);
  EXPECT_TRUE(db->getTransaction(transactions[0]->getHash()));
}

TEST_F(TransactionManagerShimFixture, rustFinalizedTransactionsInitializationRetainsLiveReferences) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 2,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));
  std::vector<vote_hash_t> reward_votes;
  auto pbft_block = std::make_shared<PbftBlock>(kNullBlockHash, kNullBlockHash, kNullBlockHash, kNullBlockHash, 2,
                                                addr_t::random(), dev::KeyPair::create().secret(), reward_votes);
  PeriodData period_data(std::move(pbft_block), {});
  period_data.transactions = {transactions[0]};

  trx_mgr.initializeRecentlyFinalizedTransactions(period_data);

  const auto transactions_out = trx_mgr.getTransactions({transactions[0]->getHash()}, 0);
  ASSERT_EQ(transactions_out.size(), 1);
  EXPECT_EQ(transactions_out[0]->getHash(), transactions[0]->getHash());
  EXPECT_EQ(trx_mgr.getTransactionCount(), 0);
}

TEST_F(TransactionManagerShimFixture, rustFinalizedTransactionsUpdateAppliesCleanupAndKnownMarking) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 2,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  std::vector<vote_hash_t> reward_votes;
  const auto cleanup_period =
      static_cast<uint64_t>(kRecentlyFinalizedTransactionsFactor * final_chain->delegationDelay()) + 1;
  auto old_period_block = std::make_shared<PbftBlock>(kNullBlockHash, kNullBlockHash, kNullBlockHash, kNullBlockHash, 1,
                                                      addr_t::random(), dev::KeyPair::create().secret(), reward_votes);
  PeriodData old_period_data(std::move(old_period_block), {});
  old_period_data.transactions = {transactions[0]};
  trx_mgr.initializeRecentlyFinalizedTransactions(old_period_data);
  ASSERT_FALSE(trx_mgr.getTransactions({transactions[0]->getHash()}, 0).empty());

  auto update_block =
      std::make_shared<PbftBlock>(kNullBlockHash, kNullBlockHash, kNullBlockHash, kNullBlockHash, cleanup_period,
                                  addr_t::random(), dev::KeyPair::create().secret(), reward_votes);
  PeriodData update_data(std::move(update_block), {});
  update_data.transactions = {transactions[1]};

  const auto expected_count = trx_mgr.getTransactionCount();
  {
    std::unique_lock lock(trx_mgr.getTransactionsMutex());
    trx_mgr.updateFinalizedTransactionsStatus(update_data);
  }

  EXPECT_EQ(trx_mgr.getTransactionCount(), expected_count + 1);
  EXPECT_EQ(db->getStatusField(StatusDbField::TrxCount), expected_count + 1);
  EXPECT_TRUE(trx_mgr.isTransactionKnown(transactions[1]->getHash()));
  EXPECT_TRUE(trx_mgr.getTransactions({transactions[1]->getHash()}, 0).size() > 0);
  EXPECT_TRUE(trx_mgr.getTransactions({transactions[0]->getHash()}, 0).empty());
}

TEST_F(TransactionManagerShimFixture, rustFinalizedTransactionsStorageFailureDoesNotMutateLiveState) {
  auto db = std::make_shared<DbStorage>(data_dir);
  db->saveStatusField(StatusDbField::TrxCount, std::numeric_limits<uint64_t>::max());
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 1,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  std::vector<vote_hash_t> reward_votes;
  auto update_block = std::make_shared<PbftBlock>(kNullBlockHash, kNullBlockHash, kNullBlockHash, kNullBlockHash, 1,
                                                  addr_t::random(), dev::KeyPair::create().secret(), reward_votes);
  PeriodData update_data(std::move(update_block), {});
  update_data.transactions = {transactions[0]};

  {
    std::unique_lock lock(trx_mgr.getTransactionsMutex());
    EXPECT_THROW(trx_mgr.updateFinalizedTransactionsStatus(update_data), DbException);
  }

  EXPECT_EQ(trx_mgr.getTransactionCount(), std::numeric_limits<uint64_t>::max());
  EXPECT_EQ(db->getStatusField(StatusDbField::TrxCount), std::numeric_limits<uint64_t>::max());
  EXPECT_FALSE(trx_mgr.isTransactionKnown(transactions[0]->getHash()));
  EXPECT_TRUE(trx_mgr.getTransactions({transactions[0]->getHash()}, 0).empty());
}

TEST_F(TransactionManagerShimFixture, rustNonFinalizedReadHelpersUseLiveState) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 3,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  for (auto const& trx : transactions) {
    ASSERT_TRUE(trx_mgr.insertTransaction(trx).first);
  }

  trx_mgr.saveTransactionsFromDagBlock({transactions[0], transactions[1]});

  const auto non_finalized = trx_mgr.getNonfinalizedTrx(
      {transactions[0]->getHash(), transactions[1]->getHash(), transactions[2]->getHash()});
  ASSERT_EQ(non_finalized.size(), 2);
  EXPECT_EQ(non_finalized[0]->getHash(), transactions[0]->getHash());
  EXPECT_EQ(non_finalized[1]->getHash(), transactions[1]->getHash());

  EXPECT_TRUE(trx_mgr.getNonFinalizedTransaction(transactions[0]->getHash()));
  EXPECT_TRUE(trx_mgr.getNonFinalizedTransaction(transactions[1]->getHash()));
  EXPECT_FALSE(trx_mgr.getNonFinalizedTransaction(transactions[2]->getHash()));
}

TEST_F(TransactionManagerShimFixture, rustExcludeFinalizedTransactionsUsesRecentCacheAndStorageState) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 3,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  for (auto const& trx : transactions) {
    ASSERT_TRUE(trx_mgr.insertTransaction(trx).first);
  }

  std::vector<vote_hash_t> reward_votes;
  auto block = std::make_shared<PbftBlock>(kNullBlockHash, kNullBlockHash, kNullBlockHash, kNullBlockHash, 1,
                                          addr_t::random(), dev::KeyPair::create().secret(), reward_votes);
  PeriodData period_data(std::move(block), {});
  period_data.transactions = {transactions[0]};
  trx_mgr.initializeRecentlyFinalizedTransactions(period_data);

  {
    auto batch = db->createWriteBatch();
    db->addTransactionLocationToBatch(batch, transactions[1]->getHash(), 1, 0);
    db->commitWriteBatch(batch);
  }

  const auto excluded = trx_mgr.excludeFinalizedTransactions(
      {transactions[0]->getHash(), transactions[1]->getHash(), transactions[2]->getHash()});
  ASSERT_EQ(excluded.size(), 1);
  EXPECT_TRUE(excluded.contains(transactions[2]->getHash()));
  EXPECT_FALSE(excluded.contains(transactions[0]->getHash()));
  EXPECT_FALSE(excluded.contains(transactions[1]->getHash()));
}

TEST_F(TransactionManagerShimFixture, rustVerifyTransactionsNotFinalizedUsesRecentCacheAndStorageState) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(0, 2,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));
  const auto pending_transactions =
      samples::createSignedTrxSamples(3, 3,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  for (auto const& trx : transactions) {
    ASSERT_TRUE(trx_mgr.insertTransaction(trx).first);
  }
  for (auto const& trx : pending_transactions) {
    ASSERT_TRUE(trx_mgr.insertTransaction(trx).first);
  }

  std::vector<vote_hash_t> reward_votes;
  auto block = std::make_shared<PbftBlock>(kNullBlockHash, kNullBlockHash, kNullBlockHash, kNullBlockHash, 1,
                                          addr_t::random(), dev::KeyPair::create().secret(), reward_votes);
  PeriodData period_data(std::move(block), {});
  period_data.transactions = {transactions[1]};
  trx_mgr.initializeRecentlyFinalizedTransactions(period_data);
  EXPECT_FALSE(trx_mgr.verifyTransactionsNotFinalized({transactions[1]}));

  {
    auto batch = db->createWriteBatch();
    db->addTransactionLocationToBatch(batch, transactions[0]->getHash(), 1, 0);
    db->commitWriteBatch(batch);
  }

  EXPECT_FALSE(trx_mgr.verifyTransactionsNotFinalized({transactions[0]}));
  EXPECT_TRUE(trx_mgr.verifyTransactionsNotFinalized(pending_transactions));
}

TEST_F(TransactionManagerShimFixture, rustPoolReadHelpersRemainCxxOwned) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 2,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  for (auto const& trx : transactions) {
    ASSERT_TRUE(trx_mgr.insertTransaction(trx).first);
  }

  const auto before = trx_mgr.getAllPoolTrxs();
  size_t before_count = 0;
  for (const auto& chunk : before) {
    before_count += chunk.size();
  }
  ASSERT_EQ(before_count, 2);

  const auto pool_lookup = trx_mgr.getPoolTransactions(
      {transactions[0]->getHash(), trx_hash_t::random(), transactions[1]->getHash()});
  ASSERT_EQ(pool_lookup.first.size(), 2);
  ASSERT_EQ(pool_lookup.second.size(), 1);

  trx_mgr.saveTransactionsFromDagBlock({transactions[0]});
  const auto after = trx_mgr.getAllPoolTrxs();
  size_t after_count = 0;
  for (const auto& chunk : after) {
    after_count += chunk.size();
  }
  EXPECT_EQ(after_count, 1);
}

TEST_F(TransactionManagerShimFixture, rustVerifyTransactionRejectsChainIdMismatch) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  auto bad_chain_id_transaction =
      std::make_shared<Transaction>(1, 100, 1000000000, 100000, dev::bytes(),
                                   dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                               dev::Secret::ConstructFromStringType::FromHex),
                                   addr_t::random(), cfg.genesis.chain_id + 1);

  const auto result = trx_mgr.verifyTransaction(bad_chain_id_transaction);
  EXPECT_FALSE(result.first);
  EXPECT_EQ(result.second,
            "chain_id mismatch " + std::to_string(cfg.genesis.chain_id + 1) + " " +
                std::to_string(cfg.genesis.chain_id));
}

TEST_F(TransactionManagerShimFixture, rustVerifyTransactionRejectsInvalidGasLimit) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  auto max_gas_limit = cfg.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;

  const auto tx = std::make_shared<Transaction>(
      1, 100, 1000000000, max_gas_limit + 1, dev::bytes(),
      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                  dev::Secret::ConstructFromStringType::FromHex),
      addr_t::random());

  const auto result = trx_mgr.verifyTransaction(tx);
  EXPECT_FALSE(result.first);
  EXPECT_EQ(result.second, "invalid gas");
}

TEST_F(TransactionManagerShimFixture, rustVerifyTransactionRejectsInvalidSignature) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();

  const auto valid_transactions = samples::createSignedTrxSamples(
      1, 1, dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                           dev::Secret::ConstructFromStringType::FromHex));
  auto valid_trx = valid_transactions[0];

  dev::RLPStream with_invalid_signature(9);
  size_t fields_processed = 0;
  for (const auto el : dev::RLP(valid_trx->rlp())) {
    auto el_modified = el.toBytes();
    ++fields_processed;
    if (fields_processed > 7) {
      for (auto& b : el_modified) {
        b = 0;
      }
    }
    with_invalid_signature << el_modified;
  }

  const auto invalid_signature_trx = std::make_shared<Transaction>(with_invalid_signature.invalidate());
  cfg.genesis.chain_id = invalid_signature_trx->getChainID();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto result = trx_mgr.verifyTransaction(invalid_signature_trx);
  EXPECT_FALSE(result.first);
  EXPECT_EQ(result.second, "invalid signature");
}

TEST_F(TransactionManagerShimFixture, rustInsertTransactionRejectsKnownTransaction) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto transactions =
      samples::createSignedTrxSamples(1, 1,
                                      dev::Secret("3800b2875669d9b2053c1aff9224ecfdc411423aac5b5a73d7a45ced1c3b9dcd",
                                                  dev::Secret::ConstructFromStringType::FromHex));

  ASSERT_TRUE(trx_mgr.insertTransaction(transactions[0]).first);

  const auto result = trx_mgr.insertTransaction(transactions[0]);
  EXPECT_FALSE(result.first);
  EXPECT_EQ(result.second, "Transaction already in transactions pool");
}

TEST_F(TransactionManagerShimFixture, rustInsertTransactionRejectsAlreadyFinalizedTransaction) {
  auto db = std::make_shared<DbStorage>(data_dir);
  auto cfg = node_cfgs.front();
  auto final_chain = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t());
  TransactionManager trx_mgr(cfg, db, final_chain, addr_t());
  const auto unknown_sender = dev::Secret::random();
  const auto finalized_tx =
      std::make_shared<Transaction>(0, 100, 1000000000, 100000, dev::bytes(),
                                   unknown_sender,
                                   addr_t::random());
  constexpr uint64_t kFinalizedPeriod = 11;

  {
    auto batch = db->createWriteBatch();
    db->addTransactionLocationToBatch(batch, finalized_tx->getHash(), kFinalizedPeriod, 0);
    db->commitWriteBatch(batch);
  }

  const auto result = trx_mgr.insertTransaction(finalized_tx);
  EXPECT_FALSE(result.first);
  EXPECT_EQ(result.second, "Transaction already finalized in period" + std::to_string(kFinalizedPeriod));
}
#endif

}  // namespace taraxa::core_tests
