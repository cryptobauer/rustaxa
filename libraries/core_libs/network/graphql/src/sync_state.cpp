
#include "graphql/sync_state.hpp"

#include <exception>

#include "network/tarcap/packets_handlers/latest/pbft_sync_packet_handler.hpp"

namespace graphql::taraxa {

namespace {
SyncStateReader makeSyncStateReader(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                                    std::weak_ptr<::taraxa::Network> network) {
  SyncStateReader reader;
  reader.current_block = [final_chain = std::move(final_chain)] { return final_chain->lastBlockNumber(); };
  reader.highest_block = [network = std::move(network)]() -> std::optional<uint64_t> {
    auto net = network.lock();
    if (!net) {
      return std::nullopt;
    }

    const auto peer = net->getMaxChainPeer();
    if (!peer) {
      return std::nullopt;
    }

    return peer->pbft_chain_size_.load();
  };
  return reader;
}

std::function<std::optional<uint64_t>()> makeHighestBlockReader(::taraxa::net::LiveStatusReader live_status) {
  return [live_status = std::move(live_status)]() -> std::optional<uint64_t> {
    const auto snapshot = live_status();
    return snapshot.max_peer_pbft_chain_size;
  };
}

void fillMissingSyncStateReaderCallbacks(SyncStateReader& reader,
                                         std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                                         std::weak_ptr<::taraxa::Network> network) {
  auto defaults = makeSyncStateReader(std::move(final_chain), std::move(network));
  if (!reader.current_block) {
    reader.current_block = std::move(defaults.current_block);
  }
  if (!reader.highest_block) {
    reader.highest_block = std::move(defaults.highest_block);
  }
}
}  // namespace

SyncState::SyncState(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                     std::weak_ptr<::taraxa::Network> network, std::function<uint64_t()> current_block_query,
                     ::taraxa::net::LiveStatusReader live_status) noexcept
    : reader_{std::move(current_block_query), live_status ? makeHighestBlockReader(std::move(live_status))
                                                          : std::function<std::optional<uint64_t>()>{}} {
  fillMissingSyncStateReaderCallbacks(reader_, std::move(final_chain), std::move(network));
}

SyncState::SyncState(SyncStateReader reader) noexcept : reader_(std::move(reader)) {}

response::Value SyncState::getStartingBlock() const noexcept { return response::Value(0); }

response::Value SyncState::getCurrentBlock() const noexcept {
  try {
    return response::Value(static_cast<int>(reader_.current_block()));
  } catch (const std::exception&) {
    return {};
  }
}

response::Value SyncState::getHighestBlock() const noexcept {
  try {
    const auto highest_block = reader_.highest_block();
    if (highest_block) {
      return response::Value(static_cast<int>(*highest_block));
    }
    return {};
  } catch (const std::exception&) {
    return {};
  }
}

std::optional<response::Value> SyncState::getPulledStates() const noexcept { return std::nullopt; }

std::optional<response::Value> SyncState::getKnownStates() const noexcept { return std::nullopt; }

}  // namespace graphql::taraxa
