#pragma once

#include <memory>
#include <utility>

#include "pbft/pbft_service.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

struct FullNodeConfig;
class DbStorage;

/** Builds the single fully restored Rust-mode consensus application root. */
SharedConsensusApplication createConsensusApplication(const FullNodeConfig& config, DbStorage& db);

}  // namespace taraxa
