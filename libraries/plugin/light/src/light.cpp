#include "plugin/light.hpp"

#include "common/config_exception.hpp"
#include "config/config.hpp"
#include "final_chain/final_chain.hpp"
#ifndef RUSTAXA_ENABLE
#include "dag/dag_manager.hpp"
#endif

#ifdef RUSTAXA_ENABLE
#include "consensus/consensus_application.hpp"
#include "network/consensus_query.hpp"
#endif

namespace taraxa::plugin {

namespace bpo = boost::program_options;
constexpr auto HISTORY = "light.history";
constexpr auto NO_STATE_DB_PRUNING = "light.no_state_db_pruning";
constexpr auto NO_LIVE_CLEANUP = "light.no_live_cleanup";

namespace {
#ifndef RUSTAXA_ENABLE
void clearNonBlockData(const std::shared_ptr<DbStorage>& db, PbftPeriod start, PbftPeriod end, bool live_cleanup,
                       uint64_t periods_to_keep_non_block_data);
void recreateNonBlockData(const std::shared_ptr<DbStorage>& db, PbftPeriod last_block_number,
                          uint64_t periods_to_keep_non_block_data);
#endif

LightHistoryApi makeLightHistoryApi(std::weak_ptr<AppBase> app) {
  LightHistoryApi api;
  api.subscribe_finalized_block = [app](std::function<void()> callback, std::shared_ptr<util::ThreadPool> executor) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("LIGHT_HISTORY_API_APP_EXPIRED");
    }
    node->getFinalChain()->block_finalized_.subscribe(
        [callback = std::move(callback)](std::shared_ptr<final_chain::FinalizationResult>) { callback(); },
        std::move(executor));
  };
  api.history_facts = [app] {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("LIGHT_HISTORY_API_APP_EXPIRED");
    }
#ifdef RUSTAXA_ENABLE
    const auto query_api = node->getConsensusApplication()->queryClient();
    if (!query_api) {
      throw std::runtime_error("LIGHT_HISTORY_API_QUERY_UNAVAILABLE");
    }
    const auto status = (*query_api)->consensus_query_live_dag_status();
    return LightHistoryFacts{status.period, status.expiry_level, node->getConfig().max_levels_per_period};
#else
    auto dag_manager = node->getDagManager();
    return LightHistoryFacts{static_cast<uint64_t>(dag_manager->getLatestPeriod()), dag_manager->getDagExpiryLevel(),
                             dag_manager->getMaxLevelsPerPeriod()};
#endif
  };
  api.proposal_period_for_dag_level = [app](uint64_t dag_level) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("LIGHT_HISTORY_API_APP_EXPIRED");
    }
#ifdef RUSTAXA_ENABLE
    const auto query_api = node->getConsensusApplication()->queryClient();
    if (!query_api) {
      throw std::runtime_error("LIGHT_HISTORY_API_QUERY_UNAVAILABLE");
    }
    const auto lookup = (*query_api)->consensus_query_proposal_period_for_dag_level(dag_level);
    if (!lookup.found) {
      return std::optional<uint64_t>{};
    }
    return std::optional<uint64_t>{lookup.value};
#else
    return node->getDB()->getProposalPeriodForDagLevel(dag_level);
#endif
  };
  api.clear_history = [app](PbftPeriod end_period, uint64_t dag_level_to_keep, bool live_cleanup,
                            uint64_t periods_to_keep_non_block_data) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("LIGHT_HISTORY_API_APP_EXPIRED");
    }
#ifdef RUSTAXA_ENABLE
    node->getConsensusApplication()->pruneLightHistory(end_period, dag_level_to_keep, live_cleanup,
                                                       periods_to_keep_non_block_data);
#else
    auto db = node->getDB();
    auto it = db->getColumnIterator(DbStorage::Columns::period_data);
    // Find the first non-deleted period
    it->SeekToFirst();
    if (!it->Valid()) {
      return;
    }

    uint64_t start_period;
    memcpy(&start_period, it->key().data(), sizeof(uint64_t));
    if (start_period >= end_period) {
      return;
    }
    clearNonBlockData(db, start_period, end_period, live_cleanup, periods_to_keep_non_block_data);

    db->DeleteRange(DbStorage::Columns::period_data, start_period, end_period);
    db->DeleteRange(DbStorage::Columns::pillar_block, start_period, end_period);
    db->DeleteRange(DbStorage::Columns::final_chain_receipt_by_period, start_period, end_period);
    db->DeleteRange(DbStorage::Columns::period_lambda, start_period, end_period);
    db->CompactRange(DbStorage::Columns::period_data, start_period, end_period);
    db->CompactRange(DbStorage::Columns::pillar_block, start_period, end_period);
    db->CompactRange(DbStorage::Columns::final_chain_receipt_by_period, start_period, end_period);
    db->CompactRange(DbStorage::Columns::period_lambda, start_period, end_period);

    it = db->getColumnIterator(DbStorage::Columns::dag_blocks_level);
    it->SeekToFirst();
    if (!it->Valid()) {
      return;
    }
    uint64_t start_level;
    memcpy(&start_level, it->key().data(), sizeof(uint64_t));

    uint64_t dag_level_end = dag_level_to_keep - 1;
    // Validate range before operations
    if (start_level >= dag_level_end) {
      return;
    }

    db->DeleteRange(DbStorage::Columns::dag_blocks_level, start_level, dag_level_end);
    db->CompactRange(DbStorage::Columns::dag_blocks_level, start_level, dag_level_end);
