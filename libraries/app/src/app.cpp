#include "app/app.hpp"

#include <libdevcore/CommonJS.h>

#include <boost/algorithm/string.hpp>
#include <boost/algorithm/string/split.hpp>
#include <boost/filesystem.hpp>
#include <memory>

#include "common/config_exception.hpp"
#include "config/config_utils.hpp"
#include "dag/dag.hpp"
#include "dag/dag_block.hpp"
#include "dag/dag_block_proposer.hpp"
#include "dag/dag_manager.hpp"
#include "final_chain/final_chain.hpp"
#ifndef RUSTAXA_ENABLE
#include "key_manager/key_manager.hpp"
#endif
#include "metrics/metrics_service.hpp"
#include "metrics/network_metrics.hpp"
#include "metrics/pbft_metrics.hpp"
#include "metrics/transaction_queue_metrics.hpp"
#ifdef RUSTAXA_ENABLE
#include "network/consensus_network_api.hpp"
#include "network/consensus_query.hpp"
#endif
#include "pbft/pbft_manager.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
#include "storage/migration/block_stats.hpp"
#include "storage/migration/migration_manager.hpp"
#ifndef RUSTAXA_ENABLE
#include "slashing_manager/slashing_manager.hpp"
#include "transaction/gas_pricer.hpp"
#endif
#include "transaction/transaction_manager.hpp"
#include "vote_manager/vote_manager.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa {

#ifdef RUSTAXA_ENABLE
namespace {

/**
 * Stops and joins the database-rebuild queue worker on every exit path.
 *
 * The guard borrows the stop flag and future created by `App::rebuildDb`.
 * `stop` is idempotent; destruction performs the same cleanup during exception
 * unwinding so an encoding or native queue-admission failure can propagate
 * without the async future waiting forever on a worker that was never stopped.
 */
class RebuildQueueWorkerGuard final {
 public:
  RebuildQueueWorkerGuard(std::atomic_bool &stop_requested, std::future<void> &worker)
      : stop_requested_(stop_requested), worker_(worker) {}
  ~RebuildQueueWorkerGuard() { stop(); }

  void stop() noexcept {
    if (stopped_) {
      return;
    }
    stop_requested_ = true;
    if (worker_.valid()) {
      worker_.wait();
    }
    stopped_ = true;
  }

 private:
  std::atomic_bool &stop_requested_;
  std::future<void> &worker_;
  bool stopped_ = false;
};

/**
 * Copies canonical database bytes into the stable CXX vector carrier.
 *
 * The input remains available to the rebuild loop for materialized logging and
 * sender prewarming. Allocation failure propagates to abort rebuild without a
 * partial queue push.
 */
rust::Vec<uint8_t> toRustBytes(const dev::bytes &input) {
  rust::Vec<uint8_t> output;
  output.reserve(input.size());
  for (const auto byte : input) {
    output.push_back(static_cast<uint8_t>(byte));
  }
  return output;
}

/**
 * Encodes one certificate-vote set for Rust queue ownership.
 *
 * Each non-null vote is encoded with its signature and optional weight exactly
 * as the PBFT manager adapter encoded it. A null vote is rejected before the
 * Rust queue mutates, and encoding or allocation errors propagate to abort the
 * rebuild.
 */
rust::Vec<rustaxa::PbftCertVoteRlp> toRustCertVotePayloads(const std::vector<std::shared_ptr<PbftVote>> &votes) {
  rust::Vec<rustaxa::PbftCertVoteRlp> payloads;
  payloads.reserve(votes.size());
  for (const auto &vote : votes) {
    if (!vote) {
      throw std::runtime_error("PBFT manager period-data queue: cannot push period data with a null PBFT cert vote");
    }
    rustaxa::PbftCertVoteRlp payload;
    payload.vote_rlp = toRustBytes(vote->rlp(true, vote->getWeight().has_value()));
    payloads.push_back(std::move(payload));
  }
  return payloads;
}

}  // namespace
#endif

App::App() {}

App::~App() { close(); }

