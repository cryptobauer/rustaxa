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

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

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

#ifdef RUSTAXA_ENABLE
// ConsensusQueryReader is GraphQL Query's narrow Rust consensus-query boundary.
// It exposes only the stable DTO lookups needed by public GraphQL methods so
// the Query object does not retain storage or consensus-manager handles after
// compatibility construction.
struct ConsensusQueryReader {
  std::function<rustaxa::FinalChainBlockNumberLookup(const dev::h256&)> final_chain_block_number_by_hash;
  std::function<uint64_t()> final_chain_last_block_number;
  std::function<rustaxa::FinalChainBlockView(uint64_t)> final_chain_block_by_number;
  std::function<uint64_t(::taraxa::EthBlockNumber)> transaction_count_by_block_number;
  std::function<rustaxa::TransactionPublicView(::taraxa::EthBlockNumber, uint64_t)>
      transaction_by_block_number_and_index;
  std::function<rustaxa::TransactionPublicView(const ::taraxa::trx_hash_t&)> transaction_by_hash;
  std::function<rustaxa::TransactionReceiptPublicView(const ::taraxa::trx_hash_t&)> transaction_receipt_by_hash;
  std::function<rustaxa::ConsensusStatusView()> status;
  std::function<rustaxa::DagBlockPublicView(const ::taraxa::blk_hash_t&)> dag_block_by_hash;
  std::function<std::vector<rustaxa::DagBlockPublicView>(::taraxa::level_t, uint64_t)> dag_blocks_by_level;
  std::function<std::vector<rustaxa::DagBlockPublicView>(uint64_t)> finalized_dag_blocks_by_period;
};
#endif

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
#ifdef RUSTAXA_ENABLE
  ConsensusQueryReader consensus_query;
#endif
};

class Query {
 public:
  explicit Query(QueryReaders readers, uint64_t chain_id = 0) noexcept;
  explicit Query(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                 std::shared_ptr<::taraxa::DagManager> dag_manager, std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                 std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                 std::shared_ptr<::taraxa::DbStorage> db, std::shared_ptr<::taraxa::GasPricer> gas_pricer,
                 std::weak_ptr<::taraxa::Network> network, uint64_t chain_id,
                 ::taraxa::net::LiveStatusReader live_status = {}) noexcept;
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
#ifdef RUSTAXA_ENABLE
  ConsensusQueryReader consensus_query_reader_;
#endif
  std::function<std::shared_ptr<object::Block>(::taraxa::EthBlockNumber)> get_block_by_num_;
};

}  // namespace graphql::taraxa
