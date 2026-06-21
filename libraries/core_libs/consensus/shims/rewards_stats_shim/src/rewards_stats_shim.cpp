#include "rewards/rewards_stats.hpp"

#include <algorithm>
#include <stdexcept>
#include <utility>

#include "common/encoding_rlp.hpp"
#include "transaction/transaction.hpp"

namespace taraxa::rewards {
namespace {

constexpr uint8_t kRewardsStatsApplied = 0;

template <size_t N, typename Hash>
std::array<uint8_t, N> toBridgeArray(const Hash& hash) {
  std::array<uint8_t, N> out{};
  std::copy_n(hash.asArray().begin(), N, out.begin());
  return out;
}

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (auto byte : bytes) {
    out.push_back(byte);
  }
  return out;
}

rust::Vec<uint8_t> gasPriceBytes(const u256& gas_price) { return toBridgeBytes(dev::toBigEndian(gas_price)); }

rust::Vec<rustaxa::RewardsFrequencyRule> makeFrequencyRules(const HardforksConfig& hardforks) {
  rust::Vec<rustaxa::RewardsFrequencyRule> rules;
  rules.reserve(hardforks.rewards_distribution_frequency.size());
  for (const auto& [from_period, frequency] : hardforks.rewards_distribution_frequency) {
    rustaxa::RewardsFrequencyRule rule{};
    rule.from_period = from_period;
    rule.frequency = frequency;
    rules.push_back(rule);
  }
  return rules;
}

rustaxa::RewardsStatsConfig makeRewardsConfig(uint32_t committee_size, const HardforksConfig& hardforks) {
  rustaxa::RewardsStatsConfig config{};
  config.committee_size = committee_size;
  config.magnolia_period = hardforks.magnolia_hf.block_num;
  config.aspen_part_one_period = hardforks.aspen_hf.block_num_part_one;
  return config;
}

dev::bytes toDevBytes(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

std::vector<uint8_t> toStdBytes(const rust::Vec<uint8_t>& bytes) { return std::vector<uint8_t>(bytes.begin(), bytes.end()); }

std::runtime_error rewardsStatsError(const std::string& message) {
  return std::runtime_error("rewards::Stats Rust shim: " + message);
}

}  // namespace

Stats::Stats(uint32_t committee_size, const HardforksConfig& hardforks, std::shared_ptr<DbStorage> db,
             std::function<uint64_t(EthBlockNumber)>&& dpos_eligible_total_vote_count, EthBlockNumber last_blk_num)
    : kCommitteeSize(committee_size),
      kHardforksConfig(hardforks),
      dpos_eligible_total_vote_count_(std::move(dpos_eligible_total_vote_count)),
      rust_stats_(rustaxa::create_rewards_stats_runtime(db->rustStorage(), makeRewardsConfig(kCommitteeSize, hardforks),
                                                        makeFrequencyRules(hardforks), last_blk_num)) {
  recoverFromDb(last_blk_num);
}

Stats::~Stats() = default;

void Stats::recoverFromDb(EthBlockNumber last_blk_num) {
  (void)last_blk_num;
  replaceCacheRlp(rust_stats_->rewards_stats_runtime_cached_stats());
}

std::vector<BlockStats> Stats::processStats(const PeriodData& current_blk, uint32_t blocks_per_year,
                                            const std::vector<gas_t>& trxs_gas_used, Batch& write_batch) {
  auto fact = makeProcessFact(current_blk, blocks_per_year, trxs_gas_used);
  auto plan = rust_stats_->process_finalized_period_rewards_stats(std::move(fact));
  if (plan.status != kRewardsStatsApplied) {
    throw rewardsStatsError("planner rejected period " + std::to_string(plan.current_period) + ": " +
                            std::string(plan.error_code));
  }

  if (plan.cache_current_period) {
    cacheStatsRlp(plan.current_period, plan.current_block_stats_rlp);
    appendStorageWrites(plan, write_batch);
  } else if (plan.clear_cached_stats) {
    replaceCacheRlp(plan.distribution_stats);
  }

  return decodeDistributionStats(plan.distribution_stats);
}

FinalChainPublicationRewardsStats Stats::processStatsForFinalChainPublication(
    const PeriodData& current_blk, uint32_t blocks_per_year, const std::vector<gas_t>& trxs_gas_used) {
  auto fact = makeProcessFact(current_blk, blocks_per_year, trxs_gas_used);
  auto plan = rust_stats_->process_finalized_period_rewards_stats(std::move(fact));
  if (plan.status != kRewardsStatsApplied) {
    throw rewardsStatsError("planner rejected period " + std::to_string(plan.current_period) + ": " +
                            std::string(plan.error_code));
  }

  if (plan.cache_current_period) {
    cacheStatsRlp(plan.current_period, plan.current_block_stats_rlp);
  } else if (plan.clear_cached_stats) {
    replaceCacheRlp(plan.distribution_stats);
  }

  FinalChainPublicationRewardsStats result;
  result.distribution_stats = decodeDistributionStats(plan.distribution_stats);
  result.storage_update.current_period = plan.current_period;
  result.storage_update.cache_current_period = plan.cache_current_period;
  result.storage_update.clear_cached_stats = plan.clear_cached_stats;
  if (plan.cache_current_period) {
    result.storage_update.current_block_stats_rlp = std::move(plan.current_block_stats_rlp);
  }
  return result;
}

void Stats::clear(uint64_t current_period) {
  const auto frequency = kHardforksConfig.getRewardsDistributionFrequency(current_period);
  if (frequency > 1 && current_period % frequency == 0) {
    auto result = rust_stats_->rewards_stats_runtime_clear_storage_and_state(current_period, false);
    if (result.status != kRewardsStatsApplied) {
      throw rewardsStatsError("storage clear rejected period " + std::to_string(result.current_period) + ": " +
                              std::string(result.error_code));
    }
    blocks_stats_rlp_.clear();
    blocks_stats_.clear();
    return;
  }
  rust_stats_->rewards_stats_runtime_clear_committed(current_period);
}

void Stats::clearCommittedAfterFinalChainPublication(uint64_t current_period) {
  const auto frequency = kHardforksConfig.getRewardsDistributionFrequency(current_period);
  if (frequency > 1 && current_period % frequency == 0) {
    blocks_stats_rlp_.clear();
    blocks_stats_.clear();
  }
  rust_stats_->rewards_stats_runtime_clear_committed(current_period);
}

rustaxa::RewardsStatsProcessFact Stats::makeProcessFact(const PeriodData& current_blk, uint32_t blocks_per_year,
                                                        const std::vector<gas_t>& trxs_gas_used) const {
  if (!current_blk.pbft_blk) {
    throw rewardsStatsError("cannot process rewards stats without a PBFT block");
  }
  if (!trxs_gas_used.empty() && trxs_gas_used.size() < current_blk.transactions.size()) {
    throw rewardsStatsError("gas-used vector is shorter than finalized transactions");
  }

  rustaxa::RewardsStatsProcessFact fact{};
  fact.period = current_blk.pbft_blk->getPeriod();
  fact.block_author = toBridgeArray<20>(current_blk.pbft_blk->getBeneficiary());
  fact.blocks_per_year = blocks_per_year;
  fact.dpos_eligible_total_vote_count = kCommitteeSize;

  if (!current_blk.previous_block_cert_votes.empty()) {
    const auto previous_vote_period = current_blk.previous_block_cert_votes.front()->getPeriod();
    fact.dpos_eligible_total_vote_count = dpos_eligible_total_vote_count_(previous_vote_period - 1);
  }

  fact.transactions.reserve(current_blk.transactions.size());
  for (size_t i = 0; i < current_blk.transactions.size(); ++i) {
    const auto& trx = current_blk.transactions[i];
    rustaxa::RewardsTransactionFact transaction_fact{};
    transaction_fact.hash = toBridgeArray<32>(trx->getHash());
    transaction_fact.gas_price_be = gasPriceBytes(trx->getGasPrice());
    transaction_fact.gas_used = trxs_gas_used.empty() ? 0 : trxs_gas_used[i];
    fact.transactions.push_back(std::move(transaction_fact));
  }

  fact.dag_blocks.reserve(current_blk.dag_blocks.size());
  for (const auto& dag_block : current_blk.dag_blocks) {
    rustaxa::RewardsDagBlockFact dag_fact{};
    dag_fact.author = toBridgeArray<20>(dag_block->getSender());
    dag_fact.difficulty = dag_block->getDifficulty();
    dag_fact.transaction_hashes.reserve(dag_block->getTrxs().size());
    for (const auto& trx_hash : dag_block->getTrxs()) {
      dag_fact.transaction_hashes.push_back(rustaxa::RewardsHash{toBridgeArray<32>(trx_hash)});
    }
    fact.dag_blocks.push_back(std::move(dag_fact));
  }

  fact.cert_votes.reserve(current_blk.previous_block_cert_votes.size());
  for (const auto& vote : current_blk.previous_block_cert_votes) {
    auto weight = vote->getWeight();
    if (!weight) {
      throw rewardsStatsError("cert vote is missing validator weight");
    }
    rustaxa::RewardsCertVoteFact vote_fact{};
    vote_fact.voter = toBridgeArray<20>(vote->getVoterAddr());
    vote_fact.weight = *weight;
    vote_fact.period = vote->getPeriod();
    fact.cert_votes.push_back(vote_fact);
  }

  return fact;
}

std::vector<BlockStats> Stats::decodeDistributionStats(const rust::Vec<rustaxa::PeriodRlp>& stats) const {
  std::vector<BlockStats> decoded;
  decoded.reserve(stats.size());
  for (const auto& stat : stats) {
    decoded.push_back(decodeBlockStats(stat.data));
  }
  return decoded;
}

BlockStats Stats::decodeBlockStats(const rust::Vec<uint8_t>& stats_rlp) const {
  auto bytes = toDevBytes(stats_rlp);
  return util::rlp_dec<BlockStats>(dev::RLP(bytes));
}

void Stats::cacheStatsRlp(PbftPeriod period, const rust::Vec<uint8_t>& stats_rlp) {
  blocks_stats_rlp_[period] = toStdBytes(stats_rlp);
  blocks_stats_[period] = decodeBlockStats(stats_rlp);
}

void Stats::replaceCacheRlp(const rust::Vec<rustaxa::PeriodRlp>& stats) {
  blocks_stats_rlp_.clear();
  blocks_stats_.clear();
  for (const auto& stat : stats) {
    cacheStatsRlp(stat.period, stat.data);
  }
}

void Stats::appendStorageWrites(const rustaxa::RewardsStatsProcessResult& plan, Batch& write_batch) const {
  (void)write_batch;
  auto result = rust_stats_->rewards_stats_runtime_apply_storage_writes(plan, false);
  if (result.status != kRewardsStatsApplied) {
    throw rewardsStatsError("storage appender rejected period " + std::to_string(result.current_period) + ": " +
                            std::string(result.error_code));
  }
}

}  // namespace taraxa::rewards
