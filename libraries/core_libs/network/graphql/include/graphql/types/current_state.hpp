#pragma once

#include <cstdint>
#include <functional>
#include <memory>

#include "CurrentStateObject.h"
#include "dag/dag_manager.hpp"
#include "final_chain/final_chain.hpp"

namespace graphql::taraxa {

class CurrentState {
 public:
  explicit CurrentState(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                        std::shared_ptr<::taraxa::DagManager> dag_manager,
                        std::function<uint64_t()> final_block_query = {}) noexcept;

  response::Value getFinalBlock() const noexcept;
  response::Value getDagBlockLevel() const noexcept;
  response::Value getDagBlockPeriod() const noexcept;

 private:
  std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain_;
  std::shared_ptr<::taraxa::DagManager> dag_manager_;
  std::function<uint64_t()> final_block_query_;
};

}  // namespace graphql::taraxa
