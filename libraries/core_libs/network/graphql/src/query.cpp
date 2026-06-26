#include "graphql/query.hpp"

#include <libdevcore/CommonJS.h>

#include "graphql/account.hpp"
#include "graphql/block.hpp"
#include "graphql/log.hpp"
#include "graphql/sync_state.hpp"
#include "graphql/transaction.hpp"
#include "graphql/types/current_state.hpp"
#include "graphql/types/dag_block.hpp"

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

using namespace std::literals;

namespace graphql::taraxa {

#ifdef RUSTAXA_ENABLE
namespace {
dev::h256 hashFromBridge(const std::array<uint8_t, 32>& hash) {
  return dev::h256(hash.data(), dev::h256::ConstructFromPointer);
}
}  // namespace
#endif

Query::Query(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
             std::shared_ptr<::taraxa::DagManager> dag_manager, std::shared_ptr<::taraxa::PbftManager> pbft_manager,
             std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
             std::shared_ptr<::taraxa::DbStorage> db,  // RUSTAXA_QUERY_COMPAT_READ: GraphQL query storage owner.
             std::shared_ptr<::taraxa::GasPricer> gas_pricer, std::weak_ptr<::taraxa::Network> network,
             uint64_t chain_id) noexcept
    : final_chain_(std::move(final_chain)),
      dag_manager_(std::move(dag_manager)),
      pbft_manager_(std::move(pbft_manager)),
      transaction_manager_(std::move(transaction_manager)),
      db_(std::move(db)),  // RUSTAXA_QUERY_COMPAT_READ: GraphQL query compatibility storage owner.
      gas_pricer_(std::move(gas_pricer)),
      network_(std::move(network)),
      kChainId(chain_id) {
  get_block_by_num_ = [&](::taraxa::EthBlockNumber num) {
    return getBlock(response::Value(static_cast<int>(num)), std::nullopt);
  };
}

std::shared_ptr<object::Block> Query::getBlock(std::optional<response::Value>&& number,
                                               std::optional<response::Value>&& hash) const {
  std::optional<::taraxa::EthBlockNumber> block_number;
  if (number) {
    block_number = number->get<int>();
    if (const auto last_block_number = final_chain_->lastBlockNumber(); last_block_number < block_number) {
      return nullptr;
    }
  }
  if (hash) {
    block_number = final_chain_->blockNumber(dev::h256(hash->get<std::string>()));
    if (!block_number) {
      return nullptr;
    }
  }
  auto block_header = final_chain_->blockHeader(block_number);
  if (!block_header) {
    return nullptr;
  }

  // Special case for genesis
  if (block_number == 0) [[unlikely]] {
    return std::make_shared<object::Block>(std::make_shared<Block>(
        final_chain_, transaction_manager_, get_block_by_num_, ::taraxa::blk_hash_t(), block_header));
  }

#ifdef RUSTAXA_ENABLE
  const auto query_api = rustaxa::create_consensus_query_api(db_->rustStorage());
  const auto pbft_block_hash = query_api->consensus_query_pbft_block_hash_by_period(block_header->number);
  if (!pbft_block_hash.found) {
    // shouldn't be possible
    return nullptr;
  }
  return std::make_shared<object::Block>(std::make_shared<Block>(final_chain_, transaction_manager_, get_block_by_num_,
                                                                 hashFromBridge(pbft_block_hash.hash), block_header));
#endif

  auto pbft_block = db_->getPbftBlock(block_header->number);  // RUSTAXA_QUERY_COMPAT_READ
  if (!pbft_block) {
    // shouldn't be possible
    return nullptr;
  }
  return std::make_shared<object::Block>(std::make_shared<Block>(final_chain_, transaction_manager_, get_block_by_num_,
                                                                 pbft_block->getBlockHash(), block_header));
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

  const int last_block_number = final_chain_->lastBlockNumber();
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
  if (auto transaction = transaction_manager_->getTransaction(::taraxa::trx_hash_t(hashArg.get<std::string>()))) {
    return std::make_shared<object::Transaction>(
        std::make_shared<Transaction>(final_chain_, transaction_manager_, get_block_by_num_, std::move(transaction)));
  }
  return nullptr;
}

std::shared_ptr<object::Account> Query::getAccount(response::Value&& addressArg,
                                                   std::optional<response::Value>&& blockArg) const {
  const auto address = ::taraxa::addr_t(addressArg.get<std::string>());
  if (blockArg) {
    return std::make_shared<object::Account>(std::make_shared<Account>(final_chain_, address, blockArg->get<int>()));
  } else {
    return std::make_shared<object::Account>(std::make_shared<Account>(final_chain_, address));
  }
}

response::Value Query::getGasPrice() const { return response::Value(dev::toJS(gas_pricer_->bid())); }

std::shared_ptr<object::SyncState> Query::getSyncing() const {
  return std::make_shared<object::SyncState>(std::make_shared<SyncState>(final_chain_, network_));
}

response::Value Query::getChainID() const { return response::Value(dev::toJS(kChainId)); }

std::shared_ptr<object::DagBlock> Query::getDagBlock(std::optional<response::Value>&& hashArg) const {
#ifdef RUSTAXA_ENABLE
  const auto dag_queries = rustaxa::create_dag_storage_queries(db_->rustStorage());
  if (hashArg) {
    if (const auto hash = ::taraxa::blk_hash_t(hashArg->get<response::StringType>());
        hash != ::taraxa::kNullBlockHash) {
      auto rust_dag_block = dag_queries->get_dag_block_public_view(hash.asArray());
      if (rust_dag_block.found) {
        return std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
            std::move(rust_dag_block), final_chain_, pbft_manager_, transaction_manager_, get_block_by_num_));
      }
    }
  } else {
    auto rust_dag_blocks = dag_queries->get_dag_block_views_at_level(dag_manager_->getMaxLevel(), 1);
    for (auto& rust_dag_block : rust_dag_blocks) {
      return std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
          std::move(rust_dag_block), final_chain_, pbft_manager_, transaction_manager_, get_block_by_num_));
    }
  }
  return nullptr;
