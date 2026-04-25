#include "final_chain/final_chain.hpp"

#include <utility>

namespace taraxa::final_chain {

FinalChain::FinalChain(const std::shared_ptr<DbStorage>& db, const taraxa::FullNodeConfig& config,
                       const addr_t& node_addr)
    : FinalChainOld(db, config, node_addr) {
  rust_final_chain_ = rustaxa::create_final_chain(db->rustStorage());
}

void FinalChain::stop() { FinalChainOld::stop(); }

EthBlockNumber FinalChain::delegationDelay() const { return FinalChainOld::delegationDelay(); }

std::future<std::shared_ptr<const FinalizationResult>> FinalChain::finalize(
    PeriodData&& period_data, std::vector<h256>&& finalized_dag_blk_hashes, uint32_t blocks_per_year,
    std::shared_ptr<DagBlock>&& anchor) {
  return FinalChainOld::finalize(std::move(period_data), std::move(finalized_dag_blk_hashes), blocks_per_year,
                                 std::move(anchor));
}

std::shared_ptr<const BlockHeader> FinalChain::blockHeader(std::optional<EthBlockNumber> n) const {
  return FinalChainOld::blockHeader(n);
}

EthBlockNumber FinalChain::lastBlockNumber() const { return FinalChainOld::lastBlockNumber(); }

std::optional<EthBlockNumber> FinalChain::blockNumber(h256 const& h) const { return FinalChainOld::blockNumber(h); }

std::optional<h256> FinalChain::blockHash(std::optional<EthBlockNumber> n) const {
  return FinalChainOld::blockHash(n);
}

std::optional<h256> FinalChain::finalChainHash(EthBlockNumber n) const { return FinalChainOld::finalChainHash(n); }

void FinalChain::updateStateConfig(const state_api::Config& new_config) {
  FinalChainOld::updateStateConfig(new_config);
}

std::shared_ptr<const TransactionHashes> FinalChain::transactionHashes(std::optional<EthBlockNumber> n) const {
  return FinalChainOld::transactionHashes(n);
}

const SharedTransactions FinalChain::transactions(std::optional<EthBlockNumber> n) const {
  return FinalChainOld::transactions(n);
}

std::optional<TransactionLocation> FinalChain::transactionLocation(h256 const& trx_hash) const {
  return FinalChainOld::transactionLocation(trx_hash);
}

std::optional<TransactionReceipt> FinalChain::transactionReceipt(EthBlockNumber blk_n, uint64_t position,
                                                                 std::optional<trx_hash_t> trx_hash) const {
  return FinalChainOld::transactionReceipt(blk_n, position, trx_hash);
}

std::shared_ptr<Transaction> FinalChain::transaction(EthBlockNumber blk_n, uint32_t position) const {
  return FinalChainOld::transaction(blk_n, position);
}

uint64_t FinalChain::transactionCount(std::optional<EthBlockNumber> n) const {
  return FinalChainOld::transactionCount(n);
}

std::vector<EthBlockNumber> FinalChain::withBlockBloom(LogBloom const& b, EthBlockNumber from,
                                                       EthBlockNumber to) const {
  return FinalChainOld::withBlockBloom(b, from, to);
}

std::optional<state_api::Account> FinalChain::getAccount(addr_t const& addr,
                                                         std::optional<EthBlockNumber> blk_n) const {
  return FinalChainOld::getAccount(addr, blk_n);
}

h256 FinalChain::getAccountStorage(addr_t const& addr, u256 const& key, std::optional<EthBlockNumber> blk_n) const {
  return FinalChainOld::getAccountStorage(addr, key, blk_n);
}

bytes FinalChain::getCode(addr_t const& addr, std::optional<EthBlockNumber> blk_n) const {
  return FinalChainOld::getCode(addr, blk_n);
}

state_api::ExecutionResult FinalChain::call(state_api::EVMTransaction const& trx,
                                            std::optional<EthBlockNumber> blk_n) const {
  return FinalChainOld::call(trx, blk_n);
}

std::string FinalChain::trace(std::vector<state_api::EVMTransaction> state_trxs,
                              std::vector<state_api::EVMTransaction> trxs, EthBlockNumber blk_n,
                              std::optional<state_api::Tracing> params) const {
  return FinalChainOld::trace(std::move(state_trxs), std::move(trxs), blk_n, std::move(params));
}

uint64_t FinalChain::dposEligibleTotalVoteCount(EthBlockNumber blk_num) const {
  return FinalChainOld::dposEligibleTotalVoteCount(blk_num);
}

uint64_t FinalChain::dposEligibleVoteCount(EthBlockNumber blk_num, addr_t const& addr) const {
  return FinalChainOld::dposEligibleVoteCount(blk_num, addr);
}

bool FinalChain::dposIsEligible(EthBlockNumber blk_num, addr_t const& addr) const {
  return FinalChainOld::dposIsEligible(blk_num, addr);
}

vrf_wrapper::vrf_pk_t FinalChain::dposGetVrfKey(EthBlockNumber blk_n, const addr_t& addr) const {
  return FinalChainOld::dposGetVrfKey(blk_n, addr);
}

void FinalChain::prune(EthBlockNumber blk_n) { FinalChainOld::prune(blk_n); }

void FinalChain::waitForFinalized() { FinalChainOld::waitForFinalized(); }

std::vector<state_api::ValidatorStake> FinalChain::dposValidatorsTotalStakes(EthBlockNumber blk_num) const {
  return FinalChainOld::dposValidatorsTotalStakes(blk_num);
}

uint256_t FinalChain::dposTotalAmountDelegated(EthBlockNumber blk_num) const {
  return FinalChainOld::dposTotalAmountDelegated(blk_num);
}

std::vector<state_api::ValidatorVoteCount> FinalChain::dposValidatorsEligibleVoteCounts(EthBlockNumber blk_num) const {
  return FinalChainOld::dposValidatorsEligibleVoteCounts(blk_num);
}

uint64_t FinalChain::dposYield(EthBlockNumber blk_num) const { return FinalChainOld::dposYield(blk_num); }

u256 FinalChain::dposTotalSupply(EthBlockNumber blk_num) const { return FinalChainOld::dposTotalSupply(blk_num); }

h256 FinalChain::getBridgeRoot(EthBlockNumber blk_num) const { return FinalChainOld::getBridgeRoot(blk_num); }

h256 FinalChain::getBridgeEpoch(EthBlockNumber blk_num) const { return FinalChainOld::getBridgeEpoch(blk_num); }

std::pair<val_t, bool> FinalChain::getBalance(addr_t const& addr) const { return FinalChainOld::getBalance(addr); }

std::shared_ptr<const FinalizationResult> FinalChain::finalize_(PeriodData&& new_blk,
                                                                std::vector<h256>&& finalized_dag_blk_hashes,
                                                                uint32_t blocks_per_year,
                                                                std::shared_ptr<DagBlock>&& anchor) {
  return FinalChainOld::finalize_(std::move(new_blk), std::move(finalized_dag_blk_hashes), blocks_per_year,
                                  std::move(anchor));
}

SharedTransactionReceipts FinalChain::blockReceipts(std::optional<EthBlockNumber> n) const {
  return FinalChainOld::blockReceipts(n);
}

}  // namespace taraxa::final_chain
