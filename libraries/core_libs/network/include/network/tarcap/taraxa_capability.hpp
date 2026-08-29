#pragma once

#include <libp2p/Capability.h>
#include <libp2p/Common.h>
#include <libp2p/Host.h>
#include <libp2p/Session.h>

#include <memory>

#include "common/thread_pool.hpp"
#include "config/config.hpp"
#include "network/consensus_query.hpp"
#include "network/tarcap/packets_handler.hpp"
#include "network/tarcap/shared_states/peers_state.hpp"
#include "network/tarcap/tarcap_version.hpp"
#include "network/threadpool/tarcap_thread_pool.hpp"
#ifdef RUSTAXA_ENABLE
#include "network/consensus_network_api.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "slashing_manager/slashing_manager.hpp"
#endif

namespace taraxa {
#ifndef RUSTAXA_ENABLE
class DbStorage;
class PbftManager;
#endif
class PbftChain;
class VoteManager;
class DagManager;
class TransactionManager;
#ifndef RUSTAXA_ENABLE
class SlashingManager;
#endif
enum class TransactionStatus;

namespace pillar_chain {
#ifndef RUSTAXA_ENABLE
class PillarChainManager;
#endif
}  // namespace pillar_chain

namespace final_chain {
class FinalChain;
}

}  // namespace taraxa

namespace taraxa::network::tarcap {

class ISyncPacketHandler;
class IVotePacketHandler;
class IPillarVotePacketHandler;
#ifndef RUSTAXA_ENABLE
class IGetPillarVotesBundlePacketHandler;
#endif
class ITransactionPacketHandler;
class IDagBlockPacketHandler;

class PbftSyncingState;
class TaraxaPeer;

class TaraxaCapability final : public dev::p2p::CapabilityFace {
 public:
  /**
   * @brief Function signature for creating taraxa capability packets handlers
   */
  using InitPacketsHandlers = std::function<std::shared_ptr<PacketsHandler>(
      const std::string &logs_prefix, const FullNodeConfig &config, const h256 &genesis_hash,
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
      const std::shared_ptr<final_chain::FinalChain> &final_chain, TarcapVersion version, const addr_t &node_addr)>;

  /**
   * @brief Default InitPacketsHandlers function definition with the latest version of packets handlers
   */
  static const InitPacketsHandlers kInitLatestVersionHandlers;
  static const InitPacketsHandlers kInitV5VersionHandlers;

 public:
  TaraxaCapability(TarcapVersion version, const FullNodeConfig &conf, const h256 &genesis_hash,
                   std::weak_ptr<dev::p2p::Host> host,
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
                   std::shared_ptr<final_chain::FinalChain> final_chain,
#ifdef RUSTAXA_ENABLE
                   network::ConsensusNetworkApiShared consensus_network_api,
#endif
                   InitPacketsHandlers init_packets_handlers = kInitLatestVersionHandlers);

  virtual ~TaraxaCapability();
  TaraxaCapability(const TaraxaCapability &ro) = delete;
  TaraxaCapability &operator=(const TaraxaCapability &ro) = delete;
  TaraxaCapability(TaraxaCapability &&ro) = delete;
  TaraxaCapability &operator=(TaraxaCapability &&ro) = delete;

  // CapabilityFace implemented interface
  std::string name() const override;
  TarcapVersion version() const override;
  unsigned messageCount() const override;
  void onConnect(std::weak_ptr<dev::p2p::Session> session, u256 const &) override;
  void onDisconnect(dev::p2p::NodeID const &_nodeID) override;
  void interpretCapabilityPacket(std::weak_ptr<dev::p2p::Session> session, unsigned _id, dev::RLP const &_r) override;
  std::string packetTypeToString(unsigned _packetType) const override;

  const std::shared_ptr<PeersState> &getPeersState();

