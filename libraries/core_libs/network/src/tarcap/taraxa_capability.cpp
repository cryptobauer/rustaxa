#include "network/tarcap/taraxa_capability.hpp"
#ifdef RUSTAXA_ENABLE_NETWORK
#include "network/tarcap/taraxa_capability_shim.hpp"
#endif

#include <chrono>
#include <cstdint>
#include <exception>
#include <memory>

#include "common/app_base.hpp"
#include "network/tarcap/packets_handler.hpp"
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/interface/sync_packet_handler.hpp"
#include "network/tarcap/packets_handlers/latest/dag_block_packet_handler.hpp"
#include "network/tarcap/packets_handlers/latest/dag_sync_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/dag_block_packet_handler.hpp"
#include "network/tarcap/packets_handlers/rust/dag_sync_packet_handler.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/latest/get_dag_sync_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/get_dag_sync_packet_handler.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/latest/get_next_votes_bundle_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/get_next_votes_bundle_packet_handler.hpp"
#include "network/tarcap/packets_handlers/rust/pbft_blocks_bundle_packet_handler.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/get_pbft_sync_packet_handler.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/latest/get_pillar_votes_bundle_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/get_pillar_votes_bundle_packet_handler.hpp"
#endif
#include "network/tarcap/packets_handlers/latest/pbft_blocks_bundle_packet_handler.hpp"
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/latest/pbft_sync_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/pbft_sync_packet_handler.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/latest/pillar_vote_packet_handler.hpp"
#include "network/tarcap/packets_handlers/latest/pillar_votes_bundle_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/pillar_vote_packet_handler.hpp"
#include "network/tarcap/packets_handlers/rust/pillar_votes_bundle_packet_handler.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/latest/status_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/status_packet_handler.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/latest/transaction_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/transaction_packet_handler.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/latest/vote_packet_handler.hpp"
#include "network/tarcap/packets_handlers/latest/votes_bundle_packet_handler.hpp"
#else
#include "network/tarcap/packets_handlers/rust/vote_packet_handler.hpp"
#include "network/tarcap/packets_handlers/rust/votes_bundle_packet_handler.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/packets_handlers/v4/get_pbft_sync_packet_handler.hpp"
#endif
#include "network/consensus_query.hpp"
#ifndef RUSTAXA_ENABLE
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "pillar_chain/pillar_chain_manager.hpp"
#include "transaction/transaction_manager.hpp"
#endif

#ifndef RUSTAXA_ENABLE
#include "pbft/pbft_manager.hpp"
#endif

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
#define RUSTAXA_LEGACY_DB_ARG
#define RUSTAXA_NETWORK_API_ARG consensus_network_api,
#else
#define RUSTAXA_LEGACY_DB_ARG db,
#define RUSTAXA_NETWORK_API_ARG
#endif

TaraxaCapability::TaraxaCapability(
    TarcapVersion version, const FullNodeConfig &conf, const h256 &genesis_hash, std::weak_ptr<dev::p2p::Host> host,
    std::shared_ptr<network::threadpool::PacketsThreadPool> threadpool,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats,
#ifndef RUSTAXA_ENABLE
    std::shared_ptr<PbftSyncingState> syncing_state,
#endif
#ifndef RUSTAXA_ENABLE
    std::shared_ptr<DbStorage> db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy tarcap handler wiring.
    std::shared_ptr<PbftManager> pbft_mgr,
#else
    network::ConsensusLiveStatusProvider consensus_status,
#endif
    net::ConsensusQueryClient pbft_chain,
#ifndef RUSTAXA_ENABLE
    std::shared_ptr<VoteManager> vote_mgr, std::shared_ptr<DagManager> dag_mgr,
    std::shared_ptr<TransactionManager> trx_mgr, std::shared_ptr<SlashingManager> slashing_manager,
    std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_mgr,
#endif
#ifdef RUSTAXA_ENABLE
    SharedConsensusApplication consensus_application, network::ConsensusNetworkApiShared consensus_network_api,
#else
    std::shared_ptr<final_chain::FinalChain> final_chain,
#endif
#ifdef RUSTAXA_ENABLE_NETWORK
    RustaxaNetworkShim &rust_network_shim,
#endif
    InitPacketsHandlers init_packets_handlers)
    : version_(version),
      all_packets_stats_(std::move(packets_stats)),
      kConf(conf),
      peers_state_(nullptr),
#ifndef RUSTAXA_ENABLE
      pbft_syncing_state_(std::move(syncing_state)),
#endif
      packets_handlers_(std::make_shared<PacketsHandler>()),
      thread_pool_(std::move(threadpool))
