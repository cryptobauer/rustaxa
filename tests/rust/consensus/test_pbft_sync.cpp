#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <vector>

#include "rustaxa-bridge/ffi.rs.h"

using namespace rustaxa;

namespace {

std::array<uint8_t, 32> h256(uint8_t last_byte) {
  std::array<uint8_t, 32> hash{};
  hash[31] = last_byte;
  return hash;
}

PbftSyncTransactionHash tx(uint8_t last_byte) { return PbftSyncTransactionHash{h256(last_byte)}; }

std::vector<std::array<uint8_t, 32>> hashes(const rust::Vec<PbftSyncTransactionHash>& input) {
  std::vector<std::array<uint8_t, 32>> out;
  out.reserve(input.size());
  for (const auto& hash : input) {
    out.push_back(hash.hash);
  }
  return out;
}

}  // namespace

TEST(RustPbftSyncTest, TransactionQueryPlansUniqueMissingDagTransactionsInOrder) {
  PbftSyncTransactionQueryFact fact;
  fact.dag_transaction_hashes.push_back(tx(1));
  fact.dag_transaction_hashes.push_back(tx(2));
  fact.dag_transaction_hashes.push_back(tx(1));
  fact.dag_transaction_hashes.push_back(tx(3));
  fact.dag_transaction_hashes.push_back(tx(4));
  fact.period_data_transaction_hashes.push_back(tx(2));
  fact.period_data_transaction_hashes.push_back(tx(4));

  const auto plan = plan_pbft_sync_transaction_query(std::move(fact));

  EXPECT_EQ(hashes(plan.finalized_lookup_hashes), (std::vector{h256(1), h256(3)}));
}
