#pragma once

#include <chrono>
#include <mutex>

#include "network/consensus_network_api.hpp"
#include "network/consensus_query.hpp"
#include "network/tarcap/packets_handlers/latest/common/packet_handler.hpp"

namespace taraxa::network::tarcap {

/**
 * Rust-mode packet-handler base containing only physical consensus transport leaves.
 *
 * Native consensus owns peer selection, sync policy, canonical payload construction, and effect ordering. This base
 * snapshots socket-owned peer facts and executes exact pending-DAG and next-vote sends selected by the application
 * network service. It deliberately exposes no bridge handle or consensus-object materializer.
 */
class RustConsensusTransportPacketHandler : public PacketHandler {
 public:
  RustConsensusTransportPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                                      std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                      net::ConsensusQueryClient consensus_query,
                                      network::ConsensusLiveStatusProvider consensus_status,
                                      network::ConsensusNetworkApiShared consensus_network_api, const addr_t& node_addr,
                                      const std::string& log_channel);
  ~RustConsensusTransportPacketHandler() override;

  RustConsensusTransportPacketHandler& operator=(const RustConsensusTransportPacketHandler&) = delete;
  RustConsensusTransportPacketHandler& operator=(RustConsensusTransportPacketHandler&&) = delete;

 protected:
  void requestPendingDagBlocks(std::shared_ptr<TaraxaPeer> peer = nullptr);
  bool requestPbftNextVotesAtPeriodRound(const dev::p2p::NodeID& peer_id, PbftPeriod peer_pbft_period,
                                         PbftRound peer_pbft_round);
  /** Returns the shared exact-id transport executor used by every native packet family. */
  network::ConsensusTransportExecutor consensusTransportExecutor();
  /** Snapshots immutable socket-owned eligibility/known facts for exact native object probes. */
  std::vector<network::ConsensusEgressPeerSnapshot> consensusEgressPeerSnapshots(
      const std::vector<network::ConsensusEgressProbe>& probes) const;
  /** Runs native prepare, peer snapshot, exact target planning, transport, and acknowledgement on one lane. */
  network::ConsensusPacketOutcome routeConsensusEgress(network::ConsensusEgressRequest request);
  /** Builds the plain native request shared by vote and pillar packet adapters. */
  network::ConsensusPacketRequest consensusPacketRequest(const threadpool::PacketData& packet_data,
                                                         const std::shared_ptr<TaraxaPeer>& peer,
                                                         uint32_t transport_lane,
                                                         bool validate_max_round_step) const;

  net::ConsensusQueryClient pbft_chain_;
  network::ConsensusLiveStatusProvider consensus_status_;
  network::ConsensusNetworkApiShared rust_consensus_network_api_;

 private:
  static constexpr auto kVoteSyncRequestInterval = std::chrono::seconds(10);
  bool tryReservePbftSyncRequest();
  bool tryReserveNextVotesSyncRequest();

  mutable std::mutex sync_request_mutex_;
  std::chrono::steady_clock::time_point last_pbft_sync_request_ = std::chrono::steady_clock::now();
  std::chrono::steady_clock::time_point last_next_votes_sync_request_ = std::chrono::steady_clock::now();
};

}  // namespace taraxa::network::tarcap
