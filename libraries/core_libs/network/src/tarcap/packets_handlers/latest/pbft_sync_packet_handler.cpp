#include "network/tarcap/packets_handlers/latest/pbft_sync_packet_handler.hpp"

#include <cassert>
#include <exception>
#include <stdexcept>

#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#include "pbft/pbft_chain.hpp"
#include "pbft/pbft_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif
#include "transaction/transaction_manager.hpp"
#include "vote/pbft_vote.hpp"
#include "vote/votes_bundle_rlp.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindPbftPeriodData = 4;
constexpr uint32_t kNetworkPacketKindPbftSync = 11;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

rust::Vec<uint8_t> toBridgeBytes(const bytes &input) {
  rust::Vec<uint8_t> output;
  output.reserve(input.size());
  for (const auto byte : input) {
    output.push_back(static_cast<uint8_t>(byte));
  }
  return output;
}

}  // namespace

struct PbftSyncPacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

PbftSyncPacketHandler::PbftSyncPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                             std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                             std::shared_ptr<PbftSyncingState> pbft_syncing_state,
                                             std::shared_ptr<PbftChain> pbft_chain,
                                             std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<DagManager> dag_mgr,
                                             std::shared_ptr<VoteManager> vote_mgr,
#ifndef RUSTAXA_ENABLE
                                             std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY:
                                                                             // legacy PBFT sync handler.
#endif
                                             const addr_t &node_addr, const std::string &logs_prefix)
    : ISyncPacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_syncing_state),
                         std::move(pbft_chain), std::move(pbft_mgr), std::move(dag_mgr),
#ifndef RUSTAXA_ENABLE
                         std::move(db),
