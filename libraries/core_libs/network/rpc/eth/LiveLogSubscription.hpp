#pragma once

#include <algorithm>
#include <cstdint>
#include <functional>
#include <vector>

#include "LogFilter.hpp"

namespace taraxa::net::rpc::eth {

// LiveLogBlock is the minimal block-execution event shape needed by ETH log
// subscriptions. It carries the finalized block identity, bloom, transaction
// hashes, and receipts already produced by the external execution boundary; it
// does not expose FinalChain or storage objects to websocket subscription code.
struct LiveLogBlock {
  EthBlockNumber block_number = 0;
  blk_hash_t block_hash;
  LogBloom log_bloom;
  TransactionHashes transaction_hashes;
  TransactionReceipts transaction_receipts;
};

// LiveLogSubscriptionApi is the subscription-facing API for live ETH logs.
// Callers supply a parsed `LogFilter` and a live execution event. The API
// returns localized log entries ready for JSON-RPC subscription formatting.
// Missing callbacks mean live log matching is disabled for that consumer.
struct LiveLogSubscriptionApi {
  std::function<std::vector<LocalisedLogEntry>(const LogFilter&, const LiveLogBlock&)> matching_logs;
};

// Creates the default compatibility implementation. It preserves legacy ETH
// filter semantics while keeping receipt traversal behind the subscription API
// boundary.
inline LiveLogSubscriptionApi makeLiveLogSubscriptionApi() {
  LiveLogSubscriptionApi api;
  api.matching_logs = [](const LogFilter& filter, const LiveLogBlock& block) {
    std::vector<LocalisedLogEntry> logs;
    if (!filter.matches(block.log_bloom)) {
      return logs;
    }

    const auto receipt_count = std::min(block.transaction_receipts.size(), block.transaction_hashes.size());
    for (uint32_t idx = 0; idx < receipt_count; ++idx) {
      ExtendedTransactionLocation loc{{{block.block_number, idx}, block.block_hash}, block.transaction_hashes[idx]};
      filter.match_one(loc, block.transaction_receipts[idx],
                       [&](const LocalisedLogEntry& entry) { logs.push_back(entry); });
    }
    return logs;
  };
  return api;
}

}  // namespace taraxa::net::rpc::eth
