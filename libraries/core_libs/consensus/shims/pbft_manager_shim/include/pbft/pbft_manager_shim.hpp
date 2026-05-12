#pragma once

#include <cstdint>
#include <memory>
#include <optional>
#include <vector>

#include "common/types.hpp"
#include "final_chain/final_chain.hpp"
#include "vote/pillar_vote.hpp"

namespace taraxa {

class PeriodData;

namespace pillar_chain {
class PillarChainManager;
}

/**
 * @brief Deterministic helper used by C++ sync flow to pre-validate synced pillar vote bundles
 * in Rust-enabled pillar-votes shim mode.
 */
std::optional<uint64_t> validateSyncPillarVotesBundleDeterministically(
    const std::vector<std::shared_ptr<PillarVote>>& pillar_votes, PbftPeriod required_votes_period,
    const blk_hash_t& required_pillar_block_hash, uint64_t required_threshold,
    const std::shared_ptr<final_chain::FinalChain>& final_chain);

/**
 * @brief Rust-enabled PBFT pillar-vote validation path owned by the shim layer.
 *
 * This helper mirrors the legacy sync validation side effects while routing
 * deterministic bundle acceptance through Rust. It exists so the upstream-owned
 * `PbftManager` body can keep a narrow early-return hook until a complete
 * `PbftManager` overlay owns this method directly.
 */
bool validatePbftBlockPillarVotesWithRust(
    const PeriodData& period_data, const std::shared_ptr<pillar_chain::PillarChainManager>& pillar_chain_mgr,
    const std::shared_ptr<final_chain::FinalChain>& final_chain);

}  // namespace taraxa