#endif
  std::shared_ptr<::taraxa::DagBlock> taraxa_dag_block = nullptr;

  if (hashArg) {
    if (const auto hash = ::taraxa::blk_hash_t(hashArg->get<response::StringType>());
        hash != ::taraxa::kNullBlockHash) {
      taraxa_dag_block = dag_manager_->getDagBlock(hash);
    }
  } else {
    auto dag_blocks = db_->getDagBlocksAtLevel(dag_manager_->getMaxLevel(), 1);  // RUSTAXA_QUERY_COMPAT_READ

    if (dag_blocks.size() > 0) {
      taraxa_dag_block = dag_blocks.front();
    }
  }
  if (taraxa_dag_block) {
    return std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
        std::move(taraxa_dag_block), final_chain_, pbft_manager_, transaction_manager_, get_block_by_num_));
  }
  return nullptr;
}

std::vector<std::shared_ptr<object::DagBlock>> Query::getPeriodDagBlocks(
    std::optional<response::Value>&& periodArg) const {
  std::vector<std::shared_ptr<object::DagBlock>> blocks;
  uint32_t period;
  if (periodArg) {
    period = periodArg->get<int>();
  } else {
    period = final_chain_->lastBlockNumber();
  }
#ifdef RUSTAXA_ENABLE
  const auto period_queries = rustaxa::create_period_storage_queries(db_->rustStorage());
  auto rust_dag_blocks = period_queries->get_period_dag_block_views(period);
  if (rust_dag_blocks.size()) {
    blocks.reserve(rust_dag_blocks.size());
    for (auto& block : rust_dag_blocks) {
      blocks.emplace_back(std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
          std::move(block), final_chain_, pbft_manager_, transaction_manager_, get_block_by_num_)));
    }
  }
  return blocks;
#endif
  auto dag_blocks = db_->getFinalizedDagBlockByPeriod(period);  // RUSTAXA_QUERY_COMPAT_READ
  if (dag_blocks.size()) {
    blocks.reserve(dag_blocks.size());
    for (auto block : dag_blocks) {
      blocks.emplace_back(std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
          std::move(block), final_chain_, pbft_manager_, transaction_manager_, get_block_by_num_)));
    }
  }
  return blocks;
}

