#include <gtest/gtest.h>

#include <stdexcept>
#include <type_traits>

#include "dag/dag.hpp"

namespace taraxa::core_tests {

template <typename T, typename = void>
struct HasVertexType : std::false_type {};

template <typename T>
struct HasVertexType<T, std::void_t<typename T::vertex_t>> : std::true_type {};

TEST(DagShimTest, rustModeDagKeepsGraphTypesHashOnly) {
#ifdef RUSTAXA_ENABLE
  static_assert(!HasVertexType<Dag>::value);
  static_assert(!HasVertexType<PivotTree>::value);
  SUCCEED();
#else
  GTEST_SKIP() << "Dag shim is disabled";
#endif
}

TEST(DagShimTest, rustModeCopyAttemptsThrow) {
#ifdef RUSTAXA_ENABLE
  Dag graph(blk_hash_t(1), addr_t());
  Dag other(blk_hash_t(1), addr_t());
  PivotTree pivot_tree(blk_hash_t(1), addr_t());
  PivotTree other_pivot_tree(blk_hash_t(1), addr_t());

  EXPECT_THROW(
      {
        Dag copy(graph);
        (void)copy;
      },
      std::logic_error);
  EXPECT_THROW(graph = other, std::logic_error);

  EXPECT_THROW(
      {
        PivotTree copy(pivot_tree);
        (void)copy;
      },
      std::logic_error);
  EXPECT_THROW(pivot_tree = other_pivot_tree, std::logic_error);
#else
  GTEST_SKIP() << "Dag shim is disabled";
#endif
}

TEST(DagShimTest, rustModeRejectsZeroVertexBeforeBridge) {
#ifdef RUSTAXA_ENABLE
  Dag graph(blk_hash_t(1), addr_t());

  EXPECT_THROW(Dag(blk_hash_t(), addr_t()), std::invalid_argument);
  EXPECT_THROW(graph.addVEEs(blk_hash_t(), blk_hash_t(1), {}), std::invalid_argument);
#else
  GTEST_SKIP() << "Dag shim is disabled";
#endif
}

}  // namespace taraxa::core_tests
