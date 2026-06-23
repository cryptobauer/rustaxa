#include "network/tarcap/packets_handlers/latest/get_next_votes_bundle_packet_handler.hpp"

#include <libdevcore/RLP.h>

#include <cassert>
#include <exception>
#include <stdexcept>

#include "pbft/pbft_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif
#include "vote_manager/vote_manager.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindPbftNextVotesBundleEgressRequest = 7;
constexpr uint32_t kNetworkPacketKindGetNextVotesSync = 2;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

vote_hash_t bridgeHashToVoteHash(const std::array<uint8_t, 32>& hash) {
  return vote_hash_t(hash.data(), vote_hash_t::ConstructFromPointer);
}

dev::bytes bridgeBytesToDevBytes(const rust::Vec<uint8_t>& bytes) {
  dev::bytes out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(byte);
  }
  return out;
}

rustaxa::PbftOptimizedVoteBundleBuildRequest buildOptimizedBundleRequest(
    const rustaxa::PbftOptimizedVoteBundlePlan& plan) {
  rustaxa::PbftOptimizedVoteBundleBuildRequest request;
  request.kind = plan.kind;
  request.block_hash = plan.block_hash;
  request.period = plan.period;
  request.round = plan.round;
  request.step = plan.step;
  return request;
}

bool hasVotesToPlan(const rustaxa::PbftOptimizedVoteBundlePlan& plan) {
  return plan.found && !plan.vote_hashes.empty();
}

std::array<uint8_t, 32> pbftNextVotesBundleEgressRequestKey(uint64_t period, uint64_t round,
                                                            uint64_t source_payload_id) {
  std::array<uint8_t, 32> key{};
  for (size_t i = 0; i < sizeof(uint64_t); ++i) {
    key[i] = static_cast<uint8_t>(period >> ((sizeof(uint64_t) - 1 - i) * 8));
    key[8 + i] = static_cast<uint8_t>(round >> ((sizeof(uint64_t) - 1 - i) * 8));
    key[16 + i] = static_cast<uint8_t>(source_payload_id >> ((sizeof(uint64_t) - 1 - i) * 8));
  }
  return key;
}

}  // namespace

struct GetNextVotesBundlePacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

