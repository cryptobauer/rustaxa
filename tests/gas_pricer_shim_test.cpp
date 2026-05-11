#include <gtest/gtest.h>

#include <type_traits>

#include "transaction/gas_pricer.hpp"

namespace taraxa::core_tests {

TEST(GasPricerShimTest, rustModeGasPricerDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_GAS_PRICER
  static_assert(!std::is_base_of_v<GasPricerOld, GasPricer>);
  SUCCEED();
#else
  GTEST_SKIP() << "GasPricer shim is disabled";
#endif
}

}  // namespace taraxa::core_tests
