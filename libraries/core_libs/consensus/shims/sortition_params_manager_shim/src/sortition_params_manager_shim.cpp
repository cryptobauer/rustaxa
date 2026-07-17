#include <config/config.hpp>
#include <cstdint>
#include <stdexcept>
#include <string>

#include "dag/sortition_params_manager.hpp"

namespace taraxa {

SortitionParamsChange::SortitionParamsChange(PbftPeriod period, uint16_t efficiency, const VrfParams& vrf)
    : period(period), vrf_params(vrf), interval_efficiency(efficiency) {}

bytes SortitionParamsChange::rlp() const {
  dev::RLPStream s;
  s.appendList(3);
  s << vrf_params.threshold_upper;
  s << period;
  s << interval_efficiency;

  return s.invalidate();
}

SortitionParamsChange SortitionParamsChange::from_rlp(const dev::RLP& rlp) {
  SortitionParamsChange p;

  p.vrf_params.threshold_upper = rlp[0].toInt<uint16_t>();
  p.period = rlp[1].toInt<PbftPeriod>();
  p.interval_efficiency = rlp[2].toInt<uint16_t>();

  return p;
}

namespace {

struct PeriodEfficiencyCounts {
  bool has_pivot = false;
  uint64_t unique_transactions = 0;
  uint64_t total_dag_transaction_refs = 0;
};

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

[[noreturn]] void throw_unimplemented_sortition_api(const char* api_name) {
  throw std::logic_error("SortitionParamsManager::" + std::string(api_name) + " is not implemented in Rust shim mode");
}

}  // namespace

SortitionParamsManager::SortitionParamsManager([[maybe_unused]] const addr_t& node_addr, const FullNodeConfig& config,
                                               std::shared_ptr<DbStorage> db)
    : SortitionParamsManager(node_addr, config, db, createDagTransactionService(config, *db)) {}

SortitionParamsManager::SortitionParamsManager([[maybe_unused]] const addr_t& node_addr, const FullNodeConfig& config,
                                               std::shared_ptr<DbStorage> db,
                                               SharedDagTransactionService dag_transaction_service)
    : kConfig(config), sortition_config_(config.genesis.sortition) {
  static_cast<void>(db);
  dag_transaction_service_ = std::move(dag_transaction_service);
  if (!dag_transaction_service_) {
    throw std::invalid_argument("SortitionParamsManager requires a DAG/transaction service");
  }
  if (!dag_transaction_service_->service().dag_transaction_service_has_sortition()) {
    throw std::invalid_argument("SortitionParamsManager requires a DAG/transaction service with sortition state");
  }
  params_changes_ = from_rust_changes(dag_transaction_service_->service().sortition_params_changes());
  apply_rust_params(sortition_config_, dag_transaction_service_->service().sortition_current_params());
}

SortitionParams SortitionParamsManager::getSortitionParams(std::optional<PbftPeriod> for_period) const {
  if (!for_period.has_value()) {
    return from_rust_params(dag_transaction_service_->service().sortition_current_params());
  }

  return from_rust_params(dag_transaction_service_->service().sortition_params_for_period_from_storage(*for_period));
}

rustaxa::SortitionRuntimeParams SortitionParamsManager::rustSortitionParamsForRust(PbftPeriod for_period) const {
  return dag_transaction_service_->service().sortition_params_for_period_from_storage(for_period);
}

uint16_t SortitionParamsManager::calculateDagEfficiency(const PeriodData& block) const {
  const auto counts = period_efficiency_counts(block);
  const auto result = dag_transaction_service_->service().sortition_calculate_dag_efficiency(
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
  dag_transaction_service_->service().sortition_record_finalized_period_and_persist(
      period, counts.has_pivot, counts.unique_transactions, counts.total_dag_transaction_refs,
      non_empty_pbft_chain_size);
  params_changes_ = from_rust_changes(dag_transaction_service_->service().sortition_params_changes());
  apply_rust_params(sortition_config_, dag_transaction_service_->service().sortition_current_params());
}

std::optional<SortitionParamsChange> SortitionParamsManager::prepareBlockForSortitionFinalization(
    const PeriodData& block, PbftPeriod non_empty_pbft_chain_size) {
  const auto counts = period_efficiency_counts(block);
  const auto period = block.pbft_blk->getPeriod();
  auto outcome = dag_transaction_service_->service().sortition_preview_finalized_period(
      period, counts.has_pivot, counts.unique_transactions, counts.total_dag_transaction_refs,
      non_empty_pbft_chain_size);
  if (outcome.changed) {
    return from_rust_change(outcome);
  }
  return std::nullopt;
}

uint16_t SortitionParamsManager::averageDagEfficiency() {
  return dag_transaction_service_->service().sortition_average_dag_efficiency();
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
