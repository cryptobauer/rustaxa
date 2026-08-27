#include "graphql/query.hpp"

#include <libdevcore/CommonJS.h>

#include <stdexcept>

#include "graphql/account.hpp"
#include "graphql/block.hpp"
#include "graphql/log.hpp"
#include "graphql/sync_state.hpp"
#include "graphql/transaction.hpp"
#include "graphql/types/current_state.hpp"
#include "graphql/types/dag_block.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/system_transaction.hpp"
#endif

using namespace std::literals;

namespace graphql::taraxa {

namespace {
#ifdef RUSTAXA_ENABLE
dev::h256 hashFromBridge(const std::array<uint8_t, 32>& hash) {
  return dev::h256(hash.data(), dev::h256::ConstructFromPointer);
}

std::shared_ptr<::taraxa::DagBlock> materializeDagBlockView(const rustaxa::DagBlockPublicView& view) {
  if (!view.found) {
    return nullptr;
  }
  auto block = std::make_shared<::taraxa::DagBlock>(dev::bytes(view.block_rlp.begin(), view.block_rlp.end()));
  if (block->getHash() != hashFromBridge(view.hash)) {
    throw std::runtime_error("CONSENSUS_QUERY_GRAPHQL_DAG_BLOCK_HASH_MISMATCH");
  }
  return block;
}

ConsensusQueryReader makeConsensusQueryReader(::taraxa::net::ConsensusQueryApiPtr query_api) {
  ConsensusQueryReader reader;
  if (!query_api) {
    return reader;
  }
  reader.final_chain_block_number_by_hash = [query_api](const dev::h256& block_hash) {
    return (*query_api)->consensus_query_final_chain_block_number_by_hash(block_hash.asArray());
  };
  reader.final_chain_last_block_number = [query_api] {
    return (*query_api)->consensus_query_final_chain_last_block_number();
  };
  reader.final_chain_block_by_number = [query_api](uint64_t block_number) {
    return (*query_api)->consensus_query_final_chain_block_by_number(block_number);
  };
  reader.transaction_count_by_block_number = [query_api](::taraxa::EthBlockNumber block_number) {
    return (*query_api)->consensus_query_transaction_count_by_block_number(block_number);
  };
  reader.transaction_by_block_number_and_index = [query_api](::taraxa::EthBlockNumber block_number, uint64_t index) {
    return (*query_api)->consensus_query_transaction_by_block_number_and_index(block_number, index);
  };
  reader.transaction_by_hash = [query_api](const ::taraxa::trx_hash_t& transaction_hash) {
    return (*query_api)->consensus_query_transaction_by_hash(transaction_hash.asArray());
  };
  reader.transaction_receipt_by_hash = [query_api](const ::taraxa::trx_hash_t& transaction_hash) {
    return (*query_api)->consensus_query_transaction_receipt_by_hash(transaction_hash.asArray());
  };
  reader.status = [query_api] { return (*query_api)->consensus_query_status(); };
  reader.dag_block_by_hash = [query_api](const ::taraxa::blk_hash_t& block_hash) {
    return (*query_api)->consensus_query_dag_block_by_hash(block_hash.asArray());
  };
  reader.dag_blocks_by_level = [query_api](::taraxa::level_t level, uint64_t number_of_levels) {
    auto views = (*query_api)->consensus_query_dag_blocks_by_level(level, number_of_levels);
    std::vector<rustaxa::DagBlockPublicView> result;
    result.reserve(views.size());
    for (auto& view : views) {
      result.emplace_back(std::move(view));
    }
    return result;
  };
  reader.finalized_dag_blocks_by_period = [query_api](uint64_t period) {
    auto views = (*query_api)->consensus_query_finalized_dag_blocks_by_period(period);
    std::vector<rustaxa::DagBlockPublicView> result;
    result.reserve(views.size());
    for (auto& view : views) {
      result.emplace_back(std::move(view));
    }
    return result;
  };
  return reader;
}

bool hasConsensusQueryReader(const ConsensusQueryReader& reader) {
  return reader.final_chain_block_number_by_hash && reader.final_chain_last_block_number &&
         reader.final_chain_block_by_number && reader.transaction_count_by_block_number &&
         reader.transaction_by_block_number_and_index && reader.transaction_by_hash &&
         reader.transaction_receipt_by_hash && reader.status && reader.dag_block_by_hash &&
         reader.dag_blocks_by_level && reader.finalized_dag_blocks_by_period;
}
#endif

QueryBlockReader makeQueryBlockReader(const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain,
                                      const std::shared_ptr<::taraxa::DbStorage>& db
#ifdef RUSTAXA_ENABLE
                                      ,
                                      ::taraxa::net::ConsensusQueryApiPtr consensus_query_api
#endif
) {
  QueryBlockReader reader;
  reader.latest_block_number = [final_chain] { return final_chain ? final_chain->lastBlockNumber() : uint64_t(0); };
  reader.block_number_by_hash = [final_chain](const dev::h256& hash) {
    if (!final_chain) {
      return std::optional<::taraxa::EthBlockNumber>{};
    }
    return final_chain->blockNumber(hash);
  };
  reader.block_header = [final_chain](std::optional<::taraxa::EthBlockNumber> block_number) {
    if (!final_chain) {
      return std::shared_ptr<const ::taraxa::final_chain::BlockHeader>{};
    }
    return final_chain->blockHeader(block_number);
  };
  reader.pbft_hash_by_period = [db
#ifdef RUSTAXA_ENABLE
                                ,
                                consensus_query_api
#endif
  ](::taraxa::EthBlockNumber period) -> std::optional<::taraxa::blk_hash_t> {
#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      return std::nullopt;
    }
    auto lookup = (*consensus_query_api)->consensus_query_pbft_block_hash_by_period(period);
    return lookup.found ? std::optional<::taraxa::blk_hash_t>{hashFromBridge(lookup.hash)} : std::nullopt;
#else
    if (!db) {
      return std::nullopt;
    }
    auto pbft_block = db->getPbftBlock(period);
    if (!pbft_block) {
      return std::nullopt;
    }
    return pbft_block->getBlockHash();
#endif
  };
  return reader;
}

void fillMissingQueryBlockReaderCallbacks(QueryBlockReader& reader,
                                          const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain,
                                          const std::shared_ptr<::taraxa::DbStorage>& db
#ifdef RUSTAXA_ENABLE
                                          ,
                                          ::taraxa::net::ConsensusQueryApiPtr consensus_query_api
#endif
) {
  auto defaults = makeQueryBlockReader(final_chain, db
#ifdef RUSTAXA_ENABLE
                                       ,
                                       std::move(consensus_query_api)
#endif
  );
  if (!reader.latest_block_number) {
    reader.latest_block_number = std::move(defaults.latest_block_number);
  }
  if (!reader.block_number_by_hash) {
    reader.block_number_by_hash = std::move(defaults.block_number_by_hash);
  }
  if (!reader.block_header) {
    reader.block_header = std::move(defaults.block_header);
  }
  if (!reader.pbft_hash_by_period) {
    reader.pbft_hash_by_period = std::move(defaults.pbft_hash_by_period);
  }
}

BlockTransactionReader makeQueryBlockTransactionReader(
    const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain) {
  BlockTransactionReader reader;
  reader.transaction_count = [final_chain](::taraxa::EthBlockNumber block_number) {
    return final_chain ? final_chain->transactionCount(block_number) : 0;
  };
  reader.transactions = [final_chain](::taraxa::EthBlockNumber block_number) {
    if (!final_chain) {
      return std::vector<std::shared_ptr<::taraxa::Transaction>>{};
    }
    return final_chain->transactions(block_number);
  };
  return reader;
}

void fillMissingBlockTransactionReaderCallbacks(BlockTransactionReader& reader,
                                                const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain) {
  auto defaults = makeQueryBlockTransactionReader(final_chain);
  if (!reader.transaction_count) {
    reader.transaction_count = std::move(defaults.transaction_count);
  }
  if (!reader.transactions) {
    reader.transactions = std::move(defaults.transactions);
  }
}

#ifdef RUSTAXA_ENABLE
QueryTransactionReader makeQueryTransactionReader() {
#else
QueryTransactionReader makeQueryTransactionReader(
    const std::shared_ptr<::taraxa::TransactionManager>& transaction_manager) {
#endif
  QueryTransactionReader reader;
#ifdef RUSTAXA_ENABLE
  reader.transaction_by_hash = [](const ::taraxa::trx_hash_t&) { return std::shared_ptr<::taraxa::Transaction>{}; };
#else
  reader.transaction_by_hash = [transaction_manager](const ::taraxa::trx_hash_t& hash) {
    return transaction_manager ? transaction_manager->getTransaction(hash) : nullptr;
  };
#endif
  return reader;
}

void fillMissingQueryTransactionReaderCallbacks(QueryTransactionReader& reader
#ifndef RUSTAXA_ENABLE
                                                ,
                                                const std::shared_ptr<::taraxa::TransactionManager>& transaction_manager
#endif
) {
  auto defaults = makeQueryTransactionReader(
#ifndef RUSTAXA_ENABLE
      transaction_manager
#endif
  );
  if (!reader.transaction_by_hash) {
    reader.transaction_by_hash = std::move(defaults.transaction_by_hash);
  }
}

TransactionReceiptReader makeQueryTransactionReceiptReader(
    const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain) {
  TransactionReceiptReader reader;
  reader.location = [final_chain](const ::taraxa::trx_hash_t& hash) {
    if (!final_chain) {
      return std::optional<::taraxa::TransactionLocation>{};
    }
    return final_chain->transactionLocation(hash);
  };
  reader.receipt = [final_chain](::taraxa::EthBlockNumber period, uint32_t position, const ::taraxa::trx_hash_t& hash) {
    if (!final_chain) {
      return std::optional<::taraxa::TransactionReceipt>{};
    }
    return final_chain->transactionReceipt(period, position, hash);
  };
  return reader;
}

void fillMissingQueryTransactionReceiptReaderCallbacks(
    TransactionReceiptReader& reader, const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain) {
  auto defaults = makeQueryTransactionReceiptReader(final_chain);
  if (!reader.location) {
    reader.location = std::move(defaults.location);
  }
  if (!reader.receipt) {
    reader.receipt = std::move(defaults.receipt);
  }
}

#ifdef RUSTAXA_ENABLE
QueryGasPriceReader makeQueryGasPriceReader() {
  QueryGasPriceReader reader;
  reader.bid = [] { return dev::u256(0); };
  return reader;
}

void fillMissingQueryGasPriceReaderCallbacks(QueryGasPriceReader& reader) {
  auto defaults = makeQueryGasPriceReader();
  if (!reader.bid) {
    reader.bid = std::move(defaults.bid);
  }
}

#else
QueryGasPriceReader makeQueryGasPriceReader(const std::shared_ptr<::taraxa::GasPricer>& gas_pricer) {
  QueryGasPriceReader reader;
  reader.bid = [gas_pricer] { return gas_pricer ? gas_pricer->bid() : dev::u256(0); };
  return reader;
}

void fillMissingQueryGasPriceReaderCallbacks(QueryGasPriceReader& reader,
                                             const std::shared_ptr<::taraxa::GasPricer>& gas_pricer) {
  auto defaults = makeQueryGasPriceReader(gas_pricer);
  if (!reader.bid) {
    reader.bid = std::move(defaults.bid);
  }
}
#endif

QueryDagBlockReader makeQueryDagBlockReader(const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain,
#ifndef RUSTAXA_ENABLE
                                            const std::shared_ptr<::taraxa::DagManager>& dag_manager,
#endif
                                            const std::shared_ptr<::taraxa::DbStorage>& db
#ifdef RUSTAXA_ENABLE
                                            ,
                                            ::taraxa::net::ConsensusQueryApiPtr consensus_query_api
#endif
) {
  QueryDagBlockReader reader;
#ifdef RUSTAXA_ENABLE
  reader.block_by_hash = [consensus_query_api](const ::taraxa::blk_hash_t& hash) {
    return consensus_query_api
               ? materializeDagBlockView((*consensus_query_api)->consensus_query_dag_block_by_hash(hash.asArray()))
               : std::shared_ptr<::taraxa::DagBlock>{};
  };
  reader.latest_level = [consensus_query_api] {
    return consensus_query_api ? (*consensus_query_api)->consensus_query_live_dag_status().max_level
                               : ::taraxa::level_t(0);
  };
#else
  reader.block_by_hash = [dag_manager](const ::taraxa::blk_hash_t& hash) {
    return dag_manager ? dag_manager->getDagBlock(hash) : nullptr;
  };
  reader.latest_level = [dag_manager] { return dag_manager ? dag_manager->getMaxLevel() : ::taraxa::level_t(0); };
#endif
  reader.latest_finalized_period = [final_chain] { return final_chain ? final_chain->lastBlockNumber() : uint64_t(0); };
  reader.blocks_by_level = [db
#ifdef RUSTAXA_ENABLE
                            ,
                            consensus_query_api
#endif
  ](::taraxa::level_t level) {
#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      return std::vector<std::shared_ptr<::taraxa::DagBlock>>{};
    }
    auto views = (*consensus_query_api)->consensus_query_dag_blocks_by_level(level, 1);
    std::vector<std::shared_ptr<::taraxa::DagBlock>> blocks;
    blocks.reserve(views.size());
    for (const auto& view : views) {
      blocks.emplace_back(materializeDagBlockView(view));
    }
    return blocks;
#else
    if (!db) {
      return std::vector<std::shared_ptr<::taraxa::DagBlock>>{};
    }
    return db->getDagBlocksAtLevel(level, 1);
#endif
  };
  reader.finalized_blocks_by_period = [db
#ifdef RUSTAXA_ENABLE
                                       ,
                                       consensus_query_api
#endif
  ](uint64_t period) {
#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      return std::vector<std::shared_ptr<::taraxa::DagBlock>>{};
    }
    auto views = (*consensus_query_api)->consensus_query_finalized_dag_blocks_by_period(period);
    std::vector<std::shared_ptr<::taraxa::DagBlock>> blocks;
    blocks.reserve(views.size());
    for (const auto& view : views) {
      blocks.emplace_back(materializeDagBlockView(view));
    }
    return blocks;
#else
    if (!db) {
      return std::vector<std::shared_ptr<::taraxa::DagBlock>>{};
    }
    return db->getFinalizedDagBlockByPeriod(period);
#endif
  };
  return reader;
}

