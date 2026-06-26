#pragma once

#include <jsonrpccpp/common/exception.h>
#include <jsonrpccpp/server.h>
#include <libdevcore/Common.h>

#include <functional>
#include <memory>
#include <optional>
#include <vector>

#include "TaraxaFace.h"
#include "common/app_base.hpp"
#include "libweb3jsonrpc/ModularServer.h"

namespace taraxa {
class DagBlock;
struct Transaction;
}  // namespace taraxa

namespace taraxa::net {

// TaraxaDposReader is the Taraxa RPC boundary for DPoS facts that still live
// on the external FinalChain/StateAPI side of the rewrite. Inputs are finalized
// block numbers and, when needed, validator addresses; outputs are scalar DPoS
// values ready for public RPC JSON formatting. Missing callbacks are completed
// from the app's FinalChain by the Taraxa constructor.
struct TaraxaDposReader {
  std::function<uint64_t(EthBlockNumber)> eligible_total_vote_count;
  std::function<uint64_t(EthBlockNumber, const addr_t&)> eligible_vote_count;
  std::function<uint64_t(EthBlockNumber)> dpos_yield;
  std::function<u256(EthBlockNumber)> total_supply;
};

// TaraxaDagStatusReader is the Taraxa RPC boundary for live DAG status facts.
// It supplies the latest DAG level and proposal period without exposing
// DagManager to public RPC methods. Rust-mode production routes prefer
// ConsensusQueryApi when an app/storage handle is available.
struct TaraxaDagStatusReader {
  std::function<uint64_t()> latest_level;
  std::function<uint64_t()> latest_period;
};

// TaraxaDagBlockReader is the Taraxa RPC boundary for legacy DAG block
// materialization. It supplies block, finalized-period, and optional
// transaction payload facts without exposing DAG, PBFT, or transaction managers
// to public RPC methods. Rust-mode production routes prefer ConsensusQueryApi.
struct TaraxaDagBlockReader {
  std::function<std::shared_ptr<::taraxa::DagBlock>(const blk_hash_t&)> block_by_hash;
  std::function<std::vector<std::shared_ptr<::taraxa::DagBlock>>(level_t)> blocks_by_level;
  std::function<std::optional<uint64_t>(const blk_hash_t&)> period_by_hash;
  std::function<std::shared_ptr<::taraxa::Transaction>(const trx_hash_t&)> transaction_by_hash;
};

// TaraxaChainStatsView is the storage-backed chain summary formatted by
// taraxa_getChainStats. The fields are intentionally scalar so public RPC
// methods do not need FinalChain or DbStorage handles for legacy fallback
// materialization.
struct TaraxaChainStatsView {
  uint64_t pbft_period = 0;
  uint64_t dag_blocks_executed = 0;
  uint64_t transactions_executed = 0;
};

// TaraxaPersistentReader is the Taraxa RPC boundary for persisted consensus
// metadata still needed by legacy fallback paths. Rust-enabled production
// routes prefer ConsensusQueryApi when app storage is available; this reader is
// the audited compatibility point for scalar DbStorage/FinalChain facts.
struct TaraxaPersistentReader {
  std::function<std::optional<blk_hash_t>(uint64_t)> pbft_block_hash_by_period;
  std::function<TaraxaChainStatsView()> chain_stats;
  std::function<std::optional<uint64_t>(uint64_t)> period_lambda;
};

// TaraxaScheduleReader is the Taraxa RPC boundary for legacy PBFT schedule
// block materialization. It returns the public schedule JSON payload because
// the fallback path is a temporary RPC compatibility adapter around legacy
// PbftBlock formatting; Rust-mode production routes still use typed
// ConsensusQueryApi DTOs before formatting in the public method.
struct TaraxaScheduleReader {
  std::function<std::optional<Json::Value>(uint64_t)> schedule_block_by_period;
};

class Taraxa : public TaraxaFace {
 public:
  explicit Taraxa(std::shared_ptr<taraxa::AppBase> app, TaraxaDposReader dpos_reader = {},
                  TaraxaDagStatusReader dag_status_reader = {}, TaraxaDagBlockReader dag_block_reader = {},
                  TaraxaPersistentReader persistent_reader = {}, TaraxaScheduleReader schedule_reader = {});

  virtual RPCModules implementedModules() const override { return RPCModules{RPCModule{"taraxa", "1.0"}}; }

  virtual std::string taraxa_protocolVersion() override;
  virtual Json::Value taraxa_getVersion() override;
  virtual Json::Value taraxa_getDagBlockByHash(const std::string& _blockHash, bool _includeTransactions) override;
  virtual Json::Value taraxa_getDagBlockByLevel(const std::string& _blockLevel, bool _includeTransactions) override;
  virtual std::string taraxa_dagBlockLevel() override;
  virtual std::string taraxa_dagBlockPeriod() override;
  virtual Json::Value taraxa_getScheduleBlockByPeriod(const std::string& _period) override;
  virtual Json::Value taraxa_getNodeVersions() override;
  virtual std::string taraxa_pbftBlockHashByPeriod(const std::string& _period) override;
  virtual Json::Value taraxa_getConfig() override;
  virtual Json::Value taraxa_getChainStats() override;
  virtual std::string taraxa_yield(const std::string& _period) override;
  virtual std::string taraxa_totalSupply(const std::string& _period) override;
  virtual Json::Value taraxa_getPillarBlockData(const std::string& pillar_block_period,
                                                bool include_signatures) override;
  virtual std::string taraxa_getPeriodLambda(const std::string& period) override;

 protected:
  std::weak_ptr<taraxa::AppBase> app_;
  TaraxaDposReader dpos_reader_;
  TaraxaDagStatusReader dag_status_reader_;
  TaraxaDagBlockReader dag_block_reader_;
  TaraxaPersistentReader persistent_reader_;
  TaraxaScheduleReader schedule_reader_;

 private:
  Json::Value version;

  std::shared_ptr<taraxa::AppBase> tryGetApp();
};

}  // namespace taraxa::net
