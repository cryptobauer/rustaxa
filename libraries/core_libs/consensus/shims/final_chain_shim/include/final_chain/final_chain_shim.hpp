#pragma once

#include <atomic>
#include <cstdint>
#include <future>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include "common/event.hpp"
#include "common/types.hpp"
#include "common/vrf_wrapper.hpp"
#include "config/config.hpp"
#include "final_chain/data.hpp"
#include "final_chain/state_api.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"

namespace taraxa {
class DagManager;
}

namespace taraxa::final_chain {

// Rust-mode final-chain shim facade.
// This class is a standalone surface in Rust-enabled builds.
class FinalChain {
  friend class ::taraxa::DagManager;

 protected:
  util::event::EventEmitter<std::shared_ptr<FinalizationResult>> const block_finalized_emitter_{};
  util::event::EventEmitter<uint64_t> const block_applying_emitter_{};

 public:
  decltype(block_finalized_emitter_)::Subscriber const& block_finalized_ = block_finalized_emitter_;
  decltype(block_applying_emitter_)::Subscriber const& block_applying_ = block_applying_emitter_;

  ~FinalChain() = default;
  FinalChain(const std::shared_ptr<DbStorage>& db, const taraxa::FullNodeConfig& config,
             [[maybe_unused]] const addr_t& node_addr);
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

  /**
   * Looks up account state at a finalized block.
   *
   * Rust mode first uses the external EVM state API for blocks that are
   * committed there and otherwise uses the Rust account snapshot sidecar. If a
   * polled EVM read races a Rust FinalChain publication and reports a future
   * block, the method returns `std::nullopt` so callers can retry instead of
   * aborting the consensus/RPC test process.
   */
  std::optional<state_api::Account> getAccount(addr_t const& addr, std::optional<EthBlockNumber> blk_n = {}) const;
  /**
   * Looks up contract storage through the external EVM state API.
   *
   * Future-state races return the zero hash, matching an unavailable storage
   * value for retry-oriented callers while the Rust storage sidecar remains the
   * durable FinalChain block owner.
   */
  h256 getAccountStorage(addr_t const& addr, u256 const& key, std::optional<EthBlockNumber> blk_n = {}) const;
  /**
   * Looks up account bytecode through the external EVM state API.
   *
   * If the requested finalized block is visible in Rust storage but not yet
   * committed in the external EVM state database, returns an empty byte vector
   * so RPC/test polling can wait for publication to finish.
   */
  bytes getCode(addr_t const& addr, std::optional<EthBlockNumber> blk_n = {}) const;

  /**
   * Executes a read-only call against committed external EVM state or the Rust
   * native-call subset.
   *
   * External EVM future-state races are reported as `consensus_err` instead of
   * escaping as exceptions, preserving the existing RPC error surface.
   */
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
  rustaxa::PbftFinalChainFacts collectPbftFinalChainFacts(rustaxa::PbftFinalChainFactRequest request) const;
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
   * Thin adapter for the external EVM `StateAPI` client used by Rust-enabled FinalChain publication.
   *
   * Inputs are Rust bridge request DTOs plus the legacy C++ transaction material that the external executor still
   * requires. Reward statistics arrive as Rust-produced canonical RLP and are decoded only inside the rewards call.
   * Outputs are Rust bridge report DTOs and temporary C++ receipts needed to preserve the public `FinalizationResult`
   * surface. The adapter is the only Rust-mode finalization helper that may execute arbitrary EVM work, query
   * bridge-contract state, distribute rewards in `StateAPI`, or commit `state_db/`; it does not publish Rust FinalChain
   * storage or decide consensus session state.
   */
  class ExternalEvmStateApiClient {
   public:
    struct ExecutionOutcome {
      rustaxa::FinalChainEvmExecutionReport report;
      TransactionReceipts receipts;
    };

    struct RewardsOutcome {
      rustaxa::FinalChainEvmRewardsReport report;
    };

    ExternalEvmStateApiClient(StateAPI& state_api, std::mutex& state_api_mutex);

    rustaxa::FinalChainSystemTransactionPlanFact collectSystemTransactionFacts(
        const rustaxa::FinalChainSystemTransactionRequest& request, bool is_pillar_block_period,
        uint64_t block_gas_limit, const addr_t& bridge_contract_address);

    ExecutionOutcome executeTransactions(const rustaxa::FinalChainEvmExecutionRequest& request,
                                         const std::vector<SharedTransaction>& transactions, const addr_t& beneficiary,
                                         uint64_t block_gas_limit, uint64_t timestamp);

