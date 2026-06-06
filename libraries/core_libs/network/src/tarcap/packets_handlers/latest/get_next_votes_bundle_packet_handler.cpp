#include "network/tarcap/packets_handlers/latest/get_next_votes_bundle_packet_handler.hpp"

#include <libdevcore/RLP.h>

#include "pbft/pbft_manager.hpp"
#include "vote_manager/vote_manager.hpp"

namespace taraxa::network::tarcap {

GetNextVotesBundlePacketHandler::GetNextVotesBundlePacketHandler(
    const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, std::shared_ptr<PbftManager> pbft_mgr,
    std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<VoteManager> vote_mgr,
    std::shared_ptr<SlashingManager> slashing_manager, const addr_t &node_addr, const std::string &logs_prefix)
    : IVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_mgr),
                         std::move(pbft_chain), std::move(vote_mgr), std::move(slashing_manager), node_addr,
                         logs_prefix + "GET_NEXT_VOTES_BUNDLE_PH") {}

void GetNextVotesBundlePacketHandler::process(const threadpool::PacketData &packet_data,
                                              const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<GetNextVotesBundlePacket>(packet_data.rlp_);

  LOG(log_dg_) << "Received GetNextVotesSyncPacket request";
  const auto [pbft_round, pbft_period] = pbft_mgr_->getPbftRoundAndPeriod();

  // Send votes only for current_period == peer_period && current_period >= peer_round
  if (pbft_period != packet.peer_pbft_period || pbft_round == 1 || pbft_round < packet.peer_pbft_round) {
    LOG(log_nf_) << "No previous round next votes sync packet will be sent. pbft_period " << pbft_period
                 << ", peer_pbft_period " << packet.peer_pbft_period << ", pbft_round " << pbft_round
                 << ", peer_pbft_round " << packet.peer_pbft_round;
    return;
  }

#ifdef RUSTAXA_ENABLE
  constexpr uint8_t kPbftOptimizedBundleReady = 0;

  auto has_votes_to_plan = [](const rustaxa::PbftOptimizedVoteBundlePlan& plan) {
    return plan.found && !plan.vote_hashes.empty();
  };

  auto bridge_hash_to_vote_hash = [](const std::array<uint8_t, 32>& hash) {
    return vote_hash_t(hash.data(), vote_hash_t::ConstructFromPointer);
  };

  auto bridge_bytes_to_dev_bytes = [](const rust::Vec<uint8_t>& bytes) {
    dev::bytes out;
    out.reserve(bytes.size());
    for (const auto byte : bytes) {
      out.push_back(byte);
    }
    return out;
  };

  auto build_request = [](const rustaxa::PbftOptimizedVoteBundlePlan& plan) {
    rustaxa::PbftOptimizedVoteBundleBuildRequest request;
    request.kind = plan.kind;
    request.block_hash = plan.block_hash;
    request.period = plan.period;
    request.round = plan.round;
    request.step = plan.step;
    return request;
  };

  auto send_optimized_bundle = [this, &peer, &bridge_hash_to_vote_hash,
                                &bridge_bytes_to_dev_bytes](rustaxa::PbftOptimizedVoteBundleBuildResult result,
                                                            const char* label) {
    if (result.status != kPbftOptimizedBundleReady) {
      LOG(log_er_) << "Rust optimized " << label << " vote bundle build failed with status "
                   << static_cast<uint32_t>(result.status) << ", error "
                   << static_cast<std::string>(result.error_code);
      return;
    }
    if (result.votes_bundle_rlp.empty() || result.vote_hashes.empty()) {
      return;
    }

    auto votes_bundle_rlp = bridge_bytes_to_dev_bytes(result.votes_bundle_rlp);
    dev::RLPStream packet(1);
    packet.appendRaw(votes_bundle_rlp);
    if (sealAndSend(peer->getId(), SubprotocolPacketType::kVotesBundlePacket, packet.invalidate())) {
      LOG(log_nf_) << "Send Rust optimized " << label << " votes bundle with " << result.vote_hashes.size()
                   << " votes to " << peer->getId();
      for (const auto& vote_hash : result.vote_hashes) {
        peer->markPbftVoteAsKnown(bridge_hash_to_vote_hash(vote_hash.hash));
      }
    }
  };

  auto send_planned_bundle = [this, &peer, &has_votes_to_plan, &build_request, &bridge_hash_to_vote_hash,
                              &send_optimized_bundle](const rustaxa::PbftOptimizedVoteBundlePlan& plan,
                                                      const char* label) {
    if (!has_votes_to_plan(plan)) {
      return;
    }

    auto request = build_request(plan);
    for (const auto& vote_hash : plan.vote_hashes) {
      if (peer->isPbftVoteKnown(bridge_hash_to_vote_hash(vote_hash.hash))) {
        continue;
      }

      request.vote_hashes.push_back(vote_hash);
      if (request.vote_hashes.size() == kMaxVotesInBundleRlp) {
        send_optimized_bundle(vote_mgr_->buildOptimizedVotesBundleEgress(std::move(request)), label);
        request = build_request(plan);
      }
    }

    if (!request.vote_hashes.empty()) {
      send_optimized_bundle(vote_mgr_->buildOptimizedVotesBundleEgress(std::move(request)), label);
    }
  };

  auto egress_plan = vote_mgr_->planNextVotesBundleEgress(pbft_period, pbft_round - 1);

  // TODO(rustaxa): replace this guarded network hook with a network shim once consensus egress packets are fully
  // routed from Rust-owned payload records. This keeps upstream C++ behavior intact in non-Rust builds.
  if (!has_votes_to_plan(egress_plan.next_votes) && !has_votes_to_plan(egress_plan.next_null_votes)) {
    const auto [tmp_pbft_round, tmp_pbft_period] = pbft_mgr_->getPbftRoundAndPeriod();
    if (pbft_period == tmp_pbft_period && pbft_round == tmp_pbft_round) {
      LOG(log_er_) << "No next votes returned for period " << tmp_pbft_period << ", round " << tmp_pbft_round - 1;
      return;
    }

    if (tmp_pbft_round == 1) {
      LOG(log_wr_) << "No next votes returned for period " << tmp_pbft_period << ", round " << tmp_pbft_round - 1
                   << " due to race condition - pbft already moved to the next period & round == 1";
      return;
    }

    egress_plan = vote_mgr_->planNextVotesBundleEgress(pbft_period, pbft_round - 1);
    if (!has_votes_to_plan(egress_plan.next_votes) && !has_votes_to_plan(egress_plan.next_null_votes)) {
      LOG(log_er_) << "No next votes returned for period " << tmp_pbft_period << ", round " << tmp_pbft_round - 1;
      return;
    }
  }

  send_planned_bundle(egress_plan.next_votes, "next");
  send_planned_bundle(egress_plan.next_null_votes, "next null");
  return;
#endif

  auto next_votes =
      vote_mgr_->getTwoTPlusOneVotedBlockVotes(pbft_period, pbft_round - 1, TwoTPlusOneVotedBlockType::NextVotedBlock);
  auto next_null_votes = vote_mgr_->getTwoTPlusOneVotedBlockVotes(pbft_period, pbft_round - 1,
                                                                  TwoTPlusOneVotedBlockType::NextVotedNullBlock);

  // In edge case this could theoretically happen due to race condition when we moved to the next period or round
  // right before calling getAllTwoTPlusOneNextVotes with specific period & round
  if (next_votes.empty() && next_null_votes.empty()) {
    // Try to get period & round values again
    const auto [tmp_pbft_round, tmp_pbft_period] = pbft_mgr_->getPbftRoundAndPeriod();
    // No changes in period & round or new round == 1
    if (pbft_period == tmp_pbft_period && pbft_round == tmp_pbft_round) {
      LOG(log_er_) << "No next votes returned for period " << tmp_pbft_period << ", round " << tmp_pbft_round - 1;
      return;
    }

    if (tmp_pbft_round == 1) {
      LOG(log_wr_) << "No next votes returned for period " << tmp_pbft_period << ", round " << tmp_pbft_round - 1
                   << " due to race condition - pbft already moved to the next period & round == 1";
      return;
    }

    next_votes = vote_mgr_->getTwoTPlusOneVotedBlockVotes(pbft_period, pbft_round - 1,
                                                          TwoTPlusOneVotedBlockType::NextVotedBlock);
    next_null_votes = vote_mgr_->getTwoTPlusOneVotedBlockVotes(pbft_period, pbft_round - 1,
                                                               TwoTPlusOneVotedBlockType::NextVotedNullBlock);
    if (next_votes.empty() && next_null_votes.empty()) {
      LOG(log_er_) << "No next votes returned for period " << tmp_pbft_period << ", round " << tmp_pbft_round - 1;
      return;
    }
  }

  if (!next_votes.empty()) {
    LOG(log_nf_) << "Send next votes bundle with " << next_votes.size() << " votes to " << peer->getId();
    sendPbftVotesBundle(peer, std::move(next_votes));
  }

  if (!next_null_votes.empty()) {
    LOG(log_nf_) << "Send next null votes bundle with " << next_null_votes.size() << " votes to " << peer->getId();
    sendPbftVotesBundle(peer, std::move(next_null_votes));
  }
}

}  // namespace taraxa::network::tarcap
