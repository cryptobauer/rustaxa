
#pragma once

#include <json/value.h>

#include <functional>
#include <memory>
#include <optional>
#include <vector>

#include "DebugFace.h"
#include "common/app_base.hpp"
#include "final_chain/state_api_data.hpp"

namespace taraxa {
class PbftVote;
struct Transaction;
}  // namespace taraxa

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

// DebugTraceReader is the debug RPC boundary for external EVM trace execution.
// Inputs are already materialized EVM transaction DTOs and a finalized block
// number; outputs are the legacy trace JSON string produced by the execution
// engine. Account and latest-block callbacks are used only to complete
// synthetic call parameters. Missing callbacks are completed from FinalChain by
// the Debug constructor and throw if used after the app expires.
struct DebugTraceReader {
  std::function<std::string(std::vector<state_api::EVMTransaction>, std::vector<state_api::EVMTransaction>,
                            EthBlockNumber, std::optional<state_api::Tracing>)>
      trace;
  std::function<std::optional<state_api::Account>(const Address&, EthBlockNumber)> account_at;
  std::function<EthBlockNumber()> latest_finalized_block_number;
};

// DebugPreviousBlockCertVotesView is the debug RPC view of the previous-block
// cert-vote bundle. Votes have already passed the compatibility validation
// step in the reader; the public RPC method only formats the values.
struct DebugPreviousBlockCertVotesView {
  bool found = false;
  uint64_t total_votes_count = 0;
  uint64_t round = 0;
  std::vector<std::shared_ptr<PbftVote>> votes;
};

// DebugPreviousBlockCertVotesReader is the debug RPC boundary for
// storage-backed previous-block cert-vote lookup and validation. The default
// adapter owns the temporary DbStorage/VoteManager/FinalChain compatibility
// reads while Rust-enabled nodes prefer ConsensusQueryApi inside that adapter.
struct DebugPreviousBlockCertVotesReader {
  std::function<DebugPreviousBlockCertVotesView(uint64_t)> cert_votes_by_period;
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
  explicit Debug(std::shared_ptr<taraxa::AppBase> app, uint64_t gas_limit, DebugDposReader dpos_reader = {},
                 DebugTraceReader trace_reader = {}, DebugPreviousBlockCertVotesReader previous_cert_votes_reader = {});
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
  DebugTraceReader trace_reader_;
  DebugPreviousBlockCertVotesReader previous_cert_votes_reader_;
  const uint64_t kGasLimit = ((uint64_t)1 << 53) - 1;
};

}  // namespace taraxa::net
