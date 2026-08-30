#include "Debug.h"

#include <libdevcore/CommonData.h>
#include <libdevcore/CommonJS.h>

#include "common/jsoncpp.hpp"
#include "common/rpc_utils.hpp"
#include "final_chain/final_chain.hpp"
#include "final_chain/state_api_data.hpp"
#include "network/rpc/eth/data.hpp"
#include "transaction/system_transaction.hpp"
#include "transaction/transaction.hpp"
#include "vote/pbft_vote.hpp"
#ifndef RUSTAXA_ENABLE
#include "vote_manager/vote_manager.hpp"
#endif

#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif

using namespace std;
using namespace dev;
using namespace jsonrpc;
using namespace taraxa;

namespace taraxa::net {

namespace {
DebugDposReader makeDebugDposReader(std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                    ,
                                    ConsensusQueryApiPtr consensus_query_api
#endif
) {
  DebugDposReader reader;
  reader.eligible_total_vote_count = [app
#ifdef RUSTAXA_ENABLE
                                      ,
                                      consensus_query_api
#endif
  ](EthBlockNumber block_number) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_DPOS_READER_APP_EXPIRED");
    }
#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      throw std::runtime_error("DEBUG_DPOS_QUERY_UNAVAILABLE");
    }
    return (*consensus_query_api)->consensus_query_final_chain_dpos_eligible_total_vote_count(block_number);
#else
    return node->getFinalChain()->dposEligibleTotalVoteCount(block_number);
#endif
  };
  reader.validators_total_stakes = [app
#ifdef RUSTAXA_ENABLE
                                    ,
                                    consensus_query_api
#endif
  ](EthBlockNumber block_number) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_DPOS_READER_APP_EXPIRED");
    }
#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      throw std::runtime_error("DEBUG_DPOS_QUERY_UNAVAILABLE");
    }
    const auto rust_stakes =
        (*consensus_query_api)->consensus_query_final_chain_dpos_validators_total_stakes(block_number);
    std::vector<state_api::ValidatorStake> stakes;
    stakes.reserve(rust_stakes.size());
    for (const auto& stake : rust_stakes) {
      stakes.push_back(state_api::ValidatorStake{
          addr_t(stake.address.data(), addr_t::ConstructFromPointer),
          dev::fromBigEndian<dev::u256>(dev::bytes(stake.stake.begin(), stake.stake.end())),
      });
    }
    return stakes;
#else
    return node->getFinalChain()->dposValidatorsTotalStakes(block_number);
#endif
  };
  reader.total_amount_delegated = [app
#ifdef RUSTAXA_ENABLE
                                   ,
                                   consensus_query_api
#endif
  ](EthBlockNumber block_number) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_DPOS_READER_APP_EXPIRED");
    }
#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      throw std::runtime_error("DEBUG_DPOS_QUERY_UNAVAILABLE");
    }
    const auto delegated =
        (*consensus_query_api)->consensus_query_final_chain_dpos_total_amount_delegated(block_number);
    return dev::fromBigEndian<dev::u256>(dev::bytes(delegated.begin(), delegated.end()));
#else
    return node->getFinalChain()->dposTotalAmountDelegated(block_number);
#endif
  };
  return reader;
}

void fillMissingDebugDposReaderCallbacks(DebugDposReader& reader, std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                         ,
                                         ConsensusQueryApiPtr consensus_query_api
#endif
) {
  auto defaults = makeDebugDposReader(std::move(app)
#ifdef RUSTAXA_ENABLE
                                          ,
                                      std::move(consensus_query_api)
#endif
  );
  if (!reader.eligible_total_vote_count) {
    reader.eligible_total_vote_count = std::move(defaults.eligible_total_vote_count);
  }
  if (!reader.validators_total_stakes) {
    reader.validators_total_stakes = std::move(defaults.validators_total_stakes);
  }
  if (!reader.total_amount_delegated) {
    reader.total_amount_delegated = std::move(defaults.total_amount_delegated);
  }
}

