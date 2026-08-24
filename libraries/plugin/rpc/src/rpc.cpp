#include "plugin/rpc.hpp"

#include <boost/program_options.hpp>

#include "graphql/http_processor.hpp"
#include "graphql/ws_server.hpp"
#include "metrics/metrics_service.hpp"
#include "network/consensus_query.hpp"
#include "network/rpc/Debug.h"
#include "network/rpc/Net.h"
#include "network/rpc/Taraxa.h"
#include "network/rpc/Test.h"
#include "network/rpc/eth/Eth.h"
#include "network/rpc/jsonrpc_http_processor.hpp"
#include "network/rpc/jsonrpc_ws_server.hpp"
#include "pillar_chain/pillar_chain_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "consensus/consensus_application.hpp"
#else
#include "vote_manager/vote_manager.hpp"
#endif

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::plugin {

namespace bpo = boost::program_options;
constexpr auto THREADS = "rpc.threads";
constexpr auto ENABLE_TEST_RPC = "rpc.enable-test-rpc";
constexpr auto ENABLE_DEBUG = "rpc.debug";

void Rpc::init(const boost::program_options::variables_map &opts) {
  if (!opts[THREADS].empty()) {
    threads_ = opts[THREADS].as<uint32_t>();
  }
  if (!opts[ENABLE_TEST_RPC].empty()) {
    enable_test_rpc_ = opts[ENABLE_TEST_RPC].as<bool>();
  }
  if (!opts[ENABLE_DEBUG].empty()) {
    enable_debug_ = opts[ENABLE_DEBUG].as<bool>();
  }
}

void Rpc::addOptions(boost::program_options::options_description &opts) {
  opts.add_options()(THREADS, bpo::value<uint32_t>(), "Number of threads for RPC");
  opts.add_options()(ENABLE_TEST_RPC, bpo::bool_switch()->default_value(false),
                     "Enables Test JsonRPC. Disabled by default");
  opts.add_options()(ENABLE_DEBUG, bpo::bool_switch()->default_value(false),
                     "Enables Debug RPC interface. Disabled by default");
}

void Rpc::start() {
  auto conf = app()->getConfig();
  if (threads_) {
    conf.network.rpc->threads_num = threads_;
  }
  std::shared_ptr<metrics::JsonRpcMetrics> jsonrpc_metrics;
  if (app()->getMetrics()) jsonrpc_metrics = app()->getMetrics()->getMetrics<metrics::JsonRpcMetrics>();
  net::LiveStatusReader live_status_reader = [app = app()] {
    net::LiveStatusSnapshot snapshot;
#ifdef RUSTAXA_ENABLE
    const auto consensus_application = app->getConsensusApplication();
    const auto runtime_status = consensus_application->runtimeStatus();
    const auto chain_size = runtime_status.finalized_chain_size;
    const auto dpos_total_votes = consensus_application->currentDposTotalVotesCount();
    const auto dpos_node_votes = consensus_application->currentNodeVotesCount();
#else
    const auto chain_size = app->getPbftProgress().finalized_period;
    const auto dpos_total_votes = app->getPbftManager()->getCurrentDposTotalVotesCount();
    const auto dpos_node_votes = app->getPbftManager()->getCurrentNodeVotesCount();
#endif
#ifdef RUSTAXA_ENABLE
    const auto query_api = consensus_application->queryClient();
    const auto threshold =
        (*query_api)->consensus_query_pbft_vote_threshold(chain_size, static_cast<uint8_t>(PbftVoteTypes::cert_vote));
    const auto dpos_quorum = threshold.has_threshold ? std::optional<uint64_t>{threshold.threshold} : std::nullopt;
#else
    const auto dpos_quorum = app->getVoteManager()->getPbftTwoTPlusOne(chain_size, PbftVoteTypes::cert_vote);
#endif

    snapshot.pbft_syncing = app->getNetwork()->pbft_syncing();
    snapshot.syncing_seconds = app->getNetwork()->syncTimeSeconds();
    snapshot.peer_count = app->getNetwork()->getPeerCount();
    snapshot.node_count = app->getNetwork()->getNodeCount();
    snapshot.pbft_chain_size = chain_size;
#ifdef RUSTAXA_ENABLE
    snapshot.pbft_sync_period = runtime_status.syncing_period;
    snapshot.pbft_round = runtime_status.round;
#else
    snapshot.pbft_sync_period = app->getPbftManager()->pbftSyncingPeriod();
    snapshot.pbft_round = app->getPbftManager()->getPbftRound();
#endif
    snapshot.dpos_total_votes = dpos_total_votes.value_or(0);
    snapshot.dpos_node_votes = dpos_node_votes.value_or(0);
    snapshot.dpos_quorum = dpos_quorum.value_or(0);
#ifdef RUSTAXA_ENABLE
    snapshot.pbft_sync_queue_size = runtime_status.sync_queue_size;
#else
    snapshot.pbft_sync_queue_size = app->getPbftManager()->periodDataQueueSize();
#endif
#ifdef RUSTAXA_ENABLE
    const auto transaction_status = (*query_api)->consensus_query_live_transaction_status();
    snapshot.transaction_pool_size = transaction_status.queue_size;
    snapshot.nonfinalized_transaction_size = transaction_status.non_finalized_size;
#else
    snapshot.transaction_pool_size = app->getTransactionManager()->getTransactionPoolSize();
    snapshot.nonfinalized_transaction_size = app->getTransactionManager()->getNonfinalizedTrxSize();
#endif
    if (const auto peer = app->getNetwork()->getMaxChainPeer()) {
      snapshot.max_peer_pbft_chain_size = peer->pbft_chain_size_.load();
    }
    snapshot.compatibility_network_status = app->getNetwork()->getStatus();
    return snapshot;
  };

#ifdef RUSTAXA_ENABLE
  auto consensus_query_api = app()->getConsensusApplication()->queryClient();
#endif

  // Inits rpc related members
  if (conf.network.rpc) {
    rpc_thread_pool_ = std::make_unique<util::ThreadPool>(conf.network.rpc->threads_num);
    net::rpc::eth::EthParams eth_rpc_params;
    eth_rpc_params.address = app()->getAddress();
    eth_rpc_params.chain_id = conf.genesis.chain_id;
    eth_rpc_params.gas_limit = conf.genesis.dag.gas_limit;
    eth_rpc_params.final_chain = app()->getFinalChain();
#ifdef RUSTAXA_ENABLE
    eth_rpc_params.gas_pricer = [query = consensus_query_api]() {
      return dev::fromBigEndian<dev::u256>((*query)->consensus_query_live_transaction_status().gas_price_bid);
    };
#else
    eth_rpc_params.gas_pricer = [gas_pricer = app()->getGasPricer()]() { return gas_pricer->bid(); };
#endif
    eth_rpc_params.get_earliest_block = [db = app()->getDB()]() { return db->getEarliestBlockNumber(); };
    eth_rpc_params.get_trx = [db = app()->getDB()](auto const &trx_hash) { return db->getTransaction(trx_hash); };
#ifdef RUSTAXA_ENABLE
    eth_rpc_params.query_transaction = [query_api = consensus_query_api](auto const &trx_hash) {
      return (*query_api)->consensus_query_transaction_by_hash(trx_hash.asArray());
    };
    eth_rpc_params.query_transaction_by_block_number_and_index = [query_api = consensus_query_api](
                                                                     auto block_number, auto transaction_index) {
      return (*query_api)->consensus_query_transaction_by_block_number_and_index(block_number, transaction_index);
    };
    eth_rpc_params.query_transaction_by_block_hash_and_index = [query_api = consensus_query_api](
                                                                   auto const &block_hash, auto transaction_index) {
      return (*query_api)->consensus_query_transaction_by_block_hash_and_index(block_hash.asArray(), transaction_index);
    };
    eth_rpc_params.query_transaction_count_by_block_number = [query_api = consensus_query_api](auto block_number) {
      return (*query_api)->consensus_query_transaction_count_by_block_number(block_number);
    };
    eth_rpc_params.query_transaction_count_by_block_hash = [query_api = consensus_query_api](auto const &block_hash) {
      return (*query_api)->consensus_query_transaction_count_by_block_hash(block_hash.asArray());
    };
    eth_rpc_params.query_transaction_receipt = [query_api = consensus_query_api](auto const &trx_hash) {
      return (*query_api)->consensus_query_transaction_receipt_by_hash(trx_hash.asArray());
    };
    eth_rpc_params.query_transaction_receipts_by_block_number = [query_api = consensus_query_api](auto block_number) {
      return (*query_api)->consensus_query_transaction_receipts_by_block_number(block_number);
    };
    eth_rpc_params.query_final_chain_block_by_number = [query_api = consensus_query_api](auto block_number) {
      return (*query_api)->consensus_query_final_chain_block_by_number(block_number);
    };
    eth_rpc_params.query_final_chain_block_number_by_hash = [query_api = consensus_query_api](auto const &block_hash) {
      return (*query_api)->consensus_query_final_chain_block_number_by_hash(block_hash.asArray());
    };
    eth_rpc_params.query_final_chain_last_block_number = [query_api = consensus_query_api]() {
      return (*query_api)->consensus_query_final_chain_last_block_number();
    };
    net::rpc::eth::FinalizedLogReplayApi log_replay_api;
    log_replay_api.latest_finalized_block_number = eth_rpc_params.query_final_chain_last_block_number;
    log_replay_api.blocks_with_bloom = [query_api = consensus_query_api](auto const &bloom, auto from, auto to) {
      return (*query_api)->consensus_query_final_chain_blocks_with_bloom(bloom, from, to);
    };
    log_replay_api.transaction_receipts_by_block_number = eth_rpc_params.query_transaction_receipts_by_block_number;
    eth_rpc_params.query_log_replay = std::move(log_replay_api);
    eth_rpc_params.query_account = [final_chain = app()->getFinalChain()](auto const &address, auto block_number) {
      return final_chain->getAccount(address, block_number);
    };
    eth_rpc_params.query_account_storage = [final_chain = app()->getFinalChain()](auto const &address, auto const &key,
                                                                                  auto block_number) {
      return final_chain->getAccountStorage(address, key, block_number);
    };
    eth_rpc_params.query_account_code = [final_chain = app()->getFinalChain()](auto const &address, auto block_number) {
      return final_chain->getCode(address, block_number);
    };
#endif
#ifdef RUSTAXA_ENABLE
    eth_rpc_params.send_trx = [application = app()->getConsensusApplication(), config = conf,
                               final_chain = app()->getFinalChain()](auto const &trx) {
      const auto report = application->submitTransaction(trx, config, *final_chain);
      if (!report.accepted) {
        BOOST_THROW_EXCEPTION(
            std::runtime_error(fmt("Transaction is rejected.\n"
                                   "RLP: %s\n"
                                   "Reason: %s",
                                   dev::toJS(trx->rlp()), report.message)));
      }
    };
#else
    eth_rpc_params.send_trx = [trx_manager = app()->getTransactionManager()](auto const &trx) {
      if (auto [ok, err_msg] = trx_manager->insertTransaction(trx); !ok) {
        BOOST_THROW_EXCEPTION(
            std::runtime_error(fmt("Transaction is rejected.\n"
                                   "RLP: %s\n"
                                   "Reason: %s",
                                   dev::toJS(trx->rlp()), err_msg)));
      }
    };
#endif
    eth_rpc_params.live_status = live_status_reader;

    auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));
    std::shared_ptr<net::Test> test_json_rpc;
    if (enable_test_rpc_) {
      //  TODO Because this object refers to App, the lifecycle/dependency management is more complicated);
      test_json_rpc =
          std::make_shared<net::Test>(app(), live_status_reader, net::TestTransactionApi{}, 0, net::TestNetworkReader{},
                                      net::TestNodeStatusReader{}, net::TestSortitionReader{}
#ifdef RUSTAXA_ENABLE
                                      ,
                                      consensus_query_api
#endif
          );
    }

    std::shared_ptr<net::Debug> debug_json_rpc;
    if (enable_debug_) {
      // TODO Because this object refers to App, the lifecycle/dependency management is more complicated);
      debug_json_rpc = std::make_shared<net::Debug>(app(), conf.genesis.dag.gas_limit, net::DebugDposReader{},
                                                    net::DebugTraceReader{}, net::DebugPreviousBlockCertVotesReader{},
                                                    net::DebugPeriodDagBlocksReader{},
                                                    net::DebugPeriodTransactionsReader{}, net::DebugTraceReplayReader{}
#ifdef RUSTAXA_ENABLE
                                                    ,
                                                    consensus_query_api
#endif
      );
    }

    jsonrpc_api_ = std::make_unique<JsonRpcServer>(
        std::make_shared<net::Taraxa>(app(), net::TaraxaDposReader{}, net::TaraxaDagStatusReader{},
                                      net::TaraxaDagBlockReader{}, net::TaraxaPersistentReader{},
                                      net::TaraxaScheduleReader{}, net::TaraxaNodeVersionReader{},
                                      net::TaraxaPillarBlockDataReader{}
#ifdef RUSTAXA_ENABLE
                                      ,
                                      consensus_query_api
#endif
                                      ),    // TODO Because this object refers to App, the
                                            // lifecycle/dependency management is more complicated
        std::make_shared<net::Net>(app()),  // TODO Because this object refers to App, the
                                            // lifecycle/dependency management is more complicated
        eth_json_rpc, test_json_rpc, debug_json_rpc);

    if (conf.network.rpc->http_port) {
      auto json_rpc_processor = std::make_shared<net::JsonRpcHttpProcessor>();
      jsonrpc_http_ = std::make_shared<net::HttpServer>(
          rpc_thread_pool_->unsafe_get_io_context(),
          boost::asio::ip::tcp::endpoint{conf.network.rpc->address, *conf.network.rpc->http_port}, app()->getAddress(),
          json_rpc_processor, jsonrpc_metrics);
      jsonrpc_api_->addConnector(json_rpc_processor);
      jsonrpc_http_->start();
    }
    if (conf.network.rpc->ws_port) {
      jsonrpc_ws_ = std::make_shared<net::JsonRpcWsServer>(
          rpc_thread_pool_->unsafe_get_io_context(),
          boost::asio::ip::tcp::endpoint{conf.network.rpc->address, *conf.network.rpc->ws_port}, app()->getAddress(),
          jsonrpc_metrics);
      jsonrpc_api_->addConnector(jsonrpc_ws_);
      jsonrpc_ws_->run();
    }
    if (!conf.db_config.rebuild_db) {
      app()->getFinalChain()->block_finalized_.subscribe(
          [eth_json_rpc = as_weak(eth_json_rpc), ws = as_weak(jsonrpc_ws_),
           db = as_weak(app()->getDB())](const auto &res) {
            if (auto _eth_json_rpc = eth_json_rpc.lock()) {
              _eth_json_rpc->note_block_executed(*res->final_chain_blk, res->trxs, res->trx_receipts);
            }
            if (auto _ws = ws.lock()) {
              if (_ws->numberOfSessions()) {
                auto trx_hashes = hashes_from_transactions(res->trxs);
                _ws->newEthBlock(*res->final_chain_blk, trx_hashes);
                _ws->newLogs(*res->final_chain_blk, std::move(trx_hashes), res->trx_receipts);
                if (auto _db = db.lock()) {
                  auto pbft_blk = _db->getPbftBlock(res->hash);
                  if (const auto &hash = pbft_blk->getPivotDagBlockHash(); hash != kNullBlockHash) {
                    _ws->newDagBlockFinalized(hash, pbft_blk->getPeriod());
                  }
                  _ws->newPbftBlockExecuted(*pbft_blk, res->dag_blk_hashes);
                }
              }
            }
          },
          rpc_thread_pool_);
    }

#ifdef RUSTAXA_ENABLE
    app()->getConsensusApplication()->transactionObserved().subscribe(
        [eth_json_rpc = as_weak(eth_json_rpc), ws = as_weak(jsonrpc_ws_)](const trx_hash_t &trx_hash) {
          if (auto rpc = eth_json_rpc.lock()) {
            rpc->note_pending_transaction(trx_hash);
          }
          if (auto socket = ws.lock()) {
            socket->newPendingTransaction(trx_hash);
          }
        },
        rpc_thread_pool_);
    app()->getConsensusApplication()->dagBlockObserved().subscribe(
        [ws = as_weak(jsonrpc_ws_)](const std::shared_ptr<DagBlock> &dag_block) {
          if (auto socket = ws.lock()) {
            socket->newDagBlock(dag_block);
          }
        },
        rpc_thread_pool_);
#else
    app()->getTransactionManager()->transaction_added_.subscribe(
        [eth_json_rpc = as_weak(eth_json_rpc), ws = as_weak(jsonrpc_ws_)](const auto &trx_hash) {
          if (auto _eth_json_rpc = eth_json_rpc.lock()) {
            _eth_json_rpc->note_pending_transaction(trx_hash);
          }
          if (auto _ws = ws.lock()) {
            _ws->newPendingTransaction(trx_hash);
          }
        },
        rpc_thread_pool_);
    app()->getDagManager()->block_verified_.subscribe(
        [eth_json_rpc = as_weak(eth_json_rpc), ws = as_weak(jsonrpc_ws_)](const auto &dag_block) {
          if (auto _ws = ws.lock()) {
            _ws->newDagBlock(dag_block);
          }
        },
        rpc_thread_pool_);
#endif

    app()->getPillarChainManager()->pillar_block_finalized_.subscribe(
        [ws_weak = as_weak(jsonrpc_ws_)](const auto &pillar_block_data) {
          if (auto ws = ws_weak.lock()) {
            ws->newPillarBlockData(pillar_block_data);
          }
        },
        rpc_thread_pool_);
  }
  if (conf.network.graphql) {
    graphql_thread_pool_ = std::make_shared<util::ThreadPool>(conf.network.graphql->threads_num);
    if (conf.network.graphql->ws_port) {
      graphql_ws_ = std::make_shared<net::GraphQlWsServer>(
          graphql_thread_pool_->unsafe_get_io_context(),
          boost::asio::ip::tcp::endpoint{conf.network.graphql->address, *conf.network.graphql->ws_port},
          app()->getAddress(), jsonrpc_metrics);
      // graphql_ws_->run();
    }

    if (conf.network.graphql->http_port) {
#ifndef RUSTAXA_ENABLE
      auto graphql_query = std::make_shared<graphql::taraxa::Query>(
          app()->getFinalChain(), app()->getDagManager(), app()->getPbftManager(), app()->getTransactionManager(),
          app()->getDB(), app()->getGasPricer(), as_weak(app()->getNetwork()), conf.genesis.chain_id,
          live_status_reader);
#else
      auto gas_price_reader = graphql::taraxa::QueryGasPriceReader{[query = consensus_query_api]() {
        return dev::fromBigEndian<dev::u256>((*query)->consensus_query_live_transaction_status().gas_price_bid);
      }};
      auto graphql_query = std::make_shared<graphql::taraxa::Query>(app()->getFinalChain(), std::move(gas_price_reader),
                                                                    as_weak(app()->getNetwork()), conf.genesis.chain_id,
                                                                    live_status_reader, consensus_query_api);
#endif
#ifdef RUSTAXA_ENABLE
      graphql::taraxa::MutationTransactionApi mutation_api;
      mutation_api.insert_transaction = [application = app()->getConsensusApplication(), config = conf,
                                         final_chain = app()->getFinalChain()](const SharedTransaction &trx) {
        const auto report = application->submitTransaction(trx, config, *final_chain);
        return std::pair{report.accepted, report.message};
      };
      auto graphql_mutation = std::make_shared<graphql::taraxa::Mutation>(std::move(mutation_api));
#else
      auto graphql_mutation = std::make_shared<graphql::taraxa::Mutation>(app()->getTransactionManager());
#endif
      graphql_http_ = std::make_shared<net::HttpServer>(
          graphql_thread_pool_->unsafe_get_io_context(),
          boost::asio::ip::tcp::endpoint{conf.network.graphql->address, *conf.network.graphql->http_port},
          app()->getAddress(),
          std::make_shared<net::GraphQlHttpProcessor>(
              net::GraphQlOperations{std::move(graphql_query), std::move(graphql_mutation), {}}),
          jsonrpc_metrics);
      graphql_http_->start();
    }
  }
}

void Rpc::shutdown() {
  jsonrpc_api_ = nullptr;  // TODO remove this line - we should not care about destroying objects explicitly, the
                           // lifecycle of objects should be as declarative as possible (RAII).
                           // This line is needed because jsonrpc_api_ indirectly refers to App (produces
                           // self-reference from App to App).
}

}  // namespace taraxa::plugin
