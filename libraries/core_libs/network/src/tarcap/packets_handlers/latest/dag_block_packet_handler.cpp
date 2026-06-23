#include "network/tarcap/packets_handlers/latest/dag_block_packet_handler.hpp"

#include <cassert>
#include <exception>
#include <stdexcept>

#include "dag/dag_manager.hpp"
#include "network/tarcap/packets_handlers/latest/transaction_packet_handler.hpp"
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif
#include "transaction/transaction_manager.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindDagBlock = 3;
constexpr uint32_t kNetworkPacketKindDagBlock = 5;

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

struct DagBlockPacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

DagBlockPacketHandler::DagBlockPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                             std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                             std::shared_ptr<PbftSyncingState> pbft_syncing_state,
                                             std::shared_ptr<PbftChain> pbft_chain,
                                             std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<DagManager> dag_mgr,
                                             std::shared_ptr<TransactionManager> trx_mgr,
#ifndef RUSTAXA_ENABLE
                                             std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY:
                                                                            // legacy DAG handler.
#endif
                                             const addr_t &node_addr, const std::string &logs_prefix)
    : IDagBlockPacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_syncing_state),
                             std::move(pbft_chain), std::move(pbft_mgr), std::move(dag_mgr),
#ifndef RUSTAXA_ENABLE
                             std::move(db),