#ifdef RUSTAXA_ENABLE
      ,
      rust_consensus_network_api_(std::move(consensus_network_api))
#endif
#ifdef RUSTAXA_ENABLE_NETWORK
      ,
      rust_network_shim_(rust_network_shim)
#endif
{
  // const std::string logs_prefix = "V" + std::to_string(version) + "_";
  const std::string logs_prefix = "";
  const auto &node_addr = kConf.getFirstWallet().node_addr;

  LOG_OBJECTS_CREATE(logs_prefix + "TARCAP");

  peers_state_ = std::make_shared<PeersState>(host, kConf);
  packets_handlers_ = init_packets_handlers(logs_prefix, conf, genesis_hash, peers_state_,
#ifndef RUSTAXA_ENABLE
                                            pbft_syncing_state_,
#endif
                                            all_packets_stats_,
#ifdef RUSTAXA_ENABLE
                                            rust_consensus_network_api_, consensus_status,
#endif
#ifndef RUSTAXA_ENABLE
                                            db, pbft_mgr,
#endif
                                            pbft_chain,
#ifndef RUSTAXA_ENABLE
                                            vote_mgr, dag_mgr, trx_mgr, slashing_manager, pillar_chain_mgr,
#endif
#ifdef RUSTAXA_ENABLE
                                            consensus_application, version, node_addr);
#else
                                            final_chain, version, node_addr);
#endif

  // Must be called after init_packets_handlers
  thread_pool_->setPacketsHandlers(version, packets_handlers_);
}

TaraxaCapability::~TaraxaCapability() = default;

std::string TaraxaCapability::name() const { return TARAXA_CAPABILITY_NAME; }

TarcapVersion TaraxaCapability::version() const { return version_; }

unsigned TaraxaCapability::messageCount() const { return SubprotocolPacketType::kPacketCount; }

void TaraxaCapability::onConnect(std::weak_ptr<dev::p2p::Session> session, u256 const &) {
  const auto session_p = session.lock();
  if (!session_p) {
    LOG(log_er_) << "Unable to obtain session ptr !";
    return;
  }

  const auto node_id = session_p->id();

  if (peers_state_->is_peer_malicious(node_id)) {
    session_p->disconnect(dev::p2p::UserReason);
    LOG(log_wr_) << "Node " << node_id << " connection dropped - malicious node";
    return;
  }

  // If queue is over the limit do not allow new nodes to connect until queue size is reduced
  if (queue_over_limit_ && peers_state_->getPeersCount() >= last_disconnect_number_of_peers_) {
    session_p->disconnect(dev::p2p::UserReason);
    LOG(log_wr_) << "Node " << node_id << " connection dropped - queue over limit";
    return;
  }

#ifdef RUSTAXA_ENABLE_NETWORK
  if (!rust_network_shim_.connectPeer(node_id)) {
    session_p->disconnect(dev::p2p::UserReason);
    return;
  }
#endif

  peers_state_->addPendingPeer(node_id, session_p->info().host + ":" + std::to_string(session_p->info().port));
  LOG(log_nf_) << "Node " << node_id << " connected";

  sendStatus(node_id, true);
}

void TaraxaCapability::onDisconnect(dev::p2p::NodeID const &_nodeID) {
#ifdef RUSTAXA_ENABLE_NETWORK
  try {
    rust_network_shim_.disconnectPeer(_nodeID);
  } catch (const std::exception &error) {
    LOG(log_wr_) << "Rust ingress peer disconnect failed: " << error.what();
  }
#endif

  LOG(log_nf_) << "Node " << _nodeID << " disconnected";
  peers_state_->erasePeer(_nodeID);

#ifdef RUSTAXA_ENABLE
  const auto now_ms =
      std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
          .count();
  const auto sync = rust_consensus_network_api_->pbftSyncStatus(now_ms);
  const auto outcome = rust_consensus_network_api_->handlePbftSyncDisconnect(sync.generation, _nodeID.asArray());
  if (outcome.restart_sync) {
    if (peers_state_->getPeersCount() > 0) {
      LOG(log_dg_) << "Restart PBFT/DAG syncing due to syncing peer disconnect.";
      startSyncingPbft();
    } else {
      LOG(log_dg_) << "Stop PBFT/DAG syncing due to syncing peer disconnect and no other peers available.";
    }
  }
#else
  const auto syncing_peer = pbft_syncing_state_->syncingPeer();
  if (pbft_syncing_state_->isPbftSyncing() && syncing_peer && syncing_peer->getId() == _nodeID) {
    pbft_syncing_state_->setPbftSyncing(false);
    if (peers_state_->getPeersCount() > 0) {
      LOG(log_dg_) << "Restart PBFT/DAG syncing due to syncing peer disconnect.";
      startSyncingPbft();
    } else {
      LOG(log_dg_) << "Stop PBFT/DAG syncing due to syncing peer disconnect and no other peers available.";
    }
  }
#endif
}