void fillMissingQueryDagBlockReaderCallbacks(QueryDagBlockReader& reader,
                                             const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain,
#ifndef RUSTAXA_ENABLE
                                             const std::shared_ptr<::taraxa::DagManager>& dag_manager,
#endif
                                             const std::shared_ptr<::taraxa::DbStorage>& db
#ifdef RUSTAXA_ENABLE
                                             ,
                                             ::taraxa::net::ConsensusQueryApiPtr consensus_query_api
#endif
) {
  auto defaults = makeQueryDagBlockReader(final_chain,
#ifndef RUSTAXA_ENABLE
                                          dag_manager,
#endif
                                          db
#ifdef RUSTAXA_ENABLE
                                          ,
                                          std::move(consensus_query_api)
#endif
  );
  if (!reader.block_by_hash) {
    reader.block_by_hash = std::move(defaults.block_by_hash);
  }
  if (!reader.latest_level) {
    reader.latest_level = std::move(defaults.latest_level);
  }
  if (!reader.latest_finalized_period) {
    reader.latest_finalized_period = std::move(defaults.latest_finalized_period);
  }
  if (!reader.blocks_by_level) {
    reader.blocks_by_level = std::move(defaults.blocks_by_level);
  }
  if (!reader.finalized_blocks_by_period) {
    reader.finalized_blocks_by_period = std::move(defaults.finalized_blocks_by_period);
  }
}