DebugTraceReader makeDebugTraceReader(std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                      ,
                                      ConsensusQueryApiPtr consensus_query_api
#endif
) {
  DebugTraceReader reader;
  reader.trace = [app](std::vector<state_api::EVMTransaction> state_trxs, std::vector<state_api::EVMTransaction> trxs,
                       EthBlockNumber block_number, std::optional<state_api::Tracing> tracing) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_TRACE_READER_APP_EXPIRED");
    }
    if (tracing) {
      return node->getFinalChain()->trace(std::move(state_trxs), std::move(trxs), block_number, std::move(*tracing));
    }
    return node->getFinalChain()->trace(std::move(state_trxs), std::move(trxs), block_number);
  };
  reader.account_at = [app](const Address& address, EthBlockNumber block_number) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_TRACE_READER_APP_EXPIRED");
    }
    return node->getFinalChain()->getAccount(address, block_number);
  };
  reader.latest_finalized_block_number = [app
#ifdef RUSTAXA_ENABLE
                                          ,
                                          consensus_query_api
#endif
  ] {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_TRACE_READER_APP_EXPIRED");
    }
#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      throw std::runtime_error("DEBUG_TRACE_QUERY_UNAVAILABLE");
    }
    return (*consensus_query_api)->consensus_query_final_chain_last_block_number();
#else
    return node->getFinalChain()->lastBlockNumber();
#endif
  };
  return reader;
}

void fillMissingDebugTraceReaderCallbacks(DebugTraceReader& reader, std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                          ,
                                          ConsensusQueryApiPtr consensus_query_api
#endif
) {
  auto defaults = makeDebugTraceReader(std::move(app)
#ifdef RUSTAXA_ENABLE
                                           ,
                                       std::move(consensus_query_api)
#endif
  );
  if (!reader.trace) {
    reader.trace = std::move(defaults.trace);
  }
  if (!reader.account_at) {
    reader.account_at = std::move(defaults.account_at);
  }
  if (!reader.latest_finalized_block_number) {
    reader.latest_finalized_block_number = std::move(defaults.latest_finalized_block_number);
  }
}

DebugPreviousBlockCertVotesReader makeDebugPreviousBlockCertVotesReader(std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                                        ,
                                                                        ConsensusQueryApiPtr consensus_query_api
#endif
) {
  DebugPreviousBlockCertVotesReader reader;
  reader.cert_votes_by_period = [app
#ifdef RUSTAXA_ENABLE
                                 ,
                                 consensus_query_api
#endif
  ](uint64_t period) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_PREVIOUS_BLOCK_CERT_VOTES_READER_APP_EXPIRED");
    }

    DebugPreviousBlockCertVotesView view;
#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      throw std::runtime_error("DEBUG_PREVIOUS_BLOCK_CERT_VOTES_QUERY_UNAVAILABLE");
    }
    const auto cert_vote_view =
        (*consensus_query_api)->consensus_query_pbft_previous_block_cert_votes_by_period(period);
    if (!cert_vote_view.found) {
      return view;
    }

    view.found = true;
    view.total_votes_count =
        (*consensus_query_api)
            ->consensus_query_final_chain_dpos_eligible_total_vote_count(cert_vote_view.certified_period - 1);
    view.round = cert_vote_view.round;
    view.votes.reserve(cert_vote_view.votes.size());
    for (const auto& vote_view : cert_vote_view.votes) {
      view.votes.emplace_back(std::make_shared<PbftVote>(bytes(vote_view.vote_rlp.begin(), vote_view.vote_rlp.end())));
    }
    return view;
#else
    auto vote_manager = node->getVoteManager();
    auto votes = node->getDB()->getPeriodCertVotes(period);
    if (votes.empty()) {
      return view;
    }

    const auto& front_vote = votes.front();
    view.found = true;
    view.total_votes_count = node->getFinalChain()->dposEligibleTotalVoteCount(front_vote->getPeriod() - 1);
    view.round = front_vote->getRound();
    view.votes.reserve(votes.size());
    for (auto& vote : votes) {
      vote_manager->validateVote(vote);
      view.votes.emplace_back(std::move(vote));
    }
    return view;
#endif
  };
  return reader;
}

void fillMissingDebugPreviousBlockCertVotesReaderCallbacks(DebugPreviousBlockCertVotesReader& reader,
                                                           std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                           ,
                                                           ConsensusQueryApiPtr consensus_query_api
#endif
) {
  auto defaults = makeDebugPreviousBlockCertVotesReader(std::move(app)
#ifdef RUSTAXA_ENABLE
                                                            ,
                                                        std::move(consensus_query_api)
#endif
  );
  if (!reader.cert_votes_by_period) {
    reader.cert_votes_by_period = std::move(defaults.cert_votes_by_period);
  }
}

