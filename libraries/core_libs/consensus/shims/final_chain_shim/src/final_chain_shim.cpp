#include <array>
#include <cstring>
#include <stdexcept>
#include <string>

#include "final_chain/final_chain.hpp"
#include "libdevcore/CommonData.h"

namespace taraxa::final_chain {
namespace {

std::array<uint8_t, 32> into_bytes_array(const h256& hash) {
  std::array<uint8_t, 32> bytes{};
  std::memcpy(bytes.data(), hash.data(), bytes.size());
  return bytes;
}

std::array<uint8_t, 20> into_address_array(const addr_t& address) {
  std::array<uint8_t, 20> bytes{};
  std::memcpy(bytes.data(), address.data(), bytes.size());
  return bytes;
}

addr_t into_address(const std::array<uint8_t, 20>& address) {
  return addr_t(dev::bytes(address.begin(), address.end()));
}

rust::Vec<uint8_t> into_rust_vec(const dev::bytes& bytes) {
  rust::Vec<uint8_t> vec;
  vec.reserve(bytes.size());
  for (auto const byte : bytes) {
    vec.push_back(static_cast<uint8_t>(byte));
  }
  return vec;
}

rust::Vec<uint8_t> into_big_endian_vec(const u256& value) {
  rust::Vec<uint8_t> vec;
  auto bytes = dev::toBigEndian(value);
  vec.reserve(bytes.size());
  for (auto const byte : bytes) {
    vec.push_back(static_cast<uint8_t>(byte));
  }
  return vec;
}

rust::Vec<rustaxa::FinalizationTransaction> make_finalization_transactions(const SharedTransactions& transactions) {
  rust::Vec<rustaxa::FinalizationTransaction> rust_transactions;
  rust_transactions.reserve(transactions.size());
  for (const auto& transaction : transactions) {
    rustaxa::FinalizationTransaction rust_transaction;
    rust_transaction.hash = into_bytes_array(transaction->getHash());
    rust_transaction.sender = into_address_array(transaction->getSender());
    if (auto const& receiver = transaction->getReceiver(); receiver) {
      rust_transaction.receiver_found = true;
      rust_transaction.receiver = into_address_array(*receiver);
    } else {
      rust_transaction.receiver_found = false;
      rust_transaction.receiver = {};
    }
    rust_transaction.nonce = transaction->getNonce().convert_to<uint64_t>();
    rust_transaction.value = into_big_endian_vec(transaction->getValue());
    rust_transaction.gas_price = into_big_endian_vec(transaction->getGasPrice());
    rust_transaction.gas_limit = transaction->getGas();
    rust_transaction.data = into_rust_vec(transaction->getData());
    rust_transaction.rlp = into_rust_vec(transaction->rlp());
    rust_transactions.push_back(std::move(rust_transaction));
  }
  return rust_transactions;
}

rust::Vec<rustaxa::FinalizationDagBlock> make_finalization_dag_blocks(
    const std::vector<std::shared_ptr<DagBlock>>& dag_blocks) {
  rust::Vec<rustaxa::FinalizationDagBlock> rust_dag_blocks;
  rust_dag_blocks.reserve(dag_blocks.size());
  for (const auto& dag_block : dag_blocks) {
    rustaxa::FinalizationDagBlock rust_dag_block;
    rust_dag_block.author = into_address_array(dag_block->getSender());
    rust_dag_block.difficulty = dag_block->getDifficulty();
    rust_dag_block.transaction_hashes.reserve(dag_block->getTrxs().size());
    for (const auto& transaction_hash : dag_block->getTrxs()) {
      rustaxa::DagHash rust_transaction_hash;
      rust_transaction_hash.hash = into_bytes_array(transaction_hash);
      rust_dag_block.transaction_hashes.push_back(std::move(rust_transaction_hash));
    }
    rust_dag_blocks.push_back(std::move(rust_dag_block));
  }
  return rust_dag_blocks;
}

rust::Vec<rustaxa::RewardsCertVoteFact> make_rewards_cert_votes(
    const std::vector<std::shared_ptr<PbftVote>>& cert_votes) {
  rust::Vec<rustaxa::RewardsCertVoteFact> rust_votes;
  rust_votes.reserve(cert_votes.size());
  for (const auto& vote : cert_votes) {
    auto weight = vote->getWeight();
    if (!weight) {
      throw std::runtime_error("FinalChain::finalize cert vote is missing validator weight");
    }
    rustaxa::RewardsCertVoteFact fact{};
    fact.voter = into_address_array(vote->getVoterAddr());
    fact.weight = *weight;
    fact.period = vote->getPeriod();
    rust_votes.push_back(fact);
  }
  return rust_votes;
}

rust::Vec<rustaxa::GenesisAccount> make_genesis_accounts(const state_api::Config& config) {
  auto effective_balances = config.initial_balances;
  for (const auto& validator : config.dpos.initial_validators) {
    for (const auto& [delegator, amount] : validator.delegations) {
      effective_balances[delegator] -= amount;
    }
  }

  rust::Vec<rustaxa::GenesisAccount> accounts;
  accounts.reserve(effective_balances.size());
  for (const auto& [address, balance] : effective_balances) {
    rustaxa::GenesisAccount account;
    account.address = into_address_array(address);
    auto balance_bytes = dev::toBigEndian(balance);
    account.balance.reserve(balance_bytes.size());
    for (auto const byte : balance_bytes) {
      account.balance.push_back(static_cast<uint8_t>(byte));
    }
    accounts.push_back(std::move(account));
  }
  return accounts;
}

rust::Vec<rustaxa::GenesisValidator> make_genesis_validators(const state_api::Config& config) {
  rust::Vec<rustaxa::GenesisValidator> validators;
  validators.reserve(config.dpos.initial_validators.size());
  for (const auto& validator : config.dpos.initial_validators) {
    rustaxa::GenesisValidator genesis_validator;
    genesis_validator.address = into_address_array(validator.address);
    genesis_validator.owner = into_address_array(validator.owner);
    genesis_validator.vrf_key = into_bytes_array(validator.vrf_key);
    genesis_validator.commission = validator.commission;
    genesis_validator.description = rust::String(validator.description);
    genesis_validator.endpoint = rust::String(validator.endpoint);
    u256 total_stake = 0;
    for (const auto& [_, amount] : validator.delegations) {
      total_stake += amount;
    }
    genesis_validator.total_stake = into_big_endian_vec(total_stake);
    genesis_validator.delegations.reserve(validator.delegations.size());
    for (const auto& [delegator, amount] : validator.delegations) {
      rustaxa::GenesisDelegation genesis_delegation;
      genesis_delegation.delegator = into_address_array(delegator);
      genesis_delegation.stake = into_big_endian_vec(amount);
      genesis_validator.delegations.push_back(std::move(genesis_delegation));
    }
    validators.push_back(std::move(genesis_validator));
  }
  return validators;
}

rustaxa::GenesisDposConfig make_genesis_dpos_config(const state_api::DPOSConfig& config,
                                                    uint64_t dag_vdf_sortition_total_vote_count_until_period) {
  rustaxa::GenesisDposConfig dpos_config;
  dpos_config.eligibility_balance_threshold = into_big_endian_vec(config.eligibility_balance_threshold);
  dpos_config.vote_eligibility_balance_step = into_big_endian_vec(config.vote_eligibility_balance_step);
  dpos_config.validator_maximum_stake = into_big_endian_vec(config.validator_maximum_stake);
  dpos_config.minimum_deposit = into_big_endian_vec(config.minimum_deposit);
  dpos_config.delegation_delay = config.delegation_delay;
  dpos_config.dag_vdf_sortition_total_vote_count_until_period = dag_vdf_sortition_total_vote_count_until_period;
  return dpos_config;
}

rustaxa::FinalChainRewardsConfig make_final_chain_rewards_config(const taraxa::FullNodeConfig& config) {
  rustaxa::FinalChainRewardsConfig rewards_config;
  rewards_config.committee_size = config.genesis.pbft.committee_size;
  rewards_config.magnolia_period = config.genesis.state.hardforks.magnolia_hf.block_num;
  rewards_config.aspen_part_one_period = config.genesis.state.hardforks.aspen_hf.block_num_part_one;
  rewards_config.aspen_part_two_period = config.genesis.state.hardforks.aspen_hf.block_num_part_two;
  rewards_config.max_block_author_reward_percent = config.genesis.state.dpos.max_block_author_reward;
  rewards_config.dag_proposers_reward_percent = config.genesis.state.dpos.dag_proposers_reward;
  rewards_config.yield_percentage = config.genesis.state.dpos.yield_percentage;
  rewards_config.dpos_blocks_per_year = config.genesis.state.dpos.blocks_per_year;
  rewards_config.aspen_max_supply = into_big_endian_vec(config.genesis.state.hardforks.aspen_hf.max_supply);
  rewards_config.aspen_generated_rewards =
      into_big_endian_vec(config.genesis.state.hardforks.aspen_hf.generated_rewards);
  rewards_config.frequency_rules.reserve(config.genesis.state.hardforks.rewards_distribution_frequency.size());
  for (const auto& [from_period, frequency] : config.genesis.state.hardforks.rewards_distribution_frequency) {
    rustaxa::RewardsFrequencyRule rule{};
    rule.from_period = from_period;
    rule.frequency = frequency;
    rewards_config.frequency_rules.push_back(rule);
  }
  return rewards_config;
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

dev::bytes into_bytes(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

[[noreturn]] void throw_unimplemented_final_chain_api(const char* api_name) {
  throw DbException("FinalChain::" + std::string(api_name) + " is not implemented in Rust shim mode");
}

std::future<std::shared_ptr<const FinalizationResult>> ready_finalization_result(
    std::shared_ptr<const FinalizationResult> result) {
  std::promise<std::shared_ptr<const FinalizationResult>> promise;
  promise.set_value(std::move(result));
  return promise.get_future();
}

}  // namespace

FinalChain::FinalChain(const std::shared_ptr<DbStorage>& db, const taraxa::FullNodeConfig& config,
                       const addr_t& node_addr) {
  (void)node_addr;
  delegation_delay_ = config.genesis.state.dpos.delegation_delay;
  rust_final_chain_ = rustaxa::create_final_chain_with_rewards_config(
      db->rustStorage(), config.genesis.pbft.gas_limit, config.genesis.dag_genesis_block.getTimestamp(),
      make_genesis_accounts(config.genesis.state), make_genesis_validators(config.genesis.state),
      make_genesis_dpos_config(config.genesis.state.dpos, config.genesis.state.hardforks.magnolia_hf.block_num),
      make_final_chain_rewards_config(config));
}

void FinalChain::stop() {}

EthBlockNumber FinalChain::delegationDelay() const { return delegation_delay_; }

std::future<std::shared_ptr<const FinalizationResult>> FinalChain::finalize(
    PeriodData&& period_data, std::vector<h256>&& finalized_dag_blk_hashes, uint32_t blocks_per_year,
    std::shared_ptr<DagBlock>&& anchor) {
  (void)anchor;
  auto outcome =
      rust_final_chain_.value()->finalize_block_with_rewards_facts(
          into_rust_vec(period_data.pbft_blk->rlp(true)), make_finalization_transactions(period_data.transactions),
          make_finalization_dag_blocks(period_data.dag_blocks), blocks_per_year,
          make_rewards_cert_votes(period_data.previous_block_cert_votes));
  auto header_data = into_string(outcome.block_header_rlp);
  auto header = BlockHeader::fromRLP(dev::RLP(header_data));
  TransactionReceipts receipts;
  receipts.reserve(outcome.receipts.size());
  for (auto const& receipt : outcome.receipts) {
    auto receipt_data = into_string(receipt.data);
    receipts.push_back(util::rlp_dec<TransactionReceipt>(dev::RLP(receipt_data)));
  }
  auto result = std::make_shared<FinalizationResult>(FinalizationResult{
      {
          period_data.pbft_blk->getBeneficiary(),
          period_data.pbft_blk->getTimestamp(),
          std::move(finalized_dag_blk_hashes),
          period_data.pbft_blk->getBlockHash(),
      },
      std::move(header),
      std::move(period_data.transactions),
      std::move(receipts),
  });
  block_finalized_emitter_.emit(result);
  return ready_finalization_result(std::move(result));
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

std::shared_ptr<const TransactionHashes> FinalChain::transactionHashes(std::optional<EthBlockNumber> n) const {
  auto ret = std::make_shared<TransactionHashes>();
  for (auto const& transaction : transactions(n)) {
    ret->push_back(transaction->getHash());
  }
  return ret;
}

const SharedTransactions FinalChain::transactions(std::optional<EthBlockNumber> n) const {
  SharedTransactions ret;
  auto const block_number = n.value_or(lastBlockNumber());
  auto rust_transactions = rust_final_chain_.value()->get_transaction_rlps(block_number);
  ret.reserve(rust_transactions.size());
  for (auto const& transaction : rust_transactions) {
    ret.push_back(std::make_shared<Transaction>(into_bytes(transaction.data), false));
  }
  return ret;
}

std::optional<TransactionLocation> FinalChain::transactionLocation(h256 const& trx_hash) const {
  auto rust_location = rust_final_chain_.value()->get_transaction_location(into_bytes_array(trx_hash));
  if (rust_location.empty()) {
    return std::nullopt;
  }
  auto location_data = into_string(rust_location);
  return TransactionLocation::fromRlp(dev::RLP(location_data));
}

std::optional<TransactionReceipt> FinalChain::transactionReceipt(EthBlockNumber blk_n, uint64_t position,
                                                                 std::optional<trx_hash_t>) const {
  auto receipt = rust_final_chain_.value()->get_transaction_receipt(blk_n, position);
  if (receipt.empty()) {
    return std::nullopt;
  }
  auto receipt_data = into_string(receipt);
  return util::rlp_dec<TransactionReceipt>(dev::RLP(receipt_data));
}

std::shared_ptr<Transaction> FinalChain::transaction(EthBlockNumber blk_n, uint32_t position) const {
  auto block_transactions = transactions(blk_n);
  if (position >= block_transactions.size()) {
    return nullptr;
  }
  return block_transactions[position];
}

uint64_t FinalChain::transactionCount(std::optional<EthBlockNumber> n) const {
  return rust_final_chain_.value()->get_transaction_count(static_cast<uint64_t>(n.value_or(lastBlockNumber())));
}

std::vector<EthBlockNumber> FinalChain::withBlockBloom(LogBloom const&, EthBlockNumber, EthBlockNumber) const {
  return {};
}

std::optional<state_api::Account> FinalChain::getAccount(addr_t const& addr, std::optional<EthBlockNumber> blk_n) const {
  auto rust_account = blk_n.has_value()
                          ? rust_final_chain_.value()->get_account_at_block(*blk_n, into_address_array(addr))
                          : rust_final_chain_.value()->get_account(into_address_array(addr));
  if (!rust_account.found) {
    return std::nullopt;
  }

  state_api::Account account;
  account.nonce = rust_account.nonce;
  account.balance = dev::fromBigEndian<u256>(dev::bytes(rust_account.balance.begin(), rust_account.balance.end()));
  account.storage_root_hash =
      h256(dev::bytes(rust_account.storage_root_hash.begin(), rust_account.storage_root_hash.end()));
  account.code_hash = h256(dev::bytes(rust_account.code_hash.begin(), rust_account.code_hash.end()));
  account.code_size = rust_account.code_size;
  return account;
}

const rustaxa::BridgeFinalChain& FinalChain::rustFinalChainForRust() const { return *rust_final_chain_.value(); }

h256 FinalChain::getAccountStorage(addr_t const&, u256 const&, std::optional<EthBlockNumber>) const { return {}; }

bytes FinalChain::getCode(addr_t const&, std::optional<EthBlockNumber>) const { return {}; }

state_api::ExecutionResult FinalChain::call(state_api::EVMTransaction const& trx,
                                            std::optional<EthBlockNumber> blk_n) const {
  rustaxa::FinalChainCall request;
  request.block_number = blk_n.value_or(lastBlockNumber());
  request.sender = into_address_array(trx.from);
  if (trx.to) {
    request.receiver_found = true;
    request.receiver = into_address_array(*trx.to);
  } else {
    request.receiver_found = false;
    request.receiver = {};
  }
  request.value = into_big_endian_vec(trx.value);
  request.gas_price = into_big_endian_vec(trx.gas_price);
  request.gas_limit = trx.gas;
  request.input = into_rust_vec(trx.input);
  auto outcome = rust_final_chain_.value()->call(std::move(request));

  state_api::ExecutionResult result;
  result.code_retval = into_bytes(outcome.code_retval);
  result.gas_used = outcome.gas_used;
  result.code_err = std::string(outcome.code_err);
  result.consensus_err = std::string(outcome.consensus_err);
  return result;
}

std::string FinalChain::trace(std::vector<state_api::EVMTransaction>, std::vector<state_api::EVMTransaction>,
                              EthBlockNumber, std::optional<state_api::Tracing>) const {
  return {};
}

uint64_t FinalChain::dposEligibleTotalVoteCount(EthBlockNumber blk_num) const {
  return rust_final_chain_.value()->get_dpos_eligible_total_vote_count(blk_num);
}

rustaxa::DagDposAuthorizationFacts FinalChain::dagDposAuthorizationFacts(EthBlockNumber blk_num,
                                                                         const addr_t& addr) const {
  return rust_final_chain_.value()->get_dag_dpos_authorization_facts(blk_num, into_address_array(addr));
}

uint64_t FinalChain::dposEligibleVoteCount(EthBlockNumber blk_num, addr_t const& addr) const {
  return rust_final_chain_.value()->get_dpos_eligible_vote_count(blk_num, into_address_array(addr));
}

bool FinalChain::dposIsEligible(EthBlockNumber blk_num, addr_t const& addr) const {
  return rust_final_chain_.value()->get_dpos_is_eligible(blk_num, into_address_array(addr));
}

vrf_wrapper::vrf_pk_t FinalChain::dposGetVrfKey(EthBlockNumber blk_n, const addr_t& addr) const {
  auto rust_key =
      rust_final_chain_.value()->get_vrf_key_at_block(static_cast<uint64_t>(blk_n), into_address_array(addr));
  if (rust_key.empty()) {
    return {};
  }
  return vrf_wrapper::vrf_pk_t(dev::bytes(rust_key.begin(), rust_key.end()));
}

void FinalChain::prune(EthBlockNumber) {}

void FinalChain::waitForFinalized() {}

std::vector<state_api::ValidatorStake> FinalChain::dposValidatorsTotalStakes(EthBlockNumber blk_num) const {
  auto rust_stakes = rust_final_chain_.value()->get_dpos_validators_total_stakes(blk_num);
  std::vector<state_api::ValidatorStake> stakes;
  stakes.reserve(rust_stakes.size());
  for (const auto& rust_stake : rust_stakes) {
    stakes.push_back(state_api::ValidatorStake{
        into_address(rust_stake.address),
        dev::fromBigEndian<u256>(dev::bytes(rust_stake.stake.begin(), rust_stake.stake.end())),
    });
  }
  return stakes;
}

uint256_t FinalChain::dposTotalAmountDelegated(EthBlockNumber) const { return {}; }

std::vector<state_api::ValidatorVoteCount> FinalChain::dposValidatorsEligibleVoteCounts(EthBlockNumber blk_num) const {
  auto rust_vote_counts = rust_final_chain_.value()->get_dpos_validators_eligible_vote_counts(blk_num);
  std::vector<state_api::ValidatorVoteCount> vote_counts;
  vote_counts.reserve(rust_vote_counts.size());
  for (const auto& rust_vote_count : rust_vote_counts) {
    vote_counts.push_back(state_api::ValidatorVoteCount{
        into_address(rust_vote_count.address),
        rust_vote_count.vote_count,
    });
  }
  return vote_counts;
}

uint64_t FinalChain::dposYield(EthBlockNumber) const { return 0; }

u256 FinalChain::dposTotalSupply(EthBlockNumber) const { return {}; }

h256 FinalChain::getBridgeRoot(EthBlockNumber) const { return {}; }

h256 FinalChain::getBridgeEpoch(EthBlockNumber) const { return {}; }

std::pair<val_t, bool> FinalChain::getBalance(addr_t const& addr) const {
  if (auto account = getAccount(addr)) {
    return {account->balance, true};
  }
  return {0, false};
}

std::shared_ptr<const FinalizationResult> FinalChain::finalize_(PeriodData&&, std::vector<h256>&&, uint32_t,
                                                                std::shared_ptr<DagBlock>&&) {
  throw_unimplemented_final_chain_api("finalize_");
}

SharedTransactionReceipts FinalChain::blockReceipts(std::optional<EthBlockNumber> n) const {
  auto const block_number = n.value_or(lastBlockNumber());
  auto count = transactionCount(block_number);
  auto receipts = std::make_shared<std::vector<TransactionReceipt>>();
  receipts->reserve(count);
  for (uint64_t position = 0; position < count; ++position) {
    if (auto receipt = transactionReceipt(block_number, position)) {
      receipts->push_back(*receipt);
    }
  }
  return receipts;
}

}  // namespace taraxa::final_chain