std::string TaraxaCapability::packetTypeToString(unsigned _packetType) const {
  return convertPacketTypeToString(static_cast<SubprotocolPacketType>(_packetType));
}

void TaraxaCapability::interpretCapabilityPacket(std::weak_ptr<dev::p2p::Session> session, unsigned _id,
                                                 dev::RLP const &_r) {
  const auto session_p = session.lock();
  if (!session_p) {
    LOG(log_er_) << "Unable to obtain session ptr !";
    return;
  }

  auto node_id = session.lock()->id();

  auto host = peers_state_->host_.lock();
  if (!host) {
    LOG(log_er_) << "Unable to process packet, host == nullptr";
    return;
  }

  const SubprotocolPacketType packet_type = static_cast<SubprotocolPacketType>(_id);

  // Drop any packet (except StatusPacket) that comes before the connection between nodes is initialized by sending
  // and received initial status packet
  const auto peer = peers_state_->getPacketSenderPeer(node_id, packet_type);
  if (!peer.first) [[unlikely]] {
    LOG(log_wr_) << "Unable to push packet into queue. Reason: " << peer.second;
    host->disconnect(node_id, dev::p2p::UserReason);
    return;
  }

#ifdef RUSTAXA_ENABLE
  const auto now_ms =
      std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
          .count();
  const bool deep_pbft_syncing = rust_consensus_network_api_->pbftSyncStatus(now_ms).deep_syncing;
#else
  const bool deep_pbft_syncing = pbft_syncing_state_->isDeepPbftSyncing();
#endif
  if (deep_pbft_syncing && filterSyncIrrelevantPackets(packet_type)) {
    LOG(log_dg_) << "Ignored " << convertPacketTypeToString(packet_type) << " because we are still syncing";
    return;
  }

#ifndef RUSTAXA_ENABLE_NETWORK
  const auto [hp_queue_size, mp_queue_size, lp_queue_size] = thread_pool_->getQueueSize();
  const size_t tp_queue_size = hp_queue_size + mp_queue_size + lp_queue_size;

  // Check peer's max allowed packets processing time in case peer_max_packets_queue_size_limit was exceeded
  if (kConf.network.ddos_protection.peer_max_packets_queue_size_limit &&
      tp_queue_size > kConf.network.ddos_protection.peer_max_packets_queue_size_limit) {
    const auto [start_time, peer_packets_stats] = peer.first->getAllPacketsStatsCopy();
    // As start_time is reset in independent thread, it might be few ms out of sync - subtract extra 250ms for this
    const auto current_time_period = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now() - start_time - std::chrono::milliseconds{250});

    if (current_time_period <= kConf.network.ddos_protection.packets_stats_time_period_ms) {
      // Peer exceeded max allowed processing time for his packets
      if (peer_packets_stats.processing_duration_ > kConf.network.ddos_protection.peer_max_packets_processing_time_us) {
        LOG(log_er_) << "Ignored " << convertPacketTypeToString(packet_type) << " from " << node_id
                     << ". Peer's current packets processing time " << peer_packets_stats.processing_duration_.count()
                     << " us, max allowed processing time "
                     << kConf.network.ddos_protection.peer_max_packets_processing_time_us.count()
                     << " us. Peer will be disconnected";
        host->disconnect(node_id, dev::p2p::UserReason);
        return;
      }
    } else {
      LOG(log_wr_) << "Unable to validate peer's max allowed packets processing time due to invalid time period";
    }
  }

  // Check max allowed packets queue size
  if (kConf.network.ddos_protection.max_packets_queue_size &&
      tp_queue_size > kConf.network.ddos_protection.max_packets_queue_size) {
    // Queue size is over the limit
    handlePacketQueueOverLimit(host, node_id, tp_queue_size);
  } else {
    queue_over_limit_ = false;
    last_disconnect_number_of_peers_ = 0;
  }

  // TODO: we are making a copy here for each packet bytes(toBytes()), which is pretty significant. Check why RLP does
  //       not support move semantics so we can take advantage of it...
  auto packet_bytes = _r.data().toBytes();
  thread_pool_->push({version(), threadpool::PacketData(packet_type, node_id, std::move(packet_bytes))});