void fillMissingDebugPeriodDagBlocksReaderCallbacks(DebugPeriodDagBlocksReader& reader,
                                                    std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                    ,
                                                    ConsensusQueryApiPtr consensus_query_api
#endif
);
void fillMissingDebugPeriodTransactionsReaderCallbacks(DebugPeriodTransactionsReader& reader,
                                                       std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                       ,
                                                       ConsensusQueryApiPtr consensus_query_api
#endif
);
void fillMissingDebugTraceReplayReaderCallbacks(DebugTraceReplayReader& reader, std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                ,
                                                ConsensusQueryApiPtr consensus_query_api
#endif
);
}  // namespace

Debug::Debug(std::shared_ptr<taraxa::AppBase> app, uint64_t gas_limit, DebugDposReader dpos_reader,
             DebugTraceReader trace_reader, DebugPreviousBlockCertVotesReader previous_cert_votes_reader,
             DebugPeriodDagBlocksReader period_dag_blocks_reader,
             DebugPeriodTransactionsReader period_transactions_reader, DebugTraceReplayReader trace_replay_reader
#ifdef RUSTAXA_ENABLE
             ,
             ConsensusQueryApiPtr consensus_query_api
#endif
             )
    : app_(app),
      dpos_reader_(std::move(dpos_reader)),
      trace_reader_(std::move(trace_reader)),
      previous_cert_votes_reader_(std::move(previous_cert_votes_reader)),
      period_dag_blocks_reader_(std::move(period_dag_blocks_reader)),
      period_transactions_reader_(std::move(period_transactions_reader)),
      trace_replay_reader_(std::move(trace_replay_reader)),
#ifdef RUSTAXA_ENABLE
      consensus_query_api_(std::move(consensus_query_api)),
#endif
      kGasLimit(gas_limit) {
  fillMissingDebugDposReaderCallbacks(dpos_reader_, app_
#ifdef RUSTAXA_ENABLE
                                      ,
                                      consensus_query_api_
#endif
  );
  fillMissingDebugTraceReaderCallbacks(trace_reader_, app_
#ifdef RUSTAXA_ENABLE
                                       ,
                                       consensus_query_api_
#endif
  );
  fillMissingDebugPreviousBlockCertVotesReaderCallbacks(previous_cert_votes_reader_, app_
#ifdef RUSTAXA_ENABLE
                                                        ,
                                                        consensus_query_api_
#endif
  );
  fillMissingDebugPeriodDagBlocksReaderCallbacks(period_dag_blocks_reader_, app_
#ifdef RUSTAXA_ENABLE
                                                 ,
                                                 consensus_query_api_
#endif
  );
  fillMissingDebugPeriodTransactionsReaderCallbacks(period_transactions_reader_, app_
#ifdef RUSTAXA_ENABLE
                                                    ,
                                                    consensus_query_api_
#endif
  );
  fillMissingDebugTraceReplayReaderCallbacks(trace_replay_reader_, app_
#ifdef RUSTAXA_ENABLE
                                             ,
                                             consensus_query_api_
#endif
  );
}