#ifdef RUSTAXA_ENABLE
DagBlockTransactionReader makeQueryDagBlockTransactionReader() {
#else
DagBlockTransactionReader makeQueryDagBlockTransactionReader(
    const std::shared_ptr<::taraxa::TransactionManager>& transaction_manager) {
#endif
  DagBlockTransactionReader reader;
#ifdef RUSTAXA_ENABLE
  reader.transaction_by_hash = [](const ::taraxa::trx_hash_t&) { return std::shared_ptr<::taraxa::Transaction>{}; };
#else
  reader.transaction_by_hash = [transaction_manager](const ::taraxa::trx_hash_t& hash) {
    return transaction_manager ? transaction_manager->getTransaction(hash) : nullptr;
  };
#endif
  return reader;
}

DagBlockPeriodReader makeQueryDagBlockPeriodReader(
#ifdef RUSTAXA_ENABLE
    ::taraxa::net::ConsensusQueryApiPtr consensus_query_api
#else
    const std::shared_ptr<::taraxa::PbftManager>& pbft_manager
#endif
) {
  DagBlockPeriodReader reader;
#ifdef RUSTAXA_ENABLE
  reader.period_by_hash = [consensus_query_api](const ::taraxa::blk_hash_t& hash) -> std::optional<uint64_t> {
    if (!consensus_query_api) {
      return std::nullopt;
    }
    const auto view = (*consensus_query_api)->consensus_query_dag_block_by_hash(hash.asArray());
    if (!view.found || !view.finalized_period_found) {
      return std::nullopt;
    }
    return view.finalized_period;
  };
#else
  reader.period_by_hash = [pbft_manager](const ::taraxa::blk_hash_t& hash) -> std::optional<uint64_t> {
    if (!pbft_manager) {
      return std::nullopt;
    }
    const auto [has_period, period] = pbft_manager->getDagBlockPeriod(hash);
    if (!has_period) {
      return std::nullopt;
    }
    return period;
  };
#endif
  return reader;
}

void fillMissingDagBlockTransactionReaderCallbacks(
    DagBlockTransactionReader& reader
#ifndef RUSTAXA_ENABLE
    ,
    const std::shared_ptr<::taraxa::TransactionManager>& transaction_manager
#endif
) {
  auto defaults = makeQueryDagBlockTransactionReader(
#ifndef RUSTAXA_ENABLE
      transaction_manager
#endif
  );
  if (!reader.transaction_by_hash) {
    reader.transaction_by_hash = std::move(defaults.transaction_by_hash);
  }
}

void fillMissingDagBlockPeriodReaderCallbacks(DagBlockPeriodReader& reader,
#ifdef RUSTAXA_ENABLE
                                              ::taraxa::net::ConsensusQueryApiPtr consensus_query_api
#else
                                              const std::shared_ptr<::taraxa::PbftManager>& pbft_manager
#endif
) {
  auto defaults = makeQueryDagBlockPeriodReader(
#ifdef RUSTAXA_ENABLE
      std::move(consensus_query_api)
#else
      pbft_manager
#endif
  );
  if (!reader.period_by_hash) {
    reader.period_by_hash = std::move(defaults.period_by_hash);
  }
}

CurrentStateReader makeQueryCurrentStateReader(const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain
#ifndef RUSTAXA_ENABLE
                                               ,
                                               const std::shared_ptr<::taraxa::DagManager>& dag_manager) {
#else
) {
#endif
  CurrentStateReader reader;
  reader.final_block = [final_chain] { return final_chain ? final_chain->lastBlockNumber() : uint64_t(0); };
#ifdef RUSTAXA_ENABLE
  reader.dag_block_level = [] { return uint64_t(0); };
  reader.dag_block_period = [] { return uint64_t(0); };
#else
  reader.dag_block_level = [dag_manager] { return dag_manager ? dag_manager->getMaxLevel() : uint64_t(0); };
  reader.dag_block_period = [dag_manager] { return dag_manager ? dag_manager->getLatestPeriod() : uint64_t(0); };
#endif
  return reader;
}

void fillMissingCurrentStateReaderCallbacks(CurrentStateReader& reader,
                                            const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain
#ifndef RUSTAXA_ENABLE
                                            ,
                                            const std::shared_ptr<::taraxa::DagManager>& dag_manager) {
#else
) {
#endif
  auto defaults = makeQueryCurrentStateReader(final_chain
#ifndef RUSTAXA_ENABLE
                                              ,
                                              dag_manager
#endif
  );
  if (!reader.final_block) {
    reader.final_block = std::move(defaults.final_block);
  }
  if (!reader.dag_block_level) {
    reader.dag_block_level = std::move(defaults.dag_block_level);
  }
  if (!reader.dag_block_period) {
    reader.dag_block_period = std::move(defaults.dag_block_period);
  }
}

SyncStateReader makeQuerySyncStateReader(const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain,
                                         std::weak_ptr<::taraxa::Network> network,
                                         ::taraxa::net::LiveStatusReader live_status) {
  SyncStateReader reader;
  reader.current_block = [final_chain] { return final_chain ? final_chain->lastBlockNumber() : uint64_t(0); };
  reader.highest_block = [network = std::move(network),
                          live_status = std::move(live_status)]() -> std::optional<uint64_t> {
    if (live_status) {
      return live_status().max_peer_pbft_chain_size;
    }
    auto net = network.lock();
    if (!net) {
      return std::nullopt;
    }
    const auto peer = net->getMaxChainPeer();
    if (!peer) {
      return std::nullopt;
    }
    return peer->pbft_chain_size_.load();
  };
  return reader;
}

void fillMissingSyncStateReaderCallbacks(SyncStateReader& reader,
                                         const std::shared_ptr<::taraxa::final_chain::FinalChain>& final_chain,
                                         std::weak_ptr<::taraxa::Network> network,
                                         ::taraxa::net::LiveStatusReader live_status) {
  auto defaults = makeQuerySyncStateReader(final_chain, std::move(network), std::move(live_status));
  if (!reader.current_block) {
    reader.current_block = std::move(defaults.current_block);
  }
  if (!reader.highest_block) {
    reader.highest_block = std::move(defaults.highest_block);
  }
}
}  // namespace