#endif
  };
  api.state_prune_block_number = [app] {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("LIGHT_HISTORY_API_APP_EXPIRED");
    }
    const auto kPruneBlocksToKeep = kDagExpiryLevelLimit + kMaxLevelsPerPeriod + 1;
    // prune state db only if we have more than 2*kPruneBlocksToKeep blocks
    const uint64_t kPruneStateDbThreshold = 1.5 * kPruneBlocksToKeep;
    auto final_chain = node->getFinalChain();
    auto last_blk_num = final_chain->lastBlockNumber();
    if (last_blk_num > kPruneStateDbThreshold) {
      auto prune_block_num = last_blk_num - kPruneStateDbThreshold;
      auto prune_block = final_chain->blockHeader(prune_block_num);
      if (!prune_block) {
        return std::optional<uint64_t>{};
      }
      return std::optional<uint64_t>(prune_block_num);
    }
    return std::optional<uint64_t>{};
  };
  api.prune_state_db = [app](uint64_t prune_block_num) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("LIGHT_HISTORY_API_APP_EXPIRED");
    }
    node->getFinalChain()->prune(prune_block_num);
  };
  return api;
}

void fillMissingLightHistoryApiCallbacks(LightHistoryApi& api, std::weak_ptr<AppBase> app) {
  auto defaults = makeLightHistoryApi(std::move(app));
  if (!api.subscribe_finalized_block) {
    api.subscribe_finalized_block = std::move(defaults.subscribe_finalized_block);
  }
  if (!api.history_facts) {
    api.history_facts = std::move(defaults.history_facts);
  }
  if (!api.proposal_period_for_dag_level) {
    api.proposal_period_for_dag_level = std::move(defaults.proposal_period_for_dag_level);
  }
  if (!api.clear_history) {
    api.clear_history = std::move(defaults.clear_history);
  }
  if (!api.state_prune_block_number) {
    api.state_prune_block_number = std::move(defaults.state_prune_block_number);
  }
  if (!api.prune_state_db) {
    api.prune_state_db = std::move(defaults.prune_state_db);
  }
}

#ifndef RUSTAXA_ENABLE
void clearNonBlockData(const std::shared_ptr<DbStorage>& db, PbftPeriod start, PbftPeriod end, bool live_cleanup,
                       uint64_t periods_to_keep_non_block_data) {
  auto length = end - start;
  if (!live_cleanup && length > 2 * periods_to_keep_non_block_data) {
    recreateNonBlockData(db, end, periods_to_keep_non_block_data);
    return;
  }

  auto batch = db->createWriteBatch();
  for (PbftPeriod period = start; period < end; period++) {
    auto period_data = db->getPeriodData(period);
    if (!period_data.has_value()) {
      break;
    }
    for (auto t : period_data->transactions) {
      db->remove(batch, DbStorage::Columns::trx_period, t->getHash());
      db->remove(batch, DbStorage::Columns::final_chain_receipt_by_trx_hash, t->getHash());
    }
    for (auto d : period_data->dag_blocks) {
      db->remove(batch, DbStorage::Columns::dag_block_period, d->getHash());
    }
    db->remove(batch, DbStorage::Columns::pbft_block_period, period_data->pbft_blk->getBlockHash());
  }
  db->commitWriteBatch(batch);
}

void recreateNonBlockData(const std::shared_ptr<DbStorage>& db, PbftPeriod last_block_number,
                          uint64_t periods_to_keep_non_block_data) {
  std::unordered_set<trx_hash_t> trxs;
  std::unordered_set<blk_hash_t> dag_blocks;
  std::unordered_set<blk_hash_t> pbft_blocks;

  for (uint64_t period = last_block_number - periods_to_keep_non_block_data;; period++) {
    auto period_data = db->getPeriodData(period);
    if (!period_data.has_value()) {
      break;
    }
    for (auto t : period_data->transactions) {
      trxs.insert(t->getHash());
    }
    for (auto d : period_data->dag_blocks) {
      dag_blocks.insert(d->getHash());
    }
    pbft_blocks.insert(period_data->pbft_blk->getBlockHash());
  }

  db->clearColumnHistory(trxs, DbStorage::Columns::trx_period);
  db->clearColumnHistory(trxs, DbStorage::Columns::final_chain_receipt_by_trx_hash);
  db->clearColumnHistory(dag_blocks, DbStorage::Columns::dag_block_period);
  db->clearColumnHistory(pbft_blocks, DbStorage::Columns::pbft_block_period);
}
#endif
}  // namespace

