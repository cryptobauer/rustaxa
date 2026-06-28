#include <config/config.hpp>
#include <cstdint>
#include <stdexcept>
#include <string>

#include "dag/sortition_params_manager.hpp"

namespace taraxa {
namespace {

struct PeriodEfficiencyCounts {
  bool has_pivot = false;
  uint64_t unique_transactions = 0;
  uint64_t total_dag_transaction_refs = 0;
};

rustaxa::SortitionRuntimeConfig to_rust_config(const SortitionConfig& config) {
  rustaxa::SortitionRuntimeConfig rust_config;
  rust_config.threshold_upper = config.vrf.threshold_upper;
  rust_config.difficulty_min = config.vdf.difficulty_min;
  rust_config.difficulty_max = config.vdf.difficulty_max;
  rust_config.difficulty_stale = config.vdf.difficulty_stale;
  rust_config.lambda_bound = config.vdf.lambda_bound;
  rust_config.changes_count_for_average = config.changes_count_for_average;
  rust_config.dag_efficiency_target_low = config.dag_efficiency_targets.first;
  rust_config.dag_efficiency_target_high = config.dag_efficiency_targets.second;
  rust_config.changing_interval = config.changing_interval;
  rust_config.computation_interval = config.computation_interval;
  return rust_config;
}

rustaxa::SortitionParamsChangePayload to_rust_change(const SortitionParamsChange& change) {
  rustaxa::SortitionParamsChangePayload rust_change;
  rust_change.period = change.period;
  rust_change.interval_efficiency = change.interval_efficiency;
  rust_change.threshold_upper = change.vrf_params.threshold_upper;
  return rust_change;
}

SortitionParamsChange from_rust_change(const rustaxa::SortitionParamsChangePayload& change) {
  return SortitionParamsChange{change.period, change.interval_efficiency, VrfParams{change.threshold_upper}};
}

SortitionParamsChange from_rust_change(const rustaxa::SortitionParamsChangeResult& change) {
  return SortitionParamsChange{change.period, change.interval_efficiency, VrfParams{change.threshold_upper}};
}

SortitionParams from_rust_params(const rustaxa::SortitionRuntimeParams& params) {
  return SortitionParams{
      VrfParams{params.threshold_upper},
      VdfParams{params.difficulty_min, params.difficulty_max, params.difficulty_stale, params.lambda_bound},
  };
}

void apply_rust_params(SortitionConfig& config, const rustaxa::SortitionRuntimeParams& params) {
  config.vrf.threshold_upper = params.threshold_upper;
  config.vdf.difficulty_min = params.difficulty_min;
  config.vdf.difficulty_max = params.difficulty_max;
  config.vdf.difficulty_stale = params.difficulty_stale;
  config.vdf.lambda_bound = params.lambda_bound;
}

std::deque<SortitionParamsChange> from_rust_changes(const rust::Vec<rustaxa::SortitionParamsChangePayload>& changes) {
  std::deque<SortitionParamsChange> out;
  for (const auto& change : changes) {
    out.push_back(from_rust_change(change));
  }
  return out;
}

PeriodEfficiencyCounts period_efficiency_counts(const PeriodData& block) {
  PeriodEfficiencyCounts counts;
  counts.has_pivot = block.pbft_blk->getPivotDagBlockHash() != kNullBlockHash;
  counts.unique_transactions = block.transactions.size();
  for (const auto& dag_block : block.dag_blocks) {
    counts.total_dag_transaction_refs += dag_block->getTrxs().size();
  }
  return counts;
}

rustaxa::PbftFinalizationExternalEffectReport makeSortitionFinalizationLiveReport(
    const rustaxa::PbftFinalizationStorageWritePlan&,
    const rustaxa::SortitionParamsChangeResult& outcome, uint16_t current_threshold_upper,
    uint64_t params_changes_count) {
  rustaxa::PbftFinalizationExternalEffectReport report{};
  report.success = true;
  report.status = 0;
  report.sortition_changed = outcome.changed;
  report.sortition_change_period = outcome.period;
  report.sortition_change_interval_efficiency = outcome.interval_efficiency;
  report.sortition_change_threshold_upper = outcome.threshold_upper;
  report.sortition_current_threshold_upper = current_threshold_upper;
  report.sortition_params_changes_count = params_changes_count;
  return report;
}

[[noreturn]] void throw_unimplemented_sortition_api(const char* api_name) {
  throw std::logic_error("SortitionParamsManager::" + std::string(api_name) + " is not implemented in Rust shim mode");
}

}  // namespace

SortitionParamsManager::SortitionParamsManager([[maybe_unused]] const addr_t& node_addr, const FullNodeConfig& config,
                                               std::shared_ptr<DbStorage> db)
    : kConfig(config),
      sortition_config_(config.genesis.sortition) {
  rust_sortition_params_manager_ = rustaxa::create_sortition_params_manager_from_storage(
      to_rust_config(sortition_config_), db->rustStorage());
  params_changes_ = from_rust_changes(rust_sortition_params_manager_.value()->sortition_params_changes());
  apply_rust_params(sortition_config_, rust_sortition_params_manager_.value()->sortition_current_params());
}

SortitionParams SortitionParamsManager::getSortitionParams(std::optional<PbftPeriod> for_period) const {
  if (!for_period.has_value()) {
    return from_rust_params(rust_sortition_params_manager_.value()->sortition_current_params());
  }

  return from_rust_params(rust_sortition_params_manager_.value()->sortition_params_for_period_from_storage(*for_period));
}

rustaxa::SortitionRuntimeParams SortitionParamsManager::rustSortitionParamsForRust(PbftPeriod for_period) const {
  return rust_sortition_params_manager_.value()->sortition_params_for_period_from_storage(for_period);
}

uint16_t SortitionParamsManager::calculateDagEfficiency(const PeriodData& block) const {
  const auto counts = period_efficiency_counts(block);
  const auto result = rust_sortition_params_manager_.value()->sortition_calculate_dag_efficiency(
      counts.unique_transactions, counts.total_dag_transaction_refs);
  if (!result.ok) {
    throw std::runtime_error(static_cast<std::string>(result.error));
  }
  return result.value;
}

void SortitionParamsManager::pbftBlockPushed(const PeriodData& block, Batch& batch,
                                             PbftPeriod non_empty_pbft_chain_size) {
  (void)batch;
  const auto counts = period_efficiency_counts(block);
  const auto period = block.pbft_blk->getPeriod();
  rust_sortition_params_manager_.value()->sortition_record_finalized_period_and_persist(
      period, counts.has_pivot, counts.unique_transactions, counts.total_dag_transaction_refs,
      non_empty_pbft_chain_size);
  params_changes_ = from_rust_changes(rust_sortition_params_manager_.value()->sortition_params_changes());
  apply_rust_params(sortition_config_, rust_sortition_params_manager_.value()->sortition_current_params());
}

std::optional<SortitionParamsChange> SortitionParamsManager::applyBlockForSortitionRuntime(
    const PeriodData& block, PbftPeriod non_empty_pbft_chain_size) {
  const auto counts = period_efficiency_counts(block);
  const auto period = block.pbft_blk->getPeriod();
  auto outcome = rust_sortition_params_manager_.value()->sortition_record_finalized_period(
      period, counts.has_pivot, counts.unique_transactions, counts.total_dag_transaction_refs,
      non_empty_pbft_chain_size);
  std::optional<SortitionParamsChange> params_change;
  if (outcome.changed) {
    params_change = from_rust_change(outcome);
  }
  params_changes_ = from_rust_changes(rust_sortition_params_manager_.value()->sortition_params_changes());
  apply_rust_params(sortition_config_, rust_sortition_params_manager_.value()->sortition_current_params());
  return params_change;
}

std::optional<SortitionParamsChange> SortitionParamsManager::prepareBlockForSortitionFinalization(
    const PeriodData& block, PbftPeriod non_empty_pbft_chain_size) {
  const auto counts = period_efficiency_counts(block);
  const auto period = block.pbft_blk->getPeriod();
  auto outcome = rust_sortition_params_manager_.value()->sortition_preview_finalized_period(
      period, counts.has_pivot, counts.unique_transactions, counts.total_dag_transaction_refs,
      non_empty_pbft_chain_size);
  if (outcome.changed) {
    return from_rust_change(outcome);
  }
  return std::nullopt;
}

rustaxa::PbftFinalizationExternalEffectReport SortitionParamsManager::commitPreparedBlockForSortitionFinalization(
    const PeriodData& block, PbftPeriod non_empty_pbft_chain_size,
    const std::optional<SortitionParamsChange>& prepared_change,
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent) {
  const auto counts = period_efficiency_counts(block);
  const auto period = block.pbft_blk->getPeriod();
  rustaxa::SortitionParamsChangePayload expected_change{};
  if (prepared_change.has_value()) {
    expected_change = to_rust_change(*prepared_change);
  }
  auto outcome = rust_sortition_params_manager_.value()->sortition_commit_finalized_period(
      period, counts.has_pivot, counts.unique_transactions, counts.total_dag_transaction_refs,
      non_empty_pbft_chain_size, prepared_change.has_value(), expected_change);
  params_changes_ = from_rust_changes(rust_sortition_params_manager_.value()->sortition_params_changes());
  const auto current_params = rust_sortition_params_manager_.value()->sortition_current_params();
  apply_rust_params(sortition_config_, current_params);
  return makeSortitionFinalizationLiveReport(write_intent, outcome, current_params.threshold_upper,
                                             params_changes_.size());
}

uint16_t SortitionParamsManager::averageDagEfficiency() {
  return rust_sortition_params_manager_.value()->sortition_average_dag_efficiency();
}

SortitionParamsChange SortitionParamsManager::calculateChange(PbftPeriod) {
  throw_unimplemented_sortition_api("calculateChange");
}

EfficienciesMap SortitionParamsManager::getEfficienciesToUpperRange(uint16_t, int32_t) const {
  throw_unimplemented_sortition_api("getEfficienciesToUpperRange");
}

int32_t SortitionParamsManager::getNewUpperRange(uint16_t) const {
  throw_unimplemented_sortition_api("getNewUpperRange");
}

void SortitionParamsManager::cleanup() { throw_unimplemented_sortition_api("cleanup"); }

}  // namespace taraxa