#else
  if (rust_network_shim_.queueIsFull()) {
    // Queue size is over the limit
    handlePacketQueueOverLimit(host, node_id, kConf.network.ddos_protection.max_packets_queue_size);
  } else {
    // Reset in case we marked full.
    queue_over_limit_ = false;
    last_disconnect_number_of_peers_ = 0;

    rust_network_shim_.ingestPacket(packet_type, node_id, _r);
  }
#endif
}

void TaraxaCapability::handlePacketQueueOverLimit(std::shared_ptr<dev::p2p::Host> host, dev::p2p::NodeID node_id,
                                                  size_t tp_queue_size) {
  if (!queue_over_limit_) {
    queue_over_limit_start_time_ = std::chrono::system_clock::now();
    queue_over_limit_ = true;
  }

  // Check if Queue is over the limit for queue_limit_time
  if ((std::chrono::system_clock::now() - queue_over_limit_start_time_) >
      kConf.network.ddos_protection.queue_limit_time) {
    // Only disconnect if there is more than peer_disconnect_interval since last disconnect
    if ((std::chrono::system_clock::now() - last_ddos_disconnect_time_) >
        kConf.network.ddos_protection.peer_disconnect_interval) {
      auto connected_peers = peers_state_->getAllPeers();
      last_disconnect_number_of_peers_ = connected_peers.size();
      last_ddos_disconnect_time_ = std::chrono::system_clock::now();
      // Always keep at least 5 connected peers
      if (connected_peers.size() > 5) {
        // Find peers with the highest processing time and disconnect
        std::pair<std::chrono::microseconds, dev::p2p::NodeID> peer_max_processing_time{std::chrono::microseconds(0),
                                                                                        dev::p2p::NodeID()};
        for (const auto &connected_peer : connected_peers) {
          const auto peer_packets_stats = connected_peer.second->getAllPacketsStatsCopy();
          if (peer_packets_stats.second.processing_duration_ > peer_max_processing_time.first) {
            peer_max_processing_time = {peer_packets_stats.second.processing_duration_, connected_peer.first};
          }
        }

        // Disconnect peer with the highest processing time
        LOG(log_er_) << "Max allowed packets queue size " << kConf.network.ddos_protection.max_packets_queue_size
                     << " exceeded: " << tp_queue_size << ". Peer with the highest processing time "
                     << peer_max_processing_time.second << " will be disconnected";
        host->disconnect(node_id, dev::p2p::UserReason);
        connected_peers.erase(node_id);
      }
    }
  }
}

inline bool TaraxaCapability::filterSyncIrrelevantPackets(SubprotocolPacketType packet_type) const {
  switch (packet_type) {
    case SubprotocolPacketType::kStatusPacket:
    case SubprotocolPacketType::kGetPbftSyncPacket:
    case SubprotocolPacketType::kPbftSyncPacket:
      return false;
    default:
      return true;
  }
}

const std::shared_ptr<PeersState> &TaraxaCapability::getPeersState() { return peers_state_; }

bool TaraxaCapability::sendStatus(const dev::p2p::NodeID &peer_id, bool initial) {
#ifdef RUSTAXA_ENABLE
  const auto handler = std::dynamic_pointer_cast<RustStatusPacketHandler>(
      packets_handlers_->getSpecificHandler(SubprotocolPacketType::kStatusPacket));
#else
  const auto handler = getSpecificHandler<ISyncPacketHandler>(SubprotocolPacketType::kStatusPacket);
#endif
  if (!handler) {
    throw std::runtime_error("Mode-selected status packet handler is unavailable");
  }
  return handler->sendStatus(peer_id, initial);
}

void TaraxaCapability::sendStatusToPeers() {
#ifdef RUSTAXA_ENABLE
  const auto handler = std::dynamic_pointer_cast<RustStatusPacketHandler>(
      packets_handlers_->getSpecificHandler(SubprotocolPacketType::kStatusPacket));
#else
  const auto handler = getSpecificHandler<ISyncPacketHandler>(SubprotocolPacketType::kStatusPacket);
#endif
  if (!handler) {
    throw std::runtime_error("Mode-selected status packet handler is unavailable");
  }
  handler->sendStatusToPeers();
}

void TaraxaCapability::startSyncingPbft() {
#ifdef RUSTAXA_ENABLE
  const auto handler = std::dynamic_pointer_cast<RustConsensusTransportPacketHandler>(
      packets_handlers_->getSpecificHandler(SubprotocolPacketType::kPbftSyncPacket));
#else
  const auto handler = getSpecificHandler<ISyncPacketHandler>(SubprotocolPacketType::kPbftSyncPacket);
#endif
  if (!handler) {
    throw std::runtime_error("Mode-selected PBFT sync operation is unavailable");
  }
  handler->startSyncingPbft();
}

