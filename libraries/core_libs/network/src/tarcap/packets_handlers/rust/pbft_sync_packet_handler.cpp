#include "network/tarcap/packets_handlers/rust/pbft_sync_packet_handler.hpp"

#include <algorithm>
#include <stdexcept>

#include "final_chain/final_chain.hpp"
#include "network/consensus_query.hpp"
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#include "pbft/pbft_manager.hpp"
#include "transaction/transaction.hpp"

namespace taraxa::network::tarcap {

namespace {

u256 fromBridgeU256(const std::array<uint8_t, 32>& value) {
  return dev::fromBigEndian<u256>(dev::bytes(value.begin(), value.end()));
}

addr_t fromBridgeAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

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

}  // namespace

RustPbftSyncPacketHandler::RustPbftSyncPacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, std::shared_ptr<PbftSyncingState> pbft_syncing_state,
    net::ConsensusQueryClient pbft_chain, std::shared_ptr<PbftManager> pbft_mgr, std::shared_ptr<DagManager> dag_mgr,
    std::shared_ptr<TransactionManager> trx_mgr, std::shared_ptr<final_chain::FinalChain> final_chain,
    network::ConsensusNetworkApiShared consensus_network_api, const addr_t& node_addr, const std::string& logs_prefix)
    : ISyncPacketHandler(conf, std::move(peers_state), std::move(packets_stats), std::move(pbft_syncing_state),
                         std::move(pbft_chain), std::move(pbft_mgr), std::move(dag_mgr), consensus_network_api,
                         node_addr, logs_prefix + "PBFT_SYNC_PH"),
      trx_mgr_(std::move(trx_mgr)),
      final_chain_(std::move(final_chain)),
      consensus_network_api_(std::move(consensus_network_api)),
      periodic_events_tp_(1, true) {}

RustPbftSyncPacketHandler::~RustPbftSyncPacketHandler() = default;

void RustPbftSyncPacketHandler::process(const threadpool::PacketData& packet_data,
                                        const std::shared_ptr<TaraxaPeer>& peer) {
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
    pbftSyncComplete();
    return;
  }
  if (ingress.action == network::PbftSyncIngressAction::kStopSyncing) {
    pbft_syncing_state_->setPbftSyncing(false);
    return;
  }
  if (ingress.action == network::PbftSyncIngressAction::kDuplicate) {
    LOG(log_wr_) << "PBFT block " << pbft_block_hash << ", period: " << ingress.block_period << " from "
                 << peer->getId() << " already present in chain";
  } else if (ingress.action == network::PbftSyncIngressAction::kQueueRejected) {
    LOG(log_er_) << "Native PBFT-sync queue rejected period " << ingress.block_period << ": " << ingress.error_code;
  }

  const auto pbft_sync_period = pbft_mgr_->pbftSyncingPeriod();
  pbft_syncing_state_->setLastSyncPacketTime();
  if (ingress.current_cert_present) {
    pbftSyncComplete();
    return;
  }

  if (ingress.last_block) {
    if (pbft_sync_period > ingress.block_period) {
      pbft_syncing_state_->setPbftSyncing(false);
      return;
    }
    if (pbft_syncing_state_->isPbftSyncing()) {
      if (pbft_sync_period >
          net::consensusPbftProgress(pbft_chain_).finalized_period + (10 * kConf.network.sync_level_size)) {
        periodic_events_tp_.post(kDelayedPbftSyncDelayMs, [this] { delayedPbftSync(1); });
      } else if (!syncPeerPbft(pbft_sync_period + 1)) {
        pbft_syncing_state_->setPbftSyncing(false);
      }
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

  const auto& wallet = kConf.wallets[effect.wallet_index];
  bytes call_data(effect.call_data.begin(), effect.call_data.end());
  auto transaction = std::make_shared<Transaction>(
      fromBridgeU256(effect.nonce), fromBridgeU256(effect.value), trx_mgr_->gasPriceBid(), effect.gas_limit,
      std::move(call_data), wallet.node_secret, fromBridgeAddress(effect.contract_address), kConf.genesis.chain_id);
  return trx_mgr_->insertTransaction(transaction).first;
}

void RustPbftSyncPacketHandler::pbftSyncComplete() {
  if (pbft_mgr_->periodDataQueueSize()) {
    periodic_events_tp_.post(kDelayedPbftSyncDelayMs, [this] { pbftSyncComplete(); });
    return;
  }
  pbft_syncing_state_->setPbftSyncing(false);
  startSyncingPbft();
  if (!pbft_syncing_state_->isPbftSyncing()) {
    requestPendingDagBlocks();
  }
}

void RustPbftSyncPacketHandler::delayedPbftSync(uint32_t counter) {
  const uint32_t max_delayed_pbft_sync_count = 60000 / kDelayedPbftSyncDelayMs;
  const auto pbft_sync_period = pbft_mgr_->pbftSyncingPeriod();
  if (counter > max_delayed_pbft_sync_count) {
    LOG(log_er_) << "Pbft blocks stuck in queue, no new block processed in 60 seconds " << pbft_sync_period << " "
                 << net::consensusPbftProgress(pbft_chain_).finalized_period;
    pbft_syncing_state_->setPbftSyncing(false);
    return;
  }
  if (!pbft_syncing_state_->isPbftSyncing()) {
    return;
  }
  if (pbft_sync_period >
      net::consensusPbftProgress(pbft_chain_).finalized_period + (10 * kConf.network.sync_level_size)) {
    periodic_events_tp_.post(kDelayedPbftSyncDelayMs, [this, counter] { delayedPbftSync(counter + 1); });
  } else if (!syncPeerPbft(pbft_sync_period + 1)) {
    pbft_syncing_state_->setPbftSyncing(false);
  }
}

}  // namespace taraxa::network::tarcap
