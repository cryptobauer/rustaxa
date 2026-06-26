#pragma once

#include "QueryObject.h"
#include "dag/dag_manager.hpp"
#include "final_chain/final_chain.hpp"
#include "graphql/account.hpp"
#include "graphql/block.hpp"
#include "graphql/sync_state.hpp"
#include "graphql/transaction.hpp"
#include "graphql/types/current_state.hpp"
#include "graphql/types/dag_block.hpp"
#include "network/live_status.hpp"
#include "network/network.hpp"
#include "pbft/pbft_manager.hpp"
#include "transaction/gas_pricer.hpp"
#include "transaction/transaction_manager.hpp"

namespace graphql::taraxa {

// QueryBlockReader is GraphQL Query's finalized-block acquisition boundary. It
// supplies block-number, header, and PBFT-link facts for top-level block
// queries without exposing FinalChain or DbStorage reads to public GraphQL
// query methods.
struct QueryBlockReader {
  std::function<::taraxa::EthBlockNumber()> latest_block_number;
  std::function<std::optional<::taraxa::EthBlockNumber>(const dev::h256&)> block_number_by_hash;
  std::function<std::shared_ptr<const ::taraxa::final_chain::BlockHeader>(std::optional<::taraxa::EthBlockNumber>)>
      block_header;
  std::function<std::optional<::taraxa::blk_hash_t>(::taraxa::EthBlockNumber)> pbft_hash_by_period;
};

// QueryTransactionReader is GraphQL Query's top-level transaction acquisition
// boundary. It resolves transaction hashes to payloads without exposing
// TransactionManager to public GraphQL query methods.
struct QueryTransactionReader {
  std::function<std::shared_ptr<::taraxa::Transaction>(const ::taraxa::trx_hash_t&)> transaction_by_hash;
};

// QueryGasPriceReader is GraphQL Query's gas-price acquisition boundary. It
// supplies the current node gas-price bid without exposing GasPricer to public
// GraphQL query methods.
struct QueryGasPriceReader {
  std::function<dev::u256()> bid;
};

// QueryDagBlockReader is GraphQL Query's DAG block acquisition boundary. It
// supplies the top-level DAG block lists and default levels needed by Query
// without exposing DagManager, DbStorage, or FinalChain period lookups to the
// public GraphQL query methods.
struct QueryDagBlockReader {
  std::function<std::shared_ptr<::taraxa::DagBlock>(const ::taraxa::blk_hash_t&)> block_by_hash;
  std::function<::taraxa::level_t()> latest_level;
  std::function<uint64_t()> latest_finalized_period;
  std::function<std::vector<std::shared_ptr<::taraxa::DagBlock>>(::taraxa::level_t)> blocks_by_level;
  std::function<std::vector<std::shared_ptr<::taraxa::DagBlock>>(uint64_t)> finalized_blocks_by_period;
};

// QueryReaders is GraphQL Query's primary external read API bundle. It contains
// the narrow read callbacks needed by public GraphQL fields so callers can wire
// ConsensusQueryApi, live-status snapshots, or compatibility adapters without
// exposing broad consensus managers to the query object.
struct QueryReaders {
  AccountStateReader account;
  QueryBlockReader block;
  BlockTransactionReader block_transaction;
  QueryTransactionReader transaction;
  TransactionReceiptReader transaction_receipt;
  QueryGasPriceReader gas_price;
  QueryDagBlockReader dag_block;
  DagBlockTransactionReader dag_block_transaction;
  DagBlockPeriodReader dag_block_period;
  CurrentStateReader current_state;
  SyncStateReader sync_state;
};

class Query {
 public:
  explicit Query(QueryReaders readers, uint64_t chain_id = 0) noexcept;
  explicit Query(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                 std::shared_ptr<::taraxa::DagManager> dag_manager, std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                 std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                 std::shared_ptr<::taraxa::DbStorage> db,
                 std::shared_ptr<::taraxa::GasPricer> gas_pricer, std::weak_ptr<::taraxa::Network> network,
                 uint64_t chain_id, ::taraxa::net::LiveStatusReader live_status = {}) noexcept;
  explicit Query(AccountStateReader account_reader, uint64_t chain_id = 0, QueryBlockReader block_reader = {},
                 BlockTransactionReader block_transaction_reader = {}, QueryTransactionReader transaction_reader = {},
                 QueryGasPriceReader gas_price_reader = {}, QueryDagBlockReader dag_block_reader = {},
                 DagBlockTransactionReader dag_block_transaction_reader = {},
                 DagBlockPeriodReader dag_block_period_reader = {}) noexcept;

  std::shared_ptr<object::Block> getBlock(std::optional<response::Value>&& numberArg,
                                          std::optional<response::Value>&& hashArg) const;
  std::vector<std::shared_ptr<object::Block>> getBlocks(response::Value&& fromArg,
                                                        std::optional<response::Value>&& toArg) const;
  std::shared_ptr<object::Transaction> getTransaction(response::Value&& hashArg) const;
  std::shared_ptr<object::Account> getAccount(response::Value&& addressArg,
                                              std::optional<response::Value>&& blockArg) const;
  response::Value getGasPrice() const;
  std::shared_ptr<object::SyncState> getSyncing() const;
  response::Value getChainID() const;
  std::shared_ptr<object::DagBlock> getDagBlock(std::optional<response::Value>&& hashArg) const;
  std::vector<std::shared_ptr<object::DagBlock>> getPeriodDagBlocks(std::optional<response::Value>&& periodArg) const;
  std::vector<std::shared_ptr<object::DagBlock>> getDagBlocks(std::optional<response::Value>&& dagLevelArg,
                                                              std::optional<int>&& countArg,
                                                              std::optional<bool>&& reverseArg) const;
  std::shared_ptr<object::CurrentState> getNodeState() const;

 private:
  // TODO: use pagination limit for all "list" queries
  static constexpr size_t kMaxPropagationLimit{100};

  // Rust mode uses this storage handle only to construct ConsensusQueryApi. The
  // same handle also builds non-Rust compatibility readers in the legacy
  // constructor.
  std::shared_ptr<::taraxa::DbStorage> db_;
  const uint64_t kChainId;
  AccountStateReader account_reader_;
  QueryBlockReader block_reader_;
  BlockTransactionReader block_transaction_reader_;
  QueryTransactionReader transaction_reader_;
  TransactionReceiptReader transaction_receipt_reader_;
  QueryGasPriceReader gas_price_reader_;
  QueryDagBlockReader dag_block_reader_;
  DagBlockTransactionReader dag_block_transaction_reader_;
  DagBlockPeriodReader dag_block_period_reader_;
  CurrentStateReader current_state_reader_;
  SyncStateReader sync_state_reader_;
  std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num_;
};

}  // namespace graphql::taraxa