PbftProgress App::getPbftProgress() const {
#ifdef RUSTAXA_ENABLE
  const auto query_api = net::createConsensusQueryApi(db_);
  if (!query_api) {
    throw std::runtime_error("Consensus query API is unavailable");
  }
  return net::consensusPbftProgress(query_api);
#endif
#ifndef RUSTAXA_ENABLE
  return {pbft_chain_->getPbftChainSize(), pbft_chain_->getPbftChainSizeExcludingEmptyPbftBlocks()};
#endif
}

void App::addAvailablePlugin(std::shared_ptr<Plugin> plugin) { available_plugins_[plugin->name()] = plugin; }

std::shared_ptr<Plugin> App::getPlugin(const std::string &name) const {
  auto it = active_plugins_.find(name);
  if (it != active_plugins_.end()) {
    return it->second;
  }
  return nullptr;
}

void App::enablePlugin(const std::string &name) {
  if (available_plugins_[name] == nullptr) {
    throw std::runtime_error("Plugin " + name + " not found");
  }
  active_plugins_[name] = available_plugins_[name];
}

bool App::isPluginEnabled(const std::string &name) const { return active_plugins_.find(name) != active_plugins_.end(); }

void App::init(const cli::Config &cli_conf) {
  conf_ = cli_conf.getNodeConfiguration();

  fs::create_directories(conf_.db_path);
  fs::create_directories(conf_.log_path);

  // Initialize logging
  const auto &node_addr = conf_.getFirstWallet().node_addr;
  for (auto &logging : conf_.log_configs) {
    logging.InitLogging(node_addr);
  }

  LOG_OBJECTS_CREATE("FULLND");

  std::string node_addresses;
  std::string node_public_keys;
  std::string node_vrf_public_keys;
  std::for_each(conf_.wallets.begin(), conf_.wallets.end(), [&](const WalletConfig &wallet) {
    node_addresses += wallet.node_addr.toString() + " ";
    node_public_keys += wallet.node_pk.toString() + " ";
    node_vrf_public_keys += wallet.vrf_pk.toString() + " ";
  });

  LOG(log_si_) << "Node public keys: " << EthGreen << "[" << node_public_keys << "]" << std::endl
               << EthReset << "Node addresses: " << EthRed << "[" << node_addresses << "]" << std::endl
               << EthReset << "Node VRF public keys: " << EthGreen << "[" << node_vrf_public_keys << "]" << EthReset;

  if (!conf_.genesis.dag_genesis_block.verifySig()) {
    LOG(log_er_) << "Genesis block is invalid";
    assert(false);
  }
#ifdef RUSTAXA_ENABLE
  if (conf_.db_config.rebuild_db || conf_.db_config.migrate_only || conf_.db_config.migrate_receipts_by_period ||
      conf_.db_config.db_revert_to_period != 0 || conf_.db_config.rebuild_db_period != 0) {
    throw std::runtime_error("Rust consensus mode does not support rebuild, revert, or legacy migration startup modes");
  }
  consensus_application_ = createConsensusApplication(conf_);
#endif
  {
#ifndef RUSTAXA_ENABLE
    if (conf_.db_config.rebuild_db) {
      old_db_ = std::make_shared<DbStorage>(conf_.db_path, conf_.db_config.db_snapshot_each_n_pbft_block,
                                            conf_.db_config.db_max_open_files, conf_.db_config.db_max_snapshots,
                                            conf_.db_config.db_revert_to_period, node_addr, true);
    }
    db_ = std::make_shared<DbStorage>(conf_.db_path,
                                      // Snapshots should be disabled while rebuilding
                                      conf_.db_config.rebuild_db ? 0 : conf_.db_config.db_snapshot_each_n_pbft_block,
                                      conf_.db_config.db_max_open_files, conf_.db_config.db_max_snapshots,
                                      conf_.db_config.db_revert_to_period, node_addr, false);

    if (db_->hasMajorVersionChanged()) {
      LOG(log_si_) << "Major DB version has changed. Rebuilding Db";
      conf_.db_config.rebuild_db = true;
      db_ = nullptr;
      old_db_ = std::make_shared<DbStorage>(conf_.db_path, conf_.db_config.db_snapshot_each_n_pbft_block,
                                            conf_.db_config.db_max_open_files, conf_.db_config.db_max_snapshots,
                                            conf_.db_config.db_revert_to_period, node_addr, true);
      db_ = std::make_shared<DbStorage>(conf_.db_path,
                                        0,  // Snapshots should be disabled while rebuilding
                                        conf_.db_config.db_max_open_files, conf_.db_config.db_max_snapshots,
                                        conf_.db_config.db_revert_to_period, node_addr);
    }

    db_->updateDbVersions();

    auto migration_manager = storage::migration::Manager(db_);
    migration_manager.registerMigration(std::make_shared<storage::migration::BlockStats>(db_, conf_));

    migration_manager.applyAll();

    if (conf_.db_config.migrate_receipts_by_period) {
      migration_manager.applyReceiptsByPeriod();
    }
    if (db_->getDagBlocksCount() == 0) {
      db_->setGenesisHash(conf_.genesis.genesisHash());
    }
#else
    db_ = std::make_shared<DbStorage>(consensus_application_, conf_.db_path,
                                      conf_.db_config.db_snapshot_each_n_pbft_block, conf_.db_config.db_max_open_files,
                                      conf_.db_config.db_max_snapshots, conf_.db_config.db_revert_to_period, node_addr,
                                      false);
#endif
  }
  LOG(log_nf_) << "DB initialized ...";

  if (conf_.network.prometheus) {
    auto &config = *conf_.network.prometheus;
    LOG(log_nf_) << "Prometheus: server started at " << config.address << ":" << config.listen_port
                 << ". Polling interval is " << config.polling_interval_ms << "ms";
    metrics_ =
        std::make_shared<metrics::MetricsService>(config.address, config.listen_port, config.polling_interval_ms);
  } else {
    LOG(log_nf_) << "Prometheus: config values aren't specified. Metrics collecting is disabled";
  }

#ifdef RUSTAXA_ENABLE
  final_chain_ = std::make_shared<final_chain::FinalChain>(db_, conf_, node_addr, consensus_application_);
#else
  final_chain_ = std::make_shared<final_chain::FinalChain>(db_, conf_, node_addr);
#endif
#ifndef RUSTAXA_ENABLE
  key_manager_ = std::make_shared<KeyManager>(final_chain_);
#endif
#ifndef RUSTAXA_ENABLE
  trx_mgr_ = std::make_shared<TransactionManager>(conf_, db_, final_chain_, node_addr);
#endif
#ifndef RUSTAXA_ENABLE
  gas_pricer_ = std::make_shared<GasPricer>(conf_.genesis, conf_.is_light_node, conf_.blocks_gas_pricer, trx_mgr_, db_);
#endif

  auto genesis_hash = conf_.genesis.genesisHash();
  auto genesis_hash_from_db = db_->getGenesisHash();
  if (!genesis_hash_from_db.has_value()) {
    LOG(log_er_) << "Genesis hash was not found in DB. Something is wrong";
    std::terminate();
  }
  if (genesis_hash != genesis_hash_from_db) {
    LOG(log_er_) << "Genesis hash " << genesis_hash << " is different with "
                 << (genesis_hash_from_db.has_value() ? *genesis_hash_from_db : h256(0)) << " in DB";
    std::terminate();
  }

#ifdef RUSTAXA_ENABLE
  trx_mgr_ = std::make_shared<TransactionManager>(conf_, db_, final_chain_, node_addr, consensus_application_);
#else
  pbft_chain_ = std::make_shared<PbftChain>(node_addr, db_);
#endif
#ifdef RUSTAXA_ENABLE
  dag_mgr_ = std::make_shared<DagManager>(conf_, node_addr, trx_mgr_, final_chain_, db_, consensus_application_);
#else
  dag_mgr_ = std::make_shared<DagManager>(conf_, node_addr, trx_mgr_, pbft_chain_, final_chain_, db_, key_manager_);
#endif
#ifdef RUSTAXA_ENABLE
  vote_mgr_ = std::make_shared<VoteManager>(conf_, consensus_application_, final_chain_, trx_mgr_);
#else
  auto slashing_manager = std::make_shared<SlashingManager>(conf_, final_chain_, trx_mgr_, gas_pricer_);
  vote_mgr_ = std::make_shared<VoteManager>(conf_, db_, pbft_chain_, final_chain_, key_manager_, slashing_manager);
#endif
#ifdef RUSTAXA_ENABLE
  pillar_chain_mgr_ = std::make_shared<pillar_chain::PillarChainManager>(
      conf_.genesis.state.hardforks.ficus_hf, db_, consensus_application_, final_chain_, node_addr);
#else
  pillar_chain_mgr_ = std::make_shared<pillar_chain::PillarChainManager>(conf_.genesis.state.hardforks.ficus_hf, db_,
                                                                         final_chain_, key_manager_, node_addr);
#endif
#ifdef RUSTAXA_ENABLE
  pbft_mgr_ = std::make_shared<PbftManager>(conf_, db_, consensus_application_, vote_mgr_, dag_mgr_, trx_mgr_,
                                            final_chain_, pillar_chain_mgr_);
#else
  pbft_mgr_ = std::make_shared<PbftManager>(conf_, db_, pbft_chain_, vote_mgr_, dag_mgr_, trx_mgr_, final_chain_,
                                            pillar_chain_mgr_);
#endif
#ifdef RUSTAXA_ENABLE
  dag_block_proposer_ = std::make_shared<DagBlockProposer>(conf_, dag_mgr_, trx_mgr_, final_chain_);
#else
  dag_block_proposer_ = std::make_shared<DagBlockProposer>(conf_, dag_mgr_, trx_mgr_, final_chain_, db_, key_manager_);
#endif

  network_ =
      std::make_shared<Network>(conf_, genesis_hash, conf_.net_file_path().string(),
#ifndef RUSTAXA_ENABLE
                                db_,
#endif
                                pbft_mgr_,
#ifdef RUSTAXA_ENABLE
                                net::createConsensusQueryApi(db_),
#else
                                pbft_chain_,
#endif
                                vote_mgr_, dag_mgr_, trx_mgr_,
#ifndef RUSTAXA_ENABLE
                                std::move(slashing_manager),
#endif
                                pillar_chain_mgr_,
#ifdef RUSTAXA_ENABLE
                                final_chain_, std::make_shared<network::ConsensusNetworkApi>(consensus_application_));
#else
                                final_chain_);
