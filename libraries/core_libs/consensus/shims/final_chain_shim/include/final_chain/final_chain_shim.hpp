#pragma once

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
  h256 getAccountStorage(addr_t const& addr, u256 const& key, std::optional<EthBlockNumber> blk_n = {}) const;
  bytes getCode(addr_t const& addr, std::optional<EthBlockNumber> blk_n = {}) const;

  state_api::ExecutionResult call(state_api::EVMTransaction const& trx, std::optional<EthBlockNumber> blk_n = {}) const;
  std::string trace(std::vector<state_api::EVMTransaction> state_trxs, std::vector<state_api::EVMTransaction> trxs,
                    EthBlockNumber blk_n, std::optional<state_api::Tracing> params = {}) const;

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
  EthBlockNumber delegation_delay_ = 0;
  std::optional<::rust::Box<rustaxa::BridgeFinalChain>> rust_final_chain_;
};

}  // namespace taraxa::final_chain
