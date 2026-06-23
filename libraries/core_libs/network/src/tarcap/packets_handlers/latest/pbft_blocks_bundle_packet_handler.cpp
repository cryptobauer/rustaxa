#include "network/tarcap/packets_handlers/latest/pbft_blocks_bundle_packet_handler.hpp"

#include <cassert>
#include <stdexcept>

#include "final_chain/final_chain.hpp"
#include "network/tarcap/packets/latest/pbft_blocks_bundle_packet.hpp"
#include "network/tarcap/shared_states/pbft_syncing_state.hpp"
#include "pbft/pbft_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindPbftBlock = 1;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

rust::Vec<uint8_t> toBridgeBytes(const bytes &input) {
  rust::Vec<uint8_t> output;
  output.reserve(input.size());
  for (const auto byte : input) {
    output.push_back(static_cast<uint8_t>(byte));
  }
  return output;
}

}  // namespace

struct PbftBlocksBundlePacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

PbftBlocksBundlePacketHandler::PbftBlocksBundlePacketHandler(const FullNodeConfig &conf,
                                                             std::shared_ptr<PeersState> peers_state,
                                                             std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                             std::shared_ptr<PbftManager> pbft_mgr,
                                                             std::shared_ptr<final_chain::FinalChain> final_chain,
                                                             std::shared_ptr<PbftSyncingState> syncing_state,
                                                             const addr_t &node_addr, const std::string &logs_prefix)
    : PacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                    logs_prefix + "PBFT_BLOCKS_BUNDLE_PH"),
      pbft_mgr_(std::move(pbft_mgr)),
      final_chain_(std::move(final_chain)),
      pbft_syncing_state_(syncing_state) {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

PbftBlocksBundlePacketHandler::~PbftBlocksBundlePacketHandler() = default;

void PbftBlocksBundlePacketHandler::process(const threadpool::PacketData &packet_data,
                                            const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<PbftBlocksBundlePacket>(packet_data.rlp_);

  if (packet.pbft_blocks.size() > kMaxBlocksInPacket) {
    throw InvalidRlpItemsCountException("PbftBlocksBundlePacket:pbft_blocks", packet.pbft_blocks.size(),
                                        kMaxBlocksInPacket);
  }

  std::unordered_map<PbftPeriod, std::unordered_set<addr_t>> unique_authors;
  if (!pbft_syncing_state_->lastSyncingPeer()) {
    LOG(log_er_) << "PbftBlocksBundlePacket received from unexpected peer " << peer->getId().abridged()
                 << " but there is no current syncing peer set";
    return;
  }
  if (pbft_syncing_state_->lastSyncingPeer()->getId() != peer->getId()) {
    LOG(log_er_) << "PbftBlocksBundlePacket received from unexpected peer " << peer->getId().abridged()
                 << " last syncing peer " << pbft_syncing_state_->lastSyncingPeer()->getId().abridged();
    // Note: do not throw MaliciousPeerException as in some edge cases node could be already syncing with new peer.
    // In such case we can simply ignore this packet
    return;
  }

  for (const auto &proposed_block : packet.pbft_blocks) {
    const auto proposed_block_period = proposed_block->getPeriod();
    const auto proposed_block_author = proposed_block->getBeneficiary();
    const auto current_pbft_period = pbft_mgr_->getPbftPeriod();

    // Check if proposed block period is relevant compared to the current node period
    if (proposed_block_period < current_pbft_period || proposed_block_period > current_pbft_period + 5) {
      // This should not happen as sender sends PbftBlocksBundlePacket only after he sends last sync packet and
      // PbftBlocksBundlePacket processing is blocked until sync_queue is empty
      LOG(log_er_)
          << "Unable to validate proposed blocks bundle as sync packets were not processed yet. Current chain size "
          << current_pbft_period << ", proposed block period " << proposed_block_period << ", proposed block hash "
          << proposed_block->getBlockHash();

      continue;
    }

    // Check if block author is unique per period
    if (!unique_authors[proposed_block_period].insert(proposed_block_author).second) {
      std::ostringstream err_msg;
      err_msg << "Proposed pbft blocks packet contains non-unique block author " << proposed_block_author;
      throw MaliciousPeerException(err_msg.str());
    }

    // Check if block author is dpos eligible
    if (final_chain_->lastBlockNumber() >= proposed_block_period - 1 &&
        !pbft_mgr_->canParticipateInConsensus(proposed_block_period - 1, proposed_block_author)) {
      std::ostringstream err_msg;
      err_msg << "Proposed pbft blocks packet contains non-eligible block author " << proposed_block_author
              << " for period " << proposed_block_period - 1;
      throw MaliciousPeerException(err_msg.str());
    }

#ifdef RUSTAXA_ENABLE
    rustaxa::NetworkPbftProposedBlockSidecarEffects effects{};
    effects.peer_id = peer->getId().asArray();
    effects.period = proposed_block_period;
    effects.block_hash = proposed_block->getBlockHash().asArray();
    effects.pivot_hash = proposed_block->getPivotDagBlockHash().asArray();
    effects.block_rlp = toBridgeBytes(proposed_block->rlp(true));
    effects.source_payload_id = 0;
    effects.record_block = true;
    (void)queuePbftProposedBlockBundleEffects(effects);
    executeConsensusNetworkEffects(16);
#else
    pbft_mgr_->processProposedBlock(proposed_block);
#endif
    LOG(log_dg_) << "Processed received proposed block: " << proposed_block->getBlockHash();
  }
}

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkIngressDecision PbftBlocksBundlePacketHandler::queuePbftProposedBlockBundleEffects(
    const rustaxa::NetworkPbftProposedBlockSidecarEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_pbft_proposed_block_bundle_effects(effects);
}

void PbftBlocksBundlePacketHandler::executeConsensusNetworkEffects(size_t budget) {
  assert(rust_consensus_network_api_);
  const auto batch = rust_consensus_network_api_->api->consensus_network_drain_work(static_cast<uint32_t>(budget));
  rust::Vec<rustaxa::NetworkEffectResult> results;
  results.reserve(batch.effects.size());

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
      if (effect.kind != kNetworkEffectKindRecordConsensusObject || effect.object_kind != kNetworkObjectKindPbftBlock) {
        throw std::runtime_error("Network API PBFT blocks bundle effect has unsupported kind");
      }

      auto proposed_block =
          std::make_shared<PbftBlock>(bytes(effect.payload_bytes.begin(), effect.payload_bytes.end()));
      if (proposed_block->getPeriod() != effect.period ||
          proposed_block->getBlockHash().asArray() != effect.object_hash) {
        throw std::runtime_error("Network API PBFT blocks bundle effect has mismatched block payload");
      }
      pbft_mgr_->processProposedBlock(proposed_block);
    } catch (const std::exception &e) {
      result.status = kNetworkEffectResultStatusFailed;
      result.diagnostic = e.what();
    }

    results.push_back(std::move(result));
  }

  if (!results.empty()) {
    (void)rust_consensus_network_api_->api->consensus_network_report_effect_results(std::move(results));
  }
}
#endif

}  // namespace taraxa::network::tarcap
