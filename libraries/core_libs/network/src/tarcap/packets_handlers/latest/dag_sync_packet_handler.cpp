#include "network/tarcap/packets_handlers/latest/dag_sync_packet_handler.hpp"

#include <cassert>
#include <exception>
#include <stdexcept>

#include "dag/dag.hpp"
#include "network/tarcap/packets_handlers/latest/common/ext_syncing_packet_handler.hpp"
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif
#include "transaction/transaction.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindDagBlock = 3;
constexpr uint32_t kNetworkPacketKindDagSync = 6;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

rust::Vec<uint8_t> toBridgeBytes(const bytes& input) {
  rust::Vec<uint8_t> output;
  output.reserve(input.size());
  for (const auto byte : input) {
    output.push_back(static_cast<uint8_t>(byte));
  }
  return output;
}

}  // namespace

struct DagSyncPacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

DagSyncPacketHandler::DagSyncPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                           std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                           std::shared_ptr<PbftSyncingState> pbft_syncing_state,
                                           std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<PbftManager> pbft_mgr,
                                           std::shared_ptr<DagManager> dag_mgr,
                                           std::shared_ptr<TransactionManager> trx_mgr,
#ifndef RUSTAXA_ENABLE
                                           std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY:
                                                                          // legacy DAG sync handler.
#endif
                                           const addr_t& node_addr, const std::string& logs_prefix)
    : ISyncPacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_syncing_state),
                         std::move(pbft_chain), std::move(pbft_mgr), std::move(dag_mgr),
#ifndef RUSTAXA_ENABLE
                         std::move(db),
#endif
                         node_addr, logs_prefix + "DAG_SYNC_PH"),
      trx_mgr_(std::move(trx_mgr)) {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

DagSyncPacketHandler::~DagSyncPacketHandler() = default;

void DagSyncPacketHandler::process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<DagSyncPacket>(packet_data.rlp_);

  // If the periods did not match restart syncing
  if (packet.response_period > packet.request_period) {
    LOG(log_dg_) << "Received DagSyncPacket with mismatching periods: " << packet.response_period << " "
                 << packet.request_period << " from " << peer->getId();
    if (peer->pbft_chain_size_ < packet.response_period) {
      peer->pbft_chain_size_ = packet.response_period;
    }
    peer->peer_dag_syncing_ = false;
    // We might be behind, restart pbft sync if needed
    startSyncingPbft();
    return;
  } else if (packet.response_period < packet.request_period) {
    // This should not be possible for honest node
    std::ostringstream err_msg;
    err_msg << "Received DagSyncPacket with mismatching periods: response_period(" << packet.response_period
            << ") != request_period(" << packet.request_period << ")";

    throw MaliciousPeerException(err_msg.str());
  }

  std::vector<trx_hash_t> transactions_to_log;
  std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>> transactions_map;
  transactions_to_log.reserve(packet.transactions.size());
  transactions_map.reserve(packet.transactions.size());
  for (auto& trx : packet.transactions) {
    const auto tx_hash = trx->getHash();
    peer->markTransactionAsKnown(tx_hash);
    transactions_to_log.push_back(tx_hash);
    transactions_map.emplace(tx_hash, trx);

    if (trx_mgr_->isTransactionKnown(tx_hash)) {
      continue;
    }

    auto [verified, reason] = trx_mgr_->verifyTransaction(trx);
    if (!verified) {
      std::ostringstream err_msg;
      err_msg << "DagBlock transaction " << tx_hash << " validation failed: " << reason;
      throw MaliciousPeerException(err_msg.str());
    }
  }

  std::vector<blk_hash_t> dag_blocks_to_log;
  dag_blocks_to_log.reserve(packet.dag_blocks.size());
  for (auto& block : packet.dag_blocks) {
    dag_blocks_to_log.push_back(block->getHash());
    peer->markDagBlockAsKnown(block->getHash());

    if (dag_mgr_->isDagBlockKnown(block->getHash())) {
      LOG(log_tr_) << "Received known DagBlock " << block->getHash() << "from: " << peer->getId();
      continue;
    }

#ifdef RUSTAXA_ENABLE
    rustaxa::NetworkDagBlockAdmissionRequestEffects effects{};
    effects.peer_id = peer->getId().asArray();
    effects.block_hash = block->getHash().asArray();
    effects.block_rlp = toBridgeBytes(block->rlp(true));
    effects.transaction_count = transactions_map.size();
    effects.source_payload_id = 0;
    effects.admit_block = true;
    (void)queueDagSyncBlockAdmissionRequestEffects(effects);
    executeDagSyncBlockAdmissionEffect(block, peer, transactions_map);
#else
    auto verified = dag_mgr_->verifyBlock(block, transactions_map);
    if (verified.first != DagManager::VerifyBlockReturnType::Verified) {
      std::ostringstream err_msg;
      err_msg << "DagBlock " << block->getHash() << " failed verification with error code "
              << static_cast<uint32_t>(verified.first);
      throw MaliciousPeerException(err_msg.str());
    }

    if (block->getLevel() > peer->dag_level_) peer->dag_level_ = block->getLevel();

    auto status = dag_mgr_->addDagBlock(block, std::move(verified.second));
    if (!status.first) {
      std::ostringstream err_msg;
      if (status.second.size() > 0)
        err_msg << "DagBlock" << block->getHash() << " has missing pivot or/and tips " << status.second;
      else
        err_msg << "DagBlock" << block->getHash() << " could not be added to DAG";
      throw MaliciousPeerException(err_msg.str());
    }
#endif
  }

  peer->peer_dag_synced_ = true;
  peer->peer_dag_synced_time_ =
      std::chrono::duration_cast<std::chrono::seconds>(std::chrono::system_clock::now().time_since_epoch()).count();
  peer->peer_dag_syncing_ = false;

  LOG(log_dg_) << "Received DagSyncPacket with blocks: " << dag_blocks_to_log
               << " Transactions: " << transactions_to_log << " from " << peer->getId();
}

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkIngressDecision DagSyncPacketHandler::queueDagSyncBlockAdmissionRequestEffects(
    const rustaxa::NetworkDagBlockAdmissionRequestEffects& effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_dag_sync_block_admission_request_effects(effects);
}

