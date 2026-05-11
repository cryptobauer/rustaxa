#include <gtest/gtest.h>

#include <type_traits>

#include "slashing_manager/slashing_manager.hpp"

namespace taraxa::core_tests {

TEST(SlashingManagerShimTest, rustModeSlashingManagerDoesNotInheritLegacyImplementation) {
#ifdef RUSTAXA_ENABLE_SLASHING_MANAGER
  static_assert(!std::is_base_of_v<SlashingManagerOld, SlashingManager>);
  static_assert(!std::is_copy_constructible_v<SlashingManager>);
  static_assert(!std::is_copy_assignable_v<SlashingManager>);
  static_assert(!std::is_move_constructible_v<SlashingManager>);
  static_assert(!std::is_move_assignable_v<SlashingManager>);
  SUCCEED();
#else
  GTEST_SKIP() << "SlashingManager shim is disabled";
#endif
}

}  // namespace taraxa::core_tests
