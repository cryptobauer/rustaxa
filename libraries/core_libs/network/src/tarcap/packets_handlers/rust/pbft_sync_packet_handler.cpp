#include "network/tarcap/packets_handlers/rust/pbft_sync_packet_handler.hpp"

#include <algorithm>
#include <chrono>
#include <stdexcept>

#include "final_chain/final_chain.hpp"
#include "network/consensus_query.hpp"
#include "transaction/transaction.hpp"

namespace taraxa::network::tarcap {

namespace {

std::array<uint8_t, 32> toBridgeU256(const u256& value) {
  std::array<uint8_t, 32> out{};
  const auto bytes = dev::toBigEndian(value);
  std::copy(bytes.begin(), bytes.end(), out.begin() + (out.size() - bytes.size()));
  return out;
}

std::vector<network::PbftSyncSlashingSubmitterFact> makeSlashingSubmitterFacts(
    const FullNodeConfig& config, const std::shared_ptr<final_chain::FinalChain>& final_chain) {
  std::vector<network::PbftSyncSlashingSubmitterFact> submitters;
  submitters.reserve(config.wallets.size());
  for (size_t index = 0; index < config.wallets.size(); ++index) {
    const auto account = final_chain->getAccount(config.wallets[index].node_addr).value_or(state_api::ZeroAccount);
    submitters.push_back({index, toBridgeU256(account.nonce), toBridgeU256(account.balance)});
    if (account.balance != 0) {
      break;
    }
  }
  return submitters;
}

uint64_t monotonicMilliseconds() {
  return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
      .count();
}

constexpr uint8_t kPbftSyncStopCompleted = 1;
constexpr uint8_t kPbftSyncStopTransportFailed = 4;

}  // namespace

RustPbftSyncPacketHandler::RustPbftSyncPacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, net::ConsensusQueryClient pbft_chain,
    network::ConsensusLiveStatusProvider consensus_status, std::shared_ptr<final_chain::FinalChain> final_chain,
    network::ConsensusNetworkApiShared consensus_network_api, const addr_t& node_addr, const std::string& logs_prefix)
    : RustConsensusTransportPacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_chain),
                                          std::move(consensus_status), consensus_network_api, node_addr,
                                          logs_prefix + "PBFT_SYNC_PH"),
      final_chain_(std::move(final_chain)),
      consensus_network_api_(std::move(consensus_network_api)),
      periodic_events_tp_(1, true) {}

RustPbftSyncPacketHandler::~RustPbftSyncPacketHandler() = default;

void RustPbftSyncPacketHandler::process(const threadpool::PacketData& packet_data,
                                        const std::shared_ptr<TaraxaPeer>& peer) {
  const auto source_outcome =
      consensus_network_api_->admitPbftSyncSource(peer->getId().asArray(), network::PbftSyncResponseSource::kActive);
  if (!source_outcome.accepted) {
    LOG(log_wr_) << "PbftSyncPacket received from unexpected peer " << peer->getId().abridged() << ": "
                 << static_cast<std::string>(source_outcome.error_code);
    return;
  }
  const auto generation = source_outcome.generation;
  const auto peer_id = peer->getId().asArray();

  const auto packet_rlp = packet_data.rlp_.data().toBytes();
  const auto ingress = consensus_network_api_->admitPbftSyncPacket(
      packet_rlp, packet_data.id_, peer->getId().asArray(), makeSlashingSubmitterFacts(kConf, final_chain_),
      network::PbftSyncIngressExecutor{[this](const auto& effect) { return executeSlashingTransaction(effect); }});
  if (ingress.action == network::PbftSyncIngressAction::kMalicious) {
    LOG(log_er_) << "Native PBFT-sync ingress rejected packet: " << ingress.error_code;
    peers_state_->handleMaliciousSyncPeer(peer->getId());
    return;
  }
  const auto pbft_block_hash = blk_hash_t(ingress.block_hash.data(), blk_hash_t::ConstructFromPointer);
  if (peer->dag_level_ < ingress.max_dag_level) {
    peer->dag_level_ = ingress.max_dag_level;
  }
  peer->markPbftBlockAsKnown(pbft_block_hash);
  if (peer->pbft_chain_size_ < ingress.block_period) {
    peer->pbft_chain_size_ = ingress.block_period;
  }
  LOG(log_dg_) << "PbftSyncPacket admitted by native consensus. Period: " << ingress.block_period
               << ", max DAG level: " << ingress.max_dag_level << " from " << peer->getId();

  if (ingress.action == network::PbftSyncIngressAction::kDrop) {
    LOG(log_er_) << "Native PBFT-sync ingress dropped block " << pbft_block_hash << ": " << ingress.error_code;
    return;
  }
  if (ingress.action == network::PbftSyncIngressAction::kSyncComplete) {
    pbftSyncComplete(generation, peer_id);
    return;
  }
  if (ingress.action == network::PbftSyncIngressAction::kStopSyncing) {
    stopPbftSync(generation, peer_id, kPbftSyncStopCompleted);
    return;
  }
  if (ingress.action == network::PbftSyncIngressAction::kDuplicate) {
    LOG(log_wr_) << "PBFT block " << pbft_block_hash << ", period: " << ingress.block_period << " from "
                 << peer->getId() << " already present in chain";
  } else if (ingress.action == network::PbftSyncIngressAction::kQueueRejected) {
    LOG(log_er_) << "Native PBFT-sync queue rejected period " << ingress.block_period << ": " << ingress.error_code;
  }

  const auto pbft_sync_period = consensus_status_().syncing_period;
  consensus_network_api_->recordPbftSyncActivity(monotonicMilliseconds(), generation, peer_id);
  if (ingress.current_cert_present) {
    pbftSyncComplete(generation, peer_id);
    return;
  }

  if (ingress.last_block) {
    const auto outcome = consensus_network_api_->planPbftSyncLastBlock(
        monotonicMilliseconds(), generation, peer_id, pbft_sync_period,
        net::consensusPbftProgress(pbft_chain_).finalized_period, ingress.block_period, kConf.network.sync_level_size);
    if (outcome.retry) {
      periodic_events_tp_.post(kDelayedPbftSyncDelayMs,
                               [this, generation, peer_id] { delayedPbftSync(1, generation, peer_id); });
    } else if (outcome.request_next && !syncPeerPbft(pbft_sync_period + 1)) {
      stopPbftSync(generation, peer_id, kPbftSyncStopTransportFailed);
    }
  }
}