#ifdef RUSTAXA_ENABLE
bool TaraxaCapability::sendCanonicalPillarVotesBundleRequest(const dev::p2p::NodeID &peer_id,
                                                             const std::vector<uint8_t> &packet_rlp) {
  auto host = peers_state_->host_.lock();
  if (!host) {
    LOG(log_er_) << "Unable to send native pillar-vote request: host is unavailable";
    return false;
  }
  constexpr auto packet_type = SubprotocolPacketType::kGetPillarVotesBundlePacket;
  if (const auto sender = peers_state_->getPacketSenderPeer(peer_id, packet_type); !sender.first) {
    LOG(log_wr_) << "Unable to send native pillar-vote request. Reason: " << sender.second;
    host->disconnect(peer_id, dev::p2p::UserReason);
    return false;
  }
  const auto begin = std::chrono::steady_clock::now();
  const auto packet_size = packet_rlp.size();
  host->send(peer_id, TARAXA_CAPABILITY_NAME, packet_type, dev::bytes(packet_rlp.begin(), packet_rlp.end()),
             [this, begin, packet_size, peer_id]() {
               if (!kConf.network.ddos_protection.log_packets_stats) {
                 return;
               }
               const PacketStats packet_stats{
                   1, packet_size,
                   std::chrono::duration_cast<std::chrono::microseconds>(std::chrono::steady_clock::now() - begin),
                   std::chrono::microseconds{0}};
               all_packets_stats_->addSentPacket(convertPacketTypeToString(packet_type), peer_id, packet_stats);
             });
  return true;
}
#endif

#ifdef RUSTAXA_ENABLE
network::ConsensusPacketOutcome TaraxaCapability::gossipCanonicalVote(const std::vector<uint8_t> &vote_rlp,
                                                                      const std::vector<uint8_t> &proposed_block_rlp,
                                                                      bool rebroadcast, uint64_t source_payload_id) {
  return std::dynamic_pointer_cast<RustVotePacketHandler>(
             packets_handlers_->getSpecificHandler(SubprotocolPacketType::kVotePacket))
      ->gossipCanonicalVote(vote_rlp, proposed_block_rlp, rebroadcast, source_payload_id);
}

network::ConsensusPacketOutcome TaraxaCapability::gossipCanonicalVotesBundle(
    const std::vector<uint8_t> &votes_bundle_rlp, bool rebroadcast, uint64_t source_payload_id) {
  return std::dynamic_pointer_cast<RustVotesBundlePacketHandler>(
             packets_handlers_->getSpecificHandler(SubprotocolPacketType::kVotesBundlePacket))
      ->gossipCanonicalVotesBundle(votes_bundle_rlp, rebroadcast, source_payload_id);
}

network::ConsensusPacketOutcome TaraxaCapability::gossipCanonicalPillarVote(const std::vector<uint8_t> &pillar_vote_rlp,
                                                                            bool rebroadcast,
                                                                            uint64_t source_payload_id) {
  return std::dynamic_pointer_cast<RustPillarVotePacketHandler>(
             packets_handlers_->getSpecificHandler(SubprotocolPacketType::kPillarVotePacket))
      ->gossipCanonicalPillarVote(pillar_vote_rlp, rebroadcast, source_payload_id);
}

network::ConsensusPacketOutcome TaraxaCapability::gossipCanonicalDagBlock(const std::vector<uint8_t> &block_rlp,
                                                                          const std::array<uint8_t, 32> &block_hash,
                                                                          uint64_t source_payload_id) {
  return std::dynamic_pointer_cast<RustDagBlockPacketHandler>(
             packets_handlers_->getSpecificHandler(SubprotocolPacketType::kDagBlockPacket))
      ->gossipCanonicalDagBlock(block_rlp, block_hash, source_payload_id);
}
#endif