#ifdef RUSTAXA_ENABLE
namespace {
constexpr uint8_t kConsensusQueryTransactionSourceMissing = 0;
constexpr uint8_t kConsensusQueryTransactionSourcePending = 1;
constexpr uint8_t kConsensusQueryTransactionSourceFinalizedRegular = 2;
constexpr uint8_t kConsensusQueryTransactionSourceFinalizedSystem = 3;

dev::Address addressFromBridge(const std::array<uint8_t, 20>& address) {
  return dev::Address(address.data(), dev::Address::ConstructFromPointer);
}

dev::bytes bytesFromBridge(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

dev::bytes bytesFromBridge(const std::array<uint8_t, 32>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

std::shared_ptr<::taraxa::Transaction> materializeTransactionView(const rustaxa::TransactionPublicView& view) {
  if (!view.found) {
    return nullptr;
  }

  std::shared_ptr<::taraxa::Transaction> transaction;
  if (view.source == kConsensusQueryTransactionSourceFinalizedSystem) {
    transaction = std::make_shared<::taraxa::SystemTransaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.source == kConsensusQueryTransactionSourcePending ||
             view.source == kConsensusQueryTransactionSourceFinalizedRegular) {
    transaction = std::make_shared<::taraxa::Transaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.source != kConsensusQueryTransactionSourceMissing) {
    throw std::runtime_error("CONSENSUS_QUERY_TRANSACTION_UNKNOWN_SOURCE");
  }

  if (transaction && transaction->getHash() != hashFromBridge(view.hash)) {
    throw std::runtime_error("CONSENSUS_QUERY_TRANSACTION_HASH_MISMATCH");
  }
  return transaction;
}

std::shared_ptr<::taraxa::final_chain::BlockHeader> blockHeaderFromView(const rustaxa::FinalChainBlockView& view) {
  if (!view.found) {
    return nullptr;
  }
  auto header = std::make_shared<::taraxa::final_chain::BlockHeader>();
  header->hash = hashFromBridge(view.hash);
  header->parent_hash = hashFromBridge(view.parent_hash);
  header->author = addressFromBridge(view.author);
  header->state_root = hashFromBridge(view.state_root);
  header->transactions_root = hashFromBridge(view.transactions_root);
  header->receipts_root = hashFromBridge(view.receipts_root);
  if (view.log_bloom.size() != 256) {
    throw std::runtime_error("CONSENSUS_QUERY_GRAPHQL_BLOCK_LOG_BLOOM_SIZE");
  }
  header->log_bloom = ::taraxa::LogBloom(view.log_bloom.data(), ::taraxa::LogBloom::ConstructFromPointer);
  header->gas_used = view.gas_used;
  header->total_reward = dev::fromBigEndian<dev::u256>(bytesFromBridge(view.total_reward));
  header->number = view.number;
  return header;
}
}  // namespace
#endif

#ifndef RUSTAXA_ENABLE
Query::Query(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
             std::shared_ptr<::taraxa::DagManager> dag_manager,
#ifndef RUSTAXA_ENABLE
             std::shared_ptr<::taraxa::PbftManager> pbft_manager,
#endif
             std::shared_ptr<::taraxa::TransactionManager> transaction_manager, std::shared_ptr<::taraxa::DbStorage> db,
#ifdef RUSTAXA_ENABLE
             QueryGasPriceReader gas_price_reader,
#else
             std::shared_ptr<::taraxa::GasPricer> gas_pricer,
#endif
             std::weak_ptr<::taraxa::Network> network, uint64_t chain_id, ::taraxa::net::LiveStatusReader live_status
#ifdef RUSTAXA_ENABLE
             ,
             ::taraxa::net::ConsensusQueryApiPtr consensus_query_api
#endif
             ) noexcept
    : kChainId(chain_id),
      account_reader_(makeAccountStateReader(final_chain)),
      block_reader_(makeQueryBlockReader(final_chain, db
#ifdef RUSTAXA_ENABLE
                                         ,
                                         consensus_query_api
#endif
                                         )),
      block_transaction_reader_(makeQueryBlockTransactionReader(final_chain)),
      transaction_reader_(makeQueryTransactionReader(transaction_manager)),
      transaction_receipt_reader_(makeQueryTransactionReceiptReader(final_chain)),
#ifdef RUSTAXA_ENABLE
      gas_price_reader_(std::move(gas_price_reader)),
#else
      gas_price_reader_(makeQueryGasPriceReader(gas_pricer)),
#endif
      dag_block_reader_(makeQueryDagBlockReader(final_chain, dag_manager, db
#ifdef RUSTAXA_ENABLE
                                                ,
                                                consensus_query_api
#endif
                                                )),
      dag_block_transaction_reader_(makeQueryDagBlockTransactionReader(transaction_manager)),
      dag_block_period_reader_(makeQueryDagBlockPeriodReader(
#ifdef RUSTAXA_ENABLE
          consensus_query_api
#else
          pbft_manager
#endif
          )),
      current_state_reader_(makeQueryCurrentStateReader(final_chain, dag_manager)),
      sync_state_reader_(makeQuerySyncStateReader(final_chain, std::move(network), std::move(live_status)))
#ifdef RUSTAXA_ENABLE
      ,
      consensus_query_reader_(makeConsensusQueryReader(std::move(consensus_query_api)))
#endif
{
  get_block_by_num_ = [&](::taraxa::EthBlockNumber num) {
    return getBlock(response::Value(static_cast<int>(num)), std::nullopt);
  };
}
#endif

Query::Query(QueryReaders readers, uint64_t chain_id) noexcept
    : kChainId(chain_id),
      account_reader_(std::move(readers.account)),
      block_reader_(std::move(readers.block)),
      block_transaction_reader_(std::move(readers.block_transaction)),
      transaction_reader_(std::move(readers.transaction)),
      transaction_receipt_reader_(std::move(readers.transaction_receipt)),
      gas_price_reader_(std::move(readers.gas_price)),
      dag_block_reader_(std::move(readers.dag_block)),
      dag_block_transaction_reader_(std::move(readers.dag_block_transaction)),
      dag_block_period_reader_(std::move(readers.dag_block_period)),
      current_state_reader_(std::move(readers.current_state)),
      sync_state_reader_(std::move(readers.sync_state))
#ifdef RUSTAXA_ENABLE
      ,
      consensus_query_reader_(std::move(readers.consensus_query))
#endif
{
  fillMissingQueryBlockReaderCallbacks(block_reader_, nullptr, nullptr
#ifdef RUSTAXA_ENABLE
                                       ,
                                       {}
#endif
  );
  fillMissingBlockTransactionReaderCallbacks(block_transaction_reader_, nullptr);
  fillMissingQueryTransactionReaderCallbacks(transaction_reader_
#ifndef RUSTAXA_ENABLE
                                             ,
                                             nullptr
#endif
  );
  fillMissingQueryTransactionReceiptReaderCallbacks(transaction_receipt_reader_, nullptr);
  fillMissingQueryGasPriceReaderCallbacks(gas_price_reader_
#ifndef RUSTAXA_ENABLE
                                          ,
                                          nullptr
#endif
  );
  fillMissingQueryDagBlockReaderCallbacks(dag_block_reader_, nullptr,
#ifndef RUSTAXA_ENABLE
                                          nullptr,
#endif
                                          nullptr
#ifdef RUSTAXA_ENABLE
                                          ,
                                          {}
#endif
  );
  fillMissingDagBlockTransactionReaderCallbacks(dag_block_transaction_reader_
#ifndef RUSTAXA_ENABLE
                                                ,
                                                nullptr
#endif
  );
#ifdef RUSTAXA_ENABLE
  fillMissingDagBlockPeriodReaderCallbacks(dag_block_period_reader_, {});
#else
  fillMissingDagBlockPeriodReaderCallbacks(dag_block_period_reader_, nullptr);
#endif
  fillMissingCurrentStateReaderCallbacks(current_state_reader_, nullptr
#ifndef RUSTAXA_ENABLE
                                         ,
                                         nullptr
#endif
  );
  fillMissingSyncStateReaderCallbacks(sync_state_reader_, nullptr, {}, {});
  get_block_by_num_ = [&](::taraxa::EthBlockNumber num) {
    return getBlock(response::Value(static_cast<int>(num)), std::nullopt);
  };
}

#ifdef RUSTAXA_ENABLE
Query::Query(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain, QueryGasPriceReader gas_price_reader,
             std::weak_ptr<::taraxa::Network> network, uint64_t chain_id, ::taraxa::net::LiveStatusReader live_status,
             ::taraxa::net::ConsensusQueryApiPtr consensus_query_api) noexcept
    : Query(
          QueryReaders{
              makeAccountStateReader(final_chain),
              {},
              {},
              {},
              {},
              std::move(gas_price_reader),
              {},
              {},
              {},
              CurrentStateReader{[query = consensus_query_api] {
                                   return query ? (*query)->consensus_query_final_chain_last_block_number()
                                                : uint64_t(0);
                                 },
                                 [query = consensus_query_api] {
                                   return query ? (*query)->consensus_query_status().latest_dag_level : uint64_t(0);
                                 },
                                 [query = consensus_query_api] {
                                   if (!query) {
                                     return uint64_t(0);
                                   }
                                   const auto status = (*query)->consensus_query_status();
                                   return status.latest_dag_period_found ? status.latest_dag_period : uint64_t(0);
                                 }},
              makeQuerySyncStateReader(final_chain, std::move(network), std::move(live_status)),
              makeConsensusQueryReader(std::move(consensus_query_api)),
          },
          chain_id) {}
#endif

Query::Query(AccountStateReader account_reader, uint64_t chain_id, QueryBlockReader block_reader,
             BlockTransactionReader block_transaction_reader, QueryTransactionReader transaction_reader,
             QueryGasPriceReader gas_price_reader, QueryDagBlockReader dag_block_reader,
             DagBlockTransactionReader dag_block_transaction_reader,
             DagBlockPeriodReader dag_block_period_reader) noexcept
    : Query(QueryReaders{std::move(account_reader),
                         std::move(block_reader),
                         std::move(block_transaction_reader),
                         std::move(transaction_reader),
                         {},
                         std::move(gas_price_reader),
                         std::move(dag_block_reader),
                         std::move(dag_block_transaction_reader),
                         std::move(dag_block_period_reader),
                         {},
                         {}
#ifdef RUSTAXA_ENABLE
                         ,
                         {}
#endif
            },
            chain_id) {
}

std::shared_ptr<object::Block> Query::getBlock(std::optional<response::Value>&& number,
                                               std::optional<response::Value>&& hash) const {
#ifdef RUSTAXA_ENABLE
  if (hasConsensusQueryReader(consensus_query_reader_)) {
    const auto query = consensus_query_reader_;
    uint64_t block_number = 0;
    if (number) {
      const auto parsed_number = number->get<int>();
      if (parsed_number < 0) {
        return nullptr;
      }
      block_number = static_cast<uint64_t>(parsed_number);
    } else if (hash) {
      const auto block_number_lookup = query.final_chain_block_number_by_hash(dev::h256(hash->get<std::string>()));
      if (!block_number_lookup.found) {
        return nullptr;
      }
      block_number = block_number_lookup.value;
    } else {
      block_number = query.final_chain_last_block_number();
    }

    const auto block_view = query.final_chain_block_by_number(block_number);
    auto block_header = blockHeaderFromView(block_view);
    if (!block_header) {
      return nullptr;
    }

    ::taraxa::blk_hash_t pbft_block_hash;
    if (block_number != 0) {
      if (!block_view.has_pbft_hash) {
        return nullptr;
      }
      pbft_block_hash = hashFromBridge(block_view.pbft_block_hash);
    }

    return std::make_shared<object::Block>(std::make_shared<Block>(
        account_reader_, block_transaction_reader_, get_block_by_num_, pbft_block_hash, block_header,
        [query](::taraxa::EthBlockNumber block_number) {
          return query.transaction_count_by_block_number(block_number);
        },
        [query](::taraxa::EthBlockNumber block_number, uint64_t index) {
          return query.transaction_by_block_number_and_index(block_number, index);
        },
        [query](const ::taraxa::trx_hash_t& transaction_hash) {
          return query.transaction_receipt_by_hash(transaction_hash);
        }));
  }
#endif

  std::optional<::taraxa::EthBlockNumber> block_number;
  if (number) {
    block_number = number->get<int>();
    if (const auto last_block_number = block_reader_.latest_block_number(); last_block_number < block_number) {
      return nullptr;
    }
  }
  if (hash) {
    block_number = block_reader_.block_number_by_hash(dev::h256(hash->get<std::string>()));
    if (!block_number) {
      return nullptr;
    }
  }
  auto block_header = block_reader_.block_header(block_number);
  if (!block_header) {
    return nullptr;
  }

  // Special case for genesis
  if (block_number == 0) [[unlikely]] {
    return std::make_shared<object::Block>(std::make_shared<Block>(
        account_reader_, block_transaction_reader_, get_block_by_num_, ::taraxa::blk_hash_t(), block_header));
  }

  auto pbft_block_hash = block_reader_.pbft_hash_by_period(block_header->number);
  if (!pbft_block_hash) {
    // shouldn't be possible
    return nullptr;
  }
  return std::make_shared<object::Block>(std::make_shared<Block>(account_reader_, block_transaction_reader_,
                                                                 get_block_by_num_, *pbft_block_hash, block_header));
}

std::vector<std::shared_ptr<object::Block>> Query::getBlocks(response::Value&& fromArg,
                                                             std::optional<response::Value>&& toArg) const {
  std::vector<std::shared_ptr<object::Block>> blocks;

  int start_block_num = fromArg.get<int>();
  int end_block_num = toArg ? toArg->get<int>() : (start_block_num + Query::kMaxPropagationLimit);

  // Incase of reverse order of blocks
  if (start_block_num > end_block_num) {
    auto tmp = start_block_num;
    start_block_num = end_block_num;
    end_block_num = tmp;
  }

  if (end_block_num - start_block_num > static_cast<int>(Query::kMaxPropagationLimit)) {
    end_block_num = start_block_num + Query::kMaxPropagationLimit;
  }

  int last_block_number = 0;
#ifdef RUSTAXA_ENABLE
  if (hasConsensusQueryReader(consensus_query_reader_)) {
    last_block_number = static_cast<int>(consensus_query_reader_.final_chain_last_block_number());
  } else {
    last_block_number = static_cast<int>(block_reader_.latest_block_number());
  }
#else
  last_block_number = static_cast<int>(block_reader_.latest_block_number());
#endif
  if (start_block_num > last_block_number) {
    return blocks;
  } else if (end_block_num > last_block_number) {
    end_block_num = last_block_number;
  }

  blocks.reserve(end_block_num - start_block_num);

  for (int block_num = start_block_num; block_num <= end_block_num; block_num++) {
    blocks.emplace_back(getBlock(response::Value(block_num), std::nullopt));
  }

  return blocks;
}

std::shared_ptr<object::Transaction> Query::getTransaction(response::Value&& hashArg) const {
#ifdef RUSTAXA_ENABLE
  if (hasConsensusQueryReader(consensus_query_reader_)) {
    const auto transaction_hash = ::taraxa::trx_hash_t(hashArg.get<std::string>());
    auto transaction_view = consensus_query_reader_.transaction_by_hash(transaction_hash);
    auto transaction = materializeTransactionView(transaction_view);
    if (transaction) {
      auto receipt_view = consensus_query_reader_.transaction_receipt_by_hash(transaction_hash);
      return std::make_shared<object::Transaction>(
          std::make_shared<Transaction>(TransactionReceiptReader{}, account_reader_, get_block_by_num_,
                                        std::move(transaction), transaction_view, receipt_view));
    }
    return nullptr;
  }
#endif
  auto legacy_transaction = transaction_reader_.transaction_by_hash(::taraxa::trx_hash_t(hashArg.get<std::string>()));
  if (legacy_transaction) {
    return std::make_shared<object::Transaction>(std::make_shared<Transaction>(
        transaction_receipt_reader_, account_reader_, get_block_by_num_, std::move(legacy_transaction)));
  }
  return nullptr;
}

std::shared_ptr<object::Account> Query::getAccount(response::Value&& addressArg,
                                                   std::optional<response::Value>&& blockArg) const {
  const auto address = ::taraxa::addr_t(addressArg.get<std::string>());
  if (blockArg) {
    return std::make_shared<object::Account>(std::make_shared<Account>(account_reader_, address, blockArg->get<int>()));
  } else {
    return std::make_shared<object::Account>(std::make_shared<Account>(account_reader_, address));
  }
}

response::Value Query::getGasPrice() const { return response::Value(dev::toJS(gas_price_reader_.bid())); }

std::shared_ptr<object::SyncState> Query::getSyncing() const {
#ifdef RUSTAXA_ENABLE
  if (hasConsensusQueryReader(consensus_query_reader_)) {
    SyncStateReader reader = sync_state_reader_;
    reader.current_block = [query = consensus_query_reader_]() { return query.final_chain_last_block_number(); };
    return std::make_shared<object::SyncState>(std::make_shared<SyncState>(std::move(reader)));
  }
#endif
  return std::make_shared<object::SyncState>(std::make_shared<SyncState>(sync_state_reader_));
}

response::Value Query::getChainID() const { return response::Value(dev::toJS(kChainId)); }

std::shared_ptr<object::DagBlock> Query::getDagBlock(std::optional<response::Value>&& hashArg) const {
#ifdef RUSTAXA_ENABLE
  if (hasConsensusQueryReader(consensus_query_reader_)) {
    const auto query = consensus_query_reader_;
    auto transaction_query = [query](const ::taraxa::trx_hash_t& transaction_hash) {
      return query.transaction_by_hash(transaction_hash);
    };
    auto receipt_query = [query](const ::taraxa::trx_hash_t& transaction_hash) {
      return query.transaction_receipt_by_hash(transaction_hash);
    };
    if (hashArg) {
      if (const auto hash = ::taraxa::blk_hash_t(hashArg->get<response::StringType>());
          hash != ::taraxa::kNullBlockHash) {
        auto rust_dag_block = query.dag_block_by_hash(hash);
        if (rust_dag_block.found) {
          return std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
              std::move(rust_dag_block), account_reader_, get_block_by_num_, transaction_query, receipt_query));
        }
      }
    } else {
      const auto status = query.status();
      auto rust_dag_blocks = query.dag_blocks_by_level(status.latest_dag_level, 1);
      for (auto& rust_dag_block : rust_dag_blocks) {
        return std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
            std::move(rust_dag_block), account_reader_, get_block_by_num_, transaction_query, receipt_query));
      }
    }
    return nullptr;
  }
