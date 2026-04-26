#include <array>
#include <cstring>
#include <string>

#include "final_chain/final_chain.hpp"

namespace taraxa::final_chain {
namespace {

[[noreturn]] void throw_unimplemented_final_chain_api(const char* api_name) {
  throw DbException("FinalChain::" + std::string(api_name) + " is not implemented in Rust shim mode");
}

std::array<uint8_t, 32> into_bytes_array(const h256& hash) {
  std::array<uint8_t, 32> bytes{};
  std::memcpy(bytes.data(), hash.data(), bytes.size());
  return bytes;
}

h256 into_h256(const rust::Vec<uint8_t>& bytes, const char* api_name) {
  if (bytes.size() != 32) {
    throw DbException("FinalChain::" + std::string(api_name) + " returned invalid hash size: expected 32, got " +
                      std::to_string(bytes.size()));
  }
  return h256(dev::bytes(bytes.begin(), bytes.end()));
}

}  // namespace

FinalChain::FinalChain(const std::shared_ptr<DbStorage>& db, const taraxa::FullNodeConfig& config,
                       const addr_t& node_addr)
    : FinalChainOld(db, config, node_addr) {
  rust_final_chain_ = rustaxa::create_final_chain(db->rustStorage());
}

void FinalChain::stop() { throw_unimplemented_final_chain_api("stop"); }

EthBlockNumber FinalChain::delegationDelay() const { throw_unimplemented_final_chain_api("delegationDelay"); }

std::future<std::shared_ptr<const FinalizationResult>> FinalChain::finalize(PeriodData&&, std::vector<h256>&&, uint32_t,
                                                                            std::shared_ptr<DagBlock>&&) {
  throw_unimplemented_final_chain_api("finalize");
}

std::shared_ptr<const BlockHeader> FinalChain::blockHeader(std::optional<EthBlockNumber>) const {
  throw_unimplemented_final_chain_api("blockHeader");
}

EthBlockNumber FinalChain::lastBlockNumber() const { return rust_final_chain_.value()->get_last_block_number(); }

std::optional<EthBlockNumber> FinalChain::blockNumber(h256 const& h) const {
  auto rust_lookup = rust_final_chain_.value()->get_block_number(into_bytes_array(h));
  if (!rust_lookup.found) {
    return std::nullopt;
  }
  return rust_lookup.value;
}

std::optional<h256> FinalChain::blockHash(std::optional<EthBlockNumber> n) const {
  auto const block_number = n.value_or(lastBlockNumber());
  auto rust_hash = rust_final_chain_.value()->get_block_hash(static_cast<uint64_t>(block_number));
  if (rust_hash.empty()) {
    return std::nullopt;
  }
  return into_h256(rust_hash, "blockHash");
}

std::optional<h256> FinalChain::finalChainHash(EthBlockNumber) const {
  throw_unimplemented_final_chain_api("finalChainHash");
}

void FinalChain::updateStateConfig(const state_api::Config&) {
  throw_unimplemented_final_chain_api("updateStateConfig");
}

std::shared_ptr<const TransactionHashes> FinalChain::transactionHashes(std::optional<EthBlockNumber>) const {
  throw_unimplemented_final_chain_api("transactionHashes");
}

const SharedTransactions FinalChain::transactions(std::optional<EthBlockNumber>) const {
  throw_unimplemented_final_chain_api("transactions");
}

std::optional<TransactionLocation> FinalChain::transactionLocation(h256 const&) const {
  throw_unimplemented_final_chain_api("transactionLocation");
}

std::optional<TransactionReceipt> FinalChain::transactionReceipt(EthBlockNumber, uint64_t,
                                                                 std::optional<trx_hash_t>) const {
  throw_unimplemented_final_chain_api("transactionReceipt");
}

std::shared_ptr<Transaction> FinalChain::transaction(EthBlockNumber, uint32_t) const {
  throw_unimplemented_final_chain_api("transaction");
}

uint64_t FinalChain::transactionCount(std::optional<EthBlockNumber>) const {
  throw_unimplemented_final_chain_api("transactionCount");
}

std::vector<EthBlockNumber> FinalChain::withBlockBloom(LogBloom const&, EthBlockNumber, EthBlockNumber) const {
  throw_unimplemented_final_chain_api("withBlockBloom");
}

std::optional<state_api::Account> FinalChain::getAccount(addr_t const&, std::optional<EthBlockNumber>) const {
  throw_unimplemented_final_chain_api("getAccount");
}

h256 FinalChain::getAccountStorage(addr_t const&, u256 const&, std::optional<EthBlockNumber>) const {
  throw_unimplemented_final_chain_api("getAccountStorage");
}

bytes FinalChain::getCode(addr_t const&, std::optional<EthBlockNumber>) const {
  throw_unimplemented_final_chain_api("getCode");
}

state_api::ExecutionResult FinalChain::call(state_api::EVMTransaction const&, std::optional<EthBlockNumber>) const {
  throw_unimplemented_final_chain_api("call");
}

std::string FinalChain::trace(std::vector<state_api::EVMTransaction>, std::vector<state_api::EVMTransaction>,
                              EthBlockNumber, std::optional<state_api::Tracing>) const {
  throw_unimplemented_final_chain_api("trace");
}

uint64_t FinalChain::dposEligibleTotalVoteCount(EthBlockNumber) const {
  throw_unimplemented_final_chain_api("dposEligibleTotalVoteCount");
}

uint64_t FinalChain::dposEligibleVoteCount(EthBlockNumber, addr_t const&) const {
  throw_unimplemented_final_chain_api("dposEligibleVoteCount");
}

bool FinalChain::dposIsEligible(EthBlockNumber, addr_t const&) const {
  throw_unimplemented_final_chain_api("dposIsEligible");
}

vrf_wrapper::vrf_pk_t FinalChain::dposGetVrfKey(EthBlockNumber, const addr_t&) const {
  throw_unimplemented_final_chain_api("dposGetVrfKey");
}

void FinalChain::prune(EthBlockNumber) { throw_unimplemented_final_chain_api("prune"); }

void FinalChain::waitForFinalized() { throw_unimplemented_final_chain_api("waitForFinalized"); }

std::vector<state_api::ValidatorStake> FinalChain::dposValidatorsTotalStakes(EthBlockNumber) const {
  throw_unimplemented_final_chain_api("dposValidatorsTotalStakes");
}

uint256_t FinalChain::dposTotalAmountDelegated(EthBlockNumber) const {
  throw_unimplemented_final_chain_api("dposTotalAmountDelegated");
}

std::vector<state_api::ValidatorVoteCount> FinalChain::dposValidatorsEligibleVoteCounts(EthBlockNumber) const {
  throw_unimplemented_final_chain_api("dposValidatorsEligibleVoteCounts");
}

uint64_t FinalChain::dposYield(EthBlockNumber) const { throw_unimplemented_final_chain_api("dposYield"); }

u256 FinalChain::dposTotalSupply(EthBlockNumber) const { throw_unimplemented_final_chain_api("dposTotalSupply"); }

h256 FinalChain::getBridgeRoot(EthBlockNumber) const { throw_unimplemented_final_chain_api("getBridgeRoot"); }

h256 FinalChain::getBridgeEpoch(EthBlockNumber) const { throw_unimplemented_final_chain_api("getBridgeEpoch"); }

std::pair<val_t, bool> FinalChain::getBalance(addr_t const&) const {
  throw_unimplemented_final_chain_api("getBalance");
}

std::shared_ptr<const FinalizationResult> FinalChain::finalize_(PeriodData&&, std::vector<h256>&&, uint32_t,
                                                                std::shared_ptr<DagBlock>&&) {
  throw_unimplemented_final_chain_api("finalize_");
}

SharedTransactionReceipts FinalChain::blockReceipts(std::optional<EthBlockNumber>) const {
  throw_unimplemented_final_chain_api("blockReceipts");
}

}  // namespace taraxa::final_chain