const TaraxaCapability::InitPacketsHandlers TaraxaCapability::kInitLatestVersionHandlers =
    [](const std::string &logs_prefix, const FullNodeConfig &config, [[maybe_unused]] const h256 &genesis_hash,
       const std::shared_ptr<PeersState> &peers_state,
#ifndef RUSTAXA_ENABLE
       const std::shared_ptr<PbftSyncingState> &pbft_syncing_state,
#endif
       const std::shared_ptr<tarcap::TimePeriodPacketsStats> &packets_stats,
#ifdef RUSTAXA_ENABLE
       const network::ConsensusNetworkApiShared &consensus_network_api,
       const network::ConsensusLiveStatusProvider &consensus_status,
#endif
#ifndef RUSTAXA_ENABLE
       const std::shared_ptr<DbStorage> &db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy tarcap handler wiring.
       const std::shared_ptr<PbftManager> &pbft_mgr,
#endif
       const net::ConsensusQueryClient &pbft_chain,
#ifndef RUSTAXA_ENABLE
       const std::shared_ptr<VoteManager> &vote_mgr, const std::shared_ptr<DagManager> &dag_mgr,
       const std::shared_ptr<TransactionManager> &trx_mgr, const std::shared_ptr<SlashingManager> &slashing_manager,
       const std::shared_ptr<pillar_chain::PillarChainManager> &pillar_chain_mgr,
#endif
#ifdef RUSTAXA_ENABLE
       const SharedConsensusApplication &consensus_application,
#else
       const std::shared_ptr<final_chain::FinalChain> &final_chain,
#endif
       [[maybe_unused]] TarcapVersion version, const addr_t &node_addr) {
      auto packets_handlers = std::make_shared<PacketsHandler>();
      // Consensus packets with high processing priority
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustVotePacketHandler
#else
          VotePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pbft_mgr, pbft_chain, vote_mgr, slashing_manager,
#else
            consensus_status, pbft_chain,
#endif
#ifdef RUSTAXA_ENABLE
            consensus_network_api, version,
#endif
            node_addr, logs_prefix);
#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<GetNextVotesBundlePacketHandler>(
          config, peers_state, packets_stats, pbft_mgr, pbft_chain, vote_mgr, slashing_manager, node_addr, logs_prefix);
#else
      packets_handlers->registerHandler<RustGetNextVotesBundlePacketHandler>(
          config, peers_state, packets_stats, pbft_chain, consensus_status, consensus_network_api, version, node_addr,
          logs_prefix);
#endif
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustVotesBundlePacketHandler
#else
          VotesBundlePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pbft_mgr, pbft_chain, vote_mgr, slashing_manager,
#else
            consensus_status, pbft_chain,
#endif
#ifdef RUSTAXA_ENABLE
            consensus_network_api, version,
#endif
            node_addr, logs_prefix);

  // Standard packets with mid processing priority
#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<DagBlockPacketHandler>(config, peers_state, packets_stats, pbft_syncing_state,
                                                               pbft_chain, pbft_mgr, dag_mgr, trx_mgr, db, node_addr,
                                                               logs_prefix);
#else
      packets_handlers->registerHandler<RustDagBlockPacketHandler>(config, peers_state, packets_stats, pbft_chain,
                                                                   consensus_status, consensus_network_api, version,
                                                                   node_addr, logs_prefix);
#endif

#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<TransactionPacketHandler>(config, peers_state, packets_stats, trx_mgr,
                                                                  node_addr, logs_prefix);
#else
      packets_handlers->registerHandler<RustTransactionPacketHandler>(
          config, peers_state, packets_stats, consensus_network_api, version, node_addr, logs_prefix);
#endif

      // Non critical packets with low processing priority
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustStatusPacketHandler
#else
          StatusPacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pbft_syncing_state,
#endif
            pbft_chain,
#ifndef RUSTAXA_ENABLE
            pbft_mgr,
#else
            consensus_status,
#endif
#ifndef RUSTAXA_ENABLE
            dag_mgr, db,
#else
            consensus_network_api,
#endif
#ifndef RUSTAXA_ENABLE
            genesis_hash, node_addr, logs_prefix);
#else
            node_addr, logs_prefix);
#endif
#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<GetDagSyncPacketHandler>(config, peers_state, packets_stats, trx_mgr, dag_mgr,
                                                                 node_addr, logs_prefix);
#else
      packets_handlers->registerHandler<RustGetDagSyncPacketHandler>(
          config, peers_state, packets_stats, consensus_network_api, version, node_addr, logs_prefix);
#endif

#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<DagSyncPacketHandler>(config, peers_state, packets_stats, pbft_syncing_state,
                                                              pbft_chain, pbft_mgr, dag_mgr, trx_mgr, db, node_addr,
                                                              logs_prefix);
#else
      packets_handlers->registerHandler<RustDagSyncPacketHandler>(config, peers_state, packets_stats, pbft_chain,
                                                                  consensus_status, consensus_network_api, version,
                                                                  node_addr, logs_prefix);
#endif

