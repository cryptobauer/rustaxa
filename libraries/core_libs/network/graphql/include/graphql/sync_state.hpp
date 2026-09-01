#pragma once

#include <cstdint>
#include <functional>
#include <memory>

#include "SyncStateObject.h"
#ifndef RUSTAXA_ENABLE
#include "final_chain/final_chain.hpp"
#endif
#include "network/live_status.hpp"
#include "network/network.hpp"

namespace graphql::taraxa {

// SyncStateReader is the GraphQL syncing boundary for public sync fields.
// Query wiring may adapt ConsensusQueryApi, LiveStatusReader, or legacy network
// state, but GraphQL resolvers consume only these callbacks.
struct SyncStateReader {
  std::function<uint64_t()> current_block;
  std::function<std::optional<uint64_t>()> highest_block;
};

class SyncState {
 public:
#ifndef RUSTAXA_ENABLE
  explicit SyncState(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                     std::weak_ptr<::taraxa::Network> network, std::function<uint64_t()> current_block_query = {},
                     ::taraxa::net::LiveStatusReader live_status = {}) noexcept;
#endif
  explicit SyncState(SyncStateReader reader) noexcept;

  response::Value getStartingBlock() const noexcept;
  response::Value getCurrentBlock() const noexcept;
  response::Value getHighestBlock() const noexcept;
  std::optional<response::Value> getPulledStates() const noexcept;
  std::optional<response::Value> getKnownStates() const noexcept;

 private:
  SyncStateReader reader_;
};

}  // namespace graphql::taraxa
