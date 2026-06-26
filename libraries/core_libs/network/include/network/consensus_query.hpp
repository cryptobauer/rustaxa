#pragma once

#include <memory>

#include "storage/storage.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::net {

#ifdef RUSTAXA_ENABLE
// ConsensusQueryApiPtr is the public RPC/GraphQL boundary handle for the
// Rust-owned read-only consensus query facade. It is created once by app/RPC
// wiring and shared by external query adapters so endpoint methods do not
// repeatedly reach through DbStorage to construct bridge facades.
using ConsensusQueryApiPtr = std::shared_ptr<rust::Box<rustaxa::BridgeConsensusQueryApi>>;

inline ConsensusQueryApiPtr createConsensusQueryApi(const std::shared_ptr<taraxa::DbStorage>& db) {
  if (!db) {
    return {};
  }
  return std::make_shared<rust::Box<rustaxa::BridgeConsensusQueryApi>>(
      rustaxa::create_consensus_query_api(db->rustStorage()));
}
#endif

}  // namespace taraxa::net