#ifdef RUSTAXA_ENABLE
namespace {
constexpr uint8_t kConsensusQueryTransactionSourceMissing = 0;
constexpr uint8_t kConsensusQueryTransactionSourcePending = 1;
constexpr uint8_t kConsensusQueryTransactionSourceFinalizedRegular = 2;
constexpr uint8_t kConsensusQueryTransactionSourceFinalizedSystem = 3;

dev::h256 hashFromBridge(const std::array<uint8_t, 32>& hash) {
  return dev::h256(hash.data(), dev::h256::ConstructFromPointer);
}

dev::Address addressFromBridge(const std::array<uint8_t, 20>& address) {
  return dev::Address(address.data(), dev::Address::ConstructFromPointer);
}

dev::bytes bytesFromBridge(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

std::shared_ptr<Transaction> materializeReceiptTransactionView(const rustaxa::TransactionReceiptPublicView& view) {
  if (!view.found) {
    return nullptr;
  }

  std::shared_ptr<Transaction> transaction;
  if (view.transaction_source == kConsensusQueryTransactionSourceFinalizedSystem) {
    transaction = std::make_shared<SystemTransaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.transaction_source == kConsensusQueryTransactionSourcePending ||
             view.transaction_source == kConsensusQueryTransactionSourceFinalizedRegular) {
    transaction = std::make_shared<Transaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.transaction_source != kConsensusQueryTransactionSourceMissing) {
    throw std::runtime_error("CONSENSUS_QUERY_DEBUG_RECEIPT_TRANSACTION_UNKNOWN_SOURCE");
  }

  if (transaction && transaction->getHash() != hashFromBridge(view.transaction_hash)) {
    throw std::runtime_error("CONSENSUS_QUERY_DEBUG_RECEIPT_TRANSACTION_HASH_MISMATCH");
  }
  return transaction;
}

std::shared_ptr<Transaction> materializeTransactionView(const rustaxa::TransactionPublicView& view) {
  if (!view.found) {
    return nullptr;
  }

  std::shared_ptr<Transaction> transaction;
  if (view.source == kConsensusQueryTransactionSourceFinalizedSystem) {
    transaction = std::make_shared<SystemTransaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.source == kConsensusQueryTransactionSourcePending ||
             view.source == kConsensusQueryTransactionSourceFinalizedRegular) {
    transaction = std::make_shared<Transaction>(bytesFromBridge(view.transaction_rlp));
  } else if (view.source != kConsensusQueryTransactionSourceMissing) {
    throw std::runtime_error("CONSENSUS_QUERY_DEBUG_TRANSACTION_UNKNOWN_SOURCE");
  }

  if (transaction && transaction->getHash() != hashFromBridge(view.hash)) {
    throw std::runtime_error("CONSENSUS_QUERY_DEBUG_TRANSACTION_HASH_MISMATCH");
  }
  return transaction;
}

template <typename QueryApi>
SharedTransactions materializeBlockTransactionsFromQuery(uint64_t block_number, const QueryApi& query_api) {
  SharedTransactions transactions;
  const auto transaction_count = query_api->consensus_query_transaction_count_by_block_number(block_number);
  transactions.reserve(transaction_count);
  for (uint64_t transaction_index = 0; transaction_index < transaction_count; ++transaction_index) {
    auto transaction = materializeTransactionView(
        query_api->consensus_query_transaction_by_block_number_and_index(block_number, transaction_index));
    if (!transaction) {
      throw std::runtime_error("CONSENSUS_QUERY_DEBUG_BLOCK_TRANSACTION_MISSING");
    }
    transactions.emplace_back(std::move(transaction));
  }
  return transactions;
}

rpc::eth::ExtendedTransactionLocation receiptLocationFromView(const rustaxa::TransactionReceiptPublicView& view) {
  if (!view.block_hash_found) {
    throw std::runtime_error("CONSENSUS_QUERY_DEBUG_RECEIPT_BLOCK_HASH_MISSING");
  }

  rpc::eth::ExtendedTransactionLocation location;
  location.period = view.block_number;
  location.position = view.transaction_index;
  location.is_system = view.is_system;
  location.blk_h = hashFromBridge(view.block_hash);
  location.trx_hash = hashFromBridge(view.transaction_hash);
  return location;
}

Json::Value dagBlockPublicViewToJson(const rustaxa::DagBlockPublicView& view, uint64_t period) {
  Json::Value json;
  json["pivot"] = dev::toJS(hashFromBridge(view.pivot));
  json["level"] = dev::toJS(view.level);
  json["tips"] = Json::Value(Json::arrayValue);
  for (const auto& tip : view.tips) {
    json["tips"].append(dev::toJS(hashFromBridge(tip.hash)));
  }
  json["transactions"] = Json::Value(Json::arrayValue);
  for (const auto& trx : view.transactions) {
    json["transactions"].append(dev::toJS(hashFromBridge(trx.hash)));
  }
  json["trx_estimations"] = dev::toJS(view.trx_estimations);
  json["sig"] = dev::toJS(bytesFromBridge(view.signature));
  json["hash"] = dev::toJS(hashFromBridge(view.hash));
  json["sender"] = dev::toJS(addressFromBridge(view.sender));
  json["timestamp"] = dev::toJS(view.timestamp);
  if (view.has_vdf) {
    Json::Value vdf;
    vdf["proof"] = dev::toJS(bytesFromBridge(view.vdf_proof));
    vdf["sol1"] = dev::toJS(dev::toHex(bytesFromBridge(view.vdf_sol1)));
    vdf["sol2"] = dev::toJS(dev::toHex(bytesFromBridge(view.vdf_sol2)));
    vdf["difficulty"] = dev::toJS(view.vdf_difficulty);
    json["vdf"] = std::move(vdf);
  }
  json["period"] = dev::toJS(period);
  return json;
}
}  // namespace
#endif

namespace {
DebugPeriodDagBlocksReader makeDebugPeriodDagBlocksReader(std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                          ,
                                                          ConsensusQueryApiPtr consensus_query_api
#endif
) {
  DebugPeriodDagBlocksReader reader;
  reader.finalized_dag_blocks_by_period = [app
#ifdef RUSTAXA_ENABLE
                                           ,
                                           consensus_query_api
#endif
  ](uint64_t period) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_PERIOD_DAG_BLOCKS_READER_APP_EXPIRED");
    }

#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      throw std::runtime_error("DEBUG_PERIOD_DAG_BLOCKS_QUERY_UNAVAILABLE");
    }
    const auto dag_views = (*consensus_query_api)->consensus_query_finalized_dag_blocks_by_period(period);
    Json::Value result(Json::arrayValue);
    for (const auto& dag_view : dag_views) {
      result.append(dagBlockPublicViewToJson(dag_view, period));
    }
    return result;
#else
    auto dags = node->getDB()->getFinalizedDagBlockByPeriod(period);
    return util::transformToJsonParallel(dags, [&period](const auto& dag, auto) {
      auto block_json = dag->getJson();
      block_json["period"] = toJS(period);
      return block_json;
    });
#endif
  };
  return reader;
}

