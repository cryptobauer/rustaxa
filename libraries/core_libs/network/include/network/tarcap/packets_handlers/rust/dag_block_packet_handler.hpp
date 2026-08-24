#pragma once

#include "network/consensus_network_api.hpp"
#include "network/tarcap/packets_handlers/interface/dag_block_packet_handler.hpp"
#include "network/tarcap/tarcap_version.hpp"

namespace taraxa::network::tarcap {

/** Rust-mode DAG-block packet transport adapter over the application-root DAG ingress operation. */
class RustDagBlockPacketHandler final : public IDagBlockPacketHandler {
 public:
  RustDagBlockPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                            std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                            std::shared_ptr<PbftSyncingState> pbft_syncing_state,
                            net::ConsensusQueryClient consensus_query,
                            network::ConsensusLiveStatusProvider consensus_status,
                            network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
                            const addr_t& node_addr, const std::string& logs_prefix = "");

  void sendBlockWithTransactions(const std::shared_ptr<TaraxaPeer>& peer, const std::shared_ptr<DagBlock>& block,
                                 SharedTransactions&& transactions) override;

  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kDagBlockPacket;

 private:
  void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

  network::ConsensusNetworkApiShared consensus_network_api_;
  const TarcapVersion transport_lane_;
};

}  // namespace taraxa::network::tarcap
