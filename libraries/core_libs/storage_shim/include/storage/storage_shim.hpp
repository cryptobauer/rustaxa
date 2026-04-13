#pragma once

namespace taraxa {

// Rust-mode storage shim facade.
// This layer intentionally forwards behavior to the legacy implementation for now.
class DbStorage : public DbStorageOld {
 public:
  using DbStorageOld::DbStorageOld;
};

}  // namespace taraxa