#endif
                             node_addr, logs_prefix + "DAG_BLOCK_PH"),
      trx_mgr_(std::move(trx_mgr)) {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

DagBlockPacketHandler::~DagBlockPacketHandler() = default;

void DagBlockPacketHandler::process(const threadpool::PacketData &packet_data,
                                    const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<DagBlockPacket>(packet_data.rlp_);

  blk_hash_t const hash = packet.dag_block->getHash();

  for (const auto &tx : packet.transactions) {
    peer->markTransactionAsKnown(tx->getHash());
  }
  peer->markDagBlockAsKnown(hash);

  if (packet.dag_block->getLevel() > peer->dag_level_) {
    peer->dag_level_ = packet.dag_block->getLevel();
  }

  // Do not process this block in case we already have it
  if (dag_mgr_->isDagBlockKnown(packet.dag_block->getHash())) {
    LOG(log_tr_) << "Received known DagBlockPacket " << hash << "from: " << peer->getId();
    return;
  }

  std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>> txs_map;
  txs_map.reserve(packet.transactions.size());
  for (const auto &tx : packet.transactions) {
    txs_map.emplace(tx->getHash(), tx);
  }

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkDagBlockAdmissionRequestEffects effects{};
  effects.peer_id = peer->getId().asArray();
  effects.block_hash = packet.dag_block->getHash().asArray();
  effects.block_rlp = toBridgeBytes(packet.dag_block->rlp(true));
  effects.transaction_count = packet.transactions.size();
  effects.source_payload_id = 0;
  effects.admit_block = true;
  (void)queueDagBlockAdmissionRequestEffects(effects);
  executeDagBlockAdmissionEffect(std::move(packet.dag_block), peer, txs_map);
#else
  onNewBlockReceived(std::move(packet.dag_block), peer, txs_map);
#endif
}

void DagBlockPacketHandler::sendBlockWithTransactions(const std::shared_ptr<TaraxaPeer> &peer,
                                                      const std::shared_ptr<DagBlock> &block,
                                                      SharedTransactions &&trxs) {
  // This lock prevents race condition between syncing and gossiping dag blocks
  std::unique_lock lock(peer->mutex_for_sending_dag_blocks_);

  DagBlockPacket dag_block_packet{.transactions = std::move(trxs), .dag_block = block};
  if (!sealAndSend(peer->getId(), SubprotocolPacketType::kDagBlockPacket, encodePacketRlp(dag_block_packet))) {
    LOG(log_wr_) << "Sending DagBlock " << block->getHash() << " failed to " << peer->getId();
    return;
  }

  // Mark data as known if sending was successful
  peer->markDagBlockAsKnown(block->getHash());
}

void DagBlockPacketHandler::onNewBlockReceived(
    std::shared_ptr<DagBlock> &&block, const std::shared_ptr<TaraxaPeer> &peer,
    const std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>> &trxs) {
  const auto block_hash = block->getHash();
  auto verified = dag_mgr_->verifyBlock(block, trxs);
  switch (verified.first) {
    case DagManager::VerifyBlockReturnType::IncorrectTransactionsEstimation:
    case DagManager::VerifyBlockReturnType::BlockTooBig:
    case DagManager::VerifyBlockReturnType::FailedVdfVerification:
    case DagManager::VerifyBlockReturnType::NotEligible:
    case DagManager::VerifyBlockReturnType::FailedTipsVerification: {
      std::ostringstream err_msg;
      err_msg << "DagBlock " << block_hash << " failed verification with error code "
              << static_cast<uint32_t>(verified.first);
      throw MaliciousPeerException(err_msg.str());
    }
    case DagManager::VerifyBlockReturnType::MissingTransaction:
      if (peer->dagSyncingAllowed()) {
        if (trx_mgr_->transactionsDropped()) [[unlikely]] {
          LOG(log_nf_) << "NewBlock " << block_hash.toString() << " from peer " << peer->getId()
                       << " is missing transaction, our pool recently dropped transactions, requesting dag sync";
        } else {
          LOG(log_wr_) << "NewBlock " << block_hash.toString() << " from peer " << peer->getId()
                       << " is missing transaction, requesting dag sync";
        }
        peer->peer_dag_synced_ = false;
        requestPendingDagBlocks(peer);
      } else {
        if (trx_mgr_->transactionsDropped()) [[unlikely]] {
          // Disconnecting since anything after will also contain missing pivot/tips ...
          LOG(log_nf_) << "NewBlock " << block_hash.toString() << " from peer " << peer->getId()
                       << " is missing transaction, but our pool recently dropped transactions, disconnecting";
          disconnect(peer->getId(), dev::p2p::UserReason);
        } else {
          std::ostringstream err_msg;
          err_msg << "DagBlock" << block_hash << " is missing a transaction while in a dag synced state";
          throw MaliciousPeerException(err_msg.str());
        }
      }
      break;
    case DagManager::VerifyBlockReturnType::MissingTip:
      if (peer->peer_dag_synced_) {
        if (peer->dagSyncingAllowed()) {
          LOG(log_wr_) << "NewBlock " << block_hash.toString() << " from peer " << peer->getId()
                       << " is missing tip, requesting dag sync";
          peer->peer_dag_synced_ = false;
          requestPendingDagBlocks(peer);
        } else {
          std::ostringstream err_msg;
          err_msg << "DagBlock has missing tip";
          throw MaliciousPeerException(err_msg.str());
        }
      } else {
        // peer_dag_synced_ flag ensures that this can only be performed once for a peer
        requestPendingDagBlocks(peer);
      }
      break;
    case DagManager::VerifyBlockReturnType::AheadBlock:
    case DagManager::VerifyBlockReturnType::FutureBlock:
      if (peer->peer_dag_synced_) {
        LOG(log_er_) << "DagBlock" << block_hash << " is an ahead/future block. Peer " << peer->getId()
                     << " will be disconnected";
        disconnect(peer->getId(), dev::p2p::UserReason);
      }
      break;
    case DagManager::VerifyBlockReturnType::Verified: {
      auto status = dag_mgr_->addDagBlock(block, std::move(verified.second));
      if (!status.first) {
        LOG(log_dg_) << "Received DagBlockPacket " << block_hash << "from: " << peer->getId();
        // Ignore new block packets when pbft syncing
        if (pbft_syncing_state_->isPbftSyncing()) {
          LOG(log_dg_) << "Ignore new dag block " << block_hash << ", pbft syncing is on";
        } else if (peer->peer_dag_syncing_) {
          LOG(log_dg_) << "Ignore new dag block " << block_hash << ", dag syncing is on";
        } else {
          if (peer->peer_dag_synced_) {
            std::ostringstream err_msg;
            if (status.second.size() > 0)
              err_msg << "DagBlock" << block->getHash() << " has missing pivot or/and tips " << status.second;
            else
              err_msg << "DagBlock" << block->getHash() << " could not be added to DAG";
            throw MaliciousPeerException(err_msg.str());
          } else {
            // peer_dag_synced_ flag ensures that this can only be performed once for a peer
            requestPendingDagBlocks(peer);
          }
        }
      }
    } break;
    case DagManager::VerifyBlockReturnType::ExpiredBlock:
      break;
  }
}

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkIngressDecision DagBlockPacketHandler::queueDagBlockAdmissionRequestEffects(
    const rustaxa::NetworkDagBlockAdmissionRequestEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_dag_block_admission_request_effects(effects);
}

void DagBlockPacketHandler::executeDagBlockAdmissionEffect(
    std::shared_ptr<DagBlock> &&block, const std::shared_ptr<TaraxaPeer> &peer,
    const std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>> &trxs) {
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
      if (effect.kind != kNetworkEffectKindRecordConsensusObject || effect.object_kind != kNetworkObjectKindDagBlock ||
          effect.packet_kind != kNetworkPacketKindDagBlock || !block || block->getHash().asArray() != effect.object_hash ||
          block->rlp(true) != bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()) ||
          effect.dependency_id != trxs.size()) {
        throw std::runtime_error("Network API DAG block admission effect missing matching live block");
      }

      onNewBlockReceived(std::move(block), peer, trxs);
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

}  // namespace taraxa::network::tarcap