GetNextVotesBundlePacketHandler::GetNextVotesBundlePacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, std::shared_ptr<PbftManager> pbft_mgr,
    std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<VoteManager> vote_mgr,
    std::shared_ptr<SlashingManager> slashing_manager, const addr_t& node_addr, const std::string& logs_prefix)
    : IVotePacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_mgr),
                         std::move(pbft_chain), std::move(vote_mgr), std::move(slashing_manager), node_addr,
                         logs_prefix + "GET_NEXT_VOTES_BUNDLE_PH") {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

GetNextVotesBundlePacketHandler::~GetNextVotesBundlePacketHandler() = default;

void GetNextVotesBundlePacketHandler::process(const threadpool::PacketData& packet_data,
                                              const std::shared_ptr<TaraxaPeer>& peer) {
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
  rustaxa::NetworkPbftNextVotesBundleEgressRequestEffects effects{};
  effects.peer_id = peer->getId().asArray();
  effects.period = pbft_period;
  effects.round = pbft_round - 1;
  effects.source_payload_id = 0;
  effects.request_bundle = true;
  (void)queuePbftNextVotesBundleEgressRequestEffects(effects);
  executePbftNextVotesBundleEgressEffect(peer);
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

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkIngressDecision GetNextVotesBundlePacketHandler::queuePbftNextVotesBundleEgressRequestEffects(
    const rustaxa::NetworkPbftNextVotesBundleEgressRequestEffects& effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_pbft_next_votes_bundle_egress_request_effects(
      effects);
}

void GetNextVotesBundlePacketHandler::executePbftNextVotesBundleEgressEffect(const std::shared_ptr<TaraxaPeer>& peer) {
  assert(rust_consensus_network_api_);
  constexpr uint8_t kPbftOptimizedBundleReady = 0;
  const auto batch = rust_consensus_network_api_->api->consensus_network_drain_work(1);
  rust::Vec<rustaxa::NetworkEffectResult> results;
  results.reserve(batch.effects.size());
  std::exception_ptr pending_exception;

  for (const auto& effect : batch.effects) {
    rustaxa::NetworkEffectResult result{};
    result.effect_id = effect.effect_id;
    result.kind = effect.kind;
    result.peer_id = effect.peer_id;
    result.packet_kind = effect.packet_kind;
    result.object_kind = effect.object_kind;
    result.object_hash = effect.object_hash;
    result.status = kNetworkEffectResultStatusOk;

    try {
      if (effect.kind != kNetworkEffectKindRecordConsensusObject ||
          effect.object_kind != kNetworkObjectKindPbftNextVotesBundleEgressRequest ||
          effect.packet_kind != kNetworkPacketKindGetNextVotesSync || effect.peer_id != peer->getId().asArray() ||
          effect.object_hash !=
              pbftNextVotesBundleEgressRequestKey(effect.period, effect.round, effect.source_payload_id)) {
        throw std::runtime_error("Network API PBFT next-votes egress effect missing matching request");
      }

      auto send_optimized_bundle = [this, &peer](rustaxa::PbftOptimizedVoteBundleBuildResult result,
                                                 const char* label) {
        if (result.status != kPbftOptimizedBundleReady) {
          LOG(log_er_) << "Rust optimized " << label << " vote bundle build failed with status "
                       << static_cast<uint32_t>(result.status) << ", error "
                       << static_cast<std::string>(result.error_code);
          return false;
        }
        if (result.votes_bundle_rlp.empty() || result.vote_hashes.empty()) {
          return true;
        }

        auto votes_bundle_rlp = bridgeBytesToDevBytes(result.votes_bundle_rlp);
        dev::RLPStream packet(1);
        packet.appendRaw(votes_bundle_rlp);
        if (sealAndSend(peer->getId(), SubprotocolPacketType::kVotesBundlePacket, packet.invalidate())) {
          LOG(log_nf_) << "Send Rust optimized " << label << " votes bundle with " << result.vote_hashes.size()
                       << " votes to " << peer->getId();
          for (const auto& vote_hash : result.vote_hashes) {
            peer->markPbftVoteAsKnown(bridgeHashToVoteHash(vote_hash.hash));
          }
        } else {
          return false;
        }
        return true;
      };

      bool send_failed = false;
      auto record_send_result = [&send_failed](bool sent) {
        if (!sent) {
          send_failed = true;
        }
      };

      auto send_planned_bundle = [this, &peer, &send_optimized_bundle](const rustaxa::PbftOptimizedVoteBundlePlan& plan,
                                                                       const char* label) {
        bool ok = true;
        if (!hasVotesToPlan(plan)) {
          return ok;
        }

        auto request = buildOptimizedBundleRequest(plan);
        for (const auto& vote_hash : plan.vote_hashes) {
          if (peer->isPbftVoteKnown(bridgeHashToVoteHash(vote_hash.hash))) {
            continue;
          }

          request.vote_hashes.push_back(vote_hash);
          if (request.vote_hashes.size() == kMaxVotesInBundleRlp) {
            ok = send_optimized_bundle(vote_mgr_->buildOptimizedVotesBundleEgress(std::move(request)), label) && ok;
            request = buildOptimizedBundleRequest(plan);
          }
        }

        if (!request.vote_hashes.empty()) {
          ok = send_optimized_bundle(vote_mgr_->buildOptimizedVotesBundleEgress(std::move(request)), label) && ok;
        }
        return ok;
      };

      auto egress_plan = vote_mgr_->planNextVotesBundleEgress(effect.period, effect.round);
      if (!hasVotesToPlan(egress_plan.next_votes) && !hasVotesToPlan(egress_plan.next_null_votes)) {
        const auto [tmp_pbft_round, tmp_pbft_period] = pbft_mgr_->getPbftRoundAndPeriod();
        if (effect.period == tmp_pbft_period && effect.round + 1 == tmp_pbft_round) {
          LOG(log_er_) << "No next votes returned for period " << tmp_pbft_period << ", round " << tmp_pbft_round - 1;
          throw std::runtime_error("Network API PBFT next-votes egress found no matching votes");
        }

        if (tmp_pbft_round == 1) {
          LOG(log_wr_) << "No next votes returned for period " << tmp_pbft_period << ", round " << tmp_pbft_round - 1
                       << " due to race condition - pbft already moved to the next period & round == 1";
          throw std::runtime_error("Network API PBFT next-votes egress request became stale");
        }

        egress_plan = vote_mgr_->planNextVotesBundleEgress(effect.period, effect.round);
        if (!hasVotesToPlan(egress_plan.next_votes) && !hasVotesToPlan(egress_plan.next_null_votes)) {
          LOG(log_er_) << "No next votes returned for period " << tmp_pbft_period << ", round " << tmp_pbft_round - 1;
          throw std::runtime_error("Network API PBFT next-votes egress found no matching votes after retry");
        }
      }

      record_send_result(send_planned_bundle(egress_plan.next_votes, "next"));
      record_send_result(send_planned_bundle(egress_plan.next_null_votes, "next null"));
      if (send_failed) {
        throw std::runtime_error("Network API PBFT next-votes egress failed to build or send a bundle");
      }
    } catch (const std::exception& e) {
      result.status = kNetworkEffectResultStatusFailed;
      result.diagnostic = e.what();
      pending_exception = std::current_exception();
    }

    results.push_back(std::move(result));
  }

  if (!results.empty()) {
    (void)rust_consensus_network_api_->api->consensus_network_report_effect_results(std::move(results));
  }

  if (pending_exception) {
    std::rethrow_exception(pending_exception);
  }
}
#endif

}  // namespace taraxa::network::tarcap
