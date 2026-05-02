#include <array>
#include <cstring>
#include <string>

#include "final_chain/final_chain.hpp"

namespace taraxa::final_chain {
namespace {

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

std::string into_string(const rust::Vec<uint8_t>& bytes) {
  return std::string(reinterpret_cast<const char*>(bytes.data()), bytes.size());
}

std::future<std::shared_ptr<const FinalizationResult>> ready_null_finalization_result() {
  std::promise<std::shared_ptr<const FinalizationResult>> promise;
  promise.set_value(nullptr);
  return promise.get_future();
}

}  // namespace

FinalChain::FinalChain(const std::shared_ptr<DbStorage>& db, const taraxa::FullNodeConfig& config,
                       const addr_t& node_addr)
    : FinalChainOld(db, config, node_addr) {
  delegation_delay_ = config.genesis.state.dpos.delegation_delay;
  rust_final_chain_ = rustaxa::create_final_chain(db->rustStorage(), config.genesis.pbft.gas_limit,
                                                  config.genesis.dag_genesis_block.getTimestamp());
}

void FinalChain::stop() {}

EthBlockNumber FinalChain::delegationDelay() const { return delegation_delay_; }

std::future<std::shared_ptr<const FinalizationResult>> FinalChain::finalize(PeriodData&&, std::vector<h256>&&, uint32_t,
                                                                            std::shared_ptr<DagBlock>&&) {
  return ready_null_finalization_result();
}

std::shared_ptr<const BlockHeader> FinalChain::blockHeader(std::optional<EthBlockNumber> n) const {
  auto const block_number = n.value_or(lastBlockNumber());
  auto rust_header = rust_final_chain_.value()->get_block_header(static_cast<uint64_t>(block_number));
  if (rust_header.empty()) {
    return nullptr;
  }

  auto header_data = into_string(rust_header);
  return BlockHeader::fromRLP(dev::RLP(header_data));
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

std::optional<h256> FinalChain::finalChainHash(EthBlockNumber n) const {
  auto delay = delegationDelay();
  if (n <= delay) {
    return ZeroHash();
  }
  auto header = blockHeader(n - delay);
  if (!header) {
    return std::nullopt;
  }
  return header->hash;
}

void FinalChain::updateStateConfig(const state_api::Config& new_config) {
  delegation_delay_ = new_config.dpos.delegation_delay;
}

std::shared_ptr<const TransactionHashes> FinalChain::transactionHashes(std::optional<EthBlockNumber>) const {
  return std::make_shared<TransactionHashes>();
}

const SharedTransactions FinalChain::transactions(std::optional<EthBlockNumber>) const {
  return {};
}

std::optional<TransactionLocation> FinalChain::transactionLocation(h256 const& trx_hash) const {
  auto rust_location = rust_final_chain_.value()->get_transaction_location(into_bytes_array(trx_hash));
  if (rust_location.empty()) {
    return std::nullopt;
  }
  auto location_data = into_string(rust_location);
  return TransactionLocation::fromRlp(dev::RLP(location_data));
}

std::optional<TransactionReceipt> FinalChain::transactionReceipt(EthBlockNumber, uint64_t,
                                                                 std::optional<trx_hash_t>) const {
  return std::nullopt;
}

std::shared_ptr<Transaction> FinalChain::transaction(EthBlockNumber, uint32_t) const {
  return nullptr;
}

uint64_t FinalChain::transactionCount(std::optional<EthBlockNumber> n) const {
  return rust_final_chain_.value()->get_transaction_count(static_cast<uint64_t>(n.value_or(lastBlockNumber())));
}

std::vector<EthBlockNumber> FinalChain::withBlockBloom(LogBloom const&, EthBlockNumber, EthBlockNumber) const {
  return {};
}

std::optional<state_api::Account> FinalChain::getAccount(addr_t const&, std::optional<EthBlockNumber>) const {
  return state_api::ZeroAccount;
}

h256 FinalChain::getAccountStorage(addr_t const&, u256 const&, std::optional<EthBlockNumber>) const {
  return {};
}

bytes FinalChain::getCode(addr_t const&, std::optional<EthBlockNumber>) const {
  return {};
}

state_api::ExecutionResult FinalChain::call(state_api::EVMTransaction const&, std::optional<EthBlockNumber>) const {
  return {};
}

std::string FinalChain::trace(std::vector<state_api::EVMTransaction>, std::vector<state_api::EVMTransaction>,
                              EthBlockNumber, std::optional<state_api::Tracing>) const {
  return {};
}

uint64_t FinalChain::dposEligibleTotalVoteCount(EthBlockNumber) const {
  return 1;
}

uint64_t FinalChain::dposEligibleVoteCount(EthBlockNumber, addr_t const&) const {
  return 1;
}

bool FinalChain::dposIsEligible(EthBlockNumber, addr_t const&) const {
  return true;
}

vrf_wrapper::vrf_pk_t FinalChain::dposGetVrfKey(EthBlockNumber, const addr_t&) const {
  return {};
}

void FinalChain::prune(EthBlockNumber) {}

void FinalChain::waitForFinalized() {}

std::vector<state_api::ValidatorStake> FinalChain::dposValidatorsTotalStakes(EthBlockNumber) const {
  return {};
}

uint256_t FinalChain::dposTotalAmountDelegated(EthBlockNumber) const {
  return {};
}

std::vector<state_api::ValidatorVoteCount> FinalChain::dposValidatorsEligibleVoteCounts(EthBlockNumber) const {
  return {};
}

uint64_t FinalChain::dposYield(EthBlockNumber) const { return 0; }

u256 FinalChain::dposTotalSupply(EthBlockNumber) const { return {}; }

h256 FinalChain::getBridgeRoot(EthBlockNumber) const { return {}; }

h256 FinalChain::getBridgeEpoch(EthBlockNumber) const { return {}; }

std::pair<val_t, bool> FinalChain::getBalance(addr_t const&) const {
  return {0, true};
}

std::shared_ptr<const FinalizationResult> FinalChain::finalize_(PeriodData&&, std::vector<h256>&&, uint32_t,
                                                                std::shared_ptr<DagBlock>&&) {
  return nullptr;
}

SharedTransactionReceipts FinalChain::blockReceipts(std::optional<EthBlockNumber>) const {
  return std::make_shared<std::vector<TransactionReceipt>>();
}

}  // namespace taraxa::final_chain
