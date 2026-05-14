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
#endif

}  // namespace taraxa::core_tests
