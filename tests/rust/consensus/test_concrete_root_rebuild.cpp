#include <gtest/gtest.h>

#include <algorithm>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <memory>
#include <string>
#include <vector>

#include <rocksdb/db.h>

#include "consensus/consensus_application.hpp"
#include "storage/storage.hpp"
#include "test_util/test_util.hpp"

namespace taraxa {
namespace {

/**
 * Exercises the Rust-mode App startup policy at its filesystem boundary.
 *
 * The fixture first creates a production-shaped paired consensus/StateAPI
 * database, then removes only the concrete-root policy marker to model a
 * markerless synthetic-history database. Ordinary startup must fail closed.
 * The explicitly requested rebuild must move the complete old database root
 * to one timestamped backup and construct a distinct fresh database pair.
 */
class ConcreteRootRebuildTest : public NodesTest {};

void removeConcreteRootPolicyMarker(const std::filesystem::path& database_root) {
  rocksdb::Options options;
  std::vector<std::string> column_names;
  const auto list_status = rocksdb::DB::ListColumnFamilies(options, (database_root / "db").string(), &column_names);
  ASSERT_TRUE(list_status.ok()) << list_status.ToString();

  std::vector<rocksdb::ColumnFamilyDescriptor> descriptors;
  descriptors.reserve(column_names.size());
  for (const auto& name : column_names) {
    auto column_options = rocksdb::ColumnFamilyOptions();
    const auto legacy_column = std::find_if(DbStorage::Columns::all.begin(), DbStorage::Columns::all.end(),
                                            [&name](const auto& column) { return column.name() == name; });
    if (legacy_column != DbStorage::Columns::all.end() && legacy_column->comparator_) {
      column_options.comparator = legacy_column->comparator_;
    }
    descriptors.emplace_back(name, column_options);
  }

  std::vector<rocksdb::ColumnFamilyHandle*> handles;
  rocksdb::DB* raw_db = nullptr;
  const auto open_status = rocksdb::DB::Open(options, (database_root / "db").string(), descriptors, &handles, &raw_db);
  ASSERT_TRUE(open_status.ok()) << open_status.ToString();
  std::unique_ptr<rocksdb::DB> db(raw_db);

  const auto status_index = std::find(column_names.begin(), column_names.end(), DbStorage::Columns::status.name());
  ASSERT_NE(status_index, column_names.end());
  const auto status_handle = handles.at(std::distance(column_names.begin(), status_index));
  const uint8_t policy_key = 7;
  const auto delete_status = db->Delete(rocksdb::WriteOptions(), status_handle,
                                        rocksdb::Slice(reinterpret_cast<const char*>(&policy_key), sizeof(policy_key)));
  EXPECT_TRUE(delete_status.ok()) << delete_status.ToString();
  for (auto* handle : handles) {
    EXPECT_TRUE(db->DestroyColumnFamilyHandle(handle).ok());
  }
}

TEST_F(ConcreteRootRebuildTest, MarkerlessHistoryRejectsUntilTimestampedPairRebuild) {
  auto configs = make_node_cfgs(1, 1, 5);
  auto& config = configs.front();
  const auto database_root = config.db_path;
  const auto backup_prefix = database_root.filename().string() + ".concrete-root-rebuild-backup-";
  const auto sentinel = database_root / "pre_rebuild_pair_sentinel";

  {
    auto node = create_node(config, false);
    const auto query = node->getConsensusApplication()->queryClient();
    ASSERT_TRUE((*query)->consensus_query_final_chain_block_by_number(0).found);
  }
  ASSERT_TRUE(std::filesystem::exists(database_root / "db"));
  ASSERT_TRUE(std::filesystem::exists(database_root / "state_db"));
  {
    std::ofstream marker(sentinel);
    ASSERT_TRUE(marker.good());
    marker << "old paired database";
  }

  // Status key seven is the native FinalChainRootPolicy field. Removing it
  // while retaining finalized genesis reproduces pre-policy synthetic history.
  removeConcreteRootPolicyMarker(database_root);

  try {
    auto rejected = create_node(config, false);
    (void)rejected;
    FAIL() << "markerless finalized history must not start";
  } catch (const std::exception& error) {
    EXPECT_NE(std::string(error.what()).find("FINAL_CHAIN_CONCRETE_ROOT_REBUILD_REQUIRED"), std::string::npos)
        << error.what();
  }

  config.db_config.rebuild_db = true;
  {
    auto rebuilt = create_node(config, false);
    const auto query = rebuilt->getConsensusApplication()->queryClient();
    ASSERT_TRUE((*query)->consensus_query_final_chain_block_by_number(0).found);
    EXPECT_EQ((*query)->consensus_query_final_chain_last_block_number(), 0);
  }

  std::vector<std::filesystem::path> backups;
  for (const auto& entry : std::filesystem::directory_iterator(database_root.parent_path())) {
    const auto name = entry.path().filename().string();
    if (name.starts_with(backup_prefix)) {
      backups.push_back(entry.path());
    }
  }
  ASSERT_EQ(backups.size(), 1);
  EXPECT_TRUE(std::filesystem::exists(backups.front() / "db"));
  EXPECT_TRUE(std::filesystem::exists(backups.front() / "state_db"));
  EXPECT_TRUE(std::filesystem::exists(backups.front() / sentinel.filename()));
  EXPECT_TRUE(std::filesystem::exists(database_root / "db"));
  EXPECT_TRUE(std::filesystem::exists(database_root / "state_db"));
  EXPECT_FALSE(std::filesystem::exists(sentinel));
}

}  // namespace
}  // namespace taraxa