void fillMissingDebugPeriodDagBlocksReaderCallbacks(DebugPeriodDagBlocksReader& reader,
                                                    std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                    ,
                                                    ConsensusQueryApiPtr consensus_query_api
#endif
) {
  auto defaults = makeDebugPeriodDagBlocksReader(std::move(app)
#ifdef RUSTAXA_ENABLE
                                                     ,
                                                 std::move(consensus_query_api)
#endif
  );
  if (!reader.finalized_dag_blocks_by_period) {
    reader.finalized_dag_blocks_by_period = std::move(defaults.finalized_dag_blocks_by_period);
  }
}

DebugPeriodTransactionsReader makeDebugPeriodTransactionsReader(std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                                ,
                                                                ConsensusQueryApiPtr consensus_query_api
#endif
) {
  DebugPeriodTransactionsReader reader;
  reader.transactions_with_receipts_by_period = [app
#ifdef RUSTAXA_ENABLE
                                                 ,
                                                 consensus_query_api
#endif
  ](uint64_t period) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_PERIOD_TRANSACTIONS_READER_APP_EXPIRED");
    }
    auto final_chain = node->getFinalChain();
#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      throw std::runtime_error("DEBUG_PERIOD_TRANSACTIONS_QUERY_UNAVAILABLE");
    }
    const auto receipt_views = (*consensus_query_api)->consensus_query_transaction_receipts_by_block_number(period);
    Json::Value result(Json::arrayValue);
    for (const auto& view : receipt_views) {
      auto trx = materializeReceiptTransactionView(view);
      if (!trx) {
        throw std::runtime_error("CONSENSUS_QUERY_DEBUG_RECEIPT_TRANSACTION_MISSING");
      }
      auto location = receiptLocationFromView(view);
      auto transaction = rpc::eth::LocalisedTransaction{trx, location};
      auto receipt_bytes = bytesFromBridge(view.receipt_rlp);
      auto receipt = rpc::eth::LocalisedTransactionReceipt{util::rlp_dec<TransactionReceipt>(dev::RLP(receipt_bytes)),
                                                           location, trx->getSender(), trx->getReceiver()};
      auto receipt_json = rpc::eth::toJson(receipt);
      receipt_json.removeMember("transactionHash");
      result.append(util::mergeJsons(rpc::eth::toJson(transaction), std::move(receipt_json)));
    }
    return result;
