#include "pbft/pbft_manager_shim.hpp"

#include <array>
#include <exception>
#include <utility>

#include "pbft/period_data.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
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

bool validatePbftBlockPillarVotesWithRust(
    const PeriodData& period_data, const std::shared_ptr<pillar_chain::PillarChainManager>& pillar_chain_mgr,
    const std::shared_ptr<final_chain::FinalChain>& final_chain) {
  if (!pillar_chain_mgr || !period_data.pbft_blk || !period_data.pillar_votes_.has_value() ||
      period_data.pillar_votes_->empty()) {
    return false;
  }

  const auto required_votes_period = period_data.pbft_blk->getPeriod();
  const auto current_pillar_block = pillar_chain_mgr->getCurrentPillarBlock();
  if (!current_pillar_block || current_pillar_block->getPeriod() + 1 != required_votes_period) {
    return false;
  }

  const auto pillar_consensus_threshold = pillar_chain_mgr->getPillarConsensusThreshold(required_votes_period - 1);
  if (!pillar_consensus_threshold) {
    return false;
  }

  const auto rust_votes_weight = validateSyncPillarVotesBundleDeterministically(
      *period_data.pillar_votes_, required_votes_period, current_pillar_block->getHash(), *pillar_consensus_threshold,
      final_chain);
  if (!rust_votes_weight) {
    return false;
  }

  uint64_t votes_weight = 0;
  for (auto& vote : *period_data.pillar_votes_) {
    if (!vote || vote->getPeriod() != required_votes_period || vote->getBlockHash() != current_pillar_block->getHash()) {
      return false;
    }

    if (!pillar_chain_mgr->validatePillarVote(vote)) {
      return false;
    }

    if (const auto vote_weight = pillar_chain_mgr->addVerifiedPillarVote(vote); vote_weight) {
      votes_weight += vote_weight;
    } else {
      return false;
    }
  }

  return votes_weight >= *pillar_consensus_threshold;
}

}  // namespace taraxa
