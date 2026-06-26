#pragma once

#include <cstdint>
#include <functional>
#include <memory>

#include "SyncStateObject.h"
#include "final_chain/final_chain.hpp"
#include "network/live_status.hpp"
#include "network/network.hpp"

namespace graphql::taraxa {

class SyncState {
 public:
  explicit SyncState(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                     std::weak_ptr<::taraxa::Network> network, std::function<uint64_t()> current_block_query = {},
                     ::taraxa::net::LiveStatusReader live_status = {}) noexcept;

  response::Value getStartingBlock() const noexcept;
  response::Value getCurrentBlock() const noexcept;
  response::Value getHighestBlock() const noexcept;
  std::optional<response::Value> getPulledStates() const noexcept;
  std::optional<response::Value> getKnownStates() const noexcept;

 private:
  std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain_;
  std::weak_ptr<::taraxa::Network> network_;
  std::function<uint64_t()> current_block_query_;
  ::taraxa::net::LiveStatusReader live_status_;
};

}  // namespace graphql::taraxa