#else
    auto block_hash = final_chain->blockHash(period);
    auto trxs = node->getDB()->getPeriodTransactions(period);
    if (!trxs.has_value() || trxs->empty()) {
      return Json::Value(Json::arrayValue);
    }

    auto receipts = final_chain->blockReceipts(period);

    return util::transformToJsonParallel(*trxs, [&final_chain, &receipts, &block_hash, &period](const auto& trx,
                                                                                                auto index) {
      auto hash = trx->getHash();

      auto location = rpc::eth::ExtendedTransactionLocation{{TransactionLocation{period, index}, *block_hash}, hash};
      auto transaction = rpc::eth::LocalisedTransaction{trx, location};
      rpc::eth::LocalisedTransactionReceipt receipt;
      if (!receipts) {
        receipt = rpc::eth::LocalisedTransactionReceipt{final_chain->transactionReceipt(period, index, hash).value(),
                                                        location, trx->getSender(), trx->getReceiver()};
      } else {
        receipt =
            rpc::eth::LocalisedTransactionReceipt{receipts->at(index), location, trx->getSender(), trx->getReceiver()};
      }

      auto receipt_json = rpc::eth::toJson(receipt);
      receipt_json.removeMember("transactionHash");

      return util::mergeJsons(rpc::eth::toJson(transaction), std::move(receipt_json));
    });
#endif
  };
  return reader;
}

void fillMissingDebugPeriodTransactionsReaderCallbacks(DebugPeriodTransactionsReader& reader,
                                                       std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                       ,
                                                       ConsensusQueryApiPtr consensus_query_api
#endif
) {
  auto defaults = makeDebugPeriodTransactionsReader(std::move(app)
#ifdef RUSTAXA_ENABLE
                                                        ,
                                                    std::move(consensus_query_api)
#endif
  );
  if (!reader.transactions_with_receipts_by_period) {
    reader.transactions_with_receipts_by_period = std::move(defaults.transactions_with_receipts_by_period);
  }
}

DebugTraceReplayReader makeDebugTraceReplayReader(std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                  ,
                                                  ConsensusQueryApiPtr consensus_query_api
#endif
) {
  DebugTraceReplayReader reader;
  reader.transaction_with_state_by_hash = [app
#ifdef RUSTAXA_ENABLE
                                           ,
                                           consensus_query_api
#endif
  ](const trx_hash_t& transaction_hash) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_TRACE_REPLAY_READER_APP_EXPIRED");
    }

#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      throw std::runtime_error("DEBUG_TRACE_REPLAY_QUERY_UNAVAILABLE");
    }
    const auto target_view = (*consensus_query_api)->consensus_query_transaction_by_hash(transaction_hash.asArray());
    if (!target_view.found || !target_view.location_found) {
      throw std::runtime_error("Transaction not found");
    }
    auto block_transactions = materializeBlockTransactionsFromQuery(target_view.block_number, *consensus_query_api);
    if (target_view.transaction_index >= block_transactions.size()) {
      throw std::runtime_error("Transaction not found");
    }

    DebugTraceReplayTransactionView view;
    view.state_transactions =
        SharedTransactions(block_transactions.begin(), block_transactions.begin() + target_view.transaction_index);
    view.transaction = std::move(block_transactions[target_view.transaction_index]);
    view.period = target_view.block_number;
    return view;
#else
    auto final_chain = node->getFinalChain();
    auto loc = final_chain->transactionLocation(transaction_hash);
    if (!loc) {
      throw std::runtime_error("Transaction not found");
    }
    auto block_transactions = final_chain->transactions(loc->period);

    DebugTraceReplayTransactionView view;
    view.state_transactions =
        SharedTransactions(block_transactions.begin(), block_transactions.begin() + loc->position);
    view.transaction = std::move(block_transactions[loc->position]);
    view.period = loc->period;
    return view;
#endif
  };
  reader.transactions_by_block_number = [app
#ifdef RUSTAXA_ENABLE
                                         ,
                                         consensus_query_api
#endif
  ](uint64_t block_number) {
    auto node = app.lock();
    if (!node) {
      throw std::runtime_error("DEBUG_TRACE_REPLAY_READER_APP_EXPIRED");
    }

#ifdef RUSTAXA_ENABLE
    if (!consensus_query_api) {
      throw std::runtime_error("DEBUG_TRACE_REPLAY_QUERY_UNAVAILABLE");
    }
    return materializeBlockTransactionsFromQuery(block_number, *consensus_query_api);
#else
    auto legacy_transactions = node->getDB()->getPeriodTransactions(block_number);
    if (!legacy_transactions.has_value()) {
      return SharedTransactions{};
    }
    return *legacy_transactions;
#endif
  };
  return reader;
}

