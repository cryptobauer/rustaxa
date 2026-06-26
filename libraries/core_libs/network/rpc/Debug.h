
#pragma once

#include <json/value.h>

#include <functional>
#include <memory>
#include <vector>

#include "DebugFace.h"
#include "common/app_base.hpp"
#include "final_chain/state_api_data.hpp"

namespace taraxa {
struct Transaction;
}

namespace taraxa::state_api {
struct TransactionReceipt;
struct EVMTransaction;
struct Tracing;
}  // namespace taraxa::state_api

namespace dev::eth {
class Client;
}

namespace taraxa::net {

// DebugDposReader is the debug RPC boundary for DPoS facts that still live on
// the external FinalChain/StateAPI side of the rewrite. Inputs are finalized
// block numbers; outputs are plain DPoS values ready for debug JSON
// formatting. Missing callbacks are completed from the app's FinalChain by the
// Debug constructor and throw only if no app/default reader is available.
struct DebugDposReader {
  std::function<uint64_t(EthBlockNumber)> eligible_total_vote_count;
  std::function<std::vector<state_api::ValidatorStake>(EthBlockNumber)> validators_total_stakes;
  std::function<uint256_t(EthBlockNumber)> total_amount_delegated;
};

class InvalidAddress : public std::exception {
 public:
  virtual const char* what() const noexcept { return "Invalid account address"; }
};

class InvalidTracingParams : public std::exception {
 public:
  virtual const char* what() const noexcept { return "Invalid tracing params"; }
};

class Debug : public DebugFace {
 public:
  explicit Debug(std::shared_ptr<taraxa::AppBase> app, uint64_t gas_limit, DebugDposReader dpos_reader = {});
  virtual RPCModules implementedModules() const override { return RPCModules{RPCModule{"debug", "1.0"}}; }

  virtual Json::Value debug_traceTransaction(const std::string& param1) override;
  virtual Json::Value debug_traceCall(const Json::Value& param1, const std::string& param2) override;
  virtual Json::Value debug_getPeriodTransactionsWithReceipts(const std::string& _period) override;
  virtual Json::Value debug_getPeriodDagBlocks(const std::string& _period) override;
  virtual Json::Value debug_getPreviousBlockCertVotes(const std::string& _period) override;
  virtual Json::Value trace_call(const Json::Value& param1, const Json::Value& param2,
                                 const std::string& param3) override;
  virtual Json::Value trace_replayTransaction(const std::string& param1, const Json::Value& param2) override;
  virtual Json::Value trace_replayBlockTransactions(const std::string& param1, const Json::Value& param2) override;
  virtual Json::Value debug_dposValidatorTotalStakes(const std::string& param1) override;
  virtual Json::Value debug_dposTotalAmountDelegated(const std::string& param1) override;

 private:
  state_api::EVMTransaction to_eth_trx(std::shared_ptr<Transaction> t) const;
  state_api::EVMTransaction to_eth_trx(const Json::Value& json, EthBlockNumber blk_num);
  std::vector<state_api::EVMTransaction> to_eth_trxs(const std::vector<std::shared_ptr<Transaction>>& trxs);
  EthBlockNumber parse_blk_num(const std::string& blk_num_str);
  state_api::Tracing parse_tracking_parms(const Json::Value& json) const;
  Address to_address(const std::string& s) const;
  std::tuple<std::vector<state_api::EVMTransaction>, state_api::EVMTransaction, uint64_t> get_transaction_with_state(
      const std::string& transaction_hash);

  std::weak_ptr<taraxa::AppBase> app_;
  DebugDposReader dpos_reader_;
  const uint64_t kGasLimit = ((uint64_t)1 << 53) - 1;
};

}  // namespace taraxa::net
