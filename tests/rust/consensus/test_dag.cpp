#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <utility>
#include <vector>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

class RustDagGraphTest : public ::testing::Test {
 protected:
  static std::array<uint8_t, 32> h256(uint8_t last_byte) {
    std::array<uint8_t, 32> hash{};
    hash[31] = last_byte;
    return hash;
  }

  static DagHash dag_hash(uint8_t last_byte) { return DagHash{h256(last_byte)}; }

  static rust::Vec<DagHash> hashes(std::initializer_list<uint8_t> values) {
    rust::Vec<DagHash> out;
    out.reserve(values.size());
    for (auto value : values) {
      out.push_back(dag_hash(value));
    }
    return out;
  }

  static rust::Vec<DagLevelHashes> non_finalized(std::initializer_list<uint8_t> values) {
    rust::Vec<DagLevelHashes> out;
    DagLevelHashes level_hashes;
    level_hashes.level = 1;
    level_hashes.hashes = hashes(values);
    out.push_back(std::move(level_hashes));
    return out;
  }

  static std::vector<uint8_t> last_bytes(const rust::Vec<DagHash>& hashes) {
    std::vector<uint8_t> out;
    out.reserve(hashes.size());
    for (const auto& hash : hashes) {
      out.push_back(hash.hash[31]);
    }
    return out;
  }
};

TEST_F(RustDagGraphTest, BasicGraphOperationsMatchDagTestFixtures) {
  auto graph = create_dag_graph(h256(1));

  EXPECT_EQ(graph->dag_vertex_count(), 1u);
  EXPECT_EQ(graph->dag_edge_count(), 0u);
  EXPECT_TRUE(graph->dag_has_vertex(h256(1)));
  EXPECT_EQ(last_bytes(graph->dag_leaves()), std::vector<uint8_t>{1});

  EXPECT_TRUE(graph->dag_add_vertex_edges(h256(2), h256(1), hashes({})));
  EXPECT_FALSE(graph->dag_add_vertex_edges(h256(2), h256(1), hashes({})));
  EXPECT_TRUE(graph->dag_add_vertex_edges(h256(3), h256(1), hashes({})));
  EXPECT_TRUE(graph->dag_add_vertex_edges(h256(4), h256(2), hashes({3})));

  EXPECT_EQ(graph->dag_vertex_count(), 4u);
  EXPECT_EQ(graph->dag_edge_count(), 4u);
  EXPECT_EQ(last_bytes(graph->dag_leaves()), std::vector<uint8_t>{4});
}

TEST_F(RustDagGraphTest, GhostPathMatchesHeaviestSubtreeTieBreak) {
  auto graph = create_dag_graph(h256(1));

  graph->dag_add_vertex_edges(h256(3), h256(1), hashes({}));
  graph->dag_add_vertex_edges(h256(2), h256(1), hashes({}));
  graph->dag_add_vertex_edges(h256(4), h256(3), hashes({}));
  graph->dag_add_vertex_edges(h256(5), h256(3), hashes({}));

  EXPECT_EQ(last_bytes(graph->dag_ghost_path(h256(1))), (std::vector<uint8_t>{1, 3, 4}));
  EXPECT_TRUE(graph->dag_ghost_path(h256(99)).empty());
}

TEST_F(RustDagGraphTest, ComputeOrderMatchesConfluxPeriodFourFixture) {
  auto graph = create_dag_graph(h256(1));

  graph->dag_add_vertex_edges(h256(2), h256(1), hashes({}));
  graph->dag_add_vertex_edges(h256(3), h256(1), hashes({}));
  graph->dag_add_vertex_edges(h256(4), h256(2), hashes({3}));
  graph->dag_add_vertex_edges(h256(5), h256(2), hashes({}));
  graph->dag_add_vertex_edges(h256(7), h256(3), hashes({}));
  graph->dag_add_vertex_edges(h256(6), h256(4), hashes({5, 7}));
  graph->dag_add_vertex_edges(h256(8), h256(2), hashes({}));
  graph->dag_add_vertex_edges(h256(11), h256(7), hashes({}));
  graph->dag_add_vertex_edges(h256(10), h256(11), hashes({4}));
  graph->dag_add_vertex_edges(h256(9), h256(6), hashes({8, 10}));
  graph->dag_add_vertex_edges(h256(12), h256(9), hashes({}));

  const auto order = graph->dag_compute_order(h256(9), non_finalized({8, 9, 10, 11}));

  ASSERT_TRUE(order.found);
  EXPECT_EQ(last_bytes(order.hashes), (std::vector<uint8_t>{11, 10, 8, 9}));

  const auto missing = graph->dag_compute_order(h256(99), non_finalized({8, 9, 10, 11}));
  EXPECT_FALSE(missing.found);
  EXPECT_TRUE(missing.hashes.empty());
}