void fillMissingDebugTraceReplayReaderCallbacks(DebugTraceReplayReader& reader, std::weak_ptr<taraxa::AppBase> app
#ifdef RUSTAXA_ENABLE
                                                ,
                                                ConsensusQueryApiPtr consensus_query_api
#endif
) {
  auto defaults = makeDebugTraceReplayReader(std::move(app)
#ifdef RUSTAXA_ENABLE
                                                 ,
                                             std::move(consensus_query_api)
#endif
  );
  if (!reader.transaction_with_state_by_hash) {
    reader.transaction_with_state_by_hash = std::move(defaults.transaction_with_state_by_hash);
  }
  if (!reader.transactions_by_block_number) {
    reader.transactions_by_block_number = std::move(defaults.transactions_by_block_number);
  }
}
}  // namespace

Json::Value Debug::debug_traceCall(const Json::Value& call_params, const std::string& blk_num) {
  const auto block = parse_blk_num(blk_num);
  auto trx = to_eth_trx(call_params, block);
  return util::readJsonFromString(trace_reader_.trace({}, {std::move(trx)}, block, std::nullopt));
}

Json::Value Debug::trace_call(const Json::Value& call_params, const Json::Value& trace_params,
                              const std::string& blk_num) {
  const auto block = parse_blk_num(blk_num);
  auto params = parse_tracking_parms(trace_params);
  return util::readJsonFromString(trace_reader_.trace({}, {to_eth_trx(call_params, block)}, block, std::move(params)));
}

Json::Value Debug::debug_traceTransaction(const std::string& transaction_hash) {
  auto replay = trace_replay_reader_.transaction_with_state_by_hash(jsToFixed<32>(transaction_hash));
  return util::readJsonFromString(
      trace_reader_.trace({}, {to_eth_trx(std::move(replay.transaction))}, replay.period, std::nullopt));
}

Json::Value Debug::trace_replayTransaction(const std::string& transaction_hash, const Json::Value& trace_params) {
  auto params = parse_tracking_parms(trace_params);
  auto replay = trace_replay_reader_.transaction_with_state_by_hash(jsToFixed<32>(transaction_hash));
  return util::readJsonFromString(trace_reader_.trace(to_eth_trxs(replay.state_transactions),
                                                      {to_eth_trx(std::move(replay.transaction))}, replay.period,
                                                      std::move(params)));
}

bool only_transfers(const SharedTransactions& trxs) {
  return std::all_of(trxs.begin(), trxs.end(), [](const SharedTransaction& trx) {
    return trx->getReceiver().has_value() && trx->getData().empty() && trx->getGas() <= 22000;
  });
}

Json::Value Debug::trace_replayBlockTransactions(const std::string& block_num, const Json::Value& trace_params) {
  const auto block = parse_blk_num(block_num);
  auto params = parse_tracking_parms(trace_params);
  auto transactions = trace_replay_reader_.transactions_by_block_number(block);
  if (transactions.empty()) {
    return Json::Value(Json::arrayValue);
  }
  if (only_transfers(transactions)) {
    return Json::Value(Json::arrayValue);
  }
  std::vector<state_api::EVMTransaction> trxs = to_eth_trxs(transactions);
  return util::readJsonFromString(trace_reader_.trace({}, std::move(trxs), block, std::move(params)));
}