#endif
  auto cli_options = cli_conf.getCliOptions();
  for (auto &plugin : active_plugins_) {
    plugin.second->init(cli_options);
  }
}

void App::start() {
  if (bool b = true; !stopped_.compare_exchange_strong(b, !b)) {
    return;
  }

  scheduleLoggingConfigUpdate();

  if (!conf_.db_config.rebuild_db) {
    // Gas-price oracle updater
#ifndef RUSTAXA_ENABLE
    final_chain_->block_finalized_.subscribe(
        [gas_pricer = as_weak(gas_pricer_)](const auto &res) {
          if (auto gp = gas_pricer.lock()) {
            gp->update(res->trxs);
          }
        },
        subscription_pool_);
#else
    final_chain_->block_finalized_.subscribe(
        [trx_manager = as_weak(trx_mgr_)](const auto &res) {
          if (auto manager = trx_manager.lock()) {
            manager->updateGasPrice(res->trxs);
          }
        },
        subscription_pool_);
#endif

    final_chain_->block_finalized_.subscribe(
        [trx_manager = as_weak(trx_mgr_)](const auto &res) {
          if (auto trx_mgr = trx_manager.lock()) {
            trx_mgr->blockFinalized(res->final_chain_blk->number);
          }
        },
        subscription_pool_);
  }

  vote_mgr_->setNetwork(network_);
  pbft_mgr_->setNetwork(network_);
  dag_mgr_->setNetwork(network_);
  pillar_chain_mgr_->setNetwork(network_);

  if (conf_.db_config.rebuild_db) {
    rebuildDb();
    LOG(log_si_) << "Rebuild db completed successfully. Restart node without db_rebuild option";
    started_ = false;
    return;
  } else if (conf_.db_config.migrate_only) {
    LOG(log_si_) << "DB migrated successfully, please restart the node without the flag";
    started_ = false;
    return;
  }

  for (auto &plugin : active_plugins_) {
    LOG(log_nf_) << "Starting plugin " << plugin.first;
    plugin.second->start();
  }

  network_->start();
  dag_block_proposer_->setNetwork(network_);
  dag_block_proposer_->start();

  pbft_mgr_->start();

  if (metrics_) {
    setupMetricsUpdaters();
    metrics_->start();
  }
  started_ = true;
  LOG(log_nf_) << "Node started ... ";
}