std::vector<std::shared_ptr<object::DagBlock>> Query::getDagBlocks(std::optional<response::Value>&& dagLevelArg,
                                                                   std::optional<int>&& countArg,
                                                                   std::optional<bool>&& reverseArg) const {
#ifdef RUSTAXA_ENABLE
  {
    std::vector<std::shared_ptr<object::DagBlock>> rust_dag_blocks_result;
    ::taraxa::level_t rust_act_dag_level = dag_manager_->getMaxLevel();

    if (dagLevelArg) {
      rust_act_dag_level = dagLevelArg->get<int>();
      if (rust_act_dag_level < 0 || rust_act_dag_level > dag_manager_->getMaxLevel()) {
        return rust_dag_blocks_result;
      }
    }

    const auto dag_queries = rustaxa::create_dag_storage_queries(db_->rustStorage());
    auto addRustDagBlocks = [final_chain = final_chain_, pbft_manager = pbft_manager_,
                             transaction_manager = transaction_manager_, get_block_by_num = get_block_by_num_](
                                auto& rust_dag_blocks, auto& result_dag_blocks) -> size_t {
      const auto added = rust_dag_blocks.size();
      for (auto& dag_block : rust_dag_blocks) {
        result_dag_blocks.emplace_back(std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
            std::move(dag_block), final_chain, pbft_manager, transaction_manager, get_block_by_num)));
      }
      return added;
    };

    auto rust_dag_blocks = dag_queries->get_dag_block_views_at_level(rust_act_dag_level, 1);
    auto rust_act_count = addRustDagBlocks(rust_dag_blocks, rust_dag_blocks_result);

    if (!countArg) {
      return rust_dag_blocks_result;
    }

    auto count = std::min(static_cast<size_t>(countArg.value()), Query::kMaxPropagationLimit);
    bool reverse_flag = reverseArg ? reverseArg.value() : false;

    while (rust_act_count < count && rust_act_dag_level <= dag_manager_->getMaxLevel()) {
      if (!reverse_flag) {
        rust_act_dag_level++;
      } else if (rust_act_dag_level > 0) {
        rust_act_dag_level--;
      } else {
        return rust_dag_blocks_result;
      }

      auto next_rust_dag_blocks = dag_queries->get_dag_block_views_at_level(rust_act_dag_level, 1);
      rust_act_count += addRustDagBlocks(next_rust_dag_blocks, rust_dag_blocks_result);
    }

    return rust_dag_blocks_result;
  }
#endif
  std::vector<std::shared_ptr<object::DagBlock>> dag_blocks_result;
  ::taraxa::level_t act_dag_level = dag_manager_->getMaxLevel();

  if (dagLevelArg) {
    act_dag_level = dagLevelArg->get<int>();
    if (act_dag_level < 0 || act_dag_level > dag_manager_->getMaxLevel()) {
      return dag_blocks_result;
    }
  }

  auto addDagBlocks = [final_chain = final_chain_, pbft_manager = pbft_manager_,
                       transaction_manager = transaction_manager_, get_block_by_num = get_block_by_num_](
                          auto taraxa_dag_blocks, auto& result_dag_blocks) -> size_t {
    for (auto& dag_block : taraxa_dag_blocks) {
      result_dag_blocks.emplace_back(std::make_shared<object::DagBlock>(std::make_shared<DagBlock>(
          std::move(dag_block), final_chain, pbft_manager, transaction_manager, get_block_by_num)));
    }

    return taraxa_dag_blocks.size();
  };

  auto act_count =
      addDagBlocks(db_->getDagBlocksAtLevel(act_dag_level, 1), dag_blocks_result);  // RUSTAXA_QUERY_COMPAT_READ

  if (!countArg) {
    return dag_blocks_result;
  }

  auto count = std::min(static_cast<size_t>(countArg.value()), Query::kMaxPropagationLimit);
  bool reverse_flag = reverseArg ? reverseArg.value() : false;

  while (act_count < count && act_dag_level <= dag_manager_->getMaxLevel()) {
    if (!reverse_flag) {
      act_dag_level++;
    } else if (act_dag_level > 0) {
      act_dag_level--;
    } else {
      return dag_blocks_result;
    }

    act_count +=
        addDagBlocks(db_->getDagBlocksAtLevel(act_dag_level, 1), dag_blocks_result);  // RUSTAXA_QUERY_COMPAT_READ
  }

  return dag_blocks_result;
}

std::shared_ptr<object::CurrentState> Query::getNodeState() const {
  return std::make_shared<object::CurrentState>(std::make_shared<CurrentState>(final_chain_, dag_manager_));
}

}  // namespace graphql::taraxa
