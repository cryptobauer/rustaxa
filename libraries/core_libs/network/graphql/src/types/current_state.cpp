#include "graphql/types/current_state.hpp"

#include <exception>

namespace graphql::taraxa {

namespace {
#ifndef RUSTAXA_ENABLE
CurrentStateReader makeCurrentStateReader(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                                          std::shared_ptr<::taraxa::DagManager> dag_manager) {
  CurrentStateReader reader;
  reader.final_block = [final_chain = std::move(final_chain)] { return final_chain->lastBlockNumber(); };
  reader.dag_block_level = [dag_manager] { return dag_manager->getMaxLevel(); };
  reader.dag_block_period = [dag_manager = std::move(dag_manager)] { return dag_manager->getLatestPeriod(); };
  return reader;
}

void fillMissingCurrentStateReaderCallbacks(CurrentStateReader& reader,
                                            std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                                            std::shared_ptr<::taraxa::DagManager> dag_manager) {
  auto defaults = makeCurrentStateReader(std::move(final_chain), std::move(dag_manager));
  if (!reader.final_block) {
    reader.final_block = std::move(defaults.final_block);
  }
  if (!reader.dag_block_level) {
    reader.dag_block_level = std::move(defaults.dag_block_level);
  }
  if (!reader.dag_block_period) {
    reader.dag_block_period = std::move(defaults.dag_block_period);
  }
}
#endif
}  // namespace

#ifndef RUSTAXA_ENABLE
CurrentState::CurrentState(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                           std::shared_ptr<::taraxa::DagManager> dag_manager,
                           std::function<uint64_t()> final_block_query, std::function<uint64_t()> dag_block_level_query,
                           std::function<uint64_t()> dag_block_period_query) noexcept
    : reader_{std::move(final_block_query), std::move(dag_block_level_query), std::move(dag_block_period_query)} {
  fillMissingCurrentStateReaderCallbacks(reader_, std::move(final_chain), std::move(dag_manager));
}
#endif

CurrentState::CurrentState(CurrentStateReader reader) noexcept : reader_(std::move(reader)) {}

response::Value CurrentState::getFinalBlock() const noexcept {
  try {
    return response::Value(static_cast<int>(reader_.final_block()));
  } catch (const std::exception&) {
    return {};
  }
}

response::Value CurrentState::getDagBlockLevel() const noexcept {
  try {
    return response::Value(static_cast<int>(reader_.dag_block_level()));
  } catch (const std::exception&) {
    return {};
  }
}

response::Value CurrentState::getDagBlockPeriod() const noexcept {
  try {
    return response::Value(static_cast<int>(reader_.dag_block_period()));
  } catch (const std::exception&) {
    return {};
  }
}

}  // namespace graphql::taraxa