void App::scheduleLoggingConfigUpdate() {
  // no file to check updates for (e.g. tests)
  if (conf_.json_file_name.empty()) {
    return;
  }

  config_update_executor_.post([&]() {
    while (started_ && !stopped_) {
      auto path = std::filesystem::path(conf_.json_file_name);
      if (path.empty()) {
        std::cout << "FullNodeConfig: scheduleLoggingConfigUpdate: json_file_name is empty" << std::endl;
        return;
      }
      auto update_time = std::filesystem::last_write_time(path);
      if (conf_.last_json_update_time >= update_time) {
        continue;
      }
      conf_.last_json_update_time = update_time;
      try {
        auto config = getJsonFromFileOrString(conf_.json_file_name);
        conf_.log_configs = conf_.loadLoggingConfigs(config["logging"]);
        conf_.InitLogging(conf_.getFirstWallet().node_addr);
      } catch (const ConfigException &e) {
        std::cerr << "FullNodeConfig: Failed to update logging config: " << e.what() << std::endl;
        continue;
      }
      std::cout << "FullNodeConfig: Updated logging config" << std::endl;
      std::this_thread::sleep_for(std::chrono::minutes(1));
    }
  });
}

void App::setupMetricsUpdaters() {
  auto network_metrics = metrics_->getMetrics<metrics::NetworkMetrics>();
  network_metrics->setPeersCountUpdater([network = network_]() { return network->getPeerCount(); });
  network_metrics->setDiscoveredPeersCountUpdater([network = network_]() { return network->getNodeCount(); });
  network_metrics->setSyncingDurationUpdater([network = network_]() { return network->syncTimeSeconds(); });

  auto transaction_queue_metrics = metrics_->getMetrics<metrics::TransactionQueueMetrics>();
  transaction_queue_metrics->setTransactionsCountUpdater(
      [trx_mgr = trx_mgr_]() { return trx_mgr->getTransactionPoolSize(); });
#ifndef RUSTAXA_ENABLE
  transaction_queue_metrics->setGasPriceUpdater(
      [gas_pricer = gas_pricer_]() { return gas_pricer->bid().convert_to<double>(); });
#else
  transaction_queue_metrics->setGasPriceUpdater(
      [trx_manager = trx_mgr_]() { return trx_manager->gasPriceBid().convert_to<double>(); });
#endif

  auto pbft_metrics = metrics_->getMetrics<metrics::PbftMetrics>();
  pbft_metrics->setPeriodUpdater([pbft_mgr = pbft_mgr_]() { return pbft_mgr->getPbftPeriod(); });
  pbft_metrics->setRoundUpdater([pbft_mgr = pbft_mgr_]() { return pbft_mgr->getPbftRound(); });
  pbft_metrics->setStepUpdater([pbft_mgr = pbft_mgr_]() { return pbft_mgr->getPbftStep(); });
  pbft_metrics->setVotesCountUpdater(
      [pbft_mgr = pbft_mgr_]() { return pbft_mgr->getCurrentNodeVotesCount().value_or(0); });
  final_chain_->block_finalized_.subscribe(
      [pbft_metrics](const std::shared_ptr<final_chain::FinalizationResult> &res) {
        pbft_metrics->setBlockNumber(res->final_chain_blk->number);
        pbft_metrics->setBlockTransactionsCount(res->trxs.size());
        pbft_metrics->setBlockTimestamp(res->final_chain_blk->timestamp);
      },
      subscription_pool_);
}