Light::Light(std::shared_ptr<AppBase> app_, LightHistoryApi history_api)
    : Plugin(app_), history_api_(std::move(history_api)), history_(app()->getMutableConfig().light_node_history) {
  fillMissingLightHistoryApiCallbacks(history_api_, app_);
}

void Light::init(const boost::program_options::variables_map& opts) {
  auto node_addr = app()->getAddress();
  LOG_OBJECTS_CREATE("light");
  const auto& conf = app()->getConfig();

  const auto& cacti_hf = conf.genesis.state.hardforks.cacti_hf;
  // Since cacti hf introduced dynamic lambda, the number of blocks node has to keep is changins as dynamic lambda
  // changes. To keep things simple, calculate blocks_per_year for the smallest possible dynamic lambda
  const auto blocks_per_year = conf.genesis.calcBlocksPerYear(cacti_hf.lambda_min, cacti_hf.consensus_delay);
  const auto min_light_node_history_ = (blocks_per_year * conf.kDefaultLightNodeHistoryDays) / 365;
  if (!opts[HISTORY].empty()) {
    history_ = opts[HISTORY].as<uint64_t>();
    if (history_ < min_light_node_history_) {
      throw ConfigException("Min. required light node history is " + std::to_string(min_light_node_history_) +
                            " blocks (" + std::to_string(conf.kDefaultLightNodeHistoryDays) + " days)");
    }
  } else {
    history_ = min_light_node_history_;
  }
  state_db_pruning_ = !opts[NO_STATE_DB_PRUNING].as<bool>();

  live_cleanup_ = !opts[NO_LIVE_CLEANUP].as<bool>();

  app()->getMutableConfig().is_light_node = true;
}

void Light::addOptions(boost::program_options::options_description& opts) {
  opts.add_options()(HISTORY, bpo::value<uint32_t>(), "Number of blocks to keep in light node history");
  opts.add_options()(NO_STATE_DB_PRUNING, bpo::bool_switch()->default_value(false), "Prune state_db");
  opts.add_options()(NO_LIVE_CLEANUP, bpo::bool_switch()->default_value(false), "Disable live cleanup");
}

void Light::start() {
  clearLightNodeHistory();
  if (state_db_pruning_) {
    pruneStateDb();
  }
  history_api_.subscribe_finalized_block(
      [this] {
        if (!live_cleanup_) {
          return;
        }
        if (live_cleanup_in_progress_) {
          return;
        }
        live_cleanup_in_progress_ = true;
        clearLightNodeHistory(true);
        live_cleanup_in_progress_ = false;
      },
      cleanup_pool_);
}

void Light::shutdown() { cleanup_pool_->stop(); }

uint64_t Light::getCleanupPeriod(uint64_t dag_period, std::optional<uint64_t> proposal_period) const {
  return std::min(dag_period - history_, *proposal_period);
}

void Light::clearLightNodeHistory(bool live_cleanup) {
  LOG(log_nf_) << "Clear light node history: live_cleanup=" << live_cleanup << ", history_=" << history_;
  const auto facts = history_api_.history_facts();
  bool dag_expiry_level_condition = facts.dag_expiry_level > facts.max_levels_per_period + 1;
  if (facts.dag_period > history_ && dag_expiry_level_condition) {
    if (!live_cleanup) {
      LOG(log_nf_) << "Clear light node history: dag_period=" << facts.dag_period
                   << ", dag_expiry_level=" << facts.dag_expiry_level
                   << ", max_levels_per_period=" << facts.max_levels_per_period
                   << ", dag_expiry_level_condition=" << dag_expiry_level_condition << ", history_=" << history_;
    }
    const auto proposal_period =
        history_api_.proposal_period_for_dag_level(facts.dag_expiry_level - facts.max_levels_per_period - 1);
    assert(proposal_period);

    // This prevents deleting any data needed for dag blocks proposal period, we only delete periods for the expired
    // dag blocks
    const uint64_t end = getCleanupPeriod(facts.dag_period, proposal_period);
    uint64_t dag_level_to_keep = 1;
    if (facts.dag_expiry_level > facts.max_levels_per_period) {
      dag_level_to_keep = facts.dag_expiry_level - facts.max_levels_per_period;
    }

    clearHistory(end, dag_level_to_keep, live_cleanup);
    if (!live_cleanup) {
      LOG(log_nf_) << "Clear light node history completed";
    }
  }
}

void Light::clearHistory(PbftPeriod end_period, uint64_t dag_level_to_keep, bool live_cleanup) {
  history_api_.clear_history(end_period, dag_level_to_keep, live_cleanup, kPeriodsToKeepNonBlockData);
}

void Light::pruneStateDb() {
  auto prune_block_num = history_api_.state_prune_block_number();
  if (!prune_block_num) {
    LOG(log_nf_) << "Prune was done recently, skip state db pruning";
    return;
  }
  LOG(log_nf_) << "Pruning state db " << *prune_block_num << ", this might take several minutes";
  history_api_.prune_state_db(*prune_block_num);
  LOG(log_nf_) << "Pruning state db complete";
}

}  // namespace taraxa::plugin
