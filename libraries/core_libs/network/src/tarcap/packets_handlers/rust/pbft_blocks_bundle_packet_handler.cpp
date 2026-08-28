#include "network/tarcap/packets_handlers/rust/pbft_blocks_bundle_packet_handler.hpp"

#include "final_chain/final_chain.hpp"

namespace taraxa::network::tarcap {

RustPbftBlocksBundlePacketHandler::RustPbftBlocksBundlePacketHandler(
    const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
    std::shared_ptr<TimePeriodPacketsStats> packets_stats, network::ConsensusNetworkApiShared consensus_network_api,
    std::shared_ptr<final_chain::FinalChain> final_chain, const addr_t& node_addr, const std::string& logs_prefix)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                    logs_prefix + "PBFT_BLOCKS_BUNDLE_PH"),
      consensus_network_api_(std::move(consensus_network_api)),
      final_chain_(std::move(final_chain)) {}

RustPbftBlocksBundlePacketHandler::~RustPbftBlocksBundlePacketHandler() = default;

void RustPbftBlocksBundlePacketHandler::process(const threadpool::PacketData& packet_data,
                                                const std::shared_ptr<TaraxaPeer>& peer) {
  const auto source_outcome = consensus_network_api_->admitPbftSyncSource(peer->getId().asArray(),
                                                                          network::PbftSyncResponseSource::kMostRecent);
  if (!source_outcome.accepted) {
    LOG(log_er_) << "PbftBlocksBundlePacket received from unexpected peer " << peer->getId().abridged() << ": "
                 << static_cast<std::string>(source_outcome.error_code);
    // Note: do not throw MaliciousPeerException as in some edge cases node could be already syncing with new peer.
    // In such case we can simply ignore this packet.
    return;
  }

  const auto packet_rlp = packet_data.rlp_.data().toBytes();
  const auto outcome = consensus_network_api_->admitPbftBlocksBundle(packet_rlp, packet_data.id_);
  if (outcome.status != 0) {
    throw MaliciousPeerException(outcome.error_code);
  }
}

}  // namespace taraxa::network::tarcap
