#include "network/tarcap/packets_handlers/latest/get_pillar_votes_bundle_packet_handler.hpp"

#include <array>
#include <cassert>
#include <exception>
#include <stdexcept>

#include "network/tarcap/packets/latest/pillar_votes_bundle_packet.hpp"
#include "network/tarcap/packets_handlers/latest/pillar_votes_bundle_packet_handler.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindPillarVotesBundleEgressRequest = 9;
constexpr uint32_t kNetworkPacketKindGetPillarVotesBundle = 14;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

blk_hash_t bridgeHashToBlkHash(const std::array<uint8_t, 32> &hash) {
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

}  // namespace

struct GetPillarVotesBundlePacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

GetPillarVotesBundlePacketHandler::GetPillarVotesBundlePacketHandler(
    const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_manager, const addr_t &node_addr,
    const std::string &logs_prefix)
    : IGetPillarVotesBundlePacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                                         logs_prefix + "GET_PILLAR_VOTES_BUNDLE_PH"),
      pillar_chain_manager_(std::move(pillar_chain_manager)) {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

GetPillarVotesBundlePacketHandler::~GetPillarVotesBundlePacketHandler() = default;

void GetPillarVotesBundlePacketHandler::process(const threadpool::PacketData &packet_data,
                                                const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<GetPillarVotesBundlePacket>(packet_data.rlp_);

  LOG(log_dg_) << "GetPillarVotesBundlePacketHandler received from peer " << peer->getId();

  if (!kConf.genesis.state.hardforks.ficus_hf.isFicusHardfork(packet.period)) {
    std::ostringstream err_msg;
    err_msg << "Pillar votes bundle request for period " << packet.period << ", ficus hardfork block num "
            << kConf.genesis.state.hardforks.ficus_hf.block_num;
    throw MaliciousPeerException(err_msg.str());
  }

  if (!kConf.genesis.state.hardforks.ficus_hf.isPbftWithPillarBlockPeriod(packet.period)) {
    std::ostringstream err_msg;
    err_msg << "Pillar votes bundle request for period " << packet.period << ". Wrong requested period";
    throw MaliciousPeerException(err_msg.str());
  }

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkPillarVotesBundleEgressRequestEffects effects{};
  effects.peer_id = peer->getId().asArray();
  effects.period = packet.period;
  effects.pillar_block_hash = packet.pillar_block_hash.asArray();
  effects.source_payload_id = packet_data.id_;
  effects.request_bundle = true;
  (void)queuePillarVotesBundleEgressRequestEffects(effects);
  executePillarVotesBundleEgressEffect(peer);
  return;
#endif

  const auto votes = pillar_chain_manager_->getVerifiedPillarVotes(packet.period, packet.pillar_block_hash);
  if (votes.empty()) {
    LOG(log_dg_) << "No pillar votes for period " << packet.period << "and pillar block hash "
                 << packet.pillar_block_hash;
    return;
  }
  // Check if the votes size exceeds the maximum limit and split into multiple packets if needed
  const size_t total_votes = votes.size();
  size_t votes_sent = 0;

  while (votes_sent < total_votes) {
    // Determine the size of the current chunk
    const size_t chunk_size =
        std::min(PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp, total_votes - votes_sent);

    // Create PillarVotesBundlePacket
    std::vector<std::shared_ptr<PillarVote>> pillar_votes;
    pillar_votes.reserve(chunk_size);
    for (size_t i = 0; i < chunk_size; ++i) {
      pillar_votes.emplace_back(votes[votes_sent + i]);
    }
    PillarVotesBundlePacket pillar_votes_bundle_packet(OptimizedPillarVotesBundle{std::move(pillar_votes)});

    // Seal and send the chunk to the peer
    if (sealAndSend(peer->getId(), SubprotocolPacketType::kPillarVotesBundlePacket,
                    encodePacketRlp(pillar_votes_bundle_packet))) {
      // Mark the votes in this chunk as known
      for (size_t i = 0; i < chunk_size; ++i) {
        peer->markPillarVoteAsKnown(votes[votes_sent + i]->getHash());
      }

      LOG(log_nf_) << "Pillar votes bundle for period " << packet.period << ", hash " << packet.pillar_block_hash
                   << " sent to " << peer->getId() << " (Chunk "
                   << (votes_sent / PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp) + 1 << "/"
                   << (total_votes + PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp - 1) /
                          PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp
                   << ")";
    }

    // Update the votes_sent counter
    votes_sent += chunk_size;
  }
}

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkIngressDecision GetPillarVotesBundlePacketHandler::queuePillarVotesBundleEgressRequestEffects(
    const rustaxa::NetworkPillarVotesBundleEgressRequestEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_pillar_votes_bundle_egress_request_effects(effects);
}

void GetPillarVotesBundlePacketHandler::executePillarVotesBundleEgressEffect(const std::shared_ptr<TaraxaPeer> &peer) {
  assert(rust_consensus_network_api_);
  const auto batch = rust_consensus_network_api_->api->consensus_network_drain_work(1);
  rust::Vec<rustaxa::NetworkEffectResult> results;
  results.reserve(batch.effects.size());
  std::exception_ptr pending_exception;

  for (const auto &effect : batch.effects) {
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
          effect.object_kind != kNetworkObjectKindPillarVotesBundleEgressRequest ||
          effect.packet_kind != kNetworkPacketKindGetPillarVotesBundle || effect.peer_id != peer->getId().asArray()) {
        throw std::runtime_error("Network API pillar votes bundle egress effect missing matching request");
      }

      const auto pillar_block_hash = bridgeHashToBlkHash(effect.object_hash);
      const auto votes = pillar_chain_manager_->getVerifiedPillarVotes(effect.period, pillar_block_hash);
      if (votes.empty()) {
        LOG(log_dg_) << "No pillar votes for period " << effect.period << "and pillar block hash " << pillar_block_hash;
        results.push_back(std::move(result));
        continue;
      }

      const size_t total_votes = votes.size();
      size_t votes_sent = 0;
      bool send_failed = false;

      while (votes_sent < total_votes) {
        const size_t chunk_size =
            std::min(PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp, total_votes - votes_sent);

        std::vector<std::shared_ptr<PillarVote>> pillar_votes;
        pillar_votes.reserve(chunk_size);
        for (size_t i = 0; i < chunk_size; ++i) {
          pillar_votes.emplace_back(votes[votes_sent + i]);
        }
        PillarVotesBundlePacket pillar_votes_bundle_packet(OptimizedPillarVotesBundle{std::move(pillar_votes)});

        if (sealAndSend(peer->getId(), SubprotocolPacketType::kPillarVotesBundlePacket,
                        encodePacketRlp(pillar_votes_bundle_packet))) {
          for (size_t i = 0; i < chunk_size; ++i) {
            peer->markPillarVoteAsKnown(votes[votes_sent + i]->getHash());
          }

          LOG(log_nf_) << "Pillar votes bundle for period " << effect.period << ", hash " << pillar_block_hash
                       << " sent to " << peer->getId() << " (Chunk "
                       << (votes_sent / PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp) + 1 << "/"
                       << (total_votes + PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp - 1) /
                              PillarVotesBundlePacketHandler::kMaxPillarVotesInBundleRlp
                       << ")";
        } else {
          send_failed = true;
        }

        votes_sent += chunk_size;
      }

      if (send_failed) {
        result.status = kNetworkEffectResultStatusFailed;
      }
    } catch (...) {
      result.status = kNetworkEffectResultStatusFailed;
      if (!pending_exception) {
        pending_exception = std::current_exception();
      }
    }

    results.push_back(std::move(result));
  }

  (void)rust_consensus_network_api_->api->consensus_network_report_effect_results(std::move(results));
  if (pending_exception) {
    std::rethrow_exception(pending_exception);
  }
}
#endif

void GetPillarVotesBundlePacketHandler::requestPillarVotesBundle(PbftPeriod period, const blk_hash_t &pillar_block_hash,
                                                                 const std::shared_ptr<TaraxaPeer> &peer) {
  if (sealAndSend(peer->getId(), SubprotocolPacketType::kGetPillarVotesBundlePacket,
                  encodePacketRlp(GetPillarVotesBundlePacket(period, pillar_block_hash)))) {
    LOG(log_nf_) << "Requested pillar votes bundle for period " << period << " and pillar block " << pillar_block_hash
                 << " from peer " << peer->getId();
  } else {
    LOG(log_er_) << "Unable to send pillar votes bundle request for period " << period << " and pillar block "
                 << pillar_block_hash << " to peer " << peer->getId();
  }
}

}  // namespace taraxa::network::tarcap
