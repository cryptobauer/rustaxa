#pragma once

#include <cstdint>
#include <memory>
#include <optional>
#include <vector>

#include "common/types.hpp"
#include "final_chain/final_chain.hpp"
#include "vote/pillar_vote.hpp"

namespace taraxa {

/**
 * @brief Deterministic helper used by C++ sync flow to pre-validate synced pillar vote bundles
 * in Rust-enabled pillar-votes shim mode.
 */
std::optional<uint64_t> validateSyncPillarVotesBundleDeterministically(
    const std::vector<std::shared_ptr<PillarVote>>& pillar_votes, PbftPeriod required_votes_period,
    const blk_hash_t& required_pillar_block_hash, uint64_t required_threshold,
    const std::shared_ptr<final_chain::FinalChain>& final_chain);

}  // namespace taraxa
