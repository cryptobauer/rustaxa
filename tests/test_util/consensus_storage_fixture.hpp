#pragma once

#include <filesystem>
#include <memory>
#include <utility>

#include "config/config.hpp"
#include "storage/storage.hpp"
namespace taraxa::core_tests {

/**
 * Owns the production-shaped storage bootstrap dependencies used by dual-mode C++ tests.
 *
 * This legacy manager-shaped fixture is compiled only by untouched pure-C++ suites. Rust-mode
 * tests use application-root tasks and query clients instead of constructing `DbStorage`.
 */
struct ConsensusStorageFixture {
  std::shared_ptr<DbStorage> db;
};

/** Constructs a dual-mode storage fixture at `path` using a copied node configuration. */
inline ConsensusStorageFixture makeConsensusStorageFixture(FullNodeConfig config, const std::filesystem::path& path) {
  (void)config;
  return {std::make_shared<DbStorage>(path)};
}

/** Closes and reopens a fixture so native startup services restore newly seeded storage state. */
inline void reopenConsensusStorageFixture(ConsensusStorageFixture& fixture, FullNodeConfig config,
                                          const std::filesystem::path& path) {
  fixture.db.reset();
  fixture = makeConsensusStorageFixture(std::move(config), path);
}

}  // namespace taraxa::core_tests
