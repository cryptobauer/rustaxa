#pragma once

#include <memory>
#include <utility>

#include "consensus/consensus_application.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

struct FullNodeConfig;
/** Builds the sole Rust-mode root from `config`, publishing it only after storage/schema/genesis validation,
 * FinalChain construction, and DAG/transaction/PBFT restoration all succeed. */
SharedConsensusApplication createConsensusApplication(const FullNodeConfig& config);

}  // namespace taraxa
