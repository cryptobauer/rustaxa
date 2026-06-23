#include "network/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.hpp"

#include <array>
#include <cassert>
#include <exception>
#include <stdexcept>

#include "network/tarcap/packets/latest/pbft_blocks_bundle_packet.hpp"
#include "network/tarcap/packets/latest/pbft_sync_packet.hpp"
#include "network/tarcap/packets_handlers/latest/pbft_blocks_bundle_packet_handler.hpp"
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#include "pbft/pbft_chain.hpp"
#include "pbft/pbft_manager.hpp"
#ifndef RUSTAXA_ENABLE
#include "storage/storage.hpp"
#endif
#include "vote/pbft_vote.hpp"
#include "vote/votes_bundle_rlp.hpp"
#include "vote_manager/vote_manager.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindPbftSyncEgressRequest = 8;
constexpr uint32_t kNetworkPacketKindGetPbftSync = 10;
constexpr uint8_t kNetworkSyncKindPbftChain = 0;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

std::array<uint8_t, 32> pbftSyncEgressRequestKey(uint64_t from_period, uint64_t blocks_to_transfer,
                                                 uint64_t source_payload_id) {
  std::array<uint8_t, 32> key{};
  for (size_t i = 0; i < sizeof(uint64_t); ++i) {
    key[i] = static_cast<uint8_t>(from_period >> ((sizeof(uint64_t) - 1 - i) * 8));
    key[8 + i] = static_cast<uint8_t>(blocks_to_transfer >> ((sizeof(uint64_t) - 1 - i) * 8));
    key[16 + i] = static_cast<uint8_t>(source_payload_id >> ((sizeof(uint64_t) - 1 - i) * 8));
  }
  return key;
}

}  // namespace

struct GetPbftSyncPacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

GetPbftSyncPacketHandler::GetPbftSyncPacketHandler(
    const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, std::shared_ptr<PbftSyncingState> pbft_syncing_state,
    std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<VoteManager> vote_mgr,
#ifndef RUSTAXA_ENABLE
    std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY:
                                    // legacy sync egress.
#endif
    const addr_t &node_addr, const std::string &logs_prefix)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                    logs_prefix + "GET_PBFT_SYNC_PH"),
      pbft_syncing_state_(std::move(pbft_syncing_state)),
      pbft_mgr_(std::move(pbft_mgr)),
      pbft_chain_(std::move(pbft_chain)),
      vote_mgr_(std::move(vote_mgr))
#ifndef RUSTAXA_ENABLE
      ,
      db_(std::move(db))
