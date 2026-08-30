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
#include "config/config.hpp"
#include "consensus/consensus_application.hpp"
#include "final_chain/data.hpp"
#include "final_chain/state_api.hpp"
#include "rustaxa-bridge/application_host_ffi.rs.h"
#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"

namespace taraxa {
class ExternalEvmPort;
class DagManager;
class PbftManager;
class VoteManager;
}  // namespace taraxa
namespace taraxa::network {
class ConsensusNetworkApi;
}

namespace taraxa::final_chain {

/** Converts genesis state into the native application bootstrap account carrier. */
rust::Vec<rustaxa::GenesisAccount> makeGenesisAccounts(const state_api::Config& config);
/** Converts genesis validators into the native application bootstrap carrier. */
rust::Vec<rustaxa::GenesisValidator> makeGenesisValidators(const state_api::Config& config);
/** Converts DPoS policy into the native application bootstrap carrier. */
rustaxa::GenesisDposConfig makeGenesisDposConfig(const state_api::DPOSConfig& config,
                                                 uint64_t dag_vdf_sortition_total_vote_count_until_period);
/** Converts rewards and hardfork policy into the native application bootstrap carrier. */
rustaxa::FinalChainRewardsConfig makeFinalChainRewardsConfig(const taraxa::FullNodeConfig& config);

// Rust-mode final-chain shim facade.
// This class is a standalone surface in Rust-enabled builds.
class FinalChain {
  friend class ::taraxa::ExternalEvmPort;
  friend class ::taraxa::DagManager;
  friend class ::taraxa::PbftManager;
  friend class ::taraxa::VoteManager;
  friend class ::taraxa::network::ConsensusNetworkApi;

 protected:
  util::event::EventEmitter<uint64_t> const block_applying_emitter_{};

 public:
  decltype(block_applying_emitter_)::Subscriber const& block_applying_ = block_applying_emitter_;

  ~FinalChain() = default;
  FinalChain(const fs::path& state_db_path, const taraxa::FullNodeConfig& config,
             [[maybe_unused]] const addr_t& node_addr, SharedConsensusApplication consensus_application);
  FinalChain(const FinalChain&) = delete;
  FinalChain(FinalChain&&) = delete;

  FinalChain& operator=(const FinalChain&) = delete;
  FinalChain& operator=(FinalChain&&) = delete;

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
  /** Prunes finalized indexes and available external-EVM history before `blk_n`.
   * Missing or not-yet-executed EVM blocks retain external state; StateAPI and
   * storage failures propagate while Rust remains authoritative for indexes. */
  void prune(EthBlockNumber blk_n);
 private:
  /** Internal concrete-EVM header lookup; public readers use `ConsensusQueryApi`. */
  std::shared_ptr<const BlockHeader> blockHeader(std::optional<EthBlockNumber> n = {}) const;
  /** Internal concrete-EVM head lookup; public readers use `ConsensusQueryApi`. */
  EthBlockNumber lastBlockNumber() const;
  /** Internal StateAPI hash callback; public readers use `ConsensusQueryApi`. */
  std::optional<h256> blockHash(std::optional<EthBlockNumber> n = {}) const;
  /** Exact bridge-contract roots consumed only by the application execution port. */
  h256 getBridgeRoot(EthBlockNumber blk_num) const;
  h256 getBridgeEpoch(EthBlockNumber blk_num) const;
  /**
   * Thin adapter for the external EVM `StateAPI` client used by Rust-enabled FinalChain publication.
   *
   * Inputs are Rust bridge request DTOs plus the legacy C++ transaction material that the external executor still
   * requires. Reward statistics arrive as Rust-produced canonical RLP and are decoded only inside the rewards call.
   * Outputs are exact Rust bridge reports. The adapter is the only Rust-mode finalization helper that may execute
   * arbitrary EVM work, query
   * bridge-contract state, distribute rewards in `StateAPI`, or commit `state_db/`; it does not publish Rust FinalChain
   * storage or decide consensus session state.
   */
  class ExternalEvmStateApiClient {
   public:
    ExternalEvmStateApiClient(StateAPI& state_api, std::mutex& state_api_mutex);

    rustaxa::HostFinalChainSystemFactsReport loadSystemTransactionFacts(
        const rustaxa::HostFinalChainSystemFactsRequest& request);
    rustaxa::HostFinalChainPreflightReport loadCommittedState(
        const rustaxa::HostFinalChainPreflightRequest& request) const;
    rustaxa::HostFinalChainExecutionReport executeTransactions(const rustaxa::HostFinalChainExecutionRequest& request);
    rustaxa::HostFinalChainRewardsReport distributeRewards(const rustaxa::HostFinalChainRewardsRequest& request);
    rustaxa::HostFinalChainStateCommitReport commitState(const rustaxa::HostFinalChainStateCommitRequest& request);

    state_api::StateDescriptor lastCommittedStateDescriptor() const;
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

  /** Read one 32-byte bridge-contract view at the latest applicable committed EVM snapshot; failures return zero. */
  h256 readBridgeContractHash(EthBlockNumber block_number, const bytes& method, const char* api_name) const;

  /**
   * Complete any Rust-owned external-EVM FinalChain publication left pending by
   * a crash after `StateAPI::transition_state_commit()`.
   *
   * The Rust recovery path owns marker validation, rewards-cache recovery, and storage publication. This shim supplies
   * only the committed `StateAPI` descriptor and returns after Rust accepts or rejects that descriptor.
   */
  void recoverExternalEvmPendingPublication();

  SharedConsensusApplication consensus_application_;
  mutable std::mutex state_api_mutex_;
  StateAPI state_api_;
  ExternalEvmStateApiClient external_evm_state_api_;
  const taraxa::FullNodeConfig& config_;
};

}  // namespace taraxa::final_chain
