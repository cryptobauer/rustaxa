#pragma once

#include <atomic>

#include "final_chain/state_api.hpp"
#include "rewards/rewards_stats.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa::final_chain {

// Rust-mode final-chain shim facade.
// This class is a standalone surface in Rust-enabled builds and must not inherit
// or delegate behavior to FinalChainOld.
class FinalChain {
 protected:
  util::event::EventEmitter<std::shared_ptr<FinalizationResult>> const block_finalized_emitter_{};
  util::event::EventEmitter<uint64_t> const block_applying_emitter_{};

 public:
  decltype(block_finalized_emitter_)::Subscriber const& block_finalized_ = block_finalized_emitter_;
  decltype(block_applying_emitter_)::Subscriber const& block_applying_ = block_applying_emitter_;

  ~FinalChain() = default;
  FinalChain(const std::shared_ptr<DbStorage>& db, const taraxa::FullNodeConfig& config, const addr_t& node_addr);
  FinalChain(const FinalChain&) = delete;
  FinalChain(FinalChain&&) = delete;
  FinalChain& operator=(const FinalChain&) = delete;
  FinalChain& operator=(FinalChain&&) = delete;

  void stop();
  EthBlockNumber delegationDelay() const;

  std::future<std::shared_ptr<const FinalizationResult>> finalize(PeriodData&& period_data,
                                                                  std::vector<h256>&& finalized_dag_blk_hashes,
                                                                  uint32_t blocks_per_year,
                                                                  std::shared_ptr<DagBlock>&& anchor = nullptr);

  std::shared_ptr<const BlockHeader> blockHeader(std::optional<EthBlockNumber> n = {}) const;
  EthBlockNumber lastBlockNumber() const;
  std::optional<EthBlockNumber> blockNumber(h256 const& h) const;
  std::optional<h256> blockHash(std::optional<EthBlockNumber> n = {}) const;
  std::optional<h256> finalChainHash(EthBlockNumber n) const;
  void updateStateConfig(const state_api::Config& new_config);

  std::shared_ptr<const TransactionHashes> transactionHashes(std::optional<EthBlockNumber> n = {}) const;
  const SharedTransactions transactions(std::optional<EthBlockNumber> n = {}) const;
  std::optional<TransactionLocation> transactionLocation(h256 const& trx_hash) const;
  std::optional<TransactionReceipt> transactionReceipt(EthBlockNumber blk_n, uint64_t position,
                                                       std::optional<trx_hash_t> trx_hash = {}) const;
  std::shared_ptr<Transaction> transaction(EthBlockNumber blk_n, uint32_t position) const;
  uint64_t transactionCount(std::optional<EthBlockNumber> n = {}) const;
  std::vector<EthBlockNumber> withBlockBloom(LogBloom const& b, EthBlockNumber from, EthBlockNumber to) const;

  std::optional<state_api::Account> getAccount(addr_t const& addr, std::optional<EthBlockNumber> blk_n = {}) const;
  /**
   * Return the Rust FinalChain runtime used by Rust-owned consensus shims.
   *
   * This shim-owned accessor keeps Rust-mode integrations from re-entering the
   * C++ FinalChain API for data that is already owned by `BridgeFinalChain`.
   * Callers must not store the returned reference beyond the owning
   * `FinalChain` lifetime.
   */
  const rustaxa::BridgeFinalChain& rustFinalChainForRust() const;
  h256 getAccountStorage(addr_t const& addr, u256 const& key, std::optional<EthBlockNumber> blk_n = {}) const;
  bytes getCode(addr_t const& addr, std::optional<EthBlockNumber> blk_n = {}) const;

  state_api::ExecutionResult call(state_api::EVMTransaction const& trx, std::optional<EthBlockNumber> blk_n = {}) const;
  std::string trace(std::vector<state_api::EVMTransaction> state_trxs, std::vector<state_api::EVMTransaction> trxs,
                    EthBlockNumber blk_n, std::optional<state_api::Tracing> params = {}) const;

