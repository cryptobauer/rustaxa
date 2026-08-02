#include "network/tarcap/packets_handlers/latest/common/ext_pillar_vote_packet_handler.hpp"

#include <cassert>

#include "pillar_chain/pillar_chain_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {
constexpr uint8_t kPillarVoteRelevanceStatusRelevant = 0;
constexpr uint8_t kPillarVoteRelevanceStatusVoteAlreadyKnown = 1;
constexpr uint8_t kPillarVoteRelevanceStatusMissingCurrentPillarBlock = 2;
constexpr uint8_t kPillarVoteRelevanceStatusVotePeriodMismatch = 3;
constexpr uint8_t kPillarVoteRelevanceStatusVoteBlockHashMismatch = 4;

}  // namespace
#endif

ExtPillarVotePacketHandler::ExtPillarVotePacketHandler(
    const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager,
#ifdef RUSTAXA_ENABLE
    network::ConsensusNetworkApiShared consensus_network_api,
#endif
    const addr_t &node_addr, const std::string &log_channel)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, log_channel),
      pillar_chain_manager_{std::move(pillar_chain_manager)}
#ifdef RUSTAXA_ENABLE
      ,
      rust_consensus_network_api_(std::move(consensus_network_api))
#endif
{
}

bool ExtPillarVotePacketHandler::processPillarVote(const std::shared_ptr<PillarVote> &vote,
                                                   const std::shared_ptr<TaraxaPeer> &peer,
                                                   SubprotocolPacketType packet_type) {
  (void)packet_type;

#ifdef RUSTAXA_ENABLE
  const auto relevance_plan = planPillarVoteRelevance(vote);
  if (!relevance_plan.is_relevant) {
    const auto &ficus_hf_config = kConf.genesis.state.hardforks.ficus_hf;
    const auto current_pillar_block = pillar_chain_manager_->getCurrentPillarBlock();
    switch (relevance_plan.status) {
      case kPillarVoteRelevanceStatusVoteAlreadyKnown:
        LOG(this->log_dg_) << "Received vote " << vote->getHash() << " already saved";
        return false;
      case kPillarVoteRelevanceStatusMissingCurrentPillarBlock:
        LOG(this->log_nf_) << "Received vote's period " << vote->getPeriod()
                           << ", no pillar block created yet. Accepting votes with "
                           << ficus_hf_config.firstPillarBlockPeriod() + 1 << " period";
        return false;
      case kPillarVoteRelevanceStatusVotePeriodMismatch:
        if (!current_pillar_block) {
          LOG(this->log_nf_) << "Received vote's period " << vote->getPeriod() << ", current pillar block missing";
        } else {
          LOG(this->log_nf_) << "Received vote's period " << vote->getPeriod() << ", current pillar block period "
                             << current_pillar_block->getPeriod();
        }
        return false;
      case kPillarVoteRelevanceStatusVoteBlockHashMismatch:
        LOG(this->log_nf_) << "Received vote's block hash " << vote->getBlockHash() << " != current pillar block hash "
                           << current_pillar_block->getHash();
        return false;
      case kPillarVoteRelevanceStatusRelevant:
        break;
      default:
        LOG(this->log_wr_) << "Unable to evaluate pillar vote relevance for " << vote->getHash()
                           << ": network api status " << static_cast<uint32_t>(relevance_plan.status);
        return false;
    }
  }
#else
  if (!pillar_chain_manager_->isRelevantPillarVote(vote)) {
    LOG(this->log_dg_) << "Drop irrelevant pillar vote " << vote->getHash() << ", period " << vote->getPeriod()
                       << " from peer " << peer->getId();
    return false;
  }
#endif

#ifdef RUSTAXA_ENABLE
  if (!pillar_chain_manager_->validatePillarVote(vote)) {
    return false;
  }
#else
  if (!pillar_chain_manager_->validatePillarVote(vote)) {
    // TODO: enable for mainnet
    // std::ostringstream err_msg;
    // err_msg << "Invalid pillar vote " << vote->getHash() << " from peer " << peer->getId();
    // throw MaliciousPeerException(err_msg.str());
    return false;
  }
#endif

  pillar_chain_manager_->addVerifiedPillarVote(vote);

  // Mark pillar vote as known for peer
  peer->markPillarVoteAsKnown(vote->getHash());
  return true;
}

#ifdef RUSTAXA_ENABLE
rustaxa::PillarVoteRelevancePlan ExtPillarVotePacketHandler::planPillarVoteRelevance(
    const std::shared_ptr<PillarVote> &vote) const {
  assert(rust_consensus_network_api_);
  rustaxa::PillarVoteRelevanceFact fact{};
  fact.vote_period = vote->getPeriod();
  fact.vote_block_hash = vote->getBlockHash().asArray();
  const auto &ficus_hf_config = kConf.genesis.state.hardforks.ficus_hf;
  fact.first_pillar_block_period = ficus_hf_config.firstPillarBlockPeriod();
  fact.pillar_blocks_interval = ficus_hf_config.pillar_blocks_interval;
  // Duplicate rejection remains covered by validatePillarVote during this slice
  // because tarcap cannot inspect the Rust-backed pillar vote index directly.
  fact.vote_already_known = false;

  if (const auto current_pillar_block = pillar_chain_manager_->getCurrentPillarBlock(); current_pillar_block) {
    fact.has_current_pillar_block = true;
    fact.current_pillar_block_period = current_pillar_block->getPeriod();
    fact.current_pillar_block_hash = current_pillar_block->getHash().asArray();
  }

  return rust_consensus_network_api_->api().consensus_network_plan_pillar_vote_relevance(fact);
}
#endif

ExtPillarVotePacketHandler::~ExtPillarVotePacketHandler() = default;

}  // namespace taraxa::network::tarcap