bool RustPbftSyncPacketHandler::executeSlashingTransaction(const network::PbftSyncSlashingTransaction& effect) const {
  if (effect.status != 0) {
    throw std::runtime_error("Native PBFT-sync ingress returned a non-executable slashing transaction effect");
  }
  if (effect.wallet_index >= kConf.wallets.size()) {
    throw std::runtime_error("Native PBFT-sync ingress returned an invalid slashing wallet index");
  }

  return consensus_network_api_->executePbftSyncSlashingTransaction(effect, kConf);
}

void RustPbftSyncPacketHandler::stopPbftSync(uint64_t generation, const std::array<uint8_t, 64>& peer_id,
                                             uint8_t reason) const {
  consensus_network_api_->stopPbftSync(generation, peer_id, reason);
}

void RustPbftSyncPacketHandler::pbftSyncComplete(uint64_t generation, std::array<uint8_t, 64> peer_id) {
  const auto outcome = consensus_network_api_->completePbftSync(monotonicMilliseconds(), generation, peer_id,
                                                                consensus_status_().sync_queue_size);
  if (outcome.retry) {
    periodic_events_tp_.post(kDelayedPbftSyncDelayMs,
                             [this, generation, peer_id] { pbftSyncComplete(generation, peer_id); });
    return;
  }
  if (outcome.restart_sync) {
    startSyncingPbft();
  }
  if (outcome.request_pending_dag_if_idle && !consensus_network_api_->pbftSyncStatus(monotonicMilliseconds()).active) {
    requestPendingDagBlocks();
  }
}

void RustPbftSyncPacketHandler::delayedPbftSync(uint32_t counter, uint64_t generation,
                                                std::array<uint8_t, 64> peer_id) {
  const auto pbft_sync_period = consensus_status_().syncing_period;
  const auto outcome =
      consensus_network_api_->planDelayedPbftSync(monotonicMilliseconds(), generation, peer_id, pbft_sync_period,
                                                  net::consensusPbftProgress(pbft_chain_).finalized_period,
                                                  kConf.network.sync_level_size, counter, kDelayedPbftSyncDelayMs);
  if (outcome.stopped) {
    LOG(log_er_) << "Pbft blocks stuck in queue, no new block processed in 60 seconds " << pbft_sync_period << " "
                 << net::consensusPbftProgress(pbft_chain_).finalized_period;
    return;
  }
  if (outcome.retry) {
    periodic_events_tp_.post(kDelayedPbftSyncDelayMs, [this, counter, generation, peer_id] {
      delayedPbftSync(counter + 1, generation, peer_id);
    });
  } else if (outcome.request_next && !syncPeerPbft(pbft_sync_period + 1)) {
    stopPbftSync(generation, peer_id, kPbftSyncStopTransportFailed);
  }
}

}  // namespace taraxa::network::tarcap
