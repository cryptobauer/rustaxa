#include <gtest/gtest.h>

#include <type_traits>

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
#endif

}  // namespace taraxa::core_tests
