#include <gtest/gtest.h>

#include <array>
#include <type_traits>

#include "transaction/transaction_manager.hpp"
#include "transaction/transaction_queue.hpp"

namespace taraxa::core_tests {
namespace {

trx_hash_t testHash(uint8_t value) {
  std::array<uint8_t, 32> bytes{};
  bytes.fill(value);
  return trx_hash_t(bytes.data(), trx_hash_t::ConstructFromPointer);
}

}  // namespace

TEST(TransactionQueueShimTest, rustModeTransactionQueueDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_TRANSACTION_QUEUE
  static_assert(!std::is_base_of_v<TransactionQueueOld, TransactionQueue>);
  SUCCEED();
#else
  GTEST_SKIP() << "TransactionQueue shim is disabled";
#endif
}

TEST(TransactionQueueShimTest, rustModeTransactionQueueRetainsDemotedNonceReplacementAsNonProposableLiveState) {
#ifdef RUSTAXA_ENABLE_TRANSACTION_QUEUE
  TransactionQueue priority_queue(nullptr);
  const auto sender_secret = dev::KeyPair::create().secret();

  auto low_fee_tx = std::make_shared<Transaction>(0, 1, 1000, 10000, dev::bytes(), sender_secret, addr_t::random());
  auto replacement_tx = std::make_shared<Transaction>(0, 1, 2000, 10000, dev::bytes(), sender_secret, addr_t::random());

  const auto low_fee_tx_hash = low_fee_tx->getHash();
  const auto replacement_tx_hash = replacement_tx->getHash();

  EXPECT_EQ(priority_queue.insert(std::move(low_fee_tx), true, 1), TransactionStatus::Inserted);
  EXPECT_EQ(priority_queue.insert(std::move(replacement_tx), true, 1), TransactionStatus::Inserted);

  const auto ordered = priority_queue.getOrderedTransactions(10);
  ASSERT_EQ(ordered.size(), 1);
  EXPECT_EQ(ordered[0]->getHash(), replacement_tx_hash);

  const auto demoted = priority_queue.get(low_fee_tx_hash);
  ASSERT_NE(demoted, nullptr);
  EXPECT_EQ(demoted->getHash(), low_fee_tx_hash);

  const auto all = priority_queue.getAllTransactions();
  ASSERT_EQ(all.size(), 1);
  ASSERT_EQ(all[0].size(), 1);
  EXPECT_EQ(all[0][0]->getHash(), replacement_tx_hash);

  EXPECT_TRUE(priority_queue.isTransactionKnown(low_fee_tx_hash));
  EXPECT_TRUE(priority_queue.isTransactionKnown(replacement_tx_hash));
#else
  GTEST_SKIP() << "TransactionQueue shim is disabled";
#endif
}

TEST(TransactionQueueShimTest, rustModeTransactionQueueKnownCacheUsesRustOwnedFifoExpiry) {
#ifdef RUSTAXA_ENABLE_TRANSACTION_QUEUE
  TransactionQueue priority_queue(nullptr, 5);

  for (uint8_t hash = 1; hash <= 10; ++hash) {
    priority_queue.markTransactionKnown(testHash(hash));
  }
  EXPECT_TRUE(priority_queue.isTransactionKnown(testHash(1)));

  priority_queue.markTransactionKnown(testHash(11));
  EXPECT_FALSE(priority_queue.isTransactionKnown(testHash(1)));
  EXPECT_TRUE(priority_queue.isTransactionKnown(testHash(2)));
  EXPECT_TRUE(priority_queue.isTransactionKnown(testHash(11)));
#else
  GTEST_SKIP() << "TransactionQueue shim is disabled";
#endif
}

TEST(TransactionQueueShimTest, rustModeTransactionQueueTracksOverflowDropsInRust) {
#ifdef RUSTAXA_ENABLE_TRANSACTION_QUEUE
  TransactionQueue priority_queue(nullptr, 5);
  EXPECT_FALSE(priority_queue.transactionsDropped());

  for (uint8_t nonce = 0; nonce < 6; ++nonce) {
    auto trx = std::make_shared<Transaction>(nonce, 1, 1000, 10000, dev::bytes(), dev::KeyPair::create().secret(),
                                             addr_t::random());
    priority_queue.insert(std::move(trx), true, 1);
  }

  EXPECT_TRUE(priority_queue.transactionsDropped());
#else
  GTEST_SKIP() << "TransactionQueue shim is disabled";
#endif
}

TEST(TransactionQueueShimTest, rustModeTransactionQueueExpiresNonProposableWithFinalizedBlockNumber) {
#ifdef RUSTAXA_ENABLE_TRANSACTION_QUEUE
  TransactionQueue priority_queue(nullptr);
  auto old_tx =
      std::make_shared<Transaction>(1, 1, 1000, 10000, dev::bytes(), dev::KeyPair::create().secret(), addr_t::random());
  auto old_tx_hash = old_tx->getHash();

  EXPECT_EQ(priority_queue.insert(std::move(old_tx), false, 1), TransactionStatus::InsertedNonProposable);
  EXPECT_TRUE(priority_queue.isTransactionKnown(old_tx_hash));
  EXPECT_NE(priority_queue.get(old_tx_hash), nullptr);

  priority_queue.blockFinalized(25);
  EXPECT_FALSE(priority_queue.contains(old_tx_hash));
  EXPECT_FALSE(priority_queue.isTransactionKnown(old_tx_hash));
  EXPECT_EQ(priority_queue.get(old_tx_hash), nullptr);
#else
  GTEST_SKIP() << "TransactionQueue shim is disabled";
#endif
}

TEST(TransactionQueueShimTest, rustModeTransactionQueuePurgeWithoutFinalChainFailsClosedWhenNonEmpty) {
#ifdef RUSTAXA_ENABLE_TRANSACTION_QUEUE
  TransactionQueue priority_queue(nullptr);
  priority_queue.purge();

  auto tx =
      std::make_shared<Transaction>(0, 1, 1000, 10000, dev::bytes(), dev::KeyPair::create().secret(), addr_t::random());
  const auto tx_hash = tx->getHash();

  EXPECT_EQ(priority_queue.insert(std::move(tx), true, 1), TransactionStatus::Inserted);
  EXPECT_TRUE(priority_queue.contains(tx_hash));

  EXPECT_THROW(priority_queue.purge(), std::runtime_error);
  EXPECT_TRUE(priority_queue.contains(tx_hash));
#else
  GTEST_SKIP() << "TransactionQueue shim is disabled";
#endif
}

}  // namespace taraxa::core_tests
