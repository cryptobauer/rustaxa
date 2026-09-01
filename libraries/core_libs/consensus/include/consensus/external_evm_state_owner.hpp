#pragma once

#ifdef RUSTAXA_ENABLE

#include <memory>
#include <mutex>
#include <optional>
#include <vector>

#include "config/config.hpp"
#include "final_chain/data.hpp"
#include "final_chain/state_api.hpp"
#include "rustaxa-bridge/application_host_ffi.rs.h"

namespace taraxa {

class ConsensusApplication;

/**
 * Owns the process-long concrete EVM database used by native consensus.
 *
 * The owner serializes every StateAPI operation with one mutex, rejects reads
 * while a concrete execution is staged, and exposes only operation-shaped
 * requests. Native consensus remains authoritative for planning, publication,
 * retries, and finalized indexes. The owner must be bound to exactly one
 * ConsensusApplication before any query-dependent operation is invoked.
 */
class ExternalEvmStateOwner final {
 public:
  explicit ExternalEvmStateOwner(const FullNodeConfig& config);
  ~ExternalEvmStateOwner();

  ExternalEvmStateOwner(const ExternalEvmStateOwner&) = delete;
  ExternalEvmStateOwner(ExternalEvmStateOwner&&) = delete;
  ExternalEvmStateOwner& operator=(const ExternalEvmStateOwner&) = delete;
  ExternalEvmStateOwner& operator=(ExternalEvmStateOwner&&) = delete;

  /** Binds the native root used for canonical headers, native calls, and index pruning. */
  void bindApplication(const std::shared_ptr<ConsensusApplication>& application);
  /** Returns the concrete descriptor needed to restore the native application root. */
  state_api::StateDescriptor lastCommittedStateDescriptor() const;

  rustaxa::HostFinalChainSystemFactsReport loadSystemTransactionFacts(
      const rustaxa::HostFinalChainSystemFactsRequest& request);
  rustaxa::HostFinalChainPreflightReport loadCommittedState(const rustaxa::HostFinalChainPreflightRequest& request);
  rustaxa::HostFinalChainExecutionReport executeTransactions(const rustaxa::HostFinalChainExecutionRequest& request);
  rustaxa::HostFinalChainRewardsReport distributeRewards(const rustaxa::HostFinalChainRewardsRequest& request);
  rustaxa::HostFinalChainStateCommitReport commitState(const rustaxa::HostFinalChainStateCommitRequest& request);
  rustaxa::HostFinalChainPreflightReport discardState(const rustaxa::CanonicalBytes& concrete_marker);
  rustaxa::HostPillarAnchorStateReport loadPillarAnchorState(
      const rustaxa::HostPillarAnchorStateRequest& request) const;
  rustaxa::HostDagGasBatch estimateDagTransactionGas(const rustaxa::HostDagGasBatch& request) const;

  std::optional<state_api::Account> account(const addr_t& address,
                                            std::optional<EthBlockNumber> block_number = {}) const;
  h256 accountStorage(const addr_t& address, const u256& key, std::optional<EthBlockNumber> block_number = {}) const;
  bytes code(const addr_t& address, std::optional<EthBlockNumber> block_number = {}) const;
  state_api::ExecutionResult call(const state_api::EVMTransaction& transaction,
                                  std::optional<EthBlockNumber> block_number = {}) const;
  std::string trace(std::vector<state_api::EVMTransaction> state_transactions,
                    std::vector<state_api::EVMTransaction> transactions, EthBlockNumber block_number,
                    std::optional<state_api::Tracing> params = {}) const;
  /** Prunes concrete state and native finalized indexes at one operation boundary. */
  void prune(EthBlockNumber block_number);

 private:
  std::shared_ptr<ConsensusApplication> application() const;
  std::shared_ptr<const final_chain::BlockHeader> blockHeader(EthBlockNumber block_number) const;
  EthBlockNumber lastBlockNumber() const;
  h256 blockHash(EthBlockNumber block_number) const;
  h256 readBridgeContractHash(EthBlockNumber block_number, const bytes& method, const char* operation) const;
  void ensureReadableLocked() const;

  mutable std::mutex mutex_;
  mutable std::mutex application_mutex_;
  std::weak_ptr<ConsensusApplication> application_;
  StateAPI state_api_;
  addr_t bridge_contract_address_;
};

/** Converts genesis state into the native application bootstrap account carrier. */
rust::Vec<rustaxa::GenesisAccount> makeGenesisAccounts(const state_api::Config& config);
/** Converts genesis validators into the native application bootstrap carrier. */
rust::Vec<rustaxa::GenesisValidator> makeGenesisValidators(const state_api::Config& config);
/** Converts DPoS policy into the native application bootstrap carrier. */
rustaxa::GenesisDposConfig makeGenesisDposConfig(const state_api::DPOSConfig& config,
                                                 uint64_t dag_vdf_sortition_total_vote_count_until_period);
/** Converts rewards and hardfork policy into the native application bootstrap carrier. */
rustaxa::FinalChainRewardsConfig makeFinalChainRewardsConfig(const FullNodeConfig& config);

}  // namespace taraxa

#endif  // RUSTAXA_ENABLE
