#pragma once

#include <filesystem>
#include <memory>
#include <utility>

#include "config/config.hpp"
#include "storage/storage.hpp"
#ifdef RUSTAXA_ENABLE
#include "transaction/dag_transaction_service.hpp"
#endif

namespace taraxa::core_tests {

/**
 * Owns the production-shaped storage bootstrap dependencies used by dual-mode C++ tests.
 *
 * Rust mode constructs one native application root first and injects that root into the
 * storage shim. Pure-C++ mode retains the legacy standalone DbStorage construction. The
 * fixture keeps both owners alive for the lifetime of `db` and publishes no partial Rust
 * bootstrap handles.
 */
struct ConsensusStorageFixture {
#ifdef RUSTAXA_ENABLE
  SharedConsensusApplication application;
#endif
  std::shared_ptr<DbStorage> db;
};

/** Constructs a dual-mode storage fixture at `path` using a copied node configuration. */
inline ConsensusStorageFixture makeConsensusStorageFixture(FullNodeConfig config, const std::filesystem::path& path) {
#ifdef RUSTAXA_ENABLE
  config.db_path = path;
  auto application = createConsensusApplication(config);
  auto db = std::make_shared<DbStorage>(application, path);
  return {std::move(application), std::move(db)};
#else
  (void)config;
  return {std::make_shared<DbStorage>(path)};
#endif
}

/** Closes and reopens a fixture so native startup services restore newly seeded storage state. */
inline void reopenConsensusStorageFixture(ConsensusStorageFixture& fixture, FullNodeConfig config,
                                          const std::filesystem::path& path) {
  fixture.db.reset();
#ifdef RUSTAXA_ENABLE
  fixture.application.reset();
#endif
  fixture = makeConsensusStorageFixture(std::move(config), path);
}

}  // namespace taraxa::core_tests
