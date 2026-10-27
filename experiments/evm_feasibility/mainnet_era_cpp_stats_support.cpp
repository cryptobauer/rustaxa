// Input and observation leaves for unchanged C++ reward aggregation methods.
#include <algorithm>
#include <array>
#include <bit>
#include <cstring>
#include <cassert>
#include <cstdint>
#include <functional>
#include <iostream>
#include <iterator>
#include <map>
#include <memory>
#include <optional>
#include <set>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <vector>
#include <boost/multiprecision/cpp_int.hpp>
#include <boost/version.hpp>
#include <limits>
using EthBlockNumber = uint64_t;
using PbftPeriod = uint64_t;
using gas_t = uint64_t;
using Batch = int;
using addr_t = std::string;
using trx_hash_t = std::string;
using u256 = boost::multiprecision::uint256_t;
// Observation metadata only. It is copied/moved with a payload, never read by
// aggregation or scheduling. No period vector is sorted by this support code.
// Address/hash strings affect validator-map serialization only. Actual period
// keys remain uint64_t with the compiler's real std::unordered_map policy.
uint64_t observation_input_period = 0;
struct Transaction {
  trx_hash_t hash; u256 gas_price;
  const trx_hash_t& getHash() const { return hash; }
  u256 getGasPrice() const { return gas_price; }
};
using SharedTransactions = std::vector<std::shared_ptr<Transaction>>;
struct Pbft {
  uint64_t period; addr_t beneficiary;
  uint64_t getPeriod() const { return period; }
  const addr_t& getBeneficiary() const { return beneficiary; }
};
struct PbftVote {
  uint64_t period, weight; addr_t voter;
  uint64_t getPeriod() const { return period; }
  std::optional<uint64_t> getWeight() const { return weight; }
  const addr_t& getVoterAddr() const { return voter; }
};
struct Dag {
  addr_t author; uint16_t difficulty; std::vector<trx_hash_t> hashes;
  const addr_t& getSender() const { return author; }
  uint16_t getDifficulty() const { return difficulty; }
  const std::vector<trx_hash_t>& getTrxs() const { return hashes; }
};
struct PeriodData {
  std::shared_ptr<Pbft> pbft_blk;
  SharedTransactions transactions;
  std::vector<std::shared_ptr<Dag>> dag_blocks;
  std::vector<std::shared_ptr<PbftVote>> previous_block_cert_votes;
};
struct HardforksConfig {
  struct { uint64_t block_num; } magnolia_hf;
  struct { uint64_t block_num_part_one; } aspen_hf;
  std::map<uint64_t,uint32_t> rewards_distribution_frequency;
  uint32_t getRewardsDistributionFrequency(uint64_t block) const;
  // @ASPEN_METHOD@
};
// @FREQUENCY_METHOD@
#define HAS_RLP_FIELDS
#define RLP_FIELDS_DEFINE(...)
#define private public
#define protected public
// @BLOCK_HEADER@
namespace taraxa::rewards {
struct DbStorage {
  enum class Columns { block_rewards_stats };
  // storage.cpp persists raw native uint64 keys and reloads in RocksDB's
  // bytewise iterator order, inserting each decoded payload into a fresh map.
  // Retain this input-leaf order; copying an existing unordered_map would
  // preserve a different bucket history across legacy recovery.
  using Key = std::array<unsigned char,sizeof(PbftPeriod)>;
  std::map<Key,std::pair<PbftPeriod,BlockStats>> rows;
  auto getBlocksRewardsStats() {
    std::unordered_map<PbftPeriod,BlockStats> result;
    for (const auto& [key,row] : rows) result[row.first] = row.second;
    return result;
  }
  void deleteColumnData(Columns) { rows.clear(); }
  void saveBlockRewardsStats(uint64_t p, const BlockStats& b, Batch&) {
    Key key; std::memcpy(key.data(),&p,sizeof(p)); rows[key] = {p,b};
  }
};
}
// @STATS_HEADER@
#undef private
#undef protected
// @BLOCK_METHODS@
// @STATS_METHODS@
using namespace taraxa::rewards;
void emit(const BlockStats& b) {
  std::cout << "{\"number\":" << b.observation_period_ << ",\"block_author\":\"" << b.block_author_
    << "\",\"blocks_per_year\":" << b.blocks_per_year_ << ",\"max_votes_weight\":" << b.max_votes_weight_
    << ",\"total_dag_blocks_count\":" << b.total_dag_blocks_count_
    << ",\"total_votes_weight\":" << b.total_votes_weight_ << ",\"validators\":[";
  bool first = true;
  for (const auto& [address, v] : b.validators_stats_) {
    if (!first) std::cout << ',';
    first = false;
    std::cout << "{\"validator\":\"" << address << "\",\"dag_blocks_count\":" << v.dag_blocks_count_
      << ",\"vote_weight\":" << v.vote_weight_ << ",\"fees_reward\":\"" << v.fees_rewards_ << "\"}";
  }
  std::cout << "]}";
}
int main() {
  static_assert(sizeof(PbftPeriod)==8 && std::numeric_limits<PbftPeriod>::digits==64);
  std::cerr << "period_key_endian=" << (std::endian::native==std::endian::little ? "little" : "big") << " ";
  std::cerr << "compiler=" << __VERSION__ << " cplusplus=" << __cplusplus;
#ifdef __GLIBCXX__
  std::cerr << " libstdcxx=" << __GLIBCXX__;
#endif
#ifdef _GLIBCXX_RELEASE
  std::cerr << " libstdcxx_release=" << _GLIBCXX_RELEASE;
#endif
  std::cerr << " boost=" << BOOST_LIB_VERSION << " uint256_bits=" << std::numeric_limits<u256>::digits << '\n';
  // @POLICY@
  size_t runs; if (!(std::cin >> runs)) return 2;
  for (size_t run = 0; run < runs; ++run) {
    uint64_t reopen; size_t count; std::cin >> reopen >> count;
    auto db = std::make_shared<DbStorage>();
    uint64_t eligible = 0, expected_vote_head = 0;
    auto make = [&](uint64_t head) {
      return std::make_unique<Stats>(committee, hf, db, [&](uint64_t requested) {
        if (requested != expected_vote_head) throw std::runtime_error("wrong eligible-vote query identity");
        return eligible;
      }, head);
    };
    auto stats = make(0); Batch batch = 0;
    for (size_t row = 0; row < count; ++row) {
      uint64_t p; uint32_t rate; std::string author;
      size_t ntx, ndag, nvotes;
      std::cin >> p >> rate >> eligible >> author >> ntx >> ndag >> nvotes;
      observation_input_period = p;
      PeriodData b; b.pbft_blk = std::make_shared<Pbft>(Pbft{p,author});
      std::vector<gas_t> gas;
      for (size_t i=0; i<ntx; ++i) {
        std::string hash,price; uint64_t used; std::cin >> hash >> price >> used;
        b.transactions.push_back(std::make_shared<Transaction>(Transaction{hash,u256(price)})); gas.push_back(used);
      }
      for (size_t i=0; i<ndag; ++i) {
        std::string address; unsigned difficulty; size_t n; std::cin >> address >> difficulty >> n;
        auto d=std::make_shared<Dag>(); d->author=address; d->difficulty=difficulty;
        for(size_t j=0;j<n;++j) { std::string hash; std::cin >> hash; d->hashes.push_back(hash); }
        b.dag_blocks.push_back(d);
      }
      for (size_t i=0; i<nvotes; ++i) {
        uint64_t period,weight; std::string address; std::cin >> period >> weight >> address;
        b.previous_block_cert_votes.push_back(std::make_shared<PbftVote>(PbftVote{period,weight,address}));
      }
      if (!std::cin) throw std::runtime_error("incomplete input");
      expected_vote_head = b.previous_block_cert_votes.empty() ? 0 : b.previous_block_cert_votes[0]->getPeriod()-1;
      auto current=stats->getBlockStats(b,rate,gas);
      auto distributed=stats->processStats(b,rate,gas,batch);
      std::cout << "{\"run\":" << run << ",\"current\":"; emit(current);
      std::cout << ",\"scheduled\":[";
      bool first=true; for (const auto& value:distributed) { if(!first) std::cout << ','; first=false; emit(value); }
      std::cout << "]}\n";
      stats->clear(p);
      if (p==reopen) stats=make(p);
    }
  }
  std::string extra; if (std::cin >> extra) return 3;
}
