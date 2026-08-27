#pragma once

#include <memory>
#include <stdexcept>

#include "common/app_base.hpp"

#ifndef RUSTAXA_ENABLE
#include "pbft/pbft_chain.hpp"
#endif

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::net {

#ifdef RUSTAXA_ENABLE
// ConsensusQueryApiPtr is the public RPC/GraphQL boundary handle for the
// application-root-owned read-only consensus query facade.
using ConsensusQueryApiPtr = std::shared_ptr<rust::Box<rustaxa::BridgeConsensusQueryApi>>;

using ConsensusQueryClient = ConsensusQueryApiPtr;
#else
using ConsensusQueryClient = std::shared_ptr<taraxa::PbftChain>;
#endif

/** Returns coherent live PBFT progress through the mode-specific client boundary. */
inline PbftProgress consensusPbftProgress(const ConsensusQueryClient& query) {
  if (!query) {
    throw std::invalid_argument("Consensus query client is unavailable");
  }
#ifdef RUSTAXA_ENABLE
  const auto progress = (*query)->consensus_query_chain_stats();
  return {progress.pbft_period, progress.non_empty_pbft_periods};
#endif
#ifndef RUSTAXA_ENABLE
  return {query->getPbftChainSize(), query->getPbftChainSizeExcludingEmptyPbftBlocks()};
#endif
}

/** Returns finalized PBFT block membership through the mode-specific read boundary. */
inline bool consensusPbftSyncBlockExists(const ConsensusQueryClient& query, const blk_hash_t& hash) {
  if (!query) {
    throw std::invalid_argument("Consensus query client is unavailable");
  }
#ifdef RUSTAXA_ENABLE
  return (*query)->consensus_query_pbft_sync_block_exists(hash.asArray());
#endif
#ifndef RUSTAXA_ENABLE
  return query->findPbftBlockInChain(hash);
#endif
}

}  // namespace taraxa::net
