#include "graphql/types/current_state.hpp"

#include <exception>

namespace graphql::taraxa {

CurrentState::CurrentState(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                           std::shared_ptr<::taraxa::DagManager> dag_manager,
                           std::function<uint64_t()> final_block_query, std::function<uint64_t()> dag_block_level_query,
                           std::function<uint64_t()> dag_block_period_query) noexcept
    : final_chain_(std::move(final_chain)),
      dag_manager_(std::move(dag_manager)),
      final_block_query_(std::move(final_block_query)),
      dag_block_level_query_(std::move(dag_block_level_query)),
      dag_block_period_query_(std::move(dag_block_period_query)) {}

response::Value CurrentState::getFinalBlock() const noexcept {
  if (final_block_query_) {
    try {
      return response::Value(static_cast<int>(final_block_query_()));
    } catch (const std::exception&) {
      return {};
    }
  }
  return response::Value(static_cast<int>(final_chain_->lastBlockNumber()));
}

response::Value CurrentState::getDagBlockLevel() const noexcept {
  if (dag_block_level_query_) {
    try {
      return response::Value(static_cast<int>(dag_block_level_query_()));
    } catch (const std::exception&) {
      return {};
    }
  }
  return response::Value(static_cast<int>(dag_manager_->getMaxLevel()));
}

response::Value CurrentState::getDagBlockPeriod() const noexcept {
  if (dag_block_period_query_) {
    try {
      return response::Value(static_cast<int>(dag_block_period_query_()));
    } catch (const std::exception&) {
      return {};
    }
  }
  return response::Value(static_cast<int>(dag_manager_->getLatestPeriod()));
}

}  // namespace graphql::taraxa
