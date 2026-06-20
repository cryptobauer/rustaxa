#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <filesystem>
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

  static DagManagerBlock manager_block(uint8_t hash, uint8_t pivot, uint64_t level, uint32_t difficulty) {
    DagManagerBlock block;
    block.hash = h256(hash);
    block.pivot = h256(pivot);
    block.level = level;
    block.difficulty = difficulty;
    return block;
  }

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

  static rust::Vec<uint8_t> tx_payload(std::initializer_list<uint8_t> bytes) {
    rust::Vec<uint8_t> out;
    out.reserve(bytes.size());
    for (auto byte : bytes) {
      out.push_back(byte);
    }
    return out;
  }

  static std::vector<uint8_t> byte_vector(const rust::Vec<uint8_t>& bytes) {
    return std::vector<uint8_t>(bytes.begin(), bytes.end());
  }

  static rust::Box<BridgeTransactionStorageQueries> transaction_queries(const rust::Box<BridgeStorage>& storage) {
    return create_transaction_storage_queries(*storage);
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

TEST_F(RustDagGraphTest, RuntimeSelectNonFinalizedHashesExcludesKnownInOrder) {
  const auto test_dir = std::filesystem::temp_directory_path() / "rustaxa_consensus_dag_runtime_select_hashes";
  if (std::filesystem::exists(test_dir)) {
    std::filesystem::remove_all(test_dir);
  }

  auto storage = create_storage(test_dir.string());
  auto runtime = create_dag_manager_runtime_from_storage(h256(1), 32, *storage);

  runtime->dag_manager_runtime_add_block(manager_block(6, 3, 4, 85));
  runtime->dag_manager_runtime_add_block(manager_block(2, 1, 2, 100));
  runtime->dag_manager_runtime_add_block(manager_block(3, 2, 3, 90));
  runtime->dag_manager_runtime_add_block(manager_block(4, 3, 4, 80));

  const auto selected = runtime->dag_manager_runtime_select_non_finalized_hashes(hashes({2, 9, 2}));

  EXPECT_EQ(last_bytes(selected), (std::vector<uint8_t>{3, 4, 6}));

  std::filesystem::remove_all(test_dir);
}

TEST_F(RustDagGraphTest, RuntimeNonFinalizedSyncSnapshotAndTransactionRlpLookup) {
  const auto test_dir = std::filesystem::temp_directory_path() / "rustaxa_consensus_dag_runtime_snapshot_tx_rlp";
  if (std::filesystem::exists(test_dir)) {
    std::filesystem::remove_all(test_dir);
  }

  auto storage = create_storage(test_dir.string());
  auto runtime = create_dag_manager_runtime_from_storage(h256(1), 32, *storage);

  const auto tx_hash_a = h256(17);
  const auto tx_hash_b = h256(18);
  const auto tx_hash_missing = h256(19);
  storage->save_transaction(tx_hash_a, tx_payload({1, 2, 3}));
  storage->save_transaction(tx_hash_b, tx_payload({4, 5, 6}));

  runtime->dag_manager_runtime_add_block(manager_block(6, 3, 4, 85));
  runtime->dag_manager_runtime_add_block(manager_block(2, 1, 2, 100));

  const auto sync_snapshot = runtime->dag_manager_runtime_non_finalized_sync_snapshot(hashes({}));
  EXPECT_EQ(sync_snapshot.period, 0u);
  EXPECT_EQ(last_bytes(sync_snapshot.selected_hashes), (std::vector<uint8_t>{2, 6}));

  const auto trxs = transaction_queries(storage)->get_transaction_rlps_by_hashes({
      rustaxa::DagTransactionHash{tx_hash_a},
      rustaxa::DagTransactionHash{tx_hash_b},
      rustaxa::DagTransactionHash{tx_hash_missing},
  });
  ASSERT_EQ(trxs.size(), 3u);
  EXPECT_TRUE(trxs[0].found);
  EXPECT_TRUE(trxs[1].found);
  EXPECT_FALSE(trxs[2].found);
  EXPECT_EQ(trxs[0].hash, tx_hash_a);
  EXPECT_EQ(trxs[1].hash, tx_hash_b);
  EXPECT_EQ(trxs[2].hash, tx_hash_missing);
  EXPECT_EQ(byte_vector(trxs[0].tx_rlp), (std::vector<uint8_t>{1, 2, 3}));
  EXPECT_EQ(byte_vector(trxs[1].tx_rlp), (std::vector<uint8_t>{4, 5, 6}));
  EXPECT_TRUE(trxs[2].tx_rlp.empty());

  std::filesystem::remove_all(test_dir);
}