  // Returns the Rust-collected DPoS/VRF facts required by `DagManager::verifyBlock`.
  //
  // Inputs:
  // - consensus block number and sender address.
  //
  // Outputs are encoded in `rustaxa::DagDposAuthorizationFacts`; missing DPoS snapshots are represented as a status
  // value, while Rust bridge infrastructure failures still throw.
  rustaxa::DagDposAuthorizationFacts dagDposAuthorizationFacts(EthBlockNumber blk_num, addr_t const& addr) const;

  uint64_t dposEligibleTotalVoteCount(EthBlockNumber blk_num) const;
  uint64_t dposEligibleVoteCount(EthBlockNumber blk_num, addr_t const& addr) const;
  bool dposIsEligible(EthBlockNumber blk_num, addr_t const& addr) const;
  vrf_wrapper::vrf_pk_t dposGetVrfKey(EthBlockNumber blk_n, const addr_t& addr) const;
  void prune(EthBlockNumber blk_n);
  void waitForFinalized();

  std::vector<state_api::ValidatorStake> dposValidatorsTotalStakes(EthBlockNumber blk_num) const;
  uint256_t dposTotalAmountDelegated(EthBlockNumber blk_num) const;
  std::vector<state_api::ValidatorVoteCount> dposValidatorsEligibleVoteCounts(EthBlockNumber blk_num) const;
  uint64_t dposYield(EthBlockNumber blk_num) const;
  u256 dposTotalSupply(EthBlockNumber blk_num) const;
  h256 getBridgeRoot(EthBlockNumber blk_num) const;
  h256 getBridgeEpoch(EthBlockNumber blk_num) const;

  std::pair<val_t, bool> getBalance(addr_t const& addr) const;
  std::shared_ptr<const FinalizationResult> finalize_(PeriodData&& new_blk,
                                                      std::vector<h256>&& finalized_dag_blk_hashes,
                                                      uint32_t blocks_per_year, std::shared_ptr<DagBlock>&& anchor);
  SharedTransactionReceipts blockReceipts(std::optional<EthBlockNumber> n = {}) const;

 private:
  /**
   * Collect bridge-contract facts and materialize Rust-planned system transactions for an external-EVM period.
   *
   * The C++ executor boundary still owns bridge-contract state queries and the `shouldFinalizeEpoch()` dry run. Rust
   * owns the deterministic system transaction planning and canonical RLP construction. Returned `SystemTransaction`
   * objects are temporary materialization for `StateAPI` execution only.
   */
  std::vector<SharedTransaction> makeSystemTransactions(const rustaxa::FinalChainSystemTransactionRequest& request);

  /**
   * Execute and publish an external-EVM FinalChain session.
   *
   * Rust owns request identity, report validation, publication planning, and FinalChain storage writes. This shim
   * method owns only the temporary C++ executor side: bridge-contract state fact collection, `StateAPI` transaction
   * execution, rewards distribution, and staged `StateAPI` lifecycle commit.
   */
  std::shared_ptr<const FinalizationResult> finalizeExternalEvm(
      rust::Box<rustaxa::BridgeFinalChainExecutionSession> session, rustaxa::FinalChainExecutionStep initial_step,
      PeriodData&& period_data, std::vector<h256>&& finalized_dag_blk_hashes, uint32_t blocks_per_year,
      std::shared_ptr<DagBlock>&& anchor);

  std::shared_ptr<DbStorage> db_;
  std::optional<::rust::Box<rustaxa::BridgeFinalChain>> rust_final_chain_;
  EthBlockNumber delegation_delay_ = 0;
  uint64_t block_gas_limit_ = 0;
  uint32_t max_levels_per_period_ = 0;
  StateAPI state_api_;
  rewards::Stats rewards_;
  std::atomic<uint64_t> num_executed_dag_blk_ = 0;
  std::atomic<uint64_t> num_executed_trx_ = 0;
  const taraxa::FullNodeConfig& config_;
};

}  // namespace taraxa::final_chain
