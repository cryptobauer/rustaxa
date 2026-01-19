#include <gtest/gtest.h>
#include <filesystem>
#include "rustaxa-bridge/src/storage.rs.h"

using namespace rustaxa::storage;

class StorageTest : public ::testing::Test {
protected:
    void SetUp() override {
        test_dir = std::filesystem::temp_directory_path() / "rustaxa_storage_test";
        if (std::filesystem::exists(test_dir)) {
            std::filesystem::remove_all(test_dir);
        }
    }

    void TearDown() override {
        if (std::filesystem::exists(test_dir)) {
            std::filesystem::remove_all(test_dir);
        }
    }

    std::filesystem::path test_dir;
};

TEST_F(StorageTest, CreateStorage) {
    auto storage = create_storage(test_dir.string());
    // rust::Box cannot be null, so just testing that creation doesn't throw
    SUCCEED();
}