Json::Value Debug::debug_getPeriodTransactionsWithReceipts(const std::string& _period) {
  try {
    return period_transactions_reader_.transactions_with_receipts_by_period(dev::jsToInt(_period));
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Debug::debug_getPeriodDagBlocks(const std::string& _period) {
  try {
    return period_dag_blocks_reader_.finalized_dag_blocks_by_period(dev::jsToInt(_period));
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Debug::debug_getPreviousBlockCertVotes(const std::string& _period) {
  try {
    Json::Value res(Json::objectValue);
    const auto view = previous_cert_votes_reader_.cert_votes_by_period(dev::jsToInt(_period));
    if (!view.found) {
      return res;
    }

    res["total_votes_count"] = view.total_votes_count;
    res["round"] = view.round;
    res["votes"] = util::transformToJsonParallel(view.votes, [](const auto& vote, auto) { return vote->toJSON(); });
    return res;
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Debug::debug_dposValidatorTotalStakes(const std::string& _period) {
  try {
    auto period = dev::jsToInt(_period);
    auto validatorsStakes = dpos_reader_.validators_total_stakes(period);

    Json::Value res(Json::arrayValue);

    for (auto const& validatorStake : validatorsStakes) {
      Json::Value validatorStakeJson(Json::objectValue);
      validatorStakeJson["address"] = "0x" + validatorStake.addr.toString();
      validatorStakeJson["total_stake"] = validatorStake.stake.str();
      res.append(validatorStakeJson);
    }
    return res;
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

Json::Value Debug::debug_dposTotalAmountDelegated(const std::string& _period) {
  try {
    auto period = dev::jsToInt(_period);
    auto totalAmountDelegated = dpos_reader_.total_amount_delegated(period);

    return toJS(totalAmountDelegated);
  } catch (...) {
    BOOST_THROW_EXCEPTION(JsonRpcException(Errors::ERROR_RPC_INVALID_PARAMS));
  }
}

state_api::Tracing Debug::parse_tracking_parms(const Json::Value& json) const {
  state_api::Tracing ret;
  if (!json.isArray() || json.empty()) {
    throw InvalidTracingParams();
  }
  for (const auto& obj : json) {
    if (obj.asString() == "trace") ret.trace = true;
    // Disabled for now
    // if (obj.asString() == "stateDiff") ret.stateDiff = true;
    if (obj.asString() == "vmTrace") ret.vmTrace = true;
  }
  return ret;
}

std::vector<state_api::EVMTransaction> Debug::to_eth_trxs(const std::vector<std::shared_ptr<Transaction>>& trxs) {
  std::vector<state_api::EVMTransaction> eth_trxs;
  eth_trxs.reserve(trxs.size());
  std::transform(trxs.begin(), trxs.end(), std::back_inserter(eth_trxs),
                 [this](auto t) { return to_eth_trx(std::move(t)); });
  return eth_trxs;
}
state_api::EVMTransaction Debug::to_eth_trx(std::shared_ptr<Transaction> t) const {
  return state_api::EVMTransaction{
      t->getSender(), t->getGasPrice(), t->getReceiver(), t->getNonce(), t->getValue(), t->getGas(), t->getData(),
  };
}

state_api::EVMTransaction Debug::to_eth_trx(const Json::Value& json, EthBlockNumber blk_num) {
  state_api::EVMTransaction trx;
  if (!json.isObject() || json.empty()) {
    return trx;
  }

  if (!json["from"].empty()) {
    trx.from = to_address(json["from"].asString());
  } else {
    trx.from = ZeroAddress;
  }

  if (!json["to"].empty() && json["to"].asString() != "0x" && !json["to"].asString().empty()) {
    trx.to = to_address(json["to"].asString());
  }

  if (!json["value"].empty()) {
    trx.value = jsToU256(json["value"].asString());
  }

  if (!json["gas"].empty()) {
    trx.gas = jsToInt(json["gas"].asString());
  } else {
    trx.gas = kGasLimit;
  }

  if (!json["gasPrice"].empty()) {
    trx.gas_price = jsToU256(json["gasPrice"].asString());
  } else {
    trx.gas_price = 0;
  }

  if (!json["data"].empty()) {
    trx.input = jsToBytes(json["data"].asString(), OnFailed::Throw);
  }
  if (!json["code"].empty()) {
    trx.input = jsToBytes(json["code"].asString(), OnFailed::Throw);
  }
  if (!json["nonce"].empty()) {
    trx.nonce = jsToU256(json["nonce"].asString());
  } else {
    trx.nonce = trace_reader_.account_at(trx.from, blk_num).value_or(state_api::ZeroAccount).nonce;
  }

  return trx;
}

EthBlockNumber Debug::parse_blk_num(const std::string& blk_num_str) {
  if (blk_num_str == "latest" || blk_num_str == "pending" || blk_num_str.empty()) {
    return trace_reader_.latest_finalized_block_number();
  } else if (blk_num_str == "earliest") {
    return 0;
  }
  return jsToInt(blk_num_str);
}

Address Debug::to_address(const std::string& s) const {
  try {
    if (auto b = fromHex(s.substr(0, 2) == "0x" ? s.substr(2) : s, WhenError::Throw); b.size() == Address::size) {
      return Address(b);
    }
  } catch (BadHexCharacter&) {
  }
  throw InvalidAddress();
}

}  // namespace taraxa::net