void DagSyncPacketHandler::executeDagSyncBlockAdmissionEffect(
    std::shared_ptr<DagBlock>& block, const std::shared_ptr<TaraxaPeer>& peer,
    const std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>>& trxs) {
  assert(rust_consensus_network_api_);
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
      if (effect.kind != kNetworkEffectKindRecordConsensusObject || effect.object_kind != kNetworkObjectKindDagBlock ||
          effect.packet_kind != kNetworkPacketKindDagSync || !block ||
          block->getHash().asArray() != effect.object_hash ||
          block->rlp(true) != bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()) ||
          effect.dependency_id != trxs.size()) {
        throw std::runtime_error("Network API DAG sync block admission effect missing matching live block");
      }

      auto verified = dag_mgr_->verifyBlock(block, trxs);
      if (verified.first != DagManager::VerifyBlockReturnType::Verified) {
        std::ostringstream err_msg;
        err_msg << "DagBlock " << block->getHash() << " failed verification with error code "
                << static_cast<uint32_t>(verified.first);
        throw MaliciousPeerException(err_msg.str());
      }

      if (block->getLevel() > peer->dag_level_) peer->dag_level_ = block->getLevel();

      auto status = dag_mgr_->addDagBlock(block, std::move(verified.second));
      if (!status.first) {
        std::ostringstream err_msg;
        if (status.second.size() > 0)
          err_msg << "DagBlock" << block->getHash() << " has missing pivot or/and tips " << status.second;
        else
          err_msg << "DagBlock" << block->getHash() << " could not be added to DAG";
        throw MaliciousPeerException(err_msg.str());
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