    /**
     * Executes only the external `StateAPI` rewards-distribution effect requested by Rust.
     *
     * Rust supplies canonical RLP for the complete ordered distribution-stat set. This adapter decodes those values
     * into temporary legacy `BlockStats` objects only for the duration of `StateAPI::distribute_rewards`, then returns
     * the resulting state root and total reward correlated to the request. Malformed RLP and StateAPI failures
     * propagate as exceptions; no Rust FinalChain storage or rewards-cache state is mutated here.
     */
    RewardsOutcome distributeRewards(const rustaxa::FinalChainEvmRewardsRequest& request);

    rustaxa::FinalChainExternalEvmStateCommitResult commitState();

    state_api::StateDescriptor lastCommittedStateDescriptor() const;
    void updateStateConfig(const state_api::Config& new_config, EthBlockNumber& delegation_delay);
    std::optional<state_api::Account> account(EthBlockNumber block_number, const addr_t& address) const;
    h256 accountStorageOrZero(EthBlockNumber block_number, const addr_t& address, const u256& key) const;
    bytes codeOrEmpty(EthBlockNumber block_number, const addr_t& address) const;
    state_api::ExecutionResult dryRunTransaction(const BlockHeader& block_header,
                                                 const state_api::EVMTransaction& transaction, bool lock_client) const;
    bytes traceTransactions(const BlockHeader& block_header, const std::vector<state_api::EVMTransaction>& state_trxs,
                            const std::vector<state_api::EVMTransaction>& trxs,
                            std::optional<state_api::Tracing> params) const;
    bool accountHasCode(EthBlockNumber block_number, const addr_t& address) const;

   private:
    StateAPI& state_api_;
    std::mutex& state_api_mutex_;
  };

  /**
   * Collect bridge-contract facts and materialize Rust-planned system transactions for an external-EVM period.
   *
   * The C++ executor boundary still owns bridge-contract state queries and the `shouldFinalizeEpoch()` dry run. Rust
   * owns the deterministic system transaction planning and canonical RLP construction. Returned `SystemTransaction`
   * objects are temporary materialization for `StateAPI` execution only.
   */
  std::vector<SharedTransaction> makeSystemTransactions(const rustaxa::FinalChainSystemTransactionRequest& request);

  /**
   * Complete any Rust-owned external-EVM FinalChain publication left pending by
   * a crash after `StateAPI::transition_state_commit()`.
   *
   * The Rust recovery path owns marker validation, rewards-cache recovery, and storage publication. This shim supplies
   * only the committed `StateAPI` descriptor and returns after Rust accepts or rejects that descriptor.
   */
  void recoverExternalEvmPendingPublication();

  /**
   * Execute and publish an external-EVM FinalChain session.
   *
   * Rust owns request identity, report validation, publication planning, and FinalChain storage writes. This shim
   * method owns only the temporary C++ executor side: bridge-contract state fact collection, `StateAPI` transaction
   * execution, local RLP-to-`BlockStats` rewards materialization, rewards distribution, and staged `StateAPI` lifecycle
   * commit. Rust owns rewards-stat planning, cache mutation, persistence, and publication recovery.
   */
  std::shared_ptr<const FinalizationResult> finalizeExternalEvm(
      rust::Box<rustaxa::BridgeFinalChainExecutionSession> session, rustaxa::FinalChainExecutionStep initial_step,
      PeriodData&& period_data, std::vector<h256>&& finalized_dag_blk_hashes, std::shared_ptr<DagBlock>&& anchor);

  std::shared_ptr<DbStorage> db_;
  std::optional<::rust::Box<rustaxa::BridgeFinalChain>> rust_final_chain_;
  std::optional<::rust::Box<rustaxa::BridgeConsensusExecutionApi>> rust_execution_api_;
  EthBlockNumber delegation_delay_ = 0;
  uint64_t block_gas_limit_ = 0;
  uint32_t max_levels_per_period_ = 0;
  mutable std::mutex state_api_mutex_;
  StateAPI state_api_;
  ExternalEvmStateApiClient external_evm_state_api_;
  std::atomic<uint64_t> num_executed_dag_blk_ = 0;
  std::atomic<uint64_t> num_executed_trx_ = 0;
  const taraxa::FullNodeConfig& config_;
};

}  // namespace taraxa::final_chain