#endif
  std::shared_ptr<::taraxa::DagBlock> taraxa_dag_block = nullptr;

  if (hashArg) {
    if (const auto hash = ::taraxa::blk_hash_t(hashArg->get<response::StringType>());
        hash != ::taraxa::kNullBlockHash) {
      taraxa_dag_block = dag_block_reader_.block_by_hash(hash);
    }
  } else {
    auto dag_blocks = dag_block_reader_.blocks_by_level(dag_block_reader_.latest_level());

    if (dag_blocks.size() > 0) {
      taraxa_dag_block = dag_blocks.front();
    }
  }
  if (taraxa_dag_block) {
    return std::make_shared<object::DagBlock>(
        std::make_shared<DagBlock>(account_reader_, dag_block_transaction_reader_, dag_block_period_reader_,
                                   std::move(taraxa_dag_block), get_block_by_num_));
  }
  return nullptr;
}

std::vector<std::shared_ptr<object::DagBlock>> Query::getPeriodDagBlocks(
    std::optional<response::Value>&& periodArg) const {
  std::vector<std::shared_ptr<object::DagBlock>> blocks;
  uint64_t period;
  if (periodArg) {
    period = periodArg->get<int>();
  } else {
    period = dag_block_reader_.latest_finalized_period();
  }
#ifdef RUSTAXA_ENABLE
  if (hasConsensusQueryReader(consensus_query_reader_)) {
    const auto query = consensus_query_reader_;
    auto rust_dag_blocks = query.finalized_dag_blocks_by_period(period);
    if (rust_dag_blocks.size()) {
      blocks.reserve(rust_dag_blocks.size());
      for (auto& block : rust_dag_blocks) {
        blocks.emplace_back(std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
            std::move(block), account_reader_, get_block_by_num_,
            [query](const ::taraxa::trx_hash_t& transaction_hash) {
              return query.transaction_by_hash(transaction_hash);
            },
            [query](const ::taraxa::trx_hash_t& transaction_hash) {
              return query.transaction_receipt_by_hash(transaction_hash);
            })));
      }
    }
    return blocks;
  }