void App::close() {
  if (bool b = false; !stopped_.compare_exchange_strong(b, !b)) {
    return;
  }

  dag_block_proposer_->stop();
  pbft_mgr_->stop();
  LOG(log_nf_) << "Node stopped ... ";
}

void App::rebuildDb() {
  pbft_mgr_->initialState();

  // Read pbft blocks one by one
  PbftPeriod period = 1;
  std::shared_ptr<PeriodData> period_data, next_period_data;
#ifdef RUSTAXA_ENABLE
  dev::bytes period_data_raw, next_period_data_raw;
#endif
  std::atomic_bool stop_async = false;

  std::future<void> fut = std::async(std::launch::async, [this, &stop_async]() {
    while (!stop_async) {
      // While rebuilding pushSyncedPbftBlocksIntoChain will stay in its own internal loop
      pbft_mgr_->pushSyncedPbftBlocksIntoChain();
      thisThreadSleepForMilliSeconds(1);
    }
  });
#ifdef RUSTAXA_ENABLE
  RebuildQueueWorkerGuard worker_guard(stop_async, fut);
#endif

  while (true) {
    std::vector<std::shared_ptr<PbftVote>> cert_votes;
    if (next_period_data != nullptr) {
      period_data = next_period_data;
#ifdef RUSTAXA_ENABLE
      period_data_raw = std::move(next_period_data_raw);
#endif
    } else {
      auto data = old_db_->getPeriodDataRaw(period);
      if (data.size() == 0) break;
#ifdef RUSTAXA_ENABLE
      period_data = std::make_shared<PeriodData>(data);
      period_data_raw = std::move(data);
#else
      period_data = std::make_shared<PeriodData>(std::move(data));
#endif
    }
    auto data = old_db_->getPeriodDataRaw(period + 1);
    if (data.size() == 0) {
      next_period_data = nullptr;
#ifdef RUSTAXA_ENABLE
      next_period_data_raw.clear();
#endif
      // Latest finalized block cert votes are saved in db as 2t+1 cert votes
      auto votes = old_db_->getAllTwoTPlusOneVotes();
      for (auto v : votes) {
        if (v->getType() == PbftVoteTypes::cert_vote) cert_votes.push_back(v);
      }
    } else {
#ifdef RUSTAXA_ENABLE
      next_period_data = std::make_shared<PeriodData>(data);
      next_period_data_raw = std::move(data);
#else
      next_period_data = std::make_shared<PeriodData>(std::move(data));
#endif
      // More efficient to get sender(which is expensive) on this thread which is not as busy as the thread that
      // pushes blocks to chain
      for (auto &t : next_period_data->transactions) t->getSender();
      cert_votes = next_period_data->previous_block_cert_votes;
    }

    LOG(log_nf_) << "Adding PBFT block " << period_data->pbft_blk->getBlockHash().toString()
                 << " from old DB into syncing queue for processing, final chain size: "
                 << final_chain_->lastBlockNumber();

#ifdef RUSTAXA_ENABLE
    auto previous_cert_vote_payloads = toRustCertVotePayloads(period_data->previous_block_cert_votes);
    auto current_cert_vote_payloads = toRustCertVotePayloads(cert_votes);
    rustaxa::PeriodDataQueuePushOutcome outcome;
    try {
      outcome = rustaxa::pbft_manager_runtime_period_data_queue_push(
          consensus_application_->service(), toRustBytes(period_data_raw), dev::p2p::NodeID().asArray(),
          std::move(previous_cert_vote_payloads), std::move(current_cert_vote_payloads));
    } catch (const std::exception &e) {
      throw std::runtime_error("PBFT manager period-data queue: " + std::string(e.what()));
    } catch (...) {
      throw std::runtime_error("PBFT manager period-data queue: Rust push failed");
    }
    if (!outcome.accepted) {
      LOG(log_er_) << "Rejected synced period data push for period " << period << ": expected "
                   << outcome.expected_next_period << ", got " << outcome.actual_period << " (current period "
                   << outcome.current_period << ", effective queue size " << outcome.effective_size << ")";
    }
#else
    pbft_mgr_->periodDataQueuePush(std::move(*period_data), dev::p2p::NodeID(), std::move(cert_votes));
#endif
    pbft_mgr_->waitForPeriodFinalization();
    period++;
    if (period % 100 == 0) {
      while (period - getPbftProgress().finalized_period > 100) {
        thisThreadSleepForMilliSeconds(1);
      }
    }

    if (period - 1 == conf_.db_config.rebuild_db_period) {
      break;
    }

    if (period % 10000 == 0) {
      LOG(log_si_) << "Rebuilding period: " << period;
    }
  }
#ifdef RUSTAXA_ENABLE
  worker_guard.stop();
#else
  stop_async = true;
  fut.wait();
#endif
  // Handles the race case if some blocks are still in the queue
  pbft_mgr_->pushSyncedPbftBlocksIntoChain();
  LOG(log_si_) << "Rebuild completed";
}

}  // namespace taraxa
