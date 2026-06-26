#pragma once

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>

#include "data.hpp"
#include "final_chain/final_chain.hpp"
#include "network/rpc/EthFace.h"
#include "watches.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::net::rpc::eth {

#ifdef RUSTAXA_ENABLE
// FinalizedLogReplayApi is ETH RPC's bridge-facing finalized-log replay port.
// It exposes only the finalized head, bloom-index candidate blocks, and
// transaction receipt rows needed for `eth_getLogs` and installed log filters.
struct FinalizedLogReplayApi {
  std::function<EthBlockNumber()> latest_finalized_block_number;
  std::function<rust::Vec<uint64_t>(const std::array<uint8_t, 256>&, EthBlockNumber, EthBlockNumber)> blocks_with_bloom;
  std::function<rust::Vec<rustaxa::TransactionReceiptPublicView>(EthBlockNumber)> transaction_receipts_by_block_number;
};
#endif

struct EthParams {
  Address address;
  uint64_t chain_id = 0;
  uint64_t gas_limit = ((uint64_t)1 << 53) - 1;
  std::shared_ptr<final_chain::FinalChain> final_chain;
  std::function<std::shared_ptr<Transaction>(const h256&)> get_trx;
#ifdef RUSTAXA_ENABLE
  std::function<rustaxa::TransactionPublicView(const h256&)> query_transaction;
  std::function<rustaxa::TransactionPublicView(EthBlockNumber, uint64_t)> query_transaction_by_block_number_and_index;
  std::function<rustaxa::TransactionPublicView(const h256&, uint64_t)> query_transaction_by_block_hash_and_index;
  std::function<uint64_t(EthBlockNumber)> query_transaction_count_by_block_number;
  std::function<uint64_t(const h256&)> query_transaction_count_by_block_hash;
  std::function<rustaxa::TransactionReceiptPublicView(const h256&)> query_transaction_receipt;
  std::function<rust::Vec<rustaxa::TransactionReceiptPublicView>(EthBlockNumber)>
      query_transaction_receipts_by_block_number;
  std::function<rustaxa::FinalChainBlockView(EthBlockNumber)> query_final_chain_block_by_number;
  std::function<rustaxa::FinalChainBlockNumberLookup(const h256&)> query_final_chain_block_number_by_hash;
  std::function<EthBlockNumber()> query_final_chain_last_block_number;
  std::optional<FinalizedLogReplayApi> query_log_replay;
  std::function<std::optional<state_api::Account>(const Address&, EthBlockNumber)> query_account;
  std::function<h256(const Address&, const u256&, EthBlockNumber)> query_account_storage;
  std::function<bytes(const Address&, EthBlockNumber)> query_account_code;
#endif
  std::function<void(const std::shared_ptr<Transaction>& trx)> send_trx;
  std::function<u256()> gas_pricer = [] { return u256(0); };
  std::function<uint64_t()> get_earliest_block = [] { return uint64_t(0); };
  std::function<std::optional<SyncStatus>()> syncing_probe = [] { return std::nullopt; };
  WatchesConfig watches_cfg;
};

struct Eth : virtual ::taraxa::net::EthFace {
  Eth() = default;
  virtual ~Eth() = default;

  Eth(const Eth&) = default;
  Eth(Eth&&) = default;
  Eth& operator=(const Eth&) = default;
  Eth& operator=(Eth&& rhs) {
    ::taraxa::net::EthFace::operator=(std::move(rhs));
    return *this;
  }
  virtual void note_block_executed(const final_chain::BlockHeader&, const SharedTransactions&,
                                   const TransactionReceipts&) = 0;
  virtual void note_pending_transaction(const h256& trx_hash) = 0;
};

std::shared_ptr<Eth> NewEth(EthParams&&);

Address toAddress(const std::string& s);
}  // namespace taraxa::net::rpc::eth
