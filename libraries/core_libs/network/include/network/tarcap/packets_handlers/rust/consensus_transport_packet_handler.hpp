#pragma once

#include "network/consensus_network_api.hpp"
#include "network/consensus_query.hpp"
#include "network/tarcap/packets_handlers/latest/common/packet_handler.hpp"

namespace taraxa::network::tarcap {

using PbftSyncStopExecutor = std::function<PbftSyncLifecycleOutcome(uint64_t, const std::array<uint8_t, 64>&, uint8_t)>;

/**
 * Executes one started native PBFT-sync request and rolls its generation back on every physical-send failure.
 *
 * The supplied start report and effect must identify the same peer, period, and non-empty canonical packet. The
 * physical executor performs only the exact send. Failure or an exception stops the generation with the stable
 * transport-failed reason before returning a typed failure; successful sends leave the generation active.
 */
ConsensusTransportExecutionResult executePbftSyncTransportRequest(const ConsensusTransportEffect& effect,
                                                                  const PbftSyncStartOutcome& started,
                                                                  const PbftSyncStopExecutor& stop_sync,
                                                                  const ConsensusTransportExecutor& transport);

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

  /** Selects and starts one native PBFT-sync generation, then executes its exact initial request. */
  void startSyncingPbft();

 protected:
  /** Sends one exact PBFT-sync request to the peer selected by the active native generation. */
  bool syncPeerPbft(PbftPeriod request_period);
  void requestPendingDagBlocks(std::shared_ptr<TaraxaPeer> peer = nullptr);
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
                                                         uint32_t transport_lane, bool validate_max_round_step) const;

  net::ConsensusQueryClient pbft_chain_;
  network::ConsensusLiveStatusProvider consensus_status_;
  network::ConsensusNetworkApiShared rust_consensus_network_api_;
};

}  // namespace taraxa::network::tarcap
