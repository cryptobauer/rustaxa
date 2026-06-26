#include <functional>
#include <optional>

#include "common/thread_pool.hpp"
#include "logger/logger.hpp"
#include "plugin/plugin.hpp"

namespace taraxa::net {}  // namespace taraxa::net

namespace taraxa::plugin {

// LightHistoryFacts is the small cleanup snapshot the light-node plugin needs
// from consensus DAG state. It contains only scalar facts used to decide
// whether history can be pruned and which DAG level must be retained.
struct LightHistoryFacts {
  uint64_t dag_period = 0;
  uint64_t dag_expiry_level = 0;
  uint64_t max_levels_per_period = 0;
};

// LightHistoryApi is the light plugin boundary for consensus history cleanup
// and external state pruning. The plugin owns scheduling and retention policy;
// the default adapter owns temporary DAG manager, DbStorage, and FinalChain
// compatibility calls until those cleanup operations move behind Rust storage
// APIs.
struct LightHistoryApi {
  std::function<void(std::function<void()>, std::shared_ptr<util::ThreadPool>)> subscribe_finalized_block;
  std::function<LightHistoryFacts()> history_facts;
  std::function<std::optional<uint64_t>(uint64_t)> proposal_period_for_dag_level;
  std::function<void(PbftPeriod, uint64_t, bool, uint64_t)> clear_history;
  std::function<std::optional<uint64_t>()> state_prune_block_number;
  std::function<void(uint64_t)> prune_state_db;
};

class Light : public Plugin {
 public:
  explicit Light(std::shared_ptr<AppBase> app, LightHistoryApi history_api = {});

  std::string name() const override { return "light"; }
  std::string description() const override { return "Light node plugin"; }

  void init(const boost::program_options::variables_map& options) override;
  void addOptions(boost::program_options::options_description& command_line_options) override;

  void start() override;
  void shutdown() override;

  void clearLightNodeHistory(bool live_cleanup = false);

 private:
  /**
   * @brief Clears light node history
   */
  void clearHistory(PbftPeriod end_period, uint64_t dag_level_to_keep, bool live_cleanup);
  void pruneStateDb();

  uint64_t getCleanupPeriod(uint64_t dag_period, std::optional<uint64_t> proposal_period) const;

  static constexpr uint64_t kPeriodsToKeepNonBlockData = 1000;
  std::shared_ptr<util::ThreadPool> cleanup_pool_ = std::make_shared<util::ThreadPool>(1);
  LightHistoryApi history_api_;
  uint64_t& history_;
  bool state_db_pruning_;
  bool live_cleanup_;
  std::atomic<bool> live_cleanup_in_progress_ = false;

  LOG_OBJECTS_DEFINE
};

}  // namespace taraxa::plugin