#endif
                         node_addr, logs_prefix + "PBFT_SYNC_PH"),
      vote_mgr_(std::move(vote_mgr)),
      periodic_events_tp_(1, true) {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

PbftSyncPacketHandler::~PbftSyncPacketHandler() = default;

void PbftSyncPacketHandler::process(const threadpool::PacketData &packet_data,
                                    const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<PbftSyncPacket>(packet_data.rlp_);

  // Note: no need to consider possible race conditions due to concurrent processing as it is
  // disabled on priority_queue blocking dependencies level
  const auto syncing_peer = pbft_syncing_state_->syncingPeer();
  if (!syncing_peer) {
    LOG(log_wr_) << "PbftSyncPacket received from unexpected peer " << peer->getId().abridged()
                 << " but there is no current syncing peer set";
    return;
  }

  if (syncing_peer->getId() != peer->getId()) {
    LOG(log_wr_) << "PbftSyncPacket received from unexpected peer " << peer->getId().abridged()
                 << " current syncing peer " << syncing_peer->getId().abridged();
    return;
  }

  // Process received pbft blocks
  // pbft_chain_synced is the flag to indicate own PBFT chain has synced with the peer's PBFT chain
  const bool pbft_chain_synced = packet.current_block_cert_votes_bundle.has_value();
  const auto pbft_blk_hash = packet.period_data.pbft_blk->getBlockHash();

  std::string received_dag_blocks_str;  // This is just log related stuff
  for (auto const &block : packet.period_data.dag_blocks) {
    received_dag_blocks_str += block->getHash().toString() + " ";
    if (peer->dag_level_ < block->getLevel()) {
      peer->dag_level_ = block->getLevel();
    }
  }

  const auto pbft_block_period = packet.period_data.pbft_blk->getPeriod();
  LOG(log_dg_) << "PbftSyncPacket received. Period: " << pbft_block_period
               << ", dag Blocks: " << received_dag_blocks_str << " from " << peer->getId();

  peer->markPbftBlockAsKnown(pbft_blk_hash);
  // Update peer's pbft period if outdated
  if (peer->pbft_chain_size_ < pbft_block_period) {
    peer->pbft_chain_size_ = pbft_block_period;
  }

  LOG(log_tr_) << "Processing pbft block: " << pbft_blk_hash;

  if (pbft_chain_->findPbftBlockInChain(pbft_blk_hash)) {
    LOG(log_wr_) << "PBFT block " << pbft_blk_hash << ", period: " << packet.period_data.pbft_blk->getPeriod()
                 << " from " << peer->getId() << " already present in chain";
  } else {
    if (pbft_block_period != pbft_mgr_->pbftSyncingPeriod() + 1) {
      // This can happen if we just got synced and block was cert voted
      if (pbft_chain_synced && pbft_block_period == pbft_mgr_->pbftSyncingPeriod()) {
        pbftSyncComplete();
        return;
      }

      LOG(log_er_) << "Block " << pbft_blk_hash << " period unexpected: " << pbft_block_period
                   << ". Expected period: " << pbft_mgr_->pbftSyncingPeriod() + 1;
      return;
    }

    // Check cert vote matches if final synced block
    if (pbft_chain_synced) {
      for (auto const &vote : packet.current_block_cert_votes_bundle->votes) {
        if (vote->getBlockHash() != pbft_blk_hash) {
          LOG(log_er_) << "Invalid cert votes block hash " << vote->getBlockHash() << " instead of " << pbft_blk_hash
                       << " from peer " << peer->getId().abridged() << " received, stop syncing.";
          peers_state_->handleMaliciousSyncPeer(peer->getId());
          return;
        }
      }
    }

    // Check votes match the hash of previous block in the queue
    auto last_pbft_block_hash = pbft_mgr_->lastPbftBlockHashFromQueueOrChain();
    // Check cert vote matches
    for (auto const &vote : packet.period_data.previous_block_cert_votes) {
      if (vote->getBlockHash() != last_pbft_block_hash) {
        LOG(log_er_) << "Invalid cert votes block hash " << vote->getBlockHash() << " instead of "
                     << last_pbft_block_hash << " from peer " << peer->getId().abridged() << " received, stop syncing.";
        peers_state_->handleMaliciousSyncPeer(peer->getId());
        return;
      }
    }

    if (!pbft_mgr_->validatePillarDataInPeriodData(packet.period_data)) {
      peers_state_->handleMaliciousSyncPeer(peer->getId());
      return;
    }

    auto order_hash = PbftManager::calculateOrderHash(packet.period_data.dag_blocks);
    if (order_hash != packet.period_data.pbft_blk->getOrderHash()) {
      {  // This is just log related stuff
        std::vector<trx_hash_t> trx_order;
        trx_order.reserve(packet.period_data.transactions.size());
        std::vector<blk_hash_t> blk_order;
        blk_order.reserve(packet.period_data.dag_blocks.size());
        for (auto t : packet.period_data.transactions) {
          trx_order.push_back(t->getHash());
        }
        for (auto b : packet.period_data.dag_blocks) {
          blk_order.push_back(b->getHash());
        }
        LOG(log_er_) << "Order hash incorrect in period data " << pbft_blk_hash << " expected: " << order_hash
                     << " received " << packet.period_data.pbft_blk->getOrderHash() << "; Dag order: " << blk_order
                     << "; Trx order: " << trx_order << "; from " << peer->getId().abridged() << ", stop syncing.";
      }
      peers_state_->handleMaliciousSyncPeer(peer->getId());
      return;
    }

    // This is special case when queue is empty and we can not say for sure that all votes that are part of this block
    // have been verified before
    if (pbft_mgr_->periodDataQueueEmpty()) {
      for (const auto &v : packet.period_data.previous_block_cert_votes) {
        if (auto vote_is_valid = vote_mgr_->validateVote(v); vote_is_valid.first == false) {
          LOG(log_er_) << "Invalid reward votes in block " << packet.period_data.pbft_blk->getBlockHash()
                       << " from peer " << peer->getId().abridged()
                       << " received, stop syncing. Validation failed. Err: " << vote_is_valid.second;
          peers_state_->handleMaliciousSyncPeer(peer->getId());
          return;
        }

        vote_mgr_->addVerifiedVote(v);
      }

      // And now we need to replace it with verified votes
      if (auto votes = vote_mgr_->checkRewardVotes(packet.period_data.pbft_blk, true); votes.first) {
        packet.period_data.previous_block_cert_votes = std::move(votes.second);
      } else {
        // checkRewardVotes could fail because we just cert voted this block and moved to next period,
        // in that case we are probably fully synced
        if (pbft_block_period <= vote_mgr_->getRewardVotesPbftBlockPeriod()) {
          pbft_syncing_state_->setPbftSyncing(false);
          return;
        }

        LOG(log_er_) << "Invalid reward votes in block " << packet.period_data.pbft_blk->getBlockHash() << " from peer "
                     << peer->getId().abridged() << " received, stop syncing.";
        peers_state_->handleMaliciousSyncPeer(peer->getId());
        return;
      }
    }

    LOG(log_tr_) << "Synced PBFT block hash " << pbft_blk_hash << " with "
                 << packet.period_data.previous_block_cert_votes.size() << " cert votes";
    LOG(log_tr_) << "Synced PBFT block " << packet.period_data;
    std::vector<std::shared_ptr<PbftVote>> current_block_cert_votes;
    if (pbft_chain_synced) {
      current_block_cert_votes = std::move(packet.current_block_cert_votes_bundle->votes);
    }
#ifdef RUSTAXA_ENABLE
    const auto period_data_rlp = packet.period_data.rlp();
    rustaxa::NetworkPbftSyncPeriodDataAdmissionRequestEffects effects{};
    effects.peer_id = peer->getId().asArray();
    effects.block_hash = pbft_blk_hash.asArray();
    effects.period = pbft_block_period;
    effects.period_data_rlp = toBridgeBytes(period_data_rlp);
    effects.current_block_cert_vote_count = current_block_cert_votes.size();
    effects.source_payload_id = 0;
    effects.admit_period_data = true;
    (void)queuePbftSyncPeriodDataAdmissionRequestEffects(effects);
    executePbftSyncPeriodDataAdmissionEffect(packet.period_data, period_data_rlp, peer, current_block_cert_votes);
#else
    pbft_mgr_->periodDataQueuePush(std::move(packet.period_data), peer->getId(), std::move(current_block_cert_votes));
#endif
  }

  auto pbft_sync_period = pbft_mgr_->pbftSyncingPeriod();

  // Reset last sync packet received time
  pbft_syncing_state_->setLastSyncPacketTime();

  if (pbft_chain_synced) {
    pbftSyncComplete();
    return;
  }

  if (packet.last_block) {
    // If current sync period is actually bigger than the block we just received we are probably synced
    if (pbft_sync_period > pbft_block_period) {
      pbft_syncing_state_->setPbftSyncing(false);
      return;
    }
    if (pbft_syncing_state_->isPbftSyncing()) {
      if (pbft_sync_period > pbft_chain_->getPbftChainSize() + (10 * kConf.network.sync_level_size)) {
        LOG(log_tr_) << "Syncing pbft blocks too fast than processing. Has synced period " << pbft_sync_period
                     << ", PBFT chain size " << pbft_chain_->getPbftChainSize();
        periodic_events_tp_.post(kDelayedPbftSyncDelayMs, [this] { delayedPbftSync(1); });
      } else {
        if (!syncPeerPbft(pbft_sync_period + 1)) {
          pbft_syncing_state_->setPbftSyncing(false);
          return;
        }
      }
    }
  }
}

PeriodData PbftSyncPacketHandler::decodePeriodData(const dev::RLP &period_data_rlp) const {
  return PeriodData(period_data_rlp);
}

std::vector<std::shared_ptr<PbftVote>> PbftSyncPacketHandler::decodeVotesBundle(
    const dev::RLP &votes_bundle_rlp) const {
  return decodePbftVotesBundleRlp(votes_bundle_rlp);
}

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkIngressDecision PbftSyncPacketHandler::queuePbftSyncPeriodDataAdmissionRequestEffects(
    const rustaxa::NetworkPbftSyncPeriodDataAdmissionRequestEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_pbft_sync_period_data_admission_request_effects(
      effects);
}

void PbftSyncPacketHandler::executePbftSyncPeriodDataAdmissionEffect(
    PeriodData &period_data, const dev::bytes &period_data_rlp, const std::shared_ptr<TaraxaPeer> &peer,
    std::vector<std::shared_ptr<PbftVote>> &current_block_cert_votes) {
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
      const auto effect_payload = bytes(effect.payload_bytes.begin(), effect.payload_bytes.end());
      if (effect.kind != kNetworkEffectKindRecordConsensusObject ||
          effect.object_kind != kNetworkObjectKindPbftPeriodData || effect.packet_kind != kNetworkPacketKindPbftSync ||
          effect.peer_id != peer->getId().asArray() ||
          period_data.pbft_blk->getBlockHash().asArray() != effect.object_hash || period_data.rlp() != effect_payload ||
          period_data_rlp != effect_payload || effect.period != period_data.pbft_blk->getPeriod() ||
          effect.dependency_id != current_block_cert_votes.size()) {
        throw std::runtime_error(
            "Network API PBFT sync period-data admission effect missing matching live period data");
      }

      pbft_mgr_->periodDataQueuePush(std::move(period_data), peer->getId(), std::move(current_block_cert_votes));
    } catch (const std::exception &e) {
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

void PbftSyncPacketHandler::pbftSyncComplete() {
  if (pbft_mgr_->periodDataQueueSize()) {
    LOG(log_tr_) << "Syncing pbft blocks faster than processing. Remaining sync size "
                 << pbft_mgr_->periodDataQueueSize();
    periodic_events_tp_.post(kDelayedPbftSyncDelayMs, [this] { pbftSyncComplete(); });
  } else {
    LOG(log_dg_) << "Syncing PBFT is completed";
    // We are pbft synced with the node we are connected to but
    // calling startSyncingPbft will check if some nodes have
    // greater pbft chain size and we should continue syncing with
    // them, Or sync pending DAG blocks
    pbft_syncing_state_->setPbftSyncing(false);
    startSyncingPbft();
    if (!pbft_syncing_state_->isPbftSyncing()) {
      requestPendingDagBlocks();
    }
  }
}

void PbftSyncPacketHandler::delayedPbftSync(uint32_t counter) {
  const uint32_t max_delayed_pbft_sync_count = 60000 / kDelayedPbftSyncDelayMs;
  auto pbft_sync_period = pbft_mgr_->pbftSyncingPeriod();
  if (counter > max_delayed_pbft_sync_count) {
    LOG(log_er_) << "Pbft blocks stuck in queue, no new block processed in 60 seconds " << pbft_sync_period << " "
                 << pbft_chain_->getPbftChainSize();
    pbft_syncing_state_->setPbftSyncing(false);
    LOG(log_tr_) << "Syncing PBFT is stopping";
    return;
  }

  if (pbft_syncing_state_->isPbftSyncing()) {
    if (pbft_sync_period > pbft_chain_->getPbftChainSize() + (10 * kConf.network.sync_level_size)) {
      LOG(log_tr_) << "Syncing pbft blocks faster than processing " << pbft_sync_period << " "
                   << pbft_chain_->getPbftChainSize();
      periodic_events_tp_.post(kDelayedPbftSyncDelayMs, [this, counter] { delayedPbftSync(counter + 1); });
    } else {
      if (!syncPeerPbft(pbft_sync_period + 1)) {
        pbft_syncing_state_->setPbftSyncing(false);
      }
    }
  }
}

}  // namespace taraxa::network::tarcap
