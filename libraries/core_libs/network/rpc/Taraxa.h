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

class Taraxa : public TaraxaFace {
 public:
  explicit Taraxa(std::shared_ptr<taraxa::AppBase> app, TaraxaDposReader dpos_reader = {},
                  TaraxaDagStatusReader dag_status_reader = {}, TaraxaDagBlockReader dag_block_reader = {});

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

 private:
  Json::Value version;

  std::shared_ptr<taraxa::AppBase> tryGetApp();
};

}  // namespace taraxa::net