#endif
  auto dag_blocks = dag_block_reader_.finalized_blocks_by_period(period);
  if (dag_blocks.size()) {
    blocks.reserve(dag_blocks.size());
    for (auto block : dag_blocks) {
      blocks.emplace_back(std::make_shared<object::DagBlock>(
          std::make_shared<DagBlock>(account_reader_, dag_block_transaction_reader_, dag_block_period_reader_,
                                     std::move(block), get_block_by_num_)));
    }
  }
  return blocks;
}

std::vector<std::shared_ptr<object::DagBlock>> Query::getDagBlocks(std::optional<response::Value>&& dagLevelArg,
                                                                   std::optional<int>&& countArg,
                                                                   std::optional<bool>&& reverseArg) const {
#ifdef RUSTAXA_ENABLE
  if (hasConsensusQueryReader(consensus_query_reader_)) {
    std::vector<std::shared_ptr<object::DagBlock>> rust_dag_blocks_result;
    const auto query = consensus_query_reader_;
    const auto status = query.status();
    const auto rust_max_dag_level = status.latest_dag_level;
    ::taraxa::level_t rust_act_dag_level = rust_max_dag_level;

    if (dagLevelArg) {
      rust_act_dag_level = dagLevelArg->get<int>();
      if (rust_act_dag_level < 0 || static_cast<uint64_t>(rust_act_dag_level) > rust_max_dag_level) {
        return rust_dag_blocks_result;
      }
    }

    auto addRustDagBlocks = [account_reader = account_reader_, get_block_by_num = get_block_by_num_, query](
                                auto& rust_dag_blocks, auto& result_dag_blocks) -> size_t {
      const auto added = rust_dag_blocks.size();
      for (auto& dag_block : rust_dag_blocks) {
        result_dag_blocks.emplace_back(std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
            std::move(dag_block), account_reader, get_block_by_num,
            [query](const ::taraxa::trx_hash_t& transaction_hash) {
              return query.transaction_by_hash(transaction_hash);
            },
            [query](const ::taraxa::trx_hash_t& transaction_hash) {
              return query.transaction_receipt_by_hash(transaction_hash);
            })));
      }
      return added;
    };

    auto rust_dag_blocks = query.dag_blocks_by_level(rust_act_dag_level, 1);
    auto rust_act_count = addRustDagBlocks(rust_dag_blocks, rust_dag_blocks_result);

    if (!countArg) {
      return rust_dag_blocks_result;
    }

    auto count = std::min(static_cast<size_t>(countArg.value()), Query::kMaxPropagationLimit);
    bool reverse_flag = reverseArg ? reverseArg.value() : false;

    while (rust_act_count < count && static_cast<uint64_t>(rust_act_dag_level) <= rust_max_dag_level) {
      if (!reverse_flag) {
        rust_act_dag_level++;
      } else if (rust_act_dag_level > 0) {
        rust_act_dag_level--;
      } else {
        return rust_dag_blocks_result;
      }

      auto next_rust_dag_blocks = query.dag_blocks_by_level(rust_act_dag_level, 1);
      rust_act_count += addRustDagBlocks(next_rust_dag_blocks, rust_dag_blocks_result);
    }

    return rust_dag_blocks_result;
  }
