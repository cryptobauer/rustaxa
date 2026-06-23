#include "network/tarcap/packets_handlers/latest/get_dag_sync_packet_handler.hpp"

#include <array>
#include <cassert>
#include <exception>
#include <stdexcept>

#include "dag/dag_manager.hpp"
#include "network/tarcap/packets/latest/dag_sync_packet.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindDagSyncEgressRequest = 10;
constexpr uint32_t kNetworkPacketKindGetDagSync = 12;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

std::array<uint8_t, 32> dagSyncEgressRequestKey(uint64_t peer_period, uint64_t requested_hash_count,
                                                uint64_t source_payload_id) {
  std::array<uint8_t, 32> key{};
  for (size_t i = 0; i < sizeof(uint64_t); ++i) {
    key[i] = static_cast<uint8_t>(peer_period >> ((sizeof(uint64_t) - 1 - i) * 8));
    key[8 + i] = static_cast<uint8_t>(requested_hash_count >> ((sizeof(uint64_t) - 1 - i) * 8));
    key[16 + i] = static_cast<uint8_t>(source_payload_id >> ((sizeof(uint64_t) - 1 - i) * 8));
  }
  return key;
}

blk_hash_t bridgeHashToBlkHash(const std::array<uint8_t, 32> &hash) {
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

std::unordered_set<blk_hash_t> decodeRequestedBlockHashes(const rust::Vec<uint8_t> &payload_bytes) {
  if (payload_bytes.size() % 32 != 0) {
    throw std::runtime_error("Network API DAG sync egress request hash payload has invalid length");
  }

  std::unordered_set<blk_hash_t> blocks_hashes_set;
  for (size_t offset = 0; offset < payload_bytes.size(); offset += 32) {
    std::array<uint8_t, 32> hash{};
    for (size_t i = 0; i < hash.size(); ++i) {
      hash[i] = payload_bytes[offset + i];
    }
    blocks_hashes_set.insert(bridgeHashToBlkHash(hash));
  }
  return blocks_hashes_set;
}

}  // namespace

struct GetDagSyncPacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

GetDagSyncPacketHandler::GetDagSyncPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                                 std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                 std::shared_ptr<TransactionManager> trx_mgr,
                                                 std::shared_ptr<DagManager> dag_mgr, const addr_t &node_addr,
                                                 const std::string &logs_prefix)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr, logs_prefix + "GET_DAG_SYNC_PH"),
      trx_mgr_(std::move(trx_mgr)),
      dag_mgr_(std::move(dag_mgr)) {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

GetDagSyncPacketHandler::~GetDagSyncPacketHandler() = default;

void GetDagSyncPacketHandler::process(const threadpool::PacketData &packet_data,
                                      [[maybe_unused]] const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<GetDagSyncPacket>(packet_data.rlp_);

  if (!peer->requestDagSyncingAllowed()) {
    // This should not be possible for honest node
    // Each node should perform dag syncing only when allowed
    std::ostringstream err_msg;
    err_msg << "Received multiple GetDagSyncPackets from " << peer->getId().abridged();

    throw MaliciousPeerException(err_msg.str());
  }

  // This lock prevents race condition between syncing and gossiping dag blocks
  std::unique_lock lock(peer->mutex_for_sending_dag_blocks_);

  std::unordered_set<blk_hash_t> blocks_hashes_set;
  std::string blocks_hashes_to_log;
  blocks_hashes_to_log.reserve(packet.blocks_hashes.size());
  for (const auto &hash : packet.blocks_hashes) {
    if (blocks_hashes_set.insert(hash).second) {
      blocks_hashes_to_log += hash.abridged();
    }
  }

  LOG(log_dg_) << "Received GetDagSyncPacket: " << blocks_hashes_to_log << " from " << peer->getId();

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkDagSyncEgressRequestEffects effects{};
  effects.peer_id = peer->getId().asArray();
  effects.peer_period = packet.peer_period;
  effects.source_payload_id = packet_data.id_;
  effects.request_blocks = true;
  for (const auto &hash : blocks_hashes_set) {
    effects.requested_block_hashes.push_back(rustaxa::DagHash{hash.asArray()});
  }
  (void)queueDagSyncEgressRequestEffects(effects);
  executeDagSyncEgressEffect(peer);
  return;
#endif

  auto [period, blocks, transactions] = dag_mgr_->getNonFinalizedBlocksWithTransactions(blocks_hashes_set);
  if (packet.peer_period == period) {
    peer->syncing_ = false;
    peer->peer_requested_dag_syncing_ = true;
    peer->peer_requested_dag_syncing_time_ =
        std::chrono::duration_cast<std::chrono::seconds>(std::chrono::system_clock::now().time_since_epoch()).count();
  } else {
    // There is no point in sending blocks if periods do not match, but an empty packet should be sent
    blocks.clear();
    transactions.clear();
  }
  sendBlocks(peer->getId(), std::move(blocks), std::move(transactions), packet.peer_period, period);
}

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkIngressDecision GetDagSyncPacketHandler::queueDagSyncEgressRequestEffects(
    const rustaxa::NetworkDagSyncEgressRequestEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_dag_sync_egress_request_effects(effects);
}

void GetDagSyncPacketHandler::executeDagSyncEgressEffect(const std::shared_ptr<TaraxaPeer> &peer) {
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
          effect.object_kind != kNetworkObjectKindDagSyncEgressRequest ||
          effect.packet_kind != kNetworkPacketKindGetDagSync || effect.peer_id != peer->getId().asArray() ||
          effect.payload_bytes.size() != effect.dependency_id * 32 ||
          effect.object_hash !=
              dagSyncEgressRequestKey(effect.period, effect.dependency_id, effect.source_payload_id)) {
        throw std::runtime_error("Network API DAG sync egress effect missing matching request");
      }

      auto blocks_hashes_set = decodeRequestedBlockHashes(effect.payload_bytes);
      auto [period, blocks, transactions] = dag_mgr_->getNonFinalizedBlocksWithTransactions(blocks_hashes_set);
      if (effect.period == period) {
        peer->syncing_ = false;
        peer->peer_requested_dag_syncing_ = true;
        peer->peer_requested_dag_syncing_time_ =
            std::chrono::duration_cast<std::chrono::seconds>(std::chrono::system_clock::now().time_since_epoch())
                .count();
      } else {
        blocks.clear();
        transactions.clear();
      }
      sendBlocks(peer->getId(), std::move(blocks), std::move(transactions), effect.period, period);
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

void GetDagSyncPacketHandler::sendBlocks(const dev::p2p::NodeID &peer_id,
                                         std::vector<std::shared_ptr<DagBlock>> &&blocks,
                                         SharedTransactions &&transactions, PbftPeriod request_period,
                                         PbftPeriod period) {
  auto peer = peers_state_->getPeer(peer_id);
  if (!peer) return;

  DagSyncPacket dag_sync_packet(request_period, period, std::move(transactions), std::move(blocks));
  sealAndSend(peer_id, SubprotocolPacketType::kDagSyncPacket, encodePacketRlp(dag_sync_packet));
}

}  // namespace taraxa::network::tarcap
