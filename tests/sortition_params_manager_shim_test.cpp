#include <gtest/gtest.h>

#include <type_traits>

#include "dag/sortition_params_manager.hpp"

namespace taraxa::core_tests {

TEST(SortitionParamsManagerShimTest, rustModeManagerDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_SORTITION_PARAMS
  static_assert(!std::is_base_of_v<SortitionParamsManagerOld, SortitionParamsManager>);
  SUCCEED();
#else
  GTEST_SKIP() << "SortitionParamsManager shim is disabled";
#endif
}

}  // namespace taraxa::core_tests
