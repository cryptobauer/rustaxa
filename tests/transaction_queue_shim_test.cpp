#include <gtest/gtest.h>

#include <type_traits>

#include "transaction/transaction_queue.hpp"

namespace taraxa::core_tests {

TEST(TransactionQueueShimTest, rustModeTransactionQueueDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_TRANSACTION_QUEUE
  static_assert(!std::is_base_of_v<TransactionQueueOld, TransactionQueue>);
  SUCCEED();
#else
  GTEST_SKIP() << "TransactionQueue shim is disabled";
#endif
}

}  // namespace taraxa::core_tests
