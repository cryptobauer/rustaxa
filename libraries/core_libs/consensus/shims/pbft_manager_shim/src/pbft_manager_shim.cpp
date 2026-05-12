#include "pbft/pbft_manager_shim.hpp"

#include <array>
#include <exception>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

namespace {

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }

std::array<uint8_t, 20> toBridgeAddress(const addr_t& address) { return address.asArray(); }

std::optional<uint64_t> getPillarVoteWeight(const std::shared_ptr<final_chain::FinalChain>& final_chain,
                                            const std::shared_ptr<PillarVote>& vote,
                                            PbftPeriod required_votes_period) {
  try {
    const auto weight = final_chain->dposEligibleVoteCount(required_votes_period - 1, vote->getVoterAddr());
    if (weight == 0) {
      return std::nullopt;
    }
    return weight;
  } catch (const state_api::ErrFutureBlock&) {
    return std::nullopt;
  } catch (const std::exception&) {
    return std::nullopt;
  }
}

}  // namespace

std::optional<uint64_t> validateSyncPillarVotesBundleDeterministically(
    const std::vector<std::shared_ptr<PillarVote>>& pillar_votes, PbftPeriod required_votes_period,
    const blk_hash_t& required_pillar_block_hash, uint64_t required_threshold,
    const std::shared_ptr<final_chain::FinalChain>& final_chain) {
  if (!final_chain) {
    return std::nullopt;
  }

  if (required_votes_period == 0) {
    return std::nullopt;
  }

  rust::Vec<rustaxa::PillarVoteBundleFact> facts;
  facts.reserve(pillar_votes.size());
  for (const auto& vote : pillar_votes) {
    if (!vote) {
      return std::nullopt;
    }

    const auto weight = getPillarVoteWeight(final_chain, vote, required_votes_period);
    if (!weight) {
      return std::nullopt;
    }

    rustaxa::PillarVoteBundleFact fact{};
    fact.vote_hash = toBridgeHash(vote->getHash());
    fact.block_hash = toBridgeHash(vote->getBlockHash());
    fact.voter = toBridgeAddress(vote->getVoterAddr());
    fact.period = vote->getPeriod();
    fact.weight = *weight;
    fact.prevalidated = vote->verifyVote();
    facts.push_back(fact);
  }

  try {
    const auto plan = rustaxa::plan_pillar_vote_bundle(std::move(facts), required_votes_period,
                                                       toBridgeHash(required_pillar_block_hash), required_threshold);
    if (plan.status != 0) {
      return std::nullopt;
    }

    return plan.block_weight;
  } catch (const std::exception&) {
    return std::nullopt;
  }
}

}  // namespace taraxa
