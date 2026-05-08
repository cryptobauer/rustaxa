#include <gtest/gtest.h>

#include <type_traits>

#include "pbft/pbft_chain.hpp"

namespace taraxa::core_tests {

TEST(PbftChainShimTest, rustModePbftChainDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_PBFT_CHAIN
  static_assert(!std::is_base_of_v<PbftChainOld, PbftChain>);
  SUCCEED();
#else
  GTEST_SKIP() << "PbftChain shim is disabled";
#endif
}

}  // namespace taraxa::core_tests
