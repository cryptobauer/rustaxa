
#include "graphql/sync_state.hpp"

#include <exception>

#include "network/tarcap/packets_handlers/latest/pbft_sync_packet_handler.hpp"

namespace graphql::taraxa {

SyncState::SyncState(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                     std::weak_ptr<::taraxa::Network> network, std::function<uint64_t()> current_block_query) noexcept
    : final_chain_(std::move(final_chain)),
      network_(std::move(network)),
      current_block_query_(std::move(current_block_query)) {}

response::Value SyncState::getStartingBlock() const noexcept { return response::Value(0); }

response::Value SyncState::getCurrentBlock() const noexcept {
  if (current_block_query_) {
    try {
      return response::Value(static_cast<int>(current_block_query_()));
    } catch (const std::exception&) {
      return {};
    }
  }
  return response::Value(static_cast<int>(final_chain_->lastBlockNumber()));
}

response::Value SyncState::getHighestBlock() const noexcept {
  auto net = network_.lock();
  if (!net) {
    return {};
  }

  const auto peer = net->getMaxChainPeer();
  if (!peer) {
    return {};
  }

  return response::Value(static_cast<int>(peer->pbft_chain_size_));
}

std::optional<response::Value> SyncState::getPulledStates() const noexcept { return std::nullopt; }

std::optional<response::Value> SyncState::getKnownStates() const noexcept { return std::nullopt; }

}  // namespace graphql::taraxa
