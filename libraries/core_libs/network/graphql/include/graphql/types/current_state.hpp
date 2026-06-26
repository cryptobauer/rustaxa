#pragma once

#include <cstdint>
#include <functional>
#include <memory>

#include "CurrentStateObject.h"
#include "dag/dag_manager.hpp"
#include "final_chain/final_chain.hpp"

namespace graphql::taraxa {

// CurrentStateReader is the GraphQL node-state boundary for finalized and DAG
// head facts. Query wiring may back it with ConsensusQueryApi or legacy manager
// adapters, but field resolvers consume only these scalar callbacks.
struct CurrentStateReader {
  std::function<uint64_t()> final_block;
  std::function<uint64_t()> dag_block_level;
  std::function<uint64_t()> dag_block_period;
};

class CurrentState {
 public:
  explicit CurrentState(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                        std::shared_ptr<::taraxa::DagManager> dag_manager,
                        std::function<uint64_t()> final_block_query = {},
                        std::function<uint64_t()> dag_block_level_query = {},
                        std::function<uint64_t()> dag_block_period_query = {}) noexcept;
  explicit CurrentState(CurrentStateReader reader) noexcept;

  response::Value getFinalBlock() const noexcept;
  response::Value getDagBlockLevel() const noexcept;
  response::Value getDagBlockPeriod() const noexcept;

 private:
  CurrentStateReader reader_;
};

}  // namespace graphql::taraxa