#endif
{
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

GetPbftSyncPacketHandler::~GetPbftSyncPacketHandler() = default;

void GetPbftSyncPacketHandler::process(const threadpool::PacketData &packet_data,
                                       const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<GetPbftSyncPacket>(packet_data.rlp_);

  LOG(log_tr_) << "Received GetPbftSyncPacket Block";

  // Here need PBFT chain size, not synced period since synced blocks has not verified yet.
  const size_t my_chain_size = pbft_chain_->getPbftChainSize();
  if (packet.height_to_sync > my_chain_size) {
    // Node update peers PBFT chain size in status packet. Should not request syncing period bigger than pbft chain size
    std::ostringstream err_msg;
    err_msg << "Peer " << peer->getId() << " request syncing period start at " << packet.height_to_sync
            << ". That's bigger than own PBFT chain size " << my_chain_size;
    throw MaliciousPeerException(err_msg.str());
  }

  if (kConf.is_light_node && packet.height_to_sync + kConf.light_node_history <= my_chain_size) {
    std::ostringstream err_msg;
    err_msg << "Peer " << peer->getId() << " request syncing period start at " << packet.height_to_sync
            << ". Light node does not have the data " << my_chain_size;
    throw MaliciousPeerException(err_msg.str());
  }

  size_t blocks_to_transfer = 0;
  auto pbft_chain_synced = false;
  const auto total_period_data_size = my_chain_size - packet.height_to_sync + 1;
  if (total_period_data_size <= kConf.network.sync_level_size) {
    blocks_to_transfer = total_period_data_size;
    pbft_chain_synced = true;
  } else {
    blocks_to_transfer = kConf.network.sync_level_size;
  }
  LOG(log_tr_) << "Will send " << blocks_to_transfer << " PBFT blocks to " << peer->getId();

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkPbftSyncEgressRequestEffects effects{};
  effects.peer_id = peer->getId().asArray();
  effects.from_period = packet.height_to_sync;
  effects.blocks_to_transfer = blocks_to_transfer;
  effects.pbft_chain_synced = pbft_chain_synced;
  effects.source_payload_id = packet_data.id_;
  effects.request_sync = true;
  (void)queuePbftSyncEgressRequestEffects(effects);
  executePbftSyncEgressEffect(peer);
  return;
#endif

  sendPbftBlocks(peer, packet.height_to_sync, blocks_to_transfer, pbft_chain_synced);

  // Send current proposed blocks after the last sync packet
  if (pbft_chain_synced) {
    PbftBlocksBundlePacket proposed_blocks_packet;

    for (auto &&period_proposed_blocks : pbft_mgr_->getProposedBlocks()) {
      for (auto &&proposed_block : period_proposed_blocks.second) {
        proposed_blocks_packet.pbft_blocks.push_back(std::move(proposed_block));

        // Send max kMaxBlocksInPacket(10) blocks in a single packet
        if (proposed_blocks_packet.pbft_blocks.size() == PbftBlocksBundlePacketHandler::kMaxBlocksInPacket) {
          sealAndSend(peer->getId(), SubprotocolPacketType::kPbftBlocksBundlePacket,
                      encodePacketRlp(proposed_blocks_packet));
          proposed_blocks_packet.pbft_blocks.clear();
        }
      }
    }

    if (!proposed_blocks_packet.pbft_blocks.empty()) {
      sealAndSend(peer->getId(), SubprotocolPacketType::kPbftBlocksBundlePacket,
                  encodePacketRlp(proposed_blocks_packet));
    }
  }
}

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkIngressDecision GetPbftSyncPacketHandler::queuePbftSyncEgressRequestEffects(
    const rustaxa::NetworkPbftSyncEgressRequestEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_pbft_sync_egress_request_effects(effects);
}

void GetPbftSyncPacketHandler::executePbftSyncEgressEffect(const std::shared_ptr<TaraxaPeer> &peer) {
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
      const auto blocks_to_transfer = static_cast<size_t>(effect.dependency_id);
      if (effect.kind != kNetworkEffectKindRecordConsensusObject ||
          effect.object_kind != kNetworkObjectKindPbftSyncEgressRequest ||
          effect.packet_kind != kNetworkPacketKindGetPbftSync || effect.sync_kind != kNetworkSyncKindPbftChain ||
          effect.peer_id != peer->getId().asArray() || effect.sync_start != effect.period ||
          effect.object_hash !=
              pbftSyncEgressRequestKey(effect.period, effect.dependency_id, effect.source_payload_id)) {
        throw std::runtime_error("Network API PBFT sync egress effect missing matching request");
      }

      sendPbftBlocks(peer, effect.period, blocks_to_transfer, effect.reason_code != 0);

      if (effect.reason_code != 0) {
        PbftBlocksBundlePacket proposed_blocks_packet;

        for (auto &&period_proposed_blocks : pbft_mgr_->getProposedBlocks()) {
          for (auto &&proposed_block : period_proposed_blocks.second) {
            proposed_blocks_packet.pbft_blocks.push_back(std::move(proposed_block));

            if (proposed_blocks_packet.pbft_blocks.size() == PbftBlocksBundlePacketHandler::kMaxBlocksInPacket) {
              sealAndSend(peer->getId(), SubprotocolPacketType::kPbftBlocksBundlePacket,
                          encodePacketRlp(proposed_blocks_packet));
              proposed_blocks_packet.pbft_blocks.clear();
            }
          }
        }

        if (!proposed_blocks_packet.pbft_blocks.empty()) {
          sealAndSend(peer->getId(), SubprotocolPacketType::kPbftBlocksBundlePacket,
                      encodePacketRlp(proposed_blocks_packet));
        }
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

// api for pbft syncing
void GetPbftSyncPacketHandler::sendPbftBlocks(const std::shared_ptr<TaraxaPeer> &peer, PbftPeriod from_period,
                                              size_t blocks_to_transfer, bool pbft_chain_synced) {
  const auto &peer_id = peer->getId();
  LOG(log_tr_) << "sendPbftBlocks: peer want to sync from pbft chain height " << from_period << ", will send at most "
               << blocks_to_transfer << " pbft blocks to " << peer_id;

  for (auto block_period = from_period; block_period < from_period + blocks_to_transfer; block_period++) {
    bool last_block = (block_period == from_period + blocks_to_transfer - 1);
#ifndef RUSTAXA_ENABLE
    auto period_data = db_->getPeriodDataRaw(block_period);  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy tarcap sync
                                                             // reads remain outside Rust-enabled production routing.
#else
    std::vector<std::shared_ptr<PbftVote> > reward_votes;
    if (pbft_chain_synced && last_block) {
      reward_votes = vote_mgr_->getRewardVotes();
      assert(!reward_votes.empty());
    }
    const auto reward_votes_present = !reward_votes.empty();
    const auto reward_votes_period = reward_votes_present ? reward_votes[0]->getPeriod() : PbftPeriod{0};
    auto sync_payload = pbft_mgr_->getPbftSyncEgressPayload(block_period, last_block, pbft_chain_synced,
                                                            reward_votes_present, reward_votes_period);
    auto period_data = std::move(sync_payload.period_data_rlp);
#endif
    if (period_data.empty()) {
      // This can happen when switching from light node to full node setting
      LOG(log_er_) << "DB corrupted. Cannot find period " << block_period << " PBFT block in db";
      return;
    }

    std::shared_ptr<PbftSyncPacketRaw> pbft_sync_packet;

    if (pbft_chain_synced && last_block) {
#ifndef RUSTAXA_ENABLE
      // Latest finalized block cert votes are saved in db as reward votes for new blocks
      auto reward_votes = vote_mgr_->getRewardVotes();
      assert(!reward_votes.empty());
      // It is possible that the node pushed another block to the chain in the meantime
      if (reward_votes[0]->getPeriod() == block_period) {
        pbft_sync_packet = std::make_shared<PbftSyncPacketRaw>(last_block, std::move(period_data),
                                                               OptimizedPbftVotesBundle{std::move(reward_votes)});
      } else {
        pbft_sync_packet = std::make_shared<PbftSyncPacketRaw>(last_block, std::move(period_data));
      }
#else
      if (sync_payload.attach_reward_votes) {
        pbft_sync_packet = std::make_shared<PbftSyncPacketRaw>(last_block, std::move(period_data),
                                                               OptimizedPbftVotesBundle{std::move(reward_votes)});
      } else {
        pbft_sync_packet = std::make_shared<PbftSyncPacketRaw>(last_block, std::move(period_data));
      }
#endif
    } else {
      pbft_sync_packet = std::make_shared<PbftSyncPacketRaw>(last_block, std::move(period_data));
    }

    LOG(log_dg_) << "Sending PbftSyncPacket period " << block_period << " to " << peer_id;
    sealAndSend(peer_id, SubprotocolPacketType::kPbftSyncPacket, encodePacketRlp(pbft_sync_packet));
    if (pbft_chain_synced && last_block) {
      peer->syncing_ = false;
    }
  }
}

}  // namespace taraxa::network::tarcap