#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<GetPbftSyncPacketHandler>(config, peers_state, packets_stats,
                                                                  pbft_syncing_state, pbft_mgr, pbft_chain, vote_mgr,
                                                                  db, node_addr, logs_prefix);
#else
      packets_handlers->registerHandler<RustGetPbftSyncPacketHandler>(
          config, peers_state, packets_stats, consensus_network_api, version, node_addr, logs_prefix);
#endif

#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<PbftSyncPacketHandler>(config, peers_state, packets_stats, pbft_syncing_state,
                                                               pbft_chain, pbft_mgr, dag_mgr, vote_mgr, db, node_addr,
                                                               logs_prefix);
#else
      packets_handlers->registerHandler<RustPbftSyncPacketHandler>(config, peers_state, packets_stats, pbft_chain,
                                                                   consensus_status, consensus_application,
                                                                   consensus_network_api, node_addr, logs_prefix);
#endif
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustPillarVotePacketHandler
#else
          PillarVotePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pillar_chain_mgr,
#else
            consensus_network_api, version,
#endif
            node_addr, logs_prefix);
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustGetPillarVotesBundlePacketHandler
#else
          GetPillarVotesBundlePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifdef RUSTAXA_ENABLE
            pbft_chain, consensus_status, consensus_network_api, version,
#else
            pillar_chain_mgr,
#endif
            node_addr, logs_prefix);
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustPillarVotesBundlePacketHandler
#else
          PillarVotesBundlePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pillar_chain_mgr,
#else
            consensus_network_api, version,
#endif
            node_addr, logs_prefix);

#ifdef RUSTAXA_ENABLE
      packets_handlers->registerHandler<RustPbftBlocksBundlePacketHandler>(
          config, peers_state, packets_stats, consensus_network_api, node_addr, logs_prefix);
#else
      packets_handlers->registerHandler<PbftBlocksBundlePacketHandler>(
          config, peers_state, packets_stats, pbft_mgr, final_chain, pbft_syncing_state, node_addr, logs_prefix);
#endif
      return packets_handlers;
    };

const TaraxaCapability::InitPacketsHandlers TaraxaCapability::kInitV5VersionHandlers =
    [](const std::string &logs_prefix, const FullNodeConfig &config, [[maybe_unused]] const h256 &genesis_hash,
       const std::shared_ptr<PeersState> &peers_state,
#ifndef RUSTAXA_ENABLE
       const std::shared_ptr<PbftSyncingState> &pbft_syncing_state,
#endif
       const std::shared_ptr<tarcap::TimePeriodPacketsStats> &packets_stats,
#ifdef RUSTAXA_ENABLE
       const network::ConsensusNetworkApiShared &consensus_network_api,
       const network::ConsensusLiveStatusProvider &consensus_status,
#endif
#ifndef RUSTAXA_ENABLE
       const std::shared_ptr<DbStorage> &db,  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy tarcap handler wiring.
       const std::shared_ptr<PbftManager> &pbft_mgr,
#endif
       const net::ConsensusQueryClient &pbft_chain,
#ifndef RUSTAXA_ENABLE
       const std::shared_ptr<VoteManager> &vote_mgr, const std::shared_ptr<DagManager> &dag_mgr,
       const std::shared_ptr<TransactionManager> &trx_mgr, const std::shared_ptr<SlashingManager> &slashing_manager,
       const std::shared_ptr<pillar_chain::PillarChainManager> &pillar_chain_mgr,
#endif
#ifdef RUSTAXA_ENABLE
       [[maybe_unused]] const SharedConsensusApplication &consensus_application,
#else
       [[maybe_unused]] const std::shared_ptr<final_chain::FinalChain> &final_chain,
#endif
       [[maybe_unused]] TarcapVersion version, const addr_t &node_addr) {
      auto packets_handlers = std::make_shared<PacketsHandler>();
      // Consensus packets with high processing priority
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustVotePacketHandler
#else
          VotePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pbft_mgr, pbft_chain, vote_mgr, slashing_manager,
#else
            consensus_status, pbft_chain,
#endif
#ifdef RUSTAXA_ENABLE
            consensus_network_api, version,
#endif
            node_addr, logs_prefix);
#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<GetNextVotesBundlePacketHandler>(
          config, peers_state, packets_stats, pbft_mgr, pbft_chain, vote_mgr, slashing_manager, node_addr, logs_prefix);
#else
      packets_handlers->registerHandler<RustGetNextVotesBundlePacketHandler>(
          config, peers_state, packets_stats, pbft_chain, consensus_status, consensus_network_api, version, node_addr,
          logs_prefix);