  /** Sends one initial status packet through the mode-selected status operation. */
  bool sendStatus(const dev::p2p::NodeID &peer_id, bool initial);
  /** Sends periodic status packets through the mode-selected status operation. */
  void sendStatusToPeers();
  /** Starts PBFT synchronization through the mode-selected lifecycle operation. */
  void startSyncingPbft();

#ifdef RUSTAXA_ENABLE
  /** Routes one canonical PBFT vote through this capability's exact-target native egress operation. */
  network::ConsensusPacketOutcome gossipCanonicalVote(const std::vector<uint8_t> &vote_rlp,
                                                      const std::vector<uint8_t> &proposed_block_rlp, bool rebroadcast,
                                                      uint64_t source_payload_id);
  /** Routes one canonical optimized PBFT bundle through exact-target native egress. */
  network::ConsensusPacketOutcome gossipCanonicalVotesBundle(const std::vector<uint8_t> &votes_bundle_rlp,
                                                             bool rebroadcast, uint64_t source_payload_id);
  /** Routes one canonical pillar vote through exact-target native egress. */
  network::ConsensusPacketOutcome gossipCanonicalPillarVote(const std::vector<uint8_t> &pillar_vote_rlp,
                                                            bool rebroadcast, uint64_t source_payload_id);
  /** Routes one canonical DAG block through exact-target native egress. */
  network::ConsensusPacketOutcome gossipCanonicalDagBlock(const std::vector<uint8_t> &block_rlp,
                                                          const std::array<uint8_t, 32> &block_hash,
                                                          uint64_t source_payload_id);
  /** Physically sends one already-canonical pillar-vote request to an exact peer. */
  bool sendCanonicalPillarVotesBundleRequest(const dev::p2p::NodeID &peer_id, const std::vector<uint8_t> &packet_rlp);
#endif

  /**
   * @brief templated getSpecificHandler method for getting specific packet handler based on packet_type
   *
   * @tparam PacketHandlerType
   *
   * @return std::shared_ptr<PacketHandlerType>
   */
  template <typename PacketHandlerType>
  std::shared_ptr<PacketHandlerType> getSpecificHandler(SubprotocolPacketType packet_type) const;

 private:
  bool filterSyncIrrelevantPackets(SubprotocolPacketType packet_type) const;
  void handlePacketQueueOverLimit(std::shared_ptr<dev::p2p::Host> host, dev::p2p::NodeID node_id, size_t tp_queue_size);

 private:
  // Capability version
  TarcapVersion version_;

  // Packets stats per time period
  std::shared_ptr<TimePeriodPacketsStats> all_packets_stats_;

  // Node config
  const FullNodeConfig &kConf;

  // Peers state
  std::shared_ptr<PeersState> peers_state_;

#ifndef RUSTAXA_ENABLE
  // Untouched pure-C++ synchronization state.
  std::shared_ptr<PbftSyncingState> pbft_syncing_state_;
#endif

  // Packets handlers
  std::shared_ptr<PacketsHandler> packets_handlers_;

  // Main Threadpool for processing packets
  std::shared_ptr<threadpool::PacketsThreadPool> thread_pool_;

  // Last disconnect time and number of peers
  std::chrono::system_clock::time_point last_ddos_disconnect_time_ = {};
  std::chrono::system_clock::time_point queue_over_limit_start_time_ = {};
  bool queue_over_limit_ = false;
  uint32_t last_disconnect_number_of_peers_ = 0;

#ifdef RUSTAXA_ENABLE
  network::ConsensusNetworkApiShared rust_consensus_network_api_;
#endif

  LOG_OBJECTS_DEFINE
};

template <typename PacketHandlerType>
std::shared_ptr<PacketHandlerType> TaraxaCapability::getSpecificHandler(SubprotocolPacketType packet_type) const {
  // Note: Allow to manually cast only to known base classes types.
  // We support multiple taraxa capabilities, which can contain different versions of packet handlers and casting
  // directly to final classes types breaks the functionality...
  switch (packet_type) {
    case SubprotocolPacketType::kPbftSyncPacket:
    case SubprotocolPacketType::kStatusPacket:
      if (!std::is_same<ISyncPacketHandler, PacketHandlerType>::value) {
        assert(false);
      }
      break;

    case SubprotocolPacketType::kTransactionPacket:
      if (!std::is_same<ITransactionPacketHandler, PacketHandlerType>::value) {
        assert(false);
      }
      break;

    case SubprotocolPacketType::kVotePacket:
    case SubprotocolPacketType::kVotesBundlePacket:
      if (!std::is_same<IVotePacketHandler, PacketHandlerType>::value) {
        assert(false);
      }
      break;

    case SubprotocolPacketType::kPillarVotePacket:
      if (!std::is_same<IPillarVotePacketHandler, PacketHandlerType>::value) {
        assert(false);
      }
      break;

#ifndef RUSTAXA_ENABLE
    case SubprotocolPacketType::kGetPillarVotesBundlePacket:
      if (!std::is_same<IGetPillarVotesBundlePacketHandler, PacketHandlerType>::value) {
        assert(false);
      }
      break;
#endif

    case SubprotocolPacketType::kDagBlockPacket:
      if (!std::is_same<IDagBlockPacketHandler, PacketHandlerType>::value) {
        assert(false);
      }
      break;

    default:
      assert(false);
      return nullptr;
  }

  auto handler = packets_handlers_->getSpecificHandler(packet_type);
  return std::dynamic_pointer_cast<PacketHandlerType>(handler);
}

}  // namespace taraxa::network::tarcap
