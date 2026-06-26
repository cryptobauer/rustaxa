#pragma once

#include <cstdint>

#include "data.hpp"
#include "final_chain/final_chain.hpp"
#include "network/rpc/EthFace.h"
#include "watches.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::net::rpc::eth {

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
  std::function<EthBlockNumber()> query_final_chain_last_block_number;
  std::function<rust::Vec<uint64_t>(const std::array<uint8_t, 256>&, EthBlockNumber, EthBlockNumber)>
      query_blocks_with_bloom;
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