#endif
  std::vector<std::shared_ptr<object::DagBlock>> dag_blocks_result;
  const auto max_dag_level = dag_block_reader_.latest_level();
  ::taraxa::level_t act_dag_level = max_dag_level;

  if (dagLevelArg) {
    act_dag_level = dagLevelArg->get<int>();
    if (act_dag_level < 0 || act_dag_level > max_dag_level) {
      return dag_blocks_result;
    }
  }

  auto addDagBlocks = [account_reader = account_reader_, transaction_reader = dag_block_transaction_reader_,
                       period_reader = dag_block_period_reader_, get_block_by_num = get_block_by_num_](
                          auto taraxa_dag_blocks, auto& result_dag_blocks) -> size_t {
    for (auto& dag_block : taraxa_dag_blocks) {
      result_dag_blocks.emplace_back(std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
          account_reader, transaction_reader, period_reader, std::move(dag_block), get_block_by_num)));
    }

    return taraxa_dag_blocks.size();
  };

  auto act_count = addDagBlocks(dag_block_reader_.blocks_by_level(act_dag_level), dag_blocks_result);

  if (!countArg) {
    return dag_blocks_result;
  }

  auto count = std::min(static_cast<size_t>(countArg.value()), Query::kMaxPropagationLimit);
  bool reverse_flag = reverseArg ? reverseArg.value() : false;

  while (act_count < count && act_dag_level <= max_dag_level) {
    if (!reverse_flag) {
      act_dag_level++;
    } else if (act_dag_level > 0) {
      act_dag_level--;
    } else {
      return dag_blocks_result;
    }

    act_count += addDagBlocks(dag_block_reader_.blocks_by_level(act_dag_level), dag_blocks_result);
  }

  return dag_blocks_result;
}

std::shared_ptr<object::CurrentState> Query::getNodeState() const {
#ifdef RUSTAXA_ENABLE
  if (hasConsensusQueryReader(consensus_query_reader_)) {
    CurrentStateReader reader;
    reader.final_block = [query = consensus_query_reader_]() { return query.status().final_block_number; };
    reader.dag_block_level = [query = consensus_query_reader_]() { return query.status().latest_dag_level; };
    reader.dag_block_period = [query = consensus_query_reader_]() { return query.status().latest_dag_period; };
    return std::make_shared<object::CurrentState>(std::make_shared<CurrentState>(std::move(reader)));
  }
#endif
  return std::make_shared<object::CurrentState>(std::make_shared<CurrentState>(current_state_reader_));
}

}  // namespace graphql::taraxa