#endif
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustVotesBundlePacketHandler
#else
          VotesBundlePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pbft_mgr, pbft_chain, vote_mgr, slashing_manager,
#else
            consensus_status, pbft_chain,
#endif
#ifdef RUSTAXA_ENABLE
            consensus_network_api, version,
#endif
            node_addr, logs_prefix);

  // Standard packets with mid processing priority
#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<DagBlockPacketHandler>(config, peers_state, packets_stats, pbft_syncing_state,
                                                               pbft_chain, pbft_mgr, dag_mgr, trx_mgr, db, node_addr,
                                                               logs_prefix);
#else
      packets_handlers->registerHandler<RustDagBlockPacketHandler>(config, peers_state, packets_stats, pbft_chain,
                                                                   consensus_status, consensus_network_api, version,
                                                                   node_addr, logs_prefix);
#endif

#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<TransactionPacketHandler>(config, peers_state, packets_stats, trx_mgr,
                                                                  node_addr, logs_prefix);
#else
      packets_handlers->registerHandler<RustTransactionPacketHandler>(
          config, peers_state, packets_stats, consensus_network_api, version, node_addr, logs_prefix);
#endif

      // Non critical packets with low processing priority
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustStatusPacketHandler
#else
          StatusPacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pbft_syncing_state,
#endif
            pbft_chain,
#ifndef RUSTAXA_ENABLE
            pbft_mgr,
#else
            consensus_status,
#endif
#ifndef RUSTAXA_ENABLE
            dag_mgr, db,
#else
            consensus_network_api,
#endif
#ifndef RUSTAXA_ENABLE
            genesis_hash, node_addr, logs_prefix);
#else
            node_addr, logs_prefix);
#endif
#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<GetDagSyncPacketHandler>(config, peers_state, packets_stats, trx_mgr, dag_mgr,
                                                                 node_addr, logs_prefix);
#else
      packets_handlers->registerHandler<RustGetDagSyncPacketHandler>(
          config, peers_state, packets_stats, consensus_network_api, version, node_addr, logs_prefix);
#endif

#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<DagSyncPacketHandler>(config, peers_state, packets_stats, pbft_syncing_state,
                                                              pbft_chain, pbft_mgr, dag_mgr, trx_mgr, db, node_addr,
                                                              logs_prefix);
#else
      packets_handlers->registerHandler<RustDagSyncPacketHandler>(config, peers_state, packets_stats, pbft_chain,
                                                                  consensus_status, consensus_network_api, version,
                                                                  node_addr, logs_prefix);
#endif

#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<v4::GetPbftSyncPacketHandler>(config, peers_state, packets_stats,
                                                                      pbft_syncing_state, pbft_mgr, pbft_chain,
                                                                      vote_mgr, db, node_addr, logs_prefix);
#else
      packets_handlers->registerHandler<RustGetPbftSyncPacketHandler>(
          config, peers_state, packets_stats, consensus_network_api, version, node_addr, logs_prefix);
#endif

#ifndef RUSTAXA_ENABLE
      packets_handlers->registerHandler<PbftSyncPacketHandler>(config, peers_state, packets_stats, pbft_syncing_state,
                                                               pbft_chain, pbft_mgr, dag_mgr, vote_mgr, db, node_addr,
                                                               logs_prefix);
#else
      packets_handlers->registerHandler<RustPbftSyncPacketHandler>(config, peers_state, packets_stats, pbft_chain,
                                                                   consensus_status, consensus_application,
                                                                   consensus_network_api, node_addr, logs_prefix);
#endif
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustPillarVotePacketHandler
#else
          PillarVotePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pillar_chain_mgr,
#else
            consensus_network_api, version,
#endif
            node_addr, logs_prefix);
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustGetPillarVotesBundlePacketHandler
#else
          GetPillarVotesBundlePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifdef RUSTAXA_ENABLE
            pbft_chain, consensus_status, consensus_network_api, version,
#else
            pillar_chain_mgr,
#endif
            node_addr, logs_prefix);
      packets_handlers->registerHandler<
#ifdef RUSTAXA_ENABLE
          RustPillarVotesBundlePacketHandler
#else
          PillarVotesBundlePacketHandler
#endif
          >(config, peers_state, packets_stats,
#ifndef RUSTAXA_ENABLE
            pillar_chain_mgr,
#else
            consensus_network_api, version,
#endif
            node_addr, logs_prefix);
      return packets_handlers;
    };

#undef RUSTAXA_LEGACY_DB_ARG
#undef RUSTAXA_NETWORK_API_ARG

}  // namespace taraxa::network::tarcap
